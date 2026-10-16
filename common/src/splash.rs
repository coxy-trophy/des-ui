//! How the compositor's splash goes: what the shell needs to know to take over from
//! it without a seam.

use std::time::Duration;

/// How long the splash's bar stays full, once the shell has its window, before the
/// splash starts to fade.
pub const FULL_FOR: Duration = Duration::from_millis(180);
/// How long the fade takes.
pub const FADE: Duration = Duration::from_millis(260);
