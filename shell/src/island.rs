//! The status island: the centre of the status strip, which grows to show what is
//! going on (design: the "DEs-UI Status Strip" canvas, "Island · every state").
//!
//! At rest it is what the centre always was: who has the keyboard and how many
//! applications are open. Something live widens it (music, a recording), a
//! notification or a volume change grows it for a moment. Resting the pointer on
//! it (or pressing and holding, by touch) opens it upward into a card; a click
//! opens what it shows: the music player, the app a notification came from
//! (`click`). It never takes the keyboard (the compositor lifts it above
//! the applications with `Command::Island`, which leaves focus alone), and it never
//! grows over the left or right island.
//!
//! What it shows is decided by `mode`, its size by `size_of`; both are plain and
//! tested. Growing is a spring with a little overshoot, shrinking settles without
//! one (`spring`). The outline takes the colour of what it shows (`outline`), and
//! music paused for a minute leaves the island (`music_shows`).

use crate::{notify, osd, theme::Theme, ui};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};
use std::time::{Duration, Instant};

/// Height of the island in the strip, as the other islands.
pub const REST_HEIGHT: f32 = crate::panels::ISLAND_HEIGHT;
/// How long a notification shows in the island before it settles into the count.
pub const ARRIVING_FOR: Duration = Duration::from_secs(4);
const CARD_WIDTH: f32 = 400.0;
const MUSIC_CARD: Vec2 = vec2(CARD_WIDTH, 156.0);
const NOTE_CARD: Vec2 = vec2(CARD_WIDTH, 164.0);
const RECORDING_CARD: Vec2 = vec2(320.0, 116.0);
/// How long the pointer rests on the island before it opens.
const HOVER_OPEN: Duration = Duration::from_millis(300);
/// How long a press is held before it opens the island instead of clicking.
const HOLD_OPEN: Duration = Duration::from_millis(450);
/// An island the pointer opened closes this long after the pointer leaves it.
const LEAVE_CLOSE: Duration = Duration::from_millis(600);
const ARRIVING_HEIGHT: f32 = 52.0;
/// The largest corner: cards are rounded, not stadiums.
const CARD_RADIUS: f32 = 28.0;
/// The spring's natural frequency: settles in about a third of a second.
const STIFFNESS: f32 = 16.0;
/// Damping while growing: under 1, so it overshoots a little (about 2% at 60 fps) and
/// comes back, the way Apple's island opens. Shrinking is critically damped (1).
const GROW_DAMPING: f32 = 0.7;
/// An open island left alone this long (the pointer not on it) closes by itself.
pub const COLLAPSE_AFTER: Duration = Duration::from_secs(5);
/// Music paused this long leaves the island, as an activity that has ended.
pub const PAUSED_FOR: Duration = Duration::from_secs(60);
const RED: Color32 = Color32::from_rgb(0xFF, 0x5D, 0x5D);
/// Charging, and a camera in use (as the dot Apple puts there).
const GREEN: Color32 = Color32::from_rgb(0x30, 0xD1, 0x58);
/// A microphone in use.
const ORANGE: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x0A);

/// What the island is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Resting,
    /// Notifications not yet looked at, beside the resting content.
    Waiting,
    Music,
    MusicOpen,
    /// A notification has just come in.
    Arriving,
    /// A notification, whole, with its actions.
    NoteOpen,
    Recording,
    /// The recording, with its time and the button that stops it.
    RecordingOpen,
    /// Volume or brightness, for a moment.
    Level,
    /// Something the system did, for a moment: charging, a Bluetooth device
    /// connecting, Do Not Disturb (happenings.rs).
    Blip,
    /// Something under way with its progress: a download.
    Task,
}

/// The picture beside an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlipIcon {
    Battery { charging: bool },
    Bluetooth,
    Moon,
}

/// Something the system did, shown for a moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Blip {
    pub icon: BlipIcon,
    pub title: String,
    pub detail: String,
    /// A percentage to draw as a bar (a battery).
    pub level: Option<u8>,
    /// Shown in red: a battery running low.
    pub warn: bool,
}

/// Something under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub title: String,
    pub detail: String,
    pub progress: Option<f32>,
}

/// "3 h 20 min", "45 min".
pub fn duration_words(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// What is going on, as far as choosing a mode goes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    pub arriving: bool,
    pub level: bool,
    pub recording: bool,
    pub music: bool,
    pub waiting: bool,
    pub blip: bool,
    pub task: bool,
}

/// The mode, most important first: a notification coming in, then a level being
/// changed, then something the system just did, then music, then a recording (which, with music, shows as a bubble
/// beside it instead), then a download under way, then notifications waiting. Opening means something for
/// music, notifications and a recording.
pub fn mode(facts: Facts, open: bool) -> Mode {
    if facts.arriving {
        return if open { Mode::NoteOpen } else { Mode::Arriving };
    }
    if facts.level {
        return Mode::Level;
    }
    if facts.blip {
        return Mode::Blip;
    }
    if facts.music {
        return if open { Mode::MusicOpen } else { Mode::Music };
    }
    if facts.recording {
        return if open { Mode::RecordingOpen } else { Mode::Recording };
    }
    if facts.task {
        return Mode::Task;
    }
    if facts.waiting {
        return if open { Mode::NoteOpen } else { Mode::Waiting };
    }
    Mode::Resting
}

/// Whether a recording shows as a bubble of its own beside the island.
pub fn bubble(facts: Facts) -> bool {
    facts.recording && facts.music && !facts.arriving
}

/// The size a mode wants; `compact` is the width its one-line content needs.
pub fn size_of(mode: Mode, compact: f32) -> Vec2 {
    match mode {
        Mode::MusicOpen => MUSIC_CARD,
        Mode::NoteOpen => NOTE_CARD,
        Mode::RecordingOpen => RECORDING_CARD,
        Mode::Arriving => vec2(compact, ARRIVING_HEIGHT),
        _ => vec2(compact, REST_HEIGHT),
    }
}

/// The corner radius of the island at a size: a stadium in the strip, a rounded
/// card when open. The compositor clips to the same shape (`Command::Island`).
pub fn radius_of(size: Vec2) -> f32 {
    (size.y / 2.0).min(CARD_RADIUS)
}

/// One step of the spring towards `target`: a little bouncy when growing,
/// critically damped when shrinking (so it never dips below where it settles).
fn spring(position: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32) {
    // Growing lasts until it turns back from the overshoot, not until it first
    // passes the target (that would damp the overshoot away).
    let growing = target > position || velocity > 0.0;
    let damping = if growing { GROW_DAMPING } else { 1.0 };
    let acceleration = STIFFNESS * STIFFNESS * (target - position) - 2.0 * damping * STIFFNESS * velocity;
    let velocity = velocity + acceleration * dt;
    (position + velocity * dt, velocity)
}

