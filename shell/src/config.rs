//! The user's settings, kept as `key = value` lines in `~/.config/des-ui/config`.
//!
//! Everything has a default, so the file only holds what was changed. It is written
//! one key at a time, which leaves comments and unknown lines alone.

use crate::geo::{Place, parse_location};
use crate::term::{CursorShape, TermLook};
use crate::theme;
use des_common::Panels;
use std::path::PathBuf;

pub const MIN_FONT_SIZE: f32 = 10.0;
pub const MAX_FONT_SIZE: f32 = 24.0;
pub const LINE_HEIGHT_RANGE: (f32, f32) = (0.9, 1.6);
pub const SCROLLBACK_CHOICES: [usize; 4] = [1000, 5000, 20000, 100000];
const DEFAULT_FONT_SIZE: f32 = 14.0;

/// Where the terminal's colours come from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Palette {
    /// The theme file's own.
    Preset,
    /// Drawn from the accent, so that every colour agrees with it.
    Derived,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub theme: String,
    /// Optional opaque sRGB accent, independent of the preset terminal palette.
    pub accent: Option<[u8; 3]>,
    /// Whether the terminal's colours, the caution colour and the glow are the
    /// preset's own or drawn from the accent.
    pub palette: Palette,
    /// Size of the terminal's text.
    pub font_size: f32,
    /// A font file to draw the terminal with, instead of the bundled one.
    pub terminal_font: Option<PathBuf>,
    pub line_height: f32,
    pub cursor: CursorShape,
    pub cursor_blink: bool,
    pub bold_bright: bool,
    pub copy_on_select: bool,
    /// Lines kept above the screen, for new tabs.
    pub scrollback: usize,
    /// Whether the globe turns. Still, it shows this machine's side of the world.
    pub globe_rotation: bool,
    /// Whether addresses are looked up online to place them on the globe.
    pub lookups: bool,
    /// This machine's position, when set by hand.
    pub location: Option<Place>,
    /// Whether the screen locks before the machine sleeps.
    pub lock_on_sleep: bool,
    /// Whether a notification that shows also makes a sound.
    pub notification_sounds: bool,
    /// Whether notifications are kept off the screen as they arrive.
    pub do_not_disturb: bool,
    /// The animal at the foot of the left column; "off" for none.
    pub pet: String,
    /// Whether the clocks count twelve hours twice, not twenty-four once.
    pub twelve_hour: bool,
    /// Which panels are open when the desktop starts.
    pub panels: Panels,
    pub side_panel_behavior: des_common::PanelBehavior,
    pub dock_pins: Vec<String>,
    pub dock_animation: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: theme::DEFAULT_NAME.to_string(),
            accent: None,
            palette: Palette::Preset,
            font_size: DEFAULT_FONT_SIZE,
            terminal_font: None,
            line_height: 1.0,
            cursor: CursorShape::Block,
            cursor_blink: false,
            bold_bright: true,
            copy_on_select: false,
            scrollback: 5000,
            globe_rotation: true,
            lookups: true,
            location: None,
            lock_on_sleep: true,
            notification_sounds: true,
            do_not_disturb: false,
            twelve_hour: false,
            pet: "dog".to_string(),
            panels: Panels::default(),
            side_panel_behavior: des_common::PanelBehavior::default(),
            dock_pins: Vec::new(),
            dock_animation: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    des_common::input::config_path()
}

