//! The line protocol between the compositor and the shell, spoken over a Unix socket
//! whose path the compositor passes to the shell in [`SOCKET_ENV`]. Connections
//! are accepted only from the current shell process, identified by kernel peer
//! credentials; shell restarts revoke the previous process's connections.
//!
//! The compositor sends events, one per line:
//! - `state <displays> <open panels, as bits> <0|1 terminal shares the workspace>
//!   <application with the keyboard, or "-" for the terminal> <workspace in view>`
//!   followed by one tab-separated field per application, each
//!   `<display> <place>:<workspace>:<hex app_id>[:<saved place>][:<marks>] <title>`, whenever
//!   any of that changes. A place is `0` hidden, `1` shown, `2` shown in the right half;
//!   the saved place, present only when the compositor retains layouts, is the one an
//!   application hidden on another workspace will have back; the marks are `p` for an
//!   application pinned in a corner above everything (picture in picture), `u` for one
//!   that asked to be brought forward and wants attention, or `pu` for both;
//! - `dock` when the user asks for the dock on the main display, and
//!   `dock-screen <display>` for the one on another display;
//! - `launcher` when the user asks for the application launcher;
//! - `settings` when the user asks for the settings;
//! - `music` when the user asks for the music player;
//! - `notifications` when the user asks for the list of notifications;
//! - `quick` when the user asks for the quick settings, over whatever is in view;
//! - `power` when the user asks for the power menu;
//! - `shot <screen|window|region|pick> <width> <height> <x,y,w,h|-> <file>` when a
//!   screenshot has been taken: the file holds plain pixels, four bytes each, rows
//!   from the top, and the four numbers say where in it the window in use lies;
//! - `shot-failed <request-id|-> <reason>` when capture fails; tagged failures complete
//!   a held control/portal request and `-` identifies a local hotkey;
//! - `recording <on|off|failed>` when a recording starts, stops, or could not start;
//! - `lock` when the user asks for the screen to be locked, which the shell answers
//!   with its own `lock` once it is ready to show the lock screen;
//! - `locked <0|1>` whenever the screen becomes locked or unlocked, and as an
//!   acknowledgement of each `lock` or `unlock` command;
//! - `displays <seconds left to confirm a change, or 0>` followed by one tab-separated
//!   field per monitor, left to right, each `<name> <0|1 main> <x> <y> <mode in use>
//!   <modes it offers, comma-separated>`. A mode is `<width>x<height>@<millihertz>`;
//! - `layouts` followed by one `<workspace>:<display>:<single|split|columns|master>:<first
//!   divider>:<second divider>` for every display and workspace whose layout is not the
//!   plain one, the dividers as shares of the width, whenever any of them changes;
//! - `overview none` when the overview or the window switcher closes, and while one
//!   is open, whenever what it shows changes, `overview <w|s> <display> <selected
//!   picture, or -> <hex filter text, or -> <x y w h of the switcher's strip, or ->`
//!   followed by one tab-separated field per workspace, `w <n> <x> <y> <w> <h> <0|1
//!   in view>`, and per window picture, `t <app> <x> <y> <w> <h>`. `w` is the overview
//!   of every workspace, `s` the switcher; places are in the display's own logical
//!   coordinates. The compositor draws the pictures of the windows; the shell draws
//!   everything else around them, the names and the selection.
//!
//! An application's place in `state` is `0` hidden, `1` the first slot (the whole
//! workspace, or its first column), `2` the second, and `3` to `5` the further slots of
//! a layout with more than two windows.
//!
//! The shell sends, one per line:
//! - `activate <n>`, `close <n>`, `move <n>` (to the next display) for the
//!   application at that index, `workspace <n>` to bring a workspace into view,
//!   `send-to <n>:<workspace>`, and `place <n> <display> <workspace> <place>` to
//!   put an application somewhere outright, as restoring a session does;
//! - `layout <single|split|columns|master>` and `ratio <share>` for the workspace in
//!   use, and `layout-at <workspace>:<display>:<kind>:<share>:<share>` to set one
//!   outright, as restoring a session does;
//! - `pin <n>` to keep the application at that index in a corner above everything,
//!   on every workspace of its display, and `unpin` to put it back where it was;
//! - `terminal`, `split`, `fullscreen`, `panel <left|right|bottom>`;
//! - `overview` to open the overview of every workspace on the display under the
//!   pointer, or to close it;
//! - `lock` and `unlock`;
//! - `toast <x y w h|none>`, `dock <x y w h|none>` and `dock-screen <display>
//!   <x y w h|none>`: where in its window the shell is showing notifications, or
//!   the dock, that are to be seen above any application;
//! - `overlay <x y w h|none>`: a part of the shell's window on the main display to
//!   be seen above any application, such as a drop-down terminal, with the keyboard
//!   going to the shell for as long as it shows;
//! - `drag-files <uri>\t<uri>…` while a file is dragged out of the shell: the
//!   compositor starts a drag that drops those `file://` URIs (at most 64, the line
//!   under 4,000 bytes) where the pointer is let go;
//! - `hold-idle <on|off>` while an application is showing something to watch, so
//!   that the screen is not locked for want of input;
//! - `shot <screen|window|region|pick>`, `record <file>` and `record-stop`;
//! - `reload-input` (the input settings in the configuration file changed) and
//!   `reload-frame` (the side panel behavior changed);
//! - `displays-keep`, `displays-revert`, or `displays-apply` followed by one
//!   `<name>=<mode>` per monitor, left to right, with a `*` after the one that is to
//!   be the main display, then `/<scale>:<turn>:<use>`: the scale in percent, the
//!   turn in degrees, and `o` for a display of its own, `m` for one mirroring the
//!   main display, `x` for one turned off.

use crate::{Panel, Panels};

pub const SOCKET_ENV: &str = "DES_COMP_SOCKET";

/// Where an application is on its display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Hidden,
    /// Filling the workspace, or its first column when it is shared: the master.
    Main,
    /// In the second column of a shared workspace (the right half of a split).
    Side,
    /// In a further slot of a layout with more than two windows, numbered from 2:
    /// the third column, or further down the stack beside the master.
    Extra(u8),
}

impl Place {
    /// The most windows a layout shows on one display: the master and four beside it.
    pub const SLOTS: usize = 5;

    /// The slot it fills, counted from 0 for the master; none when hidden.
    pub fn slot(self) -> Option<usize> {
        match self {
            Place::Hidden => None,
            Place::Main => Some(0),
            Place::Side => Some(1),
            Place::Extra(n) => Some(n as usize),
        }
    }

    /// The place of a slot; slots past the last are the last.
    pub fn of_slot(slot: usize) -> Place {
        match slot {
            0 => Place::Main,
            1 => Place::Side,
            n => Place::Extra(n.min(Place::SLOTS - 1) as u8),
        }
    }

    /// The number it travels as: 0 hidden, 1 main, 2 side, and the slot plus one beyond.
    /// Peers from before the further slots only ever see 0, 1 and 2.
    pub fn code(self) -> u8 {
        match self {
            Place::Hidden => 0,
            other => other.slot().map_or(0, |slot| slot as u8 + 1),
        }
    }

    pub fn from_code(code: u8) -> Option<Place> {
        match code {
            0 => Some(Place::Hidden),
            n if (n as usize) <= Place::SLOTS => Some(Place::of_slot(n as usize - 1)),
            _ => None,
        }
    }
}

/// How the applications in view share a display's workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutKind {
    /// One application at a time.
    #[default]
    Single,
    /// Two side by side.
    Split,
    /// Three side by side: the three used most recently.
    Columns,
    /// The master on the left, up to four stacked on the right.
    Master,
}

impl LayoutKind {
    pub const ALL: [LayoutKind; 4] = [LayoutKind::Single, LayoutKind::Split, LayoutKind::Columns, LayoutKind::Master];

    pub fn word(self) -> &'static str {
        match self {
            LayoutKind::Single => "single",
            LayoutKind::Split => "split",
            LayoutKind::Columns => "columns",
            LayoutKind::Master => "master",
        }
    }

    pub fn from_word(word: &str) -> Option<LayoutKind> {
        LayoutKind::ALL.into_iter().find(|kind| kind.word() == word)
    }

    /// How many applications it shows at most.
    pub fn slots(self) -> usize {
        match self {
            LayoutKind::Single => 1,
            LayoutKind::Split => 2,
            LayoutKind::Columns => 3,
            LayoutKind::Master => Place::SLOTS,
        }
    }

    /// The next one round, as Super+Shift+S goes.
    pub fn next(self) -> LayoutKind {
        let all = LayoutKind::ALL;
        let at = all.iter().position(|kind| *kind == self).unwrap_or(0);
        all[(at + 1) % all.len()]
    }
}

/// The layout of one display on one workspace, and where its dividers are: `ratio`
/// is the first divider as a share of the width, `ratio2` the second, which only
/// three columns have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutInfo {
    pub workspace: usize,
    pub screen: usize,
    pub kind: LayoutKind,
    pub ratio: f32,
    pub ratio2: f32,
}

impl LayoutInfo {
    fn word(&self) -> String {
        format!("{}:{}:{}:{:.3}:{:.3}", self.workspace, self.screen, self.kind.word(), self.ratio, self.ratio2)
    }

    fn from_word(word: &str) -> Option<LayoutInfo> {
        let mut parts = word.split(':');
        let workspace = parts.next()?.parse().ok().filter(|w| *w < WORKSPACES)?;
        let screen = parts.next()?.parse().ok().filter(|s| *s < 32)?;
        let kind = LayoutKind::from_word(parts.next()?)?;
        let ratio = parse_share(parts.next()?)?;
        let ratio2 = parse_share(parts.next()?)?;
        parts.next().is_none().then_some(LayoutInfo { workspace, screen, kind, ratio, ratio2 })
    }

    /// The `layouts` line: every display on every workspace whose layout is not the
    /// plain one, so that the shell can show it and a saved session keep it.
    pub fn encode_all(layouts: &[LayoutInfo]) -> String {
        let mut line = "layouts".to_string();
        for layout in layouts {
            line.push(' ');
            line.push_str(&layout.word());
        }
        line.push('\n');
        line
    }
}

