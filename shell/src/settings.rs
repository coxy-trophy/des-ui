//! The settings screen, shown in place of the terminal. It only draws and reports what
//! the user asked for; applying and saving a change is the caller's job.

use eframe::egui;
use crate::config::{Config, LINE_HEIGHT_RANGE, MAX_FONT_SIZE, MIN_FONT_SIZE, Palette, SCROLLBACK_CHOICES};
use crate::radio::Radio;
use crate::term::CursorShape;
use des_common::input::{InputSettings, REPEAT_DELAY_RANGE, REPEAT_RATE_RANGE};
use crate::geo::{Place, Snapshot, parse_location};
use crate::sysmon::SysMon;
use crate::theme::{self, Theme};
use crate::ui;
use des_common::{
    Panel, Panels,
    ipc::{DisplayChoice, DisplayInfo, Displays, Mode, Setup, Usage},
};
use eframe::egui::{
    Align, Align2, Color32, Key, Layout, Modifiers, Rect, ScrollArea, Sense, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, vec2,
};
use std::time::{Duration, Instant};

/// Something the user changed.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Appearance(des_common::appearance::AppearanceSettings),
    SaveSession,
    RestoreSession,
    OpenMusic,
    Theme(String),
    Accent(Option<[u8; 3]>),
    FontSize(f32),
    GlobeRotation(bool),
    Lookups(bool),
    /// `None` goes back to guessing from the machine's address.
    Location(Option<Place>),
    /// Open or collapse a panel, now and at every start.
    Panel(Panel, bool),
    /// Percent.
    Volume(u8),
    Brightness(u8),
    /// Save a plain setting in the configuration file; `None` goes back to its default.
    Set(&'static str, Option<String>),
    /// Try this arrangement of the monitors, listed left to right.
    ApplyDisplays(Vec<DisplayChoice>),
    KeepDisplays,
    RevertDisplays,
    Lock,
    LogOut,
    /// Use this picture on the lock and login screens; `None` goes back to none.
    Avatar(Option<std::path::PathBuf>),
    /// Turn an autostart entry, named by its file, on or off.
    Autostart(String, bool),
    /// Start this command line with every session, through a new autostart entry.
    AutostartAdd(String),
}

/// The pictures on offer for the lock and login screens, read in the background:
/// a large photograph takes a moment to make small.
struct Faces {
    read: std::sync::Arc<std::sync::Mutex<Vec<(std::path::PathBuf, egui::ColorImage)>>>,
    shown: Vec<(std::path::PathBuf, egui::TextureHandle)>,
    /// The picture in use, once looked for.
    current: Option<Option<egui::TextureHandle>>,
    path: String,
}

impl Faces {
    fn new(ctx: &egui::Context) -> Faces {
        let read: std::sync::Arc<std::sync::Mutex<Vec<(std::path::PathBuf, egui::ColorImage)>>> = Default::default();
        let (into, ctx) = (read.clone(), ctx.clone());
        std::thread::spawn(move || {
            let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let folders = [home.join("Pictures"), std::path::PathBuf::from("/usr/share/pixmaps/faces")];
            for path in crate::greeter::pictures_in(&folders, 36) {
                // One that cannot be read is simply not offered.
                let Some(image) = image::ImageReader::open(&path).ok().and_then(|reader| reader.with_guessed_format().ok()).and_then(|reader| reader.decode().ok()) else {
                    continue;
                };
                // Square, from the middle, as it will be used.
                let side = image.width().min(image.height());
                let square = image.crop_imm((image.width() - side) / 2, (image.height() - side) / 2, side, side).thumbnail(128, 128).to_rgba8();
                let pixels = egui::ColorImage::from_rgba_unmultiplied([square.width() as usize, square.height() as usize], square.as_raw());
                into.lock().unwrap().push((path, pixels));
                ctx.request_repaint();
            }
        });
        Faces { read, shown: Vec::new(), current: None, path: String::new() }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Appearance,
    Terminal,
    Frame,
    Displays,
    Network,
    Bluetooth,
    Keyboard,
    Shortcuts,
    Pointer,
    Globe,
    Sound,
    Region,
    Users,
    Session,
    Health,
}

const SECTIONS: [(Section, &str); 15] = [
    (Section::Appearance, "APPEARANCE"),
    (Section::Terminal, "TERMINAL"),
    (Section::Frame, "FRAME"),
    (Section::Displays, "DISPLAYS"),
    (Section::Network, "NETWORK"),
    (Section::Bluetooth, "BLUETOOTH"),
    (Section::Keyboard, "KEYBOARD"),
    (Section::Shortcuts, "SHORTCUTS"),
    (Section::Pointer, "MOUSE & TOUCHPAD"),
    (Section::Globe, "GLOBE"),
    (Section::Sound, "SOUND & SCREEN"),
    (Section::Region, "DATE & REGION"),
    (Section::Users, "USERS"),
    (Section::Session, "SESSION"),
    (Section::Health, "HEALTH"),
];

/// Canonical settings destinations used by the command palette.
pub fn destinations() -> Vec<(&'static str, &'static str)> {
    SECTIONS.iter().map(|(_, title)| (*title, *title)).collect()
}

#[derive(Clone, Copy, Default, PartialEq)]
enum AppearanceTab { #[default] Wallpaper, Theme, Effects }

pub struct Settings {
    pub open: bool,
    section: Section,
    shortcut_editor: crate::shortcuts::Editor,
    appearance_tab: AppearanceTab,
    /// Installed themes and their accents, read when the screen opens.
    themes: Vec<(String, Color32)>,
    accent_draft: Option<([u8; 3], [u8; 3])>,
    pub appearance_editor: crate::appearance_editor::Editor,
    location_text: String,
    location_error: bool,
    /// A level the user just set, shown until the system reports it back.
    volume_held: Option<(u8, Instant)>,
    brightness_held: Option<(u8, Instant)>,
    /// Logging out takes two clicks; this is when the first one happened.
    log_out_armed: Option<Instant>,
    /// The arrangement being edited, left to right, and the report it started from.
    draft: Vec<DisplayChoice>,
    draft_basis: Vec<DisplayInfo>,
    /// Which monitor of the draft the controls act on.
    selected_display: usize,
    /// Monospace fonts found on this machine, read when the screen opens.
    fonts: Vec<(String, std::path::PathBuf)>,
    layout_text: String,
    radio: Option<Radio>,
    faces: Option<Faces>,
    sound: Option<crate::sound::Sound>,
    users: Option<crate::users::Users>,
    region: Option<crate::region::Region>,
    health: crate::health::Health,
    desktop_connected: Option<bool>,
    music_status: &'static str,
    live_terminal_counts: (usize, usize),
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            open: false,
            section: Section::Appearance,
            shortcut_editor: Default::default(),
            appearance_tab: AppearanceTab::default(),
            themes: Vec::new(),
            accent_draft: None,
            appearance_editor: Default::default(),
            location_text: String::new(),
            location_error: false,
            volume_held: None,
            brightness_held: None,
            log_out_armed: None,
            draft: Vec::new(),
            draft_basis: Vec::new(),
            selected_display: 0,
            fonts: Vec::new(),
            layout_text: String::new(),
            radio: None,
            faces: None,
            sound: None,
            users: None,
            region: None,
            health: Default::default(),
            desktop_connected: None,
            music_status: "unknown",
            live_terminal_counts: (0, 0),
        }
    }
}

/// The arrangement the compositor reports, as choices.
fn current_choices(monitors: &[DisplayInfo]) -> Vec<DisplayChoice> {
    monitors
        .iter()
        .map(|monitor| DisplayChoice {
            name: monitor.name.clone(),
            mode: monitor.current,
            main: monitor.main,
            setup: monitor.setup,
        })
        .collect()
}

/// The sizes a monitor offers, largest first, without repeats.
fn sizes(monitor: &DisplayInfo) -> Vec<(u32, u32)> {
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    for mode in &monitor.modes {
        if !sizes.contains(&(mode.width, mode.height)) {
            sizes.push((mode.width, mode.height));
        }
    }
    sizes
}

/// The best mode a monitor offers at a size: the fastest.
fn best_mode(monitor: &DisplayInfo, size: (u32, u32)) -> Option<Mode> {
    monitor
        .modes
        .iter()
        .filter(|mode| (mode.width, mode.height) == size)
        .max_by_key(|mode| mode.refresh)
        .copied()
}

