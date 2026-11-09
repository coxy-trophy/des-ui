//! Locking by itself: before the machine sleeps, and when asked to from outside.
//!
//! The system's login service (logind) says when the machine is about to sleep, and
//! waits a moment for the lock to go up first. Applications that are showing
//! something to watch say so over D-Bus, which holds off the lock that comes with
//! being left alone; that lock itself is the compositor's doing, as it sees the input.

use crate::sysmon::Battery;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align, Key, Layout, Modifiers, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};

const LOGIN: &str = "org.freedesktop.login1";

#[derive(Default)]
struct Shared {
    /// Set when something asked for the lock: sleep coming, or `loginctl lock-session`.
    lock_wanted: bool,
    /// Held while sleep waits on the lock; dropping it lets the machine sleep.
    waiting: Option<(OwnedFd, Instant)>,
    /// Applications asking that the screen be left alone: a cookie, and who asked.
    holds: Vec<(u32, String)>,
    last_cookie: u32,
}

pub struct Power {
    shared: Arc<Mutex<Shared>>,
    session: Option<Connection>,
    checked: Instant,
    holding: bool,
}

/// Asks logind to wait for this program before sleeping.
fn delay_sleep(manager: &Proxy) -> Option<OwnedFd> {
    let fd: zbus::zvariant::OwnedFd = manager
        .call("Inhibit", &("sleep", "DEs-UI", "Locking the screen", "delay"))
        .map_err(|e| eprintln!("des-ui: cannot lock before sleep: {e}"))
        .ok()?;
    Some(fd.into())
}

fn watch_sleep(shared: Arc<Mutex<Shared>>, ctx: egui::Context, own_power_key: bool) -> zbus::Result<()> {
    let system = Connection::system()?;
    let manager = Proxy::new(&system, LOGIN, "/org/freedesktop/login1", "org.freedesktop.login1.Manager")?;

    // The power key opens the power menu, for as long as this shell is there to show
    // it; without this the system switches off at the first press.
    let _power_key: Option<zbus::zvariant::OwnedFd> = own_power_key
        .then(|| manager.call("Inhibit", &("handle-power-key", "DEs-UI", "Showing the power menu", "block")))
        .and_then(|held| held.map_err(|e| eprintln!("des-ui: the power key keeps its usual meaning: {e}")).ok());

    // `loginctl lock-session`, and whatever else locks sessions through logind.
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        let path: zbus::Result<zbus::zvariant::OwnedObjectPath> = manager.call("GetSession", &(id.as_str(),));
        if let Ok(path) = path {
            let (system, shared, ctx) = (system.clone(), shared.clone(), ctx.clone());
            std::thread::spawn(move || {
                let Ok(session) = Proxy::new(&system, LOGIN, path, "org.freedesktop.login1.Session") else { return };
                let Ok(locks) = session.receive_signal("Lock") else { return };
                for _ in locks {
                    shared.lock().unwrap().lock_wanted = true;
                    ctx.request_repaint();
                }
            });
        }
    }

    let mut held = delay_sleep(&manager);
    for signal in manager.receive_signal("PrepareForSleep")? {
        let going: bool = signal.body().deserialize().unwrap_or(false);
        if going {
            if let Some(fd) = held.take() {
                let mut shared = shared.lock().unwrap();
                shared.lock_wanted = true;
                shared.waiting = Some((fd, Instant::now()));
            }
            ctx.request_repaint();
            // Whatever happens to the lock, sleep is not held up for long.
            let shared = shared.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                shared.lock().unwrap().waiting = None;
            });
        } else {
            // Awake again: be ready for the next time.
            shared.lock().unwrap().waiting = None;
            held = delay_sleep(&manager);
            ctx.request_repaint();
        }
    }
    Ok(())
}

