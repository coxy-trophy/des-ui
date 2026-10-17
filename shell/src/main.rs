//! des-ui: a Rust rewrite of the eDEX-UI sci-fi terminal desktop.

mod agenda;
mod apps;
mod autostart;
mod app_icons;
mod dock;
mod capture;
mod config;
mod control;
mod desktop;
mod geo;
mod health;
mod globe;
mod greeter;
mod launcher;
mod palette;
mod overview;
mod commands;
mod clipboard;
mod file_search;
mod lock;
mod music;
mod notify;
mod oklch;
mod osk;
mod panels;
mod pet;
mod polkit;
mod tray;
mod power;
mod powerup;
mod bootsplash;
mod radio;
mod remote;
mod runtime;
mod workflows;
mod screen_views;
mod scratchpad;
mod quicklook;
mod region;
mod sound;
mod users;
mod settings;
mod session_restore;
mod sensors;
mod widgets;
mod sysmon;
mod term;
mod theme;
mod ui;

use eframe::egui::{
    self, Align, CentralPanel, Event, EventFilter, Frame, Key, Layout, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, ViewportBuilder, ViewportCommand, Visuals, pos2, vec2,
};
use desktop::Desktop;
use des_common::{
    Panels,
    ipc::{Command, DesktopState},
};
use ui::Icon;
use launcher::Launcher;
use panels::FsView;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use sysmon::SysMon;
use term::TermTab;
use theme::Theme;

const MAX_TABS: usize = 9;

const USAGE: &str = "\
Usage: des-ui [OPTIONS]

Options:
  --theme <NAME|FILE>  Theme to use: its accent and terminal colours (default: signal)
  --font <FILE>        Font for prompt symbols (default: an installed Nerd Font, if any)
  --windowed           Run in a window instead of fullscreen
  --launcher           Start with the application launcher open
  --split-terminal[=below]
                       Start with the terminal split in two
  --settings[=SECTION] Start with the settings open, optionally on a section
  --music              Start with the music player open
  --power              Start with the power menu open
                       (appearance, terminal, frame, displays, network, bluetooth,
                       keyboard, mouse, globe, sound, date, users, session)
  --list-apps          Print the applications the launcher would offer, and exit
  --greeter            Run as the login screen (under greetd; alone, as a preview)
  --set-avatar <FILE>  Use this picture on the login screen
  --write-boot-splash <DIR>
                       Write the boot splash's pictures, in your theme's colours
                       (install-boot-splash.sh uses this)
  --set-location <LAT,LON[,NAME]>
                       Mark this place as yours on the globe, instead of the one
                       guessed from your address; 'auto' goes back to guessing
  --geo <on|off>       Whether to look up where addresses are, which tells an online
                       service the public addresses this machine connects to
  --screenshot <FILE>  Save a PPM screenshot after 3 seconds and exit (for testing)
  -h, --help           Show this help";

#[derive(Debug, PartialEq)]
enum LockStep { Wait, Engage, Confirm, Release, Request }

fn reconcile_lock(local_locked: bool, confirmed: bool, unlock_pending: bool, unlocked_ack: bool, compositor: Option<bool>) -> LockStep {
    if unlock_pending {
        return if unlocked_ack { LockStep::Release }
            else if compositor == Some(true) { LockStep::Confirm }
            else { LockStep::Wait };
    }
    match compositor {
        Some(true) if !local_locked => LockStep::Engage,
        Some(true) => LockStep::Confirm,
        Some(false) if local_locked && confirmed => LockStep::Release,
        Some(false) if local_locked => LockStep::Request,
        _ => LockStep::Wait,
    }
}

struct App {
    ctx: egui::Context,
    theme: Theme,
    shell: String,
    tabs: Vec<TermTab>,
    /// The tab in view; with the terminal split, the one on the left or on top.
    active: usize,
    /// The tab in the second pane of a split terminal.
    side: Option<usize>,
    /// Whether that second pane has the keyboard.
    side_focused: bool,
    /// Whether the panes are one above the other rather than side by side.
    stacked: bool,
    mon: SysMon,
    /// The animal at the foot of the left column.
    pet: pet::Pet,
    fs: FsView,
    /// The compositor connection; `None` when running as an ordinary window.
    desktop: Option<Desktop>,
    launcher: Launcher,
    dock: dock::Dock,
    secondary_docks: std::collections::BTreeMap<usize,dock::Dock>,
    secondary_areas: std::collections::BTreeMap<usize,Option<(i32,i32,i32,i32)>>,
    restore: Option<(session_restore::Restore, Vec<apps::AppEntry>, Instant)>,
    restore_placements: std::collections::VecDeque<(session_restore::Placement,String)>,
    restore_workspace: Option<usize>,
    document_openers: Vec<std::sync::mpsc::Receiver<Result<(),String>>>,
    dock_area: Option<Option<(i32, i32, i32, i32)>>,
    settings: settings::Settings,
    music: music::Music,
    notices: notify::Center,
    /// Sleep, and what holds off the lock.
    power: power::Power,
    power_menu: power::Menu,
    /// Screenshots and recordings.
    capture: capture::Capture,
    /// Whether the compositor was last told that something is being watched.
    idle_held: bool,
    /// What the desktop itself gives notice of, as last seen.
    watch: notify::Watch,
    /// Where the compositor was last told the toasts are.
    toast_area: Option<Option<(i32, i32, i32, i32)>>,
    lock: lock::Lock,
    /// Whether the compositor has confirmed the lock now showing.
    lock_confirmed: bool,
    unlock_pending: bool,
    config: config::Config,
    /// The keyboard, mouse and touchpad settings, as last saved.
    input: des_common::input::InputSettings,
    /// The font that supplies prompt symbols, kept for when fonts are set up again.
    symbols: Option<PathBuf>,
    /// Whether the saved panel choice has been sent to the compositor yet.
    panels_synced: bool,
    frame_synced: bool,
    /// Places for the globe, kept current in the background.
    geo: geo::Geo,
    /// Which panels are open when there is no compositor to say.
    local_panels: Panels,
    screenshot: Option<PathBuf>,
    started: Instant,
    screenshot_requested: bool,
    /// The socket `des-ui-ctl` speaks to; `None` when it could not be made.
    control: Option<control::Control>,
    /// What was copied lately, in memory only, for the palette.
    clipboard: clipboard::Clipboard,
    /// The frame powering up after the splash.
    powerup: powerup::PowerUp,
    /// Readings from the user's own programs, for the side panels.
    widgets: widgets::Widgets,
    /// While the terminal in use is in an ssh session, the machine it reached.
    remote: remote::Remote,
    /// The keys along the foot of the shell, for a touchscreen or a missing keyboard.
    osk: osk::Osk,
    /// The terminal that drops from the top of the workspace.
    scratchpad: scratchpad::Scratchpad,
    /// Where the compositor was last told the scratchpad is; `None` when unknown.
    overlay_area: Option<Option<(i32, i32, i32, i32)>>,
    /// A program waiting for a screenshot's file, answered once it is saved.
    pending_shot_replies: Vec<control::Reply>,
    /// A program waiting for a colour picked with one click.
    pending_color: Option<control::Reply>,
    /// Asks for an administrator's password when an application needs one.
    polkit: polkit::Polkit,
    /// The calendar under the clock.
    agenda: agenda::Agenda,
    /// Applications' status icons, for the status strip.
    tray: tray::Tray,
}

