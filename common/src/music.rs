//! Where the music service listens, agreed between it and the shell.

use std::path::PathBuf;

/// Names another socket, for trying things out beside a service that is in use.
pub const SOCKET_ENV: &str = "DES_MUSIC_SOCKET";

/// The service's socket: `$XDG_RUNTIME_DIR/des-music.sock`, or without a runtime
/// directory `$TMPDIR/des-music-<uid>/des-music.sock`, unless [`SOCKET_ENV`] says
/// otherwise.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return PathBuf::from(path);
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("des-music-{}", rustix::process::geteuid().as_raw())))
        .join("des-music.sock")
}
