//! What was copied lately, kept in memory only: never written anywhere, never logged.
//!
//! `wl-paste --watch` says when the clipboard changes; each change is read once,
//! bounded, and skipped when a password manager marked it or it is not text. Without
//! `wl-clipboard` there is no history and the palette says so once.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Unpinned entries kept; pinned ones are kept besides.
pub const MAX_ENTRIES: usize = 50;
/// Bytes kept of one entry; longer text is cut there and marked.
pub const MAX_BYTES: usize = 64 * 1024;
/// Password managers mark what they put on the clipboard with this type.
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
/// How often the fallback looks while the palette is open.
const POLL_EVERY: Duration = Duration::from_secs(2);
const HELPER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Stable through moves and removals, for the palette's context menu.
    pub id: u64,
    pub text: String,
    /// Whether the text was longer than [`MAX_BYTES`] and cut.
    pub truncated: bool,
    pub pinned: bool,
}

impl Entry {
    /// The first line, control characters out, at most `max` characters.
    pub fn first_line(&self, max: usize) -> String {
        let line = self.text.lines().find(|line| !line.trim().is_empty()).unwrap_or("");
        let cleaned: String = line.chars().filter(|c| !c.is_control()).collect();
        let shown: String = cleaned.chars().take(max).collect();
        if shown.chars().count() < cleaned.chars().count() { format!("{shown}…") } else { shown }
    }

    /// `n lines · k KB`, with what else there is to know.
    pub fn summary(&self) -> String {
        let lines = self.text.lines().count().max(1);
        let bytes = self.text.len();
        let size = if bytes < 1024 { format!("{bytes} B") } else { format!("{} KB", bytes / 1024) };
        let mut summary = format!("{lines} {} · {size}", if lines == 1 { "line" } else { "lines" });
        if self.truncated { summary.push_str(" · cut at 64 KB"); }
        if self.pinned { summary.push_str(" · pinned"); }
        summary
    }
}

