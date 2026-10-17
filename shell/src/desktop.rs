//! The shell's link to the compositor: which applications are open, and requests to
//! switch between them. Absent when the shell runs on its own.

use des_common::ipc::{Command, DesktopState, Displays, Event, SOCKET_ENV};
use eframe::egui;
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, SyncSender, sync_channel};

pub struct Desktop {
    commands: SyncSender<Command>,
    security: Arc<Mutex<Option<(u64, bool)>>>,
    rx: Receiver<Message>,
    pub state: DesktopState,
    /// Set when the compositor's launcher shortcut was pressed; cleared by the reader.
    pub launcher_requested: bool,
    pub dock_requested: bool,
    pub dock_screen_requested: Option<usize>,
    /// Likewise for the settings shortcut.
    pub settings_requested: bool,
    /// Likewise for the music shortcut.
    pub music_requested: bool,
    /// Likewise for the notifications shortcut.
    pub notifications_requested: bool,
    /// Likewise for the power key.
    pub power_requested: bool,
    /// Screenshots the compositor took, waiting to be taken over.
    pub shots: Vec<crate::capture::Shot>,
    /// What the compositor has said about the recording since last looked at.
    pub recording_news: Vec<des_common::ipc::Recording>,
    /// Set when the compositor's lock shortcut was pressed.
    pub lock_requested: bool,
    /// Whether the compositor has the screen locked.
    pub locked: bool,
    /// The monitors, as last reported.
    pub displays: Displays,
    /// The layouts other than the plain one, on every display and workspace.
    pub layouts: Vec<des_common::ipc::LayoutInfo>,
    heard: bool,
    /// Confirmation of the most recent security intent, consumed by the UI.
    pub security_confirmed: Option<bool>,
    security_generation: u64,
}

impl Desktop {
    /// Connects to the compositor named by the environment, if there is one.
    pub fn connect(ctx: egui::Context) -> Option<Desktop> {
        let path = std::env::var_os(SOCKET_ENV)?;
        Some(Self::connect_at(ctx, PathBuf::from(path)))
    }

    fn connect_at(ctx: egui::Context, path: PathBuf) -> Desktop {
        let (commands, requests) = sync_channel(32);
        let (events, rx) = sync_channel(32);
        let security = Arc::new(Mutex::new(None));
        let worker_security = security.clone();
        std::thread::spawn(move || serve(path, requests, worker_security, events, ctx));
        Desktop {
            commands,
            security,
            rx,
            state: DesktopState::default(),
            launcher_requested: false,
            dock_requested: false,
            dock_screen_requested: None,
            settings_requested: false,
            music_requested: false,
            notifications_requested: false,
            power_requested: false,
            shots: Vec::new(),
            recording_news: Vec::new(),
            lock_requested: false,
            locked: false,
            displays: Displays::default(),
            layouts: Vec::new(),
            heard: false,
            security_confirmed: None,
            security_generation: 0,
        }
    }

    /// Takes in state updates that arrived since the last frame.
    pub fn pump(&mut self) {
        for _ in 0..64 {
            let Ok(message) = self.rx.try_recv() else { break };
            let event = match message {
                Message::Ready => { self.heard = true; continue; }
                Message::Disconnected => { self.heard = false; continue; }
                Message::SecurityConfirmed(generation, locked) => {
                    if generation == self.security_generation { self.security_confirmed = Some(locked); }
                    continue;
                }
                Message::Event(event) => event,
            };
            match event {
                Event::State(state) => {
                    self.state = state;
                }
                Event::Launcher => self.launcher_requested = true,
                Event::Dock => self.dock_requested = true,
                Event::DockOn(screen) => self.dock_screen_requested = Some(screen),
                Event::Settings => self.settings_requested = true,
                Event::Music => self.music_requested = true,
                Event::Notifications => self.notifications_requested = true,
                Event::Power => self.power_requested = true,
                Event::Shot { mode, width, height, window, path } => self.shots.push(crate::capture::Shot { mode, width, height, window, path }),
                Event::Recording(state) => self.recording_news.push(state),
                Event::Displays(displays) => self.displays = displays,
                Event::Layouts(layouts) => self.layouts = layouts,
                Event::LockRequested => self.lock_requested = true,
                Event::Locked(locked) => self.locked = locked,
            }
        }
    }

    /// Whether the compositor has reported its state yet.
    pub fn connected(&self) -> bool {
        self.heard
    }

    pub fn send(&mut self, command: Command) -> std::io::Result<()> {
        match command {
            Command::Lock | Command::Unlock => {
                // A separate slot gives security requests priority over window actions.
                self.security_generation = self.security_generation.wrapping_add(1);
                self.security_confirmed = None;
                *self.security.lock().unwrap() = Some((self.security_generation, matches!(command, Command::Lock)));
                Ok(())
            }
            _ if !self.connected() => Err(ErrorKind::NotConnected.into()),
            _ => self.commands.try_send(command).map_err(|error| match error {
                mpsc::TrySendError::Full(_) => ErrorKind::WouldBlock.into(),
                mpsc::TrySendError::Disconnected(_) => ErrorKind::NotConnected.into(),
            }),
        }
    }
}