/// Playing music not heard about for this long is shown as not updating: the
/// service answers the shell's heartbeat every 20 s, so this is two missed.
pub const STALE_AFTER: Duration = Duration::from_secs(45);

/// Whether what the island shows of the music can no longer be trusted. Paused
/// music does not move, so it never goes stale.
pub fn stale(playing: bool, age: Option<Duration>) -> bool {
    playing && age.is_some_and(|age| age >= STALE_AFTER)
}

/// Whether music still shows: playing, or paused for less than `PAUSED_FOR`.
pub fn music_shows(playing: bool, paused_for: Option<Duration>) -> bool {
    playing || paused_for.is_none_or(|paused| paused < PAUSED_FOR)
}

/// Whether an open island left alone for `idle` closes.
pub fn collapse_due(idle: Duration) -> bool {
    idle >= COLLAPSE_AFTER
}

/// The outline's colour: the colour of what the island shows, faint, or the
/// usual line when it shows nothing of its own.
pub fn outline(mode: Mode, accent: Color32) -> Color32 {
    match mode {
        Mode::Recording | Mode::RecordingOpen => RED.gamma_multiply(0.55),
        Mode::Blip | Mode::Task => accent.gamma_multiply(0.45),
        Mode::Music | Mode::MusicOpen | Mode::Arriving | Mode::NoteOpen => accent.gamma_multiply(0.45),
        Mode::Resting | Mode::Waiting | Mode::Level => ui::LINE,
    }
}

/// The playing song.
pub struct Song {
    pub title: String,
    pub artists: String,
    pub cover: Option<TextureHandle>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub playing: bool,
    /// Nothing heard about it for a while: its position is a guess.
    pub stale: bool,
}

/// Everything the island can show this frame.
pub struct Feed {
    /// Who has the keyboard, upper case, and how many applications are open.
    pub holder: String,
    pub apps: Option<usize>,
    pub song: Option<Song>,
    /// How long the screen has been recorded.
    pub recording: Option<Duration>,
    pub level: Option<(osd::Kind, u8)>,
    /// The notification coming in, and how far through its time it is (0 to 1).
    pub arriving: Option<(notify::Notice, f32)>,
    /// How many notifications are waiting, and the newest one.
    pub waiting: usize,
    pub newest: Option<notify::Notice>,
    /// Show who a notification is from, not its text.
    pub private: bool,
    /// Something the system just did.
    pub blip: Option<Blip>,
    /// Something under way.
    pub task: Option<Task>,
    /// Whether an application is using the camera, and the microphone.
    pub watching: (bool, bool),
}

impl Feed {
    fn facts(&self) -> Facts {
        Facts {
            arriving: self.arriving.is_some(),
            level: self.level.is_some(),
            recording: self.recording.is_some(),
            music: self.song.is_some(),
            waiting: self.waiting > 0,
            blip: self.blip.is_some(),
            task: self.task.is_some(),
        }
    }

    /// The notification an open card is about.
    fn note(&self) -> Option<&notify::Notice> {
        self.arriving.as_ref().map(|(notice, _)| notice).or(self.newest.as_ref())
    }
}