impl Config {
    /// Reads settings from the file's text; anything missing or malformed keeps its
    /// default.
    pub fn parse(text: &str) -> Config {
        let mut config = Config::default();
        config.side_panel_behavior = des_common::PanelBehavior::parse(text);
        for line in text.lines() {
            let Some((key, value)) = line.split('#').next().unwrap_or("").split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" if !value.is_empty() => config.theme = value.to_string(),
                "accent" => {
                    config.accent = value.split(',').map(|part| part.trim().parse::<u8>())
                        .collect::<Result<Vec<_>, _>>().ok().and_then(|rgb| rgb.try_into().ok());
                }
                "palette" => config.palette = if value == "derived" { Palette::Derived } else { Palette::Preset },
                "font_size" => {
                    if let Ok(size) = value.parse::<f32>() {
                        config.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                    }
                }
                "terminal_font" if !value.is_empty() => config.terminal_font = Some(PathBuf::from(value)),
                "line_height" => {
                    if let Ok(factor) = value.parse::<f32>() {
                        config.line_height = factor.clamp(LINE_HEIGHT_RANGE.0, LINE_HEIGHT_RANGE.1);
                    }
                }
                "cursor" => {
                    config.cursor = match value {
                        "bar" => CursorShape::Bar,
                        "underline" => CursorShape::Underline,
                        _ => CursorShape::Block,
                    }
                }
                "cursor_blink" => config.cursor_blink = value == "on",
                "bold_bright" => config.bold_bright = value != "off",
                "copy_on_select" => config.copy_on_select = value == "on",
                "scrollback" => {
                    if let Ok(lines) = value.parse::<usize>() {
                        config.scrollback = lines.clamp(100, 1_000_000);
                    }
                }
                "globe_rotation" => config.globe_rotation = value != "off",
                "do_not_disturb" => config.do_not_disturb = value == "on",
                "lock_on_sleep" => config.lock_on_sleep = value != "off",
                "notification_sounds" => config.notification_sounds = value != "off",
                "dock_animation" => config.dock_animation = value != "off",
                "dock_pins" => {
                    if let Ok(pins) = serde_json::from_str::<Vec<String>>(value) {
                        let mut seen = std::collections::HashSet::new();
                        config.dock_pins = pins.into_iter().filter(|id| !id.is_empty() && !id.chars().any(char::is_control) && seen.insert(id.clone())).take(128).collect();
                    }
                }
                "clock" => config.twelve_hour = value == "12",
                "pet" if !value.is_empty() => config.pet = value.to_string(),
                "geo" => config.lookups = value != "off",
                "location" => config.location = parse_location(value),
                "panels" => {
                    let open = |name: &str| value.split(',').any(|part| part.trim() == name);
                    config.panels = Panels {
                        left: open("left"),
                        right: open("right"),
                        bottom: open("bottom"),
                    };
                }
                _ => {}
            }
        }
        config
    }

    pub fn load() -> Config {
        path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Config::parse(&text))
            .unwrap_or_default()
    }

    /// Resolve the preset and the user's accent through one path at startup and on edits.
    pub fn resolved_theme(&self) -> theme::Theme {
        let mut theme = theme::Theme::load(&self.theme);
        if let Some([r, g, b]) = self.accent {
            let accent = eframe::egui::Color32::from_rgb(r, g, b);
            if theme.cursor == theme.main { theme.cursor = accent; }
            theme.main = accent;
        }
        if self.palette == Palette::Derived {
            theme = theme.derived();
        }
        theme
    }

    /// How the terminal should look, from these settings.
    pub fn term_look(&self) -> TermLook {
        TermLook {
            font_size: self.font_size,
            line_height: self.line_height,
            cursor: self.cursor,
            cursor_blink: self.cursor_blink,
            bold_bright: self.bold_bright,
            copy_on_select: self.copy_on_select,
        }
    }

    /// The `panels` value for a set of open panels.
    pub fn panels_value(panels: Panels) -> String {
        let names: Vec<&str> = [("left", panels.left), ("right", panels.right), ("bottom", panels.bottom)]
            .into_iter()
            .filter_map(|(name, open)| open.then_some(name))
            .collect();
        if names.is_empty() { "none".to_string() } else { names.join(", ") }
    }
}

/// Sets one key in the file, or removes it (back to its default) when `value` is
/// `None`, leaving every other line as it was.
pub fn set(key: &str, value: Option<&str>) -> std::io::Result<PathBuf> {
    let path = path().ok_or_else(|| std::io::Error::other("no home directory"))?;
    set_at(&path, key, value)?;
    Ok(path)
}

