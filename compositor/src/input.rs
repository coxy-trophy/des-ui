use smithay::{
    backend::session::Session,
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
        GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer as gesture,
    input::{
        keyboard::{FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    utils::SERIAL_COUNTER,
};

use crate::{
    overview::{OverviewKey, SwitchKey},
    state::DesComp,
};
use des_common::ipc::OverviewKind;

/// The left mouse button, as the kernel numbers it.
const BTN_LEFT: u32 = 0x110;

/// A compositor shortcut, as opposed to a key press that belongs to the focused client.
#[derive(Debug)]
enum Action {
    Cycle,
    Close,
    /// Move the current application to the next display.
    MoveDisplay,
    Launcher,
    Dock,
    Settings,
    Music,
    Notifications,
    /// The power key, or its shortcut.
    Power,
    Shot(des_common::ipc::ShotMode),
    /// Bring a workspace into view.
    Workspace(usize),
    /// Send the application in use to a workspace.
    SendTo(usize),
    Lock,
    /// Super with a key that is no shortcut: swallowed, so that it types nothing.
    Ignore,
    Split,
    /// Super+Shift+S: the next layout round.
    CycleLayout,
    /// Super+Alt+Left (false) or Right (true): move the divider beside the window in use.
    NudgeRatio(bool),
    /// Super+Enter: the window in use becomes the master.
    SwapMaster,
    Fullscreen,
    /// Pin the application in use in a corner above everything, or put it back.
    Pin,
    /// Give the keyboard to the left (false) or right (true) half of a split.
    FocusHalf(bool),
    Panel(des_common::Panel),
    RestartShell,
    /// A volume or brightness key.
    Media(Media),
    /// Switch to another virtual terminal (Ctrl+Alt+F1..F12).
    SwitchVt(i32),
    /// End the session (Ctrl+Alt+Backspace).
    Quit,
    /// A key for the open overview.
    Overview(OverviewKey),
    /// A key for the open window switcher.
    Switch(SwitchKey),
    /// Open the overview, or close it (Super+W).
    ToggleOverview,
    /// Alt+Tab, Alt+Shift+Tab, and with ` the windows of the application in use.
    OpenSwitcher { forward: bool, same_app: bool },
}

/// Whether a shortcut takes the user somewhere the overview would hide: anything but
/// moving between workspaces, which the overview shows, and keys that act in place.
fn closes_overview(action: &Action) -> bool {
    !matches!(
        action,
        Action::Overview(_)
            | Action::Switch(_)
            | Action::ToggleOverview
            | Action::Workspace(_)
            | Action::SendTo(_)
            | Action::Media(_)
            | Action::SwitchVt(_)
            | Action::Ignore
    )
}

#[derive(Clone, Copy, Debug)]
enum Media {
    VolumeUp,
    VolumeDown,
    Mute,
    BrightnessUp,
    BrightnessDown,
    PlayPause,
    NextTrack,
    PreviousTrack,
}

/// Passes a playback key on to the music service, which is installed beside this
/// program. Nothing happens when the service is not running.
fn music(command: &str) {
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("des-music")));
    let program = beside.filter(|path| path.exists()).unwrap_or_else(|| "des-music".into());
    run(&program.to_string_lossy(), &["send", command]);
}

/// Runs a helper program without waiting for it.
fn run(program: &str, args: &[&str]) {
    match std::process::Command::new(program).args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!("cannot run {program}: {e}"),
    }
}

/// Steps the backlight by a tenth of its range, through the login service, which lets
/// the active session do that without special permissions.
fn step_brightness(up: bool) {
    let Some(device) = std::fs::read_dir("/sys/class/backlight")
        .ok()
        .and_then(|mut dir| dir.next()?.ok())
    else {
        return;
    };
    let read = |file: &str| -> Option<i64> {
        std::fs::read_to_string(device.path().join(file)).ok()?.trim().parse().ok()
    };
    let (Some(current), Some(max)) = (read("brightness"), read("max_brightness")) else {
        return;
    };
    let step = (max / 10).max(1);
    // Never all the way to zero: on many panels that is a black screen.
    let target = if up { current + step } else { current - step }.clamp(step / 2, max);
    run(
        "busctl",
        &[
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            "ssu",
            "backlight",
            &device.file_name().to_string_lossy(),
            &target.to_string(),
        ],
    );
}

