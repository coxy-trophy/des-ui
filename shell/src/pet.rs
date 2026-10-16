//! A companion: a small animal that lives at the foot of the left column, where there
//! is room for one. It sits, lies down, walks and runs from one side to the other,
//! and paws at the pointer when that comes near.
//!
//! The animals and how they behave come from VS Code Pets; who drew each is in
//! `assets/pets/CREDITS.md`. Their pictures are used as they are.

use crate::ui;
use eframe::egui::{self, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::time::Instant;

/// How high the strip the companion walks in is.
const STRIP: f32 = 100.0;
/// How long it sits or lies before moving on, and how long it paws, in seconds.
const REST: (f32, f32) = (4.0, 9.0);
const PAWING: f32 = 1.9;

macro_rules! picture {
    ($folder:literal, $name:literal) => {
        include_bytes!(concat!("../assets/pets/", $folder, "/", $name, "_8fps.gif")) as &[u8]
    };
}

/// One kind of animal: what the settings call it, and its animations.
pub struct Kind {
    pub name: &'static str,
    sit: &'static [u8],
    /// Not every animal lies down.
    lie: Option<&'static [u8]>,
    walk: &'static [u8],
    run: &'static [u8],
    paw: &'static [u8],
    /// How far it walks in a second; running is two and a half times that.
    pace: f32,
}

pub const KINDS: [Kind; 6] = [
    Kind {
        name: "dog",
        sit: picture!("dog", "brown_idle"),
        lie: Some(picture!("dog", "brown_lie")),
        walk: picture!("dog", "brown_walk"),
        run: picture!("dog", "brown_run"),
        paw: picture!("dog", "brown_swipe"),
        pace: 24.0,
    },
    Kind {
        name: "fox",
        sit: picture!("fox", "red_idle"),
        lie: Some(picture!("fox", "red_lie")),
        walk: picture!("fox", "red_walk"),
        run: picture!("fox", "red_run"),
        paw: picture!("fox", "red_swipe"),
        pace: 24.0,
    },
    Kind {
        name: "crab",
        sit: picture!("crab", "red_idle"),
        lie: None,
        walk: picture!("crab", "red_walk"),
        run: picture!("crab", "red_run"),
        paw: picture!("crab", "red_swipe"),
        pace: 16.0,
    },
    Kind {
        name: "duck",
        sit: picture!("rubber-duck", "yellow_idle"),
        lie: None,
        walk: picture!("rubber-duck", "yellow_walk"),
        run: picture!("rubber-duck", "yellow_run"),
        paw: picture!("rubber-duck", "yellow_swipe"),
        pace: 20.0,
    },
    Kind {
        name: "snake",
        sit: picture!("snake", "green_idle"),
        lie: None,
        walk: picture!("snake", "green_walk"),
        run: picture!("snake", "green_run"),
        paw: picture!("snake", "green_swipe"),
        pace: 14.0,
    },
    Kind {
        name: "turtle",
        sit: picture!("turtle", "green_idle"),
        lie: Some(picture!("turtle", "green_lie")),
        walk: picture!("turtle", "green_walk"),
        run: picture!("turtle", "green_run"),
        paw: picture!("turtle", "green_swipe"),
        pace: 6.0,
    },
];

/// The kind the settings name; `None` for "off" and for names not known.
pub fn kind_named(name: &str) -> Option<usize> {
    KINDS.iter().position(|kind| kind.name == name)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Doing {
    Sit,
    Lie,
    Walk,
    Run,
    Paw,
}

/// Where the companion is and what it is doing, apart from how it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Walker {
    /// How far its left edge is from the left of the strip.
    pub x: f32,
    /// Which way it faces.
    pub right: bool,
    pub doing: Doing,
    /// How much longer it rests or paws.
    left: f32,
    /// What it was doing before it started pawing.
    before: Doing,
    seed: u32,
}

impl Walker {
    pub fn new(seed: u32) -> Walker {
        Walker { x: 0.0, right: true, doing: Doing::Sit, left: REST.0, before: Doing::Sit, seed: seed | 1 }
    }

    /// The next of an endless run of numbers that look like chance.
    fn chance(&mut self, of: u32) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed % of
    }

    fn rest(&mut self, doing: Doing) {
        self.doing = doing;
        self.left = REST.0 + (REST.1 - REST.0) * self.chance(100) as f32 / 100.0;
    }

    /// Paws at something, then goes back to what it was doing.
    pub fn paw(&mut self) {
        if self.doing != Doing::Paw {
            self.before = self.doing;
            self.doing = Doing::Paw;
            self.left = PAWING;
        }
    }

    /// Moves it on by `dt` seconds, with `room` to walk in.
    pub fn step(&mut self, dt: f32, room: f32, pace: f32, lies: bool) {
        self.x = self.x.clamp(0.0, room.max(0.0));
        match self.doing {
            Doing::Paw => {
                self.left -= dt;
                if self.left <= 0.0 {
                    self.doing = self.before;
                }
            }
            Doing::Sit | Doing::Lie => {
                self.left -= dt;
                // With nowhere to go it stays where it is.
                if self.left <= 0.0 && room > 1.0 {
                    let rested = self.doing;
                    self.doing = match self.chance(if rested == Doing::Sit && lies { 3 } else { 2 }) {
                        0 => Doing::Walk,
                        1 => Doing::Run,
                        _ => Doing::Lie,
                    };
                    if self.doing == Doing::Lie {
                        self.rest(Doing::Lie);
                    } else {
                        // Off towards the side with more room.
                        self.right = self.x < room / 2.0;
                    }
                }
            }
            Doing::Walk | Doing::Run => {
                let speed = if self.doing == Doing::Run { pace * 2.5 } else { pace };
                self.x += if self.right { speed } else { -speed } * dt;
                if self.right && self.x >= room {
                    // At the right edge it turns round, walking or running.
                    self.x = room.max(0.0);
                    self.right = false;
                    self.doing = if self.chance(2) == 0 { Doing::Walk } else { Doing::Run };
                } else if !self.right && self.x <= 0.0 {
                    // Back at the left it may rest, or set off again.
                    self.x = 0.0;
                    self.right = true;
                    match self.chance(if lies { 4 } else { 3 }) {
                        0 => self.rest(Doing::Sit),
                        1 => self.doing = Doing::Walk,
                        2 => self.doing = Doing::Run,
                        _ => self.rest(Doing::Lie),
                    }
                }
            }
        }
    }
}

