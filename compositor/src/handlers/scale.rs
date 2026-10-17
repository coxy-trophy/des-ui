//! Fractional scaling: each window is told the exact scale of the display it is on,
//! 1.25 or 1.5 as well as whole numbers, so that it draws its buffer at that size
//! and, with a viewport, is shown sharp instead of a whole-number buffer scaled down.
//! A window on more than one display takes the scale of the one it is drawn on first.

use smithay::{
    delegate_fractional_scale,
    desktop::Window,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        compositor::{send_surface_state, with_states},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
    },
};

use crate::DesComp;

impl FractionalScaleHandler for DesComp {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // Said at once, for a window that asks after it was placed.
        let output = self
            .window_for(&surface)
            .and_then(|window| self.space.outputs_for_element(&window).into_iter().next())
            .or_else(|| self.space.outputs().next().cloned());
        if let Some(output) = output {
            with_states(&surface, |states| {
                with_fractional_scale(states, |fractional| fractional.set_preferred_scale(output.current_scale().fractional_scale()));
            });
        }
    }
}

delegate_fractional_scale!(DesComp);

/// The whole-number scale a client without fractional scaling should draw at: the
/// display's scale rounded up, so that it is scaled down, never blurred up.
pub fn integer_scale(fractional: f64) -> i32 {
    (fractional.ceil() as i32).max(1)
}

/// Tells every surface of `window` the scale and turn of `output`.
fn prefer(window: &Window, output: &Output) {
    let scale = output.current_scale().fractional_scale();
    let transform = output.current_transform();
    window.with_surfaces(|surface, states| {
        with_fractional_scale(states, |fractional| fractional.set_preferred_scale(scale));
        send_surface_state(surface, states, integer_scale(scale), transform);
    });
}

impl DesComp {
    /// Called once a frame, before drawing: windows that moved to another display,
    /// or whose display changed scale, hear of it. Smithay sends nothing it already sent.
    pub fn prefer_scales(&self) {
        let windows = self.shell.iter().chain(self.backdrops.iter()).chain(self.apps.iter().map(|app| &app.window));
        for window in windows {
            let output = self.space.outputs_for_element(window).into_iter().next();
            if let Some(output) = output {
                prefer(window, &output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_number_scales_round_up() {
        assert_eq!(integer_scale(1.0), 1);
        assert_eq!(integer_scale(1.25), 2);
        assert_eq!(integer_scale(1.5), 2);
        assert_eq!(integer_scale(2.0), 2);
        assert_eq!(integer_scale(0.0), 1);
    }
}
