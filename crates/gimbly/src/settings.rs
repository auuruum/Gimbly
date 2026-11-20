//! User settings behind the Software Settings window.
//!
//! The option sets follow OBSBOT Center's settings pages.
//! Settings are stored in a small INI file of our own
//! format in the per-user configuration folder; unknown or damaged entries
//! fall back to defaults instead of failing.

use crate::camera::{CaptureConfig, Transform};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// Defines a settings enum with display labels and the value written to the
/// settings file.
macro_rules! choice {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn label(self) -> &'static str {
                match self { $($name::$variant => $label),+ }
            }

            fn parse(text: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.label() == text)
            }
        }
    };
}

choice!(Resolution {
    Uhd30 => "4K 30",
    Fhd60 => "1080P 60",
    Fhd30 => "1080P 30",
    Hd60 => "720P 60",
    Hd30 => "720P 30",
});

impl Resolution {
    /// Capture width, height and frame rate.
    pub fn mode(self) -> (u32, u32, u32) {
        match self {
            Resolution::Uhd30 => (3840, 2160, 30),
            Resolution::Fhd60 => (1920, 1080, 60),
            Resolution::Fhd30 => (1920, 1080, 30),
            Resolution::Hd60 => (1280, 720, 60),
            Resolution::Hd30 => (1280, 720, 30),
        }
    }
}

choice!(PreviewMode {
    Landscape => "Landscape",
    Portrait => "Portrait",
});

choice!(Encoder {
    Hardware => "Hardware Encoding",
    Software => "Software Encoding",
});

choice!(Bitrate {
    Low => "Low",
    High => "High",
});

choice!(RecordFormat {
    Mkv => "mkv",
    Mp4 => "mp4",
    Mov => "mov",
});

choice!(CloseAction {
    FullyClosed => "Fully Closed",
    Minimize => "Minimize",
});

choice!(OscConnection {
    UdpServer => "UDP Server",
    TcpServer => "TCP Server",
});

/// A keyboard shortcut such as `Alt + Shift + Z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

/// Keys that can end a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Letter(char),
    Digit(u8),
    F(u8),
    Up,
    Down,
    Left,
    Right,
    Space,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Plus,
    Minus,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
            Key::Space => f.write_str("Space"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PgUp"),
            Key::PageDown => f.write_str("PgDown"),
            Key::Insert => f.write_str("Ins"),
            Key::Delete => f.write_str("Del"),
            Key::Plus => f.write_str("="),
            Key::Minus => f.write_str("-"),
        }
    }
}