/// An animation's pictures, each with how long it shows, in seconds.
pub fn frames_of(gif: &[u8]) -> Vec<(egui::ColorImage, f32)> {
    use image::AnimationDecoder;
    let Ok(decoder) = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif)) else {
        return Vec::new();
    };
    decoder
        .into_frames()
        .map_while(Result::ok)
        .map(|frame| {
            let (ms, per) = frame.delay().numer_denom_ms();
            let seconds = (ms as f32 / per.max(1) as f32 / 1000.0).clamp(0.05, 0.4);
            let pixels = frame.into_buffer();
            (egui::ColorImage::from_rgba_unmultiplied([pixels.width() as usize, pixels.height() as usize], pixels.as_raw()), seconds)
        })
        .collect()
}

/// Which picture of an animation shows `at` seconds into it, going round.
fn frame_at(lengths: &[f32], at: f32) -> usize {
    let whole: f32 = lengths.iter().sum();
    if whole <= 0.0 {
        return 0;
    }
    let mut at = at % whole;
    for (index, length) in lengths.iter().enumerate() {
        if at < *length {
            return index;
        }
        at -= length;
    }
    lengths.len() - 1
}

struct Animation {
    doing: Doing,
    frames: Vec<(egui::TextureHandle, f32)>,
}

/// The companion as the left column draws it.
pub struct Pet {
    kind: Option<usize>,
    /// The chosen animal's animations, read when it is first drawn.
    animations: Vec<Animation>,
    walker: Walker,
    /// How long it has been doing what it is doing.
    since: f32,
    drawn: Option<Instant>,
}

