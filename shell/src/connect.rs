//! Native DES Connect controls. The render thread never opens a socket or launches a process.
use crate::{theme::Theme, ui};
use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, VecDeque}, io::{BufRead, BufReader, Read, Write}, os::unix::net::UnixStream, path::PathBuf, process::{Command, Stdio}, sync::mpsc::{self, Receiver}, time::{Duration, Instant}};

const POLL: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const SCREEN_START_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_REPLY: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Device {
    id: String,
    name: String,
    kind: String,
    connected: bool,
    paired: bool,
    pair_state: String,
    attempt: Option<u64>,
    verification_key: Option<String>,
    battery: Option<u8>,
    charging: bool,
    last_ping: Option<i64>,
    error: Option<String>,
    files_enabled: bool,
    files_reconnect_needed: bool,
}

impl Device {
    fn files_ready(&self) -> bool { self.paired && self.connected && self.files_enabled && !self.files_reconnect_needed }
    fn verification_code(&self) -> Option<&str> {
        self.verification_key.as_deref().filter(|code| code.len() == 8 && code.bytes().all(|c| c.is_ascii_hexdigit()))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Snapshot {
    name: String,
    running: bool,
    devices: Vec<Device>,
    screens: Option<Screens>,
    screen_capabilities: Option<ScreenCapabilities>,
    transfers: Vec<Transfer>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Transfer {
    id: String,
    peer_id: String,
    direction: String,
    filename: String,
    size: u64,
    transferred: u64,
    state: String,
    error: Option<String>,
    path: Option<String>,
}
impl Transfer {
    fn cancellable(&self) -> bool { !self.id.is_empty() && matches!(self.state.as_str(), "offered" | "connecting" | "waiting" | "transferring") }
    fn incoming_offer(&self) -> bool { !self.id.is_empty() && self.direction == "receive" && self.state == "offered" }
    fn progress(&self) -> f32 { if self.size == 0 { 0.0 } else { (self.transferred.min(self.size) as f64 / self.size as f64) as f32 } }
}

#[derive(Clone)]
struct FileDraft { device_id: String, device_name: String, path: String }

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Screens {
    mode: String,
    serial: Option<String>,
    running: bool,
    control: bool,
    message: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ScreenCapabilities {
    phone_mirror: bool,
    extended_output: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    state: Option<Snapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Request {
    op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attempt: Option<u64>,
    #[serde(flatten)]
    args: BTreeMap<String, serde_json::Value>,
}
impl Request {
    fn state() -> Self { Self { op: "state", device_id: None, attempt: None, args: BTreeMap::new() } }
    fn device(op: &'static str, device: &Device) -> Self {
        Self { op, device_id: Some(device.id.clone()), attempt: matches!(op, "accept" | "reject").then_some(device.attempt).flatten(), args: BTreeMap::new() }
    }
    fn screen(op: &'static str, device: &Device, serial: &str) -> Self {
        let mut request = Self::device(op, device);
        request.args.insert("serial".into(), serde_json::json!(serial.trim()));
        if op == "mirror_phone" {
            request.args.insert("control".into(), serde_json::json!(false));
        } else if op == "extend" {
            for (key, value) in [("width", 960), ("height", 600), ("fps", 15)] {
                request.args.insert(key.into(), serde_json::json!(value));
            }
        }
        request
    }
    fn files_policy(device: &Device, enabled: bool) -> Self {
        let mut request = Self::device("set_files", device);
        request.args.insert("enabled".into(), serde_json::json!(enabled));
        request
    }
    fn send_file(draft: &FileDraft) -> Self {
        let mut request = Self { op: "send_file", device_id: Some(draft.device_id.clone()), ..Self::state() };
        request.args.insert("path".into(), serde_json::json!(draft.path));
        request
    }
    fn transfer(op: &'static str, transfer: &Transfer) -> Self {
        let mut request = Self { op, device_id: (!transfer.peer_id.is_empty()).then(|| transfer.peer_id.clone()), ..Self::state() };
        request.args.insert("transfer_id".into(), serde_json::json!(transfer.id));
        request
    }
    fn is_priority(&self) -> bool { matches!(self.op, "stop" | "stop_screen" | "cancel_transfer") }
    fn feedback(&self) -> &'static str {
        match self.op {
            "discover" => "Discovery requested. Keep DES Connect open on your phone.",
            "pair" => "Pairing requested. Compare the code on both devices.",
            "accept" => "Pairing accepted. Waiting for confirmation from your phone.",
            "reject" => "Pairing rejected.",
            "unpair" => "Device unpaired.",
            "ping" => "Ping sent.",
            "mirror_phone" => "USB screen mirror started in view-only mode.",
            "extend" => "Desktop extended to the USB device.",
            "stop_screen" => "Screen sharing stopped.",
            "stop" => "DES Connect stopped.",
            "set_files" => "File sharing setting saved. It applies on the next phone connection.",
            "reconnect" => "Reconnect requested. Waiting for the phone to connect again.",
            "send_file" => "File send requested. Check transfer progress.",
            "accept_transfer" => "Incoming file accepted. Check transfer progress.",
            "cancel_transfer" => "File cancellation requested.",
            _ => "Service connected.",
        }
    }
}

fn socket_path() -> PathBuf {
    std::env::var_os("DES_CONNECT_SOCKET").map(PathBuf::from).unwrap_or_else(|| {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw())));
        runtime.join("des-connect/control.sock")
    })
}

fn connect_local(path: &std::path::Path) -> Result<UnixStream, String> {
    use rustix::net::{socket_with, connect, AddressFamily, SocketType, SocketFlags, SocketAddrUnix};
    // AF_UNIX can block on a full listen backlog before read/write timeouts apply.
    let fd = socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK, None).map_err(|e| e.to_string())?;
    let address = SocketAddrUnix::new(path).map_err(|e| e.to_string())?;
    connect(&fd, &address).map_err(|e| e.to_string())?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    Ok(stream)
}

