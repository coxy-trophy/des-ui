//! Privileged recovery for an unusable lock screen. Ordinary session processes
//! cannot use the compositor's recovery channel.
use des_common::ipc::{Command, Event};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

fn next_lock(reader: &mut impl BufRead) -> Result<bool, String> {
    for _ in 0..64 {
        let mut line = String::new();
        Read::by_ref(reader).take(256 * 1024).read_line(&mut line)
            .map_err(|e| format!("Cannot read the compositor's response: {e}"))?;
        if !line.ends_with('\n') { return Err("The compositor did not confirm the lock state".into()); }
        if let Some(Event::Locked(locked)) = Event::decode(&line) { return Ok(locked); }
    }
    Err("The compositor did not confirm the lock state".into())
}

fn unlock_connection(stream: &mut (impl Read + Write)) -> Result<(), String> {
    let mut reader = BufReader::new(stream);
    if !next_lock(&mut reader)? { return Ok(()); }
    reader.get_mut().write_all(Command::Unlock.encode().as_bytes())
        .map_err(|e| format!("Cannot request recovery: {e}"))?;
    if next_lock(&mut reader)? { return Err("The compositor refused recovery".into()); }
    Ok(())
}

pub fn unlock() -> Result<(), String> {
    if !rustix::process::geteuid().is_root() {
        return Err("Recovery requires administrator authentication: run sudo -k -- des-ui --unlock from a text console".into());
    }
    // sudo normally drops XDG_RUNTIME_DIR. SUDO_UID identifies the caller's
    // session, while authority comes solely from the kernel's root credentials.
    let dir = match std::env::var("SUDO_UID") {
        Ok(uid) => PathBuf::from(format!("/run/user/{}", uid.parse::<u32>().map_err(|_| "Invalid sudo user ID")?)),
        Err(_) => std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).ok_or("Set XDG_RUNTIME_DIR to the session's runtime directory")?,
    };
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Cannot find desktops in {}: {e}", dir.display()))?;
    let mut reached = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("des-comp-") && name.ends_with(".sock") {
            let Ok(mut stream) = UnixStream::connect(entry.path()) else { continue; };
            stream.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            unlock_connection(&mut stream)?;
            reached += 1;
        }
    }
    if reached == 0 { return Err(format!("Found no running desktop in {}", dir.display())); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Connection { replies: Cursor<Vec<u8>>, requests: Vec<u8> }
    impl Read for Connection {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> { self.replies.read(bytes) }
    }
    impl Write for Connection {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.requests.extend_from_slice(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    fn connection(replies: &[u8]) -> Connection { Connection { replies: Cursor::new(replies.to_vec()), requests: Vec::new() } }

    #[test]
    fn recovery_waits_for_confirmation_from_the_compositor() {
        let mut peer = connection(b"state 1 7 0 - 0\nlocked 1\nlocked 0\n");
        assert!(unlock_connection(&mut peer).is_ok());
        assert_eq!(peer.requests, b"unlock\n");
        let mut peer = connection(b"locked 1\n");
        assert!(unlock_connection(&mut peer).is_err());
        let mut peer = connection(b"locked 0\n");
        assert!(unlock_connection(&mut peer).is_ok());
        assert!(peer.requests.is_empty());
    }
    #[test]
    fn malformed_or_unbounded_recovery_responses_are_refused() {
        assert!(unlock_connection(&mut connection(b"locked 0")).is_err());
        assert!(unlock_connection(&mut connection(&vec![b'x'; 300_000])).is_err());
    }
}
