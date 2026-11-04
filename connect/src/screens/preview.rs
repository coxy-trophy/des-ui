//! Official standalone scrcpy raw H.264 over a private Unix ADB forward.
//! A single latest frame, never a recording. Socket and pipe buffers are bounded.
use super::*;
use std::{io::BufReader, os::unix::fs::DirBuilderExt, sync::atomic::{AtomicBool, AtomicU64, Ordering}};

#[derive(Default)]
struct FrameState {
    latest: Option<Vec<u8>>,
    ended: Option<String>,
}

pub(super) struct Preview {
    decoder: Option<Child>,
    reader: Option<thread::JoinHandle<()>>,
    pump: Option<thread::JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
    adb: Option<(PathBuf, String, String)>,
    frames: Arc<Mutex<FrameState>>,
    directory: PathBuf,
}
impl Preview {
    pub(super) fn prepare() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut directory;
        loop {
            directory = env::temp_dir().join(format!("des-connect-preview-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("Cannot create private preview directory: {e}")),
            }
        }
        Ok(Self { decoder: None, reader: None, pump: None, cancelled: Arc::new(AtomicBool::new(false)), adb: None, frames: Arc::default(), directory })
    }
    pub(super) fn socket(&self) -> PathBuf { self.directory.join("video.sock") }
    pub(super) fn forward(&mut self, adb: &Path, serial: &str, server: &Path) -> Result<Command, String> {
        let tag = self.directory.file_name().unwrap().to_string_lossy();
        let remote = format!("/data/local/tmp/{tag}.jar");
        self.adb = Some((adb.to_owned(), serial.into(), remote.clone()));
        let mut push = Command::new(adb);
        push.args(["-s", serial, "push"]).arg(server).arg(&remote);
        run_bounded(&mut push, PROBE_TIMEOUT)?;
        let forward = format!("localfilesystem:{}", self.socket().display());
        // Session ids select separate device abstract sockets, including when
        // multiple bridges exist in one process. The private directory counter
        // is unique for this process lifetime.
        let sequence: u32 = tag.rsplit('-').next().unwrap().parse().unwrap();
        let scid = format!("{:08x}", (std::process::id().wrapping_mul(65537).wrapping_add(sequence)) & 0x7fffffff);
        let mut setup = Command::new(adb);
        setup.args(["-s", serial, "forward", "--no-rebind", &forward, &format!("localabstract:scrcpy_{scid}")]);
        run_bounded(&mut setup, PROBE_TIMEOUT)?;
        let mut command = Command::new(adb);
        command.args(raw_server_args(serial, &remote, &scid));
        Ok(command)
    }
    pub(super) fn start(&mut self, ffmpeg: &Path, log: Arc<Mutex<Vec<u8>>>) -> Result<(), String> {
        let mut command = Command::new(ffmpeg);
        command.args(["-hide_banner", "-loglevel", "error", "-probesize", "32", "-analyzeduration", "1", "-flags", "low_delay", "-threads", "1", "-f", "h264", "-i", "pipe:0", "-vf", "scale=640:640:force_original_aspect_ratio=decrease", "-threads", "1", "-f", "image2pipe", "-vcodec", "ppm", "-flush_packets", "1", "pipe:1"])
            .process_group(0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| format!("Cannot start embedded decoder: {e}"))?;
        let mut stdin = child.stdin.take().unwrap();
        let path = self.socket();
        let cancelled = self.cancelled.clone();
        let source_frames = self.frames.clone();
        self.pump = Some(thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut buffer = [0; 16384];
            'connect: while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
                let Ok(mut socket) = connect_local(&path) else { thread::sleep(Duration::from_millis(20)); continue; };
                let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
                let mut received = false;
                while !cancelled.load(Ordering::Acquire) {
                    match socket.read(&mut buffer) {
                        Ok(0) if !received => { thread::sleep(Duration::from_millis(20)); continue 'connect; }
                        Ok(0) => break 'connect,
                        Ok(n) => { received = true; if stdin.write_all(&buffer[..n]).is_err() { break 'connect; } }
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                            if !received && Instant::now() >= deadline { break 'connect; }
                        }
                        Err(_) => break 'connect,
                    }
                }
                break;
            }
            // Invalidate at transport EOF, before ffmpeg drains any final
            // buffered packets. Those late packets must not republish pixels.
            let mut state = source_frames.lock().unwrap_or_else(|e| e.into_inner());
            state.latest = None;
            state.ended = Some("Preview transport ended; select the device again to reconnect".into());
        }));
        let stdout = child.stdout.take().unwrap();
        drain(child.stderr.take().unwrap(), log);
        self.decoder = Some(child);
        let frames = self.frames.clone();
        self.reader = Some(thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let frame = ppm::read_frame(&mut input);
                let mut state = frames.lock().unwrap_or_else(|e| e.into_inner());
                match frame {
                    Ok(Some(frame)) => { if state.ended.is_none() { state.latest = Some(frame); } },
                    end => {
                        state.latest = None;
                        state.ended = Some(match end { Err(e) => format!("Preview decoding failed: {e}"), _ => "Preview stream ended; select the device again to reconnect".into() });
                        break;
                    }
                }
            }
        }));
        Ok(())
    }
    pub(super) fn error(&mut self) -> Option<String> {
        let ended = self.frames.lock().unwrap_or_else(|e| e.into_inner()).ended.clone();
        if ended.is_some() { return ended; }
        match self.decoder.as_mut().map(Child::try_wait) {
            Some(Ok(Some(exit))) => Some(format!("Preview decoder ended ({exit})")),
            Some(Err(e)) => Some(format!("Preview decoder status failed: {e}")),
            _ => None,
        }
    }
    pub(super) fn frame(&self) -> Option<Vec<u8>> {
        let state = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        if state.ended.is_some() { None } else { state.latest.clone() }
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(mut decoder) = self.decoder.take() { let _ = terminate(&mut decoder); }
        if let Some(pump) = self.pump.take() { let _ = pump.join(); }
        if let Some(reader) = self.reader.take() { let _ = reader.join(); }
        if let Some((adb, serial, remote)) = self.adb.take() {
            let mut remove = Command::new(&adb);
            remove.args(["-s", &serial, "forward", "--remove", &format!("localfilesystem:{}", self.socket().display())]);
            let _ = run_bounded(&mut remove, Duration::from_millis(500));
            let mut remove = Command::new(adb);
            remove.args(["-s", &serial, "shell", "rm", "-f", &remote]);
            let _ = run_bounded(&mut remove, Duration::from_millis(500));
        }
        let _ = fs::remove_file(self.socket());
        let _ = fs::remove_dir(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_eof_invalidates_last_frame_and_drop_removes_private_directory() {
        let tools = tempfile::tempdir().unwrap();
        let decoder = tools.path().join("ffmpeg");
        fs::write(&decoder, "#!/bin/sh\nprintf 'P6\\n1 1\\n255\\nabc'\n/bin/sleep 0.15\n").unwrap();
        fs::set_permissions(&decoder, fs::Permissions::from_mode(0o700)).unwrap();
        let mut preview = Preview::prepare().unwrap();
        let directory = preview.directory.clone();
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
        preview.start(&decoder, Arc::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while preview.frame().is_none() && Instant::now() < deadline { thread::sleep(Duration::from_millis(2)); }
        assert_eq!(preview.frame().unwrap(), b"P6\n1 1\n255\nabc");
        while preview.error().is_none() && Instant::now() < deadline { thread::sleep(Duration::from_millis(2)); }
        assert!(preview.error().is_some());
        // EOF is read concurrently; wait until its invalidation has completed.
        while preview.frame().is_some() && Instant::now() < deadline { thread::sleep(Duration::from_millis(2)); }
        assert!(preview.frame().is_none());
        drop(preview);
        assert!(!directory.exists());
    }
    #[test]
    fn stopping_a_decoder_waiting_for_unix_stream_is_bounded_and_reaps_it() {
        let Some(ffmpeg) = find_executable("ffmpeg") else { return; };
        let mut preview = Preview::prepare().unwrap();
        preview.start(&ffmpeg, Arc::default()).unwrap();
        let pid = preview.decoder.as_ref().unwrap().id();
        let directory = preview.directory.clone();
        let started = Instant::now();
        drop(preview);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert!(!directory.exists());
    }
}