/// What the user did with the island.
#[derive(Debug, Clone, PartialEq)]
pub enum Deed {
    Nothing,
    /// One of the words `Music::command` takes: `toggle`, `next`, `prev`.
    Music(&'static str),
    OpenMusic,
    StopRecording,
    Note(u32, NoteDeed),
    /// The notification list.
    OpenNotes,
    /// The pointer rests on a notification coming in: its time stands still.
    Holding(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteDeed {
    /// Looked at: it leaves the island and stays in the list.
    Read,
    /// Answered in the list, where there is room to type.
    Reply,
    /// One of its own actions, by key.
    Act(String),
    /// Clicked: what the notification itself does (its default action), or else
    /// the app it came from.
    Open,
}

/// Whether a mode has a card to open into.
pub fn opens(mode: Mode) -> bool {
    matches!(mode, Mode::Music | Mode::Arriving | Mode::Waiting | Mode::Recording)
}

/// What a click on the island does.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    Nothing,
    Open,
    Close,
    Do(Deed),
}

/// A click opens what the island shows: the music player, the app a notification
/// came from. A recording has no app: a click opens its card, and only the card's
/// button stops it, so a stray click never ends one.
pub fn click(mode: Mode, note: Option<u32>) -> Click {
    match mode {
        Mode::Music | Mode::MusicOpen => Click::Do(Deed::OpenMusic),
        Mode::Arriving | Mode::Waiting | Mode::NoteOpen => note.map_or(Click::Do(Deed::OpenNotes), |id| Click::Do(Deed::Note(id, NoteDeed::Open))),
        Mode::Recording => Click::Open,
        Mode::RecordingOpen => Click::Close,
        Mode::Resting | Mode::Level | Mode::Blip | Mode::Task => Click::Nothing,
    }
}

#[derive(Default)]
pub struct Island {
    open: bool,
    size: Option<Vec2>,
    velocity: Vec2,
    mode: Option<Mode>,
    /// When the song showing was paused, while it is.
    paused_since: Option<Instant>,
    /// While open: when the pointer was last on it (or when it opened).
    touched: Option<Instant>,
    /// Opened by the pointer resting on it: it closes soon after the pointer leaves.
    by_hover: bool,
    /// Since when the pointer rests on it (closed) or has been off it (opened by hover).
    hover_since: Option<Instant>,
    away_since: Option<Instant>,
    /// After a click, the pointer still on it does not open it, until it leaves.
    hover_blocked: bool,
    /// A held press opened it: the click its release makes is not a click.
    held: bool,
}

impl Island {
    /// Shuts the card, if it is open.
    pub fn close(&mut self) {
        self.open = false;
        self.touched = None;
        self.by_hover = false;
        self.away_since = None;
    }

    fn open_now(&mut self, by_hover: bool) {
        self.open = true;
        self.by_hover = by_hover;
        self.touched = None;
        self.away_since = None;
    }

    /// Draws the island centred on `room` (the part of the strip between the left
    /// and right islands), its bottom on the strip's line, growing upward. Returns
    /// where it is (for the compositor to lift, when it reaches above the strip)
    /// and what was done.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, room: Rect, feed: &Feed, animate: bool) -> (Rect, Deed) {
        let mut facts = feed.facts();
        // Paused music stays a minute, then leaves the island.
        match &feed.song {
            // Not while its card is open: that is someone looking at it.
            Some(song) if !song.playing && !self.open => {
                let since = *self.paused_since.get_or_insert_with(Instant::now);
                let paused_for = since.elapsed();
                if !music_shows(false, Some(paused_for)) {
                    facts.music = false;
                } else {
                    ctx.request_repaint_after(PAUSED_FOR - paused_for);
                }
            }
            _ => self.paused_since = None,
        }
        // Nothing left to open: the card closes by itself.
        if self.open && !(facts.music || facts.arriving || facts.waiting || facts.recording) {
            self.close();
        }
        let mode = mode(facts, self.open);
        let compact = self.compact_width(ctx, mode, feed).min(room.width()).max(REST_HEIGHT);
        let target = size_of(mode, compact).min(vec2(room.width(), f32::INFINITY));
        let size = match (self.size, animate) {
            (Some(size), true) => {
                let dt = ctx.input(|input| input.unstable_dt).clamp(0.0, 0.05);
                let (w, vw) = spring(size.x, self.velocity.x, target.x, dt);
                let (h, vh) = spring(size.y, self.velocity.y, target.y, dt);
                self.velocity = vec2(vw, vh);
                // An overshoot never reaches over the left or right island.
                let size = vec2(w.min(room.width()), h);
                if (size - target).length() < 0.5 && self.velocity.length() < 1.0 {
                    self.velocity = Vec2::ZERO;
                    target
                } else {
                    ctx.request_repaint();
                    size
                }
            }
            _ => target,
        };
        self.size = Some(size);
        if self.mode != Some(mode) {
            self.mode = Some(mode);
            ctx.request_repaint();
        }
        let rect = Rect::from_min_size(pos2(room.center().x - size.x / 2.0, room.bottom() - size.y), size);
        // How far the shape has arrived: content fades in as it does.
        let settled = (1.0 - (size - target).length() / 60.0).clamp(0.0, 1.0);

        let mut deed = Deed::Nothing;
        let area = egui::Area::new(Id::new("status-island"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(rect.expand(24.0));
                let response = ui.allocate_rect(rect, Sense::click());
                let radius = radius_of(rect.size());
                let line = match (&mode, &feed.blip) {
                    (Mode::Blip, Some(blip)) => blip_colour(blip, theme.main).gamma_multiply(0.55),
                    _ => outline(mode, theme.main),
                };
                ui.painter().rect(rect, radius, Color32::from_rgba_unmultiplied(10, 14, 18, 247), Stroke::new(1.0_f32, line), StrokeKind::Inside);
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                inner.set_clip_rect(rect);
                // Music that is not updating shows dimmed.
                let dim = matches!(mode, Mode::Music | Mode::MusicOpen) && feed.song.as_ref().is_some_and(|song| song.stale);
                inner.set_opacity(settled * if dim { 0.55 } else { 1.0 });
                let done = draw(&mut inner, theme, mode, rect, feed);
                // Pressed and held: it opens, and the release is not a click.
                let held_for = ui.input(|input| input.pointer.press_start_time().map(|start| input.time - start)).unwrap_or(0.0);
                if !self.open && opens(mode) && response.is_pointer_button_down_on() && held_for >= HOLD_OPEN.as_secs_f64() {
                    self.open_now(false);
                    self.held = true;
                    ctx.request_repaint();
                } else if response.is_pointer_button_down_on() && !self.open && opens(mode) {
                    ctx.request_repaint_after(HOLD_OPEN);
                }
                if done != Deed::Nothing {
                    deed = done;
                } else if response.clicked() {
                    if std::mem::take(&mut self.held) {
                        // The release of the press that opened it.
                    } else {
                        self.hover_blocked = true;
                        match click(mode, feed.note().map(|notice| notice.id)) {
                            Click::Nothing => {}
                            Click::Open => self.open_now(false),
                            Click::Close => self.close(),
                            Click::Do(done) => deed = done,
                        }
                    }
                } else if !ui.input(|input| input.pointer.any_down()) {
                    // A long hold makes no click on release: nothing left to swallow.
                    self.held = false;
                }
                if deed == Deed::Nothing && response.hovered() {
                    // A notification's time stands still while the pointer is on it.
                    if let Some((notice, _)) = &feed.arriving {
                        deed = Deed::Holding(notice.id);
                    }
                }
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label(mode, feed)));
            });
        let on_it = ctx.input(|input| input.pointer.hover_pos()).is_some_and(|at| rect.contains(at));
        let now = Instant::now();
        // Resting the pointer on it opens it; opened that way, it closes soon after
        // the pointer leaves.
        if !on_it {
            self.hover_blocked = false;
            self.hover_since = None;
        }
        if !self.open && on_it && opens(mode) && !self.hover_blocked {
            let since = *self.hover_since.get_or_insert(now);
            if now - since >= HOVER_OPEN {
                self.open_now(true);
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(HOVER_OPEN - (now - since));
            }
        }
        if self.open && self.by_hover {
            if on_it {
                self.away_since = None;
            } else {
                let away = *self.away_since.get_or_insert(now);
                if now - away >= LEAVE_CLOSE {
                    self.close();
                    self.hover_since = None;
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(LEAVE_CLOSE - (now - away));
                }
            }
        }
        // Left alone, it closes by itself; the pointer on it keeps it open.
        if self.open {
            let touched = *self.touched.get_or_insert(now);
            if on_it {
                self.touched = Some(now);
                ctx.request_repaint_after(COLLAPSE_AFTER);
            } else if collapse_due(now - touched) {
                self.close();
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(COLLAPSE_AFTER - (now - touched));
            }
        } else {
            self.touched = None;
        }
        // Away from it, or Escape, shuts the card. A click in an application never
        // reaches the shell: the compositor sends `island-away` for that (close()).
        if self.open {
            let pressed_away = ctx.input(|input| input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|at| !rect.contains(at)));
            if pressed_away || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.close();
                ctx.request_repaint();
            }
        }
        if bubble(facts) {
            let at = pos2(rect.right() + 8.0, room.bottom() - REST_HEIGHT);
            if at.x + REST_HEIGHT <= room.right() {
                let dot = Rect::from_min_size(at, Vec2::splat(REST_HEIGHT));
                let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("status-island-bubble")));
                painter.rect(dot, REST_HEIGHT / 2.0, Color32::from_rgba_unmultiplied(10, 14, 18, 247), Stroke::new(1.0_f32, outline(Mode::Recording, theme.main)), StrokeKind::Inside);
                painter.circle_filled(dot.center(), 4.0, RED);
            }
        }
        // A camera (green) or a microphone (orange) in use: a dot just left of the
        // island, at the strip's height, whatever the island shows.
        let (camera, microphone) = feed.watching;
        if camera || microphone {
            let at = pos2(rect.left() - 10.0, room.bottom() - REST_HEIGHT / 2.0);
            if at.x - 4.0 >= room.left() {
                let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("status-island-watching")));
                painter.circle_filled(at, 4.0, if camera { GREEN } else { ORANGE });
            }
        }
        if facts.recording || facts.arriving || feed.song.as_ref().is_some_and(|song| song.playing) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        let _ = area;
        (rect, deed)
    }

    /// The width a one-line mode needs.
    fn compact_width(&self, ctx: &egui::Context, mode: Mode, feed: &Feed) -> f32 {
        let width = |text: &str, font: egui::FontId| ctx.fonts(|fonts| fonts.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x);
        let resting = 20.0 + 16.0 + 8.0 + width(&feed.holder, ui::display_bold(12.0))
            + feed.apps.map_or(0.0, |count| 8.0 + width(&apps_text(count), ui::mono(11.0))) + 20.0;
        match mode {
            Mode::Resting => resting,
            Mode::Waiting => resting + 1.0 + 16.0 + 66.0,
            Mode::Music => feed.song.as_ref().map_or(resting, |song| {
                7.0 + 30.0 + 10.0 + width(&truncate(&song.title, 28), ui::display_bold(13.0)) + 6.0
                    + width(&format!("· {}", truncate(&song.artists, 22)), ui::display(13.0)) + 12.0 + 22.0 + 16.0
            }),
            Mode::Recording => 18.0 + 8.0 + 10.0 + width("REC", ui::mono(12.0)) + 8.0 + width("00:00", ui::mono(13.0)) + 20.0,
            Mode::Level => 18.0 + 20.0 + 12.0 + 180.0 + 12.0 + 34.0 + 18.0,
            Mode::Arriving => 360.0,
            Mode::MusicOpen | Mode::NoteOpen => CARD_WIDTH,
            Mode::RecordingOpen => RECORDING_CARD.x,
            Mode::Blip => feed.blip.as_ref().map_or(resting, |blip| {
                18.0 + 24.0 + 10.0 + width(&truncate(&blip.title, 26), ui::display_bold(13.0)) + 8.0
                    + width(&blip.detail, ui::mono(12.0)) + if blip.level.is_some() { 12.0 + 44.0 } else { 0.0 } + 20.0
            }),
            Mode::Task => feed.task.as_ref().map_or(resting, |task| {
                18.0 + 20.0 + 10.0 + width(&task.title, ui::display_bold(13.0)) + 8.0
                    + width(&truncate(&task.detail, 22), ui::display(13.0)) + 10.0 + width("100%", ui::mono(12.0)) + 20.0
            }),
        }
    }
}

