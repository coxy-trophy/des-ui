//! The file manager's search: everything under a folder whose name, or whose
//! text, holds the words asked for. It walks on a thread of its own, sending what
//! it finds as it goes, within bounds, and stops as soon as it is dropped.

use eframe::egui;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Something found, and the line of text that matched when it was the contents.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit { pub path: PathBuf, pub directory: bool, pub bytes: u64, pub modified: Option<SystemTime>, pub line: Option<String> }

const MAX_HITS: usize = 5_000;
const MAX_DEPTH: usize = 16;
const MAX_TIME: Duration = Duration::from_secs(30);
/// Larger files are matched by name only.
const MAX_TEXT: u64 = 1024 * 1024;
/// Folders full of what people did not write, skipped unless searched from inside.
const SKIP: [&str; 9] = ["node_modules", "target", ".git", ".cache", "__pycache__", ".venv", "venv", ".cargo", ".rustup"];

enum Message { Hits(Vec<Hit>), Done { limited: bool } }

pub struct Search {
    pub query: String,
    pub root: PathBuf,
    pub hits: Vec<Hit>,
    pub done: bool,
    /// Stopped at a bound before the end.
    pub limited: bool,
    receiver: Receiver<Message>,
    stop: Arc<AtomicBool>,
}

impl Drop for Search { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); } }

impl Search {
    pub fn start(root: PathBuf, query: &str, hidden: bool, ctx: &egui::Context) -> Search {
        let (send, receiver) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (from, words, halt, wake) = (root.clone(), words(query), stop.clone(), ctx.clone());
        std::thread::Builder::new().name("files-search".into()).spawn(move || walk(&from, &words, hidden, &halt, &send, &wake)).ok();
        Search { query: query.trim().to_string(), root, hits: Vec::new(), done: false, limited: false, receiver, stop }
    }

    /// Takes in what was found since last time; true when anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(message) = self.receiver.try_recv() {
            changed = true;
            match message {
                Message::Hits(hits) => self.hits.extend(hits),
                Message::Done { limited } => { self.done = true; self.limited = limited; },
            }
        }
        changed
    }
}

fn words(query: &str) -> Vec<String> { query.to_lowercase().split_whitespace().map(str::to_string).collect() }

/// Every word somewhere in `text`, in any case.
pub fn matches(text: &str, words: &[String]) -> bool {
    let text = text.to_lowercase();
    !words.is_empty() && words.iter().all(|word| text.contains(word.as_str()))
}

/// The first line of a text file holding every word, cut to a readable length.
pub fn matching_line(path: &Path, words: &[String]) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_TEXT).read_to_end(&mut bytes).ok()?;
    if !crate::quicklook::looks_like_text(&bytes[..bytes.len().min(4096)]) { return None; }
    let text = String::from_utf8_lossy(&bytes);
    let line = text.lines().find(|line| matches(line, words))?.trim();
    let mut cut: String = line.chars().take(140).collect();
    if line.chars().count() > 140 { cut.push('…'); }
    Some(cut)
}

fn walk(root: &Path, words: &[String], hidden: bool, stop: &AtomicBool, send: &Sender<Message>, ctx: &egui::Context) {
    let started = Instant::now();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut batch = Vec::new();
    let mut found = 0usize;
    let mut sent_at = Instant::now();
    let mut limited = false;
    let content = words.iter().map(|w| w.chars().count()).sum::<usize>() >= 3;
    'walk: while let Some((folder, depth)) = stack.pop() {
        let Ok(listing) = std::fs::read_dir(&folder) else { continue };
        for item in listing.flatten() {
            if stop.load(Ordering::Relaxed) { return; }
            if found >= MAX_HITS || started.elapsed() > MAX_TIME { limited = true; break 'walk; }
            let name = item.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') { continue; }
            let Ok(kind) = item.file_type() else { continue };
            let path = item.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIP.contains(&name.as_str()) { stack.push((path.clone(), depth + 1)); } else if depth >= MAX_DEPTH { limited = true; }
            }
            let named = matches(&name, words);
            let line = if !named && content && kind.is_file() && item.metadata().is_ok_and(|m| m.len() <= MAX_TEXT) { matching_line(&path, words) } else { None };
            if named || line.is_some() {
                let metadata = item.metadata().ok();
                batch.push(Hit { path, directory: kind.is_dir(), bytes: if kind.is_file() { metadata.as_ref().map_or(0, |m| m.len()) } else { 0 },
                    modified: metadata.and_then(|m| m.modified().ok()), line });
                found += 1;
            }
            if !batch.is_empty() && sent_at.elapsed() > Duration::from_millis(80) {
                if send.send(Message::Hits(std::mem::take(&mut batch))).is_err() { return; }
                ctx.request_repaint(); sent_at = Instant::now();
            }
        }
    }
    if !batch.is_empty() { let _ = send.send(Message::Hits(batch)); }
    let _ = send.send(Message::Done { limited });
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(mut search: Search) -> Search {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !search.done { search.pump(); assert!(Instant::now() < deadline, "search did not finish"); std::thread::sleep(Duration::from_millis(5)); }
        search
    }
    #[test]
    fn finds_names_and_lines_inside_text_but_skips_hidden_and_build_folders() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("Work/Budgets")).unwrap();
        std::fs::create_dir_all(p.join("target/debug")).unwrap();
        std::fs::create_dir_all(p.join(".secret")).unwrap();
        std::fs::write(p.join("Work/budget-2026.ods"), b"PK\x03\x04binary").unwrap();
        std::fs::write(p.join("Work/notes.md"), b"# Notes\nMove the travel Budget into Q1\n").unwrap();
        std::fs::write(p.join("target/debug/budget.txt"), b"x").unwrap();
        std::fs::write(p.join(".secret/budget.txt"), b"x").unwrap();
        std::fs::write(p.join("picture.png"), [0x89u8, b'P', b'N', b'G', 0, 0, 0, 0]).unwrap();
        let search = finish(Search::start(p.into(), "budget", false, &egui::Context::default()));
        let mut names: Vec<String> = search.hits.iter().map(|h| h.path.strip_prefix(p).unwrap().display().to_string()).collect();
        names.sort();
        assert_eq!(names, ["Work/Budgets", "Work/budget-2026.ods", "Work/notes.md"]);
        let notes = search.hits.iter().find(|h| h.path.ends_with("notes.md")).unwrap();
        assert_eq!(notes.line.as_deref(), Some("Move the travel Budget into Q1"));
        assert!(!search.limited);
        let hidden = finish(Search::start(p.into(), "budget", true, &egui::Context::default()));
        assert!(hidden.hits.iter().any(|h| h.path.ends_with(".secret/budget.txt")), "hidden folders are searched when shown");
        assert!(matches("Travel BUDGET q1", &words("budget q1")) && !matches("budget", &words("budget q1")));
    }
}
