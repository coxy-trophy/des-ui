//! The overview: every workspace of a display side by side, each with its windows as
//! live pictures, to see everything at once and pick one. Super pressed and let go on
//! its own, Super+W, three fingers up on the touchpad, or `des-ui-ctl overview` opens
//! it; the same, Escape, or a choice closes it.
//!
//! The compositor draws the pictures, from the windows' own buffers, scaled. Text it
//! cannot draw: it tells the shell where every workspace and picture is, and the shell
//! draws the names, the selection and what was typed around them. On the main display
//! the shell's window is drawn beneath the pictures for that; on the others a plain
//! ground is. The window switcher (Alt+Tab, `switcher.rs`) shares the pictures and the
//! drawing, in a strip over the desktop instead.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{
            Id, Kind,
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::RescaleRenderElement,
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    desktop::{Space, Window, space::SpaceRenderElements},
    output::Output,
    utils::{IsAlive, Logical, Point, Rectangle, Size},
};

use des_common::ipc::{OverviewKind, OverviewLayout, OverviewThumb, OverviewWorkspace, WORKSPACES};

use crate::{motion::Timeline, state::DesComp};

/// How long the overview takes to open or close.
const OPEN: Duration = Duration::from_millis(200);
/// Super must be let go this soon, with nothing pressed meanwhile, to count as a tap.
const TAP: Duration = Duration::from_millis(400);
/// How far a picture must be dragged before it is being moved rather than clicked.
const DRAG: f64 = 6.0;

/// Room around the overview: above for what the shell writes there, at the sides and
/// below to breathe.
const TOP: i32 = 72;
const EDGE: i32 = 48;
const GAP: i32 = 24;
/// Each workspace's heading, which the shell writes at the top of its cell.
const HEADER: i32 = 40;
/// Between pictures, and under each one for its name.
const THUMB_GAP: i32 = 16;
pub const LABEL: i32 = 24;

/// The ground the shell's frame is painted in, for displays where the shell draws
/// nothing beneath the pictures.
const GROUND: [f32; 4] = [0.024, 0.031, 0.043, 1.0];
const PANEL: [f32; 4] = [0.039, 0.055, 0.071, 0.96];

type Surface = WaylandSurfaceRenderElement<GlesRenderer>;

/// The four workspaces' cells on a display of `size`, in its own coordinates: side by
/// side on a wide display, two by two on one narrower than 1.6 times its height.
pub fn cells(size: Size<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
    let (cols, rows) = if (size.w as f64) < 1.6 * size.h as f64 { (2, 2) } else { (WORKSPACES as i32, 1) };
    let width = ((size.w - 2 * EDGE - GAP * (cols - 1)) / cols).max(1);
    let height = ((size.h - TOP - EDGE - GAP * (rows - 1)) / rows).max(1);
    (0..WORKSPACES as i32)
        .map(|n| {
            let (col, row) = (n % cols, n / cols);
            Rectangle::new((EDGE + col * (width + GAP), TOP + row * (height + GAP)).into(), (width, height).into())
        })
        .collect()
}

