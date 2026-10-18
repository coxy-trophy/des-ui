//! Private tmux sessions belonging to this compositor lifetime.
//! Every client uses -N: an absent compositor-owned server is a visible failure,
//! never permission to launch a detached replacement server.
use portable_pty::CommandBuilder;
use std::{ffi::OsString, io::Read, path::Path, process::{Command, Stdio}, time::{Duration, Instant}};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole { Tab, Scratchpad }
#[derive(Debug, Clone)]
pub struct SessionRef { pub id: String, pub role: SessionRole, pub pane_pid: u32 }
const LIMIT: usize = 2 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(3);
fn role_of(id: &str) -> Option<SessionRole> {
    let (tail, role) = if let Some(tail) = id.strip_prefix("des-tab-") { (tail, SessionRole::Tab) }
        else { (id.strip_prefix("des-scratch-")?, SessionRole::Scratchpad) };
    (tail.len() == 32 && tail.bytes().all(|b| b.is_ascii_hexdigit())).then_some(role)
}
fn base(dir: &Path) -> Command {
    let mut c = Command::new("tmux");
    c.arg("-N").arg("-S").arg(dir.join("tmux.sock")).args(["-f", "/dev/null"]);
    c.env_remove("TMUX"); c
}
/// Bound both wall time and retained output, including a stalled server.
fn run(dir: &Path, args: &[OsString]) -> anyhow::Result<Vec<u8>> {
    let mut command = base(dir);
    command.args(args);
    collect(command)
}
fn collect(mut command: Command) -> anyhow::Result<Vec<u8>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    for fd in [&out as &dyn std::os::fd::AsFd, &err as &dyn std::os::fd::AsFd] {
        let flags = match rustix::fs::fcntl_getfl(fd) { Ok(flags) => flags, Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error.into()); } };
        if let Err(error) = rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK) {
            let _ = child.kill(); let _ = child.wait(); return Err(error.into());
        }
    }
    let mut readers: [Box<dyn Read>; 2] = [Box::new(out), Box::new(err)];
    let mut buffers = [Vec::new(), Vec::new()];
    let mut eof = [false; 2];
    let deadline = Instant::now() + DEADLINE;
    let mut status = None;
    loop {
        for i in 0..2 {
            if eof[i] { continue; }
            let mut chunk = [0u8; 8192];
            match readers[i].read(&mut chunk) {
                Ok(0) => eof[i] = true,
                Ok(n) => {
                    if buffers[i].len() + n > LIMIT {
                        let _ = child.kill(); let _ = child.wait(); anyhow::bail!("private terminal backend output exceeded limit");
                    }
                    buffers[i].extend_from_slice(&chunk[..n]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
                Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error.into()); }
            }
        }
        if status.is_none() { status = match child.try_wait() { Ok(status) => status, Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error.into()); } }; }
        if status.is_some() && eof.iter().all(|end| *end) { break; }
        if Instant::now() >= deadline { let _ = child.kill(); let _ = child.wait(); anyhow::bail!("private terminal backend timed out"); }
        std::thread::sleep(Duration::from_millis(1));
    }
    let status = status.unwrap();
    let [output, error] = buffers;
    anyhow::ensure!(status.success(), "private terminal backend: {}", String::from_utf8_lossy(&error));
    Ok(output)
}
fn args(words: &[&str]) -> Vec<OsString> { words.iter().map(OsString::from).collect() }
pub fn live_sessions() -> anyhow::Result<Vec<SessionRef>> {
    let Some(dir) = des_common::terminal::directory()? else { return Ok(Vec::new()); };
    if !dir.join("tmux.sock").exists() { return Ok(Vec::new()); }
    let out = run(&dir, &args(&["list-panes", "-a", "-F", "#{session_name}\t#{pane_pid}"]))?;
    Ok(String::from_utf8_lossy(&out).lines().filter_map(|line| {
        let (id, pid) = line.split_once('\t')?;
        Some(SessionRef { id: id.into(), role: role_of(id)?, pane_pid: pid.parse().ok()? })
    }).collect())
}
// tmux's argv parser treats even an argument's trailing semicolon as a
// command separator. Escape that one byte for tmux, without shell quoting.
fn literal_arg(value: &std::ffi::OsStr) -> OsString {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut bytes = value.as_bytes().to_vec();
    if bytes.last() == Some(&b';') { bytes.insert(bytes.len() - 1, b'\\'); }
    OsString::from_vec(bytes)
}
fn terminal_overrides(cmd: &CommandBuilder) -> Vec<OsString> {
    let mut a = Vec::new();
    for key in ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "ZDOTDIR", "DES_UI_USER_ZDOTDIR", "XDG_DATA_DIRS"] {
        if let Some(value) = cmd.get_env(key) {
            let mut assignment = OsString::from(key); assignment.push("="); assignment.push(value);
            a.push("-e".into()); a.push(assignment);
        }
    }
    a.push("-e".into()); a.push("DES_UI_PERSISTENT_TERMINAL=1".into());
    a
}
/// A single command queue applies server defaults before starting the pane.
fn create_prefix(scrollback: usize) -> Vec<OsString> {
    args(&["set-option", "-g", "status", "off", ";",
        "set-option", "-g", "prefix", "None", ";", "set-option", "-g", "prefix2", "None", ";",
        "set-option", "-g", "default-terminal", "xterm-256color", ";",
        "set-option", "-g", "history-limit", &scrollback.min(10000).to_string(), ";",
        "set-window-option", "-g", "allow-passthrough", "on", ";"])
}
pub(super) fn rollback(dir: &Path, id: &str) {
    if role_of(id).is_some() {
        let _ = run(dir, &args(&["kill-session", "-t", &format!("={id}")]));
    }
}
pub(super) fn create(dir: &Path, cmd: &CommandBuilder, role: SessionRole, scrollback: usize) -> anyhow::Result<SessionRef> {
    let mut random = [0u8;16]; std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let id = format!("des-{}-{token}", if role == SessionRole::Tab { "tab" } else { "scratch" });
    let mut a = create_prefix(scrollback);
    let pane_start = a.len();
    a.extend(args(&["new-session", "-d", "-P", "-F", "#{pane_pid}", "-s", &id, "-x", "80", "-y", "24"]));
    if let Some(cwd) = cmd.get_cwd() { a.push("-c".into()); a.push(cwd.clone()); }
    // Only explicit, non-secret terminal/integration overrides enter tmux argv.
    // The private server inherits the GUI environment without serialising it.
    a.extend(terminal_overrides(cmd));
    // Multiple command arguments force direct exec, including a program with no args.
    a.push("/usr/bin/env".into()); a.push("--".into());
    a.extend(cmd.get_argv().iter().cloned());
    for word in &mut a[pane_start..] { *word = literal_arg(word); }
    let result = (|| -> anyhow::Result<SessionRef> {
        let result = run(dir, &a)?;
        let pane_pid = String::from_utf8(result)?.trim().parse()?;
        Ok(SessionRef { id: id.clone(), role, pane_pid })
    })();
    if result.is_err() { rollback(dir, &id); }
    result
}
pub(super) fn attach(session: &SessionRef) -> anyhow::Result<(CommandBuilder, Vec<u8>, (u16, u16))> {
    anyhow::ensure!(role_of(&session.id) == Some(session.role), "invalid terminal identity");
    let dir = des_common::terminal::directory()?.ok_or_else(|| anyhow::anyhow!("terminal runtime unavailable"))?;
    let target = format!("={}", session.id);
    let dimensions = run(&dir, &args(&["display-message", "-p", "-t", &target, "#{pane_height}\t#{pane_width}"]))?;
    let dimensions = String::from_utf8(dimensions)?;
    let (rows, cols) = dimensions.trim().split_once('\t').ok_or_else(|| anyhow::anyhow!("invalid pane dimensions"))?;
    let size = (rows.parse::<u16>()?.clamp(1, 1000), cols.parse::<u16>()?.clamp(1, 1000));
    let history = run(&dir, &args(&["capture-pane", "-p", "-e", "-t", &target, "-S", "-1000", "-E", "-1"]))?;
    let mut cmd = CommandBuilder::new("tmux");
    cmd.arg("-N"); cmd.arg("-S"); cmd.arg(dir.join("tmux.sock")); cmd.args(["-f", "/dev/null", "attach-session", "-t", &target]);
    cmd.env_remove("TMUX");
    Ok((cmd, history_bytes(&history), size))
}
fn history_bytes(capture: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(capture.len() + capture.len() / 80);
    for byte in capture {
        if *byte == b'\n' && bytes.last() != Some(&b'\r') { bytes.push(b'\r'); }
        bytes.push(*byte);
    }
    bytes
}
pub(super) fn foreground_pid(stat: &str) -> Option<u32> {
    let fields = stat.rsplit_once(") ")?.1;
    fields.split_whitespace().nth(5)?.parse::<u32>().ok().filter(|p| *p > 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_session_survives_client_detach_and_literal_arguments() {
        let runtime = des_common::terminal::Runtime::new().expect("compositor-owned tmux server must start");
        let dir = runtime.path();
        let marker = dir.join("literal");
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf '%s' \"$1\" > literal; read answer; exit", "sh", "$(touch injected); spaced ' value"]);
        command.cwd(dir);
        let session = create(dir, &command, SessionRole::Tab, 100).expect("private tmux session must start");
        let deadline = Instant::now() + DEADLINE;
        while !marker.exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "$(touch injected); spaced ' value");
        assert!(!dir.join("injected").exists());
        let target = format!("={}", session.id);
        for _ in 0..2 {
            let mut attach = CommandBuilder::new("tmux");
            attach.arg("-N"); attach.arg("-S"); attach.arg(dir.join("tmux.sock"));
            attach.args(["-f", "/dev/null", "attach-session", "-t", &target]);
            let tab = super::super::TermTab::spawn_with(attach, eframe::egui::Context::default(), 100).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            drop(tab);
            let pid = run(dir, &args(&["display-message", "-p", "-t", &target, "#{pane_pid}"])).unwrap();
            assert_eq!(String::from_utf8(pid).unwrap().trim(), session.pane_pid.to_string());
        }
        run(dir, &args(&["send-keys", "-t", &target, "Enter"])).unwrap();
        let deadline = Instant::now() + DEADLINE;
        while run(dir, &args(&["has-session", "-t", &target])).is_ok() {
            assert!(Instant::now() < deadline, "exiting inner shell left a session behind");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(run(dir, &args(&["has-session", "-t", &target])).is_err());
    }
    #[test]
    fn all_helper_clients_forbid_automatic_server_creation() {
        let command = base(Path::new("/tmp/example"));
        assert_eq!(command.get_args().next(), Some(std::ffi::OsStr::new("-N")));
    }
    #[test]
    fn literal_argv_escapes_tmux_command_separators_only() {
        assert_eq!(literal_arg(std::ffi::OsStr::new(";")), "\\;");
        assert_eq!(literal_arg(std::ffi::OsStr::new("foo;")), "foo\\;");
        assert_eq!(literal_arg(std::ffi::OsStr::new("foo;bar")), "foo;bar");
        assert_eq!(literal_arg(std::ffi::OsStr::new("foo\\;")), "foo\\\\;");
    }
    #[test]
    fn session_defaults_are_configured_before_any_pane_starts() {
        let commands = create_prefix(3456);
        let text: Vec<_> = commands.iter().map(|word| word.to_str().unwrap()).collect();
        assert_eq!(text, ["set-option", "-g", "status", "off", ";", "set-option", "-g", "prefix", "None", ";", "set-option", "-g", "prefix2", "None", ";", "set-option", "-g", "default-terminal", "xterm-256color", ";", "set-option", "-g", "history-limit", "3456", ";", "set-window-option", "-g", "allow-passthrough", "on", ";"]);
    }
    #[test]
    fn helper_output_is_bounded() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 3000000 /dev/zero"]);
        assert!(collect(command).unwrap_err().to_string().contains("output exceeded limit"));
    }
    #[test]
    fn descendant_holding_output_open_cannot_block_helper() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 4 & exit 0"]);
        let started = Instant::now();
        assert!(collect(command).unwrap_err().to_string().contains("timed out"));
        assert!(started.elapsed() < DEADLINE + Duration::from_secs(1));
    }
    #[test]
    fn environment_overrides_never_serialize_secrets() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.env("SECRET_TOKEN", "super-secret-marker");
        cmd.env("TERM", "xterm-256color");
        let overrides = terminal_overrides(&cmd);
        assert!(overrides.iter().any(|v| v == "TERM=xterm-256color"));
        assert!(overrides.iter().all(|v| !v.to_string_lossy().contains("secret")));
    }
    #[test]
    fn captured_history_starts_each_line_in_column_zero() {
        let mut parser = vt100::Parser::new(4, 20, 10);
        parser.process(&history_bytes(b"first\nsecond\n"));
        assert_eq!(parser.screen().rows(0, 20).take(2).collect::<Vec<_>>(), ["first", "second"]);
    }
    #[test]
    fn session_names_are_strict_and_role_scoped() {
        assert_eq!(role_of("des-tab-0123456789abcdef0123456789abcdef"), Some(SessionRole::Tab));
        assert_eq!(role_of("des-scratch-0123456789abcdef0123456789abcdef"), Some(SessionRole::Scratchpad));
        for id in ["other", "des-tab-../../x", "des-tab-0123", "des-tab-0123456789abcdef0123456789abcdeg"] {
            assert_eq!(role_of(id), None);
        }
    }
    #[test]
    fn proc_stat_handles_spaces_and_parentheses_in_name() {
        assert_eq!(foreground_pid("21 (a ) b) S 1 21 21 34816 87 0 0"), Some(87));
        assert_eq!(foreground_pid("21 (sh) S 1 21 21 0 -1 0"), None);
    }
}