impl App {
    fn new(
        ctx: egui::Context,
        theme: Theme,
        config: config::Config,
        symbols: Option<PathBuf>,
        screenshot: Option<PathBuf>,
    ) -> anyhow::Result<App> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let first = TermTab::spawn(&shell, &home, ctx.clone(), config.scrollback)?;
        let desktop = Desktop::connect(ctx.clone());
        let desktop_present = desktop.is_some();
        let music = music::Music::new(ctx.clone());
        let capture = capture::Capture::new(ctx.clone());
        let power = power::Power::start(ctx.clone(), desktop.is_some());
        let mut notices = notify::Center::new(ctx.clone(), config.do_not_disturb, true);
        let control = match control::Control::start(ctx.clone()) {
            Ok(control) => Some(control),
            Err(error) => {
                eprintln!("des-ui: no control socket: {error}");
                None
            }
        };
        let clipboard = clipboard::Clipboard::start(ctx.clone());
        let widgets = widgets::Widgets::new(ctx.clone());
        let tray = tray::Tray::start(ctx.clone());
        let polkit = polkit::Polkit::start(ctx.clone());
        let agenda = agenda::Agenda::start(ctx.clone());
        // What the user asked to start at login, once per session, and only in one.
        if desktop_present {
            autostart::start();
        }
        let mut osk = osk::Osk::default();
        osk.open = config.osk;
        let mut launcher = Launcher::default();
        launcher.set_clipboard(clipboard.handle());
        notices.sounds = config.notification_sounds;
        notices.schedule = config.dnd_schedule;
        region::set_twelve_hour(config.twelve_hour);
        Ok(App {
            ctx,
            theme,
            shell,
            tabs: vec![first],
            active: 0,
            side: None,
            side_focused: false,
            stacked: false,
            mon: SysMon::new(),
            pet: pet::Pet::new(&config.pet),
            fs: FsView::new(),
            desktop,
            launcher,
            dock: dock::Dock::default(),
            secondary_docks: Default::default(),
            secondary_areas: Default::default(),
            restore: None,
            restore_placements: Default::default(),
            restore_workspace: None,
            document_openers: Vec::new(),
            dock_area: None,
            settings: settings::Settings::default(),
            music,
            notices,
            power,
            power_menu: power::Menu::default(),
            capture,
            idle_held: false,
            watch: notify::Watch::default(),
            toast_area: None,
            lock: lock::Lock::new(),
            lock_confirmed: false,
            unlock_pending: false,
            geo: geo::Geo::start(config.lookups, config.location.clone()),
            local_panels: config.panels,
            config,
            input: des_common::input::InputSettings::load(),
            symbols,
            panels_synced: false,
            frame_synced: false,
            screenshot,
            started: Instant::now(),
            screenshot_requested: false,
            control,
            clipboard,
            widgets,
            remote: remote::Remote::new(),
            osk,
            polkit,
            agenda,
            tray,
            scratchpad: scratchpad::Scratchpad::default(),
            overlay_area: None,
            pending_shot_replies: Vec::new(),
            pending_color: None,
            // Alone in a window there is no splash to wait for.
            powerup: if desktop_present { powerup::PowerUp::under_splash() } else { powerup::PowerUp::new(Duration::ZERO) },
        })
    }

    fn apply_dock_action(&mut self, action: dock::Action) {
        match action {
            dock::Action::Command(command) => {
                self.launcher.open=false;
                self.settings.open=false;
                self.music.open=false;
                self.notices.open=false;
                self.power_menu.open=false;
                self.send(command);
            }
            dock::Action::Launcher => self.toggle_launcher(),
            dock::Action::Unavailable(name) => self.notices.post("Application unavailable", &format!("{name} is no longer installed. Right-click its dock icon to unpin it."),false),
            dock::Action::Launch(entry) => {
                self.launcher.open=false;
                self.settings.open=false;
                self.music.open=false;
                self.notices.open=false;
                self.power_menu.open=false;
                if entry.terminal {
                    self.run_in_terminal(&entry);
                    self.launcher.open=false;
                    self.settings.open=false;
                    self.music.open=false;
                    self.notices.open=false;
                    self.power_menu.open=false;
                    self.send(Command::Terminal);
                } else if let Err(error) = apps::launch(&entry) {
                    self.notices.post("Could not open application", &format!("{}: {error}",entry.name),true);
                }
            }
            action @ (dock::Action::Pin(_,_) | dock::Action::MovePin(_,_)) => {
                let mut pins=self.config.dock_pins.clone();
                match action {
                    dock::Action::Pin(id,true) if !id.is_empty() && !pins.contains(&id) && pins.len()<128 => pins.push(id),
                    dock::Action::Pin(id,false) => pins.retain(|pin| pin!=&id),
                    dock::Action::MovePin(id,left) => {
                        if let Some(index)=pins.iter().position(|pin| pin==&id) {
                            let next=if left { index.saturating_sub(1) } else { (index+1).min(pins.len()-1) };
                            pins.swap(index,next);
                        }
                    }
                    _ => {}
                }
                if pins == self.config.dock_pins { return; }
                match config::set("dock_pins",Some(&serde_json::to_string(&pins).unwrap())) {
                    Ok(_) => self.config.dock_pins=pins,
                    Err(error) => self.notices.post("Could not save dock pins",&error.to_string(),true),
                }
            }
        }
    }

    /// Opens a new tab in the active tab's working directory.
    fn new_tab(&mut self) {
        if self.tabs.len() >= MAX_TABS {
            return;
        }
        let cwd = self
            .tabs
            .get(self.focus_tab())
            .and_then(|t| t.cwd())
            .unwrap_or_else(|| PathBuf::from("/"));
        match TermTab::spawn(&self.shell, &cwd, self.ctx.clone(), self.config.scrollback) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.show_tab(self.tabs.len() - 1);
            }
            Err(e) => eprintln!("des-ui: cannot open tab: {e}"),
        }
    }

    /// The tab that has the keyboard.
    fn focus_tab(&self) -> usize {
        match self.side {
            Some(side) if self.side_focused => side,
            _ => self.active,
        }
    }

    /// Brings a tab into view: in the pane it is already in, or else the pane in use.
    fn show_tab(&mut self, index: usize) {
        if self.side == Some(index) {
            self.side_focused = true;
        } else if index == self.active {
            self.side_focused = false;
        } else if self.side_focused {
            self.side = Some(index);
        } else {
            self.active = index;
        }
    }

    /// Splits the terminal in two, side by side or one above the other, with a new
    /// tab in the second pane; or, when it is split, makes it whole again around the
    /// pane in use. Both tabs stay open either way.
    fn split_terminal(&mut self, stacked: bool) {
        if let Some(side) = self.side.take() {
            if self.side_focused {
                self.active = side;
            }
            self.side_focused = false;
            return;
        }
        let before = self.tabs.len();
        self.side_focused = false;
        let left = self.active;
        self.new_tab();
        // Out of tabs: put one that is already open in the second pane instead.
        let second = if self.tabs.len() > before {
            Some(self.tabs.len() - 1)
        } else {
            (0..self.tabs.len()).find(|index| *index != left)
        };
        self.active = left;
        if let Some(second) = second {
            self.side = Some(second);
            self.side_focused = true;
            self.stacked = stacked;
        }
    }

    /// Opens or closes the settings, which like the launcher take the terminal's place.
    fn toggle_settings(&mut self) {
        if self.settings.open {
            self.settings.open = false;
            self.music.open = false;
            self.notices.open = false;
            self.power_menu.open = false;
            return;
        }
        self.launcher.open = false;
        self.music.open = false;
        self.notices.open = false;
        self.power_menu.open = false;
        self.settings.open(&self.config);
        self.send(Command::Terminal);
    }

    /// Opens or closes the music player, which also takes the terminal's place.
    fn toggle_music(&mut self) {
        self.launcher.open = false;
        self.settings.open = false;
        self.notices.open = false;
        self.power_menu.open = false;
        self.music.toggle();
        if self.music.open {
            self.send(Command::Terminal);
        }
    }

    /// Opens or closes the list of notifications, which also takes the terminal's place.
    fn toggle_notices(&mut self) {
        self.launcher.open = false;
        self.settings.open = false;
        self.music.open = false;
        self.notices.toggle();
        if self.notices.open {
            self.send(Command::Terminal);
        }
    }

    fn toggle_recording(&mut self) {
        if let Some(command) = self.capture.toggle_recording(&mut self.notices) {
            self.send(command);
        }
    }

    /// Opens or closes the power menu, which also takes the terminal's place.
    fn toggle_power(&mut self) {
        self.launcher.open = false;
        self.settings.open = false;
        self.music.open = false;
        self.notices.open = false;
        self.power_menu.toggle();
        if self.power_menu.open {
            self.send(Command::Terminal);
        }
    }

    /// Brings into view the application a notification came from, if it has a window.
    fn follow(&mut self, chosen: notify::Chosen) {
        let name = chosen.app.to_lowercase();
        let index = self.desktop.as_ref().and_then(|desktop| {
            desktop.state.apps.iter().position(|app| !name.is_empty() && app.title.to_lowercase().contains(&name))
        });
        if let Some(index) = index {
            self.notices.open = false;
            self.power_menu.open = false;
            self.send(Command::Activate(index));
        }
    }

    /// Locks the screen, if a password can be checked to unlock it again.
    /// The output of the last command in the terminal in use goes to the clipboard.
    fn copy_last_output(&mut self) {
        let index = self.focus_tab();
        match self.tabs[index].copy_last_output_to_clipboard() {
            Some(lines) => self.notices.post("Copied", &format!("The last command's output: {lines} lines."), false),
            None => self.notices.post("Nothing to copy", "No command has finished in this terminal yet.", false),
        }
    }

    /// Shows or hides the on-screen keyboard, and remembers it for the next start.
    fn toggle_osk(&mut self) {
        self.osk.open = !self.osk.open;
        self.config.osk = self.osk.open;
        if let Err(error) = config::set("osk", self.osk.open.then_some("on")) {
            self.notices.post("Could not save the keyboard setting", &error.to_string(), false);
        }
    }

    fn lock_now(&mut self) {
        self.lock.prepare(&self.ctx);
    }

    fn finish_lock(&mut self) {
        self.unlock_pending = false;
        self.launcher.open = false;
        self.settings.open = false;
        self.music.open = false;
        self.notices.open = false;
        self.power_menu.open = false;
        // Nothing is captured over the lock; whoever waited is told so.
        self.capture.cancel();
        self.answer_held(None, true);
        self.lock.engage();
        self.lock_confirmed = false;
        self.send(Command::Lock);
    }

    fn send(&mut self, command: Command) {
        if let Some(desktop) = &mut self.desktop {
            if let Err(error) = desktop.send(command) {
                eprintln!("des-ui: desktop command could not be sent: {error}");
                self.notices.post("Desktop reconnecting", "Try that action again once the desktop reconnects.", false);
            }
        }
    }

    /// Carries out and saves what the user changed in the settings.
    fn apply(&mut self, ctx: &egui::Context, change: settings::Change) {
        use settings::Change;
        let save = |key: &str, value: Option<&str>| {
            if let Err(e) = config::set(key, value) {
                eprintln!("des-ui: cannot save the setting: {e}");
            }
        };
        match change {
            Change::Theme(name) => {
                let mut preview = self.config.clone();
                preview.theme = name.clone();
                let theme = preview.resolved_theme();
                apply_style(ctx, &theme);
                self.theme = theme;
                save("theme", Some(&name));
                self.config.theme = name;
                // The compositor tints its glow with the theme's accent.
                self.send(Command::ReloadInput);
            }
            Change::Accent(accent) => {
                let value = accent.map(|[r, g, b]| format!("{r}, {g}, {b}"));
                match config::set("accent", value.as_deref()) {
                    Ok(_) => {
                        self.config.accent = accent;
                        self.theme = self.config.resolved_theme();
                        apply_style(ctx, &self.theme);
                    }
                    Err(error) => self.notices.post("Could not save accent", &format!("{error}. Your previous colour is still active."), false),
                }
            }
            Change::FontSize(size) => {
                self.config.font_size = size;
                save("font_size", Some(&size.to_string()));
            }
            Change::GlobeRotation(on) => {
                self.config.globe_rotation = on;
                save("globe_rotation", (!on).then_some("off"));
            }
            Change::Lookups(on) => {
                self.config.lookups = on;
                save("geo", (!on).then_some("off"));
                self.geo = geo::Geo::start(on, self.config.location.clone());
            }
            Change::Location(place) => {
                let value = place.as_ref().map(|p| format!("{}, {}, {}", p.lat, p.lon, p.label));
                save("location", value.as_deref());
                self.config.location = place;
                self.geo = geo::Geo::start(self.config.lookups, self.config.location.clone());
            }
            Change::Panel(panel, open) => {
                let mut panels = self.config.panels;
                match panel {
                    des_common::Panel::Left => panels.left = open,
                    des_common::Panel::Right => panels.right = open,
                    des_common::Panel::Bottom => panels.bottom = open,
                }
                self.config.panels = panels;
                save("panels", Some(&config::Config::panels_value(panels)));
                // The frame follows at once; with a compositor, on its next report.
                self.local_panels = panels;
                self.panels_synced = false;
            }
            Change::Set(key, value) => {
                save(key, value.as_deref());
                // Read everything back, so that what is shown is what was saved.
                let theme = std::mem::take(&mut self.config.theme);
                let font = self.config.terminal_font.clone();
                self.config = config::Config::load();
                self.config.theme = theme;
                self.notices.sounds = self.config.notification_sounds;
                self.notices.schedule = self.config.dnd_schedule;
                region::set_twelve_hour(self.config.twelve_hour);
                self.pet.choose(&self.config.pet);
                self.input = des_common::input::InputSettings::load();
                if self.config.terminal_font != font {
                    ui::install_fonts(ctx, self.symbols.as_deref(), self.config.terminal_font.as_deref());
                }
                if key == "side_panel_behavior" {
                    self.frame_synced = false;
                }
                if key == "osk" {
                    self.osk.open = self.config.osk;
                }
                if matches!(key, "animate" | "glow" | "scanlines" | "night_light") {
                    // The compositor re-reads the setting where it reads the input ones.
                    self.send(Command::ReloadInput);
                }
                if key == "palette" {
                    self.theme = self.config.resolved_theme();
                    apply_style(ctx, &self.theme);
                }
                // The compositor owns the devices these are about.
                if ["keyboard_", "mouse_", "touchpad_", "lock_after", "screen_off_after"].iter().any(|prefix| key.starts_with(prefix)) {
                    self.send(Command::ReloadInput);
                }
            }
            Change::Volume(percent) => {
                let level = format!("{:.2}", percent as f32 / 100.0);
                spawn_quietly("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &level]);
            }
            Change::Brightness(percent) => set_brightness(percent),
            Change::ApplyDisplays(choices) => self.send(Command::ApplyDisplays(choices)),
            Change::KeepDisplays => self.send(Command::KeepDisplays),
            Change::RevertDisplays => self.send(Command::RevertDisplays),
            Change::Lock => self.lock_now(),
            Change::OpenMusic => self.toggle_music(),
            Change::SaveSession => self.save_session(),
            Change::RestoreSession => self.restore_session(),
            Change::Avatar(picture) => {
                let outcome = match &picture {
                    Some(path) => greeter::set_avatar(path).map(|_| ()),
                    None => greeter::remove_avatar(),
                };
                match outcome {
                    Ok(()) => {
                        self.lock.forget_picture();
                        self.settings.forget_picture();
                    }
                    Err(why) => self.notices.post("Picture not changed", &why, true),
                }
            }
            Change::Autostart(file_name, on) => {
                if let Err(error) = autostart::set_enabled(&autostart::Dirs::from_env(), &file_name, on) {
                    self.notices.post("Autostart not changed", &format!("{file_name}: {error}"), true);
                }
            }
            Change::AutostartAdd(command_line) => match autostart::add(&autostart::Dirs::from_env(), &command_line) {
                Ok(path) => self.notices.post("Added to autostart", &format!("{}\nIt starts from the next session.", path.display()), false),
                Err(error) => self.notices.post("Not added to autostart", &error.to_string(), true),
            },
            Change::LogOut => ctx.send_viewport_cmd(ViewportCommand::Close),
        }
    }

    /// Opens or closes the launcher. It is drawn where the terminal is, so anything
    /// covering that must make way.
    fn toggle_launcher(&mut self) {
        self.settings.open = false;
        self.music.open = false;
        self.notices.open = false;
        self.power_menu.open = false;
        self.launcher.toggle();
        if self.launcher.open {
            self.send(Command::Terminal);
        }
    }

    /// Runs a terminal application in a tab of its own.
    fn run_in_terminal(&mut self, entry: &apps::AppEntry) {
        // When no more tabs can be opened, this types into the current one.
        self.new_tab();
        let words: Vec<String> = entry.exec.iter().map(|word| panels::shell_quote(word)).collect();
        let line = format!("{}\n", words.join(" "));
        let tab = self.focus_tab();
        self.tabs[tab].input(line.as_bytes());
    }

    /// Implements `--screenshot`: captures one frame, writes it as a PPM, then quits.
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.screenshot else {
            return;
        };
        if !self.screenshot_requested && self.started.elapsed() > Duration::from_secs(3) {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let mut data = format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
            for pixel in &image.pixels {
                data.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
            }
            if let Err(e) = std::fs::write(path, data) {
                eprintln!("des-ui: cannot write {}: {e}", path.display());
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Handles app-level shortcuts and returns the events left over for the terminal.
    fn take_shortcuts(&mut self, ctx: &egui::Context) -> Vec<Event> {
        // The password dialog has the keyboard; no shortcut fires behind it.
        if self.polkit.active() {
            return Vec::new();
        }
        let mut rest = Vec::new();
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            {
                let count = self.tabs.len();
                if modifiers.ctrl && modifiers.shift && *key == Key::Backtick {
                    self.scratchpad.toggle();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::K {
                    self.toggle_osk();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::D {
                    self.dock.reveal_keyboard();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::T {
                    self.new_tab();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::A {
                    self.toggle_launcher();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::S {
                    self.toggle_settings();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::M {
                    self.toggle_music();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::N {
                    self.toggle_notices();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::P {
                    self.toggle_power();
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && *key == Key::Q {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    continue;
                }
                if modifiers.ctrl && modifiers.shift && matches!(key, Key::E | Key::O) {
                    self.split_terminal(*key == Key::O);
                    continue;
                }
                // With the terminal split, Ctrl+Shift and an arrow moves between panes.
                let arrow = matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown);
                if modifiers.ctrl && modifiers.shift && arrow && self.side.is_some() {
                    self.side_focused = matches!(key, Key::ArrowRight | Key::ArrowDown);
                    continue;
                }
                if modifiers.ctrl && *key == Key::Tab {
                    // The next tab for the pane in use, passing over the other pane's.
                    let other = if self.side_focused { Some(self.active) } else { self.side };
                    let step = if modifiers.shift { count - 1 } else { 1 };
                    let mut next = self.focus_tab();
                    for _ in 0..count {
                        next = (next + step) % count;
                        if Some(next) != other {
                            break;
                        }
                    }
                    if Some(next) != other {
                        self.show_tab(next);
                    }
                    continue;
                }
            }
            rest.push(event);
        }
        rest
    }
}

/// Where the tabs in view end up once tabs whose shells exited are removed. `alive`
/// says which tabs remain. Returns the new first-pane tab and second-pane tab; the
/// split ends if either of its tabs is gone.
fn remap_panes(alive: &[bool], active: usize, side: Option<usize>) -> (usize, Option<usize>) {
    let moved = |old: usize| alive.get(old).copied().unwrap_or(false).then(|| alive[..old].iter().filter(|a| **a).count());
    let remaining = alive.iter().filter(|a| **a).count();
    match (moved(active), side.and_then(moved)) {
        (Some(active), side) => (active, side),
        // The first pane's tab is gone: the second pane's takes over.
        (None, Some(side)) => (side, None),
        // Neither is left: the tab that was next to the one in view.
        (None, None) => {
            let before = alive[..active.min(alive.len())].iter().filter(|a| **a).count();
            (before.min(remaining.saturating_sub(1)), None)
        }
    }
}

/// Draws one region of the frame: a panel-coloured rectangle with its own layout,
/// clipped to its bounds so that nothing in it can push its neighbours.
fn region<R>(
    ui: &mut Ui,
    rect: Rect,
    margin: egui::Vec2,
    layout: Layout,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.painter().rect_filled(rect, 0.0, ui::PANEL);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(margin)).layout(layout));
    child.set_clip_rect(rect);
    add(&mut child)
}

/// The layout of the main display on the workspace in view, as the compositor says;
/// a split made some other way still shows as a split.
fn layout_in_view(state: &DesktopState, layouts: &[des_common::ipc::LayoutInfo]) -> des_common::ipc::LayoutKind {
    use des_common::ipc::LayoutKind;
    let said = layouts.iter().find(|layout| layout.workspace == state.workspace && layout.screen == 0).map(|layout| layout.kind);
    let shared = state.apps.iter().any(|app| app.screen == 0 && app.place.slot().is_some_and(|slot| slot > 0));
    match said {
        Some(kind) => kind,
        None if shared => LayoutKind::Split,
        None => LayoutKind::Single,
    }
}

/// What the top bar asks for in one frame.
#[derive(Default)]
struct BarActions {
    command: Option<Command>,
    open_tab: bool,
    split_terminal: bool,
    toggle_launcher: bool,
    find_terminal: bool,
}

/// How one tab in the top bar looks.
struct Tab<'a> {
    /// A two-digit position, for terminal tabs.
    number: Option<String>,
    name: &'a str,
    /// The program running in a terminal tab.
    detail: Option<&'a str>,
    /// The display an application is on, when there are several.
    badge: Option<String>,
    selected: bool,
}

/// Draws a tab: a label, underlined in the accent when it is the one in view.
fn tab(ui: &mut Ui, theme: &Theme, tab: Tab) -> Response {
    let ink = if tab.selected { ui::TEXT } else { ui::SECONDARY };
    let quiet = if tab.selected { ui::SECONDARY } else { ui::LABEL };
    let name_font = if tab.selected { ui::display_bold(13.0) } else { ui::display(13.0) };
    let number = tab
        .number
        .map(|n| ui::galley(ui, &n, ui::display(13.0), if tab.selected { theme.main } else { ui::LABEL }, 0.0));
    let name = ui::galley(ui, &tab.name.to_uppercase(), name_font, ink, 1.5);
    let detail = tab.detail.map(|d| ui::galley(ui, d, ui::mono(11.0), quiet, 0.0));
    let badge_ink = if tab.selected { theme.main } else { ui::SECONDARY };
    let badge = tab.badge.map(|b| ui::galley(ui, &b, ui::mono(10.0), badge_ink, 0.0));

    let gap = 8.0;
    let mut width = name.size().x;
    for part in [&number, &detail].into_iter().flatten() {
        width += part.size().x + gap;
    }
    if let Some(badge) = &badge {
        width += badge.size().x + 10.0 + gap;
    }
    let (rect, response) = ui.allocate_exact_size(vec2(width + 28.0, ui.available_height()), Sense::click());
    let spoken = match tab.detail {
        Some(detail) => format!("{}, {detail}", tab.name),
        None => tab.name.to_string(),
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, tab.selected, &spoken));
    let painter = ui.painter();
    let mut x = rect.left() + 14.0;
    let mut put = |galley: std::sync::Arc<egui::Galley>, color| {
        let size = galley.size();
        painter.galley(pos2(x, rect.center().y - size.y / 2.0), galley, color);
        x += size.x + gap;
    };
    if let Some(number) = number {
        put(number, ui::LABEL);
    }
    put(name, if response.hovered() { ui::TEXT } else { ink });
    if let Some(detail) = detail {
        put(detail, quiet);
    }
    if let Some(badge) = badge {
        let frame = Rect::from_min_size(
            pos2(x, rect.center().y - badge.size().y / 2.0 - 2.0),
            badge.size() + vec2(10.0, 4.0),
        );
        let edge = if tab.selected { theme.main } else { ui::LINE_STRONG };
        painter.rect_stroke(frame, 0.0, Stroke::new(1.0, edge), StrokeKind::Inside);
        painter.galley(frame.min + vec2(5.0, 2.0), badge, badge_ink);
    }
    if tab.selected {
        let underline = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max);
        painter.rect_filled(underline, 0.0, theme.main);
    }
    response
}

impl App {
    /// Draws terminal tabs and workspace controls. Graphical apps live in the dock.
    fn top_bar(&mut self, ui: &mut Ui, theme: &Theme, state: Option<&DesktopState>) -> BarActions {
        let layouts = self.desktop.as_ref().map(|desktop| desktop.layouts.clone());
        let mut actions = BarActions::default();
        ui.spacing_mut().item_spacing.x = 4.0;
        let covered = state.and_then(|s| s.covering()).is_some();
        let terminal_in_use = !covered && !self.launcher.open && !self.settings.open && !self.music.open && !self.notices.open && !self.power_menu.open;

        for i in 0..self.tabs.len() {
            let process = self.tabs[i]
                .foreground_process()
                .unwrap_or_else(|| "shell".to_string());
            let look = Tab {
                number: Some(format!("{:02}", i + 1)),
                name: if i == 0 { "main" } else { "term" },
                detail: Some(&process),
                badge: None,
                selected: (i == self.active || Some(i) == self.side) && terminal_in_use,
            };
            if tab(ui, theme, look).clicked() {
                self.show_tab(i);
                self.launcher.open = false;
                self.settings.open = false;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = false;
                actions.command = Some(Command::Terminal);
            }
        }
        if self.tabs.len() < MAX_TABS {
            let (rect, response) = ui.allocate_exact_size(vec2(32.0, ui.available_height()), Sense::click());
            let ink = if response.hovered() { ui::TEXT } else { ui::SECONDARY };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", ui::mono(16.0), ink);
            if response.on_hover_text("New terminal tab").clicked() {
                actions.open_tab = true;
                self.launcher.open = false;
                actions.command = Some(Command::Terminal);
            }
        }

        if ui::icon_button(ui, theme, Icon::Copy, 28.0, false).on_hover_text("Copy last command output").clicked() { self.copy_last_output(); }
        if ui::icon_button(ui,theme,Icon::Search,28.0,self.tabs[self.focus_tab()].searching()).on_hover_text("Find in terminal (Ctrl+Shift+F)").clicked() { actions.find_terminal=true; actions.command=Some(Command::Terminal); }

        let split_hint = if self.side.is_some() {
            "Make the terminal whole again (Ctrl+Shift+E)"
        } else {
            "Split the terminal: Ctrl+Shift+E side by side, Ctrl+Shift+O one above the other"
        };
        if ui::icon_button(ui, theme, Icon::SplitPane, 28.0, self.side.is_some())
            .on_hover_text(split_hint)
            .clicked()
        {
            actions.split_terminal = true;
            self.launcher.open = false;
            self.settings.open = false;
            self.music.open = false;
            self.notices.open = false;
            self.power_menu.open = false;
            actions.command = Some(Command::Terminal);
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let hint = if self.launcher.open { "Close applications (Esc)" } else { "Applications (Super+Space)" };
            if ui::icon_button(ui, theme, Icon::Apps, 28.0, self.launcher.open).on_hover_text(hint).clicked() {
                actions.toggle_launcher = true;
            }
            if let Some(state) = state {
                // How the main display shares the workspace in view; laid out right to
                // left, so the last of the four comes first.
                let shown = layout_in_view(state, layouts.as_deref().unwrap_or(&[]));
                ui.spacing_mut().item_spacing.x = 2.0;
                for kind in des_common::ipc::LayoutKind::ALL.into_iter().rev() {
                    let hint = match kind {
                        des_common::ipc::LayoutKind::Single => "One application at a time (Super+S makes a split whole)",
                        des_common::ipc::LayoutKind::Split => "Two side by side (Super+S); drag the gap to resize, Super+Alt+Left/Right",
                        des_common::ipc::LayoutKind::Columns => "Three columns side by side (Super+Shift+S goes round the layouts)",
                        des_common::ipc::LayoutKind::Master => "A master and up to four stacked beside it (Super+Enter makes the window in use the master)",
                    };
                    if ui::icon_button(ui, theme, Icon::of_layout(kind), 24.0, shown == kind).on_hover_text(hint).clicked() {
                        actions.command = Some(Command::Layout(kind));
                    }
                }
                ui.spacing_mut().item_spacing.x = 8.0;
                if state.apps.iter().any(|app| app.workspace == state.workspace)
                    && ui::icon_button(ui, theme, Icon::Fullscreen, 28.0, false)
                        .on_hover_text("Let the application cover the whole display (Super+F)")
                        .clicked()
                {
                    actions.command = Some(Command::Fullscreen);
                }
                // The workspaces, last to first since this side fills from the right.
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                for workspace in (0..des_common::ipc::WORKSPACES).rev() {
                    let here = workspace == state.workspace;
                    let count = state.apps.iter().filter(|app| app.workspace == workspace).count();
                    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                    let painter = ui.painter();
                    if here {
                        painter.rect_filled(rect, 0.0, theme.main);
                    } else {
                        let line = if response.hovered() { ui::SECONDARY } else { ui::LINE_STRONG };
                        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), egui::StrokeKind::Inside);
                    }
                    let ink = if here { ui::GROUND } else if count > 0 { ui::TEXT } else { ui::LABEL };
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, (workspace + 1).to_string(), ui::display_bold(12.0), ink);
                    // A mark on those that hold something.
                    if count > 0 && !here {
                        painter.rect_filled(egui::Rect::from_center_size(rect.center_bottom() - vec2(0.0, 4.0), vec2(8.0, 2.0)), 0.0, theme.main);
                    }
                    let about = match count {
                        0 => format!("Workspace {} (Super+{}): empty", workspace + 1, workspace + 1),
                        1 => format!("Workspace {} (Super+{}): 1 application", workspace + 1, workspace + 1),
                        many => format!("Workspace {} (Super+{}): {many} applications", workspace + 1, workspace + 1),
                    };
                    let response = response.on_hover_text(format!("{about}\nShift+click sends the application in use here (Super+Shift+{})", workspace + 1));
                    if response.clicked() {
                        let shift = ui.input(|i| i.modifiers.shift);
                        actions.command = Some(match state.focus.filter(|_| shift) {
                            Some(index) => Command::SendTo(index, workspace),
                            None => Command::Workspace(workspace),
                        });
                    }
                }
            }
        });
        actions
    }
}