fn exchange(path: &std::path::Path, request: &Request) -> Result<Reply, String> {
    let mut stream = connect_local(path).map_err(|e| format!("DES Connect service unavailable: {e}. Start the service or retry."))?;
    let read_timeout = if matches!(request.op, "mirror_phone" | "extend") { SCREEN_START_TIMEOUT } else { IO_TIMEOUT };
    stream.set_read_timeout(Some(read_timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut stream, request).map_err(|e| format!("Could not send command: {e}"))?;
    stream.write_all(b"\n").map_err(|e| format!("Could not send command: {e}"))?;
    let mut bytes = Vec::new();
    BufReader::new(stream.take(MAX_REPLY + 1)).read_until(b'\n', &mut bytes)
        .map_err(|e| format!("Service did not respond: {e}. Retry the command."))?;
    if bytes.len() as u64 > MAX_REPLY || !bytes.ends_with(b"\n") {
        return Err("Invalid or incomplete service reply. Retry after restarting DES Connect.".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Unrecognized service reply: {e}. Update DES Connect and retry."))
}

fn start_service(path: &std::path::Path) -> Result<Reply, String> {
    // Respect an existing daemon; launching a second instance is unnecessary.
    if let Ok(reply) = exchange(path, &Request::state()) { return Ok(reply); }
    let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.join("des-connectd")));
    let executable = sibling.filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("des-connectd"));
    let mut child = Command::new(executable).arg("--socket").arg(path)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|e| format!("Could not start DES Connect: {e}. Install des-connectd beside des-ui or on PATH."))?;
    let deadline = Instant::now() + Duration::from_secs(4);
    let result = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Err(format!("DES Connect exited ({status}). Check its configuration, then retry."));
        }
        if let Ok(reply) = exchange(path, &Request::state()) { break Ok(reply); }
        if Instant::now() >= deadline { break Err("DES Connect started but is not responding. Check the service and retry.".into()); }
        std::thread::sleep(Duration::from_millis(150));
    };
    // Reap an explicitly started daemon on eventual exit without tying it to shell lifetime.
    std::thread::spawn(move || { let _ = child.wait(); });
    result
}

struct Pending {
    rx: Receiver<Result<Reply, String>>,
    feedback: &'static str,
    passive: bool,
    op: &'static str,
}

