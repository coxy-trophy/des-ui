//! The status island: the centre of the status strip, which grows to show what is
//! going on (design: the "DEs-UI Status Strip" canvas, "Island · every state").
//!
//! At rest it is what the centre always was: who has the keyboard and how many
//! applications are open. Something live widens it (music, a recording), a
//! notification or a volume change grows it for a moment, and a click opens it
//! upward into a card. It never takes the keyboard (the compositor lifts it above
//! the applications with `Command::Island`, which leaves focus alone), and it never
//! grows over the left or right island.
//!
//! What it shows is decided by `mode`, its size by `size_of`; both are plain and
//! tested. Moving between sizes is a critically damped spring.

use crate::{notify, osd, theme::Theme, ui};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};
use std::time::Duration;

/// Height of the island in the strip, as the other islands.
pub const REST_HEIGHT: f32 = crate::panels::ISLAND_HEIGHT;
/// How long a notification shows in the island before it settles into the count.
pub const ARRIVING_FOR: Duration = Duration::from_secs(4);
const CARD_WIDTH: f32 = 400.0;
const MUSIC_CARD: Vec2 = vec2(CARD_WIDTH, 156.0);
const NOTE_CARD: Vec2 = vec2(CARD_WIDTH, 164.0);
const ARRIVING_HEIGHT: f32 = 52.0;
/// The largest corner: cards are rounded, not stadiums.
const CARD_RADIUS: f32 = 28.0;
/// The spring's natural frequency: settles in about a third of a second.
const STIFFNESS: f32 = 16.0;
const RED: Color32 = Color32::from_rgb(0xFF, 0x5D, 0x5D);

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
    /// Volume or brightness, for a moment.
    Level,
}

/// What is going on, as far as choosing a mode goes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    pub arriving: bool,
    pub level: bool,
    pub recording: bool,
    pub music: bool,
    pub waiting: bool,
}

/// The mode, most important first: a notification coming in, then a level being
/// changed, then music, then a recording (which, with music, shows as a bubble
/// beside it instead), then notifications waiting. Opening only means something
/// for music and notifications.
pub fn mode(facts: Facts, open: bool) -> Mode {
    if facts.arriving {
        return if open { Mode::NoteOpen } else { Mode::Arriving };
    }
    if facts.level {
        return Mode::Level;
    }
    if facts.music {
        return if open { Mode::MusicOpen } else { Mode::Music };
    }
    if facts.recording {
        return Mode::Recording;
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
        Mode::Arriving => vec2(compact, ARRIVING_HEIGHT),
        _ => vec2(compact, REST_HEIGHT),
    }
}

/// One step of a critically damped spring towards `target`.
fn spring(position: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32) {
    let acceleration = STIFFNESS * STIFFNESS * (target - position) - 2.0 * STIFFNESS * velocity;
    let velocity = velocity + acceleration * dt;
    (position + velocity * dt, velocity)
}

/// The playing song.
pub struct Song {
    pub title: String,
    pub artists: String,
    pub cover: Option<TextureHandle>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub playing: bool,
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
}

