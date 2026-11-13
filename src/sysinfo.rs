//! System statistics, clocks, account info, autostart registration.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Performance::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::System::Time::*;
use windows::Win32::System::WindowsProgramming::GetUserNameW;

use crate::gfx::Color;

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub cpu: f32,
    pub mem: f32,
    pub disk: f32,
    pub gpu: Option<f32>,
    pub mem_used_gb: f32,
    pub mem_total_gb: f32,
    /// Download speed in bytes per second.
    pub net_down: Option<f32>,
    /// Download speed relative to the fastest seen recently (0..1).
    pub net_frac: f32,
    /// Upload speed in bytes per second.
    pub net_up: Option<f32>,
    /// Video memory in use and in total, in GB.
    pub vram_used: Option<f32>,
    pub vram_total: f32,
    /// Temperatures in °C.
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
}

#[derive(Clone)]
pub struct Drive {
    /// Root path such as `C:\\`.
    pub root: String,
    pub total: u64,
    pub free: u64,
}

/// Fixed and removable drives that currently hold a volume.
pub fn drives() -> Vec<Drive> {
    let mut out = Vec::new();
    unsafe {
        let mask = GetLogicalDrives();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            let kind = GetDriveTypeW(&HSTRING::from(root.as_str()));
            if kind != 3 && kind != 2 {
                continue;
            }
            let (mut total, mut free) = (0u64, 0u64);
            if GetDiskFreeSpaceExW(&HSTRING::from(root.as_str()), None, Some(&mut total), Some(&mut free)).is_ok() && total > 0 {
                out.push(Drive { root, total, free });
            }
        }
    }
    out
}

pub fn fmt_size(bytes: u64) -> String {
    let gb = bytes as f64 / 1073741824.0;
    if gb >= 1000.0 {
        format!("{:.1} TB", gb / 1024.0)
    } else if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// Compact transfer rate: "0", "840K", "3.6M".
pub fn fmt_rate(bps: f32) -> String {
    if bps >= 1048576.0 {
        format!("{:.1}M", bps / 1048576.0)
    } else if bps >= 1024.0 {
        format!("{:.0}K", bps / 1024.0)
    } else {
        "0".into()
    }
}

/// One wildcard performance counter ("GPU Engine", "Network Interface").
struct Counter {
    q: PDH_HQUERY,
    c: PDH_HCOUNTER,
}

impl Counter {
    unsafe fn open(path: PCWSTR) -> Option<Counter> {
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 {
            return None;
        }
        let mut c = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(q, path, 0, &mut c) != 0 {
            PdhCloseQuery(q);
            return None;
        }
        PdhCollectQueryData(q);
        Some(Counter { q, c })
    }

    /// Returns (sum, largest) over all instances of the counter.
    unsafe fn read(&self) -> Option<(f64, f64)> {
        if PdhCollectQueryData(self.q) != 0 {
            return None;
        }
        let (mut size, mut count) = (0u32, 0u32);
        PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if size == 0 {
            return None;
        }
        // u64 backing store keeps the buffer aligned for the item structs.
        let mut buf = vec![0u64; size as usize / 8 + 2];
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        if PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
            return None;
        }
        let (mut sum, mut max) = (0.0f64, 0.0f64);
        for i in 0..count as usize {
            let it = &*items.add(i);
            if it.FmtValue.CStatus == 0 {
                let v = it.FmtValue.Anonymous.doubleValue;
                sum += v;
                max = max.max(v);
            }
        }
        Some((sum, max))
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.q);
        }
    }
}

pub struct Sampler {
    prev_idle: u64,
    prev_total: u64,
    gpu: Option<Counter>,
    gpu_tried: bool,
    net: Option<Counter>,
    net_tried: bool,
    net_peak: f32,
    net_up: Option<Counter>,
    gpu_dev: Option<crate::sensors::Gpu>,
    gpu_dev_tried: bool,
    lhm: crate::sensors::Lhm,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            prev_idle: 0,
            prev_total: 0,
            gpu: None,
            gpu_tried: false,
            net: None,
            net_tried: false,
            net_peak: 0.0,
            net_up: None,
            gpu_dev: None,
            gpu_dev_tried: false,
            lhm: crate::sensors::Lhm::new(),
        }
    }