/// Where the pictures go inside `area`, each keeping its shape (`aspects`, width over
/// height) with room for its name beneath. The number of columns is the one that
/// leaves the pictures largest together.
pub fn pack(area: Rectangle<i32, Logical>, aspects: &[f64]) -> Vec<Rectangle<i32, Logical>> {
    let n = aspects.len();
    if n == 0 {
        return Vec::new();
    }
    let aspect = |i: usize| {
        let aspect = aspects[i];
        if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 }
    };
    let fit = |slot_w: f64, slot_h: f64, aspect: f64| {
        let w = slot_w.min(slot_h * aspect).max(1.0);
        (w, (w / aspect).max(1.0))
    };
    let mut best: Option<(f64, usize)> = None;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let slot_w = (area.size.w - THUMB_GAP * (cols as i32 - 1)) as f64 / cols as f64;
        let slot_h = (area.size.h - THUMB_GAP * (rows as i32 - 1)) as f64 / rows as f64 - LABEL as f64;
        if slot_w < 1.0 || slot_h < 1.0 {
            continue;
        }
        let total: f64 = (0..n).map(|i| fit(slot_w, slot_h, aspect(i))).map(|(w, h)| w * h).sum();
        if best.is_none_or(|(area, _)| total > area) {
            best = Some((total, cols));
        }
    }
    let cols = best.map_or(n, |(_, cols)| cols);
    let rows = n.div_ceil(cols);
    let slot_w = ((area.size.w - THUMB_GAP * (cols as i32 - 1)) as f64 / cols as f64).max(1.0);
    let slot_h = ((area.size.h - THUMB_GAP * (rows as i32 - 1)) as f64 / rows as f64 - LABEL as f64).max(1.0);
    (0..n)
        .map(|i| {
            let (col, row) = ((i % cols) as f64, (i / cols) as f64);
            let (w, h) = fit(slot_w, slot_h, aspect(i));
            let x = area.loc.x as f64 + col * (slot_w + THUMB_GAP as f64) + (slot_w - w) / 2.0;
            let y = area.loc.y as f64 + row * (slot_h + LABEL as f64 + THUMB_GAP as f64) + (slot_h - h) / 2.0;
            Rectangle::new((x.round() as i32, y.round() as i32).into(), ((w.round() as i32).max(1), (h.round() as i32).max(1)).into())
        })
        .collect()
}

/// Whether a window answers what was typed: part of its title or application id,
/// regardless of case.
pub fn matches(filter: &str, title: &str, app_id: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty() || title.to_lowercase().contains(&filter) || app_id.to_lowercase().contains(&filter)
}

/// Which way an arrow key moves the selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The picture an arrow leads to from `from`: the next or previous in order for left
/// and right, the nearest one above or below for up and down, or `from` itself when
/// there is none that way.
pub fn neighbour(rects: &[Rectangle<i32, Logical>], from: usize, direction: Direction) -> usize {
    if rects.is_empty() {
        return 0;
    }
    let from = from.min(rects.len() - 1);
    match direction {
        Direction::Left => crate::switcher::step(from, rects.len(), false),
        Direction::Right => crate::switcher::step(from, rects.len(), true),
        Direction::Up | Direction::Down => {
            let centre = |rect: &Rectangle<i32, Logical>| (rect.loc.x as f64 + rect.size.w as f64 / 2.0, rect.loc.y as f64 + rect.size.h as f64 / 2.0);
            let (x, y) = centre(&rects[from]);
            rects
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != from)
                .filter_map(|(i, rect)| {
                    let (cx, cy) = centre(rect);
                    let ahead = if direction == Direction::Up { y - cy } else { cy - y };
                    (ahead > 1.0).then(|| (ahead + (cx - x).abs() * 2.0, i))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(from, |(_, i)| i)
        }
    }
}

/// Super pressed and let go with nothing else in between, quickly: a tap.
#[derive(Debug, Default)]
pub struct SuperTap {
    down: Option<Instant>,
    spoiled: bool,
}

impl SuperTap {
    pub fn press(&mut self, now: Instant) {
        if self.down.is_none() {
            self.down = Some(now);
            self.spoiled = false;
        }
    }

    /// Another key or a button while Super is down: Super was a modifier after all.
    pub fn spoil(&mut self) {
        self.spoiled = true;
    }

    /// Super let go: whether that made a tap.
    pub fn release(&mut self, now: Instant) -> bool {
        let tap = self.down.take().is_some_and(|down| !self.spoiled && now.saturating_duration_since(down) < TAP);
        self.spoiled = false;
        tap
    }
}

/// A key while the overview is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverviewKey {
    Close,
    Choose,
    Move(Direction),
    Type(char),
    Erase,
    /// Anything else: swallowed, so that nothing behind the overview hears it.
    Ignore,
}

/// A key while the switcher is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SwitchKey {
    Next,
    Previous,
    Choose,
    Cancel,
    Ignore,
}

/// A picture being dragged to another workspace.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub thumb: usize,
    pub start: Point<f64, Logical>,
    pub at: Point<f64, Logical>,
    pub moved: bool,
}

