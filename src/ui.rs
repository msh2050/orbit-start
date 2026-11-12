//! Menu state, layout, drawing and input handling. Immediate mode: every
//! frame draws the whole menu and records the clickable regions it produced.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::time::Instant;

use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::System::Time::DYNAMIC_TIME_ZONE_INFORMATION;

use crate::apps::{self, AppEntry, FileEntry, IconData, IconReq};
use crate::calc;
use crate::config::*;
use crate::gfx::*;
use crate::news::{self, NewsItem};
use crate::sysinfo::{self, Sampler, Stats};
use crate::text::TextField;
use crate::wins::{self, Closed, Win};

const PAD: f32 = 28.0;
const RIGHT_W: f32 = 272.0;
const HEADER_H: f32 = 72.0;
const SIDE_W: f32 = 236.0;
/// Width of the widget column in the Apps style.
const APPS_LEFT: f32 = 452.0;

/// Windows Settings pages that search can jump to.
const SETTINGS_PAGES: [(&str, &str); 30] = [
    ("Display", "ms-settings:display"),
    ("Night light", "ms-settings:nightlight"),
    ("Sound", "ms-settings:sound"),
    ("Notifications", "ms-settings:notifications"),
    ("Power and battery", "ms-settings:powersleep"),
    ("Storage", "ms-settings:storagesense"),
    ("Bluetooth and devices", "ms-settings:bluetooth"),
    ("Printers and scanners", "ms-settings:printers"),
    ("Mouse", "ms-settings:mousetouchpad"),
    ("Network and internet", "ms-settings:network"),
    ("Wi-Fi", "ms-settings:network-wifi"),
    ("VPN", "ms-settings:network-vpn"),
    ("Proxy", "ms-settings:network-proxy"),
    ("Personalization", "ms-settings:personalization"),
    ("Background and wallpaper", "ms-settings:personalization-background"),
    ("Colors", "ms-settings:colors"),
    ("Themes", "ms-settings:themes"),
    ("Lock screen", "ms-settings:lockscreen"),
    ("Taskbar settings", "ms-settings:taskbar"),
    ("Installed apps", "ms-settings:appsfeatures"),
    ("Default apps", "ms-settings:defaultapps"),
    ("Startup apps", "ms-settings:startupapps"),
    ("Your account", "ms-settings:yourinfo"),
    ("Sign-in options", "ms-settings:signinoptions"),
    ("Date and time", "ms-settings:dateandtime"),
    ("Language and region", "ms-settings:regionlanguage"),
    ("Keyboard and typing", "ms-settings:typing"),
    ("Windows Update", "ms-settings:windowsupdate"),
    ("Privacy and security", "ms-settings:privacy"),
    ("About this PC", "ms-settings:about"),
];

/// Apps shown in demo mode (common Windows apps only, never this PC's own list).
const DEMO_APPS: [&str; 34] = [
    "Calculator", "Camera", "Clock", "File Explorer", "Microsoft Edge", "Notepad", "Paint", "Photos", "Settings",
    "Snipping Tool", "Terminal", "Task Manager", "Media Player", "Microsoft Store", "Sticky Notes", "Control Panel",
    "Command Prompt", "Windows PowerShell", "Word", "Excel", "PowerPoint", "Outlook", "OneNote", "Visual Studio Code",
    "Firefox", "Google Chrome", "Microsoft Teams", "Weather", "Sound Recorder", "Character Map", "Registry Editor",
    "Windows Security", "Remote Desktop Connection", "Microsoft To Do",
];
const DEMO_USAGE: [&str; 8] =
    ["Microsoft Edge", "Visual Studio Code", "Terminal", "File Explorer", "Outlook", "Word", "Photos", "Snipping Tool"];

/// UTF-16 range of the query inside a title, for highlighting.
fn hl_of(title: &str, q: &str) -> (u32, u32) {
    let l = title.to_lowercase();
    match l.find(q) {
        Some(i) => (l[..i].encode_utf16().count() as u32, q.encode_utf16().count() as u32),
        None => (0, 0),
    }
}

/// A tab in the app groups widget.
#[derive(Clone)]
struct Group {
    name: String,
    apps: Vec<String>,
    /// Index into the user's workspaces when this tab is one of them.
    ws: Option<usize>,
}
const GAP: f32 = 22.0;

