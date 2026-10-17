//! The polkit authentication agent: when a program asks for something only an
//! administrator may do (`pkexec`, installing software, formatting a disk), polkit asks
//! this agent, and the shell shows a dialog for the password.
//!
//! The password is checked by polkit's own setuid helper, `polkit-agent-helper-1`,
//! which speaks PAM. The cookie and the password reach it on its standard input only;
//! neither is ever in a process's arguments, its environment or a log. The dialog is
//! never drawn over the lock screen: a request that arrives while locked waits.

use crate::{theme::Theme, ui};
use eframe::egui::{self, Align2, Id, Key, Order, Stroke, StrokeKind, TextEdit, vec2};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use zbus::zvariant::{OwnedValue, Value};

const PATH: &str = "/org/desui/PolicyKit1/AuthenticationAgent";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";
const HELPERS: [&str; 3] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/x86_64-linux-gnu/polkit-1/polkit-agent-helper-1",
];
/// Wrong passwords before the request is given up.
const TRIES: u32 = 3;
/// How long the dialog waits for someone to answer it.
const ANSWER_TIME: Duration = Duration::from_secs(120);

/// What polkit hears back about one request.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
}

/// One request from polkit, waiting for the user.
struct Request {
    action_id: String,
    message: String,
    cookie: String,
    /// Who may answer it: users, by id, in the order polkit gave them.
    uids: Vec<u32>,
    reply: async_channel::Sender<Outcome>,
}

/// One line of what the helper says.
#[derive(Debug, Clone, PartialEq)]
pub enum HelperLine {
    /// It wants an answer; `echo` when what is typed may be shown.
    Prompt { echo: bool, text: String },
    Error(String),
    Info(String),
    Success,
    Failure,
    Other(String),
}

pub fn parse_line(line: &str) -> HelperLine {
    let line = line.trim_end_matches(['\n', '\r']);
    let rest = |prefix: &str| line.strip_prefix(prefix).map(|text| text.trim_start().to_string());
    if let Some(text) = rest("PAM_PROMPT_ECHO_OFF") {
        HelperLine::Prompt { echo: false, text }
    } else if let Some(text) = rest("PAM_PROMPT_ECHO_ON") {
        HelperLine::Prompt { echo: true, text }
    } else if let Some(text) = rest("PAM_ERROR_MSG") {
        HelperLine::Error(text)
    } else if let Some(text) = rest("PAM_TEXT_INFO") {
        HelperLine::Info(text)
    } else if line == "SUCCESS" {
        HelperLine::Success
    } else if line == "FAILURE" {
        HelperLine::Failure
    } else {
        HelperLine::Other(line.to_string())
    }
}

pub fn helper_path() -> Option<PathBuf> {
    HELPERS.iter().map(PathBuf::from).find(|path| path.exists())
}

/// One run of the helper for one user.
struct Attempt {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<HelperLine>,
}

impl Attempt {
    fn start(helper: &Path, user: &str, cookie: &str, ctx: Option<egui::Context>) -> std::io::Result<Attempt> {
        let mut child = Command::new(helper)
            .arg(user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .spawn()?;
        let mut stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no output from the helper"))?;
        // The cookie goes on standard input, never in the arguments.
        if let Some(stdin) = stdin.as_mut() {
            stdin.write_all(cookie.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
        }
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let parsed = parse_line(&line);
                let last = matches!(parsed, HelperLine::Success | HelperLine::Failure);
                if send.send(parsed).is_err() || last {
                    break;
                }
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            }
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        });
        Ok(Attempt { child, stdin, lines })
    }

    /// Hands the helper an answer. The password's bytes are overwritten once written.
    fn answer(&mut self, text: &mut String) -> std::io::Result<()> {
        let mut bytes = std::mem::take(text).into_bytes();
        let written = match self.stdin.as_mut() {
            Some(stdin) => stdin.write_all(&bytes).and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()),
            None => Err(std::io::Error::other("the helper has gone")),
        };
        bytes.fill(0);
        written
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What the dialog is doing for the request in front.
struct Dialog {
    request: Request,
    /// Index into `request.uids`.
    chosen: usize,
    attempt: Option<Attempt>,
    prompt: Option<(bool, String)>,
    password: String,
    note: Option<(String, bool)>,
    failures: u32,
    since: Instant,
    focus: bool,
}

