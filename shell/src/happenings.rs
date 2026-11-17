//! What the system is doing that the status island shows for a moment or while it
//! lasts (island.rs, `Mode::Blip` and `Mode::Task`, and the camera and microphone
//! dots).
//!
//! The frame loop hands in what it already knows (the battery, Do Not Disturb);
//! a background thread looks every two seconds for what nothing else watches:
//! Bluetooth devices and their batteries (BlueZ over the system bus), whether an
//! application is using the camera or the microphone (PipeWire's streams and open
//! `/dev/video*` devices), and download progress, which browsers and file managers
//! announce with the `com.canonical.Unity.LauncherEntry` signal on the session bus.
//!
//! Everything that decides something is a plain function, tested without the buses.

use crate::island::{Blip, BlipIcon, Task};
use crate::sysmon::Battery;
use eframe::egui;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::zvariant::{OwnedValue, Value};

/// How long each event shows in the island.
pub const BLIP_FOR: Duration = Duration::from_millis(3200);
/// How often the background thread looks.
const LOOK_EVERY: Duration = Duration::from_secs(2);
/// Download progress not heard of for this long is over (the sender went away).
const TASK_GONE: Duration = Duration::from_secs(20);

/// A Bluetooth device as BlueZ describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct BtDevice {
    pub path: String,
    pub name: String,
    pub connected: bool,
    pub battery: Option<u8>,
}

/// A property value, as far as this module cares.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Bool(bool),
    Text(String),
    Byte(u8),
    Number(f64),
}

fn prop(value: &Value) -> Option<Prop> {
    Some(match value {
        Value::Bool(b) => Prop::Bool(*b),
        Value::Str(s) => Prop::Text(s.to_string()),
        Value::U8(n) => Prop::Byte(*n),
        Value::F64(n) => Prop::Number(*n),
        Value::I64(n) => Prop::Number(*n as f64),
        Value::Value(inner) => return prop(inner),
        _ => return None,
    })
}

type Interfaces = BTreeMap<String, BTreeMap<String, Prop>>;

/// The Bluetooth devices among BlueZ's objects, with their battery when they
/// report one (`org.bluez.Battery1`).
pub fn bluetooth(objects: &BTreeMap<String, Interfaces>) -> Vec<BtDevice> {
    objects.iter().filter_map(|(path, interfaces)| {
        let device = interfaces.get("org.bluez.Device1")?;
        let text = |key: &str| match device.get(key) { Some(Prop::Text(t)) => Some(t.clone()), _ => None };
        let name = text("Alias").or_else(|| text("Name")).unwrap_or_else(|| "Bluetooth device".into());
        let connected = matches!(device.get("Connected"), Some(Prop::Bool(true)));
        let battery = match interfaces.get("org.bluez.Battery1").and_then(|battery| battery.get("Percentage")) {
            Some(Prop::Byte(p)) => Some((*p).min(100)),
            _ => None,
        };
        Some(BtDevice { path: path.clone(), name, connected, battery })
    }).collect()
}

/// Devices that have just connected, as island events.
pub fn bluetooth_blips(before: &[BtDevice], now: &[BtDevice]) -> Vec<Blip> {
    now.iter().filter(|device| device.connected)
        .filter(|device| !before.iter().any(|old| old.path == device.path && old.connected))
        .map(|device| Blip {
            icon: BlipIcon::Bluetooth,
            title: device.name.clone(),
            detail: device.battery.map_or_else(|| "Connected".into(), |p| format!("{p}%")),
            level: device.battery,
            warn: device.battery.is_some_and(|p| p <= 15),
        })
        .collect()
}

/// Whether, by PipeWire's graph (`pw-dump`), an application is taking sound from
/// a microphone, and whether one is taking video (a camera through PipeWire).
pub fn capturing(pw_dump: &str) -> (bool, bool) {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_str::<serde_json::Value>(pw_dump) else { return (false, false) };
    let mut microphone = false;
    let mut camera = false;
    for object in &objects {
        let info = &object["info"];
        if info["state"].as_str() != Some("running") {
            continue;
        }
        match info["props"]["media.class"].as_str() {
            Some("Stream/Input/Audio") => microphone = true,
            Some("Stream/Input/Video") => camera = true,
            _ => {}
        }
    }
    (microphone, camera)
}

