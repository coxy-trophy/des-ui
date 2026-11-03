//! Explicit USB-only screen sessions. Pairing and lock policy belong to the caller.
//!
//! `running` means that the owned subprocess is alive, not that a video frame has
//! been observed. Poll status and stop on desktop lock, peer loss or USB loss.
use serde::Serialize;
use std::{
    env, fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::{fs::{FileTypeExt, MetadataExt, PermissionsExt}, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const LOG_LIMIT: usize = 4096;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const SCRCPY_LOCAL: &str = ".local/share/des-connect/tools/scrcpy-linux-x86_64-v5.0.1/scrcpy";

#[derive(Clone, Debug, Serialize)]
pub struct ScreenStatus {
    pub mode: String,
    pub serial: Option<String>,
    pub running: bool,
    pub control: bool,
    pub message: String,
}
impl Default for ScreenStatus {
    fn default() -> Self {
        Self { mode: "idle".into(), serial: None, running: false, control: false, message: "No screen session".into() }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ScreenCapabilities {
    pub phone_mirror: bool,
    pub extended_output: bool,
    pub desktop_mirror: bool,
    pub message: String,
}
struct ExtendedSession {
    socket: PathBuf,
    compositor_pid: rustix::process::Pid,
    serial: String,
    port: u16,
    adb: PathBuf,
    // The lease prevents another DES Connect instance owning this output.
    lease: PathBuf,
}

pub struct ScreenBridge {
    child: Option<Child>,
    log: Arc<Mutex<Vec<u8>>>,
    state: ScreenStatus,
    extended: Option<ExtendedSession>,
    last_usb_check: Instant,
}
impl Default for ScreenBridge { fn default() -> Self { Self::new() } }
impl ScreenBridge {
    pub fn new() -> Self {
        Self { child: None, log: Arc::default(), state: ScreenStatus::default(), extended: None, last_usb_check: Instant::now() }
    }
    pub fn capabilities(&self) -> ScreenCapabilities {
        let phone_mirror = find_scrcpy().is_some() && find_executable("adb").is_some();
        let extended = matching_virtual_socket();
        let extended_output = extended.is_ok() && find_helper().is_some() && find_executable("python3").is_some() && find_executable("adb").is_some();
        ScreenCapabilities {
            phone_mirror, extended_output, desktop_mirror: false,
            message: format!("Phone mirror: {}. Extended output: {}. Physical desktop mirroring is unavailable.",
                if phone_mirror { "USB scrcpy available" } else { "install scrcpy and adb" },
                if extended_output { "experimental USB output available".into() } else { extended.err().unwrap_or_else(|| "viewer helper, Python 3 or adb unavailable".into()) }),
        }
    }
    /// Caller-only compatibility entrypoint. Services should use the guarded
    /// variant so lock and peer policy is revalidated after USB preparation.
    pub fn start_phone(&mut self, serial: &str, control: bool) -> Result<ScreenStatus, String> {
        self.start_phone_guarded(serial, control, || Ok(()))
    }
    /// Revalidate live caller policy after USB preparation, immediately before
    /// spawning the viewer. No viewer or input starts when the guard rejects.
    pub fn start_phone_guarded(&mut self, serial: &str, control: bool, guard: impl FnOnce() -> Result<(), String>) -> Result<ScreenStatus, String> {
        self.ensure_idle()?;
        validate_serial(serial)?;
        let scrcpy = find_scrcpy().ok_or("scrcpy unavailable: install the pinned official release; see connect/SCREEN-BRIDGE.md")?;
        let adb = find_executable("adb").ok_or("ADB unavailable; install Android platform-tools")?;
        self.start_phone_with_tools(serial, control, scrcpy, adb, guard)
    }
    fn start_phone_with_tools(&mut self, serial: &str, control: bool, scrcpy: PathBuf, adb: PathBuf, guard: impl FnOnce() -> Result<(), String>) -> Result<ScreenStatus, String> {
        usb_probe_with(&adb, serial, PROBE_TIMEOUT)?;
        let mut command = Command::new(&scrcpy);
        command.args(phone_args(serial, control)).env("ADB", &adb);
        // The portable distribution resolves scrcpy-server relative to itself.
        if let Some(parent) = scrcpy.parent() {
            let server = parent.join("scrcpy-server");
            if server.is_file() { command.env("SCRCPY_SERVER_PATH", server); }
        }
        self.launch_guarded(command, "phone", serial, control, None, guard)
    }
    /// Caller-only compatibility entrypoint; prefer `start_extended_guarded` in
    /// a service that enforces current desktop lock and paired-peer policy.
    pub fn start_extended(&mut self, serial: &str, width: u32, height: u32, fps: u32) -> Result<ScreenStatus, String> {
        self.start_extended_guarded(serial, width, height, fps, || Ok(()))
    }
    /// The final guard runs after USB/socket/helper preparation. A rejection
    /// releases the prepared lease without starting a viewer or reverse mapping.
    pub fn start_extended_guarded(&mut self, serial: &str, width: u32, height: u32, fps: u32, guard: impl FnOnce() -> Result<(), String>) -> Result<ScreenStatus, String> {
        self.ensure_idle()?;
        validate_serial(serial)?;
        if !(320..=1920).contains(&width) || !(240..=1200).contains(&height) || !(1..=30).contains(&fps) {
            return Err("Extended output must be 320–1920 × 240–1200 at 1–30 fps".into());
        }
        let adb = require_usb(serial)?;
        let (socket, compositor_pid) = matching_virtual_socket()?;
        let helper = find_helper().ok_or("USB virtual display helper unavailable")?;
        let python = find_executable("python3").ok_or("Python 3 unavailable")?;
        let lease = socket.with_extension("des-connect-lease");
        let lease_file = fs::OpenOptions::new().write(true).create_new(true).open(&lease)
            .map_err(|e| format!("Cannot reserve virtual output (remove a stale lease only after checking other viewers): {e}"))?;
        if let Err(e) = lease_file.set_permissions(fs::Permissions::from_mode(0o600)) { let _ = fs::remove_file(&lease); return Err(e.to_string()); }
        // Refuse to replace a manually started viewer or an output owned elsewhere.
        let available = socket_reply(&socket, "frame", 128).map(|reply| reply == b"STOPPED\n");
        if available != Ok(true) { let _ = fs::remove_file(&lease); return Err("Virtual output is active, locked, or unavailable; stop its existing viewer first".into()); }
        let port = match TcpListener::bind(("127.0.0.1", 0)).and_then(|listener| listener.local_addr().map(|a| a.port())) {
            Ok(p) => p, Err(e) => { let _ = fs::remove_file(&lease); return Err(e.to_string()); }
        };
        let mut command = Command::new(python);
        command.arg(helper).arg("--socket").arg(&socket).arg("--serial").arg(serial)
            .arg("--width").arg(width.to_string()).arg("--height").arg(height.to_string())
            .arg("--fps").arg(fps.to_string()).arg("--port").arg(port.to_string());
        // The existing helper calls adb by name; provide only the resolved adb directory.
        let adb_dir = adb.parent().ok_or("adb needs an absolute path")?;
        command.env("PATH", adb_dir);
        self.launch_guarded(command, "extended", serial, false, Some(ExtendedSession { socket, compositor_pid, serial: serial.into(), port, adb, lease }), guard)
    }
    fn ensure_idle(&mut self) -> Result<(), String> {
        if self.status().running { Err("Stop the current screen session before starting another".into()) } else { Ok(()) }
    }
    #[cfg(test)]
    fn launch(&mut self, command: Command, mode: &str, serial: &str, control: bool, extended: Option<ExtendedSession>) -> Result<ScreenStatus, String> {
        self.launch_guarded(command, mode, serial, control, extended, || Ok(()))
    }
    fn discard_prepared_extended(&mut self) {
        // Before spawn neither the output nor the ADB mapping has been created
        // by us. Only our own lease exists, so do not send stop/remove commands.
        if let Some(session) = self.extended.take() { let _ = fs::remove_file(session.lease); }
    }
    fn launch_guarded(&mut self, mut command: Command, mode: &str, serial: &str, control: bool, extended: Option<ExtendedSession>, guard: impl FnOnce() -> Result<(), String>) -> Result<ScreenStatus, String> {
        self.log = Arc::default();
        self.extended = extended;
        command.process_group(0).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Err(error) = guard() { self.discard_prepared_extended(); return Err(error); }
        match command.spawn() {
            Ok(mut child) => {
                drain(child.stdout.take().unwrap(), self.log.clone());
                drain(child.stderr.take().unwrap(), self.log.clone());
                self.child = Some(child);
                self.last_usb_check = Instant::now();
                self.state = ScreenStatus { mode: mode.into(), serial: Some(serial.into()), running: true, control, message: "Starting USB screen session; waiting for viewer".into() };
                Ok(self.status())
            }
            Err(e) => { self.discard_prepared_extended(); Err(format!("Could not start screen viewer: {e}")) }
        }
    }
    pub fn status(&mut self) -> ScreenStatus {
        // The browser helper stays alive after unplugging USB. End that session
        // explicitly; reconnection always requires another user request.
        if self.child.is_some() && self.last_usb_check.elapsed() >= Duration::from_secs(2) {
            self.last_usb_check = Instant::now();
            if let Some(serial) = self.state.serial.clone() {
                if let Err(error) = usb_probe(&serial, Duration::from_millis(500)) {
                    let mut ended = self.state.clone();
                    let _ = self.stop();
                    ended.running = false;
                    ended.control = false;
                    ended.message = format!("USB disconnected or unavailable: {error}. Select the device again to reconnect.");
                    self.state = ended;
                    return self.state.clone();
                }
            }
        }
        let result = self.child.as_mut().map(Child::try_wait);
        match result {
            Some(Ok(Some(exit))) => {
                self.child.take();
                let deadline = Instant::now() + Duration::from_millis(50);
                while Arc::strong_count(&self.log) > 1 && Instant::now() < deadline { thread::sleep(Duration::from_millis(2)); }
                self.cleanup_extended();
                self.state.running = false;
                self.state.control = false;
                self.state.message = format!("Screen session ended ({exit}). {}", self.diagnostics());
            }
            Some(Err(e)) => {
                let _ = self.stop();
                self.state.message = format!("Screen process status failed: {e}");
            }
            Some(Ok(None)) => {
                let log = self.diagnostics();
                if !log.is_empty() { self.state.message = log; }
            }
            None => {}
        }
        self.state.clone()
    }
    pub fn stop(&mut self) -> Result<ScreenStatus, String> {
        let result = self.child.take().map(|mut child| terminate(&mut child)).transpose();
        self.cleanup_extended();
        self.state = ScreenStatus::default();
        result.map(|_| self.state.clone())
    }
    fn diagnostics(&self) -> String {
        let bytes = self.log.lock().unwrap_or_else(|e| e.into_inner());
        // Strip terminal escape/control characters before returning to the UI.
        String::from_utf8_lossy(&bytes).chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect::<String>().trim().into()
    }
    fn cleanup_extended(&mut self) {
        if let Some(session) = self.extended.take() {
            // Never send stop to a replacement compositor reusing this socket path.
            if peer_pid(&session.socket).ok() == Some(session.compositor_pid) {
                let _ = socket_reply(&session.socket, "stop", 128);
            }
            let mut command = Command::new(session.adb);
            command.args(["-s", &session.serial, "reverse", "--remove", &format!("tcp:{}", session.port)]);
            let _ = run_bounded(&mut command, Duration::from_millis(500));
            let _ = fs::remove_file(session.lease);
        }
    }
}
impl Drop for ScreenBridge { fn drop(&mut self) { let _ = self.stop(); } }

fn validate_serial(serial: &str) -> Result<(), String> {
    if serial.is_empty() || serial.len() > 128 || serial.starts_with('-') || !serial.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
        return Err("Select a USB ADB serial (1–128 ASCII letters, digits, dot, underscore or hyphen); network serials are refused".into());
    }
    Ok(())
}
fn phone_args(serial: &str, control: bool) -> Vec<String> {
    let mut args: Vec<String> = [format!("--serial={serial}"), "--no-audio".into(), "--no-power-on".into(), "--no-clipboard-autosync".into(), "--max-size=1920".into(), "--max-fps=30".into(), "--window-title=DES Connect: Android screen".into()].into();
    if !control { args.push("--no-control".into()); }
    args
}
fn require_usb(serial: &str) -> Result<PathBuf, String> { usb_probe(serial, PROBE_TIMEOUT) }
fn usb_probe(serial: &str, timeout: Duration) -> Result<PathBuf, String> {
    let adb = find_executable("adb").ok_or("ADB unavailable; install Android platform-tools")?;
    usb_probe_with(&adb, serial, timeout)?;
    Ok(adb)
}
fn usb_probe_with(adb: &Path, serial: &str, timeout: Duration) -> Result<(), String> {
    let mut command = Command::new(adb);
    command.args(["-s", serial, "get-devpath"]);
    let result = run_bounded(&mut command, timeout)?;
    if !result.trim().starts_with("usb:") { return Err("Selected serial is not a connected USB device; authorize USB debugging and reconnect".into()); }
    Ok(())
}
fn find_scrcpy() -> Option<PathBuf> {
    env::var_os("DES_CONNECT_SCRCPY").map(PathBuf::from).filter(|p| p.is_absolute() && executable(p))
        .or_else(|| env::var_os("HOME").map(PathBuf::from).map(|p| p.join(SCRCPY_LOCAL)).filter(|p| executable(p)))
        .or_else(|| find_executable("scrcpy"))
}
fn find_helper() -> Option<PathBuf> {
    env::var_os("DES_CONNECT_VIEWER").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file())
        .or_else(|| { let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/virtual-display-viewer.py"); p.is_file().then_some(p) })
        .or_else(|| { let p = PathBuf::from("/usr/local/share/des-ui/virtual-display-viewer.py"); p.is_file().then_some(p) })
}
fn executable(path: &Path) -> bool { fs::metadata(path).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0) }
fn find_executable(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| env::split_paths(&value).filter(|p| p.is_absolute()).map(|p| p.join(name)).find(|p| executable(p)))
}
fn matching_virtual_socket() -> Result<(PathBuf, rustix::process::Pid), String> {
    let socket = env::var_os("DES_VIRTUAL_SOCKET").map(PathBuf::from).ok_or("Current compositor has no opt-in DES_VIRTUAL_SOCKET")?;
    let meta = fs::symlink_metadata(&socket).map_err(|e| format!("Virtual output socket unavailable: {e}"))?;
    let parent = socket.parent().ok_or("Virtual socket needs a private directory")?;
    let dir = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    let uid = rustix::process::geteuid().as_raw();
    if !socket.is_absolute() || !meta.file_type().is_socket() || meta.uid() != uid || meta.mode() & 0o077 != 0 || !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o077 != 0 {
        return Err("Virtual socket must be this user's private socket in a mode-0700 directory".into());
    }
    let compositor = if let Some(path) = env::var_os("DES_COMP_SOCKET") { PathBuf::from(path) } else {
        let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR unavailable")?;
        let display = env::var_os("WAYLAND_DISPLAY").ok_or("WAYLAND_DISPLAY unavailable")?;
        let name = Path::new(&display).file_name().ok_or("Invalid WAYLAND_DISPLAY")?.to_string_lossy();
        PathBuf::from(runtime).join(format!("des-comp-{name}.sock"))
    };
    let virtual_pid = peer_pid(&socket)?;
    if peer_pid(&compositor)? != virtual_pid { return Err("Virtual output belongs to another compositor; select the current desktop session".into()); }
    Ok((socket, virtual_pid))
}
// A hung compositor must not block the service's lock/disconnect handling.
// AF_UNIX nonblocking connect either connects immediately or returns an error
// (including a full listener backlog); no background connector thread is leaked.
fn connect_local(path: &Path) -> Result<UnixStream, String> {
    use rustix::net::{socket_with, connect, AddressFamily, SocketType, SocketFlags, SocketAddrUnix};
    let fd = socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK, None).map_err(|e| e.to_string())?;
    let address = SocketAddrUnix::new(path).map_err(|e| e.to_string())?;
    connect(&fd, &address).map_err(|e| e.to_string())?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    Ok(stream)
}
fn peer_pid(path: &Path) -> Result<rustix::process::Pid, String> {
    let stream = connect_local(path).map_err(|e| format!("Cannot connect to compositor {}: {e}", path.display()))?;
    let cred = rustix::net::sockopt::socket_peercred(&stream).map_err(|e| e.to_string())?;
    if cred.uid != rustix::process::geteuid() { return Err("Compositor socket belongs to another user".into()); }
    Ok(cred.pid)
}
fn socket_reply(path: &Path, command: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut stream = connect_local(path)?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_millis(300))).map_err(|e| e.to_string())?;
    stream.write_all(format!("{command}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    stream.take(limit).read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}
fn append_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() >= LOG_LIMIT { tail.clear(); tail.extend_from_slice(&bytes[bytes.len()-LOG_LIMIT..]); }
    else { let remove = (tail.len()+bytes.len()).saturating_sub(LOG_LIMIT); tail.drain(..remove); tail.extend_from_slice(bytes); }
}
fn drain(mut reader: impl Read + Send + 'static, tail: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || { let mut bytes = [0; 1024]; while let Ok(n) = reader.read(&mut bytes) { if n == 0 { break; } append_tail(&mut tail.lock().unwrap_or_else(|e| e.into_inner()), &bytes[..n]); } });
}
fn terminate(child: &mut Child) -> Result<(), String> {
    if child.try_wait().map_err(|e| e.to_string())?.is_some() { return Ok(()); }
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) { let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM); }
    let deadline = Instant::now()+Duration::from_millis(300);
    while Instant::now() < deadline {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() { return Ok(()); }
        thread::sleep(Duration::from_millis(10));
    }
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) { let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL); }
    child.kill().map_err(|e| e.to_string())?;
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}
fn run_bounded(command: &mut Command, timeout: Duration) -> Result<String, String> {
    command.process_group(0).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("Cannot run USB tool: {e}"))?;
    let stdout: Arc<Mutex<Vec<u8>>> = Arc::default();
    let stderr: Arc<Mutex<Vec<u8>>> = Arc::default();
    drain(child.stdout.take().unwrap(), stdout.clone()); drain(child.stderr.take().unwrap(), stderr.clone());
    let deadline = Instant::now()+timeout;
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                // Reader threads may still be draining the final short message.
                for _ in 0..5 { if Arc::strong_count(&stdout)==1 && Arc::strong_count(&stderr)==1 { break; } thread::sleep(Duration::from_millis(2)); }
                if !exit.success() { return Err(format!("USB tool failed ({exit}): {}", String::from_utf8_lossy(&stderr.lock().unwrap_or_else(|e| e.into_inner())).trim())); }
                return Ok(String::from_utf8_lossy(&stdout.lock().unwrap_or_else(|e| e.into_inner())).into());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => { let _ = terminate(&mut child); return Err("USB tool timed out; check authorization and cable".into()); }
            Err(e) => { let _ = terminate(&mut child); return Err(e.to_string()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serial_rejects_network_and_argument_injection() {
        for invalid in ["", "-x", "phone;touch /tmp/pwn", "192.168.1.2:5555", "a\nb", "a b"] {
            assert!(validate_serial(invalid).is_err(), "{invalid:?}");
        }
        assert!(validate_serial("5203a_B-12.3").is_ok());
    }
    #[test]
    fn viewing_never_enables_input_audio_clipboard_or_power_changes() {
        let args = phone_args("USB123", false);
        for flag in ["--no-control", "--no-audio", "--no-clipboard-autosync", "--no-power-on", "--max-size=1920", "--max-fps=30"] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(!phone_args("USB123", true).iter().any(|a| a == "--no-control"));
    }
    #[test]
    fn bounded_process_timeout_reaps_only_its_own_child() {
        let mut command = Command::new("/bin/sleep"); command.arg("20");
        let start = Instant::now();
        assert!(run_bounded(&mut command, Duration::from_millis(50)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn diagnostics_are_bounded_and_nonzero_exit_is_reported() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf 'USB disconnected' >&2; exit 7"]);
        let error = run_bounded(&mut command, Duration::from_secs(1)).unwrap_err();
        assert!(error.contains("USB disconnected")); assert!(error.contains('7'));
        let mut tail = Vec::new(); append_tail(&mut tail, &vec![b'x'; 100_000]);
        assert_eq!(tail.len(), LOG_LIMIT);
    }
    #[test]
    fn stopping_and_drop_reap_owned_children() {
        let mut bridge = ScreenBridge::new();
        let mut cmd = Command::new("/bin/sleep"); cmd.arg("20");
        bridge.launch(cmd, "phone", "TEST", false, None).unwrap();
        assert!(bridge.status().running);
        assert!(!bridge.stop().unwrap().running);
        assert!(bridge.child.is_none());
    }
    #[test]
    fn drop_reaps_the_owned_child_and_leaves_an_unrelated_process_alone() {
        let mut unrelated = Command::new("/bin/sleep").arg("20").spawn().unwrap();
        let pid;
        {
            let mut bridge = ScreenBridge::new();
            let mut cmd = Command::new("/bin/sleep"); cmd.arg("20");
            bridge.launch(cmd, "phone", "TEST", false, None).unwrap();
            pid = bridge.child.as_ref().unwrap().id();
        }
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap(); unrelated.wait().unwrap();
    }
    #[test]
    fn poll_reports_viewer_failure_without_restarting_it() {
        let mut bridge = ScreenBridge::new();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "printf 'USB disconnected' >&2; exit 9"]);
        bridge.launch(cmd, "phone", "TEST", false, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while bridge.status().running && Instant::now() < deadline { thread::sleep(Duration::from_millis(10)); }
        let state = bridge.status();
        assert!(!state.running); assert!(state.message.contains("USB disconnected"));
        assert!(bridge.child.is_none());
    }

    #[test]
    fn local_socket_connect_is_nonblocking_and_reports_peer_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("viewer.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let stream = connect_local(&path).unwrap();
        let cred = rustix::net::sockopt::socket_peercred(&stream).unwrap();
        assert_eq!(cred.pid, rustix::process::getpid());
        assert_eq!(cred.uid, rustix::process::geteuid());
        drop(listener);
        assert!(connect_local(&path).is_err());
    }

    fn fake_tool(path: &Path, script: &str) {
        fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn final_guard_rejects_policy_changed_during_delayed_usb_preparation() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let launched = dir.path().join("viewer-started");
        let adb = dir.path().join("adb");
        let viewer = dir.path().join("viewer");
        fs::write(&allowed, b"unlocked and paired").unwrap();
        fake_tool(&adb, &format!("/bin/sleep 0.1\n/bin/rm -- '{}'\nprintf 'usb:fixture\\n'", allowed.display()));
        fake_tool(&viewer, &format!("/usr/bin/touch -- '{}'", launched.display()));
        let mut bridge = ScreenBridge::new();
        let called = std::cell::Cell::new(0);
        let error = bridge.start_phone_with_tools("TEST", false, viewer, adb, || {
            called.set(called.get()+1);
            if allowed.exists() { Ok(()) } else { Err("desktop locked or peer disconnected".into()) }
        }).unwrap_err();
        assert!(error.contains("desktop locked or peer disconnected"));
        assert_eq!(called.get(), 1);
        assert!(!launched.exists());
        assert!(!bridge.status().running);
        assert!(bridge.child.is_none());
    }
    #[test]
    fn final_guard_rejection_releases_only_prepared_extended_lease() {
        let dir = tempfile::tempdir().unwrap();
        let lease = dir.path().join("output.des-connect-lease");
        let launched = dir.path().join("viewer-started");
        let adb_called = dir.path().join("adb-called");
        let adb = dir.path().join("adb");
        let viewer = dir.path().join("viewer");
        fs::write(&lease, b"").unwrap();
        fake_tool(&adb, &format!("/usr/bin/touch -- '{}'", adb_called.display()));
        fake_tool(&viewer, &format!("/usr/bin/touch -- '{}'", launched.display()));
        let mut bridge = ScreenBridge::new();
        let prepared = ExtendedSession { socket: dir.path().join("unused.sock"), compositor_pid: rustix::process::getpid(), serial: "TEST".into(), port: 8767, adb, lease: lease.clone() };
        let error = bridge.launch_guarded(Command::new(viewer), "extended", "TEST", false, Some(prepared), || Err("locked".into())).unwrap_err();
        assert_eq!(error, "locked");
        assert!(!lease.exists());
        assert!(!launched.exists());
        assert!(!adb_called.exists());
        assert!(bridge.extended.is_none());
        assert!(bridge.child.is_none());
    }

}
