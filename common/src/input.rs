//! Settings for keyboards, mice, touchpads and hand control. The shell's settings screen writes them
//! to the configuration file; the compositor, which owns the devices, reads them there.

use std::path::PathBuf;

/// Where the settings live: `key = value` lines, shared by the shell and the compositor.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("des-ui/config"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputSettings {
    /// Content follows the fingers, as on a phone.
    pub touchpad_natural_scroll: bool,
    pub touchpad_tap: bool,
    /// Ignore the touchpad while keys are being pressed.
    pub touchpad_while_typing_off: bool,
    /// Pointer speed, from -1 (slowest) to 1 (fastest).
    pub touchpad_speed: f32,
    pub mouse_natural_scroll: bool,
    pub mouse_speed: f32,
    pub mouse_left_handed: bool,
    /// Logical pixels used by the desktop pointer.
    pub cursor_size: u32,
    /// Opaque RGB fill for the desktop pointer.
    pub cursor_color: [u8; 3],
    /// An XKB layout such as `us` or `de`; empty for the system's default.
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    /// Milliseconds a key is held before it starts repeating.
    pub repeat_delay: u32,
    /// Repeats per second.
    pub repeat_rate: u32,
    /// Minutes without input after which the screen locks; `None` for never.
    pub lock_after: Option<u32>,
    /// Minutes without input after which the screens are turned off; `None` for never.
    pub screen_off_after: Option<u32>,
    /// The pointer is moved by a hand in front of the webcam, from the start of the
    /// session; Super+H turns it on or off for the while.
    pub hand_control: bool,
    /// How far the pointer goes for the hand's movement, from -1 to 1.
    pub hand_speed: f32,
    /// Pinch forgiveness, from -1 (firm) to 1 (gentle).
    pub hand_pinch_sensitivity: f32,
    /// Scrolling speed, from -1 (slowest) to 1 (fastest).
    pub hand_scroll_speed: f32,
    /// A second pinch and movement grabs; another pinch drops without holding.
    pub hand_drag_lock: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        InputSettings {
            touchpad_natural_scroll: true,
            touchpad_tap: true,
            touchpad_while_typing_off: true,
            touchpad_speed: 0.0,
            mouse_natural_scroll: false,
            mouse_speed: 0.0,
            mouse_left_handed: false,
            cursor_size: CURSOR_BASE_SIZE,
            cursor_color: DEFAULT_CURSOR_COLOR,
            keyboard_layout: String::new(),
            keyboard_variant: String::new(),
            repeat_delay: 250,
            repeat_rate: 30,
            lock_after: Some(10),
            screen_off_after: None,
            hand_control: false,
            hand_speed: 0.0,
            hand_pinch_sensitivity: 0.0,
            hand_scroll_speed: 0.0,
            hand_drag_lock: true,
        }
    }
}

pub const CURSOR_BASE_SIZE: u32 = 24;
/// The solid arrow in base-size logical pixels, relative to the pointer tip.
/// Shared by the compositor renderer and Settings preview.
pub const CURSOR_ARROW_POINTS: [[f32; 2]; 3] = [[0.0, 0.0], [15.0, 6.0], [6.0, 16.0]];
pub const CURSOR_SIZE_RANGE: (u32, u32) = (16, 64);
pub const DEFAULT_CURSOR_COLOR: [u8; 3] = [12, 16, 21];

pub const REPEAT_DELAY_RANGE: (u32, u32) = (150, 800);
pub const REPEAT_RATE_RANGE: (u32, u32) = (10, 60);

