//! A terminal that drops from the top of the workspace and goes again, over whatever
//! is in view. Above applications it is seen through the compositor's overlay, which
//! also gives the shell the keyboard while it is down.

use crate::term::{TermLook, TermTab};
use crate::theme::Theme;
use eframe::egui::{self, Event, EventFilter, Order, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::path::Path;
use std::time::{Duration, Instant};

/// Space left on either side of the workspace.
const MARGIN: f32 = 48.0;
/// Share of the screen's height it takes when down.
const HEIGHT: f32 = 0.45;
/// The one easing and the one duration the desktop's motion uses.
const SLIDE: Duration = Duration::from_millis(180);
/// Inside the border, around the grid.
const PADDING: f32 = 10.0;

/// Where the scratchpad sits when fully down: under the top of the workspace, its full
/// width less a margin each side, and 45 % of the screen tall, kept inside the screen.
pub fn drop_rect(workspace: Rect, screen: Rect) -> Rect {
    // A narrow workspace gives up its margins before the terminal gives up width.
    let margin = MARGIN.min((workspace.width() * 0.05).max(0.0));
    let width = (workspace.width() - 2.0 * margin).max(0.0);
    let room = (screen.bottom() - workspace.top()).max(0.0);
    let height = (screen.height() * HEIGHT).max(120.0).min(room);
    Rect::from_min_size(pos2(workspace.left() + margin, workspace.top()), vec2(width, height))
}

/// How far down it is, 0 up to 1 down, after `elapsed` more of moving toward `open`.
pub fn step(progress: f32, open: bool, elapsed: Duration, animate: bool) -> f32 {
    let target = if open { 1.0 } else { 0.0 };
    if !animate {
        return target;
    }
    let moved = elapsed.as_secs_f32() / SLIDE.as_secs_f32();
    if open { (progress + moved).min(1.0) } else { (progress - moved).max(0.0) }
}

/// The part of `full` in view at `progress`, growing down from its top edge.
pub fn shown(full: Rect, progress: f32) -> Rect {
    let height = full.height() * crate::powerup::ease(progress.clamp(0.0, 1.0));
    Rect::from_min_size(full.min, vec2(full.width(), height))
}

/// A rectangle as the compositor takes it, in whole logical pixels.
pub fn area(rect: Rect) -> (i32, i32, i32, i32) {
    (rect.left().floor() as i32, rect.top().floor() as i32, rect.width().ceil() as i32 + 1, rect.height().ceil() as i32 + 1)
}

/// Whether `now` differs from what was last sent, recording it if so. `None` in the
/// cache means nothing is known to have been sent, so anything counts as a change.
pub fn changed<T: PartialEq + Copy>(sent: &mut Option<T>, now: T) -> bool {
    if *sent == Some(now) {
        return false;
    }
    *sent = Some(now);
    true
}

pub struct Scratchpad {
    pub open: bool,
    progress: f32,
    last: Option<Instant>,
    tab: Option<TermTab>,
    completions_enabled: bool,
}

impl Default for Scratchpad {
    fn default() -> Self {
        Scratchpad { open: false, progress: 0.0, last: None, tab: None, completions_enabled: true }
    }
}

impl Scratchpad {
    pub fn set_completions(&mut self, enabled: bool) {
        self.completions_enabled = enabled;
        if let Some(tab) = &mut self.tab { tab.set_completions(enabled); }
    }
    /// Reconnect a hidden scratchpad; showing it remains an explicit user action.
    pub fn restore_tab(&mut self, tab: TermTab) { self.tab = Some(tab); self.close_now(); }
    pub fn persistent_id(&self) -> Option<&str> { self.tab.as_ref().and_then(|tab| tab.persistent_id.as_deref()) }

    pub fn take_links(&mut self) -> Vec<String> {
        self.tab.as_mut().map_or_else(Vec::new, TermTab::take_links)
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Up at once, as when the screen locks.
    pub fn close_now(&mut self) {
        self.open = false;
        self.progress = 0.0;
        self.last = None;
    }

    /// Whether it is down or on its way, and so takes the keyboard.
    pub fn visible(&self) -> bool {
        self.open || self.progress > 0.0
    }

    /// Feeds its shell's output; call each frame like the tabs' pump.
    pub fn pump(&mut self) {
        if let Some(tab) = &mut self.tab {
            tab.pump();
        }
    }

    /// Draws it over everything and gives it `events`, the terminal's share of this
    /// frame's input. Returns the part in view, for the compositor's overlay.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: Rect,
        screen: Rect,
        theme: &Theme,
        look: &TermLook,
        animate: bool,
        spawn: (&str, &Path, usize, bool),
        events: &[Event],
    ) -> Option<Rect> {
        let now = Instant::now();
        let elapsed = self.last.map_or(Duration::ZERO, |last| now - last);
        self.last = Some(now);
        // The shell inside ended: put it away; the next showing starts another.
        if self.tab.as_ref().is_some_and(|tab| tab.exited) {
            self.tab = None;
            self.open = false;
        }
        self.progress = step(self.progress, self.open, elapsed, animate);
        if !self.visible() {
            self.last = None;
            return None;
        }
        if self.progress > 0.0 && self.progress < 1.0 {
            ctx.request_repaint();
        }
        if self.tab.is_none() {
            let (shell, home, scrollback, keep) = spawn;
            match TermTab::spawn_managed(shell, home, ctx.clone(), scrollback, keep, crate::term::SessionRole::Scratchpad) {
                Ok(mut tab) => { tab.set_completions(self.completions_enabled); self.tab = Some(tab); },
                Err(error) => {
                    eprintln!("des-ui: no scratchpad terminal: {error}");
                    self.close_now();
                    return None;
                }
            }
        }
        let full = drop_rect(workspace, screen);
        let visible = shown(full, self.progress);
        let tab = self.tab.as_mut()?;
        egui::Area::new(egui::Id::new("scratchpad"))
            .order(Order::Foreground)
            .fixed_pos(full.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_clip_rect(visible);
                let painter = ui.painter();
                crate::material::paint(ctx, painter, full, 0.0, crate::material::Role::Terminal);
                painter.rect_stroke(full, 0.0, Stroke::new(1.0, theme.main), StrokeKind::Inside);
                let inner = full.shrink(PADDING);
                let response = ui.interact(inner, ui.id().with("scratchpad-grid"), Sense::click_and_drag());
                if self.open && !tab.searching() {
                    ui.memory_mut(|memory| {
                        memory.request_focus(response.id);
                        memory.set_focus_lock_filter(
                            response.id,
                            EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                        );
                    });
                }
                if self.open {
                    let (alt, shift) = ui.input(|input| (input.modifiers.alt, input.modifiers.shift));
                    tab.handle_events(events, alt, shift, ctx);
                }
                tab.show(ui, inner, theme, &response, look, self.open);
            });
        Some(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    #[test]
    fn drops_under_the_workspace_top_with_margins() {
        let screen = rect(0.0, 0.0, 1920.0, 1080.0);
        let workspace = rect(360.0, 44.0, 1200.0, 700.0);
        let full = drop_rect(workspace, screen);
        assert_eq!(full.left(), 408.0);
        assert_eq!(full.width(), 1104.0);
        assert_eq!(full.top(), 44.0);
        assert_eq!(full.height(), 1080.0 * 0.45);
    }

    #[test]
    fn a_tiny_screen_keeps_it_inside() {
        let screen = rect(0.0, 0.0, 300.0, 200.0);
        let workspace = rect(0.0, 44.0, 300.0, 120.0);
        let full = drop_rect(workspace, screen);
        assert!(full.left() >= workspace.left() && full.right() <= workspace.right());
        assert!(full.bottom() <= screen.bottom());
        assert!(full.width() > 250.0, "margins shrink first: {}", full.width());
        let none = drop_rect(rect(0.0, 0.0, 0.0, 0.0), rect(0.0, 0.0, 0.0, 0.0));
        assert_eq!(none.size(), vec2(0.0, 0.0));
    }

    #[test]
    fn slides_over_180_ms_or_at_once() {
        let half = Duration::from_millis(90);
        let p = step(0.0, true, half, true);
        assert!((p - 0.5).abs() < 1e-3);
        assert_eq!(step(p, true, half * 2, true), 1.0);
        assert_eq!(step(1.0, false, Duration::from_millis(180), true), 0.0);
        assert_eq!(step(0.0, true, Duration::ZERO, false), 1.0);
        assert_eq!(step(1.0, false, Duration::ZERO, false), 0.0);
        let full = rect(0.0, 10.0, 100.0, 200.0);
        assert_eq!(shown(full, 0.0).height(), 0.0);
        assert_eq!(shown(full, 1.0), full);
        assert!(shown(full, 0.5).height() > 100.0, "eased: quicker at the start");
    }

    #[test]
    fn overlay_is_sent_only_on_change() {
        let mut sent = None;
        assert!(changed(&mut sent, None::<(i32, i32, i32, i32)>), "unknown counts as a change");
        assert!(!changed(&mut sent, None));
        assert!(changed(&mut sent, Some((1, 2, 3, 4))));
        assert!(!changed(&mut sent, Some((1, 2, 3, 4))));
        assert!(changed(&mut sent, None));
        assert_eq!(area(rect(10.4, 20.6, 99.2, 50.0)), (10, 20, 101, 51));
    }
}
