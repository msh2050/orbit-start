//! Orbit Start - a lightweight, customizable Start menu for Windows 11.
#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod apps;
mod calc;
mod config;
mod gfx;
mod news;
mod sysinfo;
mod taskbar;
mod text;
mod ui;
mod wins;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::Instant;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use config::{Backdrop, Config, Layout, Position, Widget};
use gfx::{Color, Gfx};
use ui::{App, Cmd, Pending};

const WM_TOGGLE: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const WM_APPS_READY: u32 = WM_APP + 3;
const WM_FILES_READY: u32 = WM_APP + 4;
const WM_ICON_READY: u32 = WM_APP + 5;
const WM_NEWS_READY: u32 = WM_APP + 6;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CF_UNICODETEXT: u32 = 13;
const WM_CLICKAWAY: u32 = WM_APP + 7;
const CLASS: PCWSTR = w!("OrbitStartMenu");
const ISLAND_CLASS: PCWSTR = w!("OrbitStartIsland");
/// Island layout, in layout units: stats pill, gap, folder pill.
const ISL_STATS_W: f32 = 300.0;
const ISL_GAP: f32 = 8.0;
const ISL_BTN: f32 = 40.0;
const ISL_FOLDERS: [(&str, &str); 3] = [("\u{E896}", "shell:Downloads"), ("\u{E8A5}", "shell:Personal"), ("\u{E8B7}", "shell:Profile")];
const ANIM_STEPS: u8 = 7;
/// Marks keystrokes we inject ourselves so the hook ignores them.
const MAGIC: usize = 0x4F52_4254;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static HOOK_ON: AtomicBool = AtomicBool::new(false);
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static WIN_OTHER: AtomicBool = AtomicBool::new(false);
static WIN_TIME: AtomicU32 = AtomicU32::new(0);
/// Redirect clicks on the taskbar Start button to this menu.
static START_ON: AtomicBool = AtomicBool::new(false);
static SWALLOW_UP: AtomicBool = AtomicBool::new(false);
/// Whether the menu is on screen, and where (physical pixels), for click-away.
static MENU_VISIBLE: AtomicBool = AtomicBool::new(false);
static MENU_RECT: [std::sync::atomic::AtomicI32; 4] = [
    std::sync::atomic::AtomicI32::new(0),
    std::sync::atomic::AtomicI32::new(0),
    std::sync::atomic::AtomicI32::new(0),
    std::sync::atomic::AtomicI32::new(0),
];
static NO_HOOKS: AtomicBool = AtomicBool::new(false);
/// Message id Windows uses to tell us about app windows opening and closing.
static SHELLHOOK: AtomicU32 = AtomicU32::new(0);
/// Message Explorer broadcasts when it (re)creates the taskbar.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// Appends a line to %APPDATA%\\OrbitStart\\log.txt (kept small).
fn log(msg: &str) {
    use std::io::Write;
    let path = Config::dir().join("log.txt");
    let _ = std::fs::create_dir_all(Config::dir());
    if std::fs::metadata(&path).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let t = sysinfo::local_time();
        let _ = writeln!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}  {}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, msg
        );
    }
}
static TASKBAR_TX: Mutex<Option<Sender<()>>> = Mutex::new(None);

/// Asks the taskbar thread to re-read button positions and re-apply the shape.
fn ping_taskbar() {
    if let Ok(tx) = TASKBAR_TX.lock() {
        if let Some(tx) = tx.as_ref() {
            let _ = tx.send(());
        }
    }
}

fn sync_taskbar_settings() {
    if let Some((style, start)) = with(|s| (s.app.cfg.taskbar_style, s.app.cfg.hook_start_button)) {
        taskbar::set_style(style);
        START_ON.store(start && !NO_HOOKS.load(Ordering::Relaxed), Ordering::Relaxed);
        ping_taskbar();
    }
}

struct Shell {
    hwnd: HWND,
    gfx: Gfx,
    app: App,
    visible: bool,
    /// Debug: keep the menu open when it loses focus.
    pin: bool,
    /// A dialog or popup menu of ours is open; do not auto-hide.
    modal: bool,
    /// Physical pixels per layout unit.
    k: f32,
    last_hide: Instant,
    tracking: bool,
    /// The menu has been the active window since it was opened.
    was_active: bool,
    /// Seconds in a row that another window has been in front.
    away: u8,
    /// Remaining steps of the open animation.
    anim: u8,
    /// Small always-on-top window drawn in the taskbar's free space.
    island: Option<(HWND, Gfx)>,
    /// The island is showing and samples the system stats itself.
    island_live: bool,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    static HIGH_SURROGATE: Cell<u16> = const { Cell::new(0) };
}

/// Runs `f` on the UI state. Returns None if the state is busy (re-entrant
/// window message) or not created yet.
fn with<R>(f: impl FnOnce(&mut Shell) -> R) -> Option<R> {
    SHELL.with(|s| s.try_borrow_mut().ok().and_then(|mut o| o.as_mut().map(f)))
}

fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(Ordering::Relaxed) as *mut _)
}

fn invalidate() {
    unsafe {
        let _ = InvalidateRect(Some(main_hwnd()), None, false);
    }
}

// ------------------------------------------------------------ Windows key

