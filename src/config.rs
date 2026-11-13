//! Persistent settings, themes, and shareable layouts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::gfx::Color;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    TwoPane,
    Compact,
    Dashboard,
    /// Widgets on the left, the full app list always visible on the right.
    Apps,
}

/// How the Windows taskbar is shaped. Applied from outside Explorer by
/// clipping the taskbar window, so it is fully reversible.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TaskbarStyle {
    Default,
    Rounded,
    Islands,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    BottomCenter,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    Acrylic,
    Blur,
    Mica,
    Solid,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Widget {
    Clock,
    Gauges,
    WorldClocks,
    Calendar,
    News,
    Pinned,
    Notes,
    Calculator,
    Recent,
    Workspaces,
    OpenWindows,
    Closed,
    Groups,
    Drives,
    Run,
    Folder,
}

impl Widget {
    pub const ALL: [Widget; 16] = [
        Widget::Clock,
        Widget::Gauges,
        Widget::WorldClocks,
        Widget::Calendar,
        Widget::News,
        Widget::Pinned,
        Widget::Notes,
        Widget::Calculator,
        Widget::Recent,
        Widget::Workspaces,
        Widget::OpenWindows,
        Widget::Closed,
        Widget::Groups,
        Widget::Drives,
        Widget::Run,
        Widget::Folder,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Widget::Clock => "Clock",
            Widget::Gauges => "System stats",
            Widget::WorldClocks => "World clocks",
            Widget::Calendar => "Calendar",
            Widget::News => "News feed",
            Widget::Pinned => "Pinned apps",
            Widget::Notes => "Notes",
            Widget::Calculator => "Calculator",
            Widget::Recent => "Recent files",
            Widget::Workspaces => "Workspaces",
            Widget::OpenWindows => "Open windows",
            Widget::Closed => "Recently closed apps",
            Widget::Groups => "App groups (suggested)",
            Widget::Drives => "Drives",
            Widget::Run => "Run command",
            Widget::Folder => "Downloads folder",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Page {
    pub widgets: Vec<Widget>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct WorldClock {
    pub label: String,
    /// Windows time zone id, e.g. "GMT Standard Time". Empty means local time.
    pub tz: String,
}

/// A named set of apps that launch together.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Workspace {
    pub name: String,
    pub apps: Vec<String>,
}

pub const MAX_CLOCKS: usize = 5;

/// Cities offered when adding a world clock: (city, country, Windows time zone id).
/// An empty id means the PC's own time zone.
pub const CITIES: [(&str, &str, &str); 80] = [
    ("Local", "This PC", ""),
    ("Baghdad", "Iraq", "Arabic Standard Time"),
    ("Erbil", "Iraq", "Arabic Standard Time"),
    ("Basra", "Iraq", "Arabic Standard Time"),
    ("Riyadh", "Saudi Arabia", "Arab Standard Time"),
    ("Mecca", "Saudi Arabia", "Arab Standard Time"),
    ("Kuwait City", "Kuwait", "Arab Standard Time"),
    ("Doha", "Qatar", "Arab Standard Time"),
    ("Manama", "Bahrain", "Arab Standard Time"),
    ("Dubai", "United Arab Emirates", "Arabian Standard Time"),
    ("Abu Dhabi", "United Arab Emirates", "Arabian Standard Time"),
    ("Muscat", "Oman", "Arabian Standard Time"),
    ("Amman", "Jordan", "Jordan Standard Time"),
    ("Damascus", "Syria", "Syria Standard Time"),
    ("Beirut", "Lebanon", "Middle East Standard Time"),
    ("Jerusalem", "", "Israel Standard Time"),
    ("Cairo", "Egypt", "Egypt Standard Time"),
    ("Khartoum", "Sudan", "Sudan Standard Time"),
    ("Tripoli", "Libya", "Libya Standard Time"),
    ("Tunis", "Tunisia", "W. Central Africa Standard Time"),
    ("Algiers", "Algeria", "W. Central Africa Standard Time"),
    ("Casablanca", "Morocco", "Morocco Standard Time"),
    ("Tehran", "Iran", "Iran Standard Time"),
    ("Istanbul", "Turkey", "Turkey Standard Time"),
    ("Ankara", "Turkey", "Turkey Standard Time"),
    ("Toronto", "Canada", "Eastern Standard Time"),
    ("Ottawa", "Canada", "Eastern Standard Time"),
    ("Montreal", "Canada", "Eastern Standard Time"),
    ("Vancouver", "Canada", "Pacific Standard Time"),
    ("Calgary", "Canada", "Mountain Standard Time"),
    ("Edmonton", "Canada", "Mountain Standard Time"),
    ("Winnipeg", "Canada", "Central Standard Time"),
    ("Regina", "Canada", "Canada Central Standard Time"),
    ("Halifax", "Canada", "Atlantic Standard Time"),
    ("St. John's", "Canada", "Newfoundland Standard Time"),
    ("New York", "United States", "Eastern Standard Time"),
    ("Washington", "United States", "Eastern Standard Time"),
    ("Chicago", "United States", "Central Standard Time"),
    ("Houston", "United States", "Central Standard Time"),
    ("Denver", "United States", "Mountain Standard Time"),
    ("Phoenix", "United States", "US Mountain Standard Time"),
    ("Los Angeles", "United States", "Pacific Standard Time"),
    ("San Francisco", "United States", "Pacific Standard Time"),
    ("Anchorage", "United States", "Alaskan Standard Time"),
    ("Honolulu", "United States", "Hawaiian Standard Time"),
    ("Mexico City", "Mexico", "Central Standard Time (Mexico)"),
    ("Bogotá", "Colombia", "SA Pacific Standard Time"),
    ("São Paulo", "Brazil", "E. South America Standard Time"),
    ("Buenos Aires", "Argentina", "Argentina Standard Time"),
    ("London", "United Kingdom", "GMT Standard Time"),
    ("Dublin", "Ireland", "GMT Standard Time"),
    ("Lisbon", "Portugal", "GMT Standard Time"),
    ("Paris", "France", "Romance Standard Time"),
    ("Madrid", "Spain", "Romance Standard Time"),
    ("Brussels", "Belgium", "Romance Standard Time"),
    ("Berlin", "Germany", "W. Europe Standard Time"),
    ("Amsterdam", "Netherlands", "W. Europe Standard Time"),
    ("Rome", "Italy", "W. Europe Standard Time"),
    ("Stockholm", "Sweden", "W. Europe Standard Time"),
    ("Vienna", "Austria", "W. Europe Standard Time"),
    ("Athens", "Greece", "GTB Standard Time"),
    ("Kyiv", "Ukraine", "FLE Standard Time"),
    ("Moscow", "Russia", "Russian Standard Time"),
    ("Lagos", "Nigeria", "W. Central Africa Standard Time"),
    ("Nairobi", "Kenya", "E. Africa Standard Time"),
    ("Johannesburg", "South Africa", "South Africa Standard Time"),
    ("Karachi", "Pakistan", "Pakistan Standard Time"),
    ("Kabul", "Afghanistan", "Afghanistan Standard Time"),
    ("New Delhi", "India", "India Standard Time"),
    ("Mumbai", "India", "India Standard Time"),
    ("Dhaka", "Bangladesh", "Bangladesh Standard Time"),
    ("Bangkok", "Thailand", "SE Asia Standard Time"),
    ("Jakarta", "Indonesia", "SE Asia Standard Time"),
    ("Kuala Lumpur", "Malaysia", "Singapore Standard Time"),
    ("Singapore", "Singapore", "Singapore Standard Time"),
    ("Beijing", "China", "China Standard Time"),
    ("Seoul", "South Korea", "Korea Standard Time"),
    ("Tokyo", "Japan", "Tokyo Standard Time"),
    ("Sydney", "Australia", "AUS Eastern Standard Time"),
    ("Auckland", "New Zealand", "New Zealand Standard Time"),
];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Missing in files written by the first release, which reads as 0.
    #[serde(default)]
    pub version: u32,
    pub taskbar_style: TaskbarStyle,
    pub hook_start_button: bool,
    /// Start with Windows. The app registers itself on launch when this is on.
    pub autostart: bool,
    /// Show network speed, CPU, memory and folder buttons in the taskbar's free space.
    pub taskbar_info: bool,
    /// How the system stats widget looks: "graphs" or "rings".
    pub stats_style: String,
    pub workspaces: Vec<Workspace>,
    /// How many times each app was opened from this menu.
    pub launches: std::collections::BTreeMap<String, u32>,
    pub run_history: Vec<String>,
    pub style: Style,
    pub position: Position,
    pub backdrop: Backdrop,
    pub theme: String,
    pub tint_opacity: f32,
    /// "system" or a hex colour such as "#3b9cff".
    pub accent: String,
    pub scale: f32,
    pub icon_size: f32,
    pub hook_win_key: bool,
    pub show_seconds: bool,
    pub clock_24h: bool,
    /// "software" keeps memory lowest; "gpu" uses the graphics card.
    pub renderer: String,
    pub pages: Vec<Page>,
    pub gauges: Vec<String>,
    pub world_clocks: Vec<WorldClock>,
    pub news_title: String,
    pub news_url: String,
    /// Web search address; `{}` is replaced with the query.
    pub web_search: String,
    pub pinned: Vec<String>,
    pub pinned_initialized: bool,
    pub notes: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 4,
            taskbar_style: TaskbarStyle::Default,
            hook_start_button: true,
            autostart: false,
            taskbar_info: true,
            stats_style: "graphs".into(),
            workspaces: Vec::new(),
            launches: Default::default(),
            run_history: Vec::new(),
            style: Style::TwoPane,
            position: Position::BottomCenter,
            backdrop: Backdrop::Acrylic,
            theme: "midnight".into(),
            tint_opacity: 0.72,
            accent: "#3b9cff".into(),
            scale: 1.0,
            icon_size: 40.0,
            hook_win_key: true,
            show_seconds: true,
            clock_24h: false,
            renderer: "software".into(),
            pages: vec![
                Page { widgets: vec![Widget::Clock, Widget::Gauges, Widget::Pinned, Widget::Drives, Widget::Run] },
                Page { widgets: vec![Widget::Groups, Widget::Folder, Widget::News] },
                Page { widgets: vec![Widget::WorldClocks, Widget::Calendar, Widget::Notes] },
                Page { widgets: vec![Widget::Calculator, Widget::Recent, Widget::OpenWindows, Widget::Closed] },
            ],
            gauges: vec!["cpu".into(), "gpu".into(), "mem".into(), "vram".into(), "net".into()],
            world_clocks: vec![
                WorldClock { label: "Local".into(), tz: String::new() },
                WorldClock { label: "London".into(), tz: "GMT Standard Time".into() },
                WorldClock { label: "New York".into(), tz: "Eastern Standard Time".into() },
                WorldClock { label: "Tokyo".into(), tz: "Tokyo Standard Time".into() },
            ],
            news_title: "NASA".into(),
            news_url: "https://www.nasa.gov/feed/".into(),
            web_search: "https://www.google.com/search?q={}".into(),
            pinned: Vec::new(),
            pinned_initialized: false,
            notes: String::new(),
        }
    }
}

