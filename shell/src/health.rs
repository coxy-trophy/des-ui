//! Settings → Health: read-only probes of the compositor and music service, the
//! helper programs the desktop calls, and the session logs, shown as bounded
//! excerpts with secret-bearing lines left out. Nothing here changes the system.

use crate::{theme::Theme, ui};
use eframe::egui::{self, Ui};
use std::{fs, io::{Read, Seek, SeekFrom}, os::unix::fs::{OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}, sync::mpsc};

const MAX_BYTES: usize = 8192;
const MAX_LINES: usize = 40;
const HELPERS: &[&str] = &["des-comp", "des-music", "wpctl", "brightnessctl", "nmcli", "bluetoothctl", "xdg-open", "ffmpeg", "wl-copy", "unix_chkpwd"];

#[derive(Debug)]
enum LogReading { Ready(String), Missing, Failed(String) }
struct Log { path: PathBuf, reading: LogReading }
struct Snapshot { helpers: Vec<(&'static str, Option<PathBuf>)>, logs: Vec<Log> }

/// Only local status is retained; there is deliberately no diagnostics export.
#[derive(Default)]
pub struct Health {
    snapshot: Option<Snapshot>,
    pending: Option<mpsc::Receiver<Snapshot>>,
    error: Option<String>,
    opener: Option<mpsc::Receiver<Result<(), String>>>,
}

fn log_paths(root: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(root) = root.filter(|p| p.is_absolute()) else { return Vec::new() };
    ["session.log", "session-fallback.log", "apps.log"].into_iter().map(|name| root.join("des-ui").join(name)).collect()
}

fn sanitize_excerpt(bytes: &[u8]) -> String {
    let tail = &bytes[bytes.len().saturating_sub(MAX_BYTES)..];
    let text = String::from_utf8_lossy(tail);
    let mut lines: Vec<_> = text.lines().rev().take(MAX_LINES).map(|line| {
        let lower = line.to_ascii_lowercase();
        if ["password", "passwd", "token", "secret", "authorization", "cookie", "private key", "api_key", "apikey", "credential"].iter().any(|word| lower.contains(word)) {
            "[sensitive line omitted]".to_string()
        } else {
            line.chars().filter(|c| !c.is_control() || *c == '\t').collect::<String>()
        }
    }).collect();
    lines.reverse();
    let mut out = lines.join("\n");
    if out.len() > MAX_BYTES {
        let mut boundary = out.len() - MAX_BYTES;
        while !out.is_char_boundary(boundary) { boundary += 1; }
        out.drain(..boundary);
    }
    out
}

fn read_log(path: &Path) -> LogReading {
    // O_NOFOLLOW/O_NONBLOCK prevent final symlinks and named pipes from blocking a worker.
    let result = (|| -> std::io::Result<String> {
        let mut file = fs::OpenOptions::new().read(true).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32 | rustix::fs::OFlags::NONBLOCK.bits() as i32).open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() { return Err(std::io::Error::other("not a regular log file")); }
        let offset = metadata.len().saturating_sub(MAX_BYTES as u64);
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64).read_to_end(&mut bytes)?;
        // A partial first line could hide a secret's label: always discard it.
        if offset > 0 {
            match bytes.iter().position(|b| *b == b'\n') {
                Some(end) => { bytes.drain(..=end); }
                None => bytes.clear(),
            }
        }
        Ok(sanitize_excerpt(&bytes))
    })();
    match result {
        Ok(text) => LogReading::Ready(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => LogReading::Missing,
        Err(error) => LogReading::Failed(error.to_string()),
    }
}

fn probe() -> Snapshot {
    let search = std::env::var_os("PATH").unwrap_or_default();
    let helpers = HELPERS.iter().map(|&name| {
        let found = std::env::split_paths(&search).filter(|p| p.is_absolute()).take(64)
            .map(|dir| dir.join(name)).find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0));
        (name, found)
    }).collect();
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".local/state")));
    let logs = log_paths(state).into_iter().map(|path| Log { reading: read_log(&path), path }).collect();
    Snapshot { helpers, logs }
}

