//! A solid cursor family drawn from vector outlines. The point stays fixed while
//! silhouettes interpolate: applications name a semantic cursor, and the compositor
//! supplies its own triangle, needle, lozenge or directional shape at output density.
use std::{collections::VecDeque, time::{Duration, Instant}};

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    delegate_cursor_shape,
    input::pointer::CursorIcon,
    utils::{Logical, Point, Transform},
};
use crate::DesComp;

mod art;

delegate_cursor_shape!(DesComp);

// Cursor shapes may be asked for a drawing tablet's pen too; there is none to draw.
impl smithay::wayland::tablet_manager::TabletSeatHandler for DesComp {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Arrow, Link, Beam, VerticalBeam, Grab, Grabbing,
    ResizeHorizontal, ResizeVertical, ResizeNwSe, ResizeNeSw,
    /// Clockwise eighth-turns, beginning at east. Single edges/corners retain
    /// their direction instead of becoming an ambiguous bidirectional arrow.
    Direction(u8),
    Working, Forbidden, Copy, Move, Alias, Crosshair, Cell, ZoomIn, ZoomOut, Help, Menu,
}

pub fn shape_for(icon: CursorIcon) -> Shape {
    match icon {
        CursorIcon::Text => Shape::Beam,
        CursorIcon::VerticalText => Shape::VerticalBeam,
        CursorIcon::Pointer => Shape::Link,
        CursorIcon::Grab => Shape::Grab,
        CursorIcon::Grabbing => Shape::Grabbing,
        CursorIcon::EwResize | CursorIcon::ColResize => Shape::ResizeHorizontal,
        CursorIcon::NsResize | CursorIcon::RowResize => Shape::ResizeVertical,
        CursorIcon::NwseResize => Shape::ResizeNwSe,
        CursorIcon::NeswResize => Shape::ResizeNeSw,
        CursorIcon::EResize => Shape::Direction(0),
        CursorIcon::SeResize => Shape::Direction(1),
        CursorIcon::SResize => Shape::Direction(2),
        CursorIcon::SwResize => Shape::Direction(3),
        CursorIcon::WResize => Shape::Direction(4),
        CursorIcon::NwResize => Shape::Direction(5),
        CursorIcon::NResize => Shape::Direction(6),
        CursorIcon::NeResize => Shape::Direction(7),
        CursorIcon::Wait | CursorIcon::Progress => Shape::Working,
        CursorIcon::NotAllowed | CursorIcon::NoDrop => Shape::Forbidden,
        CursorIcon::Copy => Shape::Copy,
        CursorIcon::Move | CursorIcon::AllScroll | CursorIcon::AllResize => Shape::Move,
        CursorIcon::Alias => Shape::Alias,
        CursorIcon::Crosshair => Shape::Crosshair,
        CursorIcon::Cell => Shape::Cell,
        CursorIcon::ZoomIn => Shape::ZoomIn,
        CursorIcon::ZoomOut => Shape::ZoomOut,
        CursorIcon::Help | CursorIcon::DndAsk => Shape::Help,
        CursorIcon::ContextMenu => Shape::Menu,
        _ => Shape::Arrow,
    }
}

/// Pointer appearance, independent of the output's raster density.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferences {
    pub size: u32,
    pub color: [u8; 3],
}

impl Default for Preferences {
    fn default() -> Self { Self { size: des_common::input::CURSOR_BASE_SIZE, color: des_common::input::DEFAULT_CURSOR_COLOR } }
}

impl From<&des_common::input::InputSettings> for Preferences {
    fn from(settings: &des_common::input::InputSettings) -> Self {
        Self { size: settings.cursor_size.clamp(16,64), color: settings.cursor_color }
    }
}

impl Preferences {
    fn normalized(self) -> Self { Self { size: self.size.clamp(16,64), ..self } }
}

/// Canonical artwork for legacy Xcursor themes, using the default appearance.
#[cfg(test)]
pub fn theme_image(icon: CursorIcon, nominal_size: u32) -> (u32, u32, u32, u32, Vec<u8>) {
    theme_image_with_preferences(icon,nominal_size,Preferences::default())
}

