//! The rotating globe in the right column: dotted land on a wire sphere, with this
//! machine and the places it is connected to marked on it.

use crate::geo::{Place, Snapshot};
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{Pos2, Sense, Stroke, Ui, pos2, vec2};

/// Land, as ranges of 10-degree cells of longitude (0 is 180°W) for each 10-degree band
/// of latitude, from the North Pole down. Coarse on purpose: enough to read as Earth.
const LAND: [&[(u8, u8)]; 18] = [
    &[(12, 14)],
    &[(7, 10), (12, 15), (19, 20), (26, 32)],
    &[(1, 8), (10, 11), (13, 14), (16, 16), (18, 35)],
    &[(2, 2), (5, 8), (10, 11), (17, 33)],
    &[(5, 11), (17, 31), (32, 32)],
    &[(6, 10), (17, 19), (21, 31)],
    &[(7, 8), (16, 30)],
    &[(8, 9), (16, 23), (25, 25), (27, 28), (30, 30)],
    &[(10, 12), (17, 22), (27, 29)],
    &[(10, 14), (19, 21), (28, 33)],
    &[(10, 13), (19, 22), (30, 32)],
    &[(11, 13), (19, 21), (29, 33)],
    &[(11, 12), (20, 20), (29, 29), (31, 33), (35, 35)],
    &[(11, 11), (32, 32), (35, 35)],
    &[(11, 11)],
    &[(11, 12), (20, 33)],
    &[(0, 35)],
    &[(0, 35)],
];

/// How far the globe leans towards the viewer, in degrees.
const TILT: f32 = 18.0;
/// Degrees of rotation per second.
pub const SPEED: f32 = 9.0;

/// Whether the 10-degree cell containing this position is land.
pub fn is_land(lon: f32, lat: f32) -> bool {
    let row = (((90.0 - lat) / 10.0) as usize).min(17);
    let cell = ((((lon + 180.0).rem_euclid(360.0)) / 10.0) as u8).min(35);
    LAND[row].iter().any(|(from, to)| (*from..=*to).contains(&cell))
}

/// How far a link's arc rises off the surface at its middle, in globe radii.
const LIFT: f32 = 0.22;

/// The point on the unit sphere for a position, before the globe is turned.
fn sphere(lon: f32, lat: f32) -> [f32; 3] {
    let (l, p) = (lon.to_radians(), lat.to_radians());
    [p.cos() * l.sin(), p.sin(), p.cos() * l.cos()]
}

/// Turns a point with the globe and leans it towards the viewer. Returns where it
/// lands on the unit disc, or `None` when it is on the far side.
fn view(point: [f32; 3], rotation: f32, pitch: f32) -> Option<(f32, f32)> {
    let (r, t) = (rotation.to_radians(), pitch.to_radians());
    let [x, y, z] = point;
    let (x, z) = (x * r.cos() + z * r.sin(), z * r.cos() - x * r.sin());
    let depth = y * t.sin() + z * t.cos();
    (depth > 0.0).then_some((x, y * t.cos() - z * t.sin()))
}

/// Projects a position onto the unit disc for a globe turned by `rotation` degrees.
/// Returns `None` for the far side.
#[cfg(test)]
pub fn project(lon: f32, lat: f32, rotation: f32) -> Option<(f32, f32)> {
    project_view(lon, lat, rotation, TILT)
}
fn project_view(lon: f32, lat: f32, rotation: f32, pitch: f32) -> Option<(f32, f32)> {
    view(sphere(lon, lat), rotation, pitch)
}