/// Finds an installed monospace font that has prompt glyphs (a Nerd Font).
fn find_nerd_font() -> Option<PathBuf> {
    let output = std::process::Command::new("fc-list")
        .arg("--format=%{file}\t%{family}\t%{style}\n")
        .output()
        .ok()?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut candidates: Vec<&str> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (file, family, style) = (fields.next()?, fields.next()?, fields.next()?);
            let nerd = family.contains("Nerd Font") || family.split(',').any(|f| f.ends_with(" NF"));
            (nerd && style.split(',').any(|s| s == "Regular")).then_some(file)
        })
        .collect();
    candidates.sort();
    candidates.first().map(PathBuf::from)
}

/// Runs a helper program without waiting for it or showing what it prints.
fn spawn_quietly(program: &str, args: &[&str]) {
    let spawned = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("des-ui: cannot run {program}: {e}"),
    }
}

/// Sets the backlight to a percentage of its range, through the login service, which
/// lets the active session do that without special permissions.
fn set_brightness(percent: u8) {
    let Some(device) = std::fs::read_dir("/sys/class/backlight")
        .ok()
        .and_then(|mut dir| dir.next()?.ok())
    else {
        return;
    };
    let max: Option<u64> = std::fs::read_to_string(device.path().join("max_brightness"))
        .ok()
        .and_then(|text| text.trim().parse().ok());
    let Some(max) = max else {
        return;
    };
    let target = (max * percent.min(100) as u64 / 100).max(1);
    spawn_quietly(
        "busctl",
        &[
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            "ssu",
            "backlight",
            &device.file_name().to_string_lossy(),
            &target.to_string(),
        ],
    );
}

