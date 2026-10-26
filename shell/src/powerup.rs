//! The power-up: how the frame comes on at the start of a session. The compositor's
//! splash (`des_common::splash`) fades once the shell has its window; under that fade
//! the frame's lines sweep out from the terminal, then the columns, the file panel,
//! the terminal's text and the status strip come in one after another, and the
//! instruments sweep up from zero to their readings. Seven hundred milliseconds,
//! under the one easing everything else uses, and then nothing more: once it is done
//! every reading here costs nothing.
//!
//! The panels do not take it as an argument. Each frame the sequence's stage is put
//! in the egui context ([`PowerUp::publish`]) and read back where it is applied
//! ([`stage`]); a context without one is a finished sequence, so tests and the
//! secondary displays draw as they always did.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use des_common::Panels;
use eframe::egui::{self, Id, Painter, Rect, Stroke, Ui};

/// The whole sequence, from when it starts.
pub const TOTAL: Duration = Duration::from_millis(700);
/// How far a part slides as it comes in, in points.
const SLIDE: f32 = 10.0;

/// What comes on, in the order it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The one-pixel lines between the frame's regions.
    Lines,
    Left,
    Right,
    /// The file panel under the terminal.
    Bottom,
    /// The terminal's text.
    Terminal,
    /// The status strip and the dock.
    Status,
    /// Bars and graphs, which sweep from zero to their readings.
    Instruments,
}

impl Part {
    /// When the part starts and when it is fully there, in milliseconds from the
    /// start. The columns follow the lines eighty milliseconds apart.
    pub fn span(self) -> (u64, u64) {
        match self {
            Part::Lines => (0, 200),
            Part::Left => (200, 440),
            Part::Right => (280, 520),
            Part::Bottom => (360, 600),
            Part::Terminal => (400, 600),
            Part::Status => (480, 700),
            Part::Instruments => (400, 700),
        }
    }
}

/// Cubic out, as the compositor's motion and the dock use.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// How far on a part is, eased, `since` the sequence started.
pub fn progress_for(part: Part, since: Duration) -> f32 {
    let (from, to) = part.span();
    let at = since.as_millis() as f32;
    ease((at - from as f32) / (to - from) as f32)
}

/// Where the sequence stands in one frame: a number for each part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stage {
    lines: f32,
    left: f32,
    right: f32,
    bottom: f32,
    terminal: f32,
    status: f32,
    instruments: f32,
}

impl Stage {
    /// Everything on, as after the sequence, or where there never was one.
    pub fn finished() -> Stage {
        Stage { lines: 1.0, left: 1.0, right: 1.0, bottom: 1.0, terminal: 1.0, status: 1.0, instruments: 1.0 }
    }

    pub fn at(since: Duration) -> Stage {
        Stage {
            lines: progress_for(Part::Lines, since),
            left: progress_for(Part::Left, since),
            right: progress_for(Part::Right, since),
            bottom: progress_for(Part::Bottom, since),
            terminal: progress_for(Part::Terminal, since),
            status: progress_for(Part::Status, since),
            instruments: progress_for(Part::Instruments, since),
        }
    }

    pub fn progress(&self, part: Part) -> f32 {
        match part {
            Part::Lines => self.lines,
            Part::Left => self.left,
            Part::Right => self.right,
            Part::Bottom => self.bottom,
            Part::Terminal => self.terminal,
            Part::Status => self.status,
            Part::Instruments => self.instruments,
        }
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        *self == Stage::finished()
    }

    /// How much of a part shows.
    pub fn alpha(&self, part: Part) -> f32 {
        self.progress(part)
    }

    /// How far a part still is from its place, in points, as it slides in.
    pub fn offset(&self, part: Part) -> f32 {
        SLIDE * (1.0 - self.progress(part))
    }

    /// Fades a panel's `ui` and moves its content down by what is left of its slide.
    /// Nothing is touched once the part is there, so a finished sequence costs nothing.
    pub fn apply(&self, ui: &mut Ui, part: Part) {
        let progress = self.progress(part);
        if progress < 1.0 {
            ui.set_opacity(progress);
            ui.add_space(self.offset(part));
        }
    }

    /// A reading swept: the value itself once the sweep is over.
    pub fn swept(&self, value: f32) -> f32 {
        value * self.instruments
    }