/// What is shown, in desktop coordinates.
#[derive(Debug, Default, Clone)]
pub struct Layout {
    /// The display it is on.
    pub display: Rectangle<i32, Logical>,
    pub workspaces: Vec<(usize, Rectangle<i32, Logical>, bool)>,
    pub thumbs: Vec<(Window, usize, Rectangle<i32, Logical>)>,
    pub strip: Option<Rectangle<i32, Logical>>,
}

pub struct Overview {
    pub kind: OverviewKind,
    pub screen: usize,
    pub filter: String,
    pub selected: usize,
    /// The switcher's windows, in the order they were last used when it opened.
    pub order: Vec<Window>,
    pub opened: Instant,
    pub closing: Option<Instant>,
    pub animate: bool,
    pub drag: Option<Drag>,
    pub layout: Layout,
    /// A press on a workspace's empty ground, to go there when let go.
    pub pressed_workspace: Option<usize>,
    told: Option<String>,
    ground_id: Id,
    strip_id: Id,
    commit: CommitCounter,
}

impl Overview {
    fn new(kind: OverviewKind, screen: usize, order: Vec<Window>, selected: usize, animate: bool, now: Instant) -> Overview {
        Overview {
            kind,
            screen,
            filter: String::new(),
            selected,
            order,
            opened: now,
            closing: None,
            animate,
            drag: None,
            layout: Layout::default(),
            pressed_workspace: None,
            told: None,
            ground_id: Id::new(),
            strip_id: Id::new(),
            commit: CommitCounter::default(),
        }
    }

    /// How far open, eased, from 0 (the desktop as it was) to 1.
    pub fn openness(&self, now: Instant) -> f32 {
        if !self.animate {
            return if self.closing.is_some() { 0.0 } else { 1.0 };
        }
        let opening = Timeline { started: self.opened, duration: OPEN }.progress(now);
        match self.closing {
            Some(since) => opening.min(1.0 - Timeline { started: since, duration: OPEN }.progress(now)),
            None => opening,
        }
    }

    pub fn done_closing(&self, now: Instant) -> bool {
        self.closing.is_some_and(|since| !self.animate || now >= since + OPEN)
    }

    /// The picture under `pos`, by its place in the list.
    pub fn thumb_at(&self, pos: Point<f64, Logical>) -> Option<usize> {
        self.layout.thumbs.iter().position(|(_, _, rect)| rect.to_f64().contains(pos))
    }

    pub fn workspace_at(&self, pos: Point<f64, Logical>) -> Option<usize> {
        self.layout.workspaces.iter().find(|(_, rect, _)| rect.to_f64().contains(pos)).map(|(workspace, _, _)| *workspace)
    }

    /// What the shell is told, in the display's own coordinates.
    fn describe(&self) -> OverviewLayout {
        let origin = self.layout.display.loc;
        let local = |rect: &Rectangle<i32, Logical>| (rect.loc.x - origin.x, rect.loc.y - origin.y, rect.size.w.max(1), rect.size.h.max(1));
        OverviewLayout {
            kind: self.kind,
            screen: self.screen,
            selected: (!self.layout.thumbs.is_empty()).then_some(self.selected.min(self.layout.thumbs.len().saturating_sub(1))),
            filter: self.filter.clone(),
            strip: self.layout.strip.as_ref().map(local),
            workspaces: self
                .layout
                .workspaces
                .iter()
                .map(|(workspace, rect, current)| OverviewWorkspace { workspace: *workspace, area: local(rect), current: *current })
                .collect(),
            thumbs: self.layout.thumbs.iter().map(|(_, app, rect)| OverviewThumb { app: *app, area: local(rect) }).collect(),
        }
    }
}

fn aspect_of(window: &Window) -> f64 {
    let size = window.geometry().size;
    if size.w > 0 && size.h > 0 { size.w as f64 / size.h as f64 } else { 16.0 / 9.0 }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// A rectangle part of the way from `from` to `to`.
fn between(from: Rectangle<f64, Logical>, to: Rectangle<f64, Logical>, t: f64) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (lerp(from.loc.x, to.loc.x, t), lerp(from.loc.y, to.loc.y, t)).into(),
        (lerp(from.size.w, to.size.w, t), lerp(from.size.h, to.size.h, t)).into(),
    )
}

