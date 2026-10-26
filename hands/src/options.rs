//! Bounded command-line preferences for camera-driven hand control.
pub const USAGE: &str = "Usage: des-hands [OPTIONS]
  -h, --help                  Show help without opening the camera
  --camera PATH               Camera device (default: /dev/video0)
  --seconds N                 Stop live tracking after N seconds
  --record FILE               Save live tracking
  --serve                     Send gesture events to the compositor
  --speed S                   Pointer speed (-1 to 1)
  --scroll-speed S            Scroll speed (-1 to 1)
  --pinch-sensitivity S       Pinch sensitivity (-1 to 1)
  --drag-lock on|off           Carry a drag between pinches
  --replay FILE               Replay recorded gestures
  --learn FILE                Record labeled gestures
  --picture FILE              Analyze one picture
  --describe                  Describe model outputs
  --camera-only               Measure camera frame rate";

/// Reject invalid invocations before loading models or touching any device.
pub fn validate(args: &[String]) -> Result<(), String> {
    let mut words = args.iter();
    while let Some(flag) = words.next() {
        match flag.as_str() {
            "--serve" | "--describe" | "--camera-only" => {},
            "--camera" | "--seconds" | "--record" | "--replay" | "--learn" | "--picture" |
            "--speed" | "--scroll-speed" | "--pinch-sensitivity" | "--drag-lock" => {
                let value = words.next().filter(|word| !word.is_empty() && !word.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                let valid = match flag.as_str() {
                    "--seconds" => value.parse::<u64>().is_ok(),
                    "--speed" | "--scroll-speed" | "--pinch-sensitivity" => value.parse::<f32>().is_ok_and(f32::is_finite),
                    "--drag-lock" => matches!(value.as_str(), "on" | "off"),
                    _ => true,
                };
                if !valid { return Err(format!("invalid value for {flag}: {value}")); }
            }
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    Ok(())
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServeOptions {
    pub speed: f32,
    pub scroll_speed: f32,
    pub pinch_sensitivity: f32,
    pub drag_lock: bool,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self { speed: 0.0, scroll_speed: 0.0, pinch_sensitivity: 0.0, drag_lock: true }
    }
}

impl ServeOptions {
    pub fn parse(args: &[String]) -> Self {
        let mut options = Self::default();
        for pair in args.windows(2) {
            let number = || pair[1].parse::<f32>().ok().filter(|number| number.is_finite()).map(|number| number.clamp(-1.0, 1.0));
            match pair[0].as_str() {
                "--speed" => if let Some(value) = number() { options.speed = value; },
                "--scroll-speed" => if let Some(value) = number() { options.scroll_speed = value; },
                "--pinch-sensitivity" => if let Some(value) = number() { options.pinch_sensitivity = value; },
                "--drag-lock" => match pair[1].as_str() {
                    "on" => options.drag_lock = true,
                    "off" => options.drag_lock = false,
                    _ => {},
                },
                _ => {},
            }
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(words: &[&str]) -> ServeOptions {
        ServeOptions::parse(&words.iter().map(|word| word.to_string()).collect::<Vec<_>>())
    }
    #[test]
    fn defaults_match_desktop_preferences() {
        let options = parsed(&[]);
        assert_eq!((options.speed, options.scroll_speed, options.pinch_sensitivity), (0.0, 0.0, 0.0));
        assert!(options.drag_lock);
    }
    #[test]
    fn each_preference_reaches_the_process_independently() {
        let options = parsed(&["--speed", "0.4", "--scroll-speed", "-0.5", "--pinch-sensitivity", "0.7", "--drag-lock", "off"]);
        assert_eq!((options.speed, options.scroll_speed, options.pinch_sensitivity), (0.4, -0.5, 0.7));
        assert!(!options.drag_lock);
    }
    #[test]
    fn invalid_values_never_poison_motion() {
        let options = parsed(&["--speed", "NaN", "--scroll-speed", "inf", "--pinch-sensitivity", "-inf", "--drag-lock", "perhaps"]);
        assert_eq!(options, ServeOptions::default());
        let options = parsed(&["--speed", "9", "--scroll-speed", "-9", "--pinch-sensitivity", "2"]);
        assert_eq!((options.speed, options.scroll_speed, options.pinch_sensitivity), (1.0, -1.0, 1.0));
    }
}
