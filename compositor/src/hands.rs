//! Hand control: des-hands follows a hand in front of the webcam and says what it
//! does, a line at a time on its standard output; here that becomes the pointer
//! moving, clicks, scrolling and workspaces, through the same paths as a mouse's.
//!
//! The camera sends ten to thirty pictures a second, fewer than the displays show.
//! So each movement is spread over the time until the next one is due, and the
//! pointer glides at the display's rate instead of jumping at the camera's.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use des_common::hands::{Button, Message, Side};
use des_common::ipc::{Event, Hands};
use smithay::backend::input::{Axis, AxisSource, ButtonState};
use smithay::input::pointer::AxisFrame;
use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken, generic::Generic};

use crate::DesComp;

/// The buttons, as the kernel numbers them.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// How far to scroll, in logical pixels, for the hand going the whole view.
const SCROLL: f64 = 1200.0;
/// The time between the camera's movements is taken to be within these.
const SHORTEST: Duration = Duration::from_millis(16);
const LONGEST: Duration = Duration::from_millis(120);

/// des-hands running, and the pointer's movement still to make.
pub struct Running {
    child: Child,
    source: RegistrationToken,
    /// Preferences this process was started with.
    settings: HandSettings,
    /// Bytes of a line not yet ended.
    partial: Vec<u8>,
    /// Logical pixels still to go, and in how long.
    pending: (f64, f64),
    glide_left: Duration,
    last_glide: Instant,
    /// How long, lately, between one movement and the next.
    interval: Duration,
    last_motion: Option<Instant>,
    /// Buttons it has pressed and not let go.
    held: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
struct HandSettings {
    speed: f32,
    pinch_sensitivity: f32,
    scroll_speed: f32,
    drag_lock: bool,
    camera: Option<des_common::hands::CameraSelection>,
}

impl From<&des_common::input::InputSettings> for HandSettings {
    fn from(input: &des_common::input::InputSettings) -> Self {
        Self { speed: input.hand_speed, pinch_sensitivity: input.hand_pinch_sensitivity,
            scroll_speed: input.hand_scroll_speed, drag_lock: input.hand_drag_lock,
            camera: des_common::hands::CameraSelection::load().ok().flatten() }
    }
}

impl HandSettings {
    fn args(&self) -> Vec<String> {
        vec!["--serve".into(), "--speed".into(), self.speed.to_string(),
            "--pinch-sensitivity".into(), self.pinch_sensitivity.to_string(),
            "--scroll-speed".into(), self.scroll_speed.to_string(),
            "--drag-lock".into(), if self.drag_lock { "on" } else { "off" }.into()]
    }
}

// A temporary Super+H choice remains in effect when adjusting preferences.
fn restart_choice(wanted: bool, previous: Option<bool>, running: bool, changed: bool) -> Option<bool> {
    if previous != Some(wanted) { Some(wanted) }
    else if running && changed { Some(true) }
    else { None }
}

/// How much of `pending` to move by now, `elapsed` since the last time, with
/// `left` to go: all of it once the time is up, otherwise its share.
pub fn glide_step(pending: (f64, f64), left: Duration, elapsed: Duration) -> (f64, f64) {
    let share = if left <= elapsed { 1.0 } else { elapsed.as_secs_f64() / left.as_secs_f64() };
    (pending.0 * share, pending.1 * share)
}

/// The time between movements, as it goes on: mostly what it was, partly the newest.
pub fn next_interval(interval: Duration, gap: Duration) -> Duration {
    if gap > Duration::from_millis(300) {
        return interval;
    }
    let blended = interval.as_secs_f64() * 0.7 + gap.as_secs_f64() * 0.3;
    Duration::from_secs_f64(blended).clamp(SHORTEST, LONGEST)
}

/// The program: beside this one when installed together, or wherever the search
/// path finds it.
fn program() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("des-hands"))
        .filter(|path| path.exists())
        .unwrap_or_else(|| "des-hands".into())
}

impl DesComp {
    fn tell_hands(&mut self, state: Hands) {
        if self.hands_state != state {
            self.hands_state = state;
            // Commands temporarily take the socket out of the compositor. Queue
            // the transition there too, including camera-on before any hand is seen.
            self.tell(Event::hands(state));
        }
    }

    /// Hand control as the settings have it, when they have changed: on or off, and
    /// started again at a new speed.
    pub fn apply_hand_settings(&mut self) {
        let wanted = self.input_settings.hand_control;
        let settings = HandSettings::from(&self.input_settings);
        let changed = self.hands.as_ref().is_some_and(|running| running.settings != settings);
        let restart = restart_choice(wanted, self.hands_setting, self.hands.is_some(), changed);
        self.hands_setting = Some(wanted);
        if let Some(start) = restart {
            self.stop_hands();
            if start {
                self.start_hands();
            }
        }
    }

    /// Super+H, or the indicator: on if it is off, off if it is on, for now.
    pub fn toggle_hands(&mut self) {
        if self.hands.is_some() {
            self.stop_hands();
        } else {
            self.start_hands();
        }
    }

