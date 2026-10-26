//! X11 applications: runs an X server (Xwayland) as a client of this compositor and
//! acts as its window manager, so that X11 windows are placed like any other.

use std::{os::fd::OwnedFd, process::Stdio};

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
                set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::DesComp;

impl DesComp {
    /// Starts the X server. Applications can connect at once; their windows appear as
    /// soon as it is ready.
    pub fn start_xwayland(&mut self) {
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            crate::cursor_theme::client_environment(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(spawned) => spawned,
            Err(e) => {
                tracing::warn!("cannot start Xwayland, X11 applications will not open: {e}");
                return;
            }
        };
        let number = xwayland.display_number();
        let handle = self.loop_handle.clone();
        let inserted = self.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, .. } => {
                match X11Wm::start_wm(handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        tracing::info!(display = number, "Xwayland ready");
                        state.xwm = Some(wm);
                    }
                    Err(e) => tracing::error!("cannot manage X11 windows: {e}"),
                }
            }
            XWaylandEvent::Error => {
                tracing::error!("Xwayland failed to start, X11 applications will not open");
                state.xwm = None;
            }
        });
        match inserted {
            Ok(_) => self.x_display = Some(number),
            Err(e) => tracing::warn!("cannot watch Xwayland: {e}"),
        }
    }

    fn x11_app(&self, surface: &X11Surface) -> Option<usize> {
        self.apps
            .iter()
            .position(|app| app.window.x11_surface() == Some(surface))
    }

    fn forget_x11(&mut self, surface: &X11Surface) {
        if let Some(index) = self.x_popups.iter().position(|w| w.x11_surface() == Some(surface)) {
            let window = self.x_popups.remove(index);
            self.space.unmap_elem(&window);
        } else if let Some(index) = self.x11_app(surface) {
            self.forget_app(index);
            tracing::info!(remaining = self.apps.len(), "X11 application window closed");
            self.arrange();
        }
    }
}

impl XWaylandShellHandler for DesComp {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}
delegate_xwayland_shell!(DesComp);

impl XwmHandler for DesComp {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("X11 event without a window manager")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    // An ordinary window asks to be shown: it becomes an application.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(e) = window.set_mapped(true) {
            tracing::warn!("cannot map X11 window: {e}");
            return;
        }
        self.add_window(Window::new_x11_window(window), false);
    }

    // Menus, tooltips and dropdowns place themselves, in the same coordinates as ours.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        let window = Window::new_x11_window(window);
        self.space.map_element(window.clone(), location, false);
        self.x_popups.push(window);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11(&window);
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // Applications are placed by the compositor; tell them where they still are.
        if self.x11_app(&window).is_some() {
            let _ = window.configure(None);
            return;
        }
        // Before it is shown, a window may choose its size.
        let mut geometry = window.geometry();
        if let Some(w) = w {
            geometry.size.w = w as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h as i32;
        }
        let _ = window.configure(geometry);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // A menu moved itself.
        if let Some(popup) = self.x_popups.iter().find(|w| w.x11_surface() == Some(&window)).cloned() {
            self.space.map_element(popup, geometry.loc, false);
        }
    }

    // Windows cannot be moved or resized by their applications.
    fn resize_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32, _edge: ResizeEdge) {}
    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    // The clipboard is shared between X11 and Wayland applications.
    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        true
    }

    // The clipboard and the primary selection (select, then middle-click) are both
    // shared between X11 and Wayland applications, each through its own protocol.
    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let failed = match selection {
            SelectionTarget::Clipboard => request_data_device_client_selection(&self.seat, mime_type, fd).err().map(|e| e.to_string()),
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd).err().map(|e| e.to_string()),
        };
        if let Some(e) = failed {
            tracing::warn!(?selection, "cannot hand the selection to an X11 application: {e}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&self.display_handle, &self.seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }
}
