//! GPU memory and temperatures.
//!
//! * VRAM: total from DXGI, current use from the "GPU Adapter Memory"
//!   performance counter of the same adapter.
//! * GPU temperature: the same driver query Task Manager uses
//!   (D3DKMT adapter performance data), no vendor library needed.
//! * CPU temperature: Windows has no reliable built-in source (the ACPI
//!   "thermal zone" is usually a fixed fake value), and reading the real sensor
//!   needs a kernel driver. We read it from LibreHardwareMonitor when that is
//!   running, which publishes its sensors through WMI.

use std::time::{Duration, Instant};

use windows::core::*;
use windows::Wdk::Graphics::Direct3D::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Performance::*;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::*;

pub struct Gpu {
    /// Dedicated video memory in bytes.
    pub vram_total: u64,
    vram_q: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
    kmt: Option<u32>,
}

impl Gpu {
    /// The hardware adapter with the most dedicated memory.
    pub fn open() -> Option<Gpu> {
        unsafe {
            let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let mut best: Option<DXGI_ADAPTER_DESC1> = None;
            let mut i = 0;
            while let Ok(a) = f.EnumAdapters1(i) {
                i += 1;
                let Ok(d) = a.GetDesc1() else { continue };
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                if best.as_ref().map(|b| d.DedicatedVideoMemory > b.DedicatedVideoMemory).unwrap_or(true) {
                    best = Some(d);
                }
            }
            let d = best?;
            let luid = d.AdapterLuid;
            let path = format!(
                "\\GPU Adapter Memory(luid_0x{:08x}_0x{:08x}_phys_0)\\Dedicated Usage",
                luid.HighPart as u32, luid.LowPart
            );
            let mut q = PDH_HQUERY::default();
            let mut vram_q = None;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) == 0 {
                let mut c = PDH_HCOUNTER::default();
                if PdhAddEnglishCounterW(q, &HSTRING::from(path), 0, &mut c) == 0 {
                    PdhCollectQueryData(q);
                    vram_q = Some((q, c));
                } else {
                    PdhCloseQuery(q);
                }
            }
            let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: luid, hAdapter: 0 };
            let kmt = (D3DKMTOpenAdapterFromLuid(&mut open).is_ok()).then_some(open.hAdapter);
            Some(Gpu {
                vram_total: d.DedicatedVideoMemory as u64,
                vram_q,
                kmt,
            })
        }
    }

    /// Dedicated video memory in use, in bytes.
    pub fn vram_used(&self) -> Option<u64> {
        let (q, c) = self.vram_q?;
        unsafe {
            if PdhCollectQueryData(q) != 0 {
                return None;
            }
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(c, PDH_FMT_LARGE, None, &mut v) != 0 || v.CStatus != 0 {
                return None;
            }
            Some(v.Anonymous.largeValue.max(0) as u64)
        }
    }

    /// GPU temperature in °C, if the driver reports it.
    pub fn temperature(&self) -> Option<f32> {
        let h = self.kmt?;
        unsafe {
            let mut perf = D3DKMT_ADAPTER_PERFDATA { PhysicalAdapterIndex: 0, ..Default::default() };
            let mut q = D3DKMT_QUERYADAPTERINFO {
                hAdapter: h,
                Type: KMTQAITYPE_ADAPTERPERFDATA,
                pPrivateDriverData: &mut perf as *mut _ as *mut _,
                PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
            };
            if D3DKMTQueryAdapterInfo(&mut q).is_err() || perf.Temperature == 0 {
                return None;
            }
            // Reported in tenths of a degree Celsius.
            Some(perf.Temperature as f32 / 10.0)
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            if let Some((q, _)) = self.vram_q.take() {
                PdhCloseQuery(q);
            }
            if let Some(h) = self.kmt.take() {
                let close = D3DKMT_CLOSEADAPTER { hAdapter: h };
                let _ = D3DKMTCloseAdapter(&close);
            }
        }
    }
}