/// Video memory and temperatures (read only while the stats are on screen).
    fn extra(&mut self, s: &mut Stats) {
        if !self.gpu_dev_tried {
            self.gpu_dev_tried = true;
            self.gpu_dev = crate::sensors::Gpu::open();
        }
        if let Some(g) = &self.gpu_dev {
            s.vram_total = g.vram_total as f32 / 1073741824.0;
            s.vram_used = g.vram_used().map(|b| b as f32 / 1073741824.0);
            s.gpu_temp = g.temperature();
        }
        s.cpu_temp = crate::sensors::helper_cpu_temp();
        if s.cpu_temp.is_none() || s.gpu_temp.is_none() {
            // LibreHardwareMonitor's own app, if it happens to be running.
            let (cpu, gpu) = self.lhm.read();
            s.cpu_temp = s.cpu_temp.or(cpu);
            s.gpu_temp = s.gpu_temp.or(gpu);
        }
    }

    pub fn sample(&mut self, want_gpu: bool, want_net: bool, want_extra: bool) -> Stats {
        let mut s = Stats::default();
        unsafe {
            let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
                let idle = ft(idle);
                let total = ft(kernel) + ft(user);
                let (di, dt) = (idle.saturating_sub(self.prev_idle), total.saturating_sub(self.prev_total));
                if self.prev_total != 0 && dt > 0 {
                    s.cpu = (1.0 - di as f32 / dt as f32).clamp(0.0, 1.0);
                }
                self.prev_idle = idle;
                self.prev_total = total;
            }

            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if GlobalMemoryStatusEx(&mut ms).is_ok() {
                s.mem = ms.dwMemoryLoad as f32 / 100.0;
                s.mem_total_gb = ms.ullTotalPhys as f32 / 1073741824.0;
                s.mem_used_gb = (ms.ullTotalPhys - ms.ullAvailPhys) as f32 / 1073741824.0;
            }

            let (mut total, mut free) = (0u64, 0u64);
            if GetDiskFreeSpaceExW(w!("C:\\"), None, Some(&mut total), Some(&mut free)).is_ok() && total > 0 {
                s.disk = 1.0 - free as f32 / total as f32;
            }

            if want_gpu {
                if !self.gpu_tried {
                    self.gpu_tried = true;
                    self.gpu = Counter::open(w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"));
                } else if let Some(c) = &self.gpu {
                    s.gpu = c.read().map(|(sum, _)| (sum as f32 / 100.0).clamp(0.0, 1.0));
                }
            }
            if want_net {
                if !self.net_tried {
                    self.net_tried = true;
                    self.net = Counter::open(w!("\\Network Interface(*)\\Bytes Received/sec"));
                    self.net_up = Counter::open(w!("\\Network Interface(*)\\Bytes Sent/sec"));
                } else if let Some(c) = &self.net {
                    if let Some(up) = &self.net_up {
                        s.net_up = up.read().map(|(_, max)| max as f32);
                    }
                    // The busiest adapter: a VPN tunnel and its physical adapter carry
                    // the same traffic, so adding them up would double it.
                    if let Some((_, max)) = c.read() {
                        let v = max as f32;
                        self.net_peak = (self.net_peak * 0.995).max(v).max(1048576.0);
                        s.net_down = Some(v);
                        s.net_frac = (v / self.net_peak).clamp(0.0, 1.0);
                    }
                }
            }
        }
        if want_extra {
            self.extra(&mut s);
        }
        s
    }
}

pub fn local_time() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

/// Looks up a Windows time zone by its registry key name ("Tokyo Standard Time").
pub fn find_tz(name: &str) -> Option<DYNAMIC_TIME_ZONE_INFORMATION> {
    if name.is_empty() {
        return None;
    }
    unsafe {
        let mut i = 0;
        loop {
            let mut tz = DYNAMIC_TIME_ZONE_INFORMATION::default();
            if EnumDynamicTimeZoneInformation(i, &mut tz) != 0 {
                return None;
            }
            let len = tz.TimeZoneKeyName.iter().position(|&c| c == 0).unwrap_or(tz.TimeZoneKeyName.len());
            if String::from_utf16_lossy(&tz.TimeZoneKeyName[..len]).eq_ignore_ascii_case(name) {
                return Some(tz);
            }
            i += 1;
        }
    }
}

/// Minutes between the time in `tz` and UTC right now (DST included).
pub fn utc_offset_min(tz: &Option<DYNAMIC_TIME_ZONE_INFORMATION>) -> i64 {
    let utc = unsafe { GetSystemTime() };
    let local = time_in(tz);
    let m = |t: &SYSTEMTIME| {
        days_from_civil(t.wYear as i64, t.wMonth as i64, t.wDay as i64) * 1440 + t.wHour as i64 * 60 + t.wMinute as i64
    };
    m(&local) - m(&utc)
}

pub fn fmt_offset(m: i64) -> String {
    let sign = if m < 0 { '−' } else { '+' };
    if m % 60 == 0 {
        format!("UTC{sign}{}", m.abs() / 60)
    } else {
        format!("UTC{sign}{}:{:02}", m.abs() / 60, m.abs() % 60)
    }
}