unsafe extern "system" fn kb_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && HOOK_ON.load(Ordering::Relaxed) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        if k.dwExtraInfo != MAGIC {
            let is_win = k.vkCode == VK_LWIN.0 as u32 || k.vkCode == VK_RWIN.0 as u32;
            let msg = wp.0 as u32;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                // Ctrl+Esc is the other keyboard way into the Windows Start menu.
                // (Ctrl+Shift+Esc is Task Manager and is left alone.)
                if k.vkCode == VK_ESCAPE.0 as u32
                    && GetAsyncKeyState(VK_CONTROL.0 as i32) < 0
                    && GetAsyncKeyState(VK_SHIFT.0 as i32) >= 0
                    && GetAsyncKeyState(VK_MENU.0 as i32) >= 0
                {
                    let _ = PostMessageW(Some(main_hwnd()), WM_TOGGLE, WPARAM(0), LPARAM(0));
                    return LRESULT(1);
                }
                if is_win {
                    // A fresh press (not auto-repeat) starts a new "lone Win key" candidate.
                    let stale = k.time.wrapping_sub(WIN_TIME.load(Ordering::Relaxed)) > 1100;
                    if !WIN_DOWN.swap(true, Ordering::Relaxed) || stale {
                        WIN_OTHER.store(false, Ordering::Relaxed);
                    }
                    WIN_TIME.store(k.time, Ordering::Relaxed);
                } else if WIN_DOWN.load(Ordering::Relaxed) {
                    WIN_OTHER.store(true, Ordering::Relaxed);
                }
            } else if is_win && WIN_DOWN.swap(false, Ordering::Relaxed) && !WIN_OTHER.load(Ordering::Relaxed) {
                // Win was tapped on its own. Swallow the release and replay it
                // behind a no-op key so Windows does not open its own Start menu.
                let key = |vk: u16, up: bool| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            wScan: 0,
                            dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                            time: 0,
                            dwExtraInfo: MAGIC,
                        },
                    },
                };
                let seq = [key(0xE8, false), key(0xE8, true), key(k.vkCode as u16, true)];
                SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
                let _ = PostMessageW(Some(main_hwnd()), WM_TOGGLE, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let msg = wp.0 as u32;
        if (msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN || msg == WM_MBUTTONDOWN) && MENU_VISIBLE.load(Ordering::Relaxed) {
            // A click anywhere outside the open menu closes it, even if the
            // menu never managed to become the active window.
            let m = &*(lp.0 as *const MSLLHOOKSTRUCT);
            let r = |i: usize| MENU_RECT[i].load(Ordering::Relaxed);
            let inside = m.pt.x >= r(0) && m.pt.x < r(2) && m.pt.y >= r(1) && m.pt.y < r(3);
            if !inside && !(msg == WM_LBUTTONDOWN && START_ON.load(Ordering::Relaxed) && taskbar::point_on_start(m.pt)) {
                let _ = PostMessageW(Some(main_hwnd()), WM_CLICKAWAY, WPARAM(0), LPARAM(0));
            }
        }
        if msg == WM_LBUTTONDOWN {
            if START_ON.load(Ordering::Relaxed) {
                let m = &*(lp.0 as *const MSLLHOOKSTRUCT);
                if taskbar::point_on_start(m.pt) {
                    // Take the click away from the Windows Start button.
                    SWALLOW_UP.store(true, Ordering::Relaxed);
                    let _ = PostMessageW(Some(main_hwnd()), WM_TOGGLE, WPARAM(0), LPARAM(0));
                    return LRESULT(1);
                }
            }
        } else if msg == WM_LBUTTONUP && SWALLOW_UP.swap(false, Ordering::Relaxed) {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

// --------------------------------------------------------------- backdrop

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionAttr {
    attr: u32,
    data: *mut core::ffi::c_void,
    size: usize,
}

unsafe fn set_accent(hwnd: HWND, state: u32, gradient: u32) {
    type Func = unsafe extern "system" fn(HWND, *mut CompositionAttr) -> BOOL;
    let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
    let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
    let f: Func = std::mem::transmute(f);
    let mut policy = AccentPolicy { state, flags: 0, gradient, animation: 0 };
    let mut data = CompositionAttr {
        attr: 19,
        data: &mut policy as *mut _ as *mut _,
        size: std::mem::size_of::<AccentPolicy>(),
    };
    let _ = f(hwnd, &mut data);
}

unsafe fn dwm_attr(hwnd: HWND, attr: i32, value: i32) {
    let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(attr), &value as *const _ as *const _, 4);
}

unsafe fn apply_backdrop(hwnd: HWND, backdrop: Backdrop, bg: Color, light: bool) {
    dwm_attr(hwnd, 20, !light as i32); // immersive dark mode
    dwm_attr(hwnd, 33, 2); // rounded corners
    let (margin, system, accent, gradient) = match backdrop {
        Backdrop::Acrylic => (0, 1, 4, 0x0100_0000 | bg.to_bgr()),
        Backdrop::Blur => (0, 1, 3, 0),
        Backdrop::Mica => (-1, 2, 0, 0),
        Backdrop::Solid => (0, 1, 0, 0),
    };
    let m = MARGINS { cxLeftWidth: margin, cxRightWidth: margin, cyTopHeight: margin, cyBottomHeight: margin };
    let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
    dwm_attr(hwnd, 38, system);
    set_accent(hwnd, accent, gradient);
}

// ------------------------------------------------------------ show / hide

/// Sizes the menu for the monitor under the cursor and returns its window
/// rectangle in physical pixels.
unsafe fn place(s: &mut Shell) -> (i32, i32, i32, i32) {
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut mi);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    let wa = mi.rcWork;
    let (aw, ah) = ((wa.right - wa.left) as f32, (wa.bottom - wa.top) as f32);
    let dpi = dx as f32 / 96.0;
    let margin = 12.0 * dpi;
    let k = (dpi * s.app.cfg.scale)
        .min((ah - 2.0 * margin) / s.app.h)
        .min((aw - 2.0 * margin) / s.app.w)
        .max(0.3);
    let (w, h) = ((s.app.w * k).round(), (s.app.h * k).round());
    let cx = wa.left as f32 + (aw - w) / 2.0;
    let bottom = wa.bottom as f32 - h - margin;
    let (x, y) = match s.app.cfg.position {
        Position::BottomCenter => (cx, bottom),
        Position::BottomLeft => (wa.left as f32 + margin, bottom),
        Position::BottomRight => (wa.right as f32 - w - margin, bottom),
        Position::Center => (cx, wa.top as f32 + (ah - h) / 2.0),
    };
    s.k = k;
    s.gfx.resize(w as u32, h as u32);
    s.gfx.set_scale(k);
    s.app.icon_px = ((48.0 * k).round() as i32).clamp(32, 128);
    (x.round() as i32, y.round() as i32, w as i32, h as i32)
}

