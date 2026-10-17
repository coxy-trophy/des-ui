//! A private socket where `des-ui-ctl` asks the shell for things, in a closed
//! vocabulary. The compositor refuses everyone but the shell; this is how a script or
//! a key binding reaches the desktop. Only this user's own processes are answered,
//! each request is bounded in size and time, and nothing in a request is ever run.
//!
//! A client sends one line, such as `workspace 2` or `toast Build finished`, and reads
//! one line back: `ok`, `ok <json>` for `state`, or `error <why>`.

use des_common::Panel;
use des_common::ipc::{DesktopState, Place, ShotMode, WORKSPACES};
use eframe::egui;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::Duration;

const MAX_CLIENTS: usize = 8;
const MAX_LINE: usize = 4096;
/// How long a client may take to send a line, and the shell to answer it.
const REQUEST_TIME: Duration = Duration::from_secs(10);
/// How long an answer may wait on the user: a picture being marked out, a pixel chosen.
const INTERACTION_TIME: Duration = Duration::from_secs(120);
const WRITE_TIME: Duration = Duration::from_secs(5);
const MAX_TOAST: usize = 512;
const MAX_SECTION: usize = 32;
/// Requests waiting for the interface to look at them.
const QUEUE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MusicWord { Toggle, Next, Prev }

impl MusicWord {
    /// The word the music service takes.
    pub fn word(self) -> &'static str {
        match self { MusicWord::Toggle => "toggle", MusicWord::Next => "next", MusicWord::Prev => "prev" }
    }
}

/// What a client asked for. Each is carried out by the interface, never here.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Ping,
    State,
    Lock,
    Workspace(usize),
    Toast(String),
    Dock,
    Launcher,
    Settings(Option<String>),
    Music(MusicWord),
    Panel(Panel),
    Screenshot(ShotMode),
    /// A screenshot whose saved file the client is told of: the answer is
    /// `{"uri": "file:///..."}` once the picture is on disk, or `error cancelled`.
    ScreenshotFile(ShotMode),
    /// The colour of a pixel the user picks: `{"r": 0-255, "g": 0-255, "b": 0-255}`,
    /// or `error cancelled`.
    PickColor,
    /// Drop the scratchpad terminal, or put it away.
    Scratchpad,
    /// Open the overview of every workspace, or close it.
    Overview,
}

impl Request {
    /// How long the interface may take over this before the client is told it did not
    /// answer. Requests that wait on the user get the time a person takes.
    pub fn deadline(&self) -> Duration {
        match self {
            Request::ScreenshotFile(_) | Request::PickColor => INTERACTION_TIME,
            _ => REQUEST_TIME,
        }
    }
}

/// What the client is told.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer { Ok, Json(String), Error(String) }

impl Answer {
    fn line(&self) -> String {
        match self {
            Answer::Ok => "ok\n".to_string(),
            Answer::Json(json) => format!("ok {json}\n"),
            Answer::Error(why) => format!("error {}\n", why.replace(['\n', '\r'], " ")),
        }
    }
}

/// The way back to the client that asked; answered once, after the request is done.
pub struct Reply(SyncSender<Answer>);

impl Reply {
    pub fn send(self, answer: Answer) {
        let _ = self.0.send(answer);
    }
}

