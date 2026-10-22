//! Desktop policy: which window is shown where.
//!
//! The shell's frame fills the main display, and one application at a time covers the
//! workspace inside it. Every further display shows one application at full size, on
//! top of a backdrop window that the shell provides for it.

use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    input::pointer::MotionEvent,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{compositor::with_states, seat::WaylandFocus, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{DesComp, focus::Focus};
use std::time::Instant;
use des_common::{
    Panel,
    ipc::{AppInfo, Command, DesktopState, Event, LayoutInfo, LayoutKind, Place},
};
use crate::layout::{self, Ratios};

/// An application window and its place on the desktop.
pub struct App {
    pub window: Window,
    /// Index into `DesComp::screens` of the display it lives on.
    pub screen: usize,
    /// Where it is on that display. At most one application is in each visible place.
    /// Hidden while its workspace is not the one in view.
    pub place: Place,
    /// The workspace it belongs to.
    pub workspace: usize,
    /// Where it was when its workspace was last in view, to go back to.
    pub kept: Place,
    /// The application it opened on top of, which comes back when this one closes:
    /// a dialog gives the window that asked for it its place back.
    pub over: Option<Window>,
    /// Kept in a corner above everything on its display, on every workspace.
    pub pinned: bool,
    /// Where it goes back to when no longer pinned, and the shape it keeps meanwhile.
    pub pin_return: Place,
    pub pin_aspect: f64,
    /// Whether the window rules have been looked at for it; they are, once.
    pub ruled: bool,
    /// It asked to be brought forward and could not be; cleared once it is.
    pub urgent: bool,
    /// When it last had the keyboard, by `DesComp::use_clock`: higher is more recent.
    pub used: u64,
}

/// How far the pinned application keeps from the edges of its workspace.
const PIP_MARGIN: i32 = 16;

/// Where a pinned application goes: a quarter of the workspace wide, at most half
/// of it high, its shape kept, in the bottom right corner, clear of the strip along
/// the bottom edge that reveals the dock. `aspect` is width over height; a window
/// without a size yet is taken to be sixteen by nine.
pub fn pip_rect(area: Rectangle<i32, Logical>, aspect: f64, margin: i32, below: i32) -> Rectangle<i32, Logical> {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    let mut width = (area.size.w / 4).max(1) as f64;
    let mut height = width / aspect;
    let most = (area.size.h / 2).max(1) as f64;
    if height > most {
        height = most;
        width = height * aspect;
    }
    let (w, h) = ((width.round() as i32).max(1), (height.round() as i32).max(1));
    let x = (area.loc.x + area.size.w - w - margin).max(area.loc.x);
    let y = (area.loc.y + area.size.h - h - margin - below).max(area.loc.y);
    Rectangle::new((x, y).into(), (w, h).into())
}

/// How a display's workspace is being used.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScreenMode {
    /// Applications cover the whole display instead of the workspace inside the frame.
    pub fullscreen: bool,
    /// The slot whose application has the keyboard; 0 is the master, or the terminal
    /// beside an application on the main display.
    pub focus_slot: usize,
    /// How the applications in view share the workspace.
    pub layout: LayoutKind,
    /// Where its dividers are, kept per layout.
    pub ratios: Ratios,
}

fn is_toplevel(window: &Window, surface: &WlSurface) -> bool {
    window.wl_surface().as_deref() == Some(surface)
}

/// What to give the keyboard to for it to reach `window`.
pub fn focus_of(window: &Window) -> Option<Focus> {
    match window.x11_surface() {
        Some(x11) => Some(Focus::X11(x11.clone())),
        None => window.wl_surface().map(|surface| Focus::Surface(surface.into_owned())),
    }
}

fn occupies_restored_slot(active:usize,workspace:usize,live:Place,kept:Place,target:Place)->bool{
    (if workspace==active{live}else{kept})==target
}
fn dismisses_dock_keyboard(keyboard_screen:usize,cleared_screen:usize,area_none:bool)->bool{
    area_none && keyboard_screen==cleared_screen
}
/// What the shell is told of one display's layout on one workspace: nothing for the
/// plain one, and a split for a workspace shared some other way than by choosing it.
fn layout_info(workspace: usize, screen: usize, mode: &ScreenMode, split_anyway: bool) -> Option<LayoutInfo> {
    let kind = if split_anyway { LayoutKind::Split } else { mode.layout };
    if kind == LayoutKind::Single && mode.ratios == Ratios::default() {
        return None;
    }
    Some(LayoutInfo { workspace, screen, kind, ratio: mode.ratios.primary(kind), ratio2: mode.ratios.secondary(kind) })
}

fn command_allowed(locked: bool, picking:bool, command: &Command) -> bool {
    (!locked || matches!(command, Command::Lock | Command::Unlock | Command::ReloadInput)) && (!picking || !matches!(command,Command::Dock(Some(_))|Command::DockOn(_,Some(_))|Command::Overlay(Some(_))|Command::DragFiles(_)|Command::Overview|Command::AppWindows(..)))
}

#[cfg(test)]
mod lock_command_tests {
    use super::*;
    #[test]
    fn docks_cannot_override_lock_or_screenshot_and_clears_are_screen_local(){
        assert!(!command_allowed(true,false,&Command::DockOn(1,None)));
        assert!(!command_allowed(false,true,&Command::DockOn(1,Some((1,2,3,4)))));
        assert!(command_allowed(false,true,&Command::Dock(None)));
        assert!(command_allowed(false,true,&Command::DockOn(1,None)));
        assert!(command_allowed(false,true,&Command::Toast(None)));
        assert!(!dismisses_dock_keyboard(1,0,true));
        assert!(dismisses_dock_keyboard(1,1,true));
        assert!(!dismisses_dock_keyboard(1,1,false));
    }
    #[test]
    fn restore_collisions_use_live_slots_only_on_active_workspace(){
        assert!(!occupies_restored_slot(0,0,Place::Main,Place::Side,Place::Side));
        assert!(occupies_restored_slot(0,0,Place::Main,Place::Side,Place::Main));
        assert!(occupies_restored_slot(0,1,Place::Hidden,Place::Side,Place::Side));
    }
    #[test]
    fn only_layouts_other_than_the_plain_one_are_reported() {
        let plain = ScreenMode::default();
        assert_eq!(layout_info(0, 0, &plain, false), None);
        let split = layout_info(1, 0, &plain, true).unwrap();
        assert_eq!((split.kind, split.ratio), (LayoutKind::Split, 0.5));
        let mut columns = ScreenMode { layout: LayoutKind::Columns, ..ScreenMode::default() };
        columns.ratios.restore(LayoutKind::Columns, 0.25, 0.55);
        let info = layout_info(2, 1, &columns, false).unwrap();
        assert_eq!((info.workspace, info.screen, info.ratio, info.ratio2), (2, 1, 0.25, 0.55));
        // Layouts are refused behind the lock like every other window action.
        assert!(!command_allowed(true, false, &Command::Layout(LayoutKind::Master)));
        assert!(!command_allowed(true, false, &Command::Ratio(0.3)));
    }

    #[test]
    fn the_overview_never_opens_behind_the_lock_or_over_a_screenshot() {
        assert!(command_allowed(false, false, &Command::Overview));
        assert!(!command_allowed(true, false, &Command::Overview));
        assert!(!command_allowed(false, true, &Command::Overview));
    }
    #[test]
    fn already_locked_requests_can_receive_their_confirmation() {
        assert!(command_allowed(true, false, &Command::Lock));
        assert!(command_allowed(true, false, &Command::Unlock));
        assert!(!command_allowed(true, false, &Command::Activate(0)));
        assert!(!command_allowed(true, false, &Command::Dock(Some((0, 0, 100, 100)))));
        assert!(!command_allowed(true, false, &Command::Dock(None)));
    }
}

