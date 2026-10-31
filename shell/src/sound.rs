//! Sound: which device plays and which one listens, how loud each is, and how loud
//! each application is. All of it goes through `wpctl`, PipeWire's own tool, on
//! background threads, so that a slow answer never stalls the interface.

use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, Sense, Stroke, StrokeKind, Ui, vec2};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Something that plays, listens, or is an application making sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    /// The one in use, among the outputs or the inputs.
    pub chosen: bool,
    /// Percent; `None` when it has not been read.
    pub volume: Option<u8>,
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Devices {
    pub outputs: Vec<Node>,
    pub inputs: Vec<Node>,
    /// Applications playing or recording something now.
    pub streams: Vec<Node>,
    /// Outputs a sound card has but cannot use as it is set up: the HDMI of a card
    /// set for its own speakers, say, which can play only one or the other.
    pub elsewhere: Vec<Elsewhere>,
}

/// An output that a sound card reaches only by changing over to another profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Elsewhere {
    /// The card, as PipeWire numbers it.
    pub device: u32,
    /// The profile to change over to.
    pub profile: u32,
    /// The connector route to use on that profile.
    pub route: u32,
    pub name: String,
}

/// Reads `pw-dump` for the outputs each sound card could change over to. The profile
/// chosen for one keeps the card's microphone if the current profile has it, and is
/// otherwise the card's own favourite among those that reach it.
pub fn parse_elsewhere(dump: &str) -> Vec<Elsewhere> {
    use serde_json::Value;
    let Ok(Value::Array(objects)) = serde_json::from_str::<Value>(dump) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for object in &objects {
        let info = &object["info"];
        if info["props"]["media.class"] != "Audio/Device" {
            continue;
        }
        let Some(device) = object["id"].as_u64() else { continue };
        let params = &info["params"];
        let list = |name: &str| params[name].as_array().cloned().unwrap_or_default();
        let Some(active) = list("Profile").first().and_then(|profile| profile["index"].as_u64()) else { continue };
        let profiles = list("EnumProfile");
        let profile = |index: u64| profiles.iter().find(|profile| profile["index"].as_u64() == Some(index));
        // Whether a profile listens as well as plays.
        let listens = |profile: &Value| profile["name"].as_str().is_some_and(|name| name.contains("input:"));
        let keep_input = profile(active).is_some_and(listens);
        let product = info["props"]["device.description"].as_str().unwrap_or("Sound card");
        for route in list("EnumRoute") {
            if route["direction"] != "Output" || route["available"] == "no" {
                continue;
            }
            let reaches: Vec<u64> = route["profiles"].as_array().map(|all| all.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
            if reaches.is_empty() || reaches.contains(&active) {
                continue;
            }
            let best = reaches
                .iter()
                .filter_map(|index| profile(*index))
                .filter(|profile| profile["available"] != "no")
                .max_by_key(|profile| (listens(profile) == keep_input, profile["priority"].as_i64().unwrap_or(0)));
            let Some(route_index) = route["index"].as_u64() else { continue };
            let Some(index) = best.and_then(|profile| profile["index"].as_u64()) else { continue };
            // What is plugged in names itself, a TV say; otherwise the card and the port.
            let info = route["info"].as_array().cloned().unwrap_or_default();
            let plugged = info.iter().skip(1).collect::<Vec<_>>().chunks(2).find(|pair| pair[0] == "device.product.name").and_then(|pair| pair.get(1)?.as_str().map(str::to_string));
            let port = route["description"].as_str().unwrap_or("Output");
            let name = match plugged {
                Some(plugged) => format!("{plugged} ({port})"),
                None => format!("{product} {port}"),
            };
            if !found.iter().any(|known: &Elsewhere| known.device == device as u32 && known.name == name) {
                found.push(Elsewhere { device: device as u32, profile: index as u32, route: route_index as u32, name });
            }
        }
    }
    found
}

/// Wait for the requested profile, then match the route's ACP device indices to
/// the sink's card.profile.device (the same mapping wpctl set-route uses).
fn output_of(dump: &str, place: &Elsewhere) -> Option<u32> {
    let objects: Vec<serde_json::Value> = serde_json::from_str(dump).ok()?;
    let card = objects.iter().find(|object| object["id"].as_u64() == Some(place.device as u64))?;
    let params = &card["info"]["params"];
    if !params["Profile"].as_array()?.iter().any(|profile| profile["index"].as_u64() == Some(place.profile as u64)) {
        return None;
    }
    let route = params["EnumRoute"].as_array()?.iter().find(|route| {
        route["index"].as_u64() == Some(place.route as u64) && route["direction"] == "Output"
    })?;
    let devices = route["devices"].as_array()?;
    objects.iter().find_map(|object| {
        let props = &object["info"]["props"];
        let route_device = props["card.profile.device"].as_u64()?;
        let belongs = props["media.class"] == "Audio/Sink"
            && props["device.id"].as_u64() == Some(place.device as u64)
            && devices.iter().any(|device| device.as_u64() == Some(route_device));
        belongs.then(|| object["id"].as_u64()).flatten().map(|id| id as u32)
    })
}

/// The callbacks keep profile transitions testable without changing system audio.
fn change_profile(
    place: &Elsewhere,
    mut command: impl FnMut(&[&str]) -> Option<String>,
    mut dump: impl FnMut() -> Option<String>,
    mut wait: impl FnMut(),
) {
    if command(&["set-profile", &place.device.to_string(), &place.profile.to_string()]).is_none() {
        return;
    }
    for _ in 0..20 {
        if let Some(output) = dump().and_then(|dump| output_of(&dump, place)) {
            if command(&["set-route", &output.to_string(), &place.route.to_string()]).is_some() {
                let _ = command(&["set-default", &output.to_string()]);
            }
            return;
        }
        wait();
    }
}

fn pw_dump() -> Option<String> {
    let output = Command::new("timeout").arg("5").arg("pw-dump").stdin(std::process::Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads a `[vol: 0.64]` or `[vol: 0.40 MUTED]` mark, or `Volume: 0.40 [MUTED]`.
pub fn parse_level(text: &str) -> (Option<u8>, bool) {
    let number = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|word| word.contains('.') && word.parse::<f32>().is_ok())
        .and_then(|word| word.parse::<f32>().ok());
    (number.map(|level| (level * 100.0).round().clamp(0.0, 150.0) as u8), text.contains("MUTED"))
}

/// Reads `wpctl status`: the outputs, inputs and streams of its audio part.
pub fn parse_status(status: &str) -> Devices {
    #[derive(PartialEq)]
    enum Part {
        Other,
        Outputs,
        Inputs,
        Streams,
    }
    let mut devices = Devices::default();
    let (mut audio, mut part) = (false, Part::Other);
    for line in status.lines() {
        // The parts are told apart by the lines that head them.
        let bare = line.trim_matches(|c: char| c.is_whitespace() || "│├└─".contains(c));
        match bare {
            "Audio" => {
                audio = true;
                continue;
            }
            "Video" | "Settings" => {
                audio = false;
                continue;
            }
            "Sinks:" => part = Part::Outputs,
            "Sources:" => part = Part::Inputs,
            "Streams:" => part = Part::Streams,
            "Devices:" | "Filters:" => part = Part::Other,
            _ => {}
        }
        if !audio || part == Part::Other || bare.ends_with(':') {
            continue;
        }
        // A stream's own lines are followed by those of its channels, which point somewhere.
        if bare.contains(" > ") {
            continue;
        }
        let (chosen, rest) = match bare.strip_prefix('*') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, bare),
        };
        let Some((id, rest)) = rest.split_once(". ") else { continue };
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        let (name, level) = match rest.rfind("[vol:") {
            Some(at) => (&rest[..at], parse_level(&rest[at..])),
            None => (rest, (None, false)),
        };
        let node = Node { id, name: name.trim().to_string(), chosen, volume: level.0, muted: level.1 };
        if node.name.is_empty() {
            continue;
        }
        match part {
            Part::Outputs => devices.outputs.push(node),
            Part::Inputs => devices.inputs.push(node),
            Part::Streams => devices.streams.push(node),
            Part::Other => {}
        }
    }
    devices
}

