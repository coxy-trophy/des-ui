//! The login screen: an avatar, a name and a password line, and nothing else.
//!
//! It runs under greetd, the login manager, and talks to it over the socket named in
//! `GREETD_SOCK`: it hands over a user name and a password, and on success names the
//! session to start. Without that socket it runs as a preview that signs nobody in.

use crate::theme::Theme;
use crate::ui;
use eframe::egui::{
    self, Align2, CentralPanel, Color32, Event, Frame, Key, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui,
    ViewportCommand, pos2, vec2,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// Where users' pictures for the login screen are kept, readable by the login screen.
pub const AVATAR_DIR: &str = "/var/lib/des-ui/avatars";

/// A desktop that can be started after signing in.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub command: Vec<String>,
    /// The value for `XDG_CURRENT_DESKTOP`.
    pub desktop: String,
}

/// Reads a session from a `.desktop` file in `wayland-sessions`.
pub fn parse_session(text: &str) -> Option<Session> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|value| value.trim().to_string())
    };
    if field("Hidden").as_deref() == Some("true") || field("NoDisplay").as_deref() == Some("true") {
        return None;
    }
    let command = crate::apps::split_exec(&field("Exec")?);
    (!command.is_empty()).then(|| Session {
        name: field("Name").unwrap_or_else(|| command[0].clone()),
        command,
        desktop: field("DesktopNames").unwrap_or_default().trim_end_matches(';').replace(';', ":"),
    })
}

/// The sessions installed on this machine, this desktop's own first.
fn sessions() -> Vec<Session> {
    let mut found: Vec<Session> = std::fs::read_dir("/usr/share/wayland-sessions")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "desktop"))
        .filter_map(|entry| parse_session(&std::fs::read_to_string(entry.path()).ok()?))
        .collect();
    found.sort_by_key(|session| (!session.command[0].contains("des-ui"), session.name.to_lowercase()));
    found
}

/// The people who can sign in: accounts with a home and a real shell.
pub fn parse_users(passwd: &str) -> Vec<String> {
    passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let uid: u32 = fields.get(2)?.parse().ok()?;
            let shell = fields.get(6)?;
            let human = (1000..60000).contains(&uid) && !shell.ends_with("nologin") && !shell.ends_with("false");
            human.then(|| fields[0].to_string())
        })
        .collect()
}

/// A picture made from a name: a 5×5 pattern, the same on both sides, in which each
/// cell is empty (0), accent (1) or muted (2). The same name always gives the same one.
pub fn sigil(name: &str) -> [[u8; 5]; 5] {
    // FNV-1a, stirred further so that short names differ in more than one cell.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    let mut grid = [[0u8; 5]; 5];
    for (row, cells) in grid.iter_mut().enumerate() {
        for column in 0..3 {
            hash ^= hash >> 29;
            hash = hash.wrapping_mul(0xbf58476d1ce4e5b9);
            hash ^= hash >> 32;
            let cell = match hash % 5 {
                0 | 1 => 1,
                2 => 2,
                _ => 0,
            };
            cells[column] = cell;
            cells[4 - column] = cell;
        }
        // No row is left blank: an empty band reads as a gap in the picture.
        if cells.iter().all(|cell| *cell == 0) {
            cells[2] = 1 + (row as u8 % 2);
        }
    }
    grid
}

/// One message to greetd: a length, then JSON.
fn send(stream: &mut UnixStream, message: &Value) -> std::io::Result<()> {
    let body = message.to_string();
    stream.write_all(&(body.len() as u32).to_ne_bytes())?;
    stream.write_all(body.as_bytes())
}

