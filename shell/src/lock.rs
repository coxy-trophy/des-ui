//! The lock screen: the session stays, hidden behind a password.
//!
//! The compositor does the hiding: once told to lock, it shows nothing but the shell
//! and sends every key to it. The shell draws this screen over all of its own, checks
//! the password with the system's helper for that, and tells the compositor to unlock.

use crate::greeter::load_picture;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, CentralPanel, Color32, Event, Frame, Key, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
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
    /// The clock, the notifications tag and the field, as coloured liquid glass.
    glass: crate::glass::Glass,
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
            glass: crate::glass::Glass::default(),
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
            State::Checking => (ui::SECONDARY, "Checking…".to_string(), GLASS_TEXT),
            State::Unlocking => (ui::SECONDARY, "Unlocking…".to_string(), GLASS_TEXT),
            State::Denied(why) => (ui::CAUTION, why.clone(), ui::CAUTION),
        };
        let denied = matches!(self.state, State::Denied(_));
        let busy = matches!(self.state, State::Checking | State::Unlocking);
        let mut submit = false;
        CentralPanel::default().frame(Frame::new().fill(ui::GROUND)).show(ctx, |ui| {
            let screen = ui.max_rect();
            let painter = ui.painter().clone();
            // The desktop's own wallpaper, whole and sharp, darkened a little at the
            // top and bottom where the words are; without one, the usual glow.
            let backdrop = wallpaper(ctx);
            // Where the glass goes, under everything drawn on it, once the shapes
            // are known; it paints the whole screen when the GPU can draw it.
            // Glass is worked out whenever it can be drawn; until the first is
            // ready, the plain version shows.
            let glassy = backdrop.is_some() && self.glass.available();
            let liquid = glassy && self.glass.ready();
            let mut panes = Vec::new();
            let soft = match backdrop {
                Some((sharp, soft, dest, uv)) => {
                    painter.rect_filled(screen, 0.0, ui::GROUND);
                    painter.image(sharp, dest, uv, Color32::WHITE);
                    scrim(&painter, screen, true);
                    scrim(&painter, screen, false);
                    Some((soft, dest, uv))
                }
                None => {
                    painter.rect_filled(screen, 0.0, ui::GROUND);
                    crate::greeter::glow(&painter, screen, theme.main);
                    None
                }
            };
            let slot = painter.add(egui::Shape::Noop);
            let cx = screen.center().x;
            let short = screen.height() < 640.0;

            // At the top right: the connection and the battery.
            let mut x = screen.right() - 30.0;
            let y = screen.top() + 28.0;
            if let Some((percent, plugged)) = glance.battery {
                let low = !plugged && percent < 20;
                let ink = if low { ui::CAUTION } else { GLASS_TEXT };
                let galley = painter.layout_no_wrap(format!("{percent}%"), ui::mono(12.5), ink);
                x -= galley.size().x;
                painter.galley(pos2(x, y - galley.size().y / 2.0), galley, ink);
                x -= 20.0;
                ui::paint_battery(&painter, pos2(x, y), ink, percent, plugged && percent < 100, plugged && percent >= 100, 1.0);
                x -= 34.0;
            }
            if glance.online { ui::paint_wifi(&painter, pos2(x, y), GLASS_TEXT, 3, 0.9); }

            // The bird, the date, and the time, large; glass, so the picture shows
            // through it.
            let now = chrono::Local::now();
            let mut y = screen.top() + if short { 24.0 } else { (screen.height() * 0.065).max(40.0) };
            y += bird(&painter, &mut panes, glassy, liquid, cx, y, short);
            let date = ui::galley(ui, &now.format("%A %-d %B").to_string(), ui::display_bold(if short { 15.0 } else { 21.0 }), GLASS_TEXT, 1.5);
            painter.galley(pos2(cx - date.size().x / 2.0, y), date.clone(), GLASS_TEXT);
            y += date.size().y - 4.0;
            let size = if short { 84.0 } else { (screen.height() * 0.2).clamp(96.0, 240.0) };
            let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display_bold(size), CLOCK, 2.0);
            let at = pos2(cx - time.size().x / 2.0, y);
            if glassy { panes.push(crate::glass::Shape::Text(time.clone(), at)); }
            if !liquid {
                painter.galley(at + vec2(0.0, 3.0), time.clone(), Color32::from_black_alpha(70));
                painter.galley(at, time.clone(), CLOCK);
            }
            y += time.size().y;
            // How many notifications wait, never what they say.
            if glance.notices > 0 {
                let label = if glance.notices == 1 { "1 notification".to_string() } else { format!("{} notifications", glance.notices) };
                let galley = painter.layout_no_wrap(label, ui::mono(11.5), GLASS_TEXT);
                let size = galley.size() + vec2(40.0, 14.0);
                let chip = Rect::from_center_size(pos2(cx, y + 6.0 + size.y / 2.0), size);
                if glassy { panes.push(crate::glass::Shape::Pill(chip)); }
                if !liquid { glass(&painter, chip, soft.as_ref(), Stroke::new(1.0, Color32::from_white_alpha(36))); }
                ui::paint_icon(&painter, Rect::from_center_size(pos2(chip.left() + 17.0, chip.center().y), vec2(13.0, 13.0)), ui::Icon::Notifications, GLASS_TEXT, false);
                painter.galley(pos2(chip.left() + 28.0, chip.center().y - galley.size().y / 2.0), galley, GLASS_TEXT);
            }

            // At the bottom: the picture, the name and the password field.
            let side = if short { 52.0 } else { 64.0 };
            let field_height = 40.0;
            let bottom = screen.bottom() - if short { 28.0 } else { (screen.height() * 0.06).max(40.0) } - 22.0;
            let field = Rect::from_center_size(pos2(cx, bottom - field_height / 2.0), vec2(268.0_f32.min(screen.width() - 48.0), field_height));
            let name = ui::galley(ui, &self.user, ui::display_bold(17.0), GLASS_TEXT, 0.5);
            let name_top = field.top() - 14.0 - name.size().y;
            painter.galley(pos2(cx - name.size().x / 2.0, name_top), name, GLASS_TEXT);
            let face = Rect::from_center_size(pos2(cx, name_top - 10.0 - side / 2.0), vec2(side, side));
            self.paint_face(ui, &painter, theme, face, if denied { ui::CAUTION } else { Color32::from_white_alpha(90) }, busy);

            let field_edge = if denied { ui::CAUTION } else if self.password.is_empty() { Color32::from_white_alpha(56) } else { edge };
            if glassy { panes.push(crate::glass::Shape::Pill(field)); }
            if liquid {
                // Only a change of state is outlined on the glass: typing or denied.
                if denied || !self.password.is_empty() { painter.rect_stroke(field, field.height() / 2.0, Stroke::new(1.0, field_edge), StrokeKind::Inside); }
            } else {
                glass(&painter, field, soft.as_ref(), Stroke::new(1.0, field_edge));
            }
            let go = Rect::from_center_size(pos2(field.right() - 20.0, field.center().y), vec2(28.0, 28.0));
            let room = Rect::from_min_max(pos2(field.left() + 16.0, field.top()), pos2(go.left() - 8.0, field.bottom()));
            let mut x = room.left();
            if self.revealed && !self.password.is_empty() {
                // The end of what was typed, where the caret is, stays in view.
                let mut chars: Vec<char> = self.password.chars().collect();
                let mut galley = ui::galley(ui, &self.password, ui::mono(14.0), GLASS_TEXT, 0.0);
                while galley.size().x > room.width() - 6.0 && !chars.is_empty() {
                    chars.remove(0);
                    galley = ui::galley(ui, &format!("…{}", chars.iter().collect::<String>()), ui::mono(14.0), GLASS_TEXT, 0.0);
                }
                painter.galley(pos2(x, field.center().y - galley.size().y / 2.0), galley.clone(), GLASS_TEXT);
                x += galley.size().x + 2.0;
            } else if self.password.is_empty() {
                painter.text(pos2(x, field.center().y), Align2::LEFT_CENTER, "Enter password", ui::display(13.5), Color32::from_white_alpha(150));
            } else {
                let fit = ((room.width() - 8.0) / 14.0).max(1.0) as usize;
                for _ in 0..self.password.chars().count().min(fit) {
                    painter.circle_filled(pos2(x + 3.5, field.center().y), 3.5, GLASS_TEXT);
                    x += 14.0;
                }
            }
            if !busy && !self.password.is_empty() && ui.input(|i| i.time).rem_euclid(1.0) < 0.5 {
                painter.rect_filled(Rect::from_center_size(pos2(x + 1.0, field.center().y), vec2(2.0, 18.0)), 0.0, theme.main);
            }
            let ready = !self.password.is_empty() && !busy;
            let response = ui.interact(go, ui.id().with("submit-password"), if ready { Sense::click() } else { Sense::hover() });
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, "Unlock"));
            let fill = if ready { if response.hovered() { theme.main.gamma_multiply(0.85) } else { theme.main } } else { Color32::from_white_alpha(28) };
            painter.circle_filled(go.center(), 14.0, fill);
            let c = go.center();
            let stroke = Stroke::new(1.8, if ready { ui::GROUND } else { Color32::from_white_alpha(170) });
            painter.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], stroke);
            painter.line(vec![c + vec2(0.5, -4.5), c + vec2(5.0, 0.0), c + vec2(0.5, 4.5)], stroke);
            if response.clicked() { submit = true; }
            // Only what happened: checking, or why it did not unlock. No instructions.
            if !message.is_empty() {
                painter.text(pos2(cx, field.bottom() + 16.0), Align2::CENTER_CENTER, &message, ui::mono(11.5), ink);
            }

            if let (true, Some((sharp, soft, dest, uv))) = (glassy, backdrop) {
                let backdrop = crate::glass::Backdrop { sharp, soft, dest, uv };
                if let Some(shape) = self.glass.shape(ctx, screen, backdrop, &panes, theme.main, crate::glass::Look::default()) {
                    painter.set(slot, shape);
                }
            }

            // What plays, between the time and the sign-in.
            let within = Rect::from_min_max(pos2(screen.left() + 56.0, screen.top() + 40.0), pos2(screen.right() - 56.0, face.top() - 24.0));
            below(ui, within);
        });
        if submit { self.submit(ctx); }
        // The clock and the cursor keep moving.
        ctx.request_repaint_after(Duration::from_millis(250));
        false
    }

    fn paint_face(&self, ui: &Ui, painter: &egui::Painter, theme: &Theme, face: Rect, ring: Color32, busy: bool) {
        let picture = self.picture.as_ref().and_then(|picture| picture.as_ref());
        paint_face(ui, painter, theme, face, ring, busy, picture, &self.user);
    }
}