fn wpctl(args: &[&str]) -> Option<String> {
    let output = Command::new("timeout").arg("5").arg("wpctl").args(args).stdin(std::process::Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read() -> Option<Devices> {
    let mut devices = parse_status(&wpctl(&["status"])?);
    // The list does not say how loud each application is; that is asked one by one.
    for stream in devices.streams.iter_mut().take(12) {
        if let Some(level) = wpctl(&["get-volume", &stream.id.to_string()]) {
            (stream.volume, stream.muted) = parse_level(&level);
        }
    }
    devices.elsewhere = pw_dump().map(|dump| parse_elsewhere(&dump)).unwrap_or_default();
    Some(devices)
}

struct Shared {
    /// `None` until first read, or when sound cannot be asked about.
    devices: Option<Devices>,
    read_at: Option<Instant>,
    reading: bool,
    /// Whether anything was ever read: tells "not yet" from "cannot".
    tried: bool,
}

/// The sound part of the settings.
pub struct Sound {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    /// A level being dragged, held here until the system reports it back: the node,
    /// the level, and when.
    held: Option<(u32, u8, Instant)>,
}

impl Sound {
    pub fn new(ctx: egui::Context) -> Sound {
        Sound { shared: Arc::new(Mutex::new(Shared { devices: None, read_at: None, reading: false, tried: false })), ctx, held: None }
    }

    /// Does something with `wpctl`, if anything, then reads how things stand.
    fn act(&self, steps: Vec<Vec<String>>) {
        let something = !steps.is_empty();
        let work = move || {
            for step in steps {
                let args: Vec<&str> = step.iter().map(String::as_str).collect();
                let _ = wpctl(&args);
            }
        };
        self.act_with(work, something);
    }

    /// Changes a card over to the profile that reaches an output, then plays through
    /// that output once it appears.
    fn change_over(&self, place: &Elsewhere) {
        let place = place.clone();
        self.act_with(
            move || change_profile(&place, wpctl, pw_dump, || std::thread::sleep(Duration::from_millis(150))),
            true,
        );
    }

    /// Runs `work` in the background, then reads how things stand. With nothing to
    /// do, a reading already under way is enough.
    fn act_with(&self, work: impl FnOnce() + Send + 'static, something: bool) {
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        {
            let mut state = shared.lock().unwrap();
            if state.reading && !something {
                return;
            }
            state.reading = true;
        }
        std::thread::spawn(move || {
            work();
            let devices = read();
            let mut state = shared.lock().unwrap();
            state.devices = devices;
            state.read_at = Some(Instant::now());
            state.reading = false;
            state.tried = true;
            drop(state);
            ctx.request_repaint();
        });
    }

    /// The level to show for a node: the one being set, while that is fresh.
    fn level(&mut self, node: &Node) -> u8 {
        match self.held {
            Some((id, level, at)) if id == node.id && at.elapsed() < Duration::from_millis(1500) => level,
            _ => node.volume.unwrap_or(0),
        }
    }

    /// A slider for a node's level, with a way to silence it.
    fn level_row(&mut self, ui: &mut Ui, theme: &Theme, node: &Node, name: &str) {
        let level = self.level(node);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            // The slider takes what the name, the reading and the mute leave it.
            let wide = (ui.available_width() - 330.0).clamp(110.0, 260.0);
            let (label, _) = ui.allocate_exact_size(vec2(130.0, 24.0), Sense::hover());
            ui.painter().text(label.left_center(), Align2::LEFT_CENTER, crate::panels::truncate(name, 15), ui::display(13.0), if node.muted { ui::LABEL } else { ui::TEXT });
            if let Some(frac) = ui::slider(ui, theme, wide, level.min(100) as f32 / 100.0, name) {
                let new = (frac * 100.0).round() as u8;
                if new != level {
                    self.held = Some((node.id, new, Instant::now()));
                    self.act(vec![vec!["set-volume".into(), "-l".into(), "1.0".into(), node.id.to_string(), format!("{:.2}", new as f32 / 100.0)]]);
                }
            }
            let (reading, _) = ui.allocate_exact_size(vec2(34.0, 24.0), Sense::hover());
            ui.painter().text(reading.left_center(), Align2::LEFT_CENTER, format!("{level}"), ui::mono(12.0), ui::TEXT);
            if ui::chip(ui, theme, if node.muted { "MUTED" } else { "MUTE" }, None, node.muted).clicked() {
                self.act(vec![vec!["set-mute".into(), node.id.to_string(), "toggle".into()]]);
            }
        });
        ui.add_space(6.0);
    }

    /// The devices of one kind, to choose among, and the level of the one in use.
    fn devices(&mut self, ui: &mut Ui, theme: &Theme, title: &str, none: &str, nodes: &[Node], elsewhere: &[Elsewhere]) {
        ui::section(ui, title, |_| {});
        if nodes.is_empty() && elsewhere.is_empty() {
            ui::text(ui, none, ui::mono(12.0), ui::LABEL);
            ui.add_space(22.0);
            return;
        }
        for node in nodes {
            let state = if node.chosen { "IN USE" } else { "USE" };
            if row(ui, theme, &node.name, state, node.chosen) && !node.chosen {
                self.act(vec![vec!["set-default".into(), node.id.to_string()]]);
            }
        }
        for place in elsewhere {
            if row(ui, theme, &place.name, "USE", false) {
                self.change_over(place);
            }
        }
        ui.add_space(10.0);
        if let Some(chosen) = nodes.iter().find(|node| node.chosen) {
            self.level_row(ui, theme, chosen, "Level");
        }
        ui.add_space(16.0);
    }

    /// The outputs to choose among, compactly, for the quick settings.
    pub fn quick_outputs(&mut self, ui: &mut Ui, theme: &Theme) {
        let (devices, stale, tried) = {
            let state = self.shared.lock().unwrap();
            (state.devices.clone(), state.read_at.is_none_or(|at| at.elapsed() > Duration::from_secs(3)), state.tried)
        };
        if stale {
            self.act(Vec::new());
        }
        let Some(devices) = devices else {
            ui::text(ui, if tried { "Could not read output devices" } else { "Looking for output devices…" }, ui::display(12.0), ui::SECONDARY);
            return;
        };
        if devices.outputs.is_empty() && devices.elsewhere.is_empty() {
            ui::text(ui, "No output devices available", ui::display(12.0), ui::SECONDARY);
            return;
        }
        for node in devices.outputs.iter().take(4) {
            let state = if node.chosen { "IN USE" } else { "USE" };
            if row(ui, theme, &crate::panels::truncate(&node.name, 26), state, node.chosen) && !node.chosen {
                self.act(vec![vec!["set-default".into(), node.id.to_string()]]);
            }
        }
        for place in devices.elsewhere.iter().take(4usize.saturating_sub(devices.outputs.len())) {
            if row(ui, theme, &crate::panels::truncate(&place.name, 26), "USE", false) {
                self.change_over(place);
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme) {
        let (devices, stale, tried) = {
            let state = self.shared.lock().unwrap();
            (state.devices.clone(), state.read_at.is_none_or(|at| at.elapsed() > Duration::from_secs(3)), state.tried)
        };
        // Devices are plugged in and applications start and stop: keep looking.
        if stale {
            self.act(Vec::new());
            ui.ctx().request_repaint_after(Duration::from_secs(3));
        }
        let Some(devices) = devices else {
            ui::section(ui, "OUTPUT", |_| {});
            let text = if tried { "Sound cannot be asked about: wpctl, PipeWire's tool, gave no answer." } else { "Looking for sound devices…" };
            ui::text(ui, text, ui::mono(12.0), ui::LABEL);
            return;
        };
        self.devices(ui, theme, "OUTPUT", "No device to play through was found.", &devices.outputs, &devices.elsewhere);
        self.devices(ui, theme, "INPUT", "No microphone was found.", &devices.inputs, &[]);

        ui::section(ui, "APPLICATIONS", |_| {});
        if devices.streams.is_empty() {
            ui::text(ui, "Nothing is playing or recording at the moment.", ui::mono(12.0), ui::LABEL);
        }
        for stream in &devices.streams {
            let name = stream.name.clone();
            self.level_row(ui, theme, stream, &name);
        }
    }
}

/// One device in a list. Returns true when it was clicked.
fn row(ui: &mut Ui, theme: &Theme, name: &str, state: &str, live: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let painter = ui.painter_at(rect);
    if live {
        painter.rect_filled(rect, 0.0, theme.tint());
        painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    } else {
        if response.hovered() {
            painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::PANEL);
        }
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ui::TRACK));
    }
    let mark = egui::Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(6.0, 6.0));
    painter.rect_filled(mark, 0.0, if live { theme.main } else { ui::LINE_STRONG });
    painter.text(rect.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, crate::panels::truncate(name, 48), ui::mono(13.0), ui::TEXT);
    let ink = if live { theme.main } else if response.hovered() { ui::TEXT } else { ui::LABEL };
    painter.text(rect.right_center() - vec2(14.0, 0.0), Align2::RIGHT_CENTER, state, ui::mono(11.0), ink);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "PipeWire 'pipewire-0' [1.6.2, ada@workstation, cookie:1]
 └─ Clients:
        33. pipewire                            [1.6.2, ada@workstation, pid:2315]