impl Feed {
    fn facts(&self) -> Facts {
        Facts {
            arriving: self.arriving.is_some(),
            level: self.level.is_some(),
            recording: self.recording.is_some(),
            music: self.song.is_some(),
            waiting: self.waiting > 0,
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
}

#[derive(Default)]
pub struct Island {
    open: bool,
    size: Option<Vec2>,
    velocity: Vec2,
    mode: Option<Mode>,
}

impl Island {
    /// Shuts the card, if it is open.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Draws the island centred on `room` (the part of the strip between the left
    /// and right islands), its bottom on the strip's line, growing upward. Returns
    /// where it is (for the compositor to lift, when it reaches above the strip)
    /// and what was done.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, room: Rect, feed: &Feed, animate: bool) -> (Rect, Deed) {
        let facts = feed.facts();
        // Nothing left to open: the card closes by itself.
        if self.open && !(facts.music || facts.arriving || facts.waiting) {
            self.open = false;
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
                let size = vec2(w, h);
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
                let radius = (rect.height() / 2.0).min(CARD_RADIUS);
                ui.painter().rect_filled(rect.translate(vec2(0.0, 6.0)).expand(2.0), radius, Color32::from_black_alpha(70));
                ui.painter().rect(rect, radius, Color32::from_rgba_unmultiplied(10, 14, 18, 247), Stroke::new(1.0_f32, ui::LINE), StrokeKind::Inside);
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                inner.set_clip_rect(rect);
                inner.set_opacity(settled);
                let done = draw(&mut inner, theme, mode, rect, feed);
                if done != Deed::Nothing {
                    deed = done;
                } else if response.clicked() {
                    match mode {
                        Mode::Music | Mode::Arriving | Mode::Waiting | Mode::MusicOpen | Mode::NoteOpen => self.open = !self.open,
                        Mode::Recording => deed = Deed::StopRecording,
                        Mode::Resting | Mode::Level => {}
                    }
                } else if mode == Mode::Arriving && response.hovered() {
                    if let Some((notice, _)) = &feed.arriving {
                        deed = Deed::Holding(notice.id);
                    }
                }
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label(mode, feed)));
            });
        // Away from it, or Escape, shuts the card.
        if self.open {
            let pressed_away = ctx.input(|input| input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|at| !rect.contains(at)));
            if pressed_away || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.open = false;
                ctx.request_repaint();
            }
        }
        if bubble(facts) {
            let at = pos2(rect.right() + 8.0, room.bottom() - REST_HEIGHT);
            if at.x + REST_HEIGHT <= room.right() {
                let dot = Rect::from_min_size(at, Vec2::splat(REST_HEIGHT));
                let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("status-island-bubble")));
                painter.rect(dot, REST_HEIGHT / 2.0, Color32::from_rgba_unmultiplied(10, 14, 18, 247), Stroke::new(1.0_f32, ui::LINE), StrokeKind::Inside);
                painter.circle_filled(dot.center(), 4.0, RED);
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
            Mode::Recording => 18.0 + 8.0 + 10.0 + width("REC", ui::mono(12.0)) + 8.0 + width("00:00", ui::mono(13.0)) + 10.0 + 30.0 + 7.0,
            Mode::Level => 18.0 + 20.0 + 12.0 + 180.0 + 12.0 + 34.0 + 18.0,
            Mode::Arriving => 360.0,
            Mode::MusicOpen | Mode::NoteOpen => CARD_WIDTH,
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
        Mode::Recording => "Recording the screen. Stop".into(),
        Mode::Level => "Level".into(),
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
                let lift = if song.playing { 0.5 + 0.5 * (time * 7.0 + phase).sin() } else { 0.25 };
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
            let body = notify::plain(&notice.body).replace('\n', " ");
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
            let body = ui.painter().layout(notify::plain(&notice.body), ui::display(13.0), ui::TEXT, rect.width() - 36.0);
            let clip = Rect::from_min_max(pos2(rect.left() + 18.0, rect.top() + 58.0), pos2(rect.right() - 18.0, rect.bottom() - 48.0));
            ui.painter().with_clip_rect(clip).galley(clip.min, body, ui::TEXT);
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
            let stop = pos2(rect.right() - 22.0, middle);
            let (clicked, r) = round_button(ui, stop, 30.0, Color32::from_rgb(0x2A, 0x14, 0x14), "Stop recording");
            ui.painter().rect_filled(Rect::from_center_size(r.center(), Vec2::splat(10.0)), 2.0, RED);
            if clicked { deed = Deed::StopRecording; }
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
        Facts { arriving, level, recording, music, waiting }
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
        assert_eq!(mode(facts(false, false, true, false, false), true), Mode::Recording);
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
    fn the_spring_arrives_without_overshooting() {
        let (mut x, mut v) = (214.0_f32, 0.0_f32);
        let mut peak = x;
        for _ in 0..120 {
            (x, v) = spring(x, v, 400.0, 1.0 / 60.0);
            peak = peak.max(x);
        }
        assert!((x - 400.0).abs() < 0.5, "{x}");
        assert!(peak <= 400.5, "overshot to {peak}");
    }

    #[test]
    fn initials_come_from_the_title_or_the_app() {
        let mut notice = notify::Notice::sample("WhatsApp", "Ama Kwarteng", "See you at 6?");
        assert_eq!(initials(&notice), "AK");
        notice.title = String::new();
        assert_eq!(initials(&notice), "W");
    }
}
