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

type Area=(i32,i32,i32,i32);

/// Latest lifted rectangles are state, not one-shot window actions. In particular,
/// a clear must never disappear because an action queue is full or reconnecting.
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
struct Presentation {
    toast:Option<Option<Area>>,
    overlay:Option<Option<Area>>,
}

pub struct Desktop {
    commands: SyncSender<Command>,
    presentation:Arc<Mutex<Presentation>>,
    security: Arc<Mutex<Option<(u64, bool)>>>,
    rx: Receiver<Message>,
    pub state: DesktopState,
    /// Set when the compositor's launcher shortcut was pressed; cleared by the reader.
    pub launcher_requested: bool,
    pub dock_requested: bool,
    pub dock_screen_requested: Option<usize>,
    pub dock_dismissed: Vec<usize>,
    /// Likewise for the settings shortcut.
    pub settings_requested: bool,
    pub shortcuts_requested: bool,
    pub companion_requested: bool,
    pub companion_voice_requested: bool,
    pub companion_blur_requested: bool,
    pub companion_pointer: Option<(i32, i32, usize)>,
    /// Pending fixed companion controls, drained by the runtime each frame.
    pub companion_controls: Vec<des_common::ipc::CompanionAction>,
    /// Likewise for the music shortcut.
    pub music_requested: bool,
    /// Likewise for the notifications shortcut.
    pub notifications_requested: bool,
    /// Likewise for the quick settings shortcut.
    pub quick_requested: bool,
    /// Likewise for the power key.
    pub power_requested: bool,
    /// Screenshots the compositor took, waiting to be taken over.
    pub shots: Vec<crate::capture::Shot>,
    pub shot_failures: Vec<(Option<u64>, String)>,
    /// What the compositor has said about the recording since last looked at.
    pub recording_news: Vec<des_common::ipc::Recording>,
    /// How hand control stands, as the compositor last said.
    pub hands: des_common::ipc::Hands,
    /// Set when the compositor's lock shortcut was pressed.
    pub lock_requested: bool,
    /// Whether the compositor has the screen locked.
    pub locked: bool,
    /// The monitors, as last reported.
    pub displays: Displays,
    /// The layouts other than the plain one, on every display and workspace.
    pub layouts: Vec<des_common::ipc::LayoutInfo>,
    /// The overview or window switcher the compositor has open, for the shell to
    /// draw the names and the selection around its pictures.
    pub overview: Option<des_common::ipc::OverviewLayout>,
    pub window_shelf: bool,
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
        let presentation=Arc::new(Mutex::new(Presentation::default()));
        let worker_presentation=presentation.clone();
        std::thread::spawn(move || serve(path, requests, worker_security, worker_presentation, events, ctx));
        Desktop {
            commands,
            presentation,
            security,
            rx,
            state: DesktopState::default(),
            launcher_requested: false,
            dock_requested: false,
            dock_screen_requested: None,
            dock_dismissed: Vec::new(),
            settings_requested: false,
            shortcuts_requested: false,
            companion_requested: false,
            companion_voice_requested: false,
            companion_blur_requested: false,
            companion_pointer: None,
            companion_controls: Vec::new(),
            music_requested: false,
            notifications_requested: false,
            quick_requested: false,
            power_requested: false,
            shots: Vec::new(),
            shot_failures: Vec::new(),
            recording_news: Vec::new(),
            hands: des_common::ipc::Hands::Off,
            lock_requested: false,
            locked: false,
            displays: Displays::default(),
            layouts: Vec::new(),
            overview: None,
            window_shelf: false,
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
                Message::Disconnected => { self.heard = false; self.overview = None; self.window_shelf = false; self.companion_pointer = None; continue; }
                Message::SecurityConfirmed(generation, locked) => {
                    if generation == self.security_generation { self.security_confirmed = Some(locked); }
                    continue;
                }
                Message::Event(event) => event,
            };
            match event {
                Event::WindowShelf => self.window_shelf = true,
                Event::State(state) => {
                    self.state = state;
                }
                Event::Launcher => self.launcher_requested = true,
                Event::Dock => self.dock_requested = true,
                Event::DockOn(screen) => self.dock_screen_requested = Some(screen),
                Event::DockDismissed(screen) => self.dock_dismissed.push(screen),
                Event::Settings => self.settings_requested = true,
                Event::Shortcuts => self.shortcuts_requested = true,
                Event::Companion => self.companion_requested = true,
                Event::VoiceCompanion => self.companion_voice_requested = true,
                Event::CompanionBlur => self.companion_blur_requested = true,
                Event::CompanionPointer { x, y, screen } => self.companion_pointer = Some((x, y, screen)),
                Event::CompanionControl(action) => {
                    if self.companion_controls.len() < 16 { self.companion_controls.push(action); }
                }
                Event::Music => self.music_requested = true,
                Event::Notifications => self.notifications_requested = true,
                Event::Quick => self.quick_requested = true,
                Event::Power => self.power_requested = true,
                Event::Shot { request, mode, width, height, window, path } => self.shots.push(crate::capture::Shot { request, mode, width, height, window, path }),
                Event::ShotFailed { request, reason } => self.shot_failures.push((request, reason)),
                Event::Recording(state) => self.recording_news.push(state),
                Event::Hands(state) => self.hands = state,
                Event::Displays(displays) => self.displays = displays,
                Event::Layouts(layouts) => self.layouts = layouts,
                Event::LockRequested => self.lock_requested = true,
                Event::Locked(locked) => { self.locked = locked; if locked { self.companion_pointer = None; } },
                Event::Overview(layout) => self.overview = layout,
            }
        }
    }

    /// Whether the compositor has reported its state yet.
    pub fn connected(&self) -> bool {
        self.heard
    }

    pub fn send(&mut self, command: Command) -> std::io::Result<()> {
        match command {
            Command::Toast(area) => {
                self.presentation.lock().unwrap_or_else(|error|error.into_inner()).toast=Some(area);
                Ok(())
            }
            Command::Overlay(area) => {
                self.presentation.lock().unwrap_or_else(|error|error.into_inner()).overlay=Some(area);
                Ok(())
            }
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

fn serve(path: PathBuf, requests: Receiver<Command>, security: Arc<Mutex<Option<(u64, bool)>>>, presentation:Arc<Mutex<Presentation>>, events: SyncSender<Message>, ctx: egui::Context) {
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
        // A full wire buffer leaves the latest presentation pending for the next
        // flush. State survives reconnect; stale window indices still do not.
        let desired=*presentation.lock().unwrap_or_else(|error|error.into_inner());
        let _=link.queue_presentation(desired);
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
    presentation:Presentation,
}

impl Link {
    fn new() -> Self {
        Self { outgoing: des_common::transport::WriteBuffer::new(64 * 1024), desired_lock: None,
            queued_lock: None, known_lock: None, seen_state: false, presentation:Presentation::default() }
    }
    fn reset(&mut self) {
        self.outgoing.clear();
        self.presentation=Presentation::default();
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
    fn queue_presentation(&mut self, desired:Presentation)->std::io::Result<()> {
        // Geometry commands are ignored while locked; retain them until unlocked.
        if !self.ready() || self.known_lock!=Some(false) || self.queued_lock.is_some(){return Ok(());}
        if let Some(area)=desired.toast.filter(|_|desired.toast!=self.presentation.toast) {
            self.outgoing.enqueue(Command::Toast(area).encode().as_bytes())?;
            self.presentation.toast=Some(area);
        }
        if let Some(area)=desired.overlay.filter(|_|desired.overlay!=self.presentation.overlay) {
            self.outgoing.enqueue(Command::Overlay(area).encode().as_bytes())?;
            self.presentation.overlay=Some(area);
        }
        Ok(())
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
    #[test]
    fn overlay_clears_are_retained_when_the_action_queue_is_full() {
        let dir=tempfile::tempdir().unwrap();
        let mut desktop=Desktop::connect_at(egui::Context::default(),dir.path().join("missing.sock"));
        desktop.heard=true;
        let (commands,_held_receiver)=sync_channel(1);
        desktop.commands=commands;
        desktop.send(Command::Activate(0)).unwrap();
        desktop.send(Command::Toast(Some((10,20,360,80)))).unwrap();
        desktop.send(Command::Toast(Some((40,50,360,80)))).unwrap();
        assert!(desktop.send(Command::Toast(None)).is_ok(),"clearing a lifted region must survive queue backpressure");
        assert!(desktop.send(Command::Overlay(None)).is_ok());
        assert_eq!(*desktop.presentation.lock().unwrap(),Presentation{toast:Some(None),overlay:Some(None)});
        desktop.heard=false;
        desktop.send(Command::Overlay(Some((1,2,300,200)))).unwrap();
        desktop.send(Command::Overlay(None)).unwrap();
        assert_eq!(desktop.presentation.lock().unwrap().overlay,Some(None),"offline updates must keep the latest clear");
        assert!(desktop.send(Command::Activate(1)).is_err(),"window actions must still be rejected offline");
    }

    use super::*;

    fn greet(link: &mut Link, locked: bool) {
        link.receive(&Event::State(DesktopState::default())).unwrap();
        link.receive(&Event::Locked(locked)).unwrap();
    }

    #[test]
    fn presentation_retries_backpressure_and_reconnect_with_latest_clear() {
        let mut link=Link::new();greet(&mut link,false);
        let shown=Presentation{toast:Some(Some((10,20,360,80))),overlay:Some(Some((30,40,500,300)))};
        link.queue_presentation(shown).unwrap();
        let mut bytes=Vec::new();link.outgoing.flush(&mut bytes).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("toast 10 20 360 80"));
        let clear=Presentation{toast:Some(None),overlay:Some(None)};
        link.outgoing.enqueue(&vec![b'x';64*1024]).unwrap();
        assert!(link.queue_presentation(clear).is_err());
        assert_eq!(link.presentation,shown,"failed enqueue must leave the clear pending");
        link.outgoing.flush(&mut Vec::new()).unwrap();
        link.queue_presentation(clear).unwrap();
        let mut bytes=Vec::new();link.outgoing.flush(&mut bytes).unwrap();
        assert_eq!(bytes,b"toast none\noverlay none\n");
        link.reset();greet(&mut link,false);
        link.queue_presentation(clear).unwrap();
        let mut bytes=Vec::new();link.outgoing.flush(&mut bytes).unwrap();
        assert_eq!(bytes,b"toast none\noverlay none\n","reconnect must restore current cleared state");
        link.queue_presentation(clear).unwrap();
        let mut duplicate=Vec::new();link.outgoing.flush(&mut duplicate).unwrap();
        assert!(duplicate.is_empty());
    }

    #[test]
    fn presentation_waits_for_handshake_and_unlock() {
        let mut link=Link::new();
        let desired=Presentation{toast:Some(None),overlay:Some(None)};
        link.queue_presentation(desired).unwrap();
        assert_eq!(link.presentation,Presentation::default());
        greet(&mut link,true);
        link.queue_presentation(desired).unwrap();
        assert_eq!(link.presentation,Presentation::default());
        link.request_lock(false).unwrap();
        link.queue_presentation(desired).unwrap();
        assert_eq!(link.presentation,Presentation::default(),"wait for the unlock acknowledgement");
        link.receive(&Event::Locked(false)).unwrap();
        link.queue_presentation(desired).unwrap();
        assert_eq!(link.presentation,desired);
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