/// Smaller about its centre.
fn shrunk(rect: Rectangle<f64, Logical>, by: f64) -> Rectangle<f64, Logical> {
    let size: Size<f64, Logical> = (rect.size.w * by, rect.size.h * by).into();
    Rectangle::new((rect.loc.x + (rect.size.w - size.w) / 2.0, rect.loc.y + (rect.size.h - size.h) / 2.0).into(), size)
}

/// The overview or switcher as something to draw on `output`, front to back: the
/// pictures, the one being dragged first; then what lies beneath them. For the
/// overview that is everything there is on the display, so the rest of the scene is
/// left out; for the switcher it is only its strip, above the desktop as it is.
pub fn elements<C>(
    renderer: &mut GlesRenderer,
    overview: &Overview,
    space: &Space<Window>,
    shell: Option<&Window>,
    output: &Output,
    now: Instant,
) -> Vec<C>
where
    C: From<SpaceRenderElements<GlesRenderer, Surface>>
        + From<RescaleRenderElement<Surface>>
        + From<crate::toast::ToastElement>
        + From<SolidColorRenderElement>,
{
    let Some(output_geo) = space.output_geometry(output) else {
        return Vec::new();
    };
    // A window split in two for testing holds two displays; elsewhere they are equal.
    if !output_geo.contains_rect(overview.layout.display) {
        return Vec::new();
    }
    let scale = output.current_scale().fractional_scale();
    let open = overview.openness(now) as f64;
    let mut elements: Vec<C> = Vec::new();
    let picture = |elements: &mut Vec<C>, renderer: &mut GlesRenderer, index: usize, offset: Point<f64, Logical>| {
        let (window, _, rect) = &overview.layout.thumbs[index];
        if !window.alive() {
            return;
        }
        let target = Rectangle::new(rect.loc.to_f64() + offset, rect.size.to_f64());
        let from = space.element_location(window).map(|loc| Rectangle::new(loc, window.geometry().size).to_f64());
        let (rect, alpha) = match from {
            Some(from) => (between(from, target, open), 1.0),
            None => (between(shrunk(target, 0.9), target, open), open as f32),
        };
        if rect.size.w >= 1.0 && rect.size.h >= 1.0 {
            elements.extend(crate::motion::window_at::<C>(renderer, window, rect, alpha, true, output_geo, scale));
        }
    };
    if let Some(drag) = overview.drag.filter(|drag| drag.moved && drag.thumb < overview.layout.thumbs.len()) {
        picture(&mut elements, renderer, drag.thumb, drag.at - drag.start);
    }
    for index in 0..overview.layout.thumbs.len() {
        if overview.drag.is_some_and(|drag| drag.moved && drag.thumb == index) {
            continue;
        }
        picture(&mut elements, renderer, index, (0.0, 0.0).into());
    }
    let physical = |rect: Rectangle<i32, Logical>| Rectangle::new(rect.loc - output_geo.loc, rect.size).to_f64().to_physical(scale).to_i32_round();
    let alpha = |colour: [f32; 4], by: f64| {
        let a = colour[3] * by as f32;
        [colour[0] * a, colour[1] * a, colour[2] * a, a]
    };
    match overview.kind {
        OverviewKind::Workspaces => match shell.filter(|_| overview.screen == 0) {
            // The shell draws the overview's ground, names and selection in its window.
            Some(shell) => {
                if let Some(loc) = space.element_location(shell) {
                    let rect = Rectangle::new(loc, shell.geometry().size).to_f64();
                    elements.extend(crate::motion::window_at::<C>(renderer, shell, rect, 1.0, false, output_geo, scale));
                }
            }
            None => elements.push(C::from(SolidColorRenderElement::new(
                overview.ground_id.clone(),
                physical(output_geo),
                overview.commit,
                GROUND,
                Kind::Unspecified,
            ))),
        },
        OverviewKind::Switcher => {
            if let Some(strip) = overview.layout.strip {
                let primary = Some(output_geo.loc);
                let shell = shell.filter(|_| overview.screen == 0);
                let lifted = crate::toast::elements(renderer, shell, Some(strip), primary, &overview.strip_id, output_geo.loc, scale);
                if lifted.is_empty() {
                    elements.push(C::from(SolidColorRenderElement::new(
                        overview.strip_id.clone(),
                        physical(strip),
                        overview.commit,
                        alpha(PANEL, open),
                        Kind::Unspecified,
                    )));
                } else {
                    elements.extend(lifted.into_iter().map(C::from));
                }
            }
        }
    }
    elements
}