fn apps_text(count: usize) -> String {
    format!("· {count} {}", if count == 1 { "APP" } else { "APPS" })
}

fn truncate(text: &str, most: usize) -> String {
    if text.chars().count() <= most { text.to_string() } else { format!("{}…", text.chars().take(most - 1).collect::<String>()) }
}

/// Two letters for whoever sent a notification.
pub fn initials(notice: &notify::Notice) -> String {
    let source = if notice.title.trim().is_empty() { &notice.app } else { &notice.title };
    let mut letters = source.split_whitespace().filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    let first = letters.next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "·".into());
    let second = letters.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    format!("{first}{second}")
}

fn label(mode: Mode, feed: &Feed) -> String {
    match mode {
        Mode::Resting => format!("{} has the keyboard", feed.holder.to_lowercase()),
        Mode::Waiting => format!("{} notifications waiting. Open them", feed.waiting),
        Mode::Music | Mode::MusicOpen => feed.song.as_ref().map_or_else(String::new, |song| format!("{}, {}. Open the player", song.title, song.artists)),
        Mode::Arriving | Mode::NoteOpen => feed.note().map_or_else(String::new, |notice| format!("Notification from {}: {}", notice.app, notice.title)),
        Mode::Recording => "Recording the screen. Open".into(),
        Mode::RecordingOpen => "Recording the screen".into(),
        Mode::Level => "Level".into(),
        Mode::Blip => feed.blip.as_ref().map_or_else(String::new, |blip| format!("{}: {}", blip.title, blip.detail)),
        Mode::Task => feed.task.as_ref().map_or_else(String::new, |task| format!("{}: {}", task.title, task.detail)),
    }
}

/// Text at a point, left aligned and vertically centred; returns its width.
fn put(ui: &Ui, at: Pos2, text: &str, font: egui::FontId, color: Color32) -> f32 {
    let galley = ui::galley(ui, text, font, color, 0.0);
    let width = galley.size().x;
    ui.painter().galley(pos2(at.x, at.y - galley.size().y / 2.0), galley, color);
    width
}

/// A round, clickable spot; returns whether it was clicked.
fn round_button(ui: &mut Ui, centre: Pos2, size: f32, fill: Color32, name: &str) -> (bool, Rect) {
    let rect = Rect::from_center_size(centre, Vec2::splat(size));
    let response = ui.interact(rect, ui.id().with(("island-button", name)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let fill = if response.hovered() && fill == Color32::TRANSPARENT { ui::LINE } else { fill };
    ui.painter().circle_filled(centre, size / 2.0, fill);
    (response.clicked(), rect)
}

fn play_shape(painter: &egui::Painter, centre: Pos2, playing: bool, color: Color32) {
    if playing {
        for dx in [-3.5, 3.5] {
            painter.rect_filled(Rect::from_center_size(centre + vec2(dx, 0.0), vec2(4.0, 13.0)), 1.0, color);
        }
    } else {
        painter.add(egui::Shape::convex_polygon(vec![centre + vec2(-4.0, -7.0), centre + vec2(7.0, 0.0), centre + vec2(-4.0, 7.0)], color, Stroke::NONE));
    }
}

fn skip_shape(painter: &egui::Painter, centre: Pos2, forward: bool, color: Color32) {
    let sign = if forward { 1.0 } else { -1.0 };
    painter.add(egui::Shape::convex_polygon(vec![centre + vec2(-5.0 * sign, -6.0), centre + vec2(4.0 * sign, 0.0), centre + vec2(-5.0 * sign, 6.0)], color, Stroke::NONE));
    painter.rect_filled(Rect::from_center_size(centre + vec2(5.5 * sign, 0.0), vec2(2.0, 12.0)), 0.5, color);
}

fn cover(ui: &Ui, rect: Rect, song: &Song, accent: Color32, radius: f32) {
    match &song.cover {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            egui::Image::new(texture).uv(uv).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, radius, ui::LINE_STRONG);
            ui.painter().circle_filled(rect.center(), rect.width() * 0.18, accent);
        }
    }
}