fn apply_style(ctx: &egui::Context, theme: &Theme) {
    let mut style = (*ctx.style()).clone();
    style.override_font_id = Some(ui::mono(12.0));
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(ui::TEXT);
    visuals.panel_fill = ui::GROUND;
    visuals.window_fill = ui::PANEL;
    visuals.extreme_bg_color = ui::GROUND;
    visuals.selection.bg_fill = theme.selection();
    visuals.selection.stroke = Stroke::new(1.0, theme.main);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, ui::LINE);
    visuals.widgets.hovered.weak_bg_fill = ui::TRACK;
    visuals.widgets.active.weak_bg_fill = ui::LINE;
    visuals.window_stroke = Stroke::new(1.0, ui::LINE_STRONG);
    style.visuals = visuals;
    ctx.set_style(style);
}

fn main() -> eframe::Result {
    let mut windowed = false;
    let mut screenshot = None;
    let mut font = None;
    let mut launcher = false;
    let mut lyrics = false;
    let mut music = false;
    let mut power_menu = false;
    let mut greeter = false;
    let mut locked = false;
    let mut split = None;
    let mut settings: Option<String> = None;
    let mut config = config::Config::load();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--windowed" => windowed = true,
            "--launcher" => launcher = true,
            "--music" => music = true,
            "--music=lyrics" => {
                music = true;
                lyrics = true;
            }
            "--power" => power_menu = true,
            "--greeter" => greeter = true,
            "--locked" => locked = true,
            "--split-terminal" => split = Some(false),
            "--split-terminal=below" => split = Some(true),
            "--unlock" => {
                eprintln!("des-ui: unlock the desktop with your password on the lock screen");
                std::process::exit(2);
            }
            "--write-boot-splash" => {
                let outcome = match args.next() {
                    Some(dir) => {
                        let config = config::Config::load();
                        bootsplash::write(std::path::Path::new(&dir), &config.resolved_theme())
                    }
                    None => Err("--write-boot-splash needs a folder".to_string()),
                };
                match outcome {
                    Ok(count) => println!("Wrote {count} pictures for the boot splash."),
                    Err(e) => {
                        eprintln!("des-ui: {e}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "--set-avatar" => {
                let outcome = match args.next() {
                    Some(file) => greeter::set_avatar(std::path::Path::new(&file)),
                    None => Err("--set-avatar needs a picture file".to_string()),
                };
                match outcome {
                    Ok(path) => println!("Saved to {}. The login screen shows it from now on.", path.display()),
                    Err(e) => {
                        eprintln!("des-ui: {e}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "--settings" => settings = Some(String::new()),
            section if section.starts_with("--settings=") => {
                settings = Some(section["--settings=".len()..].to_string());
            }
            "--set-location" | "--geo" => {
                let (key, value) = match (arg.as_str(), args.next()) {
                    ("--set-location", Some(value)) if value == "auto" => ("location", None),
                    ("--set-location", Some(value)) if geo::parse_location(&value).is_some() => {
                        ("location", Some(value))
                    }
                    ("--geo", Some(value)) if value == "on" => ("geo", None),
                    ("--geo", Some(value)) if value == "off" => ("geo", Some(value)),
                    _ => {
                        eprintln!("des-ui: {arg} needs a valid value\n\n{USAGE}");
                        std::process::exit(2);
                    }
                };
                match config::set(key, value.as_deref()) {
                    Ok(path) => println!("Saved to {}. It takes effect when the shell next starts.", path.display()),
                    Err(e) => {
                        eprintln!("des-ui: cannot save the setting: {e}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "--list-apps" => {
                for entry in apps::scan() {
                    let terminal = if entry.terminal { " (terminal)" } else { "" };
                    println!("{}{terminal}\t{}", entry.name, entry.exec.join(" "));
                }
                return Ok(());
            }
            "--theme" => match args.next() {
                Some(name) => config.theme = name,
                None => {
                    eprintln!("des-ui: --theme needs a value\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            "--font" => match args.next() {
                Some(path) => font = Some(PathBuf::from(path)),
                None => {
                    eprintln!("des-ui: --font needs a value\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            "--screenshot" => match args.next() {
                Some(path) => screenshot = Some(PathBuf::from(path)),
                None => {
                    eprintln!("des-ui: --screenshot needs a value\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("des-ui: unknown option '{other}'\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let theme = config.resolved_theme();
    if greeter {
        let options = eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_title("DEs-UI login")
                .with_app_id("des-ui-greeter")
                .with_inner_size([1280.0, 760.0])
                .with_fullscreen(!windowed),
            ..Default::default()
        };
        return eframe::run_native(
            "des-ui-greeter",
            options,
            Box::new(move |cc| {
                apply_style(&cc.egui_ctx, &theme);
                ui::install_fonts(&cc.egui_ctx, None, None);
                Ok(Box::new(greeter::Greeter::new(theme, screenshot)))
            }),
        );
    }
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("des-ui")
            .with_app_id("des-ui")
            .with_inner_size([1280.0, 760.0])
            .with_fullscreen(!windowed),
        ..Default::default()
    };
    eframe::run_native(
        "des-ui",
        options,
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx, &theme);
            let symbols = font.or_else(find_nerd_font);
            ui::install_fonts(&cc.egui_ctx, symbols.as_deref(), config.terminal_font.as_deref());
            let mut app = App::new(cc.egui_ctx.clone(), theme, config, symbols, screenshot)?;
            if locked {
                app.lock.engage();
            }
            if let Some(stacked) = split {
                app.split_terminal(stacked);
            }
            if let Some(section) = &settings {
                app.settings.open_at(&app.config.clone(), section);
            }
            if launcher {
                app.launcher.open();
            }
            app.music.open = music;
            if lyrics {
                app.music.show_lyrics();
            }
            app.power_menu.open = power_menu;
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{layout_in_view, remap_panes, reconcile_lock, LockStep};
    use des_common::ipc::{DesktopState, Place};

    #[test]
    fn the_layout_chooser_shows_what_the_compositor_says_or_a_split_seen_otherwise() {
        use des_common::ipc::{AppInfo, LayoutInfo, LayoutKind};
        let app = |place| AppInfo { app_id: String::new(), title: String::new(), screen: 0, place, workspace: 1, pinned: false, urgent: false };
        let mut state = DesktopState { workspace: 1, ..DesktopState::default() };
        state.apps = vec![app(Place::Main)];
        assert_eq!(layout_in_view(&state, &[]), LayoutKind::Single);
        state.apps.push(app(Place::Side));
        assert_eq!(layout_in_view(&state, &[]), LayoutKind::Split);
        let other = LayoutInfo { workspace: 0, screen: 0, kind: LayoutKind::Master, ratio: 0.6, ratio2: 0.5 };
        let here = LayoutInfo { workspace: 1, kind: LayoutKind::Columns, ..other };
        assert_eq!(layout_in_view(&state, &[other]), LayoutKind::Split, "another workspace's layout is not this one's");
        assert_eq!(layout_in_view(&state, &[other, here]), LayoutKind::Columns);
    }

    #[test]
    fn reconnect_unlock_confirmation_does_not_relock_the_session() {
        assert_eq!(reconcile_lock(true, false, true, true, Some(false)), LockStep::Release);
        assert_eq!(reconcile_lock(true, false, true, true, None), LockStep::Release);
    }

    #[test]
    fn cached_unlocked_state_cannot_confirm_a_new_unlock_request() {
        assert_eq!(reconcile_lock(true, false, true, false, Some(false)), LockStep::Wait);
        assert_eq!(reconcile_lock(true, true, true, false, Some(false)), LockStep::Wait);
    }

    #[test]
    fn tabs_in_view_keep_their_places_when_others_close() {
        // Nothing closed.
        assert_eq!(remap_panes(&[true, true, true], 1, Some(2)), (1, Some(2)));
        // A tab to the left closes: both shift down.
        assert_eq!(remap_panes(&[false, true, true], 1, Some(2)), (0, Some(1)));
        // A tab between them closes.
        assert_eq!(remap_panes(&[true, false, true], 0, Some(2)), (0, Some(1)));
    }

    #[test]
    fn a_split_ends_when_one_of_its_tabs_closes() {
        // The second pane's tab closes: whole again, on the first.
        assert_eq!(remap_panes(&[true, true, false], 1, Some(2)), (1, None));
        // The first pane's tab closes: the second pane's tab takes over.
        assert_eq!(remap_panes(&[true, false, true], 1, Some(2)), (1, None));
        // Both close: the nearest tab left.
        assert_eq!(remap_panes(&[true, false, false], 1, Some(2)), (0, None));
        assert_eq!(remap_panes(&[false, false, true], 0, Some(1)), (0, None));
    }

    #[test]
    fn without_a_split_the_tab_in_view_follows_its_neighbours() {
        assert_eq!(remap_panes(&[true, false, true], 1, None), (1, None));
        assert_eq!(remap_panes(&[true, true, false], 2, None), (1, None));
        assert_eq!(remap_panes(&[false, true], 1, None), (0, None));
    }
}