impl DesComp {
    /// Which picture is open and taking input; `None` while closed or closing.
    pub fn overview_open(&self) -> Option<OverviewKind> {
        self.overview.as_ref().filter(|overview| overview.closing.is_none()).map(|overview| overview.kind)
    }

    /// Whether the overview covers the whole of `output`, so that nothing else of the
    /// desktop is to be drawn there.
    pub fn overview_covers(&self, display: Rectangle<i32, Logical>) -> bool {
        self.overview.as_ref().is_some_and(|overview| overview.kind == OverviewKind::Workspaces && overview.layout.display == display)
    }

    fn may_open_overview(&self) -> bool {
        !self.locked && !self.picking && !self.screens.is_empty()
    }

    /// Opens the overview on the display under the pointer, or closes it.
    pub fn toggle_overview(&mut self) {
        match self.overview_open() {
            Some(OverviewKind::Workspaces) => self.close_overview(),
            Some(OverviewKind::Switcher) => {}
            None => {
                if !self.may_open_overview() {
                    return;
                }
                let pointer = self.seat.get_pointer().unwrap().current_location();
                let screen = self.screen_at(pointer);
                self.begin_overview(Overview::new(OverviewKind::Workspaces, screen, Vec::new(), 0, self.motions.enabled, Instant::now()));
            }
        }
    }

    /// Opens the switcher on the display in use, with the window used before this one
    /// chosen, or the one used last of all going backwards. With `same_app`, only the
    /// windows of the application in use. Nothing opens without a window to show.
    pub fn open_switcher(&mut self, forward: bool, same_app: bool) {
        if !self.may_open_overview() || self.overview.is_some() {
            return;
        }
        let screen = self.focus_screen.min(self.screens.len() - 1);
        let in_use = self.keyboard_app().map(|index| crate::policy::window_app_id(&self.apps[index].window));
        let candidates: Vec<Window> = self
            .apps
            .iter()
            .filter(|app| app.screen == screen)
            .filter(|app| !same_app || Some(crate::policy::window_app_id(&app.window)) == in_use)
            .map(|app| app.window.clone())
            .collect();
        if candidates.is_empty() {
            return;
        }
        let order = self.recency.sort(&candidates);
        let selected = crate::switcher::first_choice(order.len(), forward);
        self.begin_overview(Overview::new(OverviewKind::Switcher, screen, order, selected, self.motions.enabled, Instant::now()));
    }

    fn begin_overview(&mut self, overview: Overview) {
        self.release_pointer_constraint();
        self.dock_keyboard = false;
        self.super_tap = SuperTap::default();
        self.overview = Some(overview);
        self.refresh_overview();
    }

    /// Closes whatever is open, easing the pictures back to their windows.
    pub fn close_overview(&mut self) {
        let Some(overview) = self.overview.as_mut() else {
            return;
        };
        if overview.closing.is_none() {
            overview.closing = Some(Instant::now());
            overview.drag = None;
            self.tell(OverviewLayout::encode(None));
        }
        self.refresh_overview();
    }

    /// Closes at once: the lock, a screenshot.
    pub fn close_overview_now(&mut self) {
        if self.overview.take().is_some_and(|overview| overview.closing.is_none()) {
            self.tell(OverviewLayout::encode(None));
        }
    }

