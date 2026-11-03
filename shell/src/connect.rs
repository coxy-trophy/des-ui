//! Native DES Connect controls. The render thread never opens a socket or launches a process.
use crate::{theme::Theme, ui};
use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::{BufRead, BufReader, Read, Write}, os::unix::net::UnixStream, path::PathBuf, process::{Command, Stdio}, sync::mpsc::{self, Receiver}, time::{Duration, Instant}};

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
}

impl Device {
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
}

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

#[derive(Clone, Debug, Serialize)]
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
    fn is_stop(&self) -> bool { matches!(self.op, "stop" | "stop_screen") }
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
    queued_stop: Option<Request>,
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
                        if let Some(state) = reply.state { self.snapshot = Some(state); }
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
            if let Some(stop) = self.queued_stop.take() { self.request(ctx, stop, false); }
        }
        if visible && self.pending.is_none() && self.last_poll.is_none_or(|at| at.elapsed() >= POLL) {
            self.request(ctx, Request::state(), false);
        }
        if self.pending.is_some() { ctx.request_repaint_after(Duration::from_millis(100)); }
        else if visible { ctx.request_repaint_after(POLL.saturating_sub(self.last_poll.map_or(POLL, |at| at.elapsed()))); }
    }

    fn request(&mut self, ctx: &egui::Context, request: Request, start: bool) {
        if self.pending.is_some() {
            if request.is_stop() {
                if self.pending.as_ref().is_some_and(|pending| pending.op == "stop" || pending.op == request.op) { return; }
                // Preserve command ordering: a slow start cannot overtake its queued stop.
                if self.queued_stop.as_ref().is_none_or(|queued| queued.op != "stop") { self.queued_stop = Some(request); }
                self.notice = Some("Stop requested. Waiting for the current command to finish…".into());
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
        assert_eq!(connect.queued_stop.as_ref().unwrap().op, "stop_screen");
        assert_eq!(connect.pending.as_ref().unwrap().op, "mirror_phone");
        connect.request(&ctx, Request { op: "stop", ..Request::state() }, false);
        connect.request(&ctx, Request { op: "stop_screen", ..Request::state() }, false);
        assert_eq!(connect.queued_stop.as_ref().unwrap().op, "stop");
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