/// An event's colour: red when it warns, green while charging, else the accent.
fn blip_colour(blip: &Blip, accent: Color32) -> Color32 {
    match blip.icon {
        _ if blip.warn => RED,
        BlipIcon::Battery { charging: true } => GREEN,
        _ => accent,
    }
}

/// A small battery, filled to `level`, with a bolt while charging.
fn battery_shape(painter: &egui::Painter, centre: Pos2, level: u8, charging: bool, colour: Color32) {
    let body = Rect::from_center_size(centre - vec2(1.0, 0.0), vec2(20.0, 11.0));
    painter.rect(body, 3.0, Color32::TRANSPARENT, Stroke::new(1.2_f32, ui::SECONDARY), StrokeKind::Inside);
    painter.rect_filled(Rect::from_center_size(pos2(body.right() + 1.5, centre.y), vec2(2.0, 4.0)), 1.0, ui::SECONDARY);
    let inner = body.shrink(2.0);
    painter.rect_filled(Rect::from_min_size(inner.min, vec2(inner.width() * level.min(100) as f32 / 100.0, inner.height())), 1.5, colour);
    if charging {
        let c = body.center();
        painter.add(egui::Shape::convex_polygon(vec![c + vec2(1.0, -5.5), c + vec2(-3.0, 0.8), c + vec2(0.0, 0.8)], ui::TEXT, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -0.8), c + vec2(3.0, -0.8), c + vec2(-1.0, 5.5)], ui::TEXT, Stroke::NONE));
    }
}