    /// Works out what is shown again, from the windows as they are now, and tells the
    /// shell if that changed. Called before every frame while something is open.
    pub fn refresh_overview(&mut self) {
        let now = Instant::now();
        if self.overview.is_none() {
            return;
        }
        if self.locked || self.picking {
            self.close_overview_now();
            return;
        }
        if self.overview.as_ref().is_some_and(|overview| overview.done_closing(now)) {
            self.overview = None;
            return;
        }
        let overview = self.overview.as_ref().unwrap();
        let screen = overview.screen.min(self.screens.len().saturating_sub(1));
        let display = self.screens.get(screen).copied().unwrap_or_default();
        let index_of = |window: &Window| self.apps.iter().position(|app| &app.window == window);
        let mut layout = Layout { display, ..Layout::default() };
        match overview.kind {
            OverviewKind::Workspaces => {
                for (workspace, cell) in cells(display.size).into_iter().enumerate() {
                    let cell = Rectangle::new(cell.loc + display.loc, cell.size);
                    layout.workspaces.push((workspace, cell, workspace == self.workspace));
                    let windows: Vec<(usize, &Window)> = self
                        .apps
                        .iter()
                        .enumerate()
                        .filter(|(_, app)| app.screen == screen && app.workspace == workspace)
                        .filter(|(_, app)| {
                            matches(&overview.filter, &crate::policy::window_title(&app.window), &crate::policy::window_app_id(&app.window))
                        })
                        .map(|(index, app)| (index, &app.window))
                        .collect();
                    let area = Rectangle::new((cell.loc.x, cell.loc.y + HEADER).into(), (cell.size.w, (cell.size.h - HEADER).max(1)).into());
                    let aspects: Vec<f64> = windows.iter().map(|(_, window)| aspect_of(window)).collect();
                    for ((index, window), rect) in windows.into_iter().zip(pack(area, &aspects)) {
                        layout.thumbs.push((window.clone(), index, rect));
                    }
                }
            }
            OverviewKind::Switcher => {
                let windows: Vec<(usize, Window)> = overview
                    .order
                    .iter()
                    .filter(|window| window.alive())
                    .filter_map(|window| Some((index_of(window)?, window.clone())))
                    .collect();
                let aspects: Vec<f64> = windows.iter().map(|(_, window)| aspect_of(window)).collect();
                let (strip, rects) = crate::switcher::strip(display.size, &aspects);
                layout.strip = Some(Rectangle::new(strip.loc + display.loc, strip.size));
                for ((index, window), rect) in windows.into_iter().zip(rects) {
                    layout.thumbs.push((window, index, Rectangle::new(rect.loc + display.loc, rect.size)));
                }
            }
        }
        let empty_switcher = overview.kind == OverviewKind::Switcher && layout.thumbs.is_empty();
        let overview = self.overview.as_mut().unwrap();
        overview.screen = screen;
        overview.selected = overview.selected.min(layout.thumbs.len().saturating_sub(1));
        overview.layout = layout;
        if overview.closing.is_none() {
            let line = OverviewLayout::encode(Some(&overview.describe()));
            if overview.told.as_ref() != Some(&line) {
                overview.told = Some(line.clone());
                self.tell(line);
            }
        }
        if empty_switcher {
            self.close_overview_now();
        }
    }

    /// The window a picture shows, by its place in the list, as an application index.
    fn pictured(&self, thumb: usize) -> Option<usize> {
        let (window, _, _) = self.overview.as_ref()?.layout.thumbs.get(thumb)?;
        self.apps.iter().position(|app| &app.window == window)
    }

    /// Brings a pictured window forward and closes.
    fn choose(&mut self, thumb: usize) {
        if let Some(index) = self.pictured(thumb) {
            self.activate(index);
            self.arrange();
            // The overview eases back; the workspace need not slide as well.
            self.motions.settle();
        }
        self.close_overview();
    }

    pub fn overview_key(&mut self, key: OverviewKey) {
        let Some(overview) = self.overview.as_mut() else {
            return;
        };
        match key {
            OverviewKey::Close => self.close_overview(),
            OverviewKey::Choose => {
                if overview.layout.thumbs.is_empty() {
                    self.close_overview();
                } else {
                    let selected = overview.selected;
                    self.choose(selected);
                }
            }
            OverviewKey::Move(direction) => {
                let rects: Vec<Rectangle<i32, Logical>> = overview.layout.thumbs.iter().map(|(_, _, rect)| *rect).collect();
                overview.selected = neighbour(&rects, overview.selected, direction);
                self.refresh_overview();
            }
            OverviewKey::Type(c) => {
                if overview.filter.len() < 64 {
                    overview.filter.push(c);
                    overview.selected = 0;
                }
                self.refresh_overview();
            }
            OverviewKey::Erase => {
                overview.filter.pop();
                overview.selected = 0;
                self.refresh_overview();
            }
            OverviewKey::Ignore => {}
        }
    }