/// Temperatures published by LibreHardwareMonitor, when it is running.
pub struct Lhm {
    svc: Option<IWbemServices>,
    next_try: Instant,
}

impl Lhm {
    pub fn new() -> Lhm {
        Lhm { svc: None, next_try: Instant::now() }
    }

    fn connect(&mut self) {
        if self.svc.is_some() || Instant::now() < self.next_try {
            return;
        }
        // Not running? Look again in half a minute.
        self.next_try = Instant::now() + Duration::from_secs(30);
        unsafe {
            let Ok(loc) = CoCreateInstance::<_, IWbemLocator>(&WbemLocator, None, CLSCTX_INPROC_SERVER) else { return };
            let empty = BSTR::new();
            let Ok(svc) = loc.ConnectServer(
                &BSTR::from("root\\LibreHardwareMonitor"),
                &empty,
                &empty,
                &empty,
                0,
                &empty,
                None,
            ) else {
                return;
            };
            let _ = CoSetProxyBlanket(
                &svc,
                10, // RPC_C_AUTHN_WINNT
                0,  // RPC_C_AUTHZ_NONE
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            );
            self.svc = Some(svc);
        }
    }

    /// (CPU package temperature, GPU core temperature) in °C.
    pub fn read(&mut self) -> (Option<f32>, Option<f32>) {
        self.connect();
        let Some(svc) = self.svc.clone() else { return (None, None) };
        let (mut cpu, mut cpu_any, mut gpu) = (None, None::<f32>, None);
        unsafe {
            let query = BSTR::from("SELECT Name, Identifier, Value FROM Sensor WHERE SensorType='Temperature'");
            let Ok(en) = svc.ExecQuery(&BSTR::from("WQL"), &query, WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)
            else {
                // LibreHardwareMonitor was closed.
                self.svc = None;
                return (None, None);
            };
            loop {
                let mut objs = [None];
                let mut got = 0u32;
                if en.Next(1000, &mut objs, &mut got).is_err() || got == 0 {
                    break;
                }
                let Some(o) = objs[0].take() else { break };
                let text = |name: PCWSTR| {
                    let mut v = VARIANT::default();
                    o.Get(name, 0, &mut v, None, None).ok().and_then(|_| BSTR::try_from(&v).ok()).map(|b| b.to_string())
                };
                let mut v = VARIANT::default();
                let Some(value) = o.Get(w!("Value"), 0, &mut v, None, None).ok().and_then(|_| f64::try_from(&v).ok()) else {
                    continue;
                };
                let value = value as f32;
                let id = text(w!("Identifier")).unwrap_or_default().to_lowercase();
                let name = text(w!("Name")).unwrap_or_default().to_lowercase();
                if id.contains("cpu") {
                    if name.contains("package") || name.contains("tctl") || name.contains("tdie") {
                        cpu = Some(value);
                    }
                    cpu_any = Some(cpu_any.map_or(value, |m: f32| m.max(value)));
                } else if id.contains("gpu") && (name == "gpu core" || name.contains("core")) && gpu.is_none() {
                    gpu = Some(value);
                }
            }
        }
        (cpu.or(cpu_any), gpu)
    }
}

/// CPU temperature written by the Orbit Start sensor helper
/// (`%ProgramData%\\OrbitStart\\sensors.txt`), if it is fresh.
pub fn helper_cpu_temp() -> Option<f32> {
    let dir = std::env::var_os("ProgramData")?;
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("OrbitStart").join("sensors.txt")).ok()?;
    let mut cpu = None;
    let mut time = 0i64;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("cpu=") {
            cpu = v.trim().parse::<f32>().ok();
        } else if let Some(v) = line.strip_prefix("time=") {
            time = v.trim().parse().unwrap_or(0);
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    // The helper writes every 2 seconds; older data means it has stopped.
    if (now - time).abs() > 15 {
        return None;
    }
    cpu.filter(|t| *t > 0.0 && *t < 125.0)
}