/// Whether any process this user can see has a camera device open (applications
/// such as browsers open `/dev/videoN` themselves, not through PipeWire).
fn camera_open() -> bool {
    // No camera, nothing to look for (the scan reads every open file's link).
    let any_camera = std::fs::read_dir("/dev").is_ok_and(|dev| dev.flatten().any(|entry| entry.file_name().to_string_lossy().starts_with("video")));
    if !any_camera {
        return false;
    }
    let Ok(processes) = std::fs::read_dir("/proc") else { return false };
    let me = std::process::id().to_string();
    for process in processes.flatten() {
        let name = process.file_name();
        let Some(pid) = name.to_str().filter(|pid| pid.bytes().all(|b| b.is_ascii_digit())) else { continue };
        if pid == me {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path()).is_ok_and(|target| target.to_string_lossy().starts_with("/dev/video")) {
                return true;
            }
        }
    }
    false
}

/// What a battery reading means next to the one before: plugged in, unplugged,
/// or running low (at 20% and again at 10%, once each until plugged in).
pub fn battery_blip(before: Option<Battery>, now: Battery, warned: &mut u8) -> Option<Blip> {
    let before = before?;
    if now.plugged {
        *warned = 100;
    }
    if now.plugged && !before.plugged {
        return Some(Blip { icon: BlipIcon::Battery { charging: true }, title: "Charging".into(), detail: format!("{}%", now.percent), level: Some(now.percent), warn: false });
    }
    if !now.plugged && before.plugged {
        let left = now.minutes.map(|minutes| format!(" · {} left", crate::island::duration_words(minutes))).unwrap_or_default();
        return Some(Blip { icon: BlipIcon::Battery { charging: false }, title: "On battery".into(), detail: format!("{}%{left}", now.percent), level: Some(now.percent), warn: now.percent <= 20 });
    }
    if !now.plugged {
        for threshold in [10_u8, 20] {
            if now.percent <= threshold && *warned > threshold {
                *warned = threshold;
                let title = if threshold == 10 { "Battery very low" } else { "Battery low" };
                return Some(Blip { icon: BlipIcon::Battery { charging: false }, title: title.into(), detail: format!("{}%", now.percent), level: Some(now.percent), warn: true });
            }
        }
    }
    None
}

/// Do Not Disturb turning on or off.
pub fn quiet_blip(before: Option<bool>, now: bool) -> Option<Blip> {
    (before? != now).then(|| Blip {
        icon: BlipIcon::Moon,
        title: "Do Not Disturb".into(),
        detail: if now { "On".into() } else { "Off".into() },
        level: None,
        warn: false,
    })
}

/// One sender's progress, from a `com.canonical.Unity.LauncherEntry` `Update`:
/// its application (`application://brave-browser.desktop`) and properties.
pub fn launcher_progress(app_uri: &str, properties: &BTreeMap<String, Prop>) -> Option<(String, Option<f32>)> {
    let visible = matches!(properties.get("progress-visible"), Some(Prop::Bool(true)));
    let progress = match properties.get("progress") { Some(Prop::Number(p)) => Some(p.clamp(0.0, 1.0) as f32), _ => None };
    let app = app_uri.trim_start_matches("application://").trim_end_matches(".desktop");
    let app = app.rsplit('.').next().unwrap_or(app);
    let name = app.split(['-', '_']).filter(|word| !word.is_empty())
        .map(|word| { let mut c = word.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default() })
        .collect::<Vec<_>>().join(" ");
    Some((name, if visible { progress.or(Some(0.0)) } else { None }))
}

#[derive(Default)]
struct Seen {
    bluetooth: Option<Vec<BtDevice>>,
    blips: Vec<Blip>,
    microphone: bool,
    camera: bool,
    /// App name → progress and when it was last heard.
    tasks: BTreeMap<String, (f32, Instant)>,
}

pub struct Happenings {
    seen: Arc<Mutex<Seen>>,
    queue: VecDeque<Blip>,
    showing: Option<(Blip, Instant)>,
    battery: Option<Battery>,
    warned: u8,
    quiet: Option<bool>,
}

impl Happenings {
    pub fn new(ctx: egui::Context) -> Self {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        {
            let seen = seen.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new().name("island-happenings".into()).spawn(move || watch(seen, ctx)).ok();
        }
        {
            let seen = seen.clone();
            std::thread::Builder::new().name("island-launcher".into()).spawn(move || listen_launcher(seen, ctx)).ok();
        }
        Self { seen, queue: VecDeque::new(), showing: None, battery: None, warned: 100, quiet: None }
    }

    /// Takes in this frame's battery and Do Not Disturb, and what the thread found.
    pub fn tick(&mut self, battery: Option<Battery>, quiet: bool) {
        if let Some(battery) = battery {
            if let Some(blip) = battery_blip(self.battery, battery, &mut self.warned) {
                self.queue.push_back(blip);
            }
            self.battery = Some(battery);
        }
        if let Some(blip) = quiet_blip(self.quiet, quiet) {
            self.queue.push_back(blip);
        }
        self.quiet = Some(quiet);
        if let Ok(mut seen) = self.seen.lock() {
            self.queue.extend(seen.blips.drain(..));
        }
        if self.showing.as_ref().is_some_and(|(_, since)| since.elapsed() >= BLIP_FOR) {
            self.showing = None;
        }
        if self.showing.is_none() {
            self.showing = self.queue.pop_front().map(|blip| (blip, Instant::now()));
        }
    }