/// A refresh rate the way people say it: 60 Hz, 59.94 Hz.
fn hertz(refresh: u32) -> String {
    let hz = refresh as f32 / 1000.0;
    if (hz - hz.round()).abs() < 0.006 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

/// A level that follows the system's reading, except just after the user moved it.
fn held(slot: &mut Option<(u8, Instant)>, reading: Option<u8>) -> Option<u8> {
    match slot {
        Some((value, at)) if at.elapsed() < Duration::from_secs(4) => Some(*value),
        _ => {
            *slot = None;
            reading
        }
    }
}

impl Settings {
    pub fn set_live_terminal_counts(&mut self, managed: usize, total: usize) { self.live_terminal_counts = (managed, total); }
    pub fn set_music_status(&mut self,status:&'static str){self.music_status=status;}

    pub fn set_desktop_connected(&mut self, connected: bool) {
        self.desktop_connected = Some(connected);
    }

    pub fn open(&mut self, config: &Config) {
        self.shortcut_editor.cancel();
        self.appearance_editor.reset(&config.appearance);
        self.themes = theme::installed();
        self.fonts = ui::monospace_fonts();
        self.location_text = match &config.location {
            Some(place) => format!("{}, {}, {}", place.lat, place.lon, place.label),
            None => String::new(),
        };
        self.location_error = false;
        self.log_out_armed = None;
        self.open = true;
    }

    /// Opens on the section with this name, as the `--settings=<name>` option asks.
    pub fn open_at(&mut self, config: &Config, name: &str) {
        self.open(config);
        let wanted = name.to_uppercase();
        if let Some((section, _)) = SECTIONS.iter().find(|(_, title)| title.starts_with(&wanted)) {
            self.section = *section;
        }
    }

    pub fn shortcut_capture_active(&self) -> bool {
        self.open && self.section == Section::Shortcuts && self.shortcut_editor.capturing()
    }

    /// Draws the screen into `rect` and returns what was changed this frame.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        theme: &Theme,
        config: &Config,
        panels: Panels,
        mon: &SysMon,
        geo: &Snapshot,
        displays: Option<&Displays>,
        input: &InputSettings,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        // Escape closes, unless it is being used to leave the text field.
        if !self.shortcut_capture_active() && !self.appearance_editor.picker_open() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.appearance_editor.reset(&config.appearance);
            self.open = false;
            return changes;
        }

        self.page(ui, rect, theme, |settings, ui| {
                        match settings.section {
                            Section::Health => {
                                settings.health.show(ui, theme, settings.desktop_connected, settings.music_status);
                                if ui::button(ui,theme,"OPEN MUSIC",None,false).clicked(){changes.push(Change::OpenMusic);}
                            },
                            Section::Appearance => settings.appearance(ui, theme, config, &mut changes),
                            Section::Terminal => settings.terminal(ui, theme, config, &mut changes),
                            Section::Frame => settings.frame(ui, theme, config, panels, &mut changes),
                            Section::Network => {
                                let radio = settings.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
                                radio.show_wifi(ui, theme);
                                radio.show_links(ui, theme);
                            }
                            Section::Bluetooth => {
                                let radio = settings.radio.get_or_insert_with(|| Radio::new(ui.ctx().clone()));
                                radio.show_bluetooth(ui, theme);
                            }
                            Section::Shortcuts => {
                                if let Some(bindings) = settings.shortcut_editor.show(ui, theme, &config.shortcuts) {
                                    changes.push(Change::Set("shortcuts", Some(bindings.encode())));
                                }
                            }
                            Section::Keyboard => settings.keyboard(ui, theme, config, input, &mut changes),
                            Section::Pointer => settings.pointer(ui, theme, input, &mut changes),
                            Section::Displays => settings.displays(ui, theme, displays, &mut changes),
                            Section::Globe => settings.globe(ui, theme, config, geo, &mut changes),
                            Section::Sound => settings.sound(ui, theme, config, mon, &mut changes),
                            Section::Region => settings.region.get_or_insert_with(|| crate::region::Region::new(ui.ctx().clone())).show(ui, theme, config.twelve_hour, &mut changes),
                            Section::Users => settings.users.get_or_insert_with(|| crate::users::Users::new(ui.ctx().clone())).show(ui, theme),
                            Section::Session => settings.session(ui, theme, config, input, &mut changes),
                        }
        });
        if !self.open { self.appearance_editor.reset(&config.appearance); }
        if !self.open || self.section != Section::Shortcuts { self.shortcut_editor.cancel(); }
        changes
    }

    /// One responsive page owner for every Settings section. Narrow workspaces
    /// use a section selector; wide ones keep navigation beside the content.
    fn page(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme, draw: impl FnOnce(&mut Self, &mut Ui)) {
        let padding = (rect.width() * 0.025).clamp(12.0, 28.0);
        let area = rect.shrink2(vec2(padding, 16.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                ui::tracked(ui, "SETTINGS", ui::display_bold(22.0), ui::TEXT, 0.6);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui::button(ui, theme, "DONE", Some("ESC"), false).clicked() {
                        self.open = false;
                    }
                });
            });
            ui.add_space(16.0);
            let wide = ui.available_width() >= 720.0;
            if wide {
                let navigation = 176.0;
                let content = (ui.available_width() - navigation - 24.0).max(1.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.allocate_ui_with_layout(vec2(navigation, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                        ScrollArea::vertical().id_salt("settings-sections").auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            for (section, title) in SECTIONS {
                                let label = egui::RichText::new(title).font(ui::display(12.0));
                                if ui.add_sized(vec2(navigation, 34.0), egui::Button::new(label).selected(self.section == section)).clicked() {
                                    self.section = section; self.log_out_armed = None;
                                }
                            }
                        });
                    });
                    ui.allocate_ui_with_layout(vec2(content, ui.available_height()), Layout::top_down(Align::Min), |ui| self.page_content(ui, draw));
                });
            } else {
                let current = SECTIONS.iter().find(|(section, _)| *section == self.section).map_or("APPEARANCE", |(_, title)| *title);
                egui::ComboBox::from_id_salt("settings-section").width(ui.available_width()).selected_text(current).show_ui(ui, |ui| {
                    for (section, title) in SECTIONS {
                        if ui.selectable_value(&mut self.section, section, title).changed() { self.log_out_armed = None; }
                    }
                });
                ui.add_space(16.0);
                self.page_content(ui, draw);
            }
        });
    }
    fn page_content(&mut self, ui: &mut Ui, draw: impl FnOnce(&mut Self, &mut Ui)) {
        if self.section == Section::Appearance {
        ui.horizontal_wrapped(|ui| {
            for (tab, title) in [(AppearanceTab::Wallpaper, "Wallpaper"), (AppearanceTab::Theme, "Theme"), (AppearanceTab::Effects, "Effects")] {
                ui.selectable_value(&mut self.appearance_tab, tab, title);
            }
        });
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);
        }
        ScrollArea::vertical().id_salt(("settings-content", self.section as u8, self.appearance_tab as u8))
            .auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(960.0));
                ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
                draw(self, ui);
            });
    }

    fn appearance(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, changes: &mut Vec<Change>) {
        match self.appearance_tab {
            AppearanceTab::Wallpaper => if let Some(appearance) = self.appearance_editor.show(ui, &config.appearance) { changes.push(Change::Appearance(appearance)); },
            AppearanceTab::Theme => self.theme_appearance(ui, theme, config, changes),
            AppearanceTab::Effects => self.effects_appearance(ui, theme, config, changes),
        }
    }

    fn theme_appearance(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, changes: &mut Vec<Change>) {
        heading(ui, "THEME", "Sets the accent colour and the terminal's colours. The greys stay.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for (name, accent) in &self.themes {
                let chosen = *name == config.theme;
                if ui::chip(ui, theme, &name.to_uppercase(), Some(*accent), chosen).clicked() && !chosen {
                    changes.push(Change::Theme(name.clone()));
                }
            }
        });
        ui.add_space(20.0);
        heading(ui, "ACCENT COLOUR", "Choose a custom accent, then apply it. It stays selected when you switch presets.");
        let current = [theme.main.r(), theme.main.g(), theme.main.b()];
        let (basis, draft) = self.accent_draft.get_or_insert((current, current));
        if *basis != current { *basis = current; *draft = current; }
        ui.horizontal_wrapped(|ui| {
            ui.label("Custom accent");
            ui.color_edit_button_srgb(draft).on_hover_text("Choose accent colour");
            ui.label(egui::RichText::new(format!("#{:02X}{:02X}{:02X}", draft[0], draft[1], draft[2])).font(ui::mono(12.0)));
            if ui.add_enabled(*draft != current, egui::Button::new("Apply color")).clicked() {
                changes.push(Change::Accent(Some(*draft)));
            }
            if ui.add_enabled(config.accent.is_some() || *draft != current, egui::Button::new("Reset to theme")).clicked() {
                *draft = current;
                changes.push(Change::Accent(None));
            }
        });
        note(ui, "The swatch previews your colour. Apply saves it for future sessions.");
        ui.add_space(20.0);
        heading(ui, "PALETTE", "The terminal's colours, the caution colour and the glow can be drawn from the accent, so that everything agrees.");
        let derived = config.palette == Palette::Derived;
        if switch_row(ui, theme, "Derive the palette from the accent", "Off keeps the preset's own terminal colours", derived) {
            changes.push(Change::Set("palette", (!derived).then(|| "derived".to_string())));
        }
        ui.add_space(10.0);
        swatches(ui, theme);
    }

    fn effects_appearance(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, changes: &mut Vec<Change>) {
        heading(ui, "PHOSPHOR", "Drawn by the compositor over everything: a glow on bright text and the accent, and the lines of a tube. Screenshots stay clean.");
        if let Some(frac) = slider_row(ui, theme, "Glow", config.glow, &format!("{:.0}%", config.glow * 100.0)) {
            let glow = (frac * 20.0).round() / 20.0;
            if (glow - config.glow).abs() > f32::EPSILON {
                changes.push(Change::Set("glow", (glow > 0.0).then(|| format!("{glow:.2}"))));
            }
        }
        if switch_row(ui, theme, "Scanlines", "Faint lines across every display", config.scanlines) {
            changes.push(Change::Set("scanlines", (!config.scanlines).then(|| "on".to_string())));
        }
        ui.add_space(20.0);
        heading(ui, "NIGHT LIGHT", "Warmer colours in the evening, eased in and out over a minute.");
        let night = config.night_light;
        if switch_row(ui, theme, "Night light", "From 19:00 to 07:00 unless you choose other hours", night.is_some()) {
            let value = if night.is_some() { None } else { Some(crate::config::night_light_value((19, 7, 4500))) };
            changes.push(Change::Set("night_light", value));
        }
        if let Some((from, to, kelvin)) = night {
            let (mut start, mut end) = (from, to);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(egui::RichText::new("From").font(ui::mono(13.0)));
                ui.add(egui::DragValue::new(&mut start).range(0..=23).suffix(":00"));
                ui.label(egui::RichText::new("to").font(ui::mono(13.0)));
                ui.add(egui::DragValue::new(&mut end).range(0..=23).suffix(":00"));
            });
            ui.add_space(12.0);
            // Left is warmest: 2700 K, as far as is comfortable; right is daylight.
            let frac = (kelvin.saturating_sub(2700)) as f32 / 3800.0;
            let mut warmth = kelvin;
            if let Some(frac) = slider_row(ui, theme, "Warmth", 1.0 - frac, &format!("{kelvin} K")) {
                warmth = (6500.0 - frac * 3800.0).round() as u32 / 100 * 100;
            }
            if (start, end, warmth) != (from, to, kelvin) && start != end {
                changes.push(Change::Set("night_light", Some(crate::config::night_light_value((start, end, warmth)))));
            }
        }
        ui.add_space(20.0);
        note(ui, "Fonts, text size and the cursor are under Terminal.");
    }

    fn terminal(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, changes: &mut Vec<Change>) {
        heading(ui, "FONT", "The typeface the terminal is drawn in. Prompt symbols still come from your Nerd Font.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let bundled = config.terminal_font.is_none();
            if ui::chip(ui, theme, "JETBRAINS MONO", None, bundled).clicked() && !bundled {
                changes.push(Change::Set("terminal_font", None));
            }
            for (family, file) in self.fonts.iter().filter(|(family, _)| family != "JetBrains Mono") {
                let chosen = config.terminal_font.as_deref() == Some(file.as_path());
                if ui::chip(ui, theme, &family.to_uppercase(), None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("terminal_font", Some(file.to_string_lossy().into_owned())));
                }
            }
        });
        ui.add_space(22.0);

        heading(ui, "TEXT", "");
        let size_span = MAX_FONT_SIZE - MIN_FONT_SIZE;
        if let Some(frac) = slider_row(ui, theme, "Size", (config.font_size - MIN_FONT_SIZE) / size_span, &format!("{:.0} px", config.font_size)) {
            let size = (MIN_FONT_SIZE + frac * size_span).round();
            if size != config.font_size {
                changes.push(Change::FontSize(size));
            }
        }
        let (low, high) = LINE_HEIGHT_RANGE;
        if let Some(frac) = slider_row(ui, theme, "Line height", (config.line_height - low) / (high - low), &format!("{:.2}", config.line_height)) {
            let factor = ((low + frac * (high - low)) * 20.0).round() / 20.0;
            if factor != config.line_height {
                changes.push(Change::Set("line_height", Some(format!("{factor:.2}"))));
            }
        }
        if switch_row(ui, theme, "Bold text in bright colours", "As most terminals do. Off keeps bold text in its own colour", config.bold_bright) {
            changes.push(Change::Set("bold_bright", config.bold_bright.then(|| "off".to_string())));
        }
        ui.add_space(22.0);

        heading(ui, "CURSOR", "");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (shape, name, value) in [
                (CursorShape::Block, "BLOCK", None),
                (CursorShape::Bar, "BAR", Some("bar")),
                (CursorShape::Underline, "UNDERLINE", Some("underline")),
            ] {
                let chosen = config.cursor == shape;
                if ui::chip(ui, theme, name, None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("cursor", value.map(str::to_string)));
                }
            }
        });
        ui.add_space(8.0);
        if switch_row(ui, theme, "Blink", "The cursor flashes once a second", config.cursor_blink) {
            changes.push(Change::Set("cursor_blink", (!config.cursor_blink).then(|| "on".to_string())));
        }
        ui.add_space(22.0);

        heading(ui, "BEHAVIOUR", "");
        if switch_row(ui, theme, "Copy on select", "Selected text goes to the clipboard at once, without Ctrl+Shift+C", config.copy_on_select) {
            changes.push(Change::Set("copy_on_select", (!config.copy_on_select).then(|| "on".to_string())));
        }
        ui.add_space(14.0);
        note(ui, "Lines kept above the screen. Applies to tabs opened from now on.");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for lines in SCROLLBACK_CHOICES {
                let chosen = config.scrollback == lines;
                let label = if lines >= 1000 { format!("{} 000", lines / 1000) } else { lines.to_string() };
                if ui::chip(ui, theme, &label, None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("scrollback", Some(lines.to_string())));
                }
            }
        });
    }

    fn keyboard(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, input: &InputSettings, changes: &mut Vec<Change>) {
        heading(ui, "ON SCREEN", "Keys along the foot of the desktop, for a touchscreen or without a keyboard.");
        if switch_row(ui, theme, "On-screen keyboard", "Types into the terminal and desktop fields. Configure its key mapping in Shortcuts.", config.osk) {
            changes.push(Change::Set("osk", (!config.osk).then(|| "on".to_string())));
        }
        ui.add_space(24.0);
        heading(ui, "LAYOUT", "Which characters the keys produce.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let default = input.keyboard_layout.is_empty();
            if ui::chip(ui, theme, "SYSTEM DEFAULT", None, default).clicked() && !default {
                changes.push(Change::Set("keyboard_layout", None));
                changes.push(Change::Set("keyboard_variant", None));
            }
            for (code, name) in LAYOUTS {
                let chosen = input.keyboard_layout == code;
                if ui::chip(ui, theme, name, None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("keyboard_layout", Some(code.to_string())));
                    changes.push(Change::Set("keyboard_variant", None));
                }
            }
        });
        ui.add_space(12.0);
        note(ui, "Another, by its XKB name and optional variant");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let edit = ui.add_sized(
                vec2(200.0_f32.min(ui.available_width()), ui::CONTROL_HEIGHT),
                TextEdit::singleline(&mut self.layout_text)
                    .font(ui::mono(13.0))
                    .text_color(ui::TEXT)
                    .hint_text("us dvorak"),
            );
            let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui::button(ui, theme, "USE", None, false).clicked() || entered) && !self.layout_text.trim().is_empty() {
                let mut words = self.layout_text.split_whitespace();
                changes.push(Change::Set("keyboard_layout", words.next().map(str::to_string)));
                changes.push(Change::Set("keyboard_variant", words.next().map(str::to_string)));
            }
        });
        if !input.keyboard_layout.is_empty() {
            ui.add_space(8.0);
            let variant = if input.keyboard_variant.is_empty() { String::new() } else { format!(", variant {}", input.keyboard_variant) };
            note(ui, &format!("In use: {}{variant}. A name the system does not know is ignored.", input.keyboard_layout));
        }
        ui.add_space(24.0);

        heading(ui, "KEY REPEAT", "What happens while a key is held down.");
        let (low, high) = REPEAT_DELAY_RANGE;
        let frac = (input.repeat_delay - low) as f32 / (high - low) as f32;
        if let Some(frac) = slider_row(ui, theme, "Delay before repeating", frac, &format!("{} ms", input.repeat_delay)) {
            let delay = (low as f32 + frac * (high - low) as f32) / 10.0;
            let delay = delay.round() as u32 * 10;
            if delay != input.repeat_delay {
                changes.push(Change::Set("keyboard_repeat_delay", Some(delay.to_string())));
            }
        }
        let (low, high) = REPEAT_RATE_RANGE;
        let frac = (input.repeat_rate - low) as f32 / (high - low) as f32;
        if let Some(frac) = slider_row(ui, theme, "Repeats per second", frac, &input.repeat_rate.to_string()) {
            let rate = (low as f32 + frac * (high - low) as f32).round() as u32;
            if rate != input.repeat_rate {
                changes.push(Change::Set("keyboard_repeat_rate", Some(rate.to_string())));
            }
        }
        ui.add_space(8.0);
        note(ui, "Keyboard changes reach applications as they are next given the keyboard.");
    }

    fn pointer(&mut self, ui: &mut Ui, theme: &Theme, input: &InputSettings, changes: &mut Vec<Change>) {
        let on_off = |on: bool| Some(if on { "on" } else { "off" }.to_string());
        let speed = |ui: &mut Ui, key: &'static str, value: f32, changes: &mut Vec<Change>| {
            let label = if value.abs() < 0.05 { "normal".to_string() } else { format!("{value:+.1}") };
            if let Some(frac) = slider_row(ui, theme, "Pointer speed", (value + 1.0) / 2.0, &label) {
                let new = ((frac * 2.0 - 1.0) * 10.0).round() / 10.0;
                if new != value {
                    changes.push(Change::Set(key, Some(format!("{new:.1}"))));
                }
            }
        };

        heading(ui, "TOUCHPAD", "");
        if switch_row(ui, theme, "Natural scrolling", "The page follows your fingers, as on a phone", input.touchpad_natural_scroll) {
            changes.push(Change::Set("touchpad_natural_scroll", on_off(!input.touchpad_natural_scroll)));
        }
        if switch_row(ui, theme, "Tap to click", "A light tap counts as a click; two fingers for a right click", input.touchpad_tap) {
            changes.push(Change::Set("touchpad_tap", on_off(!input.touchpad_tap)));
        }
        if switch_row(ui, theme, "Ignore while typing", "Stops a resting palm from moving the pointer", input.touchpad_while_typing_off) {
            changes.push(Change::Set("touchpad_while_typing_off", on_off(!input.touchpad_while_typing_off)));
        }
        ui.add_space(12.0);
        speed(ui, "touchpad_speed", input.touchpad_speed, changes);
        ui.add_space(22.0);

        heading(ui, "MOUSE", "");
        if switch_row(ui, theme, "Natural scrolling", "Reverses the wheel", input.mouse_natural_scroll) {
            changes.push(Change::Set("mouse_natural_scroll", on_off(!input.mouse_natural_scroll)));
        }
        if switch_row(ui, theme, "Left-handed", "Swaps the left and right buttons", input.mouse_left_handed) {
            changes.push(Change::Set("mouse_left_handed", on_off(!input.mouse_left_handed)));
        }
        ui.add_space(12.0);
        speed(ui, "mouse_speed", input.mouse_speed, changes);
        ui.add_space(22.0);

        heading(ui, "HAND CONTROL", "Move the pointer with your hand in front of the webcam.");
        if switch_row(ui, theme, "Hand control", "On from the start of each session; Super+H turns it on or off for the while", input.hand_control) {
            changes.push(Change::Set("hand_control", on_off(!input.hand_control)));
        }
        ui.add_space(12.0);
        speed(ui, "hand_speed", input.hand_speed, changes);
        ui.add_space(8.0);
        note(ui, "Open hand: moves the pointer. Pinch thumb and index: click; keep pinching and move to drag; \
                  hold the pinch still to right-click. Index and middle up: scroll, or sweep sideways for the next workspace. \
                  A fist lifts the hand, to start again elsewhere.");
        ui.add_space(4.0);
        note(ui, "Hold your hand up where the camera sees your wrist. The camera is on only while the status bar says so; \
                  nothing it sees leaves this computer.");
    }

    fn frame(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, panels: Panels, changes: &mut Vec<Change>) {
        heading(ui, "PANELS", "Which parts of the frame are open. The choice is kept for next time.");
        for (panel, open, name, what) in [
            (Panel::Left, panels.left, "Left column", "Clock, system, processor, memory, processes"),
            (Panel::Right, panels.right, "Right column", "Network, globe, displays, controls, keys"),
            (Panel::Bottom, panels.bottom, "File browser", "The folder the terminal is in"),
        ] {
            if switch_row(ui, theme, name, what, open) {
                changes.push(Change::Panel(panel, !open));
            }
        }
        ui.add_space(16.0);
        note(ui, "Toggle panels from the status bar or using your configured shortcuts.");
        ui.add_space(26.0);

        heading(ui, "MINIMIZED SIDE PANELS", "Keep a narrow rail, or hide the side columns completely.");
        ui.horizontal(|ui| {
            for (behavior, label, value) in [
                (des_common::PanelBehavior::Collapse, "COLLAPSE", "collapse"),
                (des_common::PanelBehavior::Hide, "HIDE", "hide"),
            ] {
                let selected = config.side_panel_behavior == behavior;
                if ui::chip(ui, theme, label, None, selected).clicked() && !selected {
                    changes.push(Change::Set("side_panel_behavior", Some(value.to_string())));
                }
            }
        });
        ui.add_space(26.0);

        heading(ui, "DOCK", "Move to the bottom edge to show open and pinned apps. Configure dock keyboard navigation in Shortcuts.");
        if switch_row(ui,theme,"Show windows while dragging","Hold over a window to open it, then drop inside. Applies to desktop data drags.",config.drag_overview) {
            changes.push(Change::Set("drag_overview",Some(if config.drag_overview {"off"}else{"on"}.into())));
        }
        let capable=ui.ctx().data(|data|data.get_temp::<bool>(egui::Id::new("window-shelf-capable"))).unwrap_or(false);
        if !capable {note(ui,"Live previews and drag destinations become available with the updated compositor.");}
        if switch_row(ui, theme, "Animate", "Windows, workspaces and the dock; turn off for reduced motion", config.animate) {
            changes.push(Change::Set("animate", (!config.animate).then_some("off".to_string())));
        }
        ui.add_space(26.0);

        heading(ui, "COMPANION", "An animal at the foot of the left column, where there is room for one.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let none = crate::pet::kind_named(&config.pet).is_none();
            if ui::chip(ui, theme, "NONE", None, none).clicked() && !none {
                changes.push(Change::Set("pet", Some("off".to_string())));
            }
            for kind in &crate::pet::KINDS {
                let chosen = config.pet == kind.name;
                if ui::chip(ui, theme, &kind.name.to_uppercase(), None, chosen).clicked() && !chosen {
                    changes.push(Change::Set("pet", Some(kind.name.to_string())));
                }
            }
        });
        ui.add_space(8.0);
        note(ui, "It paws at the pointer. The animals are from VS Code Pets.");
    }

    fn displays(&mut self, ui: &mut Ui, theme: &Theme, displays: Option<&Displays>, changes: &mut Vec<Change>) {
        let Some(displays) = displays.filter(|displays| !displays.monitors.is_empty()) else {
            heading(ui, "DISPLAYS", "");
            note(ui, "Displays can be set when des-ui runs as your login session.");
            return;
        };
        // Start over from what is real whenever that changes underneath the draft.
        if self.draft_basis != displays.monitors {
            self.draft = current_choices(&displays.monitors);
            self.draft_basis = displays.monitors.clone();
        }
        self.selected_display = self.selected_display.min(self.draft.len().saturating_sub(1));

        // A change being tried comes first: it is the one thing that cannot wait.
        if displays.pending > 0 {
            ui::tile(ui, theme, true, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let seconds = displays.pending;
                    let plural = if seconds == 1 { "" } else { "S" };
                    ui::tracked(ui, &format!("KEEP THIS LAYOUT?  GOING BACK IN {seconds} SECOND{plural}"), ui::display_bold(13.0), ui::TEXT, 1.5);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui::button(ui, theme, "GO BACK", None, false).clicked() {
                            changes.push(Change::RevertDisplays);
                        }
                        if ui::button(ui, theme, "KEEP", None, true).clicked() {
                            changes.push(Change::KeepDisplays);
                        }
                    });
                });
            });
            ui.add_space(18.0);
        }

        heading(ui, "ARRANGEMENT", "As they stand on your desk, left to right. Click one to change it.");
        // The monitors to scale, in a row.
        // Each is drawn the shape it has on the desktop: turned, and as large as its
        // scale leaves it. Those that are no display of their own keep a place here,
        // so that they can be chosen and changed back.
        let shape = |choice: &DisplayChoice| {
            let (width, height) = choice.setup.logical(choice.mode);
            (width as f32, height as f32)
        };
        let total: f32 = self.draft.iter().map(|choice| shape(choice).0).sum();
        let tallest = self.draft.iter().map(|choice| shape(choice).1).fold(1.0, f32::max);
        let scale = ((ui.available_width() - 8.0 * self.draft.len() as f32) / total.max(1.0)).min(130.0 / tallest);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (index, choice) in self.draft.iter().enumerate() {
                let size = vec2(shape(choice).0 * scale, shape(choice).1 * scale).max(vec2(92.0, 64.0));
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                let chosen = index == self.selected_display;
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, if chosen { theme.tint() } else { ui::PANEL });
                let line = if chosen || response.has_focus() { theme.main } else if response.hovered() { ui::SECONDARY } else { ui::LINE_STRONG };
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), StrokeKind::Inside);
                let lit = choice.setup.usage != Usage::Off;
                painter.text(rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, &choice.name, ui::mono(12.0), if lit { ui::TEXT } else { ui::LABEL });
                let detail = format!("{} × {}", choice.mode.width, choice.mode.height);
                painter.text(rect.left_top() + vec2(10.0, 26.0), Align2::LEFT_TOP, detail, ui::mono(11.0), if lit { ui::SECONDARY } else { ui::LABEL });
                let role = match choice.setup.usage {
                    Usage::Mirror => Some("MIRROR"),
                    Usage::Off => Some("OFF"),
                    Usage::Own => None,
                };
                if let Some(role) = role {
                    let label = ui::galley(ui, role, ui::display_bold(11.0), ui::SECONDARY, 2.0);
                    painter.galley(rect.left_bottom() + vec2(10.0, -8.0 - label.size().y), label, ui::SECONDARY);
                } else if choice.main {
                    let label = ui::galley(ui, "MAIN", ui::display_bold(11.0), theme.main, 2.0);
                    painter.galley(rect.left_bottom() + vec2(10.0, -8.0 - label.size().y), label, theme.main);
                }
                if response.clicked() {
                    self.selected_display = index;
                }
            }
        });
        ui.add_space(14.0);

        let selected = self.selected_display;
        let Some(monitor) = displays.monitors.iter().find(|monitor| monitor.name == self.draft[selected].name) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if self.draft.len() > 1 {
                if selected > 0 && ui::button(ui, theme, "MOVE LEFT", None, false).clicked() {
                    self.draft.swap(selected, selected - 1);
                    self.selected_display = selected - 1;
                }
                if selected + 1 < self.draft.len() && ui::button(ui, theme, "MOVE RIGHT", None, false).clicked() {
                    self.draft.swap(selected, selected + 1);
                    self.selected_display = selected + 1;
                }
                let is_main = self.draft[self.selected_display].main;
                let label = if is_main { "MAIN DISPLAY" } else { "MAKE MAIN" };
                if ui::button(ui, theme, label, None, is_main).clicked() && !is_main {
                    let chosen = self.selected_display;
                    for (index, choice) in self.draft.iter_mut().enumerate() {
                        choice.main = index == chosen;
                    }
                    // The main display is one of its own.
                    self.draft[chosen].setup.usage = Usage::Own;
                }
            }
        });
        if self.draft.len() > 1 {
            ui.add_space(6.0);
            note(ui, "The main display carries the frame and the terminal.");
        }
        ui.add_space(24.0);

        let selected = self.selected_display;
        if self.draft.len() > 1 {
            heading(ui, "USE", &format!("What {} shows.", monitor.name));
            let is_main = self.draft[selected].main;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for (usage, label) in [(Usage::Own, "ITS OWN DESKTOP"), (Usage::Mirror, "MIRROR THE MAIN DISPLAY"), (Usage::Off, "OFF")] {
                    let chosen = self.draft[selected].setup.usage == usage;
                    // The main display stays one: another has to be made main first.
                    if (usage == Usage::Own || !is_main) && ui::chip(ui, theme, label, None, chosen).clicked() {
                        self.draft[selected].setup.usage = usage;
                    }
                }
            });
            if is_main {
                ui.add_space(6.0);
                note(ui, "To mirror or turn off this one, make another the main display first.");
            }
            ui.add_space(22.0);
        }

        heading(ui, "RESOLUTION", &format!("For {}.", monitor.name));
        let mode = self.draft[selected].mode;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for size in sizes(monitor).into_iter().take(14) {
                let chosen = (mode.width, mode.height) == size;
                let label = format!("{} × {}", size.0, size.1);
                if ui::chip(ui, theme, &label, None, chosen).clicked() && !chosen {
                    if let Some(best) = best_mode(monitor, size) {
                        self.draft[selected].mode = best;
                    }
                }
            }
        });
        ui.add_space(22.0);

        heading(ui, "REFRESH RATE", "");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let rates = monitor.modes.iter().filter(|other| (other.width, other.height) == (mode.width, mode.height));
            for rate in rates {
                let chosen = rate.refresh == mode.refresh;
                if ui::chip(ui, theme, &hertz(rate.refresh), None, chosen).clicked() {
                    self.draft[selected].mode = *rate;
                }
            }
        });
        ui.add_space(22.0);

        heading(ui, "SCALE", "How large everything is drawn. Larger suits a dense or distant screen.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for scale in Setup::SCALES {
                let chosen = self.draft[selected].setup.scale == scale;
                if ui::chip(ui, theme, &format!("{scale}%"), None, chosen).clicked() {
                    self.draft[selected].setup.scale = scale;
                }
            }
        });
        ui.add_space(22.0);

        heading(ui, "TURN", "How the monitor itself stands.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for (turn, label) in [(0, "UPRIGHT"), (90, "ON ITS RIGHT SIDE"), (180, "UPSIDE DOWN"), (270, "ON ITS LEFT SIDE")] {
                let chosen = self.draft[selected].setup.turn == turn;
                if ui::chip(ui, theme, label, None, chosen).clicked() {
                    self.draft[selected].setup.turn = turn;
                }
            }
        });
        ui.add_space(26.0);

        let changed = self.draft != current_choices(&displays.monitors);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if changed {
                if ui::button(ui, theme, "APPLY", None, true).clicked() {
                    changes.push(Change::ApplyDisplays(self.draft.clone()));
                }
                if ui::button(ui, theme, "DISCARD", None, false).clicked() {
                    self.draft = current_choices(&displays.monitors);
                }
            }
        });
        ui.add_space(8.0);
        note(ui, if changed {
            "Applying tries the layout for 15 seconds. If you cannot see it to confirm, it goes back by itself."
        } else {
            "This is how the displays are set now."
        });
    }

    fn globe(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, geo: &Snapshot, changes: &mut Vec<Change>) {
        heading(ui, "GLOBE", "");
        if switch_row(ui, theme, "Rotation", "Off holds the globe still on your side of the world, and saves power", config.globe_rotation) {
            changes.push(Change::GlobeRotation(!config.globe_rotation));
        }
        if switch_row(
            ui,
            theme,
            "Look up places online",
            "Asks get.geojs.io where addresses are. It learns which public addresses this machine connects to",
            config.lookups,
        ) {
            changes.push(Change::Lookups(!config.lookups));
        }
        ui.add_space(26.0);

        heading(ui, "YOUR LOCATION", "Latitude, longitude and a name, such as  48.8566, 2.3522, Paris");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let edit = ui.add_sized(
                vec2(360.0_f32.min(ui.available_width()), ui::CONTROL_HEIGHT),
                TextEdit::singleline(&mut self.location_text)
                    .font(ui::mono(13.0))
                    .text_color(ui::TEXT)
                    .hint_text("latitude, longitude, name"),
            );
            if self.location_error {
                ui.painter().rect_stroke(edit.rect, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::CAUTION), StrokeKind::Inside);
            }
            let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if ui::button(ui, theme, "SET", None, false).clicked() || entered {
                match parse_location(&self.location_text) {
                    Some(place) => {
                        self.location_error = false;
                        changes.push(Change::Location(Some(place)));
                    }
                    None => self.location_error = true,
                }
            }
            if config.location.is_some() && ui::button(ui, theme, "GUESS FROM MY ADDRESS", None, false).clicked() {
                self.location_text.clear();
                self.location_error = false;
                changes.push(Change::Location(None));
            }
        });
        ui.add_space(10.0);
        if self.location_error {
            ui::text(ui, "That is not a position: latitude is -90 to 90, longitude -180 to 180.", ui::mono(12.0), ui::CAUTION);
        } else {
            let now = match (&geo.home, geo.home_is_set) {
                (Some(home), true) => format!("Shown at {}, as you set it.", home.label),
                (Some(home), false) => format!("Shown at {}, guessed from your address.", home.label),
                (None, _) if !config.lookups => "Not shown: lookups are off and no position is set.".to_string(),
                (None, _) => "Not known yet.".to_string(),
            };
            note(ui, &now);
        }
    }

    fn sound(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, mon: &SysMon, changes: &mut Vec<Change>) {
        heading(ui, "SOUND & SCREEN", "The volume and brightness keys do the same.");
        let volume = held(&mut self.volume_held, mon.volume());
        let brightness = held(&mut self.brightness_held, mon.brightness);
        let level = |ui: &mut Ui, name: &str, value: Option<u8>| -> Option<u8> {
            let mut picked = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (label, _) = ui.allocate_exact_size(vec2(110.0, 20.0), Sense::hover());
                ui.painter().text(label.left_center(), Align2::LEFT_CENTER, name, ui::display(14.0), ui::TEXT);
                match value {
                    Some(value) => {
                        if let Some(frac) = ui::slider(ui, theme, (ui.available_width() - 60.0).clamp(40.0, 300.0), value.min(100) as f32 / 100.0, name) {
                            let new = (frac * 100.0).round() as u8;
                            if new != value {
                                picked = Some(new);
                            }
                        }
                        ui::text(ui, &format!("{value}"), ui::mono(12.0), ui::TEXT);
                    }
                    None => {
                        ui::text(ui, "not available on this machine", ui::mono(12.0), ui::LABEL);
                    }
                }
            });
            ui.add_space(14.0);
            picked
        };
        if let Some(new) = level(ui, "Volume", volume) {
            self.volume_held = Some((new, Instant::now()));
            changes.push(Change::Volume(new));
        }
        if let Some(new) = level(ui, "Brightness", brightness) {
            // Never all the way down: on many panels that is a black screen.
            let new = new.max(5);
            self.brightness_held = Some((new, Instant::now()));
            changes.push(Change::Brightness(new));
        }
        if let Some(battery) = mon.battery {
            note(ui, &format!("Battery at {battery}%."));
        }
        ui.add_space(22.0);
        self.sound.get_or_insert_with(|| crate::sound::Sound::new(ui.ctx().clone())).show(ui, theme);
        ui.add_space(26.0);
        heading(ui, "NOTIFICATIONS", "");
        if switch_row(ui, theme, "Sounds", "A notification that shows also makes a sound, unless it asks not to", config.notification_sounds) {
            changes.push(Change::Set("notification_sounds", config.notification_sounds.then(|| "off".to_string())));
        }
        if switch_row(ui, theme, "Processor temperature warnings", "Notify when the processor is running hot. Turning this off keeps other alerts enabled", config.processor_temperature_warnings) {
            changes.push(Change::Set("processor_temperature_warnings", config.processor_temperature_warnings.then(|| "off".to_string())));
        }
        // Do not disturb by the clock: what is not urgent waits in the list.
        let schedule = config.dnd_schedule;
        if switch_row(ui, theme, "Quiet hours", "Do not disturb every day between these hours; urgent ones still show", schedule.is_some()) {
            let value = if schedule.is_some() { None } else { Some(crate::notify::schedule_value((22 * 60, 7 * 60))) };
            changes.push(Change::Set("dnd_schedule", value));
        }
        if let Some((from, to)) = schedule {
            let (mut start, mut end) = (from / 60, to / 60);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(egui::RichText::new("From").font(ui::mono(13.0)));
                ui.add(egui::DragValue::new(&mut start).range(0..=23).suffix(":00"));
                ui.label(egui::RichText::new("to").font(ui::mono(13.0)));
                ui.add(egui::DragValue::new(&mut end).range(0..=23).suffix(":00"));
            });
            // Only a change of hour is written, so minutes set by hand survive.
            if (start, end) != (from / 60, to / 60) && start != end {
                changes.push(Change::Set("dnd_schedule", Some(crate::notify::schedule_value((start * 60, end * 60)))));
            }
        }
    }

    /// The user's picture changed: show the new one.
    pub fn forget_picture(&mut self) {
        if let Some(faces) = &mut self.faces {
            faces.current = None;
        }
    }

    /// Choosing the picture the lock and login screens show.
    fn picture(&mut self, ui: &mut Ui, theme: &Theme, changes: &mut Vec<Change>) {
        heading(ui, "YOUR PICTURE", "Shown on the lock screen and the login screen.");
        let user = std::env::var("USER").unwrap_or_default();
        let faces = self.faces.get_or_insert_with(|| Faces::new(ui.ctx()));
        for (path, pixels) in std::mem::take(&mut *faces.read.lock().unwrap()) {
            let texture = ui.ctx().load_texture(format!("face-{}", path.display()), pixels, egui::TextureOptions::LINEAR);
            faces.shown.push((path, texture));
        }
        let current = faces.current.get_or_insert_with(|| crate::greeter::load_picture(ui.ctx(), &user)).clone();
        let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (frame, _) = ui.allocate_exact_size(vec2(84.0, 84.0), Sense::hover());
            crate::material::paint(ui.ctx(), ui.painter(), frame, ui::CONTROL_RADIUS, crate::material::Role::Panel);
            ui.painter().rect_stroke(frame, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
            match &current {
                Some(texture) => {
                    ui.painter().image(texture.id(), frame.shrink(6.0), whole, egui::Color32::WHITE);
                }
                None => {
                    ui.painter().text(frame.center(), Align2::CENTER_CENTER, "NONE", ui::display_bold(11.0), ui::LABEL);
                }
            }
            ui.vertical(|ui| {
                ui.add_space(8.0);
                if current.is_some() {
                    if ui::button(ui, theme, "REMOVE", None, false).clicked() {
                        changes.push(Change::Avatar(None));
                    }
                } else {
                    ui::text(ui, "Without one, a pattern made from your name is drawn.", ui::mono(12.0), ui::LABEL);
                }
            });
        });
        ui.add_space(14.0);
        if faces.shown.is_empty() {
            note(ui, "Pictures in ~/Pictures are offered here.");
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for (path, texture) in &faces.shown {
                let (tile, response) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                ui.painter().image(texture.id(), tile, whole, egui::Color32::WHITE);
                let line = if response.hovered() || response.has_focus() { theme.main } else { ui::LINE_STRONG };
                ui.painter().rect_stroke(tile, ui::CONTROL_RADIUS, Stroke::new(1.0, line), StrokeKind::Inside);
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                if response.on_hover_text(name).clicked() {
                    changes.push(Change::Avatar(Some(path.clone())));
                }
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let edit = ui.add_sized(
                vec2(420.0_f32.min((ui.available_width() - 80.0).max(1.0)), ui::CONTROL_HEIGHT),
                egui::TextEdit::singleline(&mut faces.path).font(ui::mono(13.0)).text_color(ui::TEXT).hint_text("or the path to any picture"),
            );
            let entered = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let ready = !faces.path.trim().is_empty();
            if (ui::button(ui, theme, "USE", None, ready).clicked() || entered) && ready {
                let typed = faces.path.trim().to_string();
                let path = match typed.strip_prefix("~/") {
                    Some(rest) => std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
                    None => std::path::PathBuf::from(typed),
                };
                faces.path.clear();
                changes.push(Change::Avatar(Some(path)));
            }
        });
    }

    fn session(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, input: &InputSettings, changes: &mut Vec<Change>) {
        heading(ui, "LIVE TERMINALS", "Keep running commands alive while the graphical shell restarts. Reconnection is automatic within this desktop session.");
        if switch_row(ui, theme, "Keep terminals across shell restarts", "Applies to new terminals. Existing managed terminals remain reconnectable", config.keep_terminal_sessions) {
            changes.push(Change::Set("keep_terminal_sessions", config.keep_terminal_sessions.then(|| "off".to_string())));
        }
        let (managed, total) = self.live_terminal_counts;
        note(ui, &format!("{managed} live terminal tab(s); {} temporary tab(s) that close on shell restart.", total.saturating_sub(managed)));
        note(ui, "Requires the updated desktop session and tmux 3.3+. Logout and reboot end these live sessions.");
        note(ui, "Terminals opened before this feature cannot be converted; save their work before your first restart.");
        ui.add_space(18.0);

        heading(ui, "SAVED SESSION", "Save application placement and terminal folders. Restore keeps existing terminals; documents and commands are not replayed.");
        ui.horizontal_wrapped(|ui| {
            if ui::button(ui, theme, "SAVE SESSION", None, false).clicked() { changes.push(Change::SaveSession); }
            if ui::button(ui, theme, "RESTORE SESSION", None, false).clicked() { changes.push(Change::RestoreSession); }
        });
        ui.add_space(18.0);

        heading(ui, "SESSION", "");
        for (name, value) in [
            ("Version", env!("CARGO_PKG_VERSION")),
            ("Settings file", "~/.config/des-ui/config"),
            ("Session log", "~/.local/state/des-ui/session.log"),
            ("Application output", "~/.local/state/des-ui/apps.log"),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.add_sized(vec2(160.0, 24.0), egui::Label::new(egui::RichText::new(name).font(ui::display(13.0)).color(ui::SECONDARY)));
                ui.add(egui::Label::new(egui::RichText::new(value).font(ui::mono(12.0)).color(ui::TEXT)).wrap());
            });
        }
        ui.add_space(22.0);
        note(ui, "Restart shell is configurable in Shortcuts. Applications and managed terminal sessions stay running.");
        ui.add_space(22.0);
        autostart(ui, theme, changes);
        ui.add_space(22.0);
        self.picture(ui, theme, changes);
        ui.add_space(26.0);

        if ui::button(ui, theme, "LOCK NOW", Some("SUPER L"), false).clicked() {
            changes.push(Change::Lock);
        }
        ui.add_space(8.0);
        note(ui, "Locking hides everything behind your password. Everything keeps running.");
        ui.add_space(14.0);
        note(ui, "Lock by itself when the machine is left alone for");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for minutes in [Some(1), Some(5), Some(10), Some(30), Some(60), None] {
                let chosen = input.lock_after == minutes;
                let label = minutes.map_or("NEVER".to_string(), |minutes| format!("{minutes} MIN"));
                if ui::chip(ui, theme, &label, None, chosen).clicked() && !chosen {
                    let value = minutes.map_or("never".to_string(), |minutes| minutes.to_string());
                    changes.push(Change::Set("lock_after", Some(value)));
                }
            }
        });
        ui.add_space(8.0);
        note(ui, "Not while an application is playing something to watch.");
        ui.add_space(8.0);
        note(ui, "Turn the screen off when the machine is left alone for");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for minutes in [Some(2), Some(5), Some(15), Some(30), None] {
                let chosen = input.screen_off_after == minutes;
                let label = minutes.map_or("NEVER".to_string(), |minutes| format!("{minutes} MIN"));
                if ui::chip(ui, theme, &label, None, chosen).clicked() && !chosen {
                    let value = minutes.map(|minutes| minutes.to_string());
                    changes.push(Change::Set("screen_off_after", value));
                }
            }
        });
        ui.add_space(8.0);
        note(ui, "Any key or movement of the pointer turns it back on.");
        ui.add_space(8.0);
        if switch_row(ui, theme, "Lock before sleep", "When the lid closes or the machine is put to sleep, it wakes to the lock screen", config.lock_on_sleep) {
            changes.push(Change::Set("lock_on_sleep", config.lock_on_sleep.then(|| "off".to_string())));
        }
        ui.add_space(22.0);

        let armed = self.log_out_armed.is_some_and(|at| at.elapsed() < Duration::from_secs(5));
        let label = if armed { "CLICK AGAIN TO LOG OUT" } else { "LOG OUT" };
        if ui::button(ui, theme, label, None, armed).clicked() {
            if armed {
                changes.push(Change::LogOut);
            } else {
                self.log_out_armed = Some(Instant::now());
            }
        }
        if !armed {
            self.log_out_armed = None;
        }
        ui.add_space(8.0);
        note(ui, "Logging out closes every application and terminal tab.");
    }
}

