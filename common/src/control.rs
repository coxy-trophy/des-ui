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