Audio
 ├─ Devices:
 │      55. Built-in Audio                      [alsa]
 │  
 ├─ Sinks:
 │  *   44. Built-in Audio Analog Stereo        [vol: 1.00]
 │      67. Kitchen Speaker                     [vol: 0.64 MUTED]
 │  
 ├─ Sources:
 │  *   54. Built-in Audio Analog Stereo        [vol: 0.27]
 │  
 ├─ Filters:
 │  
 └─ Streams:
        69. pw-cat                                                      
             72. output_FR       > ALC255 Analog:playback_FR\t[active]
             73. output_FL       > ALC255 Analog:playback_FL\t[active]
        81. Brave                                                       
             82. output_FL       > ALC255 Analog:playback_FL\t[active]

Video
 ├─ Sources:
 │  *   53. HD WebCam (V4L2)                   

Settings
 └─ Default Configured Devices:
         0. Audio/Sink    alsa_output.pci-0000_00_1f.3.hdmi-stereo
";

    #[test]
    fn reads_devices_and_streams() {
        let devices = parse_status(STATUS);
        assert_eq!(
            devices.outputs,
            [
                Node { id: 44, name: "Built-in Audio Analog Stereo".into(), chosen: true, volume: Some(100), muted: false },
                Node { id: 67, name: "Kitchen Speaker".into(), chosen: false, volume: Some(64), muted: true },
            ]
        );
        assert_eq!(devices.inputs.len(), 1, "the camera is no microphone");
        assert_eq!((devices.inputs[0].id, devices.inputs[0].volume), (54, Some(27)));
        let streams: Vec<(u32, &str)> = devices.streams.iter().map(|stream| (stream.id, stream.name.as_str())).collect();
        assert_eq!(streams, [(69, "pw-cat"), (81, "Brave")], "applications, not their channels");
        assert_eq!(parse_status(""), Devices::default());
    }

    /// A built-in card set for its speakers, with a TV on its HDMI, trimmed from `pw-dump`.
    const DUMP: &str = r#"[
      {"id": 49, "type": "PipeWire:Interface:Device", "info": {
        "props": {"media.class": "Audio/Device", "device.description": "Built-in Audio"},
        "params": {
          "Profile": [{"index": 1, "name": "output:analog-stereo+input:analog-stereo"}],
          "EnumProfile": [
            {"index": 1, "name": "output:analog-stereo+input:analog-stereo", "priority": 6565, "available": "yes"},
            {"index": 3, "name": "output:hdmi-stereo+input:analog-stereo", "priority": 5965, "available": "yes"},
            {"index": 4, "name": "output:hdmi-stereo", "priority": 5900, "available": "yes"},
            {"index": 5, "name": "output:hdmi-stereo-extra1+input:analog-stereo", "priority": 5765, "available": "no"}
          ],
          "EnumRoute": [
            {"index": 2, "direction": "Output", "name": "analog-output-speaker", "description": "Speakers", "available": "unknown", "profiles": [1, 2]},
            {"index": 0, "direction": "Input", "name": "analog-input-internal-mic", "description": "Internal Microphone", "available": "unknown", "profiles": [1, 3]},
            {"index": 4, "direction": "Output", "name": "hdmi-output-0", "description": "HDMI / DisplayPort", "available": "yes",
             "info": [2, "port.type", "hdmi", "device.product.name", "Beyond TV"], "devices": [7], "profiles": [4, 3]},
            {"index": 5, "direction": "Output", "name": "hdmi-output-1", "description": "HDMI / DisplayPort 2", "available": "no", "profiles": [5]}
          ]
        }}},
      {"id": 53, "info": {"props": {"media.class": "Audio/Sink", "device.id": 49, "card.profile.device": 0}}}
    ]"#;

    #[test]
    fn finds_outputs_a_card_has_to_change_over_to() {
        // The TV, by the profile that keeps the microphone; nothing for the unplugged port.
        assert_eq!(parse_elsewhere(DUMP), [Elsewhere { device: 49, profile: 3, route: 4, name: "Beyond TV (HDMI / DisplayPort)".into() }]);
        // Once changed over, the speakers are the ones elsewhere.
        let changed = DUMP.replace(r#""Profile": [{"index": 1,"#, r#""Profile": [{"index": 3,"#);
        let elsewhere = parse_elsewhere(&changed);
        assert_eq!(elsewhere.len(), 1);
        assert_eq!((elsewhere[0].profile, elsewhere[0].name.as_str()), (1, "Built-in Audio Speakers"));
        assert!(parse_elsewhere("not json").is_empty());
    }

    #[test]
    fn does_not_select_the_old_sink_while_a_profile_changes() {
        // The original analog sink still belongs to this card until teardown finishes.
        assert_eq!(output_of(DUMP, &parse_elsewhere(DUMP)[0]), None);
    }

    fn hdmi_ready() -> String {
        let mut objects: Vec<serde_json::Value> = serde_json::from_str(DUMP).unwrap();
        objects[0]["info"]["params"]["Profile"][0]["index"] = 3.into();
        // The analog sink is still present, before the HDMI sink in the dump.
        objects.push(serde_json::json!({"id": 54, "info": {"props": {
            "media.class": "Audio/Sink", "device.id": 49, "card.profile.device": 7
        }}}));
        serde_json::to_string(&objects).unwrap()
    }

    #[test]
    fn chooses_the_requested_route_among_multiple_card_outputs() {
        let place = &parse_elsewhere(DUMP)[0];
        assert_eq!(output_of(&hdmi_ready(), place), Some(54));
        // An active profile alone is insufficient while the new sink is not ready.
        let mut objects: Vec<serde_json::Value> = serde_json::from_str(&hdmi_ready()).unwrap();
        objects.pop();
        assert_eq!(output_of(&serde_json::to_string(&objects).unwrap(), place), None);
    }

    #[test]
    fn waits_for_hdmi_then_sets_its_route_and_default() {
        let place = &parse_elsewhere(DUMP)[0];
        let mut snapshots = [DUMP.to_string(), hdmi_ready()].into_iter();
        let mut commands = Vec::new();
        let mut waits = 0;
        change_profile(place, |args| {
            commands.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            Some(String::new())
        }, || snapshots.next(), || waits += 1);
        assert_eq!(waits, 1);
        assert_eq!(commands, [
            vec!["set-profile", "49", "3"],
            vec!["set-route", "54", "4"],
            vec!["set-default", "54"],
        ]);
    }

    #[test]
    fn failed_profile_or_route_never_changes_the_default() {
        let place = &parse_elsewhere(DUMP)[0];
        let mut commands = Vec::new();
        change_profile(place, |args| {
            commands.push(args[0].to_string());
            None
        }, || panic!("a failed profile must not poll for sinks"), || panic!("must not wait"));
        assert_eq!(commands, ["set-profile"]);

        commands.clear();
        change_profile(place, |args| {
            commands.push(args[0].to_string());
            (args[0] != "set-route").then(String::new)
        }, || Some(hdmi_ready()), || panic!("sink is already ready"));
        assert_eq!(commands, ["set-profile", "set-route"]);
    }

    #[test]
    fn absent_target_times_out_without_selecting_another_sink() {
        let place = &parse_elsewhere(DUMP)[0];
        let mut commands = Vec::new();
        let mut waits = 0;
        change_profile(place, |args| {
            commands.push(args[0].to_string());
            Some(String::new())
        }, || Some(DUMP.to_string()), || waits += 1);
        assert_eq!(commands, ["set-profile"]);
        assert_eq!(waits, 20);
    }

    #[test]
    fn reads_levels() {
        assert_eq!(parse_level("Volume: 0.55"), (Some(55), false));
        assert_eq!(parse_level("Volume: 0.40 [MUTED]"), (Some(40), true));
        assert_eq!(parse_level("[vol: 1.20]"), (Some(120), false));
        assert_eq!(parse_level("nothing"), (None, false));
    }
}
