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
    CancelDrag,
    Cycle,
    Close,
    /// Move the current application to the next display.
    MoveDisplay,
    Launcher,
    Dock,
    Settings,
    Shortcuts,
    Companion,
    VoiceCompanion,
    CompanionControl(des_common::ipc::CompanionAction),
    Music,
    Notifications,
    /// Super+A: the quick settings, over whatever is in view.
    Quick,
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
    /// Super+H: hand control on or off.
    ToggleHands,
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

/// Match the exact chord first; Ctrl+Alt is the nested-session alias for Super.
fn desktop_shortcut(bindings: &des_common::shortcuts::Bindings, chord: &des_common::shortcuts::Chord) -> Option<Action> {
    use des_common::shortcuts::{Chord, Scope, DEFINITIONS};
    let mut candidates = vec![chord.clone()];
    if chord.ctrl && chord.alt && !chord.super_key {
        let mut alias = chord.clone(); alias.ctrl=false; alias.alt=false; alias.super_key=true;
        candidates.push(alias.clone());
        // Ctrl+Alt+Shift+arrows historically stands for Super+Alt+arrows.
        if nudges(false, chord.ctrl, chord.alt, chord.shift) && matches!(chord.key.as_str(), "Left"|"Right") { alias.shift=false; alias.alt=true; candidates.push(alias); }
    }
    let lock=Chord::parse("Super+L").unwrap();
    if candidates.contains(&lock) { return Some(Action::Lock) }
    let id=DEFINITIONS.iter().find(|d|d.scope==Scope::Desktop && candidates.iter().any(|c|bindings.matches(d.id,c)))?.id;
    Some(match id {
        "desktop.cycle" => Action::Cycle,
        "desktop.close" => Action::Close,
        "desktop.move_display" => Action::MoveDisplay,
        "desktop.launcher" => Action::Launcher,
        "desktop.dock" => Action::Dock,
        "desktop.settings" => Action::Settings,
        "desktop.music" => Action::Music,
        "desktop.notifications" => Action::Notifications,
        "desktop.quick" => Action::Quick,
        "desktop.power" => Action::Power,
        "desktop.split" => Action::Split,
        "desktop.cycle_layout" => Action::CycleLayout,
        "desktop.nudge_left" => Action::NudgeRatio(false),
        "desktop.nudge_right" => Action::NudgeRatio(true),
        "desktop.swap_master" => Action::SwapMaster,
        "desktop.fullscreen" => Action::Fullscreen,
        "desktop.pin" => Action::Pin,
        "desktop.focus_left" => Action::FocusHalf(false),
        "desktop.focus_right" => Action::FocusHalf(true),
        "desktop.panel_left" => Action::Panel(des_common::Panel::Left),
        "desktop.panel_right" => Action::Panel(des_common::Panel::Right),
        "desktop.panel_bottom" => Action::Panel(des_common::Panel::Bottom),
        "desktop.restart_shell" => Action::RestartShell,
        "desktop.hands" => Action::ToggleHands,
        "desktop.overview" => Action::ToggleOverview,
        "desktop.help" => Action::Shortcuts,
        "desktop.companion" => Action::Companion,
        "desktop.voice_companion" => Action::VoiceCompanion,
        "desktop.companion_keep" => Action::CompanionControl(des_common::ipc::CompanionAction::Keep),
        "desktop.companion_mute" => Action::CompanionControl(des_common::ipc::CompanionAction::Mute),
        "desktop.companion_stop" => Action::CompanionControl(des_common::ipc::CompanionAction::Stop),
        "desktop.companion_quiet" => Action::CompanionControl(des_common::ipc::CompanionAction::Quiet),
        "desktop.workspace_1" => Action::Workspace(0),
        "desktop.send_1" => Action::SendTo(0),
        "desktop.workspace_2" => Action::Workspace(1),
        "desktop.send_2" => Action::SendTo(1),
        "desktop.workspace_3" => Action::Workspace(2),
        "desktop.send_3" => Action::SendTo(2),
        "desktop.workspace_4" => Action::Workspace(3),
        "desktop.send_4" => Action::SendTo(3),
        _ => return None,
    })
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
        let cursor = crate::handlers::cursor_shape::Preferences::from(&self.input_settings);
        self.cursors.configure(cursor);
        // The worker deduplicates successful/in-flight preferences. Re-requesting
        // on reload also lets a transient cache failure recover after reconnect.
        self.cursor_theme_worker.request(cursor);
        self.shortcut_bindings = des_common::shortcuts::Bindings::load();
        self.drag_overview_enabled=crate::dnd::overview_wanted();
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
        self.apply_hand_settings();
    }

    /// The pointer moved by `delta`, from a mouse, a touchpad or a hand; `utime` is
    /// when, in microseconds, and `time` in milliseconds.
    pub fn pointer_moved(&mut self, delta: smithay::utils::Point<f64, smithay::utils::Logical>, delta_unaccel: smithay::utils::Point<f64, smithay::utils::Logical>, utime: u64, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let from = pointer.current_location();
        if self.drag_view_open() {
            let pos=self.clamp_to_screens(from,from+delta);
            self.drag_motion(pos,time);
            return;
        }
        // Over the overview or the switcher the pointer is theirs alone.
        if self.overview_open().is_some() {
            let pos = self.clamp_to_screens(from, from + delta);
            pointer.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
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
                delta,
                delta_unaccel,
                utime,
            },
        );
        let pos = match self.pointer_hold() {
            // Held in place: the pointer stays where the application put it.
            crate::handlers::input_protocols::Hold::Locked => {
                pointer.frame(self);
                return;
            }
            crate::handlers::input_protocols::Hold::Confined => {
                let to = self.clamp_to_screens(from, from + delta);
                if self.confined_allows(to) { to } else { from }
            }
            crate::handlers::input_protocols::Hold::Free => self.clamp_to_screens(from, from + delta),
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
                time,
            },
        );
        pointer.frame(self);
        self.activate_pointer_constraint();
    }

    /// A pointer button went down or up, on a mouse, a touchpad or by hand.
    pub fn pointer_button(&mut self, button: u32, button_state: ButtonState, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();

        // A drag out of the shell may only begin while a button is held.
        self.buttons_down = match button_state {
            ButtonState::Pressed => self.buttons_down.saturating_add(1),
            ButtonState::Released => self.buttons_down.saturating_sub(1),
        };

        if button_state == ButtonState::Pressed {
            self.super_tap.spoil();
        }
        if self.drag_view_open() {
            let at=pointer.current_location();
            pointer.motion(self,None,&MotionEvent{location:at,serial,time});
            pointer.button(self,&ButtonEvent{button,state:button_state,serial,time});
            pointer.frame(self);
            self.tick_drag_overview();
            return;
        }
        // The overview and the switcher take their own clicks.
        if self.overview_open().is_some() {
            self.overview_button(pointer.current_location(), button_state == ButtonState::Pressed);
            return;
        }

        // Click to focus: the keyboard follows the display that was clicked.
        // Shell overlays own their clicks; only their explicit commands change focus.
        let on_overlay = self.shell_overlay_contains(pointer.current_location());
        if button_state == ButtonState::Pressed {
            self.blur_companion_chat_on_press(pointer.is_grabbed(), on_overlay);
            self.dismiss_dock_keyboard_on_press(pointer.is_grabbed(), on_overlay);
        }
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
                time,
            },
        );
        pointer.frame(self);
    }

    /// Something was done by hand: the screens wake, and locking for idleness waits.
    pub fn input_activity(&mut self) {
        self.last_input = std::time::Instant::now();
        self.idle_lock_asked = false;
        if self.screens_off {
            self.blank(false);
        }
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        self.input_activity();
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
                        if state.drag_session.is_some() && sym==Keysym::Escape {return FilterResult::Intercept(Action::CancelDrag);}
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
                            if matches!(open,Some(OverviewKind::Workspaces | OverviewKind::Windows | OverviewKind::Drag)) && !held {
                                if open==Some(OverviewKind::Drag) {return FilterResult::Intercept(Action::Ignore);}
                                use crate::overview::Direction;
                                return FilterResult::Intercept(Action::Overview(match sym {
                                    Keysym::Escape => OverviewKey::Close,
                                    Keysym::Return | Keysym::KP_Enter => OverviewKey::Choose,
                                    Keysym::Left => OverviewKey::Move(Direction::Left),
                                    Keysym::Right => OverviewKey::Move(Direction::Right),
                                    Keysym::Up => OverviewKey::Move(Direction::Up),
                                    Keysym::Down => OverviewKey::Move(Direction::Down),
                                    Keysym::BackSpace => OverviewKey::Erase,
                                    Keysym::Page_Down => OverviewKey::Page(true),
                                    Keysym::Page_Up => OverviewKey::Page(false),
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
                        if locked { return FilterResult::Forward; }
                        // Recovery and lock remain available regardless of user mappings.
                        if modifiers.ctrl && modifiers.alt && sym == Keysym::BackSpace {
                            return FilterResult::Intercept(Action::Quit);
                        }
                        let raw = handle.raw_syms().first().copied().unwrap_or(sym);
                        let key = match raw {
                            Keysym::Return | Keysym::KP_Enter => "Enter".to_string(),
                            Keysym::Escape => "Escape".to_string(),
                            Keysym::Tab | Keysym::ISO_Left_Tab => "Tab".to_string(),
                            Keysym::BackSpace => "Backspace".to_string(),
                            Keysym::Home => "Home".to_string(), Keysym::End => "End".to_string(),
                            Keysym::Insert => "Insert".to_string(), Keysym::Delete => "Delete".to_string(),
                            Keysym::Page_Up => "PageUp".to_string(), Keysym::Page_Down => "PageDown".to_string(),
                            Keysym::Left => "Left".to_string(), Keysym::Right => "Right".to_string(),
                            Keysym::Up => "Up".to_string(), Keysym::Down => "Down".to_string(),
                            Keysym::space => "Space".to_string(),
                            other if (0xffbe..=0xffd5).contains(&other.raw()) => format!("F{}", other.raw()-0xffbe+1),
                            other => match other.key_char() { Some(c) => c.to_string(), None => return FilterResult::Forward },
                        };
                        let Ok(mut chord) = des_common::shortcuts::Chord::parse(&key) else { return FilterResult::Forward };
                        chord.ctrl = modifiers.ctrl; chord.alt = modifiers.alt;
                        chord.shift = modifiers.shift; chord.super_key = modifiers.logo;
                        if let Some(action) = desktop_shortcut(&state.shortcut_bindings, &chord) {
                            return FilterResult::Intercept(action);
                        }
                        if modifiers.logo && sym.key_char().is_some_and(|c| !c.is_control()) {
                            FilterResult::Intercept(Action::Ignore)
                        } else { FilterResult::Forward }

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
                if self.drag_session.is_some() && action.as_ref().is_some_and(|action|!matches!(action,Action::Ignore|Action::Media(_)|Action::CancelDrag)) {
                    self.cancel_file_drag();
                }
                // A compositor shortcut lets go of the pointer, so a game cannot keep it.
                if action.is_some() {
                    self.release_pointer_constraint();
                }
                match action {
                    Some(Action::CancelDrag) => self.cancel_file_drag(),
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
                    Some(Action::VoiceCompanion) => {
                        if let Some(line) = self.companion_pointer_event() { self.tell(line); }
                        self.tell(des_common::ipc::Event::VOICE_COMPANION.into());
                    }
                    Some(Action::Companion) => self.tell(des_common::ipc::Event::COMPANION.into()),
                    Some(Action::CompanionControl(action)) => self.tell(action.encode().into()),
                    Some(Action::Shortcuts) => self.tell(des_common::ipc::Event::SHORTCUTS.into()),
                    Some(Action::Music) => self.open_music(),
                    Some(Action::Notifications) => self.ask_notifications(),
                    Some(Action::Quick) => self.ask_quick(),
                    // Behind the lock there is no menu to show: the key puts the machine to sleep.
                    // Nothing behind the lock is to be pictured.
                    Some(Action::Shot(_)) if self.locked => {}
                    Some(Action::Shot(mode)) => self.take_shot(mode),
                    Some(Action::Power) if self.locked => run("systemctl", &["suspend"]),
                    Some(Action::Power) => self.open_power(),
                    Some(Action::Lock) => self.request_lock(),
                    Some(Action::Ignore) => {}
                    Some(Action::RestartShell) => self.restart_shell(),
                    Some(Action::ToggleHands) => self.toggle_hands(),
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
                self.pointer_moved(event.delta(), event.delta_unaccel(), event.time(), event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                if self.drag_view_open() {self.drag_motion(pos,event.time_msec());return;}
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
                self.pointer_button(event.button_code(), event.state(), event.time_msec());
            }
            // Nothing behind the overview scrolls.
            InputEvent::PointerAxis { event, .. } if self.overview_open().is_some() => {
                if self.overview_open()==Some(OverviewKind::Windows) {
                    let amount=event.amount_v120(Axis::Vertical).unwrap_or_else(||event.amount(Axis::Vertical).unwrap_or(0.0));
                    if amount!=0.0 {self.overview_key(OverviewKey::Page(amount>0.0));}
                }
            }
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

#[cfg(test)]
mod shortcut_routing_tests {
    use super::*;
    use des_common::shortcuts::{Bindings,Chord};
    #[test] fn remapped_and_disabled_default_never_fire() {
        let mut b=Bindings::default();b.set("desktop.help","Super+Shift+Slash").unwrap();
        assert!(desktop_shortcut(&b,&Chord::parse("Super+Slash").unwrap()).is_none());
        assert!(matches!(desktop_shortcut(&b,&Chord::parse("Super+Shift+Slash").unwrap()),Some(Action::Shortcuts)));
        b.set("desktop.help","").unwrap();assert!(desktop_shortcut(&b,&Chord::parse("Super+Shift+Slash").unwrap()).is_none());
    }
    #[test] fn exact_modifiers_and_nested_alias() {
        let b=Bindings::default();assert!(desktop_shortcut(&b,&Chord::parse("Super+Ctrl+D").unwrap()).is_none());
        assert!(matches!(desktop_shortcut(&b,&Chord::parse("Ctrl+Alt+D").unwrap()),Some(Action::Dock)));
        assert!(matches!(desktop_shortcut(&b,&Chord::parse("Super+Shift+1").unwrap()),Some(Action::SendTo(0))));
        assert!(matches!(desktop_shortcut(&b,&Chord::parse("Super+L").unwrap()),Some(Action::Lock)));
    }
}
