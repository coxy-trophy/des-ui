//! A small meter when the volume or the brightness changes from outside the shell's
//! own sliders: the media keys, `wpctl` in a terminal, an application. It hangs from
//! the top right corner of the workspace, where the toasts begin, so the compositor
//! lifts it above applications with the toasts' rectangle and nothing more.

use crate::powerup::ease;
use crate::theme::Theme;
use crate::ui::{self, Icon};
use eframe::egui::{self, Order, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use std::time::{Duration, Instant};

/// How long the meter stays after the last change.
const SHOWN: Duration = Duration::from_millis(1500);
/// The last part of that, over which it fades.
const FADE: Duration = Duration::from_millis(300);
pub const WIDTH: f32 = 260.0;
pub const HEIGHT: f32 = 44.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Volume,
    Brightness,
}

#[derive(Default)]
pub struct Osd {
    /// The last reading of each, volume then brightness.
    last: [Option<u8>; 2],
    /// Until when a change counts as the user's own, set by the shell's sliders.
    held: [Option<Instant>; 2],
    shown: Option<(Kind, u8, Instant)>,
}

impl Osd {
    /// The shell itself is about to change this level; do not announce it.
    pub fn hold(&mut self, kind: Kind, now: Instant) {
        self.held[kind as usize] = Some(now + SHOWN);
    }

    /// Takes a fresh reading. A change shows the meter unless it is the shell's own,
    /// or `quiet` (the settings or quick settings, where the level is on screen
    /// already); the first reading only sets where things start.
    pub fn observe(&mut self, kind: Kind, reading: Option<u8>, quiet: bool, now: Instant) {
        let Some(level) = reading else { return };
        let before = self.last[kind as usize].replace(level);
        let own = self.held[kind as usize].is_some_and(|until| now < until);
        if before.is_some_and(|before| before != level) && !quiet && !own {
            self.shown = Some((kind, level, now));
        }
    }

    /// What to draw now, and how opaque: `None` once it has gone.
    pub fn visible(&self, now: Instant, animate: bool) -> Option<(Kind, u8, f32)> {
        let (kind, level, at) = self.shown?;
        let age = now.checked_duration_since(at).unwrap_or_default();
        if age >= SHOWN {
            return None;
        }
        let left = SHOWN - age;
        let alpha = if animate && left < FADE { ease(left.as_secs_f32() / FADE.as_secs_f32()) } else { 1.0 };
        Some((kind, level, alpha))
    }

    /// Draws the meter with its top right at `corner`; returns where it is.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, corner: Pos2, animate: bool) -> Option<Rect> {
        let now = Instant::now();
        let Some((kind, level, alpha)) = self.visible(now, animate) else {
            self.shown = None;
            return None;
        };
        let rect = Rect::from_min_size(pos2(corner.x - WIDTH, corner.y), vec2(WIDTH, HEIGHT));
        egui::Area::new(egui::Id::new("osd")).order(Order::Tooltip).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
            ui.set_opacity(alpha);
            let painter = ui.painter();
            crate::material::paint(ctx, painter, rect, ui::SURFACE_RADIUS, crate::material::Role::Floating);
            painter.rect_stroke(rect, ui::SURFACE_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
            let icon = match kind {
                Kind::Volume if level == 0 => Icon::Muted,
                Kind::Volume => Icon::Volume,
                Kind::Brightness => Icon::Brightness,
            };
            let icon_rect = Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(20.0, 20.0));
            ui::paint_icon(painter, icon_rect, icon, theme.main, true);
            let track = Rect::from_min_max(pos2(rect.left() + 46.0, rect.center().y - 2.0), pos2(rect.right() - 58.0, rect.center().y + 2.0));
            painter.rect_filled(track, 0.0, ui::TRACK);
            let filled = track.width() * (level.min(100) as f32 / 100.0);
            painter.rect_filled(Rect::from_min_size(track.min, vec2(filled, track.height())), 0.0, theme.main);
            painter.text(pos2(rect.right() - 14.0, rect.center().y), egui::Align2::RIGHT_CENTER, format!("{level}%"), ui::mono(13.0), ui::TEXT);
        });
        ctx.request_repaint_after(Duration::from_millis(33));
        Some(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_from_elsewhere_shows_and_the_first_reading_does_not() {
        let now = Instant::now();
        let mut osd = Osd::default();
        osd.observe(Kind::Volume, Some(40), false, now);
        assert!(osd.visible(now, true).is_none(), "the first reading is where things start");
        osd.observe(Kind::Volume, Some(45), false, now);
        assert_eq!(osd.visible(now, true).map(|(kind, level, _)| (kind, level)), Some((Kind::Volume, 45)));
    }

    #[test]
    fn the_shell_s_own_slider_and_quiet_screens_do_not_show_it() {
        let now = Instant::now();
        let mut osd = Osd::default();
        osd.observe(Kind::Brightness, Some(50), false, now);
        osd.hold(Kind::Brightness, now);
        osd.observe(Kind::Brightness, Some(60), false, now + Duration::from_millis(200));
        assert!(osd.visible(now, true).is_none());
        osd.observe(Kind::Volume, Some(10), false, now);
        osd.observe(Kind::Volume, Some(20), true, now);
        assert!(osd.visible(now, true).is_none());
        // Once the hold has passed, a change from elsewhere shows again.
        osd.observe(Kind::Brightness, Some(70), false, now + SHOWN);
        assert!(osd.visible(now + SHOWN, true).is_some());
    }

    #[test]
    fn repeated_changes_keep_it_up_and_it_fades_at_the_end() {
        let now = Instant::now();
        let mut osd = Osd::default();
        osd.observe(Kind::Volume, Some(10), false, now);
        osd.observe(Kind::Volume, Some(20), false, now);
        let later = now + Duration::from_millis(1400);
        osd.observe(Kind::Volume, Some(30), false, later);
        assert_eq!(osd.visible(now + Duration::from_millis(2000), true).map(|(_, level, alpha)| (level, alpha)), Some((30, 1.0)));
        let fading = osd.visible(later + SHOWN - Duration::from_millis(100), true).unwrap().2;
        assert!(fading > 0.0 && fading < 1.0);
        assert_eq!(osd.visible(later + SHOWN - Duration::from_millis(100), false).unwrap().2, 1.0, "no fade without animation");
        assert!(osd.visible(later + SHOWN, true).is_none());
    }
}