impl DesComp {
    /// Tells the policy which displays exist, the main one first. Called by the backend
    /// at startup and whenever a display is added, removed or resized.
    pub fn set_screens(&mut self, screens: Vec<Rectangle<i32, Logical>>) {
        if self.screens != screens && self.drag_session.is_some() {
            self.cancel_file_drag();
        }
        if screens.is_empty() {
            return;
        }
        // Applications on a display that went away come home, out of view.
        for app in &mut self.apps {
            if app.screen >= screens.len() {
                app.screen = 0;
                app.place = Place::Hidden;
            }
        }
        self.docks.retain(|screen,_|*screen<screens.len());
        self.dock_keyboard_screen=self.dock_keyboard_screen.min(screens.len()-1);
        self.focus_screen = self.focus_screen.min(screens.len() - 1);
        if self.screens != screens {
            tracing::info!(?screens, "display layout");
        }
        let first = self.screens.is_empty();
        self.modes.resize(screens.len(), ScreenMode::default());
        self.screens = screens;
        self.arrange();

        if first {
            // Start with the pointer in the middle of the main display.
            let main = self.screens[0].to_f64();
            let location = main.loc + Point::from((main.size.w / 2.0, main.size.h / 2.0));
            let pointer = self.seat.get_pointer().unwrap();
            let under = self.surface_under(location);
            pointer.motion(
                self,
                under,
                &MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 0,
                },
            );
            pointer.frame(self);
        }
    }

    /// The display under `pos`, or the main one if `pos` is on none.
    pub fn screen_at(&self, pos: Point<f64, Logical>) -> usize {
        self.screens
            .iter()
            .position(|rect| rect.to_f64().contains(pos))
            .unwrap_or(0)
    }

    /// Where a pointer at `from` ends up when asked to move to `to`: it may cross onto
    /// another display, but never leave them all.
    pub fn clamp_to_screens(&self, from: Point<f64, Logical>, to: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.screens.iter().any(|rect| rect.to_f64().contains(to)) {
            return to;
        }
        let Some(rect) = self.screens.get(self.screen_at(from)) else {
            return to;
        };
        let rect = rect.to_f64();
        Point::from((
            to.x.clamp(rect.loc.x, rect.loc.x + rect.size.w - 1.0),
            to.y.clamp(rect.loc.y, rect.loc.y + rect.size.h - 1.0),
        ))
    }

    /// Finds the shell, backdrop or application window whose toplevel is `surface`.
    ///
    /// Hidden applications are not in the `Space`, so lookups must go through here.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.shell
            .iter()
            .chain(self.backdrops.iter())
            .chain(self.apps.iter().map(|app| &app.window))
            .find(|window| is_toplevel(window, surface))
            .cloned()
    }

    /// The display a window is on.
    pub fn screen_of(&self, window: &Window) -> usize {
        if let Some(app) = self.apps.iter().find(|app| &app.window == window) {
            return app.screen;
        }
        self.backdrops
            .iter()
            .position(|backdrop| backdrop == window)
            .map_or(0, |index| index + 1)
    }

    /// The application in a slot of a display's workspace.
    fn in_place(&self, screen: usize, place: Place) -> Option<usize> {
        self.apps
            .iter()
            .position(|app| app.screen == screen && app.place == place)
    }

    fn mode(&self, screen: usize) -> ScreenMode {
        self.modes.get(screen).copied().unwrap_or_default()
    }

    /// The application that has the keyboard on a display: the one in the half last
    /// used, or the only one there is.
    pub fn focused_on(&self, screen: usize) -> Option<usize> {
        let slot = self.mode(screen).focus_slot;
        self.in_place(screen, Place::of_slot(slot)).filter(|_| slot > 0).or_else(|| self.in_place(screen, Place::Main))
    }

    /// How many slots of a display are in use: one past the last occupied. The first
    /// counts even when empty, for the terminal or the backdrop is there.
    fn slots_in_use(&self, screen: usize) -> usize {
        self.apps
            .iter()
            .filter(|app| app.screen == screen && !app.pinned)
            .filter_map(|app| app.place.slot())
            .map(|slot| slot + 1)
            .max()
            .unwrap_or(1)
    }

    /// The layout a display is drawn with: the chosen one, unless more applications
    /// are in view than it has room for (put there by a rule or a restored session),
    /// when the master and its stack, which has room for all.
    fn shown_layout(&self, screen: usize) -> LayoutKind {
        let mode = self.mode(screen);
        let count = self.slots_in_use(screen);
        let kind = if count > mode.layout.slots().max(2) { LayoutKind::Master } else { mode.layout };
        layout::effective(kind, count)
    }

    /// Where each slot in use of a display's workspace is, in slot order.
    fn slot_areas(&self, screen: usize) -> Vec<Rectangle<i32, Logical>> {
        let mode = self.mode(screen);
        layout::slot_rects(self.shown_layout(screen), &mode.ratios, self.workspace_of(screen), self.slots_in_use(screen))
    }

    /// Moves a display's applications down into any slot left empty, so that a closed
    /// window leaves no hole. The first slot is left alone: when it is empty the
    /// terminal or the backdrop is there on purpose.
    fn close_gaps(&mut self, screen: usize) {
        let mut taken: Vec<(usize, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter(|(_, app)| app.screen == screen && !app.pinned)
            .filter_map(|(i, app)| app.place.slot().filter(|slot| *slot > 0).map(|slot| (slot, i)))
            .collect();
        taken.sort();
        let focus = self.modes.get(screen).map_or(0, |mode| mode.focus_slot);
        let mut new_focus = focus;
        for (n, (slot, index)) in taken.into_iter().enumerate() {
            let to = n + 1;
            if slot != to {
                self.apps[index].place = Place::of_slot(to);
                if focus == slot {
                    new_focus = to;
                }
            }
        }
        if let Some(mode) = self.modes.get_mut(screen) {
            mode.focus_slot = new_focus;
        }
    }

    /// Puts the application at `index` in a slot of its display, in place of whatever
    /// was there.
    fn place(&mut self, index: usize, place: Place) {
        let Some(screen) = self.apps.get(index).map(|app| app.screen) else {
            return;
        };
        for (i, app) in self.apps.iter_mut().enumerate() {
            if i == index {
                app.place = place;
            } else if app.screen == screen && app.place == place {
                app.place = Place::Hidden;
            }
        }
    }

    /// Brings the application at `index` into view and gives it the keyboard.
    pub(crate) fn activate(&mut self, index: usize) {
        self.dock_keyboard = false;
        if let Some(app) = self.apps.get_mut(index) {
            app.urgent = false;
        }
        if self.apps.get(index).is_some_and(|app| app.pinned) {
            self.pin_focused = true;
            self.focus_screen = self.apps[index].screen;
            return;
        }
        self.pin_focused = false;
        let Some(on) = self.apps.get(index).map(|app| app.workspace) else {
            return;
        };
        // An application on another workspace brings that workspace into view.
        self.switch_workspace(on);
        let app = &self.apps[index];
        let screen = app.screen;
        // An application already in view stays in its slot.
        let slot = match app.place.slot() {
            Some(slot) => slot,
            None => {
                self.place(index, Place::Main);
                0
            }
        };
        if let Some(mode) = self.modes.get_mut(screen) {
            mode.focus_slot = slot;
        }
        self.focus_screen = screen;
    }

    /// The area applications occupy on a display: the workspace inside the frame on the
    /// main display, everything on the others, and everything anywhere in full screen.
    /// Bars and other layer surfaces that reserve an edge keep applications clear of it.
    pub(crate) fn workspace_of(&self, screen: usize) -> Rectangle<i32, Logical> {
        let rect = self.screens.get(screen).or(self.screens.first()).copied().unwrap_or_default();
        // The terminal lives inside the frame, so full screen needs an application in
        // the main slot: beside the terminal, an application stays in the workspace.
        let covered = self.in_place(screen, Place::Main).is_some();
        let area = if screen != 0 || (self.mode(screen).fullscreen && covered) {
            rect
        } else {
            to_rectangle(
                des_common::workspace_with_behavior(rect.size.w as f32, rect.size.h as f32, self.panels, self.side_panel_behavior),
                rect.loc,
            )
        };
        self.clear_of_layers(screen, area)
    }

    /// Applies the layout to every window and gives the keyboard to the right one: the
    /// application in use on the display in use, or else the shell's terminal.
    pub fn arrange(&mut self) {
        let Some(primary) = self.screens.first().copied() else {
            return;
        };
        // No holes between the windows in view, and the keyboard on a slot in use.
        for screen in 0..self.modes.len() {
            self.close_gaps(screen);
            let slot = self.modes[screen].focus_slot;
            if slot > 0 && self.in_place(screen, Place::of_slot(slot)).is_none() {
                self.modes[screen].focus_slot = 0;
            }
        }
        let focus_app = self.keyboard_app().filter(|_| !self.locked);
        // The application with the keyboard is the one used most recently.
        if let Some(i) = focus_app {
            if self.apps[i].used != self.use_clock || self.use_clock == 0 {
                self.use_clock += 1;
                self.apps[i].used = self.use_clock;
            }
        }
        // Nothing moves behind the lock or while the display stands still for a
        // screenshot: whatever was on its way arrives at once.
        let now = Instant::now();
        let animate = crate::motion::allowed(self.motions.enabled, self.locked, self.picking);
        if !animate {
            self.motions.settle();
        }

        if let Some(shell) = self.shell.clone() {
            configure(&shell, primary, true, focus_app.is_none());
            self.space.map_element(shell, primary.loc, false);
        }
        for (i, backdrop) in self.backdrops.clone().into_iter().enumerate() {
            if self.locked {self.space.unmap_elem(&backdrop);continue;}
            match self.screens.get(i + 1).copied() {
                Some(rect) => {
                    configure(&backdrop, rect, true, false);
                    self.space.map_element(backdrop, rect.loc, false);
                }
                None => self.space.unmap_elem(&backdrop),
            }
        }
        let areas: Vec<Vec<Rectangle<i32, Logical>>> = (0..self.screens.len()).map(|screen| self.slot_areas(screen)).collect();
        // While a divider is dragged, the windows on its display are told their sizes
        // only now and then, and drawn scaled into place in between.
        let held = self.resize.filter(|drag| drag.last_configure.elapsed() < layout::RECONFIGURE_EVERY).map(|drag| drag.screen);
        if let Some(drag) = self.resize.as_mut().filter(|drag| Some(drag.screen) != held) {
            drag.last_configure = now;
        }
        for i in 0..self.apps.len() {
            // The pinned application is placed after the others, above them.
            if self.apps[i].pinned {
                continue;
            }
            let (window, screen, place) = {
                let app = &self.apps[i];
                // Locked, nothing but the shell is shown, wherever it was.
                let place = if self.locked { Place::Hidden } else { app.place };
                (app.window.clone(), app.screen, place)
            };
            let slots = areas.get(screen).map(Vec::as_slice).unwrap_or(&[]);
            let whole = self.workspace_of(screen);
            let area = place.slot().and_then(|slot| slots.get(slot)).or(slots.first()).copied().unwrap_or(whole);
            // An application alone on a display in full screen is told so, which lets
            // it drop its own toolbars; sharing it is merely as large as it can be.
            let alone = self.mode(screen).fullscreen && slots.len() <= 1 && area == self.screens.get(screen).copied().unwrap_or_default();
            if held != Some(screen) || place == Place::Hidden {
                configure(&window, area, alone, focus_app == Some(i));
            }
            if place == Place::Hidden {
                self.motions.hide(&window, now, animate);
                self.space.unmap_elem(&window);
            } else {
                self.motions.show(window.clone(), area, now, animate);
                // Mapping also raises, which puts the application above what is behind it.
                self.space.map_element(window, area.loc, false);
            }
        }

        if let Some(i) = self.pinned_app() {
            let window = self.apps[i].window.clone();
            if self.locked {
                self.motions.hide(&window, now, animate);
                self.space.unmap_elem(&window);
            } else {
                let rect = self.pin_rect(i);
                configure(&window, rect, false, focus_app == Some(i));
                self.motions.show(window.clone(), rect, now, animate);
                self.space.map_element(window, rect.loc, false);
            }
        }

        // X11 menus stay above the windows that were just raised.
        for popup in &self.x_popups {
            if self.locked {
                self.space.unmap_elem(popup);
            } else {
                self.space.raise_element(popup, false);
            }
        }

        // The dock and the shell's overlay take the keyboard while they are up; a
        // launcher or the like in a layer surface takes it while it asks to.
        let focus = self.layer_keyboard().filter(|_| !self.dock_keyboard && self.overlay_area().is_none()).or_else(|| focus_app.filter(|_| !self.dock_keyboard && self.overlay_area().is_none())
            .and_then(|i|focus_of(&self.apps[i].window))
            .or_else(||self.shell_on(if self.dock_keyboard{self.dock_keyboard_screen}else{0}).and_then(focus_of)));
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, focus, SERIAL_COUNTER.next_serial());
    }

    /// Registers a new toplevel: the shell's first window is the frame, its later ones
    /// are backdrops for further displays, and anything else is an application.
    pub fn add_window(&mut self, window: Window, from_shell: bool) {
        if from_shell && self.shell.is_none() {
            tracing::info!("shell window connected");
            self.shell = Some(window);
            if let Some(splash) = &mut self.splash {
                splash.shell_ready();
            }
        } else if from_shell {
            tracing::info!(count = self.backdrops.len() + 1, "shell backdrop connected");
            self.backdrops.push(window);
        } else {
            // A launched application owns the keyboard once its window arrives.
            self.dock_keyboard = false;
            // Applications open on the display the pointer is on, in the half in use.
            let pointer = self.seat.get_pointer().unwrap().current_location();
            let screen = self.screen_at(pointer);
            tracing::info!(screen, "application window opened");
            // A new window takes the place of the one in use, in whichever slot that is.
            let slot = self.mode(screen).focus_slot;
            let place = if slot > 0 && self.in_place(screen, Place::of_slot(slot)).is_some() { Place::of_slot(slot) } else { Place::Main };
            let over = self.in_place(screen, place).map(|index| self.apps[index].window.clone());
            self.apps.push(App {
                window,
                screen,
                place: Place::Hidden,
                workspace: self.workspace,
                kept: Place::Hidden,
                over,
                pinned: false,
                pin_return: Place::Hidden,
                pin_aspect: 0.0,
                ruled: false,
                urgent: false,
                used: 0,
            });
            self.place(self.apps.len() - 1, place);
            self.focus_screen = screen;
            // An X11 window says what it is from the start; a Wayland one usually
            // does in its first commit, where the rules are looked at instead.
            let window = self.apps[self.apps.len() - 1].window.clone();
            if !window_app_id(&window).is_empty() {
                self.apply_rules(&window);
            }
        }
        self.arrange();
    }

    /// Forgets the window whose toplevel is `surface`.
    pub fn remove_window(&mut self, surface: &WlSurface) {
        if self.shell.as_ref().is_some_and(|w| is_toplevel(w, surface)) {
            // Whether this ends the session is decided when the shell's process exits.
            tracing::info!("shell window closed");
            self.toast = None;
            self.overlay = None;
            self.dock = None;
            self.docks.clear();
            self.dock_keyboard = false;
            if let Some(window) = self.shell.take() {
                self.space.unmap_elem(&window);
            }
        } else if let Some(index) = self.backdrops.iter().position(|w| is_toplevel(w, surface)) {
            let window = self.backdrops.remove(index);
            self.space.unmap_elem(&window);
        } else if let Some(index) = self.apps.iter().position(|app| is_toplevel(&app.window, surface)) {
            self.forget_app(index);
            tracing::info!(remaining = self.apps.len(), "application window closed");
        }
        self.arrange();
    }

    /// Takes a closed application out of the desktop. The one it opened on top of
    /// takes its place back, if that is still open and out of view; otherwise what
    /// is behind shows, which is the terminal or the backdrop.
    pub fn forget_app(&mut self, index: usize) {
        let app = self.apps.remove(index);
        self.space.unmap_elem(&app.window);
        self.motions.forget(&app.window);
        self.recency.forget(&app.window);
        // Whatever opened on top of this one is now on top of what this one was on.
        for other in self.apps.iter_mut().filter(|other| other.over.as_ref() == Some(&app.window)) {
            other.over = app.over.clone();
        }
        let Some(under) = app.over.as_ref().and_then(|over| self.apps.iter().position(|other| &other.window == over)) else {
            return;
        };
        if self.apps[under].screen != app.screen || self.apps[under].workspace != app.workspace {
            return;
        }
        if app.workspace == self.workspace {
            if app.place != Place::Hidden && self.apps[under].place == Place::Hidden && self.in_place(app.screen, app.place).is_none() {
                self.apps[under].place = app.place;
            }
        } else if app.kept != Place::Hidden && self.apps[under].kept == Place::Hidden {
            // On a workspace out of view, it is there when that one is come back to.
            let taken = self.apps.iter().any(|other| other.workspace == app.workspace && other.screen == app.screen && other.kept == app.kept);
            if !taken {
                self.apps[under].kept = app.kept;
            }
        }
    }

    /// Switches the slot in use to the next application on its display. The main slot
    /// passes through the terminal (or the backdrop) after the last one.
    pub fn cycle(&mut self) {
        let screen = self.focus_screen;
        let slot = match self.focused_on(screen) {
            Some(index) => self.apps[index].place,
            None => Place::Main,
        };
        // Applications in the other slot are not candidates.
        let here: Vec<usize> = (0..self.apps.len())
            .filter(|i| self.apps[*i].workspace == self.workspace && !self.apps[*i].pinned)
            .filter(|i| self.apps[*i].screen == screen && [slot, Place::Hidden].contains(&self.apps[*i].place))
            .collect();
        let current = here.iter().position(|i| self.apps[*i].place == slot);
        let next = match current {
            None => here.first().copied(),
            Some(position) => here.get(position + 1).copied(),
        };
        match next {
            Some(index) => self.place(index, slot),
            // Past the last one: the terminal for the main slot, the first again for
            // the side slot, which is never empty.
            None if slot == Place::Side => {
                if let Some(first) = here.first().copied() {
                    self.place(first, slot);
                }
            }
            None => {
                for app in self.apps.iter_mut().filter(|app| app.screen == screen && app.place == slot) {
                    app.place = Place::Hidden;
                }
            }
        }
        self.arrange();
    }

    /// Brings another workspace into view: its applications where they were, and the
    /// ones of the workspace left behind out of sight until it is come back to.
    pub fn switch_workspace(&mut self, to: usize) {
        if to == self.workspace || to >= des_common::ipc::WORKSPACES {
            return;
        }
        let from = self.workspace;
        for app in self.apps.iter_mut() {
            // Pinned, it is in view on every workspace.
            if app.pinned {
                continue;
            }
            if app.workspace == from {
                app.kept = app.place;
                app.place = Place::Hidden;
            } else if app.workspace == to {
                app.place = app.kept;
            }
        }
        // Split and full screen belong to the workspace too.
        if self.kept_modes.len() < des_common::ipc::WORKSPACES {
            self.kept_modes.resize(des_common::ipc::WORKSPACES, Vec::new());
        }
        self.kept_modes[from] = self.modes.clone();
        let mut modes = std::mem::take(&mut self.kept_modes[to]);
        modes.resize(self.modes.len(), ScreenMode::default());
        self.modes = modes;
        self.workspace = to;
        tracing::info!(workspace = to + 1, "workspace in view");
        // Higher workspaces lie to the right: what arrives comes from there.
        self.motions.slide = Some(if to > from { 1 } else { -1 });
        self.arrange();
        self.motions.slide = None;
    }

    /// Sends an application to another workspace, where it is the one in view on its
    /// display the next time that workspace is.
    pub fn send_to_workspace(&mut self, index: usize, to: usize) {
        if to >= des_common::ipc::WORKSPACES || index >= self.apps.len() || self.apps[index].workspace == to {
            return;
        }
        if self.apps[index].pinned {
            // In view everywhere already; it goes back to this workspace when unpinned.
            self.apps[index].workspace = to;
            self.arrange();
            return;
        }
        let screen = self.apps[index].screen;
        if to == self.workspace {
            // Brought here from elsewhere: into view at once.
            self.apps[index].workspace = to;
            self.place(index, Place::Main);
        } else {
            for app in self.apps.iter_mut().filter(|app| app.workspace == to && app.screen == screen && app.kept == Place::Main) {
                app.kept = Place::Hidden;
            }
            let app = &mut self.apps[index];
            app.workspace = to;
            app.kept = Place::Main;
            app.place = Place::Hidden;
        }
        self.arrange();
    }

    /// Sends the application in use to another workspace.
    pub fn send_active_to(&mut self, to: usize) {
        if let Some(index) = self.focused_on(self.focus_screen) {
            self.send_to_workspace(index, to);
        }
    }

    /// Asks the current application to close. The terminal cannot be closed this way.
    pub fn close_active(&mut self) {
        if let Some(index) = self.keyboard_app() {
            self.close(index);
        }
    }

    fn close(&self, index: usize) {
        let Some(app) = self.apps.get(index) else {
            return;
        };
        if let Some(toplevel) = app.window.toplevel() {
            toplevel.send_close();
        } else if let Some(x11) = app.window.x11_surface() {
            let _ = x11.close();
        }
    }

    /// Moves the current application to the next display.
    pub fn move_active(&mut self) {
        if let Some(index) = self.keyboard_app() {
            self.move_app(index);
        }
    }

    fn move_app(&mut self, index: usize) {
        if self.screens.len() < 2 || index >= self.apps.len() {
            return;
        }
        if self.apps[index].pinned {
            self.apps[index].screen = (self.apps[index].screen + 1) % self.screens.len();
            self.focus_screen = self.apps[index].screen;
            self.arrange();
            return;
        }
        self.apps[index].screen = (self.apps[index].screen + 1) % self.screens.len();
        self.apps[index].place = Place::Hidden;
        self.activate(index);
        self.arrange();
    }

    /// Splits the workspace in use between two windows, or makes it whole again.
    ///
    /// The window in view keeps the left half. The right half gets the application
    /// used before it; with a single application open, that one moves to the right
    /// and the terminal takes the left.
    pub fn toggle_split(&mut self) {
        let screen = self.focus_screen;
        let kind = if self.slots_in_use(screen) > 1 { LayoutKind::Single } else { LayoutKind::Split };
        self.set_layout(screen, kind);
    }

    /// Shares a display's workspace another way; see `layout::plan`.
    pub fn set_layout(&mut self, screen: usize, kind: LayoutKind) {
        if screen >= self.modes.len() {
            return;
        }
        let focused = self.focused_on(screen);
        let members: Vec<usize> = (0..self.apps.len())
            .filter(|i| {
                let app = &self.apps[*i];
                app.screen == screen && app.workspace == self.workspace && !app.pinned
            })
            .collect();
        let places: Vec<Place> = members.iter().map(|i| self.apps[*i].place).collect();
        let used: Vec<u64> = members.iter().map(|i| self.apps[*i].used).collect();
        let focused = focused.and_then(|f| members.iter().position(|i| *i == f));
        let (places, focus) = layout::plan(kind, &places, &used, focused, screen == 0);
        for (i, place) in members.into_iter().zip(places) {
            self.apps[i].place = place;
        }
        if let Some(slot) = focus {
            self.modes[screen].focus_slot = slot;
        }
        self.modes[screen].layout = kind;
        self.arrange();
    }

    /// Super+Shift+S: the next layout round on the display in use.
    pub fn cycle_layout(&mut self) {
        let screen = self.focus_screen;
        let current = self.shown_layout(screen);
        // A split made some other way counts as one.
        let current = if current == LayoutKind::Single && self.slots_in_use(screen) > 1 { LayoutKind::Split } else { current };
        self.set_layout(screen, current.next());
    }

    /// Moves the first divider of the display in use to `share` of its width.
    pub fn set_ratio(&mut self, share: f32) {
        let screen = self.focus_screen;
        let kind = self.shown_layout(screen);
        if let Some(mode) = self.modes.get_mut(screen) {
            mode.ratios.set(kind, 0, share);
            self.arrange();
        }
    }

    /// Super+Alt+Left or Right: moves the divider beside the application in use.
    pub fn nudge_ratio(&mut self, right: bool) {
        let screen = self.focus_screen;
        if screen >= self.modes.len() || self.slots_in_use(screen) < 2 {
            return;
        }
        let kind = self.shown_layout(screen);
        let mode = &mut self.modes[screen];
        let divider = layout::nudged_divider(kind, mode.focus_slot);
        let now = if divider == 0 { mode.ratios.primary(kind) } else { mode.ratios.secondary(kind) };
        let by = if right { layout::NUDGE } else { -layout::NUDGE };
        mode.ratios.set(kind, divider, now + by);
        self.arrange();
    }

    /// Super+Enter: the application in use becomes the master, and the master takes
    /// its slot. Beside the terminal, it takes the terminal's place.
    pub fn swap_master(&mut self) {
        let screen = self.focus_screen;
        let Some(focused) = self.focused_on(screen) else {
            return;
        };
        let members: Vec<usize> = (0..self.apps.len()).filter(|i| self.apps[*i].screen == screen && !self.apps[*i].pinned).collect();
        let mut places: Vec<Place> = members.iter().map(|i| self.apps[*i].place).collect();
        let Some(at) = members.iter().position(|i| *i == focused) else {
            return;
        };
        if !layout::swap_into_master(&mut places, at) {
            return;
        }
        for (i, place) in members.into_iter().zip(places) {
            self.apps[i].place = place;
        }
        self.modes[screen].focus_slot = 0;
        self.arrange();
    }

    /// Sets the layout of one display on one workspace outright, without moving any
    /// application: the placements of a restored session do that.
    pub fn layout_at(&mut self, info: LayoutInfo) {
        if info.screen >= self.screens.len() || info.workspace >= des_common::ipc::WORKSPACES {
            return;
        }
        let apply = |mode: &mut ScreenMode| {
            mode.layout = info.kind;
            mode.ratios.restore(info.kind, info.ratio, info.ratio2);
        };
        if info.workspace == self.workspace {
            if let Some(mode) = self.modes.get_mut(info.screen) {
                apply(mode);
            }
            self.arrange();
        } else {
            if self.kept_modes.len() < des_common::ipc::WORKSPACES {
                self.kept_modes.resize(des_common::ipc::WORKSPACES, Vec::new());
            }
            let kept = &mut self.kept_modes[info.workspace];
            if kept.len() <= info.screen {
                kept.resize(info.screen + 1, ScreenMode::default());
            }
            apply(&mut kept[info.screen]);
        }
    }

    /// The layouts that are not the plain one, on every display and workspace, as the
    /// shell is told them. A split made some other way than by choosing it counts.
    pub fn layout_report(&self) -> Vec<LayoutInfo> {
        let mut report = Vec::new();
        for workspace in 0..des_common::ipc::WORKSPACES {
            let here = workspace == self.workspace;
            let modes = if here {
                &self.modes
            } else {
                match self.kept_modes.get(workspace) {
                    Some(modes) => modes,
                    None => continue,
                }
            };
            for (screen, mode) in modes.iter().enumerate().take(self.screens.len()) {
                let split_anyway = here && mode.layout == LayoutKind::Single && self.slots_in_use(screen) > 1;
                if let Some(info) = layout_info(workspace, screen, mode, split_anyway) {
                    report.push(info);
                }
            }
        }
        report
    }

    /// Gives the keyboard to the slot to the left or right of the one in use (before
    /// or after it, for the stack beside the master). On the main display the terminal
    /// beside an application is the first slot.
    pub fn focus_half(&mut self, right: bool) {
        let screen = self.focus_screen;
        let count = self.slots_in_use(screen);
        if count < 2 || screen >= self.modes.len() {
            return;
        }
        let usable: Vec<usize> = (0..count)
            .filter(|slot| (*slot == 0 && screen == 0) || self.in_place(screen, Place::of_slot(*slot)).is_some())
            .collect();
        let now = self.modes[screen].focus_slot;
        let next = if right {
            usable.iter().copied().find(|slot| *slot > now)
        } else {
            usable.iter().rev().copied().find(|slot| *slot < now)
        };
        if let Some(slot) = next {
            self.modes[screen].focus_slot = slot;
            self.arrange();
        }
    }

    /// The divider a press at `pos` would grab, and what it divides. None over the
    /// lock, the screenshot chooser, the shell's overlays or the pinned window.
    pub fn divider_under(&self, pos: Point<f64, Logical>) -> Option<layout::Drag> {
        if self.locked || self.picking || self.shell_overlay_contains(pos) {
            return None;
        }
        if self.pinned_app().is_some_and(|i| self.pin_rect(i).to_f64().contains(pos)) {
            return None;
        }
        let screen = self.screen_at(pos);
        let count = self.slots_in_use(screen);
        if count < 2 {
            return None;
        }
        let kind = self.shown_layout(screen);
        let area = self.workspace_of(screen);
        let divider = layout::divider_at(&layout::dividers(kind, &self.mode(screen).ratios, area, count), pos)?;
        Some(layout::Drag { screen, divider: divider.index, kind, area, last_configure: Instant::now() })
    }

    /// A press on a divider: it follows the pointer until let go. Returns whether the
    /// press was taken for that.
    pub fn start_resize(&mut self, pos: Point<f64, Logical>) -> bool {
        let Some(drag) = self.divider_under(pos) else {
            return false;
        };
        self.resize = Some(drag);
        self.resize_hover = true;
        true
    }

    /// The pointer moved to `x` while a divider is held.
    pub fn drag_resize(&mut self, x: f64) {
        let Some(drag) = self.resize else {
            return;
        };
        if drag.screen >= self.modes.len() || self.locked {
            self.resize = None;
            return;
        }
        let share = layout::share_at(drag.kind, drag.divider, drag.area, x);
        self.modes[drag.screen].ratios.set(drag.kind, drag.divider, share);
        self.arrange();
    }

    /// The divider is let go: every window is told its final size.
    pub fn end_resize(&mut self) {
        if self.resize.take().is_some() {
            self.arrange();
        }
    }


    /// Lets the applications on the display in use cover all of it, or stops that.
    pub fn toggle_fullscreen(&mut self) {
        let screen = self.focus_screen;
        self.set_fullscreen(screen, !self.mode(screen).fullscreen);
    }

    pub fn set_fullscreen(&mut self, screen: usize, fullscreen: bool) {
        if let Some(mode) = self.modes.get_mut(screen) {
            mode.fullscreen = fullscreen;
            self.arrange();
        }
    }

    /// Opens or collapses one of the frame's panels.
    pub fn toggle_panel(&mut self, panel: Panel) {
        self.panels.toggle(panel);
        self.arrange();
    }

    /// Uncovers the terminal and gives it the keyboard.
    fn show_terminal(&mut self) {
        self.dock_keyboard = false;
        self.pin_focused = false;
        for app in self.apps.iter_mut().filter(|app| app.screen == 0 && app.place == Place::Main) {
            app.place = Place::Hidden;
        }
        self.modes[0].focus_slot = 0;
        self.focus_screen = 0;
        self.arrange();
    }

    /// The pinned application, if one is.
    pub fn pinned_app(&self) -> Option<usize> {
        self.apps.iter().position(|app| app.pinned)
    }

    /// The application with the keyboard: the pinned one after a click on it, or
    /// else the one in use on the display in use.
    pub fn keyboard_app(&self) -> Option<usize> {
        self.pinned_app().filter(|_| self.pin_focused).or_else(|| self.focused_on(self.focus_screen))
    }

    fn pin_rect(&self, index: usize) -> Rectangle<i32, Logical> {
        let app = &self.apps[index];
        pip_rect(self.workspace_of(app.screen), app.pin_aspect, PIP_MARGIN, des_common::DOCK_TRIGGER_HEIGHT.ceil() as i32)
    }

    /// Keeps the application at `index` in a corner above everything, on every
    /// workspace of its display. Only one is pinned at a time.
    pub fn pin(&mut self, index: usize) {
        if index >= self.apps.len() || self.apps[index].pinned {
            return;
        }
        if let Some(other) = self.pinned_app() {
            self.unpin_app(other);
        }
        let size = self.apps[index].window.geometry().size;
        let current = self.workspace;
        let app = &mut self.apps[index];
        app.pin_aspect = if size.h > 0 { size.w as f64 / size.h as f64 } else { 0.0 };
        app.pin_return = if app.place == Place::Hidden { Place::Main } else { app.place };
        if app.workspace != current {
            app.kept = Place::Hidden;
        }
        app.place = Place::Hidden;
        app.pinned = true;
        self.pin_focused = false;
        tracing::info!(index, "pinned in a corner");
        self.arrange();
    }

    /// Puts whatever is pinned back where it was.
    pub fn unpin(&mut self) {
        if let Some(index) = self.pinned_app() {
            self.unpin_app(index);
            self.arrange();
        }
    }

    /// Super+P: pins the application in use, or puts the pinned one back.
    pub fn toggle_pin(&mut self) {
        match self.pinned_app() {
            Some(_) => self.unpin(),
            None => {
                if let Some(index) = self.focused_on(self.focus_screen) {
                    self.pin(index);
                }
            }
        }
    }

    /// The pinned application goes back to the place it was pinned from, on its own
    /// workspace: in view if that is the one in view, else for when it is.
    fn unpin_app(&mut self, index: usize) {
        self.pin_focused = false;
        let (screen, workspace, back) = {
            let app = &mut self.apps[index];
            app.pinned = false;
            (app.screen, app.workspace, app.pin_return)
        };
        if workspace == self.workspace {
            self.place(index, back);
            self.focus_screen = screen;
            if let Some(mode) = self.modes.get_mut(screen) {
                mode.focus_slot = back.slot().unwrap_or(0);
            }
        } else {
            for app in self.apps.iter_mut().filter(|app| app.workspace == workspace && app.screen == screen && app.kept == back) {
                app.kept = Place::Hidden;
            }
            self.apps[index].kept = back;
        }
    }

    /// Applies the first window rule that matches a new application, once. Returns
    /// whether it moved anything.
    pub fn apply_rules(&mut self, window: &Window) -> bool {
        let Some(index) = self.apps.iter().position(|app| &app.window == window) else {
            return false;
        };
        if std::mem::replace(&mut self.apps[index].ruled, true) {
            return false;
        }
        let identity = window_app_id(window);
        let rules = crate::rules::load();
        let Some(rule) = crate::rules::find(&rules, &identity).cloned() else {
            return false;
        };
        let (old_screen, old_workspace, old_place) = {
            let app = &self.apps[index];
            (app.screen, app.workspace, app.place)
        };
        let screen = rule.display.filter(|display| *display < self.screens.len()).unwrap_or(old_screen);
        let workspace = rule.workspace.unwrap_or(old_workspace);
        let place = if rule.side { Place::Side } else if old_place == Place::Hidden { Place::Main } else { old_place };
        if (screen, workspace, place) == (old_screen, old_workspace, old_place) {
            return false;
        }
        tracing::info!(%identity, ?rule, "window rule");
        let over = {
            let app = &mut self.apps[index];
            app.screen = screen;
            app.workspace = workspace;
            app.place = Place::Hidden;
            app.over.take()
        };
        if workspace == self.workspace {
            self.place(index, place);
            self.focus_screen = screen;
            if let Some(mode) = self.modes.get_mut(screen) {
                mode.focus_slot = place.slot().unwrap_or(0);
            }
        } else {
            for app in self.apps.iter_mut().filter(|app| app.workspace == workspace && app.screen == screen && app.kept == place) {
                app.kept = Place::Hidden;
            }
            self.apps[index].kept = place;
        }
        // What it opened on top of gets its place back, as when a window closes.
        if let Some(under) = over.and_then(|over| self.apps.iter().position(|other| other.window == over)) {
            let back = &self.apps[under];
            let returns = back.screen == old_screen && back.workspace == old_workspace && back.place == Place::Hidden;
            if returns && old_workspace == self.workspace && old_place != Place::Hidden && self.in_place(old_screen, old_place).is_none() {
                self.apps[under].place = old_place;
            }
        }
        true
    }

    /// Asks the shell to lock the screen. It answers when it can show the lock screen,
    /// so that a shell unable to unlock is never locked.
    pub fn request_lock(&mut self) {
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.send(Event::LOCK_REQUESTED);
        }
    }

    /// Asks for the lock once the machine has been left alone for as long as the
    /// settings say, unless something on screen is being watched.
    pub fn check_idle(&mut self) {
        self.idle_inhibitors.retain(smithay::utils::IsAlive::alive);
        let watched = self.idle_held || !self.idle_inhibitors.is_empty();
        if watched {
            // The wait starts over when the watching ends.
            self.last_input = std::time::Instant::now();
            return;
        }
        if let Some(minutes) = self.input_settings.screen_off_after {
            if !self.screens_off && self.last_input.elapsed() >= std::time::Duration::from_secs(minutes as u64 * 60) {
                tracing::info!(minutes, "left alone: turning the screens off");
                self.blank(true);
            }
        }
        let Some(minutes) = self.input_settings.lock_after else {
            return;
        };
        let waited = self.last_input.elapsed() >= std::time::Duration::from_secs(minutes as u64 * 60);
        if waited && !self.locked && !self.idle_lock_asked && self.shell.is_some() {
            tracing::info!(minutes, "left alone: asking for the lock");
            self.idle_lock_asked = true;
            self.request_lock();
        }
    }

    /// Hides everything but the shell and gives it the keyboard, or undoes that.
    pub fn set_locked(&mut self, locked: bool) {
        if self.locked != locked {
            tracing::info!(locked, "screen lock");
            self.locked = locked;
            if locked {
                // No pre-lock keyboard grab may intercept the lock screen.
                if let Some(keyboard) = self.seat.get_keyboard() {
                    keyboard.unset_grab(self);
                }
                self.release_pointer_constraint();
                self.cancel_file_drag();
                self.close_overview_now();
                self.dock_keyboard = false;
                self.dock = None;
                self.docks.clear();
                self.overlay = None;
                self.pin_focused = false;
                self.stop_recording();
                self.focus_screen = 0;
            }
            self.arrange();
        }
    }

    /// Asks the shell to show its application launcher.
    pub fn open_launcher(&mut self) {
        self.ask_shell(Event::LAUNCHER);
    }

    /// Asks the shell to show the settings.
    pub fn open_settings(&mut self) {
        self.ask_shell(Event::SETTINGS);
    }

    /// Asks the shell to show the music player.
    pub fn open_music(&mut self) {
        self.ask_shell(Event::MUSIC);
    }

    /// Asks the shell to show the power menu.
    pub fn open_power(&mut self) {
        self.ask_shell(Event::POWER);
    }

    /// Asks the shell to show the list of notifications.
    pub fn ask_notifications(&mut self) {
        self.ask_shell(Event::NOTIFICATIONS);
    }

    /// Asks the shell for the quick settings. They float above applications, so the
    /// workspace stays as it is.
    pub fn ask_quick(&mut self) {
        if self.modes.is_empty() || self.locked || self.picking {
            return;
        }
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.send(Event::QUICK);
        }
    }

    /// Uncovers the shell's workspace, where it shows such things, and sends `event`.
    fn ask_shell(&mut self, event: &str) {
        if self.modes.is_empty() || self.locked {
            return;
        }
        self.show_terminal();
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.send(event);
        }
    }

    /// A click landed at `pos`: that display, and that half of it, are now in use.
    pub fn focus_at(&mut self, pos: Point<f64, Logical>) {
        // A click on a layer surface that takes the keyboard on demand goes to it.
        if self.layer_click(pos) {
            return;
        }
        let screen = self.screen_at(pos);
        if screen >= self.modes.len() {
            return;
        }
        // A click on the pinned application gives it the keyboard; one anywhere
        // else takes the keyboard back.
        let on_pin = self.pinned_app().filter(|_| !self.locked).is_some_and(|i| self.pin_rect(i).to_f64().contains(pos));
        if on_pin != self.pin_focused {
            self.pin_focused = on_pin;
            self.arrange();
        }
        if on_pin {
            return;
        }
        let areas = self.slot_areas(screen);
        let slot = if areas.len() < 2 {
            0
        } else {
            // A click on the frame itself, or between windows, changes nothing.
            areas.iter().position(|area| area.to_f64().contains(pos)).unwrap_or(self.modes[screen].focus_slot)
        };
        if screen != self.focus_screen || slot != self.modes[screen].focus_slot {
            self.focus_screen = screen;
            self.modes[screen].focus_slot = slot;
            self.arrange();
        }
    }

    /// Carries out the shell's requests and tells it about any change in open applications.
    pub fn pump_ipc(&mut self) {
        let Some(mut ipc) = self.ipc.take() else {
            return;
        };
        for command in ipc.poll(self.shell_pid) {
            if self.modes.is_empty() {
                break;
            }
            // Lock requests remain idempotent and acknowledged while already locked.
            if !command_allowed(self.locked, self.picking, &command) {
                continue;
            }
            match command {
                // The window may have closed while the request was on its way.
                Command::Activate(index) if index < self.apps.len() => {
                    self.activate(index);
                    self.arrange();
                }
                Command::Activate(_) => {}
                Command::PlaceApp(index,screen,workspace,place) => {
                    if index<self.apps.len() && screen<self.screens.len() && workspace<des_common::ipc::WORKSPACES {
                        if place!=Place::Hidden { for (i,app) in self.apps.iter_mut().enumerate() {
                            if i!=index && app.screen==screen && app.workspace==workspace && occupies_restored_slot(self.workspace,workspace,app.place,app.kept,place) {app.place=Place::Hidden;app.kept=Place::Hidden;}
                        }}
                        let app=&mut self.apps[index];app.pinned=false;app.screen=screen;app.workspace=workspace;app.kept=place;app.over=None;
                        app.place=if workspace==self.workspace{place}else{Place::Hidden};
                        self.arrange();
                    }
                }
                Command::Close(index) => self.close(index),
                Command::Move(index) => self.move_app(index),
                Command::Workspace(workspace) => self.switch_workspace(workspace),
                Command::SendTo(index, workspace) => self.send_to_workspace(index, workspace),
                Command::Terminal => self.show_terminal(),
                Command::Split => self.toggle_split(),
                Command::Fullscreen => self.toggle_fullscreen(),
                Command::TogglePanel(panel) => self.toggle_panel(panel),
                Command::ApplyDisplays(choices) => self.apply_displays(choices),
                Command::KeepDisplays => self.keep_displays(),
                Command::RevertDisplays => self.revert_displays(),
                Command::ReloadInput => self.reload_input(),
                Command::ReloadFrame => {
                    self.side_panel_behavior = des_common::PanelBehavior::load();
                    self.arrange();
                },
                Command::Lock => {
                    self.set_locked(true);
                    self.tell(Event::locked(true));
                }
                Command::Unlock => {
                    self.set_locked(false);
                    self.tell(Event::locked(false));
                }
                Command::Shot(mode) => self.take_shot(mode),
                Command::ShotRequest { id, mode } => self.take_requested_shot(mode, Some(id)),
                Command::Record(path) => self.start_recording(&path),
                Command::RecordStop => self.stop_recording(),
                Command::HoldIdle(held) => {
                    tracing::info!(held, "an application is asking that the screen be left alone");
                    self.idle_held = held;
                }
                Command::DockOn(screen,area) => {
                    if screen==0 { self.dock=area.map(|(x,y,w,h)|Rectangle::new((x,y).into(),(w,h).into())); }
                    else if screen<self.screens.len() { match area {
                        Some((x,y,w,h))=>{let id=self.docks.get(&screen).map(|(_,id)|id.clone()).unwrap_or_else(smithay::backend::renderer::element::Id::new);self.docks.insert(screen,(Rectangle::new((x,y).into(),(w,h).into()),id));},
                        None=>{self.docks.remove(&screen);}
                    }}
                    if dismisses_dock_keyboard(self.dock_keyboard_screen,screen,area.is_none()) && std::mem::take(&mut self.dock_keyboard){if !self.picking{self.arrange();}}
                }
                Command::Dock(area) => {
                    self.dock = area.map(|(x, y, w, h)| Rectangle::new((x, y).into(), (w, h).into()));
                    if dismisses_dock_keyboard(self.dock_keyboard_screen,0,area.is_none()) && std::mem::take(&mut self.dock_keyboard) {
                        if !self.picking {self.arrange();}
                    }
                }
                Command::Layout(kind) => self.set_layout(self.focus_screen, kind),
                Command::Ratio(share) => self.set_ratio(share),
                Command::LayoutAt(info) => self.layout_at(info),
                Command::Pin(index) => self.pin(index),
                Command::Unpin => self.unpin(),
                Command::DragFiles(uris) => self.start_file_drag(uris),
                Command::Overview => self.toggle_overview(),
                Command::AppWindows(screen,id) => self.open_app_windows(screen,&id),
                Command::Overlay(area) => {
                    self.overlay = area.map(|(x, y, w, h)| Rectangle::new((x, y).into(), (w, h).into()));
                    // The keyboard goes to the shell while the overlay shows, and back after.
                    if !self.picking {
                        self.arrange();
                    }
                }
                Command::Toast(area) => {
                    self.toast = area.map(|(x, y, w, h)| Rectangle::new((x, y).into(), (w.max(0), h.max(0)).into()));
                    // The shell is done choosing what to picture: the keyboard goes
                    // back to whatever had it.
                    let whole = self.screens.first().copied();
                    if self.picking && !self.toast.zip(whole).is_some_and(|(toast, whole)| toast.contains_rect(whole)) {
                        self.picking = false;
                        self.arrange();
                    }
                }
            }
        }
        if !ipc.has_shell_client() {
            self.dock = None;
            self.docks.clear();
            if self.overlay.take().is_some() {
                self.arrange();
            }
            if std::mem::take(&mut self.dock_keyboard) { self.arrange(); }
        }
        let last_screen = self.screens.len().saturating_sub(1);
        ipc.publish(&DesktopState {
            saved_places:self.apps.iter().map(|app|if app.workspace==self.workspace{app.place}else{app.kept}).collect(),
            screens: self.screens.len().max(1),
            panels: self.panels,
            terminal_split: self.in_place(0, Place::Side).is_some() && self.in_place(0, Place::Main).is_none(),
            focus: self.keyboard_app(),
            workspace: self.workspace,
            apps: self
                .apps
                .iter()
                .map(|app| AppInfo {
                    app_id: window_app_id(&app.window),
                    title: window_title(&app.window),
                    screen: app.screen.min(last_screen),
                    place: app.place,
                    workspace: app.workspace,
                    pinned: app.pinned,
                    urgent: app.urgent,
                })
                .collect(),
        });
        ipc.publish_displays(&self.display_report());
        ipc.publish_layouts(&self.layout_report());
        ipc.publish_locked(self.locked);
        // What handling the commands gave rise to, while the socket was out of reach.
        for line in std::mem::take(&mut self.unsaid) {
            ipc.send(&line);
        }
        self.ipc = Some(ipc);
    }

    /// Says something to the shell: at once, or as soon as the socket is back in hand.
    pub fn tell(&mut self, line: String) {
        match self.ipc.as_mut() {
            Some(ipc) => ipc.send(&line),
            None => self.unsaid.push(line),
        }
    }
}