/// Export the same artwork in a requested physical nominal size and RGB color.
/// Theme writers supply several sizes; `nominal_size` determines this image's
/// geometry, while preferences provide its color. Allocation is bounded to 16–64.
pub fn theme_image_with_preferences(icon: CursorIcon, nominal_size: u32, preferences: Preferences) -> (u32, u32, u32, u32, Vec<u8>) {
    let size=nominal_size.clamp(16,64);
    let shape=shape_for(icon);
    let pixels=art::raster_custom(&art::boundary(shape,0.0),shape,1.0,size,preferences.color,1.0);
    let side=size*2;
    let hotspot=art::logical_point([16.0,16.0],size)[0] as u32;
    (side,side,hotspot,hotspot,pixels)
}

/// Configured logical artwork at an integer output density. Scale the integer
/// logical anchor exactly as `get` does; large physical images retain their full
/// resolution instead of being clamped as though their size were logical.
pub fn theme_image_at_density(icon: CursorIcon, preferences: Preferences, density: u32) -> (u32,u32,u32,u32,Vec<u8>) {
    let preferences=preferences.normalized();
    let density=density.clamp(1,4);
    let shape=shape_for(icon);
    let pixels=art::raster_custom(&art::boundary(shape,0.0),shape,density as f32,preferences.size,preferences.color,1.0);
    let side=2*preferences.size*density;
    let hotspot=art::logical_point([16.0,16.0],preferences.size)[0] as u32*density;
    (side,side,hotspot,hotspot,pixels)
}

const MORPH: Duration = Duration::from_millis(120);
const CACHE_LIMIT: usize = 64;

fn density(scale: f64) -> usize {
    if scale.is_finite() { scale.ceil().clamp(1.0, 4.0) as usize } else { 1 }
}

#[derive(PartialEq, Eq)]
struct Key {
    shape: Shape,
    density: usize,
    detail: u8,
    points: [[i32; 2]; art::SAMPLES],
}

struct Cached {
    key: Key,
    // Boxes keep the cached buffer's identity/address stable when LRU entries move.
    buffer: Box<MemoryRenderBuffer>,
}

/// Bounded raster cache and one logical transition shared by all output scales.
/// A scale change only changes raster density; it never restarts the morph.
pub struct Cursors {
    preferences: Preferences,
    speaking_until: Option<Instant>,
    speaking_started: Option<Instant>,
    shape: Option<Shape>,
    from: art::Boundary,
    started: Option<Instant>,
    transitioning: bool,
    target_offset: usize,
    cache: VecDeque<Cached>,
}

impl Cursors {
    pub fn new() -> Self {
        Self {
            preferences: Preferences::default(),
            speaking_until: None,
            speaking_started: None,
            shape: None,
            from: art::boundary(Shape::Arrow, 0.0),
            started: None,
            transitioning: false,
            target_offset: 0,
            cache: VecDeque::new(),
        }
    }

    /// Re-rasterize appearance changes without restarting the canonical morph.
    /// All cached renderer identities belong to one normalized appearance only.
    pub fn configure(&mut self, preferences: Preferences) {
        let preferences=preferences.normalized();
        if self.preferences != preferences {
            self.preferences=preferences;
            self.cache.clear();
        }
    }

    /// A shell heartbeat is deliberately temporary, including after shell crashes.
    pub fn set_speaking(&mut self, active: bool, now: Instant) {
        if active {
            if self.speaking_until.is_none_or(|until| now >= until) {
                self.speaking_started = Some(now);
            }
            self.speaking_until = Some(now + Duration::from_secs(2));
        } else {
            self.speaking_until = None;
            self.speaking_started = None;
        }
    }

    fn speaking_boundary(&self, boundary: art::Boundary, now: Instant, animate: bool) -> art::Boundary {
        if !animate || !self.speaking_until.is_some_and(|until| now < until) { return boundary; }
        let elapsed = now.saturating_duration_since(self.speaking_started.unwrap_or(now)).as_secs_f32();
        // Sixteen cached phases; scale around the exact logical hotspot. The
        // canvas, configured colour and semantic silhouette remain unchanged.
        let phase = ((elapsed % 1.2) / 1.2 * 16.0).floor();
        let amount = 1.0 + 0.04 * (1.0 - (phase / 16.0 * std::f32::consts::TAU).cos());
        boundary.map(|p| p.map(|v| 16.0 + (v - 16.0) * amount))
    }