/// The service applications call to keep the screen as it is while they play something.
struct ScreenSaver {
    shared: Arc<Mutex<Shared>>,
}

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn inhibit(&self, #[zbus(header)] header: zbus::message::Header<'_>, _application: String, _reason: String) -> u32 {
        let sender = header.sender().map(|name| name.to_string()).unwrap_or_default();
        let mut shared = self.shared.lock().unwrap();
        shared.last_cookie = shared.last_cookie.wrapping_add(1).max(1);
        let cookie = shared.last_cookie;
        shared.holds.push((cookie, sender));
        cookie
    }

    fn un_inhibit(&self, cookie: u32) {
        self.shared.lock().unwrap().holds.retain(|(held, _)| *held != cookie);
    }
}

impl Power {
    /// `own_power_key` when this shell is the desktop, and so can show the power menu.
    pub fn start(ctx: egui::Context, own_power_key: bool) -> Power {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let for_sleep = shared.clone();
        std::thread::spawn(move || {
            if let Err(e) = watch_sleep(for_sleep, ctx, own_power_key) {
                eprintln!("des-ui: not watching for sleep: {e}");
            }
        });
        let saver = || ScreenSaver { shared: shared.clone() };
        let session = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.freedesktop.ScreenSaver"))
            .and_then(|builder| builder.serve_at("/org/freedesktop/ScreenSaver", saver()))
            .and_then(|builder| builder.serve_at("/ScreenSaver", saver()))
            .and_then(|builder| builder.build())
            .map_err(|e| eprintln!("des-ui: applications cannot hold off the lock: {e}"))
            .ok();
        Power { shared, session, checked: Instant::now(), holding: false }
    }

    /// Whether something asked for the screen to be locked since last asked here.
    pub fn lock_wanted(&mut self) -> bool {
        std::mem::take(&mut self.shared.lock().unwrap().lock_wanted)
    }

    /// Whether the machine is waiting on the lock before it sleeps.
    pub fn sleep_waiting(&self) -> bool {
        self.shared.lock().unwrap().waiting.is_some()
    }

    /// Lets the machine sleep: the lock is up, or there is to be none.
    pub fn let_sleep(&self) {
        self.shared.lock().unwrap().waiting = None;
    }

    /// Whether an application is asking that the screen be left alone. Looked into
    /// every few seconds, dropping the requests of applications that have gone.
    pub fn holding(&mut self) -> bool {
        if self.checked.elapsed() < Duration::from_secs(5) {
            return self.holding;
        }
        self.checked = Instant::now();
        let holds = self.shared.lock().unwrap().holds.clone();
        let mut gone = Vec::new();
        if let Some(bus) = self.session.as_ref().and_then(|session| zbus::blocking::fdo::DBusProxy::new(session).ok()) {
            for (cookie, sender) in &holds {
                let alive = zbus::names::BusName::try_from(sender.as_str())
                    .ok()
                    .and_then(|name| bus.name_has_owner(name).ok())
                    .unwrap_or(false);
                if !alive {
                    gone.push(*cookie);
                }
            }
        }
        let mut shared = self.shared.lock().unwrap();
        shared.holds.retain(|(cookie, _)| !gone.contains(cookie));
        self.holding = !shared.holds.is_empty();
        self.holding
    }
}

/// What the power menu was asked to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    Lock,
    Suspend,
    Restart,
    ShutDown,
    LogOut,
}

impl Act {
    /// Carries out what the system does; locking and logging out are the shell's own.
    pub fn run(self) {
        let verb = match self {
            Act::Suspend => "suspend",
            Act::Restart => "reboot",
            Act::ShutDown => "poweroff",
            Act::Lock | Act::LogOut => return,
        };
        if let Err(e) = std::process::Command::new("systemctl").arg(verb).spawn() {
            eprintln!("des-ui: cannot {verb}: {e}");
        }
    }
}

