//! Locking by itself: before the machine sleeps, and when asked to from outside.
//!
//! The system's login service (logind) says when the machine is about to sleep, and
//! waits a moment for the lock to go up first. Applications that are showing
//! something to watch say so over D-Bus, which holds off the lock that comes with
//! being left alone; that lock itself is the compositor's doing, as it sees the input.

use eframe::egui;
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

fn watch_sleep(shared: Arc<Mutex<Shared>>, ctx: egui::Context) -> zbus::Result<()> {
    let system = Connection::system()?;
    let manager = Proxy::new(&system, LOGIN, "/org/freedesktop/login1", "org.freedesktop.login1.Manager")?;

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
    pub fn start(ctx: egui::Context) -> Power {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let for_sleep = shared.clone();
        std::thread::spawn(move || {
            if let Err(e) = watch_sleep(for_sleep, ctx) {
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
