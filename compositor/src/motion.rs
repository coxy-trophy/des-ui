//! Motion: how windows move, appear and leave. One easing, two durations.
//!
//! Nothing here changes what a client is told. A window is configured to its final
//! place at once, and only where it is drawn is eased there; a client that is slow to
//! resize is merely scaled into its new area meanwhile. Each frame the render path
//! asks where every window is at that moment, and the damage tracker sees the
//! change, so frames keep coming while a motion is live and stop when all are done.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{AsRenderElements, surface::WaylandSurfaceRenderElement, utils::RescaleRenderElement},
        gles::GlesRenderer,
    },
    desktop::{Space, Window, space::SpaceRenderElements},
    output::Output,
    utils::{IsAlive, Logical, Point, Rectangle, Scale},
};

/// How long a window takes to reach a new place or size.
pub const GEOMETRY: Duration = Duration::from_millis(180);
/// How long a workspace takes to slide in or out.
pub const SLIDE: Duration = Duration::from_millis(220);
/// How much smaller a window is when it materialises, and how much smaller one that
/// goes out of view shrinks to.
const MATERIALISE_FROM: f64 = 0.96;
const FADE_TO: f64 = 0.98;

/// Cubic ease-out: fast to start, settling gently.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// When something started and how long it takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeline {
    pub started: Instant,
    pub duration: Duration,
}

impl Timeline {
    /// How far along, eased, from 0 to 1.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        ease(elapsed / self.duration.as_secs_f32())
    }

    pub fn done(&self, now: Instant) -> bool {
        now >= self.started + self.duration
    }
}

/// One window on its way from where it was drawn to where it belongs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub from: Rectangle<f64, Logical>,
    pub to: Rectangle<i32, Logical>,
    pub alpha_from: f32,
    pub alpha_to: f32,
    pub timeline: Timeline,
}

fn shrunk(rect: Rectangle<i32, Logical>, by: f64) -> Rectangle<f64, Logical> {
    let rect = rect.to_f64();
    let size = (rect.size.w * by, rect.size.h * by);
    Rectangle::new(
        (rect.loc.x + (rect.size.w - size.0) / 2.0, rect.loc.y + (rect.size.h - size.1) / 2.0).into(),
        size.into(),
    )
}

fn shifted(rect: Rectangle<f64, Logical>, by: f64) -> Rectangle<f64, Logical> {
    Rectangle::new((rect.loc.x + by, rect.loc.y).into(), rect.size)
}

impl Motion {
    fn new(from: Rectangle<f64, Logical>, to: Rectangle<i32, Logical>, alpha: (f32, f32), duration: Duration, now: Instant) -> Motion {
        Motion {
            from,
            to,
            alpha_from: alpha.0,
            alpha_to: alpha.1,
            timeline: Timeline { started: now, duration },
        }
    }

    /// From one place or size to another.
    pub fn geometry(from: Rectangle<f64, Logical>, to: Rectangle<i32, Logical>, now: Instant) -> Motion {
        Motion::new(from, to, (1.0, 1.0), GEOMETRY, now)
    }

    /// A new window: a little smaller and clear, growing into place about its centre.
    pub fn materialise(to: Rectangle<i32, Logical>, now: Instant) -> Motion {
        Motion::new(shrunk(to, MATERIALISE_FROM), to, (0.0, 1.0), GEOMETRY, now)
    }

    /// A window going out of view where it is: it fades, shrinking a little.
    pub fn fade(from: Rectangle<f64, Logical>, now: Instant) -> Motion {
        let to = from.to_i32_round();
        let mut end = Motion::new(from, to, (1.0, 0.0), GEOMETRY, now);
        // `to` is where the window is; the shrink is applied when asked where it is.
        end.from = from;
        end.to = shrunk(to, FADE_TO).to_i32_round();
        end
    }

    /// A window of the workspace coming into view, arriving from the side the
    /// workspace lies on: `direction` is positive when going to a higher workspace,
    /// which is to the right.
    pub fn slide_in(to: Rectangle<i32, Logical>, direction: i32, now: Instant) -> Motion {
        let from = shifted(to.to_f64(), direction.signum() as f64 * to.size.w as f64);
        Motion::new(from, to, (0.0, 1.0), SLIDE, now)
    }