impl Pet {
    pub fn new(name: &str) -> Pet {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |now| now.subsec_nanos());
        Pet { kind: kind_named(name), animations: Vec::new(), walker: Walker::new(seed), since: 0.0, drawn: None }
    }

    /// Changes the animal, or sends it away.
    pub fn choose(&mut self, name: &str) {
        let kind = kind_named(name);
        if kind != self.kind {
            self.kind = kind;
            self.animations.clear();
        }
    }

    fn read(&mut self, ctx: &egui::Context, kind: &Kind) {
        let all = [(Doing::Sit, Some(kind.sit)), (Doing::Lie, kind.lie), (Doing::Walk, Some(kind.walk)), (Doing::Run, Some(kind.run)), (Doing::Paw, Some(kind.paw))];
        for (doing, gif) in all {
            let Some(gif) = gif else { continue };
            let frames = frames_of(gif)
                .into_iter()
                .enumerate()
                // Drawn dot for dot, as it was made.
                .map(|(index, (pixels, seconds))| (ctx.load_texture(format!("pet-{}-{doing:?}-{index}", kind.name), pixels, egui::TextureOptions::NEAREST), seconds))
                .collect();
            self.animations.push(Animation { doing, frames });
        }
    }

    /// Draws the companion at the foot of the column, if there is room left under
    /// what is already there.
    pub fn show(&mut self, ui: &mut Ui) {
        let Some(kind) = self.kind.and_then(|kind| KINDS.get(kind)) else {
            return;
        };
        let column = ui.max_rect();
        if column.bottom() - ui.cursor().top() < STRIP + 16.0 {
            self.drawn = None;
            return;
        }
        if self.animations.is_empty() {
            self.read(ui.ctx(), kind);
        }
        let strip = Rect::from_min_max(pos2(column.left(), column.bottom() - STRIP), column.right_bottom());
        // Every animation is drawn in a box as wide as the widest, so that changing
        // from one to another does not move the animal.
        let widest = self.animations.iter().flat_map(|animation| &animation.frames).map(|(texture, _)| texture.size_vec2().x).fold(0.0, f32::max);
        let room = strip.width() - widest;

        // A long gap, as after the column was closed, is not caught up on.
        let dt = self.drawn.map_or(0.0, |at| at.elapsed().as_secs_f32()).min(0.5);
        self.drawn = Some(Instant::now());
        let was = self.walker.doing;
        self.walker.step(dt, room, kind.pace, kind.lie.is_some());
        self.since = if self.walker.doing == was { self.since + dt } else { 0.0 };

        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, ui::TRACK));
        let Some(animation) = self.animations.iter().find(|animation| animation.doing == self.walker.doing && !animation.frames.is_empty()) else {
            return;
        };
        let lengths: Vec<f32> = animation.frames.iter().map(|(_, seconds)| *seconds).collect();
        let (texture, _) = &animation.frames[frame_at(&lengths, self.since)];
        let size = texture.size_vec2();
        let left = (strip.left() + self.walker.x + (widest - size.x) / 2.0).round();
        let at = Rect::from_min_size(pos2(left, strip.bottom() - 1.0 - size.y), size);
        // The pictures face right; going left, they are turned over.
        let across = if self.walker.right { (0.0, 1.0) } else { (1.0, 0.0) };
        let whole = Rect::from_min_max(pos2(across.0, 0.0), pos2(across.1, 1.0));
        ui.painter().image(texture.id(), at, whole, egui::Color32::WHITE);

        if ui.interact(at.shrink2(vec2(size.x * 0.2, 0.0)), ui.id().with("pet"), Sense::hover()).hovered() {
            self.walker.paw();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_animation_can_be_read() {
        for kind in &KINDS {
            for gif in [Some(kind.sit), kind.lie, Some(kind.walk), Some(kind.run), Some(kind.paw)].into_iter().flatten() {
                let frames = frames_of(gif);
                assert!(frames.len() >= 2, "{} has an animation that does not move", kind.name);
                assert!(frames.iter().all(|(pixels, seconds)| pixels.size[0] > 0 && *seconds >= 0.05 && *seconds <= 0.4));
            }
        }
        assert_eq!(kind_named("fox"), Some(1));
        assert_eq!(kind_named("off"), None);
        assert_eq!(kind_named(""), None);
        assert!(frames_of(b"not a picture").is_empty());
    }

    #[test]
    fn goes_round_an_animation_by_each_picture_s_length() {
        let lengths = [0.1, 0.3, 0.1];
        assert_eq!(frame_at(&lengths, 0.0), 0);
        assert_eq!(frame_at(&lengths, 0.15), 1);
        assert_eq!(frame_at(&lengths, 0.45), 2);
        assert_eq!(frame_at(&lengths, 0.55), 0, "round again");
        assert_eq!(frame_at(&[], 3.0), 0);
    }

    #[test]
    fn walks_to_each_side_and_never_leaves_the_strip() {
        let mut walker = Walker::new(12345);
        let (mut sides, mut rested) = ([false, false], false);
        let mut was = walker.doing;
        for _ in 0..20_000 {
            walker.step(0.1, 150.0, 24.0, true);
            assert!((0.0..=150.0).contains(&walker.x), "left the strip at {}", walker.x);
            sides[0] |= walker.x == 0.0 && was != Doing::Sit;
            sides[1] |= walker.x == 150.0;
            rested |= walker.doing == Doing::Lie;
            was = walker.doing;
        }
        assert!(sides[0] && sides[1] && rested);
    }

    #[test]
    fn stays_put_without_room_and_only_lies_if_it_can() {
        let mut walker = Walker::new(99);
        for _ in 0..2_000 {
            walker.step(0.1, -20.0, 24.0, true);
            assert_eq!((walker.x, walker.doing), (0.0, Doing::Sit));
        }
        let mut walker = Walker::new(99);
        for _ in 0..20_000 {
            walker.step(0.1, 150.0, 24.0, false);
            assert_ne!(walker.doing, Doing::Lie);
        }
    }

    #[test]
    fn paws_for_a_moment_and_goes_back_to_what_it_was_doing() {
        let mut walker = Walker::new(5);
        walker.doing = Doing::Walk;
        walker.x = 40.0;
        walker.paw();
        walker.paw();
        assert_eq!(walker.doing, Doing::Paw);
        walker.step(1.0, 150.0, 24.0, true);
        assert_eq!((walker.doing, walker.x), (Doing::Paw, 40.0), "it stands still to paw");
        walker.step(1.0, 150.0, 24.0, true);
        assert_eq!(walker.doing, Doing::Walk);
    }
}