/// Reads one line of a request.
pub fn parse(line: &str) -> Result<Request, String> {
    let line = line.trim();
    let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let none = |request: Request| if rest.is_empty() { Ok(request) } else { Err(format!("'{verb}' takes nothing after it")) };
    match verb {
        "ping" => none(Request::Ping),
        "state" => none(Request::State),
        "lock" => none(Request::Lock),
        "dock" => none(Request::Dock),
        "launcher" => none(Request::Launcher),
        "workspace" => match rest.parse::<usize>() {
            Ok(n) if n < WORKSPACES => Ok(Request::Workspace(n)),
            _ => Err(format!("'workspace' takes a number from 0 to {}", WORKSPACES - 1)),
        },
        "toast" => {
            let text = sanitize_toast(rest);
            if text.is_empty() { Err("'toast' takes the text to show".to_string()) } else { Ok(Request::Toast(text)) }
        }
        "settings" => {
            if rest.is_empty() {
                Ok(Request::Settings(None))
            } else if rest.chars().count() <= MAX_SECTION && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '&') {
                Ok(Request::Settings(Some(rest.to_string())))
            } else {
                Err("'settings' takes a section name: letters, digits, spaces and '&'".to_string())
            }
        }
        "music" => match rest {
            "toggle" => Ok(Request::Music(MusicWord::Toggle)),
            "next" => Ok(Request::Music(MusicWord::Next)),
            "prev" => Ok(Request::Music(MusicWord::Prev)),
            _ => Err("'music' takes toggle, next or prev".to_string()),
        },
        "panel" => match rest {
            "left" => Ok(Request::Panel(Panel::Left)),
            "right" => Ok(Request::Panel(Panel::Right)),
            "bottom" => Ok(Request::Panel(Panel::Bottom)),
            _ => Err("'panel' takes left, right or bottom".to_string()),
        },
        "screenshot" => match rest {
            "" | "screen" => Ok(Request::Screenshot(ShotMode::Screen)),
            "window" => Ok(Request::Screenshot(ShotMode::Window)),
            "region" => Ok(Request::Screenshot(ShotMode::Region)),
            _ => Err("'screenshot' takes screen, window or region".to_string()),
        },
        "screenshot-file" => match rest {
            "" | "screen" => Ok(Request::ScreenshotFile(ShotMode::Screen)),
            "window" => Ok(Request::ScreenshotFile(ShotMode::Window)),
            "region" => Ok(Request::ScreenshotFile(ShotMode::Region)),
            _ => Err("'screenshot-file' takes screen, window or region".to_string()),
        },
        "pick-color" => none(Request::PickColor),
        "scratchpad" => none(Request::Scratchpad),
        "overview" => none(Request::Overview),
        "" => Err("nothing asked".to_string()),
        other => Err(format!("unknown request '{}'; des-ui-ctl --help lists them", other.chars().take(32).collect::<String>())),
    }
}

/// Toast text as it may be shown: no control characters, and not endless.
pub fn sanitize_toast(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_TOAST)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A local path as a `file://` URI, every byte but the unreserved ones and `/`
/// percent-encoded, as the desktop portal hands it to applications.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => uri.push(byte as char),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The answer to a held `screenshot-file`, once the picture is saved or was not.
pub fn shot_answer(saved: &Result<std::path::PathBuf, String>) -> Answer {
    match saved {
        Ok(path) => Answer::Json(serde_json::json!({ "uri": file_uri(path) }).to_string()),
        Err(why) => Answer::Error(why.clone()),
    }
}

/// The answer to a held `pick-color`: the colour, or that it was called off.
pub fn colour_answer(picked: Option<[u8; 3]>) -> Answer {
    match picked {
        Some([r, g, b]) => Answer::Json(serde_json::json!({ "r": r, "g": g, "b": b }).to_string()),
        None => Answer::Error("cancelled".into()),
    }
}

/// Whether a connection may be answered: the kernel must say it is this user's.
pub fn peer_allowed(peer_uid: Option<u32>, own_uid: u32) -> bool {
    peer_uid == Some(own_uid)
}

/// The desktop as `state` reports it.
pub fn state_json(state: Option<&DesktopState>, locked: bool) -> String {
    let place = |place: Place| match place { Place::Hidden => "hidden", Place::Main => "main", Place::Side => "side" };
    let apps: Vec<serde_json::Value> = state.map(|s| &s.apps[..]).unwrap_or(&[]).iter().enumerate().map(|(index, app)| {
        serde_json::json!({
            "index": index,
            "app_id": app.app_id,
            "title": app.title,
            "screen": app.screen,
            "workspace": app.workspace,
            "place": place(app.place),
        })
    }).collect();
    serde_json::json!({
        "connected": state.is_some(),
        "locked": locked,
        "screens": state.map_or(1, |s| s.screens),
        "workspace": state.map_or(0, |s| s.workspace),
        "focus": state.and_then(|s| s.focus),
        "terminal_split": state.is_some_and(|s| s.terminal_split),
        "panels": {
            "left": state.is_none_or(|s| s.panels.left),
            "right": state.is_none_or(|s| s.panels.right),
            "bottom": state.is_none_or(|s| s.panels.bottom),
        },
        "apps": apps,
    }).to_string()
}