    pub fn switcher_key(&mut self, key: SwitchKey) {
        let Some(overview) = self.overview.as_mut() else {
            return;
        };
        let len = overview.layout.thumbs.len();
        match key {
            SwitchKey::Next => overview.selected = crate::switcher::step(overview.selected, len, true),
            SwitchKey::Previous => overview.selected = crate::switcher::step(overview.selected, len, false),
            SwitchKey::Choose => {
                let selected = overview.selected;
                self.choose(selected);
                return;
            }
            SwitchKey::Cancel => {
                self.close_overview();
                return;
            }
            SwitchKey::Ignore => return,
        }
        self.refresh_overview();
    }

    /// The pointer moved while something is open: a drag follows it, and the picture
    /// under it is the one chosen.
    pub fn overview_pointer(&mut self, pos: Point<f64, Logical>) {
        let Some(overview) = self.overview.as_mut() else {
            return;
        };
        if let Some(drag) = overview.drag.as_mut() {
            drag.at = pos;
            let moved = drag.at - drag.start;
            drag.moved |= (moved.x * moved.x + moved.y * moved.y).sqrt() > DRAG;
        }
        if let Some(thumb) = overview.thumb_at(pos) {
            if overview.selected != thumb {
                overview.selected = thumb;
                self.refresh_overview();
            }
        }
    }

