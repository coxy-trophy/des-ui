//! Read-only connected-device inventory. One bounded worker owns all filesystem
//! and service queries; painting never probes hardware or opens capture streams.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;
use des_common::hands::CameraSelection;
use eframe::egui::{self, Ui};
use crate::{settings::note, theme::Theme, ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind { Camera, Audio, Input, Storage, Display, Bluetooth, Usb }
impl Kind {
    fn title(self) -> &'static str { match self {
        Self::Camera => "CAMERAS", Self::Audio => "AUDIO", Self::Input => "KEYBOARDS & POINTERS",
        Self::Storage => "STORAGE", Self::Display => "DISPLAYS", Self::Bluetooth => "BLUETOOTH", Self::Usb => "USB DEVICES",
    }}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub status: String,
    pub detail: String,
    pub camera: Option<CameraSelection>,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
    pub selected: Option<CameraSelection>,
    pub ready: bool,
}
enum Request { Refresh, Camera(CameraSelection) }
struct Update { snapshot: Snapshot, notices: Vec<String> }
#[derive(Clone, Copy)]
pub enum Destination { Sound, Bluetooth, Displays, Pointer, Keyboard }

pub struct Devices {
    requests: SyncSender<Request>,
    updates: Receiver<Update>,
    pub snapshot: Snapshot,
    pending_camera: Option<CameraSelection>,
}
impl Devices {
    pub fn new(ctx: egui::Context) -> Self {
        let (requests, receive) = mpsc::sync_channel(1);
        let (send, updates) = mpsc::channel();
        std::thread::Builder::new().name("device-inventory".into()).spawn(move || {
            let mut previous: Option<Snapshot> = None;
            let mut message = None;
            loop {
                let mut save_error = None;
                if let Some(Request::Camera(selection)) = message.take() {
                    if let Err(error) = selection.save() {
                        save_error = Some(format!("Camera choice was not saved: {error}"));
                    }
                }
                let mut snapshot = inventory(previous.as_ref());
                if let Some(error) = save_error { snapshot.warnings.push(error); }
                let notices = previous.as_ref().map(|old| changes(old, &snapshot)).unwrap_or_default();
                let changed = previous.as_ref().is_none_or(|old| old.devices != snapshot.devices || old.warnings != snapshot.warnings || old.selected != snapshot.selected);
                if changed {
                    if send.send(Update { snapshot: snapshot.clone(), notices }).is_err() { break; }
                    ctx.request_repaint();
                }
                previous = Some(snapshot);
                match receive.recv_timeout(Duration::from_secs(3)) {
                    Ok(request) => message = Some(request),
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        }).expect("device inventory worker");
        Self { requests, updates, snapshot: Snapshot::default(), pending_camera: None }
    }
    /// Call every shell update, including while Settings is closed. First scan is
    /// deliberately quiet. Return strings through the existing notification owner.
    pub fn poll(&mut self) -> Vec<String> {
        let mut notices = Vec::new();
        for update in self.updates.try_iter() {
            self.snapshot = update.snapshot;
            if self.pending_camera.as_ref().is_some_and(|pending| self.snapshot.selected.as_ref() == Some(pending))
                || self.snapshot.warnings.iter().any(|warning| warning.starts_with("Camera choice was not saved")) {
                self.pending_camera = None;
            }
            notices.extend(update.notices);
        }
        notices
    }
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme) -> Option<Destination> {
        let mut destination = None;
        if ui::button(ui, theme, "REFRESH", None, false).clicked() { let _ = self.requests.try_send(Request::Refresh); }
        if !self.snapshot.ready { note(ui, "Looking for devices…"); return None; }
        for warning in &self.snapshot.warnings { note(ui, warning); }
        note(ui, "Device detection never turns on a camera or microphone. Choose a camera below, then enable Hand control in Mouse & touchpad. Restart hand control after changing cameras.");
        if let Some(selected) = &self.snapshot.selected {
            if !self.snapshot.devices.iter().any(|d| d.camera.as_ref().is_some_and(|c| c.identity == selected.identity)) {
                note(ui, "Your selected hand camera is disconnected or unavailable. Hand control will not use a different camera.");
            }
        }
        if self.snapshot.devices.is_empty() { note(ui, "No supported devices were reported. Check the cable, power and system device permissions, then Refresh."); }
        for kind in [Kind::Camera, Kind::Audio, Kind::Input, Kind::Storage, Kind::Display, Kind::Bluetooth, Kind::Usb] {
            let devices: Vec<_> = self.snapshot.devices.iter().filter(|d| d.kind == kind).collect();
            if devices.is_empty() { continue; }
            ui.add_space(14.0);
            ui::section(ui, kind.title(), |_| {});
            for device in devices {
                ui.add(egui::Label::new(egui::RichText::new(&device.name).font(ui::display(14.0)).color(ui::TEXT)).wrap());
                note(ui, &format!("{} · {}", device.status, device.detail));
                if let Some(camera) = &device.camera {
                    let selected = self.snapshot.selected.as_ref().is_some_and(|s| s.identity == camera.identity);
                    if selected { note(ui, "Selected for hand control"); }
                    else if self.pending_camera.as_ref() == Some(camera) { note(ui, "Saving camera choice…"); }
                    else if ui.add_enabled_ui(self.pending_camera.is_none(), |ui| ui::button(ui, theme, "USE FOR HAND CONTROL", None, false)).inner.clicked() {
                        if self.requests.try_send(Request::Camera(camera.clone())).is_ok() { self.pending_camera = Some(camera.clone()); }
                    }
                }
                ui.add_space(8.0);
            }
            let action = match kind {
                Kind::Audio => Some(("OPEN SOUND SETTINGS", Destination::Sound)),
                Kind::Bluetooth => Some(("OPEN BLUETOOTH SETTINGS", Destination::Bluetooth)),
                Kind::Display => Some(("OPEN DISPLAY SETTINGS", Destination::Displays)),
                Kind::Input => Some(("OPEN MOUSE & TOUCHPAD", Destination::Pointer)),
                Kind::Camera => Some(("OPEN HAND CONTROL", Destination::Pointer)),
                _ => None,
            };
            if let Some((label, target)) = action { if ui::button(ui, theme, label, None, false).clicked() { destination = Some(target); } }
            if kind == Kind::Input && ui::button(ui, theme, "OPEN KEYBOARD SETTINGS", None, false).clicked() { destination = Some(Destination::Keyboard); }
            if kind == Kind::Storage { note(ui, "Mounted locations are reported for this session. Mounting and safe removal remain in your file manager."); }
        }
        destination
    }
}
fn read(path: impl AsRef<Path>) -> String { std::fs::read_to_string(path).unwrap_or_default().trim().to_string() }
fn entries(path: impl AsRef<Path>) -> Vec<PathBuf> { std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| e.path()).collect() }
fn properties(path: &Path, block: bool) -> BTreeMap<String, String> {
    let dev = read(path.join("dev"));
    parse_properties(&read(format!("/run/udev/data/{}{dev}", if block { "b" } else { "c" })))
}
fn parse_properties(text: &str) -> BTreeMap<String, String> {
    text.lines().filter_map(|l| l.strip_prefix("E:")?.split_once('=').map(|(k,v)| (k.to_string(),v.to_string()))).collect()
}
fn capture(properties: &BTreeMap<String, String>) -> bool {
    properties.get("ID_V4L_CAPABILITIES").is_some_and(|v| v.split(':').any(|c| c == "capture"))
}
fn output(program: &str, args: &[&str]) -> Option<String> {
    let result = Command::new("timeout").args(["2", program]).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    result.status.success().then(|| String::from_utf8_lossy(&result.stdout).into_owned())
}
fn inventory(previous: Option<&Snapshot>) -> Snapshot {
    let mut result = Snapshot { ready: true, ..Snapshot::default() };
    match CameraSelection::load() { Ok(selection) => result.selected = selection, Err(_) => result.warnings.push("Saved camera choice could not be read. Choose your hand camera again.".into()) }
    for path in entries("/sys/class/video4linux") {
        let node = path.file_name().unwrap().to_string_lossy().into_owned();
        let props = properties(&path, false);
        // udev's V4L helper has queried each node's capabilities already. The
        // metadata sibling is not a second camera and cannot be selected.
        if !capture(&props) { continue; }
        let identity = des_common::hands::camera_identity(&path);
        let camera = identity.as_ref().map(|identity| CameraSelection { path: PathBuf::from("/dev").join(&node), identity: identity.clone() });
        result.devices.push(Device { id: identity.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| format!("camera:{node}")), name: read(path.join("name")), kind: Kind::Camera,
            status: "Detected".into(), detail: format!("/dev/{node} · video capture; permission and availability checked when hand control starts"), camera });
    }
    if !entries("/sys/class/video4linux").is_empty() && result.devices.is_empty() {
        result.warnings.push("Video devices are present, but udev has not identified a usable capture node. Metadata-only nodes are excluded; check the camera driver and udev permissions.".into());
    }
    for path in entries("/sys/class/input") {
        let node = path.file_name().unwrap().to_string_lossy().into_owned();
        if !node.starts_with("event") { continue; }
        let props = properties(&path, false);
        let kind = if props.get("ID_INPUT_TOUCHPAD").is_some_and(|v|v=="1") { "Touchpad" }
            else if props.get("ID_INPUT_MOUSE").is_some_and(|v|v=="1") { "Mouse" }
            else if props.get("ID_INPUT_KEYBOARD").is_some_and(|v|v=="1") { "Keyboard" }
            else if props.get("ID_INPUT_TOUCHSCREEN").is_some_and(|v|v=="1") { "Touchscreen" }
            else { "Input device" };
        let identity = std::fs::canonicalize(path.join("device")).unwrap_or(path.clone()).to_string_lossy().into_owned();
        if result.devices.iter().any(|d|d.id == identity) { continue; }
        result.devices.push(Device { id: identity, name: read(path.join("device/name")), kind: Kind::Input, status: "Detected".into(), detail: format!("{kind} · /dev/input/{node}"), camera: None });
    }
    for path in entries("/sys/bus/usb/devices") {
        let product = read(path.join("product"));
        if product.is_empty() || read(path.join("bDeviceClass")) == "09" { continue; }
        result.devices.push(Device { id: format!("usb:{}", path.display()), name: product, kind: Kind::Usb, status: "Connected".into(), detail: read(path.join("manufacturer")), camera: None });
    }
    let mounts = read("/proc/self/mountinfo");
    for path in entries("/sys/class/block") {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.join("partition").exists() || name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") || !path.join("device").exists() { continue; }
        let props = properties(&path, true);
        let model = props.get("ID_MODEL_FROM_DATABASE").cloned().or_else(|| props.get("ID_MODEL").map(|s|s.replace('_'," "))).unwrap_or_else(|| read(path.join("device/model")));
        let mut locations = Vec::new();
        for child in std::iter::once(path.clone()).chain(entries(&path).into_iter().filter(|p|p.join("partition").exists())) {
            locations.extend(mount_locations(&mounts, &read(child.join("dev"))));
        }
        locations.sort(); locations.dedup();
        let detail = if locations.is_empty() { format!("/dev/{name} · no mounted location in this session") } else { format!("/dev/{name} · {}", locations.join(", ")) };
        result.devices.push(Device { id: format!("storage:{}",path.display()), name: if model.is_empty(){name}else{model}, kind: Kind::Storage, status: if locations.is_empty(){"Detected"}else{"Mounted"}.into(), detail, camera: None });
    }
    for path in entries("/sys/class/drm") {
        if !path.join("status").exists() || read(path.join("status")) != "connected" { continue; }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        result.devices.push(Device { id: format!("display:{name}"), name, kind: Kind::Display, status: "Connected".into(), detail: if read(path.join("enabled"))=="enabled"{"Display output enabled"}else{"Display output disabled"}.into(), camera: None });
    }
    match output("pw-dump", &["-N"]).and_then(|text| parse_audio(&text)) {
        Some(devices) => result.devices.extend(devices),
        None => retain_service(&mut result, previous, Kind::Audio, "Audio service unavailable; last reported endpoints may be stale."),
    }
    match output("busctl", &["--system", "--json=short", "call", "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects"]).and_then(|text| parse_bluetooth(&text)) {
        Some(devices) => result.devices.extend(devices),
        None => retain_service(&mut result, previous, Kind::Bluetooth, "Bluetooth service unavailable; last reported devices may be stale."),
    }
    result.devices.sort_by(|a,b| (a.kind,&a.name,&a.id).cmp(&(b.kind,&b.name,&b.id)));
    result
}
fn retain_service(result: &mut Snapshot, previous: Option<&Snapshot>, kind: Kind, warning: &str) {
    if let Some(previous) = previous { result.devices.extend(previous.devices.iter().filter(|d|d.kind==kind).cloned()); }
    result.warnings.push(warning.into());
}
fn mount_locations(text: &str, device: &str) -> Vec<String> {
    text.lines().filter_map(|line| { let words: Vec<_> = line.split_whitespace().collect();
        (words.get(2)==Some(&device)).then(|| words.get(4).map(|s|s.replace("\\040"," ").replace("\\011","\t").replace("\\134","\\"))).flatten()
    }).collect()
}
fn parse_audio(text: &str) -> Option<Vec<Device>> {
    let objects: Vec<serde_json::Value> = serde_json::from_str(text).ok()?;
    Some(objects.iter().filter_map(|object| {
        let props = &object["info"]["props"];
        let class = props["media.class"].as_str()?;
        if class != "Audio/Sink" && class != "Audio/Source" { return None; }
        let name = props["node.description"].as_str().or_else(||props["node.nick"].as_str()).or_else(||props["node.name"].as_str())?;
        let id = props["node.name"].as_str()?;
        Some(Device { id: format!("audio:{id}"), name:name.into(), kind:Kind::Audio, status:"Available".into(), detail: if class=="Audio/Sink"{"Playback endpoint"}else{"Microphone / capture endpoint; inventory does not record"}.into(), camera:None })
    }).collect())
}
fn parse_bluetooth(text: &str) -> Option<Vec<Device>> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let objects = root["data"][0].as_object()?;
    Some(objects.iter().filter_map(|(path,interfaces)| {
        let props = interfaces.get("org.bluez.Device1")?;
        let connected = props["Connected"]["data"].as_bool().unwrap_or(false);
        let paired = props["Paired"]["data"].as_bool().unwrap_or(false);
        if !connected && !paired { return None; }
        let name = props["Alias"]["data"].as_str().or_else(||props["Name"]["data"].as_str()).unwrap_or("Bluetooth device");
        Some(Device { id:format!("bluetooth:{path}"), name:name.into(), kind:Kind::Bluetooth, status:if connected{"Connected"}else{"Paired · disconnected"}.into(), detail:"Managed by Bluetooth settings".into(), camera:None })
    }).collect())
}
fn changes(old: &Snapshot, new: &Snapshot) -> Vec<String> {
    let before: BTreeMap<_, _> = old.devices.iter().filter(|d| d.kind != Kind::Audio).map(|d| (&d.id,d)).collect();
    let after: BTreeMap<_, _> = new.devices.iter().filter(|d| d.kind != Kind::Audio).map(|d| (&d.id,d)).collect();
    let mut added = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut changed = BTreeSet::new();
    for (id,device) in &after {
        match before.get(id) {
            None => { added.insert(device.name.as_str()); },
            Some(old) if old.status != device.status => { changed.insert(device.name.as_str()); },
            _ => {},
        }
    }
    for (id,device) in &before { if !after.contains_key(id) { removed.insert(device.name.as_str()); } }
    [("Device available",added),("Device removed",removed),("Device status changed",changed)].into_iter().filter_map(|(label,names)| {
        (!names.is_empty()).then(||format!("{label}: {}{}", names.iter().take(3).copied().collect::<Vec<_>>().join(", "), if names.len()>3{"…"}else{""}))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_siblings_never_become_selectable_cameras() {
        assert!(capture(&parse_properties("E:ID_V4L_CAPABILITIES=:capture:\n")));
        for value in [":metadata:", ":output:", ":meta_capture:", ""] {
            assert!(!capture(&parse_properties(&format!("E:ID_V4L_CAPABILITIES={value}\n"))));
        }
    }
    #[test]
    fn audio_is_available_without_claiming_it_is_connected_or_capturing() {
        let devices = parse_audio(r#"[{"info":{"props":{"media.class":"Audio/Source","node.name":"usb-mic","node.description":"USB microphone"}}},{"info":{"props":{"media.class":"Stream/Input/Audio","node.name":"app"}}}]"#).unwrap();
        assert_eq!(devices.len(),1); assert_eq!(devices[0].status,"Available");
    }
    #[test]
    fn bluetooth_pairing_does_not_claim_a_live_connection() {
        let devices = parse_bluetooth(r#"{"data":[{"/org/bluez/hci0/dev_a":{"org.bluez.Device1":{"Alias":{"data":"Headphones"},"Paired":{"data":true},"Connected":{"data":false}}}}]}"#).unwrap();
        assert_eq!(devices[0].status,"Paired · disconnected");
    }
    #[test]
    fn mount_paths_use_device_numbers_and_decode_spaces() {
        assert_eq!(mount_locations("20 1 8:1 / /media/USB\\040Drive rw - ext4 /dev/sda1 rw", "8:1"), ["/media/USB Drive"]);
        assert!(mount_locations("20 1 8:1 / /media/x rw", "8:2").is_empty());
    }
    #[test]
    fn identical_snapshots_and_service_failures_are_quiet() {
        let device = Device { id:"bluetooth:a".into(), name:"Headphones".into(), kind:Kind::Bluetooth, status:"Connected".into(), detail:String::new(), camera:None };
        let old = Snapshot { devices:vec![device], ..Snapshot::default() };
        assert!(changes(&old,&old).is_empty());
        let mut stale = Snapshot::default(); retain_service(&mut stale,Some(&old),Kind::Bluetooth,"Unavailable");
        assert!(changes(&old,&stale).is_empty());
        assert_eq!(changes(&old,&Snapshot::default()).len(),1);
    }
}