impl Health {
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() { return; }
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || { let _ = sender.send(probe()); ctx.request_repaint(); });
    }

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, connected: Option<bool>, music:&str) {
        crate::settings::heading(ui, "DESKTOP HEALTH", "Read-only checks. Refresh to check this machine again.");
        ui::text(ui, match connected { Some(true) => "Compositor: connected", Some(false) => "Compositor: disconnected", None => "Compositor: unknown" }, ui::mono(13.0), ui::TEXT);
        ui::text(ui,&format!("Music service: {music}"),ui::mono(13.0),ui::TEXT);
        ui.add_space(12.0);
        if self.snapshot.is_none() && self.pending.is_none() && self.error.is_none() { self.refresh(ui.ctx()); }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(snapshot) => { self.snapshot = Some(snapshot); self.pending = None; }
                Err(mpsc::TryRecvError::Disconnected) => { self.pending = None; self.error = Some("Health check failed. Refresh to retry.".into()); }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_some() { ui::text(ui, "Checking helpers and logs…", ui::mono(12.0), ui::SECONDARY); }
        let busy = self.pending.is_some();
        ui.add_enabled_ui(!busy, |ui| { if ui::button(ui, theme, "REFRESH", None, false).clicked() { self.refresh(ui.ctx()); } });
        if let Some(error) = &self.error { ui::text(ui, error, ui::mono(12.0), ui::CAUTION); }
        if self.opener.is_some(){ui::text(ui,"Opening viewer…",ui::mono(12.0),ui::SECONDARY);}
        if let Some(receiver) = &self.opener {
            match receiver.try_recv(){
                Ok(result)=>{self.opener=None;if let Err(error)=result{self.error=Some(error);}},
                Err(mpsc::TryRecvError::Disconnected)=>{self.opener=None;self.error=Some("Document opener stopped unexpectedly.".into());},
                Err(mpsc::TryRecvError::Empty)=>{},
            }
        }
        let Some(snapshot) = &self.snapshot else { return };
        ui.add_space(20.0);
        crate::settings::heading(ui, "NATIVE HELPERS", "Availability on PATH; no helpers are run by these checks.");
        for (name, path) in &snapshot.helpers {
            let status = path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "missing".into());
            ui::text(ui, &format!("{name}: {status}"), ui::mono(12.0), ui::TEXT);
            ui.add_space(6.0);
        }
        ui.add_space(20.0);
        crate::settings::heading(ui, "RECENT LOGS", "Up to 40 lines / 8 KiB each. Common secret-bearing lines are hidden; raw logs can contain private data.");
        if snapshot.logs.is_empty() { ui::text(ui, "Log location unknown: no absolute state or home directory.", ui::mono(12.0), ui::SECONDARY); }
        for log in &snapshot.logs {
            ui::text(ui, &log.path.display().to_string(), ui::mono(12.0), ui::TEXT);
            match &log.reading {
                LogReading::Missing => { ui::text(ui, "Missing — no log at this location.", ui::mono(12.0), ui::SECONDARY); }
                LogReading::Failed(error) => { ui::text(ui, &format!("Could not read log: {error}"), ui::mono(12.0), ui::CAUTION); }
                LogReading::Ready(text) => {
                    let can_open=self.opener.is_none();
                    if ui.add_enabled_ui(can_open,|ui|ui::button(ui,theme,"OPEN LOG",None,false)).inner.clicked(){
                        self.opener=Some(crate::apps::open_document(log.path.clone(),ui.ctx().clone()));
                    }
                    egui::ScrollArea::vertical().id_salt(&log.path).max_height(180.0).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(if text.is_empty() { "No recent lines." } else { text }).font(ui::mono(11.0)).color(ui::SECONDARY)).wrap());
                    });
                }
            }
            ui.add_space(18.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpt_is_bounded_and_redacts_sensitive_lines() {
        let input = format!("{}\npassword=hunter2\nAuthorization: Bearer abc\nTOKEN=abc\nordinary event\n", "x".repeat(MAX_BYTES * 2));
        let excerpt = sanitize_excerpt(input.as_bytes());
        assert!(excerpt.len() <= MAX_BYTES);
        assert!(!excerpt.contains("hunter2"));
        assert!(!excerpt.contains("Bearer abc"));
        assert!(!excerpt.contains("TOKEN=abc"));
        assert!(excerpt.contains("ordinary event"));
        assert!(excerpt.lines().count() <= MAX_LINES);
    }
    #[test]
    fn rejects_relative_roots_and_non_regular_logs() {
        assert!(log_paths(Some(std::path::PathBuf::from("relative"))).is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(read_log(dir.path()), LogReading::Failed(_)));
        let missing = dir.path().join("missing.log");
        assert!(matches!(read_log(&missing), LogReading::Missing));
        std::os::unix::fs::symlink("/dev/zero", &missing).unwrap();
        assert!(matches!(read_log(&missing), LogReading::Failed(_)));
    }
    #[test]
    fn tail_reads_recent_lines_and_bounds_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.log");
        std::fs::write(&path, format!("{}\nlast event\n", "old\n".repeat(MAX_BYTES))).unwrap();
        let LogReading::Ready(text) = read_log(&path) else { panic!("expected regular log") };
        assert!(text.ends_with("last event"));
        assert!(text.len() <= MAX_BYTES);
        assert!(text.lines().count() <= MAX_LINES);
    }
}