unsafe fn force_foreground(hwnd: HWND) {
    let _ = SetForegroundWindow(hwnd);
    let fg = GetForegroundWindow();
    if fg != hwnd && !fg.is_invalid() {
        let other = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        if other != 0 && other != me {
            let _ = AttachThreadInput(me, other, true);
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            let _ = AttachThreadInput(me, other, false);
        }
    }
    if GetForegroundWindow() != hwnd {
        // Last resort: a synthetic Alt tap lifts Windows' foreground lock.
        let key = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_MENU,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: MAGIC,
                },
            },
        };
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd);
    }
    let _ = SetFocus(Some(hwnd));
}

/// Recomputes size, position and backdrop, then repaints.
unsafe fn relayout(show: bool) {
    let hwnd = main_hwnd();
    let Some((rect, backdrop, bg, light)) = with(|s| {
        let rect = place(s);
        if show {
            s.app.on_show();
            s.visible = true;
        }
        s.app.draw(&mut s.gfx);
        (rect, s.app.cfg.backdrop, s.app.theme.bg, s.app.theme.light)
    }) else {
        return;
    };
    apply_backdrop(hwnd, backdrop, bg, light);
    for (i, v) in [rect.0, rect.1, rect.0 + rect.2, rect.1 + rect.3].into_iter().enumerate() {
        MENU_RECT[i].store(v, Ordering::Relaxed);
    }
    let flags = if show { SWP_SHOWWINDOW } else { SWP_NOACTIVATE };
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.0, rect.1, rect.2, rect.3, flags);
}

unsafe fn show_menu() {
    // Start slightly low and slide up into place.
    with(|s| {
        s.was_active = false;
        s.away = 0;
        s.anim = ANIM_STEPS;
        s.gfx.set_offset(18.0);
    });
    relayout(true);
    let hwnd = main_hwnd();
    MENU_VISIBLE.store(true, Ordering::Relaxed);
    force_foreground(hwnd);
    SetTimer(Some(hwnd), 1, 1000, None);
    SetTimer(Some(hwnd), 2, 12, None);
    ping_taskbar();
    run_pending();
}

unsafe fn hide_menu() {
    let was_visible = with(|s| {
        if !s.visible {
            return false;
        }
        s.visible = false;
        s.last_hide = Instant::now();
        s.app.on_hide();
        // Drop the full-size drawing buffers; they are recreated on the next open.
        s.gfx.resize(1, 1);
        s.gfx.trim();
        true
    });
    if was_visible != Some(true) {
        return;
    }
    let hwnd = main_hwnd();
    MENU_VISIBLE.store(false, Ordering::Relaxed);
    let _ = ShowWindow(hwnd, SW_HIDE);
    let _ = KillTimer(Some(hwnd), 1);
    let _ = KillTimer(Some(hwnd), 2);
    // Hand idle memory back to the system while the menu is closed.
    let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
}

unsafe fn toggle() {
    match with(|s| s.visible) {
        Some(true) => hide_menu(),
        Some(false) => show_menu(),
        None => {}
    }
}

// ---------------------------------------------------------------- actions

unsafe fn popup(items: &[(usize, &str)]) -> usize {
    let hwnd = main_hwnd();
    let Ok(menu) = CreatePopupMenu() else { return 0 };
    for (id, label) in items {
        if *id == 0 {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        } else {
            let _ = AppendMenuW(menu, MF_STRING, *id, &HSTRING::from(*label));
        }
    }
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    with(|s| s.modal = true);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
    with(|s| s.modal = false);
    let _ = DestroyMenu(menu);
    cmd.0 as usize
}

unsafe fn power(cmd: Cmd) {
    use std::os::windows::process::CommandExt;
    let shutdown = |arg: &str| {
        let _ = std::process::Command::new("shutdown.exe").args([arg, "/t", "0"]).creation_flags(0x0800_0000).spawn();
    };
    match cmd {
        Cmd::Lock => {
            let _ = LockWorkStation();
        }
        Cmd::Sleep => {
            SetSuspendState(false, false, false);
        }
        Cmd::SignOut => {
            let _ = ExitWindowsEx(EWX_LOGOFF, SHUTDOWN_REASON(0));
        }
        Cmd::Restart => shutdown("/r"),
        Cmd::Shutdown => shutdown("/s"),
        Cmd::Quit => {
            let _ = DestroyWindow(main_hwnd());
        }
        Cmd::Customize => {}
    }
}

unsafe fn clipboard_text(hwnd: HWND) -> Option<String> {
    OpenClipboard(Some(hwnd)).ok()?;
    let mut out = None;
    if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            let mut len = 0;
            while len < 100_000 && *p.add(len) != 0 {
                len += 1;
            }
            out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    out
}