    /// A window of the workspace left behind, going out the opposite side.
    pub fn slide_out(from: Rectangle<f64, Logical>, direction: i32, now: Instant) -> Motion {
        let to = shifted(from, -(direction.signum() as f64) * from.size.w).to_i32_round();
        Motion::new(from, to, (1.0, 0.0), SLIDE, now)
    }

    /// Where the window is drawn at `now`, and how opaque.
    pub fn at(&self, now: Instant) -> (Rectangle<f64, Logical>, f32) {
        let t = self.timeline.progress(now) as f64;
        let to = self.to.to_f64();
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        let rect = Rectangle::new(
            (lerp(self.from.loc.x, to.loc.x), lerp(self.from.loc.y, to.loc.y)).into(),
            (lerp(self.from.size.w, to.size.w), lerp(self.from.size.h, to.size.h)).into(),
        );
        (rect, self.alpha_from + (self.alpha_to - self.alpha_from) * t as f32)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.timeline.done(now)
    }
}

/// Whether anything may move: never behind the lock, never while the shell has the
/// display standing still for a screenshot, and never when turned off.
pub fn allowed(enabled: bool, locked: bool, picking: bool) -> bool {
    enabled && !locked && !picking
}

/// The `animate` key of the configuration file: `on` unless it says `off`.
pub fn parse_animate(text: &str) -> bool {
    text.lines()
        .filter_map(|line| line.split('#').next().unwrap_or("").split_once('='))
        .filter(|(key, _)| key.trim() == "animate")
        .map(|(_, value)| value.trim() != "off")
        .last()
        .unwrap_or(true)
}

/// Reads whether the user wants motion at all, from the shared configuration file.
pub fn wanted() -> bool {
    des_common::input::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or(true, |text| parse_animate(&text))
}

/// Every window's motion, by the window. Generic over the key so that the
/// bookkeeping can be tested without a compositor.
pub struct Motions<K> {
    /// Whether the user wants motion. Checked again when the settings are reloaded.
    pub enabled: bool,
    /// The direction of the workspace switch being arranged, if one is.
    pub slide: Option<i32>,
    /// Windows in view, on their way to their place.
    live: HashMap<K, Motion>,
    /// Windows no longer in view, still on their way out.
    leaving: Vec<(K, Motion)>,
    /// Where each window in view belongs, which is where it is drawn once still.
    drawn: HashMap<K, Rectangle<i32, Logical>>,
}

impl<K: Hash + Eq + Clone> Motions<K> {
    pub fn new(enabled: bool) -> Motions<K> {
        Motions {
            enabled,
            slide: None,
            live: HashMap::new(),
            leaving: Vec::new(),
            drawn: HashMap::new(),
        }
    }

    /// Where a window in view is drawn at `now`.
    pub fn current(&self, key: &K, now: Instant) -> Option<Rectangle<f64, Logical>> {
        match self.live.get(key) {
            Some(motion) => Some(motion.at(now).0),
            None => self.drawn.get(key).map(|rect| rect.to_f64()),
        }
    }

    /// The window is to be seen at `to`. With `animate`, it eases there from where it
    /// is, materialises if it was not in view, or slides in during a workspace switch.
    pub fn show(&mut self, key: K, to: Rectangle<i32, Logical>, now: Instant, animate: bool) {
        self.leaving.retain(|(other, _)| *other != key);
        if !animate {
            self.live.remove(&key);
            self.drawn.insert(key, to);
            return;
        }
        let heading_there = self.live.get(&key).is_some_and(|motion| motion.to == to);
        let already_there = !self.live.contains_key(&key) && self.drawn.get(&key) == Some(&to);
        if heading_there || already_there {
            return;
        }
        let motion = match self.current(&key, now) {
            Some(from) => Motion::geometry(from, to, now),
            None => match self.slide {
                Some(direction) => Motion::slide_in(to, direction, now),
                None => Motion::materialise(to, now),
            },
        };
        self.live.insert(key.clone(), motion);
        self.drawn.insert(key, to);
    }

