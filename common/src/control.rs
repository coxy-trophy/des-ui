//! Where the shell listens for `des-ui-ctl`, agreed between the two programs.

use std::path::PathBuf;

/// Names another socket, for trying things out beside a shell that is in use.
pub const SOCKET_ENV: &str = "DES_UI_CONTROL_SOCKET";

/// The shell's control socket: `$XDG_RUNTIME_DIR/des-ui-control.sock`, or without a
/// runtime directory `$TMPDIR/des-ui-<uid>/des-ui-control.sock`, unless
/// [`SOCKET_ENV`] says otherwise.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return PathBuf::from(path);
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("des-ui-{}", rustix::process::geteuid().as_raw())))
        .join("des-ui-control.sock")
}

/// A connection-scoped request lifetime, shared with the UI holding its reply.
#[derive(Clone, Default)]
pub struct Cancellation(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) { self.0.store(true, std::sync::atomic::Ordering::Release); }
    pub fn is_cancelled(&self) -> bool { self.0.load(std::sync::atomic::Ordering::Acquire) }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancellation_is_shared_only_with_the_originating_request() {
        let first = Cancellation::default();
        let held = first.clone();
        let other = Cancellation::default();
        first.cancel();
        assert!(held.is_cancelled());
        assert!(!other.is_cancelled());
    }
}