/// The part of the configuration that describes a layout. This is what gets
/// exported and imported; it deliberately leaves out notes and pinned apps.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Layout {
    pub orbit_layout: u32,
    pub taskbar_style: TaskbarStyle,
    pub style: Style,
    pub position: Position,
    pub backdrop: Backdrop,
    pub theme: String,
    pub tint_opacity: f32,
    pub accent: String,
    pub scale: f32,
    pub icon_size: f32,
    pub show_seconds: bool,
    pub clock_24h: bool,
    pub pages: Vec<Page>,
    pub gauges: Vec<String>,
    pub world_clocks: Vec<WorldClock>,
    pub news_title: String,
    pub news_url: String,
}

impl Default for Layout {
    fn default() -> Self {
        Config::default().layout()
    }
}

impl Config {
    /// Settings live in the user profile, outside AppData: Windows redirects
    /// AppData writes of sandboxed (packaged) apps into a private copy, which
    /// would give a sandboxed launch and a normal launch different settings.
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join(".orbit-start")
    }

    /// Where earlier releases kept the settings file.
    fn legacy_path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("OrbitStart").join("config.json")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.json")
    }

    pub fn load() -> Config {
        if !Self::path().exists() && Self::legacy_path().exists() {
            let _ = std::fs::create_dir_all(Self::dir());
            let _ = std::fs::copy(Self::legacy_path(), Self::path());
        }
        let mut c: Config = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if c.version < 3 {
            // Settings from an earlier release. If the pages were never edited,
            // move to the current default pages; either way pick up the net gauge.
            use Widget::*;
            let v1 = vec![vec![Clock, Gauges, News, Pinned], vec![WorldClocks, Calendar, Notes], vec![Calculator, Recent]];
            let mut v2 = v1.clone();
            v2.push(vec![Workspaces, OpenWindows, Closed]);
            let current: Vec<Vec<Widget>> = c.pages.iter().map(|p| p.widgets.clone()).collect();
            if current == v1 || current == v2 {
                c.pages = Config::default().pages;
            } else if !current.iter().flatten().any(|w| *w == Groups) && c.pages.len() < 8 {
                // Customized pages: add the new widgets as a second page.
                c.pages.insert(1.min(c.pages.len()), Page { widgets: vec![Groups, Drives, Run, Folder] });
            }
            if c.gauges == ["cpu", "mem", "gpu", "disk"] {
                c.gauges = Config::default().gauges;
            }
            c.version = 3;
        }
        if c.version < 4 {
            // Video memory joined the stats; add it to the stock gauge list.
            if c.gauges == ["cpu", "mem", "gpu", "net"] {
                c.gauges = Config::default().gauges;
            }
            c.version = 4;
        }
        c.sanitize();
        c
    }

    pub fn save(&self) {
        if DEMO.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let _ = std::fs::create_dir_all(Self::dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }

    pub fn sanitize(&mut self) {
        self.scale = self.scale.clamp(0.7, 1.6);
        self.tint_opacity = self.tint_opacity.clamp(0.0, 1.0);
        self.icon_size = self.icon_size.clamp(24.0, 64.0);
        if self.pages.is_empty() {
            self.pages = Config::default().pages;
        }
        self.gauges.truncate(5);
        self.world_clocks.truncate(MAX_CLOCKS);
    }

    pub fn layout(&self) -> Layout {
        Layout {
            orbit_layout: 1,
            taskbar_style: self.taskbar_style,
            style: self.style,
            position: self.position,
            backdrop: self.backdrop,
            theme: self.theme.clone(),
            tint_opacity: self.tint_opacity,
            accent: self.accent.clone(),
            scale: self.scale,
            icon_size: self.icon_size,
            show_seconds: self.show_seconds,
            clock_24h: self.clock_24h,
            pages: self.pages.clone(),
            gauges: self.gauges.clone(),
            world_clocks: self.world_clocks.clone(),
            news_title: self.news_title.clone(),
            news_url: self.news_url.clone(),
        }
    }

    pub fn apply_layout(&mut self, l: Layout) {
        self.taskbar_style = l.taskbar_style;
        self.style = l.style;
        self.position = l.position;
        self.backdrop = l.backdrop;
        self.theme = l.theme;
        self.tint_opacity = l.tint_opacity;
        self.accent = l.accent;
        self.scale = l.scale;
        self.icon_size = l.icon_size;
        self.show_seconds = l.show_seconds;
        self.clock_24h = l.clock_24h;
        self.pages = l.pages;
        self.gauges = l.gauges;
        self.world_clocks = l.world_clocks;
        self.news_title = l.news_title;
        self.news_url = l.news_url;
        self.sanitize();
    }
}

