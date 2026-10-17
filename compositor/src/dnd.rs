//! Files dragged out of the shell's file panel. The shell is an ordinary client whose
//! toolkit cannot start a Wayland drag, so it names the files over IPC and the
//! compositor starts the drag itself, as its own data source, while the button that
//! began it is still held.

use std::io::Write;
use std::os::fd::OwnedFd;

use smithay::input::Seat;
use smithay::input::pointer::{GrabStartData, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::{ServerDndGrabHandler, SourceMetadata, start_dnd};

use crate::DesComp;

/// What a dropped file is offered as: the list browsers and file managers take, and a
/// plain path for anything that only reads text.
pub const MIME_TYPES: [&str; 2] = ["text/uri-list", "text/plain;charset=utf-8"];
const LEFT_BUTTON: u32 = 0x110;

/// The bytes a client is sent for `mime`: URIs one per line for `text/uri-list`, as the
/// list's RFC asks (CRLF), and the decoded paths for plain text.
pub fn payload(uris: &[String], mime: &str) -> Option<Vec<u8>> {
    if mime == "text/uri-list" {
        return Some(uris.iter().map(|uri| format!("{uri}\r\n")).collect::<String>().into_bytes());
    }
    if mime.starts_with("text/plain") {
        let paths: Vec<Vec<u8>> = uris.iter().filter_map(|uri| path_bytes(uri)).collect();
        return Some(paths.join(&b'\n'));
    }
    None
}

/// `file:///a%20b` → `/a b`, byte for byte; `None` for anything else or bad escapes.
pub fn path_bytes(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    bytes.starts_with(b"/").then_some(bytes)
}

/// Whether a drag may start now: never while locked or while a screenshot is being
/// marked, only from the shell, and only while the button that began it is still down.
pub fn may_start(locked: bool, picking: bool, from_shell: bool, button_down: bool) -> bool {
    !locked && !picking && from_shell && button_down
}

impl DesComp {
    /// Starts dragging `uris` from where the pointer is, if the shell is the one being
    /// dragged from; otherwise does nothing.
    pub fn start_file_drag(&mut self, uris: Vec<String>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let shell_client = self.shell.as_ref().and_then(|shell| shell.wl_surface()).and_then(|surface| surface.client());
        let from_shell = pointer
            .current_focus()
            .and_then(|surface| surface.client())
            .zip(shell_client)
            .is_some_and(|(focus, shell)| focus == shell);
        // The press that began it holds an implicit click grab, which the drag replaces.
        let button_down = self.buttons_down > 0;
        if uris.is_empty() || !may_start(self.locked, self.picking, from_shell, button_down) {
            return;
        }
        let location = pointer.current_location();
        let start = GrabStartData { focus: None, button: LEFT_BUTTON, location };
        let metadata = SourceMetadata {
            mime_types: MIME_TYPES.iter().map(|mime| mime.to_string()).collect(),
            dnd_action: DndAction::Copy,
        };
        self.dragged_files = Some(uris);
        let dh = self.display_handle.clone();
        let seat = self.seat.clone();
        start_dnd(&dh, &seat, self, SERIAL_COUNTER.next_serial(), Some(start), None, metadata);
    }
}

impl DesComp {
    /// Ends a drag out of the shell without dropping anything, as when the screen locks.
    pub fn cancel_file_drag(&mut self) {
        if self.dragged_files.take().is_none() {
            return;
        }
        if let Some(pointer) = self.seat.get_pointer() {
            let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as u32);
            // Leave whatever the drag is over first: ending the grab drops on the surface
            // under it, and a lock must drop nothing.
            let location = pointer.current_location();
            pointer.motion(self, None, &MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time });
            pointer.frame(self);
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
    }
}

impl ServerDndGrabHandler for DesComp {
    fn send(&mut self, mime_type: String, fd: OwnedFd, _seat: Seat<Self>) {
        let Some(bytes) = self.dragged_files.as_deref().and_then(|uris| payload(uris, &mime_type)) else { return };
        // A slow reader must not hold up the compositor.
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(&bytes);
        });
    }

    fn cancelled(&mut self, _seat: Seat<Self>) {
        self.dragged_files = None;
    }

    fn finished(&mut self, _seat: Seat<Self>) {
        self.dragged_files = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_and_plain_paths() {
        let uris = vec!["file:///home/a/notes%20one.txt".to_string(), "file:///tmp/%C3%A9t%C3%A9".to_string()];
        assert_eq!(payload(&uris, "text/uri-list").unwrap(), b"file:///home/a/notes%20one.txt\r\nfile:///tmp/%C3%A9t%C3%A9\r\n");
        assert_eq!(payload(&uris, "text/plain;charset=utf-8").unwrap(), "/home/a/notes one.txt\n/tmp/été".as_bytes());
        assert_eq!(payload(&uris, "image/png"), None);
    }

    #[test]
    fn bad_uris_give_no_path() {
        assert_eq!(path_bytes("http://x/y"), None);
        assert_eq!(path_bytes("file://%zz"), None);
        assert_eq!(path_bytes("file:///a%2"), None);
        assert_eq!(path_bytes("file://relative"), None);
        assert_eq!(path_bytes("file:///a%2Fb").unwrap(), b"/a/b");
    }

    #[test]
    fn drags_start_only_from_the_shell_with_the_button_down_and_never_locked() {
        assert!(may_start(false, false, true, true));
        assert!(!may_start(true, false, true, true));
        assert!(!may_start(false, true, true, true));
        assert!(!may_start(false, false, false, true));
        assert!(!may_start(false, false, true, false));
    }
}
