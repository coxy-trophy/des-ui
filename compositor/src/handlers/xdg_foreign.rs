//! xdg-foreign: a window of one application can be the parent of another's, as a
//! sandboxed application's file chooser, drawn by a portal, belongs to the window
//! that asked for it. Smithay keeps the parent; placement follows it like any dialog.

use smithay::{
    delegate_xdg_foreign,
    wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState},
};

use crate::DesComp;

impl XdgForeignHandler for DesComp {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}

delegate_xdg_foreign!(DesComp);
