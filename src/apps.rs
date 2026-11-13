//! Application and file indexing, icon extraction, launching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL};

#[derive(Clone)]
pub struct AppEntry {
    pub name: String,
    pub lname: String,
    /// Shell parsing name, e.g. `shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`.
    pub id: String,
    /// Publisher from the app's uninstall record, when one matches by name.
    pub publisher: String,
}

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub lname: String,
    pub path: String,
}

pub struct IconReq {
    pub key: String,
    pub px: i32,
}

pub struct IconData {
    pub key: String,
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

unsafe fn take_pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Enumerates everything in the shell "Applications" folder: classic Start
/// menu shortcuts and packaged (Store) apps alike.
pub fn index_apps() -> Vec<AppEntry> {
    let mut out = Vec::new();
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else {
            return out;
        };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return out;
        };
        loop {
            let mut items = [None];
            let mut fetched = 0u32;
            if en.Next(&mut items, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = items[0].take() else { break };
            let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY) else { continue };
            let name = take_pwstr(name);
            let Ok(rel) = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING) else { continue };
            let rel = take_pwstr(rel);
            if name.is_empty() || rel.is_empty() {
                continue;
            }
            let lname = name.to_lowercase();
            let lrel = rel.to_lowercase();
            let junk = lname.starts_with("uninstall")
                || lname.contains("readme")
                || lname.ends_with(" website")
                || lname.ends_with(" help")
                || [".url", ".chm", ".txt", ".htm", ".html", ".pdf"].iter().any(|e| lrel.ends_with(e))
                || lrel.starts_with("http");
            if junk {
                continue;
            }
            out.push(AppEntry { name, lname, id: format!("shell:AppsFolder\\{rel}"), publisher: String::new() });
        }
    }
    out.sort_by(|a, b| a.lname.cmp(&b.lname));
    out.dedup_by(|a, b| a.id == b.id);
    let pubs = publishers();
    for a in &mut out {
        // Uninstall names often add a version: "Foo 2.1" belongs to app "Foo".
        if let Some((_, p)) = pubs.iter().find(|(n, _)| *n == a.lname || n.starts_with(&format!("{} ", a.lname))) {
            a.publisher = p.clone();
        }
    }
    out
}

/// (lowercase display name, publisher) of installed programs, from the
/// uninstall records Windows keeps for Settings > Installed apps.
fn publishers() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let roots = [
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ];
    let read = |key: HKEY, name: &str| -> Option<String> {
        let mut buf = [0u16; 260];
        let mut size = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(key, None, &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size))
        };
        if r != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..len]).trim().to_string()).filter(|s| !s.is_empty())
    };
    for (root, path) in roots {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(root, &HSTRING::from(path), None, KEY_READ, &mut key) != ERROR_SUCCESS {
                continue;
            }
            for i in 0..4000u32 {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                let r = RegEnumKeyExW(key, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None);
                if r != ERROR_SUCCESS {
                    break;
                }
                let mut sub = HKEY::default();
                if RegOpenKeyExW(key, PCWSTR(name.as_ptr()), None, KEY_READ, &mut sub) == ERROR_SUCCESS {
                    if let (Some(n), Some(p)) = (read(sub, "DisplayName"), read(sub, "Publisher")) {
                        out.push((n.to_lowercase(), p));
                    }
                    let _ = RegCloseKey(sub);
                }
            }
            let _ = RegCloseKey(key);
        }
    }
    out
}

/// Short group name for a publisher: "Learning Machines LLC" -> "Learning Machines".
pub fn short_publisher(p: &str) -> String {
    let mut s = p.trim().trim_end_matches('.').to_string();
    for suffix in [", Inc", " Inc", " LLC", " Ltd", " Limited", " Corporation", " Corp", " GmbH", " Co", ","] {
        if s.to_lowercase().ends_with(&suffix.to_lowercase()) {
            s.truncate(s.len() - suffix.len());
            s = s.trim().trim_end_matches(',').to_string();
        }
    }
    if s.is_empty() {
        p.to_string()
    } else {
        s
    }
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        Some(PathBuf::from(take_pwstr(p)))
    }
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<FileEntry>) {
    if out.len() >= 30000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.starts_with("~$") || name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            let l = name.to_ascii_lowercase();
            if matches!(l.as_str(), "node_modules" | "target" | "appdata" | "__pycache__" | "venv" | "$recycle.bin") {
                continue;
            }
            if depth > 0 && !ft.is_symlink() {
                walk(&e.path(), depth - 1, out);
            }
        }
        out.push(FileEntry { lname: name.to_lowercase(), name, path: e.path().to_string_lossy().into_owned() });
        if out.len() >= 30000 {
            return;
        }
    }
}