/// A share of a width, strictly between nothing and all of it.
fn parse_share(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|share| share.is_finite() && *share > 0.0 && *share < 1.0)
}

/// How many workspaces there are: sets of applications, one of which is in view.
pub const WORKSPACES: usize = 4;

/// One open application.
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    /// Wayland app_id or X11 WM_CLASS; never inferred from a title.
    pub app_id: String,
    pub title: String,
    /// Index of the display the application lives on; 0 is the one with the frame.
    pub screen: usize,
    /// Where it is on that display; hidden when its workspace is not the one in view.
    pub place: Place,
    /// The workspace it belongs to, counted from 0.
    pub workspace: usize,
    /// Kept in a corner above everything, on every workspace of its display.
    pub pinned: bool,
    /// Asked to be brought forward when it could not be: it wants attention.
    pub urgent: bool,
}

/// The displays and open applications, as the compositor reports them.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopState {
    pub screens: usize,
    pub panels: Panels,
    /// Whether the terminal has only the left half of the workspace, an application
    /// being in the right half.
    pub terminal_split: bool,
    /// The application with the keyboard; `None` when the shell has it.
    pub focus: Option<usize>,
    /// The workspace in view, counted from 0.
    pub workspace: usize,
    pub apps: Vec<AppInfo>,
    /// Layout retained while an application is hidden on an inactive workspace.
    /// Empty for peers predating this optional metadata.
    pub saved_places: Vec<Place>,
}

impl Default for DesktopState {
    fn default() -> Self {
        DesktopState {
            screens: 1,
            panels: Panels::default(),
            terminal_split: false,
            focus: None,
            workspace: 0,
            apps: Vec::new(),
            saved_places: Vec::new(),
        }
    }
}

impl DesktopState {
    /// The application covering the terminal on the main display, if any.
    pub fn covering(&self) -> Option<usize> {
        self.apps
            .iter()
            .position(|app| app.screen == 0 && app.place == Place::Main)
    }

    pub fn encode(&self) -> String {
        let focus = match self.focus {
            Some(index) => index.to_string(),
            None => "-".to_string(),
        };
        let mut line = format!(
            "state {} {} {} {focus} {}",
            self.screens,
            self.panels.bits(),
            self.terminal_split as u8,
            self.workspace
        );
        let saved_wanted = !self.saved_places.is_empty() || self.apps.iter().any(|app| app.pinned || app.urgent);
        for (index, app) in self.apps.iter().enumerate() {
            let place = app.place.code();
            let app_id: String = app.app_id.chars().filter(|c| !c.is_control()).collect();
            let encoded_id: String = app_id.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
            let saved = if !saved_wanted { String::new() } else {
                let value = self.saved_places.get(index).copied().unwrap_or(app.place).code();
                format!(":{value}")
            };
            // Marks: `p` pinned, `u` wanting attention; none at all leaves the field out.
            let marks = format!("{}{}", if app.pinned { "p" } else { "" }, if app.urgent { "u" } else { "" });
            let marks = if marks.is_empty() { marks } else { format!(":{marks}") };
            line.push_str(&format!("\t{} {place}:{}:{encoded_id}{saved}{marks} ", app.screen, app.workspace));
            // Titles come from applications and must not break the framing.
            line.extend(app.title.chars().map(|c| if c.is_control() { ' ' } else { c }));
        }
        line.push('\n');
        line
    }

    pub fn decode(line: &str) -> Option<DesktopState> {
        let mut fields = line.trim_end_matches('\n').split('\t');
        let mut head = fields.next()?.strip_prefix("state ")?.split(' ');
        let screens: usize = head.next()?.parse().ok()?;
        let panels = Panels::from_bits(head.next()?.parse().ok()?)?;
        let terminal_split = match head.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let focus = match head.next()? {
            "-" => None,
            index => Some(index.parse().ok()?),
        };
        // A compositor from before there were workspaces says nothing of them.
        let workspace: usize = match head.next() {
            Some(workspace) => workspace.parse().ok().filter(|workspace| *workspace < WORKSPACES)?,
            None => 0,
        };
        if screens == 0 || head.next().is_some() {
            return None;
        }
        let mut apps = Vec::new();
        let mut saved_places = Vec::new();
        let mut has_saved_places = false;
        for field in fields {
            let mut parts = field.splitn(3, ' ');
            let screen: usize = parts.next()?.parse().ok()?;
            let word = parts.next()?;
            let mut metadata = word.split(':');
            let place_word = metadata.next()?;
            let workspace_word = metadata.next();
            let app_id = match metadata.next() {
                Some(hex) => {
                    if hex.len() % 2 != 0 || !hex.is_ascii() { return None; }
                    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
                    let id = String::from_utf8(bytes?).ok()?;
                    if id.chars().any(char::is_control) { return None; }
                    id
                }
                None => String::new(),
            };
            let (place, on) = match workspace_word.map(|on| (place_word, on)) {
                Some((place, on)) => (place, on.parse::<usize>().ok().filter(|on| *on < WORKSPACES)?),
                None => (word, 0),
            };
            let place = place.parse::<u8>().ok().and_then(Place::from_code)?;
            let saved_place = match metadata.next() {
                Some(code) => {
                    has_saved_places = true;
                    code.parse::<u8>().ok().and_then(Place::from_code)?
                }
                None => place,
            };
            let (pinned, urgent) = match metadata.next() {
                Some("p") => (true, false),
                Some("u") => (false, true),
                Some("pu") => (true, true),
                Some(_) => return None,
                None => (false, false),
            };
            if metadata.next().is_some() { return None; }
            saved_places.push(saved_place);
            if screen >= screens {
                return None;
            }
            apps.push(AppInfo {
                app_id,
                title: parts.next()?.to_string(),
                screen,
                place,
                workspace: on,
                pinned,
                urgent,
            });
        }
        if focus.is_some_and(|index| index >= apps.len()) {
            return None;
        }
        Some(DesktopState {
            screens,
            panels,
            terminal_split,
            focus,
            workspace,
            apps,
            saved_places: if has_saved_places { saved_places } else { Vec::new() },
        })
    }
}

/// A resolution and refresh rate a monitor can run at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz, as monitors report it: 59940 for 59.94 Hz.
    pub refresh: u32,
}

impl Mode {
    pub fn encode(&self) -> String {
        format!("{}x{}@{}", self.width, self.height, self.refresh)
    }

    pub fn decode(text: &str) -> Option<Mode> {
        let (size, refresh) = text.split_once('@')?;
        let (width, height) = size.split_once('x')?;
        let mode = Mode {
            width: width.parse().ok()?,
            height: height.parse().ok()?,
            refresh: refresh.parse().ok()?,
        };
        (mode.width > 0 && mode.height > 0 && mode.refresh > 0).then_some(mode)
    }
}

/// What a monitor is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Usage {
    /// A display of its own, beside the others.
    #[default]
    Own,
    /// Showing what the main display shows.
    Mirror,
    Off,
}

/// How a monitor is set beyond its mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    /// How large things are drawn, in percent: 100 is one to one.
    pub scale: u32,
    /// Quarter turns clockwise, in degrees: 0, 90, 180 or 270.
    pub turn: u16,
    pub usage: Usage,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { scale: 100, turn: 0, usage: Usage::Own }
    }
}

impl Setup {
    /// The scales on offer.
    pub const SCALES: [u32; 5] = [100, 125, 150, 175, 200];

    pub fn encode(&self) -> String {
        let usage = match self.usage {
            Usage::Own => 'o',
            Usage::Mirror => 'm',
            Usage::Off => 'x',
        };
        format!("{}:{}:{usage}", self.scale, self.turn)
    }

    pub fn decode(text: &str) -> Option<Setup> {
        let mut parts = text.split(':');
        let scale: u32 = parts.next()?.parse().ok()?;
        let turn: u16 = parts.next()?.parse().ok()?;
        let usage = match parts.next()? {
            "o" => Usage::Own,
            "m" => Usage::Mirror,
            "x" => Usage::Off,
            _ => return None,
        };
        (parts.next().is_none() && (50..=400).contains(&scale) && matches!(turn, 0 | 90 | 180 | 270)).then_some(Setup { scale, turn, usage })
    }

    /// The size a mode comes to on the desktop: turned, then scaled.
    pub fn logical(&self, mode: Mode) -> (i32, i32) {
        let (width, height) = if self.turn % 180 == 90 { (mode.height, mode.width) } else { (mode.width, mode.height) };
        let scaled = |side: u32| ((side as f64 * 100.0 / self.scale.max(1) as f64).round() as i32).max(1);
        (scaled(width), scaled(height))
    }
}

/// One connected monitor, as the compositor is driving it.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    /// The connector's name, such as `HDMI-A-1`.
    pub name: String,
    /// Whether it is the display with the frame.
    pub main: bool,
    pub x: i32,
    pub y: i32,
    pub current: Mode,
    pub modes: Vec<Mode>,
    pub setup: Setup,
}

/// The connected monitors, left to right.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Displays {
    /// Seconds left to confirm a change before it is undone; 0 when none is waiting.
    pub pending: u32,
    pub monitors: Vec<DisplayInfo>,
}

impl Displays {
    pub fn encode(&self) -> String {
        let mut line = format!("displays {}", self.pending);
        for monitor in &self.monitors {
            let modes: Vec<String> = monitor.modes.iter().map(Mode::encode).collect();
            let name: String = monitor.name.chars().filter(|c| !c.is_whitespace()).collect();
            line.push_str(&format!(
                "\t{name} {} {} {} {} {} {}",
                monitor.main as u8,
                monitor.x,
                monitor.y,
                monitor.current.encode(),
                modes.join(","),
                monitor.setup.encode()
            ));
        }
        line.push('\n');
        line
    }