/// Set for `--demo` runs (screenshots): settings are never written.
pub static DEMO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct Theme {
    pub light: bool,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub faint: Color,
    /// Fill used for hover states and cards (drawn with low alpha).
    pub card: Color,
    pub field: Color,
    pub accent: Color,
}

pub const THEMES: [(&str, &str, u32); 6] = [
    ("midnight", "Midnight", 0x15233b),
    ("graphite", "Graphite", 0x1f2125),
    ("wine", "Wine", 0x3b1a28),
    ("forest", "Forest", 0x13291f),
    ("ocean", "Ocean", 0x0e2b3b),
    ("light", "Light", 0xf2f4f8),
];

pub const ACCENTS: [u32; 7] = [0x3b9cff, 0x8b6cff, 0xff5fa2, 0xff8a3d, 0x34c38f, 0x22c1c3, 0xf2c14e];

impl Theme {
    pub fn from(cfg: &Config, system_accent: Color) -> Theme {
        let (id, _, hex) = THEMES.iter().find(|t| t.0 == cfg.theme).copied().unwrap_or(THEMES[0]);
        let light = id == "light";
        let accent = if cfg.accent == "system" {
            system_accent
        } else {
            Color::parse(&cfg.accent).unwrap_or(Color::rgb(ACCENTS[0]))
        };
        if light {
            Theme {
                light,
                bg: Color::rgb(hex),
                fg: Color::rgb(0x14171c),
                dim: Color::rgb(0x14171c).a(0.66),
                faint: Color::rgb(0x14171c).a(0.14),
                card: Color::rgb(0x000000),
                field: Color::rgb(0xffffff).a(0.75),
                accent,
            }
        } else {
            Theme {
                light,
                bg: Color::rgb(hex),
                fg: Color::rgb(0xffffff),
                dim: Color::rgb(0xffffff).a(0.68),
                faint: Color::rgb(0xffffff).a(0.16),
                card: Color::rgb(0xffffff),
                field: Color::rgb(0x000000).a(0.55),
                accent,
            }
        }
    }
}