    /// A history swept, copied only while the sweep is on.
    pub fn swept_values<'a>(&self, values: &'a VecDeque<f32>) -> Cow<'a, VecDeque<f32>> {
        if self.instruments >= 1.0 {
            Cow::Borrowed(values)
        } else {
            Cow::Owned(values.iter().map(|value| value * self.instruments).collect())
        }
    }

    /// A row of readings swept, likewise.
    pub fn swept_slice<'a>(&self, values: &'a [f32]) -> Cow<'a, [f32]> {
        if self.instruments >= 1.0 {
            Cow::Borrowed(values)
        } else {
            Cow::Owned(values.iter().map(|value| value * self.instruments).collect())
        }
    }
}

impl Default for Stage {
    fn default() -> Stage {
        Stage::finished()
    }
}

fn slot() -> Id {
    Id::new("des-ui-power-up")
}

/// The stage published for this frame; finished where none was.
pub fn stage(ctx: &egui::Context) -> Stage {
    ctx.data(|data| data.get_temp(slot())).unwrap_or_else(Stage::finished)
}

/// The sequence itself: when it began, and whether it is over.
pub struct PowerUp {
    began: Option<Instant>,
    /// How long after the first frame the sequence starts: under the compositor,
    /// the time the splash's bar stays full, so that the lines sweep out as the
    /// splash fades over them.
    delay: Duration,
    finished: bool,
}

impl PowerUp {
    /// A sequence that starts at the first frame published, `delay` after it.
    pub fn new(delay: Duration) -> PowerUp {
        PowerUp { began: None, delay, finished: false }
    }

    /// The sequence under the compositor's splash: it starts as the splash fades.
    pub fn under_splash() -> PowerUp {
        PowerUp::new(des_common::splash::FULL_FOR)
    }

    /// Run again from the start, as after the shell restarts.
    #[cfg(test)]
    pub fn restart(&mut self) {
        self.began = None;
        self.finished = false;
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.finished
    }

    /// Where the sequence stands at `now`, starting it if it has not. Off, or with
    /// the lock up, it is over at once: nothing powers up behind the lock screen.
    pub fn stage_at(&mut self, now: Instant, animate: bool, locked: bool) -> Stage {
        if self.finished {
            return Stage::finished();
        }
        if !animate || locked {
            self.finished = true;
            return Stage::finished();
        }
        let began = *self.began.get_or_insert(now);
        let since = now.saturating_duration_since(began).saturating_sub(self.delay);
        if since >= TOTAL {
            self.finished = true;
            return Stage::finished();
        }
        Stage::at(since)
    }

    /// Works out the stage for this frame, puts it where the panels read it, and
    /// asks for the next frame while there is more to come.
    pub fn publish(&mut self, ctx: &egui::Context, animate: bool, locked: bool) -> Stage {
        let stage = self.stage_at(Instant::now(), animate, locked);
        if self.finished {
            ctx.data_mut(|data| data.remove_temp::<Stage>(slot()));
        } else {
            ctx.data_mut(|data| data.insert_temp(slot(), stage));
            ctx.request_repaint();
        }
        stage
    }
}

/// The part of a line from `from` to `to` that shows when it has swept `progress`
/// of the way out from `center`, both ends at once.
pub fn swept_span(center: f32, from: f32, to: f32, progress: f32) -> Option<(f32, f32)> {
    if progress <= 0.0 || to <= from {
        return None;
    }
    let reach = (center - from).abs().max((to - center).abs()) * progress;
    Some(((center - reach).max(from), (center + reach).min(to)))
}

