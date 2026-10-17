//! What applications need from the keyboard and the pointer beyond plain keys and
//! movement: an input method composing text for them (text-input and input-method),
//! the pointer held in place or kept inside a window (pointer constraints) with the
//! raw movement still reported (relative pointer), the compositor's shortcuts passed
//! to a remote desktop or a virtual machine (keyboard-shortcuts-inhibit), and keys
//! typed by the shell's own on-screen keyboard (virtual-keyboard).
//!
//! None of it outranks the lock: while the screen is locked, or while a screenshot
//! area is being picked, no constraint holds the pointer and no inhibitor holds back
//! a shortcut.

use std::sync::atomic::{AtomicU32, Ordering};

use smithay::{
    delegate_input_method_manager, delegate_keyboard_shortcuts_inhibit, delegate_pointer_constraints,
    delegate_relative_pointer, delegate_text_input_manager, delegate_virtual_keyboard_manager,
    backend::renderer::utils::RendererSurfaceStateUserData,
    desktop::{PopupKind, PopupManager},
    input::pointer::PointerHandle,
    reexports::wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        compositor::with_states,
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
            KeyboardShortcutsInhibitorSeat,
        },
        pointer_constraints::{PointerConstraint, PointerConstraintsHandler, PointerConstraintsState, with_pointer_constraint},
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        text_input::TextInputManagerState,
        virtual_keyboard::VirtualKeyboardManagerState,
    },
};

use crate::DesComp;

/// The process id of the shell now running, 0 for none. The virtual keyboard is the
/// shell's alone: any other client could otherwise type into whatever has the keyboard.
pub static SHELL_PID: AtomicU32 = AtomicU32::new(0);

/// State the protocols keep beyond their globals.
pub struct InputProtocols {
    pub shortcuts_inhibit: KeyboardShortcutsInhibitState,
}

/// Announces the globals.
pub fn init(dh: &DisplayHandle) -> InputProtocols {
    TextInputManagerState::new::<DesComp>(dh);
    // Any input method may connect; it composes only for the window with the keyboard.
    InputMethodManagerState::new::<DesComp, _>(dh, |_client| true);
    PointerConstraintsState::new::<DesComp>(dh);
    RelativePointerManagerState::new::<DesComp>(dh);
    let handle = dh.clone();
    VirtualKeyboardManagerState::new::<DesComp, _>(dh, move |client| {
        let shell = SHELL_PID.load(Ordering::Relaxed);
        shell != 0 && client.get_credentials(&handle).is_ok_and(|credentials| credentials.pid as u32 == shell)
    });
    InputProtocols { shortcuts_inhibit: KeyboardShortcutsInhibitState::new::<DesComp>(dh) }
}

/// Whether an application may hold the pointer or the compositor's shortcuts now.
pub fn grabs_allowed(locked: bool, picking: bool) -> bool {
    !locked && !picking
}

/// Where an input method's candidate list goes, relative to the surface it composes
/// for: just under the text cursor, or above it when there is no room below, and
/// pulled back inside the display either way.
pub fn ime_popup_offset(
    cursor: Rectangle<i32, Logical>,
    popup: Size<i32, Logical>,
    parent_origin: Point<i32, Logical>,
    display: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let below = cursor.loc.y + cursor.size.h;
    let mut y = below;
    if parent_origin.y + below + popup.h > display.loc.y + display.size.h {
        y = cursor.loc.y - popup.h;
    }
    let mut x = cursor.loc.x;
    let right = display.loc.x + display.size.w;
    if parent_origin.x + x + popup.w > right {
        x = right - popup.w - parent_origin.x;
    }
    x = x.max(display.loc.x - parent_origin.x);
    y = y.max(display.loc.y - parent_origin.y);
    (x, y).into()
}

/// What the pointer under the window with a constraint may do.
#[derive(Debug, Clone, PartialEq)]
pub enum Hold {
    Free,
    /// The pointer stays where it is; only relative motion is reported.
    Locked,
    /// The pointer moves only where the window's region (or the window) has it.
    Confined,
}

impl DesComp {
    /// Whether the window with the keyboard has asked for the compositor's shortcuts,
    /// and is allowed them now.
    pub fn shortcuts_inhibited(&self) -> bool {
        if !grabs_allowed(self.locked, self.picking) {
            return false;
        }
        let Some(focus) = self.seat.get_keyboard().and_then(|keyboard| keyboard.current_focus()) else {
            return false;
        };
        let Some(surface) = focus.wl_surface() else {
            return false;
        };
        self.seat
            .keyboard_shortcuts_inhibitor_for_surface(&surface)
            .is_some_and(|inhibitor| inhibitor.is_active())
    }

    /// What an active constraint on the pointer's window does to movement.
    pub fn pointer_hold(&self) -> Hold {
        let Some(pointer) = self.seat.get_pointer() else {
            return Hold::Free;
        };
        let Some(surface) = pointer.current_focus() else {
            return Hold::Free;
        };
        with_pointer_constraint(&surface, &pointer, |constraint| match constraint {
            Some(constraint) if constraint.is_active() => match &*constraint {
                PointerConstraint::Locked(_) => Hold::Locked,
                PointerConstraint::Confined(_) => Hold::Confined,
            },
            _ => Hold::Free,
        })
    }

    /// Whether `pos` is somewhere the confined pointer may go: inside the window under
    /// it and inside the region it asked for, if any.
    pub fn confined_allows(&self, pos: Point<f64, Logical>) -> bool {
        let Some(pointer) = self.seat.get_pointer() else {
            return true;
        };
        let Some((surface, origin)) = pointer.current_focus().and_then(|focus| {
            self.surface_under(pos).filter(|(under, _)| *under == focus)
        }) else {
            return false;
        };
        let local = (pos - origin).to_i32_round::<i32>();
        with_pointer_constraint(&surface, &pointer, |constraint| {
            constraint.and_then(|constraint| constraint.region().cloned()).is_none_or(|region| region.contains(local))
        })
    }