/// Built-in layout gallery: ready-made looks that can be applied in one click.
pub fn presets() -> Vec<(&'static str, Layout)> {
    use Widget::*;
    let base = Config::default().layout();
    let make = |style: Style, theme: &str, accent: &str, tint: f32, pages: Vec<Vec<Widget>>| Layout {
        style,
        theme: theme.into(),
        accent: accent.into(),
        tint_opacity: tint,
        pages: pages.into_iter().map(|widgets| Page { widgets }).collect(),
        ..base.clone()
    };
    vec![
        ("Classic", base.clone()),
        (
            "Dashboard",
            make(Style::Dashboard, "graphite", "#3b9cff", 0.8, vec![
                vec![Calendar, WorldClocks, Groups, Gauges, Drives, Notes],
                vec![News, Folder, Run, OpenWindows, Closed, Calculator],
            ]),
        ),
        (
            "App list",
            make(Style::Apps, "wine", "#ff8a3d", 0.74, vec![vec![Groups, Folder], vec![Clock, Gauges, Drives, Run]]),
        ),
        ("Minimal", make(Style::Compact, "graphite", "#22c1c3", 0.6, vec![vec![Clock, Pinned], vec![Notes]])),
        (
            "System monitor",
            make(Style::TwoPane, "forest", "#34c38f", 0.75, vec![vec![Gauges, OpenWindows, Closed], vec![Clock, Pinned]]),
        ),
        (
            "Daylight",
            make(Style::Compact, "light", "#8b6cff", 0.8, vec![vec![Clock, Calendar, Notes], vec![News, Pinned]]),
        ),
        (
            "Focus",
            make(Style::TwoPane, "wine", "#ff5fa2", 0.78, vec![vec![WorldClocks, Notes, Calculator], vec![Workspaces, Pinned]]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_zones_exist() {
        // Every zone in the city list must be a real Windows time zone.
        for (city, _, tz) in CITIES {
            if !tz.is_empty() {
                assert!(crate::sysinfo::find_tz(tz).is_some(), "{city}: unknown time zone {tz}");
            }
        }
    }
}