fn clock(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The content of each mode inside `rect`.
fn draw(ui: &mut Ui, theme: &Theme, mode: Mode, rect: Rect, feed: &Feed) -> Deed {
    let accent = theme.main;
    let middle = rect.center().y;
    let mut deed = Deed::Nothing;
    match mode {
        Mode::Resting | Mode::Waiting => {
            let mut x = rect.left() + 20.0;
            ui::paint_icon(ui.painter(), Rect::from_center_size(pos2(x + 8.0, middle), Vec2::splat(16.0)), ui::Icon::Keyboard, ui::SECONDARY, false);
            x += 24.0;
            x += put(ui, pos2(x, middle), &feed.holder, ui::display_bold(12.0), ui::TEXT) + 8.0;
            if let Some(count) = feed.apps {
                x += put(ui, pos2(x, middle), &apps_text(count), ui::mono(11.0), ui::SECONDARY) + 8.0;
            }
            if mode == Mode::Waiting {
                ui.painter().line_segment([pos2(x, middle - 9.0), pos2(x, middle + 9.0)], Stroke::new(1.0_f32, ui::LINE));
                let chip = Rect::from_min_size(pos2(x + 8.0, middle - 15.0), vec2(58.0, 30.0));
                let tint = accent.gamma_multiply(0.18);
                ui.painter().rect_filled(chip, 15.0, tint);
                let face = Rect::from_center_size(pos2(chip.left() + 15.0, middle), Vec2::splat(22.0));
                ui.painter().circle_filled(face.center(), 11.0, accent);
                let who = feed.newest.as_ref().map(initials).unwrap_or_default();
                ui.painter().text(face.center(), Align2::CENTER_CENTER, who, ui::mono(10.0), ui::GROUND);
                put(ui, pos2(chip.left() + 31.0, middle), &format!("+{}", feed.waiting.saturating_sub(1).max(1)), ui::mono(11.0), accent);
            }
        }
        Mode::Music => {
            let Some(song) = &feed.song else { return deed };
            let art = Rect::from_center_size(pos2(rect.left() + 22.0, middle), Vec2::splat(30.0));
            cover(ui, art, song, accent, 15.0);
            let mut x = art.right() + 10.0;
            x += put(ui, pos2(x, middle), &truncate(&song.title, 28), ui::display_bold(13.0), ui::TEXT) + 6.0;
            put(ui, pos2(x, middle), &format!("· {}", truncate(&song.artists, 22)), ui::display(13.0), ui::SECONDARY);
            // The level: four bars that move while it plays.
            let time = ui.input(|input| input.time) as f32;
            for (i, phase) in [0.0_f32, 1.7, 3.1, 4.6].into_iter().enumerate() {
                let lift = if song.playing && !song.stale { 0.5 + 0.5 * (time * 7.0 + phase).sin() } else { 0.25 };
                let height = 4.0 + 10.0 * lift;
                let bar = Rect::from_min_size(pos2(rect.right() - 38.0 + i as f32 * 6.0, middle + 7.0 - height), vec2(3.0, height));
                ui.painter().rect_filled(bar, 1.5, accent);
            }
            if song.duration_ms > 0 {
                let line = Rect::from_min_max(pos2(rect.left() + 22.0, rect.bottom() - 2.0), pos2(rect.right() - 22.0, rect.bottom() - 1.0));
                ui.painter().rect_filled(line, 0.0, ui::LINE);
                let done = song.position_ms as f32 / song.duration_ms as f32;
                ui.painter().rect_filled(Rect::from_min_size(line.min, vec2(line.width() * done.clamp(0.0, 1.0), line.height())), 0.0, accent);
            }
        }
        Mode::MusicOpen => {
            let Some(song) = &feed.song else { return deed };
            let art = Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(rect.height() - 32.0));
            cover(ui, art, song, accent, 18.0);
            let left = art.right() + 16.0;
            let right = rect.right() - 16.0;
            put(ui, pos2(left, rect.top() + 26.0), &truncate(&song.title, 26), ui::display_bold(16.0), ui::TEXT);
            put(ui, pos2(left, rect.top() + 46.0), &truncate(&song.artists, 30), ui::display(13.0), ui::SECONDARY);
            if song.stale {
                put(ui, pos2(left, rect.top() + 8.0), "NOT UPDATING", ui::mono(9.0), RED);
            }
            let bar = Rect::from_min_max(pos2(left, rect.top() + 72.0), pos2(right, rect.top() + 76.0));
            ui.painter().rect_filled(bar, 2.0, ui::LINE);
            if song.duration_ms > 0 {
                let done = (song.position_ms as f32 / song.duration_ms as f32).clamp(0.0, 1.0);
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * done, bar.height())), 2.0, accent);
                put(ui, pos2(left, bar.bottom() + 10.0), &clock(song.position_ms), ui::mono(11.0), ui::SECONDARY);
                let left_ms = song.duration_ms.saturating_sub(song.position_ms);
                let rest = format!("-{}", clock(left_ms));
                let galley = ui::galley(ui, &rest, ui::mono(11.0), ui::SECONDARY, 0.0);
                ui.painter().galley(pos2(right - galley.size().x, bar.bottom() + 10.0 - galley.size().y / 2.0), galley, ui::SECONDARY);
            }
            let row = rect.bottom() - 30.0;
            let centre = (left + right) / 2.0;
            let (prev, r) = round_button(ui, pos2(centre - 52.0, row), 32.0, Color32::TRANSPARENT, "Previous");
            skip_shape(ui.painter(), r.center(), false, ui::TEXT);
            let (toggle, r) = round_button(ui, pos2(centre, row), 38.0, accent, if song.playing { "Pause" } else { "Play" });
            play_shape(ui.painter(), r.center(), song.playing, ui::GROUND);
            let (next, r) = round_button(ui, pos2(centre + 52.0, row), 32.0, Color32::TRANSPARENT, "Next");
            skip_shape(ui.painter(), r.center(), true, ui::TEXT);
            let (open, r) = round_button(ui, pos2(right - 12.0, rect.top() + 26.0), 26.0, Color32::TRANSPARENT, "Open Music");
            ui::paint_icon(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(14.0)), ui::Icon::Music, ui::SECONDARY, false);
            deed = if prev { Deed::Music("prev") } else if toggle { Deed::Music("toggle") } else if next { Deed::Music("next") } else if open { Deed::OpenMusic } else { deed };
        }
        Mode::Arriving => {
            let Some((notice, through)) = &feed.arriving else { return deed };
            let face = Rect::from_center_size(pos2(rect.left() + 26.0, middle), Vec2::splat(36.0));
            ui.painter().circle_filled(face.center(), 18.0, accent.gamma_multiply(0.18));
            ui.painter().text(face.center(), Align2::CENTER_CENTER, initials(notice), ui::display_bold(13.0), accent);
            let left = face.right() + 10.0;
            let title = if notice.title.trim().is_empty() { notice.app.clone() } else { notice.title.replace('\n', " ") };
            let body = if feed.private { String::new() } else { notify::plain(&notice.body).replace('\n', " ") };
            // One line, centred, when there is nothing more to say.
            let head = if body.trim().is_empty() { middle } else { middle - 9.0 };
            let mut x = left + put(ui, pos2(left, head), &truncate(&title, 30), ui::display_bold(13.0), ui::TEXT);
            if !notice.title.trim().is_empty() && !notice.app.is_empty() {
                x += 6.0;
                put(ui, pos2(x, head), &format!("· {}", truncate(&notice.app, 18)), ui::display(13.0), ui::SECONDARY);
            }
            if !body.trim().is_empty() {
                put(ui, pos2(left, middle + 9.0), &truncate(&body, 46), ui::display(12.0), ui::SECONDARY);
            }
            // Its time running out, along the bottom edge.
            let line = Rect::from_min_max(pos2(rect.left() + 26.0, rect.bottom() - 2.0), pos2(rect.right() - 26.0, rect.bottom() - 1.0));
            ui.painter().rect_filled(Rect::from_min_size(line.min, vec2(line.width() * (1.0 - through.clamp(0.0, 1.0)), 1.0)), 0.0, accent.gamma_multiply(0.6));
        }
        Mode::NoteOpen => {
            let Some(notice) = feed.note() else { return deed };
            let face = Rect::from_center_size(pos2(rect.left() + 34.0, rect.top() + 32.0), Vec2::splat(32.0));
            ui.painter().circle_filled(face.center(), 16.0, accent.gamma_multiply(0.18));
            ui.painter().text(face.center(), Align2::CENTER_CENTER, initials(notice), ui::display_bold(12.0), accent);
            let left = face.right() + 10.0;
            put(ui, pos2(left, rect.top() + 25.0), &truncate(if notice.title.trim().is_empty() { &notice.app } else { &notice.title }, 30), ui::display_bold(14.0), ui::TEXT);
            put(ui, pos2(left, rect.top() + 42.0), &format!("{} · {}", notice.app.to_uppercase(), notice.clock), ui::mono(11.0), ui::SECONDARY);
            if feed.waiting > 1 {
                let count = format!("{} WAITING", feed.waiting);
                let galley = ui::galley(ui, &count, ui::mono(11.0), ui::SECONDARY, 0.0);
                ui.painter().galley(pos2(rect.right() - 18.0 - galley.size().x, rect.top() + 25.0 - galley.size().y / 2.0), galley, ui::SECONDARY);
            }
            let (text, colour) = if feed.private { ("Hidden in the island. Open to read it.".to_string(), ui::SECONDARY) } else { (notify::plain(&notice.body), ui::TEXT) };
            let body = ui.painter().layout(text, ui::display(13.0), colour, rect.width() - 36.0);
            let clip = Rect::from_min_max(pos2(rect.left() + 18.0, rect.top() + 58.0), pos2(rect.right() - 18.0, rect.bottom() - 48.0));
            ui.painter().with_clip_rect(clip).galley(clip.min, body, colour);
            // Its own actions, then what is always possible.
            let mut actions: Vec<(String, String)> = notice.actions.iter().filter(|(key, _)| key != "default").take(2).cloned().collect();
            let has_reply = actions.iter().any(|(key, _)| key == "inline-reply");
            if !has_reply && actions.len() < 2 { actions.push(("@read".into(), "Mark read".into())); }
            actions.push(("@open".into(), "Open".into()));
            let row = Rect::from_min_max(pos2(rect.left() + 18.0, rect.bottom() - 44.0), pos2(rect.right() - 18.0, rect.bottom() - 12.0));
            let each = (row.width() - 8.0 * (actions.len() as f32 - 1.0)) / actions.len() as f32;
            for (i, (key, words)) in actions.iter().enumerate() {
                let button = Rect::from_min_size(pos2(row.left() + i as f32 * (each + 8.0), row.top()), vec2(each, row.height()));
                let response = ui.interact(button, ui.id().with(("island-action", key.as_str())), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, words.as_str()));
                let first = i == 0;
                if first {
                    ui.painter().rect_filled(button, 16.0, if response.hovered() { accent.gamma_multiply(0.85) } else { accent });
                } else {
                    ui.painter().rect(button, 16.0, if response.hovered() { ui::LINE } else { Color32::TRANSPARENT }, Stroke::new(1.0_f32, ui::LINE_STRONG), StrokeKind::Inside);
                }
                ui.painter().text(button.center(), Align2::CENTER_CENTER, truncate(words, 14), ui::display_bold(12.0), if first { ui::GROUND } else { ui::TEXT });
                if response.clicked() {
                    deed = match key.as_str() {
                        "@read" => Deed::Note(notice.id, NoteDeed::Read),
                        "@open" => Deed::OpenNotes,
                        "inline-reply" => Deed::Note(notice.id, NoteDeed::Reply),
                        key => Deed::Note(notice.id, NoteDeed::Act(key.to_string())),
                    };
                }
            }
        }
        Mode::Recording => {
            let Some(length) = feed.recording else { return deed };
            let dot = pos2(rect.left() + 22.0, middle);
            ui.painter().circle_filled(dot, 7.0, RED.gamma_multiply(0.18));
            ui.painter().circle_filled(dot, 4.0, RED);
            let mut x = dot.x + 14.0;
            x += put(ui, pos2(x, middle), "REC", ui::mono(12.0), ui::TEXT) + 8.0;
            let seconds = length.as_secs();
            put(ui, pos2(x, middle), &format!("{:02}:{:02}", seconds / 60, seconds % 60), ui::mono(13.0), ui::TEXT);
        }
        Mode::RecordingOpen => {
            let Some(length) = feed.recording else { return deed };
            let dot = pos2(rect.left() + 26.0, rect.top() + 30.0);
            ui.painter().circle_filled(dot, 8.0, RED.gamma_multiply(0.18));
            ui.painter().circle_filled(dot, 4.5, RED);
            put(ui, pos2(dot.x + 16.0, dot.y), "Recording the screen", ui::display_bold(14.0), ui::TEXT);
            let seconds = length.as_secs();
            let time = format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
            let galley = ui::galley(ui, &time, ui::mono(13.0), ui::SECONDARY, 0.0);
            ui.painter().galley(pos2(rect.right() - 20.0 - galley.size().x, dot.y - galley.size().y / 2.0), galley, ui::SECONDARY);
            let button = Rect::from_min_max(pos2(rect.left() + 18.0, rect.bottom() - 50.0), pos2(rect.right() - 18.0, rect.bottom() - 16.0));
            let response = ui.interact(button, ui.id().with("island-stop-recording"), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Stop recording"));
            ui.painter().rect_filled(button, 17.0, if response.hovered() { RED.gamma_multiply(0.85) } else { RED });
            let square = Rect::from_center_size(pos2(button.center().x - 52.0, button.center().y), Vec2::splat(10.0));
            ui.painter().rect_filled(square, 2.0, ui::GROUND);
            ui.painter().text(pos2(square.right() + 10.0, button.center().y), Align2::LEFT_CENTER, "Stop recording", ui::display_bold(13.0), ui::GROUND);
            if response.clicked() { deed = Deed::StopRecording; }
        }
        Mode::Blip => {
            let Some(blip) = &feed.blip else { return deed };
            let colour = blip_colour(blip, accent);
            let icon_at = pos2(rect.left() + 30.0, middle);
            match blip.icon {
                BlipIcon::Battery { charging } => battery_shape(ui.painter(), icon_at, blip.level.unwrap_or(0), charging, colour),
                BlipIcon::Bluetooth => ui::paint_icon(ui.painter(), Rect::from_center_size(icon_at, Vec2::splat(18.0)), ui::Icon::Bluetooth, colour, false),
                BlipIcon::Moon => ui::paint_icon(ui.painter(), Rect::from_center_size(icon_at, Vec2::splat(18.0)), ui::Icon::Moon, colour, false),
            }
            let mut x = rect.left() + 52.0;
            x += put(ui, pos2(x, middle), &truncate(&blip.title, 26), ui::display_bold(13.0), ui::TEXT) + 8.0;
            put(ui, pos2(x, middle), &blip.detail, ui::mono(12.0), if blip.warn { RED } else { ui::SECONDARY });
            if let Some(level) = blip.level {
                let track = Rect::from_min_max(pos2(rect.right() - 20.0 - 44.0, middle - 2.5), pos2(rect.right() - 20.0, middle + 2.5));
                ui.painter().rect_filled(track, 2.5, ui::LINE);
                ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(track.width() * level.min(100) as f32 / 100.0, track.height())), 2.5, colour);
            }
        }
        Mode::Task => {
            let Some(task) = &feed.task else { return deed };
            ui::paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 28.0, middle), Vec2::splat(16.0)), ui::Icon::Download, accent, false);
            let mut x = rect.left() + 48.0;
            x += put(ui, pos2(x, middle), &task.title, ui::display_bold(13.0), ui::TEXT) + 8.0;
            put(ui, pos2(x, middle), &truncate(&task.detail, 22), ui::display(13.0), ui::SECONDARY);
            if let Some(progress) = task.progress {
                let words = format!("{}%", (progress * 100.0).round() as u32);
                let galley = ui::galley(ui, &words, ui::mono(12.0), ui::TEXT, 0.0);
                ui.painter().galley(pos2(rect.right() - 20.0 - galley.size().x, middle - galley.size().y / 2.0), galley, ui::TEXT);
                let line = Rect::from_min_max(pos2(rect.left() + 22.0, rect.bottom() - 2.0), pos2(rect.right() - 22.0, rect.bottom() - 1.0));
                ui.painter().rect_filled(line, 0.0, ui::LINE);
                ui.painter().rect_filled(Rect::from_min_size(line.min, vec2(line.width() * progress.clamp(0.0, 1.0), line.height())), 0.0, accent);
            }
        }
        Mode::Level => {
            let Some((kind, level)) = feed.level else { return deed };
            let icon = match kind { osd::Kind::Volume if level == 0 => ui::Icon::Muted, osd::Kind::Volume => ui::Icon::Volume, osd::Kind::Brightness => ui::Icon::Brightness };
            ui::paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 28.0, middle), Vec2::splat(20.0)), icon, ui::TEXT, false);
            let track = Rect::from_min_max(pos2(rect.left() + 50.0, middle - 3.0), pos2(rect.right() - 64.0, middle + 3.0));
            ui.painter().rect_filled(track, 3.0, ui::LINE);
            ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(track.width() * level as f32 / 100.0, track.height())), 3.0, accent);
            let words = format!("{level}%");
            let galley = ui::galley(ui, &words, ui::mono(12.0), ui::TEXT, 0.0);
            ui.painter().galley(pos2(rect.right() - 18.0 - galley.size().x, middle - galley.size().y / 2.0), galley, ui::TEXT);
        }
    }
    deed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(arriving: bool, level: bool, recording: bool, music: bool, waiting: bool) -> Facts {
        Facts { arriving, level, recording, music, waiting, ..Default::default() }
    }

    #[test]
    fn an_event_comes_after_a_level_and_a_download_after_a_recording() {
        let event = Facts { blip: true, music: true, ..Default::default() };
        assert_eq!(mode(event, false), Mode::Blip);
        assert_eq!(mode(Facts { level: true, ..event }, false), Mode::Level);
        let download = Facts { task: true, waiting: true, ..Default::default() };
        assert_eq!(mode(download, false), Mode::Task);
        assert_eq!(mode(Facts { recording: true, ..download }, false), Mode::Recording);
        assert_eq!(mode(Facts { music: true, ..download }, false), Mode::Music);
        assert_eq!(click(Mode::Blip, None), Click::Nothing);
        assert!(!opens(Mode::Task));
    }

    #[test]
    fn durations_read_as_words() {
        assert_eq!(duration_words(45), "45 min");
        assert_eq!(duration_words(120), "2 h");
        assert_eq!(duration_words(200), "3 h 20 min");
    }

    #[test]
    fn the_most_important_thing_takes_the_island() {
        assert_eq!(mode(facts(false, false, false, false, false), false), Mode::Resting);
        assert_eq!(mode(facts(true, true, true, true, true), false), Mode::Arriving);
        assert_eq!(mode(facts(false, true, true, true, true), false), Mode::Level);
        assert_eq!(mode(facts(false, false, true, true, true), false), Mode::Music);
        assert_eq!(mode(facts(false, false, true, false, true), false), Mode::Recording);
        assert_eq!(mode(facts(false, false, false, false, true), false), Mode::Waiting);
    }

    #[test]
    fn opening_means_something_only_for_music_and_notifications() {
        assert_eq!(mode(facts(false, false, false, true, false), true), Mode::MusicOpen);
        assert_eq!(mode(facts(true, false, false, true, false), true), Mode::NoteOpen);
        assert_eq!(mode(facts(false, false, false, false, true), true), Mode::NoteOpen);
        assert_eq!(mode(facts(false, false, true, false, false), true), Mode::RecordingOpen);
        assert_eq!(mode(facts(false, false, false, false, false), true), Mode::Resting);
    }

    #[test]
    fn a_recording_beside_music_is_a_bubble_of_its_own() {
        assert!(bubble(facts(false, false, true, true, false)));
        assert!(!bubble(facts(false, false, true, false, false)));
        assert!(!bubble(facts(true, false, true, true, false)));
    }

    #[test]
    fn cards_grow_upward_and_one_line_modes_stay_in_the_strip() {
        assert_eq!(size_of(Mode::Resting, 214.0), vec2(214.0, REST_HEIGHT));
        assert_eq!(size_of(Mode::Music, 300.0).y, REST_HEIGHT);
        assert!(size_of(Mode::Arriving, 360.0).y > REST_HEIGHT);
        assert_eq!(size_of(Mode::MusicOpen, 300.0), MUSIC_CARD);
        assert_eq!(size_of(Mode::NoteOpen, 300.0), NOTE_CARD);
    }

    #[test]
    fn growing_overshoots_a_little_and_settles() {
        let (mut x, mut v) = (214.0_f32, 0.0_f32);
        let mut peak = x;
        for _ in 0..120 {
            (x, v) = spring(x, v, 400.0, 1.0 / 60.0);
            peak = peak.max(x);
        }
        assert!((x - 400.0).abs() < 0.5, "{x}");
        assert!(peak > 400.5, "no overshoot: peak {peak}");
        assert!(peak < 400.0 + 186.0 * 0.05, "overshot too far, to {peak}");
    }

    #[test]
    fn shrinking_never_dips_below_where_it_settles() {
        let (mut x, mut v) = (400.0_f32, 0.0_f32);
        let mut low = x;
        for _ in 0..120 {
            (x, v) = spring(x, v, 214.0, 1.0 / 60.0);
            low = low.min(x);
        }
        assert!((x - 214.0).abs() < 0.5, "{x}");
        assert!(low >= 213.5, "dipped to {low}");
    }

    #[test]
    fn a_click_opens_what_the_island_shows() {
        assert_eq!(click(Mode::Music, None), Click::Do(Deed::OpenMusic));
        assert_eq!(click(Mode::MusicOpen, None), Click::Do(Deed::OpenMusic));
        assert_eq!(click(Mode::Arriving, Some(7)), Click::Do(Deed::Note(7, NoteDeed::Open)));
        assert_eq!(click(Mode::Waiting, None), Click::Do(Deed::OpenNotes));
        assert_eq!(click(Mode::Resting, None), Click::Nothing);
    }

    #[test]
    fn a_stray_click_never_stops_a_recording() {
        assert_eq!(click(Mode::Recording, None), Click::Open);
        assert_eq!(click(Mode::RecordingOpen, None), Click::Close);
        assert!(opens(Mode::Recording));
        assert!(!opens(Mode::Resting) && !opens(Mode::Level));
    }

    #[test]
    fn an_open_island_left_alone_closes() {
        assert!(!collapse_due(Duration::ZERO));
        assert!(!collapse_due(COLLAPSE_AFTER - Duration::from_millis(1)));
        assert!(collapse_due(COLLAPSE_AFTER));
    }

    #[test]
    fn closing_forgets_when_it_was_touched() {
        let mut island = Island { open: true, touched: Some(Instant::now()), ..Default::default() };
        island.close();
        assert!(!island.open);
        assert!(island.touched.is_none());
    }

    #[test]
    fn playing_music_unheard_of_goes_stale_and_paused_never_does() {
        assert!(!stale(true, None));
        assert!(!stale(true, Some(Duration::from_secs(30))));
        assert!(stale(true, Some(STALE_AFTER)));
        assert!(!stale(false, Some(Duration::from_secs(600))));
    }

    #[test]
    fn paused_music_leaves_after_a_minute() {
        assert!(music_shows(true, None));
        assert!(music_shows(false, None));
        assert!(music_shows(false, Some(Duration::from_secs(59))));
        assert!(!music_shows(false, Some(PAUSED_FOR)));
        // Playing again brings it straight back.
        assert!(music_shows(true, Some(Duration::from_secs(600))));
    }

    #[test]
    fn the_outline_takes_the_colour_of_what_it_shows() {
        let accent = Color32::from_rgb(0x4F, 0xB3, 0xFF);
        assert_eq!(outline(Mode::Resting, accent), ui::LINE);
        assert_eq!(outline(Mode::Recording, accent), RED.gamma_multiply(0.55));
        assert_eq!(outline(Mode::Music, accent), accent.gamma_multiply(0.45));
    }

    #[test]
    fn initials_come_from_the_title_or_the_app() {
        let mut notice = notify::Notice::sample("WhatsApp", "Ama Kwarteng", "See you at 6?");
        assert_eq!(initials(&notice), "AK");
        notice.title = String::new();
        assert_eq!(initials(&notice), "W");
    }
}