/// The agent: its D-Bus side runs on zbus's own thread, its dialog in the shell.
pub struct Polkit {
    requests: Receiver<Request>,
    cancels: Receiver<String>,
    waiting: VecDeque<Request>,
    dialog: Option<Dialog>,
    helper: Option<PathBuf>,
    me: u32,
    passwd: String,
    ctx: Option<egui::Context>,
    registration: Option<Registration>,
}

struct Registration {
    bus: zbus::blocking::Connection,
    subject: Subject,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = authority(&self.bus).and_then(|proxy| {
            proxy.call_method("UnregisterAuthenticationAgent", &(self.subject.value(), PATH)).map(|_| ())
        });
    }
}

fn authority(bus: &zbus::blocking::Connection) -> zbus::Result<zbus::blocking::Proxy<'_>> {
    zbus::blocking::Proxy::new(bus, AUTHORITY, AUTHORITY_PATH, AUTHORITY_INTERFACE)
}

/// Who the agent speaks for: this login session, or failing that this process.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    Session(String),
    Process { pid: u32, start_time: u64 },
}

impl Subject {
    pub fn here() -> Option<Subject> {
        if let Some(id) = std::env::var("XDG_SESSION_ID").ok().filter(|id| !id.is_empty()) {
            return Some(Subject::Session(id));
        }
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        Some(Subject::Process { pid: std::process::id(), start_time: start_time(&stat)? })
    }

    fn value(&self) -> (&'static str, HashMap<&'static str, Value<'_>>) {
        match self {
            Subject::Session(id) => ("unix-session", HashMap::from([("session-id", Value::from(id.as_str()))])),
            Subject::Process { pid, start_time } => {
                ("unix-process", HashMap::from([("pid", Value::from(*pid)), ("start-time", Value::from(*start_time))]))
            }
        }
    }
}

/// Field 22 of `/proc/<pid>/stat`, counted after the command's closing parenthesis.
pub fn start_time(stat: &str) -> Option<u64> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19)?.parse().ok()
}

/// The D-Bus object polkit calls.
struct Agent {
    requests: Sender<Request>,
    cancels: Sender<String>,
    ctx: Option<egui::Context>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let uids = users_of(&identities, &group, &passwd);
        if uids.is_empty() {
            return Err(AgentError::Failed("No one here may answer this request.".into()));
        }
        let (reply, outcome) = async_channel::bounded(1);
        let request = Request { action_id, message, cookie, uids, reply };
        self.requests.send(request).map_err(|_| AgentError::Failed("The desktop is not listening.".into()))?;
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        match outcome.recv().await {
            Ok(Outcome::Done) => Ok(()),
            Ok(Outcome::Cancelled) => Err(AgentError::Cancelled("Cancelled.".into())),
            Ok(Outcome::Failed(why)) => Err(AgentError::Failed(why)),
            Err(_) => Err(AgentError::Failed("The desktop closed the request.".into())),
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        let _ = self.cancels.send(cookie);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }
}