fn set_at(path: &std::path::Path, key: &str, value: Option<&str>) -> std::io::Result<()> {
    use std::io::{ErrorKind, Write};
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().ok_or_else(|| std::io::Error::other("no config directory"))?;
    std::fs::create_dir_all(dir)?;
    // Lock a separate file: locking the config itself would lock the old inode
    // after replacement and let another writer race the next update.
    let lock = std::fs::OpenOptions::new()
        .read(true).write(true).create(true).truncate(false).mode(0o600)
        .open(dir.join("config.lock"))?;
    lock.lock()?;
    let old = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut lines: Vec<String> = old
        .lines()
        .filter(|line| line.split_once('=').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_string)
        .collect();
    if let Some(value) = value {
        lines.push(format!("{key} = {value}"));
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    let mut fresh = tempfile::NamedTempFile::new_in(dir)?;
    fresh.write_all(text.as_bytes())?;
    fresh.as_file().sync_all()?;
    fresh.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_accent_round_trips_and_invalid_values_use_theme() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        set_at(&path, "accent", Some("12, 128, 255")).unwrap();
        let config = Config::parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(config.accent, Some([12, 128, 255]));
        assert_eq!(config.resolved_theme().main, eframe::egui::Color32::from_rgb(12, 128, 255));
        for invalid in ["", "256, 0, 0", "-1, 2, 3", "1, 2", "1, 2, 3, 4", "red"] {
            assert_eq!(Config::parse(&format!("accent = {invalid}")).accent, None);
        }
        set_at(&path, "accent", None).unwrap();
        assert_eq!(Config::parse(&std::fs::read_to_string(&path).unwrap()).accent, None);
        let base = theme::Theme::load(&config.theme);
        let resolved = config.resolved_theme();
        assert_eq!(resolved.ansi, base.ansi);
        assert_eq!(resolved.term_fg, base.term_fg);
    }

    #[test]
    fn a_derived_palette_follows_the_accent_and_anything_else_keeps_the_preset() {
        assert_eq!(Config::parse("palette = derived").palette, Palette::Derived);
        for other in ["preset", "", "theme", "Derived"] {
            assert_eq!(Config::parse(&format!("palette = {other}")).palette, Palette::Preset, "{other:?}");
        }
        let config = Config::parse("palette = derived\naccent = 12, 128, 255\n");
        let resolved = config.resolved_theme();
        let accent = eframe::egui::Color32::from_rgb(12, 128, 255);
        assert_eq!(resolved.main, accent);
        let preset = Config::parse("accent = 12, 128, 255\n").resolved_theme();
        assert_ne!(resolved.ansi, preset.ansi);
        assert_eq!(resolved.ansi, preset.derived().ansi);
        assert_eq!(resolved.caution, preset.derived().caution);
    }

    #[test]
    fn dock_pins_round_trip_and_invalid_preferences_are_safe() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("config");
        let pins=vec!["firefox.desktop".to_string(),"org.example.Editor.desktop".to_string()];
        set_at(&path,"dock_pins",Some(&serde_json::to_string(&pins).unwrap())).unwrap();
        assert_eq!(Config::parse(&std::fs::read_to_string(&path).unwrap()).dock_pins,pins);
        assert!(Config::parse("dock_pins = broken").dock_pins.is_empty());
        assert_eq!(Config::parse(r#"dock_pins = ["a.desktop","a.desktop",""]"#).dock_pins,vec!["a.desktop"]);
        assert!(!Config::parse("dock_animation = off").dock_animation);
    }

    #[test]
    fn minimized_panel_preference_survives_saving_and_keeps_open_panels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, "panels = right, bottom\n").unwrap();
        for (value, wanted) in [("hide", des_common::PanelBehavior::Hide), ("collapse", des_common::PanelBehavior::Collapse)] {
            set_at(&path, "side_panel_behavior", Some(value)).unwrap();
            let config = Config::parse(&std::fs::read_to_string(&path).unwrap());
            assert_eq!(config.side_panel_behavior, wanted);
            assert_eq!(config.panels, Panels { left: false, right: true, bottom: true });
        }
    }

    #[test]
    fn updating_settings_replaces_the_file_without_truncating_existing_readers() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let original = "# mine\nunknown = keep\ntheme = signal\n";
        std::fs::write(&path, original).unwrap();
        let mut reader = std::fs::File::open(&path).unwrap();
        set_at(&path, "theme", Some("blade")).unwrap();
        let mut old = String::new();
        reader.read_to_string(&mut old).unwrap();
        assert_eq!(old, original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\nunknown = keep\ntheme = blade\n");
        set_at(&path, "theme", None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\nunknown = keep\n");
    }

    #[test]
    fn concurrent_settings_updates_keep_every_key() {
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("config"));
        let ready = Arc::new(Barrier::new(24));
        let writers: Vec<_> = (0..24).map(|index| {
            let path = path.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                set_at(&path, &format!("key{index}"), Some("kept")).unwrap();
            })
        }).collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = std::fs::read_to_string(&*path).unwrap();
        for index in 0..24 {
            assert!(text.lines().any(|line| line == format!("key{index} = kept")));
        }
    }

    #[test]
    fn unreadable_existing_settings_are_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let original = [0xff, 0xfe];
        std::fs::write(&path, original).unwrap();
        assert!(set_at(&path, "theme", Some("blade")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn missing_settings_keep_their_defaults() {
        assert_eq!(Config::parse(""), Config::default());
        assert_eq!(Config::parse("nonsense\nother = 1\nfont_size = big\nlocation = nowhere\n"), Config::default());
    }

    #[test]
    fn reads_every_setting() {
        let config = Config::parse(
            "# mine\ntheme = matrix\nfont_size = 16\nglobe_rotation = off\ngeo = off # private\n\
             location = 48.85, 2.35, Paris\npanels = left, bottom\nclock = 12\n",
        );
        assert!(config.twelve_hour);
        assert_eq!(config.theme, "matrix");
        assert_eq!(config.font_size, 16.0);
        assert!(!config.globe_rotation);
        assert!(!config.lookups);
        assert_eq!(config.location.unwrap().label, "Paris");
        assert_eq!(config.panels, Panels { left: true, right: false, bottom: true });
    }

    #[test]
    fn reads_terminal_settings() {
        let config = Config::parse(
            "terminal_font = /usr/share/fonts/x.ttf\nline_height = 1.3\ncursor = bar\ncursor_blink = on\n\
             bold_bright = off\ncopy_on_select = on\nscrollback = 20000\n",
        );
        assert_eq!(config.terminal_font, Some(PathBuf::from("/usr/share/fonts/x.ttf")));
        assert_eq!(config.line_height, 1.3);
        assert_eq!(config.cursor, CursorShape::Bar);
        assert!(config.cursor_blink && !config.bold_bright && config.copy_on_select);
        assert_eq!(config.scrollback, 20000);
        assert_eq!(Config::parse("line_height = 9\ncursor = star\nscrollback = 1\n").line_height, LINE_HEIGHT_RANGE.1);
        assert_eq!(Config::parse("cursor = star").cursor, CursorShape::Block);
        assert_eq!(Config::parse("scrollback = 1").scrollback, 100);
    }

    #[test]
    fn font_size_stays_readable() {
        assert_eq!(Config::parse("font_size = 2").font_size, MIN_FONT_SIZE);
        assert_eq!(Config::parse("font_size = 200").font_size, MAX_FONT_SIZE);
    }

    #[test]
    fn panels_round_trip() {
        for bits in 0..8 {
            let panels = Panels::from_bits(bits).unwrap();
            let text = format!("panels = {}", Config::panels_value(panels));
            assert_eq!(Config::parse(&text).panels, panels, "{text}");
        }
    }
}