    fn progress(&self, now: Instant) -> f32 {
        let elapsed = self.started.map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
        // Quantization bounds nearly-identical animation frames while retaining
        // more than one interpolation sample per display frame at 120 Hz.
        ((elapsed.as_secs_f32() / MORPH.as_secs_f32()).clamp(0.0, 1.0) * 60.0).round() / 60.0
    }

    fn current(&self, now: Instant, animate: bool) -> art::Boundary {
        let shape = self.shape.unwrap_or(Shape::Arrow);
        let turn = if animate && shape == Shape::Working {
            let elapsed = self.started.map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
            // A quiet four-second turn, cached in 48 phases. Reduced motion
            // freezes the seed at its upright silhouette.
            let phase = ((elapsed.as_secs_f64() % 4.0) * 12.0).floor() as u32;
            phase as f32 / 48.0 * std::f32::consts::TAU
        } else { 0.0 };
        let mut to = art::boundary(shape, turn);
        if animate && self.transitioning {
            to.rotate_left(self.target_offset);
            let t = self.progress(now);
            art::interpolate(&self.from, &to, t*t*(3.0-2.0*t))
        } else { to }
    }

    /// The raster and one logical hotspot at the configured size, constant across
    /// shapes, output scales and interpolation frames. Retargeting starts at the
    /// current visible outline.
    pub fn get(&mut self, icon: CursorIcon, scale: f64, animate: bool, now: Instant) -> (&MemoryRenderBuffer, Point<i32, Logical>) {
        let shape = shape_for(icon);
        if self.shape != Some(shape) {
            self.from = self.current(now, animate);
            self.target_offset = art::alignment(&self.from, &art::boundary(shape, 0.0));
            self.transitioning = animate && self.shape.is_some();
            self.shape = Some(shape);
            self.started = Some(now);
        }
        if !animate || self.progress(now) >= 1.0 {
            self.transitioning = false;
        }
        let boundary = self.speaking_boundary(self.current(now, animate), now, animate);
        let detail = if self.transitioning { (self.progress(now)*255.0).round() as u8 } else { 255 };
        let density = density(scale);
        let key = Key {
            shape, density, detail,
            points: boundary.map(|p| p.map(|v| (v*1024.0).round() as i32)),
        };
        if let Some(at) = self.cache.iter().position(|entry| entry.key == key) {
            let entry = self.cache.remove(at).expect("cache index exists");
            self.cache.push_back(entry);
        } else {
            let data = art::raster_custom(&boundary, shape, density as f32, self.preferences.size, self.preferences.color, detail as f32/255.0);
            let side = (self.preferences.size as usize * 2 * density) as i32;
            let buffer = MemoryRenderBuffer::from_slice(&data, Fourcc::Argb8888, (side, side), density as i32, Transform::Normal, None);
            if self.cache.len() == CACHE_LIMIT { self.cache.pop_front(); }
            self.cache.push_back(Cached { key, buffer: Box::new(buffer) });
        }
        let hotspot=art::logical_point([16.0,16.0],self.preferences.size)[0] as i32;
        (&self.cache.back().expect("cursor was cached").buffer, (hotspot,hotspot).into())
    }