/// Name, what it does, the act, and whether it takes a second click to mean it.
const CHOICES: [(&str, &str, Act, bool); 5] = [
    ("LOCK", "Hide everything behind your password", Act::Lock, false),
    ("SLEEP", "Everything stays as it is, using almost no power", Act::Suspend, false),
    ("RESTART", "Closes everything and starts again", Act::Restart, true),
    ("SHUT DOWN", "Closes everything and switches off", Act::ShutDown, true),
    ("LOG OUT", "Closes everything and returns to the login screen", Act::LogOut, true),
];

/// Hours and minutes, as the frame writes a duration.
pub fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} MIN"),
        (hours, 0) => format!("{hours} H"),
        (hours, minutes) => format!("{hours} H {minutes:02} MIN"),
    }
}

/// The battery in a line, for the power menu.
pub fn battery_line(battery: Battery) -> String {
    let left = battery.minutes.map(duration);
    match (battery.plugged, left) {
        (true, Some(left)) => format!("Battery at {}%, charging. Full in {}.", battery.percent, left.to_lowercase()),
        (true, None) => format!("Battery at {}%, on mains power.", battery.percent),
        (false, Some(left)) => format!("Battery at {}%. About {} left.", battery.percent, left.to_lowercase()),
        (false, None) => format!("Battery at {}%.", battery.percent),
    }
}

/// The power menu, in place of the terminal.
#[derive(Default)]
pub struct Menu {
    pub open: bool,
    /// The choice clicked once and waiting for its second click, and when.
    armed: Option<(Act, Instant)>,
}

