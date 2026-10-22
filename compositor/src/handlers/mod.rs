mod buffers;
mod compositor;
pub mod input_protocols;
pub mod cursor_shape;
pub mod presentation;
mod primary_selection;
pub mod scale;
pub mod xdg_activation;
mod xdg_foreign;
mod xdg_shell;
pub mod layer_shell;

use crate::{DesComp, focus::Focus};
use smithay::wayland::seat::WaylandFocus;

//
// Wl Seat
//

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::{delegate_data_device, delegate_output, delegate_seat};

impl SeatHandler for DesComp {
    type KeyboardFocus = Focus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<DesComp> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.cursor_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Focus>) {
        let dh = &self.display_handle;
        let client = focused
            .and_then(|focus| focus.wl_surface())
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        smithay::wayland::selection::primary_selection::set_primary_focus(dh, seat, client);
        // The window switcher lists windows in the order they had the keyboard.
        let given = focused.and_then(|focus| {
            self.apps.iter().find(|app| match focus {
                Focus::X11(x11) => app.window.x11_surface() == Some(x11),
                Focus::Surface(surface) => app.window.wl_surface().as_deref() == Some(surface),
            })
        });
        if let Some(window) = given.map(|app| app.window.clone()) {
            self.recency.touch(&window);
        }
    }
}

delegate_seat!(DesComp);

//
// Wl Data Device
//

impl SelectionHandler for DesComp {
    type SelectionUserData = ();

    // What a Wayland application copies is offered to X11 applications too.
    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(e) = xwm.new_selection(ty, source.map(|source| source.mime_types())) {
                tracing::warn!("cannot offer the clipboard to X11 applications: {e}");
            }
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(e) = xwm.send_selection(ty, mime_type, fd, self.loop_handle.clone()) {
                tracing::warn!("cannot fetch the clipboard from an X11 application: {e}");
            }
        }
    }
}

impl DataDeviceHandler for DesComp {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for DesComp {
    fn started(&mut self, source:Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>, icon:Option<WlSurface>, _seat:Seat<Self>) {
        // Source-less drags are restricted to one client; touch drags have their own routing.
        if source.is_none() || self.buttons_down==0 || self.locked || self.picking {return;}
        let focus=self.seat.get_pointer().and_then(|p|p.current_focus());
        let origin=focus.and_then(|mut surface| {
            while let Some(parent)=smithay::wayland::compositor::get_parent(&surface) {surface=parent;}
            self.window_for(&surface)
        });
        self.drag_icon=icon;
        self.drag_session=Some(crate::dnd::Session::new(source,origin));
    }
    fn dropped(&mut self, _target:Option<WlSurface>, _validated:bool, _seat:Seat<Self>) {
        self.drag_icon=None;
        if let Some(session)=self.drag_session.as_mut(){session.ended=true;}
    }
}

delegate_data_device!(DesComp);

//
// Wl Output & Xdg Output
//

impl OutputHandler for DesComp {}
delegate_output!(DesComp);

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for DesComp {
    fn inhibit(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        if !self.idle_inhibitors.contains(&surface) {
            self.idle_inhibitors.push(surface);
        }
    }

    fn uninhibit(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        self.idle_inhibitors.retain(|held| *held != surface);
    }
}
smithay::delegate_idle_inhibit!(DesComp);

impl smithay::wayland::selection::wlr_data_control::DataControlHandler for DesComp {
    fn data_control_state(&self) -> &smithay::wayland::selection::wlr_data_control::DataControlState {
        &self.data_control_state
    }
}
smithay::delegate_data_control!(DesComp);