impl Key {
    fn parse(text: &str) -> Option<Self> {
        let named = [
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Space,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Insert,
            Key::Delete,
            Key::Plus,
            Key::Minus,
        ];
        if let Some(k) = named.into_iter().find(|k| k.to_string() == text) {
            return Some(k);
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_uppercase() => return Some(Key::Letter(c)),
            (Some(c), None) if c.is_ascii_digit() => return Some(Key::Digit(c as u8 - b'0')),
            _ => {}
        }
        let n: u8 = text.strip_prefix('F')?.parse().ok()?;
        (1..=24).contains(&n).then_some(Key::F(n))
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (
                self.win,
                if cfg!(target_os = "macos") {
                    "Cmd"
                } else {
                    "Win"
                },
            ),
        ] {
            if on {
                write!(f, "{name} + ")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl KeyCombo {
    pub const fn alt(key: Key) -> Self {
        Self {
            ctrl: false,
            alt: true,
            shift: false,
            win: false,
            key,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut combo = KeyCombo::alt(Key::Space);
        combo.alt = false;
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part {
                "Ctrl" => combo.ctrl = true,
                "Alt" => combo.alt = true,
                "Shift" => combo.shift = true,
                "Win" | "Cmd" => combo.win = true,
                "" if text.trim_end().ends_with('+') => key = Some(Key::Plus),
                other => key = Some(Key::parse(other)?),
            }
        }
        combo.key = key?;
        // A global shortcut needs at least one modifier.
        (combo.ctrl || combo.alt || combo.win).then_some(combo)
    }
}

/// Actions on the Global Hotkeys page, in OBSBOT Center's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HotkeyAction {
    ToggleShortcuts,
    ToggleVirtualCamera,
    Device1,
    Device2,
    Device3,
    Device4,
    Pip1,
    Pip2,
    GimbalReset,
    GimbalUp,
    GimbalDown,
    GimbalLeft,
    GimbalRight,
    ZoomIn,
    ZoomOut,
    ZoomMax,
    ZoomMin,
    Snapshot,
    AiLock,
    UpperBody,
    CloseUp,
    Headless,
    LowerBody,
    Group,
    HandTracking,
    DeskMode,
    Whiteboard,
    Preset1,
    Preset2,
    Preset3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyGroup {
    Universal,
    TinySeries,
}

impl HotkeyGroup {
    pub fn label(self) -> &'static str {
        match self {
            HotkeyGroup::Universal => "Universal Shortcut Key",
            HotkeyGroup::TinySeries => "Tiny Series Shortcut Keys",
        }
    }
}

impl HotkeyAction {
    pub const ALL: &'static [HotkeyAction] = &[
        HotkeyAction::ToggleShortcuts,
        HotkeyAction::ToggleVirtualCamera,
        HotkeyAction::Device1,
        HotkeyAction::Device2,
        HotkeyAction::Device3,
        HotkeyAction::Device4,
        HotkeyAction::Pip1,
        HotkeyAction::Pip2,
        HotkeyAction::GimbalReset,
        HotkeyAction::GimbalUp,
        HotkeyAction::GimbalDown,
        HotkeyAction::GimbalLeft,
        HotkeyAction::GimbalRight,
        HotkeyAction::ZoomIn,
        HotkeyAction::ZoomOut,
        HotkeyAction::ZoomMax,
        HotkeyAction::ZoomMin,
        HotkeyAction::Snapshot,
        HotkeyAction::AiLock,
        HotkeyAction::UpperBody,
        HotkeyAction::CloseUp,
        HotkeyAction::Headless,
        HotkeyAction::LowerBody,
        HotkeyAction::Group,
        HotkeyAction::HandTracking,
        HotkeyAction::DeskMode,
        HotkeyAction::Whiteboard,
        HotkeyAction::Preset1,
        HotkeyAction::Preset2,
        HotkeyAction::Preset3,
    ];

    pub fn label(self) -> &'static str {
        use HotkeyAction::*;
        match self {
            ToggleShortcuts => "Turn On/Off Shortcut Keys",
            ToggleVirtualCamera => "Enable/Disable Virtual Camera",
            Device1 => "Device #1",
            Device2 => "Device #2",
            Device3 => "Device #3",
            Device4 => "Device #4",
            Pip1 => "PiP1",
            Pip2 => "PiP2",
            GimbalReset => "Gimbal Reset",
            GimbalUp => "Gimbal move up\nView move up",
            GimbalDown => "Gimbal move down\nView move down",
            GimbalLeft => "Gimbal move left\nView move left",
            GimbalRight => "Gimbal move right\nView move right",
            ZoomIn => "Zoom in",
            ZoomOut => "Zoom out",
            ZoomMax => "Zoom max",
            ZoomMin => "Zoom min",
            Snapshot => "Snapshot",
            AiLock => "AI Lock/Unlock",
            UpperBody => "Upper Body/Cancel",
            CloseUp => "Close-up/Cancel",
            Headless => "Headless/Cancel",
            LowerBody => "Lower Body/Cancel",
            Group => "Group/Cancel",
            HandTracking => "Hand Tracking/Cancel",
            DeskMode => "Desk Mode/Cancel",
            Whiteboard => "Whiteboard/Cancel",
            Preset1 => "Trigger preset position1",
            Preset2 => "Trigger preset position2",
            Preset3 => "Trigger preset position3",
        }
    }

