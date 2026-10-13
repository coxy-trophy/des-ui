//! Serves the desktop protocol to the shell. Unix peer credentials, not an
//! inherited environment variable, authorize commands. Root can only recover a lock.

use des_common::ipc::{Command, DesktopState};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}};
use std::path::{Path, PathBuf};
use smithay::reexports::rustix::net::sockopt::socket_peercred;

const MAX_CLIENTS: usize = 8;
const MAX_LINE: usize = 4096;
const MAX_COMMANDS: usize = 64;
const IO_PER_POLL: usize = 8192;
const MAX_OUTPUT: usize = 256 * 1024;

struct Client<S = UnixStream> {
    stream: S,
    pending: Vec<u8>,
    outgoing: VecDeque<u8>,
    pid: u32,
    uid: u32,
}

impl<S: Read + Write> Client<S> {
    fn queue(&mut self, line: &str) -> bool {
        if line.len() > MAX_OUTPUT.saturating_sub(self.outgoing.len()) { return false; }
        self.outgoing.extend(line.as_bytes());
        true
    }

    fn flush_output(&mut self) -> bool {
        let mut written = 0;
        // Bound syscall count as well as bytes, even with a peer that accepts one
        // byte at a time or repeated interrupted system calls.
        for _ in 0..32 {
            if self.outgoing.is_empty() || written >= IO_PER_POLL { break; }
            let bytes = self.outgoing.as_slices().0;
            let count = bytes.len().min(IO_PER_POLL - written);
            match self.stream.write(&bytes[..count]) {
                Ok(0) => return false,
                Ok(n) => { self.outgoing.drain(..n); written += n; }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }

    fn poll(&mut self, shell: Option<u32>, commands: &mut Vec<Command>) -> bool {
        let from_shell = shell == Some(self.pid);
        if !from_shell && self.uid != 0 { return false; }
        let mut read = 0;
        let mut buf = [0u8; 512];
        while commands.len() < MAX_COMMANDS {
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                if end >= MAX_LINE { return false; }
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                if let Ok(line) = std::str::from_utf8(&line) {
                    if let Some(command) = Command::decode(line.trim_end()) {
                        if from_shell || matches!(command, Command::Unlock) { commands.push(command); }
                    }
                }
                continue;
            }
            if self.pending.len() >= MAX_LINE { return false; }
            if read >= IO_PER_POLL { break; }
            let count = buf.len().min(MAX_LINE - self.pending.len()).min(IO_PER_POLL - read);
            match self.stream.read(&mut buf[..count]) {
                Ok(0) => return false,
                Ok(n) => { self.pending.extend_from_slice(&buf[..n]); read += n; }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // Yield rather than retry forever on repeated signals.
                Err(e) if e.kind() == ErrorKind::Interrupted => break,
                Err(_) => return false,
            }
        }
        self.flush_output()
    }
}

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    clients: Vec<Client>,
    shell_pid: Option<u32>,
    last: String,
    last_displays: String,
    last_locked: String,
}

impl IpcServer {
    pub fn bind(path: PathBuf) -> std::io::Result<IpcServer> {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(IpcServer {
            listener, path, clients: Vec::new(), shell_pid: None,
            last: DesktopState::default().encode(),
            last_displays: String::new(),
            last_locked: des_common::ipc::Event::locked(false),
        })
    }

    pub fn path(&self) -> &Path { &self.path }

    /// Revoke old connections on restart; a previous shell must not unlock its
    /// replacement's screen. A new connection is checked against this PID.
    pub fn authorize_shell(&mut self, pid: u32) {
        self.clients.clear();
        self.shell_pid = Some(pid);
    }

    pub fn poll(&mut self) -> Vec<Command> {
        for _ in 0..MAX_CLIENTS {
            let Ok((stream, _)) = self.listener.accept() else { break; };
            let Ok(peer) = socket_peercred(&stream) else { continue; };
            let pid = peer.pid.as_raw_nonzero().get() as u32;
            let uid = peer.uid.as_raw();
            if (self.shell_pid != Some(pid) && uid != 0) || self.clients.len() >= MAX_CLIENTS { continue; }
            if stream.set_nonblocking(true).is_err() { continue; }
            let mut client = Client { stream, pending: Vec::new(), outgoing: VecDeque::new(), pid, uid };
            if client.queue(&self.last) && client.queue(&self.last_displays) && client.queue(&self.last_locked) {
                self.clients.push(client);
            }
        }
        let mut commands = Vec::new();
        self.clients.retain_mut(|client| client.poll(self.shell_pid, &mut commands));
        // Rotate peers so an always-busy connection cannot starve the next one.
        if !self.clients.is_empty() { self.clients.rotate_left(1); }
        commands
    }

    pub fn send(&mut self, line: &str) {
        self.clients.retain_mut(|client| client.queue(line) && client.flush_output());
    }

    pub fn publish(&mut self, state: &DesktopState) {
        let line = state.encode();
        if line != self.last { self.send(&line); self.last = line; }
    }

    pub fn publish_locked(&mut self, locked: bool) {
        let line = des_common::ipc::Event::locked(locked);
        if line != self.last_locked { self.send(&line); self.last_locked = line; }
    }