    /// Activates a constraint the pointer's window asked for, once the pointer is in
    /// the region it named and the lock or the screenshot chooser is not up.
    pub fn activate_pointer_constraint(&mut self) {
        if !grabs_allowed(self.locked, self.picking) {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let location = pointer.current_location();
        let Some((surface, origin)) = self.surface_under(location) else {
            return;
        };
        if pointer.current_focus().as_ref() != Some(&surface) {
            return;
        }
        let local = (location - origin).to_i32_round::<i32>();
        with_pointer_constraint(&surface, &pointer, |constraint| {
            if let Some(constraint) = constraint {
                if !constraint.is_active() && constraint.region().is_none_or(|region| region.contains(local)) {
                    constraint.activate();
                }
            }
        });
    }

    /// Lets go of the pointer: on lock, for the screenshot chooser, and for a
    /// compositor shortcut, so that the user is never trapped in a window.
    pub fn release_pointer_constraint(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(surface) = pointer.current_focus() else {
            return;
        };
        with_pointer_constraint(&surface, &pointer, |constraint| {
            if let Some(constraint) = constraint.filter(|constraint| constraint.is_active()) {
                constraint.deactivate();
            }
        });
    }

    /// Moves an input method's candidate list under the text cursor, inside its display.
    pub fn place_ime_popup(&self, surface: &WlSurface) {
        let Some(PopupKind::InputMethod(popup)) = self.popups.find_popup(surface) else {
            return;
        };
        let Some(parent) = popup.get_parent().map(|parent| parent.surface.clone()) else {
            return;
        };
        let origin = self
            .window_for(&parent)
            .and_then(|window| self.space.element_geometry(&window))
            .map_or(Point::default(), |geometry| geometry.loc);
        let display = self
            .screens
            .get(self.screen_at(origin.to_f64()))
            .copied()
            .unwrap_or_else(|| Rectangle::new(origin, (i32::MAX / 2, i32::MAX / 2).into()));
        let size = with_states(popup.wl_surface(), |states| {
            states.data_map.get::<RendererSurfaceStateUserData>().and_then(|data| data.lock().ok()?.surface_size())
        })
        .unwrap_or_default();
        popup.set_location(ime_popup_offset(popup.text_input_rectangle(), size, origin, display));
    }
}

impl InputMethodHandler for DesComp {
    fn new_popup(&mut self, surface: PopupSurface) {
        let wl_surface = surface.wl_surface().clone();
        if let Err(e) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("cannot show the input method's popup: {e}");
            return;
        }
        self.place_ime_popup(&wl_surface);
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        self.place_ime_popup(surface.wl_surface());
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.window_for(parent).map(|window| window.geometry()).unwrap_or_default()
    }
}

delegate_input_method_manager!(DesComp);
delegate_text_input_manager!(DesComp);

impl PointerConstraintsHandler for DesComp {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            self.activate_pointer_constraint();
        }
    }

    fn cursor_position_hint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>, location: Point<f64, Logical>) {
        // Where a game draws its own cursor: the pointer is put there when the lock ends.
        let active = with_pointer_constraint(surface, pointer, |constraint| constraint.is_some_and(|c| c.is_active()));
        if !active {
            return;
        }
        if let Some((under, origin)) = self.surface_under(pointer.current_location()) {
            if under == *surface {
                pointer.set_location(origin + location);
            }
        }
    }
}

delegate_pointer_constraints!(DesComp);
delegate_relative_pointer!(DesComp);

impl KeyboardShortcutsInhibitHandler for DesComp {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.input_protocols.shortcuts_inhibit
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Allowed for any application that asks; the safety keys stay ours regardless
        // (see input.rs `inhibitable`), and nothing is held back while locked.
        tracing::info!(client = ?inhibitor.wl_surface().id(), "an application takes the compositor's shortcuts");
        inhibitor.activate();
    }
}

delegate_keyboard_shortcuts_inhibit!(DesComp);
delegate_virtual_keyboard_manager!(DesComp);

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn grabs_end_on_lock_and_while_picking() {
        assert!(grabs_allowed(false, false));
        assert!(!grabs_allowed(true, false));
        assert!(!grabs_allowed(false, true));
        assert!(!grabs_allowed(true, true));
    }

    #[test]
    fn candidates_go_under_the_cursor() {
        let display = rect(0, 0, 1920, 1080);
        let offset = ime_popup_offset(rect(100, 200, 2, 20), (300, 120).into(), (400, 100).into(), display);
        assert_eq!(offset, (100, 220).into());
    }

    #[test]
    fn candidates_go_above_the_cursor_near_the_bottom() {
        let display = rect(0, 0, 1920, 1080);
        let offset = ime_popup_offset(rect(100, 900, 2, 20), (300, 120).into(), (400, 100).into(), display);
        assert_eq!(offset, (100, 780).into());
    }

    #[test]
    fn candidates_stay_inside_the_display_sideways() {
        let display = rect(0, 0, 1920, 1080);
        // The cursor near the right edge: the list is pulled left so it ends at the edge.
        let offset = ime_popup_offset(rect(1400, 10, 2, 20), (300, 120).into(), (400, 100).into(), display);
        assert_eq!(400 + offset.x + 300, 1920);
        // On a second display to the right, never left of its own edge.
        let second = rect(1920, 0, 1280, 1024);
        let offset = ime_popup_offset(rect(-50, 10, 2, 20), (300, 120).into(), (1930, 100).into(), second);
        assert_eq!(1930 + offset.x, 1920);
    }
}
