//! Explicit, bounded file payload jobs for authenticated KDE Connect sessions.
//! Control packets must travel through the caller's paired TLS actor. This module
//! never trusts a packet-provided host, accepts a remote local path, or auto-launches received files.
use crate::{identity, packet::Packet};
use openssl::{pkey::{PKey, Private}, ssl::{ErrorCode, Ssl, SslContext, SslContextBuilder, SslMethod, SslStream, SslVerifyMode, SslVersion}, x509::X509};
use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    env, fs::File, io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread::{self, JoinHandle}, time::{Duration, Instant},
};
pub type Result<T> = std::result::Result<T, String>;
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;
const BUFFER_SIZE: usize = 64 * 1024;
const MAX_ACTIVE: usize = 2;
const MAX_PENDING: usize = 8;
const MAX_HISTORY: usize = 32;
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
const TLS_TIMEOUT: Duration = Duration::from_secs(5);
const STALL_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const TICK: Duration = Duration::from_millis(10);

/// Construct only from a currently paired, authenticated control-session actor.
/// In particular, `address` must be its actual socket peer IP, never a JSON host.
#[derive(Clone)]
pub struct PeerTls {
    local_key: PKey<Private>, local_certificate: X509,
    peer_id: String, pin: [u8; 32], address: IpAddr,
}
impl PeerTls {
    pub fn new(local_key: PKey<Private>, local_certificate: X509, peer_id: String,
        peer_certificate: X509, pin: [u8; 32], address: IpAddr) -> Result<Self> {
        if address.is_unspecified() || address.is_multicast() { return Err("Invalid authenticated peer address".into()); }
        let digest = identity::peer_digest(&peer_certificate, &peer_id).map_err(|e| e.to_string())?;
        if digest != pin { return Err("Payload peer certificate does not match the paired pin".into()); }
        let names: Vec<_> = local_certificate.subject_name().entries_by_nid(openssl::nid::Nid::COMMONNAME).collect();
        if names.len() != 1 { return Err("Local certificate must have one device identity".into()); }
        let local_id = names[0].data().to_string().map_err(|e| e.to_string())?;
        identity::peer_digest(&local_certificate, &local_id).map_err(|e| e.to_string())?;
        if local_id == peer_id || !local_certificate.public_key().map_err(|e| e.to_string())?.public_eq(&local_key) {
            return Err("Invalid local payload identity or key".into());
        }
        Ok(Self { local_key, local_certificate, peer_id, pin, address })
    }
    fn context(&self) -> Result<SslContext> {
        let mut context = SslContextBuilder::new(SslMethod::tls()).map_err(|e| e.to_string())?;
        context.set_min_proto_version(Some(SslVersion::TLS1_2)).map_err(|e| e.to_string())?;
        context.set_max_proto_version(Some(SslVersion::TLS1_2)).map_err(|e| e.to_string())?;
        context.set_certificate(&self.local_certificate).map_err(|e| e.to_string())?;
        context.set_private_key(&self.local_key).map_err(|e| e.to_string())?;
        context.check_private_key().map_err(|e| e.to_string())?;
        let expected = self.peer_id.clone(); let pin = self.pin;
        context.set_verify_callback(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT, move |_, verify| {
            verify.error_depth() == 0 && verify.current_cert().is_some_and(|cert| {
                // Reject a different DER fingerprint before expensive public-key work.
                cert.digest(openssl::hash::MessageDigest::sha256()).is_ok_and(|digest| digest.as_ref() == pin)
                    && identity::peer_digest(&cert.to_owned(), &expected).is_ok_and(|digest| digest == pin)
            })
        });
        Ok(context.build())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TransferSnapshot {
    pub id: String, pub peer_id: String, pub direction: String,
    pub filename: String, pub size: u64, pub transferred: u64,
    pub state: String, pub error: Option<String>, pub path: Option<String>,
}
/// A locally selected source opened before acquiring a shared manager mutex.
/// Opening/statting a regular file can still block in a filesystem driver.
pub struct PreparedSource { file:File,filename:String,size:u64 }
impl PreparedSource {
    pub fn open(path:&Path)->Result<Self> {let(file,filename,size)=open_source(path)?;Ok(Self{file,filename,size})}
}
pub struct SendTicket { pub id: String, pub packet: Packet }
struct IncomingHeader { filename: String, size: u64, port: u16 }
impl IncomingHeader {
    fn from_packet(packet: &Packet) -> Result<Self> {
        if packet.kind != "kdeconnect.share.request" { return Err("Not a file-sharing request".into()); }
        let filename = packet.body.get("filename").and_then(|v| v.as_str()).ok_or("File request is missing a filename")?;
        validate_filename(filename)?;
        let size = packet.payload_size.and_then(|n| u64::try_from(n).ok()).filter(|n| (1..=MAX_FILE_SIZE).contains(n))
            .ok_or("Files must have a known size between 1 byte and 1 GiB")?;
        let info = packet.payload_transfer_info.as_ref().ok_or("Missing payload transfer information")?;
        let port = info.get("port").and_then(|n| n.as_u64()).and_then(|n| u16::try_from(n).ok())
            .filter(|n| (1739..=1764).contains(n)).ok_or("Payload port must be 1739–1764")?;
        Ok(Self { filename: filename.into(), size, port })
    }
}
struct Pending { peer: PeerTls, header: IncomingHeader }
struct JobState {
    view: Mutex<TransferSnapshot>, cancelled: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}
impl JobState {
    fn check(&self, deadline: Instant) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) { return Err("Transfer cancelled".into()); }
        if Instant::now() >= deadline { return Err("Transfer deadline exceeded".into()); }
        Ok(())
    }
    fn phase(&self, phase: &str) { self.view.lock().unwrap().state = phase.into(); }
    fn progress(&self, bytes: u64) { self.view.lock().unwrap().transferred = bytes; }
    fn register(&self, socket: &TcpStream) -> Result<()> {
        let mut owned = self.socket.lock().unwrap();
        if self.cancelled.load(Ordering::Acquire) { let _ = socket.shutdown(Shutdown::Both); return Err("Transfer cancelled".into()); }
        *owned = Some(socket.try_clone().map_err(|e| e.to_string())?); Ok(())
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(socket) = self.socket.lock().unwrap().take() { let _ = socket.shutdown(Shutdown::Both); }
    }
    fn finish(&self, result: Result<Option<String>>) {
        self.socket.lock().unwrap().take();
        let mut view = self.view.lock().unwrap();
        match result {
            // Completion wins a cancellation racing the atomic final commit.
            Ok(path) => { view.state = "complete".into(); view.path = path; view.error = None; }
            Err(error) => {
                view.state = if self.cancelled.load(Ordering::Acquire) { "cancelled" } else { "failed" }.into();
                view.error = Some(error.chars().take(512).collect());
            }
        }
    }
}
struct Job { shared: Arc<JobState>, pending: Option<Pending>, handle: Option<JoinHandle<()>>, created: Instant }