fn to_rectangle(rect: des_common::Rect, origin: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (origin.x + rect.x.round() as i32, origin.y + rect.y.round() as i32).into(),
        (rect.width.round() as i32, rect.height.round() as i32).into(),
    )
}

/// The protocol identity supplied by the application itself.
pub(crate) fn window_app_id(window: &Window) -> String {
    if let Some(x11) = window.x11_surface() { return x11.class(); }
    let Some(toplevel) = window.toplevel() else { return String::new(); };
    with_states(toplevel.wl_surface(), |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap().app_id.clone().unwrap_or_default()
    })
}

/// The title a window gave itself, falling back to its application id.
pub(crate) fn window_title(window: &Window) -> String {
    if let Some(x11) = window.x11_surface() {
        let title = x11.title();
        return if title.is_empty() { x11.class() } else { title };
    }
    let Some(toplevel) = window.toplevel() else {
        return String::new();
    };
    with_states(toplevel.wl_surface(), |states| {
        let data = states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .unwrap()
            .lock()
            .unwrap();
        data.title
            .clone()
            .filter(|title| !title.is_empty())
            .or_else(|| data.app_id.clone())
            .unwrap_or_else(|| "untitled".to_string())
    })
}

/// Tells a window its place, whether it has a display to itself, and whether it has
/// the keyboard.
fn configure(window: &Window, area: Rectangle<i32, Logical>, fullscreen: bool, activated: bool) {
    if let Some(x11) = window.x11_surface() {
        if x11.is_fullscreen() != fullscreen {
            let _ = x11.set_fullscreen(fullscreen);
        }
        // X11 windows are told their position too: their menus are placed against it.
        if x11.geometry() != area {
            let _ = x11.configure(area);
        }
        if x11.is_maximized() == fullscreen {
            let _ = x11.set_maximized(!fullscreen);
        }
        if x11.is_activated() != activated {
            let _ = x11.set_activated(activated);
        }
        return;
    }
    let Some(toplevel) = window.toplevel() else {
        return;
    };
    toplevel.with_pending_state(|state| {
        state.size = Some(area.size);
        let (on, off) = if fullscreen {
            (xdg_toplevel::State::Fullscreen, xdg_toplevel::State::Maximized)
        } else {
            (xdg_toplevel::State::Maximized, xdg_toplevel::State::Fullscreen)
        };
        state.states.set(on);
        state.states.unset(off);
        if activated {
            state.states.set(xdg_toplevel::State::Activated);
        } else {
            state.states.unset(xdg_toplevel::State::Activated);
        }
    });
    // Before the first commit, the commit handler sends the initial configure instead.
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

#[cfg(test)]
mod pin_and_overlay_tests {
    use super::*;

    #[test]
    fn picture_in_picture_is_a_quarter_wide_in_the_corner_and_keeps_its_shape() {
        let area = Rectangle::new((100, 44).into(), (1600, 900).into());
        let pip = pip_rect(area, 16.0 / 9.0, 16, 4);
        assert_eq!((pip.size.w, pip.size.h), (400, 225));
        assert_eq!(pip.loc.x + pip.size.w, 100 + 1600 - 16);
        assert_eq!(pip.loc.y + pip.size.h, 44 + 900 - 16 - 4);
        assert!(area.contains_rect(pip));
        // A tall window is held to half the height, and keeps its shape.
        let tall = pip_rect(area, 0.25, 16, 4);
        assert_eq!(tall.size.h, 450);
        assert!((tall.size.w as f64 / tall.size.h as f64 - 0.25).abs() < 0.01);
        assert!(area.contains_rect(tall));
        // No size yet: sixteen by nine.
        let no_size = pip_rect(area, 0.0, 16, 4);
        assert_eq!((no_size.size.w, no_size.size.h), (400, 225));
        // A tiny workspace still holds it.
        let tiny = Rectangle::new((0, 0).into(), (40, 30).into());
        assert!(tiny.contains_rect(pip_rect(tiny, 16.0 / 9.0, 16, 4)));
    }

    #[test]
    fn overlays_wait_for_the_lock_and_screenshots_but_can_always_be_cleared_unlocked() {
        let area = Some((0, 44, 1920, 400));
        assert!(command_allowed(false, false, &Command::Overlay(area)));
        assert!(!command_allowed(true, false, &Command::Overlay(area)));
        assert!(!command_allowed(false, true, &Command::Overlay(area)));
        assert!(command_allowed(false, true, &Command::Overlay(None)));
        let drag = Command::DragFiles(vec!["file:///a".into()]);
        assert!(command_allowed(false, false, &drag));
        assert!(!command_allowed(true, false, &drag));
        assert!(!command_allowed(false, true, &drag));
        assert!(!command_allowed(true, false, &Command::Pin(0)));
        assert!(!command_allowed(true, false, &Command::Unpin));
        assert!(command_allowed(false, false, &Command::Pin(0)));
    }
}