impl Menu {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.armed = None;
    }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme, battery: Option<Battery>) -> Option<Act> {
        self.armed = self.armed.filter(|(_, at)| at.elapsed() < WAIT);
        // Esc first lets go of a choice waiting for its second click, then closes.
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            if self.armed.take().is_none() { self.open = false; }
            return None;
        }
        let mut chosen = None;
        let margin = (rect.width() * 0.05).clamp(16.0, 56.0);
        let area = Rect::from_min_max(rect.min + vec2(margin, 24.0), rect.max - vec2(margin, 14.0));
        // The head: the name, the battery, and Done.
        let mut head = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(area.min, vec2(area.width(), 34.0))).layout(Layout::left_to_right(Align::Center)));
        head.spacing_mut().item_spacing.x = 14.0;
        ui::tracked(&mut head, "POWER", ui::display_bold(20.0), ui::TEXT, 3.0);
        if let Some(battery) = battery {
            let low = !battery.plugged && battery.percent <= 15;
            let line = battery_line(battery);
            let short = line.trim_end_matches('.').replacen("Battery at ", "", 1).replace(". ", " · ").replace(", ", " · ");
            crate::panels::pill(&mut head, &short, if low { ui::CAUTION } else { ui::SECONDARY }).on_hover_text(line);
        }
        head.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui::button(ui, theme, "DONE", Some("ESC"), false).clicked() { self.open = false; }
        });

        // The five choices across the middle; past a divider, the three that close
        // everything.
        let body = Rect::from_min_max(pos2(area.left(), area.top() + 50.0), area.max);
        let places = choice_rects(body, CHOICES.len());
        // A click anywhere but a choice lets go of one that is waiting.
        let pressed_elsewhere = ui.input(|input| input.pointer.any_pressed() && input.pointer.interact_pos().is_some_and(|at| rect.contains(at) && !places.iter().any(|place| place.contains(at))));
        if pressed_elsewhere { self.armed = None; }
        if places.len() == 5 && places[1].center().y == places[2].center().y {
            let x = (places[1].right() + places[2].left()) / 2.0;
            let y = places[1].top() + 38.0;
            ui.painter().vline(x, egui::Rangef::new(y - 44.0, y + 44.0), Stroke::new(1.0, ui::LINE));
        }
        let waiting = self.armed.map(|(act, _)| act);
        for ((name, what, act, twice), place) in CHOICES.iter().zip(&places) {
            let response = ui.interact(*place, ui.id().with(("power", *name)), Sense::click());
            let armed = waiting == Some(*act);
            let dim = waiting.is_some() && !armed;
            let spoken = if armed { format!("Click again to {}", name.to_lowercase()) } else { format!("{}: {what}", title_case(name)) };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
            let painter = ui.painter();
            let fade = |c: egui::Color32| if dim { c.gamma_multiply(0.4) } else { c };
            let centre = pos2(place.center().x, place.top() + 38.0);
            let hot = response.hovered() || response.has_focus();
            let (ring, ink, fill) = if armed {
                (ui::CAUTION, ui::CAUTION, ui::CAUTION.gamma_multiply(0.10))
            } else if hot {
                (theme.main, theme.main, ui::TRACK)
            } else {
                (ui::LINE_STRONG, ui::SECONDARY, ui::PANEL)
            };
            if armed { painter.circle_filled(centre, 44.0, ui::CAUTION.gamma_multiply(0.06)); }
            painter.circle_filled(centre, 38.0, fade(fill));
            painter.circle_stroke(centre, 37.5, Stroke::new(1.0, fade(ring)));
            if response.has_focus() { painter.circle_stroke(centre, 41.0, Stroke::new(1.0, theme.main)); }
            paint_choice(painter, centre, *act, fade(ink));
            let (title, under) = if let Some((_, at)) = self.armed.filter(|_| armed) {
                // A ring around it runs down the time left for the second click.
                let left = (WAIT.as_secs_f32() - at.elapsed().as_secs_f32()).max(0.0);
                let share = left / WAIT.as_secs_f32();
                let start = -std::f32::consts::FRAC_PI_2;
                let points = (0..=48).map(|i| {
                    let a = start + share * std::f32::consts::TAU * i as f32 / 48.0;
                    centre + vec2(a.cos(), a.sin()) * 43.0
                }).collect::<Vec<_>>();
                painter.line(points, Stroke::new(2.5, ui::CAUTION));
                ui.ctx().request_repaint();
                ("CLICK AGAIN".to_string(), format!("to {} · {} s", name.to_lowercase(), left.ceil() as u32))
            } else {
                (name.to_string(), what.to_string())
            };
            let name_ink = if armed { ui::CAUTION } else if hot { ui::TEXT } else { fade(ui::TEXT) };
            let galley = painter.layout_no_wrap(title, ui::display_bold(13.0), name_ink);
            painter.galley(pos2(centre.x - galley.size().x / 2.0, place.top() + 88.0), galley, name_ink);
            let mut job = egui::text::LayoutJob::simple(under, ui::mono(11.0), if armed { ui::CAUTION } else { fade(ui::LABEL) }, place.width());
            job.halign = Align::Center;
            let galley = painter.layout_job(job);
            painter.galley(pos2(centre.x, place.top() + 110.0), galley, ui::LABEL);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if *twice && !armed {
                    self.armed = Some((*act, Instant::now()));
                } else {
                    chosen = Some(*act);
                }
            }
        }
        ui.allocate_rect(rect, Sense::hover());
        if chosen.is_some() {
            self.open = false;
            self.armed = None;
        }
        chosen
    }
}

/// How long a choice that closes everything waits for its second click.
const WAIT: Duration = Duration::from_secs(5);

/// Where each choice goes: in one row across the middle when they fit, else in rows
/// of as many as fit, each row centred.
fn choice_rects(body: Rect, count: usize) -> Vec<Rect> {
    let (width, height, gap) = (170.0_f32, 150.0_f32, 22.0_f32);
    let divider = 24.0;
    let one_row = count as f32 * (width + gap) - gap + divider;
    // When they do not fit in one row, narrower columns, so more go to a row.
    let width = if body.width() >= one_row { width } else { 140.0_f32.min(body.width()) };
    let per_row = if body.width() >= one_row { count } else { (((body.width() + gap) / (width + gap)).floor() as usize).clamp(1, count - 1) };
    let rows = count.div_ceil(per_row);
    let total_height = rows as f32 * height + (rows - 1) as f32 * 12.0;
    let top = body.top() + ((body.height() - total_height) / 2.0).max(0.0);
    (0..count).map(|index| {
        let (row, column) = (index / per_row, index % per_row);
        let in_row = (count - row * per_row).min(per_row);
        let extra = if in_row == count { divider } else { 0.0 };
        let row_width = in_row as f32 * width + (in_row - 1) as f32 * gap + extra;
        let left = body.center().x - row_width / 2.0 + column as f32 * (width + gap) + if in_row == count && column >= 2 { extra } else { 0.0 };
        Rect::from_min_size(pos2(left, top + row as f32 * (height + 12.0)), vec2(width, height))
    }).collect()
}

