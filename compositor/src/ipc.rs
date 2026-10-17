//! Serves the desktop protocol (see `des_common::ipc`) to the shell.

use des_common::ipc::{Command, DesktopState};
use des_common::transport::WriteBuffer;
use std::io::{ErrorKind, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

// The peer identity comes from the kernel, never from protocol text.
fn is_shell(peer_pid: Option<u32>, shell_pid: Option<u32>) -> bool {
    peer_pid.is_some() && peer_pid == shell_pid
}

fn decode_command(line: &str, peer_pid: Option<u32>, shell_pid: Option<u32>) -> Option<Command> {
    is_shell(peer_pid, shell_pid).then(|| Command::decode(line)).flatten()
}

struct Client {
    peer_pid: Option<u32>,
    stream: UnixStream,
    pending: Vec<u8>,
    outgoing: WriteBuffer,
}

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    clients: Vec<Client>,
    /// The most recent state line, sent to clients as they connect.
    last: String,
    /// Likewise for the displays line.
    last_displays: String,
    last_layouts: String,
    /// Likewise for whether the screen is locked.
    last_locked: String,
    last_recording: String,
}

impl IpcServer {
    pub fn bind(path: PathBuf) -> std::io::Result<IpcServer> {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(IpcServer {
            listener,
            path,
            clients: Vec::new(),
            last: DesktopState::default().encode(),
            last_displays: String::new(),
            last_layouts: des_common::ipc::LayoutInfo::encode_all(&[]),
            last_locked: des_common::ipc::Event::locked(false),
            last_recording: des_common::ipc::Event::recording("off"),
        })
    }

    pub fn has_shell_client(&self) -> bool {
        !self.clients.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accepts new clients and returns the commands received since the last call.
    /// Never blocks; meant to be called once per frame.
    pub fn poll(&mut self, shell_pid: Option<u32>) -> Vec<Command> {
        for _ in 0..16 {
            let Ok((stream, _)) = self.listener.accept() else { break };
            let peer_pid = smithay::reexports::rustix::net::sockopt::socket_peercred(&stream)
                .ok()
                .map(|credentials| credentials.pid.as_raw_pid() as u32);
            if !is_shell(peer_pid, shell_pid) {
                continue;
            }
            if stream.set_nonblocking(true).is_ok() {
                // A client that has already left cannot be greeted, but may have said
                // something first, which the reading below still takes in.
                let mut outgoing = WriteBuffer::new(1024 * 1024);
                if outgoing.enqueue(self.last.as_bytes())
                    .and_then(|()| outgoing.enqueue(self.last_displays.as_bytes()))
                    .and_then(|()| outgoing.enqueue(self.last_layouts.as_bytes()))
                    .and_then(|()| outgoing.enqueue(self.last_locked.as_bytes()))
                    .and_then(|()| outgoing.enqueue(self.last_recording.as_bytes())).is_err() {
                    continue;
                }
                self.clients.push(Client {
                    peer_pid,
                    stream,
                    pending: Vec::new(),
                    outgoing,
                });
            }
        }

        let mut commands = Vec::new();
        self.clients.retain_mut(|client| {
            // A restarted shell revokes its predecessor's connections too.
            if !is_shell(client.peer_pid, shell_pid) {
                return false;
            }
            if client.outgoing.flush(&mut client.stream).is_err() {
                return false;
            }
            let mut buf = [0u8; 512];
            let mut read = 0;
            // A client may say its piece and leave at once: what it said still counts.
            let mut gone = false;
            while read < 64 * 1024 {
                match client.stream.read(&mut buf) {
                    Ok(0) => {
                        gone = true;
                        break;
                    }
                    Ok(n) => {
                        read += n;
                        client.pending.extend_from_slice(&buf[..n]);
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        gone = true;
                        break;
                    }
                }
                while let Some(end) = client.pending.iter().position(|byte| *byte == b'\n') {
                    if end >= 4096 { return false; }
                    let line: Vec<u8> = client.pending.drain(..=end).collect();
                    if let Some(command) = decode_command(String::from_utf8_lossy(&line).trim_end(), client.peer_pid, shell_pid) {
                        commands.push(command);
                    }
                }
                if client.pending.len() >= 4096 { return false; }
            }
            // A client that never finishes a line is not speaking the protocol.
            !gone && client.pending.len() < 4096
        });
        commands
    }

    /// Sends one protocol line to every client.
    pub fn send(&mut self, line: &str) {
        if line.starts_with("recording ") { self.last_recording = line.to_string(); }
        self.clients.retain_mut(|client| {
            client.outgoing.enqueue(line.as_bytes()).is_ok()
                && client.outgoing.flush(&mut client.stream).is_ok()
        });
    }

    /// Sends `state` to every client, unless it is what they already have.
    pub fn publish(&mut self, state: &DesktopState) {
        let line = state.encode();
        if line == self.last {
            return;
        }
        self.send(&line);
        self.last = line;
    }
}

impl IpcServer {
    /// Tells every client whether the screen is locked, when that changes. A shell
    /// started while it is locked is told as it connects.
    pub fn publish_locked(&mut self, locked: bool) {
        let line = des_common::ipc::Event::locked(locked);
        if line != self.last_locked {
            self.send(&line);
            self.last_locked = line;
        }
    }