    pub fn decode(line: &str) -> Option<Displays> {
        let mut fields = line.trim_end_matches('\n').split('\t');
        let pending = fields.next()?.strip_prefix("displays ")?.parse().ok()?;
        let mut monitors = Vec::new();
        for field in fields {
            let mut parts = field.split(' ');
            let name = parts.next().filter(|name| !name.is_empty())?.to_string();
            let main = match parts.next()? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let x = parts.next()?.parse().ok()?;
            let y = parts.next()?.parse().ok()?;
            let current = Mode::decode(parts.next()?)?;
            let modes: Option<Vec<Mode>> = parts.next()?.split(',').map(Mode::decode).collect();
            // A compositor from before monitors could be turned or scaled says nothing more.
            let setup = match parts.next() {
                Some(setup) => Setup::decode(setup)?,
                None => Setup::default(),
            };
            if parts.next().is_some() {
                return None;
            }
            monitors.push(DisplayInfo {
                name,
                main,
                x,
                y,
                current,
                modes: modes?,
                setup,
            });
        }
        Some(Displays { pending, monitors })
    }
}

/// What the user wants one monitor to do.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayChoice {
    pub name: String,
    pub mode: Mode,
    pub main: bool,
    pub setup: Setup,
}

/// What a screenshot is of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotMode {
    /// The display in use.
    Screen,
    /// The window in use.
    Window,
    /// A piece of the display, which the user marks out in the shell.
    Region,
    /// Whichever of those the user then picks, in the shell.
    Pick,
}

impl ShotMode {
    pub fn word(self) -> &'static str {
        match self {
            ShotMode::Screen => "screen",
            ShotMode::Window => "window",
            ShotMode::Region => "region",
            ShotMode::Pick => "pick",
        }
    }

    pub fn from_word(word: &str) -> Option<ShotMode> {
        Some(match word {
            "screen" => ShotMode::Screen,
            "window" => ShotMode::Window,
            "region" => ShotMode::Region,
            "pick" => ShotMode::Pick,
            _ => return None,
        })
    }
}

/// How a recording stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Recording {
    On,
    Off,
    /// It could not be started.
    Failed,
}

/// How hand control stands: off; on, with no hand in view or resting; following a
/// hand; or stopped because it could not go on, as when there is no camera.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Hands {
    #[default]
    Off,
    Watching,
    Following,
    Failed,
}

impl Hands {
    pub fn word(self) -> &'static str {
        match self {
            Hands::Off => "off",
            Hands::Watching => "watching",
            Hands::Following => "following",
            Hands::Failed => "failed",
        }
    }
}

/// Which of the two pictures of the windows is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverviewKind {
    /// Every workspace of a display side by side, each with its windows.
    Workspaces,
    /// An explicit app window chooser; independent of Alt being held.
    Windows,
    /// Destination previews while the original data drag remains active.
    Drag,
    /// The windows of a display in the order they were last used, while Alt is held.
    Switcher,
}

/// A rectangle in a display's own logical coordinates: left, top, width, height.
pub type Area = (i32, i32, i32, i32);

/// One workspace as the overview shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewWorkspace {
    pub workspace: usize,
    pub area: Area,
    /// The one in view.
    pub current: bool,
}

/// One window's picture: the application at this index, drawn in `area`.
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewThumb {
    pub app: usize,
    pub area: Area,
}

/// Where the compositor draws the pictures of the windows, so that the shell can draw
/// what goes around them.
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewLayout {
    pub kind: OverviewKind,
    pub screen: usize,
    /// Index into `thumbs` of the picture that Enter would choose.
    pub selected: Option<usize>,
    /// What has been typed to narrow the pictures down.
    pub filter: String,
    /// The switcher's strip, which the shell draws behind its pictures.
    pub strip: Option<Area>,
    pub workspaces: Vec<OverviewWorkspace>,
    pub thumbs: Vec<OverviewThumb>,
}

/// At most this many pictures are described, however many windows there are.
pub const OVERVIEW_THUMBS: usize = 256;
const OVERVIEW_FILTER_BYTES: usize = 256;

fn area_ok((_, _, w, h): Area) -> bool {
    w > 0 && h > 0
}

fn decode_area(words: &mut std::str::SplitWhitespace<'_>) -> Option<Area> {
    let area = (words.next()?.parse().ok()?, words.next()?.parse().ok()?, words.next()?.parse().ok()?, words.next()?.parse().ok()?);
    area_ok(area).then_some(area)
}

impl OverviewLayout {
    /// The line telling the shell what is open, or that nothing is.
    pub fn encode(layout: Option<&OverviewLayout>) -> String {
        let Some(layout) = layout else {
            return "overview none\n".to_string();
        };
        let kind = match layout.kind {
            OverviewKind::Workspaces => "w",
            OverviewKind::Switcher => "s",
            OverviewKind::Windows => "p",
            OverviewKind::Drag => "d",
        };
        let selected = layout.selected.map_or("-".to_string(), |selected| selected.to_string());
        let mut filter: String = layout.filter.chars().filter(|c| !c.is_control()).collect();
        while filter.len() > OVERVIEW_FILTER_BYTES {
            filter.pop();
        }
        let filter = if filter.is_empty() { "-".to_string() } else { filter.bytes().map(|b| format!("{b:02x}")).collect() };
        let strip = layout.strip.map_or("-".to_string(), |(x, y, w, h)| format!("{x} {y} {w} {h}"));
        let mut line = format!("overview {kind} {} {selected} {filter} {strip}", layout.screen);
        for workspace in &layout.workspaces {
            let (x, y, w, h) = workspace.area;
            line.push_str(&format!("\tw {} {x} {y} {w} {h} {}", workspace.workspace, workspace.current as u8));
        }
        for thumb in layout.thumbs.iter().take(OVERVIEW_THUMBS) {
            let (x, y, w, h) = thumb.area;
            line.push_str(&format!("\tt {} {x} {y} {w} {h}", thumb.app));
        }
        line.push('\n');
        line
    }

    /// Reads an `overview` line: `Some(None)` when it closed, `None` when the line is
    /// not one or is malformed.
    pub fn decode(line: &str) -> Option<Option<OverviewLayout>> {
        let line = line.trim_end_matches('\n').strip_prefix("overview ")?;
        if line == "none" {
            return Some(None);
        }
        let mut fields = line.split('\t');
        let mut words = fields.next()?.split_whitespace();
        let kind = match words.next()? {
            "w" => OverviewKind::Workspaces,
            "s" => OverviewKind::Switcher,
            "p" => OverviewKind::Windows,
            "d" => OverviewKind::Drag,
            _ => return None,
        };
        let screen = words.next()?.parse::<usize>().ok().filter(|screen| *screen < 32)?;
        let selected = match words.next()? {
            "-" => None,
            index => Some(index.parse::<usize>().ok()?),
        };
        let filter = match words.next()? {
            "-" => String::new(),
            hex => {
                if hex.len() % 2 != 0 || hex.len() > OVERVIEW_FILTER_BYTES * 2 {
                    return None;
                }
                let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
                String::from_utf8(bytes?).ok()?
            }
        };
        let strip = match words.clone().next()? {
            "-" => {
                words.next();
                None
            }
            _ => Some(decode_area(&mut words)?),
        };
        if words.next().is_some() {
            return None;
        }
        let mut workspaces = Vec::new();
        let mut thumbs = Vec::new();
        for field in fields {
            let mut words = field.split_whitespace();
            match words.next()? {
                "w" => {
                    let workspace = words.next()?.parse::<usize>().ok().filter(|w| *w < WORKSPACES)?;
                    let area = decode_area(&mut words)?;
                    let current = match words.next()? {
                        "0" => false,
                        "1" => true,
                        _ => return None,
                    };
                    workspaces.push(OverviewWorkspace { workspace, area, current });
                }
                "t" => {
                    let app = words.next()?.parse::<usize>().ok()?;
                    thumbs.push(OverviewThumb { app, area: decode_area(&mut words)? });
                }
                _ => return None,
            }
            if words.next().is_some() || thumbs.len() > OVERVIEW_THUMBS {
                return None;
            }
        }
        if selected.is_some_and(|selected| selected >= thumbs.len()) {
            return None;
        }
        Some(Some(OverviewLayout { kind, screen, selected, filter, strip, workspaces, thumbs }))
    }
}

/// Fixed companion operations accepted across the compositor IPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionAction { Keep, Mute, Stop, Quiet }

impl CompanionAction {
    pub fn encode(self) -> &'static str {
        match self {
            Self::Keep => "companion-keep\n",
            Self::Mute => "companion-mute\n",
            Self::Stop => "companion-stop\n",
            Self::Quiet => "companion-quiet\n",
        }
    }
}

/// A message from the compositor to the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The compositor supports live dock previews and drag destinations.
    WindowShelf,
    State(DesktopState),
    /// The dock keyboard shortcut was pressed.
    Dock,
    DockOn(usize),
    DockDismissed(usize),
    /// The launcher shortcut was pressed.
    Launcher,
    /// The settings shortcut was pressed.
    Settings,
    /// Open the full keyboard shortcut reference.
    Shortcuts,
    Companion,
    VoiceCompanion,
    CompanionControl(CompanionAction),
    CompanionBlur,
    /// Pointer position relative to the primary shell surface, only during voice UI.
    CompanionPointer { x: i32, y: i32, screen: usize },
    /// The music shortcut was pressed.
    Music,
    /// The notifications shortcut was pressed.
    Notifications,
    /// The quick settings shortcut was pressed.
    Quick,
    /// The power key or shortcut was pressed.
    Power,
    /// A screenshot was taken: its width and height, and the file of plain pixels
    /// (four bytes each, rows from the top) that holds it. `window` is where in it
    /// the window in use lies (left, top, width, height), when the shell may need to
    /// cut that out.
    Shot { request: Option<u64>, mode: ShotMode, width: u32, height: u32, window: Option<(u32, u32, u32, u32)>, path: String },
    /// A capture could not produce a picture. Tagged failures complete that request.
    ShotFailed { request: Option<u64>, reason: String },
    Recording(Recording),
    Hands(Hands),
    /// The lock shortcut was pressed.
    LockRequested,
    /// The screen is now locked, or no longer.
    Locked(bool),
    Displays(Displays),
    /// The layouts that are not the plain one, on every display and workspace.
    Layouts(Vec<LayoutInfo>),
    /// The overview or the window switcher opened or changed (`Some`), or closed.
    Overview(Option<OverviewLayout>),
}

impl Event {
    pub fn companion_pointer(x: i32, y: i32, screen: usize) -> String { format!("companion-pointer {x} {y} {screen}\n") }