/// Rounded panel outlines use the existing startup progress without bridging gaps.
#[allow(clippy::too_many_arguments)]
pub fn frame_lines(painter: &Painter, screen: Rect, _workspace: Rect, status_top: f32, open: Panels, collapse: bool, line: Stroke, stage: &Stage) {
    let layout=des_common::frame::Layout::new(screen.width(),status_top-screen.top()+des_common::STATUS_HEIGHT,open,
        if collapse {des_common::PanelBehavior::Collapse}else{des_common::PanelBehavior::Hide});
    let progress=stage.progress(Part::Lines);
    for rect in [Some(layout.center),layout.left,layout.right,layout.bottom].into_iter().flatten() {
        let rect=Rect::from_min_size(screen.min+egui::vec2(rect.x,rect.y),egui::vec2(rect.width,rect.height));
        if let Some((from,to))=swept_span(rect.center().x,rect.left(),rect.right(),progress) {
            let clip=Rect::from_min_max(egui::pos2(from,rect.top()),egui::pos2(to,rect.bottom())).intersect(painter.clip_rect());
            painter.with_clip_rect(clip).rect_stroke(rect,des_common::frame::RADIUS,line,egui::StrokeKind::Inside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_lines_come_first_and_the_columns_follow_eighty_milliseconds_apart() {
        let early = Stage::at(ms(100));
        assert!(early.progress(Part::Lines) > 0.0);
        assert_eq!(early.progress(Part::Left), 0.0);
        assert_eq!(early.progress(Part::Terminal), 0.0);
        let later = Stage::at(ms(300));
        assert!(later.progress(Part::Left) > 0.0);
        assert!(later.progress(Part::Right) > 0.0);
        assert_eq!(later.progress(Part::Bottom), 0.0);
        assert_eq!(Part::Left.span().0 + 80, Part::Right.span().0);
        assert_eq!(Part::Right.span().0 + 80, Part::Bottom.span().0);
        assert!(Part::Terminal.span().0 > Part::Lines.span().1);
        assert_eq!(Part::Status.span().1, TOTAL.as_millis() as u64);
        assert_eq!(Part::Instruments.span(), (400, 700));
    }

    #[test]
    fn every_part_only_ever_goes_forward_and_is_there_at_the_end() {
        let parts = [Part::Lines, Part::Left, Part::Right, Part::Bottom, Part::Terminal, Part::Status, Part::Instruments];
        for part in parts {
            let mut last = 0.0;
            for at in (0..=700).step_by(10) {
                let now = progress_for(part, ms(at));
                assert!(now >= last, "{part:?} went back at {at} ms");
                assert!((0.0..=1.0).contains(&now));
                last = now;
            }
            assert_eq!(progress_for(part, ms(700)), 1.0);
        }
        assert!(!Stage::at(ms(699)).done());
        assert!(Stage::at(ms(700)).done());
    }

    #[test]
    fn the_sequence_is_over_seven_hundred_milliseconds_after_its_delay() {
        let start = Instant::now();
        let mut power = PowerUp::new(ms(180));
        assert!(!power.stage_at(start, true, false).done());
        // Under the delay nothing has started yet.
        assert_eq!(power.stage_at(start + ms(100), true, false), Stage::at(ms(0)));
        assert!(!power.stage_at(start + ms(180 + 699), true, false).done());
        assert!(power.stage_at(start + ms(180 + 700), true, false).done());
        assert!(power.done());
        power.restart();
        assert!(!power.stage_at(start + ms(2000), true, false).done());
    }

    #[test]
    fn off_or_locked_it_is_over_at_once() {
        let now = Instant::now();
        let mut off = PowerUp::new(ms(0));
        assert!(off.stage_at(now, false, false).done());
        assert!(off.done());
        let mut locked = PowerUp::new(ms(0));
        assert!(locked.stage_at(now, true, true).done());
        // And stays over once the lock goes.
        assert!(locked.stage_at(now + ms(300), true, false).done());
    }

    #[test]
    fn the_instruments_sweep_up_to_their_readings() {
        let values: VecDeque<f32> = [10.0, 50.0, 100.0].into();
        let mid = Stage::at(ms(550));
        let swept = mid.swept_values(&values);
        assert!(swept.iter().zip(values.iter()).all(|(s, v)| *s < *v && *s > 0.0));
        let end = Stage::at(ms(700));
        assert!(matches!(end.swept_values(&values), Cow::Borrowed(_)));
        assert!(matches!(end.swept_slice(&[1.0, 2.0]), Cow::Borrowed(_)));
        assert_eq!(end.swept(37.0), 37.0);
        assert_eq!(Stage::at(ms(400)).swept(37.0), 0.0);
    }

    #[test]
    fn the_lines_sweep_out_from_the_middle_and_end_whole() {
        assert_eq!(swept_span(50.0, 0.0, 100.0, 0.0), None);
        let (a, b) = swept_span(50.0, 0.0, 100.0, 0.5).unwrap();
        assert!(a > 0.0 && a < 50.0 && b > 50.0 && b < 100.0);
        assert_eq!(swept_span(50.0, 0.0, 100.0, 1.0), Some((0.0, 100.0)));
        // A line off to one side of the middle still ends whole.
        assert_eq!(swept_span(500.0, 0.0, 100.0, 1.0), Some((0.0, 100.0)));
    }

    #[test]
    fn a_context_without_a_stage_reads_as_finished_and_publishing_puts_one_there() {
        let ctx = egui::Context::default();
        assert!(stage(&ctx).done());
        let mut power = PowerUp::new(ms(0));
        let published = power.publish(&ctx, true, false);
        assert!(!published.done());
        assert_eq!(stage(&ctx), published);
        let mut off = PowerUp::new(ms(0));
        off.publish(&ctx, false, false);
        assert!(stage(&ctx).done());
    }
}