/// Points along the shortest path between two places, rising off the surface in the
/// middle like a trajectory.
fn arc(from: &Place, to: &Place) -> Vec<[f32; 3]> {
    let (a, b) = (sphere(from.lon, from.lat), sphere(to.lon, to.lat));
    let angle = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0).acos();
    // The same place, or its exact opposite: there is no single path to draw.
    if angle < 1e-3 || (std::f32::consts::PI - angle) < 1e-3 {
        return Vec::new();
    }
    (0..=32)
        .map(|step| {
            let t = step as f32 / 32.0;
            let (wa, wb) = (((1.0 - t) * angle).sin() / angle.sin(), (t * angle).sin() / angle.sin());
            let lift = 1.0 + LIFT * (std::f32::consts::PI * t).sin();
            [0, 1, 2].map(|i| (a[i] * wa + b[i] * wb) * lift)
        })
        .collect()
}

/// Draws the globe in a square of the given size, turned by `rotation` degrees, with
/// the places in `geo` marked.
fn draw(ui: &mut Ui, theme: &Theme, size: f32, rotation: f32, pitch: f32, zoom: f32, geo: &Snapshot, selected: Option<&Place>) -> eframe::egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size), Sense::click_and_drag());
    let center = rect.center();
    // Room around the sphere for the arcs that rise off it.
    let radius = (size / 2.0 - 4.0) / (1.0 + LIFT * 0.6) * zoom;
    let painter = ui.painter_at(rect);
    let at = |(x, y): (f32, f32)| pos2(center.x + radius * x, center.y - radius * y);

    painter.circle_filled(center, radius, ui::GROUND);
    painter.circle_stroke(center, radius, Stroke::new(1.0, ui::LINE_STRONG));

    // The wire sphere: lines are broken where they pass behind the globe.
    let grid = Stroke::new(1.0, ui::LINE);
    let wire = |points: Vec<Option<Pos2>>| {
        for pair in points.windows(2) {
            if let [Some(a), Some(b)] = pair {
                painter.line_segment([*a, *b], grid);
            }
        }
    };
    for lon in (-180..180).step_by(30) {
        wire((-90..=90).step_by(6).map(|lat| project_view(lon as f32, lat as f32, rotation, pitch).map(at)).collect());
    }
    for lat in (-60..=60).step_by(30) {
        wire((-180..=180).step_by(6).map(|lon| project_view(lon as f32, lat as f32, rotation, pitch).map(at)).collect());
    }

    let land = theme.muted();
    for lat in (-875..900).step_by(50) {
        for lon in (-1775..1800).step_by(50) {
            let (lon, lat) = (lon as f32 / 10.0, lat as f32 / 10.0);
            if is_land(lon, lat) {
                if let Some(point) = project_view(lon, lat, rotation, pitch) {
                    painter.circle_filled(at(point), 1.1, land);
                }
            }
        }
    }

    // Links first, so that this machine's mark sits on top of where they meet.
    let link = Stroke::new(1.2, theme.main);
    for place in &geo.links {
        let link = if selected == Some(place) { Stroke::new(2.0, ui::TEXT) } else { link };
        if let Some(home) = &geo.home {
            let points: Vec<Option<Pos2>> = arc(home, place).into_iter().map(|p| view(p, rotation, pitch).map(at)).collect();
            for pair in points.windows(2) {
                if let [Some(a), Some(b)] = pair {
                    painter.line_segment([*a, *b], link);
                }
            }
        }
        if let Some(point) = project_view(place.lon, place.lat, rotation, pitch) {
            painter.circle_filled(at(point), 2.6, theme.main);
            if selected == Some(place) { painter.circle_stroke(at(point), 7.0, Stroke::new(1.5, ui::TEXT)); }
        }
    }
    if let Some(point) = geo.home.as_ref().and_then(|home| project_view(home.lon, home.lat, rotation, pitch)) {
        painter.circle_filled(at(point), 3.4, ui::TEXT);
        painter.circle_stroke(at(point), 6.5, Stroke::new(1.0, ui::TEXT));
    }
    response
}