/// "SHUT DOWN" as "Shut down", for the screen reader.
fn title_case(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut letters = lower.chars();
    letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
}

/// The outline of a crescent: the edge of a disc at `c` left uncovered by a second
/// disc moved by `shift`, then back along the second disc's edge.
fn crescent(c: egui::Pos2, outer: f32, shift: egui::Vec2, inner: f32) -> Vec<egui::Pos2> {
    let steps = 180;
    let tau = std::f32::consts::TAU;
    let on = |centre: egui::Pos2, radius: f32, a: f32| centre + vec2(a.cos(), a.sin()) * radius;
    let hollow = c + shift;
    let shows = |i: usize| (on(c, outer, i as f32 / steps as f32 * tau) - hollow).length() > inner;
    let Some(start) = (0..steps).find(|&i| !shows(i) && shows((i + 1) % steps)).map(|i| i + 1) else {
        return (0..=steps).map(|i| on(c, outer, i as f32 / steps as f32 * tau)).collect();
    };
    let mut points: Vec<egui::Pos2> = (0..steps).map(|k| (start + k) % steps).take_while(|&i| shows(i)).map(|i| on(c, outer, i as f32 / steps as f32 * tau)).collect();
    let (first, last) = (points[0], points[points.len() - 1]);
    // Back from the last point to the first along the hollow's edge, the way that
    // stays inside the disc.
    let from = (last - hollow).angle();
    let mut to = (first - hollow).angle();
    while to < from { to += tau; }
    let middle = on(hollow, inner, (from + to) / 2.0);
    let to = if (middle - c).length() <= outer { to } else { to - tau };
    points.extend((1..=48).map(|i| on(hollow, inner, from + (to - from) * i as f32 / 48.0)));
    points
}

