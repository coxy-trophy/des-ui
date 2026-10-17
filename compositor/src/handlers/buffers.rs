//! Two ways for applications to say less with their buffers: a viewport crops and
//! scales a buffer to the size it is shown at, which fractional scaling relies on,
//! and a single-pixel buffer is a plain colour with no memory behind it. The
//! renderer honours both once the globals exist.

use smithay::{delegate_single_pixel_buffer, delegate_viewporter};

use crate::DesComp;

delegate_viewporter!(DesComp);
delegate_single_pixel_buffer!(DesComp);