    /// Hidden and client-supplied cursors break compositor animation continuity.
    /// Resuming a named cursor displays its target immediately, without a stale morph.
    pub fn suspend(&mut self) {
        self.shape = None;
        self.started = None;
        self.transitioning = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaking_expires_and_reduced_motion_keeps_the_same_outline() {
        let now = Instant::now();
        let mut cursors = Cursors::new();
        cursors.get(CursorIcon::Default, 1.0, false, now);
        let original = cursors.current(now, false);
        cursors.set_speaking(true, now);
        assert_eq!(cursors.current(now + Duration::from_millis(400), false), original);
        assert_ne!(cursors.speaking_boundary(original, now + Duration::from_millis(400), true), original);
        assert_eq!(cursors.speaking_boundary(original, now + Duration::from_secs(3), true), original);
        for shape in SHAPES {
            let boundary = art::boundary(*shape, 0.0);
            for phase in 0..16 {
                let pulsed = cursors.speaking_boundary(boundary, now + Duration::from_millis(phase*75), true);
                for point in pulsed {
                    assert!(point.into_iter().all(|v| (0.0..=art::SIDE as f32).contains(&v)));
                }
            }
        }
        cursors.set_speaking(false, now);
        assert_eq!(cursors.speaking_boundary(original, now, true), original);
    }

    #[test]
    fn speaking_keeps_hotspot_and_cache_bounded() {
        let now = Instant::now();
        let mut cursors = Cursors::new();
        let hotspot = cursors.get(CursorIcon::Pointer, 2.0, true, now).1;
        cursors.set_speaking(true, now);
        for frame in 0..240 {
            assert_eq!(cursors.get(CursorIcon::Pointer, 2.0, true, now + Duration::from_millis(frame*7)).1, hotspot);
            assert!(cursors.cache.len() <= CACHE_LIMIT);
        }
    }

    #[test]
    fn semantic_shapes_are_not_collapsed_to_the_default_arrow() {
        for icon in [CursorIcon::Wait, CursorIcon::NotAllowed, CursorIcon::Copy, CursorIcon::Move, CursorIcon::Alias, CursorIcon::Crosshair, CursorIcon::Cell, CursorIcon::ZoomIn, CursorIcon::ZoomOut, CursorIcon::NsResize, CursorIcon::NwseResize, CursorIcon::NeswResize] {
            assert_ne!(shape_for(icon), Shape::Arrow, "{icon:?}");
        }
        assert_ne!(shape_for(CursorIcon::NwseResize), shape_for(CursorIcon::NeswResize));
        assert_ne!(shape_for(CursorIcon::Grab), shape_for(CursorIcon::Grabbing));
    }
    const SHAPES: &[Shape] = &[
        Shape::Arrow, Shape::Link, Shape::Beam, Shape::VerticalBeam, Shape::Grab,
        Shape::Grabbing, Shape::ResizeHorizontal, Shape::ResizeVertical,
        Shape::ResizeNwSe, Shape::ResizeNeSw, Shape::Working, Shape::Forbidden,
        Shape::Copy, Shape::Move, Shape::Alias, Shape::Crosshair, Shape::Cell,
        Shape::ZoomIn, Shape::ZoomOut, Shape::Help, Shape::Menu,
        Shape::Direction(0), Shape::Direction(1), Shape::Direction(2),
        Shape::Direction(3), Shape::Direction(4), Shape::Direction(5),
        Shape::Direction(6), Shape::Direction(7),
    ];

    fn connected(pixels: &[u8], side: usize) -> bool {
        let occupied: Vec<bool> = pixels.chunks_exact(4).map(|p| p[3] > 0).collect();
        let Some(first) = occupied.iter().position(|v| *v) else { return false };
        let mut visited = vec![false; occupied.len()];
        let mut queue = VecDeque::from([first]);
        visited[first] = true;
        while let Some(at) = queue.pop_front() {
            let (x, y) = ((at%side) as i32, (at/side) as i32);
            for dy in -1..=1 { for dx in -1..=1 {
                let (nx, ny) = (x+dx, y+dy);
                if nx < 0 || ny < 0 || nx >= side as i32 || ny >= side as i32 { continue; }
                let next = ny as usize*side+nx as usize;
                if occupied[next] && !visited[next] { visited[next] = true; queue.push_back(next); }
            }}
        }
        occupied.iter().zip(visited).all(|(filled, visited)| !filled || visited)
    }

    #[test]
    fn solid_art_has_one_component_filled_body_soft_shadow_and_transparent_border() {
        for shape in SHAPES.iter().copied() {
            for scale in [1, 2] {
                let pixels = art::raster(&art::boundary(shape, 0.0), shape, scale, 1.0);
                let side = art::SIDE * scale;
                assert!(connected(&pixels, side), "disconnected {shape:?} at {scale}");
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 255 && p[0] > 230), "no light fill {shape:?}");
                assert!(pixels.chunks_exact(4).any(|p| p[3]>0 && p[3]<200), "no antialiased shadow {shape:?}");
                assert!(pixels.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]), "not premultiplied");
                for edge in 0..side {
                    for at in [edge, (side-1)*side+edge, edge*side, edge*side+side-1] {
                        assert_eq!(pixels[at*4+3], 0, "clipped {shape:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn pointer_and_text_bodies_have_no_opaque_outline_at_any_size_or_density() {
        for shape in [Shape::Arrow,Shape::Link,Shape::Beam,Shape::Grab,Shape::ResizeHorizontal] {
            for size in [16,24,40,64] {for density in [1.0,2.0,4.0] {for color in [[247,246,242],[19,23,29]] {
                let pixels=art::raster_custom(&art::boundary(shape,0.0),shape,density,size,color,1.0);
                let body:Vec<_>=pixels.chunks_exact(4).filter(|pixel|pixel[3]==255).collect();
                assert!(!body.is_empty(),"missing body at {shape:?} {size}/{density}");
                assert!(body.iter().all(|pixel|pixel[..3]==[color[2],color[1],color[0]]),"opaque contrast outline returned at {shape:?} {size}/{density}");
                assert!(pixels.chunks_exact(4).any(|pixel|pixel[3]>0 && pixel[3]<128),"missing subtle shadow/edge");
            }}}
        }
    }
    #[test]
    fn grab_is_a_slanted_pointed_lozenge_and_grabbing_is_compressed() {
        let grab=art::boundary(Shape::Grab,0.0);
        let grabbing=art::boundary(Shape::Grabbing,0.0);
        assert!(grab[0][0]<12.0 && grab[0][1]<12.0, "grab points upper left");
        let area=|boundary: &art::Boundary| (0..art::SAMPLES).map(|i| {
            let a=boundary[i]; let b=boundary[(i+1)%art::SAMPLES]; a[0]*b[1]-b[0]*a[1]
        }).sum::<f32>().abs();
        assert!(area(&grabbing)<area(&grab));
        let seed=art::boundary(Shape::Working,0.0);
        assert!(seed[0][0]<12.0 && seed[0][1]<12.0, "working seed initially points upper left");
    }

    #[test]
    fn insertion_needle_keeps_a_light_core_along_the_stem_at_one_x() {
        let pixels=art::raster(&art::boundary(Shape::Beam,0.0),Shape::Beam,1,1.0);
        for y in 9..22 {
            assert!((14..19).any(|x| { let at=(y*art::SIDE+x)*4; pixels[at+3]==255 && pixels[at]>230 }), "dark-only insertion stem at row {y}");
        }
    }

    #[test]
    fn default_is_a_convex_scalene_triangle_and_morphs_stay_connected() {
        let arrow = art::boundary(Shape::Arrow, 0.0);
        for i in 0..art::SAMPLES {
            let a=arrow[i]; let b=arrow[(i+1)%art::SAMPLES]; let c=arrow[(i+2)%art::SAMPLES];
            let cross=(b[0]-a[0])*(c[1]-b[1])-(b[1]-a[1])*(c[0]-b[0]);
            assert!(cross >= -0.001, "default outline has a notch");
        }
        for shape in [Shape::Link, Shape::Beam, Shape::Grab, Shape::ResizeHorizontal, Shape::Working] {
            let to=art::boundary(shape, 0.0);
            for step in 0..=4 {
                let boundary=art::interpolate(&arrow, &to, step as f32/4.0);
                assert!(connected(&art::raster(&boundary, shape, 1, 1.0), art::SIDE), "{shape:?} at {step}");
            }
        }
    }

    #[test]
    fn opposite_direction_morph_does_not_collapse_at_its_midpoint() {
        let now=Instant::now(); let mut cursors=Cursors::new();
        cursors.get(CursorIcon::EResize,1.0,true,now);
        cursors.get(CursorIcon::WResize,1.0,true,now);
        let boundary=cursors.current(now+Duration::from_millis(60),true);
        let pixels=art::raster(&boundary,Shape::Direction(4),1,1.0);
        assert!(connected(&pixels,art::SIDE), "opposite directions must keep a visible silhouette");
        assert!(pixels.chunks_exact(4).filter(|p| p[3]>0).count()>20);
    }

    #[test]
    fn hotspot_is_constant_through_scales_morphs_and_midflight_retargeting() {
        let now=Instant::now();
        let mut cursors=Cursors::new();
        assert_eq!(cursors.get(CursorIcon::Default, 1.0, true, now).1, (16,16).into());
        cursors.get(CursorIcon::Text, 1.0, true, now);
        let midway=now+Duration::from_millis(60);
        let visible=cursors.current(midway, true);
        assert_ne!(visible, art::boundary(Shape::Arrow, 0.0));
        assert_ne!(visible, art::boundary(Shape::Beam, 0.0));
        assert_eq!(cursors.get(CursorIcon::Pointer, 2.5, true, midway).1, (16,16).into());
        assert_eq!(cursors.from, visible, "retarget must start at the visible outline");
        assert_eq!(cursors.current(midway, true), visible);
        for (milliseconds, scale) in [(1,1.0), (60,1.5), (120,4.0)] {
            assert_eq!(cursors.get(CursorIcon::Pointer, scale, true, midway+Duration::from_millis(milliseconds)).1, (16,16).into());
        }
        assert_eq!(cursors.current(midway+MORPH, true), art::boundary(Shape::Link, 0.0));
    }

    #[test]
    fn reduced_motion_snaps_and_freezes_working_and_suspend_discards_stale_morphs() {
        let now=Instant::now(); let mut cursors=Cursors::new();
        cursors.get(CursorIcon::Default, 1.0, true, now);
        cursors.get(CursorIcon::Text, 1.0, true, now);
        cursors.get(CursorIcon::Text, 1.0, false, now+Duration::from_millis(20));
        assert!(!cursors.transitioning);
        assert_eq!(cursors.current(now, false), art::boundary(Shape::Beam, 0.0));
        let first=cursors.get(CursorIcon::Wait, 1.0, false, now).0 as *const _;
        let later=cursors.get(CursorIcon::Wait, 1.0, false, now+Duration::from_secs(10)).0 as *const _;
        assert_eq!(first, later);
        cursors.get(CursorIcon::Wait, 1.0, true, now+Duration::from_secs(11));
        assert_ne!(cursors.current(now+Duration::from_secs(11), true), art::boundary(Shape::Working, 0.0));
        cursors.suspend();
        cursors.get(CursorIcon::Pointer, 1.0, true, now+Duration::from_secs(12));
        assert!(!cursors.transitioning);
        assert_eq!(cursors.current(now, true), art::boundary(Shape::Link, 0.0));
    }

    #[test]
    fn density_is_ceil_bounded_and_idle_rasters_reuse_the_same_buffer_identity() {
        for (scale, expected) in [(0.0,1), (-2.0,1), (1.0,1), (1.1,2), (1.75,2), (3.1,4), (100.0,4), (f64::NAN,1), (f64::INFINITY,1)] {
            assert_eq!(density(scale), expected);
        }
        let now=Instant::now(); let mut cursors=Cursors::new();
        let first=cursors.get(CursorIcon::Default, 1.0, true, now).0 as *const _;
        let idle=cursors.get(CursorIcon::Default, 1.0, true, now+Duration::from_secs(60)).0 as *const _;
        assert_eq!(first, idle);
        cursors.get(CursorIcon::Text, 1.0, false, now);
        let back=cursors.get(CursorIcon::Default, 1.0, false, now).0 as *const _;
        assert_eq!(first, back, "cached buffer identities survive LRU movement");
        let hi=cursors.get(CursorIcon::Default, 1.25, false, now).0 as *const _;
        assert_ne!(first, hi);
        assert_eq!(hi, cursors.get(CursorIcon::Default, 1.75, false, now).0 as *const _);
    }

    #[test]
    fn working_frames_cannot_grow_the_raster_cache_without_bound() {
        let now=Instant::now(); let mut cursors=Cursors::new();
        for frame in 0..80 {
            cursors.get(CursorIcon::Wait, if frame%2==0 {1.0} else {2.0}, true, now+Duration::from_millis(frame*87));
            assert!(cursors.cache.len() <= CACHE_LIMIT);
        }
        for frame in 0..25 {
            cursors.get(CursorIcon::Wait, 3.0, true, now+Duration::from_millis(frame*87));
            assert!(cursors.cache.len() <= CACHE_LIMIT);
        }
        assert_eq!(cursors.cache.len(), CACHE_LIMIT);
    }

    #[test]
    fn resize_directions_and_semantic_details_are_visually_distinct() {
        let pixels=|icon| { let shape=shape_for(icon); art::raster(&art::boundary(shape,0.0),shape,1,1.0) };
        for (a,b) in [(CursorIcon::EwResize,CursorIcon::NsResize),(CursorIcon::NwseResize,CursorIcon::NeswResize),(CursorIcon::NwResize,CursorIcon::SeResize),(CursorIcon::EResize,CursorIcon::WResize),(CursorIcon::Copy,CursorIcon::Alias),(CursorIcon::ZoomIn,CursorIcon::ZoomOut),(CursorIcon::Crosshair,CursorIcon::Cell),(CursorIcon::Grab,CursorIcon::Grabbing),(CursorIcon::Text,CursorIcon::VerticalText)] {
            assert_ne!(pixels(a),pixels(b), "{a:?} versus {b:?}");
        }
    }

    #[test]
    fn legacy_theme_exports_use_the_same_art_at_fractional_nominal_sizes() {
        for nominal in [24,32,48] {
            let (w,h,x,y,pixels)=theme_image(CursorIcon::Default,nominal);
            assert_eq!((w,h),(nominal*2,nominal*2));
            assert_eq!(x,y);
            assert_eq!(x,(16.0*nominal as f32/24.0).round() as u32);
            assert_eq!(pixels.len(),(w*h*4) as usize);
            assert!(connected(&pixels,w as usize));
        }
        assert_eq!(theme_image(CursorIcon::Default,24).4,art::raster(&art::boundary(Shape::Arrow,0.0),Shape::Arrow,1,1.0));
    }

    /// Optional review artifact only: DES_CURSOR_PREVIEW=/tmp/cursors.ppm.
    #[test]
    fn export_contact_sheet_when_requested() {
        let Ok(path)=std::env::var("DES_CURSOR_PREVIEW") else { return };
        let columns=10; let cell=144; let rows=SHAPES.len().div_ceil(columns)*2;
        let (width,height)=(columns*cell,rows*cell);
        let mut rgb=vec![0; width*height*3];
        for y in 0..height { for x in 0..width {
            let light=(y/cell)%2==0; let color=if light {[250,249,246]} else {[24,27,32]};
            rgb[(y*width+x)*3..(y*width+x)*3+3].copy_from_slice(&color);
        }}
        for (index,shape) in SHAPES.iter().copied().enumerate() {
            let pixels=art::raster(&art::boundary(shape,0.0),shape,3,1.0);
            for background in 0..2 {
                let (left,top)=((index%columns)*cell,(index/columns*2+background)*cell);
                for y in 0..cell { for x in 0..cell {
                    let src=(y*cell+x)*4; let dest=((top+y)*width+left+x)*3;
                    let alpha=pixels[src+3] as f32/255.0;
                    for channel in 0..3 {
                        rgb[dest+channel]=(pixels[src+2-channel] as f32+rgb[dest+channel] as f32*(1.0-alpha)).round() as u8;
                    }
                }}
            }
        }
        let mut bytes=format!("P6\n{width} {height}\n255\n").into_bytes(); bytes.extend(rgb);
        std::fs::write(path,bytes).unwrap();
    }
    #[test]
    fn legacy_density_exports_keep_logical_size_and_integer_scaled_anchor() {
        for size in [32,64] {
            for density in [2,3,4] {
                let color=[12,99,202];
                let (w,h,x,y,pixels)=theme_image_at_density(CursorIcon::Default,Preferences {size,color},density);
                assert_eq!((w,h),(2*size*density,2*size*density));
                let anchor=(16.0*size as f32/des_common::input::CURSOR_BASE_SIZE as f32).round() as u32*density;
                assert_eq!((x,y),(anchor,anchor));
                assert_eq!(pixels.len(),(w*h*4) as usize);
                assert!(pixels.chunks_exact(4).any(|p|p==[202,99,12,255]));
                assert!(pixels.chunks_exact(4).all(|p|p[0]<=p[3]&&p[1]<=p[3]&&p[2]<=p[3]));
            }
        }
        assert_eq!(theme_image_at_density(CursorIcon::Default,Preferences {size:0,..Preferences::default()},0).0,32);
        assert_eq!(theme_image_at_density(CursorIcon::Default,Preferences {size:u32::MAX,..Preferences::default()},u32::MAX).0,512);
    }

    #[test]
    fn input_settings_convert_to_the_same_normalized_cursor_appearance() {
        let mut settings=des_common::input::InputSettings::default();
        assert_eq!(Preferences::from(&settings),Preferences::default());
        settings.cursor_size=40;settings.cursor_color=[19,88,201];
        assert_eq!(Preferences::from(&settings),Preferences {size:40,color:[19,88,201]});
        settings.cursor_size=u32::MAX;
        assert_eq!(Preferences::from(&settings).size,64);
    }

    #[test]
    fn configurable_size_uses_an_exact_integer_anchor_at_every_supported_size() {
        let now=Instant::now();
        let mut cursors=Cursors::new();
        for size in [16,24,32,48,64] {
            cursors.configure(Preferences {size,..Preferences::default()});
            let expected=(16.0*size as f32/24.0).round() as i32;
            for icon in [CursorIcon::Default,CursorIcon::Text,CursorIcon::Pointer,CursorIcon::Wait] {
                for scale in [1.0,1.5,4.0] {
                    assert_eq!(cursors.get(icon,scale,false,now).1,(expected,expected).into());
                }
            }
            let p=art::logical_point([16.0,16.0],size);
            assert_eq!(p,[expected as f32,expected as f32],"raster anchor must match returned hotspot exactly");
            let (w,h,x,y,pixels)=theme_image_with_preferences(CursorIcon::Default,size,Preferences::default());
            assert_eq!((w,h),(size*2,size*2)); assert_eq!((x,y),(expected as u32,expected as u32));
            assert_eq!(pixels.len(),(w*h*4) as usize);
        }
    }

    #[test]
    fn configure_invalidates_cached_pixels_without_restarting_an_active_morph() {
        let now=Instant::now();let mut cursors=Cursors::new();
        cursors.get(CursorIcon::Default,1.0,true,now);
        cursors.get(CursorIcon::Text,1.0,true,now);
        let midway=now+Duration::from_millis(60);
        let outline=cursors.current(midway,true);let started=cursors.started;
        let prefs=Preferences {size:32,color:[12,99,202]};
        cursors.configure(prefs);
        assert!(cursors.cache.is_empty());assert_eq!(cursors.started,started);
        assert_eq!(cursors.current(midway,true),outline);assert!(cursors.transitioning);
        cursors.get(CursorIcon::Text,2.0,true,midway);
        assert_eq!(cursors.cache.len(),1);
        cursors.configure(prefs);assert_eq!(cursors.cache.len(),1,"unchanged preferences retain cache");
        cursors.configure(Preferences {size:0,..prefs});assert_eq!(cursors.preferences.size,16);
        cursors.configure(Preferences {size:999,..prefs});assert_eq!(cursors.preferences.size,64);
    }

    #[test]
    fn requested_rgb_survives_in_the_solid_body_and_argb_is_premultiplied() {
        for color in [[12,99,202],[247,246,242],[0,0,0],[255,255,255],[255,0,128]] {
            for size in [16,24,32,48,64] {
                let (_,_,_,_,pixels)=theme_image_with_preferences(CursorIcon::Pointer,size,Preferences {size:24,color});
                let bgra=[color[2],color[1],color[0],255];
                assert!(pixels.chunks_exact(4).any(|p|p==bgra),"missing requested body color {color:?} at {size}");
                assert!(pixels.chunks_exact(4).all(|p|p[0]<=p[3]&&p[1]<=p[3]&&p[2]<=p[3]));
            }
        }
    }

    #[test]
    fn black_and_white_cursors_keep_contrast_on_both_backgrounds() {
        for color in [[0,0,0],[255,255,255]] {
            for icon in [CursorIcon::Default,CursorIcon::Pointer,CursorIcon::Text,CursorIcon::EwResize] {
                let (w,_,_,_,pixels)=theme_image_with_preferences(icon,24,Preferences {size:24,color});
                assert!(connected(&pixels,w as usize));
                for background in [0.0_f32,255.0] {
                    let visible=pixels.chunks_exact(4).filter(|pixel| {
                        let alpha=pixel[3] as f32/255.0;
                        let channel=pixel[0] as f32+background*(1.0-alpha);
                        (channel-background).abs()>18.0
                    }).count();
                    assert!(visible>4,"cursor lacks definition on background {background}: {icon:?} {color:?}");
                }
            }
        }
    }


}