    /// Stable name used in the settings file.
    fn key(self) -> String {
        format!("{self:?}")
    }

    pub fn group(self) -> HotkeyGroup {
        if self >= HotkeyAction::ZoomIn {
            HotkeyGroup::TinySeries
        } else {
            HotkeyGroup::Universal
        }
    }

    /// Whether Gimbly can perform the action yet.
    pub fn implemented(self) -> bool {
        use HotkeyAction::*;
        matches!(
            self,
            ToggleShortcuts
                | ToggleVirtualCamera
                | Device1
                | Device2
                | Device3
                | Device4
                | ZoomIn
                | ZoomOut
                | ZoomMax
                | ZoomMin
                | Snapshot
                | GimbalReset
                | Preset1
                | Preset2
                | Preset3
        )
    }

    /// Bindings OBSBOT Center shows by default (observed on its settings
    /// page); the remaining actions start unbound.
    pub fn default_combo(self) -> Option<KeyCombo> {
        use HotkeyAction::*;
        Some(match self {
            ToggleShortcuts => KeyCombo {
                shift: true,
                ..KeyCombo::alt(Key::Letter('Z'))
            },
            ToggleVirtualCamera => KeyCombo::alt(Key::Letter('X')),
            Device1 => KeyCombo::alt(Key::Digit(1)),
            Device2 => KeyCombo::alt(Key::Digit(2)),
            Device3 => KeyCombo::alt(Key::Digit(3)),
            Device4 => KeyCombo::alt(Key::Digit(4)),
            Pip1 => KeyCombo::alt(Key::Digit(5)),
            Pip2 => KeyCombo::alt(Key::Digit(6)),
            GimbalReset => KeyCombo::alt(Key::Letter('R')),
            GimbalUp => KeyCombo::alt(Key::Up),
            GimbalDown => KeyCombo::alt(Key::Down),
            GimbalLeft => KeyCombo::alt(Key::Left),
            GimbalRight => KeyCombo::alt(Key::Right),
            _ => return None,
        })
    }
}

pub type Bindings = BTreeMap<HotkeyAction, Option<KeyCombo>>;

pub fn default_bindings() -> Bindings {
    HotkeyAction::ALL
        .iter()
        .map(|&a| (a, a.default_combo()))
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub resolution: Resolution,
    pub preview_mode: PreviewMode,
    pub encoder: Encoder,
    pub bitrate: Bitrate,
    pub record_format: RecordFormat,
    pub media_dir: String,
    /// Rotation and mirror chosen in the preview window.
    pub transform: Transform,
    pub hotkeys_enabled: bool,
    pub hotkeys: Bindings,
    pub close_action: CloseAction,
    pub osc_connection: OscConnection,
    pub osc_host: String,
    pub osc_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resolution: Resolution::Fhd30,
            preview_mode: PreviewMode::Landscape,
            encoder: Encoder::Hardware,
            bitrate: Bitrate::Low,
            record_format: RecordFormat::Mkv,
            media_dir: default_media_dir(),
            transform: Transform::default(),
            hotkeys_enabled: false,
            hotkeys: default_bindings(),
            close_action: CloseAction::FullyClosed,
            osc_connection: OscConnection::UdpServer,
            osc_host: String::new(),
            osc_port: 16284,
        }
    }
}

fn default_media_dir() -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let folder = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    home.map(|h| PathBuf::from(h).join(folder).display().to_string())
        .unwrap_or_default()
}

/// The settings file: `%APPDATA%\Gimbly\settings.ini` on Windows,
/// `~/Library/Application Support/Gimbly/settings.ini` on macOS and
/// `$XDG_CONFIG_HOME/gimbly/settings.ini` (or `~/.config/...`) elsewhere.
pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Gimbly")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Gimbly")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
            .join("gimbly")
    };
    Some(base.join("settings.ini"))
}

