//! The lock screen: the session stays, hidden behind a password.
//!
//! The compositor does the hiding: once told to lock, it shows nothing but the shell
//! and sends every key to it. The shell draws this screen over all of its own, checks
//! the password with the system's helper for that, and tells the compositor to unlock.

use crate::greeter::{Asked, Identity, load_picture};
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, CentralPanel, Event, Frame, Key, Rect, TextureHandle, vec2};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// The helper that checks a user's own password against the system's records.
const CHECKER: &str = "/usr/sbin/unix_chkpwd";

/// A quick installation check for the real-helper regression.
#[cfg(test)]
fn can_check() -> bool {
    executable(std::path::Path::new(CHECKER)).is_ok()
}

const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

fn executable(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|_| "The password checker is unavailable".to_string())?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("The password checker is not executable".to_string());
    }
    Ok(())
}

fn ready_with(path: &std::path::Path, user: &str, timeout: Duration) -> Result<(), String> {
    executable(path)?;
    // chkexpiry reads the account database without needing the user's password.
    // Linux-PAM also reports readable but expired accounts with statuses 12/13.
    match run_helper(path, user, "chkexpiry", None, timeout)?.code() {
        Some(0 | 12 | 13) => Ok(()),
        _ => Err("The password checker cannot read your account information".to_string()),
    }
}

/// Checks `user`'s password. Only PAM_AUTH_ERR means a wrong password.
pub fn check(user: &str, password: &str) -> Result<bool, String> {
    check_with(std::path::Path::new(CHECKER), user, password, CHECK_TIMEOUT)
}

fn check_with(path: &std::path::Path, user: &str, password: &str, timeout: Duration) -> Result<bool, String> {
    if password.contains('\0') {
        return Err("The password contains an unsupported character".to_string());
    }
    // Linux-PAM unix_chkpwd.c and _pam_types.h define these exit statuses.
    match run_helper(path, user, "nullok", Some(password), timeout)?.code() {
        Some(0) => Ok(true),
        Some(7) => Ok(false),
        _ => Err("The password checker failed; try again or contact your administrator".to_string()),
    }
}