fn receive(stream: &mut UnixStream) -> std::io::Result<Value> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > 1 << 20 {
        return Err(std::io::Error::other("greetd sent an oversized message"));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

/// Signs `user` in with `password` and asks for `session` to be started. On success
/// the login screen only has to exit. The error is a sentence for the user.
pub fn sign_in(socket: &std::path::Path, user: &str, password: &str, session: &Session) -> Result<(), String> {
    let lost = |e: std::io::Error| format!("The login service went away: {e}");
    let mut stream = UnixStream::connect(socket).map_err(lost)?;
    send(&mut stream, &json!({ "type": "create_session", "username": user })).map_err(lost)?;
    loop {
        let reply = receive(&mut stream).map_err(lost)?;
        match reply["type"].as_str() {
            Some("success") => break,
            // A question: the password, whatever it is worded as. Notices need no answer.
            Some("auth_message") => {
                let asked = matches!(reply["auth_message_type"].as_str(), Some("secret" | "visible"));
                let answer = json!({ "type": "post_auth_message_response", "response": asked.then_some(password) });
                send(&mut stream, &answer).map_err(lost)?;
            }
            _ => {
                // The attempt has to be closed before another can be made.
                let _ = send(&mut stream, &json!({ "type": "cancel_session" }));
                let _ = receive(&mut stream);
                return Err(match reply["error_type"].as_str() {
                    Some("auth_error") => "That password did not match".to_string(),
                    _ => reply["description"].as_str().unwrap_or("Signing in failed").to_string(),
                });
            }
        }
    }
    let mut env = vec!["XDG_SESSION_TYPE=wayland".to_string()];
    if !session.desktop.is_empty() {
        env.push(format!("XDG_CURRENT_DESKTOP={}", session.desktop));
        env.push(format!("XDG_SESSION_DESKTOP={}", session.desktop.split(':').next().unwrap_or_default()));
    }
    send(&mut stream, &json!({ "type": "start_session", "cmd": session.command, "env": env })).map_err(lost)?;
    match receive(&mut stream).map_err(lost)?["type"].as_str() {
        Some("success") => Ok(()),
        _ => Err("The session could not be started".to_string()),
    }
}

/// Reads a picture whatever its file is called. The pictures kept for users carry the
/// user's name and nothing to say what kind of file they are, so the kind is read
/// from what is in them.
pub fn read_picture(path: &std::path::Path) -> Option<image::DynamicImage> {
    image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()
}

/// Loads the picture kept for a user, if there is one.
pub fn load_picture(ctx: &egui::Context, user: &str) -> Option<TextureHandle> {
    let candidates = [
        PathBuf::from(AVATAR_DIR).join(user),
        PathBuf::from("/var/lib/AccountsService/icons").join(user),
    ];
    let image = candidates.iter().find_map(|path| read_picture(path))?;
    let image = image.thumbnail(320, 320).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(format!("avatar-{user}"), pixels, egui::TextureOptions::LINEAR))
}


enum State {
    Ready,
    Checking,
    Denied(String),
}

/// What a power button does, and the command that does it.
const POWER: [(&str, &str); 3] = [("Suspend", "suspend"), ("Restart", "reboot"), ("Shut down", "poweroff")];

pub struct Greeter {
    theme: Theme,
    users: Vec<String>,
    user: usize,
    sessions: Vec<Session>,
    session: usize,
    password: String,
    state: State,
    result: Option<Receiver<Result<(), String>>>,
    socket: Option<PathBuf>,
    /// The picture for the user shown, once looked for: none means draw a sigil.
    picture: Option<(usize, Option<TextureHandle>)>,
    /// A power button clicked once, waiting for the second click.
    armed: Option<(usize, Instant)>,
    /// The password is shown as typed, not as dots, until hidden again.
    revealed: bool,
    screenshot: Option<PathBuf>,
    started: Instant,
    screenshot_requested: bool,
    /// The wallpaper and accent the shown user left for the login screen, once
    /// looked for, and the loader that reads the wallpaper.
    shared: Option<(usize, Option<crate::login_share::Shared>)>,
    wallpaper: crate::wallpaper::Loader,
    /// The clock, the field and the buttons, as coloured liquid glass.
    glass: crate::glass::Glass,
}

impl Greeter {
    pub fn new(theme: Theme, screenshot: Option<PathBuf>) -> Greeter {
        let mut users = parse_users(&std::fs::read_to_string("/etc/passwd").unwrap_or_default());
        if users.is_empty() {
            users.push(std::env::var("USER").unwrap_or_else(|_| "user".to_string()));
        }
        Greeter {
            theme,
            users,
            user: 0,
            sessions: sessions(),
            session: 0,
            password: String::new(),
            state: State::Ready,
            result: None,
            socket: std::env::var_os("GREETD_SOCK").map(PathBuf::from),
            picture: None,
            armed: None,
            revealed: false,
            screenshot,
            started: Instant::now(),
            screenshot_requested: false,
            shared: None,
            wallpaper: Default::default(),
            glass: Default::default(),
        }
    }

