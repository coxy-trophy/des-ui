//! What des-hands tells the compositor about the hand in front of the webcam: one
//! line each on its standard output, which the compositor reads.

/// The pointer's buttons a hand can press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// Which way two fingers swept, as the hand sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Message {
    /// The pointer moves by this much, as fractions of the screen across and down.
    Motion { dx: f32, dy: f32 },
    Press(Button),
    Release(Button),
    /// Two fingers moved up (less than nothing) or down, by this fraction of the view
    /// taken twice.
    Scroll(f32),
    Sweep(Side),
    /// A fist: the pointer rests; or the fist opened again.
    Rest,
    Wake,
    /// A hand came into view, or left it.
    Seen,
    Gone,
}

fn button(word: &str) -> Option<Button> {
    match word {
        "left" => Some(Button::Left),
        "right" => Some(Button::Right),
        _ => None,
    }
}

impl Message {
    pub fn encode(&self) -> String {
        let side = |left: bool| if left { "left" } else { "right" };
        match self {
            Message::Motion { dx, dy } => format!("motion {dx:.5} {dy:.5}\n"),
            Message::Press(which) => format!("press {}\n", side(*which == Button::Left)),
            Message::Release(which) => format!("release {}\n", side(*which == Button::Left)),
            Message::Scroll(by) => format!("scroll {by:.5}\n"),
            Message::Sweep(which) => format!("sweep {}\n", side(*which == Side::Left)),
            Message::Rest => "rest\n".into(),
            Message::Wake => "wake\n".into(),
            Message::Seen => "seen\n".into(),
            Message::Gone => "gone\n".into(),
        }
    }

    /// Reads one line; anything else is `None`. Numbers out of all reason are refused,
    /// so that nothing sends the pointer to the far side of everything.
    pub fn decode(line: &str) -> Option<Message> {
        let mut words = line.split_whitespace();
        let name = words.next()?;
        let mut number = || words.next()?.parse::<f32>().ok().filter(|value| value.is_finite() && value.abs() <= 4.0);
        let message = match name {
            "motion" => Message::Motion { dx: number()?, dy: number()? },
            "scroll" => Message::Scroll(number()?),
            "press" | "release" | "sweep" => {
                let which = words.next()?;
                match name {
                    "press" => Message::Press(button(which)?),
                    "release" => Message::Release(button(which)?),
                    _ => Message::Sweep(if button(which)? == Button::Left { Side::Left } else { Side::Right }),
                }
            }
            "rest" => Message::Rest,
            "wake" => Message::Wake,
            "seen" => Message::Seen,
            "gone" => Message::Gone,
            _ => return None,
        };
        words.next().is_none().then_some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        for message in [
            Message::Motion { dx: 0.0125, dy: -0.5 },
            Message::Press(Button::Left),
            Message::Release(Button::Right),
            Message::Scroll(-0.25),
            Message::Sweep(Side::Left),
            Message::Sweep(Side::Right),
            Message::Rest,
            Message::Wake,
            Message::Seen,
            Message::Gone,
        ] {
            assert_eq!(Message::decode(&message.encode()), Some(message));
        }
        assert_eq!(Message::decode("motion 1e9 0"), None, "too far");
        assert_eq!(Message::decode("motion NaN 0"), None);
        assert_eq!(Message::decode("press middle"), None);
        assert_eq!(Message::decode("rest now"), None);
        assert_eq!(Message::decode(""), None);
    }
}


/// An explicitly selected camera. Identity is the kernel device path, not a
/// recyclable /dev/video number. This file is separate from general preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSelection {
    pub path: std::path::PathBuf,
    pub identity: std::path::PathBuf,
}

fn camera_settings_path() -> Option<std::path::PathBuf> {
    Some(crate::input::config_path()?.with_file_name("hand-camera"))
}

impl CameraSelection {
    pub fn load() -> std::io::Result<Option<Self>> {
        let Some(path) = camera_settings_path() else { return Ok(None) };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Self::parse(&text).map(Some)
    }

