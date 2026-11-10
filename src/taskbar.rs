//! Taskbar integration without touching Explorer's code.
//!
//! A background thread reads the taskbar's button positions through UI
//! Automation. That gives us (a) where the Start button is, so a click on it
//! can open this menu, and (b) the extents of the app buttons and the tray, so
//! the taskbar window can be clipped into a rounded bar or separate islands.
//! Clipping uses SetWindowRgn and is undone by clearing the region.

use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::TaskbarStyle;

/// Screen rectangles of the Start button (physical pixels).
pub static START_RECTS: Mutex<Vec<RECT>> = Mutex::new(Vec::new());
static STYLE: AtomicU8 = AtomicU8::new(0);
/// Screen x of the leftmost taskbar button; free space lies to its left.
static LEFTMOST: AtomicI32 = AtomicI32::new(i32::MAX);

pub fn leftmost() -> i32 {
    LEFTMOST.load(Ordering::Relaxed)
}
/// Window property set on the taskbar while we have it clipped.
const MARK: PCWSTR = w!("OrbitStartShaped");

pub fn set_style(s: TaskbarStyle) {
    STYLE.store(
        match s {
            TaskbarStyle::Default => 0,
            TaskbarStyle::Rounded => 1,
            TaskbarStyle::Islands => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn point_on_start(pt: POINT) -> bool {
    match START_RECTS.try_lock() {
        Ok(v) => v.iter().any(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom),
        Err(_) => false,
    }
}

unsafe fn tray_window() -> Option<HWND> {
    FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok().filter(|h| !h.is_invalid())
}

/// Removes any clipping we applied to the taskbar.
pub fn restore() {
    unsafe {
        if let Some(tray) = tray_window() {
            SetWindowRgn(tray, None, true);
            let _ = RemovePropW(tray, MARK);
        }
    }
}

/// Start button plus every app button, in screen coordinates.
unsafe fn buttons(uia: &IUIAutomation, tray: HWND) -> (Option<RECT>, Vec<RECT>) {
    let mut start = None;
    let mut out = Vec::new();
    let Ok(root) = uia.ElementFromHandle(tray) else { return (start, out) };
    let Ok(cond) = uia.CreateTrueCondition() else { return (start, out) };
    let Ok(all) = root.FindAll(TreeScope_Descendants, &cond) else { return (start, out) };
    for i in 0..all.Length().unwrap_or(0) {
        let Ok(el) = all.GetElement(i) else { continue };
        let id = el.CurrentAutomationId().map(|b| b.to_string()).unwrap_or_default();
        let is_start = id == "StartButton";
        if is_start || id == "SearchButton" || id == "TaskViewButton" || id == "WidgetsButton" || id.starts_with("Appid:") {
            if let Ok(r) = el.CurrentBoundingRectangle() {
                if r.right > r.left {
                    if is_start {
                        start = Some(r);
                    }
                    out.push(r);
                }
            }
        }
    }
    (start, out)
}

/// Rounded rectangles (in taskbar-window coordinates) that stay visible.
unsafe fn shape(style: u8, tray: HWND, wr: RECT, buttons: &[RECT]) -> Vec<[i32; 4]> {
    let dpi = GetDpiForWindow(tray).max(96) as f32 / 96.0;
    let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
    let m = (4.0 * dpi) as i32;
    let pad = (8.0 * dpi) as i32;
    let whole = vec![[m, m, w - m, h - m]];
    match style {
        1 => whole,
        2 => {
            if buttons.is_empty() {
                return whole;
            }
            let mut sorted = buttons.to_vec();
            sorted.sort_by_key(|r| r.left);
            let gap = (60.0 * dpi) as i32;
            let mut out: Vec<[i32; 4]> = Vec::new();
            let (mut l, mut r) = (sorted[0].left, sorted[0].right);
            for b in &sorted[1..] {
                if b.left - r > gap {
                    out.push([l - pad - wr.left, m, r + pad - wr.left, h - m]);
                    l = b.left;
                }
                r = r.max(b.right);
            }
            out.push([l - pad - wr.left, m, r + pad - wr.left, h - m]);
            if let Ok(notify) = FindWindowExW(Some(tray), None, w!("TrayNotifyWnd"), PCWSTR::null()) {
                let mut nr = RECT::default();
                if GetWindowRect(notify, &mut nr).is_ok() && nr.right > nr.left {
                    out.push([nr.left - pad - wr.left, m, w - m, h - m]);
                }
            }
            for r in &mut out {
                r[0] = r[0].max(m);
                r[2] = r[2].min(w - m);
            }
            out
        }
        _ => Vec::new(),
    }
}

unsafe fn apply(tray: HWND, rects: &[[i32; 4]]) {
    if rects.is_empty() {
        SetWindowRgn(tray, None, true);
        let _ = RemovePropW(tray, MARK);
        return;
    }
    // Leave a marker so a later run can undo the shape even if this one is killed.
    let _ = SetPropW(tray, MARK, Some(HANDLE(1 as *mut _)));
    let dpi = GetDpiForWindow(tray).max(96) as f32 / 96.0;
    let d = (24.0 * dpi) as i32;
    let rgn = CreateRectRgn(0, 0, 0, 0);
    for r in rects {
        let part = CreateRoundRectRgn(r[0], r[1], r[2] + 1, r[3] + 1, d, d);
        CombineRgn(Some(rgn), Some(rgn), Some(part), RGN_OR);
        let _ = DeleteObject(part.into());
    }
    // The system owns the region after a successful call.
    if SetWindowRgn(tray, Some(rgn), true) == 0 {
        let _ = DeleteObject(rgn.into());
    }
}

unsafe fn has_region(tray: HWND) -> bool {
    let probe = CreateRectRgn(0, 0, 0, 0);
    let kind = GetWindowRgn(tray, probe);
    let _ = DeleteObject(probe.into());
    kind != RGN_ERROR
}

/// Runs until the sender is dropped. Any message on `rx` means "look again now".
pub fn worker(rx: Receiver<()>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let uia: Option<IUIAutomation> = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
        // (taskbar window, shape) most recently applied.
        let mut applied: Option<(isize, Vec<[i32; 4]>)> = None;
        loop {
            let style = STYLE.load(Ordering::Relaxed);
            if let (Some(uia), Some(tray)) = (&uia, tray_window()) {
                let mut wr = RECT::default();
                let _ = GetWindowRect(tray, &mut wr);
                let (start, btns) = buttons(uia, tray);
                if let Ok(mut s) = START_RECTS.lock() {
                    *s = start.into_iter().collect();
                }
                LEFTMOST.store(btns.iter().map(|r| r.left).min().unwrap_or(i32::MAX), Ordering::Relaxed);
                let rects = shape(style, tray, wr, &btns);
                let key = (tray.0 as isize, rects.clone());
                let lost = !rects.is_empty() && !has_region(tray);
                let was_shaped = applied.as_ref().map(|a| !a.1.is_empty()).unwrap_or(false)
                    || !GetPropW(tray, MARK).is_invalid();
                if applied.as_ref() != Some(&key) || lost {
                    if !rects.is_empty() || was_shaped {
                        apply(tray, &rects);
                    }
                    applied = Some(key);
                }
            }
            let wait = if style == 2 { Duration::from_millis(1500) } else { Duration::from_secs(20) };
            if let Err(RecvTimeoutError::Disconnected) = rx.recv_timeout(wait) {
                break;
            }
        }
    }
}
