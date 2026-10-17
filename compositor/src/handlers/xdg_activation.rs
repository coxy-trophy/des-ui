//! xdg-activation: an application asks for one of its windows, or another
//! application's, to be brought forward, as a link clicked in the terminal asks for
//! the browser. A request backed by recent input from the window with the keyboard
//! is followed; any other marks the window as wanting attention instead, so that
//! nothing steals the keyboard. Nothing is brought forward over the lock or while a
//! screenshot is being picked.

use std::time::Duration;

use smithay::{
    delegate_xdg_activation,
    input::Seat,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
};

use crate::DesComp;

/// How long a token is good for after it was made.
const TOKEN_LIFETIME: Duration = Duration::from_secs(10);

/// What becomes of a request to bring a window forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// Bring it forward and give it the keyboard.
    Focus,
    /// Leave things as they are; the window wants attention.
    Attention,
    /// Leave things as they are and say nothing.
    Ignore,
}

/// `earned`: the token was made while its maker had the keyboard, from recent input.
pub fn activation(locked: bool, picking: bool, earned: bool) -> Activation {
    if locked {
        // Marking it would show through nothing anyway; the unlock brings the desktop back as it was.
        Activation::Ignore
    } else if picking || !earned {
        Activation::Attention
    } else {
        Activation::Focus
    }
}

impl XdgActivationHandler for DesComp {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(&mut self, token: XdgActivationToken, token_data: XdgActivationTokenData, surface: WlSurface) {
        // A token is used once.
        self.xdg_activation_state.remove_token(&token);
        let fresh = token_data.timestamp.elapsed() < TOKEN_LIFETIME;
        let earned = fresh
            && token_data.serial.as_ref().is_some_and(|(serial, seat)| {
                let keyboard = self.seat.get_keyboard().unwrap();
                Seat::<DesComp>::from_resource(seat).as_ref() == Some(&self.seat)
                    && keyboard.last_enter().is_some_and(|last| serial.is_no_older_than(&last))
            });
        let Some(window) = self.window_for(&surface) else {
            return;
        };
        let Some(index) = self.apps.iter().position(|app| app.window == window) else {
            // The shell and its backdrops are never brought forward by others.
            return;
        };
        match activation(self.locked, self.picking, earned) {
            Activation::Focus => {
                tracing::info!(index, "brought forward on request");
                self.activate(index);
                self.arrange();
            }
            Activation::Attention => self.apps[index].urgent = true,
            Activation::Ignore => {}
        }
    }
}

delegate_xdg_activation!(DesComp);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_earned_requests_take_the_keyboard_and_never_over_the_lock() {
        assert_eq!(activation(false, false, true), Activation::Focus);
        assert_eq!(activation(false, false, false), Activation::Attention);
        assert_eq!(activation(false, true, true), Activation::Attention);
        assert_eq!(activation(true, false, true), Activation::Ignore);
        assert_eq!(activation(true, true, false), Activation::Ignore);
    }
}