/// The users an identity list names: `unix-user` by uid, and the members of each
/// `unix-group`, without repeats, in order.
fn users_of(identities: &[(String, HashMap<String, OwnedValue>)], group: &str, passwd: &str) -> Vec<u32> {
    let number = |details: &HashMap<String, OwnedValue>, key: &str| {
        details.get(key).and_then(|value| u32::try_from(value).ok().or_else(|| i32::try_from(value).ok().and_then(|n| u32::try_from(n).ok())))
    };
    let mut uids = Vec::new();
    for (kind, details) in identities {
        match kind.as_str() {
            "unix-user" => uids.extend(number(details, "uid")),
            "unix-group" => {
                if let Some(gid) = number(details, "gid") {
                    uids.extend(group_members(group, gid).iter().filter_map(|name| uid_of(passwd, name)));
                }
            }
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    uids.retain(|uid| seen.insert(*uid));
    uids
}

pub fn group_members(group: &str, gid: u32) -> Vec<String> {
    group
        .lines()
        .map(|line| line.split(':').collect::<Vec<_>>())
        .find(|fields| fields.len() >= 4 && fields[2].parse() == Ok(gid))
        .map(|fields| fields[3].split(',').filter(|name| !name.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn name_of(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().map(|line| line.split(':').collect::<Vec<_>>()).find(|fields| fields.len() >= 3 && fields[2].parse() == Ok(uid)).map(|fields| fields[0].to_string())
}

fn uid_of(passwd: &str, name: &str) -> Option<u32> {
    passwd.lines().map(|line| line.split(':').collect::<Vec<_>>()).find(|fields| fields.len() >= 3 && fields[0] == name).and_then(|fields| fields[2].parse().ok())
}

/// Whom the dialog offers first: the person at the desk when they may answer.
pub fn first_choice(uids: &[u32], me: u32) -> usize {
    uids.iter().position(|uid| *uid == me).unwrap_or(0)
}

impl Polkit {
    /// Serves the agent on the system bus and registers it with polkit. Without a
    /// system bus or polkit, the shell carries on without one and says so once.
    pub fn start(ctx: egui::Context) -> Polkit {
        let (requests_in, requests) = mpsc::channel();
        let (cancels_in, cancels) = mpsc::channel();
        let mut polkit = Polkit::with_channels(requests, cancels, helper_path(), Some(ctx.clone()));
        let agent = Agent { requests: requests_in, cancels: cancels_in, ctx: Some(ctx) };
        let registered = (|| -> zbus::Result<Registration> {
            let bus = zbus::blocking::connection::Builder::system()?.serve_at(PATH, agent)?.build()?;
            let subject = Subject::here().ok_or_else(|| zbus::Error::Failure("no session or process to speak for".into()))?;
            let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
            authority(&bus)?.call_method("RegisterAuthenticationAgent", &(subject.value(), locale.as_str(), PATH))?;
            Ok(Registration { bus, subject })
        })();
        match registered {
            Ok(registration) => polkit.registration = Some(registration),
            Err(error) => eprintln!("des-ui: no polkit agent: {error}"),
        }
        polkit
    }

    fn with_channels(requests: Receiver<Request>, cancels: Receiver<String>, helper: Option<PathBuf>, ctx: Option<egui::Context>) -> Polkit {
        Polkit {
            requests,
            cancels,
            waiting: VecDeque::new(),
            dialog: None,
            helper,
            me: rustix::process::getuid().as_raw(),
            passwd: std::fs::read_to_string("/etc/passwd").unwrap_or_default(),
            ctx,
            registration: None,
        }
    }

    /// Whether a dialog is waiting to be answered; the parent keeps keys from elsewhere.
    pub fn active(&self) -> bool {
        self.dialog.is_some()
    }

    /// Takes in new requests and cancellations, and what the helper said.
    /// Locked, nothing new is begun and a dialog in progress is put back to wait.
    pub fn pump(&mut self, locked: bool) {
        while let Ok(request) = self.requests.try_recv() {
            self.waiting.push_back(request);
        }
        while let Ok(cookie) = self.cancels.try_recv() {
            self.cancel(&cookie);
        }
        if locked {
            if let Some(mut dialog) = self.dialog.take() {
                dialog.attempt = None;
                dialog.password.clear();
                self.waiting.push_front(dialog.request);
            }
            return;
        }
        if self.dialog.is_none() {
            if let Some(request) = self.waiting.pop_front() {
                let chosen = first_choice(&request.uids, self.me);
                let mut dialog = Dialog { request, chosen, attempt: None, prompt: None, password: String::new(), note: None, failures: 0, since: Instant::now(), focus: true };
                self.begin(&mut dialog);
                self.dialog = Some(dialog);
            }
        }
        self.step();
    }

    fn cancel(&mut self, cookie: &str) {
        if self.dialog.as_ref().is_some_and(|dialog| dialog.request.cookie == cookie) {
            self.finish(Outcome::Cancelled);
        }
        self.waiting.retain(|request| {
            let gone = request.cookie == cookie;
            if gone {
                let _ = request.reply.try_send(Outcome::Cancelled);
            }
            !gone
        });
    }

    /// Starts the helper for the chosen user.
    fn begin(&self, dialog: &mut Dialog) {
        dialog.attempt = None;
        dialog.prompt = None;
        let Some(helper) = &self.helper else {
            dialog.note = Some(("polkit's helper is not installed here.".into(), true));
            return;
        };
        let uid = dialog.request.uids[dialog.chosen];
        let Some(user) = name_of(&self.passwd, uid) else {
            dialog.note = Some((format!("No account has the number {uid}."), true));
            return;
        };
        match Attempt::start(helper, &user, &dialog.request.cookie, self.ctx.clone()) {
            Ok(attempt) => dialog.attempt = Some(attempt),
            Err(error) => dialog.note = Some((format!("Could not ask for the password: {error}"), true)),
        }
    }

    /// Reads what the helper said since the last frame.
    fn step(&mut self) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        if dialog.since.elapsed() > ANSWER_TIME {
            self.finish(Outcome::Failed("No answer came in time.".into()));
            return;
        }
        let mut ended = None;
        if let Some(attempt) = dialog.attempt.as_mut() {
            loop {
                match attempt.lines.try_recv() {
                    Ok(HelperLine::Prompt { echo, text }) => {
                        dialog.prompt = Some((echo, text));
                        dialog.focus = true;
                    }
                    Ok(HelperLine::Error(text)) => dialog.note = Some((text, true)),
                    Ok(HelperLine::Info(text)) => dialog.note = Some((text, false)),
                    Ok(HelperLine::Success) => ended = Some(true),
                    Ok(HelperLine::Failure) => ended = Some(false),
                    Ok(HelperLine::Other(_)) => {}
                    Err(TryRecvError::Empty) => break,
                    // It went without a word: that is a refusal too.
                    Err(TryRecvError::Disconnected) => ended = Some(false),
                }
                if ended.is_some() {
                    break;
                }
            }
        }
        match ended {
            Some(true) => self.finish(Outcome::Done),
            Some(false) => {
                let mut dialog = self.dialog.take().expect("dialog");
                dialog.failures += 1;
                if dialog.failures >= TRIES {
                    self.dialog = Some(dialog);
                    self.finish(Outcome::Failed("The password was refused three times.".into()));
                } else {
                    dialog.note = Some(("That password was refused. Try again.".into(), true));
                    self.begin(&mut dialog);
                    self.dialog = Some(dialog);
                }
            }
            None => {}
        }
    }

    /// Sends the typed answer to the helper.
    fn submit(&mut self) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let Some(attempt) = dialog.attempt.as_mut() else { return };
        if dialog.prompt.take().is_none() {
            return;
        }
        if let Err(error) = attempt.answer(&mut dialog.password) {
            dialog.note = Some((format!("Could not hand over the password: {error}"), true));
        }
        dialog.since = Instant::now();
    }

    fn finish(&mut self, outcome: Outcome) {
        if let Some(mut dialog) = self.dialog.take() {
            dialog.attempt = None;
            let mut bytes = std::mem::take(&mut dialog.password).into_bytes();
            bytes.fill(0);
            let _ = dialog.request.reply.try_send(outcome);
        }
    }

    /// Draws the dialog, centred above everything. The parent never calls it while locked.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, locked: bool) {
        self.pump(locked);
        if locked {
            return;
        }
        let Some(dialog) = self.dialog.as_mut() else { return };
        let mut submit = false;
        let mut cancel = false;
        let mut choose = None;
        let passwd = &self.passwd;
        egui::Area::new(Id::new("polkit-dialog"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let width = 440.0_f32.min(ctx.screen_rect().width() - 32.0);
                egui::Frame::new().fill(ui::PANEL).stroke(Stroke::new(1.0, theme.main)).inner_margin(24.0).show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui::tracked(ui, "AUTHENTICATION REQUIRED", ui::display(15.0), theme.main, 1.5);
                    ui.add(egui::Label::new(egui::RichText::new(&dialog.request.message).font(ui::display(14.0)).color(ui::TEXT)).wrap());
                    ui.add(egui::Label::new(egui::RichText::new(&dialog.request.action_id).font(ui::mono(10.0)).color(ui::LABEL)).wrap());
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for (index, uid) in dialog.request.uids.iter().enumerate() {
                            let name = name_of(passwd, *uid).unwrap_or_else(|| format!("uid {uid}"));
                            let chosen = index == dialog.chosen;
                            if dialog.request.uids.len() == 1 {
                                ui::text(ui, &name.to_uppercase(), ui::mono(12.0), ui::TEXT);
                            } else if ui::chip(ui, theme, &name.to_uppercase(), None, chosen).clicked() && !chosen {
                                choose = Some(index);
                            }
                        }
                    });
                    let (echo, prompt) = dialog.prompt.clone().unwrap_or((false, "Waiting for the system…".into()));
                    ui::text(ui, &prompt, ui::mono(12.0), ui::SECONDARY);
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, ui::GROUND);
                    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme.main), StrokeKind::Inside);
                    let edit = ui.put(
                        rect.shrink2(vec2(8.0, 4.0)),
                        TextEdit::singleline(&mut dialog.password)
                            .id(Id::new("polkit-password"))
                            .password(!echo)
                            .frame(false)
                            .font(ui::mono(13.0))
                            .text_color(ui::TEXT)
                            .interactive(dialog.prompt.is_some()),
                    );
                    if dialog.focus && dialog.prompt.is_some() {
                        edit.request_focus();
                        dialog.focus = false;
                    }
                    if edit.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                    if let Some((note, caution)) = &dialog.note {
                        ui::text(ui, note, ui::mono(12.0), if *caution { ui::CAUTION } else { ui::SECONDARY });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui::button(ui, theme, "AUTHENTICATE", Some("ENTER"), false).clicked() {
                            submit = true;
                        }
                        if ui::button(ui, theme, "CANCEL", Some("ESC"), false).clicked() || ui.input(|input| input.key_pressed(Key::Escape)) {
                            cancel = true;
                        }
                    });
                });
            });
        if let Some(index) = choose {
            dialog.chosen = index;
            dialog.note = None;
            dialog.password.clear();
            let mut dialog = self.dialog.take().expect("dialog");
            self.begin(&mut dialog);
            self.dialog = Some(dialog);
        } else if cancel {
            self.finish(Outcome::Cancelled);
        } else if submit {
            self.submit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn helper(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    /// Accepts "right", refuses anything else; checks that the cookie arrived first.
    const CHECKER: &str = r#"read cookie
[ "$cookie" = "the-cookie" ] || { echo FAILURE; exit 1; }
echo "PAM_TEXT_INFO hello"
echo "PAM_PROMPT_ECHO_OFF Password: "
read answer
if [ "$answer" = "right" ]; then echo SUCCESS; else echo FAILURE; fi"#;

    fn polkit_with(helper: PathBuf) -> (Polkit, Sender<Request>, Sender<String>) {
        let (requests_in, requests) = mpsc::channel();
        let (cancels_in, cancels) = mpsc::channel();
        let mut polkit = Polkit::with_channels(requests, cancels, Some(helper), None);
        polkit.passwd = format!("me:x:{}:1000::/home/me:/bin/sh\n", polkit.me);
        (polkit, requests_in, cancels_in)
    }

    fn request(polkit: &Polkit, cookie: &str) -> (Request, async_channel::Receiver<Outcome>) {
        let (reply, outcome) = async_channel::bounded(1);
        (Request { action_id: "org.example.do".into(), message: "Do it".into(), cookie: cookie.into(), uids: vec![polkit.me], reply }, outcome)
    }

    /// Pumps until the helper has asked, or the request has ended.
    fn until_prompt(polkit: &mut Polkit, outcome: &async_channel::Receiver<Outcome>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            polkit.pump(false);
            if !outcome.is_empty() || polkit.dialog.as_ref().is_some_and(|dialog| dialog.prompt.is_some()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the helper never asked");
    }

    fn until_outcome(polkit: &mut Polkit, outcome: &async_channel::Receiver<Outcome>) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            polkit.pump(false);
            if let Ok(outcome) = outcome.try_recv() {
                return outcome;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no outcome");
    }

    #[test]
    fn reads_the_helper_s_lines() {
        assert_eq!(parse_line("PAM_PROMPT_ECHO_OFF Password: \n"), HelperLine::Prompt { echo: false, text: "Password: ".into() });
        assert_eq!(parse_line("PAM_PROMPT_ECHO_ON Login:"), HelperLine::Prompt { echo: true, text: "Login:".into() });
        assert_eq!(parse_line("PAM_ERROR_MSG Authentication failure"), HelperLine::Error("Authentication failure".into()));
        assert_eq!(parse_line("PAM_TEXT_INFO Touch the key"), HelperLine::Info("Touch the key".into()));
        assert_eq!(parse_line("SUCCESS"), HelperLine::Success);
        assert_eq!(parse_line("FAILURE\r\n"), HelperLine::Failure);
        assert_eq!(parse_line("something"), HelperLine::Other("something".into()));
    }

    #[test]
    fn the_right_password_succeeds_and_never_reaches_the_arguments() {
        let (_dir, path) = helper(&format!("{CHECKER}\nsleep 0"));
        let (mut polkit, requests, _cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "the-cookie");
        requests.send(request).unwrap();
        until_prompt(&mut polkit, &outcome);
        let dialog = polkit.dialog.as_ref().unwrap();
        assert_eq!(dialog.note, Some(("hello".into(), false)));
        let pid = dialog.attempt.as_ref().unwrap().child.id();
        polkit.dialog.as_mut().unwrap().password = "right".into();
        polkit.submit();
        // The helper's arguments: its path and the user, nothing secret.
        if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
            let cmdline = String::from_utf8_lossy(&cmdline);
            assert!(!cmdline.contains("right") && !cmdline.contains("the-cookie"), "{cmdline}");
        }
        assert!(polkit.dialog.as_ref().unwrap().password.is_empty(), "the field is emptied once sent");
        assert_eq!(until_outcome(&mut polkit, &outcome), Outcome::Done);
        assert!(!polkit.active());
    }

    #[test]
    fn three_wrong_passwords_fail_the_request() {
        let (_dir, path) = helper(CHECKER);
        let (mut polkit, requests, _cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "the-cookie");
        requests.send(request).unwrap();
        for attempt in 0..TRIES {
            until_prompt(&mut polkit, &outcome);
            assert!(outcome.is_empty(), "ended early, at attempt {attempt}");
            polkit.dialog.as_mut().unwrap().password = "wrong".into();
            polkit.submit();
        }
        assert!(matches!(until_outcome(&mut polkit, &outcome), Outcome::Failed(_)));
    }

    #[test]
    fn a_refusal_lets_the_user_try_again() {
        let (_dir, path) = helper(CHECKER);
        let (mut polkit, requests, _cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "the-cookie");
        requests.send(request).unwrap();
        until_prompt(&mut polkit, &outcome);
        polkit.dialog.as_mut().unwrap().password = "wrong".into();
        polkit.submit();
        until_prompt(&mut polkit, &outcome);
        assert_eq!(polkit.dialog.as_ref().unwrap().failures, 1);
        polkit.dialog.as_mut().unwrap().password = "right".into();
        polkit.submit();
        assert_eq!(until_outcome(&mut polkit, &outcome), Outcome::Done);
    }

    #[test]
    fn cancelling_kills_and_reaps_the_helper() {
        let (_dir, path) = helper("read cookie\necho 'PAM_PROMPT_ECHO_OFF Password: '\nexec sleep 30");
        let (mut polkit, requests, cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "c1");
        requests.send(request).unwrap();
        until_prompt(&mut polkit, &outcome);
        let pid = polkit.dialog.as_ref().unwrap().attempt.as_ref().unwrap().child.id();
        cancels.send("c1".into()).unwrap();
        polkit.pump(false);
        assert_eq!(outcome.try_recv(), Ok(Outcome::Cancelled));
        // Reaped: no process, not even a zombie, is left with that number.
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn a_request_waits_while_locked_and_resumes_after() {
        let (_dir, path) = helper(CHECKER);
        let (mut polkit, requests, _cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "the-cookie");
        requests.send(request).unwrap();
        polkit.pump(true);
        assert!(!polkit.active());
        assert_eq!(polkit.waiting.len(), 1);
        until_prompt(&mut polkit, &outcome);
        assert!(polkit.active());
        polkit.pump(true);
        assert!(!polkit.active(), "locking puts the dialog away");
        assert_eq!(polkit.waiting.len(), 1);
        assert!(outcome.is_empty());
    }

    #[test]
    fn a_helper_that_vanishes_counts_as_a_refusal() {
        let (_dir, path) = helper("read cookie\nexit 1");
        let (mut polkit, requests, _cancels) = polkit_with(path);
        let (request, outcome) = request(&polkit, "x");
        requests.send(request).unwrap();
        assert!(matches!(until_outcome(&mut polkit, &outcome), Outcome::Failed(_)));
    }

    #[test]
    fn the_person_at_the_desk_is_offered_first() {
        assert_eq!(first_choice(&[0, 1000, 1001], 1000), 1);
        assert_eq!(first_choice(&[0, 1001], 1000), 0);
    }

    #[test]
    fn users_come_from_user_and_group_identities_without_repeats() {
        let passwd = "root:x:0:0::/root:/bin/sh\nada:x:1000:1000::/home/ada:/bin/sh\nbob:x:1001:1001::/home/bob:/bin/sh\n";
        let group = "root:x:0:\nsudo:x:27:ada,bob\n";
        let user = |uid: u32| ("unix-user".to_string(), HashMap::from([("uid".to_string(), OwnedValue::from(uid))]));
        let wheel = ("unix-group".to_string(), HashMap::from([("gid".to_string(), OwnedValue::from(27u32))]));
        assert_eq!(users_of(&[user(1000), wheel], group, passwd), vec![1000, 1001]);
        assert_eq!(name_of(passwd, 1001).as_deref(), Some("bob"));
        assert_eq!(group_members(group, 0), Vec::<String>::new());
    }

    #[test]
    fn speaks_for_the_session_or_else_the_process() {
        let stat = "1234 (des ui) S 1 1234 1234 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 4 0 987654 123456 789";
        assert_eq!(start_time(stat), Some(987654));
        let session = Subject::Session("c2".into());
        let (kind, details) = session.value();
        assert_eq!(kind, "unix-session");
        assert_eq!(details["session-id"], Value::from("c2"));
        let process = Subject::Process { pid: 7, start_time: 9 };
        let (kind, details) = process.value();
        assert_eq!(kind, "unix-process");
        assert_eq!(details["pid"], Value::from(7u32));
        assert_eq!(details["start-time"], Value::from(9u64));
    }
}