unsafe fn set_clipboard_text(hwnd: HWND, s: &str) {
    let wide: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else { return };
    let p = GlobalLock(hg) as *mut u16;
    if p.is_null() {
        return;
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
    let _ = GlobalUnlock(hg);
    if OpenClipboard(Some(hwnd)).is_ok() {
        let _ = EmptyClipboard();
        let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0)));
        let _ = CloseClipboard();
    }
}

/// Shows an open or save dialog for layout files and returns the chosen path.
unsafe fn layout_dialog(save: bool) -> Option<String> {
    let hwnd = main_hwnd();
    let filter: Vec<u16> = "Orbit Start layout (*.json)\0*.json\0\0".encode_utf16().collect();
    let mut file = [0u16; 520];
    if save {
        for (i, c) in "orbit-layout.json".encode_utf16().enumerate() {
            file[i] = c;
        }
    }
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrDefExt: w!("json"),
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST },
        ..Default::default()
    };
    with(|s| s.modal = true);
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) };
    with(|s| s.modal = false);
    force_foreground(hwnd);
    if !ok.as_bool() {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&file[..len]))
}

fn post_box<T>(msg: u32, value: T) {
    let ptr = Box::into_raw(Box::new(value));
    unsafe {
        if PostMessageW(Some(main_hwnd()), msg, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
}

/// Runs the work the UI queued up. Never called while the UI state is borrowed.
unsafe fn run_pending() {
    loop {
        let Some(list) = with(|s| std::mem::take(&mut s.app.pending)) else { return };
        if list.is_empty() {
            return;
        }
        for p in list {
            match p {
                Pending::Hide => hide_menu(),
                Pending::Launch { target, admin } => apps::shell_open(&target, if admin { "runas" } else { "open" }),
                Pending::ContextApp(id) => {
                    let pinned = with(|s| s.app.is_pinned(&id)).unwrap_or(false);
                    let pin_label = if pinned { "Unpin from Start" } else { "Pin to Start" };
                    let names = with(|s| s.app.workspace_names()).unwrap_or_default();
                    let labels: Vec<String> = names.iter().map(|n| format!("Add to {n}")).collect();
                    let current = with(|s| s.app.current_custom_group(&id)).flatten();
                    let remove_label = current.as_ref().map(|c| format!("Remove from {}", c.1));
                    let mut items: Vec<(usize, &str)> =
                        vec![(1, "Open"), (2, "Run as administrator"), (0, ""), (3, pin_label), (0, "")];
                    for (i, l) in labels.iter().enumerate() {
                        items.push((10 + i, l.as_str()));
                    }
                    if names.len() < 8 {
                        items.push((9, "Add to new group"));
                    }
                    if let Some(l) = &remove_label {
                        items.push((0, ""));
                        items.push((8, l.as_str()));
                    }
                    match popup(&items) {
                        8 => {
                            if let Some((ws, _)) = &current {
                                with(|s| s.app.remove_from_workspace(*ws, &id));
                            }
                        }
                        9 => {
                            with(|s| s.app.add_to_workspace(None, &id));
                        }
                        n if n >= 10 => {
                            with(|s| s.app.add_to_workspace(Some(n - 10), &id));
                        }
                        1 => {
                            apps::shell_open(&id, "open");
                            hide_menu();
                        }
                        2 => {
                            apps::shell_open(&id, "runas");
                            hide_menu();
                        }
                        3 => {
                            with(|s| s.app.toggle_pin(&id));
                        }
                        _ => {}
                    }
                }
                Pending::PowerMenu => {
                    let pick = popup(&[(1, "Lock"), (2, "Sleep"), (3, "Sign out"), (0, ""), (4, "Restart"), (5, "Shut down")]);
                    let cmd = match pick {
                        1 => Some(Cmd::Lock),
                        2 => Some(Cmd::Sleep),
                        3 => Some(Cmd::SignOut),
                        4 => Some(Cmd::Restart),
                        5 => Some(Cmd::Shutdown),
                        _ => None,
                    };
                    if let Some(cmd) = cmd {
                        hide_menu();
                        power(cmd);
                    }
                }
                Pending::Power(cmd) => power(cmd),
                Pending::ContextWorkspace(i) => match popup(&[
                    (1, "Open all"),
                    (3, "Choose apps"),
                    (4, "Rename"),
                    (0, ""),
                    (2, "Delete group"),
                ]) {
                    1 => {
                        with(|s| s.app.launch_workspace(i));
                    }
                    3 => {
                        with(|s| s.app.edit_group(i, false));
                    }
                    4 => {
                        with(|s| s.app.edit_group(i, true));
                    }
                    2 => {
                        with(|s| s.app.delete_workspace(i));
                    }
                    _ => {}
                },
                Pending::Activate(h) => wins::activate(h),
                Pending::Run { file, params } => apps::shell_run(&file, &params),
                Pending::TaskbarChanged => sync_taskbar_settings(),
                Pending::AddWidget(page) => {
                    let items: Vec<(usize, &str)> =
                        Widget::ALL.iter().enumerate().map(|(i, w)| (i + 1, w.label())).collect();
                    let pick = popup(&items);
                    if pick >= 1 && pick <= Widget::ALL.len() {
                        with(|s| s.app.add_widget(page, Widget::ALL[pick - 1]));
                    }
                }
                Pending::Copy(text) => set_clipboard_text(main_hwnd(), &text),
                Pending::Export => {
                    if let Some(path) = layout_dialog(true) {
                        if let Some(layout) = with(|s| s.app.cfg.layout()) {
                            if let Ok(json) = serde_json::to_string_pretty(&layout) {
                                let _ = std::fs::write(path, json);
                            }
                        }
                    }
                }
                Pending::Import => {
                    if let Some(path) = layout_dialog(false) {
                        let layout = std::fs::read_to_string(path)
                            .ok()
                            .filter(|s| s.contains("\"orbit_layout\""))
                            .and_then(|s| serde_json::from_str::<Layout>(&s).ok());
                        if let Some(layout) = layout {
                            with(|s| {
                                s.app.cfg.apply_layout(layout);
                                s.app.page = 0;
                                s.app.refresh_theme();
                                s.app.dirty = true;
                            });
                            relayout(false);
                        }
                    }
                }
                Pending::Relayout => relayout(false),
                Pending::HookChanged => {
                    if let Some(on) = with(|s| s.app.cfg.hook_win_key) {
                        HOOK_ON.store(on, Ordering::Relaxed);
                    }
                }
                Pending::FetchNews => {
                    if let Some(url) = with(|s| s.app.cfg.news_url.clone()) {
                        std::thread::spawn(move || {
                            unsafe {
                                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                            }
                            post_box(WM_NEWS_READY, news::fetch(&url));
                        });
                    }
                }
                Pending::RefreshFiles => {
                    std::thread::spawn(|| {
                        unsafe {
                            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                        }
                        post_box(WM_FILES_READY, apps::index_files());
                    });
                }
            }
        }
        invalidate();
    }
}

// ------------------------------------------------------------------- demo

/// Settings for a `--demo` run: a built-in layout plus optional overrides.
/// Nothing is read from or written to the user's settings file.
fn demo_config(value: &dyn Fn(&str) -> Option<String>) -> Config {
    config::DEMO.store(true, Ordering::Relaxed);
    let mut cfg = Config::default();
    if let Some(i) = value("--preset").and_then(|v| v.parse::<usize>().ok()) {
        if let Some((_, l)) = config::presets().into_iter().nth(i) {
            cfg.apply_layout(l);
        }
    }
    let parse = |k: &str| value(k).map(|v| format!("\"{v}\""));
    if let Some(s) = parse("--style").and_then(|v| serde_json::from_str(&v).ok()) {
        cfg.style = s;
    }
    if let Some(b) = parse("--backdrop").and_then(|v| serde_json::from_str(&v).ok()) {
        cfg.backdrop = b;
    }
    if let Some(t) = value("--theme") {
        cfg.theme = t;
    }
    if let Some(a) = value("--accent") {
        cfg.accent = a;
    }
    cfg.position = Position::Center;
    cfg.hook_win_key = false;
    cfg.taskbar_info = false;
    cfg.pinned_initialized = false;
    cfg.sanitize();
    cfg
}

unsafe extern "system" fn plain_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            let _ = ValidateRect(Some(hwnd), None);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

thread_local! {
    static WALLPAPER: RefCell<Option<Gfx>> = const { RefCell::new(None) };
}

/// Covers the screen with a neutral gradient so screenshots show nothing of
/// the real desktop (the menu's translucent backdrop blurs what is behind it).
unsafe fn demo_wallpaper(gfx: &Gfx, hinst: HINSTANCE) {
    let wc = WNDCLASSW {
        lpfnWndProc: Some(plain_proc),
        hInstance: hinst,
        lpszClassName: w!("OrbitStartDemoWallpaper"),
        ..Default::default()
    };
    RegisterClassW(&wc);
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut mi);
    let r = mi.rcMonitor;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP | WS_EX_NOACTIVATE,
        w!("OrbitStartDemoWallpaper"),
        w!("Orbit Start demo"),
        WS_POPUP,
        r.left,
        r.top,
        w,
        h,
        None,
        None,
        Some(hinst),
        None,
    ) else {
        return;
    };
    let Ok(mut g) = gfx.sibling(hwnd, w as u32, h as u32) else { return };
    // Four-corner gradient: deep blue, violet, rose, amber.
    let corners = [Color::rgb(0x1e2a6e), Color::rgb(0x6b3fa8), Color::rgb(0xc4527c), Color::rgb(0xf0a057)];
    let mix = |a: Color, b: Color, t: f32| Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    };
    g.begin();
    let cell = 16.0;
    let (fw, fh) = (w as f32, h as f32);
    let mut y = 0.0;
    while y < fh {
        let v = y / fh;
        let mut x = 0.0;
        while x < fw {
            let u = x / fw;
            let top = mix(corners[0], corners[1], u);
            let bottom = mix(corners[2], corners[3], u);
            g.fill_rect(gfx::Rect::new(x, y, cell + 1.0, cell + 1.0), mix(top, bottom, v));
            x += cell;
        }
        y += cell;
    }
    // A few soft glows.
    for (cx, cy, rad, c) in [
        (0.25, 0.35, 0.30, Color::rgb(0x8fb3ff)),
        (0.78, 0.25, 0.25, Color::rgb(0xff9ad5)),
        (0.60, 0.85, 0.35, Color::rgb(0xffd08a)),
    ] {
        for k in 0..12 {
            let f = 1.0 - k as f32 / 12.0;
            g.fill_circle(fw * cx, fh * cy, fw * rad * f, c.a(0.035));
        }
    }
    g.end();
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), r.left, r.top, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    WALLPAPER.with(|s| *s.borrow_mut() = Some(g));
}