/// The entries, newest first, pinned ones before the rest. Pure bookkeeping.
#[derive(Debug, Default)]
pub struct History {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl History {
    /// Takes note of new clipboard text. Blank text, and anything offered with the
    /// password manager's type, is left out; a repeat moves to the front.
    pub fn push(&mut self, text: &str, types: &[String]) -> bool {
        if types.iter().any(|t| t == PASSWORD_HINT) || text.trim().is_empty() {
            return false;
        }
        let (text, truncated) = cut(text);
        if let Some(index) = self.entries.iter().position(|entry| entry.text == text) {
            let entry = self.entries.remove(index).expect("position found");
            self.insert(entry);
            return true;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.insert(Entry { id, text, truncated, pinned: false });
        self.evict();
        true
    }

    /// Drops the oldest unpinned entries beyond [`MAX_ENTRIES`].
    fn evict(&mut self) {
        while self.entries.iter().filter(|entry| !entry.pinned).count() > MAX_ENTRIES {
            match self.entries.iter().rposition(|entry| !entry.pinned) {
                Some(index) => { self.entries.remove(index); }
                None => break,
            }
        }
    }

    /// Puts an entry at the front of its kind: pinned ones first, then the rest.
    fn insert(&mut self, entry: Entry) {
        let at = if entry.pinned { 0 } else { self.entries.iter().position(|e| !e.pinned).unwrap_or(self.entries.len()) };
        self.entries.insert(at, entry);
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn pin(&mut self, id: u64, pinned: bool) {
        if let Some(index) = self.entries.iter().position(|entry| entry.id == id) {
            let mut entry = self.entries.remove(index).expect("position found");
            entry.pinned = pinned;
            self.insert(entry);
            self.evict();
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.retain(|entry| entry.id != id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// At most [`MAX_BYTES`], cut on a character boundary and marked with an ellipsis.
fn cut(text: &str) -> (String, bool) {
    if text.len() <= MAX_BYTES {
        return (text.to_string(), false);
    }
    let mut end = MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &text[..end]), true)
}

/// Whether the types offered say there is text to read.
fn offers_text(types: &[String]) -> bool {
    types.iter().any(|t| t.starts_with("text/plain") || matches!(t.as_str(), "TEXT" | "STRING" | "UTF8_STRING"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Availability {
    /// `wl-paste --watch` is telling us about changes.
    Watching,
    /// Only looked at now and then while the palette is open.
    Polling,
    /// `wl-clipboard` is not installed.
    Missing,
}

/// A handle the palette reads and acts through; cloning shares the history.
#[derive(Clone)]
pub struct Handle {
    history: Arc<Mutex<History>>,
    availability: Arc<Mutex<Availability>>,
}

impl Handle {
    pub fn availability(&self) -> Availability {
        *self.availability.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A copy of the entries, newest first, pinned ones first.
    pub fn entries(&self) -> Vec<Entry> {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).entries().cloned().collect()
    }

    pub fn pin(&self, id: u64, pinned: bool) {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).pin(id, pinned);
    }

    pub fn remove(&self, id: u64) {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    pub fn clear(&self) {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// For tests and the fallback reader: a change seen by other means.
    pub fn push(&self, text: &str, types: &[String]) -> bool {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).push(text, types)
    }
}

/// Owns the watcher. The shell keeps one and calls [`Clipboard::poll`] each frame.
pub struct Clipboard {
    handle: Handle,
    program: Option<PathBuf>,
    watcher: Option<Child>,
    last_poll: Instant,
    reading: Option<std::thread::JoinHandle<()>>,
    ctx: eframe::egui::Context,
}

impl Clipboard {
    pub fn start(ctx: eframe::egui::Context) -> Clipboard {
        let program = find_program("wl-paste");
        let handle = Handle {
            history: Arc::new(Mutex::new(History::default())),
            availability: Arc::new(Mutex::new(if program.is_some() { Availability::Polling } else { Availability::Missing })),
        };
        let mut clipboard = Clipboard { handle, program, watcher: None, last_poll: Instant::now() - POLL_EVERY, reading: None, ctx };
        clipboard.watch();
        clipboard
    }

    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Starts `wl-paste --watch`, which runs `echo` at each change: one line per
    /// change on its output, nothing of the clipboard itself.
    fn watch(&mut self) {
        let Some(program) = &self.program else { return };
        let child = Command::new(program)
            .args(["--watch", "echo"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return };
        let Some(stdout) = child.stdout.take() else { return };
        self.watcher = Some(child);
        *self.handle.availability.lock().unwrap_or_else(|e| e.into_inner()) = Availability::Watching;
        let (handle, program, ctx) = (self.handle.clone(), program.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut line = String::new();
            // The clipboard as it is now, then each change.
            read_once(&program, &handle);
            ctx.request_repaint();
            loop {
                line.clear();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        read_once(&program, &handle);
                        ctx.request_repaint();
                    }
                }
            }
            *handle.availability.lock().unwrap_or_else(|e| e.into_inner()) = Availability::Polling;
        });
    }

    /// Called each frame. Without a watcher, looks every two seconds while the
    /// palette is open, one look at a time.
    pub fn poll(&mut self, palette_open: bool) {
        if let Some(watcher) = &mut self.watcher {
            if matches!(watcher.try_wait(), Ok(Some(_)) | Err(_)) {
                self.watcher = None;
                *self.handle.availability.lock().unwrap_or_else(|e| e.into_inner()) = Availability::Polling;
            }
        }
        if self.reading.as_ref().is_some_and(|reading| reading.is_finished()) {
            self.reading = None;
        }
        if self.watcher.is_some() || !palette_open || self.reading.is_some() || self.last_poll.elapsed() < POLL_EVERY {
            return;
        }
        let Some(program) = self.program.clone() else { return };
        self.last_poll = Instant::now();
        let (handle, ctx) = (self.handle.clone(), self.ctx.clone());
        self.reading = Some(std::thread::spawn(move || {
            if read_once(&program, &handle) {
                ctx.request_repaint();
            }
        }));
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        if let Some(mut watcher) = self.watcher.take() {
            let _ = watcher.kill();
            let _ = watcher.wait();
        }
    }
}

/// Reads the clipboard once: its types first, then the text if there is any and no
/// password manager put it there. Whether the history changed.
fn read_once(program: &PathBuf, handle: &Handle) -> bool {
    let Some(types) = run_bounded(program, &["--list-types"], 4096) else { return false };
    let types: Vec<String> = String::from_utf8_lossy(&types).lines().map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
    if types.iter().any(|t| t == PASSWORD_HINT) || !offers_text(&types) {
        return false;
    }
    let Some(bytes) = run_bounded(program, &["--no-newline", "--type", "text/plain"], MAX_BYTES + 1) else { return false };
    handle.push(&String::from_utf8_lossy(&bytes), &types)
}

/// Runs a helper and takes at most `limit` bytes of what it prints, within a
/// deadline; the helper is ended either way.
fn run_bounded(program: &PathBuf, args: &[&str], limit: usize) -> Option<Vec<u8>> {
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let done = Arc::new(Mutex::new(false));
    let killer = {
        let (done, pid) = (done.clone(), child.id());
        std::thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < HELPER_TIMEOUT {
                std::thread::sleep(Duration::from_millis(50));
                if *done.lock().unwrap_or_else(|e| e.into_inner()) { return; }
            }
            // Past the deadline: end it, which also ends the read below.
            let _ = rustix::process::kill_process(rustix::process::Pid::from_raw(pid as i32).expect("child pid"), rustix::process::Signal::KILL);
        })
    };
    let mut bytes = Vec::new();
    let read = (&mut stdout).take(limit as u64).read_to_end(&mut bytes);
    *done.lock().unwrap_or_else(|e| e.into_inner()) = true;
    let _ = child.kill();
    let status = child.wait();
    let _ = killer.join();
    match (read, status) {
        (Ok(_), Ok(status)) if status.success() || bytes.len() >= limit => Some(bytes),
        _ => None,
    }
}

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|dir| PathBuf::from(dir).join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_types() -> Vec<String> {
        vec!["text/plain;charset=utf-8".into(), "TEXT".into()]
    }

    #[test]
    fn keeps_fifty_newest_and_moves_a_repeat_to_the_front() {
        let mut history = History::default();
        for i in 0..60 {
            assert!(history.push(&format!("entry {i}"), &text_types()));
        }
        let texts: Vec<_> = history.entries().map(|e| e.text.as_str()).collect();
        assert_eq!(texts.len(), MAX_ENTRIES);
        assert_eq!(texts[0], "entry 59");
        assert_eq!(texts[49], "entry 10");
        assert!(history.push("entry 30", &text_types()));
        assert_eq!(history.entries().next().unwrap().text, "entry 30");
        assert_eq!(history.entries().count(), MAX_ENTRIES);
    }

    #[test]
    fn blank_text_and_password_manager_copies_are_left_out() {
        let mut history = History::default();
        assert!(!history.push("   \n\t", &text_types()));
        assert!(!history.push("hunter2", &["text/plain".to_string(), PASSWORD_HINT.to_string()]));
        assert!(history.is_empty());
        assert!(!offers_text(&["image/png".to_string()]));
        assert!(offers_text(&text_types()));
    }

    #[test]
    fn pinned_entries_stay_first_and_survive_eviction() {
        let mut history = History::default();
        history.push("keep me", &text_types());
        let id = history.entries().next().unwrap().id;
        history.pin(id, true);
        for i in 0..70 {
            history.push(&format!("noise {i}"), &text_types());
        }
        let first = history.entries().next().unwrap();
        assert_eq!((first.text.as_str(), first.pinned), ("keep me", true));
        assert_eq!(history.entries().count(), MAX_ENTRIES + 1);
        // Unpinned, it is the newest of the rest, and the oldest noise makes room.
        history.pin(id, false);
        assert_eq!(history.entries().next().unwrap().text, "keep me");
        assert_eq!(history.entries().count(), MAX_ENTRIES);
        assert!(history.entries().all(|e| e.text != "noise 20"));
        history.remove(id);
        assert!(history.entries().all(|e| e.text != "keep me"));
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn long_text_is_cut_at_a_character_boundary_and_marked() {
        let mut history = History::default();
        let long = "é".repeat(MAX_BYTES);
        history.push(&long, &text_types());
        let entry = history.entries().next().unwrap();
        assert!(entry.truncated);
        assert!(entry.text.len() <= MAX_BYTES + '…'.len_utf8());
        assert!(entry.text.ends_with('…'));
        assert!(entry.summary().contains("cut at 64 KB"));
    }

    #[test]
    fn first_line_is_clean_and_short() {
        let entry = Entry { id: 1, text: "\n\n  \x1b[31mred\x07 text\nsecond".into(), truncated: false, pinned: true };
        assert_eq!(entry.first_line(80), "  [31mred text");
        assert_eq!(entry.first_line(5), "  [31…");
        assert_eq!(entry.summary(), "4 lines · 25 B · pinned");
    }

    #[test]
    fn the_module_never_touches_the_disk() {
        let source = include_str!("clipboard.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["std::fs", "File::", "OpenOptions", "write_all", "fs::"] {
            assert!(!body.contains(forbidden), "{forbidden} in clipboard.rs");
        }
    }

    #[test]
    fn a_missing_helper_means_no_history_and_no_watcher() {
        let ctx = eframe::egui::Context::default();
        let saved = std::env::var_os("PATH");
        // SAFETY: tests in this module run with the lock below; PATH is restored at once.
        let _guard = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("PATH", "/nonexistent") };
        let mut clipboard = Clipboard::start(ctx);
        unsafe { match saved { Some(path) => std::env::set_var("PATH", path), None => std::env::remove_var("PATH") } }
        assert_eq!(clipboard.handle().availability(), Availability::Missing);
        clipboard.poll(true);
        assert!(clipboard.watcher.is_none() && clipboard.reading.is_none());
    }

    static PATH_LOCK: Mutex<()> = Mutex::new(());
}