fn run_helper(path: &std::path::Path, user: &str, option: &str, password: Option<&str>, timeout: Duration) -> Result<std::process::ExitStatus, String> {
    use std::time::Instant;
    let failure = |e: std::io::Error| format!("The password could not be checked: {e}");
    let mut child = Command::new(path)
        .args([user, option])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().map_err(failure)?;
    let deadline = Instant::now() + timeout;
    let input = child.stdin.take().expect("piped helper input");
    // Preserve existing pipe flags and keep password writes inside the deadline.
    // No password bytes enter argv, environment variables, or diagnostics.
    let nonblocking = rustix::fs::fcntl_getfl(&input)
        .and_then(|flags| rustix::fs::fcntl_setfl(&input, flags | rustix::fs::OFlags::NONBLOCK));
    if let Err(error) = nonblocking {
        let error = failure(error.into());
        drop(input);
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let mut input = password.map(|_| input);
    let bytes = password.unwrap_or("").as_bytes();
    let mut sent = 0;
    let outcome = loop {
        if Instant::now() >= deadline {
            break Err("The password checker timed out; try again".to_string());
        }
        if let Some(stream) = input.as_mut() {
            let remaining = if sent < bytes.len() { &bytes[sent..] } else { &[0][..] };
            match stream.write(remaining) {
                Ok(0) => break Err("The password checker closed its input".to_string()),
                Ok(n) => {
                    sent += n;
                    if sent > bytes.len() { input = None; }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {}
                Err(e) => break Err(failure(e)),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break if input.is_none() || password.is_none() { Ok(status) } else { Err("The password checker did not read the password".to_string()) },
            Ok(None) => {},
            Err(e) => break Err(failure(e)),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(input);
    if outcome.is_err() {
        // Always terminate and reap the helper on timeout or transport failure.
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

enum State {
    Ready,
    Checking,
    Unlocking,
    Denied(String),
}

pub struct Lock {
    pub locked: bool,
    user: String,
    password: String,
    state: State,
    result: Option<Receiver<Result<bool, String>>>,
    /// The user's picture, once looked for.
    picture: Option<Option<TextureHandle>>,
    preparation: Option<Receiver<Result<(), String>>>,
    /// The password is shown as typed, not as dots, until hidden again or the next lock.
    revealed: bool,
}

/// What the lock screen may say about the machine without unlocking it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Glance {
    /// How full the battery is, and whether it is on mains power.
    pub battery: Option<(u8, bool)>,
    pub online: bool,
    /// How many notifications wait; never what they say.
    pub notices: usize,
}

impl Lock {
    pub fn new() -> Lock {
        Lock {
            locked: false,
            user: std::env::var("USER").unwrap_or_else(|_| "user".to_string()),
            password: String::new(),
            state: State::Ready,
            result: None,
            picture: None,
            preparation: None,
            revealed: false,
        }
    }

    /// Probe account access before committing to a compositor lock.
    pub fn prepare(&mut self, ctx: &egui::Context) {
        if self.locked || self.preparation.is_some() { return; }
        let (tx, rx) = channel();
        self.preparation = Some(rx);
        let user = self.user.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ready_with(std::path::Path::new(CHECKER), &user, CHECK_TIMEOUT));
            ctx.request_repaint();
        });
    }

    pub fn preparing(&self) -> bool { self.preparation.is_some() }

    pub fn take_prepared(&mut self) -> Option<Result<(), String>> {
        let result = match self.preparation.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("The password checker stopped unexpectedly".to_string()),
        };
        self.preparation = None;
        Some(result)
    }

    /// The user's picture changed: look for it again when next shown.
    pub fn forget_picture(&mut self) {
        self.picture = None;
    }

    pub fn engage(&mut self) {
        self.preparation = None;
        self.result = None;
        self.locked = true;
        self.password.clear();
        self.revealed = false;
        self.state = State::Ready;
    }

    /// Unlocks without a password: for when the compositor says the lock is off.
    pub fn release(&mut self) {
        self.preparation = None;
        self.locked = false;
        self.password.clear();
        self.revealed = false;
        self.result = None;
    }

    /// Keep the lock visible while its authenticated release is acknowledged.
    pub fn await_unlock(&mut self) {
        self.engage();
        self.state = State::Unlocking;
    }

    fn submit(&mut self, ctx: &egui::Context) {
        if self.password.is_empty() || matches!(self.state, State::Checking | State::Unlocking) {
            return;
        }
        self.state = State::Checking;
        let (tx, rx) = channel();
        self.result = Some(rx);
        let password = std::mem::take(&mut self.password);
        let user = self.user.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = check(&user, &password);
            // A wrong guess costs time, so that guessing is slow.
            if outcome == Ok(false) {
                std::thread::sleep(Duration::from_millis(1500));
            }
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
    }

    /// Takes the keyboard and draws the lock screen over everything. Returns true on
    /// the frame the right password is given.
    ///
    /// `below` draws whatever else belongs on the screen (what is playing), given the
    /// part of it inside the margins.
    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme, glance: Glance, below: impl FnOnce(&mut egui::Ui, egui::Rect)) -> bool {
        if let Some(outcome) = self.result.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.result = None;
            match outcome {
                Ok(true) => {
                    self.release();
                    return true;
                }
                Ok(false) => { self.state = State::Denied("That password did not match".to_string()); self.revealed = false; }
                Err(why) => { self.state = State::Denied(why); self.revealed = false; }
            }
        }

        for event in ctx.input(|i| i.events.clone()) {
            if matches!(self.state, State::Checking | State::Unlocking) {
                break;
            }
            match event {
                Event::Text(text) | Event::Paste(text) => {
                    self.password.extend(text.chars().filter(|c| !c.is_control()));
                    self.state = State::Ready;
                }
                Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::Backspace if modifiers.ctrl => self.password.clear(),
                    Key::Backspace => {
                        self.password.pop();
                    }
                    Key::H if modifiers.ctrl || modifiers.command => self.revealed = !self.revealed,
                    Key::Escape => self.password.clear(),
                    Key::Enter => self.submit(ctx),
                    _ => {}
                },
                _ => {}
            }
        }

        if self.picture.is_none() {
            self.picture = Some(load_picture(ctx, &self.user));
        }
        let (edge, message, ink) = match &self.state {
            State::Ready => (theme.main, String::new(), ui::SECONDARY),
            State::Checking => (ui::SECONDARY, "Checking…".to_string(), ui::SECONDARY),
            State::Unlocking => (ui::SECONDARY, "Unlocking…".to_string(), ui::SECONDARY),
            State::Denied(why) => (ui::CAUTION, why.clone(), ui::CAUTION),
        };
        let mut asked = Asked::default();
        CentralPanel::default().frame(Frame::new().fill(ui::GROUND)).show(ctx, |ui| {
            let screen = ui.max_rect();
            let painter = ui.painter().clone();
            painter.rect_filled(screen, 0.0, ui::GROUND);
            crate::greeter::glow(&painter, screen, theme.main);
            let inner = screen.shrink2(vec2(56.0, 40.0));

            // Top: the desktop's name and that it is locked; at the right, the
            // connection and the battery.
            let mut top = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(inner.min, vec2(inner.width(), 24.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            top.spacing_mut().item_spacing.x = 14.0;
            ui::tracked(&mut top, "DEs-UI", ui::display_bold(13.0), ui::SECONDARY, 4.0);
            crate::panels::pill(&mut top, "LOCKED", if matches!(self.state, State::Denied(_)) { ui::CAUTION } else { ui::SECONDARY });
            top.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some((percent, plugged)) = glance.battery {
                    ui::text(ui, &format!("{percent}%"), ui::mono(12.0), if !plugged && percent <= 15 { ui::CAUTION } else { ui::SECONDARY });
                    ui::icon(ui, if plugged { ui::Icon::Charging } else { ui::Icon::Battery }, 18.0, ui::SECONDARY);
                    ui.add_space(8.0);
                }
                if glance.online { ui::icon(ui, ui::Icon::Wifi, 18.0, ui::SECONDARY); }
            });

            asked = Identity {
                user: &self.user,
                picture: self.picture.as_ref().and_then(|picture| picture.as_ref()),
                edge,
                message: &message,
                message_ink: ink,
                typed: self.password.chars().count(),
                shown: self.revealed.then_some(self.password.as_str()),
                busy: matches!(self.state, State::Checking | State::Unlocking),
                hint: "Enter to unlock · Esc clears · Ctrl+H shows or hides",
                accounts: None,
            }
            .draw(ui, theme);

            // Bottom: that the session is still there, and how many notifications
            // wait, without what they say.
            painter.text(inner.left_bottom(), Align2::LEFT_BOTTOM, "Your session is still running behind this screen.", ui::mono(12.0), ui::LABEL);
            if glance.notices > 0 {
                let label = if glance.notices == 1 { "1 NOTIFICATION".to_string() } else { format!("{} NOTIFICATIONS", glance.notices) };
                let galley = ui::galley(ui, &label, ui::mono(10.5), ui::SECONDARY, 1.0);
                let size = galley.size() + vec2(22.0, 10.0);
                let chip = Rect::from_min_size(inner.right_bottom() - size, size);
                painter.rect_stroke(chip, size.y / 2.0, egui::Stroke::new(1.0, ui::LINE_STRONG), egui::StrokeKind::Inside);
                painter.galley(chip.center() - galley.size() / 2.0, galley, ui::SECONDARY);
            }
            below(ui, inner);
        });
        if asked.reveal { self.revealed = !self.revealed; }
        if asked.submit { self.submit(ctx); }
        // The clock and the cursor keep moving.
        ctx.request_repaint_after(Duration::from_millis(250));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    fn helper(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checker");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }

    fn painted_texts(output: &egui::FullOutput) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => for shape in shapes { walk(shape, out); },
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes { walk(&clipped.shape, &mut out); }
        out
    }

    #[test]
    fn the_password_shows_only_while_asked_and_hides_again_on_the_next_lock() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        let mut lock = Lock::new();
        lock.engage();
        let key = |key: Key, ctrl: bool| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers { ctrl, ..Default::default() } };
        let screen = Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0)));
        let frame = |lock: &mut Lock, events: Vec<Event>| {
            let output = ctx.run(egui::RawInput { screen_rect: screen, events, ..Default::default() }, |ctx| { lock.show(ctx, &theme, Glance::default(), |_, _| {}); });
            painted_texts(&output)
        };
        let typed = frame(&mut lock, vec![Event::Text("hunter2".into())]);
        assert!(!typed.iter().any(|text| text.contains("hunter2")), "typed characters show as dots");
        let shown = frame(&mut lock, vec![key(Key::H, true)]);
        assert!(lock.revealed);
        assert!(shown.iter().any(|text| text.contains("hunter2")), "Ctrl+H shows what was typed");
        assert_eq!(lock.password, "hunter2", "Ctrl+H is not typed into the password");
        frame(&mut lock, vec![key(Key::H, true)]);
        assert!(!lock.revealed, "Ctrl+H again hides it");
        lock.revealed = true;
        lock.release();
        lock.engage();
        assert!(!lock.revealed, "a new lock starts hidden");
        assert!(lock.password.is_empty());
    }

    #[test]
    #[ignore]
    fn native_lock_preview() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        struct Preview { lock: Lock, frames: usize, output: std::path::PathBuf }
        impl eframe::App for Preview {
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                for event in ctx.input(|i| i.events.clone()) {
                    if let Event::Screenshot { image, .. } = event {
                        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        image::save_buffer(&self.output, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).unwrap();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                let theme = Theme::load("signal");
                self.lock.show(ctx, &theme, Glance { battery: Some((68, false)), online: true, notices: 3 }, |_, _| {});
                self.frames += 1;
                if self.frames == 4 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
                ctx.request_repaint();
            }
        }
        let output = std::path::PathBuf::from(std::env::var("DESUI_LOCK_PREVIEW").expect("preview output"));
        let mut lock = Lock::new();
        lock.engage();
        lock.user = "preview".into();
        lock.picture = Some(None);
        lock.password = "correct horse".into();
        lock.revealed = std::env::var("DESUI_LOCK_PREVIEW_MODE").as_deref() != Ok("hidden");
        let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]), event_loop_builder: Some(Box::new(|builder| { builder.with_x11().with_any_thread(true); })), ..Default::default() };
        eframe::run_native("lock preview", options, Box::new(move |cc| {
            ui::install_fonts(&cc.egui_ctx, None, None);
            crate::apply_style(&cc.egui_ctx, &Theme::load("signal"));
            Ok(Box::new(Preview { lock, frames: 0, output }))
        })).unwrap();
    }

    #[test]
    fn a_wrong_password_is_refused() {
        if !can_check() { return; }
        let user = std::env::var("USER").unwrap();
        if ready_with(std::path::Path::new(CHECKER), &user, CHECK_TIMEOUT).is_err() {
            eprintln!("real password-check test unavailable: account data cannot be read here");
            return;
        }
        assert_eq!(check(&user, "surely-not-this-one-7f3a"), Ok(false));
        assert_eq!(check(&user, ""), Ok(false));
    }

    #[test]
    fn helper_status_distinguishes_mismatch_from_system_failure() {
        for (code, expected) in [(0, Some(true)), (7, Some(false)), (4, None), (9, None), (127, None)] {
            let (_dir, path) = helper(&format!("cat >/dev/null; exit {code}"));
            let outcome = check_with(&path, "user", "password", Duration::from_secs(1));
            match expected {
                Some(value) => assert_eq!(outcome, Ok(value)),
                None => assert!(outcome.is_err(), "status {code} was not an error"),
            }
        }
    }

    #[test]
    fn password_is_sent_only_on_stdin_with_nul_terminator() {
        let (_dir, path) = helper("[ \"$#\" = 2 ] && [ \"$1\" = user ] && [ \"$2\" = nullok ] || exit 4\nbytes=$(od -An -tx1 | tr -d ' \"\\n\"'); [ \"$bytes\" = 73656372657400 ] || exit 4\nexit 0");
        assert_eq!(check_with(&path, "user", "secret", Duration::from_secs(1)), Ok(true));
    }

    #[test]
    fn signaled_helper_fails_closed() {
        let (_dir, path) = helper("cat >/dev/null; kill -KILL $$");
        assert!(check_with(&path, "user", "secret", Duration::from_secs(1)).is_err());
    }

    #[test]
    fn readiness_requires_executable_and_readable_account() {
        let (dir, path) = helper("exit 0");
        assert!(ready_with(&path, "user", Duration::from_secs(1)).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(ready_with(&path, "user", Duration::from_secs(1)).is_err());
        assert!(ready_with(&dir.path().join("missing"), "user", Duration::from_secs(1)).is_err());
        let (_dir, path) = helper("exit 9");
        assert!(ready_with(&path, "user", Duration::from_secs(1)).is_err());
    }

    #[test]
    fn blocked_input_times_out_and_helper_is_reaped() {
        let (dir, path) = helper("echo $$ > \"$0.pid\"; exec sleep 30");
        let began = std::time::Instant::now();
        assert!(check_with(&path, "user", &"x".repeat(2_000_000), Duration::from_millis(100)).is_err());
        assert!(began.elapsed() < Duration::from_secs(2));
        let pid = std::fs::read_to_string(dir.path().join("checker.pid")).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{}", pid.trim())).exists());
    }

    #[test]
    fn hung_helper_times_out_after_reading_input() {
        let (_dir, path) = helper("cat >/dev/null; exec sleep 30");
        let began = std::time::Instant::now();
        assert!(check_with(&path, "user", "password", Duration::from_millis(100)).is_err());
        assert!(began.elapsed() < Duration::from_secs(2));
    }
}