/// At most two worker threads, eight unaccepted incoming offers and 32 records.
/// Progress is coalesced into one snapshot per job; there is no progress queue.
pub struct TransferManager { directory: Arc<File>, directory_path: PathBuf, jobs: BTreeMap<String, Job> }
impl TransferManager {
    pub fn new(download_directory: PathBuf) -> Result<Self> {
        let directory = open_directory(&download_directory, true, true)?;
        Ok(Self { directory, directory_path: download_directory, jobs: BTreeMap::new() })
    }
    pub fn offer_receive(&mut self, peer: PeerTls, packet: &Packet) -> Result<TransferSnapshot> {
        let header = IncomingHeader::from_packet(packet)?;
        self.sweep();
        if self.jobs.values().filter(|job| job.pending.is_some()).count() >= MAX_PENDING { return Err("Too many pending incoming files".into()); }
        let id = self.reserve()?;
        let shared = new_state(&id, &peer.peer_id, "receive", &header.filename, header.size, "offered");
        let view = shared.view.lock().unwrap().clone();
        self.jobs.insert(id, Job { shared, pending: Some(Pending { peer, header }), handle: None, created: Instant::now() });
        Ok(view)
    }
    /// Call only for explicit local approval while this offer's peer is still
    /// paired and connected. On peer loss/unpair, call `cancel_peer` promptly.
    pub fn accept(&mut self, id: &str) -> Result<TransferSnapshot> {
        self.sweep(); self.check_capacity()?;
        let job = self.jobs.get_mut(id).ok_or("Unknown or expired transfer")?;
        let pending = job.pending.take().ok_or("Incoming offer is no longer pending")?;
        let shared = job.shared.clone(); let directory = self.directory.clone(); let root = self.directory_path.clone();
        shared.phase("connecting");
        let handle = thread::Builder::new().name("des-file-receive".into()).spawn(move || {
            let result = receive_file(&shared, pending, directory, root);
            shared.finish(result);
        });
        match handle {
            Ok(handle) => job.handle = Some(handle),
            Err(error) => { job.shared.finish(Err(error.to_string())); return Err(error.to_string()); }
        }
        Ok(job.shared.view.lock().unwrap().clone())
    }
    /// The returned packet must immediately be sent on this peer's authenticated
    /// control channel. Its listener is already bound. Cancel on control-send
    /// failure; the listener also expires after 30 seconds without a valid peer.
    pub fn send(&mut self, peer: PeerTls, path: &Path) -> Result<SendTicket> {
        self.send_prepared(peer,PreparedSource::open(path)?)
    }
    /// Insert a source already prepared outside the caller's global manager lock.
    pub fn send_prepared(&mut self,peer:PeerTls,source:PreparedSource)->Result<SendTicket> {
        self.sweep(); self.check_capacity()?;
        let PreparedSource{file,filename,size}=source;
        let id = self.reserve()?;
        let listener = bind_payload(peer.address)?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let shared = new_state(&id, &peer.peer_id, "send", &filename, size, "waiting");
        let worker = shared.clone();
        let handle = thread::Builder::new().name("des-file-send".into()).spawn(move || {
            let result = send_file(&worker, peer, listener, file, size);
            worker.finish(result.map(|_| None));
        }).map_err(|e| e.to_string())?;
        let packet = share_packet(&filename, size as i64, port);
        self.jobs.insert(id.clone(), Job { shared, pending: None, handle: Some(handle), created: Instant::now() });
        Ok(SendTicket { id, packet })
    }
    pub fn snapshots(&mut self) -> Vec<TransferSnapshot> {
        self.sweep();
        let mut jobs: Vec<_> = self.jobs.values().collect();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.created));
        jobs.into_iter().map(|job| job.shared.view.lock().unwrap().clone()).collect()
    }
    pub fn cancel(&mut self, id: &str) -> Result<()> {
        let job = self.jobs.get_mut(id).ok_or("Unknown transfer")?;
        let terminal = is_terminal(&job.shared.view.lock().unwrap().state);
        if terminal { return Ok(()); }
        job.shared.cancel();
        if job.pending.take().is_some() {
            let mut view = job.shared.view.lock().unwrap(); view.state = "cancelled".into(); view.error = None;
        }
        Ok(())
    }
    pub fn cancel_peer(&mut self, peer_id: &str) {
        let ids: Vec<_> = self.jobs.iter().filter(|(_, job)| job.shared.view.lock().unwrap().peer_id == peer_id).map(|(id, _)| id.clone()).collect();
        for id in ids { let _ = self.cancel(&id); }
    }
    fn check_capacity(&self) -> Result<()> {
        if self.jobs.values().filter(|job| job.handle.as_ref().is_some_and(|h| !h.is_finished())).count() >= MAX_ACTIVE {
            Err("Two file transfers are already active".into())
        } else { Ok(()) }
    }
    fn reserve(&mut self) -> Result<String> {
        while self.jobs.len() >= MAX_HISTORY {
            let oldest = self.jobs.iter().filter(|(_, job)| job.pending.is_none() && job.handle.is_none())
                .min_by_key(|(_, job)| job.created).map(|(id, _)| id.clone()).ok_or("File-transfer history is busy")?;
            self.jobs.remove(&oldest);
        }
        for _ in 0..4 {
            let mut bytes = [0; 16]; openssl::rand::rand_bytes(&mut bytes).map_err(|e| e.to_string())?;
            let id = identity::hex(&bytes);
            if !self.jobs.contains_key(&id) { return Ok(id); }
        }
        Err("Could not allocate a unique transfer ID".into())
    }
    fn sweep(&mut self) {
        for job in self.jobs.values_mut() {
            if job.pending.is_some() && job.created.elapsed() >= OFFER_TIMEOUT {
                job.pending.take(); let mut view = job.shared.view.lock().unwrap();
                view.state = "expired".into(); view.error = Some("File offer expired; ask the sender to retry".into());
            }
            if job.handle.as_ref().is_some_and(JoinHandle::is_finished) { let _ = job.handle.take().unwrap().join(); }
        }
    }
}
impl Drop for TransferManager {
    fn drop(&mut self) {
        for job in self.jobs.values() { job.shared.cancel(); }
        for job in self.jobs.values_mut() { if let Some(handle) = job.handle.take() { let _ = handle.join(); } }
    }
}
pub fn default_download_directory() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or("HOME is unavailable")?;
    Ok(PathBuf::from(home).join("Downloads/DESConnect"))
}
fn is_terminal(state: &str) -> bool { matches!(state, "complete" | "failed" | "cancelled" | "expired") }
fn new_state(id: &str, peer: &str, direction: &str, filename: &str, size: u64, state: &str) -> Arc<JobState> {
    Arc::new(JobState { view: Mutex::new(TransferSnapshot { id: id.into(), peer_id: peer.into(), direction: direction.into(), filename: filename.into(), size, transferred: 0, state: state.into(), error: None, path: None }), cancelled: AtomicBool::new(false), socket: Mutex::new(None) })
}
fn share_packet(filename: &str, size: i64, port: u16) -> Packet {
    let body = json!({"filename": filename, "numberOfFiles": 1, "totalPayloadSize": size, "open": false});
    Packet { id: 0, kind: "kdeconnect.share.request".into(), body: body.as_object().unwrap().clone(), payload_size: Some(size),
        payload_transfer_info: Some(json!({"port": port}).as_object().unwrap().clone()), extra: Default::default() }
}
fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty() || filename.len() > 220 || matches!(filename, "." | "..") || filename.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return Err("Unsafe filename: use a single name of at most 220 UTF-8 bytes without separators or controls".into());
    }
    Ok(())
}