    /// Sends the layouts that are not the plain one, unless none of them changed.
    pub fn publish_layouts(&mut self, layouts: &[des_common::ipc::LayoutInfo]) {
        let line = des_common::ipc::LayoutInfo::encode_all(layouts);
        if line != self.last_layouts {
            self.send(&line);
            self.last_layouts = line;
        }
    }

    /// Sends the monitors to every client, unless nothing about them changed.
    pub fn publish_displays(&mut self, displays: &des_common::ipc::Displays) {
        let line = displays.encode();
        if line != self.last_displays {
            self.send(&line);
            self.last_displays = line;
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn rejects_commands_from_other_processes_and_unknown_peers() {
        for line in ["unlock", "lock", "record /tmp/private.mp4", "hold-idle on"] {
            assert_eq!(decode_command(line, Some(42), Some(7)), None, "{line}");
            assert_eq!(decode_command(line, None, Some(7)), None, "{line}");
            assert_eq!(decode_command(line, Some(7), None), None, "{line}");
            assert_eq!(decode_command(line, None, None), None, "{line}");
        }
    }

    #[test]
    fn accepts_shell_commands_and_rejects_the_previous_shell() {
        assert_eq!(decode_command("unlock", Some(7), Some(7)), Some(Command::Unlock));
        assert_eq!(decode_command("unlock", Some(7), Some(8)), None);
        assert_eq!(decode_command("unlock", Some(8), Some(8)), Some(Command::Unlock));
    }

    #[test]
    fn refuses_socket_clients_without_the_shell_identity() {
        let path = std::env::temp_dir().join(format!("des-ipc-denied-{}.sock", std::process::id()));
        let mut server = IpcServer::bind(path.clone()).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(b"unlock\n").unwrap();
        assert!(server.poll(Some(std::process::id() + 1)).is_empty());
        assert!(server.clients.is_empty());

        // Even an authenticated connection loses authority on a shell restart.
        let mut client = UnixStream::connect(&path).unwrap();
        assert!(server.poll(Some(std::process::id())).is_empty());
        assert_eq!(server.clients.len(), 1);
        client.write_all(b"unlock\n").unwrap();
        assert!(server.poll(None).is_empty());
        assert!(server.clients.is_empty());
    }

    #[test]
    fn serves_state_and_receives_commands() {
        let path = std::env::temp_dir().join(format!("des-ipc-test-{}.sock", std::process::id()));
        let mut server = IpcServer::bind(path.clone()).unwrap();

        let mut client = UnixStream::connect(&path).unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        assert!(server.poll(Some(std::process::id())).is_empty());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(DesktopState::decode(&line), Some(DesktopState::default()));

        let state = DesktopState {
            screens: 2,
            focus: Some(0),
            apps: vec![des_common::ipc::AppInfo {
                app_id: String::new(),
                title: "Editor".into(),
                screen: 1,
                place: des_common::ipc::Place::Main,
                workspace: 0,
                pinned: false,
                urgent: false,
            }],
            ..Default::default()
        };
        server.publish(&state);
        // Other things a new client is told, such as the lock, come in between.
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.starts_with("state ") {
                break;
            }
            assert!(line == "locked 0\n" || line == "recording off\n" || line == "layouts\n", "{line}");
        }
        assert_eq!(DesktopState::decode(&line), Some(state));

        // A split write and a junk line must still yield exactly the valid commands.
        client.write_all(b"activate 0\nnonsense\nclo").unwrap();
        client.write_all(b"se 0\n").unwrap();
        let mut commands = Vec::new();
        for _ in 0..50 {
            commands.extend(server.poll(Some(std::process::id())));
            if commands.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(commands, [Command::Activate(0), Command::Close(0)]);

        drop(server);
        assert!(!path.exists());
    }
}