    fn submit(&mut self, ctx: &egui::Context) {
        if self.password.is_empty() || matches!(self.state, State::Checking) {
            return;
        }
        self.state = State::Checking;
        let (tx, rx) = channel();
        self.result = Some(rx);
        let password = std::mem::take(&mut self.password);
        let user = self.users[self.user].clone();
        let session = self.sessions.get(self.session).cloned();
        let socket = self.socket.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = match (socket, session) {
                (Some(socket), Some(session)) => sign_in(&socket, &user, &password, &session),
                (Some(_), None) => Err("No session is installed to sign in to".to_string()),
                (None, _) => {
                    std::thread::sleep(Duration::from_millis(900));
                    Err("This is a preview: nothing was checked".to_string())
                }
            };
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
    }

    /// The lock screen's look, for whoever is chosen: their wallpaper whole, the
    /// date and a glass clock above, their picture, name and the password field
    /// below; the session to start at the bottom left and the power buttons at the
    /// bottom right, all glass of their accent.
    fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let user = self.users[self.user].clone();
        if self.picture.as_ref().is_none_or(|(index, _)| *index != self.user) {
            self.picture = Some((self.user, load_picture(&ctx, &user)));
        }
        if self.shared.as_ref().is_none_or(|(index, _)| *index != self.user) {
            let shared = crate::login_share::for_user(&user);
            match shared.as_ref().and_then(|s| s.wallpaper.as_ref()) {
                Some(path) => self.wallpaper.request(&ctx, path, 18.0, false),
                None => self.wallpaper.clear(),
            }
            self.shared = Some((self.user, shared));
        }
        self.wallpaper.poll(&ctx);
        self.wallpaper.error = None;
        let shared = self.shared.as_ref().and_then(|(_, shared)| shared.clone());
        let fit = shared.as_ref().map_or(des_common::appearance::WallpaperFit::Fill, |s| s.fit);
        let appearance = des_common::appearance::AppearanceSettings { wallpaper: shared.as_ref().and_then(|s| s.wallpaper.clone()), fit, ..Default::default() };
        crate::material::publish(&ctx, appearance, self.wallpaper.view.clone());
        // Their accent, where they left one.
        let mut theme = self.theme.clone();
        if let Some(accent) = shared.as_ref().and_then(|s| s.accent) { theme.main = accent; }

        let painter = ui.painter().clone();
        let backdrop = crate::lock::wallpaper(&ctx);
        let glassy = backdrop.is_some() && self.glass.available();
        let liquid = glassy && self.glass.ready();
        let mut panes = Vec::new();
        painter.rect_filled(screen, 0.0, ui::GROUND);
        let soft = match backdrop {
            Some((sharp, soft, dest, uv)) => {
                painter.image(sharp, dest, uv, Color32::WHITE);
                crate::lock::scrim(&painter, screen, true);
                crate::lock::scrim(&painter, screen, false);
                Some((soft, dest, uv))
            }
            None => { glow(&painter, screen, theme.main); None }
        };
        let slot = painter.add(egui::Shape::Noop);
        let text = crate::lock::GLASS_TEXT;
        // A pane: glass when it can be drawn, otherwise frosted.
        let pane = |panes: &mut Vec<crate::glass::Shape>, rect: Rect, edge: Stroke| {
            if glassy { panes.push(crate::glass::Shape::Pill(rect)); }
            if !liquid { crate::lock::glass(&painter, rect, soft.as_ref(), edge); }
        };

        let edge = match self.state {
            State::Ready => theme.main,
            State::Checking => ui::SECONDARY,
            State::Denied(_) => ui::CAUTION,
        };
        let denied = matches!(self.state, State::Denied(_));
        let busy = matches!(self.state, State::Checking);
        let cx = screen.center().x;
        let short = screen.height() < 640.0;