    fn parse(text: &str) -> std::io::Result<Self> {
        let mut lines = text.lines();
        let path = std::path::PathBuf::from(lines.next().unwrap_or_default());
        let identity = std::path::PathBuf::from(lines.next().unwrap_or_default());
        if !path.starts_with("/dev") || !identity.starts_with("/sys/devices") || lines.next().is_some() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid hand camera selection"));
        }
        Ok(Self { path, identity })
    }

    pub fn save(&self) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let text = format!("{}\n{}\n", self.path.display(), self.identity.display());
        Self::parse(&text)?;
        let path = camera_settings_path().ok_or_else(|| std::io::Error::other("Configuration folder unavailable"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension("tmp");
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(temp, path)
    }

    /// Resolve only the chosen device, including after a /dev/video number changes.
    /// A disconnected camera is an error; never substitute another camera.
    pub fn resolve(&self) -> std::io::Result<std::path::PathBuf> {
        for entry in std::fs::read_dir("/sys/class/video4linux")? {
            let entry = entry?;
            if camera_identity(&entry.path()).as_ref() == Some(&self.identity) {
                let node = std::path::PathBuf::from("/dev").join(entry.file_name());
                if node.exists() { return Ok(node); }
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Selected hand camera is disconnected"))
    }
}

/// Unconfigured installations retain their existing default. Once chosen, a
/// camera must remain the same device; malformed preferences also fail closed.
pub fn camera_path() -> std::io::Result<std::path::PathBuf> {
    CameraSelection::load()?.map_or_else(|| Ok("/dev/video0".into()), |selection| selection.resolve())
}

#[cfg(test)]
mod camera_tests {
    use super::*;
    #[test]
    fn selection_rejects_relative_paths_and_extra_lines() {
        assert!(CameraSelection::parse("/dev/video2\n/sys/devices/pci/camera/video4linux/video2\n").is_ok());
        for bad in ["video2\n/sys/devices/camera", "/dev/video2\nrelative", "/dev/video2\n/sys/devices/camera\nextra", ""] {
            assert!(CameraSelection::parse(bad).is_err());
        }
    }
}

/// Physical device location plus the driver's node index, excluding the volatile
/// videoN name. No camera is opened to establish this identity.
pub fn camera_identity(sysfs_node: &std::path::Path) -> Option<std::path::PathBuf> {
    let path = std::fs::canonicalize(sysfs_node).ok()?;
    let index = std::fs::read_to_string(sysfs_node.join("index")).ok()?;
    let index: u32 = index.trim().parse().ok()?;
    let physical = path.parent()?.parent()?;
    // Include the USB product and serial when provided. A different camera in
    // the same port must not inherit an earlier camera's permission.
    let mut fingerprint = String::new();
    for ancestor in physical.ancestors() {
        if ancestor.join("idVendor").exists() {
            for key in ["idVendor", "idProduct", "serial"] {
                let value = std::fs::read(ancestor.join(key)).unwrap_or_default();
                fingerprint.push('-');
                for byte in value { fingerprint.push_str(&format!("{byte:02x}")); }
            }
            break;
        }
    }
    Some(physical.join(format!("video-index-{index}{fingerprint}")))
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn video_number_changes_preserve_identity_but_node_index_and_usb_serial_do_not() {
        let root = std::env::temp_dir().join(format!("des-camera-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let first = root.join("video4linux/video0");
        let renumbered = root.join("video4linux/video4");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&renumbered).unwrap();
        std::fs::write(first.join("index"), "0").unwrap();
        std::fs::write(renumbered.join("index"), "0").unwrap();
        std::fs::write(root.join("idVendor"), "1234").unwrap();
        std::fs::write(root.join("idProduct"), "5678").unwrap();
        std::fs::write(root.join("serial"), "fixture-a").unwrap();
        let original = camera_identity(&first).unwrap();
        assert_eq!(Some(original.clone()),camera_identity(&renumbered));
        std::fs::write(renumbered.join("index"), "1").unwrap();
        assert_ne!(Some(original.clone()),camera_identity(&renumbered));
        std::fs::write(root.join("serial"), "fixture-b").unwrap();
        assert_ne!(Some(original),camera_identity(&first));
        std::fs::remove_dir_all(root).unwrap();
    }
}