impl InputSettings {
    /// Reads the settings from the configuration file's text. Anything missing or
    /// malformed keeps its default.
    pub fn parse(text: &str) -> InputSettings {
        let mut settings = InputSettings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split('#').next().unwrap_or("").split_once('=') else {
                continue;
            };
            let value = value.trim();
            let switch = |default: bool| match value {
                "on" => true,
                "off" => false,
                _ => default,
            };
            let speed = |default: f32| value.parse::<f32>().ok().filter(|speed| speed.is_finite()).map_or(default, |speed| speed.clamp(-1.0, 1.0));
            // Layout names are passed to the keymap compiler: letters, digits and a few
            // marks only.
            let name = |default: &str| {
                let plain = value.chars().all(|c| c.is_ascii_alphanumeric() || "_-,".contains(c));
                if plain { value.to_string() } else { default.to_string() }
            };
            match key.trim() {
                "touchpad_natural_scroll" => settings.touchpad_natural_scroll = switch(settings.touchpad_natural_scroll),
                "touchpad_tap" => settings.touchpad_tap = switch(settings.touchpad_tap),
                "touchpad_while_typing_off" => {
                    settings.touchpad_while_typing_off = switch(settings.touchpad_while_typing_off)
                }
                "touchpad_speed" => settings.touchpad_speed = speed(settings.touchpad_speed),
                "mouse_natural_scroll" => settings.mouse_natural_scroll = switch(settings.mouse_natural_scroll),
                "mouse_speed" => settings.mouse_speed = speed(settings.mouse_speed),
                "mouse_left_handed" => settings.mouse_left_handed = switch(settings.mouse_left_handed),
                "cursor_size" => {
                    if let Ok(size) = value.parse::<u32>() {
                        settings.cursor_size = size.clamp(CURSOR_SIZE_RANGE.0, CURSOR_SIZE_RANGE.1);
                    }
                }
                "cursor_color" => {
                    if value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        settings.cursor_color = [0, 2, 4].map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap());
                    }
                }
                "hand_control" => settings.hand_control = switch(settings.hand_control),
                "hand_speed" => settings.hand_speed = speed(settings.hand_speed),
                "hand_pinch_sensitivity" => settings.hand_pinch_sensitivity = speed(settings.hand_pinch_sensitivity),
                "hand_scroll_speed" => settings.hand_scroll_speed = speed(settings.hand_scroll_speed),
                "hand_drag_lock" => settings.hand_drag_lock = switch(settings.hand_drag_lock),
                "keyboard_layout" => settings.keyboard_layout = name(&settings.keyboard_layout),
                "keyboard_variant" => settings.keyboard_variant = name(&settings.keyboard_variant),
                "keyboard_repeat_delay" => {
                    if let Ok(delay) = value.parse::<u32>() {
                        settings.repeat_delay = delay.clamp(REPEAT_DELAY_RANGE.0, REPEAT_DELAY_RANGE.1);
                    }
                }
                "keyboard_repeat_rate" => {
                    if let Ok(rate) = value.parse::<u32>() {
                        settings.repeat_rate = rate.clamp(REPEAT_RATE_RANGE.0, REPEAT_RATE_RANGE.1);
                    }
                }
                "screen_off_after" => {
                    settings.screen_off_after = match value.parse::<u32>() {
                        Ok(minutes) => Some(minutes.clamp(1, 240)),
                        Err(_) if value == "never" => None,
                        Err(_) => settings.screen_off_after,
                    }
                }
                "lock_after" => {
                    settings.lock_after = match value.parse::<u32>() {
                        Ok(minutes) => Some(minutes.clamp(1, 240)),
                        Err(_) if value == "never" => None,
                        Err(_) => settings.lock_after,
                    }
                }
                _ => {}
            }
        }
        settings
    }

    /// Reads the settings from the configuration file, or the defaults without one.
    pub fn load() -> InputSettings {
        config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| InputSettings::parse(&text))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_what_laptops_usually_do() {
        let settings = InputSettings::parse("");
        assert!(settings.touchpad_natural_scroll);
        assert!(settings.touchpad_tap);
        assert!(!settings.mouse_natural_scroll);
        assert_eq!(settings, InputSettings::default());
    }

    #[test]
    fn reads_every_setting() {
        let settings = InputSettings::parse(
            "touchpad_natural_scroll = off\ntouchpad_tap = off\ntouchpad_while_typing_off = off\n\
             touchpad_speed = 0.4\nmouse_natural_scroll = on\nmouse_speed = -0.5\nmouse_left_handed = on\n\
             keyboard_layout = de\nkeyboard_variant = nodeadkeys\nkeyboard_repeat_delay = 400\n\
             keyboard_repeat_rate = 40 # fast\ntheme = matrix\nhand_control = on\nhand_speed = 0.3\n",
        );
        assert!(!settings.touchpad_natural_scroll && !settings.touchpad_tap && !settings.touchpad_while_typing_off);
        assert_eq!(settings.touchpad_speed, 0.4);
        assert!(settings.mouse_natural_scroll && settings.mouse_left_handed);
        assert_eq!(settings.mouse_speed, -0.5);
        assert_eq!((settings.keyboard_layout.as_str(), settings.keyboard_variant.as_str()), ("de", "nodeadkeys"));
        assert_eq!((settings.repeat_delay, settings.repeat_rate), (400, 40));
        assert!(settings.hand_control);
        assert_eq!(settings.hand_speed, 0.3);
        assert!(!InputSettings::default().hand_control, "the camera is never on unasked");
    }

    #[test]
    fn comfortable_hand_controls_have_safe_defaults_and_parse() {
        let defaults = InputSettings::parse("");
        assert_eq!(defaults.hand_pinch_sensitivity, 0.0);
        assert_eq!(defaults.hand_scroll_speed, 0.0);
        assert!(defaults.hand_drag_lock);
        let settings = InputSettings::parse("hand_pinch_sensitivity = 0.7\nhand_scroll_speed = -0.4\nhand_drag_lock = off");
        assert_eq!(settings.hand_pinch_sensitivity, 0.7);
        assert_eq!(settings.hand_scroll_speed, -0.4);
        assert!(!settings.hand_drag_lock);
        let clamped = InputSettings::parse("hand_pinch_sensitivity = 8\nhand_scroll_speed = -8");
        assert_eq!(clamped.hand_pinch_sensitivity, 1.0);
        assert_eq!(clamped.hand_scroll_speed, -1.0);
    }

    #[test]
    fn normalized_speeds_reject_nonfinite_and_invalid_values() {
        for key in ["touchpad_speed", "mouse_speed", "hand_speed", "hand_pinch_sensitivity", "hand_scroll_speed"] {
            for invalid in ["NaN", "inf", "-inf", "fast", ""] {
                let parsed = InputSettings::parse(&format!("{key} = {invalid}"));
                assert_eq!(parsed, InputSettings::default(), "{key} = {invalid}");
                let valid = InputSettings::parse(&format!("{key} = 0.4"));
                let repeated = InputSettings::parse(&format!("{key} = 0.4\n{key} = {invalid}"));
                assert_eq!(repeated, valid, "{key} must retain its previous finite value");
            }
        }
        assert!(InputSettings::parse("hand_drag_lock = invalid").hand_drag_lock);
        assert!(!InputSettings::parse("hand_drag_lock = off\nhand_drag_lock = invalid").hand_drag_lock);
    }

    #[test]
    fn cursor_preferences_parse_and_keep_defaults_for_invalid_values() {
        let defaults = InputSettings::default();
        assert_eq!(defaults.cursor_size, 24);
        assert_eq!(defaults.cursor_color, DEFAULT_CURSOR_COLOR);
        assert_eq!(CURSOR_SIZE_RANGE, (16, 64));
        let settings = InputSettings::parse("cursor_size = 40\ncursor_color = 12aBef");
        assert_eq!(settings.cursor_size, 40);
        assert_eq!(settings.cursor_color, [18, 171, 239]);
        assert_eq!(InputSettings::parse("cursor_size = 0").cursor_size, 16);
        assert_eq!(InputSettings::parse("cursor_size = 999").cursor_size, 64);
        for invalid in ["-2", "large", "NaN", "24.5", "4294967296", ""] {
            assert_eq!(InputSettings::parse(&format!("cursor_size = 40\ncursor_size = {invalid}")).cursor_size, 40);
        }
        for invalid in ["#ffffff", "abc", "11223344", "gg1122", "12 456", ""] {
            assert_eq!(InputSettings::parse(&format!("cursor_color = 123456\ncursor_color = {invalid}")).cursor_color, [18, 52, 86]);
        }
    }

    #[test]
    fn bad_values_are_ignored_or_clamped() {
        let settings = InputSettings::parse(
            "touchpad_speed = 9\nmouse_speed = fast\nkeyboard_repeat_delay = 1\nkeyboard_repeat_rate = 999\n\
             touchpad_tap = maybe\nkeyboard_layout = us; rm -rf /\n",
        );
        assert_eq!(settings.touchpad_speed, 1.0);
        assert_eq!(settings.mouse_speed, 0.0);
        assert_eq!(settings.repeat_delay, REPEAT_DELAY_RANGE.0);
        assert_eq!(settings.repeat_rate, REPEAT_RATE_RANGE.1);
        assert!(settings.touchpad_tap);
        assert_eq!(settings.keyboard_layout, "");
    }
}