// Each component is opened relative to the previously verified directory FD.
// No symlink is followed, including in ancestors. An owned private final directory
// remains anchored even if a pathname is renamed concurrently.
fn open_directory(path: &Path, create: bool, private: bool) -> Result<Arc<File>> {
    if !path.is_absolute() || path.as_os_str().as_bytes().len() > 4096 { return Err("Select an absolute local directory path".into()); }
    let components: Vec<_> = path.components().collect();
    if components.len() > 128 || components.iter().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) {
        return Err("Directory paths must not contain traversal components".into());
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = File::from(rustix::fs::open("/", flags, Mode::empty()).map_err(|e| e.to_string())?);
    for component in components {
        let Component::Normal(name) = component else { continue };
        let next = match rustix::fs::openat(&directory, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) if create => {
                match rustix::fs::mkdirat(&directory, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}, Err(e) => return Err(e.to_string()),
                }
                rustix::fs::openat(&directory, name, flags, Mode::empty()).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(format!("Unsafe or unavailable directory: {e}")),
        };
        directory = File::from(next);
        let metadata = directory.metadata().map_err(|e| e.to_string())?;
        let uid = rustix::process::geteuid().as_raw();
        if private && metadata.uid() != 0 && metadata.uid() != uid { return Err("A parent directory belongs to another user".into()); }
        // Root-owned sticky /tmp is safe for disposable test/private roots; other
        // group/other-writable ancestors are refused.
        if private && metadata.mode() & 0o022 != 0 && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0) {
            return Err("A parent directory is writable by other users".into());
        }
    }
    let metadata = directory.metadata().map_err(|e| e.to_string())?;
    if private && (metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0) {
        return Err("Download directory must be this user's private mode-0700 directory".into());
    }
    Ok(Arc::new(directory))
}
fn open_source(path: &Path) -> Result<(File, String, u64)> {
    let filename = path.file_name().and_then(|n| n.to_str()).ok_or("Select a regular file with a UTF-8 filename")?;
    validate_filename(filename)?;
    let directory = open_directory(path.parent().ok_or("File parent is missing")?, false, false)?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = File::from(rustix::fs::openat(&*directory, filename, flags, Mode::empty()).map_err(|e| e.to_string())?);
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || !(1..=MAX_FILE_SIZE).contains(&metadata.len()) { return Err("Select a nonempty regular file no larger than 1 GiB".into()); }
    Ok((file, filename.into(), metadata.len()))
}
struct PartialFile { directory: Arc<File>, name: String, file: File, committed: bool }
impl PartialFile {
    fn create(directory: Arc<File>, id: &str) -> Result<Self> {
        let name = format!(".des-connect-{id}.part");
        let fd = rustix::fs::openat(&*directory, &name, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR).map_err(|e| e.to_string())?;
        Ok(Self { directory, name, file: File::from(fd), committed: false })
    }
    fn commit(&mut self, filename: &str) -> Result<String> {
        self.file.sync_all().map_err(|e| e.to_string())?;
        for suffix in 0..1000 {
            let candidate = if suffix == 0 { filename.into() } else { format!("{suffix}-{filename}") };
            match rustix::fs::renameat_with(&*self.directory, &self.name, &*self.directory, &candidate, RenameFlags::NOREPLACE) {
                Ok(()) => { self.committed = true; let _ = self.directory.sync_all(); return Ok(candidate); }
                Err(rustix::io::Errno::EXIST) => {}, Err(e) => return Err(e.to_string()),
            }
        }
        Err("Too many existing files with that name".into())
    }
}
impl Drop for PartialFile {
    fn drop(&mut self) { if !self.committed { let _ = rustix::fs::unlinkat(&*self.directory, &self.name, AtFlags::empty()); } }
}