pub fn time_in(tz: &Option<DYNAMIC_TIME_ZONE_INFORMATION>) -> SYSTEMTIME {
    unsafe {
        match tz {
            None => GetLocalTime(),
            Some(tz) => {
                let utc = GetSystemTime();
                let mut out = SYSTEMTIME::default();
                if SystemTimeToTzSpecificLocalTimeEx(Some(tz), &utc, &mut out).is_ok() {
                    out
                } else {
                    GetLocalTime()
                }
            }
        }
    }
}

pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
pub const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Days since 1970-01-01 for a civil date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 0 = Sunday.
pub fn weekday(y: i64, m: i64, d: i64) -> i64 {
    (days_from_civil(y, m, d) + 4).rem_euclid(7)
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

pub fn format_time(t: &SYSTEMTIME, h24: bool) -> (String, &'static str) {
    if h24 {
        (format!("{:02}:{:02}", t.wHour, t.wMinute), "")
    } else {
        let h = match t.wHour % 12 {
            0 => 12,
            h => h,
        };
        (format!("{}:{:02}", h, t.wMinute), if t.wHour < 12 { "AM" } else { "PM" })
    }
}

fn user_sid() -> Option<String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; len as usize / 8 + 2];
        let ok = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr() as *mut _), len, &mut len).is_ok();
        let _ = CloseHandle(tok);
        if !ok {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        out
    }
}

fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 520];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        if r != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// Path of the signed-in user's account picture, if one can be found.
pub fn avatar_path() -> Option<String> {
    let sid = user_sid()?;
    let key = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AccountPicture\\Users\\{sid}");
    for v in ["Image208", "Image192", "Image240", "Image96"] {
        if let Some(p) = reg_string(HKEY_LOCAL_MACHINE, &key, v) {
            if std::path::Path::new(&p).exists() {
                return Some(p);
            }
        }
    }
    let dir = format!("C:\\Users\\Public\\AccountPictures\\{sid}");
    let mut best: Option<(u64, String)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "jpg" || ext == "png" {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().map(|b| len > b.0).unwrap_or(true) {
                best = Some((len, p.to_string_lossy().into_owned()));
            }
        }
    }
    best.map(|b| b.1)
}

pub fn user_name() -> String {
    unsafe {
        // Full display name ("Jane Doe") when the account has one.
        let mut buf = [0u16; 257];
        let mut len = buf.len() as u32;
        if GetUserNameExW(NameDisplay, Some(PWSTR(buf.as_mut_ptr())), &mut len) && len > 0 {
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            if !name.trim().is_empty() {
                return name;
            }
        }
        let mut buf = [0u16; 257];
        let mut len = buf.len() as u32;
        if GetUserNameW(Some(PWSTR(buf.as_mut_ptr())), &mut len).is_ok() && len > 1 {
            return String::from_utf16_lossy(&buf[..len as usize - 1]);
        }
    }
    std::env::var("USERNAME").unwrap_or_else(|_| "User".into())
}

pub fn system_accent() -> Color {
    unsafe {
        let mut c = 0u32;
        let mut opaque = BOOL(0);
        if DwmGetColorizationColor(&mut c, &mut opaque).is_ok() {
            return Color::rgb(c & 0xffffff);
        }
    }
    Color::rgb(0x3b9cff)
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "OrbitStart";

pub fn autostart_enabled() -> bool {
    reg_string(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME).is_some()
}

/// Windows only launches a Run entry at sign-in when it also has an
/// "approved" record here (the same one Task Manager's Startup tab writes).
const APPROVED_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

/// True when the sign-in registration is already in place for this
/// executable (or the user switched it off in Task Manager, which we respect).
pub fn autostart_ok() -> bool {
    let Ok(exe) = std::env::current_exe() else { return true };
    if reg_string(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME) != Some(format!("\"{}\"", exe.display())) {
        return false;
    }
    unsafe {
        let mut data = [0u8; 16];
        let mut size = data.len() as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(APPROVED_KEY),
            &HSTRING::from(RUN_NAME),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        r == ERROR_SUCCESS && size >= 1
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let cmd: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(RUN_NAME),
                REG_SZ.0,
                Some(cmd.as_ptr() as *const _),
                (cmd.len() * 2) as u32,
            );
            // 02 = enabled, followed by an (unused) timestamp.
            let approved = [2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(APPROVED_KEY),
                &HSTRING::from(RUN_NAME),
                REG_BINARY.0,
                Some(approved.as_ptr() as *const _),
                approved.len() as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME));
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_KEY), &HSTRING::from(RUN_NAME));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_math() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(2026, 11, 10), 2); // Tuesday
        assert_eq!(weekday(2026, 10, 1), 4); // Thursday
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }
}