/// The listening end, owned by the interface.
pub struct Control {
    requests: Receiver<(Request, Reply)>,
    /// Held for as long as the shell listens; a second shell cannot take the socket.
    _lock: std::fs::File,
    path: PathBuf,
}

impl Control {
    /// Starts listening where `des-ui-ctl` looks.
    pub fn start(ctx: egui::Context) -> io::Result<Control> {
        Control::start_at(ctx, des_common::control::socket_path())
    }

    pub fn start_at(ctx: egui::Context, path: PathBuf) -> io::Result<Control> {
        prepare_parent(&path)?;
        let lock = lock(&path)?;
        remove_stale(&path)?;
        let listener = UnixListener::bind(&path)?;
        restrict(&path)?;
        let (sender, requests) = mpsc::sync_channel(QUEUE);
        let clients = Arc::new(AtomicUsize::new(0));
        let own_uid = rustix::process::geteuid().as_raw();
        std::thread::Builder::new().name("des-ui-control".into()).spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let peer_uid = rustix::net::sockopt::socket_peercred(&stream).ok().map(|c| c.uid.as_raw());
                if !peer_allowed(peer_uid, own_uid) {
                    continue;
                }
                let _ = stream.set_write_timeout(Some(WRITE_TIME));
                if clients.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
                    clients.fetch_sub(1, Ordering::SeqCst);
                    let _ = (&stream).write_all(Answer::Error("busy".into()).line().as_bytes());
                    continue;
                }
                let guard = Counted(clients.clone());
                let sender = sender.clone();
                let ctx = ctx.clone();
                let _ = std::thread::Builder::new().name("des-ui-control-client".into()).spawn(move || {
                    let _guard = guard;
                    serve(stream, &sender, &ctx);
                });
            }
        })?;
        Ok(Control { requests, _lock: lock, path })
    }

    /// What clients have asked since the last frame. Answer each through its reply.
    pub fn take(&mut self) -> Vec<(Request, Reply)> {
        let mut taken = Vec::new();
        while let Ok(item) = self.requests.try_recv() {
            taken.push(item);
        }
        taken
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = remove_stale(&self.path);
    }
}

/// One fewer client when the connection's thread ends, however it ends.
struct Counted(Arc<AtomicUsize>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One connection: lines in, answers out, until the client goes or misbehaves.
fn serve(stream: UnixStream, sender: &SyncSender<(Request, Reply)>, ctx: &egui::Context) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIME));
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_line_bounded(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => return,
            Err(error) => {
                let _ = out.write_all(Answer::Error(error.to_string()).line().as_bytes());
                return;
            }
        };
        let answer = match parse(&line) {
            Err(why) => Answer::Error(why),
            Ok(request) => {
                let (reply, answered) = mpsc::sync_channel(1);
                let deadline = request.deadline();
                match sender.try_send((request, Reply(reply))) {
                    Ok(()) => {
                        ctx.request_repaint();
                        match answered.recv_timeout(deadline) {
                            Ok(answer) => answer,
                            Err(mpsc::RecvTimeoutError::Timeout) => Answer::Error("the shell did not answer in time".into()),
                            Err(mpsc::RecvTimeoutError::Disconnected) => Answer::Error("the shell did not answer".into()),
                        }
                    }
                    Err(TrySendError::Full(_)) => Answer::Error("busy".into()),
                    Err(TrySendError::Disconnected(_)) => Answer::Error("the shell is going".into()),
                }
            }
        };
        if out.write_all(answer.line().as_bytes()).is_err() {
            return;
        }
    }
}

/// One line, or `None` at the end; an error when it goes on past the limit.
fn read_line_bounded(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    reader.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("a request is at most {MAX_LINE} bytes")));
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

// The endpoint is kept as the music service keeps its own: a private directory, a
// lock held for the shell's lifetime, and a stale socket replaced only when it is a
// socket of this user's.

fn prepare_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the control socket needs an absolute path"))?;
    match std::fs::DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the control socket's directory must be this user's, mode 0700"));
    }
    Ok(())
}

fn lock(path: &Path) -> io::Result<std::fs::File> {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(lock_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the control socket's lock must be a private file"));
    }
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    Ok(file)
}