/// Keyboard layouts offered as one click: XKB name, and what to call it.
const LAYOUTS: [(&str, &str); 14] = [
    ("us", "ENGLISH (US)"),
    ("gb", "ENGLISH (UK)"),
    ("fr", "FRENCH"),
    ("de", "GERMAN"),
    ("es", "SPANISH"),
    ("pt", "PORTUGUESE"),
    ("br", "PORTUGUESE (BRAZIL)"),
    ("it", "ITALIAN"),
    ("nl", "DUTCH"),
    ("se", "SWEDISH"),
    ("pl", "POLISH"),
    ("tr", "TURKISH"),
    ("ru", "RUSSIAN"),
    ("ara", "ARABIC"),
];

/// A named slider with its reading. Returns the new position while it is moved.
/// Settings → Session → AUTOSTART: what starts with each session, with a switch each,
/// and a field to add a command. The list is read again every few seconds and after a
/// change, so that what is shown is what is on disk.
fn autostart(ui: &mut Ui, theme: &Theme, changes: &mut Vec<Change>) {
    type Cached = std::sync::Arc<(Instant, Vec<crate::autostart::Listed>)>;
    let list_id = egui::Id::new("settings-autostart-list");
    let text_id = egui::Id::new("settings-autostart-add");
    let cached = ui.data(|data| data.get_temp::<Cached>(list_id));
    let listed = match cached.filter(|cached| cached.0.elapsed() < Duration::from_secs(3)) {
        Some(cached) => cached,
        None => {
            let fresh: Cached = std::sync::Arc::new((Instant::now(), crate::autostart::list(&crate::autostart::Dirs::from_env())));
            ui.data_mut(|data| data.insert_temp(list_id, fresh.clone()));
            fresh
        }
    };
    heading(ui, "AUTOSTART", "Programs that start with every session, from ~/.config/autostart and the system's own list. Turning one off keeps a note of it in your folder.");
    let mut changed = false;
    if listed.1.is_empty() {
        note(ui, "Nothing starts with the session.");
    }
    for entry in &listed.1 {
        let what = entry.blocked.map_or(if entry.on { "Starts with the session" } else { "Off" }, crate::autostart::Skip::reason);
        if switch_row(ui, theme, &entry.name, what, entry.on) {
            changes.push(Change::Autostart(entry.file_name.clone(), !entry.on));
            changed = true;
        }
    }
    ui.add_space(10.0);
    let mut text = ui.data(|data| data.get_temp::<String>(text_id)).unwrap_or_default();
    let mut add = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let width = (ui.available_width() - 110.0).clamp(160.0, 420.0);
        add = crate::users::field(ui, theme, &mut text, "A command to start, e.g. syncthing serve", false, width);
        add |= ui::button(ui, theme, "ADD", None, false).clicked();
    });
    if add && !text.trim().is_empty() {
        changes.push(Change::AutostartAdd(std::mem::take(&mut text)));
        changed = true;
    }
    ui.data_mut(|data| {
        data.insert_temp(text_id, text);
        if changed {
            data.remove::<Cached>(list_id);
        }
    });
    note(ui, "The command runs as written, without a shell. It starts from the next session.");
}