/// One camera and selection shared by the panel and workspace explorer.
pub struct Explorer {
    pub expanded: bool,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    paused: bool,
    hovered: bool,
    selected: Option<Place>,
    hovered_place: Option<Place>,
    query: String,
    last_tick: Option<f64>,
    search_focus: bool,
}
impl Default for Explorer {
    fn default() -> Self {
        Self { expanded: false, yaw: 0.0, pitch: TILT, zoom: 1.0, paused: false,
            hovered: false, selected: None, hovered_place: None, query: String::new(), last_tick: None, search_focus: false }
    }
}
impl Explorer {
    pub fn tick(&mut self, now: f64, rotating: bool, home: Option<&Place>) {
        match self.last_tick.replace(now) {
            None => { self.yaw = home.map_or(0.0, |home| -home.lon).rem_euclid(360.0); }
            Some(previous) if self.animating(rotating) => {
                self.yaw = (self.yaw + (now - previous).clamp(0.0, 1.0) as f32 * SPEED).rem_euclid(360.0);
            }
            _ => {}
        }
        self.hovered = false;
    }
    pub fn animating(&self, rotating: bool) -> bool { rotating && !self.paused && !self.hovered }
    fn turn(&mut self, yaw: f32, pitch: f32) {
        self.yaw = (self.yaw + yaw).rem_euclid(360.0);
        self.pitch = (self.pitch + pitch).clamp(-85.0, 85.0);
        self.paused = true;
    }
    fn zoom_by(&mut self, amount: f32) {
        self.zoom = (self.zoom + amount).clamp(1.0, 2.5);
    }
    fn home(&mut self, home: Option<&Place>) {
        self.yaw = home.map_or(0.0, |home| -home.lon).rem_euclid(360.0);
        self.pitch = home.map_or(TILT, |home| home.lat.clamp(-85.0, 85.0));
        self.zoom = 1.0;
        self.selected = None;
        self.hovered_place = None;
        self.paused = true;
    }
    fn select(&mut self, place: &Place) {
        self.yaw = (-place.lon).rem_euclid(360.0);
        self.pitch = place.lat.clamp(-85.0, 85.0);
        self.selected = Some(place.clone());
        self.paused = true;
    }
    pub fn open(&mut self) { self.expanded = true; self.search_focus = true; }