#[derive(Default)]
pub struct Connect {
    snapshot: Option<Snapshot>,
    pending: Option<Pending>,
    last_poll: Option<Instant>,
    error: Option<String>,
    notice: Option<String>,
    confirm_unpair: Option<String>,
    usb_serial: String,
    queued_controls: VecDeque<Request>,
    file_paths: BTreeMap<String, String>,
    confirm_send: Option<FileDraft>,
}
impl Connect {
    /// Drain results even when hidden, but only request state while the panel is visible.
    pub fn tick(&mut self, ctx: &egui::Context, visible: bool) {
        if let Some(pending) = &self.pending {
            let response = match pending.rx.try_recv() {
                Ok(response) => Some(response),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("DES Connect worker stopped. Retry the command.".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(response) = response {
                let pending = self.pending.take().unwrap();
                match response {
                    Ok(reply) => {
                        if let Some(state) = reply.state {
                            self.file_paths.retain(|id, _| state.devices.iter().any(|device| &device.id == id && device.paired));
                            if self.confirm_send.as_ref().is_some_and(|draft| !state.devices.iter().any(|device| device.id == draft.device_id && device.files_ready())) { self.confirm_send = None; }
                            self.snapshot = Some(state);
                        }
                        if reply.ok {
                            self.error = None;
                            if pending.op == "stop" { self.snapshot = None; }
                            if pending.op == "stop_screen" {
                                if let Some(screens) = self.snapshot.as_mut().and_then(|state| state.screens.as_mut()) { screens.running = false; }
                            }
                            if !pending.passive { self.notice = Some(pending.feedback.into()); }
                        } else {
                            self.notice = None;
                            self.error = Some(reply.error.unwrap_or_else(|| "The service rejected this command. Refresh and retry.".into()));
                        }
                    }
                    Err(error) => { self.snapshot = None; self.notice = None; self.error = Some(error); }
                }
            }
        }
        if self.pending.is_none() {
            if let Some(control) = self.queued_controls.pop_front() { self.request(ctx, control, false); }
        }
        if visible && self.pending.is_none() && self.last_poll.is_none_or(|at| at.elapsed() >= POLL) {
            self.request(ctx, Request::state(), false);
        }
        if self.pending.is_some() { ctx.request_repaint_after(Duration::from_millis(100)); }
        else if visible { ctx.request_repaint_after(POLL.saturating_sub(self.last_poll.map_or(POLL, |at| at.elapsed()))); }
    }

    fn request(&mut self, ctx: &egui::Context, request: Request, start: bool) {
        if self.pending.is_some() {
            if request.is_priority() {
                if self.pending.as_ref().is_some_and(|pending| pending.op == "stop" || (request.op != "cancel_transfer" && pending.op == request.op)) { return; }
                if self.queued_controls.iter().any(|queued| queued.op == "stop") { return; }
                // Preserve ordering without losing cancellation of a different transfer.
                if request.op == "stop" { self.queued_controls.clear(); self.queued_controls.push_front(request); }
                else if !self.queued_controls.contains(&request) {
                    if self.queued_controls.len() >= 32 {
                        self.error = Some("Too many cancellations pending. Retry after the current command finishes.".into());
                        return;
                    }
                    if request.op == "stop_screen" { self.queued_controls.push_front(request); }
                    else { self.queued_controls.push_back(request); }
                }
                self.notice = Some("Cancellation or stop requested. Waiting for the current command to finish…".into());
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            return;
        }
        let (tx, rx) = mpsc::channel();
        let path = socket_path();
        let feedback = if start { "DES Connect started." } else { request.feedback() };
        let passive = request.op == "state" && !start;
        let op = request.op;
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = if start { start_service(&path) } else { exchange(&path, &request) };
            let _ = tx.send(result);
            context.request_repaint();
        });
        self.last_poll = Some(Instant::now());
        if !passive { self.notice = Some("Working…".into()); }
        self.pending = Some(Pending { rx, feedback, passive, op });
    }

    fn transfers_panel(&mut self, ui: &mut egui::Ui, theme: &Theme, snapshot: &Snapshot, busy: bool) {
        if snapshot.transfers.is_empty() { return; }
        ui.add_space(ui::SPACE);
        ui::section(ui, "FILE TRANSFERS", |_| {});
        for transfer in &snapshot.transfers {
            ui.push_id(("file-transfer", &transfer.id), |ui| {
                copy(ui, if transfer.filename.is_empty() { "Unnamed file" } else { &transfer.filename }, ui::TEXT);
                let peer = snapshot.devices.iter().find(|device| device.id == transfer.peer_id);
                let name = peer.map(|device| device.name.as_str()).filter(|name| !name.is_empty()).unwrap_or("Disconnected device");
                let direction = if transfer.direction == "receive" { "From" } else { "To" };
                copy(ui, &format!("{direction} {name}"), ui::SECONDARY);
                let status = match transfer.state.as_str() {
                    "offered" => "Waiting for your approval", "connecting" => "Connecting", "waiting" => "Waiting for the phone",
                    "transferring" => "Transferring", "complete" => "Complete", "failed" => "Failed", "cancelled" => "Cancelled", "expired" => "Offer expired", _ => "Status unavailable",
                };
                copy(ui, status, if transfer.error.is_some() { ui::CAUTION } else { ui::SECONDARY });
                copy(ui, &format!("{} / {}", crate::quicklook::human_size(transfer.transferred), crate::quicklook::human_size(transfer.size)), ui::SECONDARY);
                if matches!(transfer.state.as_str(), "connecting" | "waiting" | "transferring") {
                    ui.add(egui::ProgressBar::new(transfer.progress()).desired_width(ui.available_width()).show_percentage());
                }
                if let Some(error) = &transfer.error { copy(ui, error, ui::CAUTION); }
                if transfer.state == "complete" {
                    if let Some(path) = &transfer.path {
                        copy(ui, "Saved to", ui::SECONDARY);
                        ui.add(egui::Label::new(RichText::new(path).font(ui::mono(11.0)).color(ui::TEXT)).wrap());
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if transfer.incoming_offer() {
                        ui.add_enabled_ui(!busy && peer.is_some_and(Device::files_ready), |ui| {
                            if ui::button(ui, theme, "Accept file", None, false).clicked() {
                                self.request(ui.ctx(), Request::transfer("accept_transfer", transfer), false);
                            }
                        });
                    }
                    // Deliberately independent of connection, policy and other in-flight commands.
                    if transfer.cancellable() && ui::button(ui, theme, "Cancel transfer", None, false).clicked() {
                        self.request(ui.ctx(), Request::transfer("cancel_transfer", transfer), false);
                    }
                });
                if transfer.incoming_offer() && !peer.is_some_and(Device::files_ready) {
                    copy(ui, "Connect the trusted device and apply its file sharing setting before accepting.", ui::SECONDARY);
                }
                ui.add_space(4.0);
                ui.separator();
            });
        }
        ui.add_space(ui::SPACE);
    }

    fn file_controls(&mut self, ui: &mut egui::Ui, theme: &Theme, device: &Device, busy: bool) {
        ui.add_space(4.0);
        ui.add_enabled_ui(!busy, |ui| {
            if crate::settings::switch_row(ui, theme, "File sharing", "Allow file sends and incoming offers.", device.files_enabled) {
                self.confirm_send = None;
                self.request(ui.ctx(), Request::files_policy(device, !device.files_enabled), false);
            }
        });
        if device.files_reconnect_needed {
            copy(ui, "Reconnect the phone to apply this setting. Turn its connection off and on, or reopen DES Connect on Android.", ui::SECONDARY);
            ui.add_enabled_ui(!busy && device.connected, |ui| {
                if ui::button(ui, theme, "Reconnect to apply", None, false).clicked() {
                    self.confirm_send = None;
                    self.request(ui.ctx(), Request::device("reconnect", device), false);
                }
            });
        }
        if !device.files_enabled { return; }
        if !device.files_ready() {
            if !device.connected { copy(ui, "Connect this device to send or accept files.", ui::SECONDARY); }
            return;
        }
        copy(ui, "Incoming files always need your approval.", ui::SECONDARY);
        ui.add_space(4.0);
        let label = ui.label(RichText::new("Path from Files panel").font(ui::display(13.0)).color(ui::TEXT));
        let mut path = self.file_paths.get(&device.id).cloned().unwrap_or_default();
        ui.add_enabled_ui(!busy, |ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut path).id_salt(("connect-file-path", &device.id)).char_limit(4096).desired_width(ui.available_width()).hint_text("/absolute/path/to/file")).labelled_by(label.id);
            if response.changed() {
                self.file_paths.insert(device.id.clone(), path.clone());
                if self.confirm_send.as_ref().is_some_and(|draft| draft.device_id == device.id) { self.confirm_send = None; }
            }
        });
        copy(ui, "Find the file in the bottom Files panel, then paste its full absolute path here. The service checks the file when you send.", ui::SECONDARY);
        if !path.is_empty() { if let Some(error) = file_path_error(&path) { copy(ui, error, ui::CAUTION); } }
        ui.add_enabled_ui(!busy && file_path_error(&path).is_none(), |ui| {
            if ui::button(ui, theme, "Review send", None, false).clicked() {
                self.confirm_send = Some(FileDraft { device_id: device.id.clone(), device_name: device.name.clone(), path: path.clone() });
            }
        });
        if let Some(draft) = self.confirm_send.as_ref().filter(|draft| draft.device_id == device.id).cloned() {
            copy(ui, &format!("Send this file to {}?", draft.device_name), ui::TEXT);
            ui.add(egui::Label::new(RichText::new(&draft.path).font(ui::mono(11.0)).color(ui::TEXT)).wrap());
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui::button(ui, theme, "Confirm send", None, false).clicked() {
                        self.confirm_send = None;
                        self.request(ui.ctx(), Request::send_file(&draft), false);
                    }
                    if ui::button(ui, theme, "Cancel", None, false).clicked() { self.confirm_send = None; }
                });
            });
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        self.tick(ui.ctx(), true);
        ui::section(ui, "DES CONNECT", |ui| ui::meta(ui, if self.snapshot.as_ref().is_some_and(|s| s.running) { "ONLINE" } else { "OFFLINE" }, ui::SECONDARY));
        let busy = self.pending.is_some();
        let online = self.snapshot.as_ref().is_some_and(|s| s.running);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if !online && ui::button(ui, theme, "Start service", None, false).clicked() {
                    self.request(ui.ctx(), Request::state(), true);
                }
                if online && ui::button(ui, theme, "Rescan", None, false).clicked() {
                    self.request(ui.ctx(), Request { op: "discover", ..Request::state() }, false);
                }
                if ui::button(ui, theme, "Refresh", None, false).clicked() {
                    self.request(ui.ctx(), Request::state(), false);
                }
            });
            if online && ui::button(ui, theme, "Stop service", None, false).clicked() {
                self.request(ui.ctx(), Request { op: "stop", ..Request::state() }, false);
            }
        });
        ui.add_space(ui::SPACE);
        egui::ScrollArea::vertical().id_salt("connect-devices").auto_shrink([false, false]).show(ui, |ui| {
            if let Some(error) = &self.error { copy(ui, error, ui::CAUTION); }
            if let Some(notice) = &self.notice { copy(ui, notice, ui::SECONDARY); }
            if busy { copy(ui, "Contacting DES Connect…", ui::SECONDARY); }
            let Some(snapshot) = self.snapshot.clone().filter(|s| s.running) else {
                if self.error.is_none() && !busy { copy(ui, "Start DES Connect to discover your phone on this network.", ui::SECONDARY); }
                return;
            };
            if !snapshot.name.is_empty() { copy(ui, &format!("This computer: {}", snapshot.name), ui::SECONDARY); }
            self.transfers_panel(ui, theme, &snapshot, busy);
            let screens_running = snapshot.screens.as_ref().is_some_and(|screens| screens.running);
            if let Some(screens) = &snapshot.screens {
                if screens.running {
                    ui.add_space(ui::SPACE);
                    copy(ui, match screens.mode.as_str() { "phone" => "USB screen mirror active", "extended" => "Extended USB display active", _ => "Screen sharing active" }, theme.main);
                    if let Some(serial) = &screens.serial { copy(ui, &format!("USB device: {serial}"), ui::SECONDARY); }
                    copy(ui, if screens.control { "Remote input active" } else { "View only" }, ui::SECONDARY);
                    if ui::button(ui, theme, "Stop sharing", None, false).clicked() {
                        self.request(ui.ctx(), Request { op: "stop_screen", ..Request::state() }, false);
                    }
                }
                if !screens.message.is_empty() { copy(ui, &screens.message, ui::SECONDARY); }
            }
            if let Some(capabilities) = &snapshot.screen_capabilities {
                if capabilities.phone_mirror || capabilities.extended_output {
                    ui.add_space(ui::SPACE);
                    let label = ui.label(RichText::new("USB device").font(ui::display(13.0)).color(ui::TEXT));
                    ui.add_enabled_ui(!busy && !screens_running, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.usb_serial).id_salt("connect-usb-serial").hint_text("USB serial").desired_width(ui.available_width())).labelled_by(label.id);
                    });
                    copy(ui, "Enter the USB serial explicitly. USB debugging must be enabled and authorized on that device. Pairing alone does not identify a USB device.", ui::SECONDARY);
                }
                if !capabilities.extended_output { copy(ui, "Extended USB display is unavailable on this desktop.", ui::SECONDARY); }
                if !capabilities.message.is_empty() { copy(ui, &capabilities.message, ui::SECONDARY); }
            }
            if snapshot.devices.is_empty() {
                copy(ui, "No devices found. Open DES Connect on your phone, connect both devices to the same network, then Rescan.", ui::SECONDARY);
            }
            for device in &snapshot.devices {
                ui.add_space(ui::SPACE);
                ui.separator();
                ui.add_space(4.0);
                ui.push_id(&device.id, |ui| {
                    copy(ui, if device.name.is_empty() { "Unnamed device" } else { &device.name }, ui::TEXT);
                    let status = if device.paired { if device.connected { "Connected · Trusted" } else { "Offline · Trusted" } }
                        else if device.connected { "Available · Not paired" } else { "Offline · Not paired" };
                    copy(ui, status, if device.connected { theme.main } else { ui::SECONDARY });
                    if !device.kind.is_empty() { copy(ui, &device.kind, ui::SECONDARY); }
                    if let Some(battery) = device.battery.filter(|n| *n <= 100) {
                        copy(ui, &format!("Battery {battery}%{}", if device.charging { " · Charging" } else { "" }), ui::SECONDARY);
                    }
                    if let Some(error) = &device.error { copy(ui, error, ui::CAUTION); }
                    if matches!(device.pair_state.as_str(), "incoming" | "outgoing" | "accepting") {
                        copy(ui, match device.pair_state.as_str() {
                            "incoming" => "Incoming pairing request",
                            "outgoing" => "Waiting for your phone to accept",
                            _ => "Confirming pairing…",
                        }, ui::TEXT);
                        if let Some(attempt) = device.attempt { copy(ui, &format!("Attempt {attempt}"), ui::SECONDARY); }
                        else { copy(ui, "Pairing details incomplete. Refresh before responding.", ui::CAUTION); }
                        if let Some(code) = device.verification_code() {
                            ui.label(RichText::new(code).font(ui::mono(20.0)).color(ui::TEXT));
                            copy(ui, "Compare this code on both devices. Accept only if they match.", ui::SECONDARY);
                        } else { copy(ui, "Verification code unavailable. Do not accept until it appears.", ui::CAUTION); }
                    }
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if device.pair_state == "incoming" {
                                ui.add_enabled_ui(device.attempt.is_some() && device.verification_code().is_some(), |ui| {
                                    if ui::button(ui, theme, "Accept", None, false).clicked() { self.request(ui.ctx(), Request::device("accept", device), false); }
                                });
                            }
                            if matches!(device.pair_state.as_str(), "incoming" | "outgoing") {
                                ui.add_enabled_ui(device.attempt.is_some(), |ui| {
                                    if ui::button(ui, theme, "Reject", None, false).clicked() { self.request(ui.ctx(), Request::device("reject", device), false); }
                                });
                            } else if !device.paired && device.connected && device.pair_state != "accepting" {
                                if ui::button(ui, theme, "Pair", None, false).clicked() { self.request(ui.ctx(), Request::device("pair", device), false); }
                            }
                            if device.paired {
                                ui.add_enabled_ui(device.connected, |ui| {
                                    if ui::button(ui, theme, "Ping", None, false).clicked() { self.request(ui.ctx(), Request::device("ping", device), false); }
                                });
                                if ui::button(ui, theme, "Unpair", None, false).clicked() { self.confirm_unpair = Some(device.id.clone()); }
                            }
                        });
                        if self.confirm_unpair.as_deref() == Some(&device.id) {
                            copy(ui, "Remove trust for this device? Pair again to reconnect.", ui::CAUTION);
                            ui.horizontal_wrapped(|ui| {
                                if ui::button(ui, theme, "Confirm unpair", None, false).clicked() {
                                    self.confirm_unpair = None;
                                    self.request(ui.ctx(), Request::device("unpair", device), false);
                                }
                                if ui::button(ui, theme, "Cancel", None, false).clicked() { self.confirm_unpair = None; }
                            });
                        }
                    });
                    if device.paired { self.file_controls(ui, theme, device, busy); }
                    if device.paired && device.connected {
                        if let Some(capabilities) = &snapshot.screen_capabilities {
                            if capabilities.phone_mirror || capabilities.extended_output {
                                ui.add_space(4.0);
                                ui.add_enabled_ui(!busy && !screens_running && !self.usb_serial.trim().is_empty(), |ui| {
                                    if capabilities.phone_mirror && ui::button(ui, theme, "Mirror USB screen", None, false).clicked() {
                                        self.request(ui.ctx(), Request::screen("mirror_phone", device, &self.usb_serial), false);
                                    }
                                    if capabilities.extended_output && ui::button(ui, theme, "Extend to USB device", None, false).clicked() {
                                        self.request(ui.ctx(), Request::screen("extend", device, &self.usb_serial), false);
                                    }
                                });
                                if self.usb_serial.trim().is_empty() { copy(ui, "Enter a USB serial above to start sharing.", ui::SECONDARY); }
                            }
                        }
                    }
                    if device.last_ping.is_some() { copy(ui, "Ping received", ui::SECONDARY); }
                });
            }
        });
    }
}

