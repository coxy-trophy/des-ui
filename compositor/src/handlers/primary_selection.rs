//! The primary selection: text selected in one application is pasted into another
//! with the middle button. What it holds is shared with X11 applications the same
//! way the clipboard is (see `SelectionHandler` beside this and `xwayland.rs`).

use smithay::{
    delegate_primary_selection,
    wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
};

use crate::DesComp;

impl PrimarySelectionHandler for DesComp {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

delegate_primary_selection!(DesComp);