    pub fn dock_dismissed(screen: usize) -> String { format!("dock-dismissed {screen}\n") }
    pub fn dock_on(screen: usize) -> String { format!("dock-screen {screen}\n") }

    pub fn dock() -> String { "dock\n".to_string() }

    pub const LAUNCHER: &str = "launcher\n";
    pub const SETTINGS: &str = "settings\n";
    /// ```
    /// assert_eq!(des_common::ipc::Event::decode(des_common::ipc::Event::SHORTCUTS), Some(des_common::ipc::Event::Shortcuts));
    /// ```
    pub const VOICE_COMPANION: &str = "voice-companion\n";
    pub const COMPANION_BLUR: &str = "companion-blur\n";
    pub const COMPANION: &str = "companion\n";
    pub const SHORTCUTS: &str = "shortcuts\n";
    pub const MUSIC: &str = "music\n";
    pub const NOTIFICATIONS: &str = "notifications\n";
    pub const QUICK: &str = "quick\n";
    pub const POWER: &str = "power\n";

    pub fn shot(mode: ShotMode, width: i32, height: i32, window: Option<(i32, i32, i32, i32)>, path: &str) -> String {
        let window = window.map_or("-".to_string(), |(x, y, w, h)| format!("{x},{y},{w},{h}"));
        format!("shot {} {width} {height} {window} {path}\n", mode.word())
    }

    pub fn shot_request(id: u64, mode: ShotMode, width: i32, height: i32, window: Option<(i32, i32, i32, i32)>, path: &str) -> String {
        format!("shot-request {id} {}", Self::shot(mode, width, height, window, path).strip_prefix("shot ").unwrap())
    }

    pub fn shot_failed(request: Option<u64>, reason: &str) -> String {
        let reason: String = reason.chars().take(512).map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
        let reason = if reason.trim().is_empty() { "capture failed" } else { reason.trim() };
        format!("shot-failed {} {reason}\n", request.map_or("-".into(), |id| id.to_string()))
    }

    /// `state` is `on`, `off` or `failed`.
    pub fn recording(state: &str) -> String {
        format!("recording {state}\n")
    }

    pub fn hands(state: Hands) -> String {
        format!("hands {}\n", state.word())
    }
    pub const LOCK_REQUESTED: &str = "lock\n";

    pub fn locked(locked: bool) -> String {
        format!("locked {}\n", locked as u8)
    }

    pub fn decode(line: &str) -> Option<Event> {
        if let Some(rest) = line.trim_end_matches('\n').strip_prefix("companion-pointer ") {
            let mut words = rest.split_whitespace();
            let x = words.next()?.parse().ok()?;
            let y = words.next()?.parse().ok()?;
            let screen = words.next()?.parse::<usize>().ok().filter(|screen| *screen < 32)?;
            if words.next().is_some() { return None; }
            return Some(Event::CompanionPointer { x, y, screen });
        }
        if let Some(screen)=line.trim_end().strip_prefix("dock-dismissed ") { return screen.parse::<usize>().ok().filter(|s|*s<32).map(Event::DockDismissed); }
        if let Some(screen)=line.trim_end().strip_prefix("dock-screen ") { return screen.parse::<usize>().ok().filter(|s|*s<32).map(Event::DockOn); }

        match line.trim_end_matches('\n') {
            "window-shelf" => return Some(Event::WindowShelf),
            "dock" => return Some(Event::Dock),
            "launcher" => return Some(Event::Launcher),
            "settings" => return Some(Event::Settings),
            "shortcuts" => return Some(Event::Shortcuts),
            "companion" => return Some(Event::Companion),
            "companion-blur" => return Some(Event::CompanionBlur),
            "voice-companion" => return Some(Event::VoiceCompanion),
            "companion-keep" => return Some(Event::CompanionControl(CompanionAction::Keep)),
            "companion-mute" => return Some(Event::CompanionControl(CompanionAction::Mute)),
            "companion-stop" => return Some(Event::CompanionControl(CompanionAction::Stop)),
            "companion-quiet" => return Some(Event::CompanionControl(CompanionAction::Quiet)),
            "music" => return Some(Event::Music),
            "notifications" => return Some(Event::Notifications),
            "quick" => return Some(Event::Quick),
            "power" => return Some(Event::Power),
            "recording on" => return Some(Event::Recording(Recording::On)),
            "recording off" => return Some(Event::Recording(Recording::Off)),
            "recording failed" => return Some(Event::Recording(Recording::Failed)),
            "hands off" => return Some(Event::Hands(Hands::Off)),
            "hands watching" => return Some(Event::Hands(Hands::Watching)),
            "hands following" => return Some(Event::Hands(Hands::Following)),
            "hands failed" => return Some(Event::Hands(Hands::Failed)),
            "lock" => return Some(Event::LockRequested),
            "locked 0" => return Some(Event::Locked(false)),
            "locked 1" => return Some(Event::Locked(true)),
            _ => {}
        }
        if line.starts_with("displays ") {
            return Displays::decode(line).map(Event::Displays);
        }
        if let Some(rest) = line.trim_end_matches('\n').strip_prefix("layouts") {
            if !rest.is_empty() && !rest.starts_with(' ') {
                return None;
            }
            let layouts: Option<Vec<LayoutInfo>> = rest.split_whitespace().take(WORKSPACES * 32).map(LayoutInfo::from_word).collect();
            return layouts.map(Event::Layouts);
        }
        if line.starts_with("overview ") {
            return OverviewLayout::decode(line).map(Event::Overview);
        }
        if let Some(failed) = line.trim_end_matches('\n').strip_prefix("shot-failed ") {
            let (id, reason) = failed.split_once(' ')?;
            let request = if id == "-" { None } else { Some(id.parse::<u64>().ok().filter(|id| *id != 0)?) };
            if reason.trim().is_empty() || reason.len() > 2048 || reason.chars().any(char::is_control) { return None; }
            return Some(Event::ShotFailed { request, reason: reason.into() });
        }
        if let Some(tagged) = line.strip_prefix("shot-request ") {
            let (id, rest) = tagged.split_once(' ')?;
            let request = id.parse::<u64>().ok().filter(|id| *id != 0)?;
            let Event::Shot { mode, width, height, window, path, .. } = Event::decode(&format!("shot {rest}"))? else { return None };
            return Some(Event::Shot { request: Some(request), mode, width, height, window, path });
        }
        if let Some(shot) = line.trim_end_matches('\n').strip_prefix("shot ") {
            // The path comes last and may hold spaces.
            let mut parts = shot.splitn(5, ' ');
            let mode = ShotMode::from_word(parts.next()?)?;
            let width = parts.next()?.parse().ok()?;
            let height = parts.next()?.parse().ok()?;
            let window = match parts.next()? {
                "-" => None,
                sides => {
                    let mut sides = sides.split(',').map(|side| side.parse::<u32>().ok());
                    let window = (sides.next()??, sides.next()??, sides.next()??, sides.next()??);
                    sides.next().is_none().then_some(window)
                }
            };
            let path = parts.next().filter(|path| !path.is_empty())?.to_string();
            return Some(Event::Shot { request: None, mode, width, height, window, path });
        }
        DesktopState::decode(line).map(Event::State)
    }
}

/// A request from the shell to the compositor.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Show the application at this index, on whichever display it lives.
    Activate(usize),
    /// Ask the application at this index to close.
    Close(usize),
    /// Move the application at this index to the next display.
    Move(usize),
    /// Bring this workspace into view.
    Workspace(usize),
    /// Send the application at this index to this workspace.
    SendTo(usize, usize),
    /// Uncover the terminal and give it the keyboard.
    Terminal,
    /// Split the workspace in use between two windows, or undo that.
    Split,
    /// Let the applications on the display in use cover all of it, or undo that.
    Fullscreen,
    /// Open or collapse one of the frame's panels.
    TogglePanel(Panel),
    /// Drive the monitors this way, listed left to right. The change is undone
    /// unless [`Command::KeepDisplays`] follows in time.
    ApplyDisplays(Vec<DisplayChoice>),
    KeepDisplays,
    RevertDisplays,
    /// The keyboard, mouse or touchpad settings were changed: read them again.
    ReloadInput,
    /// Turn hand control on, or off, for now.
    ToggleHands,
    /// Re-read the side panel behavior and rearrange application windows.
    ReloadFrame,
    /// Center-column height reserved for the shell on-screen keyboard (0 closes).
    KeyboardHeight(u16),
    /// Hide everything but the shell, which is showing the lock screen.
    Lock,
    /// The password was given: bring everything back.
    Unlock,
    /// The shell is showing notifications in this part of its window (x, y, width,
    /// height), which is to be seen above any application; or is showing none.
    Toast(Option<(i32, i32, i32, i32)>),
    Dock(Option<(i32, i32, i32, i32)>),
    DockOn(usize, Option<(i32, i32, i32, i32)>),
    DockKeyboard { screen: usize, active: bool },
    PlaceApp(usize, usize, usize, Place),
    /// An application is showing something to watch (or no longer): do not lock the
    /// screen for want of input meanwhile.
    HoldIdle(bool),
    /// Ephemeral companion playback activity; refreshed while audio is audible.
    CompanionSpeaking(bool),
    /// Identifies a chat editor overlay that releases keyboard focus on outside click.
    CompanionChat(bool),
    /// Render-only shell crop: never participates in keyboard or pointer focus.
    CompanionBubble(Option<(i32, i32, i32, i32)>),
    /// Take a screenshot.
    Shot(ShotMode),
    /// A control capture, correlated end to end independently of local hotkeys.
    ShotRequest { id: u64, mode: ShotMode },
    /// Record the display in use into this file.
    Record(String),
    RecordStop,
    /// Keep the application at this index in a corner above everything, on every
    /// workspace of its display: picture in picture.
    Pin(usize),
    /// Put whatever is pinned back where it was.
    Unpin,
    /// The shell is showing something in this part of its window on the main display
    /// (x, y, width, height) that is to be seen above any application, and has the
    /// keyboard meanwhile; or no longer.
    Overlay(Option<(i32, i32, i32, i32)>),
    /// The pointer is dragging these files out of the shell: start a drag that drops
    /// them as `file://` URIs into whatever is under the pointer when it is let go.
    DragFiles(Vec<String>),
    /// Share the workspace in use this way.
    Layout(LayoutKind),
    /// Move the first divider of the workspace in use to this share of its width.
    Ratio(f32),
    /// Set the layout of one display on one workspace outright, as restoring a
    /// session does.
    LayoutAt(LayoutInfo),
    /// Open the overview on the display under the pointer, or close it.
    Overview,
    /// Show this application ID on a display; identity is never a stale window index.
    AppWindows(usize, Vec<String>),
}