        // The date and the time, large; glass, so the picture shows through it.
        let now = chrono::Local::now();
        let mut y = screen.top() + if short { 24.0 } else { (screen.height() * 0.065).max(40.0) } + 30.0;
        let date = ui::galley(ui, &now.format("%A %-d %B").to_string(), ui::display_bold(if short { 15.0 } else { 21.0 }), text, 1.5);
        painter.galley(pos2(cx - date.size().x / 2.0, y), date.clone(), text);
        y += date.size().y - 4.0;
        let size = if short { 84.0 } else { (screen.height() * 0.2).clamp(96.0, 240.0) };
        let time = ui::galley(ui, &crate::region::hours_minutes(&now), ui::display_bold(size), crate::lock::CLOCK, 2.0);
        let at = pos2(cx - time.size().x / 2.0, y);
        if glassy { panes.push(crate::glass::Shape::Text(time.clone(), at)); }
        if !liquid {
            painter.galley(at + vec2(0.0, 3.0), time.clone(), Color32::from_black_alpha(70));
            painter.galley(at, time, crate::lock::CLOCK);
        }

        // At the bottom middle: the picture, the name and the password field.
        let side = if short { 52.0 } else { 64.0 };
        let field_height = 40.0;
        let bottom = screen.bottom() - if short { 28.0 } else { (screen.height() * 0.06).max(40.0) } - 22.0;
        let field = Rect::from_center_size(pos2(cx, bottom - field_height / 2.0), vec2(268.0_f32.min(screen.width() - 48.0), field_height));
        let name = ui::galley(ui, &user, ui::display_bold(17.0), text, 0.5);
        let name_top = field.top() - 14.0 - name.size().y;
        painter.galley(pos2(cx - name.size().x / 2.0, name_top), name, text);
        let face = Rect::from_center_size(pos2(cx, name_top - 10.0 - side / 2.0), vec2(side, side));
        let picture = self.picture.as_ref().and_then(|(_, texture)| texture.as_ref());
        crate::lock::paint_face(ui, &painter, &theme, face, if denied { ui::CAUTION } else { Color32::from_white_alpha(90) }, busy, picture, &user);
        // Either side of the picture, the way to the other accounts.
        if self.users.len() > 1 {
            for (dx, label) in [(-1.0, "Previous account"), (1.0, "Next account")] {
                let rect = Rect::from_center_size(face.center() + vec2(dx * (side / 2.0 + 34.0), 0.0), vec2(34.0, 34.0));
                let response = ui.interact(rect, ui.id().with(("account", label)), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
                pane(&mut panes, rect, Stroke::new(1.0, Color32::from_white_alpha(36)));
                if response.hovered() { painter.circle_filled(rect.center(), 17.0, Color32::from_white_alpha(22)); }
                let c = rect.center();
                painter.line(vec![c + vec2(2.5 * dx, -5.0), c + vec2(-2.5 * dx, 0.0), c + vec2(2.5 * dx, 5.0)], Stroke::new(1.8, text));
                if response.on_hover_text(label).clicked() { if dx < 0.0 { self.previous_user(); } else { self.next_user(); } }
            }
        }

        let field_edge = if denied { ui::CAUTION } else if self.password.is_empty() { Color32::from_white_alpha(56) } else { edge };
        pane(&mut panes, field, Stroke::new(1.0, field_edge));
        if liquid && (denied || !self.password.is_empty()) { painter.rect_stroke(field, field.height() / 2.0, Stroke::new(1.0, field_edge), StrokeKind::Inside); }
        let go = Rect::from_center_size(pos2(field.right() - 20.0, field.center().y), vec2(28.0, 28.0));
        let room = Rect::from_min_max(pos2(field.left() + 16.0, field.top()), pos2(go.left() - 8.0, field.bottom()));
        let mut x = room.left();
        if self.revealed && !self.password.is_empty() {
            // The end of what was typed, where the caret is, stays in view.
            let mut chars: Vec<char> = self.password.chars().collect();
            let mut galley = ui::galley(ui, &self.password, ui::mono(14.0), text, 0.0);
            while galley.size().x > room.width() - 6.0 && !chars.is_empty() {
                chars.remove(0);
                galley = ui::galley(ui, &format!("…{}", chars.iter().collect::<String>()), ui::mono(14.0), text, 0.0);
            }
            painter.galley(pos2(x, field.center().y - galley.size().y / 2.0), galley.clone(), text);
            x += galley.size().x + 2.0;
        } else if self.password.is_empty() {
            painter.text(pos2(x, field.center().y), Align2::LEFT_CENTER, "Enter password", ui::display(13.5), Color32::from_white_alpha(150));
        } else {
            let fit = ((room.width() - 8.0) / 14.0).max(1.0) as usize;
            for _ in 0..self.password.chars().count().min(fit) {
                painter.circle_filled(pos2(x + 3.5, field.center().y), 3.5, text);
                x += 14.0;
            }
        }
        if !busy && !self.password.is_empty() && ui.input(|i| i.time).rem_euclid(1.0) < 0.5 {
            painter.rect_filled(Rect::from_center_size(pos2(x + 1.0, field.center().y), vec2(2.0, 18.0)), 0.0, theme.main);
        }
        let ready = !self.password.is_empty() && !busy;
        let response = ui.interact(go, ui.id().with("submit-password"), if ready { Sense::click() } else { Sense::hover() });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, "Sign in"));
        let fill = if ready { if response.hovered() { theme.main.gamma_multiply(0.85) } else { theme.main } } else { Color32::from_white_alpha(28) };
        painter.circle_filled(go.center(), 14.0, fill);
        let c = go.center();
        let stroke = Stroke::new(1.8, if ready { ui::GROUND } else { Color32::from_white_alpha(170) });
        painter.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], stroke);
        painter.line(vec![c + vec2(0.5, -4.5), c + vec2(5.0, 0.0), c + vec2(0.5, 4.5)], stroke);
        if response.clicked() { self.submit(&ctx); }
        // Only what happened: checking, or why it did not sign in. No instructions.
        let message = match &self.state {
            State::Ready => None,
            State::Checking => Some(("Checking…".to_string(), text)),
            State::Denied(why) => Some((why.clone(), ui::CAUTION)),
        };
        if let Some((message, ink)) = message {
            painter.text(pos2(cx, field.bottom() + 16.0), Align2::CENTER_CENTER, message, ui::mono(11.5), ink);
        }

        let inner = screen.shrink2(vec2(40.0, 32.0));
        // Bottom left: which desktop starts.
        let session_name = self.sessions.get(self.session).map_or("None installed", |session| session.name.as_str()).to_string();
        let chosen = ui::galley(ui, &session_name, ui::display_bold(13.0), text, 0.5);
        let several = self.sessions.len() > 1;
        let pill = Rect::from_min_size(pos2(inner.left(), inner.bottom() - 36.0), vec2(chosen.size().x + if several { 52.0 } else { 36.0 }, 36.0));
        let response = ui.interact(pill, ui.id().with("session"), if several { Sense::click() } else { Sense::hover() });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, several, format!("Session: {session_name}")));
        pane(&mut panes, pill, Stroke::new(1.0, Color32::from_white_alpha(36)));
        if response.hovered() && several { painter.rect_filled(pill, 18.0, Color32::from_white_alpha(22)); }
        let chosen_width = chosen.size().x;
        painter.galley(pos2(pill.left() + 18.0, pill.center().y - chosen.size().y / 2.0), chosen, text);
        if several {
            let c = pos2(pill.left() + 30.0 + chosen_width, pill.center().y);
            painter.line(vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)], Stroke::new(1.6, text));
            if response.on_hover_text("Another session (F1)").clicked() {
                self.session = (self.session + 1) % self.sessions.len();
            }
        }

        // Bottom right: power, round glass buttons with their names under them. Each
        // takes two clicks, so that none happens by accident.
        self.armed = self.armed.filter(|(_, at)| at.elapsed() < Duration::from_secs(4));
        for (index, (name, command)) in POWER.iter().enumerate() {
            let column = Rect::from_min_size(pos2(inner.right() - 84.0 * (POWER.len() - index) as f32, inner.bottom() - 62.0), vec2(84.0, 62.0));
            let circle = Rect::from_center_size(pos2(column.center().x, column.top() + 20.0), vec2(40.0, 40.0));
            let armed = self.armed.is_some_and(|(which, _)| which == index);
            let response = ui.interact(column, ui.id().with(("power", index)), Sense::click());
            let label = if armed { format!("Click again to {}", name.to_lowercase()) } else { name.to_string() };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            let ink = if armed { ui::CAUTION } else { text };
            pane(&mut panes, circle, Stroke::new(1.0, if armed { ui::CAUTION } else { Color32::from_white_alpha(36) }));
            if response.hovered() { painter.circle_filled(circle.center(), 20.0, Color32::from_white_alpha(22)); }
            if armed && liquid { painter.circle_stroke(circle.center(), 19.5, Stroke::new(1.0, ui::CAUTION)); }
            power_icon(&painter, circle.center(), index, Stroke::new(1.5, ink));
            let caption = if armed { format!("Again to {}", name.to_lowercase()) } else { name.to_string() };
            painter.text(pos2(column.center().x, column.bottom() - 6.0), Align2::CENTER_CENTER, caption, ui::mono(10.5), ink);
            if response.clicked() {
                if armed {
                    self.armed = None;
                    let _ = std::process::Command::new("systemctl").arg(command).spawn();
                } else {
                    self.armed = Some((index, Instant::now()));
                }
            }
        }

        if let (true, Some((sharp, soft, dest, uv))) = (glassy, backdrop) {
            let backdrop = crate::glass::Backdrop { sharp, soft, dest, uv };
            if let Some(shape) = self.glass.shape(&ctx, screen, backdrop, &panes, theme.main, crate::glass::Look::default()) {
                painter.set(slot, shape);
            }
        }
    }

    fn next_user(&mut self) {
        self.user = (self.user + 1) % self.users.len();
        self.password.clear();
        self.revealed = false;
        self.state = State::Ready;
    }

    fn previous_user(&mut self) {
        self.user = (self.user + self.users.len() - 1) % self.users.len();
        self.password.clear();
        self.revealed = false;
        self.state = State::Ready;
    }
}