/// `section -> key -> value`, from a minimal INI dialect: `[Section]`
/// headers, `key=value` lines, `;` or `#` comments.
fn parse_ini(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
        } else if let Some((key, value)) = line.split_once('=') {
            out.entry(section.clone())
                .or_default()
                .insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    out
}

impl Settings {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| Self::from_ini(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or_else(|| std::io::Error::other("no config folder"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write a sibling file first so a crash never leaves half a file.
        let tmp = path.with_extension("ini.tmp");
        std::fs::write(&tmp, self.to_ini())?;
        std::fs::rename(&tmp, &path)
    }

    pub fn from_ini(text: &str) -> Self {
        let ini = parse_ini(text);
        let get = |section: &str, key: &str| ini.get(section).and_then(|s| s.get(key));
        let mut s = Settings::default();
        if let Some(v) = get("Output", "Resolution").and_then(|v| Resolution::parse(v)) {
            s.resolution = v;
        }
        if let Some(v) = get("Output", "PreviewMode").and_then(|v| PreviewMode::parse(v)) {
            s.preview_mode = v;
        }
        if let Some(v) = get("Output", "VideoEncoding").and_then(|v| Encoder::parse(v)) {
            s.encoder = v;
        }
        if let Some(v) = get("Output", "RecordingBitrate").and_then(|v| Bitrate::parse(v)) {
            s.bitrate = v;
        }
        if let Some(v) = get("Output", "RecordingFormat").and_then(|v| RecordFormat::parse(v)) {
            s.record_format = v;
        }
        if let Some(v) = get("Output", "MediaFileLocation").filter(|v| !v.is_empty()) {
            s.media_dir = v.clone();
        }
        let rotation = get("Output", "PreviewRotation")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|d| d % 90 == 0 && *d < 360);
        let mirror = get("Output", "PreviewMirror").is_some_and(|v| v == "true");
        s.transform = Transform::new(rotation.unwrap_or(0), mirror);
        if let Some(v) = get("General", "CloseTheSoftware").and_then(|v| CloseAction::parse(v)) {
            s.close_action = v;
        }
        if let Some(v) = get("Hotkeys", "Enabled") {
            s.hotkeys_enabled = v == "true";
        }
        if let Some(bindings) = ini.get("Hotkeys.Bindings") {
            for action in HotkeyAction::ALL {
                if let Some(v) = bindings.get(&action.key()) {
                    s.hotkeys.insert(*action, KeyCombo::parse(v));
                }
            }
        }
        if let Some(v) = get("OSC", "ConnectionType").and_then(|v| OscConnection::parse(v)) {
            s.osc_connection = v;
        }
        if let Some(v) = get("OSC", "Host") {
            s.osc_host = v.clone();
        }
        if let Some(v) = get("OSC", "ReceivePort").and_then(|v| v.parse().ok()) {
            s.osc_port = v;
        }
        s
    }

    pub fn to_ini(&self) -> String {
        let mut out = String::from("; Gimbly settings\n");
        out += "[Output]\n";
        out += &format!("Resolution={}\n", self.resolution.label());
        out += &format!("PreviewMode={}\n", self.preview_mode.label());
        out += &format!("VideoEncoding={}\n", self.encoder.label());
        out += &format!("RecordingBitrate={}\n", self.bitrate.label());
        out += &format!("RecordingFormat={}\n", self.record_format.label());
        out += &format!("MediaFileLocation={}\n", self.media_dir);
        out += &format!("PreviewRotation={}\n", self.transform.degrees());
        out += &format!("PreviewMirror={}\n", self.transform.mirror);
        out += "\n[General]\n";
        out += &format!("CloseTheSoftware={}\n", self.close_action.label());
        out += "\n[Hotkeys]\n";
        out += &format!("Enabled={}\n", self.hotkeys_enabled);
        out += "\n[Hotkeys.Bindings]\n";
        for (action, combo) in &self.hotkeys {
            let value = combo.map(|c| c.to_string()).unwrap_or_default();
            out += &format!("{}={value}\n", action.key());
        }
        out += "\n[OSC]\n";
        out += &format!("ConnectionType={}\n", self.osc_connection.label());
        out += &format!("Host={}\n", self.osc_host);
        out += &format!("ReceivePort={}\n", self.osc_port);
        out
    }

    pub fn capture_config(&self) -> CaptureConfig {
        let (width, height, fps) = self.resolution.mode();
        CaptureConfig {
            width,
            height,
            fps,
            portrait: self.preview_mode == PreviewMode::Portrait,
            transform: self.transform,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_ini() {
        let mut s = Settings {
            resolution: Resolution::Uhd30,
            preview_mode: PreviewMode::Portrait,
            bitrate: Bitrate::High,
            record_format: RecordFormat::Mp4,
            media_dir: r"D:\Clips".into(),
            transform: Transform::new(270, true),
            hotkeys_enabled: true,
            close_action: CloseAction::Minimize,
            osc_connection: OscConnection::TcpServer,
            osc_host: "192.168.1.5".into(),
            osc_port: 9000,
            ..Settings::default()
        };
        s.hotkeys
            .insert(HotkeyAction::ZoomIn, KeyCombo::parse("Ctrl + Alt + ="));
        s.hotkeys.insert(HotkeyAction::Device1, None);
        assert_eq!(Settings::from_ini(&s.to_ini()), s);
    }

    #[test]
    fn damaged_files_fall_back_to_defaults() {
        let s =
            Settings::from_ini("[Output]\nResolution=8K 240\nnot a line\n[OSC]\nReceivePort=abc\n");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn parses_and_prints_key_combos() {
        for text in [
            "Alt + Shift + Z",
            "Alt + X",
            "Ctrl + Alt + F12",
            "Alt + Up",
            "Alt + =",
        ] {
            let combo = KeyCombo::parse(text).unwrap();
            assert_eq!(combo.to_string(), text);
        }
        let system_combo = KeyCombo::parse("Win + 4").unwrap();
        assert_eq!(KeyCombo::parse("Cmd + 4"), Some(system_combo));
        let system_label = if cfg!(target_os = "macos") {
            "Cmd + 4"
        } else {
            "Win + 4"
        };
        assert_eq!(system_combo.to_string(), system_label);
        assert_eq!(KeyCombo::parse("Shift + A"), None, "needs Ctrl, Alt or Win");
        assert_eq!(KeyCombo::parse("Alt + Banana"), None);
    }

    #[test]
    fn defaults_match_the_observed_settings_page() {
        let d = default_bindings();
        assert_eq!(
            d[&HotkeyAction::ToggleShortcuts].unwrap().to_string(),
            "Alt + Shift + Z"
        );
        assert_eq!(
            d[&HotkeyAction::ToggleVirtualCamera].unwrap().to_string(),
            "Alt + X"
        );
        assert_eq!(d[&HotkeyAction::Device4].unwrap().to_string(), "Alt + 4");
        assert_eq!(
            d[&HotkeyAction::GimbalRight].unwrap().to_string(),
            "Alt + Right"
        );
        assert_eq!(d[&HotkeyAction::ZoomIn], None);
        assert_eq!(HotkeyAction::ZoomIn.group(), HotkeyGroup::TinySeries);
        assert_eq!(HotkeyAction::GimbalRight.group(), HotkeyGroup::Universal);
    }

    #[test]
    fn resolution_drives_the_capture_config() {
        let s = Settings {
            resolution: Resolution::Hd60,
            preview_mode: PreviewMode::Portrait,
            ..Settings::default()
        };
        let c = s.capture_config();
        assert_eq!(
            (c.width, c.height, c.fps, c.portrait),
            (1280, 720, 60, true)
        );
    }
}