/// The picture inside a choice's ring, drawn larger than the shell's icons.
fn paint_choice(painter: &egui::Painter, c: egui::Pos2, act: Act, color: egui::Color32) {
    let stroke = Stroke::new(1.8, color);
    let pi = std::f32::consts::PI;
    let arc = |radius: f32, from: f32, to: f32| (0..=40).map(|i| { let a = from + (to - from) * i as f32 / 40.0; c + vec2(a.cos(), a.sin()) * radius }).collect::<Vec<_>>();
    match act {
        Act::Lock => {
            painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, 4.0), vec2(20.0, 15.0)), 3.0, stroke, egui::StrokeKind::Middle);
            let mut shackle = vec![c + vec2(-6.0, -3.5)];
            shackle.extend(arc(6.0, pi, 2.0 * pi).into_iter().map(|p| p + vec2(0.0, -6.5)));
            shackle.push(c + vec2(6.0, -3.5));
            painter.line(shackle, stroke);
            painter.circle_filled(c + vec2(0.0, 3.0), 1.6, color);
            painter.line_segment([c + vec2(0.0, 3.0), c + vec2(0.0, 6.5)], Stroke::new(1.6, color));
        }
        Act::Suspend => { painter.line(crescent(c, 10.5, vec2(-5.5, -4.5), 9.0), stroke); }
        Act::Restart => {
            let (from, to) = (-pi / 2.0 + 0.6, 1.5 * pi - 0.6);
            painter.line(arc(10.5, from, to), stroke);
            // The head at the end, pointing on round the circle.
            let tip = c + vec2(to.cos(), to.sin()) * 10.5;
            let along = vec2(-to.sin(), to.cos());
            let out = vec2(to.cos(), to.sin());
            painter.line(vec![tip - along * 5.0 + out * 4.0, tip, tip - along * 5.0 - out * 4.0], stroke);
        }
        Act::ShutDown => {
            painter.line(arc(10.5, -pi / 2.0 + 0.7, 1.5 * pi - 0.7), stroke);
            painter.line_segment([c + vec2(0.0, -13.0), c + vec2(0.0, -1.0)], stroke);
        }
        Act::LogOut => {
            painter.line(vec![c + vec2(2.0, -11.0), c + vec2(10.0, -11.0), c + vec2(10.0, 11.0), c + vec2(2.0, 11.0)], stroke);
            painter.line_segment([c + vec2(-11.0, 0.0), c + vec2(4.0, 0.0)], stroke);
            painter.line(vec![c + vec2(-6.0, -5.0), c + vec2(-11.0, 0.0), c + vec2(-6.0, 5.0)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_fit_wide_and_narrow_areas() {
        for width in [320.0, 700.0, 1280.0] {
            let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 700.0));
            let places = choice_rects(body, 5);
            assert_eq!(places.len(), 5);
            for place in &places { assert!(body.contains_rect(*place), "{place:?} outside at {width}"); }
            for (i, a) in places.iter().enumerate() {
                for b in &places[i + 1..] { assert!(!a.intersects(*b), "overlap at {width}"); }
            }
        }
        let wide = choice_rects(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 700.0)), 5);
        assert!(wide.iter().all(|place| place.top() == wide[0].top()), "one row when they fit");
    }

    #[test]
    fn restart_waits_for_a_second_click_and_esc_lets_it_go() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None); ctx.enable_accesskit();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let mut menu = Menu { open: true, armed: None };
        let frame = |menu: &mut Menu, events: Vec<egui::Event>| {
            let mut act = None;
            let output = ctx.run(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| { act = menu.show(ui, screen, &Theme::load("signal"), None); });
            });
            (output, act)
        };
        let restart = |output: egui::FullOutput| {
            let bounds = output.platform_output.accesskit_update.unwrap().nodes.into_iter().find_map(|(_, node)|
                node.label().is_some_and(|label| label.starts_with("Restart") || label == "Click again to restart").then(|| node.bounds()).flatten()).expect("restart");
            pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0)
        };
        let click = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut menu, Vec::new());
        let (output, _) = frame(&mut menu, Vec::new());
        let at = restart(output);
        frame(&mut menu, vec![egui::Event::PointerMoved(at), click(at, true)]);
        let (_, act) = frame(&mut menu, vec![click(at, false)]);
        assert_eq!(act, None, "the first click only asks");
        assert!(menu.armed.is_some() && menu.open);
        let escape = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&mut menu, vec![escape.clone()]);
        assert!(menu.armed.is_none() && menu.open, "Esc lets go of the waiting choice first");
        frame(&mut menu, Vec::new());
        frame(&mut menu, vec![click(at, true)]);
        frame(&mut menu, vec![click(at, false)]);
        frame(&mut menu, vec![click(at, true)]);
        let (_, act) = frame(&mut menu, vec![click(at, false)]);
        assert_eq!(act, Some(Act::Restart));
        assert!(!menu.open);
    }

    #[test]
    fn writes_durations_and_the_battery() {
        assert_eq!(duration(45), "45 MIN");
        assert_eq!(duration(120), "2 H");
        assert_eq!(duration(72), "1 H 12 MIN");
        let battery = |plugged, minutes| Battery { percent: 23, plugged, minutes };
        assert_eq!(battery_line(battery(false, Some(72))), "Battery at 23%. About 1 h 12 min left.");
        assert_eq!(battery_line(battery(true, Some(40))), "Battery at 23%, charging. Full in 40 min.");
        assert_eq!(battery_line(battery(true, None)), "Battery at 23%, on mains power.");
    }
}