/// The DesOS bird above the date, in glass when the glass is drawn; the room it
/// takes, gap included.
pub(crate) fn bird(painter: &egui::Painter, panes: &mut Vec<crate::glass::Shape>, glassy: bool, liquid: bool, cx: f32, top: f32, short: bool) -> f32 {
    let height = if short { 34.0 } else { 92.0 };
    let rect = Rect::from_center_size(pos2(cx, top + height / 2.0), vec2(height * crate::bird::aspect(), height));
    if glassy { panes.push(crate::glass::Shape::Bird(rect)); }
    if !liquid {
        let ppp = painter.ctx().pixels_per_point();
        let texture = crate::bird::texture(painter.ctx(), (height * ppp).round() as usize);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(texture.id(), rect.translate(vec2(0.0, 2.0)), uv, Color32::from_black_alpha(60));
        painter.image(texture.id(), rect, uv, GLASS_TEXT);
    }
    height + if short { 10.0 } else { 18.0 }
}

/// The user's picture, or the pattern made from their name, in a thin ring that
/// turns while the password is checked.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_face(ui: &Ui, painter: &egui::Painter, theme: &Theme, face: Rect, ring: Color32, busy: bool, picture: Option<&TextureHandle>, user: &str) {
    let radius = face.width() / 2.0;
    if busy {
        painter.circle_stroke(face.center(), radius - 1.0, Stroke::new(2.0, Color32::from_white_alpha(40)));
        let turn = (ui.input(|i| i.time) as f32 * 4.0) % std::f32::consts::TAU;
        let arc: Vec<egui::Pos2> = (0..=16).map(|k| { let a = turn + k as f32 * std::f32::consts::FRAC_PI_2 / 16.0; face.center() + vec2(a.cos(), a.sin()) * (radius - 1.0) }).collect();
        painter.line(arc, Stroke::new(2.0, GLASS_TEXT));
        ui.ctx().request_repaint();
    } else {
        painter.circle_stroke(face.center(), radius - 1.0, Stroke::new(2.0, ring));
    }
    let inner = face.shrink(5.0);
    match picture {
        Some(texture) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.add(egui::epaint::RectShape::filled(inner, inner.width() / 2.0, Color32::WHITE).with_texture(texture.id(), uv));
        }
        None => {
            painter.circle_filled(inner.center(), inner.width() / 2.0, ui::TRACK);
            let grid = inner.shrink(inner.width() * 0.2);
            let cell = grid.width() / 5.0;
            for (row, cells) in crate::greeter::sigil(user).iter().enumerate() {
                for (column, kind) in cells.iter().enumerate() {
                    let color = match kind { 1 => theme.main, 2 => theme.muted(), _ => continue };
                    painter.rect_filled(Rect::from_min_size(grid.min + vec2(column as f32 * cell, row as f32 * cell), vec2(cell, cell)), 0.0, color);
                }
            }
        }
    }
}