/// A soft light of the accent above the middle of the screen.
pub fn glow(painter: &egui::Painter, screen: Rect, color: Color32) {
    let center = pos2(screen.center().x, screen.top() + screen.height() * 0.32);
    let radius = screen.width().min(screen.height() * 1.4) * 0.5;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color.gamma_multiply(0.11));
    let steps = 64;
    let rings = [(0.45, 0.06), (1.0, 0.0)];
    for (ring, &(reach, strength)) in rings.iter().enumerate() {
        for step in 0..steps {
            let a = step as f32 * std::f32::consts::TAU / steps as f32;
            mesh.colored_vertex(center + vec2(a.cos(), a.sin() * 0.7) * radius * reach, color.gamma_multiply(strength));
        }
        let base = 1 + ring as u32 * steps;
        for step in 0..steps {
            let (a, b) = (base + step, base + (step + 1) % steps);
            if ring == 0 { mesh.add_triangle(0, a, b); } else { mesh.add_triangle(a - steps, a, b); mesh.add_triangle(a - steps, b, b - steps); }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Draws the suspend (moon), restart (turning arrow) and shut down (power) marks.
fn power_icon(painter: &egui::Painter, center: egui::Pos2, which: usize, stroke: Stroke) {
    let arc = |from: f32, to: f32, radius: f32| -> Vec<egui::Pos2> {
        (0..=24)
            .map(|step| {
                let angle = (from + (to - from) * step as f32 / 24.0).to_radians();
                center + vec2(radius * angle.cos(), radius * angle.sin())
            })
            .collect()
    };
    match which {
        0 => {
            painter.line(arc(-60.0, 150.0, 7.5), stroke);
            let inner: Vec<egui::Pos2> = arc(-75.0, 165.0, 7.5).into_iter().map(|p| p + vec2(3.2, -3.2)).rev().collect();
            painter.line(inner[6..19].to_vec(), stroke);
        }
        1 => {
            let ring = arc(-30.0, 270.0, 6.5);
            painter.line(ring.clone(), stroke);
            let tip = ring[0];
            painter.line_segment([tip, tip + vec2(-5.0, 0.5)], stroke);
            painter.line_segment([tip, tip + vec2(0.5, -5.0)], stroke);
        }
        _ => {
            painter.line(arc(-55.0, 235.0, 6.5), stroke);
            painter.line_segment([center + vec2(0.0, -8.5), center + vec2(0.0, -1.0)], stroke);
        }
    }
}

impl eframe::App for Greeter {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui::GROUND.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(outcome) = self.result.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.result = None;
            match outcome {
                // greetd starts the session as soon as the login screen is gone.
                Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
                Err(why) => { self.state = State::Denied(why); self.revealed = false; }
            }
        }

        // The keyboard belongs to the password line: no field has to be clicked first.
        for event in ctx.input(|i| i.events.clone()) {
            if matches!(self.state, State::Checking) {
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
                    Key::F1 if !self.sessions.is_empty() => self.session = (self.session + 1) % self.sessions.len(),
                    Key::Tab if self.users.len() > 1 => self.next_user(),
                    _ => {}
                },
                _ => {}
            }
        }

        CentralPanel::default()
            .frame(Frame::new().fill(ui::GROUND))
            .show(ctx, |ui| self.draw(ui));

        // For testing: capture one frame and leave.
        if let Some(path) = self.screenshot.clone() {
            // Once the wallpaper and its glass are ready, or after a while regardless.
            let settled = !self.wallpaper.loading && (self.wallpaper.view.is_none() || self.glass.ready() || !self.glass.available());
            if !self.screenshot_requested && self.started.elapsed() > Duration::from_secs(2) && (settled || self.started.elapsed() > Duration::from_secs(30)) {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let mut data = format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
                for pixel in &image.pixels {
                    data.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
                }
                let _ = std::fs::write(path, data);
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
        // The clock and the cursor keep moving.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

/// Stops using a picture: the screens go back to the pattern made from the name.
pub fn remove_avatar() -> Result<(), String> {
    let user = std::env::var("USER").map_err(|_| "cannot tell who you are: USER is not set".to_string())?;
    let target = PathBuf::from(AVATAR_DIR).join(&user);
    match std::fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", target.display())),
    }
}

/// The pictures in these folders that could be someone's picture, newest first within
/// each folder, and no more than `most` in all.
pub fn pictures_in(folders: &[PathBuf], most: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for folder in folders {
        let mut here: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let kind = path.extension()?.to_str()?.to_ascii_lowercase();
                let meta = entry.metadata().ok()?;
                (meta.is_file() && matches!(kind.as_str(), "png" | "jpg" | "jpeg")).then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), path))
            })
            .collect();
        here.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        found.extend(here.into_iter().map(|(_, path)| path));
    }
    found.truncate(most);
    found
}