    /// The event showing now, and how long it has left (for a repaint).
    pub fn blip(&self) -> Option<(Blip, Duration)> {
        self.showing.as_ref().map(|(blip, since)| (blip.clone(), BLIP_FOR.saturating_sub(since.elapsed())))
    }

    /// A download or copy under way, the furthest from done first.
    pub fn task(&self) -> Option<Task> {
        let seen = self.seen.lock().ok()?;
        let live: Vec<_> = seen.tasks.iter().filter(|(_, (_, heard))| heard.elapsed() < TASK_GONE).collect();
        let (app, (progress, _)) = live.iter().min_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
        let more = live.len().saturating_sub(1);
        Some(Task {
            title: "Downloading".into(),
            detail: if more > 0 { format!("{app} +{more}") } else { app.to_string() },
            progress: Some(*progress),
        })
    }

    /// Whether an application is using the camera, and the microphone.
    pub fn watching(&self) -> (bool, bool) {
        self.seen.lock().map_or((false, false), |seen| (seen.camera, seen.microphone))
    }
}

fn read_bluetooth(system: &zbus::blocking::Connection) -> Option<Vec<BtDevice>> {
    let reply = system.call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &()).ok()?;
    let raw: std::collections::HashMap<zbus::zvariant::OwnedObjectPath, std::collections::HashMap<String, std::collections::HashMap<String, OwnedValue>>> = reply.body().deserialize().ok()?;
    let objects = raw.into_iter().map(|(path, interfaces)| {
        let interfaces = interfaces.into_iter().map(|(name, props)| {
            (name, props.iter().filter_map(|(key, value)| Some((key.clone(), prop(value)?))).collect())
        }).collect();
        (path.to_string(), interfaces)
    }).collect();
    Some(bluetooth(&objects))
}

fn watch(seen: Arc<Mutex<Seen>>, ctx: egui::Context) {
    let system = zbus::blocking::Connection::system().ok();
    loop {
        let devices = system.as_ref().and_then(read_bluetooth);
        let (microphone, pw_camera) = std::process::Command::new("pw-dump").output().ok()
            .map(|out| capturing(&String::from_utf8_lossy(&out.stdout))).unwrap_or((false, false));
        let camera = pw_camera || camera_open();
        let mut changed = false;
        if let Ok(mut seen) = seen.lock() {
            if let Some(devices) = devices {
                if let Some(before) = &seen.bluetooth {
                    let blips = bluetooth_blips(before, &devices);
                    changed |= !blips.is_empty();
                    seen.blips.extend(blips);
                }
                seen.bluetooth = Some(devices);
            }
            changed |= seen.microphone != microphone || seen.camera != camera;
            seen.microphone = microphone;
            seen.camera = camera;
        }
        if changed {
            ctx.request_repaint();
        }
        std::thread::sleep(LOOK_EVERY);
    }
}