    fn controls(&mut self, ui: &mut Ui, theme: &Theme, geo: &Snapshot, rotating: bool, expanded: bool) {
        ui.horizontal_wrapped(|ui| {
            if ui::button(ui, theme, "HOME", None, false).on_hover_text("Return to your location and reset zoom").clicked() { self.home(geo.home.as_ref()); }
            ui.add_enabled_ui(rotating, |ui| {
                if ui::button(ui, theme, if self.paused { "RESUME" } else { "PAUSE" }, None, self.paused)
                    .on_hover_text(if rotating { "Control automatic rotation" } else { "Enable animation and globe rotation in Settings to rotate automatically" }).clicked() {
                    self.paused = !self.paused;
                    if !self.paused { self.selected = None; }
                }
            });
            if !expanded && ui::button(ui, theme, "EXPAND", None, false).clicked() { self.open(); }
            if expanded {
                if ui::button(ui, theme, "ZOOM −", None, false).clicked() { self.zoom_by(-0.25); }
                if ui::button(ui, theme, "ZOOM +", None, false).clicked() { self.zoom_by(0.25); }
                if ui::button(ui, theme, "LEFT", None, false).clicked() { self.turn(-20.0, 0.0); }
                if ui::button(ui, theme, "RIGHT", None, false).clicked() { self.turn(20.0, 0.0); }
                if ui::button(ui, theme, "UP", None, false).clicked() { self.turn(0.0, 15.0); }
                if ui::button(ui, theme, "DOWN", None, false).clicked() { self.turn(0.0, -15.0); }
            }
        });
    }
    fn canvas(&mut self, ui: &mut Ui, theme: &Theme, size: f32, geo: &Snapshot) -> eframe::egui::Response {
        let response = draw(ui, theme, size, self.yaw, self.pitch, self.zoom, geo, self.selected.as_ref().or(self.hovered_place.as_ref()));
        response.widget_info(|| eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, ui.is_enabled(), "Interactive globe: drag to rotate, scroll to zoom, click a destination"));
        let center = response.rect.center();
        let radius = (size / 2.0 - 4.0) / (1.0 + LIFT * 0.6) * self.zoom;
        let pointer = response.hover_pos();
        self.hovered |= response.hovered();
        if response.dragged() {
            let delta = ui.input(|input| input.pointer.delta());
            self.turn(delta.x * 180.0 / radius, delta.y * 90.0 / radius);
            ui.ctx().request_repaint();
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom_by(scroll * 0.003);
                ui.input_mut(|input| { input.smooth_scroll_delta.y = 0.0; input.raw_scroll_delta.y = 0.0; });
            }
        }
        let nearest = pointer.and_then(|pointer| {
            geo.home.iter().chain(geo.links.iter()).filter_map(|place| {
                let (x, y) = project_view(place.lon, place.lat, self.yaw, self.pitch)?;
                let at = pos2(center.x + radius * x, center.y - radius * y);
                let distance = pointer.distance(at);
                (response.rect.contains(at) && distance <= 12.0).then_some((place, distance))
            }).min_by(|a,b| a.1.total_cmp(&b.1)).map(|(place, _)| place.clone())
        });
        self.hovered_place = nearest.clone();
        if response.clicked() {
            if let Some(place) = &nearest { self.select(place); }
        }
        let tooltip = nearest.map_or_else(|| "Drag to rotate · scroll to zoom · select a destination in the expanded view".into(),
            |place| format!("{}\n{:.2}°, {:.2}°\nClick to pin destination", place.label, place.lat, place.lon));
        response.clone().on_hover_text(tooltip).on_hover_cursor(if response.dragged() {
            eframe::egui::CursorIcon::Grabbing
        } else { eframe::egui::CursorIcon::Grab });
        response
    }
    pub fn panel(&mut self, ui: &mut Ui, theme: &Theme, size: f32, geo: &Snapshot, _rotating: bool) {
        if self.expanded {
            ui::text(ui, "Explorer open in workspace", ui::mono(11.0), ui::LABEL);
            return;
        }
        let response = self.canvas(ui, theme, size, geo);
        if response.double_clicked() { self.open(); }
        // A compact globe is an ambient instrument: interacting pauses it only
        // while the pointer/touch is there. Manual pause remains in the explorer.
        if !response.hovered() && !response.dragged() { self.paused = false; }
        response.on_hover_text("Drag to rotate · scroll to zoom · double-click to explore");
    }
    pub fn workspace(&mut self, ui: &mut Ui, theme: &Theme, geo: &Snapshot, rotating: bool, lookups: bool) {
        use eframe::egui::{ScrollArea, TextEdit};
        ui.horizontal_wrapped(|ui| {
            ui::text(ui, "WORLD VIEW", ui::display(20.0), theme.main);
            if ui::button(ui, theme, "CLOSE", Some("Esc"), false).clicked() { self.expanded = false; }
        });
        ui::text(ui, "Drag to rotate · scroll to zoom · choose a destination to focus", ui::mono(12.0), ui::LABEL);
        self.controls(ui, theme, geo, rotating, true);
        self.hovered |= ui.rect_contains_pointer(ui.max_rect());
        let search_label = ui::text(ui, "SEARCH DESTINATIONS", ui::mono(11.0), ui::LABEL);
        let search = ui.horizontal(|ui| {
            let response = ui.add_sized([(ui.available_width() - 90.0).clamp(40.0, 300.0), 28.0], TextEdit::singleline(&mut self.query)
                .font(ui::mono(12.0)).hint_text("Name of a destination")).labelled_by(search_label.id);
            if !self.query.is_empty() && ui::button(ui, theme, "CLEAR", None, false).clicked() {
                self.query.clear(); response.request_focus();
            }
            response
        }).inner;
        if self.search_focus { search.request_focus(); self.search_focus = false; }
        ScrollArea::vertical().id_salt("world-explorer").show(ui, |ui| {
            if ui.available_width() >= 680.0 {
                ui.columns(2, |columns| {
                    let size = columns[0].available_width().min(400.0);
                    self.canvas(&mut columns[0], theme, size, geo);
                    self.destinations(&mut columns[1], theme, geo, lookups);
                });
            } else {
                let size = ui.available_width().clamp(80.0, 350.0);
                self.canvas(ui, theme, size, geo);
                self.destinations(ui, theme, geo, lookups);
            }
        });
    }
    fn destinations(&mut self, ui: &mut Ui, theme: &Theme, geo: &Snapshot, lookups: bool) {
        use eframe::egui::RichText;
        ui.separator();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 110.0), eframe::egui::Layout::top_down(eframe::egui::Align::Min), |ui| {
            if let Some(place) = self.selected.clone() {
                ui::text(ui, &place.label, ui::display(16.0), ui::TEXT).on_hover_text(&place.label);
                ui::text(ui, &format!("{:.4}°, {:.4}°", place.lat, place.lon), ui::mono(12.0), ui::LABEL);
                if geo.home.as_ref() == Some(&place) {
                    ui::text(ui, if geo.home_is_set { "Your location · set by you" } else { "Your location · approximate IP location" }, ui::mono(11.0), ui::LABEL);
                } else {
                    ui::text(ui, if geo.links.contains(&place) { "Current destination · approximate IP location" } else { "No longer in current destinations" }, ui::mono(11.0), ui::LABEL);
                }
                if ui::button(ui, theme, "CLEAR SELECTION", None, false).clicked() { self.selected = None; }
            } else {
                ui::text(ui, "Select a marker or destination to inspect its location.", ui::mono(12.0), ui::LABEL);
            }
        });
        ui.separator();
        ui::text(ui, "DESTINATIONS", ui::display(13.0), theme.main);
        if !lookups { ui::text(ui, "Online location lookup is off. Set your location in Settings → Globe.", ui::mono(12.0), ui::LABEL); }
        else if geo.home.is_none() && geo.links.is_empty() { ui::text(ui, "No locations available yet. Location lookups may be pending or unavailable.", ui::mono(12.0), ui::LABEL); }
        let query = self.query.trim().to_lowercase();
        let mut shown = 0;
        for (place, home) in geo.home.iter().map(|place| (place, true)).chain(geo.links.iter().map(|place| (place, false))) {
            if !place.label.to_lowercase().contains(&query) { continue; }
            shown += 1;
            let label = format!("{}  ·  {}", place.label, if home { "Your location" } else { "Destination" });
            let selected = self.selected.as_ref() == Some(place);
            if ui.selectable_label(selected, RichText::new(label).font(ui::mono(13.0)))
                .on_hover_text(format!("{:.4}°, {:.4}°", place.lat, place.lon)).clicked() { self.select(place); }
        }
        if shown == 0 {
            ui::text(ui, if query.is_empty() { "No destinations to show." } else { "No destinations match. Clear the search to see all locations." }, ui::mono(12.0), ui::LABEL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_controls_are_bounded_and_home_resets_the_view() {
        let home = Place { lat: 5.6, lon: -0.2, label: "Accra".into() };
        let mut explorer = Explorer::default();
        explorer.turn(1080.0, 200.0);
        assert!((0.0..360.0).contains(&explorer.yaw));
        assert_eq!(explorer.pitch, 85.0);
        explorer.zoom_by(100.0);
        assert_eq!(explorer.zoom, 2.5);
        explorer.zoom_by(-100.0);
        assert_eq!(explorer.zoom, 1.0);
        explorer.home(Some(&home));
        assert!((explorer.yaw - 0.2).abs() < 0.001);
        assert_eq!(explorer.pitch, home.lat);
        assert_eq!(explorer.zoom, 1.0);
        assert!(explorer.paused);
        assert!(explorer.selected.is_none());
    }

    #[test]
    fn compact_globe_resumes_after_the_pointer_leaves() {
        use eframe::egui::{CentralPanel, Context, Event, RawInput, Rect};
        let ctx = Context::default();
        ui::install_fonts(&ctx, None, None);
        let mut explorer = Explorer::default();
        let theme = Theme::load("signal");
        explorer.tick(1.0, true, None);
        explorer.turn(20.0, 0.0);
        assert!(explorer.paused);
        let _ = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))), events: vec![Event::PointerGone], ..Default::default() }, |ctx| {
            CentralPanel::default().show(ctx, |ui| explorer.panel(ui, &theme, 180.0, &Snapshot::default(), true));
        });
        assert!(!explorer.paused);
        let before = explorer.yaw;
        explorer.tick(2.0, true, None);
        assert_ne!(before, explorer.yaw);
    }

    #[test]
    fn browsing_pauses_motion_and_selection_survives_disconnection() {
        let mut explorer = Explorer::default();
        explorer.tick(1.0, true, None);
        explorer.tick(2.0, true, None);
        let moving = explorer.yaw;
        assert!(moving > 0.0);
        explorer.hovered = true;
        explorer.tick(3.0, true, None);
        assert_eq!(explorer.yaw, moving);
        explorer.hovered = false;
        explorer.tick(4.0, false, None);
        assert_eq!(explorer.yaw, moving);
        let target = Place { lat: 35.0, lon: 139.0, label: "Tokyo".into() };
        explorer.select(&target);
        assert_eq!(explorer.selected, Some(target.clone()));
        assert!(explorer.paused);
        assert!(project_view(target.lon, target.lat, explorer.yaw, explorer.pitch).is_some());
        explorer.tick(5.0, true, None);
        assert_eq!(explorer.selected, Some(target));
        assert_eq!(explorer.pitch, 35.0);
    }

    #[test]
    fn pointer_selection_and_zoom_only_affect_the_hovered_globe() {
        use eframe::egui::{CentralPanel, Context, Event, Modifiers, MouseWheelUnit, PointerButton, RawInput, Rect};
        let ctx = Context::default();
        ui::install_fonts(&ctx, None, None);
        let mut explorer = Explorer::default();
        let home = Place { lat: TILT, lon: 0.0, label: "Home".into() };
        let geo = Snapshot { home: Some(home.clone()), ..Default::default() };
        let theme = Theme::parse(include_str!("../../themes/signal.json")).unwrap();
        let frame = |events: Vec<Event>, explorer: &mut Explorer| {
            let mut rect = Rect::NOTHING;
            let _ = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))), events, ..Default::default() }, |ctx| {
                CentralPanel::default().show(ctx, |ui| { rect = explorer.canvas(ui, &theme, 220.0, &geo).rect; });
            });
            rect
        };
        let rect = frame(Vec::new(), &mut explorer);
        let point = rect.center();
        frame(vec![Event::PointerMoved(point), Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }], &mut explorer);
        frame(vec![Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }], &mut explorer);
        assert_eq!(explorer.selected, Some(home));
        let before = explorer.zoom;
        frame(vec![Event::PointerMoved(pos2(390.0, 450.0)), Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, 100.0), modifiers: Modifiers::NONE }], &mut explorer);
        assert_eq!(explorer.zoom, before, "scrolling another panel must not zoom the globe");
        frame(vec![Event::PointerMoved(point), Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, 100.0), modifiers: Modifiers::NONE }], &mut explorer);
        assert!(explorer.zoom > before);
        let orientation = (explorer.yaw, explorer.pitch);
        frame(vec![Event::PointerButton { pos: point, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }], &mut explorer);
        let dragged = point + vec2(30.0, 20.0);
        frame(vec![Event::PointerMoved(dragged)], &mut explorer);
        frame(vec![Event::PointerButton { pos: dragged, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }], &mut explorer);
        assert_ne!((explorer.yaw, explorer.pitch), orientation, "dragging must turn the camera");
        assert!(explorer.paused);
    }

    #[test]
    fn explorer_renders_empty_filtered_and_long_destinations_at_narrow_widths() {
        use eframe::egui::{CentralPanel, Context, RawInput, Rect};
        let ctx = Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::parse(include_str!("../../themes/signal.json")).unwrap();
        for width in [240.0, 800.0] {
            for lookups in [false, true] {
                let mut explorer = Explorer::default();
                explorer.open();
                explorer.query.clear();
                let geo = Snapshot { links: vec![Place { lat: -20.0, lon: 130.0, label: "Long destination ".repeat(12) }], ..Default::default() };
                let _ = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 640.0))), ..Default::default() }, |ctx| {
                    CentralPanel::default().show(ctx, |ui| {
                        explorer.workspace(ui, &theme, &geo, false, lookups);
                        assert!(ui.min_rect().width() <= ui.max_rect().width() + 1.0, "explorer overflowed its narrow viewport");
                    });
                });
                explorer.query = "absent".into();
                let _ = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 640.0))), ..Default::default() }, |ctx| {
                    CentralPanel::default().show(ctx, |ui| explorer.workspace(ui, &theme, &Snapshot::default(), false, lookups));
                });
                assert!(!explorer.animating(false));
            }
        }
    }

    #[test]
    fn opening_the_explorer_focuses_search_and_accepts_typing() {
        use eframe::egui::{CentralPanel, Context, Event, RawInput};
        let ctx = Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::parse(include_str!("../../themes/signal.json")).unwrap();
        let mut explorer = Explorer::default();
        explorer.open();
        for events in [Vec::new(), vec![Event::Text("Tokyo".into())]] {
            let _ = ctx.run(RawInput { events, ..Default::default() }, |ctx| {
                CentralPanel::default().show(ctx, |ui| explorer.workspace(ui, &theme, &Snapshot::default(), false, false));
            });
        }
        assert_eq!(explorer.query, "Tokyo");
        assert!(explorer.expanded);
    }

    #[test]
    fn knows_land_from_sea() {
        assert!(is_land(10.0, 50.0), "central Europe");
        assert!(is_land(-100.0, 40.0), "North America");
        assert!(is_land(135.0, -25.0), "Australia");
        assert!(is_land(0.0, -85.0), "Antarctica");
        assert!(!is_land(-150.0, 0.0), "the Pacific");
        assert!(!is_land(-30.0, 30.0), "the Atlantic");
        assert!(!is_land(80.0, -30.0), "the Indian Ocean");
    }

    #[test]
    fn arcs_join_their_ends_and_rise_between() {
        let place = |lat, lon| Place { lat, lon, label: String::new() };
        let (from, to) = (place(0.0, 0.0), place(0.0, 90.0));
        let points = arc(&from, &to);
        let length = |p: &[f32; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((length(&points[0]) - 1.0).abs() < 1e-4);
        assert!((length(points.last().unwrap()) - 1.0).abs() < 1e-4);
        assert!((length(&points[16]) - (1.0 + LIFT)).abs() < 1e-3, "highest in the middle");
        assert!(arc(&from, &from).is_empty());
        assert!(arc(&from, &place(0.0, 180.0)).is_empty());
    }

    #[test]
    fn projects_the_near_side_only() {
        let (x, y) = project(0.0, TILT, 0.0).unwrap();
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5, "the centre faces the viewer");
        assert!(project(180.0, 0.0, 0.0).is_none());
        assert!(project(0.0, 0.0, 180.0).is_none());
        let (x, _) = project(60.0, 0.0, 0.0).unwrap();
        assert!(x > 0.0 && x <= 1.0);
    }
}