// ----------------------------------------------------------------- island

fn draw_island(g: &mut Gfx, app: &App, h: f32) {
    use gfx::{ts, Rect};
    let t = app.theme;
    let st = app.stats();
    g.begin();
    let pill = |g: &mut Gfx, r: Rect| {
        g.fill_rr(r, 12.0, t.bg.a(0.93));
        g.stroke_rr(r, 12.0, t.card.a(0.12), 1.0);
    };
    let stats = Rect::new(0.0, 0.0, ISL_STATS_W, h);
    pill(g, stats);
    let rate = |v: Option<f32>| v.map(|v| format!("{}/s", sysinfo::fmt_rate(v))).unwrap_or_else(|| "–".into());
    let mut x = 14.0;
    for (arrow, col, val) in [("↓", Color::rgb(0xa78bfa), rate(st.net_down)), ("↑", Color::rgb(0x4fd1c5), rate(st.net_up))] {
        g.text(arrow, Rect::new(x, 0.0, 14.0, h), ts(14.0).bold().center(), col);
        g.text(&val, Rect::new(x + 16.0, 0.0, 54.0, h), ts(12.5).semi(), t.fg);
        x += 72.0;
    }
    g.fill_rect(Rect::new(x, h * 0.28, 1.0, h * 0.44), t.card.a(0.16));
    x += 10.0;
    for (label, val) in [("CPU", st.cpu), ("MEM", st.mem)] {
        g.text(label, Rect::new(x, 0.0, 30.0, h), ts(10.5).bold(), t.dim);
        g.text(&format!("{:.0}%", val * 100.0), Rect::new(x + 31.0, 0.0, 38.0, h), ts(12.5).semi(), t.fg);
        x += 68.0;
    }
    let fx = ISL_STATS_W + ISL_GAP;
    pill(g, Rect::new(fx, 0.0, ISL_BTN * ISL_FOLDERS.len() as f32 + 12.0, h));
    for (i, (glyph, _)) in ISL_FOLDERS.iter().enumerate() {
        g.text(glyph, Rect::new(fx + 6.0 + i as f32 * ISL_BTN, 0.0, ISL_BTN, h), ts(16.0).icon().center(), t.fg);
    }
    g.end();
}