// Lexical validation only: no filesystem access, stat, canonicalize or readability probe on the UI thread.
fn file_path_error(path: &str) -> Option<&'static str> {
    if !std::path::Path::new(path).is_absolute() { Some("Enter the full absolute file path.") }
    else if path.len() > 4096 || path.chars().any(char::is_control) { Some("Use a path of at most 4096 bytes without control characters.") }
    else { None }
}

fn copy(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(ui::display(13.0)).color(color)).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_forward_compatible_state_and_optional_device_fields() {
        let reply: Reply = serde_json::from_str(r#"{"ok":true,"state":{"name":"Desktop","running":true,"future":1,"devices":[{"id":"phone","name":"Phone","connected":true}]}}"#).unwrap();
        let device = &reply.state.unwrap().devices[0];
        assert_eq!(device.id, "phone");
        assert!(!device.paired);
        assert_eq!(device.battery, None);
    }
    #[test]
    fn screen_requests_bind_explicit_usb_serial_and_use_view_only_defaults() {
        let device = Device { id: "network-peer".into(), ..Default::default() };
        let mirror = serde_json::to_value(Request::screen("mirror_phone", &device, " USB-123 ")).unwrap();
        assert_eq!(mirror["device_id"], "network-peer");
        assert_eq!(mirror["serial"], "USB-123");
        assert_eq!(mirror["control"], false);
        assert!(mirror.get("args").is_none());
        let extend = serde_json::to_value(Request::screen("extend", &device, "USB-456")).unwrap();
        assert_eq!(extend["serial"], "USB-456");
        assert_eq!(extend["width"], 960);
        assert_eq!(extend["height"], 600);
        assert_eq!(extend["fps"], 15);
        assert!(extend.get("control").is_none());
    }
    #[test]
    fn screen_state_and_capabilities_remain_optional_and_forward_compatible() {
        let state: Snapshot = serde_json::from_str(r#"{"running":true,"screens":{"mode":"phone","serial":"USB-123","running":true},"screen_capabilities":{"phone_mirror":true,"desktop_mirror":false,"message":"USB only"}}"#).unwrap();
        assert!(state.screens.unwrap().running);
        let capabilities = state.screen_capabilities.unwrap();
        assert!(capabilities.phone_mirror);
        assert!(!capabilities.extended_output);
        assert_eq!(capabilities.message, "USB only");
    }
    #[test]
    fn stop_is_queued_during_commands_and_service_stop_has_priority() {
        let ctx = egui::Context::default();
        let (_tx, rx) = mpsc::channel();
        let mut connect = Connect { pending: Some(Pending { rx, passive: false, feedback: "Started", op: "mirror_phone" }), ..Default::default() };
        connect.request(&ctx, Request { op: "stop_screen", ..Request::state() }, false);
        assert_eq!(connect.queued_controls.front().unwrap().op, "stop_screen");
        assert_eq!(connect.pending.as_ref().unwrap().op, "mirror_phone");
        connect.request(&ctx, Request { op: "stop", ..Request::state() }, false);
        connect.request(&ctx, Request { op: "stop_screen", ..Request::state() }, false);
        assert_eq!(connect.queued_controls.front().unwrap().op, "stop");
    }
    #[test]
    fn successful_service_stop_clears_stale_online_state_without_polling_hidden() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Reply { ok: true, error: None, state: None })).unwrap();
        let mut connect = Connect { pending: Some(Pending { rx, passive: false, feedback: "Stopped", op: "stop" }), snapshot: Some(Snapshot { running: true, ..Default::default() }), ..Default::default() };
        connect.tick(&ctx, false);
        assert!(connect.snapshot.is_none());
        assert!(connect.pending.is_none());
        assert_eq!(connect.notice.as_deref(), Some("Stopped"));
    }
    #[test]
    fn file_requests_bind_peer_and_transfer_ids_and_preserve_literal_absolute_path() {
        let device = Device { id: "paired-phone".into(), ..Default::default() };
        let policy = serde_json::to_value(Request::files_policy(&device, true)).unwrap();
        assert_eq!(policy["op"], "set_files"); assert_eq!(policy["device_id"], "paired-phone"); assert_eq!(policy["enabled"], true);
        let draft = FileDraft { device_id: device.id.clone(), device_name: "Phone".into(), path: "/home/test/Literal $(data); file.txt".into() };
        let send = serde_json::to_value(Request::send_file(&draft)).unwrap();
        assert_eq!(send["op"], "send_file"); assert_eq!(send["path"], draft.path);
        let transfer = Transfer { id: "opaque-transfer".into(), peer_id: device.id, ..Default::default() };
        for op in ["accept_transfer", "cancel_transfer"] {
            let request = serde_json::to_value(Request::transfer(op, &transfer)).unwrap();
            assert_eq!(request["transfer_id"], "opaque-transfer"); assert_eq!(request["device_id"], "paired-phone");
        }
    }
    #[test]
    fn file_models_are_default_off_and_readiness_requires_applied_current_connection() {
        let mut device: Device = serde_json::from_str(r#"{"id":"phone","paired":true,"connected":true}"#).unwrap();
        assert!(!device.files_enabled && !device.files_ready());
        device.files_enabled = true; assert!(device.files_ready());
        device.files_reconnect_needed = true; assert!(!device.files_ready());
        device.files_reconnect_needed = false; device.connected = false; assert!(!device.files_ready());
        let state: Snapshot = serde_json::from_str(r#"{"running":true,"transfers":[{"id":"t","peer_id":"phone","direction":"receive","filename":"report.pdf","size":1024,"transferred":256,"state":"offered","future":true}]}"#).unwrap();
        assert!(state.transfers[0].incoming_offer()); assert!(state.transfers[0].cancellable()); assert_eq!(state.transfers[0].progress(), 0.25);
        assert_eq!(Transfer { size: 1, transferred: 2, ..Default::default() }.progress(),1.0);
        assert_eq!(Transfer::default().progress(),0.0);
    }
    #[test]
    fn file_path_validation_is_lexical_and_does_not_need_existing_files() {
        assert!(file_path_error("/a/nonexistent/location/file with spaces.txt").is_none());
        assert!(file_path_error("relative/file.txt").is_some());
        assert!(file_path_error("").is_some()); assert!(file_path_error("/bad\0path").is_some());
        assert!(file_path_error(&format!("/{}", "x".repeat(4096))).is_some());
    }
    #[test]
    fn busy_cancellation_queue_preserves_distinct_transfer_ids_and_stop_priority() {
        let ctx = egui::Context::default(); let (_tx, rx) = mpsc::channel();
        let mut connect = Connect { pending: Some(Pending { rx, passive:false, feedback:"Working", op:"send_file" }), ..Default::default() };
        for id in ["first", "second", "first"] {
            connect.request(&ctx, Request::transfer("cancel_transfer", &Transfer { id:id.into(), peer_id:"offline-phone".into(), ..Default::default() }), false);
        }
        assert_eq!(connect.queued_controls.len(),2);
        assert_eq!(connect.queued_controls[0].args["transfer_id"],"first");
        assert_eq!(connect.queued_controls[1].args["transfer_id"],"second");
        connect.request(&ctx, Request { op:"stop", ..Request::state() }, false);
        assert_eq!(connect.queued_controls.len(),1); assert_eq!(connect.queued_controls[0].op,"stop");
    }
    #[test]
    fn transfer_terminal_states_cannot_be_accepted_or_cancelled() {
        for state in ["complete", "failed", "cancelled", "expired", "future"] {
            let transfer = Transfer { id:"id".into(), direction:"receive".into(), state:state.into(), ..Default::default() };
            assert!(!transfer.cancellable() && !transfer.incoming_offer());
        }
    }
    #[test]
    fn invalid_verification_code_cannot_enable_acceptance() {
        let mut device = Device::default();
        for code in ["", "ABC", "ABCDEFGH", "<script>"] {
            device.verification_key = Some(code.into());
            assert!(device.verification_code().is_none());
        }
        device.verification_key = Some("ab12CD34".into());
        assert_eq!(device.verification_code(), Some("ab12CD34"));
    }
    #[test]
    fn approval_carries_current_attempt_but_other_commands_do_not() {
        let device = Device { id: "phone".into(), attempt: Some(77), ..Default::default() };
        for op in ["accept", "reject"] {
            let json = serde_json::to_value(Request::device(op, &device)).unwrap();
            assert_eq!(json["attempt"], 77);
        }
        let json = serde_json::to_value(Request::device("pair", &device)).unwrap();
        assert!(json.get("attempt").is_none());
        assert_eq!(json["device_id"], "phone");
    }
    #[test]
    fn hidden_panel_does_not_schedule_requests() {
        let ctx = egui::Context::default();
        let mut connect = Connect::default();
        connect.tick(&ctx, false);
        assert!(connect.pending.is_none());
        assert!(connect.last_poll.is_none());
    }
    #[test]
    fn drains_failed_command_while_hidden_without_retrying() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        tx.send(Err("Service stopped".into())).unwrap();
        let mut connect = Connect { pending: Some(Pending { rx, passive: false, feedback: "Done", op: "ping" }), ..Default::default() };
        connect.tick(&ctx, false);
        assert!(connect.pending.is_none());
        assert_eq!(connect.error.as_deref(), Some("Service stopped"));
    }
    #[test]
    fn operation_error_uses_latest_attempt_and_clears_pending_feedback() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Reply { ok: false, error: Some("Pairing attempt changed".into()), state: Some(Snapshot { running: true, devices: vec![Device { attempt: Some(88), ..Default::default() }], ..Default::default() }) })).unwrap();
        let mut connect = Connect { pending: Some(Pending { rx, passive: false, feedback: "Accepted", op: "accept" }), notice: Some("Working…".into()), ..Default::default() };
        connect.tick(&ctx, false);
        assert_eq!(connect.snapshot.unwrap().devices[0].attempt, Some(88));
        assert_eq!(connect.error.as_deref(), Some("Pairing attempt changed"));
        assert!(connect.notice.is_none());
    }
    #[test]
    fn visible_panel_respects_two_second_poll_interval() {
        let ctx = egui::Context::default();
        let mut connect = Connect { last_poll: Some(Instant::now()), ..Default::default() };
        connect.tick(&ctx, true);
        assert!(connect.pending.is_none());
    }
    #[test]
    fn full_private_socket_backlog_cannot_block_the_worker_before_timeouts() {
        use rustix::net::{socket_with, bind, listen, AddressFamily, SocketType, SocketFlags, SocketAddrUnix};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("full.sock");
        let listener = socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK, None).unwrap();
        bind(&listener, &SocketAddrUnix::new(&path).unwrap()).unwrap();
        listen(&listener, 0).unwrap();
        let queued = connect_local(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || { let _ = tx.send(exchange(&path, &Request::state())); });
        let response = rx.recv_timeout(Duration::from_millis(500));
        // Release the disposable backlog even on a regression, so the worker can exit.
        drop(queued);
        drop(listener);
        worker.join().unwrap();
        let error = response.expect("connect blocked behind a full AF_UNIX backlog").unwrap_err();
        assert!(error.contains("service unavailable"));
    }
    #[test]
    fn delayed_screen_preparation_replies_succeed_but_state_remains_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut servers = Vec::new();
        let mut clients = Vec::new();
        for op in ["state", "mirror_phone", "extend"] {
            let path = dir.path().join(format!("{op}.sock"));
            let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
            servers.push(std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
                assert_eq!(serde_json::from_str::<serde_json::Value>(&line).unwrap()["op"], op);
                // Exercise actual socket waiting across the ordinary two-second deadline.
                std::thread::sleep(IO_TIMEOUT + Duration::from_millis(300));
                let _ = stream.write_all(b"{\"ok\":true}\n");
            }));
            clients.push((op, std::thread::spawn(move || exchange(&path, &Request { op, ..Request::state() }))));
        }
        for (op, client) in clients {
            let result = client.join().unwrap();
            if op == "state" {
                assert!(result.unwrap_err().contains("Service did not respond"));
            } else {
                assert!(result.unwrap().ok, "{op} must allow bounded USB preparation");
            }
        }
        for server in servers { server.join().unwrap(); }
    }
    #[test]
    fn ipc_uses_one_newline_delimited_command_and_handles_failure_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&line).unwrap()["op"], "state");
            stream.write_all(b"{\"ok\":false,\"error\":\"refresh required\"}\n").unwrap();
        });
        let reply = exchange(&path, &Request::state()).unwrap();
        assert!(!reply.ok);
        assert_eq!(reply.error.as_deref(), Some("refresh required"));
        server.join().unwrap();
    }
    #[test]
    #[ignore = "isolated native fixture; needs DESUI_CONNECT_PREVIEW and Xvfb"]
    fn native_connect_preview() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        struct Preview { connect: Connect, frames: usize, output: PathBuf, live: bool, started: Instant }
        impl eframe::App for Preview {
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                for event in ctx.input(|i| i.events.clone()) {
                    if let egui::Event::Screenshot { image, .. } = event {
                        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        image::save_buffer(&self.output, &bytes, image.size[0] as u32, image.size[1] as u32, image::ColorType::Rgba8).unwrap();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    if self.live {
                        // Real IPC state only: automated live inspection cannot pair, launch or stop anything.
                        ui.add_enabled_ui(false, |ui| self.connect.panel(ui, &Theme::load("signal")));
                    } else { self.connect.panel(ui, &Theme::load("signal")); }
                });
                let ready = self.connect.snapshot.is_some() || self.connect.error.is_some();
                if self.live && ready { assert!(self.connect.snapshot.as_ref().is_some_and(|state| state.running), "Live preview requires a responding DES Connect daemon"); }
                assert!(!self.live || ready || self.started.elapsed() < Duration::from_secs(10), "Live preview IPC did not complete");
                if !self.live || ready { self.frames += 1; }
                if self.frames == 4 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
                ctx.request_repaint();
            }
        }
        let mode = std::env::var("DESUI_CONNECT_PREVIEW_MODE").unwrap_or_default();
        let output = PathBuf::from(std::env::var("DESUI_CONNECT_PREVIEW").expect("preview output"));
        let width = std::env::var("DESUI_CONNECT_PREVIEW_WIDTH").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(280.0);
        let mut connect = Connect { last_poll: Some(Instant::now() + Duration::from_secs(3600)), ..Default::default() };
        if mode == "error" { connect.error = Some("DES Connect service unavailable. Start the service or retry.".into()); }
        else {
            connect.snapshot = Some(Snapshot { name: "Workstation".into(), running: true, devices: if mode == "empty" { Vec::new() } else { vec![
                Device { id: "tablet".into(), name: "Samsung tablet".into(), kind: "tablet".into(), connected: true, pair_state: "incoming".into(), attempt: Some(17), verification_key: Some("ABCDEF12".into()), ..Default::default() },
                Device { id: "phone".into(), name: "Personal phone".into(), kind: "phone".into(), connected: true, paired: true, pair_state: "paired".into(), battery: Some(82), charging: true, ..Default::default() },
                Device { id: "offline".into(), name: "Offline phone".into(), paired: true, pair_state: "offline".into(), ..Default::default() },
            ] }, ..Default::default() });
        }
        if mode.starts_with("screen") {
            connect.usb_serial = "USB-123456".into();
            let snapshot = connect.snapshot.as_mut().unwrap();
            snapshot.devices.retain(|device| device.paired && device.connected);
            snapshot.screen_capabilities = Some(ScreenCapabilities { phone_mirror: true, extended_output: mode != "screen-unavailable", message: if mode == "screen-unavailable" { "Extended USB display unavailable: this desktop has no virtual-output support.".into() } else { "USB screen sharing available.".into() } });
            if mode == "screen-active" {
                snapshot.screens = Some(Screens { mode: "phone".into(), serial: Some("USB-123456".into()), running: true, control: false, message: "Disconnecting or locking stops sharing.".into() });
            }
        }
        if mode.starts_with("files") {
            let snapshot = connect.snapshot.as_mut().unwrap();
            snapshot.devices.retain(|device| device.paired && device.connected);
            let device = &mut snapshot.devices[0];
            device.files_enabled = true;
            device.files_reconnect_needed = mode == "files-policy";
            device.connected = mode != "files-offline";
            connect.file_paths.insert(device.id.clone(), "/home/test/Documents/Project report.pdf".into());
            if mode == "files-confirm" {
                connect.confirm_send = Some(FileDraft { device_id:device.id.clone(), device_name:device.name.clone(), path:"/home/test/Documents/Project report.pdf".into() });
            }
            if matches!(mode.as_str(), "files-transfers" | "files-offline") {
                snapshot.transfers = vec![
                    Transfer { id:"incoming".into(), peer_id:device.id.clone(), direction:"receive".into(), filename:"Report from phone.pdf".into(), size:1024*1024, state:"offered".into(), ..Default::default() },
                    Transfer { id:"sending".into(), peer_id:device.id.clone(), direction:"send".into(), filename:"Project archive.zip".into(), size:10*1024*1024, transferred:4*1024*1024, state:"transferring".into(), ..Default::default() },
                    Transfer { id:"failed".into(), peer_id:device.id.clone(), direction:"send".into(), filename:"Earlier transfer.txt".into(), size:1024, state:"failed".into(), error:Some("The phone disconnected. Select the file again to retry.".into()), ..Default::default() },
                ];
            }
        }
        if mode == "live" {
            connect.snapshot = None;
            connect.last_poll = None;
        }
        let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([width, 720.0]), event_loop_builder: Some(Box::new(|builder| { builder.with_x11().with_any_thread(true); })), ..Default::default() };
        eframe::run_native("DES Connect fixture", options, Box::new(move |cc| {
            ui::install_fonts(&cc.egui_ctx, None, None);
            crate::apply_style(&cc.egui_ctx, &Theme::load("signal"));
            Ok(Box::new(Preview { connect, frames: 0, output, live: mode == "live", started: Instant::now() }))
        })).unwrap();
    }
    #[test]
    fn file_controls_and_transfer_rows_fit_a_narrow_panel() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx,None,None);
        let theme = Theme::load("signal"); crate::apply_style(&ctx,&theme);
        let device = Device { id:"phone".into(), name:"A trusted phone with a long name".into(), paired:true, connected:true, files_enabled:true, ..Default::default() };
        let mut connect = Connect { last_poll:Some(Instant::now()), snapshot:Some(Snapshot { running:true, devices:vec![device.clone()], transfers:vec![Transfer { id:"transfer".into(), peer_id:device.id.clone(), direction:"receive".into(), filename:"A long incoming filename that must wrap.pdf".into(), size:4096, state:"offered".into(), ..Default::default() }], ..Default::default() }), confirm_send:Some(FileDraft { device_id:device.id.clone(), device_name:device.name.clone(), path:"/home/test/long folder/a long file name.pdf".into() }), ..Default::default() };
        for _ in 0..3 { let _=ctx.run(egui::RawInput { screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(220.0,400.0))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                connect.panel(ui,&theme);
                assert!(ui.min_rect().right()<=220.0); assert!(ui.min_rect().bottom()<=400.0);
            });
        }); }
    }
    #[test]
    fn device_panel_stays_within_narrow_bounds() {
        let ctx = egui::Context::default();
        ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal");
        crate::apply_style(&ctx, &theme);
        let mut connect = Connect { last_poll: Some(Instant::now()), snapshot: Some(Snapshot { name: "Desktop".into(), running: true, devices: vec![Device { id: "phone".into(), name: "A long phone name that must wrap inside the panel".into(), connected: true, pair_state: "incoming".into(), attempt: Some(12), verification_key: Some("ABCDEF12".into()), ..Default::default() }], screen_capabilities: Some(ScreenCapabilities { phone_mirror: true, extended_output: true, ..Default::default() }), ..Default::default() }), ..Default::default() };
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(260.0, 400.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    connect.panel(ui, &theme);
                    assert!(ui.min_rect().right() <= 260.0);
                    assert!(ui.min_rect().bottom() <= 400.0);
                });
            });
        }
    }
}