    fn start_hands(&mut self) {
        let settings = HandSettings::from(&self.input_settings);
        let camera = match des_common::hands::camera_path() {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!("cannot use selected hand camera: {error}");
                self.tell_hands(Hands::Failed);
                return;
            }
        };
        let spawned = Command::new(program())
            .args(settings.args())
            .arg("--camera").arg(camera)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                tracing::warn!("cannot start hand control: {e}");
                self.tell_hands(Hands::Failed);
                return;
            }
        };
        let stdout = child.stdout.take().expect("piped");
        let _ = smithay::reexports::rustix::io::ioctl_fionbio(&stdout, true);
        let inserted = self.loop_handle.insert_source(Generic::new(stdout, Interest::READ, Mode::Level), |_, stdout, state| {
            let mut buffer = [0u8; 4096];
            loop {
                // Safety: the pipe is only read from, never closed, here.
                match unsafe { stdout.get_mut() }.read(&mut buffer) {
                    Ok(0) => {
                        state.hands_ended();
                        return Ok(PostAction::Remove);
                    }
                    Ok(count) => state.hands_bytes(&buffer[..count]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        state.hands_ended();
                        return Ok(PostAction::Remove);
                    }
                }
            }
        });
        let source = match inserted {
            Ok(source) => source,
            Err(e) => {
                tracing::warn!("cannot listen to hand control: {e}");
                let _ = child.kill();
                let _ = child.wait();
                self.tell_hands(Hands::Failed);
                return;
            }
        };
        tracing::info!(pid = child.id(), "hand control on");
        self.hands = Some(Running {
            child,
            source,
            settings,
            partial: Vec::new(),
            pending: (0.0, 0.0),
            glide_left: Duration::ZERO,
            last_glide: Instant::now(),
            interval: Duration::from_millis(60),
            last_motion: None,
            held: Vec::new(),
        });
        self.tell_hands(Hands::Watching);
    }

    /// Stops des-hands, which lets go of the camera, and of any button it held.
    pub fn stop_hands(&mut self) {
        self.hand_feedback.reset();
        let Some(mut running) = self.hands.take() else {
            return;
        };
        self.loop_handle.remove(running.source);
        let _ = running.child.kill();
        let _ = running.child.wait();
        self.let_go(std::mem::take(&mut running.held));
        tracing::info!("hand control off");
        self.tell_hands(Hands::Off);
    }

    /// des-hands stopped by itself: without a camera, say.
    fn hands_ended(&mut self) {
        self.hand_feedback.reset();
        let Some(mut running) = self.hands.take() else {
            return;
        };
        let status = running.child.wait();
        self.let_go(std::mem::take(&mut running.held));
        tracing::warn!(?status, "hand control stopped");
        self.tell_hands(Hands::Failed);
    }

    fn let_go(&mut self, held: Vec<u32>) {
        let time = self.start_time.elapsed().as_millis() as u32;
        for button in held {
            self.pointer_button(button, ButtonState::Released, time);
        }
    }

    fn hands_bytes(&mut self, bytes: &[u8]) {
        let Some(running) = self.hands.as_mut() else {
            return;
        };
        running.partial.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(end) = running.partial.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = running.partial.drain(..=end).collect();
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
        // Nothing sensible is that long without ending.
        if running.partial.len() > 4096 {
            running.partial.clear();
        }
        for line in lines {
            if let Some(message) = Message::decode(&line) {
                self.hand_message(message);
            }
        }
    }

    /// The size of the display the pointer is on.
    fn pointer_screen(&self) -> (f64, f64) {
        let at = self.seat.get_pointer().unwrap().current_location();
        self.screens
            .iter()
            .find(|screen| screen.to_f64().contains(at))
            .or(self.screens.first())
            .map_or((1920.0, 1080.0), |screen| (screen.size.w as f64, screen.size.h as f64))
    }

    fn hand_message(&mut self, message: Message) {
        let time = self.start_time.elapsed().as_millis() as u32;
        match message {
            Message::Motion { dx, dy } => {
                let (width, height) = self.pointer_screen();
                let Some(running) = self.hands.as_mut() else { return };
                let now = Instant::now();
                if let Some(last) = running.last_motion.replace(now) {
                    running.interval = next_interval(running.interval, now - last);
                }
                running.pending.0 += dx as f64 * width;
                running.pending.1 += dy as f64 * height;
                running.glide_left = running.interval;
                running.last_glide = now;
            }
            Message::Press(which) | Message::Release(which) => {
                let button = if which == Button::Left { BTN_LEFT } else { BTN_RIGHT };
                let pressed = matches!(message, Message::Press(_));
                // Click where the pointer is visibly resting, not at a later
                // camera sample that has not yet reached the display.
                if let Some(running) = self.hands.as_mut() {
                    running.pending = (0.0, 0.0);
                    running.glide_left = Duration::ZERO;
                }
                if let Some(running) = self.hands.as_mut() {
                    if pressed {
                        if running.held.contains(&button) { return; }
                        running.held.push(button);
                    } else if let Some(at) = running.held.iter().position(|held| *held == button) {
                        running.held.remove(at);
                    } else {
                        return;
                    }
                }
                self.hand_feedback.button(which, pressed, Instant::now());
                self.input_activity();
                self.pointer_button(button, if pressed { ButtonState::Pressed } else { ButtonState::Released }, time);
            }
            Message::Scroll(by) if self.overview_open().is_none() => {
                self.input_activity();
                // As on a touchpad: with natural scrolling the page follows the hand.
                let sign = if self.input_settings.touchpad_natural_scroll { -1.0 } else { 1.0 };
                let amount = sign * by as f64 / 2.0 * SCROLL;
                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, AxisFrame::new(time).source(AxisSource::Continuous).value(Axis::Vertical, amount));
                pointer.frame(self);
            }
            Message::Scroll(_) => {}
            // As on a touchpad: towards the left is the next workspace.
            Message::Sweep(side) => {
                if self.locked || self.picking { return; }
                self.input_activity();
                match side {
                    Side::Left => self.switch_workspace(self.workspace + 1),
                    Side::Right if self.workspace > 0 => self.switch_workspace(self.workspace - 1),
                    Side::Right => {}
                }
            }
            Message::Seen | Message::Wake => self.tell_hands(Hands::Following),
            Message::Rest | Message::Gone => {
                // Rest is a lift, not a final movement. Do not jump to a pending
                // camera sample or leave a malformed/lost gesture holding a button.
                if let Some(running) = self.hands.as_mut() {
                    running.pending = (0.0, 0.0);
                    running.glide_left = Duration::ZERO;
                    let held = std::mem::take(&mut running.held);
                    self.let_go(held);
                }
                self.hand_feedback.reset();
                self.tell_hands(Hands::Watching);
            }
        }
    }

    /// Once a frame: the pointer goes its share of the way.
    pub fn glide_hands(&mut self) {
        let Some(running) = self.hands.as_mut() else {
            return;
        };
        let now = Instant::now();
        let elapsed = now - running.last_glide;
        running.last_glide = now;
        if running.pending == (0.0, 0.0) {
            return;
        }
        let step = glide_step(running.pending, running.glide_left, elapsed);
        running.pending = (running.pending.0 - step.0, running.pending.1 - step.1);
        running.glide_left = running.glide_left.saturating_sub(elapsed);
        if running.glide_left.is_zero() {
            running.pending = (0.0, 0.0);
        }
        self.move_by_hand(step);
    }

    fn move_by_hand(&mut self, step: (f64, f64)) {
        let elapsed = self.start_time.elapsed();
        self.input_activity();
        let delta = smithay::utils::Point::from(step);
        self.pointer_moved(delta, delta, elapsed.as_micros() as u64, elapsed.as_millis() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_reload_preserves_temporary_on_and_off_choices() {
        assert_eq!(restart_choice(false, Some(false), true, true), Some(true));
        assert_eq!(restart_choice(true, Some(true), false, true), None);
        assert_eq!(restart_choice(false, Some(false), true, false), None);
        assert_eq!(restart_choice(false, Some(true), true, true), Some(false));
        assert_eq!(restart_choice(true, Some(false), false, true), Some(true));
    }

    #[test]
    fn every_gesture_preference_is_passed_to_the_camera_process() {
        let input = des_common::input::InputSettings::parse("hand_speed = 0.4\nhand_pinch_sensitivity = 0.8\nhand_scroll_speed = -0.5\nhand_drag_lock = off");
        let settings = HandSettings::from(&input);
        assert_eq!(settings.args(), ["--serve", "--speed", "0.4", "--pinch-sensitivity", "0.8", "--scroll-speed", "-0.5", "--drag-lock", "off"]);
        for key in ["hand_speed", "hand_pinch_sensitivity", "hand_scroll_speed", "hand_drag_lock"] {
            let value = if key == "hand_drag_lock" { "off" } else { "0.7" };
            assert_ne!(settings, HandSettings::from(&des_common::input::InputSettings::parse(&format!("{key} = {value}"))));
        }
    }

    #[test]
    fn glides_its_share_and_then_the_rest() {
        let ms = Duration::from_millis;
        // A quarter of the time gone: a quarter of the way.
        assert_eq!(glide_step((100.0, -40.0), ms(40), ms(10)), (25.0, -10.0));
        // Late: all of it.
        assert_eq!(glide_step((100.0, 0.0), ms(10), ms(16)), (100.0, 0.0));
    }

    #[test]
    fn learns_how_often_the_camera_moves() {
        let ms = Duration::from_millis;
        // Ten pictures a second, from a guess of sixty milliseconds: towards a hundred.
        let mut interval = ms(60);
        for _ in 0..20 {
            interval = next_interval(interval, ms(100));
        }
        assert!(interval > ms(95) && interval <= ms(100), "{interval:?}");
        // A pause is not the camera slowing down; nor is anything past the bounds.
        assert_eq!(next_interval(ms(60), Duration::from_secs(2)), ms(60));
        assert_eq!(next_interval(ms(16), ms(1)), SHORTEST);
    }
}