fn bind_payload(address: IpAddr) -> Result<TcpListener> {
    let wildcard: IpAddr = if address.is_ipv4() { Ipv4Addr::UNSPECIFIED.into() } else { Ipv6Addr::UNSPECIFIED.into() };
    for port in 1739..=1764 {
        if let Ok(listener) = TcpListener::bind(SocketAddr::new(wildcard, port)) {
            listener.set_nonblocking(true).map_err(|e| e.to_string())?; return Ok(listener);
        }
    }
    Err("No free payload port in 1739–1764".into())
}
fn payload_tls(tcp: TcpStream, peer: &PeerTls, client: bool, state: &JobState, deadline: Instant) -> Result<SslStream<TcpStream>> {
    tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    state.register(&tcp)?;
    let context = peer.context()?;
    let mut stream = SslStream::new(Ssl::new(&context).map_err(|e| e.to_string())?, tcp).map_err(|e| e.to_string())?;
    let handshake_end = deadline.min(Instant::now() + TLS_TIMEOUT);
    loop {
        state.check(handshake_end)?;
        match if client { stream.connect() } else { stream.accept() } {
            Ok(()) => break,
            Err(e) if matches!(e.code(), ErrorCode::WANT_READ | ErrorCode::WANT_WRITE) => thread::sleep(TICK),
            Err(e) => return Err(format!("Payload TLS authentication failed: {e}")),
        }
    }
    let certificate = stream.ssl().peer_certificate().ok_or("Payload peer sent no certificate")?;
    if identity::peer_digest(&certificate, &peer.peer_id).map_err(|e| e.to_string())? != peer.pin {
        return Err("Payload certificate changed from the paired certificate".into());
    }
    Ok(stream)
}
fn send_file(state: &JobState, peer: PeerTls, listener: TcpListener, mut file: File, size: u64) -> Result<()> {
    let deadline = Instant::now() + OFFER_TIMEOUT;
    let mut rejected = 0;
    let mut stream = loop {
        state.check(deadline)?;
        match listener.accept() {
            Ok((tcp, address)) => {
                if address.ip() != peer.address { continue; }
                match payload_tls(tcp, &peer, false, state, deadline) {
                    Ok(stream) => break stream,
                    Err(error) => {
                        state.socket.lock().unwrap().take();
                        state.check(deadline)?; rejected += 1;
                        if rejected >= 8 { return Err(error); }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(TICK),
            Err(e) => return Err(e.to_string()),
        }
    };
    drop(listener);
    state.phase("transferring");
    let total_end = Instant::now() + TRANSFER_TIMEOUT;
    let mut buffer = [0; BUFFER_SIZE]; let mut transferred = 0;
    while transferred < size {
        state.check(total_end)?;
        let wanted = (size - transferred).min(BUFFER_SIZE as u64) as usize;
        let n = file.read(&mut buffer[..wanted]).map_err(|e| e.to_string())?;
        if n == 0 { return Err("Selected source file was truncated during transfer".into()); }
        let mut written = 0; let mut stalled = Instant::now() + STALL_TIMEOUT;
        while written < n {
            state.check(total_end.min(stalled))?;
            match stream.write(&buffer[written..n]) {
                Ok(0) => return Err("Payload connection closed while sending".into()),
                Ok(count) => { written += count; transferred += count as u64; state.progress(transferred); stalled = Instant::now() + STALL_TIMEOUT; }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => thread::sleep(TICK),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    if file.metadata().map_err(|e| e.to_string())?.len() != size { return Err("Selected source file changed size during transfer".into()); }
    // TLS is nonblocking; the receiver already has authenticated size framing.
    let _ = stream.shutdown();
    Ok(())
}
fn receive_file(state: &JobState, pending: Pending, directory: Arc<File>, root: PathBuf) -> Result<Option<String>> {
    let address = SocketAddr::new(pending.peer.address, pending.header.port);
    state.check(Instant::now() + TRANSFER_TIMEOUT)?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|e| format!("Cannot connect to payload: {e}"))?;
    let mut stream = payload_tls(tcp, &pending.peer, true, state, Instant::now() + TLS_TIMEOUT)?;
    state.phase("transferring");
    let id = state.view.lock().unwrap().id.clone();
    let mut partial = PartialFile::create(directory, &id)?;
    let mut buffer = [0; BUFFER_SIZE]; let mut transferred = 0;
    let total_end = Instant::now() + TRANSFER_TIMEOUT; let mut stalled = Instant::now() + STALL_TIMEOUT;
    while transferred < pending.header.size {
        state.check(total_end.min(stalled))?;
        let wanted = (pending.header.size - transferred).min(BUFFER_SIZE as u64) as usize;
        match stream.read(&mut buffer[..wanted]) {
            Ok(0) => return Err("Payload ended before the advertised file size".into()),
            Ok(n) => {
                partial.file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                transferred += n as u64; state.progress(transferred); stalled = Instant::now() + STALL_TIMEOUT;
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => thread::sleep(TICK),
            Err(e) => return Err(format!("Incomplete payload: {e}")),
        }
    }
    state.check(total_end)?;
    let filename = partial.commit(&pending.header.filename)?;
    Ok(Some(root.join(filename).to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    fn private_temp() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    #[test]
    fn metadata_rejects_traversal_unknown_lengths_and_non_payload_ports() {
        for name in ["../secret", "/etc/passwd", "a/b", "a\\b", ".", "..", "bad\nname", "bad\u{202e}txt"] {
            assert!(validate_filename(name).is_err(), "{name:?}");
        }
        assert!(validate_filename("photo 01.jpg").is_ok());
        assert!(IncomingHeader::from_packet(&share_packet("ok.txt", 1, 1739)).is_ok());
        for size in [0, -1, (MAX_FILE_SIZE+1) as i64] {
            assert!(IncomingHeader::from_packet(&share_packet("ok.txt", size, 1739)).is_err());
        }
        for port in [0, 22, 1738, 1765] {
            assert!(IncomingHeader::from_packet(&share_packet("ok.txt", 1, port)).is_err());
        }
    }
    #[test]
    fn anchored_files_refuse_symlinks_and_never_overwrite_collisions() {
        let dir = private_temp();
        let root = dir.path().join("downloads");
        let directory = open_directory(&root, true, true).unwrap();
        fs::write(root.join("report.txt"), b"original").unwrap();
        let mut partial = PartialFile::create(directory.clone(), "fixture").unwrap();
        partial.file.write_all(b"received").unwrap();
        let name = partial.commit("report.txt").unwrap();
        assert_ne!(name, "report.txt");
        assert_eq!(fs::read(root.join("report.txt")).unwrap(), b"original");
        assert_eq!(fs::read(root.join(name)).unwrap(), b"received");
        std::os::unix::fs::symlink(&root, dir.path().join("link")).unwrap();
        assert!(open_directory(&dir.path().join("link"), false, true).is_err());
        std::os::unix::fs::symlink(root.join("report.txt"), root.join("source-link")).unwrap();
        assert!(open_source(&root.join("source-link")).is_err());
    }
    // Payload tests share a finite port range. Serialize them so a released-port
    // assertion cannot race another fixture legitimately claiming that port.
    static NETWORK_TESTS: Mutex<()> = Mutex::new(());
    struct Fixture { directory: tempfile::TempDir, a: identity::Store, b: identity::Store, _network_guard: std::sync::MutexGuard<'static, ()> }
    impl Fixture {
        fn new() -> Self {
            let network_guard = NETWORK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
            let directory = private_temp();
            let a = identity::Store::open(&directory.path().join("a")).unwrap();
            let b = identity::Store::open(&directory.path().join("b")).unwrap();
            Self { directory, a, b, _network_guard: network_guard }
        }
        fn peer(local: &identity::Store, remote: &identity::Store) -> PeerTls {
            PeerTls::new(local.key.clone(), local.certificate.clone(), remote.id().into(), remote.certificate.clone(),
                identity::peer_digest(&remote.certificate, remote.id()).unwrap(), Ipv4Addr::LOCALHOST.into()).unwrap()
        }
        fn manager(&self, name: &str) -> TransferManager {
            TransferManager::new(self.directory.path().join(name)).unwrap()
        }
    }
    fn wait_job(manager: &mut TransferManager, id: &str) -> TransferSnapshot {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let view = manager.snapshots().into_iter().find(|v| v.id == id).unwrap();
            if is_terminal(&view.state) { return view; }
            assert!(Instant::now() < deadline, "Transfer did not finish: {view:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn real_mutual_tls_streams_only_after_explicit_accept_and_preserves_existing_files() {
        let fixture = Fixture::new();
        let source = fixture.directory.path().join("photo.bin");
        let content = vec![0x5a; BUFFER_SIZE * 3 + 73];
        fs::write(&source, &content).unwrap();
        let mut sender = fixture.manager("send-downloads");
        let mut receiver = fixture.manager("received");
        fs::write(receiver.directory_path.join("photo.bin"), b"keep").unwrap();
        let ticket = sender.send(Fixture::peer(&fixture.a, &fixture.b), &source).unwrap();
        let mut untrusted_header = ticket.packet.clone();
        untrusted_header.payload_transfer_info.as_mut().unwrap().insert("host".into(), json!("203.0.113.77"));
        let offered = receiver.offer_receive(Fixture::peer(&fixture.b, &fixture.a), &untrusted_header).unwrap();
        assert_eq!(offered.state, "offered");
        thread::sleep(Duration::from_millis(50));
        assert_eq!(sender.snapshots()[0].transferred, 0);
        assert_eq!(fs::read_dir(&receiver.directory_path).unwrap().count(), 1);
        receiver.accept(&offered.id).unwrap();
        let received = wait_job(&mut receiver, &offered.id);
        assert_eq!(received.state, "complete", "{received:?}");
        assert_eq!(received.transferred, content.len() as u64);
        assert_eq!(fs::read(received.path.unwrap()).unwrap(), content);
        assert_eq!(fs::read(receiver.directory_path.join("photo.bin")).unwrap(), b"keep");
        assert_eq!(wait_job(&mut sender, &ticket.id).state, "complete");
    }
    #[test]
    fn wrong_payload_certificate_is_rejected_and_does_not_consume_the_legitimate_offer() {
        let fixture = Fixture::new();
        let attacker = identity::Store::open(&fixture.directory.path().join("attacker")).unwrap();
        let source = fixture.directory.path().join("secret.txt");
        fs::write(&source, b"paired peer only").unwrap();
        let mut sender = fixture.manager("send-downloads");
        let mut receiver = fixture.manager("received");
        let ticket = sender.send(Fixture::peer(&fixture.a, &fixture.b), &source).unwrap();
        let port = IncomingHeader::from_packet(&ticket.packet).unwrap().port;
        let tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        // The attacker copies the expected device ID/CN but owns another key.
        // The payload's DER pin must reject it even though its self-signature and CN are valid.
        let mut forged = X509::builder().unwrap();
        forged.set_version(2).unwrap();
        let serial = openssl::bn::BigNum::from_u32(123).unwrap().to_asn1_integer().unwrap();
        forged.set_serial_number(&serial).unwrap();
        forged.set_subject_name(fixture.b.certificate.subject_name()).unwrap();
        forged.set_issuer_name(fixture.b.certificate.subject_name()).unwrap();
        forged.set_pubkey(&attacker.key).unwrap();
        let start = openssl::asn1::Asn1Time::days_from_now(0).unwrap();
        let end = openssl::asn1::Asn1Time::days_from_now(1).unwrap();
        forged.set_not_before(&start).unwrap(); forged.set_not_after(&end).unwrap();
        forged.sign(&attacker.key, openssl::hash::MessageDigest::sha256()).unwrap();
        let attacker_tls = PeerTls::new(attacker.key.clone(), forged.build(), fixture.a.id().into(), fixture.a.certificate.clone(),
            identity::peer_digest(&fixture.a.certificate, fixture.a.id()).unwrap(), Ipv4Addr::LOCALHOST.into()).unwrap();
        let fake = new_state("fake", fixture.a.id(), "receive", "secret.txt", 16, "test");
        let rejected = payload_tls(tcp, &attacker_tls, true, &fake, Instant::now()+Duration::from_secs(3));
        assert!(rejected.is_err());
        assert_eq!(sender.snapshots()[0].transferred, 0);
        let offer = receiver.offer_receive(Fixture::peer(&fixture.b, &fixture.a), &ticket.packet).unwrap();
        receiver.accept(&offer.id).unwrap();
        let received = wait_job(&mut receiver, &offer.id);
        assert_eq!(received.state, "complete", "{received:?}");
        assert_eq!(fs::read(received.path.unwrap()).unwrap(), b"paired peer only");
        assert_eq!(wait_job(&mut sender, &ticket.id).state, "complete");
    }
    #[test]
    fn early_eof_removes_partial_and_does_not_publish_a_complete_file() {
        let fixture = Fixture::new();
        let listener = bind_payload(Ipv4Addr::LOCALHOST.into()).unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = Fixture::peer(&fixture.a, &fixture.b);
        let server = thread::spawn(move || {
            let end = Instant::now()+Duration::from_secs(5);
            let tcp = loop { if let Ok((tcp, _)) = listener.accept() { break tcp; } assert!(Instant::now()<end); thread::sleep(TICK); };
            let state = new_state("fake", &peer.peer_id, "send", "short.bin", 1000, "test");
            let mut stream = payload_tls(tcp, &peer, false, &state, end).unwrap();
            stream.write_all(b"short").unwrap();
            let _ = stream.shutdown();
        });
        let mut receiver = fixture.manager("received");
        let offer = receiver.offer_receive(Fixture::peer(&fixture.b, &fixture.a), &share_packet("short.bin", 1000, port)).unwrap();
        receiver.accept(&offer.id).unwrap();
        let failed = wait_job(&mut receiver, &offer.id);
        assert_eq!(failed.state, "failed", "{failed:?}");
        assert!(failed.path.is_none());
        assert_eq!(fs::read_dir(&receiver.directory_path).unwrap().count(), 0);
        server.join().unwrap();
    }
    #[test]
    fn cancel_shuts_down_the_actual_payload_socket_and_removes_partial() {
        let fixture = Fixture::new();
        let listener = bind_payload(Ipv4Addr::LOCALHOST.into()).unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = Fixture::peer(&fixture.a, &fixture.b);
        let (closed_tx, closed_rx) = std::sync::mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let end = Instant::now()+Duration::from_secs(5);
            let tcp = loop { if let Ok((tcp, _)) = listener.accept() { break tcp; } assert!(Instant::now()<end); thread::sleep(TICK); };
            let state = new_state("fake", &peer.peer_id, "send", "slow.bin", 1000, "test");
            let mut stream = payload_tls(tcp, &peer, false, &state, end).unwrap();
            stream.write_all(b"start").unwrap();
            let mut b = [0; 1];
            loop {
                assert!(Instant::now()<end, "Cancellation did not close the payload socket");
                match stream.read(&mut b) {
                    Err(e) if e.kind()==std::io::ErrorKind::WouldBlock => thread::sleep(TICK),
                    _ => { closed_tx.send(()).unwrap(); break; }
                }
            }
        });
        let mut receiver = fixture.manager("received");
        let offer = receiver.offer_receive(Fixture::peer(&fixture.b, &fixture.a), &share_packet("slow.bin", 1000, port)).unwrap();
        receiver.accept(&offer.id).unwrap();
        let end = Instant::now()+Duration::from_secs(5);
        while receiver.snapshots()[0].transferred==0 { assert!(Instant::now()<end); thread::sleep(TICK); }
        receiver.cancel(&offer.id).unwrap();
        assert_eq!(wait_job(&mut receiver, &offer.id).state, "cancelled");
        closed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(fs::read_dir(&receiver.directory_path).unwrap().count(), 0);
        server.join().unwrap();
    }
    #[test]
    fn pending_limits_expiry_history_and_sparse_file_size_are_bounded() {
        let fixture = Fixture::new(); let mut manager = fixture.manager("received");
        let peer = Fixture::peer(&fixture.b, &fixture.a);
        let packet = share_packet("file.txt", 1, 1739);
        let mut ids = Vec::new();
        for _ in 0..MAX_PENDING { ids.push(manager.offer_receive(peer.clone(), &packet).unwrap().id); }
        assert!(manager.offer_receive(peer.clone(), &packet).is_err());
        for id in ids { manager.cancel(&id).unwrap(); }
        for _ in 0..MAX_HISTORY*2 {
            let id = manager.offer_receive(peer.clone(), &packet).unwrap().id;
            manager.cancel(&id).unwrap();
        }
        assert!(manager.snapshots().len() <= MAX_HISTORY);
        let offered = manager.offer_receive(peer, &packet).unwrap();
        manager.jobs.get_mut(&offered.id).unwrap().created = Instant::now()-OFFER_TIMEOUT;
        assert!(manager.accept(&offered.id).is_err());
        let source = fixture.directory.path().join("too-large.bin");
        File::create(&source).unwrap().set_len(MAX_FILE_SIZE+1).unwrap();
        assert!(open_source(&source).is_err());
        assert_eq!(BUFFER_SIZE, 64*1024);
    }
    #[test]
    fn payload_handshake_has_an_absolute_deadline() {
        let fixture = Fixture::new();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _stalled = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (tcp, _) = listener.accept().unwrap();
        let state = new_state("fake", fixture.b.id(), "send", "x", 1, "test");
        let start = Instant::now();
        assert!(payload_tls(tcp, &Fixture::peer(&fixture.a, &fixture.b), false, &state, start+Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn concurrency_and_drop_cancel_waiting_listeners_and_release_ports() {
        let fixture = Fixture::new();
        let source = fixture.directory.path().join("file.txt"); fs::write(&source, b"data").unwrap();
        let mut manager = fixture.manager("received");
        let peer = Fixture::peer(&fixture.a, &fixture.b);
        let first = manager.send(peer.clone(), &source).unwrap();
        let first_port = IncomingHeader::from_packet(&first.packet).unwrap().port;
        let second = manager.send(peer.clone(), &source).unwrap();
        let second_port = IncomingHeader::from_packet(&second.packet).unwrap().port;
        assert!(manager.send(peer, &source).is_err());
        manager.cancel(&first.id).unwrap();
        assert_eq!(wait_job(&mut manager, &first.id).state, "cancelled");
        let start = Instant::now(); drop(manager);
        assert!(start.elapsed() < Duration::from_secs(1));
        let a = TcpListener::bind((Ipv4Addr::UNSPECIFIED, first_port)).unwrap();
        let b = TcpListener::bind((Ipv4Addr::UNSPECIFIED, second_port)).unwrap();
        drop((a, b));
    }

}
