//! des-ui: a Rust rewrite of the eDEX-UI sci-fi terminal desktop.

mod agenda;
mod apps;
mod autostart;
mod app_icons;
mod dock;
mod capture;
mod companion;
mod config;
mod control;
mod desktop;
mod devices;
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
mod files;
mod file_ops;
mod lock;
mod music;
mod notify;
mod oklch;
mod osk;
mod osd;
mod quick;
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
mod session_ready;
mod live_terminals;
mod screen_views;
mod scratchpad;
mod quicklook;
mod region;
mod sound;
mod users;
mod settings;
mod shortcuts;
mod session_restore;
mod sensors;
mod widgets;
mod wallpaper;
mod material;
mod appearance_editor;
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
    files_browser: Option<files::Browser>,
    file_clipboard: Option<files::Clipboard>,
    file_job: Option<files::Task>,
    /// The compositor connection; `None` when running as an ordinary window.
    desktop: Option<Desktop>,
    launcher: Launcher,
    dock: dock::Dock,
    secondary_docks: std::collections::BTreeMap<usize,dock::Dock>,
    secondary_areas: std::collections::BTreeMap<usize,Option<(i32,i32,i32,i32)>>,
    restore: Option<(session_restore::Restore, Vec<apps::AppEntry>, Instant)>,
    restore_placements: std::collections::VecDeque<(session_restore::Placement,String)>,
    restore_workspace: Option<usize>,
    live_terminal_lease: Option<live_terminals::Lease>,
    live_terminal_layout: Option<live_terminals::Layout>,
    live_terminal_save_failed: bool,
    live_terminal_recovery_incomplete: bool,
    readiness: session_ready::Readiness,
    browser_openers: Vec<std::sync::mpsc::Receiver<Result<(), String>>>,
    document_openers: Vec<std::sync::mpsc::Receiver<Result<(),String>>>,
    dock_area: Option<Option<(i32, i32, i32, i32)>>,
    right_apps: bool,
    apps_panel_pending: bool,
    dock_keyboard: std::collections::BTreeMap<usize,bool>,
    settings: settings::Settings,
    shortcut_reference: shortcuts::Reference,
    companion: companion::Companion,
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
    wallpaper: wallpaper::Wallpaper,
    /// The keyboard, mouse and touchpad settings, as last saved.
    input: des_common::input::InputSettings,
    /// The font that supplies prompt symbols, kept for when fonts are set up again.
    symbols: Option<PathBuf>,
    /// Whether the saved panel choice has been sent to the compositor yet.
    panels_synced: bool,
    frame_synced: bool,
    keyboard_reserved: Option<u16>,
    shortcuts_synced: bool,
    /// Places for the globe, kept current in the background.
    geo: geo::Geo,
    world: globe::Explorer,
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
    /// The pull-down of things changed many times a day.
    quick: quick::Quick,
    /// The meter shown when the volume or brightness changes from outside.
    osd: osd::Osd,
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
        let mut terminal_notes = Vec::new();
        let live_terminal_lease = match live_terminals::Lease::acquire() {
            Ok(lease) => lease,
            Err(error) => {
                terminal_notes.push(("Live terminals unavailable", format!("{error}. This window will use temporary terminals that close on restart.")));
                None
            }
        };
        let saved_layout = live_terminal_lease.as_ref().and_then(|lease| match live_terminals::load_in(lease.directory()) {
            Ok(layout) => layout,
            Err(error) => {
                terminal_notes.push(("Terminal layout unavailable", format!("{error}. Running terminals will be recovered in server order.")));
                None
            }
        }).unwrap_or_default();
        // Reconnect existing managed sessions even if the preference was turned off:
        // the setting affects new terminals and must never strand existing work.
        let recovery_started = Instant::now();
        let mut enumeration_failed = false;
        let live: Vec<term::SessionRef> = if live_terminal_lease.is_some() {
            match term::live_sessions() {
                Ok(live) => live,
                Err(error) => { enumeration_failed = true; terminal_notes.push(("Live terminal recovery failed", format!("{error}. Existing work has not been terminated."))); Vec::new() }
            }
        } else { Vec::new() };
        let ids: Vec<String> = live.iter().filter(|session| session.role == term::SessionRole::Tab).map(|session| session.id.clone()).collect();
        let order = saved_layout.order(&ids, MAX_TABS);
        let mut tabs = Vec::new();
        for id in order {
            if recovery_started.elapsed() >= Duration::from_secs(6) {
                terminal_notes.push(("Some terminals are still detached", "Recovery took longer than expected. Their processes remain running; try restarting the shell again.".into()));
                break;
            }
            if let Some(session) = live.iter().find(|session| session.id == id) {
                match TermTab::reattach(session, ctx.clone(), config.scrollback) {
                    Ok(tab) => tabs.push(tab),
                    Err(error) => {
                        terminal_notes.push(("Terminal could not reconnect", format!("{error}. Existing running processes were left alone; try restarting the shell again.")));
                        break;
                    },
                }
            }
        }
        let reconnected = tabs.len();
        if tabs.is_empty() {
            let first = match TermTab::spawn_managed(&shell, &home, ctx.clone(), config.scrollback,
                config.keep_terminal_sessions && live_terminal_lease.is_some() && ids.is_empty(), term::SessionRole::Tab) {
                Ok(tab) => tab,
                Err(error) => {
                    terminal_notes.push(("Live terminal restore unavailable", format!("{error}. Started a temporary terminal; this tab will close on shell restart.")));
                    TermTab::spawn(&shell, &home, ctx.clone(), config.scrollback)?
                }
            };
            tabs.push(first);
        }
        let connected_ids: Vec<String> = tabs.iter().filter_map(|tab| tab.persistent_id.clone()).collect();
        let (active, side) = saved_layout.selected(&connected_ids);
        let active = active.min(tabs.len() - 1);
        let side = side.filter(|index| *index < tabs.len() && *index != active);
        let mut scratchpad = scratchpad::Scratchpad::default();
        let scratch = saved_layout.scratchpad.as_ref().and_then(|id| live.iter().find(|session| &session.id == id && session.role == term::SessionRole::Scratchpad))
            .or_else(|| live.iter().find(|session| session.role == term::SessionRole::Scratchpad));
        if let Some(session) = scratch.filter(|_| recovery_started.elapsed() < Duration::from_secs(6)) {
            match TermTab::reattach(session, ctx.clone(), config.scrollback) {
                Ok(tab) => scratchpad.restore_tab(tab),
                Err(error) => terminal_notes.push(("Scratchpad could not reconnect", format!("{error}. Its running process was left alone."))),
            }
        }
        let live_terminal_recovery_incomplete = enumeration_failed || reconnected < ids.len().min(MAX_TABS)
            || (scratch.is_some() && scratchpad.persistent_id().is_none());
        let desktop = Desktop::connect(ctx.clone());
        let desktop_present = desktop.is_some();
        if config.keep_terminal_sessions && live_terminal_lease.is_none() && desktop_present && terminal_notes.is_empty() {
            terminal_notes.push(("Terminal preservation unavailable", "This desktop has no live terminal backend. These tabs are temporary and close on shell restart. Use the updated compositor with tmux 3.3+ to enable preservation.".into()));
        }
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
        for (title, body) in terminal_notes { notices.post(title, &body, false); }
        if reconnected > 0 { notices.post("Terminals reconnected", &format!("{reconnected} running terminal(s) reconnected. Commands were not restarted."), false); }
        region::set_twelve_hour(config.twelve_hour);
        Ok(App {
            ctx,
            theme,
            shell,
            tabs,
            active,
            side,
            side_focused: saved_layout.side_focused && side.is_some(),
            stacked: saved_layout.stacked,
            mon: SysMon::new(),
            pet: pet::Pet::new(&config.pet),
            fs: FsView::new(),
            files_browser: None,
            file_clipboard: None,
            file_job: None,
            desktop,
            launcher,
            dock: dock::Dock::default(),
            secondary_docks: Default::default(),
            secondary_areas: Default::default(),
            restore: None,
            restore_placements: Default::default(),
            restore_workspace: None,
            live_terminal_lease,
            live_terminal_layout: None,
            live_terminal_save_failed: false,
            live_terminal_recovery_incomplete,
            readiness: session_ready::Readiness::from_env(),
            browser_openers: Vec::new(),
            document_openers: Vec::new(),
            dock_area: None,
            right_apps: false,
            apps_panel_pending: false,
            dock_keyboard: Default::default(),
            settings: settings::Settings::default(),
            shortcut_reference: Default::default(),
            companion: Default::default(),
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
            world: globe::Explorer::default(),
            local_panels: config.panels,
            config,
            wallpaper: Default::default(),            input: des_common::input::InputSettings::load(),
            symbols,
            panels_synced: false,
            frame_synced: false,
            keyboard_reserved: None,
            shortcuts_synced: false,
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
            quick: quick::Quick::default(),
            osd: osd::Osd::default(),
            tray,
            scratchpad,
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
                self.settings.open=false;self.files_browser=None;
                self.music.open=false;
                self.notices.open=false;
                self.power_menu.open=false;
                self.send(command);
            }
            dock::Action::Launcher => self.toggle_launcher(),
            dock::Action::Unavailable(name) => self.notices.post("Application unavailable", &format!("{name} is no longer installed. Right-click it in Apps to unpin it."),false),
            dock::Action::Launch(entry) => {
                self.launcher.open=false;
                self.settings.open=false;self.files_browser=None;
                self.music.open=false;
                self.notices.open=false;
                self.power_menu.open=false;
                if entry.terminal {
                    self.run_in_terminal(&entry);
                    self.launcher.open=false;
                    self.settings.open=false;self.files_browser=None;
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
                    Err(error) => self.notices.post("Could not save pinned apps",&error.to_string(),true),
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
        let preserve = self.config.keep_terminal_sessions && self.live_terminal_lease.is_some();
        match TermTab::spawn_managed(&self.shell, &cwd, self.ctx.clone(), self.config.scrollback,
            preserve, term::SessionRole::Tab) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.show_tab(self.tabs.len() - 1);
            }
            Err(error) if preserve => {
                // Only a fresh interactive shell may fall back. Never replay a
                // terminal command or replace an existing managed session.
                match TermTab::spawn(&self.shell, &cwd, self.ctx.clone(), self.config.scrollback) {
                    Ok(tab) => {
                        self.tabs.push(tab);
                        self.show_tab(self.tabs.len() - 1);
                        self.notices.post("Temporary terminal opened", &format!("Live terminal backend: {error}. This tab will close on shell restart. Existing terminals were left alone."), false);
                    }
                    Err(fallback) => self.notices.post("Terminal not opened", &format!("{error}. Temporary terminal also failed: {fallback}. Existing terminals are still running."), false),
                }
            }
            Err(error) => self.notices.post("Terminal not opened", &format!("{error}. Existing terminals are still running."), false),
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
        self.files_browser = None;
        self.world.expanded = false;
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
        self.companion.blur_text();
        self.world.expanded = false;
        if self.settings.open {
            self.settings.appearance_editor.reset(&self.config.appearance);
            self.settings.open = false; self.files_browser = None;
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
        self.companion.blur_text();
        self.world.expanded = false;
        self.launcher.open = false;
        self.settings.open = false; self.files_browser = None;
        self.notices.open = false;
        self.power_menu.open = false;
        self.music.toggle();
        if self.music.open {
            self.send(Command::Terminal);
        }
    }

    /// Opens or closes the list of notifications, which also takes the terminal's place.
    fn toggle_notices(&mut self) {
        self.companion.blur_text();
        self.world.expanded = false;
        self.launcher.open = false;
        self.settings.open = false; self.files_browser = None;
        self.music.open = false;
        self.notices.toggle();
        if self.notices.open {
            self.send(Command::Terminal);
        }
    }

    fn toggle_recording(&mut self) {
        self.companion.close();
        if let Some(command) = self.capture.toggle_recording(&mut self.notices) {
            self.send(command);
        }
    }

    /// Opens or closes the power menu, which also takes the terminal's place.
    fn toggle_power(&mut self) {
        self.companion.blur_text();
        self.world.expanded = false;
        self.launcher.open = false;
        self.settings.open = false; self.files_browser = None;
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
        self.companion.close();
        self.lock.prepare(&self.ctx);
    }

    fn finish_lock(&mut self) {
        self.unlock_pending = false;
        self.launcher.open = false;
        self.settings.open = false; self.files_browser = None;
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

    fn start_file_drag(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        let uri = control::file_uri(path);
        if !des_common::ipc::drag_uri_ok(&uri) || uri.len() + 12 >= des_common::ipc::MAX_DRAG_LINE {
            self.notices.post("Could not drag file", "The file path is too long to transfer safely.", false);
            return;
        }
        let result = self.desktop.as_mut().filter(|desktop| desktop.connected())
            .ok_or_else(|| std::io::Error::other("Drag files from the installed desktop session"))
            .and_then(|desktop| desktop.send(Command::DragFiles(vec![uri])));
        match result {
            Ok(()) => {
                // The compositor owns the held button now. Its server data source
                // cannot send egui a toolkit drag-end callback, so release local state.
                ctx.stop_dragging();
                ctx.input_mut(|input| input.pointer = Default::default());
            }
            Err(error) => self.notices.post("Could not drag file", &error.to_string(), false),
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
            Change::Appearance(appearance) => {
                if let Err(error) = appearance.validate() {
                    self.notices.post("Could not apply appearance", error, false);
                    return;
                }
                match config::save_appearance(&appearance) {
                    Ok(durability_warning) => {
                        let previous = self.config.appearance.wallpaper.clone();
                        if let Some(view) = self.settings.appearance_editor.cached_view(&appearance) { self.wallpaper.adopt(&appearance, view); }
                        self.settings.appearance_editor.saved(&appearance);
                        self.config.appearance = appearance;
                        self.send(Command::ReloadInput);
                        // If rename durability is uncertain, retain both assets so a
                        // crash cannot roll the config back to an already deleted file.
                        if durability_warning.is_none() && previous != self.config.appearance.wallpaper {
                            if let Some(path) = previous { wallpaper::remove_managed(&path); }
                        }
                        if let Some(error) = durability_warning {
                            self.notices.post("Appearance applied", &format!("The settings were saved, but the disk could not confirm durability: {error}. Check storage before restarting."), false);
                        }
                        ctx.request_repaint();
                    },
                    Err(error) => self.notices.post("Could not save appearance", &format!("{error}. Your previous appearance is still active."), false),
                }
            }
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
                        // The compositor tints its glow with the accent too.
                        self.send(Command::ReloadInput);
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
            Change::Set("shortcuts", value) => {
                let next = match value.as_deref() {
                    Some(value) => match des_common::shortcuts::Bindings::parse(value) {
                        Some(next) => next,
                        None => { self.notices.post("Could not save shortcuts", "The shortcut configuration is invalid. Your previous mappings remain active.", false); return; }
                    },
                    None => Default::default(),
                };
                match config::save_shortcuts(&next) {
                    Ok(warning) => {
                        self.config.shortcuts = next;
                        shortcuts::publish(ctx, &self.config.shortcuts);
                        // Retry after queue pressure or disconnection; the file is
                        // already committed even if the compositor is unavailable.
                        self.shortcuts_synced = self.desktop.as_mut().is_some_and(|desktop| desktop.send(Command::ReloadInput).is_ok());
                        if let Some(error) = warning { self.notices.post("Shortcuts applied", &format!("Mappings are saved, but disk durability could not be confirmed: {error}"), false); }
                    },
                    Err(error) => self.notices.post("Could not save shortcuts", &format!("{error}. Your previous mappings remain active."), false),
                }
            }
            Change::Set("drag_overview", value) => {
                let enabled=value.as_deref()!=Some("off");
                match config::save_drag_overview(enabled) {
                    Ok(warning)=>{
                        self.config.drag_overview=enabled;
                        self.shortcuts_synced=self.desktop.as_mut().is_some_and(|desktop|desktop.send(Command::ReloadInput).is_ok());
                        if let Some(error)=warning {self.notices.post("Dock preference saved",&format!("Disk durability could not be confirmed: {error}"),false);}
                    }
                    Err(error)=>self.notices.post("Could not save dock preference",&format!("{error}. Your previous preference remains active."),false),
                }
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
                if key.starts_with("cursor_") {
                    // Reuse the existing reload retry path if the compositor is
                    // temporarily disconnected; the saved preference remains authoritative.
                    self.shortcuts_synced = self.desktop.as_mut().is_some_and(|desktop| desktop.send(Command::ReloadInput).is_ok());
                }
                if ["keyboard_", "mouse_", "touchpad_", "hand_", "lock_after", "screen_off_after"].iter().any(|prefix| key.starts_with(prefix)) {
                    self.send(Command::ReloadInput);
                }
            }
            Change::Volume(percent) => {
                // The user is setting it; there is no news to announce.
                self.osd.hold(osd::Kind::Volume, Instant::now());
                let level = format!("{:.2}", percent as f32 / 100.0);
                spawn_quietly("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &level]);
            }
            Change::Brightness(percent) => {
                self.osd.hold(osd::Kind::Brightness, Instant::now());
                set_brightness(percent);
            }
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
        self.companion.blur_text();
        self.world.expanded = false;
        self.settings.open = false; self.files_browser = None;
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

    fn summon_voice(&mut self, ctx: &egui::Context) {
        if self.capture.marking() || self.polkit.active() || self.lock.locked {return;}
        self.companion.start_voice(ctx);
    }

    fn toggle_companion(&mut self, ctx: &egui::Context) {
        if self.capture.marking() || self.polkit.active() || self.lock.locked {return;}
        self.shortcut_reference.close();self.quick.open=false;self.scratchpad.close_now();self.dock.reset();
        self.fs.blur();
        self.companion.open(ctx);self.companion.panel_requested=false;
        let bottom=self.desktop.as_ref().map_or(self.local_panels.bottom,|d|d.state.panels.bottom);
        if !bottom {
            if self.desktop.is_some(){self.send(Command::TogglePanel(des_common::Panel::Bottom));}
            else {self.local_panels.bottom=true;}
        }
    }

    fn toggle_shortcut_reference(&mut self) {
        self.companion.blur_text();
        if self.capture.marking() { return; }
        if self.shortcut_reference.open { self.shortcut_reference.close(); }
        else { self.shortcut_reference.open = true; self.quick.open = false; self.dock.reset(); }
    }

    /// Reveal Apps while preserving the compositor's asynchronous panel state.
    fn focus_apps(&mut self, screen: usize) {
        if screen > 0 { self.secondary_docks.entry(screen).or_default().reveal_keyboard(); return; }
        self.right_apps = true;
        self.shortcut_reference.close();
        self.quick.open = false;
        self.scratchpad.close_now();
        self.companion.blur_text();
        self.fs.blur();
        self.dock.reveal_keyboard();
        let open = self.desktop.as_ref().map_or(self.local_panels.right, |d| d.state.panels.right);
        let request_open = !open && !self.apps_panel_pending;
        self.apps_panel_pending = !open;
        if request_open {
            if self.desktop.is_some() { self.send(Command::TogglePanel(des_common::Panel::Right)); }
            else { self.local_panels.right = true; }
        }
    }

    /// Focused editors and modals own input before registry shortcuts.
    fn take_shortcuts(&mut self, ctx: &egui::Context) -> Vec<Event> {
        if self.companion.open || self.dock.captures_keyboard() { return Vec::new(); }
        let files_modal = self.files_browser.as_ref().is_some_and(files::Browser::captures_keyboard) && !self.scratchpad.visible() && !self.quick.open;
        if self.polkit.active() || self.shortcut_reference.open || self.settings.shortcut_capture_active() || files_modal || (self.settings.open && self.settings.appearance_editor.picker_open()) {
            return Vec::new();
        }
        let mut rest = Vec::new();
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Key { key, pressed: true, modifiers, .. } = &event {
                if self.files_browser.is_some() && *key == Key::Escape && !self.scratchpad.visible() {
                    self.files_browser = None;
                    ctx.input_mut(|input| { input.consume_key(egui::Modifiers::NONE, Key::Escape); });
                    continue;
                }
                if self.world.expanded && *key == Key::Escape && !self.scratchpad.visible() {
                    self.world.expanded = false;
                    ctx.input_mut(|input| { input.consume_key(egui::Modifiers::NONE, Key::Escape); });
                    continue;
                }
                // Files retains its standard New folder binding in that workspace.
                if self.files_browser.is_some() && !self.scratchpad.visible() && !self.quick.open && modifiers.ctrl && modifiers.shift && !modifiers.alt && !modifiers.mac_cmd && *key == Key::N { continue; }
                let action = shell_shortcut(&self.config.shortcuts, *key, *modifiers);
                match action {
                    Some("shell.voice_companion") => { self.summon_voice(ctx); return Vec::new(); }
                    Some("shell.companion") => { self.toggle_companion(ctx); return Vec::new(); }
                    Some("shell.help") => { self.toggle_shortcut_reference(); return Vec::new(); }
                    Some("shell.scratchpad") => self.scratchpad.toggle(),
                    Some("shell.keyboard") => self.toggle_osk(),
                    Some("shell.dock") => self.focus_apps(0),
                    Some("shell.new_tab") => self.new_tab(),
                    Some("shell.launcher") => self.toggle_launcher(),
                    Some("shell.settings") => self.toggle_settings(),
                    Some("shell.music") => self.toggle_music(),
                    Some("shell.notifications") => self.toggle_notices(),
                    Some("shell.power") => self.toggle_power(),
                    Some("shell.quit") => { ctx.send_viewport_cmd(ViewportCommand::Close); }
                    Some("shell.split_right") => self.split_terminal(false),
                    Some("shell.split_down") => self.split_terminal(true),
                    Some("shell.focus_left" | "shell.focus_up") if self.side.is_some() => self.side_focused = false,
                    Some("shell.focus_right" | "shell.focus_down") if self.side.is_some() => self.side_focused = true,
                    Some(id @ ("shell.next_tab" | "shell.previous_tab")) => {
                        let count = self.tabs.len();
                        if count > 0 {
                            let other = if self.side_focused { Some(self.active) } else { self.side };
                            let step = if id == "shell.previous_tab" { count - 1 } else { 1 };
                            let mut next = self.focus_tab();
                            for _ in 0..count { next = (next + step) % count; if Some(next) != other { break; } }
                            if Some(next) != other { self.show_tab(next); }
                        }
                    }
                    _ => { rest.push(event); continue; }
                }
                ctx.input_mut(|input| { input.consume_key(*modifiers, *key); });
                continue;
            }
            rest.push(event);
        }
        rest
    }

}

fn shell_shortcut(bindings: &des_common::shortcuts::Bindings, key: Key, modifiers: egui::Modifiers) -> Option<&'static str> {
    let chord = shortcuts::chord_for_event(key, modifiers)?;
    des_common::shortcuts::DEFINITIONS.iter().find(|definition| definition.scope == des_common::shortcuts::Scope::Shell && bindings.matches(definition.id, &chord)).map(|definition| definition.id)
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
    region_surface(ui, rect, margin, layout, 0.0, add)
}

fn frame_region<R>(ui: &mut Ui, rect: Rect, margin: egui::Vec2, layout: Layout, add: impl FnOnce(&mut Ui) -> R) -> R {
    region_surface(ui, rect, margin, layout, des_common::frame::RADIUS, add)
}

fn region_surface<R>(ui: &mut Ui, rect: Rect, margin: egui::Vec2, layout: Layout, radius: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    material::paint(ui.ctx(), ui.painter(), rect, radius, material::Role::Panel);
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
fn tab(ui: &mut Ui, theme: &Theme, tab: Tab, animate: bool) -> Response {
    let ink = if tab.selected { ui::TEXT } else { ui::SECONDARY };
    let quiet = if tab.selected { ui::SECONDARY } else { ui::LABEL };
    let name_font = if tab.selected { ui::display_bold(13.0) } else { ui::display(13.0) };
    let number = tab
        .number
        .map(|n| ui::galley(ui, &n, ui::display(13.0), if tab.selected { theme.main } else { ui::LABEL }, 0.0));
    let name = ui::galley(ui, tab.name, name_font, ink, 0.5);
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
    let tab_surface = rect.shrink2(vec2(2.0, 5.0));
    if tab.selected || response.hovered() || response.has_focus() {
        painter.rect_filled(tab_surface, ui::CONTROL_RADIUS, if tab.selected { theme.tint() } else { ui::TRACK });
    }
    if response.has_focus() { painter.rect_stroke(tab_surface, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
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
    let progress = if animate { ui.ctx().animate_bool_with_time(response.id, tab.selected, 0.14) } else if tab.selected { 1.0 } else { 0.0 };
    if progress > 0.0 {
        let width = (rect.width() - 28.0).max(0.0) * progress;
        let underline = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 2.0), vec2(width, 2.0));
        painter.rect_filled(underline, 0.0, theme.main);
    }
    response
}

impl App {
    /// Draws terminal tabs and workspace controls. Graphical apps live in the right Apps panel.
    fn top_bar(&mut self, ui: &mut Ui, theme: &Theme, state: Option<&DesktopState>) -> BarActions {
        let layouts = self.desktop.as_ref().map(|desktop| desktop.layouts.clone());
        let mut actions = BarActions::default();
        ui.spacing_mut().item_spacing.x = 4.0;
        let covered = state.and_then(|s| s.covering()).is_some();
        let terminal_in_use = !covered && !self.launcher.open && !self.settings.open && self.files_browser.is_none() && !self.music.open && !self.notices.open && !self.power_menu.open;

        // Reserve the right-hand controls first; tabs can scroll in their own clipped area.
        let available = ui.available_rect_before_wrap();
        let controls_width: f32 = if state.is_some() { 342.0 } else { 76.0 };
        let right_width = controls_width.min((available.width() * 0.6).max(0.0));
        let gap = 4.0_f32.min((available.width() - right_width).max(0.0));
        let right_rect = Rect::from_min_max(pos2(available.right() - right_width, available.top()), available.max);
        let left_rect = Rect::from_min_max(available.min, pos2((right_rect.left() - gap).max(available.left()), available.bottom()));
        let active_id = egui::Id::new("terminal-header-active");
        let active_changed = ui.data_mut(|data| {
            let changed = data.get_temp::<usize>(active_id) != Some(self.active);
            data.insert_temp(active_id, self.active);
            changed
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(right_rect));
            egui::ScrollArea::horizontal().id_salt("header-controls").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_min_width(controls_width);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let available = self.tabs.len() < MAX_TABS;
            let hint = if available { "New terminal tab" } else { "Maximum of 9 terminal tabs" };
            if ui.add_enabled(available, egui::Button::new(egui::RichText::new("+").font(ui::mono(16.0))).min_size(vec2(32.0, ui::CONTROL_HEIGHT))).on_hover_text(hint).clicked() {
                actions.open_tab = true;
                self.launcher.open = false;
                actions.command = Some(Command::Terminal);
            }
            let hint = if self.launcher.open { "Close applications (Esc)".into() } else { shortcuts::hint(ui.ctx(), "Applications", "desktop.launcher") };
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
                        des_common::ipc::LayoutKind::Single => "One application at a time",
                        des_common::ipc::LayoutKind::Split => "Two side by side; drag the gap to resize",
                        des_common::ipc::LayoutKind::Columns => "Three columns side by side",
                        des_common::ipc::LayoutKind::Master => "A master and up to four stacked beside it",
                    };
                    if ui::icon_button(ui, theme, Icon::of_layout(kind), 24.0, shown == kind).on_hover_text(hint).clicked() {
                        actions.command = Some(Command::Layout(kind));
                    }
                }
                ui.spacing_mut().item_spacing.x = 8.0;
                if state.apps.iter().any(|app| app.workspace == state.workspace)
                    && ui::icon_button(ui, theme, Icon::Fullscreen, 28.0, false)
                        .on_hover_text(shortcuts::hint(ui.ctx(), "Let the application cover the whole display", "desktop.fullscreen"))
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
                    let name = shortcuts::hint(ui.ctx(), &format!("Workspace {}", workspace + 1), &format!("desktop.workspace_{}", workspace + 1));
                    let about = match count { 0 => format!("{name}: empty"), 1 => format!("{name}: 1 application"), many => format!("{name}: {many} applications") };
                    let send = shortcuts::hint(ui.ctx(), "Shift+click sends the application here", &format!("desktop.send_{}", workspace + 1));
                    let response = response.on_hover_text(format!("{about}\n{send}"));
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
            });
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(left_rect));
            egui::ScrollArea::horizontal().id_salt("terminal-tabs").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal(|ui| {
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
            let persistence = if self.tabs[i].persistent_id.is_some() {
                "Live terminal: reconnects after a graphical-shell restart"
            } else { "Temporary terminal: closes when the graphical shell restarts" };
            let response = tab(ui, theme, look, self.config.terminal_tab_animation && self.config.animate && !self.config.appearance.reduce_motion).on_hover_text(persistence);
            if response.gained_focus() || (active_changed && i == self.active) { response.scroll_to_me(None); }
            if response.clicked() {
                self.show_tab(i);
                self.launcher.open = false;
                self.settings.open = false; self.files_browser = None;
                self.music.open = false;
                self.notices.open = false;
                self.power_menu.open = false;
                actions.command = Some(Command::Terminal);
            }
        }
        if ui::icon_button(ui, theme, Icon::Copy, 28.0, false).on_hover_text("Copy last command output").clicked() { self.copy_last_output(); }
        if ui::icon_button(ui,theme,Icon::Search,28.0,self.tabs[self.focus_tab()].searching()).on_hover_text(shortcuts::hint(ui.ctx(), "Find in terminal", "terminal.find")).clicked() { actions.find_terminal=true; actions.command=Some(Command::Terminal); }

        let split_hint = if self.side.is_some() {
            shortcuts::hint(ui.ctx(), "Make the terminal whole again", "shell.split_right")
        } else {
            format!("{}; {}", shortcuts::hint(ui.ctx(), "Split side by side", "shell.split_right"), shortcuts::hint(ui.ctx(), "Split above and below", "shell.split_down"))
        };
        if ui::icon_button(ui, theme, Icon::SplitPane, 28.0, self.side.is_some())
            .on_hover_text(split_hint)
            .clicked()
        {
            actions.split_terminal = true;
            self.launcher.open = false;
            self.settings.open = false; self.files_browser = None;
            self.music.open = false;
            self.notices.open = false;
            self.power_menu.open = false;
            actions.command = Some(Command::Terminal);
        }

                });
            });
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
    style.override_font_id = Some(ui::display(13.0));
    style.spacing.item_spacing = vec2(ui::SPACE, ui::SPACE);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.interact_size.y = ui::CONTROL_HEIGHT;
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(ui::TEXT);
    visuals.panel_fill = ui::GROUND;
    visuals.window_fill = ui::PANEL;
    visuals.extreme_bg_color = ui::GROUND;
    visuals.selection.bg_fill = theme.selection();
    visuals.selection.stroke = Stroke::new(1.0, theme.main);
    visuals.hyperlink_color = theme.main;
    visuals.faint_bg_color = ui::TRACK;
    visuals.window_corner_radius = egui::CornerRadius::same(ui::SURFACE_RADIUS as u8);
    visuals.menu_corner_radius = egui::CornerRadius::same(ui::CONTROL_RADIUS as u8);
    for widget in [&mut visuals.widgets.noninteractive, &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
        widget.corner_radius = egui::CornerRadius::same(ui::CONTROL_RADIUS as u8);
        widget.fg_stroke = Stroke::new(1.0, ui::TEXT);
        widget.bg_stroke = Stroke::new(1.0, ui::LINE);
        widget.bg_fill = ui::TRACK;
        widget.weak_bg_fill = ui::PANEL;
        widget.expansion = 0.0;
    }
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ui::LINE_STRONG);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, theme.main);
    visuals.widgets.active.bg_fill = theme.tint();
    visuals.widgets.hovered.weak_bg_fill = ui::TRACK;
    visuals.widgets.active.weak_bg_fill = theme.tint();
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
    fn remapped_shell_shortcuts_replace_defaults_and_match_modifiers_exactly() {
        let mut bindings = des_common::shortcuts::Bindings::default();
        let modifiers = eframe::egui::Modifiers::CTRL | eframe::egui::Modifiers::SHIFT;
        assert_eq!(super::shell_shortcut(&bindings, eframe::egui::Key::T, modifiers), Some("shell.new_tab"));
        bindings.set("shell.new_tab", "Ctrl+Shift+Y").unwrap();
        assert_eq!(super::shell_shortcut(&bindings, eframe::egui::Key::T, modifiers), None);
        assert_eq!(super::shell_shortcut(&bindings, eframe::egui::Key::Y, modifiers), Some("shell.new_tab"));
        assert_eq!(super::shell_shortcut(&bindings, eframe::egui::Key::Y, modifiers | eframe::egui::Modifiers::ALT), None);
    }

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