/// Copies a picture to where the login screen can read it, as a small PNG.
pub fn set_avatar(source: &std::path::Path) -> Result<PathBuf, String> {
    let user = std::env::var("USER").map_err(|_| "cannot tell who you are: USER is not set".to_string())?;
    let image = read_picture(source).ok_or_else(|| format!("{} is not a picture this desktop can read (PNG or JPEG)", source.display()))?;
    // Square, from the middle, and no larger than the screen shows it.
    let side = image.width().min(image.height());
    let (x, y) = ((image.width() - side) / 2, (image.height() - side) / 2);
    let square = image.crop_imm(x, y, side, side).thumbnail(320, 320);
    let target = PathBuf::from(AVATAR_DIR).join(&user);
    square
        .save_with_format(&target, image::ImageFormat::Png)
        .map_err(|e| format!("cannot write {}: {e} (has ./install-login.sh been run?)", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn sigils_are_stable_symmetric_and_distinct() {
        let a = sigil("coxy");
        assert_eq!(a, sigil("coxy"));
        for row in a {
            assert_eq!([row[0], row[1]], [row[4], row[3]]);
            assert!(row.iter().any(|cell| *cell != 0), "no blank rows");
        }
        let different = ["alice", "bob", "root", "coxy2", ""].iter().filter(|name| sigil(name) != a).count();
        assert_eq!(different, 5);
    }

    #[test]
    fn reads_a_picture_whose_name_says_nothing() {
        // As the pictures kept for users are: named after the user, with no ending.
        let path = std::env::temp_dir().join(format!("des-face-{}", std::process::id()));
        image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255])).save_with_format(&path, image::ImageFormat::Png).unwrap();
        let read = read_picture(&path).expect("read by what is in it");
        assert_eq!((read.width(), read.height()), (4, 3));
        std::fs::write(&path, b"not a picture").unwrap();
        assert!(read_picture(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lists_pictures_newest_first() {
        let folder = std::env::temp_dir().join(format!("des-faces-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("album.png")).unwrap();
        for name in ["old.JPG", "notes.txt", "new.png"] {
            std::fs::write(folder.join(name), b"x").unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        let found = pictures_in(&[folder.clone(), folder.join("missing")], 10);
        assert_eq!(found, [folder.join("new.png"), folder.join("old.JPG")], "pictures only, and no folders");
        assert_eq!(pictures_in(&[folder.clone()], 1).len(), 1);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn reads_sessions_and_users() {
        let session = parse_session("[Desktop Entry]\nName=Ubuntu\nExec=/usr/bin/gnome-session --session=ubuntu\nDesktopNames=ubuntu;GNOME\n").unwrap();
        assert_eq!(session.name, "Ubuntu");
        assert_eq!(session.command, ["/usr/bin/gnome-session", "--session=ubuntu"]);
        assert_eq!(session.desktop, "ubuntu:GNOME");
        assert_eq!(parse_session("[Desktop Entry]\nName=X\n"), None);
        assert_eq!(parse_session("[Desktop Entry]\nName=X\nExec=x\nHidden=true\n"), None);

        let passwd = "root:x:0:0:root:/root:/bin/bash\ndaemon:x:1:1::/usr/sbin:/usr/sbin/nologin\n\
                      ada:x:1000:1000:Ada:/home/ada:/usr/bin/zsh\nsvc:x:1001:1001::/home/svc:/usr/sbin/nologin\n\
                      nobody:x:65534:65534::/nonexistent:/usr/sbin/nologin\nbob:x:1002:1002::/home/bob:/bin/bash\n";
        assert_eq!(parse_users(passwd), ["ada", "bob"]);
    }

    /// A stand-in for greetd that accepts one password.
    fn fake_greetd(path: PathBuf, right: &'static str) -> std::thread::JoinHandle<Vec<Value>> {
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            loop {
                let Ok(message) = receive(&mut stream) else { break };
                seen.push(message.clone());
                let reply = match message["type"].as_str().unwrap() {
                    "create_session" => json!({"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password: "}),
                    "post_auth_message_response" if message["response"] == right => json!({"type": "success"}),
                    "post_auth_message_response" => json!({"type": "error", "error_type": "auth_error", "description": "pam says no"}),
                    _ => json!({"type": "success"}),
                };
                send(&mut stream, &reply).unwrap();
            }
            seen
        })
    }

    #[test]
    fn signs_in_through_the_login_service() {
        let session = Session { name: "DEs-UI".into(), command: vec!["/usr/local/bin/des-ui-session".into()], desktop: "des-ui".into() };
        let dir = std::env::temp_dir().join(format!("des-greeter-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("good.sock");
        let server = fake_greetd(path.clone(), "open sesame");
        assert_eq!(sign_in(&path, "ada", "open sesame", &session), Ok(()));
        let seen = server.join().unwrap();
        assert_eq!(seen[0]["username"], "ada");
        let start = seen.last().unwrap();
        assert_eq!(start["type"], "start_session");
        assert_eq!(start["cmd"][0], "/usr/local/bin/des-ui-session");
        assert!(start["env"].as_array().unwrap().iter().any(|v| v == "XDG_CURRENT_DESKTOP=des-ui"));

        let path = dir.join("bad.sock");
        let server = fake_greetd(path.clone(), "open sesame");
        assert_eq!(sign_in(&path, "ada", "guess", &session), Err("That password did not match".to_string()));
        let seen = server.join().unwrap();
        assert_eq!(seen.last().unwrap()["type"], "cancel_session", "a failed attempt is closed");
        assert!(!seen.iter().any(|m| m["type"] == "start_session"));

        assert!(sign_in(&dir.join("absent.sock"), "ada", "x", &session).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