impl Media {
    fn from_keysym(sym: Keysym) -> Option<Media> {
        Some(match sym {
            Keysym::XF86_AudioRaiseVolume => Media::VolumeUp,
            Keysym::XF86_AudioLowerVolume => Media::VolumeDown,
            Keysym::XF86_AudioMute => Media::Mute,
            Keysym::XF86_MonBrightnessUp => Media::BrightnessUp,
            Keysym::XF86_MonBrightnessDown => Media::BrightnessDown,
            Keysym::XF86_AudioPlay | Keysym::XF86_AudioPause => Media::PlayPause,
            Keysym::XF86_AudioNext => Media::NextTrack,
            Keysym::XF86_AudioPrev => Media::PreviousTrack,
            _ => return None,
        })
    }

    fn apply(self) {
        const SINK: &str = "@DEFAULT_AUDIO_SINK@";
        match self {
            Media::VolumeUp => run("wpctl", &["set-volume", "-l", "1.0", SINK, "5%+"]),
            Media::VolumeDown => run("wpctl", &["set-volume", SINK, "5%-"]),
            Media::Mute => run("wpctl", &["set-mute", SINK, "toggle"]),
            Media::BrightnessUp => step_brightness(true),
            Media::BrightnessDown => step_brightness(false),
            Media::PlayPause => music("toggle"),
            Media::NextTrack => music("next"),
            Media::PreviousTrack => music("prev"),
        }
    }
}

/// Whether a shortcut is passed to an application that asked for the compositor's
/// keys, such as a remote desktop or a virtual machine. Locking, ending the session,
/// changing virtual terminal, the power key and the machine's own volume and
/// brightness keys always stay with the compositor, so no window can trap the user.
fn inhibitable(action: &Action) -> bool {
    !matches!(action, Action::Lock | Action::Quit | Action::SwitchVt(_) | Action::Power | Action::Media(_))
}

/// Whether an arrow with these modifiers moves a divider rather than the keyboard:
/// Super+Alt, or Ctrl+Alt+Shift where Ctrl+Alt stands in for Super.
fn nudges(logo: bool, ctrl: bool, alt: bool, shift: bool) -> bool {
    (logo && alt) || (!logo && ctrl && alt && shift)
}

/// The keysyms that Ctrl+Alt+F1..F12 produce.
const SWITCH_VT_1: u32 = 0x1008_FE01;
const SWITCH_VT_12: u32 = 0x1008_FE0C;