    /// The window is no longer to be seen. With `animate`, it slides out during a
    /// workspace switch and fades otherwise.
    pub fn hide(&mut self, key: &K, now: Instant, animate: bool) {
        let from = self.current(key, now);
        self.live.remove(key);
        if self.drawn.remove(key).is_none() {
            return;
        }
        if let (true, Some(from)) = (animate, from) {
            let motion = match self.slide {
                Some(direction) => Motion::slide_out(from, direction, now),
                None => Motion::fade(from, now),
            };
            self.leaving.retain(|(other, _)| other != key);
            self.leaving.push((key.clone(), motion));
        }
    }

    /// The window is gone for good.
    pub fn forget(&mut self, key: &K) {
        self.live.remove(key);
        self.drawn.remove(key);
        self.leaving.retain(|(other, _)| other != key);
    }

    /// Everything goes to its place at once.
    pub fn settle(&mut self) {
        self.live.clear();
        self.leaving.clear();
    }

    /// Drops what has arrived.
    pub fn tick(&mut self, now: Instant) {
        self.live.retain(|_, motion| !motion.done(now));
        self.leaving.retain(|(_, motion)| !motion.done(now));
    }

    pub fn is_live(&self) -> bool {
        !self.live.is_empty() || !self.leaving.is_empty()
    }

    /// Where a window in view is drawn and how opaque, while it is moving.
    pub fn moving(&self, key: &K, now: Instant) -> Option<(Rectangle<f64, Logical>, f32)> {
        self.live.get(key).map(|motion| motion.at(now))
    }

    /// The windows on their way out, with where each is drawn and how opaque.
    pub fn leaving(&self, now: Instant) -> impl Iterator<Item = (&K, Rectangle<f64, Logical>, f32)> {
        self.leaving.iter().map(move |(key, motion)| {
            let (rect, alpha) = motion.at(now);
            (key, rect, alpha)
        })
    }
}

type Surface = WaylandSurfaceRenderElement<GlesRenderer>;

/// What one window contributes to a display: drawn at `rect`, scaled from the size
/// the client has given it, and `alpha` opaque.
pub(crate) fn window_at<C>(
    renderer: &mut GlesRenderer,
    window: &Window,
    rect: Rectangle<f64, Logical>,
    alpha: f32,
    moving: bool,
    output_geo: Rectangle<i32, Logical>,
    scale: f64,
) -> Vec<C>
where
    C: From<SpaceRenderElements<GlesRenderer, Surface>> + From<RescaleRenderElement<Surface>>,
{
    let geometry = window.geometry();
    let location = rect.loc - geometry.loc.to_f64() - output_geo.loc.to_f64();
    let elements: Vec<Surface> =
        window.render_elements(renderer, location.to_physical_precise_round(scale), Scale::from(scale), alpha);
    if !moving {
        return elements.into_iter().map(|e| C::from(SpaceRenderElements::Surface(e))).collect();
    }
    let origin: Point<i32, smithay::utils::Physical> = (rect.loc - output_geo.loc.to_f64()).to_physical_precise_round(scale);
    let by = Scale {
        x: if geometry.size.w > 0 { rect.size.w / geometry.size.w as f64 } else { 1.0 },
        y: if geometry.size.h > 0 { rect.size.h / geometry.size.h as f64 } else { 1.0 },
    };
    elements
        .into_iter()
        .map(|e| C::from(RescaleRenderElement::from_element(e, origin, by)))
        .collect()
}