/// Once a second: place the island in the taskbar's free space, refresh the
/// numbers, or hide it when it does not fit or something is full screen.
unsafe fn island_tick() {
    let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) else { return };
    let mut tb = RECT::default();
    let _ = GetWindowRect(tray, &mut tb);
    let k = GetDpiForWindow(tray).max(96) as f32 / 96.0;
    let busy = matches!(
        SHQueryUserNotificationState(),
        Ok(QUNS_BUSY) | Ok(QUNS_RUNNING_D3D_FULL_SCREEN) | Ok(QUNS_PRESENTATION_MODE)
    );
    let place = with(|s| {
        let on = s.app.cfg.taskbar_info;
        let Some((ih, ig)) = s.island.as_mut() else { return None };
        let m = (4.0 * k) as i32;
        let total = ISL_STATS_W + ISL_GAP + ISL_BTN * ISL_FOLDERS.len() as f32 + 12.0;
        let (w, h) = ((total * k).round() as i32, tb.bottom - tb.top - 2 * m);
        let x = tb.left + m + (6.0 * k) as i32;
        let fits = (x + w + (16.0 * k) as i32) < taskbar::leftmost() && h > 10 && IsWindowVisible(tray).as_bool();
        s.island_live = on && fits && !busy;
        if !s.island_live {
            return Some((*ih, None));
        }
        s.app.tick();
        ig.resize(w as u32, h as u32);
        ig.set_scale(k);
        draw_island(ig, &s.app, h as f32 / k);
        Some((*ih, Some((x, tb.top + m, w, h))))
    });
    match place.flatten() {
        Some((ih, Some((x, y, w, h)))) => {
            let _ = SetWindowPos(ih, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        Some((ih, None)) => {
            let _ = ShowWindow(ih, SW_HIDE);
        }
        None => {}
    }
}

unsafe extern "system" fn island_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => island_tick(),
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            let _ = ValidateRect(Some(hwnd), None);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_LBUTTONUP => {
            let k = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let x = mouse_pos(lp).0 / k;
            if x < ISL_STATS_W {
                apps::shell_open("taskmgr.exe", "open");
            } else {
                let i = ((x - ISL_STATS_W - ISL_GAP - 6.0) / ISL_BTN).floor();
                if i >= 0.0 && (i as usize) < ISL_FOLDERS.len() {
                    apps::shell_open(ISL_FOLDERS[i as usize].1, "open");
                }
            }
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

// ------------------------------------------------------------------- tray

unsafe fn make_icon() -> HICON {
    const N: usize = 32;
    let mut px = vec![0u8; N * N * 4];
    for (ty, tx) in [(3usize, 3usize), (3, 17), (17, 3), (17, 17)] {
        for y in 0..12 {
            for x in 0..12 {
                let corner = (x == 0 || x == 11) && (y == 0 || y == 11);
                if !corner {
                    let i = ((ty + y) * N + tx + x) * 4;
                    px[i..i + 4].copy_from_slice(&[0xff, 0x9c, 0x3b, 0xff]);
                }
            }
        }
    }
    let mask = vec![0u8; N * N / 8];
    CreateIcon(None, N as i32, N as i32, 1, 32, mask.as_ptr(), px.as_ptr()).unwrap_or_default()
}

unsafe fn tray(hwnd: HWND, add: bool) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        ..Default::default()
    };
    if add {
        nid.hIcon = make_icon();
        for (i, c) in "Orbit Start".encode_utf16().enumerate() {
            nid.szTip[i] = c;
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    } else {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

// ----------------------------------------------------------------- window

fn mouse_pos(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
        // Explorer restarted: put the tray icon back and re-read the taskbar.
        tray(hwnd, true);
        ping_taskbar();
        return LRESULT(0);
    }
    if msg != 0 && msg == SHELLHOOK.load(Ordering::Relaxed) {
        match wp.0 & 0x7fff {
            1 => {
                with(|s| s.app.win_created(lp.0));
            }
            2 => {
                with(|s| s.app.win_destroyed(lp.0));
            }
            _ => {}
        }
        return LRESULT(0);
    }
    match msg {
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE => ping_taskbar(),
        WM_TOGGLE => toggle(),
        WM_TRAY => match lp.0 as u32 {
            WM_LBUTTONUP => {
                // Clicking the tray icon first deactivates (and hides) the menu;
                // do not immediately reopen it.
                let just_hidden = with(|s| !s.visible && s.last_hide.elapsed().as_millis() < 350).unwrap_or(false);
                if !just_hidden {
                    toggle();
                }
            }
            WM_RBUTTONUP => match popup(&[(1, "Open"), (2, "Customize"), (0, ""), (3, "Quit Orbit Start")]) {
                1 => show_menu(),
                2 => {
                    show_menu();
                    with(|s| s.app.view = ui::View::Settings);
                    invalidate();
                }
                3 => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            },
            _ => {}
        },
        WM_APPS_READY => {
            let v = Box::from_raw(lp.0 as *mut Vec<apps::AppEntry>);
            with(|s| s.app.set_apps(*v));
            invalidate();
        }
        WM_FILES_READY => {
            let v = Box::from_raw(lp.0 as *mut Vec<apps::FileEntry>);
            with(|s| s.app.files = *v);
        }
        WM_ICON_READY => {
            let d = Box::from_raw(lp.0 as *mut apps::IconData);
            with(|s| s.app.set_icon(&mut s.gfx, *d));
            invalidate();
        }
        WM_NEWS_READY => {
            let v = Box::from_raw(lp.0 as *mut Vec<news::NewsItem>);
            with(|s| {
                s.app.news_loading = false;
                if !v.is_empty() {
                    s.app.news = *v;
                    s.app.news_at = Some(Instant::now());
                }
            });
            invalidate();
        }
        WM_TIMER if wp.0 == 2 => {
            // Open animation: ease the content up to its resting position.
            let done = with(|s| {
                s.anim = s.anim.saturating_sub(1);
                let f = s.anim as f32 / ANIM_STEPS as f32;
                s.gfx.set_offset(18.0 * f * f);
                s.anim == 0
            });
            if done != Some(false) {
                let _ = KillTimer(Some(hwnd), 2);
            }
        }
        WM_TIMER => {
            let fg = GetForegroundWindow();
            let close = with(|s| {
                if !s.visible {
                    return false;
                }
                if !s.island_live {
                    s.app.tick();
                }
                // Safety net for a missed "lost focus" notification.
                if fg == hwnd {
                    s.was_active = true;
                    s.away = 0;
                } else if s.was_active && !s.pin && !s.modal {
                    s.away += 1;
                }
                s.away >= 2
            });
            if close == Some(true) {
                hide_menu();
            } else {
                invalidate();
            }
        }
        WM_CLICKAWAY => {
            if with(|s| s.visible && !s.pin && !s.modal).unwrap_or(false) {
                hide_menu();
            }
        }
        WM_PAINT => {
            let _ = ValidateRect(Some(hwnd), None);
            let again = with(|s| {
                if s.visible {
                    s.app.draw(&mut s.gfx)
                }
                std::mem::take(&mut s.app.redraw)
            });
            if again == Some(true) {
                invalidate();
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEMOVE => {
            let (x, y) = mouse_pos(lp);
            let redraw = with(|s| {
                if !s.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    s.tracking = TrackMouseEvent(&mut tme).is_ok();
                }
                s.app.mouse_move(x / s.k, y / s.k)
            });
            if redraw == Some(true) {
                invalidate();
            }
        }
        WM_MOUSELEAVE => {
            let redraw = with(|s| {
                s.tracking = false;
                s.app.mouse_leave()
            });
            if redraw == Some(true) {
                invalidate();
            }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = mouse_pos(lp);
            SetCapture(hwnd);
            with(|s| s.app.mouse_down(x / s.k, y / s.k));
            invalidate();
        }
        WM_LBUTTONUP => {
            let (x, y) = mouse_pos(lp);
            let _ = ReleaseCapture();
            with(|s| s.app.mouse_up(x / s.k, y / s.k));
            run_pending();
            invalidate();
        }
        WM_RBUTTONUP => {
            let (x, y) = mouse_pos(lp);
            with(|s| s.app.right_click(x / s.k, y / s.k));
            run_pending();
            invalidate();
        }
        WM_MOUSEWHEEL => {
            let notches = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            if with(|s| s.app.wheel(notches)) == Some(true) {
                invalidate();
            }
        }
        WM_CHAR => {
            let unit = wp.0 as u16;
            let ch = if (0xD800..0xDC00).contains(&unit) {
                HIGH_SURROGATE.with(|h| h.set(unit));
                None
            } else if (0xDC00..0xE000).contains(&unit) {
                let hi = HIGH_SURROGATE.with(|h| h.replace(0));
                char::decode_utf16([hi, unit]).next().and_then(|r| r.ok())
            } else {
                char::from_u32(unit as u32)
            };
            if let Some(ch) = ch {
                if with(|s| s.app.on_char(ch)) == Some(true) {
                    invalidate();
                }
            }
        }
        WM_KEYDOWN => {
            let vk = wp.0 as u32;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if ctrl && vk == 0x56 {
                if let Some(text) = clipboard_text(hwnd) {
                    with(|s| s.app.paste(&text));
                }
            } else {
                with(|s| s.app.on_key(vk, ctrl));
            }
            run_pending();
            invalidate();
        }
        WM_ACTIVATE => {
            if (wp.0 & 0xffff) as u32 != WA_INACTIVE {
                with(|s| s.was_active = true);
            }
            if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                let auto_hide = with(|s| s.visible && !s.pin && !s.modal).unwrap_or(false);
                if auto_hide {
                    hide_menu();
                }
            }
        }
        WM_CLOSE => hide_menu(),
        WM_DESTROY => {
            with(|s| s.app.on_hide());
            // Give the taskbar its normal shape back before leaving.
            taskbar::restore();
            tray(hwnd, false);
            PostQuitMessage(0);
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

fn main() {
    log(&format!("starting {}", std::env::args().skip(1).collect::<Vec<_>>().join(" ")));
    // Early in sign-in the graphics or shell may not be ready yet; try again.
    for attempt in 1..=10 {
        match real_main() {
            Ok(()) => {
                log("exit");
                return;
            }
            Err(e) => {
                log(&format!("start attempt {attempt} failed: {e}"));
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        // Keep the sign-in registration in place. This runs before the
        // single-instance check on purpose: launching the program by hand
        // repairs the registration even while another copy is running.
        if !flag("--pin") && Config::load().autostart && !sysinfo::autostart_ok() {
            sysinfo::set_autostart(true);
            log("registered to start at sign-in");
        }

        // Single instance: a second launch just opens the running menu.
        let _mutex = CreateMutexW(None, true, w!("OrbitStartMenu.SingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(existing) = FindWindowW(CLASS, PCWSTR::null()) {
                let _ = PostMessageW(Some(existing), WM_TOGGLE, WPARAM(0), LPARAM(0));
            }
            return Ok(());
        }

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
            CLASS,
            w!("Orbit Start"),
            WS_POPUP,
            0,
            0,
            400,
            400,
            None,
            None,
            Some(hinst),
            None,
        )?;
        MAIN_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        let demo = flag("--demo");
        let cfg = if demo { demo_config(&value) } else { Config::load() };
        let software = cfg.renderer != "gpu";
        let mut gfx = Gfx::new(hwnd, 400, 400, software).or_else(|_| Gfx::new(hwnd, 400, 400, true))?;

        let (icon_tx, icon_rx) = std::sync::mpsc::channel();
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || apps::icon_worker(icon_rx, raw, WM_ICON_READY));
        std::thread::spawn(|| {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            post_box(WM_APPS_READY, apps::index_apps());
        });

        if demo {
            demo_wallpaper(&gfx, hinst);
        }
        let mut app = App::new(cfg, icon_tx);
        if demo {
            app.enable_demo();
        } else {
            app.avatar = sysinfo::avatar_path().and_then(|p| gfx.load_image(&p));
        }
        if let Some(p) = value("--page").and_then(|p| p.parse::<usize>().ok()) {
            app.page = p;
        }
        HOOK_ON.store(app.cfg.hook_win_key && !flag("--no-hook"), Ordering::Relaxed);

        SHELL.with(|s| {
            *s.borrow_mut() = Some(Shell {
                hwnd,
                gfx,
                app,
                visible: false,
                pin: flag("--pin"),
                modal: false,
                k: 1.0,
                last_hide: Instant::now(),
                tracking: false,
                was_active: false,
                away: 0,
                anim: 0,
                island: None,
                island_live: false,
            })
        });

        // Island window in the taskbar's free space (shares the menu's graphics device).
        let iwc = WNDCLASSW {
            lpfnWndProc: Some(island_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_HAND)?,
            lpszClassName: ISLAND_CLASS,
            ..Default::default()
        };
        RegisterClassW(&iwc);
        if let Ok(ih) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP | WS_EX_NOACTIVATE,
            ISLAND_CLASS,
            w!("Orbit Start island"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinst),
            None,
        ) {
            with(|s| {
                if let Ok(ig) = s.gfx.sibling(ih, 10, 10) {
                    s.island = Some((ih, ig));
                }
            });
            if !flag("--no-island") {
                SetTimer(Some(ih), 1, 1000, None);
            }
        }

        tray(hwnd, true);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), Some(hinst), 0).ok();
        let mhook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinst), 0).ok();

        // Window open/close notifications (for "recently closed").
        SHELLHOOK.store(RegisterWindowMessageW(w!("SHELLHOOK")), Ordering::Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        log("ready");
        let _ = RegisterShellHookWindow(hwnd);

        // Taskbar thread: Start button position and taskbar shape.
        NO_HOOKS.store(flag("--no-hook"), Ordering::Relaxed);
        let (tb_tx, tb_rx) = std::sync::mpsc::channel();
        if let Ok(mut slot) = TASKBAR_TX.lock() {
            *slot = Some(tb_tx);
        }
        sync_taskbar_settings();
        std::thread::spawn(move || taskbar::worker(tb_rx));

        if flag("--show") {
            show_menu();
            match value("--view").as_deref() {
                Some("apps") => {
                    with(|s| s.app.view = ui::View::AllApps);
                }
                Some("settings") => {
                    with(|s| s.app.view = ui::View::Settings);
                }
                _ => {}
            }
            if let Some(q) = value("--query") {
                with(|s| s.app.paste(&q));
            }
            invalidate();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if let Some(h) = hook {
            let _ = UnhookWindowsHookEx(h);
        }
        if let Some(h) = mhook {
            let _ = UnhookWindowsHookEx(h);
        }
        with(|s| {
            let _ = s.hwnd;
        });
    }
    Ok(())
}