fn remove_stale(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refusing to replace what is not this user's socket"));
    }
    std::fs::remove_file(path)
}

fn restrict(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use des_common::ipc::AppInfo;
    use std::time::Instant;

    /// A private folder for a socket; the temporary folder's own mode follows the umask.
    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn reads_every_request_and_refuses_the_rest() {
        assert_eq!(parse("ping\n"), Ok(Request::Ping));
        assert_eq!(parse("state"), Ok(Request::State));
        assert_eq!(parse("lock"), Ok(Request::Lock));
        assert_eq!(parse("dock"), Ok(Request::Dock));
        assert_eq!(parse("launcher"), Ok(Request::Launcher));
        assert_eq!(parse("workspace 3"), Ok(Request::Workspace(3)));
        assert_eq!(parse("toast  Build finished "), Ok(Request::Toast("Build finished".into())));
        assert_eq!(parse("settings"), Ok(Request::Settings(None)));
        assert_eq!(parse("settings date & region"), Ok(Request::Settings(Some("date & region".into()))));
        assert_eq!(parse("music next"), Ok(Request::Music(MusicWord::Next)));
        assert_eq!(parse("panel bottom"), Ok(Request::Panel(Panel::Bottom)));
        assert_eq!(parse("screenshot"), Ok(Request::Screenshot(ShotMode::Screen)));
        assert_eq!(parse("screenshot window"), Ok(Request::Screenshot(ShotMode::Window)));
        assert_eq!(parse("screenshot-file"), Ok(Request::ScreenshotFile(ShotMode::Screen)));
        assert_eq!(parse("screenshot-file region"), Ok(Request::ScreenshotFile(ShotMode::Region)));
        assert_eq!(parse("scratchpad"), Ok(Request::Scratchpad));
        assert_eq!(parse("overview"), Ok(Request::Overview));
        assert!(parse("overview all").is_err());
        assert!(parse("scratchpad now").is_err());
        assert_eq!(parse("pick-color"), Ok(Request::PickColor));
        for bad in ["", "reboot", "workspace 4", "workspace x", "workspace", "toast", "toast \t\n",
                    "music loud", "panel top", "screenshot pick", "screenshot-file pick", "pick-color now", "settings $(id)", "lock now", "ping pong",
                    &format!("settings {}", "a".repeat(33))] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(parse("rm -rf /").unwrap_err().contains("unknown request"));
    }

    #[test]
    fn what_waits_on_the_user_is_given_the_time_a_person_takes() {
        assert_eq!(Request::Ping.deadline(), REQUEST_TIME);
        assert_eq!(Request::Screenshot(ShotMode::Screen).deadline(), REQUEST_TIME);
        assert_eq!(Request::ScreenshotFile(ShotMode::Region).deadline(), INTERACTION_TIME);
        assert_eq!(Request::PickColor.deadline(), INTERACTION_TIME);
    }

    #[test]
    fn toast_text_loses_control_characters_and_its_tail() {
        assert_eq!(sanitize_toast(" one\x1b[31m two\n"), "one [31m two");
        let long = "é".repeat(MAX_TOAST + 50);
        assert_eq!(sanitize_toast(&long).chars().count(), MAX_TOAST);
    }

    #[test]
    fn saved_files_are_uris_that_survive_any_name() {
        assert_eq!(file_uri(Path::new("/home/me/Pictures/Screenshots/a.png")), "file:///home/me/Pictures/Screenshots/a.png");
        assert_eq!(file_uri(Path::new("/tmp/my shot é.png")), "file:///tmp/my%20shot%20%C3%A9.png");
        assert_eq!(file_uri(Path::new("/x/a#b?c%")), "file:///x/a%23b%3Fc%25");
        assert_eq!(
            shot_answer(&Ok(std::path::PathBuf::from("/tmp/a b.png"))),
            Answer::Json(r#"{"uri":"file:///tmp/a%20b.png"}"#.into())
        );
        assert_eq!(shot_answer(&Err("disk full".into())), Answer::Error("disk full".into()));
        assert_eq!(colour_answer(Some([1, 2, 3])), Answer::Json(r#"{"b":3,"g":2,"r":1}"#.into()));
        assert_eq!(colour_answer(None), Answer::Error("cancelled".into()));
    }

    #[test]
    fn only_this_user_is_answered() {
        assert!(peer_allowed(Some(1000), 1000));
        assert!(!peer_allowed(Some(0), 1000));
        assert!(!peer_allowed(None, 1000));
    }

    #[test]
    fn lines_are_bounded() {
        let mut short = io::Cursor::new(b"ping\nstate\n".to_vec());
        assert_eq!(read_line_bounded(&mut short).unwrap().as_deref(), Some("ping\n"));
        assert_eq!(read_line_bounded(&mut short).unwrap().as_deref(), Some("state\n"));
        assert_eq!(read_line_bounded(&mut short).unwrap(), None);
        let mut long = io::Cursor::new(vec![b'a'; MAX_LINE + 10]);
        assert!(read_line_bounded(&mut long).is_err());
        let mut exact = io::Cursor::new({ let mut v = vec![b'a'; MAX_LINE - 1]; v.push(b'\n'); v });
        assert_eq!(read_line_bounded(&mut exact).unwrap().unwrap().len(), MAX_LINE);
    }

    #[test]
    fn state_is_plain_json() {
        let mut state = DesktopState::default();
        state.screens = 2;
        state.workspace = 1;
        state.apps.push(AppInfo { app_id: "org.example.App".into(), title: "A \"quoted\" title".into(), screen: 1, workspace: 1, place: Place::Side, pinned: false, urgent: false });
        let json: serde_json::Value = serde_json::from_str(&state_json(Some(&state), true)).unwrap();
        assert_eq!(json["screens"], 2);
        assert_eq!(json["locked"], true);
        assert_eq!(json["apps"][0]["place"], "side");
        assert_eq!(json["apps"][0]["title"], "A \"quoted\" title");
        let json: serde_json::Value = serde_json::from_str(&state_json(None, false)).unwrap();
        assert_eq!(json["connected"], false);
        assert_eq!(json["apps"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_client_is_answered_after_the_interface_acts() {
        let dir = private_dir();
        let path = dir.path().join("control.sock");
        let mut control = Control::start_at(egui::Context::default(), path.clone()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(Control::start_at(egui::Context::default(), path.clone()).is_err(), "a second shell cannot take the socket");
        let client = std::thread::spawn(move || {
            let stream = UnixStream::connect(&path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut answers = Vec::new();
            for line in ["workspace 9\n", "ping\n", "toast  hello\x07 there\n", "state\n"] {
                (&stream).write_all(line.as_bytes()).unwrap();
                let mut answer = String::new();
                reader.read_line(&mut answer).unwrap();
                answers.push(answer);
            }
            answers
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while seen.len() < 3 && Instant::now() < deadline {
            for (request, reply) in control.take() {
                let answer = match &request {
                    Request::Ping => Answer::Ok,
                    Request::Toast(_) => Answer::Ok,
                    Request::State => Answer::Json(state_json(None, false)),
                    other => panic!("unexpected request {other:?}"),
                };
                seen.push(request);
                reply.send(answer);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let answers = client.join().unwrap();
        assert!(answers[0].starts_with("error 'workspace' takes"), "{answers:?}");
        assert_eq!(answers[1], "ok\n");
        assert_eq!(answers[2], "ok\n");
        assert!(answers[3].starts_with("ok {\"") && answers[3].ends_with("}\n"), "{answers:?}");
        assert_eq!(seen[1], Request::Toast("hello  there".into()));
    }

    #[test]
    fn an_unanswered_request_is_told_so() {
        let dir = private_dir();
        let path = dir.path().join("control.sock");
        let mut control = Control::start_at(egui::Context::default(), path.clone()).unwrap();
        let client = std::thread::spawn(move || {
            let stream = UnixStream::connect(&path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            (&stream).write_all(b"lock\n").unwrap();
            let mut answer = String::new();
            BufReader::new(stream).read_line(&mut answer).unwrap();
            answer
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut dropped = false;
        while !dropped && Instant::now() < deadline {
            for (_, reply) in control.take() {
                drop(reply);
                dropped = true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(client.join().unwrap(), "error the shell did not answer\n");
    }
}
