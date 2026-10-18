//! A compositor's private live-terminal backend lifetime.
use std::{fs, io::{self, Read}, os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt}, path::{Component, Path, PathBuf}, process::{Child, Command, ExitStatus, Stdio}, time::{Duration, Instant}};

pub const ENV: &str = "DES_UI_TERMINAL_DIR";

/// Standalone shells have no managed backend. Never accept a shared or redirected directory.
pub fn directory() -> io::Result<Option<PathBuf>> {
    std::env::var_os(ENV).map(|value| {
        let path = PathBuf::from(value);
        validate(&path)?;
        Ok(path)
    }).transpose()
}

fn validate(path: &Path) -> io::Result<fs::Metadata> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal directory must be absolute"));
    }
    let mut part = PathBuf::new();
    for component in path.components() {
        part.push(component);
        if fs::symlink_metadata(&part)?.file_type().is_symlink() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "terminal directory must not contain symlinks"));
        }
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o7777 != 0o700 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "terminal directory must be a user-owned mode 0700 directory"));
    }
    Ok(meta)
}

/// Owned by the compositor, never by a replaceable graphical shell.
pub struct Runtime {
    path: PathBuf,
    device: u64,
    inode: u64,
    server: Option<Child>,
    socket_identity: Option<(u64, u64)>,
}
impl Runtime {
    pub fn new() -> io::Result<Self> {
        let parent = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
            .filter(|path| validate(path).is_ok()).unwrap_or_else(|| PathBuf::from("/tmp"));
        let mut runtime = match Self::create(&parent) {
            Ok(runtime) => runtime,
            // A sandbox can expose an otherwise valid XDG runtime directory as
            // read-only. Access restrictions permit the same private /tmp fallback
            // used when XDG is absent; allocation/I/O errors still reach the caller.
            Err(error) if parent != Path::new("/tmp") && matches!(error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem) =>
                Self::create(Path::new("/tmp"))?,
            Err(error) => return Err(error),
        };
        let socket = runtime.path.join("tmux.sock");
        // -D keeps the server in the compositor/watchdog process group and turns
        // exit-empty off. Never let a client daemonize a replacement server.
        let mut command = Command::new("tmux");
        command
            .args(["-D", "-S"]).arg(&socket).args(["-f", "/dev/null"])
            .env_remove("TMUX").stdin(Stdio::null()).stdout(Stdio::null())
            .stderr(Stdio::null());
        bind_to_parent(&mut command);
        runtime.server = Some(command.spawn()?);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = runtime.server.as_mut().unwrap().try_wait()? {
                return Err(io::Error::other(format!("private tmux server exited during startup: {status}")));
            }
            if let Ok(meta) = fs::symlink_metadata(&socket) {
                use std::os::unix::fs::FileTypeExt;
                if !meta.file_type().is_socket() || meta.uid() != rustix::process::geteuid().as_raw() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid private tmux socket"));
                }
                fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
                runtime.socket_identity = Some((meta.dev(), meta.ino()));
                // allow-passthrough requires tmux 3.3+, the backend minimum.
                let mut probe = Command::new("tmux");
                probe.args(["-N", "-S"]).arg(&socket)
                    .args(["-f", "/dev/null", "show-window-options", "-g", "-v", "allow-passthrough"])
                    .env_remove("TMUX").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                if wait_bounded(&mut probe.spawn()?, Duration::from_millis(500))?.success() {
                    return Ok(runtime);
                }
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "private tmux server did not become ready"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn create(parent: &Path) -> io::Result<Self> {
        for _ in 0..8 {
            let mut random = [0u8; 16];
            fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
            let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = parent.join(format!("des-ui-terminal-{suffix}"));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    // Establish the exact contract even under an unusually strict umask.
                    if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o700)) {
                        let _ = fs::remove_dir(&path);
                        return Err(error);
                    }
                    let meta = match validate(&path) {
                        Ok(meta) => meta,
                        Err(error) => { let _ = fs::remove_dir(&path); return Err(error); }
                    };
                    return Ok(Self { path, device: meta.dev(), inode: meta.ino(), server: None, socket_identity: None });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "cannot allocate private terminal directory"))
    }
    pub fn path(&self) -> &Path { &self.path }
}
/// Keep the foreground server tied to the compositor even if only its PID is
/// terminated and no Rust destructors can run.
fn bind_to_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let compositor = rustix::process::getpid();
    // SAFETY: after fork this closure only invokes async-signal-safe syscalls.
    // Capture the parent before fork and check after prctl to close the death race.
    unsafe {
        command.pre_exec(move || {
            rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
            if rustix::process::getppid() != Some(compositor) {
                return Err(io::Error::from_raw_os_error(10)); // ECHILD; no allocation.
            }
            Ok(())
        });
    }
}