// The worker owns all socket I/O. Bounded channels keep both directions finite.
enum Message { Event(Event), Ready, Disconnected, SecurityConfirmed(u64, bool) }

fn deliver(events: &SyncSender<Message>, ctx: &egui::Context, message: Message) -> bool {
    if events.send(message).is_err() { return false; }
    ctx.request_repaint();
    true
}

fn serve(path: PathBuf, requests: Receiver<Command>, security: Arc<Mutex<Option<(u64, bool)>>>, events: SyncSender<Message>, ctx: egui::Context) {
    let mut socket: Option<UnixStream> = None;
    let mut retry_at = Instant::now();
    let mut link = Link::new();
    let mut incoming = Vec::new();
    let mut announced = false;
    let mut desired_generation = 0;
    loop {
        if let Some((generation, locked)) = security.lock().unwrap().take() {
            desired_generation = generation;
            if link.request_lock(locked).is_err() {
                socket = None;
            } else if link.desired_lock.is_none() && !deliver(&events, &ctx, Message::SecurityConfirmed(desired_generation, locked)) {
                return;
            }
        }
        // These commands describe the current window list. Drop them while offline.
        for _ in 0..32 {
            match requests.try_recv() {
                Ok(command) if socket.is_some() && link.ready() => {
                    if link.outgoing.enqueue(command.encode().as_bytes()).is_err() { socket = None; }
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        if socket.is_none() {
            link.reset();
            incoming.clear();
            if announced {
                announced = false;
                if !deliver(&events, &ctx, Message::Disconnected) { return; }
            }
            if Instant::now() >= retry_at {
                // Connecting can wait on a full listen backlog, so it belongs here.
                match UnixStream::connect(&path).and_then(|stream| {
                    stream.set_nonblocking(true)?;
                    Ok(stream)
                }) {
                    Ok(stream) => socket = Some(stream),
                    Err(_) => retry_at = Instant::now() + Duration::from_millis(250),
                }
            }
        }
        let mut failed = false;
        if let Some(stream) = socket.as_mut() {
            if link.outgoing.flush(stream).is_err() { failed = true; }
            let mut read = 0;
            let mut bytes = [0u8; 4096];
            while !failed && read < 64 * 1024 {
                match stream.read(&mut bytes) {
                    Ok(0) => { failed = true; break; }
                    Ok(n) => { incoming.extend_from_slice(&bytes[..n]); read += n; }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => { failed = true; break; }
                }
                while let Some(end) = incoming.iter().position(|byte| *byte == b'\n') {
                    if end >= 1024 * 1024 { failed = true; break; }
                    let line: Vec<u8> = incoming.drain(..=end).collect();
                    if let Some(event) = Event::decode(String::from_utf8_lossy(&line).trim_end()) {
                        let wanted = link.desired_lock;
                        if link.receive(&event).is_err() { failed = true; break; }
                        if !deliver(&events, &ctx, Message::Event(event)) { return; }
                        if link.ready() && !announced {
                            announced = true;
                            if !deliver(&events, &ctx, Message::Ready) { return; }
                        }
                        if let Some(locked) = wanted.filter(|_| link.desired_lock.is_none()) {
                            if !deliver(&events, &ctx, Message::SecurityConfirmed(desired_generation, locked)) { return; }
                        }
                    }
                }
                if incoming.len() >= 1024 * 1024 { failed = true; }
            }
        }
        if failed {
            socket = None;
            retry_at = Instant::now() + Duration::from_millis(250);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Link {
    outgoing: des_common::transport::WriteBuffer,
    desired_lock: Option<bool>,
    queued_lock: Option<bool>,
    known_lock: Option<bool>,
    seen_state: bool,
}

impl Link {
    fn new() -> Self {
        Self { outgoing: des_common::transport::WriteBuffer::new(64 * 1024), desired_lock: None,
            queued_lock: None, known_lock: None, seen_state: false }
    }
    fn reset(&mut self) {
        self.outgoing.clear();
        // Retry the last lock intent, but never replay stale window indices.
        self.queued_lock = None;
        self.known_lock = None;
        self.seen_state = false;
    }
    fn receive(&mut self, event: &Event) -> std::io::Result<()> {
        match event {
            Event::State(_) => self.seen_state = true,
            Event::Locked(locked) => {
                self.known_lock = Some(*locked);
                if self.queued_lock == Some(*locked) { self.queued_lock = None; }
                if self.desired_lock == Some(*locked) && self.queued_lock.is_none() {
                    self.desired_lock = None;
                }
            }
            _ => {}
        }
        self.queue_security()
    }
    fn ready(&self) -> bool { self.seen_state && self.known_lock.is_some() }
    fn request_lock(&mut self, locked: bool) -> std::io::Result<()> {
        self.desired_lock = Some(locked);
        self.queue_security()
    }
    fn queue_security(&mut self) -> std::io::Result<()> {
        if self.ready() && self.queued_lock.is_none() {
            if let Some(locked) = self.desired_lock {
                if self.known_lock == Some(locked) {
                    self.desired_lock = None;
                    self.queued_lock = None;
                } else if self.queued_lock != Some(locked) {
                    self.outgoing.enqueue(if locked { b"lock\n" } else { b"unlock\n" })?;
                    self.queued_lock = Some(locked);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greet(link: &mut Link, locked: bool) {
        link.receive(&Event::State(DesktopState::default())).unwrap();
        link.receive(&Event::Locked(locked)).unwrap();
    }

    #[test]
    fn lock_requests_survive_disconnect_until_acknowledged() {
        let mut link = Link::new();
        link.request_lock(true).unwrap();
        greet(&mut link, false);
        let mut sent = Vec::new();
        link.outgoing.flush(&mut sent).unwrap();
        assert_eq!(sent, b"lock\n");
        link.reset();
        greet(&mut link, false);
        let mut resent = Vec::new();
        link.outgoing.flush(&mut resent).unwrap();
        assert_eq!(resent, b"lock\n");
        link.receive(&Event::Locked(true)).unwrap();
        link.reset();
        greet(&mut link, false);
        let mut after_ack = Vec::new();
        link.outgoing.flush(&mut after_ack).unwrap();
        assert!(after_ack.is_empty());
    }

    #[test]
    fn reconnect_discards_stale_window_commands_but_keeps_authenticated_unlock() {
        let mut link = Link::new();
        greet(&mut link, true);
        link.outgoing.enqueue(Command::Close(2).encode().as_bytes()).unwrap();
        link.request_lock(false).unwrap();
        link.reset();
        greet(&mut link, true);
        let mut sent = Vec::new();
        link.outgoing.flush(&mut sent).unwrap();
        assert_eq!(sent, b"unlock\n");
    }

    #[test]
    fn stale_security_confirmations_cannot_complete_a_new_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut desktop = Desktop::connect_at(egui::Context::default(), dir.path().join("missing.sock"));
        let (events, rx) = sync_channel(4);
        desktop.rx = rx;
        desktop.send(Command::Lock).unwrap();
        let old = desktop.security_generation;
        desktop.send(Command::Unlock).unwrap();
        events.send(Message::SecurityConfirmed(old, false)).unwrap();
        desktop.pump();
        assert_eq!(desktop.security_confirmed, None);
        events.send(Message::SecurityConfirmed(desktop.security_generation, false)).unwrap();
        desktop.pump();
        assert_eq!(desktop.security_confirmed, Some(false));
    }

    #[test]
    fn a_new_lock_intent_waits_for_the_previous_request_confirmation() {
        let mut link = Link::new();
        greet(&mut link, false);
        link.request_lock(true).unwrap();
        link.request_lock(false).unwrap();
        let mut first = Vec::new();
        link.outgoing.flush(&mut first).unwrap();
        assert_eq!(first, b"lock\n");
        link.receive(&Event::Locked(true)).unwrap();
        let mut next = Vec::new();
        link.outgoing.flush(&mut next).unwrap();
        assert_eq!(next, b"unlock\n");
        link.receive(&Event::Locked(false)).unwrap();
        link.reset();
        greet(&mut link, true);
        let mut acknowledged = Vec::new();
        link.outgoing.flush(&mut acknowledged).unwrap();
        assert!(acknowledged.is_empty());
    }

    #[test]
    fn transport_reconnects_and_retries_unacknowledged_lock() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            for connection in 0..2 {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                        Err(error) => panic!("connection did not arrive: {error}"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.write_all(DesktopState::default().encode().as_bytes()).unwrap();
                stream.write_all(b"locked 0\n").unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut command = String::new();
                reader.read_line(&mut command).unwrap();
                assert_eq!(command, "lock\n");
                if connection == 1 { stream.write_all(b"locked 1\n").unwrap(); }
                // The first connection disappears before confirming the lock.
            }
        });
        let mut desktop = Desktop::connect_at(egui::Context::default(), path);
        desktop.send(Command::Lock).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !desktop.locked && Instant::now() < deadline {
            desktop.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(desktop.locked, "lock request was lost during reconnect");
        drop(desktop);
        server.join().unwrap();
    }

    #[test]
    fn waits_for_authoritative_state_before_unlocking() {
        let mut link = Link::new();
        link.request_lock(false).unwrap();
        let mut sent = Vec::new();
        link.outgoing.flush(&mut sent).unwrap();
        assert!(sent.is_empty());
        link.receive(&Event::Locked(true)).unwrap();
        link.outgoing.flush(&mut sent).unwrap();
        assert!(sent.is_empty());
        link.receive(&Event::State(DesktopState::default())).unwrap();
        link.outgoing.flush(&mut sent).unwrap();
        assert_eq!(sent, b"unlock\n");
    }
}