/// Light text over a picture.
pub(crate) const GLASS_TEXT: Color32 = Color32::from_rgba_premultiplied(236, 242, 246, 235);
/// The clock: glass, see-through so the picture shows in the numerals.
pub(crate) const CLOCK: Color32 = Color32::from_rgba_premultiplied(150, 154, 157, 160);

/// The desktop's wallpaper as published for the panels: the sharp and the blurred
/// texture, and where they go on the screen.
pub(crate) fn wallpaper(ctx: &egui::Context) -> Option<(egui::TextureId, egui::TextureId, Rect, Rect)> {
    let (view, fit) = crate::material::wallpaper(ctx)?;
    let sharp = view.texture(false);
    let (dest, uv) = crate::wallpaper::geometry(ctx.screen_rect(), sharp.size_vec2(), fit);
    Some((sharp.id(), view.texture(true).id(), dest, uv))
}

/// A soft dark band along the top or the bottom, for the words over the picture.
pub(crate) fn scrim(painter: &egui::Painter, screen: Rect, top: bool) {
    let height = (screen.height() * if top { 0.34 } else { 0.4 }).min(380.0);
    let band = if top { Rect::from_min_size(screen.min, vec2(screen.width(), height)) } else { Rect::from_min_max(pos2(screen.left(), screen.bottom() - height), screen.max) };
    let dark = Color32::from_black_alpha(if top { 100 } else { 140 });
    let (edge, open) = if top { (band.top(), band.bottom()) } else { (band.bottom(), band.top()) };
    let mut mesh = egui::Mesh::default();
    for (y, color) in [(edge, dark), (open, Color32::TRANSPARENT)] {
        mesh.colored_vertex(pos2(band.left(), y), color);
        mesh.colored_vertex(pos2(band.right(), y), color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

/// A rounded pane of frosted glass: the blurred wallpaper behind `rect`, a light
/// wash over it and an edge. Without a wallpaper, a dark translucent pane.
pub(crate) fn glass(painter: &egui::Painter, rect: Rect, soft: Option<&(egui::TextureId, Rect, Rect)>, edge: Stroke) {
    let radius = rect.height() / 2.0;
    match soft {
        Some((texture, dest, uv)) if dest.is_positive() => {
            let local_min = (rect.min - dest.min) / dest.size();
            let local_max = (rect.max - dest.min) / dest.size();
            let sample = Rect::from_min_max(uv.min + local_min * uv.size(), uv.min + local_max * uv.size());
            painter.add(egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(*texture, sample));
            painter.rect_filled(rect, radius, Color32::from_white_alpha(18));
        }
        _ => { painter.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(20, 24, 30, 150)); }
    }
    painter.rect_stroke(rect, radius, edge, StrokeKind::Inside);
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
        // The screen gives no instructions: the keys work without being spelled out.
        for text in typed.iter().chain(&shown) {
            for word in ["Ctrl", "Enter to", "Esc", "shows or hides", "session is still running"] {
                assert!(!text.contains(word), "{text:?} explains the keys");
            }
        }
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
        struct Preview { lock: Lock, frames: usize, output: std::path::PathBuf, loader: crate::wallpaper::Loader, wallpaper: Option<std::path::PathBuf> }
        impl eframe::App for Preview {
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                for event in ctx.input(|i| i.events.clone()) {
                    if let Event::Screenshot { image, .. } = event {
                        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        image::save_buffer(&self.output, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).unwrap();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                // An optional wallpaper, published the way the desktop publishes it.
                if let Some(path) = &self.wallpaper {
                    if self.frames == 0 { self.loader.request(ctx, path, 18.0, false); }
                    self.loader.poll(ctx);
                    if self.loader.loading { self.frames = 1; ctx.request_repaint(); }
                    // The glass is worked out off the UI thread; wait for it.
                    if self.loader.view.is_some() && !self.lock.glass.ready() && self.frames > 2 { self.frames = 2; }
                    let appearance = des_common::appearance::AppearanceSettings { wallpaper: Some(path.clone()), fit: des_common::appearance::WallpaperFit::Fill, ..Default::default() };
                    crate::material::publish(ctx, appearance, self.loader.view.clone());
                }
                let mut theme = Theme::load("signal");
                // The lock screen glass setting, as written in the config (`plain`, `rrggbb`).
                if let Ok(glass) = std::env::var("DESUI_LOCK_PREVIEW_GLASS") { theme.main = crate::config::LockGlass::parse(&glass).colour(theme.main); }
                self.lock.show(ctx, &theme, Glance { battery: Some((68, false)), online: true, notices: 3 }, |_, _| {});
                self.frames += 1;
                if self.frames == 6 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
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
            let wallpaper = std::env::var_os("DESUI_LOCK_PREVIEW_WALLPAPER").map(std::path::PathBuf::from);
            Ok(Box::new(Preview { lock, frames: 0, output, loader: Default::default(), wallpaper }))
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