    pub fn publish_displays(&mut self, displays: &des_common::ipc::Displays) {
        let line = displays.encode();
        if line != self.last_displays { self.send(&line); self.last_displays = line; }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    // An IO boundary double: no compositor/display or listening socket required.
    // Writes are deliberately partial and can stall to exercise backpressure.
    #[derive(Default)]
    struct Transport {
        input: VecDeque<u8>,
        output: Vec<u8>,
        write_size: usize,
        blocked: bool,
        eof: bool,
    }
    impl Read for Transport {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.input.is_empty() {
                return if self.eof { Ok(0) } else { Err(ErrorKind::WouldBlock.into()) };
            }
            let count = buf.len().min(self.input.len());
            for byte in &mut buf[..count] { *byte = self.input.pop_front().unwrap(); }
            Ok(count)
        }
    }
    impl Write for Transport {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.blocked { return Err(ErrorKind::WouldBlock.into()); }
            let count = buf.len().min(self.write_size.max(1));
            self.output.extend_from_slice(&buf[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    fn client(input: &[u8], pid: u32, uid: u32) -> Client<Transport> {
        Client {
            stream: Transport { input: input.iter().copied().collect(), write_size: 3, ..Default::default() },
            pending: Vec::new(),
            outgoing: VecDeque::new(),
            pid,
            uid,
        }
    }
    fn poll(client: &mut Client<Transport>, shell: Option<u32>) -> (bool, Vec<Command>) {
        let mut commands = Vec::new();
        let alive = client.poll(shell, &mut commands);
        (alive, commands)
    }

    #[test]
    #[ignore = "requires Unix socket permissions; desktop CI runs with --include-ignored"]
    fn kernel_credentials_socket_permissions_and_restart_revocation() {
        use smithay::reexports::rustix::process::geteuid;
        // Run as an ordinary session user: root deliberately has recovery rights.
        assert!(!geteuid().is_root(), "run this integration test without sudo");
        let path = std::env::temp_dir().join(format!("des-ipc-integration-{}.sock", std::process::id()));
        let mut server = IpcServer::bind(path.clone()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let mut untrusted = UnixStream::connect(&path).unwrap();
        untrusted.write_all(b"unlock\nsplit\n").unwrap();
        assert!(server.poll().is_empty());

        server.authorize_shell(std::process::id());
        let mut shell = UnixStream::connect(&path).unwrap();
        shell.write_all(b"lock\nsplit\n").unwrap();
        assert_eq!(server.poll(), [Command::Lock, Command::Split]);
        // PID 0 cannot be a live shell, and replacing the shell revokes its old
        // connection even though that process still has the same UID.
        server.authorize_shell(0);
        let _ = shell.write_all(b"unlock\n");
        assert!(server.poll().is_empty());
        let mut old_shell = UnixStream::connect(&path).unwrap();
        old_shell.write_all(b"unlock\n").unwrap();
        assert!(server.poll().is_empty());

        server.authorize_shell(std::process::id());
        let mut clients = Vec::new();
        for _ in 0..12 { clients.push(UnixStream::connect(&path).unwrap()); }
        server.poll();
        server.poll();
        assert!(server.clients.len() <= 8);
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn unregistered_process_cannot_unlock_or_change_the_desktop() {
        let mut client = client(b"unlock\nsplit\n", 12, 1000);
        assert!(!poll(&mut client, Some(13)).0);
        assert!(poll(&mut client, None).1.is_empty());
    }
    #[test]
    fn shell_commands_work_but_a_previous_shell_is_rejected() {
        let mut client = client(b"activate 0\nclo", 12, 1000);
        assert_eq!(poll(&mut client, Some(12)).1, [Command::Activate(0)]);
        client.stream.input.extend(b"se 0\n");
        assert_eq!(poll(&mut client, Some(12)).1, [Command::Close(0)]);
        client.stream.input.extend(b"unlock\n");
        assert!(poll(&mut client, Some(13)).1.is_empty());
    }
    #[test]
    fn only_root_can_recover_and_only_by_unlocking() {
        let mut client = client(b"split\nunlock\nclose 0\n", 99, 0);
        assert_eq!(poll(&mut client, Some(12)).1, [Command::Unlock]);
    }
    #[test]
    fn command_flood_yields_between_frames_without_losing_commands() {
        let mut client = client("split\n".repeat(1000).as_bytes(), 12, 1000);
        let (_, commands) = poll(&mut client, Some(12));
        assert!(commands.len() <= 64 && !commands.is_empty());
        let mut count = commands.len();
        for _ in 0..1000 {
            count += poll(&mut client, Some(12)).1.len();
            if count == 1000 { break; }
        }
        assert_eq!(count, 1000);
    }
    #[test]
    fn oversized_unterminated_lines_are_disconnected_promptly() {
        let mut client = client(&vec![b'x'; 100_000], 12, 1000);
        assert!(!poll(&mut client, Some(12)).0);
        assert!(client.pending.len() <= 4096);
        assert!(client.stream.input.len() >= 100_000 - 4096);
    }
    #[test]
    fn final_command_is_received_before_eof() {
        let mut client = client(b"unlock\n", 12, 1000);
        client.stream.eof = true;
        assert_eq!(poll(&mut client, Some(12)), (false, vec![Command::Unlock]));
    }
    #[test]
    fn partial_writes_and_backpressure_preserve_protocol_lines() {
        let mut client = client(b"", 12, 1000);
        client.stream.blocked = true;
        assert!(client.queue("locked 1\nstate 1 7 0 - 0\n"));
        assert!(client.flush_output());
        assert!(client.stream.output.is_empty());
        client.stream.blocked = false;
        for _ in 0..100 { client.flush_output(); }
        assert_eq!(client.stream.output, b"locked 1\nstate 1 7 0 - 0\n");
    }
    #[test]
    fn stalled_clients_cannot_accumulate_unbounded_output() {
        let mut client = client(b"", 12, 1000);
        client.stream.blocked = true;
        let mut refused = false;
        for _ in 0..1000 {
            if !client.queue(&"x".repeat(4096)) { refused = true; break; }
        }
        assert!(refused);
        assert!(client.outgoing.len() <= 256 * 1024);
    }
}