const G_SEARCH: &str = "\u{E721}";
const G_DOC: &str = "\u{E8A5}";
const G_DOWNLOAD: &str = "\u{E896}";
const G_MUSIC: &str = "\u{E8D6}";
const G_PICTURE: &str = "\u{EB9F}";
const G_USER: &str = "\u{E77B}";
const G_CONTROL: &str = "\u{E9E9}";
const G_SETTINGS: &str = "\u{E713}";
const G_RUN: &str = "\u{E756}";
const G_APPS: &str = "\u{E71D}";
const G_RIGHT: &str = "\u{E76C}";
const G_LEFT: &str = "\u{E76B}";
const G_POWER: &str = "\u{E7E8}";
const G_BACK: &str = "\u{E72B}";
const G_CLOSE: &str = "\u{E711}";
const G_ADD: &str = "\u{E710}";
const G_GLOBE: &str = "\u{E774}";
const G_CALC: &str = "\u{E8EF}";
const G_LOCK: &str = "\u{E72E}";
const G_UP: &str = "\u{E74A}";
const G_DOWN: &str = "\u{E74B}";
const G_CUSTOMIZE: &str = "\u{E771}";
const G_FOLDER: &str = "\u{E8B7}";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    Lock,
    Sleep,
    SignOut,
    Restart,
    Shutdown,
    Customize,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tog {
    WinKey,
    Autostart,
    Seconds,
    H24,
    StartButton,
    TaskbarInfo,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SliderKind {
    Tint,
    Scale,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Act {
    FocusSearch,
    ClearSearch,
    App(String),
    Open(String),
    Web(String),
    Copy(String),
    Cmd(Cmd),
    PagePrev,
    PageNext,
    PageGo(usize),
    AllApps,
    Home,
    PowerMenu,
    Customize,
    FocusNotes,
    FocusCalc,
    SetStyle(Style),
    SetPos(Position),
    SetBackdrop(Backdrop),
    SetTheme(String),
    SetAccent(String),
    SetIcon(u32),
    Toggle(Tog),
    Slider(SliderKind, Rect),
    WidgetUp(usize, usize),
    WidgetDown(usize, usize),
    WidgetDel(usize, usize),
    WidgetAdd(usize),
    PageAdd,
    PageDel(usize),
    Export,
    Import,
    Reset,
    OpenConfig,
    Workspace(usize),
    Window(isize),
    SetTaskbar(TaskbarStyle),
    Preset(usize),
    RunCmd(String),
    GroupTab(usize),
    FocusRun,
    GroupNew,
    GroupEdit(usize),
    GroupRename(usize),
    GroupDelete(usize),
    PickToggle(String),
    FocusGroupName,
    EditClocks,
    ClockAdd(usize),
    ClockDel(usize),
    ClockUp(usize),
    ClockDown(usize),
}

/// Work that must run outside the UI state borrow (window changes, menus,
/// dialogs, launching programs).
pub enum Pending {
    Hide,
    Launch { target: String, admin: bool },
    ContextApp(String),
    PowerMenu,
    Power(Cmd),
    AddWidget(usize),
    Copy(String),
    Export,
    Import,
    Relayout,
    HookChanged,
    FetchNews,
    RefreshFiles,
    ContextWorkspace(usize),
    Activate(isize),
    TaskbarChanged,
    Run { file: String, params: String },
}

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Home,
    AllApps,
    Settings,
    /// Choosing which apps belong to the custom group with this index.
    PickApps(usize),
    /// Choosing which cities the world clocks show.
    Clocks,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Search,
    Notes,
    Calc,
    Run,
    GroupName,
}

struct Hit {
    rect: Rect,
    act: Act,
}

#[derive(Clone)]
struct ResultItem {
    title: String,
    sub: String,
    icon: Option<String>,
    glyph: &'static str,
    act: Act,
    section: &'static str,
    /// Highlighted part of the title (UTF-16 start, length).
    hl: (u32, u32),
}

pub struct App {
    pub cfg: Config,
    pub theme: Theme,
    pub dirty: bool,
    pub apps: Vec<AppEntry>,
    pub files: Vec<FileEntry>,
    pub files_at: Option<Instant>,
    recents: Vec<FileEntry>,
    icons: HashMap<String, Option<ID2D1Bitmap1>>,
    icon_pending: HashSet<String>,
    icon_tx: Sender<IconReq>,
    pub icon_px: i32,
    pub view: View,
    pub page: usize,
    scroll: f32,
    max_scroll: f32,
    pub search: TextField,
    results: Vec<ResultItem>,
    sel: usize,
    pub focus: Focus,
    notes: TextField,
    calc: TextField,
    mouse: (f32, f32),
    hits: Vec<Hit>,
    hot: Option<usize>,
    down: Option<Act>,
    clip: Option<Rect>,
    drag: Option<(SliderKind, Rect)>,
    stats: Stats,
    sampler: Sampler,
    pub news: Vec<NewsItem>,
    pub news_at: Option<Instant>,
    pub news_loading: bool,
    user: String,
    pub avatar: Option<ID2D1Bitmap1>,
    /// Screenshot mode: made-up name, files, drives and windows; nothing from this PC.
    pub demo: bool,
    pub pending: Vec<Pending>,
    autostart: bool,
    tz: HashMap<String, Option<DYNAMIC_TIME_ZONE_INFORMATION>>,
    run: TextField,
    gname: TextField,
    /// The group was just created: the first typed character replaces its default name.
    gname_fresh: bool,
    hist_idx: Option<usize>,
    groups: Vec<Group>,
    group_tab: usize,
    /// Usage score per app, from Windows' own Start menu statistics.
    usage: HashMap<String, f32>,
    usage_at: Option<Instant>,
    drives: Vec<sysinfo::Drive>,
    downloads: Vec<FileEntry>,
    /// Scroll position of the app list pane in the Apps style.
    scroll2: f32,
    max_scroll2: f32,
    /// Vertical offset of each search result, for keyboard scrolling.
    result_y: Vec<f32>,
    wins: Vec<Win>,
    closed: Vec<Closed>,
    /// Every app window we know about, by handle, with its executable.
    seen: HashMap<isize, String>,
    /// Heights of dashboard cards from the previous frame.
    card_h: HashMap<(usize, usize, u8), f32>,
    /// Set when a frame discovered it needs to be drawn again.
    pub redraw: bool,
    pub w: f32,
    pub h: f32,
}

pub fn base_size(style: Style) -> (f32, f32) {
    match style {
        Style::TwoPane => (830.0, 708.0),
        Style::Compact => (560.0, 790.0),
        Style::Dashboard => (1010.0, 724.0),
        Style::Apps => (900.0, 740.0),
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl App {
    pub fn new(cfg: Config, icon_tx: Sender<IconReq>) -> App {
        let theme = Theme::from(&cfg, sysinfo::system_accent());
        let (w, h) = base_size(cfg.style);
        let notes = TextField::new(&cfg.notes, true);
        App {
            cfg,
            theme,
            dirty: false,
            apps: Vec::new(),
            files: Vec::new(),
            files_at: None,
            recents: Vec::new(),
            icons: HashMap::new(),
            icon_pending: HashSet::new(),
            icon_tx,
            icon_px: 64,
            view: View::Home,
            page: 0,
            scroll: 0.0,
            max_scroll: 0.0,
            search: TextField::new("", false),
            results: Vec::new(),
            sel: 0,
            focus: Focus::Search,
            notes,
            calc: TextField::new("", false),
            mouse: (-1.0, -1.0),
            hits: Vec::new(),
            hot: None,
            down: None,
            clip: None,
            drag: None,
            stats: Stats::default(),
            sampler: Sampler::new(),
            news: Vec::new(),
            news_at: None,
            news_loading: false,
            user: sysinfo::user_name(),
            demo: false,
            avatar: None,
            pending: Vec::new(),
            autostart: sysinfo::autostart_enabled(),
            tz: HashMap::new(),
            run: TextField::new("", false),
            gname: TextField::new("", false),
            gname_fresh: false,
            hist_idx: None,
            groups: Vec::new(),
            group_tab: 0,
            usage: HashMap::new(),
            usage_at: None,
            drives: Vec::new(),
            downloads: Vec::new(),
            scroll2: 0.0,
            max_scroll2: 0.0,
            result_y: Vec::new(),
            wins: Vec::new(),
            closed: Vec::new(),
            seen: wins::enumerate().into_iter().map(|w| (w.hwnd, w.exe)).collect(),
            card_h: HashMap::new(),
            redraw: false,
            w,
            h,
        }
    }

    /// A window that would get a taskbar button appeared.
    pub fn win_created(&mut self, hwnd: isize) {
        if !wins::is_app_window(hwnd) {
            return;
        }
        if let Some(exe) = wins::exe_of(hwnd) {
            self.closed.retain(|c| !c.exe.eq_ignore_ascii_case(&exe));
            self.seen.insert(hwnd, exe);
        }
    }

    /// A window went away. If it was the app's last one, remember the app.
    pub fn win_destroyed(&mut self, hwnd: isize) {
        let Some(exe) = self.seen.remove(&hwnd) else { return };
        self.seen.retain(|h, _| wins::alive(*h));
        if !wins::reopenable(&exe) || self.seen.values().any(|e| e.eq_ignore_ascii_case(&exe)) {
            return;
        }
        self.closed.retain(|c| !c.exe.eq_ignore_ascii_case(&exe));
        self.closed.insert(0, Closed { name: wins::display_name(&exe), exe });
        self.closed.truncate(8);
    }

    fn usage_of(&self, id: &str) -> f32 {
        let rel = id.strip_prefix("shell:AppsFolder\\").unwrap_or(id).to_lowercase();
        self.usage.get(&rel).copied().unwrap_or(0.0) + self.cfg.launches.get(id).copied().unwrap_or(0) as f32 * 3.0
    }

    fn note_launch(&mut self, id: &str) {
        *self.cfg.launches.entry(id.to_string()).or_insert(0) += 1;
        self.dirty = true;
    }

    /// Builds the tabs of the app groups widget: the most used apps, then
    /// categories ordered by use, then the user's own workspaces.
    pub fn rebuild_groups(&mut self) {
        let mut scored: Vec<(f32, usize)> = self.apps.iter().enumerate().map(|(i, a)| (self.usage_of(&a.id), i)).collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut groups = Vec::new();
        let top: Vec<String> = scored.iter().filter(|s| s.0 > 0.0).take(8).map(|s| self.apps[s.1].id.clone()).collect();
        if !top.is_empty() {
            groups.push(Group { name: "Suggested".into(), apps: top, ws: None });
        }
        for cat in ["Productivity", "Development", "Internet", "Media", "Games"] {
            let ids: Vec<String> = scored
                .iter()
                .filter(|s| apps::category(&self.apps[s.1].lname) == Some(cat))
                .take(8)
                .map(|s| self.apps[s.1].id.clone())
                .collect();
            if ids.len() >= 2 {
                groups.push(Group { name: cat.into(), apps: ids, ws: None });
            }
        }
        for (i, w) in self.cfg.workspaces.iter().enumerate() {
            groups.push(Group { name: w.name.clone(), apps: w.apps.clone(), ws: Some(i) });
        }
        self.group_tab = self.group_tab.min(groups.len().saturating_sub(1));
        self.groups = groups;
    }

    /// Runs a typed command line and remembers it.
    fn run_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let split = |c: &str, at: Option<usize>| match at {
            Some(i) => (c[..i].to_string(), c[i + 1..].trim().to_string()),
            None => (c.to_string(), String::new()),
        };
        let (file, params) = if std::path::Path::new(cmd).exists() {
            (cmd.to_string(), String::new())
        } else if let Some(rest) = cmd.strip_prefix('"') {
            split(rest, rest.find('"'))
        } else {
            split(cmd, cmd.find(' '))
        };
        self.cfg.run_history.retain(|h| h != cmd);
        self.cfg.run_history.insert(0, cmd.to_string());
        self.cfg.run_history.truncate(20);
        self.dirty = true;
        self.run.clear();
        self.hist_idx = None;
        self.pending.push(Pending::Run { file, params });
        self.pending.push(Pending::Hide);
    }

    /// Fills the menu with made-up data for screenshots.
    pub fn enable_demo(&mut self) {
        self.demo = true;
        self.user = "Alex Morgan".into();
        self.avatar = None;
        self.usage_at = Some(Instant::now());
        self.files_at = Some(Instant::now());
        self.notes.set("Team sync at 10:00\n- Review the roadmap draft\n- Send slides to design\n- Book flights for March");
        self.cfg.notes = self.notes.text.clone();
        let gb = 1u64 << 30;
        self.drives = vec![
            sysinfo::Drive { root: "C:\\".into(), total: 476 * gb, free: 182 * gb },
            sysinfo::Drive { root: "D:\\".into(), total: 931 * gb, free: 604 * gb },
            sysinfo::Drive { root: "E:\\".into(), total: 1863 * gb, free: 251 * gb },
        ];
        // Real (empty) files in a temp folder, so the shell can give them proper icons.
        let dir = std::env::temp_dir().join("orbit-start-demo");
        let _ = std::fs::create_dir_all(dir.join("Holiday photos"));
        let names = ["Project plan.docx", "Budget 2026.xlsx", "Holiday photos", "Quarterly review.pptx", "Invoice 0142.pdf", "Notes.txt"];
        let files: Vec<FileEntry> = names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                if !p.exists() {
                    let _ = std::fs::write(&p, b"");
                }
                FileEntry { name: n.to_string(), lname: n.to_lowercase(), path: p.to_string_lossy().into_owned() }
            })
            .collect();
        self.downloads = files.clone();
        self.recents = files.into_iter().filter(|f| f.name.contains('.')).collect();
        let win = |exe: &str, title: &str| Win { hwnd: 0, exe: exe.into(), title: title.into() };
        let edge = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
        self.wins = vec![
            win(edge, "Release notes - Microsoft Edge"),
            win("C:\\Windows\\explorer.exe", "Documents"),
            win("C:\\Windows\\System32\\notepad.exe", "todo.txt - Notepad"),
        ];
        self.closed = vec![
            Closed { exe: "C:\\Windows\\System32\\cmd.exe".into(), name: "Command Prompt".into() },
            Closed { exe: "C:\\Windows\\regedit.exe".into(), name: "Registry Editor".into() },
        ];
    }

    /// Opens the app chooser for a custom group; `rename` puts the caret in its name.
    pub fn edit_group(&mut self, idx: usize, rename: bool) {
        let Some(ws) = self.cfg.workspaces.get(idx) else { return };
        self.gname.set(&ws.name.clone());
        self.search.clear();
        self.results.clear();
        self.view = View::PickApps(idx);
        self.scroll = 0.0;
        self.focus = if rename { Focus::GroupName } else { Focus::Search };
        self.gname_fresh = false;
    }

    /// The custom group whose tab is showing, if it contains this app.
    pub fn current_custom_group(&self, id: &str) -> Option<(usize, String)> {
        let g = self.groups.get(self.group_tab)?;
        let ws = g.ws?;
        g.apps.iter().any(|a| a == id).then(|| (ws, g.name.clone()))
    }

    pub fn remove_from_workspace(&mut self, idx: usize, id: &str) {
        if let Some(ws) = self.cfg.workspaces.get_mut(idx) {
            ws.apps.retain(|a| a != id);
            self.dirty = true;
        }
        self.rebuild_groups();
    }

    pub fn workspace_names(&self) -> Vec<String> {
        self.cfg.workspaces.iter().map(|w| w.name.clone()).collect()
    }

    /// Adds an app to workspace `idx`, or to a new workspace when `idx` is None.
    pub fn add_to_workspace(&mut self, idx: Option<usize>, id: &str) {
        let idx = match idx {
            Some(i) if i < self.cfg.workspaces.len() => i,
            _ => {
                if self.cfg.workspaces.len() >= 8 {
                    return;
                }
                let name = format!("Group {}", self.cfg.workspaces.len() + 1);
                self.cfg.workspaces.push(Workspace { name, apps: Vec::new() });
                self.cfg.workspaces.len() - 1
            }
        };
        let ws = &mut self.cfg.workspaces[idx];
        if !ws.apps.iter().any(|a| a == id) && ws.apps.len() < 24 {
            ws.apps.push(id.to_string());
        }
        self.dirty = true;
        self.rebuild_groups();
    }

    pub fn delete_workspace(&mut self, idx: usize) {
        if idx < self.cfg.workspaces.len() {
            self.cfg.workspaces.remove(idx);
            self.dirty = true;
            self.rebuild_groups();
        }
    }

    pub fn launch_workspace(&mut self, idx: usize) {
        if let Some(ws) = self.cfg.workspaces.get(idx) {
            for id in ws.apps.clone() {
                self.pending.push(Pending::Launch { target: id, admin: false });
            }
            self.pending.push(Pending::Hide);
        }
    }

    pub fn refresh_theme(&mut self) {
        self.theme = Theme::from(&self.cfg, sysinfo::system_accent());
        let (w, h) = base_size(self.cfg.style);
        self.w = w;
        self.h = h;
    }

    fn has_widget(&self, w: Widget) -> bool {
        self.cfg.pages.iter().any(|p| p.widgets.contains(&w))
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    fn wants_net(&self) -> bool {
        self.cfg.taskbar_info || self.has_widget(Widget::Gauges) && self.cfg.gauges.iter().any(|g| g == "net")
    }

    fn wants_gpu(&self) -> bool {
        self.has_widget(Widget::Gauges) && self.cfg.gauges.iter().any(|g| g == "gpu")
    }

    pub fn on_show(&mut self) {
        self.view = View::Home;
        self.search.clear();
        self.results.clear();
        self.sel = 0;
        self.focus = Focus::Search;
        self.scroll = 0.0;
        self.mouse = (-1.0, -1.0);
        self.hot = None;
        self.drag = None;
        self.page = self.page.min(self.cfg.pages.len().saturating_sub(1));
        let gpu = self.wants_gpu();
        let net = self.wants_net();
        self.stats = self.sampler.sample(gpu, net);
        if self.demo {
            self.scroll2 = 0.0;
            if self.has_widget(Widget::News) && !self.news_loading && self.news_at.is_none() {
                self.news_loading = true;
                self.pending.push(Pending::FetchNews);
            }
            return;
        }
        if self.has_widget(Widget::Recent) {
            self.recents = apps::recent_files();
        }
        self.scroll2 = 0.0;
        self.hist_idx = None;
        if self.has_widget(Widget::Drives) {
            self.drives = sysinfo::drives();
        }
        if self.has_widget(Widget::Folder) {
            self.downloads = apps::folder_recent(true, 6);
        }
        if self.usage_at.map(|t| t.elapsed().as_secs() > 300).unwrap_or(true) {
            self.usage = apps::usage();
            self.usage_at = Some(Instant::now());
            self.rebuild_groups();
        }
        if self.has_widget(Widget::OpenWindows) {
            self.wins = wins::enumerate();
            for w in &self.wins {
                self.seen.entry(w.hwnd).or_insert_with(|| w.exe.clone());
            }
        }
        let stale = |t: Option<Instant>, secs: u64| t.map(|t| t.elapsed().as_secs() > secs).unwrap_or(true);
        if self.has_widget(Widget::News) && !self.news_loading && stale(self.news_at, 1800) {
            self.news_loading = true;
            self.pending.push(Pending::FetchNews);
        }
        if stale(self.files_at, 900) {
            self.files_at = Some(Instant::now());
            self.pending.push(Pending::RefreshFiles);
        }
    }

    pub fn on_hide(&mut self) {
        if self.dirty {
            self.cfg.save();
            self.dirty = false;
        }
    }

    pub fn tick(&mut self) {
        let gpu = self.wants_gpu();
        let net = self.wants_net();
        self.stats = self.sampler.sample(gpu, net);
    }

    pub fn set_apps(&mut self, mut apps: Vec<AppEntry>) {
        if self.demo {
            apps.retain(|a| DEMO_APPS.iter().any(|d| a.name.eq_ignore_ascii_case(d)));
            // Pretend usage, so the Suggested tab looks lived in.
            for (i, name) in DEMO_USAGE.iter().enumerate() {
                if let Some(a) = apps.iter().find(|a| a.name.eq_ignore_ascii_case(name)) {
                    let rel = a.id.strip_prefix("shell:AppsFolder\\").unwrap_or(&a.id).to_lowercase();
                    self.usage.insert(rel, 100.0 - i as f32 * 8.0);
                }
            }
            let work: Vec<String> = ["Microsoft Edge", "Word", "Excel", "Outlook", "Notepad", "Calculator"]
                .iter()
                .filter_map(|n| apps.iter().find(|a| a.name.eq_ignore_ascii_case(n)).map(|a| a.id.clone()))
                .take(4)
                .collect();
            self.cfg.workspaces = vec![Workspace { name: "Work".into(), apps: work }];
        }
        self.apps = apps;
        if !self.cfg.pinned_initialized && !self.apps.is_empty() {
            for want in ["File Explorer", "Microsoft Edge", "Notepad", "Calculator", "Paint", "Settings", "Terminal", "Photos"] {
                if let Some(a) = self.apps.iter().find(|a| a.name.eq_ignore_ascii_case(want)) {
                    self.cfg.pinned.push(a.id.clone());
                }
            }
            self.cfg.pinned_initialized = true;
            self.dirty = true;
        }
        self.rebuild_groups();
        if !self.search.is_empty() {
            self.update_results();
        }
    }

    pub fn set_icon(&mut self, g: &mut Gfx, d: IconData) {
        self.icon_pending.remove(&d.key);
        let bmp = g.bitmap_from_bgra(d.w, d.h, &d.bgra);
        self.icons.insert(d.key, bmp);
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.cfg.pinned.iter().any(|p| p == id)
    }

    pub fn toggle_pin(&mut self, id: &str) {
        if self.is_pinned(id) {
            self.cfg.pinned.retain(|p| p != id);
        } else {
            self.cfg.pinned.push(id.to_string());
        }
        self.dirty = true;
    }

    pub fn add_widget(&mut self, page: usize, w: Widget) {
        if let Some(p) = self.cfg.pages.get_mut(page) {
            p.widgets.push(w);
            self.dirty = true;
        }
    }

    pub fn paste(&mut self, s: &str) {
        self.field_mut(self.focus).insert_str(s);
        self.after_edit();
    }

    fn field(&self, f: Focus) -> &TextField {
        match f {
            Focus::Search => &self.search,
            Focus::Notes => &self.notes,
            Focus::Calc => &self.calc,
            Focus::Run => &self.run,
            Focus::GroupName => &self.gname,
        }
    }

    fn field_mut(&mut self, f: Focus) -> &mut TextField {
        match f {
            Focus::Search => &mut self.search,
            Focus::Notes => &mut self.notes,
            Focus::Calc => &mut self.calc,
            Focus::Run => &mut self.run,
            Focus::GroupName => &mut self.gname,
        }
    }

    fn after_edit(&mut self) {
        match self.focus {
            Focus::Search => {
                self.sel = 0;
                self.scroll = 0.0;
                self.update_results();
            }
            Focus::Notes => {
                self.cfg.notes = self.notes.text.clone();
                self.dirty = true;
            }
            Focus::Calc | Focus::Run => {}
            Focus::GroupName => {
                if let View::PickApps(i) = self.view {
                    let name = self.gname.text.trim().to_string();
                    if let Some(ws) = self.cfg.workspaces.get_mut(i) {
                        if !name.is_empty() {
                            ws.name = name;
                            self.dirty = true;
                        }
                    }
                    self.rebuild_groups();
                }
            }
        }
    }

    // ---------------------------------------------------------------- search

    fn update_results(&mut self) {
        self.results.clear();
        let raw = self.search.text.trim().to_string();
        let q = raw.to_lowercase();
        if q.is_empty() {
            return;
        }
        let item = |title: String, sub: String, icon: Option<String>, glyph: &'static str, act: Act, section: &'static str| {
            let hl = hl_of(&title, &q);
            ResultItem { title, sub, icon, glyph, act, section, hl }
        };

        // "> notepad file.txt" runs a command directly.
        if let Some(cmd) = raw.strip_prefix('>') {
            let cmd = cmd.trim().to_string();
            if !cmd.is_empty() {
                let mut it = item(format!("Run  {cmd}"), "Command line".into(), None, G_RUN, Act::RunCmd(cmd), "Run");
                it.hl = (0, 0);
                self.results.push(it);
            }
            return;
        }

        if calc::is_math(&q) {
            if let Some(v) = calc::eval(&q) {
                let s = calc::fmt(v);
                let mut it = item(s.clone(), format!("{raw} =   ·   Enter copies the result"), None, G_CALC, Act::Copy(s), "Calculator");
                it.hl = (0, 0);
                self.results.push(it);
            }
        }

        // A web address or an existing path opens directly.
        let has_scheme = q.starts_with("http://") || q.starts_with("https://");
        let tlds = [".com", ".org", ".net", ".io", ".dev", ".gov", ".edu", ".co", ".ai", ".app"];
        let looks_url =
            !q.contains(' ') && (has_scheme || tlds.iter().any(|t| q.ends_with(t) || q.contains(&format!("{t}/"))));
        if looks_url {
            let url = if has_scheme { raw.clone() } else { format!("https://{raw}") };
            let mut it = item(format!("Open {raw}"), "Web address".into(), None, G_GLOBE, Act::Open(url), "Web");
            it.hl = (0, 0);
            self.results.push(it);
        }
        if raw.len() >= 3 && raw.as_bytes()[1] == b':' && std::path::Path::new(&raw).exists() {
            self.results.push(item(raw.clone(), "Open this location".into(), Some(raw.clone()), G_FOLDER, Act::Open(raw.clone()), "Files"));
        }

        // Apps: text match first, then how much each app is actually used.
        let mut scored: Vec<(i32, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(i, a)| apps::score(&a.lname, &q).map(|s| (s + (self.usage_of(&a.id).min(60.0) * 3.0) as i32, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, i) in scored.into_iter().take(7) {
            let a = &self.apps[i];
            self.results.push(item(a.name.clone(), String::new(), Some(a.id.clone()), G_APPS, Act::App(a.id.clone()), "Apps"));
        }

        if q.chars().count() >= 2 {
            let mut pages: Vec<(i32, usize)> = SETTINGS_PAGES
                .iter()
                .enumerate()
                .filter_map(|(i, p)| apps::score(&p.0.to_lowercase(), &q).map(|s| (s, i)))
                .collect();
            pages.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, i) in pages.into_iter().take(4) {
                let (name, uri) = SETTINGS_PAGES[i];
                self.results.push(item(name.into(), "Windows Settings".into(), None, G_SETTINGS, Act::Open(uri.into()), "Settings"));
            }

            let cmds: [(&str, &str, &'static str, Cmd); 7] = [
                ("lock", "Lock this PC", G_LOCK, Cmd::Lock),
                ("sleep", "Sleep", G_POWER, Cmd::Sleep),
                ("sign out", "Sign out", G_USER, Cmd::SignOut),
                ("restart", "Restart", G_POWER, Cmd::Restart),
                ("shut down", "Shut down", G_POWER, Cmd::Shutdown),
                ("customize orbit start", "Customize Orbit Start", G_CUSTOMIZE, Cmd::Customize),
                ("quit orbit start", "Quit Orbit Start", G_CLOSE, Cmd::Quit),
            ];
            for (key, title, glyph, cmd) in cmds {
                if key.starts_with(&q) || key.split(' ').any(|w| w.starts_with(&q)) {
                    self.results.push(item(title.into(), "Command".into(), None, glyph, Act::Cmd(cmd), "Commands"));
                }
            }

            let mut files: Vec<(i32, usize)> = self
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| f.lname.contains(&q))
                .map(|(i, f)| (if f.lname.starts_with(&q) { 1000 } else { 500 } - f.lname.len() as i32, i))
                .collect();
            files.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, i) in files.into_iter().take(6) {
                let f = &self.files[i];
                let folder = f.path.rsplit_once('\\').map(|p| p.0.to_string()).unwrap_or_default();
                self.results.push(item(f.name.clone(), folder, Some(f.path.clone()), G_DOC, Act::Open(f.path.clone()), "Files"));
            }
        }
        let mut web = item(format!("Search the web for \"{raw}\""), "Opens in your browser".into(), None, G_GLOBE, Act::Web(raw), "Web");
        web.hl = (0, 0);
        self.results.push(web);
    }

    // ----------------------------------------------------------------- input

    fn hit_at(&self) -> Option<usize> {
        self.hits.iter().rposition(|h| h.rect.contains(self.mouse))
    }

    fn set_slider(&mut self, kind: SliderKind, track: Rect) {
        let v = ((self.mouse.0 - track.x) / track.w).clamp(0.0, 1.0);
        match kind {
            SliderKind::Tint => self.cfg.tint_opacity = v,
            SliderKind::Scale => self.cfg.scale = ((0.7 + v * 0.9) * 20.0).round() / 20.0,
        }
        self.dirty = true;
    }

    /// Returns true when the menu needs to be redrawn.
    pub fn mouse_move(&mut self, x: f32, y: f32) -> bool {
        self.mouse = (x, y);
        if let Some((kind, track)) = self.drag {
            self.set_slider(kind, track);
            return true;
        }
        let idx = self.hit_at();
        if idx != self.hot {
            self.hot = idx;
            return true;
        }
        false
    }

    pub fn mouse_leave(&mut self) -> bool {
        if self.drag.is_some() {
            return false;
        }
        self.mouse = (-1.0, -1.0);
        self.hot.take().is_some()
    }

    pub fn mouse_down(&mut self, x: f32, y: f32) -> bool {
        self.mouse = (x, y);
        self.down = self.hit_at().map(|i| self.hits[i].act.clone());
        match &self.down {
            Some(Act::Slider(kind, track)) => {
                let (kind, track) = (*kind, *track);
                self.drag = Some((kind, track));
                self.set_slider(kind, track);
            }
            None => self.focus = Focus::Search,
            _ => {}
        }
        true
    }

    pub fn mouse_up(&mut self, x: f32, y: f32) -> bool {
        self.mouse = (x, y);
        if let Some((kind, _)) = self.drag.take() {
            if kind == SliderKind::Scale {
                self.pending.push(Pending::Relayout);
            }
            self.down = None;
            return true;
        }
        let act = self.hit_at().map(|i| self.hits[i].act.clone());
        if act.is_some() && act == self.down.take() {
            self.perform(act.unwrap());
        }
        true
    }

    pub fn right_click(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        match self.hit_at().map(|i| self.hits[i].act.clone()) {
            Some(Act::GroupTab(i)) => {
                if let Some(ws) = self.groups.get(i).and_then(|g| g.ws) {
                    self.pending.push(Pending::ContextWorkspace(ws));
                }
            }
            Some(Act::App(id)) => self.pending.push(Pending::ContextApp(id)),
            Some(Act::Workspace(i)) => self.pending.push(Pending::ContextWorkspace(i)),
            _ => {}
        }
    }

    pub fn wheel(&mut self, notches: f32) -> bool {
        if self.cfg.style == Style::Apps && self.mouse.0 > APPS_LEFT && self.mouse.1 > HEADER_H {
            let before = self.scroll2;
            self.scroll2 = (self.scroll2 - notches * 56.0).clamp(0.0, self.max_scroll2);
            return before != self.scroll2;
        }
        // Over the page dots, or on a page with nothing to scroll, the wheel turns pages.
        let paging = self.view == View::Home && self.search.is_empty() && self.cfg.pages.len() > 1;
        if paging && (self.mouse.1 > self.h - 46.0 || self.max_scroll <= 0.0) {
            self.switch_page(if notches < 0.0 { 1 } else { -1 });
            return true;
        }
        let before = self.scroll;
        self.scroll = (self.scroll - notches * 56.0).clamp(0.0, self.max_scroll);
        before != self.scroll
    }

    pub fn on_char(&mut self, c: char) -> bool {
        if c.is_control() {
            return false;
        }
        if self.focus == Focus::Search && self.view == View::Settings && self.search.is_empty() {
            self.view = View::Home;
        }
        if self.focus == Focus::GroupName && std::mem::take(&mut self.gname_fresh) {
            self.gname.clear();
        }
        self.field_mut(self.focus).insert(c);
        self.after_edit();
        true
    }

    fn switch_page(&mut self, delta: i32) {
        let n = self.cfg.pages.len() as i32;
        if n > 0 {
            self.page = (self.page as i32 + delta).rem_euclid(n) as usize;
            self.scroll = 0.0;
        }
    }

    fn reveal_selection(&mut self) {
        let top = self.result_y.get(self.sel).copied().unwrap_or(0.0);
        self.scroll = self.scroll.clamp((top - 300.0).max(0.0), top.max(0.0));
    }

    /// `vk` is a Windows virtual-key code.
    pub fn on_key(&mut self, vk: u32, ctrl: bool) -> bool {
        let searching = !self.search.is_empty();
        match vk {
            0x1B => {
                if self.focus != Focus::Search {
                    self.focus = Focus::Search;
                } else if searching {
                    self.search.clear();
                    self.update_results();
                } else if self.view != View::Home {
                    self.view = View::Home;
                    self.scroll = 0.0;
                } else {
                    self.pending.push(Pending::Hide);
                }
            }
            0x08 => {
                self.field_mut(self.focus).backspace();
                self.after_edit();
            }
            0x2E => {
                self.field_mut(self.focus).delete();
                self.after_edit();
            }
            0x0D => match self.focus {
                Focus::Notes => {
                    self.notes.insert('\n');
                    self.after_edit();
                }
                Focus::Calc => {
                    if let Some(v) = calc::eval(&self.calc.text) {
                        self.calc.set(&calc::fmt(v).replace(',', ""));
                    }
                }
                Focus::Run => {
                    let cmd = self.run.text.clone();
                    self.run_command(&cmd);
                }
                Focus::GroupName => self.focus = Focus::Search,
                // While choosing apps, the search box only filters the list.
                Focus::Search if matches!(self.view, View::PickApps(_)) => {}
                // In the clock editor, Enter adds the first matching city.
                Focus::Search if self.view == View::Clocks => {
                    if let Some(i) = self.city_matches().first().copied() {
                        self.perform(Act::ClockAdd(i));
                    }
                }
                Focus::Search => {
                    if let Some(r) = self.results.get(self.sel) {
                        let act = r.act.clone();
                        match (&act, ctrl) {
                            // Ctrl+Enter: run the app as administrator.
                            (Act::App(id), true) => {
                                let id = id.clone();
                                self.note_launch(&id);
                                self.pending.push(Pending::Launch { target: id, admin: true });
                                self.pending.push(Pending::Hide);
                            }
                            _ => self.perform(act),
                        }
                    }
                }
            },
            0x26 | 0x28 if self.focus == Focus::Run => {
                let n = self.cfg.run_history.len();
                if n > 0 {
                    self.hist_idx = match (self.hist_idx, vk == 0x26) {
                        (None, true) => Some(0),
                        (Some(i), true) => Some((i + 1).min(n - 1)),
                        (Some(0), false) | (None, false) => None,
                        (Some(i), false) => Some(i - 1),
                    };
                    match self.hist_idx {
                        Some(i) => {
                            let h = self.cfg.run_history[i].clone();
                            self.run.set(&h);
                        }
                        None => self.run.clear(),
                    }
                }
            }
            0x26 | 0x28 if self.focus == Focus::Search && searching => {
                let n = self.results.len();
                if n > 0 {
                    self.sel = if vk == 0x26 { (self.sel + n - 1) % n } else { (self.sel + 1) % n };
                    self.reveal_selection();
                }
            }
            // Tab jumps to the first result of the next section.
            0x09 if self.focus == Focus::Search && searching => {
                let cur = self.results.get(self.sel).map(|r| r.section);
                self.sel = self
                    .results
                    .iter()
                    .enumerate()
                    .skip(self.sel + 1)
                    .find(|(_, r)| Some(r.section) != cur)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.reveal_selection();
            }
            0x25 | 0x27 => {
                if self.focus == Focus::Search && !searching {
                    if self.view == View::Home {
                        self.switch_page(if vk == 0x25 { -1 } else { 1 });
                    }
                } else if vk == 0x25 {
                    self.field_mut(self.focus).left();
                } else {
                    self.field_mut(self.focus).right();
                }
            }
            0x24 => self.field_mut(self.focus).home(),
            0x23 => self.field_mut(self.focus).end(),
            0x21 if self.view == View::Home && !searching => self.switch_page(-1),
            0x22 if self.view == View::Home && !searching => self.switch_page(1),
            _ => return false,
        }
        true
    }

    fn launch(&mut self, target: String) {
        self.pending.push(Pending::Launch { target, admin: false });
        self.pending.push(Pending::Hide);
    }

    fn layout_changed(&mut self) {
        self.card_h.clear();
        self.cfg.sanitize();
        self.refresh_theme();
        self.dirty = true;
        self.pending.push(Pending::Relayout);
    }

    fn perform(&mut self, act: Act) {
        match act {
            Act::FocusSearch => self.focus = Focus::Search,
            Act::ClearSearch => {
                self.search.clear();
                self.focus = Focus::Search;
                self.update_results();
            }
            Act::App(id) => {
                self.note_launch(&id);
                self.launch(id);
            }
            Act::Open(id) => self.launch(id),
            Act::RunCmd(cmd) => self.run_command(&cmd),
            Act::GroupNew => {
                if self.cfg.workspaces.len() < 8 {
                    let name = format!("Group {}", self.cfg.workspaces.len() + 1);
                    self.cfg.workspaces.push(Workspace { name, apps: Vec::new() });
                    self.dirty = true;
                    self.rebuild_groups();
                    let idx = self.cfg.workspaces.len() - 1;
                    self.group_tab = self.groups.iter().position(|g| g.ws == Some(idx)).unwrap_or(0);
                    self.edit_group(idx, true);
                    self.gname_fresh = true;
                }
            }
            Act::GroupEdit(i) => self.edit_group(i, false),
            Act::GroupRename(i) => self.edit_group(i, true),
            Act::GroupDelete(i) => {
                self.delete_workspace(i);
                self.search.clear();
                self.results.clear();
                self.focus = Focus::Search;
                self.view = View::Home;
                self.scroll = 0.0;
            }
            Act::PickToggle(id) => {
                if let View::PickApps(i) = self.view {
                    if let Some(ws) = self.cfg.workspaces.get_mut(i) {
                        if let Some(p) = ws.apps.iter().position(|a| *a == id) {
                            ws.apps.remove(p);
                        } else if ws.apps.len() < 24 {
                            ws.apps.push(id);
                        }
                        self.dirty = true;
                    }
                    self.rebuild_groups();
                }
            }
            Act::FocusGroupName => self.focus = Focus::GroupName,
            Act::EditClocks => {
                self.search.clear();
                self.results.clear();
                self.focus = Focus::Search;
                self.view = View::Clocks;
                self.scroll = 0.0;
            }
            Act::ClockAdd(i) => {
                if let Some((city, _, tz)) = CITIES.get(i) {
                    if self.cfg.world_clocks.len() < MAX_CLOCKS
                        && !self.cfg.world_clocks.iter().any(|c| c.label == *city && c.tz == *tz)
                    {
                        self.cfg.world_clocks.push(WorldClock { label: city.to_string(), tz: tz.to_string() });
                        self.dirty = true;
                    }
                    self.search.clear();
                    self.results.clear();
                }
            }
            Act::ClockDel(i) => {
                if i < self.cfg.world_clocks.len() {
                    self.cfg.world_clocks.remove(i);
                    self.dirty = true;
                }
            }
            Act::ClockUp(i) => {
                if i > 0 && i < self.cfg.world_clocks.len() {
                    self.cfg.world_clocks.swap(i, i - 1);
                    self.dirty = true;
                }
            }
            Act::ClockDown(i) => {
                if i + 1 < self.cfg.world_clocks.len() {
                    self.cfg.world_clocks.swap(i, i + 1);
                    self.dirty = true;
                }
            }
            Act::GroupTab(i) => self.group_tab = i,
            Act::FocusRun => self.focus = Focus::Run,
            Act::Web(q) => {
                let url = self.cfg.web_search.replace("{}", &url_encode(&q));
                if url.starts_with("https://") || url.starts_with("http://") {
                    self.launch(url);
                }
            }
            Act::Copy(s) => {
                self.pending.push(Pending::Copy(s));
                self.pending.push(Pending::Hide);
            }
            Act::Cmd(Cmd::Customize) | Act::Customize => {
                self.search.clear();
                self.results.clear();
                self.view = View::Settings;
                self.scroll = 0.0;
            }
            Act::Cmd(c) => {
                self.pending.push(Pending::Hide);
                self.pending.push(Pending::Power(c));
            }
            Act::PagePrev => self.switch_page(-1),
            Act::PageNext => self.switch_page(1),
            Act::PageGo(i) => {
                self.page = i.min(self.cfg.pages.len().saturating_sub(1));
                self.scroll = 0.0;
            }
            Act::AllApps => {
                self.view = View::AllApps;
                self.scroll = 0.0;
            }
            Act::Home => {
                self.view = View::Home;
                self.scroll = 0.0;
            }
            Act::PowerMenu => self.pending.push(Pending::PowerMenu),
            Act::FocusNotes => self.focus = Focus::Notes,
            Act::FocusCalc => self.focus = Focus::Calc,
            Act::SetStyle(s) => {
                self.cfg.style = s;
                self.layout_changed();
            }
            Act::SetPos(p) => {
                self.cfg.position = p;
                self.layout_changed();
            }
            Act::SetBackdrop(b) => {
                self.cfg.backdrop = b;
                self.layout_changed();
            }
            Act::SetTheme(t) => {
                self.cfg.theme = t;
                self.layout_changed();
            }
            Act::SetAccent(a) => {
                self.cfg.accent = a;
                self.layout_changed();
            }
            Act::SetIcon(px) => {
                self.cfg.icon_size = px as f32;
                self.dirty = true;
            }
            Act::Toggle(t) => {
                match t {
                    Tog::WinKey => {
                        self.cfg.hook_win_key = !self.cfg.hook_win_key;
                        self.pending.push(Pending::HookChanged);
                    }
                    Tog::Autostart => {
                        self.cfg.autostart = !self.cfg.autostart;
                        sysinfo::set_autostart(self.cfg.autostart);
                        self.autostart = self.cfg.autostart;
                    }
                    Tog::Seconds => self.cfg.show_seconds = !self.cfg.show_seconds,
                    Tog::H24 => self.cfg.clock_24h = !self.cfg.clock_24h,
                    Tog::TaskbarInfo => self.cfg.taskbar_info = !self.cfg.taskbar_info,
                    Tog::StartButton => {
                        self.cfg.hook_start_button = !self.cfg.hook_start_button;
                        self.pending.push(Pending::TaskbarChanged);
                    }
                }
                self.dirty = true;
            }
            Act::Slider(..) => {}
            Act::WidgetUp(p, i) => {
                if let Some(pg) = self.cfg.pages.get_mut(p) {
                    if i > 0 && i < pg.widgets.len() {
                        pg.widgets.swap(i, i - 1);
                        self.dirty = true;
                    }
                }
            }
            Act::WidgetDown(p, i) => {
                if let Some(pg) = self.cfg.pages.get_mut(p) {
                    if i + 1 < pg.widgets.len() {
                        pg.widgets.swap(i, i + 1);
                        self.dirty = true;
                    }
                }
            }
            Act::WidgetDel(p, i) => {
                if let Some(pg) = self.cfg.pages.get_mut(p) {
                    if i < pg.widgets.len() {
                        pg.widgets.remove(i);
                        self.dirty = true;
                    }
                }
            }
            Act::WidgetAdd(p) => self.pending.push(Pending::AddWidget(p)),
            Act::PageAdd => {
                if self.cfg.pages.len() < 8 {
                    self.cfg.pages.push(Page { widgets: Vec::new() });
                    self.dirty = true;
                }
            }
            Act::PageDel(p) => {
                if self.cfg.pages.len() > 1 && p < self.cfg.pages.len() {
                    self.cfg.pages.remove(p);
                    self.page = self.page.min(self.cfg.pages.len() - 1);
                    self.dirty = true;
                }
            }
            Act::Export => self.pending.push(Pending::Export),
            Act::Import => self.pending.push(Pending::Import),
            Act::Reset => {
                self.cfg.apply_layout(Config::default().layout());
                self.page = 0;
                self.layout_changed();
            }
            Act::Workspace(i) => self.launch_workspace(i),
            Act::Window(h) => {
                self.pending.push(Pending::Hide);
                self.pending.push(Pending::Activate(h));
            }
            Act::SetTaskbar(s) => {
                self.cfg.taskbar_style = s;
                self.dirty = true;
                self.pending.push(Pending::TaskbarChanged);
            }
            Act::Preset(i) => {
                if let Some((_, layout)) = presets().into_iter().nth(i) {
                    self.cfg.apply_layout(layout);
                    self.page = 0;
                    self.scroll = 0.0;
                    self.layout_changed();
                    self.pending.push(Pending::TaskbarChanged);
                }
            }
            Act::OpenConfig => {
                self.cfg.save();
                self.dirty = false;
                self.launch(Config::path().to_string_lossy().into_owned());
            }
        }
    }

    // --------------------------------------------------------------- drawing

    fn hit(&mut self, r: Rect, act: Act) -> bool {
        let r = match self.clip {
            Some(c) => r.intersect(&c),
            None => r,
        };
        self.hits.push(Hit { rect: r, act });
        self.drag.is_none() && r.contains(self.mouse)
    }

    fn draw_icon(&mut self, g: &mut Gfx, key: &str, r: Rect) {
        match self.icons.get(key) {
            Some(Some(b)) => g.draw_bitmap(b, r, 1.0),
            Some(None) => g.fill_rr(r.inset(r.w * 0.12), 6.0, self.theme.card.a(0.12)),
            None => {
                g.fill_rr(r.inset(r.w * 0.12), 6.0, self.theme.card.a(0.08));
                if self.icon_pending.insert(key.to_string()) {
                    let _ = self.icon_tx.send(IconReq { key: key.to_string(), px: self.icon_px });
                }
            }
        }
    }

    pub fn draw(&mut self, g: &mut Gfx) {
        self.hits.clear();
        self.clip = None;
        let t = self.theme;
        g.begin();
        let full = Rect::new(0.0, 0.0, self.w, self.h);
        let tint = if self.cfg.backdrop == Backdrop::Solid { 1.0 } else { self.cfg.tint_opacity };
        g.fill_rect(full, t.bg.a(tint));
        g.stroke_rr(full, 8.0, t.card.a(0.10), 1.0);

        match self.cfg.style {
            Style::TwoPane => {
                let lw = self.w - RIGHT_W;
                self.draw_right(g, Rect::new(lw, 0.0, RIGHT_W, self.h));
                self.draw_main(g, Rect::new(0.0, 0.0, lw, self.h), PAD);
            }
            Style::Compact => {
                self.draw_header(g);
                self.draw_main(g, full, HEADER_H);
            }
            Style::Apps => {
                self.draw_header(g);
                self.draw_app_pane(g, Rect::new(APPS_LEFT, HEADER_H, self.w - APPS_LEFT, self.h - HEADER_H));
                self.draw_main(g, Rect::new(0.0, 0.0, APPS_LEFT, self.h), HEADER_H + 14.0);
            }
            Style::Dashboard => {
                self.draw_sidebar(g, Rect::new(0.0, 0.0, SIDE_W, self.h));
                self.draw_main(g, Rect::new(SIDE_W, 0.0, self.w - SIDE_W, self.h), PAD);
            }
        }
        g.end();
        self.hot = self.hit_at();
    }

    fn avatar(&mut self, g: &mut Gfx, cx: f32, cy: f32, r: f32) {
        let t = self.theme;
        if let Some(b) = &self.avatar {
            g.draw_bitmap_circle(b, cx, cy, r);
        } else {
            g.fill_circle(cx, cy, r, t.accent);
            let initial: String = self.user.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            g.text(&initial, Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), ts(r * 0.9).semi().center(), Color::rgb(0xffffff));
        }
        g.stroke_circle(cx, cy, r, t.card.a(0.25), 1.5);
    }

    fn round_button(&mut self, g: &mut Gfx, cx: f32, cy: f32, glyph: &str, act: Act) {
        let t = self.theme;
        let r = Rect::new(cx - 19.0, cy - 19.0, 38.0, 38.0);
        if self.hit(r, act) {
            g.fill_circle(cx, cy, 19.0, t.card.a(0.14));
        }
        g.text(glyph, r, ts(17.0).icon().center(), t.fg);
    }

    fn draw_header(&mut self, g: &mut Gfx) {
        let t = self.theme;
        self.avatar(g, PAD + 20.0, 36.0, 20.0);
        let mut btns: Vec<(&str, Act)> = vec![(G_POWER, Act::PowerMenu), (G_CUSTOMIZE, Act::Customize)];
        if self.cfg.style == Style::Apps {
            btns.push((G_SETTINGS, Act::Open("ms-settings:".into())));
            btns.push((G_CONTROL, Act::Open("control.exe".into())));
            btns.push((G_FOLDER, Act::Open("shell:Profile".into())));
        } else {
            btns.push((G_APPS, Act::AllApps));
        }
        let name = self.user.clone();
        let name_w = self.w - PAD * 2.0 - 60.0 - btns.len() as f32 * 44.0;
        g.text(&name, Rect::new(PAD + 52.0, 16.0, name_w, 40.0), ts(18.0).semi(), t.fg);
        let mut x = self.w - PAD - 19.0;
        for (glyph, act) in btns {
            self.round_button(g, x, 36.0, glyph, act);
            x -= 44.0;
        }
    }

    /// Always-visible app list on the right of the Apps style.
    fn draw_app_pane(&mut self, g: &mut Gfx, a: Rect) {
        let t = self.theme;
        g.fill_rect(a, Color::rgb(0).a(if t.light { 0.05 } else { 0.20 }));
        g.fill_rect(Rect::new(0.0, a.y, self.w, 1.0), t.card.a(0.08));
        g.fill_rect(Rect::new(a.x, a.y, 1.0, a.h), t.card.a(0.08));
        let c = Rect::new(a.x + 14.0, a.y + 10.0, a.w - 26.0, a.h - 20.0);
        g.push_clip(c);
        self.clip = Some(c);
        let total = self.app_rows(g, c, self.scroll2, 0.0);
        self.clip = None;
        g.pop_clip();
        self.max_scroll2 = (total - c.h).max(0.0);
        self.scroll2 = self.scroll2.min(self.max_scroll2);
        if self.max_scroll2 > 0.0 {
            let thumb = (c.h / total * c.h).max(28.0);
            let y = c.y + (c.h - thumb) * (self.scroll2 / self.max_scroll2);
            g.fill_rr(Rect::new(a.right() - 9.0, y, 4.0, thumb), 2.0, t.card.a(0.28));
        }
    }

    fn nav_row(&mut self, g: &mut Gfx, r: Rect, icon: Option<&str>, glyph: &str, label: &str, act: Act) {
        let t = self.theme;
        if self.hit(r, act) {
            g.fill_rr(r, 8.0, t.card.a(0.10));
        }
        match icon {
            Some(key) => self.draw_icon(g, key, Rect::new(r.x + 10.0, r.y + (r.h - 22.0) / 2.0, 22.0, 22.0)),
            None => g.text(glyph, Rect::new(r.x + 8.0, r.y, 26.0, r.h), ts(16.0).icon().center(), t.fg),
        }
        g.text(label, Rect::new(r.x + 44.0, r.y, r.w - 52.0, r.h), ts(13.5), t.fg);
    }

    /// Left column of the dashboard style: account, pinned apps, folders, power.
    fn draw_sidebar(&mut self, g: &mut Gfx, a: Rect) {
        let t = self.theme;
        g.fill_rect(a, Color::rgb(0).a(if t.light { 0.05 } else { 0.22 }));
        g.fill_rect(Rect::new(a.right() - 1.0, 0.0, 1.0, a.h), t.card.a(0.08));
        self.avatar(g, a.x + 36.0, 42.0, 18.0);
        let name = self.user.clone();
        g.text(&name, Rect::new(a.x + 64.0, 24.0, a.w - 76.0, 36.0), ts(14.5).semi(), t.fg);

        let row = 37.0;
        let bottom_y = a.h - 14.0 - 3.0 * row;
        let folders: [(&str, &str, &str); 4] = [
            (G_DOC, "Documents", "shell:Personal"),
            (G_DOWNLOAD, "Downloads", "shell:Downloads"),
            (G_MUSIC, "Music", "shell:My Music"),
            (G_PICTURE, "Pictures", "shell:My Pictures"),
        ];
        let mut y = 80.0;
        let room = ((bottom_y - 18.0 - folders.len() as f32 * row - 18.0 - y) / row).floor().max(0.0) as usize;
        let pins: Vec<String> = self.cfg.pinned.iter().take(room).cloned().collect();
        for id in pins {
            let label = self.apps.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
            self.nav_row(g, Rect::new(a.x + 12.0, y, a.w - 24.0, row - 3.0), Some(&id), "", &label, Act::App(id.clone()));
            y += row;
        }
        y += 8.0;
        g.fill_rect(Rect::new(a.x + 22.0, y, a.w - 44.0, 1.0), t.card.a(0.10));
        y += 10.0;
        for (glyph, label, target) in folders {
            self.nav_row(g, Rect::new(a.x + 12.0, y, a.w - 24.0, row - 3.0), None, glyph, label, Act::Open(target.into()));
            y += row;
        }
        let bottom: [(&str, &str, Act); 3] =
            [(G_APPS, "All apps", Act::AllApps), (G_CUSTOMIZE, "Customize", Act::Customize), (G_POWER, "Power", Act::PowerMenu)];
        let mut y = bottom_y;
        for (glyph, label, act) in bottom {
            self.nav_row(g, Rect::new(a.x + 12.0, y, a.w - 24.0, row - 3.0), None, glyph, label, act);
            y += row;
        }
    }

    fn draw_right(&mut self, g: &mut Gfx, a: Rect) {
        let t = self.theme;
        g.fill_rect(a, Color::rgb(0).a(if t.light { 0.05 } else { 0.22 }));
        g.fill_rect(Rect::new(a.x, 0.0, 1.0, a.h), t.card.a(0.08));
        let cx = a.x + a.w / 2.0;
        self.avatar(g, cx, 62.0, 38.0);
        let name = self.user.clone();
        g.text(&name, Rect::new(a.x + 12.0, 108.0, a.w - 24.0, 34.0), ts(21.0).center(), t.fg);

        let links: [(&str, String, Act); 9] = [
            (G_DOC, "Documents".into(), Act::Open("shell:Personal".into())),
            (G_DOWNLOAD, "Downloads".into(), Act::Open("shell:Downloads".into())),
            (G_MUSIC, "Music".into(), Act::Open("shell:My Music".into())),
            (G_PICTURE, "Pictures".into(), Act::Open("shell:My Pictures".into())),
            (G_USER, name, Act::Open("shell:Profile".into())),
            (G_CONTROL, "Control Panel".into(), Act::Open("control.exe".into())),
            (G_SETTINGS, "Settings".into(), Act::Open("ms-settings:".into())),
            (G_RUN, "Run…".into(), Act::Open("shell:::{2559a1f3-21d7-11d4-bdaf-00c04f60b9f0}".into())),
            (G_APPS, "All apps".into(), Act::AllApps),
        ];
        let top = 156.0;
        let row = ((a.h - top - 74.0) / links.len() as f32).min(46.0);
        for (i, (glyph, label, act)) in links.into_iter().enumerate() {
            let r = Rect::new(a.x + 22.0, top + i as f32 * row, a.w - 44.0, row - 2.0);
            let chevron = act == Act::AllApps;
            if self.hit(r, act) {
                g.fill_rr(r, 8.0, t.card.a(0.10));
            }
            g.text(glyph, Rect::new(r.x + 12.0, r.y, 28.0, r.h), ts(19.0).icon().center(), t.fg);
            g.text(&label, Rect::new(r.x + 54.0, r.y, r.w - 84.0, r.h), ts(14.5), t.fg);
            if chevron {
                g.text(G_RIGHT, Rect::new(r.right() - 30.0, r.y, 24.0, r.h), ts(12.0).icon().center(), t.fg);
            }
        }
        let by = a.h - 40.0;
        self.round_button(g, cx - 26.0, by, G_CUSTOMIZE, Act::Customize);
        self.round_button(g, cx + 26.0, by, G_POWER, Act::PowerMenu);
    }

    fn draw_field(&mut self, g: &mut Gfx, which: Focus, r: Rect, st: Ts, placeholder: &str) {
        let t = self.theme;
        let (text, caret, multi) = {
            let f = self.field(which);
            (f.text.clone(), f.caret_utf16(), f.multiline)
        };
        let st = if multi { st.wrap() } else { st };
        if text.is_empty() {
            g.text(placeholder, r, st, t.dim.a(0.75));
        }
        g.push_clip(r);
        if let Some(l) = g.layout(&text, st, if multi { r.w } else { 4000.0 }, r.h) {
            g.draw_layout(r.x, r.y, &l, t.fg);
            if self.focus == which {
                let (cx, cy, ch) = Gfx::caret_pos(&l, caret);
                let ch = if ch > 0.0 { ch } else { st.size * 1.3 };
                g.fill_rect(Rect::new(r.x + cx, r.y + cy + 1.0, 1.6, ch - 2.0), t.accent);
            }
        }
        g.pop_clip();
    }

    fn draw_main(&mut self, g: &mut Gfx, a: Rect, top: f32) {
        let t = self.theme;
        // Search box
        let sb = Rect::new(a.x + PAD, top, a.w - 2.0 * PAD, 46.0);
        g.fill_rr(sb, 23.0, t.field);
        if self.focus == Focus::Search {
            g.stroke_rr(sb, 23.0, t.card.a(0.14), 1.0);
        }
        self.hit(sb, Act::FocusSearch);
        g.text(G_SEARCH, Rect::new(sb.x + 14.0, sb.y, 30.0, sb.h), ts(16.0).icon().center(), t.fg);
        let field = Rect::new(sb.x + 52.0, sb.y, sb.w - 100.0, sb.h);
        self.draw_field(g, Focus::Search, field, ts(15.0), "Search apps and files");
        if !self.search.is_empty() {
            let x = Rect::new(sb.right() - 42.0, sb.y + 6.0, 34.0, 34.0);
            if self.hit(x, Act::ClearSearch) {
                g.fill_circle(x.x + 17.0, x.y + 17.0, 15.0, t.card.a(0.14));
            }
            g.text(G_CLOSE, x, ts(11.0).icon().center(), t.fg);
        }

        let has_nav = self.search.is_empty() && self.view == View::Home && self.cfg.pages.len() > 1;
        let cy = sb.bottom() + 18.0;
        let content = Rect::new(a.x + PAD, cy, a.w - 2.0 * PAD, a.bottom() - cy - if has_nav { 44.0 } else { 14.0 });

        g.push_clip(content);
        self.clip = Some(content);
        let total = if let View::PickApps(i) = self.view {
            self.draw_picker(g, content, i)
        } else if self.view == View::Clocks {
            self.draw_clock_editor(g, content)
        } else if !self.search.is_empty() {
            self.draw_results(g, content)
        } else {
            match self.view {
                View::Home => self.draw_page(g, content),
                View::AllApps => self.draw_all_apps(g, content),
                View::Settings => self.draw_settings(g, content),
                View::PickApps(_) | View::Clocks => 0.0,
            }
        };
        self.clip = None;
        g.pop_clip();

        self.max_scroll = (total - content.h).max(0.0);
        if self.scroll > self.max_scroll {
            self.scroll = self.max_scroll;
        }
        if self.max_scroll > 0.0 {
            let track_h = content.h;
            let thumb = (content.h / total * track_h).max(28.0);
            let y = content.y + (track_h - thumb) * (self.scroll / self.max_scroll);
            g.fill_rr(Rect::new(a.right() - 12.0, y, 4.0, thumb), 2.0, t.card.a(0.28));
        }

        if has_nav {
            let n = self.cfg.pages.len();
            let y = a.bottom() - 34.0;
            let wdt = n as f32 * 20.0 + 64.0;
            let mut x = a.x + (a.w - wdt) / 2.0;
            let pr = Rect::new(x, y, 28.0, 26.0);
            let hot = self.hit(pr, Act::PagePrev);
            g.text(G_LEFT, pr, ts(11.0).icon().center(), if hot { t.fg } else { t.dim });
            x += 32.0;
            for i in 0..n {
                let r = Rect::new(x, y, 20.0, 26.0);
                let hot = self.hit(r, Act::PageGo(i));
                let on = i == self.page;
                g.fill_circle(x + 10.0, y + 13.0, if on { 5.0 } else { 3.5 }, if on || hot { t.fg } else { t.dim.a(0.7) });
                x += 20.0;
            }
            let nx = Rect::new(x + 4.0, y, 28.0, 26.0);
            let hot = self.hit(nx, Act::PageNext);
            g.text(G_RIGHT, nx, ts(11.0).icon().center(), if hot { t.fg } else { t.dim });
        }
    }

    fn draw_page(&mut self, g: &mut Gfx, c: Rect) -> f32 {
        let t = self.theme;
        let widgets = self.cfg.pages.get(self.page).map(|p| p.widgets.clone()).unwrap_or_default();
        if widgets.is_empty() {
            g.text(
                "This page is empty. Open Customize to add widgets.",
                Rect::new(c.x, c.y + 40.0, c.w, 30.0),
                ts(14.0).center(),
                t.dim,
            );
            return 0.0;
        }
        if self.cfg.style == Style::Dashboard {
            // Two columns of cards; each widget goes in the shorter column.
            let (gap, pad) = (14.0, 16.0);
            let colw = (c.w - gap) / 2.0;
            let mut hs = [0.0f32; 2];
            for (i, w) in widgets.into_iter().enumerate() {
                let col = if hs[0] <= hs[1] { 0 } else { 1 };
                let x = c.x + col as f32 * (colw + gap);
                let y = c.y - self.scroll + hs[col];
                let key = (self.page, i, w as u8);
                let prev = self.card_h.get(&key).copied().unwrap_or(0.0);
                if prev > 0.0 {
                    let card = Rect::new(x, y, colw, prev + 2.0 * pad);
                    g.fill_rr(card, 12.0, t.card.a(0.055));
                    g.stroke_rr(card, 12.0, t.card.a(0.07), 1.0);
                }
                let h = self.draw_widget(g, w, x + pad, y + pad, colw - 2.0 * pad);
                if (h - prev).abs() > 0.5 {
                    self.card_h.insert(key, h);
                    self.redraw = true;
                }
                hs[col] += h + 2.0 * pad + gap;
            }
            return hs[0].max(hs[1]) - gap;
        }
        let mut y = c.y - self.scroll;
        for w in widgets {
            y += self.draw_widget(g, w, c.x, y, c.w) + GAP;
        }
        y + self.scroll - c.y - GAP
    }

    fn label(&mut self, g: &mut Gfx, s: &str, x: f32, y: f32, w: f32) {
        g.text(s, Rect::new(x, y, w, 20.0), ts(13.0), self.theme.dim);
    }

    fn draw_widget(&mut self, g: &mut Gfx, w: Widget, x: f32, y: f32, width: f32) -> f32 {
        let t = self.theme;
        match w {
            Widget::Clock => {
                let now = sysinfo::local_time();
                let (hm, ampm) = sysinfo::format_time(&now, self.cfg.clock_24h);
                let mut tail = String::new();
                if self.cfg.show_seconds {
                    tail = format!("{:02}", now.wSecond);
                }
                if !ampm.is_empty() {
                    if !tail.is_empty() {
                        tail.push(' ');
                    }
                    tail.push_str(ampm);
                }
                let big = ts(46.0).semi();
                let small = ts(18.0).semi();
                let (bw, _) = g.measure(&hm, big, 400.0);
                let (sw, _) = g.measure(&tail, small, 200.0);
                let sx = x + (width - (bw + 8.0 + sw)) / 2.0;
                g.text(&hm, Rect::new(sx, y, bw + 4.0, 56.0), big, t.fg);
                g.text(&tail, Rect::new(sx + bw + 8.0, y + 20.0, sw + 4.0, 28.0), small, t.accent);
                let date = format!(
                    "{} {} {} {}",
                    sysinfo::DAYS[(now.wDayOfWeek as usize) % 7],
                    now.wDay,
                    &sysinfo::MONTHS[(now.wMonth as usize).clamp(1, 12) - 1][..3],
                    now.wYear
                );
                g.text(&date, Rect::new(x, y + 56.0, width, 20.0), ts(13.5).center(), t.dim);
                78.0
            }
            Widget::Gauges => {
                let s = self.stats;
                let pct = |v: f32| format!("{:.0}%", v * 100.0);
                // (label, fraction of the ring, colour, text in the middle)
                let items: Vec<(&str, Option<f32>, Color, String)> = self
                    .cfg
                    .gauges
                    .iter()
                    .filter_map(|k| match k.as_str() {
                        "cpu" => Some(("CPU", Some(s.cpu), t.accent, pct(s.cpu))),
                        "mem" => Some(("MEM", Some(s.mem), Color::rgb(0x4fd1c5), pct(s.mem))),
                        "gpu" => Some(("GPU", s.gpu, t.accent, s.gpu.map(pct).unwrap_or_else(|| "–".into()))),
                        "disk" => Some(("C:", Some(s.disk), Color::rgb(0x9ad17a), pct(s.disk))),
                        "net" => Some((
                            "NET ↓",
                            s.net_down.map(|_| s.net_frac),
                            Color::rgb(0xa78bfa),
                            s.net_down.map(sysinfo::fmt_rate).unwrap_or_else(|| "–".into()),
                        )),
                        _ => None,
                    })
                    .collect();
                if items.is_empty() {
                    return 0.0;
                }
                let cell = width / items.len() as f32;
                for (i, (label, v, color, txt)) in items.into_iter().enumerate() {
                    let cx = x + cell * (i as f32 + 0.5);
                    let cy = y + 32.0;
                    g.stroke_circle(cx, cy, 28.0, t.faint, 4.0);
                    if let Some(v) = v {
                        let hot = v > 0.9 && label != "NET ↓";
                        g.arc(cx, cy, 28.0, v, if hot { Color::rgb(0xff7a59) } else { color }, 4.0);
                    }
                    g.text(&txt, Rect::new(cx - 28.0, cy - 14.0, 56.0, 28.0), ts(15.5).semi().center(), t.fg);
                    g.text(label, Rect::new(cx - 40.0, y + 66.0, 80.0, 18.0), ts(11.0).bold().center(), t.dim);
                }
                86.0
            }
            Widget::WorldClocks => {
                let clocks = self.cfg.world_clocks.clone();
                if clocks.is_empty() {
                    return 0.0;
                }
                let cell = width / clocks.len() as f32;
                let r = (cell / 2.0 - 12.0).min(36.0);
                for (i, c) in clocks.iter().enumerate() {
                    let tz = *self.tz.entry(c.tz.clone()).or_insert_with(|| sysinfo::find_tz(&c.tz));
                    let now = sysinfo::time_in(&tz);
                    let cx = x + cell * (i as f32 + 0.5);
                    let cy = y + r + 2.0;
                    g.fill_circle(cx, cy, r, t.card.a(0.05));
                    g.stroke_circle(cx, cy, r, t.dim.a(0.6), 1.5);
                    for k in 0..12 {
                        let a = k as f32 / 12.0 * std::f32::consts::TAU;
                        let (s, co) = (a.sin(), a.cos());
                        let inner = if k % 3 == 0 { r - 7.0 } else { r - 4.0 };
                        g.line(cx + s * inner, cy - co * inner, cx + s * (r - 1.5), cy - co * (r - 1.5), t.dim, 1.2);
                    }
                    let hand = |g: &mut Gfx, frac: f32, len: f32, wd: f32, col: Color| {
                        let a = frac * std::f32::consts::TAU;
                        g.line(cx, cy, cx + a.sin() * len, cy - a.cos() * len, col, wd);
                    };
                    let m = now.wMinute as f32 + now.wSecond as f32 / 60.0;
                    hand(g, ((now.wHour % 12) as f32 + m / 60.0) / 12.0, r * 0.5, 2.6, t.fg);
                    hand(g, m / 60.0, r * 0.76, 2.0, t.fg);
                    if self.cfg.show_seconds {
                        hand(g, now.wSecond as f32 / 60.0, r * 0.8, 1.0, t.accent);
                    }
                    g.fill_circle(cx, cy, 2.4, t.fg);
                    g.text(&c.label, Rect::new(cx - cell / 2.0, cy + r + 6.0, cell, 20.0), ts(13.5).bold().center(), t.fg);
                    let (hm, ampm) = sysinfo::format_time(&now, self.cfg.clock_24h);
                    g.text(
                        &format!("{hm} {ampm}"),
                        Rect::new(cx - cell / 2.0, cy + r + 25.0, cell, 18.0),
                        ts(12.0).mono().center(),
                        t.dim,
                    );
                }
                // Click the clocks to choose other cities.
                self.hit(Rect::new(x, y, width, 2.0 * r + 48.0), Act::EditClocks);
                2.0 * r + 48.0
            }
            Widget::Calendar => {
                let now = sysinfo::local_time();
                let (yr, mo) = (now.wYear as i64, (now.wMonth as i64).clamp(1, 12));
                g.text(
                    &format!("{} {}", sysinfo::MONTHS[mo as usize - 1], yr),
                    Rect::new(x, y, width, 22.0),
                    ts(15.0).bold(),
                    t.fg,
                );
                let cw = width / 7.0;
                for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
                    g.text(d, Rect::new(x + i as f32 * cw, y + 28.0, cw, 18.0), ts(11.0).bold().center(), t.dim);
                }
                let first = sysinfo::weekday(yr, mo, 1);
                let days = sysinfo::days_in_month(yr, mo);
                for d in 1..=days {
                    let idx = first + d - 1;
                    let r = Rect::new(x + (idx % 7) as f32 * cw, y + 50.0 + (idx / 7) as f32 * 27.0, cw, 27.0);
                    if d == now.wDay as i64 {
                        g.fill_circle(r.x + cw / 2.0, r.y + 13.5, 12.5, t.accent);
                        g.text(&d.to_string(), r, ts(13.0).bold().center(), Color::rgb(0xffffff));
                    } else {
                        g.text(&d.to_string(), r, ts(13.0).center(), t.fg);
                    }
                }
                50.0 + ((first + days + 6) / 7) as f32 * 27.0
            }
            Widget::News => {
                let title = self.cfg.news_title.clone();
                self.label(g, &title, x, y, width);
                let items: Vec<NewsItem> = self.news.iter().take(3).cloned().collect();
                if items.is_empty() {
                    let msg = if self.news_loading { "Loading headlines…" } else { "No headlines available" };
                    g.text(msg, Rect::new(x, y + 26.0, width, 22.0), ts(13.5), t.dim.a(0.8));
                    return 52.0;
                }
                let mut yy = y + 24.0;
                for it in items {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 46.0);
                    let hot = if it.link.starts_with("http") { self.hit(r, Act::Open(it.link.clone())) } else { false };
                    if hot {
                        g.fill_rr(r, 8.0, t.card.a(0.08));
                    }
                    g.fill_circle(x + 3.0, yy + 15.0, 2.6, t.accent);
                    let age = news::age(it.ts);
                    g.text(&it.title, Rect::new(x + 14.0, yy + 3.0, width - 14.0 - 40.0, 22.0), ts(15.0).semi(), t.fg);
                    g.text(&age, Rect::new(x + width - 38.0, yy + 3.0, 38.0, 22.0), ts(12.0).right(), t.dim);
                    g.text(&it.summary, Rect::new(x + 14.0, yy + 24.0, width - 14.0, 18.0), ts(12.5), t.dim);
                    yy += 48.0;
                }
                yy - y - 2.0
            }
            Widget::Pinned => {
                let pins = self.cfg.pinned.clone();
                if pins.is_empty() {
                    g.text(
                        "Right-click an app in All apps or search to pin it here.",
                        Rect::new(x, y, width, 24.0),
                        ts(13.5).center(),
                        t.dim,
                    );
                    return 24.0;
                }
                let cols = if width < 360.0 { 3usize } else { 4 };
                let icon = self.cfg.icon_size;
                let cw = width / cols as f32;
                let ch = icon + 46.0;
                for (i, id) in pins.iter().enumerate() {
                    let name = self.apps.iter().find(|a| a.id == *id).map(|a| a.name.clone()).unwrap_or_default();
                    let r = Rect::new(x + (i % cols) as f32 * cw, y + (i / cols) as f32 * ch, cw, ch - 6.0);
                    if self.hit(r.inset(2.0), Act::App(id.clone())) {
                        g.fill_rr(r.inset(2.0), 8.0, t.card.a(0.10));
                    }
                    self.draw_icon(g, id, Rect::new(r.x + (cw - icon) / 2.0, r.y + 9.0, icon, icon));
                    g.text(&name, Rect::new(r.x + 6.0, r.y + icon + 13.0, cw - 12.0, 20.0), ts(13.0).center(), t.fg);
                }
                pins.len().div_ceil(cols) as f32 * ch - 6.0
            }
            Widget::Notes => {
                self.label(g, "Notes", x, y, width);
                let st = ts(14.5);
                let (_, th) = g.measure(&self.notes.text.clone(), st.wrap(), width - 28.0);
                let bh = (th + 28.0).max(104.0);
                let r = Rect::new(x, y + 24.0, width, bh);
                g.fill_rr(r, 8.0, t.card.a(0.07));
                if self.focus == Focus::Notes {
                    g.stroke_rr(r, 8.0, t.accent.a(0.7), 1.2);
                }
                self.hit(r, Act::FocusNotes);
                self.draw_field(g, Focus::Notes, Rect::new(x + 14.0, r.y + 12.0, width - 28.0, bh - 20.0), st, "Type a note…");
                24.0 + bh
            }
            Widget::Calculator => {
                self.label(g, "Calculator", x, y, width);
                let r = Rect::new(x, y + 24.0, width, 42.0);
                g.fill_rr(r, 8.0, t.card.a(0.07));
                if self.focus == Focus::Calc {
                    g.stroke_rr(r, 8.0, t.accent.a(0.7), 1.2);
                }
                self.hit(r, Act::FocusCalc);
                self.draw_field(g, Focus::Calc, Rect::new(x + 14.0, r.y, width - 28.0, r.h), ts(15.0).mono(), "e.g. (1200 + 350) * 15%");
                let out = Rect::new(x, r.bottom() + 6.0, width - 4.0, 34.0);
                if !self.calc.is_empty() {
                    match calc::eval(&self.calc.text) {
                        Some(v) => g.text(&format!("= {}", calc::fmt(v)), out, ts(24.0).semi().right(), t.accent),
                        None => g.text("…", out, ts(20.0).right(), t.dim),
                    }
                }
                24.0 + 42.0 + 40.0
            }
            Widget::Recent => {
                self.label(g, "Recent files", x, y, width);
                let items = self.recents.clone();
                if items.is_empty() {
                    g.text("No recent files", Rect::new(x, y + 26.0, width, 22.0), ts(13.5), t.dim.a(0.8));
                    return 52.0;
                }
                let mut yy = y + 24.0;
                for f in items {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 36.0);
                    if self.hit(r, Act::Open(f.path.clone())) {
                        g.fill_rr(r, 8.0, t.card.a(0.08));
                    }
                    self.draw_icon(g, &f.path, Rect::new(x, yy + 6.0, 24.0, 24.0));
                    g.text(&f.name, Rect::new(x + 36.0, yy, width - 40.0, 36.0), ts(14.0), t.fg);
                    yy += 38.0;
                }
                yy - y - 2.0
            }
            Widget::Groups => {
                let groups = self.groups.clone();
                if groups.is_empty() {
                    g.text("App groups appear here once your apps are indexed.", Rect::new(x, y, width, 24.0), ts(13.5), t.dim.a(0.8));
                    return 24.0;
                }
                let sel = self.group_tab.min(groups.len() - 1);
                let (mut tx, mut ty) = (x, y);
                for (i, gr) in groups.iter().enumerate() {
                    let st = ts(13.0).semi();
                    let (tw, _) = g.measure(&gr.name, st, 300.0);
                    if tx + tw + 22.0 > x + width && tx > x {
                        tx = x;
                        ty += 32.0;
                    }
                    let r = Rect::new(tx, ty, tw + 22.0, 28.0);
                    let hot = self.hit(r, Act::GroupTab(i));
                    if i == sel {
                        g.fill_rr(r, 6.0, t.card.a(0.16));
                        g.fill_rr(Rect::new(r.x + 9.0, r.bottom() - 3.0, r.w - 18.0, 2.0), 1.0, t.accent);
                    } else if hot {
                        g.fill_rr(r, 6.0, t.card.a(0.08));
                    }
                    g.text(&gr.name, r, st.center(), if i == sel { t.fg } else { t.dim });
                    tx = r.right() + 4.0;
                }
                if self.cfg.workspaces.len() < 8 {
                    // "+" makes a new group of your own.
                    if tx + 30.0 > x + width && tx > x {
                        tx = x;
                        ty += 32.0;
                    }
                    let r = Rect::new(tx, ty, 30.0, 28.0);
                    let hot = self.hit(r, Act::GroupNew);
                    g.fill_rr(r, 6.0, t.card.a(if hot { 0.16 } else { 0.07 }));
                    g.text(G_ADD, r, ts(11.0).icon().center(), if hot { t.fg } else { t.dim });
                }
                let by = ty + 34.0;
                let ids = groups[sel].apps.clone();
                let cols = if width < 360.0 { 3usize } else { 4 };
                let icon = self.cfg.icon_size;
                let cw = (width - 16.0) / cols as f32;
                let ch = icon + 44.0;
                let rows = ids.len().div_ceil(cols).max(1);
                let body = Rect::new(x, by, width, rows as f32 * ch + 12.0);
                g.fill_rr(body, 8.0, t.card.a(0.06));
                if ids.is_empty() {
                    g.text("Empty group. Use Choose apps to fill it.", body, ts(13.5).center(), t.dim.a(0.8));
                }
                for (i, id) in ids.iter().enumerate() {
                    let name = self.apps.iter().find(|a| a.id == *id).map(|a| a.name.clone()).unwrap_or_default();
                    let r = Rect::new(x + 8.0 + (i % cols) as f32 * cw, by + 6.0 + (i / cols) as f32 * ch, cw, ch - 4.0);
                    if self.hit(r.inset(2.0), Act::App(id.clone())) {
                        g.fill_rr(r.inset(2.0), 8.0, t.card.a(0.10));
                    }
                    self.draw_icon(g, id, Rect::new(r.x + (cw - icon) / 2.0, r.y + 8.0, icon, icon));
                    g.text(&name, Rect::new(r.x + 4.0, r.y + icon + 11.0, cw - 8.0, 20.0), ts(12.5).center(), t.fg);
                }
                // Your own groups can be edited right here.
                match groups[sel].ws {
                    Some(w) => {
                        self.pills(g, x, body.bottom() + 6.0, vec![
                            ("Choose apps", Act::GroupEdit(w), false),
                            ("Rename", Act::GroupRename(w), false),
                            ("Open all", Act::Workspace(w), false),
                        ]);
                        body.bottom() + 42.0 - y
                    }
                    None => body.bottom() - y,
                }
            }
            Widget::Folder => {
                let hr = Rect::new(x - 6.0, y - 2.0, 140.0, 24.0);
                let hot = self.hit(hr, Act::Open("shell:Downloads".into()));
                g.text(G_DOWNLOAD, Rect::new(x, y, 18.0, 20.0), ts(12.0).icon().center(), t.accent);
                g.text("Downloads", Rect::new(x + 24.0, y, 110.0, 20.0), ts(13.0).semi(), if hot { t.fg } else { t.dim });
                let items = self.downloads.clone();
                if items.is_empty() {
                    g.text("Nothing downloaded yet", Rect::new(x, y + 26.0, width, 22.0), ts(13.5), t.dim.a(0.8));
                    return 52.0;
                }
                let cols = 3usize;
                let icon = (self.cfg.icon_size * 0.85).round();
                let cw = (width - 16.0) / cols as f32;
                let ch = icon + 42.0;
                let body = Rect::new(x, y + 28.0, width, items.len().div_ceil(cols) as f32 * ch + 12.0);
                g.fill_rr(body, 8.0, t.card.a(0.06));
                for (i, f) in items.iter().enumerate() {
                    let r = Rect::new(x + 8.0 + (i % cols) as f32 * cw, body.y + 6.0 + (i / cols) as f32 * ch, cw, ch - 4.0);
                    if self.hit(r.inset(2.0), Act::Open(f.path.clone())) {
                        g.fill_rr(r.inset(2.0), 8.0, t.card.a(0.10));
                    }
                    self.draw_icon(g, &f.path, Rect::new(r.x + (cw - icon) / 2.0, r.y + 8.0, icon, icon));
                    g.text(&f.name, Rect::new(r.x + 6.0, r.y + icon + 11.0, cw - 12.0, 20.0), ts(12.5).center(), t.fg);
                }
                body.bottom() - y
            }
            Widget::Drives => {
                let list: Vec<(String, u64, u64)> = self.drives.iter().map(|d| (d.root.clone(), d.total, d.free)).collect();
                if list.is_empty() {
                    return 0.0;
                }
                let mut yy = y;
                for (root, total, free) in list {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 28.0);
                    let hot = self.hit(r, Act::Open(root.clone()));
                    if hot {
                        g.fill_rr(r, 6.0, t.card.a(0.08));
                    }
                    let used = 1.0 - free as f32 / total as f32;
                    g.text(&root[..2], Rect::new(x, yy, 30.0, 28.0), ts(13.0).bold(), t.fg);
                    let tw = 152.0;
                    let bar = Rect::new(x + 34.0, yy + 11.5, width - 34.0 - tw - 8.0, 5.0);
                    g.fill_rr(bar, 2.5, t.faint);
                    let col = if used > 0.9 {
                        Color::rgb(0xff7a59)
                    } else if used > 0.75 {
                        Color::rgb(0xf2c14e)
                    } else {
                        Color::rgb(0x4fa8d8)
                    };
                    g.fill_rr(Rect::new(bar.x, bar.y, bar.w * used, bar.h), 2.5, col);
                    // Hovering shows the detail; clicking opens the drive.
                    let txt = if hot {
                        format!("{} of {} used", sysinfo::fmt_size(total - free), sysinfo::fmt_size(total))
                    } else {
                        format!("{} free", sysinfo::fmt_size(free))
                    };
                    g.text(&txt, Rect::new(x + width - tw, yy, tw, 28.0), ts(12.5).right(), t.dim);
                    yy += 30.0;
                }
                yy - y - 2.0
            }
            Widget::Run => {
                g.text("RUN", Rect::new(x, y, width, 18.0), ts(11.5).semi(), t.dim);
                let r = Rect::new(x, y + 22.0, width, 38.0);
                g.fill_rr(r, 6.0, t.card.a(0.07));
                g.stroke_rr(r, 6.0, if self.focus == Focus::Run { t.accent.a(0.7) } else { t.card.a(0.14) }, 1.0);
                self.hit(r, Act::FocusRun);
                g.text(">", Rect::new(x + 10.0, r.y, 14.0, r.h), ts(14.0).mono(), t.accent);
                self.draw_field(g, Focus::Run, Rect::new(x + 30.0, r.y, width - 42.0, r.h), ts(14.0).mono(), "Type a command…");
                g.text("Enter to run   ·   ↑↓ history", Rect::new(x, r.bottom() + 6.0, width, 18.0), ts(11.5), t.dim.a(0.8));
                22.0 + 38.0 + 26.0
            }
            Widget::Workspaces => {
                self.label(g, "Workspaces", x, y, width);
                let list = self.cfg.workspaces.clone();
                if list.is_empty() {
                    g.text(
                        "Your own groups appear here. Create one with + in the App groups widget.",
                        Rect::new(x, y + 26.0, width, 40.0),
                        ts(13.0).wrap(),
                        t.dim.a(0.8),
                    );
                    return 68.0;
                }
                let mut yy = y + 24.0;
                for (i, ws) in list.iter().enumerate() {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 44.0);
                    let hot = self.hit(r, Act::Workspace(i));
                    g.fill_rr(r, 8.0, t.card.a(if hot { 0.13 } else { 0.06 }));
                    let icons = ws.apps.len().min(((width * 0.5) / 30.0) as usize);
                    g.text(&ws.name, Rect::new(x + 6.0, yy, width - 12.0 - icons as f32 * 30.0, 44.0), ts(14.0).semi(), t.fg);
                    for (k, id) in ws.apps.iter().take(icons).enumerate() {
                        self.draw_icon(g, id, Rect::new(x + width - 26.0 - k as f32 * 30.0, yy + 10.0, 24.0, 24.0));
                    }
                    yy += 48.0;
                }
                yy - y - 4.0
            }
            Widget::OpenWindows => {
                self.label(g, "Open windows", x, y, width);
                let items: Vec<Win> = self.wins.iter().take(8).cloned().collect();
                if items.is_empty() {
                    g.text("No open windows", Rect::new(x, y + 26.0, width, 22.0), ts(13.5), t.dim.a(0.8));
                    return 52.0;
                }
                let mut yy = y + 24.0;
                for w in items {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 36.0);
                    if self.hit(r, Act::Window(w.hwnd)) {
                        g.fill_rr(r, 8.0, t.card.a(0.08));
                    }
                    self.draw_icon(g, &w.exe, Rect::new(x, yy + 6.0, 24.0, 24.0));
                    g.text(&w.title, Rect::new(x + 36.0, yy, width - 40.0, 36.0), ts(14.0), t.fg);
                    yy += 38.0;
                }
                yy - y - 2.0
            }
            Widget::Closed => {
                self.label(g, "Recently closed", x, y, width);
                let items = self.closed.clone();
                if items.is_empty() {
                    g.text("Apps you close will appear here", Rect::new(x, y + 26.0, width, 22.0), ts(13.5), t.dim.a(0.8));
                    return 52.0;
                }
                let mut yy = y + 24.0;
                for c in items {
                    let r = Rect::new(x - 8.0, yy, width + 16.0, 36.0);
                    if self.hit(r, Act::Open(c.exe.clone())) {
                        g.fill_rr(r, 8.0, t.card.a(0.08));
                    }
                    self.draw_icon(g, &c.exe, Rect::new(x, yy + 6.0, 24.0, 24.0));
                    g.text(&c.name, Rect::new(x + 36.0, yy, width - 40.0, 36.0), ts(14.0), t.fg);
                    yy += 38.0;
                }
                yy - y - 2.0
            }
        }
    }

    fn draw_results(&mut self, g: &mut Gfx, c: Rect) -> f32 {
        let t = self.theme;
        let items = self.results.clone();
        self.result_y.clear();
        let mut y = 0.0f32;
        let mut last = "";
        for (i, it) in items.iter().enumerate() {
            let best = i == 0;
            let section = if best { "Best match" } else { it.section };
            if section != last {
                last = section;
                let hy = c.y + y - self.scroll;
                if hy + 26.0 > c.y && hy < c.bottom() {
                    g.text(section, Rect::new(c.x + 4.0, hy + 2.0, c.w, 22.0), ts(12.5).semi(), t.dim);
                }
                y += 28.0;
            }
            let h = if best { 68.0 } else if it.sub.is_empty() { 42.0 } else { 50.0 };
            self.result_y.push(y - 28.0);
            let yy = c.y + y - self.scroll;
            y += h + 4.0;
            if yy + h < c.y || yy > c.bottom() {
                continue;
            }
            let r = Rect::new(c.x, yy, c.w, h);
            let hot = self.hit(r, it.act.clone());
            let selected = i == self.sel;
            if selected {
                g.fill_rr(r, 8.0, t.card.a(0.14));
                g.fill_rr(Rect::new(r.x, r.y + h * 0.28, 3.0, h * 0.44), 1.5, t.accent);
            } else if best {
                g.fill_rr(r, 8.0, t.card.a(0.07));
            } else if hot {
                g.fill_rr(r, 8.0, t.card.a(0.07));
            }
            let isz = if best { 40.0 } else { 26.0 };
            let ir = Rect::new(r.x + 16.0, r.y + (h - isz) / 2.0, isz, isz);
            match &it.icon {
                Some(key) => self.draw_icon(g, key, ir),
                None => g.text(it.glyph, ir, ts(isz * 0.62).icon().center(), t.fg),
            }
            let tx = ir.right() + 16.0;
            let tw = r.right() - tx - 12.0;
            let sub = if best && matches!(it.act, Act::App(_)) {
                "App   ·   Enter to open   ·   Ctrl+Enter to run as administrator".to_string()
            } else {
                it.sub.clone()
            };
            let size = if best { 17.0 } else { 14.5 };
            if sub.is_empty() {
                g.text_hl(&it.title, Rect::new(tx, r.y, tw, h), ts(size), t.fg, it.hl, t.accent);
            } else {
                let top = if best { 12.0 } else { 5.0 };
                g.text_hl(&it.title, Rect::new(tx, r.y + top, tw, 24.0), ts(size), t.fg, it.hl, t.accent);
                g.text(&sub, Rect::new(tx, r.y + top + 24.0, tw, 18.0), ts(12.0), t.dim);
            }
        }
        y
    }

    /// Cities whose name or country contains the search text, as indexes into CITIES.
    fn city_matches(&self) -> Vec<usize> {
        let q = self.search.text.trim().to_lowercase();
        CITIES
            .iter()
            .enumerate()
            .filter(|(_, (city, country, _))| {
                q.is_empty() || city.to_lowercase().contains(&q) || country.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Time zone for a Windows zone id, looked up once and cached.
    fn zone(&mut self, id: &str) -> Option<DYNAMIC_TIME_ZONE_INFORMATION> {
        *self.tz.entry(id.to_string()).or_insert_with(|| sysinfo::find_tz(id))
    }

    /// World clocks editor: the chosen cities on top, all cities below.
    fn draw_clock_editor(&mut self, g: &mut Gfx, c: Rect) -> f32 {
        let t = self.theme;
        let clocks = self.cfg.world_clocks.clone();
        let n = clocks.len();
        let head = self.back_header(g, c, "World clocks", &format!("{n} of {MAX_CLOCKS}"));
        let mut y = c.y + head - self.scroll;
        let time_txt = |app: &mut App, id: &str| {
            let tz = app.zone(id);
            let now = sysinfo::time_in(&tz);
            let (hm, ampm) = sysinfo::format_time(&now, app.cfg.clock_24h);
            (format!("{hm} {ampm}").trim().to_string(), sysinfo::fmt_offset(sysinfo::utc_offset_min(&tz)))
        };
        if clocks.is_empty() {
            g.text("No clocks yet. Pick a city below.", Rect::new(c.x, y, c.w, 36.0), ts(13.5), t.dim);
            y += 40.0;
        }
        for (i, clock) in clocks.iter().enumerate() {
            let r = Rect::new(c.x, y, c.w - 6.0, 44.0);
            g.fill_rr(r, 8.0, t.card.a(0.07));
            let (time, off) = time_txt(self, &clock.tz);
            g.text(&clock.label, Rect::new(r.x + 14.0, r.y, r.w * 0.38, r.h), ts(14.5).semi(), t.fg);
            g.text(&time, Rect::new(r.x + r.w * 0.40, r.y, 110.0, r.h), ts(13.5).mono(), t.fg);
            g.text(&off, Rect::new(r.x + r.w * 0.40 + 112.0, r.y, 100.0, r.h), ts(12.5), t.dim);
            self.mini(g, r.right() - 104.0, r.y + 7.0, G_UP, Act::ClockUp(i), i > 0);
            self.mini(g, r.right() - 72.0, r.y + 7.0, G_DOWN, Act::ClockDown(i), i + 1 < n);
            self.mini(g, r.right() - 40.0, r.y + 7.0, G_CLOSE, Act::ClockDel(i), true);
            y += 48.0;
        }
        y += 14.0;
        g.text("Add a city", Rect::new(c.x, y, 200.0, 26.0), ts(15.0).semi(), t.fg);
        let hint = if n >= MAX_CLOCKS {
            format!("Remove a clock to add another (up to {MAX_CLOCKS})")
        } else {
            "Type in the search box to filter, Enter adds the first match".to_string()
        };
        g.text(&hint, Rect::new(c.x + 110.0, y, c.w - 116.0, 26.0), ts(12.5).right(), t.dim);
        y += 32.0;
        let full = n >= MAX_CLOCKS;
        let list = self.city_matches();
        if list.is_empty() {
            g.text("No city matches. Try the country name.", Rect::new(c.x, y, c.w, 30.0), ts(13.5), t.dim);
            y += 34.0;
        }
        for i in list {
            if y + 40.0 >= c.y && y <= c.bottom() {
                let (city, country, tz) = CITIES[i];
                let added = clocks.iter().any(|k| k.label == city && k.tz == tz);
                let r = Rect::new(c.x, y, c.w - 6.0, 40.0);
                let hot = !full && !added && self.hit(r, Act::ClockAdd(i));
                if hot {
                    g.fill_rr(r, 8.0, t.card.a(0.10));
                }
                let (time, off) = time_txt(self, tz);
                let col = if added || full { t.dim } else { t.fg };
                g.text(city, Rect::new(r.x + 14.0, r.y, r.w * 0.30, r.h), ts(14.0), col);
                g.text(country, Rect::new(r.x + r.w * 0.30, r.y, r.w * 0.30, r.h), ts(12.5), t.dim);
                g.text(&time, Rect::new(r.x + r.w * 0.55, r.y, 110.0, r.h), ts(13.0).mono(), col);
                g.text(&off, Rect::new(r.right() - 120.0, r.y, 76.0, r.h), ts(12.0), t.dim);
                let mark = if added { "\u{E73E}" } else { G_ADD };
                if added || hot {
                    g.text(mark, Rect::new(r.right() - 40.0, r.y, 30.0, r.h), ts(12.0).icon().center(), if added { t.accent } else { t.fg });
                }
            }
            y += 42.0;
        }
        y + self.scroll - c.y
    }

    /// App chooser for a custom group: tick apps in or out, rename, delete.
    fn draw_picker(&mut self, g: &mut Gfx, c: Rect, wi: usize) -> f32 {
        let t = self.theme;
        let Some(ws) = self.cfg.workspaces.get(wi).cloned() else {
            self.view = View::Home;
            return 0.0;
        };
        let y = c.y - self.scroll;
        let b = Rect::new(c.x - 6.0, y, 36.0, 36.0);
        if self.hit(b, Act::Home) {
            g.fill_circle(b.x + 18.0, b.y + 18.0, 17.0, t.card.a(0.14));
        }
        g.text(G_BACK, b, ts(14.0).icon().center(), t.fg);
        let nb = Rect::new(c.x + 40.0, y + 1.0, (c.w - 190.0).min(230.0), 34.0);
        g.fill_rr(nb, 8.0, t.card.a(0.07));
        g.stroke_rr(nb, 8.0, if self.focus == Focus::GroupName { t.accent.a(0.8) } else { t.card.a(0.14) }, 1.0);
        self.hit(nb, Act::FocusGroupName);
        self.draw_field(g, Focus::GroupName, Rect::new(nb.x + 12.0, nb.y, nb.w - 24.0, nb.h), ts(15.0).semi(), "Group name");
        self.pills(g, c.right() - 116.0, y, vec![("Delete group", Act::GroupDelete(wi), false)]);
        g.text(
            &format!("{} chosen   ·   tick apps to add them   ·   type above to filter", ws.apps.len()),
            Rect::new(c.x, y + 42.0, c.w, 20.0),
            ts(12.5),
            t.dim,
        );
        let head = 70.0;
        let q = self.search.text.trim().to_lowercase();
        let list: Vec<(String, String)> = self
            .apps
            .iter()
            .filter(|a| q.is_empty() || a.lname.contains(&q))
            .map(|a| (a.name.clone(), a.id.clone()))
            .collect();
        for (i, (name, id)) in list.iter().enumerate() {
            let yy = c.y + head + i as f32 * 42.0 - self.scroll;
            if yy + 42.0 < c.y || yy > c.bottom() {
                continue;
            }
            let on = ws.apps.iter().any(|a| a == id);
            let r = Rect::new(c.x, yy, c.w - 6.0, 40.0);
            let hot = self.hit(r, Act::PickToggle(id.clone()));
            if hot || on {
                g.fill_rr(r, 8.0, t.card.a(if hot { 0.10 } else { 0.05 }));
            }
            self.draw_icon(g, id, Rect::new(r.x + 10.0, r.y + 6.0, 28.0, 28.0));
            g.text(name, Rect::new(r.x + 52.0, r.y, r.w - 100.0, r.h), ts(14.5), t.fg);
            let cb = Rect::new(r.right() - 34.0, r.y + 10.0, 20.0, 20.0);
            if on {
                g.fill_rr(cb, 5.0, t.accent);
                g.text("\u{E73E}", cb, ts(11.0).icon().center(), Color::rgb(0xffffff));
            } else {
                g.stroke_rr(cb, 5.0, t.dim, 1.2);
            }
        }
        if list.is_empty() {
            g.text("No apps match", Rect::new(c.x, c.y + head, c.w, 30.0), ts(13.5).center(), t.dim);
        }
        head + list.len() as f32 * 42.0
    }

    fn back_header(&mut self, g: &mut Gfx, c: Rect, title: &str, note: &str) -> f32 {
        let t = self.theme;
        let y = c.y - self.scroll;
        let b = Rect::new(c.x - 6.0, y, 36.0, 36.0);
        if self.hit(b, Act::Home) {
            g.fill_circle(b.x + 18.0, b.y + 18.0, 17.0, t.card.a(0.14));
        }
        g.text(G_BACK, b, ts(14.0).icon().center(), t.fg);
        g.text(title, Rect::new(c.x + 40.0, y, 200.0, 36.0), ts(17.0).semi(), t.fg);
        g.text(note, Rect::new(c.x + 240.0, y, c.w - 244.0, 36.0), ts(12.5).right(), t.dim);
        44.0
    }

    fn draw_all_apps(&mut self, g: &mut Gfx, c: Rect) -> f32 {
        let note = if self.apps.is_empty() { "Indexing…".to_string() } else { format!("{} apps", self.apps.len()) };
        let head = self.back_header(g, c, "All apps", &note);
        self.app_rows(g, c, self.scroll, head)
    }

    /// Alphabetical app list with letter headings. Returns the total height.
    fn app_rows(&mut self, g: &mut Gfx, c: Rect, scroll: f32, head: f32) -> f32 {
        let t = self.theme;
        // (y, letter header, app index)
        let mut rows: Vec<(f32, Option<char>, usize)> = Vec::new();
        let mut y = head;
        let mut last = '\0';
        for (i, a) in self.apps.iter().enumerate() {
            let ch = a.name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or('#');
            let ch = if ch.is_alphabetic() { ch } else { '#' };
            if ch != last {
                last = ch;
                rows.push((y, Some(ch), i));
                y += 30.0;
            }
            rows.push((y, None, i));
            y += 42.0;
        }
        for (ry, letter, i) in rows {
            let yy = c.y + ry - scroll;
            if yy + 42.0 < c.y || yy > c.bottom() {
                continue;
            }
            match letter {
                Some(ch) => g.text(&ch.to_string(), Rect::new(c.x + 6.0, yy, 60.0, 30.0), ts(13.0).bold(), t.accent),
                None => {
                    let (name, id) = (self.apps[i].name.clone(), self.apps[i].id.clone());
                    let r = Rect::new(c.x, yy, c.w - 6.0, 40.0);
                    if self.hit(r, Act::App(id.clone())) {
                        g.fill_rr(r, 8.0, t.card.a(0.09));
                    }
                    self.draw_icon(g, &id, Rect::new(r.x + 10.0, r.y + 6.0, 28.0, 28.0));
                    g.text(&name, Rect::new(r.x + 52.0, r.y, r.w - 60.0, r.h), ts(14.5), t.fg);
                }
            }
        }
        y
    }

    /// Row of pill buttons; returns the x after the last one.
    fn pills(&mut self, g: &mut Gfx, mut x: f32, y: f32, opts: Vec<(&str, Act, bool)>) -> f32 {
        let t = self.theme;
        for (label, act, on) in opts {
            let st = ts(13.0);
            let (tw, _) = g.measure(label, st, 300.0);
            let r = Rect::new(x, y + 3.0, tw + 26.0, 30.0);
            let hot = self.hit(r, act);
            if on {
                g.fill_rr(r, 15.0, t.accent);
                g.text(label, r, st.semi().center(), Color::rgb(0xffffff));
            } else {
                g.fill_rr(r, 15.0, t.card.a(if hot { 0.16 } else { 0.08 }));
                g.text(label, r, st.center(), t.fg);
            }
            x = r.right() + 6.0;
        }
        x
    }

    fn mini(&mut self, g: &mut Gfx, x: f32, y: f32, glyph: &str, act: Act, enabled: bool) {
        let t = self.theme;
        let r = Rect::new(x, y, 30.0, 30.0);
        if enabled && self.hit(r, act) {
            g.fill_circle(x + 15.0, y + 15.0, 14.0, t.card.a(0.14));
        }
        g.text(glyph, r, ts(11.5).icon().center(), if enabled { t.fg } else { t.dim.a(0.35) });
    }

    fn draw_settings(&mut self, g: &mut Gfx, c: Rect) -> f32 {
        let t = self.theme;
        let head = self.back_header(g, c, "Customize", "Changes apply instantly");
        let x = c.x;
        let cx = c.x + 118.0;
        let mut y = c.y + head - self.scroll + 4.0;
        let row = 42.0;
        let lab = |g: &mut Gfx, s: &str, y: f32| g.text(s, Rect::new(x, y, 116.0, 36.0), ts(13.5), t.dim);

        lab(g, "Style", y);
        let s = self.cfg.style;
        self.pills(g, cx, y, vec![
            ("Two pane", Act::SetStyle(Style::TwoPane), s == Style::TwoPane),
            ("Compact", Act::SetStyle(Style::Compact), s == Style::Compact),
            ("Dashboard", Act::SetStyle(Style::Dashboard), s == Style::Dashboard),
            ("Apps", Act::SetStyle(Style::Apps), s == Style::Apps),
        ]);
        y += row;

        lab(g, "Taskbar", y);
        let tb = self.cfg.taskbar_style;
        self.pills(g, cx, y, vec![
            ("Default", Act::SetTaskbar(TaskbarStyle::Default), tb == TaskbarStyle::Default),
            ("Rounded", Act::SetTaskbar(TaskbarStyle::Rounded), tb == TaskbarStyle::Rounded),
            ("Islands", Act::SetTaskbar(TaskbarStyle::Islands), tb == TaskbarStyle::Islands),
        ]);
        y += row;

        lab(g, "Position", y);
        let p = self.cfg.position;
        self.pills(g, cx, y, vec![
            ("Center", Act::SetPos(Position::BottomCenter), p == Position::BottomCenter),
            ("Left", Act::SetPos(Position::BottomLeft), p == Position::BottomLeft),
            ("Right", Act::SetPos(Position::BottomRight), p == Position::BottomRight),
            ("Middle", Act::SetPos(Position::Center), p == Position::Center),
        ]);
        y += row;

        lab(g, "Backdrop", y);
        let b = self.cfg.backdrop;
        self.pills(g, cx, y, vec![
            ("Acrylic", Act::SetBackdrop(Backdrop::Acrylic), b == Backdrop::Acrylic),
            ("Blur", Act::SetBackdrop(Backdrop::Blur), b == Backdrop::Blur),
            ("Mica", Act::SetBackdrop(Backdrop::Mica), b == Backdrop::Mica),
            ("Solid", Act::SetBackdrop(Backdrop::Solid), b == Backdrop::Solid),
        ]);
        y += row;

        lab(g, "Theme", y);
        let mut sx = cx + 14.0;
        let mut current = "";
        for (id, name, hex) in THEMES {
            let on = self.cfg.theme == id;
            if on {
                current = name;
            }
            let r = Rect::new(sx - 15.0, y + 3.0, 30.0, 30.0);
            let hot = self.hit(r, Act::SetTheme(id.to_string()));
            g.fill_circle(sx, y + 18.0, 11.0, Color::rgb(hex));
            g.stroke_circle(sx, y + 18.0, 11.0, t.card.a(0.35), 1.0);
            if on || hot {
                g.stroke_circle(sx, y + 18.0, 14.5, if on { t.accent } else { t.dim }, 2.0);
            }
            sx += 36.0;
        }
        g.text(current, Rect::new(sx, y, c.right() - sx, 36.0), ts(12.5), t.dim);
        y += row;

        lab(g, "Accent", y);
        let mut sx = cx + 14.0;
        for hex in ACCENTS {
            let id = format!("#{hex:06x}");
            let on = self.cfg.accent.eq_ignore_ascii_case(&id);
            let r = Rect::new(sx - 15.0, y + 3.0, 30.0, 30.0);
            let hot = self.hit(r, Act::SetAccent(id));
            g.fill_circle(sx, y + 18.0, 11.0, Color::rgb(hex));
            if on || hot {
                g.stroke_circle(sx, y + 18.0, 14.5, if on { t.fg } else { t.dim }, 2.0);
            }
            sx += 36.0;
        }
        let sys = self.cfg.accent == "system";
        self.pills(g, sx - 8.0, y, vec![("Windows", Act::SetAccent("system".into()), sys)]);
        y += row;

        let sliders = [
            ("Tint", SliderKind::Tint, self.cfg.tint_opacity, format!("{:.0}%", self.cfg.tint_opacity * 100.0)),
            ("Size", SliderKind::Scale, (self.cfg.scale - 0.7) / 0.9, format!("{:.0}%", self.cfg.scale * 100.0)),
        ];
        for (name, kind, v, txt) in sliders {
            lab(g, name, y);
            let track = Rect::new(cx + 8.0, y + 16.0, 230.0, 5.0);
            self.hit(Rect::new(track.x - 10.0, y + 2.0, track.w + 20.0, 32.0), Act::Slider(kind, track));
            g.fill_rr(track, 2.5, t.faint);
            g.fill_rr(Rect::new(track.x, track.y, track.w * v, track.h), 2.5, t.accent);
            g.fill_circle(track.x + track.w * v, track.y + 2.5, 8.0, t.fg);
            g.fill_circle(track.x + track.w * v, track.y + 2.5, 4.5, t.accent);
            g.text(&txt, Rect::new(track.right() + 16.0, y, 60.0, 36.0), ts(13.0), t.dim);
            y += row;
        }

        lab(g, "World clocks", y);
        let names: Vec<String> = self.cfg.world_clocks.iter().map(|c| c.label.clone()).collect();
        let nx = self.pills(g, cx, y, vec![("Change cities…", Act::EditClocks, false)]);
        g.text(&names.join(", "), Rect::new(nx + 6.0, y, c.right() - nx - 6.0, 36.0), ts(12.5), t.dim);
        y += row;

        lab(g, "Icon size", y);
        let ic = self.cfg.icon_size as u32;
        self.pills(g, cx, y, vec![
            ("Small", Act::SetIcon(32), ic == 32),
            ("Medium", Act::SetIcon(40), ic == 40),
            ("Large", Act::SetIcon(48), ic == 48),
        ]);
        y += row + 6.0;

        let toggles: [(&str, Tog, bool); 6] = [
            ("Show speed and folders on the taskbar", Tog::TaskbarInfo, self.cfg.taskbar_info),
            ("Open with the Windows key", Tog::WinKey, self.cfg.hook_win_key),
            ("Open from the taskbar Start button", Tog::StartButton, self.cfg.hook_start_button),
            ("Start Orbit Start when I sign in", Tog::Autostart, self.cfg.autostart),
            ("Show seconds on clocks", Tog::Seconds, self.cfg.show_seconds),
            ("24-hour clock", Tog::H24, self.cfg.clock_24h),
        ];
        for (name, tog, on) in toggles {
            let r = Rect::new(x - 8.0, y, c.w + 8.0, 36.0);
            let hot = self.hit(r, Act::Toggle(tog));
            if hot {
                g.fill_rr(r, 8.0, t.card.a(0.06));
            }
            g.text(name, Rect::new(x, y, c.w - 70.0, 36.0), ts(14.0), t.fg);
            let sw = Rect::new(c.right() - 52.0, y + 8.0, 40.0, 20.0);
            if on {
                g.fill_rr(sw, 10.0, t.accent);
                g.fill_circle(sw.right() - 10.0, sw.y + 10.0, 6.5, Color::rgb(0xffffff));
            } else {
                g.stroke_rr(sw, 10.0, t.dim, 1.2);
                g.fill_circle(sw.x + 10.0, sw.y + 10.0, 5.5, t.dim);
            }
            y += 38.0;
        }
        y += 10.0;

        g.text("Pages and widgets", Rect::new(x, y, c.w, 28.0), ts(15.0).semi(), t.fg);
        y += 34.0;
        let pages = self.cfg.pages.clone();
        let many = pages.len() > 1;
        for (pi, page) in pages.iter().enumerate() {
            g.text(&format!("Page {}", pi + 1), Rect::new(x, y, 120.0, 30.0), ts(13.0).bold(), t.accent);
            self.mini(g, c.right() - 40.0, y, G_CLOSE, Act::PageDel(pi), many);
            y += 32.0;
            let n = page.widgets.len();
            for (wi, w) in page.widgets.iter().enumerate() {
                let r = Rect::new(x, y, c.w - 8.0, 34.0);
                g.fill_rr(r, 8.0, t.card.a(0.06));
                g.text(w.label(), Rect::new(x + 14.0, y, c.w - 140.0, 34.0), ts(13.5), t.fg);
                self.mini(g, r.right() - 100.0, y + 2.0, G_UP, Act::WidgetUp(pi, wi), wi > 0);
                self.mini(g, r.right() - 68.0, y + 2.0, G_DOWN, Act::WidgetDown(pi, wi), wi + 1 < n);
                self.mini(g, r.right() - 36.0, y + 2.0, G_CLOSE, Act::WidgetDel(pi, wi), true);
                y += 38.0;
            }
            self.pills(g, x, y - 2.0, vec![("+ Add widget", Act::WidgetAdd(pi), false)]);
            y += 46.0;
        }
        if pages.len() < 8 {
            self.pills(g, x, y - 6.0, vec![("+ Add page", Act::PageAdd, false)]);
            y += 44.0;
        }

        g.text("Layout gallery", Rect::new(x, y, c.w, 28.0), ts(15.0).semi(), t.fg);
        y += 32.0;
        let names: Vec<&'static str> = presets().into_iter().map(|p| p.0).collect();
        let mut px = x;
        for (i, name) in names.into_iter().enumerate() {
            let (tw, _) = g.measure(name, ts(13.0), 300.0);
            if px + tw + 26.0 > c.right() - 6.0 {
                px = x;
                y += 38.0;
            }
            px = self.pills(g, px, y, vec![(name, Act::Preset(i), false)]);
        }
        y += 50.0;

        g.text("Share this layout", Rect::new(x, y, c.w, 28.0), ts(15.0).semi(), t.fg);
        y += 32.0;
        g.text(
            "Export saves the look, pages and widgets to a file you can send to someone. Notes and pinned apps are not included.",
            Rect::new(x, y, c.w - 10.0, 40.0),
            ts(12.5).wrap(),
            t.dim,
        );
        y += 44.0;
        self.pills(g, x, y, vec![
            ("Export…", Act::Export, false),
            ("Import…", Act::Import, false),
            ("Reset layout", Act::Reset, false),
            ("Open settings file", Act::OpenConfig, false),
        ]);
        y += 48.0;
        g.text(
            &format!("Orbit Start {}   ·   github.com/msh2050/orbit-start", env!("CARGO_PKG_VERSION")),
            Rect::new(x, y, c.w, 20.0),
            ts(12.0),
            t.dim.a(0.8),
        );
        y += 30.0;
        let _ = G_ADD;
        let _ = G_FOLDER;
        y + self.scroll - c.y
    }
}