fn listen_launcher(seen: Arc<Mutex<Seen>>, ctx: egui::Context) {
    let Ok(session) = zbus::blocking::Connection::session() else { return };
    let rule = "type='signal',interface='com.canonical.Unity.LauncherEntry',member='Update'";
    let Ok(iterator) = zbus::blocking::MessageIterator::for_match_rule(rule, &session, Some(64)) else { return };
    for message in iterator.flatten() {
        let Ok((app_uri, properties)) = message.body().deserialize::<(String, std::collections::HashMap<String, OwnedValue>)>() else { continue };
        let properties: BTreeMap<String, Prop> = properties.iter().filter_map(|(key, value)| Some((key.clone(), prop(value)?))).collect();
        let Some((app, progress)) = launcher_progress(&app_uri, &properties) else { continue };
        if let Ok(mut seen) = seen.lock() {
            match progress {
                Some(progress) => { seen.tasks.insert(app, (progress, Instant::now())); }
                None => { seen.tasks.remove(&app); }
            }
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(percent: u8, plugged: bool) -> Battery {
        Battery { percent, plugged, minutes: None }
    }

    #[test]
    fn plugging_in_and_out_shows_and_the_first_reading_does_not() {
        let mut warned = 100;
        assert!(battery_blip(None, battery(50, true), &mut warned).is_none());
        let blip = battery_blip(Some(battery(50, false)), battery(50, true), &mut warned).unwrap();
        assert_eq!((blip.title.as_str(), blip.icon), ("Charging", BlipIcon::Battery { charging: true }));
        let blip = battery_blip(Some(battery(50, true)), Battery { percent: 50, plugged: false, minutes: Some(200) }, &mut warned).unwrap();
        assert_eq!(blip.title, "On battery");
        assert_eq!(blip.detail, "50% · 3 h 20 min left");
        assert!(battery_blip(Some(battery(50, false)), battery(49, false), &mut warned).is_none());
    }

    #[test]
    fn running_low_warns_once_at_twenty_and_once_at_ten() {
        let mut warned = 100;
        assert_eq!(battery_blip(Some(battery(21, false)), battery(20, false), &mut warned).unwrap().title, "Battery low");
        assert!(battery_blip(Some(battery(20, false)), battery(19, false), &mut warned).is_none());
        assert_eq!(battery_blip(Some(battery(11, false)), battery(10, false), &mut warned).unwrap().title, "Battery very low");
        assert!(battery_blip(Some(battery(10, false)), battery(9, false), &mut warned).is_none());
        // Plugged in and out again: it may warn again.
        battery_blip(Some(battery(9, false)), battery(9, true), &mut warned);
        assert_eq!(warned, 100);
    }

    #[test]
    fn do_not_disturb_shows_when_it_changes() {
        assert!(quiet_blip(None, true).is_none());
        assert!(quiet_blip(Some(true), true).is_none());
        assert_eq!(quiet_blip(Some(false), true).unwrap().detail, "On");
        assert_eq!(quiet_blip(Some(true), false).unwrap().detail, "Off");
    }

    fn device(path: &str, name: &str, connected: bool, battery: Option<u8>) -> (String, Interfaces) {
        let mut interfaces = Interfaces::new();
        interfaces.insert("org.bluez.Device1".into(), BTreeMap::from([
            ("Alias".to_string(), Prop::Text(name.into())),
            ("Connected".to_string(), Prop::Bool(connected)),
        ]));
        if let Some(p) = battery {
            interfaces.insert("org.bluez.Battery1".into(), BTreeMap::from([("Percentage".to_string(), Prop::Byte(p))]));
        }
        (path.into(), interfaces)
    }

    #[test]
    fn a_device_connecting_shows_with_its_battery() {
        let mut objects = BTreeMap::from([device("/org/bluez/hci0/dev_A", "Buds", false, None)]);
        objects.insert("/org/bluez/hci0".into(), Interfaces::from([("org.bluez.Adapter1".to_string(), BTreeMap::new())]));
        let before = bluetooth(&objects);
        assert_eq!(before.len(), 1, "the adapter is not a device");
        let now = bluetooth(&BTreeMap::from([device("/org/bluez/hci0/dev_A", "Buds", true, Some(80))]));
        let blips = bluetooth_blips(&before, &now);
        assert_eq!(blips.len(), 1);
        assert_eq!((blips[0].title.as_str(), blips[0].detail.as_str(), blips[0].level), ("Buds", "80%", Some(80)));
        assert!(bluetooth_blips(&now, &now).is_empty(), "staying connected says nothing");
        let plain = bluetooth(&BTreeMap::from([device("/org/bluez/hci0/dev_B", "Speaker", true, None)]));
        assert_eq!(bluetooth_blips(&[], &plain)[0].detail, "Connected");
    }

    #[test]
    fn only_running_capture_streams_count() {
        let dump = r#"[
            {"info":{"state":"running","props":{"media.class":"Stream/Output/Audio"}}},
            {"info":{"state":"suspended","props":{"media.class":"Stream/Input/Audio"}}},
            {"info":{"state":"running","props":{"media.class":"Audio/Source"}}}
        ]"#;
        assert_eq!(capturing(dump), (false, false));
        let dump = r#"[{"info":{"state":"running","props":{"media.class":"Stream/Input/Audio"}}},
                       {"info":{"state":"running","props":{"media.class":"Stream/Input/Video"}}}]"#;
        assert_eq!(capturing(dump), (true, true));
        assert_eq!(capturing("not json"), (false, false));
    }

    #[test]
    fn launcher_progress_names_the_app_and_ends_when_hidden() {
        let shown = BTreeMap::from([("progress".to_string(), Prop::Number(0.42)), ("progress-visible".to_string(), Prop::Bool(true))]);
        assert_eq!(launcher_progress("application://brave-browser.desktop", &shown), Some(("Brave Browser".into(), Some(0.42))));
        assert_eq!(launcher_progress("application://org.gnome.Nautilus.desktop", &shown).unwrap().0, "Nautilus");
        let hidden = BTreeMap::from([("progress-visible".to_string(), Prop::Bool(false))]);
        assert_eq!(launcher_progress("application://brave-browser.desktop", &hidden), Some(("Brave Browser".into(), None)));
    }
}