    /// A button while something is open. A click on a picture chooses it; on a
    /// workspace's ground, goes to that workspace; elsewhere, closes. A picture dragged
    /// onto another workspace is sent there, and the overview stays.
    pub fn overview_button(&mut self, pos: Point<f64, Logical>, pressed: bool) {
        let Some(overview) = self.overview.as_mut() else {
            return;
        };
        if pressed {
            overview.pressed_workspace = None;
            match overview.thumb_at(pos) {
                Some(thumb) if overview.kind == OverviewKind::Workspaces => {
                    overview.drag = Some(Drag { thumb, start: pos, at: pos, moved: false });
                }
                Some(thumb) => {
                    overview.selected = thumb;
                    self.switcher_key(SwitchKey::Choose);
                }
                None => match overview.workspace_at(pos) {
                    Some(workspace) => overview.pressed_workspace = Some(workspace),
                    None => self.close_overview(),
                },
            }
            return;
        }
        let pressed_workspace = overview.pressed_workspace.take();
        match overview.drag.take() {
            Some(drag) if drag.moved => {
                let target = overview.workspace_at(pos);
                if let (Some(index), Some(workspace)) = (self.pictured(drag.thumb), target) {
                    if self.apps[index].workspace != workspace {
                        self.send_to_workspace(index, workspace);
                        self.motions.settle();
                    }
                }
                self.refresh_overview();
            }
            Some(drag) if overview.thumb_at(pos) == Some(drag.thumb) => self.choose(drag.thumb),
            Some(_) => self.refresh_overview(),
            None => {
                if let Some(workspace) = pressed_workspace.filter(|workspace| overview.workspace_at(pos) == Some(*workspace)) {
                    self.switch_workspace(workspace);
                    self.motions.settle();
                    self.close_overview();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: i32, h: i32) -> Size<i32, Logical> {
        (w, h).into()
    }

    #[test]
    fn workspaces_sit_side_by_side_on_a_wide_display_and_in_a_square_on_a_narrow_one() {
        let wide = cells(size(1920, 1080));
        assert_eq!(wide.len(), WORKSPACES);
        assert!(wide.iter().all(|cell| cell.loc.y == wide[0].loc.y), "one row");
        assert!(wide.windows(2).all(|pair| pair[0].loc.x + pair[0].size.w <= pair[1].loc.x));
        let narrow = cells(size(1280, 1024));
        assert_eq!(narrow[0].loc.y, narrow[1].loc.y);
        assert!(narrow[2].loc.y > narrow[0].loc.y, "two rows");
        assert_eq!(narrow[0].loc.x, narrow[2].loc.x);
        for cell in wide.iter().chain(narrow.iter()) {
            assert!(cell.loc.x >= EDGE && cell.loc.y >= TOP && cell.size.w > 0 && cell.size.h > 0);
        }
        // A rotated display is narrow too.
        assert!(cells(size(1080, 1920))[2].loc.y > 0);
    }

    #[test]
    fn pictures_fit_their_cell_without_overlapping_for_one_to_twelve_windows() {
        for area in [Rectangle::new((48, 112).into(), (420, 900).into()), Rectangle::new((0, 0).into(), (900, 380).into())] {
            for n in 1..=12 {
                let aspects: Vec<f64> = (0..n).map(|i| if i % 3 == 0 { 0.75 } else { 16.0 / 9.0 }).collect();
                let rects = pack(area, &aspects);
                assert_eq!(rects.len(), n);
                for (i, rect) in rects.iter().enumerate() {
                    let with_label = Rectangle::new(rect.loc, (rect.size.w, rect.size.h + LABEL).into());
                    assert!(area.contains_rect(*rect), "{n}/{i}: {rect:?} outside {area:?}");
                    assert!(with_label.loc.y + with_label.size.h <= area.loc.y + area.size.h + 1, "{n}/{i}: name falls out");
                    let shape = rect.size.w as f64 / rect.size.h as f64;
                    assert!((shape - aspects[i]).abs() < 0.1 * aspects[i] + 0.05, "{n}/{i}: shape kept");
                    for other in &rects[i + 1..] {
                        assert!(!rect.overlaps(*other), "{n}: {rect:?} and {other:?}");
                    }
                }
            }
        }
        assert!(pack(Rectangle::new((0, 0).into(), (100, 100).into()), &[]).is_empty());
        // A window without a size yet is drawn sixteen by nine.
        assert_eq!(pack(Rectangle::new((0, 0).into(), (320, 400).into()), &[f64::NAN]).len(), 1);
    }

    #[test]
    fn typing_narrows_by_title_or_application() {
        assert!(matches("", "anything", "x"));
        assert!(matches("FIRE", "Mozilla Firefox", "firefox"));
        assert!(matches("term", "~/src", "org.gnome.Terminal"));
        assert!(!matches("mail", "Mozilla Firefox", "firefox"));
    }

    #[test]
    fn arrows_lead_along_the_order_and_to_the_nearest_above_or_below() {
        let rects: Vec<Rectangle<i32, Logical>> = vec![
            Rectangle::new((0, 0).into(), (100, 100).into()),
            Rectangle::new((120, 0).into(), (100, 100).into()),
            Rectangle::new((0, 140).into(), (100, 100).into()),
            Rectangle::new((120, 140).into(), (100, 100).into()),
        ];
        assert_eq!(neighbour(&rects, 0, Direction::Right), 1);
        assert_eq!(neighbour(&rects, 0, Direction::Left), 3);
        assert_eq!(neighbour(&rects, 1, Direction::Down), 3);
        assert_eq!(neighbour(&rects, 2, Direction::Up), 0);
        assert_eq!(neighbour(&rects, 0, Direction::Up), 0, "nothing above stays put");
        assert_eq!(neighbour(&[], 3, Direction::Down), 0);
    }

    #[test]
    fn a_tap_is_super_alone_and_quick() {
        let now = Instant::now();
        let mut tap = SuperTap::default();
        tap.press(now);
        assert!(tap.release(now + Duration::from_millis(150)));
        tap.press(now);
        tap.spoil();
        assert!(!tap.release(now + Duration::from_millis(150)), "Super+key is a shortcut");
        tap.press(now);
        assert!(!tap.release(now + Duration::from_millis(600)), "held too long");
        assert!(!tap.release(now), "let go without being pressed");
        // Key repeat sends more presses; the first one counts.
        tap.press(now);
        tap.press(now + Duration::from_millis(300));
        assert!(!tap.release(now + Duration::from_millis(450)));
        // The next tap starts clean after a spoiled one.
        tap.press(now);
        tap.spoil();
        tap.release(now);
        tap.press(now);
        assert!(tap.release(now + Duration::from_millis(10)));
    }
}