fn slider_row(ui: &mut Ui, theme: &Theme, name: &str, frac: f32, reading: &str) -> Option<f32> {
    let mut picked = None;
    if ui.available_width() < 440.0 {
        ui.horizontal_wrapped(|ui| {
            ui::text(ui, name, ui::display(14.0), ui::TEXT);
            ui::text(ui, reading, ui::mono(12.0), ui::SECONDARY);
        });
        let width = ui.available_width().clamp(40.0, 280.0);
        picked = ui::slider(ui, theme, width, frac, name);
    } else {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (label, _) = ui.allocate_exact_size(vec2(160.0, 20.0), Sense::hover());
            ui.painter().text(label.left_center(), Align2::LEFT_CENTER, name, ui::display(14.0), ui::TEXT);
            let width = (ui.available_width() - 90.0).clamp(100.0, 280.0);
            picked = ui::slider(ui, theme, width, frac, name);
            ui::text(ui, reading, ui::mono(12.0), ui::TEXT);
        });
    }
    ui.add_space(12.0);
    picked
}

/// A group's title and, under it, what the group is for.
/// The palette in use: the sixteen terminal colours in two rows, then the caution
/// colour and the glow, as squares.
fn swatches(ui: &mut Ui, theme: &Theme) {
    const SIDE: f32 = 18.0;
    const GAP: f32 = 4.0;
    let row = |ui: &mut Ui, colors: &[Color32], caption: &str| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for color in colors {
                let (rect, _) = ui.allocate_exact_size(vec2(SIDE, SIDE), Sense::hover());
                ui.painter().rect_filled(rect, 0.0, *color);
                ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
            }
            ui.add_space(8.0);
            ui::text(ui, caption, ui::mono(10.0), ui::LABEL);
        });
    };
    row(ui, &theme.ansi[..8], "TERMINAL");
    row(ui, &theme.ansi[8..], "BRIGHT");
    row(ui, &[theme.caution, theme.glow], "CAUTION · GLOW");
}

