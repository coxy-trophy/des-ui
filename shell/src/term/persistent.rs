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
// Printable delimiters survive tmux output sanitization in non-UTF8 locales.
const SESSION_FORMAT: &str = "#{session_name}|#{pane_pid}";
const DIMENSIONS_FORMAT: &str = "#{pane_height}|#{pane_width}";
fn role_of(id: &str) -> Option<SessionRole> {
    let (tail, role) = if let Some(tail) = id.strip_prefix("des-tab-") { (tail, SessionRole::Tab) }
        else { (id.strip_prefix("des-scratch-")?, SessionRole::Scratchpad) };
    (tail.len() == 32 && tail.bytes().all(|b| b.is_ascii_hexdigit())).then_some(role)
}
fn base(dir: &Path) -> Command {
    let mut c = Command::new("tmux");
    c.arg("-N").arg("-u").arg("-S").arg(dir.join("tmux.sock")).args(["-f", "/dev/null"]);
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
    live_sessions_in(&dir)
}
fn live_sessions_in(dir: &Path) -> anyhow::Result<Vec<SessionRef>> {
    if !dir.join("tmux.sock").exists() { return Ok(Vec::new()); }
    // A foreground server can have no sessions yet. list-panes needs a
    // current target even with -a; list-sessions is target-free and supplies
    // each managed session's active pane through its format defaults.
    let out = run(dir, &args(&["list-sessions", "-F", SESSION_FORMAT]))?;
    Ok(parse_sessions(&out))
}
fn parse_sessions(out: &[u8]) -> Vec<SessionRef> {
    String::from_utf8_lossy(out).lines().filter_map(|line| {
        let (id, pid) = line.split_once('|')?;
        Some(SessionRef { id: id.into(), role: role_of(id)?, pane_pid: pid.parse().ok()? })
    }).collect()
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
/// The backend owns history, while the outer terminal sees a redrawn screen.
/// Keep wheel handling in tmux and return ordinary typing to the live program.
fn scroll_configuration() -> Vec<OsString> {
    args(&["set-option", "-g", "mouse", "on", ";",
        "set-window-option", "-g", "mode-keys", "emacs", ";",
        "bind-key", "-n", "WheelUpPane", "if-shell", "-F", "#{||:#{pane_in_mode},#{mouse_any_flag}}", "send-keys -M", "if-shell -F '#{alternate_on}' 'send-keys Up' 'copy-mode -e'", ";",
        "bind-key", "-n", "WheelDownPane", "if-shell", "-F", "#{||:#{pane_in_mode},#{mouse_any_flag}}", "send-keys -M", "if-shell -F '#{alternate_on}' 'send-keys Down'", ";",
        "unbind-key", "-a", "-T", "copy-mode", ";",
        "bind-key", "-T", "copy-mode", "WheelUpPane", "send-keys", "-X", "scroll-up", ";",
        "bind-key", "-T", "copy-mode", "WheelDownPane", "send-keys", "-X", "scroll-down", ";",
        "bind-key", "-T", "copy-mode", "Escape", "send-keys", "-X", "cancel", ";",
        // No arguments re-injects the original key, including Unicode/control keys.
        "bind-key", "-T", "copy-mode", "Any", "send-keys -X cancel ; send-keys", ";"])
}
/// A single command queue applies server defaults before starting the pane.
fn create_prefix(scrollback: usize) -> Vec<OsString> {
    let mut commands=args(&["set-option", "-g", "status", "off", ";",
        "set-option", "-g", "prefix", "None", ";", "set-option", "-g", "prefix2", "None", ";",
        "set-option", "-g", "default-terminal", "xterm-256color", ";",
        "set-option", "-g", "history-limit", &scrollback.min(10000).to_string(), ";",
        "set-window-option", "-g", "allow-passthrough", "on", ";"]);
    commands.extend(scroll_configuration());
    commands
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
    attach_in(&dir, session)
}
fn attach_in(dir: &Path, session: &SessionRef) -> anyhow::Result<(CommandBuilder, Vec<u8>, (u16, u16))> {
    let target = format!("={}", session.id);
    // A pane target needs the session separator. Bare =name is interpreted
    // as a pane/window name; display-message can silently return empty fields.
    let pane_target = format!("{}:", target);
    // Upgrade existing sessions on reconnect, in the same bounded helper queue.
    let mut inspect=scroll_configuration();
    inspect.extend(args(&["display-message", "-p", "-t", &pane_target, DIMENSIONS_FORMAT]));
    let dimensions = run(dir, &inspect)?;
    let size = parse_dimensions(&dimensions)?;
    let history = run(dir, &args(&["capture-pane", "-p", "-e", "-t", &pane_target, "-S", "-1000", "-E", "-1"]))?;
    let mut cmd = CommandBuilder::new("tmux");
    cmd.arg("-N"); cmd.arg("-u"); cmd.arg("-S"); cmd.arg(dir.join("tmux.sock")); cmd.args(["-f", "/dev/null", "attach-session", "-t", &target]);
    cmd.env_remove("TMUX");
    Ok((cmd, history_bytes(&history), size))
}
fn parse_dimensions(bytes: &[u8]) -> anyhow::Result<(u16, u16)> {
    let dimensions = std::str::from_utf8(bytes)?;
    let (rows, cols) = dimensions.trim().split_once('|').ok_or_else(|| anyhow::anyhow!("invalid pane dimensions"))?;
    Ok((rows.parse::<u16>()?.clamp(1, 1000), cols.parse::<u16>()?.clamp(1, 1000)))
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
        assert!(live_sessions_in(dir).expect("empty backend must enumerate without a current target").is_empty());
        let marker = dir.join("literal");
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf '%s' \"$1\" > literal; printf 'LIVE_RESTORE_MARKER\\n'; read answer; exit", "sh", "$(touch injected); spaced ' value"]);
        command.cwd(dir);
        let session = create(dir, &command, SessionRole::Tab, 100).expect("private tmux session must start");
        let deadline = Instant::now() + DEADLINE;
        while !marker.exists() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "$(touch injected); spaced ' value");
        assert!(!dir.join("injected").exists());
        let live = live_sessions_in(dir).expect("created terminal must enumerate");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, session.id);
        assert_eq!(live[0].pane_pid, session.pane_pid);
        let target = format!("={}", session.id);
        for _ in 0..2 {
            // Exercise the same dimension/history/attach preparation as the GUI.
            let (attach, _, (rows, cols)) = attach_in(dir, &session).expect("GUI attach must resolve this session's pane");
            assert!(rows > 0 && cols > 0);
            let mut tab = super::super::TermTab::spawn_with(attach, eframe::egui::Context::default(), 100).unwrap();
            let deadline = Instant::now() + DEADLINE;
            loop {
                tab.pump();
                assert!(!tab.exited, "tmux attach client exited instead of reconnecting");
                if tab.parser.screen().contents().contains("LIVE_RESTORE_MARKER") { break; }
                assert!(Instant::now() < deadline, "reattached GUI did not show the live pane");
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(tab);
            let pid = run(dir, &args(&["display-message", "-p", "-t", &format!("{target}:"), "#{pane_pid}"])).unwrap();
            assert_eq!(String::from_utf8(pid).unwrap().trim(), session.pane_pid.to_string());
        }
        run(dir, &args(&["send-keys", "-t", &format!("{target}:"), "Enter"])).unwrap();
        let deadline = Instant::now() + DEADLINE;
        while run(dir, &args(&["has-session", "-t", &target])).is_ok() {
            assert!(Instant::now() < deadline, "exiting inner shell left a session behind");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(run(dir, &args(&["has-session", "-t", &target])).is_err());
        assert!(live_sessions_in(dir).expect("backend must enumerate after its last terminal exits").is_empty());
    }
    #[test]
    fn managed_wheel_scrolls_backend_history_and_typing_returns_live() {
        use eframe::egui::{Event, Modifiers, MouseWheelUnit, vec2};
        let runtime=des_common::terminal::Runtime::new().expect("private backend must start");
        let dir=runtime.path();
        let mut command=CommandBuilder::new("/bin/sh");
        command.args(["-c", "seq 1 200; printf 'READY_TO_SCROLL'; IFS= read -r answer; printf '%s' \"$answer\" > received"]);
        command.cwd(dir);
        let session=create(dir,&command,SessionRole::Tab,500).unwrap();
        let (attach,_,_)=attach_in(dir,&session).unwrap();
        let mut tab=super::super::TermTab::spawn_with(attach,eframe::egui::Context::default(),500).unwrap();
        let deadline=Instant::now()+DEADLINE;
        loop {
            tab.pump();
            if tab.parser.screen().contents().contains("READY_TO_SCROLL") {break;}
            assert!(Instant::now()<deadline,"live pane did not appear");
            std::thread::sleep(Duration::from_millis(10));
        }
        let event=Event::MouseWheel{unit:MouseWheelUnit::Line,delta:vec2(0.0,60.0),modifiers:Modifiers::NONE};
        let bytes=tab.wheel.handle(tab.parser.screen_mut(),&event,(10,10),20.0,24);
        tab.write(&bytes);
        let target=format!("={}:",session.id);
        let deadline=Instant::now()+DEADLINE;
        loop {
            tab.pump();
            let mode=run(dir,&args(&["display-message","-p","-t",&target,"#{pane_in_mode}"])).unwrap();
            if mode==b"1\n" && !tab.parser.screen().contents().contains("READY_TO_SCROLL") {break;}
            assert!(Instant::now()<deadline,"wheel did not enter and scroll backend history");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!dir.join("received").exists(),"wheel must not submit shell input");
        tab.input(b"typed-after-scroll\n");
        let deadline=Instant::now()+DEADLINE;
        while !dir.join("received").exists() {
            tab.pump();
            assert!(Instant::now()<deadline,"typing remained trapped in copy mode");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(dir.join("received")).unwrap(),"typed-after-scroll");
    }

    #[test]
    fn managed_fullscreen_wheel_reaches_mouse_and_arrow_applications() {
        use eframe::egui::{Event, Modifiers, MouseWheelUnit, vec2};
        for mouse in [false,true] {
            for up in [false,true] {
                let runtime=des_common::terminal::Runtime::new().expect("private backend must start");
                let dir=runtime.path();
                let mut command=CommandBuilder::new("python3");
                let setup=if mouse {"os.write(1,b'\\x1b[?1000h\\x1b[?1006h');"}else{""};
                command.args(["-c",&format!("import os,tty,pathlib;tty.setraw(0);os.write(1,b'\\x1b[?1049h\\x1b[H');{setup}os.write(1,b'APP_READY');data=os.read(0,64);pathlib.Path('packet').write_bytes(data)")]);
                command.cwd(dir);
                let session=create(dir,&command,SessionRole::Tab,100).unwrap();
                let (attach,_,_)=attach_in(dir,&session).unwrap();
                let mut tab=super::super::TermTab::spawn_with(attach,eframe::egui::Context::default(),100).unwrap();
                let deadline=Instant::now()+DEADLINE;
                loop {
                    tab.pump();
                    if tab.parser.screen().contents().contains("APP_READY") {break;}
                    assert!(Instant::now()<deadline,"full-screen app did not appear");
                    std::thread::sleep(Duration::from_millis(10));
                }
                let event=Event::MouseWheel{unit:MouseWheelUnit::Line,delta:vec2(0.0,if up{1.0}else{-1.0}),modifiers:Modifiers::NONE};
                let bytes=tab.wheel.handle(tab.parser.screen_mut(),&event,(10,10),20.0,24);
                assert!(!bytes.is_empty());tab.write(&bytes);
                let deadline=Instant::now()+DEADLINE;
                while !dir.join("packet").exists() {
                    tab.pump();
                    assert!(Instant::now()<deadline,"wheel never reached the full-screen program");
                    std::thread::sleep(Duration::from_millis(10));
                }
                let expected=if mouse {format!("\x1b[<{};11;11M",if up{64}else{65}).into_bytes()}else if up{b"\x1b[A".to_vec()}else{b"\x1b[B".to_vec()};
                assert_eq!(std::fs::read(dir.join("packet")).unwrap(),expected);
            }
        }
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
        assert!(text.starts_with(&["set-option", "-g", "status", "off", ";", "set-option", "-g", "prefix", "None", ";", "set-option", "-g", "prefix2", "None", ";", "set-option", "-g", "default-terminal", "xterm-256color", ";", "set-option", "-g", "history-limit", "3456", ";", "set-window-option", "-g", "allow-passthrough", "on", ";"]));
        assert!(text.windows(4).any(|words|words==["set-option","-g","mouse","on"]));
        assert!(text.windows(6).any(|words|words==["bind-key","-T","copy-mode","Any","send-keys -X cancel ; send-keys",";"]));
    }
    #[test]
    fn metadata_survives_tmux_non_utf8_client_sanitization() {
        // tmux's non-UTF8 client output maps control bytes, including tabs,
        // to underscores. Exercise our actual format strings through that rule.
        let sanitize = |text: String| text.chars().map(|ch| if (' '..='~').contains(&ch) { ch } else { '_' }).collect::<String>();
        let dimensions = sanitize(DIMENSIONS_FORMAT.replace("#{pane_height}", "24").replace("#{pane_width}", "80"));
        assert_eq!(parse_dimensions(dimensions.as_bytes()).unwrap(), (24, 80));
        let id = "des-tab-0123456789abcdef0123456789abcdef";
        let sessions = sanitize(SESSION_FORMAT.replace("#{session_name}", id).replace("#{pane_pid}", "12345"));
        let parsed = parse_sessions(sessions.as_bytes());
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, id);
        assert_eq!(parsed[0].pane_pid, 12345);
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
