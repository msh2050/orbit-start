//! Open application windows: listing, activating, and noticing when the
//! last window of an app closes.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone)]
pub struct Win {
    pub hwnd: isize,
    pub exe: String,
    pub title: String,
}

#[derive(Clone)]
pub struct Closed {
    pub exe: String,
    pub name: String,
}

pub fn exe_of(hwnd: isize) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 600];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_FORMAT(0), PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn title_of(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

/// True for windows that would get a taskbar button.
pub fn is_app_window(hwnd: isize) -> bool {
    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        if GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        // Store apps keep hidden ("cloaked") windows around; skip those.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        cloaked == 0
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<Win>);
    let raw = hwnd.0 as isize;
    if is_app_window(raw) {
        let title = title_of(hwnd);
        if !title.is_empty() {
            if let Some(exe) = exe_of(raw) {
                out.push(Win { hwnd: raw, exe, title });
            }
        }
    }
    BOOL(1)
}

pub fn enumerate() -> Vec<Win> {
    let mut out: Vec<Win> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut out as *mut _ as isize));
    }
    out.retain(|w| !file_name(&w.exe).eq_ignore_ascii_case("orbit-start.exe"));
    out
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

pub fn display_name(exe: &str) -> String {
    let f = file_name(exe);
    f.strip_suffix(".exe").or_else(|| f.strip_suffix(".EXE")).unwrap_or(f).to_string()
}

/// Whether reopening this executable would bring the closed app back. Host
/// processes (Store app frames, Explorer, system UI) are left out.
pub fn reopenable(exe: &str) -> bool {
    let f = file_name(exe).to_ascii_lowercase();
    let lower = exe.to_ascii_lowercase();
    !matches!(
        f.as_str(),
        "applicationframehost.exe"
            | "explorer.exe"
            | "orbit-start.exe"
            | "textinputhost.exe"
            | "shellexperiencehost.exe"
            | "startmenuexperiencehost.exe"
            | "searchhost.exe"
            | "systemsettings.exe"
            | "lockapp.exe"
    ) && !lower.contains("\\windowsapps\\")
        && !lower.contains("\\systemapps\\")
}

pub fn alive(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}

pub fn activate(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}