pub(crate) fn heading(ui: &mut Ui, title: &str, about: &str) {
    ui::section(ui, title, |_| {});
    if !about.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(about).font(ui::display(13.0)).color(ui::SECONDARY)).wrap());
    }
    ui.add_space(14.0);
}

pub(crate) fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(ui::display(12.0)).color(ui::SECONDARY)).wrap());
}

/// A named switch with a line of explanation. Returns true when it was clicked.
pub(crate) fn switch_row(ui: &mut Ui, theme: &Theme, name: &str, what: &str, on: bool) -> bool {
    let width = ui.available_width();
    let text_width = (width - 70.0).max(1.0);
    let name = ui.painter().layout(name.to_owned(), ui::display(14.0), ui::TEXT, text_width);
    let explanation = ui.painter().layout(what.to_owned(), ui::display(12.0), ui::SECONDARY, text_width);
    let height = (name.size().y + explanation.size().y + 16.0).max(52.0);
    let (row, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let text_top = row.left_top() + vec2(0.0, 6.0);
    ui.painter().galley(text_top, name.clone(), ui::TEXT);
    ui.painter().galley(text_top + vec2(0.0, name.size().y + 4.0), explanation, ui::SECONDARY);
    let toggle_area = Rect::from_min_max(egui::pos2(row.right() - 60.0, row.top()), row.right_bottom());
    let mut inner = ui.new_child(UiBuilder::new().max_rect(toggle_area).layout(Layout::right_to_left(Align::Center)));
    let response = ui::toggle(&mut inner, theme, on);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), on, name.job.text.as_str()));
    ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, ui::TRACK));
    response.clicked()
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_appearance_tabs_wrap_explanations_and_sliders_on_narrow_screens() {
        for tab in [AppearanceTab::Wallpaper, AppearanceTab::Theme, AppearanceTab::Effects] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let config = Config::default(); let theme = config.resolved_theme();
            let mut settings = Settings::default(); settings.appearance_tab = tab;
            let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 900.0));
            let output = ctx.run(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| settings.page(ui, rect, &theme, |settings, ui| settings.appearance(ui, &theme, &config, &mut Vec::new())));
            });
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape { assert!(text.pos.x + text.galley.size().x <= 321.0, "settings text spills: {}", text.galley.job.text); }
            }
        }
    }
    #[test]
    fn settings_page_keeps_appearance_controls_inside_narrow_and_wide_workspaces() {
        for width in [320.0, 640.0, 1100.0] {
            let ctx = egui::Context::default();
            crate::ui::install_fonts(&ctx, None, None);
            let config = Config::default();
            let theme = config.resolved_theme();
            let mut settings = Settings::default();
            let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 760.0));
            let output = ctx.run(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings.page(ui, rect, &theme, |settings, ui| {
                        assert!(ui.available_width() <= width - 20.0, "content expanded beyond its viewport at {width}: {}", ui.available_width());
                        assert!(ui.available_width() >= width.min(320.0) - 64.0, "settings navigation squeezed the controls at {width}");
                        settings.appearance(ui, &theme, &config, &mut Vec::new());
                    });
                });
            });
            let texts: Vec<_> = output.shapes.iter().filter_map(|shape| if let egui::Shape::Text(text) = &shape.shape { Some(text) } else { None }).collect();
            assert!(texts.iter().any(|text| text.galley.job.text == "Wallpaper"));
            assert!(texts.iter().any(|text| text.galley.job.text == "Theme"));
            assert!(texts.iter().any(|text| text.galley.job.text == "Effects"));
            assert!(!texts.iter().any(|text| text.galley.job.text == "PHOSPHOR"), "effects should have their own tab");
            for text in texts.iter().filter(|text| ["Wallpaper", "Theme", "Effects", "Choose image", "Apply appearance"].contains(&text.galley.job.text.as_str())) {
                assert!(text.pos.x + text.galley.size().x <= width + 1.0, "control spills outside {width}: {}", text.galley.job.text);
            }
        }
    }

    use super::*;

    #[test]
    fn switch_explanations_wrap_and_reserve_their_height() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(280.0, 800.0));
        let description = "Keep the wallpaper visible while using a stronger tint behind controls so every label remains easy to read.";
        let output = ctx.run(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                switch_row(ui, &Theme::load("signal"), "Reduce transparency", description, false);
                note(ui, "Following setting");
            });
        });
        let text = |label: &str| output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text), _ => None,
        }).expect("rendered label");
        let explanation = text(description);
        assert!(explanation.galley.rows.len() > 1, "narrow descriptions wrap");
        assert!(explanation.pos.x + explanation.galley.size().x <= 212.0, "description reserves switch space");
        assert!(text("Following setting").pos.y >= explanation.pos.y + explanation.galley.size().y, "following rows must clear the complete description");
    }

    #[test]
    fn a_switch_is_read_by_the_name_of_its_row() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        ctx.enable_accesskit();
        let theme = Theme::load("signal");
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                switch_row(ui, &theme, "On-screen keyboard", "Keys along the foot", true);
            });
        });
        let tree = output.platform_output.accesskit_update.expect("accesskit is on");
        let switch = tree.nodes.iter().map(|(_, node)| node).find(|node| node.label() == Some("On-screen keyboard")).expect("the switch is named");
        assert_eq!(switch.role(), egui::accesskit::Role::CheckBox);
        assert_eq!(switch.toggled(), Some(egui::accesskit::Toggled::True));
    }

    #[test]
    fn accent_draft_requires_apply_and_reset_reports_default() {
        fn render(settings: &mut Settings, ctx: &egui::Context, config: &Config, events: Vec<egui::Event>, width: f32)
            -> (egui::FullOutput, Vec<Change>) {
            let mut changes = Vec::new();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))),
                events, ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings.theme_appearance(ui, &config.resolved_theme(), config, &mut changes);
                });
            });
            (output, changes)
        }
        fn button(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                    Some(text.pos + text.galley.size() / 2.0),
                _ => None,
            }).unwrap_or_else(|| panic!("missing button {label}"))
        }
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let config = Config { accent: Some([12, 128, 255]), ..Config::default() };
        let mut settings = Settings::default();
        for width in [320.0, 900.0] {
            let (_, changes) = render(&mut settings, &ctx, &config, Vec::new(), width);
            assert!(changes.is_empty());
        }
        settings.accent_draft = Some(([12, 128, 255], [80, 90, 100]));
        let (output, changes) = render(&mut settings, &ctx, &config, Vec::new(), 900.0);
        assert!(changes.is_empty(), "editing the draft does not save");
        let pos = button(&output, "Apply color");
        for pressed in [true, false] {
            let (_, changes) = render(&mut settings, &ctx, &config, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
            }], 900.0);
            if !pressed { assert_eq!(changes, vec![Change::Accent(Some([80, 90, 100]))]); }
        }
        let (output, _) = render(&mut settings, &ctx, &config, Vec::new(), 900.0);
        let pos = button(&output, "Reset to theme");
        for pressed in [true, false] {
            let (_, changes) = render(&mut settings, &ctx, &config, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
            }], 900.0);
            if !pressed { assert_eq!(changes, vec![Change::Accent(None)]); }
        }
    }

    #[test]
    fn the_palette_switch_saves_derived_and_back_and_shows_swatches() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let mut settings = Settings::default();
        let mut render = |config: &Config, events: Vec<egui::Event>| {
            let mut changes = Vec::new();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 900.0))),
                events, ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings.theme_appearance(ui, &config.resolved_theme(), config, &mut changes);
                });
            });
            (output, changes)
        };
        let label = "Derive the palette from the accent";
        for (palette, expected) in [(Palette::Preset, Some("derived".to_string())), (Palette::Derived, None)] {
            let config = Config { palette, ..Config::default() };
            let (output, changes) = render(&config, Vec::new());
            assert!(changes.is_empty());
            let name = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(text.pos + text.galley.size() / 2.0),
                _ => None,
            }).expect("the switch is labelled");
            let filled = output.shapes.iter().filter(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.rect.size() == vec2(18.0, 18.0) && rect.fill.a() == 255)).count();
            assert!(filled >= 18, "{filled} swatches: sixteen terminal colours, caution and glow are shown");
            // The toggle sits at the right end of the row, level with its name.
            let pos = egui::pos2(900.0 - 28.0, name.y + 12.0);
            for pressed in [true, false] {
                let (_, changes) = render(&config, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                }]);
                if !pressed { assert_eq!(changes, vec![Change::Set("palette", expected.clone())]); }
            }
        }
    }

    #[test]
    fn describes_what_a_monitor_offers() {
        let mode = |width, height, refresh| Mode { width, height, refresh };
        let monitor = DisplayInfo {
            name: "HDMI-A-1".into(),
            main: false,
            x: 0,
            y: 0,
            current: mode(1920, 1080, 60000),
            modes: vec![mode(3840, 2160, 30000), mode(1920, 1080, 60000), mode(1920, 1080, 59940), mode(1280, 720, 60000)],
            setup: Setup { scale: 150, turn: 90, usage: Usage::Own },
        };
        assert_eq!(sizes(&monitor), [(3840, 2160), (1920, 1080), (1280, 720)]);
        assert_eq!(best_mode(&monitor, (1920, 1080)), Some(mode(1920, 1080, 60000)));
        assert_eq!(best_mode(&monitor, (800, 600)), None);
        assert_eq!(hertz(60000), "60 Hz");
        assert_eq!(hertz(59940), "59.94 Hz");
        assert_eq!(hertz(60052), "60.05 Hz");
        let choices = current_choices(std::slice::from_ref(&monitor));
        assert_eq!(choices[0].mode, monitor.current);
        assert_eq!(choices[0].setup.logical(monitor.current), (720, 1280), "on its side, at one and a half times");
    }

    #[test]
    fn a_level_just_set_is_held_then_follows_the_system() {
        let mut slot = Some((40, Instant::now()));
        assert_eq!(held(&mut slot, Some(80)), Some(40));
        let mut slot = Some((40, Instant::now() - Duration::from_secs(10)));
        assert_eq!(held(&mut slot, Some(80)), Some(80));
        assert_eq!(slot, None);
        assert_eq!(held(&mut None, None), None);
    }
}