/// Every helper and owned server wait is bounded, including shutdown failure.
fn wait_bounded(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {},
            Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error); }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "private tmux child timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // Even if the directory was replaced, terminate/reap our retained child.
        let owns_directory = validate(&self.path).is_ok_and(|meta|
            (meta.dev(), meta.ino()) == (self.device, self.inode));
        let socket = self.path.join("tmux.sock");
        let owns_socket = owns_directory && fs::symlink_metadata(&socket).is_ok_and(|meta| {
            use std::os::unix::fs::FileTypeExt;
            meta.file_type().is_socket() && meta.uid() == rustix::process::geteuid().as_raw()
                && meta.mode() & 0o7777 == 0o600
                && self.socket_identity == Some((meta.dev(), meta.ino()))
        });
        if owns_socket {
            if let Ok(mut child) = Command::new("tmux").args(["-N", "-S"]).arg(&socket)
                .args(["-f", "/dev/null", "kill-server"]).env_remove("TMUX")
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                let _ = wait_bounded(&mut child, Duration::from_millis(500));
            }
        }
        if let Some(mut server) = self.server.take() {
            let _ = wait_bounded(&mut server, Duration::from_millis(750));
        }
        if owns_directory { let _ = fs::remove_dir_all(&self.path); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn parent_death_signal_survives_exec() {
        // Query in the exec'd process's main thread: the Linux setting is per
        // thread, so a libtest helper's newly spawned test thread cannot inspect it.
        let mut command = Command::new("python3");
        command.args(["-c", "import ctypes, signal; value = ctypes.c_int(); assert ctypes.CDLL(None).prctl(2, ctypes.byref(value), 0, 0, 0) == 0; assert value.value == signal.SIGKILL, value.value"])
            .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        bind_to_parent(&mut command);
        let mut child = command.spawn().unwrap();
        let status = wait_bounded(&mut child, Duration::from_secs(5)).unwrap();
        assert!(status.success(), "parent-death helper exited with {status}");
    }
    #[test]
    fn retained_foreground_child_shares_watchdog_group_and_is_reaped() {
        let mut runtime = Runtime::create(Path::new("/tmp")).unwrap();
        // Controlled fake server: no sockets, real terminal commands or user tmux.
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let group = |path: &Path| {
            fs::read_to_string(path).unwrap().rsplit_once(')').unwrap().1
                .split_whitespace().nth(2).unwrap().to_owned()
        };
        assert_eq!(group(Path::new("/proc/self/stat")),
            group(&PathBuf::from(format!("/proc/{pid}/stat"))));
        runtime.server = Some(child);
        let path = runtime.path().to_owned();
        drop(runtime);
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert!(!path.exists());
    }
    #[test]
    fn private_unique_lifetime_removes_only_its_directory() {
        let first = Runtime::create(Path::new("/tmp")).unwrap();
        let second = Runtime::create(Path::new("/tmp")).unwrap();
        assert_ne!(first.path(), second.path());
        assert!(validate(first.path()).is_ok());
        fs::write(first.path().join("layout.json"), "{}").unwrap();
        let path = first.path().to_owned();
        drop(first);
        assert!(!path.exists());
        assert!(second.path().is_dir());
    }
    #[test]
    fn rejects_relative_shared_file_and_symlink_paths() {
        assert!(validate(Path::new("relative")).is_err());
        let runtime = Runtime::create(Path::new("/tmp")).unwrap();
        let file = runtime.path().join("file");
        fs::write(&file, "").unwrap();
        assert!(validate(&file).is_err());
        let link = runtime.path().join("link");
        symlink(runtime.path(), &link).unwrap();
        assert!(validate(&link).is_err());
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate(runtime.path()).is_err());
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn replaced_directory_is_never_removed() {
        let runtime = Runtime::create(Path::new("/tmp")).unwrap();
        let original = runtime.path().to_owned();
        let moved = original.with_extension("original");
        fs::rename(&original, &moved).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&original).unwrap();
        drop(runtime);
        assert!(original.exists());
        fs::remove_dir(original).unwrap();
        fs::remove_dir(moved).unwrap();
    }
}