/// Everything in the space as something to draw on `output`, front to back, with
/// windows in motion drawn where they are at `now` and windows on their way out
/// still drawn beneath the ones in view. Replaces Smithay's space rendering so that
/// a window can be drawn anywhere but where it is mapped.
pub fn window_elements<C>(
    renderer: &mut GlesRenderer,
    space: &Space<Window>,
    output: &Output,
    motions: &Motions<Window>,
    beneath: &[&Window],
    // Drawn below the applications and above what is `beneath`: the layer surfaces
    // that belong under the windows.
    under_apps: Vec<C>,
    now: Instant,
) -> Vec<C>
where
    C: From<SpaceRenderElements<GlesRenderer, Surface>> + From<RescaleRenderElement<Surface>>,
{
    let Some(output_geo) = space.output_geometry(output) else {
        return Vec::new();
    };
    let scale = output.current_scale().fractional_scale();
    let mut elements = Vec::new();
    let mut bottom = Vec::new();
    // The space keeps its windows back to front; what is drawn is listed front to back.
    for window in space.elements().rev() {
        let Some(location) = space.element_location(window) else {
            continue;
        };
        let (rect, alpha, moving) = match motions.moving(window, now) {
            Some((rect, alpha)) => (rect, alpha, true),
            None => (Rectangle::new(location, window.geometry().size).to_f64(), 1.0, false),
        };
        if !output_geo.to_f64().overlaps(rect) {
            continue;
        }
        let drawn = window_at::<C>(renderer, window, rect, alpha, moving, output_geo, scale);
        if beneath.contains(&window) {
            bottom.extend(drawn);
        } else {
            elements.extend(drawn);
        }
    }
    for (window, rect, alpha) in motions.leaving(now) {
        if output_geo.to_f64().overlaps(rect) && window.alive() {
            elements.extend(window_at::<C>(renderer, window, rect, alpha, true, output_geo, scale));
        }
    }
    elements.extend(under_apps);
    elements.extend(bottom);
    elements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn easing_starts_fast_and_settles() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!(ease(0.5) > 0.8 && ease(0.5) < 0.9);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn a_motion_runs_from_its_start_to_its_end() {
        let now = Instant::now();
        let motion = Motion::geometry(rect(0, 0, 100, 100).to_f64(), rect(200, 50, 50, 50), now);
        let (start, alpha) = motion.at(now);
        assert_eq!(start, rect(0, 0, 100, 100).to_f64());
        assert_eq!(alpha, 1.0);
        let (end, _) = motion.at(now + GEOMETRY);
        assert_eq!(end.to_i32_round(), rect(200, 50, 50, 50));
        assert!(!motion.done(now + GEOMETRY / 2));
        assert!(motion.done(now + GEOMETRY));
        let (middle, _) = motion.at(now + GEOMETRY / 2);
        assert!(middle.loc.x > 150.0 && middle.loc.x < 200.0, "eased past halfway: {middle:?}");
    }

    #[test]
    fn materialising_keeps_the_centre_and_grows_from_clear() {
        let now = Instant::now();
        let to = rect(100, 100, 200, 100);
        let motion = Motion::materialise(to, now);
        let (start, alpha) = motion.at(now);
        assert_eq!(alpha, 0.0);
        assert!((start.size.w - 192.0).abs() < 0.01 && (start.size.h - 96.0).abs() < 0.01);
        assert!((start.loc.x + start.size.w / 2.0 - 200.0).abs() < 0.01);
        assert!((start.loc.y + start.size.h / 2.0 - 150.0).abs() < 0.01);
        assert_eq!(motion.at(now + GEOMETRY), (to.to_f64(), 1.0));
    }

    #[test]
    fn workspaces_slide_from_the_side_they_lie_on() {
        let now = Instant::now();
        let to = rect(0, 0, 800, 600);
        // Going up a workspace, the new one comes from the right and the old goes left.
        let arriving = Motion::slide_in(to, 1, now);
        assert_eq!(arriving.at(now).0.loc.x, 800.0);
        assert_eq!(arriving.at(now + SLIDE).0, to.to_f64());
        let leaving = Motion::slide_out(to.to_f64(), 1, now);
        assert_eq!(leaving.at(now + SLIDE).0.loc.x, -800.0);
        assert_eq!(leaving.at(now + SLIDE).1, 0.0);
        // Going down, the other way round.
        assert_eq!(Motion::slide_in(to, -1, now).at(now).0.loc.x, -800.0);
        assert_eq!(Motion::slide_out(to.to_f64(), -1, now).at(now + SLIDE).0.loc.x, 800.0);
    }

    #[test]
    fn nothing_moves_behind_the_lock_or_a_screenshot_or_when_turned_off() {
        assert!(allowed(true, false, false));
        assert!(!allowed(true, true, false));
        assert!(!allowed(true, false, true));
        assert!(!allowed(false, false, false));
    }

    #[test]
    fn reads_the_animate_setting() {
        assert!(parse_animate(""));
        assert!(parse_animate("animate = on\n"));
        assert!(!parse_animate("font_size = 14\nanimate = off # none\n"));
        assert!(parse_animate("animate = off\nanimate = on\n"));
        assert!(parse_animate("# animate = off\n"));
    }

    #[test]
    fn windows_materialise_then_ease_between_places() {
        let now = Instant::now();
        let mut motions: Motions<u32> = Motions::new(true);
        motions.show(1, rect(0, 0, 100, 100), now, true);
        let (start, alpha) = motions.moving(&1, now).unwrap();
        assert_eq!(alpha, 0.0);
        assert!(start.size.w < 100.0);
        // Shown again where it is: nothing new starts.
        let before = motions.moving(&1, now);
        motions.show(1, rect(0, 0, 100, 100), now + GEOMETRY / 2, true);
        assert_eq!(motions.moving(&1, now), before);
        // Still, then moved: it eases from where it was drawn.
        let later = now + GEOMETRY;
        motions.tick(later);
        assert!(!motions.is_live());
        motions.show(1, rect(50, 0, 100, 100), later, true);
        let (from, alpha) = motions.moving(&1, later).unwrap();
        assert_eq!(from, rect(0, 0, 100, 100).to_f64());
        assert_eq!(alpha, 1.0);
        assert_eq!(motions.current(&1, later + GEOMETRY), Some(rect(50, 0, 100, 100).to_f64()));
    }

    #[test]
    fn a_switch_slides_the_old_out_and_the_new_in() {
        let now = Instant::now();
        let mut motions: Motions<u32> = Motions::new(true);
        motions.show(1, rect(0, 0, 100, 100), now, true);
        motions.tick(now + GEOMETRY);
        motions.slide = Some(1);
        motions.hide(&1, now, true);
        motions.show(2, rect(0, 0, 100, 100), now, true);
        motions.slide = None;
        let leaving: Vec<_> = motions.leaving(now).collect();
        assert_eq!(leaving.len(), 1);
        assert_eq!(*leaving[0].0, 1);
        assert_eq!(leaving[0].1.loc.x, 0.0);
        assert_eq!(motions.leaving(now + SLIDE).next().unwrap().1.loc.x, -100.0);
        assert_eq!(motions.moving(&2, now).unwrap().0.loc.x, 100.0);
        assert!(motions.current(&1, now).is_none());
        // The one that left comes back before it is gone: it is no longer leaving.
        motions.show(1, rect(0, 0, 100, 100), now, true);
        assert_eq!(motions.leaving(now).count(), 0);
        motions.tick(now + SLIDE);
        assert!(!motions.is_live());
    }

    #[test]
    fn a_window_hidden_in_place_fades_and_turned_off_nothing_moves() {
        let now = Instant::now();
        let mut motions: Motions<u32> = Motions::new(true);
        motions.show(1, rect(0, 0, 100, 100), now, true);
        motions.tick(now + GEOMETRY);
        motions.hide(&1, now, true);
        let (_, _, alpha) = motions.leaving(now).next().unwrap();
        assert_eq!(alpha, 1.0);
        assert_eq!(motions.leaving(now + GEOMETRY).next().unwrap().2, 0.0);
        motions.forget(&1);
        assert!(!motions.is_live());

        motions.show(2, rect(0, 0, 100, 100), now, false);
        assert!(motions.moving(&2, now).is_none());
        assert_eq!(motions.current(&2, now), Some(rect(0, 0, 100, 100).to_f64()));
        motions.hide(&2, now, false);
        assert!(!motions.is_live());
    }
}