/// Names of files and folders in the user's main folders (names only; file
/// contents are never read).
pub fn index_files() -> Vec<FileEntry> {
    let mut out = Vec::new();
    for id in [
        &FOLDERID_Desktop,
        &FOLDERID_Documents,
        &FOLDERID_Downloads,
        &FOLDERID_Pictures,
        &FOLDERID_Music,
        &FOLDERID_Videos,
    ] {
        if let Some(p) = known_folder(id) {
            walk(&p, 4, &mut out);
        }
    }
    out
}

pub fn recent_files() -> Vec<FileEntry> {
    let Some(dir) = known_folder(&FOLDERID_Recent) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false))
        // Skip protocol launches (ms-settings:, ms-screenclip:, ...) that Windows also records here.
        .filter(|e| !e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("ms-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter()
        .take(8)
        .map(|(_, p)| {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            FileEntry { lname: name.to_lowercase(), name, path: p.to_string_lossy().into_owned() }
        })
        .collect()
}

/// Extracts the shell icon for a parsing name as premultiplied BGRA pixels.
unsafe fn icon_pixels(key: &str, px: i32) -> Option<IconData> {
    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(key), None).ok()?;
    let fac: IShellItemImageFactory = item.cast().ok()?;
    // Icon only: never asks the shell for thumbnails, so cloud files stay untouched.
    let hbmp = fac.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY).ok()?;
    let mut bm = BITMAP::default();
    GetObjectW(hbmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let mut result = None;
    if w > 0 && h > 0 && w <= 512 && h <= 512 {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let hdc = GetDC(None);
        let lines = GetDIBits(hdc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        if lines > 0 {
            // Icons without an alpha channel come back fully transparent.
            if buf.chunks_exact(4).all(|p| p[3] == 0) {
                for p in buf.chunks_exact_mut(4) {
                    p[3] = 255;
                }
            }
            result = Some(IconData { key: key.to_string(), w: w as u32, h: h as u32, bgra: buf });
        }
    }
    let _ = DeleteObject(hbmp.into());
    result
}

/// Background thread: turns icon requests into pixel buffers and posts them
/// back to the window so the UI thread never blocks on the shell.
pub fn icon_worker(rx: Receiver<IconReq>, hwnd: isize, msg: u32) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        while let Ok(req) = rx.recv() {
            let data = icon_pixels(&req.key, req.px)
                .unwrap_or(IconData { key: req.key.clone(), w: 0, h: 0, bgra: Vec::new() });
            let ptr = Box::into_raw(Box::new(data));
            if PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(ptr as isize)).is_err() {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

/// Runs a typed command line: the first word is the program, the rest its arguments.
pub fn shell_run(file: &str, params: &str) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(file), &HSTRING::from(params), None, SW_SHOWNORMAL);
    }
}

/// Most recently changed items in a known folder (names only).
pub fn folder_recent(downloads: bool, n: usize) -> Vec<FileEntry> {
    let id = if downloads { &FOLDERID_Downloads } else { &FOLDERID_Documents };
    let Some(dir) = known_folder(id) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            !n.starts_with('.') && n != "desktop.ini" && !n.ends_with(".crdownload") && !n.ends_with(".tmp")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter()
        .take(n)
        .map(|(_, p)| {
            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            FileEntry { lname: name.to_lowercase(), name, path: p.to_string_lossy().into_owned() }
        })
        .collect()
}

fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

/// How much each app has been used, from the usage counts Windows itself
/// keeps for the Start menu (run count, time in focus, last run). Keys are
/// lowercase app ids as they appear after `shell:AppsFolder\\`.
pub fn usage() -> HashMap<String, f32> {
    let mut out = HashMap::new();
    unsafe {
        let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist\\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\\Count");
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key, None, KEY_READ, &mut hkey) != ERROR_SUCCESS {
            return out;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 1024];
            let mut name_len = name.len() as u32;
            let mut data = [0u8; 128];
            let mut data_len = data.len() as u32;
            let r = RegEnumValueW(
                hkey,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            );
            i += 1;
            if r == ERROR_NO_MORE_ITEMS || i > 5000 {
                break;
            }
            if r != ERROR_SUCCESS || data_len < 68 {
                continue;
            }
            let u32_at = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
            let runs = u32_at(4) as f32;
            let focus_min = u32_at(12) as f32 / 60000.0;
            let last = (u32_at(60) as u64) | ((u32_at(64) as u64) << 32);
            let last_unix = (last / 10_000_000).saturating_sub(11_644_473_600);
            let days = now.saturating_sub(last_unix) as f32 / 86400.0;
            let mut score = runs + focus_min / 6.0;
            if last != 0 && days < 7.0 {
                score *= 1.6;
            } else if last != 0 && days > 60.0 {
                score *= 0.4;
            }
            if score > 0.0 {
                let id = rot13(&String::from_utf16_lossy(&name[..name_len as usize])).to_lowercase();
                out.insert(id, score);
            }
        }
        let _ = RegCloseKey(hkey);
    }
    out
}