/// At most this many files go in one drag.
pub const MAX_DRAG_FILES: usize = 64;
/// A drag's line stays well inside the compositor's 4 KiB line bound.
pub const MAX_DRAG_LINE: usize = 4000;

/// Whether `uri` may travel in a drag: a percent-encoded local file, nothing that could
/// break the line or the tab-separated list.
pub fn drag_uri_ok(uri: &str) -> bool {
    uri.starts_with("file:///") && uri.len() > 8 && uri.bytes().all(|b| b.is_ascii_graphic())
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Command::Activate(index) => format!("activate {index}\n"),
            Command::Close(index) => format!("close {index}\n"),
            Command::Move(index) => format!("move {index}\n"),
            Command::Workspace(workspace) => format!("workspace {workspace}\n"),
            Command::SendTo(index, workspace) => format!("send-to {index}:{workspace}\n"),
            Command::Terminal => "terminal\n".to_string(),
            Command::Split => "split\n".to_string(),
            Command::Fullscreen => "fullscreen\n".to_string(),
            Command::TogglePanel(Panel::Left) => "panel left\n".to_string(),
            Command::TogglePanel(Panel::Right) => "panel right\n".to_string(),
            Command::TogglePanel(Panel::Bottom) => "panel bottom\n".to_string(),
            Command::KeepDisplays => "displays-keep\n".to_string(),
            Command::RevertDisplays => "displays-revert\n".to_string(),
            Command::ReloadInput => "reload-input\n".to_string(),
            Command::ToggleHands => "hands-toggle\n".to_string(),
            Command::ReloadFrame => "reload-frame\n".to_string(),
            Command::KeyboardHeight(height) => format!("keyboard-height {height}\n"),
            Command::Lock => "lock\n".to_string(),
            Command::Unlock => "unlock\n".to_string(),
            Command::DockKeyboard { screen, active } => format!("dock-keyboard {screen} {}\n", *active as u8),
            Command::DockOn(screen,None) => format!("dock-screen {screen} none\n"),
            Command::DockOn(screen,Some((x,y,w,h))) => format!("dock-screen {screen} {x} {y} {w} {h}\n"),
            Command::PlaceApp(index,screen,workspace,place) => format!("place {index} {screen} {workspace} {}\n",place.code()),
            Command::Layout(kind) => format!("layout {}\n", kind.word()),
            Command::Ratio(ratio) => format!("ratio {ratio:.3}\n"),
            Command::LayoutAt(layout) => format!("layout-at {}\n", layout.word()),
            Command::Dock(None) => "dock none\n".to_string(),
            Command::Dock(Some((x, y, w, h))) => format!("dock {x} {y} {w} {h}\n"),
            Command::Toast(None) => "toast none\n".to_string(),
            Command::CompanionBubble(None) => "companion-bubble none\n".to_string(),
            Command::CompanionBubble(Some((x,y,w,h))) => format!("companion-bubble {x} {y} {w} {h}\n"),
            Command::CompanionChat(true) => "companion-chat on\n".to_string(),
            Command::CompanionChat(false) => "companion-chat off\n".to_string(),
            Command::CompanionSpeaking(true) => "companion-speaking on\n".to_string(),
            Command::CompanionSpeaking(false) => "companion-speaking off\n".to_string(),
            Command::HoldIdle(true) => "hold-idle on\n".to_string(),
            Command::HoldIdle(false) => "hold-idle off\n".to_string(),
            Command::ShotRequest { id, mode } => format!("shot-request {id} {}\n", mode.word()),
            Command::Shot(mode) => format!("shot {}\n", mode.word()),
            Command::Record(path) => format!("record {path}\n"),
            Command::RecordStop => "record-stop\n".to_string(),
            Command::Pin(index) => format!("pin {index}\n"),
            Command::Unpin => "unpin\n".to_string(),
            Command::Overview => "overview\n".to_string(),
            Command::AppWindows(screen, ids) => format!("app-windows {screen} {}\n", ids.iter().map(|id|if id.is_empty(){"-".into()}else{id.bytes().map(|b|format!("{b:02x}")).collect::<String>()}).collect::<Vec<_>>().join(":")),
            Command::Overlay(None) => "overlay none\n".to_string(),
            Command::Overlay(Some((x, y, w, h))) => format!("overlay {x} {y} {w} {h}\n"),
            Command::DragFiles(uris) => format!("drag-files {}\n", uris.join("\t")),
            Command::Toast(Some((x, y, w, h))) => format!("toast {x} {y} {w} {h}\n"),
            Command::ApplyDisplays(choices) => {
                let mut line = "displays-apply".to_string();
                for choice in choices {
                    let main = if choice.main { "*" } else { "" };
                    line.push_str(&format!(" {}={}{main}/{}", choice.name, choice.mode.encode(), choice.setup.encode()));
                }
                line.push('\n');
                line
            }
        }
    }

    pub fn decode(line: &str) -> Option<Command> {
        if let Some(rest) = line.strip_prefix("keyboard-height ") {
            let height = rest.trim_end().parse::<u16>().ok().filter(|height| *height <= 1024)?;
            return Some(Command::KeyboardHeight(height));
        }
        if let Some(rest) = line.strip_prefix("app-windows ") {
            let mut words = rest.split_whitespace();
            let screen = words.next()?.parse::<usize>().ok().filter(|n| *n < 32)?;
            let encoded = words.next()?;
            if words.next().is_some() || encoded.len()>3072 {return None;}
            let mut ids=Vec::new();
            for hex in encoded.split(':') {
                if hex.is_empty() || hex.len()>1024 || ids.len()>=128 {return None;}
                let id=if hex=="-" {String::new()} else {
                    if hex.len()%2!=0{return None;}
                    let bytes:Option<Vec<u8>>=(0..hex.len()).step_by(2).map(|n|u8::from_str_radix(hex.get(n..n+2)?,16).ok()).collect();
                    let id=String::from_utf8(bytes?).ok()?;
                    if id.chars().any(char::is_control){return None;}
                    id
                };
                if !ids.contains(&id){ids.push(id);}
            }
            return Some(Command::AppWindows(screen,ids));
        }

        if let Some(rest)=line.trim_end().strip_prefix("dock-keyboard ") {
            let words: Vec<_> = rest.split_whitespace().collect();
            let [screen, active] = words.as_slice() else { return None; };
            let screen = screen.parse::<usize>().ok().filter(|s| *s < 32)?;
            let active = match *active { "0" => false, "1" => true, _ => return None };
            return Some(Command::DockKeyboard { screen, active });
        }
        if let Some(rest)=line.trim_end().strip_prefix("dock-screen ") {
            let (screen,area)=rest.split_once(' ')?;let screen=screen.parse::<usize>().ok().filter(|s|*s<32)?;
            return match Command::decode(&format!("dock {area}"))? {Command::Dock(area)=>Some(Command::DockOn(screen,area)),_=>None};
        }
        if let Some(rest)=line.trim_end().strip_prefix("place ") {
            let n:Vec<usize>=rest.split_whitespace().map(str::parse).collect::<Result<_,_>>().ok()?;
            let [index,screen,workspace,place]=n[..]else{return None};
            if screen>=32 || workspace>=WORKSPACES{return None;}
            let place=u8::try_from(place).ok().and_then(Place::from_code)?;
            return Some(Command::PlaceApp(index,screen,workspace,place));
        }

        let mut words = line.split_whitespace();
        if line.split_whitespace().next() == Some("displays-apply") {
            let choices: Option<Vec<DisplayChoice>> = words
                .skip(1)
                .map(|word| {
                    let (name, mode) = word.split_once('=')?;
                    let (mode, setup) = match mode.split_once('/') {
                        Some((mode, setup)) => (mode, Setup::decode(setup)?),
                        None => (mode, Setup::default()),
                    };
                    let (mode, main) = match mode.strip_suffix('*') {
                        Some(mode) => (mode, true),
                        None => (mode, false),
                    };
                    Some(DisplayChoice {
                        name: name.to_string(),
                        mode: Mode::decode(mode)?,
                        main,
                        setup,
                    })
                })
                .collect();
            return choices.filter(|choices| !choices.is_empty()).map(Command::ApplyDisplays);
        }
        if let Some(path) = line.trim_end_matches('\n').strip_prefix("record ") {
            return (!path.is_empty()).then(|| Command::Record(path.to_string()));
        }
        if let Some(area) = line.trim_end().strip_prefix("companion-bubble ") {
            return match Command::decode(&format!("dock {area}"))? {
                Command::Dock(area) => Some(Command::CompanionBubble(area)), _ => None,
            };
        }
        if let Some(area) = line.trim_end().strip_prefix("dock ") {
            if area == "none" { return Some(Command::Dock(None)); }
            let numbers: Vec<i32> = area.split(' ').map(str::parse).collect::<Result<_, _>>().ok()?;
            let [x, y, w, h] = numbers[..] else { return None };
            if w <= 0 || h <= 0 || x.checked_add(w).is_none() || y.checked_add(h).is_none() { return None; }
            return Some(Command::Dock(Some((x, y, w, h))));
        }
        if let Some(list) = line.trim_end_matches('\n').strip_prefix("drag-files ") {
            if line.len() > MAX_DRAG_LINE { return None; }
            let uris: Vec<String> = list.split('\t').map(str::to_string).collect();
            if uris.len() > MAX_DRAG_FILES || !uris.iter().all(|uri| drag_uri_ok(uri)) { return None; }
            return Some(Command::DragFiles(uris));
        }
        if let Some(area) = line.trim_end().strip_prefix("overlay ") {
            if area == "none" { return Some(Command::Overlay(None)); }
            let numbers: Vec<i32> = area.split(' ').map(str::parse).collect::<Result<_, _>>().ok()?;
            let [x, y, w, h] = numbers[..] else { return None };
            if w <= 0 || h <= 0 || x.checked_add(w).is_none() || y.checked_add(h).is_none() { return None; }
            return Some(Command::Overlay(Some((x, y, w, h))));
        }
        if let Some(area) = line.trim_end().strip_prefix("toast ") {
            if area == "none" {
                return Some(Command::Toast(None));
            }
            let numbers: Vec<i32> = area.split(' ').map(str::parse).collect::<Result<_, _>>().ok()?;
            let [x, y, w, h] = numbers[..] else { return None };
            return Some(Command::Toast(Some((x, y, w, h))));
        }
        let command = match (words.next()?, words.next()) {
            ("activate", Some(index)) => Command::Activate(index.parse().ok()?),
            ("close", Some(index)) => Command::Close(index.parse().ok()?),
            ("move", Some(index)) => Command::Move(index.parse().ok()?),
            ("workspace", Some(workspace)) => Command::Workspace(workspace.parse().ok().filter(|workspace| *workspace < WORKSPACES)?),
            ("send-to", Some(pair)) => {
                let (index, workspace) = pair.split_once(':')?;
                Command::SendTo(index.parse().ok()?, workspace.parse().ok().filter(|workspace| *workspace < WORKSPACES)?)
            }
            ("shot-request", Some(id)) => Command::ShotRequest {
                id: id.parse::<u64>().ok().filter(|id| *id != 0)?,
                mode: ShotMode::from_word(words.next()?)?,
            },
            ("shot", Some(word)) => Command::Shot(ShotMode::from_word(word)?),
            ("record-stop", None) => Command::RecordStop,
            ("pin", Some(index)) => Command::Pin(index.parse().ok()?),
            ("layout", Some(word)) => Command::Layout(LayoutKind::from_word(word)?),
            ("ratio", Some(share)) => Command::Ratio(parse_share(share)?),
            ("layout-at", Some(word)) => Command::LayoutAt(LayoutInfo::from_word(word)?),
            ("unpin", None) => Command::Unpin,
            ("overview", None) => Command::Overview,
            ("companion-chat", Some("on")) => Command::CompanionChat(true),
            ("companion-chat", Some("off")) => Command::CompanionChat(false),
            ("companion-speaking", Some("on")) => Command::CompanionSpeaking(true),
            ("companion-speaking", Some("off")) => Command::CompanionSpeaking(false),
            ("hold-idle", Some("on")) => Command::HoldIdle(true),
            ("hold-idle", Some("off")) => Command::HoldIdle(false),
            ("panel", Some("left")) => Command::TogglePanel(Panel::Left),
            ("panel", Some("right")) => Command::TogglePanel(Panel::Right),
            ("panel", Some("bottom")) => Command::TogglePanel(Panel::Bottom),
            ("terminal", None) => Command::Terminal,
            ("split", None) => Command::Split,
            ("fullscreen", None) => Command::Fullscreen,
            ("displays-keep", None) => Command::KeepDisplays,
            ("displays-revert", None) => Command::RevertDisplays,
            ("reload-input", None) => Command::ReloadInput,
            ("hands-toggle", None) => Command::ToggleHands,
            ("reload-frame", None) => Command::ReloadFrame,
            ("lock", None) => Command::Lock,
            ("unlock", None) => Command::Unlock,
            _ => return None,
        };
        words.next().is_none().then_some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> OverviewLayout {
        OverviewLayout {
            kind: OverviewKind::Workspaces,
            screen: 1,
            selected: Some(1),
            filter: "fire Ω".into(),
            strip: None,
            workspaces: (0..WORKSPACES).map(|w| OverviewWorkspace { workspace: w, area: (48 + w as i32 * 400, 48, 376, 900), current: w == 2 }).collect(),
            thumbs: vec![OverviewThumb { app: 3, area: (60, 100, 300, 200) }, OverviewThumb { app: 0, area: (460, 100, 300, 170) }],
        }
    }

    #[test]
    fn keyboard_reservation_round_trips_and_rejects_unbounded_requests() {
        for height in [0, 192, 252, 1024] {
            let command = Command::KeyboardHeight(height);
            assert_eq!(Command::decode(command.encode().trim()), Some(command));
        }
        for line in ["keyboard-height -1", "keyboard-height 1025", "keyboard-height 44 extra"] {
            assert!(Command::decode(line).is_none());
        }
    }

    #[test]
    fn capture_failure_events_reach_the_shell_without_a_successful_picture() {
        assert!(Event::decode("shot-failed 42 there is no such display\n").is_some());
        assert!(Event::decode("shot-failed - cannot write the picture\n").is_some());
        assert!(Event::decode("shot-failed 0 invalid request\n").is_none());
        assert!(Event::decode("shot-failed 42 \n").is_none());
        assert_eq!(Event::decode(&Event::shot_failed(Some(42), "cannot write\ntry again")),
            Some(Event::ShotFailed { request: Some(42), reason: "cannot write try again".into() }));
        assert!(Event::decode(&format!("shot-failed 42 {}\n", "x".repeat(2049))).is_none());
    }

    #[test]
    fn tagged_capture_results_keep_the_request_identity_and_paths() {
        let wire = Event::shot_request(42, ShotMode::Region, 2, 3, None, "/tmp/a shot.rgba");
        assert_eq!(Event::decode(&wire), Some(Event::Shot { request: Some(42), mode: ShotMode::Region, width: 2, height: 3, window: None, path: "/tmp/a shot.rgba".into() }));
        assert!(Event::decode("shot-request 0 region 2 3 - /tmp/a").is_none());
        assert!(Command::decode("shot-request 0 region").is_none());
        assert!(Command::decode("shot-request 42 region extra").is_none());
    }

    #[test]
    fn the_overview_round_trips_and_closes() {
        let open = layout();
        let line = OverviewLayout::encode(Some(&open));
        assert!(line.ends_with('\n') && line.lines().count() == 1);
        assert_eq!(Event::decode(&line), Some(Event::Overview(Some(open.clone()))));
        assert_eq!(Event::decode(&OverviewLayout::encode(None)), Some(Event::Overview(None)));
        let switcher = OverviewLayout { kind: OverviewKind::Switcher, selected: None, filter: String::new(), strip: Some((100, 400, 800, 220)), workspaces: Vec::new(), ..open };
        assert_eq!(Event::decode(&OverviewLayout::encode(Some(&switcher))), Some(Event::Overview(Some(switcher))));
        assert_eq!(Command::decode(&Command::Overview.encode()), Some(Command::Overview));
    }

    #[test]
    fn malformed_overview_lines_are_refused() {
        for bad in [
            "overview x 0 - - -\n",
            "overview w 99 - - -\n",
            "overview w 0 - zz -\n",
            "overview w 0 - - -\tt 0 1 2 0 4\n",
            "overview w 0 - - -\tw 9 1 2 3 4 0\n",
            "overview w 0 3 - -\tt 0 1 2 3 4\n",
            "overview w 0 - - -\tq 1\n",
            "overview w 0 - - 1 2 3\n",
        ] {
            assert_eq!(OverviewLayout::decode(bad), None, "{bad:?}");
        }
        // Control characters typed into the filter never break the line.
        let mut odd = layout();
        odd.filter = "a\tb\nc".into();
        let line = OverviewLayout::encode(Some(&odd));
        assert_eq!(line.matches('\n').count(), 1);
        let Some(Some(back)) = OverviewLayout::decode(&line) else { panic!("{line}") };
        assert_eq!(back.filter, "abc");
    }

    fn app(title: &str, screen: usize, place: Place) -> AppInfo {
        AppInfo {
            app_id: String::new(),
            title: title.into(),
            screen,
            place,
            workspace: 0,
            pinned: false,
            urgent: false,
        }
    }

    #[test]
    fn dragged_files_round_trip_and_are_bounded() {
        let one = Command::DragFiles(vec!["file:///home/a/notes%20one.txt".into()]);
        assert_eq!(Command::decode(one.encode().trim_end()), Some(one));
        let two = Command::DragFiles(vec!["file:///a".into(), "file:///b/c".into()]);
        assert_eq!(two.encode(), "drag-files file:///a\tfile:///b/c\n");
        assert_eq!(Command::decode(two.encode().trim_end()), Some(two));
        assert_eq!(Command::decode("drag-files "), None);
        assert_eq!(Command::decode("drag-files http://example.com/x"), None);
        assert_eq!(Command::decode("drag-files file:///a b"), None);
        assert_eq!(Command::decode("drag-files file:///"), None);
        let many = vec!["file:///x".to_string(); MAX_DRAG_FILES + 1].join("\t");
        assert_eq!(Command::decode(&format!("drag-files {many}")), None);
        let long = format!("drag-files file:///{}", "a".repeat(MAX_DRAG_LINE));
        assert_eq!(Command::decode(&long), None);
    }

    #[test]
    fn pins_and_overlays_round_trip_and_old_wire_remains_unchanged() {
        for command in [Command::Pin(2), Command::Unpin, Command::Overlay(None), Command::Overlay(Some((0, 44, 1920, 400)))] {
            assert_eq!(Command::decode(command.encode().trim_end()), Some(command));
        }
        assert_eq!(Command::decode("pin x"), None);
        assert_eq!(Command::decode("pin"), None);
        assert_eq!(Command::decode("unpin 1"), None);
        assert_eq!(Command::decode("overlay 0 0 -5 5"), None);
        assert_eq!(Command::decode("overlay 0 0 5"), None);

        // Nothing pinned and nothing retained: the wire is as it was.
        let mut state = DesktopState { apps: vec![app("a", 0, Place::Main)], ..Default::default() };
        assert!(state.encode().starts_with("state 1 7 0 - 0\t0 1:0: a"), "{}", state.encode());
        assert_eq!(DesktopState::decode(&state.encode()), Some(state.clone()));
        // Pinned: the mark follows the saved place, which is then always written.
        state.apps[0].pinned = true;
        state.apps.push(app("b", 0, Place::Main));
        assert!(state.encode().contains("\t0 1:0::1:p a\t0 1:0::1 b"), "{}", state.encode());
        let decoded = DesktopState::decode(&state.encode()).unwrap();
        assert!(decoded.apps[0].pinned && !decoded.apps[1].pinned);
        assert_eq!(decoded.apps, state.apps);
        assert_eq!(DesktopState::decode("state 1 7 0 - 0\t0 1:0::1:q a"), None);
        assert_eq!(DesktopState::decode("state 1 7 0 - 0\t0 1:0::1:p:p a"), None);
    }

    #[test]
    fn urgency_round_trips_beside_the_pin_and_bad_marks_are_refused() {
        let mut state = DesktopState { apps: vec![app("a", 0, Place::Hidden), app("b", 0, Place::Main)], ..Default::default() };
        state.apps[0].urgent = true;
        assert!(state.encode().contains("\t0 0:0::0:u a\t0 1:0::1 b"), "{}", state.encode());
        assert_eq!(DesktopState::decode(&state.encode()).unwrap().apps, state.apps);
        state.apps[0].pinned = true;
        assert!(state.encode().contains(":pu a"), "{}", state.encode());
        assert_eq!(DesktopState::decode(&state.encode()).unwrap().apps, state.apps);
        for bad in ["state 1 7 0 - 0\t0 1:0::1:up a", "state 1 7 0 - 0\t0 1:0::1:uu a", "state 1 7 0 - 0\t0 1:0::1: a"] {
            assert_eq!(DesktopState::decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn screen_docks_and_saved_placements_round_trip_and_reject_bad_input() {
        for command in [Command::DockOn(1,Some((2,700,300,60))),Command::DockOn(2,None),Command::PlaceApp(4,1,2,Place::Side)] {
            assert_eq!(Command::decode(command.encode().trim_end()),Some(command));
        }
        for line in ["dock-screen 1 0 0 -1 20","dock-screen 1 0 0 20 0","place 0 1 4 1","place 0 1 0 9","dock-screen 33 none"] {assert!(Command::decode(line).is_none(),"{line}");}
        assert_eq!(Event::decode(&Event::dock_on(1)),Some(Event::DockOn(1)));
    }

    #[test]
    fn retained_places_round_trip_and_old_wire_remains_unchanged() {
        let mut state = DesktopState::default();
        state.apps = vec![app("inactive side", 0, Place::Hidden), app("inactive main", 0, Place::Hidden)];
        state.apps[0].workspace = 1; state.apps[1].workspace = 2;
        state.saved_places = vec![Place::Side, Place::Main];
        assert_eq!(DesktopState::decode(&state.encode()), Some(state.clone()));
        state.saved_places.clear();
        assert_eq!(DesktopState::decode(&state.encode()), Some(state));
        for suffix in ["6", "", "2:1"] {
            assert!(DesktopState::decode(&format!("state 1 7 0 - 0\t0 0:1:6162:{suffix} Title\n")).is_none());
        }
    }

    #[test]
    fn app_ids_round_trip_and_old_states_have_no_identity() {
        let mut state = DesktopState::default();
        let mut application = app("Title", 0, Place::Main);
        application.app_id = "org.example.Editor desktop:é".into();
        state.apps.push(application);
        assert_eq!(DesktopState::decode(&state.encode()), Some(state.clone()));
        for line in ["state 1 7 0 -\t0 1 Old\n", "state 1 7 0 - 0\t0 1:0 Old\n"] {
            assert_eq!(DesktopState::decode(line).unwrap().apps[0].app_id, "");
        }
        state.apps[0].app_id = "evil\tapp\nname\r".into();
        let encoded = state.encode();
        assert_eq!(encoded.matches('\t').count(), 1);
        assert_eq!(encoded.matches('\n').count(), 1);
        assert_eq!(DesktopState::decode(&encoded).unwrap().apps[0].app_id, "evilappname");
        for token in ["0:0:z0", "0:0:f", "0:0:ff", "0:0:0a", "0:0:é"] {
            assert!(DesktopState::decode(&format!("state 1 7 0 - 0\t0 {token} Title\n")).is_none());
        }
    }

    #[test]
    fn dock_commands_round_trip_and_reject_malformed_rectangles() {
        for command in [Command::Dock(None), Command::Dock(Some((12, 800, 900, 200)))] {
            assert_eq!(Command::decode(&command.encode()), Some(command));
        }
        for line in ["dock", "dock none extra", "dock 1 2 3", "dock 1 2 3 4 5", "dock 1 2 -3 4", "dock 1 2 0 4", "dock 1 2 3 0", "dock 2147483647 0 2 3", "dock 0 2147483647 2 3", "dock a 2 3 4"] {
            assert_eq!(Command::decode(line), None, "{line}");
        }
    }

    #[test]
    fn state_round_trips() {
        for state in [
            DesktopState::default(),
            DesktopState {
                screens: 2,
                panels: Panels::from_bits(5).unwrap(),
                terminal_split: true,
                focus: Some(1),
                workspace: 2,
                saved_places: Vec::new(),
                apps: vec![
                    app("Text Editor", 0, Place::Side),
                    app("Files — Home", 1, Place::Main),
                    AppInfo { workspace: 3, ..app("x", 0, Place::Hidden) },
                ],
            },
        ] {
            assert_eq!(DesktopState::decode(&state.encode()), Some(state.clone()));
            assert_eq!(Event::decode(&state.encode()), Some(Event::State(state)));
        }
    }

    #[test]
    fn reads_state_from_before_workspaces() {
        let state = DesktopState::decode("state 1 7 0 0\t0 1 Editor\n").unwrap();
        assert_eq!((state.workspace, state.apps[0].workspace, state.apps[0].place), (0, 0, Place::Main));
    }

    #[test]
    fn covering_is_the_main_app_on_the_main_display() {
        let state = DesktopState {
            screens: 2,
            apps: vec![
                app("a", 1, Place::Main),
                app("b", 0, Place::Side),
                app("c", 0, Place::Main),
            ],
            ..Default::default()
        };
        assert_eq!(state.covering(), Some(2));
        assert_eq!(DesktopState::default().covering(), None);
    }

    #[test]
    fn titles_cannot_break_framing() {
        let state = DesktopState {
            apps: vec![app("evil\ttitle\nstate 5", 0, Place::Main)],
            ..Default::default()
        };
        let line = state.encode();
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(DesktopState::decode(&line).unwrap().apps[0].title, "evil title state 5");
    }

    #[test]
    fn rejects_malformed_state() {
        for line in [
            "hello",
            "state x 7 0 -",
            "state 0 7 0 -",
            "state 1 9 0 -",
            "state 1 7 2 -",
            "state 1 7 0",
            "state 1 7 0 - extra",
            "state 1 7 0 - 0 extra",
            "state 1 7 0 - 9",
            "state 1 7 0 -\t0 1:9 on-a-missing-workspace",
            "state 1 7 0 0",
            "state 1 7 0 -\t1 1 on-a-missing-display",
            "state 1 7 0 -\t0 6 bad-place",
            "state 1 7 0 -\t0 1",
        ] {
            assert_eq!(DesktopState::decode(line), None, "{line}");
        }
    }

    #[test]
    fn displays_round_trip() {
        let mode = |width, height, refresh| Mode { width, height, refresh };
        let displays = Displays {
            pending: 12,
            monitors: vec![
                DisplayInfo {
                    name: "eDP-1".into(),
                    main: true,
                    x: 0,
                    y: 0,
                    current: mode(1920, 1080, 60052),
                    modes: vec![mode(1920, 1080, 60052), mode(1280, 720, 60000)],
                    setup: Setup::default(),
                },
                DisplayInfo {
                    name: "HDMI-A-1".into(),
                    main: false,
                    x: 1920,
                    y: 0,
                    current: mode(1920, 1080, 60000),
                    modes: vec![mode(3840, 2160, 30000)],
                    setup: Setup { scale: 150, turn: 90, usage: Usage::Mirror },
                },
            ],
        };
        assert_eq!(Displays::decode(&displays.encode()), Some(displays.clone()));
        assert_eq!(Event::decode(&displays.encode()), Some(Event::Displays(displays)));
        assert_eq!(Displays::decode("displays 0\n"), Some(Displays::default()));
        for line in ["displays", "displays x", "displays 0\teDP-1 1 0 0 1920x1080", "displays 0\teDP-1 2 0 0 1x1@1 1x1@1", "displays 0\teDP-1 1 0 0 0x0@0 1x1@1"] {
            assert_eq!(Displays::decode(line), None, "{line}");
        }
    }

    #[test]
    fn events_and_commands_round_trip() {
        assert_eq!(Event::decode(&Event::dock()), Some(Event::Dock));
        assert_eq!(Event::decode(Event::LAUNCHER), Some(Event::Launcher));
        assert_eq!(Event::decode(Event::SETTINGS), Some(Event::Settings));
        assert_eq!(Event::decode(Event::MUSIC), Some(Event::Music));
        assert_eq!(Event::decode(Event::NOTIFICATIONS), Some(Event::Notifications));
        assert_eq!(Event::decode(Event::QUICK), Some(Event::Quick));
        assert_eq!(Event::decode(Event::POWER), Some(Event::Power));
        assert_eq!(Event::decode(&Event::recording("failed")), Some(Event::Recording(Recording::Failed)));
        for state in [Hands::Off, Hands::Watching, Hands::Following, Hands::Failed] {
            assert_eq!(Event::decode(&Event::hands(state)), Some(Event::Hands(state)));
        }
        assert_eq!(
            Event::decode(&Event::shot(ShotMode::Region, 1920, 1080, None, "/run/user/1000/a shot.rgba")),
            Some(Event::Shot { request: None, mode: ShotMode::Region, width: 1920, height: 1080, window: None, path: "/run/user/1000/a shot.rgba".into() })
        );
        assert_eq!(
            Event::decode(&Event::shot(ShotMode::Pick, 1920, 1080, Some((300, 40, 1066, 701)), "/run/a.rgba")),
            Some(Event::Shot { request: None, mode: ShotMode::Pick, width: 1920, height: 1080, window: Some((300, 40, 1066, 701)), path: "/run/a.rgba".into() })
        );
        assert_eq!(Event::decode("shot region 1920 1080 - "), None);
        assert_eq!(Event::decode("shot pick 1920 1080 1,2,3 /run/a.rgba"), None);
        assert_eq!(Event::decode(Event::LOCK_REQUESTED), Some(Event::LockRequested));
        assert_eq!(Event::decode(&Event::locked(true)), Some(Event::Locked(true)));
        assert_eq!(Event::decode(&Event::locked(false)), Some(Event::Locked(false)));
        assert_eq!(Event::decode("locked 2"), None);
        for command in [
            Command::Activate(3),
            Command::Close(0),
            Command::Move(2),
            Command::Workspace(3),
            Command::SendTo(1, 2),
            Command::Terminal,
            Command::Split,
            Command::Fullscreen,
            Command::TogglePanel(Panel::Left),
            Command::TogglePanel(Panel::Right),
            Command::TogglePanel(Panel::Bottom),
        ] {
            assert_eq!(Command::decode(command.encode().trim_end()), Some(command));
        }
        let mode = |width, height, refresh| Mode { width, height, refresh };
        for command in [
            Command::KeepDisplays,
            Command::RevertDisplays,
            Command::ReloadInput,
            Command::ToggleHands,
            Command::ReloadFrame,
            Command::Lock,
            Command::Unlock,
            Command::Toast(None),
            Command::HoldIdle(true),
            Command::HoldIdle(false),
            Command::Shot(ShotMode::Window),
            Command::ShotRequest { id: 42, mode: ShotMode::Region },
            Command::Record("/home/ada/Videos/a film.mp4".into()),
            Command::RecordStop,
            Command::Toast(Some((1500, 52, 380, 240))),
            Command::ApplyDisplays(vec![
                DisplayChoice { name: "eDP-1".into(), mode: mode(1920, 1080, 60052), main: false, setup: Setup::default() },
                DisplayChoice { name: "HDMI-A-1".into(), mode: mode(3840, 2160, 30000), main: true, setup: Setup { scale: 200, turn: 270, usage: Usage::Off } },
            ]),
        ] {
            assert_eq!(Command::decode(command.encode().trim_end()), Some(command));
        }
        for line in ["displays-apply", "displays-apply eDP-1", "displays-apply eDP-1=big", "displays-keep 1"] {
            assert_eq!(Command::decode(line), None, "{line}");
        }
        for line in ["activate", "activate -1", "terminal 1", "close 1 2", "panel top", "split 1"] {
            assert_eq!(Command::decode(line), None, "{line}");
        }
    }

    #[test]
    fn further_slots_and_layouts_travel_and_old_places_keep_their_numbers() {
        assert_eq!(Place::Main.code(), 1);
        assert_eq!(Place::Side.code(), 2);
        for slot in 0..Place::SLOTS {
            let place = Place::of_slot(slot);
            assert_eq!(place.slot(), Some(slot));
            assert_eq!(Place::from_code(place.code()), Some(place));
        }
        assert_eq!(Place::from_code(6), None);
        let mut state = DesktopState::default();
        state.apps = vec![app("a", 0, Place::Main), app("b", 0, Place::Side), app("c", 0, Place::Extra(2)), app("d", 0, Place::Extra(4))];
        assert_eq!(DesktopState::decode(&state.encode()), Some(state));

        let layouts = vec![
            LayoutInfo { workspace: 0, screen: 0, kind: LayoutKind::Columns, ratio: 0.25, ratio2: 0.6 },
            LayoutInfo { workspace: 3, screen: 1, kind: LayoutKind::Master, ratio: 0.7, ratio2: 0.5 },
        ];
        assert_eq!(Event::decode(&LayoutInfo::encode_all(&layouts)), Some(Event::Layouts(layouts)));
        assert_eq!(Event::decode(&LayoutInfo::encode_all(&[])), Some(Event::Layouts(Vec::new())));
        for line in ["layouts 4:0:split:0.5:0.5", "layouts 0:0:grid:0.5:0.5", "layouts 0:0:split:1.5:0.5", "layouts 0:0:split:0.5", "layoutsx"] {
            assert_eq!(Event::decode(line), None, "{line}");
        }

        for command in [
            Command::Layout(LayoutKind::Master),
            Command::Ratio(0.35),
            Command::LayoutAt(LayoutInfo { workspace: 2, screen: 0, kind: LayoutKind::Split, ratio: 0.4, ratio2: 0.667 }),
            Command::PlaceApp(1, 0, 0, Place::Extra(3)),
        ] {
            assert_eq!(Command::decode(command.encode().trim_end()), Some(command));
        }
        for line in ["layout", "layout grid", "ratio 0", "ratio 1", "ratio nan", "ratio 0.5 0.5", "place 1 0 0 6"] {
            assert_eq!(Command::decode(line), None, "{line}");
        }
        assert_eq!(LayoutKind::Master.next(), LayoutKind::Single);
        assert_eq!(LayoutKind::Single.next(), LayoutKind::Split);
    }
}


#[cfg(test)]
mod window_shelf_tests {
    use super::*;
    #[test]
    fn chooser_requests_preserve_group_identity_and_reject_untrusted_input() {
        let request=Command::AppWindows(1,vec!["org.browser.Browser".into(),"browser-beta".into(),"アプリ".into()]);
        assert_eq!(Command::decode(request.encode().trim_end()),Some(request));
        assert_eq!(Command::decode("app-windows 0 -"),Some(Command::AppWindows(0,vec![String::new()])));
        for bad in ["app-windows 32 61","app-windows 0 zz","app-windows 0 0","app-windows 0 0a","app-windows 0 ff","app-windows 0 61 extra","app-windows 0 :61"] {
            assert_eq!(Command::decode(bad),None,"{bad}");
        }
        assert_eq!(Command::decode(&format!("app-windows 0 {}","61".repeat(513))),None);
        assert_eq!(Event::decode("window-shelf\n"),Some(Event::WindowShelf));
    }
    #[test]
    fn new_preview_modes_keep_bounded_layout_roundtrips() {
        for kind in [OverviewKind::Windows,OverviewKind::Drag] {
            let layout=OverviewLayout{kind,screen:0,selected:Some(0),filter:"50".into(),strip:Some((16,250,600,240)),workspaces:vec![],thumbs:vec![OverviewThumb{app:3,area:(32,290,260,150)}]};
            assert_eq!(Event::decode(&OverviewLayout::encode(Some(&layout))),Some(Event::Overview(Some(layout))));
        }
    }
}

#[cfg(test)]
mod companion_control_tests {
    use super::*;
    #[test]
    fn companion_control_events_accept_only_fixed_operations() {
        for (wire, action) in [
            ("companion-keep\n", CompanionAction::Keep),
            ("companion-mute\n", CompanionAction::Mute),
            ("companion-stop\n", CompanionAction::Stop),
            ("companion-quiet\n", CompanionAction::Quiet),
        ] {
            assert_eq!(Event::decode(wire), Some(Event::CompanionControl(action)));
            assert_eq!(action.encode(), wire);
        }
        for invalid in ["companion-run", "companion-keep extra", "companion-mute\ncompanion-stop", "companion-quiet "] {
            assert_eq!(Event::decode(invalid), None);
        }
    }
}

#[cfg(test)]
mod companion_speaking_tests {
    use super::*;
    #[test]
    fn speaking_is_a_fixed_boolean_command() {
        for active in [false, true] {
            let command = Command::CompanionSpeaking(active);
            assert_eq!(Command::decode(&command.encode()), Some(command));
        }
        assert_eq!(Command::decode("companion-speaking yes"), None);
        assert_eq!(Command::decode("companion-speaking on extra"), None);
    }
}

#[cfg(test)]
mod companion_bubble_tests {
    use super::*;
    #[test]
    fn bubble_crop_round_trips_and_rejects_invalid_bounds() {
        for area in [None, Some((4, 8, 220, 72))] {
            let command = Command::CompanionBubble(area);
            assert_eq!(Command::decode(&command.encode()), Some(command));
        }
        for wire in ["companion-bubble 0 0 0 10", "companion-bubble 0 0 -1 3", "companion-bubble 2147483647 0 2 4", "companion-bubble none extra"] {
            assert_eq!(Command::decode(wire), None);
        }
    }
}

#[cfg(test)]
mod companion_pointer_tests {
    use super::*;
    #[test]
    fn pointer_anchor_preserves_signed_primary_coordinates_and_display() {
        assert_eq!(Event::decode(&Event::companion_pointer(-24, 400, 1)), Some(Event::CompanionPointer { x: -24, y: 400, screen: 1 }));
        for wire in ["companion-pointer 0 0 32", "companion-pointer 0 0", "companion-pointer 0 0 1 extra", "companion-pointer 0 0 -1"] {
            assert_eq!(Event::decode(wire), None);
        }
    }
}

#[cfg(test)]
mod companion_chat_focus_tests {
    use super::*;
    #[test]
    fn chat_keyboard_marker_and_blur_event_are_fixed_protocol_values() {
        for active in [true, false] {
            let command = Command::CompanionChat(active);
            assert_eq!(Command::decode(&command.encode()), Some(command));
        }
        assert_eq!(Event::decode(Event::COMPANION_BLUR), Some(Event::CompanionBlur));
        assert_eq!(Event::decode("companion-blur extra"), None);
        assert_eq!(Command::decode("companion-chat on extra"), None);
    }
}

#[cfg(test)]
mod dock_keyboard_tests {
    use super::*;

    #[test]
    fn keyboard_ownership_and_dismissal_keep_display_identity() {
        for screen in [0, 1, 31] {
            for active in [false, true] {
                let command = Command::DockKeyboard { screen, active };
                assert_eq!(Command::decode(&command.encode()), Some(command));
            }
            assert_eq!(Event::decode(&Event::dock_dismissed(screen)), Some(Event::DockDismissed(screen)));
        }
        for line in ["dock-keyboard 32 1", "dock-keyboard 1 2", "dock-keyboard 1", "dock-keyboard 1 1 extra"] {
            assert_eq!(Command::decode(line), None);
        }
        assert_eq!(Event::decode("dock-dismissed 32"), None);
        assert_eq!(Event::decode("dock-dismissed 1 extra"), None);
    }
}