impl DesComp {
    /// Reads the input settings again and applies them to the keyboard and to every
    /// pointing device.
    pub fn reload_input(&mut self) {
        self.input_settings = des_common::input::InputSettings::load();
        self.motions.enabled = crate::motion::wanted();
        self.phosphor.reload();
        let settings = self.input_settings.clone();
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.change_repeat_info(settings.repeat_rate as i32, settings.repeat_delay as i32);

        // Compiling a keymap is not free, and a bad one must not replace a good one.
        let wanted = (settings.keyboard_layout.clone(), settings.keyboard_variant.clone());
        let unset = wanted.0.is_empty() && self.keymap_applied.is_none();
        if !unset && self.keymap_applied.as_ref() != Some(&wanted) {
            let config = smithay::input::keyboard::XkbConfig {
                layout: &wanted.0,
                variant: &wanted.1,
                ..Default::default()
            };
            match keyboard.set_xkb_config(self, config) {
                Ok(()) => {
                    tracing::info!(layout = %wanted.0, variant = %wanted.1, "keyboard layout set");
                    self.keymap_applied = Some(wanted);
                }
                Err(e) => tracing::warn!(layout = %wanted.0, "cannot use this keyboard layout: {e:?}"),
            }
        }
        self.configure_devices();
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        self.last_input = std::time::Instant::now();
        self.idle_lock_asked = false;
        if self.screens_off {
            self.blank(false);
        }
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                let pressed = event.state() == KeyState::Pressed;
                let locked = self.locked;
                let inhibited = self.shortcuts_inhibited();
                let action = self.seat.get_keyboard().unwrap().input(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |state, modifiers, handle| {
                        // Super pressed and let go on its own opens the overview. The
                        // key itself still goes where it was going.
                        let now = std::time::Instant::now();
                        let super_key = matches!(handle.modified_sym(), Keysym::Super_L | Keysym::Super_R);
                        if pressed && super_key {
                            state.super_tap.press(now);
                        } else if pressed {
                            state.super_tap.spoil();
                        } else if super_key && state.super_tap.release(now) && !locked && !inhibited {
                            state.overview_wanted = true;
                        }
                        let open = state.overview_open();
                        // Letting go of Alt chooses the window the switcher is on.
                        if !pressed && open == Some(OverviewKind::Switcher) && !modifiers.alt {
                            state.switcher_released = true;
                        }
                        let decide = || {
                        // Super is the real shortcut key. Ctrl+Alt also works, because a
                        // host desktop keeps Super for itself when we run in a window.
                        let held = modifiers.logo || (modifiers.ctrl && modifiers.alt);
                        if !pressed {
                            return FilterResult::Forward;
                        }
                        let sym = handle.modified_sym();
                        if (SWITCH_VT_1..=SWITCH_VT_12).contains(&sym.raw()) {
                            let vt = (sym.raw() - SWITCH_VT_1 + 1) as i32;
                            return FilterResult::Intercept(Action::SwitchVt(vt));
                        }
                        if let Some(media) = Media::from_keysym(sym) {
                            return FilterResult::Intercept(Action::Media(media));
                        }
                        // Print asks what to take; with Alt it is the window at once,
                        // with Shift a piece.
                        if sym == Keysym::Print || sym == Keysym::Sys_Req {
                            use des_common::ipc::ShotMode;
                            let mode = if modifiers.shift {
                                ShotMode::Region
                            } else if modifiers.alt {
                                ShotMode::Window
                            } else {
                                ShotMode::Pick
                            };
                            return FilterResult::Intercept(Action::Shot(mode));
                        }
                        if sym == Keysym::XF86_PowerOff {
                            return FilterResult::Intercept(Action::Power);
                        }
                        if !locked {
                            // The switcher has every key while Alt is held.
                            if open == Some(OverviewKind::Switcher) {
                                return FilterResult::Intercept(Action::Switch(match sym {
                                    Keysym::Tab if modifiers.shift => SwitchKey::Previous,
                                    Keysym::Tab | Keysym::grave | Keysym::Right => SwitchKey::Next,
                                    Keysym::ISO_Left_Tab | Keysym::asciitilde | Keysym::Left => SwitchKey::Previous,
                                    Keysym::Return | Keysym::KP_Enter => SwitchKey::Choose,
                                    Keysym::Escape => SwitchKey::Cancel,
                                    _ => SwitchKey::Ignore,
                                }));
                            }
                            // The overview has every key but the shortcuts, so that
                            // locking, workspaces and the rest still work over it.
                            if open == Some(OverviewKind::Workspaces) && !held {
                                use crate::overview::Direction;
                                return FilterResult::Intercept(Action::Overview(match sym {
                                    Keysym::Escape => OverviewKey::Close,
                                    Keysym::Return | Keysym::KP_Enter => OverviewKey::Choose,
                                    Keysym::Left => OverviewKey::Move(Direction::Left),
                                    Keysym::Right => OverviewKey::Move(Direction::Right),
                                    Keysym::Up => OverviewKey::Move(Direction::Up),
                                    Keysym::Down => OverviewKey::Move(Direction::Down),
                                    Keysym::BackSpace => OverviewKey::Erase,
                                    other => match other.key_char().filter(|c| !c.is_control()) {
                                        Some(c) if !modifiers.ctrl => OverviewKey::Type(c),
                                        _ => OverviewKey::Ignore,
                                    },
                                }));
                            }
                            // Alt+Tab, and Alt+` among the windows of the application in use.
                            if modifiers.alt && !modifiers.ctrl && !modifiers.logo {
                                let opening = match sym {
                                    Keysym::Tab => Some((!modifiers.shift, false)),
                                    Keysym::ISO_Left_Tab => Some((false, false)),
                                    Keysym::grave => Some((true, true)),
                                    Keysym::asciitilde => Some((false, true)),
                                    _ => None,
                                };
                                if let Some((forward, same_app)) = opening {
                                    return FilterResult::Intercept(Action::OpenSwitcher { forward, same_app });
                                }
                            }
                        }
                        // Locked, every other key goes to the lock screen: no shortcut
                        // reaches what is behind it, and the session cannot be ended.
                        if !held || locked {
                            return FilterResult::Forward;
                        }
                        // The digits pick a workspace; with Shift they send the application
                        // in use there. Shift turns a digit into some other character, so
                        // the key is read as it is without it.
                        let digit = handle.raw_syms().first().map(|sym| sym.raw()).filter(|raw| (0x31..=0x39).contains(raw));
                        if let Some(workspace) = digit.map(|raw| (raw - 0x31) as usize).filter(|workspace| *workspace < des_common::ipc::WORKSPACES) {
                            return FilterResult::Intercept(if modifiers.shift { Action::SendTo(workspace) } else { Action::Workspace(workspace) });
                        }
                        match sym {
                            Keysym::BackSpace if modifiers.ctrl && modifiers.alt => {
                                FilterResult::Intercept(Action::Quit)
                            }
                            Keysym::Tab => FilterResult::Intercept(Action::Cycle),
                            Keysym::d | Keysym::D => FilterResult::Intercept(Action::Dock),
                            Keysym::space => FilterResult::Intercept(Action::Launcher),
                            Keysym::comma => FilterResult::Intercept(Action::Settings),
                            Keysym::m | Keysym::M => FilterResult::Intercept(Action::Music),
                            Keysym::n | Keysym::N => FilterResult::Intercept(Action::Notifications),
                            Keysym::Escape => FilterResult::Intercept(Action::Power),
                            Keysym::l | Keysym::L => FilterResult::Intercept(Action::Lock),
                            Keysym::o | Keysym::O => FilterResult::Intercept(Action::MoveDisplay),
                            Keysym::R if modifiers.shift => FilterResult::Intercept(Action::RestartShell),
                            Keysym::s | Keysym::S if modifiers.shift => FilterResult::Intercept(Action::CycleLayout),
                            Keysym::s | Keysym::S => FilterResult::Intercept(Action::Split),
                            Keysym::Return | Keysym::KP_Enter => FilterResult::Intercept(Action::SwapMaster),
                            Keysym::f | Keysym::F => FilterResult::Intercept(Action::Fullscreen),
                            // With Alt as well the divider moves; under Ctrl+Alt in a window,
                            // Shift stands in for the Alt that is already held.
                            Keysym::Left | Keysym::Right if nudges(modifiers.logo, modifiers.ctrl, modifiers.alt, modifiers.shift) => {
                                FilterResult::Intercept(Action::NudgeRatio(sym == Keysym::Right))
                            }
                            Keysym::Left => FilterResult::Intercept(Action::FocusHalf(false)),
                            Keysym::Right => FilterResult::Intercept(Action::FocusHalf(true)),
                            Keysym::bracketleft => {
                                FilterResult::Intercept(Action::Panel(des_common::Panel::Left))
                            }
                            Keysym::bracketright => {
                                FilterResult::Intercept(Action::Panel(des_common::Panel::Right))
                            }
                            Keysym::backslash => {
                                FilterResult::Intercept(Action::Panel(des_common::Panel::Bottom))
                            }
                            Keysym::q | Keysym::Q => FilterResult::Intercept(Action::Close),
                            Keysym::p | Keysym::P => FilterResult::Intercept(Action::Pin),
                            Keysym::w | Keysym::W => FilterResult::Intercept(Action::ToggleOverview),
                            // Not a shortcut, but not a letter for the application either.
                            other if modifiers.logo && other.key_char().is_some_and(|c| !c.is_control()) => {
                                FilterResult::Intercept(Action::Ignore)
                            }
                            _ => FilterResult::Forward,
                        }
                        };
                        match decide() {
                            // An open overview or switcher is the desktop's own, whatever
                            // an application asked for.
                            FilterResult::Intercept(action) if inhibited && open.is_none() && inhibitable(&action) => FilterResult::Forward,
                            result => result,
                        }
                    },
                );
                if std::mem::take(&mut self.overview_wanted) {
                    self.toggle_overview();
                }
                if std::mem::take(&mut self.switcher_released) {
                    self.switcher_key(SwitchKey::Choose);
                }
                // A shortcut that leads elsewhere closes the overview first; moving
                // between workspaces is seen in it.
                if self.overview_open() == Some(OverviewKind::Workspaces) && action.as_ref().is_some_and(closes_overview) {
                    self.close_overview_now();
                }
                // A compositor shortcut lets go of the pointer, so a game cannot keep it.
                if action.is_some() {
                    self.release_pointer_constraint();
                }
                match action {
                    Some(Action::Overview(key)) => self.overview_key(key),
                    Some(Action::Switch(key)) => self.switcher_key(key),
                    Some(Action::ToggleOverview) => self.toggle_overview(),
                    Some(Action::OpenSwitcher { forward, same_app }) => self.open_switcher(forward, same_app),
                    Some(Action::Cycle) => self.cycle(),
                    Some(Action::Workspace(workspace)) => self.switch_workspace(workspace),
                    Some(Action::SendTo(workspace)) => self.send_active_to(workspace),
                    Some(Action::Close) => self.close_active(),
                    Some(Action::MoveDisplay) => self.move_active(),
                    Some(Action::Dock) if self.picking => {},
                    Some(Action::Dock) => {
                        let pointer=self.seat.get_pointer().unwrap().current_location();
                        self.dock_keyboard_screen=self.screen_at(pointer);
                        self.dock_keyboard = true;
                        self.arrange();
                        self.tell(des_common::ipc::Event::dock_on(self.dock_keyboard_screen));
                    }
                    Some(Action::Launcher) => self.open_launcher(),
                    Some(Action::Settings) => self.open_settings(),
                    Some(Action::Music) => self.open_music(),
                    Some(Action::Notifications) => self.ask_notifications(),
                    // Behind the lock there is no menu to show: the key puts the machine to sleep.
                    // Nothing behind the lock is to be pictured.
                    Some(Action::Shot(_)) if self.locked => {}
                    Some(Action::Shot(mode)) => self.take_shot(mode),
                    Some(Action::Power) if self.locked => run("systemctl", &["suspend"]),
                    Some(Action::Power) => self.open_power(),
                    Some(Action::Lock) => self.request_lock(),
                    Some(Action::Ignore) => {}
                    Some(Action::RestartShell) => self.restart_shell(),
                    Some(Action::Split) => self.toggle_split(),
                    Some(Action::CycleLayout) => self.cycle_layout(),
                    Some(Action::NudgeRatio(right)) => self.nudge_ratio(right),
                    Some(Action::SwapMaster) => self.swap_master(),
                    Some(Action::Fullscreen) => self.toggle_fullscreen(),
                    Some(Action::Pin) => self.toggle_pin(),
                    Some(Action::FocusHalf(right)) => self.focus_half(right),
                    Some(Action::Panel(panel)) => self.toggle_panel(panel),
                    Some(Action::Media(media)) => media.apply(),
                    Some(Action::SwitchVt(vt)) => {
                        if let Some(udev) = self.udev.as_mut() {
                            if let Err(e) = udev.session.change_vt(vt) {
                                tracing::error!("cannot switch to VT {vt}: {e}");
                            }
                        }
                    }
                    Some(Action::Quit) => self.loop_signal.stop(),
                    None => {}
                }
            }
            // Mice and touchpads report movement; the pointer position is ours to keep.
            InputEvent::PointerMotion { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let from = pointer.current_location();
                // Over the overview or the switcher the pointer is theirs alone.
                if self.overview_open().is_some() {
                    let pos = self.clamp_to_screens(from, from + event.delta());
                    pointer.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time: event.time_msec() });
                    pointer.frame(self);
                    self.overview_pointer(pos);
                    return;
                }
                // Raw movement first, for games and 3D views, wherever the pointer is held.
                let under_from = self.surface_under(from);
                pointer.relative_motion(
                    self,
                    under_from,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );
                let pos = match self.pointer_hold() {
                    // Held in place: the pointer stays where the application put it.
                    crate::handlers::input_protocols::Hold::Locked => {
                        pointer.frame(self);
                        return;
                    }
                    crate::handlers::input_protocols::Hold::Confined => {
                        let to = self.clamp_to_screens(from, from + event.delta());
                        if self.confined_allows(to) { to } else { from }
                    }
                    crate::handlers::input_protocols::Hold::Free => self.clamp_to_screens(from, from + event.delta()),
                };

                // A divider in hand follows the pointer; over one, the pointer says so.
                if self.resize.is_some() {
                    self.drag_resize(pos.x);
                }
                self.resize_hover = self.resize.is_some() || self.divider_under(pos).is_some();

                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                self.activate_pointer_constraint();
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                if self.resize.is_some() {
                    self.drag_resize(pos.x);
                }
                self.resize_hover = self.resize.is_some() || self.divider_under(pos).is_some();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                let under = if self.overview_open().is_some() { None } else { self.surface_under(pos) };
                self.overview_pointer(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();
                // A drag out of the shell may only begin while a button is held.
                self.buttons_down = match button_state {
                    ButtonState::Pressed => self.buttons_down.saturating_add(1),
                    ButtonState::Released => self.buttons_down.saturating_sub(1),
                };

                if button_state == ButtonState::Pressed {
                    self.super_tap.spoil();
                }
                // The overview and the switcher take their own clicks.
                if self.overview_open().is_some() {
                    self.overview_button(pointer.current_location(), button_state == ButtonState::Pressed);
                    return;
                }

                // Click to focus: the keyboard follows the display that was clicked.
                // Shell overlays own their clicks; only their explicit commands change focus.
                let on_overlay = self.shell_overlay_contains(pointer.current_location());
                // The left button on a divider between windows drags it; neither the
                // press nor its release reaches the windows either side.
                if button == BTN_LEFT {
                    if ButtonState::Pressed == button_state && !pointer.is_grabbed() && !on_overlay && self.start_resize(pointer.current_location()) {
                        return;
                    }
                    if ButtonState::Released == button_state && self.resize.is_some() {
                        self.end_resize();
                        self.resize_hover = self.divider_under(pointer.current_location()).is_some();
                        return;
                    }
                }
                if ButtonState::Pressed == button_state && !pointer.is_grabbed() && !on_overlay {
                    self.focus_at(pointer.current_location());
                };

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            // Nothing behind the overview scrolls.
            InputEvent::PointerAxis { .. } if self.overview_open().is_some() => {}
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            // Three or four fingers swiping are the desktop's; anything else is the
            // application's under the pointer.
            InputEvent::GestureSwipeBegin { event, .. } => {
                let fingers = event.fingers();
                if crate::gestures::ours(fingers) && !self.locked && !self.picking {
                    self.swipe = Some(crate::gestures::Swipe::default());
                } else {
                    self.swipe = None;
                    let pointer = self.seat.get_pointer().unwrap();
                    let serial = SERIAL_COUNTER.next_serial();
                    pointer.gesture_swipe_begin(self, &gesture::GestureSwipeBeginEvent { serial, time: event.time_msec(), fingers });
                }
            }
            InputEvent::GestureSwipeUpdate { event, .. } => match self.swipe.as_mut() {
                Some(swipe) => swipe.update(event.delta_x(), event.delta_y()),
                None => {
                    let pointer = self.seat.get_pointer().unwrap();
                    pointer.gesture_swipe_update(self, &gesture::GestureSwipeUpdateEvent { time: event.time_msec(), delta: event.delta() });
                }
            },
            InputEvent::GestureSwipeEnd { event, .. } => match self.swipe.take() {
                Some(swipe) => {
                    let open = self.overview_open() == Some(OverviewKind::Workspaces);
                    match swipe.outcome(event.cancelled(), open) {
                        Some(crate::gestures::Outcome::Previous) if self.workspace > 0 => self.switch_workspace(self.workspace - 1),
                        Some(crate::gestures::Outcome::Next) => self.switch_workspace(self.workspace + 1),
                        Some(crate::gestures::Outcome::OpenOverview) if self.overview.is_none() => self.toggle_overview(),
                        Some(crate::gestures::Outcome::CloseOverview) => self.close_overview(),
                        _ => {}
                    }
                }
                None => {
                    let pointer = self.seat.get_pointer().unwrap();
                    let serial = SERIAL_COUNTER.next_serial();
                    pointer.gesture_swipe_end(self, &gesture::GestureSwipeEndEvent { serial, time: event.time_msec(), cancelled: event.cancelled() });
                }
            },
            InputEvent::GesturePinchBegin { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.gesture_pinch_begin(self, &gesture::GesturePinchBeginEvent { serial, time: event.time_msec(), fingers: event.fingers() });
            }
            InputEvent::GesturePinchUpdate { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                pointer.gesture_pinch_update(
                    self,
                    &gesture::GesturePinchUpdateEvent { time: event.time_msec(), delta: event.delta(), scale: event.scale(), rotation: event.rotation() },
                );
            }
            InputEvent::GesturePinchEnd { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.gesture_pinch_end(self, &gesture::GesturePinchEndEvent { serial, time: event.time_msec(), cancelled: event.cancelled() });
            }
            InputEvent::GestureHoldBegin { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.gesture_hold_begin(self, &gesture::GestureHoldBeginEvent { serial, time: event.time_msec(), fingers: event.fingers() });
            }
            InputEvent::GestureHoldEnd { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.gesture_hold_end(self, &gesture::GestureHoldEndEvent { serial, time: event.time_msec(), cancelled: event.cancelled() });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_with_super_moves_the_divider_and_super_alone_the_keyboard() {
        assert!(nudges(true, false, true, false));
        assert!(!nudges(true, false, false, false));
        // Ctrl+Alt is the shortcut key in a window; Shift adds the divider.
        assert!(!nudges(false, true, true, false));
        assert!(nudges(false, true, true, true));
        for layout in [Action::CycleLayout, Action::NudgeRatio(true), Action::SwapMaster] {
            assert!(inhibitable(&layout), "{layout:?} goes to an application that asked");
        }
    }

    #[test]
    fn the_safety_keys_cannot_be_taken_by_an_application() {
        for kept in [Action::Lock, Action::Quit, Action::SwitchVt(2), Action::Power, Action::Media(Media::Mute)] {
            assert!(!inhibitable(&kept), "{kept:?} must stay with the compositor");
        }
        for passed in [Action::Cycle, Action::Launcher, Action::Close, Action::Workspace(1), Action::Ignore, Action::Split] {
            assert!(inhibitable(&passed), "{passed:?} goes to the application that asked");
        }
    }
}