/// Sorts an app into a broad category by its name, or None.
pub fn category(lname: &str) -> Option<&'static str> {
    const TABLE: [(&str, &[&str]); 5] = [
        (
            "Productivity",
            &[
                "word", "excel", "powerpoint", "outlook", "onenote", "access", "visio", "publisher", "notepad", "teams",
                "notion", "acrobat", "pdf", "office", "calendar", "mail", "to do", "sticky notes", "project", "onedrive",
                "copilot",
            ],
        ),
        (
            "Development",
            &[
                "visual studio", "code", "terminal", "powershell", "git", "docker", "python", "node.js", "android studio",
                "postman", "command prompt", "ubuntu", "wsl", "claude", "codex", "chatgpt", "cursor",
                "developer", "sql", "github",
            ],
        ),
        (
            "Internet",
            &[
                "edge", "chrome", "firefox", "opera", "brave", "telegram", "whatsapp", "discord", "zoom", "skype", "vpn",
                "linkedin", "messenger", "signal", "phone link",
            ],
        ),
        (
            "Media",
            &[
                "spotify", "vlc", "photos", "media player", "paint", "camera", "clipchamp", "obs", "snipping tool",
                "photoshop", "movies", "music", "sound recorder", "video",
            ],
        ),
        ("Games", &["steam", "epic games", "xbox", "game", "solitaire", "riot", "battle.net", "minecraft"]),
    ];
    for (cat, words) in TABLE {
        for w in words {
            let hit = if w.contains(' ') || w.contains('.') {
                lname.contains(w)
            } else {
                lname.split(|c: char| !c.is_alphanumeric()).any(|p| p == *w)
            };
            if hit {
                return Some(cat);
            }
        }
    }
    None
}

pub fn shell_open(target: &str, verb: &str) {
    unsafe {
        ShellExecuteW(None, &HSTRING::from(verb), &HSTRING::from(target), None, None, SW_SHOWNORMAL);
    }
}

/// Score a name against a lowercase query. Higher is better; None is no match.
pub fn score(lname: &str, q: &str) -> Option<i32> {
    if q.is_empty() {
        return None;
    }
    if lname == q {
        return Some(1000);
    }
    if lname.starts_with(q) {
        return Some(900 - lname.len() as i32);
    }
    if lname.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        return Some(700 - lname.len() as i32);
    }
    // Initials: "vsc" matches "Visual Studio Code".
    let initials: String = lname.split_whitespace().filter_map(|w| w.chars().next()).collect();
    if q.chars().count() > 1 && initials.starts_with(q) {
        return Some(650 - lname.len() as i32);
    }
    if lname.contains(q) {
        return Some(500 - lname.len() as i32);
    }
    // Loose subsequence match: must begin at the start of a word and stay tight.
    if q.chars().count() < 3 {
        return None;
    }
    let chars: Vec<char> = lname.chars().collect();
    let first = q.chars().next()?;
    for start in 0..chars.len() {
        if chars[start] != first || (start > 0 && chars[start - 1].is_alphanumeric()) {
            continue;
        }
        let mut pos = start + 1;
        let mut gaps = 0;
        let mut ok = true;
        for qc in q.chars().skip(1) {
            let mut run = 0;
            while pos < chars.len() && chars[pos] != qc {
                pos += 1;
                run += 1;
            }
            if pos >= chars.len() || run > 2 {
                ok = false;
                break;
            }
            gaps += run;
            pos += 1;
        }
        if ok {
            return Some(200 - gaps);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring() {
        assert!(score("visual studio code", "vis") > score("microsoft visio", "vis"));
        assert!(score("visual studio code", "vsc").is_some());
        assert!(score("notepad", "ntpd").is_some());
        assert!(score("notepad", "xyz").is_none());
        assert_eq!(rot13("Abgrcnq.rkr"), "Notepad.exe");
        assert_eq!(category("visual studio code"), Some("Development"));
        assert_eq!(category("microsoft edge"), Some("Internet"));
        assert_eq!(category("barcode reader"), None);
        assert_eq!(short_publisher("Learning Machines LLC"), "Learning Machines");
        assert_eq!(short_publisher("Microsoft Corporation"), "Microsoft");
        assert_eq!(short_publisher("Foo, Inc."), "Foo");
        assert!(score("services", "vis").is_none());
        assert!(score("voice access", "vis").is_none());
        assert!(score("calculator", "calc") > score("local calc tool", "calc"));
    }
}
