//! Shared native folder browser and file-picker foundation.

use eframe::egui::{self, Key, Modifiers};
use std::{path::{Path, PathBuf}, sync::{Arc, Mutex, Condvar, atomic::{AtomicBool, AtomicU64, Ordering}, OnceLock}, time::{Duration, Instant}};
use crate::ui;
use crate::theme::Theme;

#[derive(Clone)]
struct Entry { path: PathBuf, directory: bool, bytes: u64, modified: Option<std::time::SystemTime> }
struct Listing { path: PathBuf, entries: Vec<Entry>, limited: bool, trash: bool, labels: std::collections::HashMap<PathBuf, String> }
fn read_directory(path: &Path) -> Result<Listing, String> {
    let trash_root = crate::file_ops::trash_files();
    let path = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && trash_root.as_deref() == Some(path) => return Ok(Listing { path: path.into(), entries: Vec::new(), limited: false, trash: true, labels: Default::default() }),
        Err(error) => return Err(format!("Could not open folder: {error}")),
    };
    let trash = trash_root.and_then(|root| std::fs::canonicalize(root).ok()).is_some_and(|root| root == path);
    let started = Instant::now();
    let mut entries = Vec::new(); let mut limited = false; let mut labels = std::collections::HashMap::new();
    for (index, item) in std::fs::read_dir(&path).map_err(|e| format!("Could not open folder: {e}"))?.enumerate() {
        if index >= 10_000 || started.elapsed() > Duration::from_millis(500) { limited = true; break; }
        let item = item.map_err(|e| format!("Could not read an entry: {e}"))?;
        let file_type = item.file_type().map_err(|e| e.to_string())?;
        let directory = file_type.is_dir() || (file_type.is_symlink() && std::fs::metadata(item.path()).is_ok_and(|m| m.is_dir()));
        let metadata = item.metadata().ok();
        let bytes = if file_type.is_file() { metadata.as_ref().map_or(0, |m| m.len()) } else { 0 };
        let modified = metadata.and_then(|m| m.modified().ok());
        if trash {
            if let Some(name) = crate::file_ops::trash_original_name(&item.path()) {
                use std::os::unix::ffi::OsStrExt;
                if name.as_bytes().len() <= 1024 { labels.insert(item.path(), name.to_string_lossy().into_owned()); }
            }
        }
        entries.push(Entry { path: item.path(), directory, bytes, modified });
    }
    entries.sort_by(|a, b| (!a.directory, a.path.file_name().unwrap_or_default().to_string_lossy().to_lowercase(), &a.path)
        .cmp(&(!b.directory, b.path.file_name().unwrap_or_default().to_string_lossy().to_lowercase(), &b.path)));
    Ok(Listing { path, entries, limited, trash, labels })
}
type Answer = (u64, Result<Listing, String>);
struct Job { client: u64, generation: u64, path: PathBuf, ctx: egui::Context, answer: Arc<Mutex<Option<Answer>>>, closed: Arc<AtomicBool> }
struct Service { pending: Mutex<std::collections::VecDeque<Job>>, ready: Condvar }
fn service() -> Result<&'static Arc<Service>, String> {
    static SERVICE: OnceLock<Result<Arc<Service>, String>> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let service = Arc::new(Service { pending: Default::default(), ready: Condvar::new() });
        let worker = service.clone();
        std::thread::Builder::new().name("native-files".into()).spawn(move || loop {
            let job = {
                let mut pending = worker.pending.lock().unwrap();
                while pending.is_empty() { pending = worker.ready.wait(pending).unwrap(); }
                pending.pop_front().unwrap()
            };
            if job.closed.load(Ordering::Acquire) { continue; }
            let result = read_directory(&job.path);
            if !job.closed.load(Ordering::Acquire) {
                *job.answer.lock().unwrap() = Some((job.generation, result)); job.ctx.request_repaint();
            }
        }).map_err(|e| format!("Could not start file browsing: {e}"))?;
        Ok(service)
    }).as_ref().map_err(Clone::clone)
}
static CLIENT: AtomicU64 = AtomicU64::new(1);
struct Worker { service: &'static Arc<Service>, client: u64, answer: Arc<Mutex<Option<Answer>>>, closed: Arc<AtomicBool> }
impl Worker {
    fn new() -> Result<Self, String> {
        Ok(Self { service: service()?, client: CLIENT.fetch_add(1, Ordering::Relaxed), answer: Default::default(), closed: Arc::new(AtomicBool::new(false)) })
    }
    fn submit(&self, generation: u64, path: PathBuf, ctx: egui::Context) -> Result<(), String> {
        let mut pending = self.service.pending.lock().unwrap();
        pending.retain(|job| job.client != self.client && !job.closed.load(Ordering::Acquire));
        if pending.len() >= 4 { return Err("File browser is busy; try Refresh in a moment".into()); }
        pending.push_back(Job { client: self.client, generation, path, ctx, answer: self.answer.clone(), closed: self.closed.clone() });
        self.service.ready.notify_one(); Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.service.pending.lock().unwrap().retain(|job| job.client != self.client);
    }
}
#[derive(Clone, Copy, PartialEq)]
pub enum Mode { Browse, ImagePicker }
#[derive(Clone, Copy)]
enum Navigation { New, History(usize), Refresh }
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard { pub sources: Vec<PathBuf>, pub cut: bool }
pub enum Event {
    Open(PathBuf), Choose(PathBuf), Drag(PathBuf), Close, Clipboard(Clipboard),
    /// Operations to run one after another, in order.
    Operate(Vec<crate::file_ops::Operation>),
    CancelOperation,
    /// Open a terminal tab in this folder.
    Terminal(PathBuf),
    /// Open this folder in a new tab.
    NewTab(PathBuf),
    /// Put back what the last batch of work changed.
    Undo,
    /// The answer to a name already taken, and whether it is for the rest too.
    Resolve(crate::file_ops::Conflict, bool),
    PauseTransfers(bool),
    ClearTransfers,
    /// Offer these files to a phone or tablet through DES Connect.
    SendTo(String, Vec<PathBuf>),
}
enum DialogKind { NewFolder, Rename(PathBuf), Trash(Vec<PathBuf>) }
struct OperationDialog { kind: DialogKind, name: String, pending: bool, focused: bool, error: Option<String> }
impl OperationDialog { fn new(kind: DialogKind, name: String) -> Self { Self { kind, name, pending: false, focused: false, error: None } } }
#[derive(Clone, Copy, PartialEq, Debug)]
enum SortKey { Name, Kind, Modified, Size }
#[derive(Clone, Copy, PartialEq, Debug)]
enum View { List, Grid }

/// What a click, key or menu asked for this frame; one place carries them out.
#[derive(Default)]
struct Act { open: bool, new_folder: bool, rename: bool, copy: bool, cut: bool, paste: bool, trash: bool, restore: bool, undo: bool, properties: bool, open_with: bool, compress: bool, extract: bool, send: Option<String>, terminal: Option<PathBuf> }

pub struct Browser {
    path: PathBuf, path_text: String, entries: Vec<Entry>,
    /// The focused entry, which keys move and single actions act on.
    selected: Option<PathBuf>,
    /// Everything selected, the focused entry among them; empty means just the focused one.
    marked: Vec<PathBuf>,
    /// Where a Shift selection runs from.
    anchor: Option<PathBuf>,
    mode: Mode, filter: String, hidden: bool, history: Vec<PathBuf>, history_at: usize,
    worker: Option<Worker>, generation: u64, pending: Option<(PathBuf, Navigation)>,
    loading: bool, loaded: bool, error: Option<String>, limited: bool, in_trash: bool,
    clipboard: Option<Clipboard>, transfers: crate::transfers::View, transfers_open: bool, clash_all: bool,
    dialog: Option<OperationDialog>, menu_open: bool, paste_chord_handled: bool, labels: std::collections::HashMap<PathBuf, String>,
    sort: SortKey, descending: bool, editing_path: bool, focus_path: bool, focus_filter: bool,
    places: Vec<(&'static str, PathBuf)>, free: Option<u64>,
    look: crate::quicklook::QuickLook, look_path: Option<PathBuf>,
    /// The list's scroll and height last frame, to keep the focused row in view.
    list_offset: f32, list_height: f32, reveal: bool,
    view: View, grid_columns: usize, thumbs: crate::thumbs::Thumbs,
    /// A drive being mounted, to open when it is, or ejected.
    mounting: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    ejecting: Option<(Option<PathBuf>, std::sync::mpsc::Receiver<Result<(), String>>)>,
    /// A passing word in the status line, such as that a drive can be pulled out.
    message: Option<(String, Instant)>,
    /// A search of everything under this folder, what it found as entries, and the
    /// line of text each matched on.
    search: Option<crate::deep_search::Search>, found: Vec<Entry>, lines: std::collections::HashMap<PathBuf, String>,
    /// An entry to select once its folder has been read.
    reveal_after: Option<PathBuf>,
    renaming: Option<crate::file_dialogs::BatchRename>, properties: Option<crate::file_dialogs::Properties>,
    open_with: Option<crate::file_dialogs::OpenWith>, compressing: Option<crate::file_dialogs::Compress>,
    /// Phones and tablets ready for files, as (id, name).
    phones: Vec<(String, String)>,
}
pub fn home() -> PathBuf { std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/")) }
fn image_path(path: &Path) -> bool { matches!(path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg") }
fn file_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{bytes} B") }
    else if bytes < 1_048_576 { format!("{:.1} KiB", bytes as f64 / 1024.0) }
    else if bytes < 1_073_741_824 { format!("{:.1} MiB", bytes as f64 / 1_048_576.0) }
    else { format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0) }
}
/// What kind of thing a file is, from its name, in a word or two.
fn kind_of(entry: &Entry) -> &'static str {
    if entry.directory { return "Folder"; }
    let extension = entry.path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif" => "Image",
        "pdf" => "PDF document",
        "txt" | "md" | "log" | "rst" | "org" => "Text",
        "odt" | "doc" | "docx" | "rtf" => "Document",
        "ods" | "xls" | "xlsx" => "Spreadsheet",
        "csv" | "tsv" => "Table",
        "odp" | "ppt" | "pptx" => "Presentation",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "7z" | "rar" => "Archive",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => "Audio",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "Video",
        "deb" | "rpm" | "appimage" => "Package",
        "rs" | "py" | "sh" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "toml" | "json" | "yaml" | "yml" | "html" | "css" => "Code",
        _ => "File",
    }
}
fn when(time: Option<std::time::SystemTime>) -> String {
    let Some(time) = time else { return String::new() };
    let local: chrono::DateTime<chrono::Local> = time.into();
    let now = chrono::Local::now();
    if local.date_naive() == now.date_naive() { format!("Today {}", local.format("%H:%M")) }
    else if (now.date_naive() - local.date_naive()).num_days() == 1 { "Yesterday".into() }
    else if local.format("%Y").to_string() == now.format("%Y").to_string() { local.format("%-d %b").to_string() }
    else { local.format("%-d %b %Y").to_string() }
}
fn free_space(path: &Path) -> Option<u64> {
    rustix::fs::statvfs(path).ok().map(|stat| stat.f_bavail.saturating_mul(stat.f_frsize))
}
fn items(count: usize) -> String { if count == 1 { "1 item".into() } else { format!("{count} items") } }
const ROW: f32 = 30.0;
const ROW_GAP: f32 = 2.0;

impl Browser {
    pub fn new(path: PathBuf, mode: Mode) -> Self {
        let path = if path.is_absolute() { path } else { home() };
        let home = home();
        let mut places = vec![("Home", home.clone())];
        for name in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos"] {
            let folder = home.join(name);
            if folder.is_dir() { places.push((name, folder)); }
        }
        Self { path_text: path.to_string_lossy().into_owned(), history: vec![path.clone()], path, entries: Vec::new(), selected: None, marked: Vec::new(), anchor: None,
            mode, filter: String::new(), hidden: false, history_at: 0, worker: None, generation: 0, pending: None,
            loading: false, loaded: false, error: None, limited: false, in_trash: false,
            clipboard: None, transfers: Default::default(), transfers_open: false, clash_all: true, dialog: None, menu_open: false, paste_chord_handled: false, labels: Default::default(),
            sort: SortKey::Name, descending: false, editing_path: false, focus_path: false, focus_filter: false,
            places, free: None, look: Default::default(), look_path: None, list_offset: 0.0, list_height: 0.0, reveal: false,
            view: if mode == Mode::ImagePicker { View::Grid } else { View::List }, grid_columns: 1, thumbs: Default::default(), mounting: None, ejecting: None, message: None, search: None, found: Vec::new(), lines: Default::default(), reveal_after: None, renaming: None, properties: None, open_with: None, compressing: None, phones: Vec::new() }
    }
    fn navigate(&mut self, ctx: &egui::Context, path: PathBuf, navigation: Navigation) {
        if !path.is_absolute() { self.error = Some("Use an absolute folder path".into()); return; }
        if self.worker.is_none() { match Worker::new() { Ok(worker) => self.worker = Some(worker), Err(error) => { self.error = Some(error); self.loaded = true; return; } } }
        self.generation += 1; self.pending = Some((path.clone(), navigation)); self.loading = true; self.loaded = true; self.error = None;
        if let Err(error) = self.worker.as_ref().unwrap().submit(self.generation, path, ctx.clone()) { self.loading = false; self.error = Some(error); }
    }
    fn accept(&mut self, generation: u64, result: Result<Listing, String>) {
        if generation != self.generation { return; }
        self.loading = false;
        match result {
            Ok(listing) => {
                let Some((_, navigation)) = self.pending.take() else { return; };
                let path = listing.path; self.in_trash = listing.trash; self.labels = listing.labels;
                let refresh = matches!(navigation, Navigation::Refresh) && path == self.path;
                match navigation {
                    Navigation::New if path != self.path => {
                        self.history.truncate(self.history_at + 1); self.history.push(path.clone());
                        if self.history.len() > 64 { self.history.remove(0); }
                        self.history_at = self.history.len() - 1;
                    },
                    Navigation::History(index) => self.history_at = index,
                    _ => {},
                }
                self.free = free_space(&path);
                self.path = path; self.path_text = self.path.to_string_lossy().into_owned();
                self.entries = listing.entries; self.limited = listing.limited; self.error = None;
                if refresh {
                    // Keep what is still there selected after a refresh.
                    let present = |p: &PathBuf| self.entries.iter().any(|e| &e.path == p);
                    self.marked.retain(|p| self.entries.iter().any(|e| &e.path == p));
                    if !self.selected.as_ref().is_some_and(present) { self.selected = None; }
                } else {
                    self.selected = None; self.marked.clear(); self.anchor = None; self.filter.clear(); self.list_offset = 0.0; self.thumbs.clear();
                    self.search = None; self.found.clear(); self.lines.clear();
                    if let Some(path) = self.reveal_after.take().filter(|p| self.entries.iter().any(|e| &e.path == p)) { self.select_only(path); self.reveal = true; }
                }
            },
            Err(error) => { self.pending = None; self.error = Some(error); self.path_text = self.path.to_string_lossy().into_owned(); },
        }
    }
    /// What the list shows: the folder, or what a search found.
    fn items(&self) -> &[Entry] { if self.search.is_some() { &self.found } else { &self.entries } }
    fn start_search(&mut self, ctx: &egui::Context) {
        let query = self.filter.trim();
        if query.chars().count() < 2 { return; }
        self.search = Some(crate::deep_search::Search::start(self.path.clone(), query, self.hidden, ctx));
        self.found.clear(); self.lines.clear(); self.selected = None; self.marked.clear(); self.list_offset = 0.0;
    }
    fn end_search(&mut self) {
        if self.search.take().is_some() { self.found.clear(); self.lines.clear(); self.selected = None; self.marked.clear(); }
    }
    /// Where a found entry is, from the folder searched.
    fn location_of(&self, entry: &Entry) -> Option<String> {
        let search = self.search.as_ref()?;
        let parent = entry.path.parent()?;
        Some(match parent.strip_prefix(&search.root) { Ok(rest) if rest.as_os_str().is_empty() => "here".into(), Ok(rest) => rest.display().to_string(), Err(_) => parent.display().to_string() })
    }
    fn name_of(&self, entry: &Entry) -> String {
        self.labels.get(&entry.path).cloned().unwrap_or_else(|| entry.path.file_name().unwrap_or_default().to_string_lossy().into_owned())
    }
    /// The entries shown, filtered, then sorted with folders first.
    fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let mut shown: Vec<usize> = self.items().iter().enumerate().filter(|(_, e)| {
            let name = self.name_of(e);
            (self.hidden || !name.starts_with('.')) && (self.search.is_some() || name.to_lowercase().contains(&needle))
                && (self.mode == Mode::Browse || e.directory || image_path(&e.path))
        }).map(|(i, _)| i).collect();
        shown.sort_by(|&a, &b| {
            let (x, y) = (&self.items()[a], &self.items()[b]);
            let name = || self.name_of(x).to_lowercase().cmp(&self.name_of(y).to_lowercase());
            let order = match self.sort {
                SortKey::Name => name(),
                SortKey::Kind => kind_of(x).cmp(kind_of(y)).then_with(name),
                SortKey::Modified => x.modified.cmp(&y.modified).then_with(name),
                SortKey::Size => x.bytes.cmp(&y.bytes).then_with(name),
            };
            y.directory.cmp(&x.directory).then(if self.descending { order.reverse() } else { order }).then_with(|| x.path.cmp(&y.path))
        });
        shown
    }
    fn can_choose(&self) -> bool {
        self.selected.as_ref().is_some_and(|path| self.items().iter().any(|e| &e.path == path && !e.directory && (self.mode == Mode::Browse || image_path(path))))
    }
    fn activate(&mut self, ctx: &egui::Context) -> Option<Event> {
        let index = self.items().iter().position(|e| Some(&e.path) == self.selected.as_ref())?;
        if !self.visible().contains(&index) { return None; }
        let entry = self.items()[index].clone();
        if entry.directory { self.navigate(ctx, entry.path, Navigation::New); None }
        else if self.can_choose() { Some(if self.mode == Mode::ImagePicker { Event::Choose(entry.path) } else { Event::Open(entry.path) }) }
        else { None }
    }
    pub fn captures_keyboard(&self) -> bool { self.dialog.is_some() || self.menu_open || self.transfers.clash.is_some() || self.renaming.is_some() || self.properties.is_some() || self.open_with.is_some() || self.compressing.is_some() }
    pub fn set_phones(&mut self, phones: Vec<(String, String)>) { self.phones = phones; }
    /// Reads the folder again, as after another tab changed it.
    pub fn refresh(&mut self, ctx: &egui::Context) { if self.loaded { self.navigate(ctx, self.path.clone(), Navigation::Refresh); } }
    /// What a tab showing this is called.
    pub fn title(&self) -> String {
        if self.in_trash { "Trash".into() } else if self.path == home() { "Home".into() }
        else { self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "File system".into()) }
    }
    pub fn configure_operations(&mut self, clipboard: Option<Clipboard>, transfers: crate::transfers::View) {
        // A new question brings the panel up, so the answer has context.
        if transfers.clash.is_some() && self.transfers.clash.is_none() { self.transfers_open = true; self.clash_all = true; }
        self.clipboard = clipboard; self.transfers = transfers;
    }
    pub fn operation_result(&mut self, ctx: &egui::Context, result: Result<&str, &str>) {
        match result {
            Ok(_) => { self.dialog = None; self.navigate(ctx, self.path.clone(), Navigation::Refresh); },
            Err(error) => {
                self.error = Some(error.into());
                if let Some(dialog) = &mut self.dialog { dialog.pending = false; dialog.error = Some(error.into()); }
            },
        }
    }
    fn selected_visible(&self) -> Option<PathBuf> {
        self.selected.as_ref().filter(|path| self.visible().iter().any(|&i| &self.items()[i].path == *path)).cloned()
    }
    /// Everything selected and shown, in list order; at least the focused entry.
    fn chosen(&self) -> Vec<PathBuf> {
        let shown: Vec<PathBuf> = self.visible().into_iter().map(|i| self.items()[i].path.clone()).collect();
        let marked: Vec<PathBuf> = shown.iter().filter(|p| self.marked.contains(p)).cloned().collect();
        if marked.is_empty() { self.selected_visible().into_iter().collect() } else { marked }
    }
    fn is_marked(&self, path: &Path) -> bool {
        if self.marked.is_empty() { self.selected.as_deref() == Some(path) } else { self.marked.iter().any(|p| p == path) }
    }
    fn select_only(&mut self, path: PathBuf) { self.marked = vec![path.clone()]; self.anchor = Some(path.clone()); self.selected = Some(path); }
    /// A click on an entry: alone, adding with Ctrl, or a run with Shift.
    fn click(&mut self, path: PathBuf, modifiers: Modifiers, shown: &[PathBuf]) {
        if modifiers.command || modifiers.ctrl {
            if self.marked.is_empty() { if let Some(focused) = self.selected.clone() { self.marked.push(focused); } }
            if let Some(at) = self.marked.iter().position(|p| *p == path) { self.marked.remove(at); } else { self.marked.push(path.clone()); }
            self.anchor = Some(path.clone()); self.selected = Some(path);
        } else if modifiers.shift {
            let from = self.anchor.as_ref().or(self.selected.as_ref()).and_then(|anchor| shown.iter().position(|p| p == anchor));
            match (from, shown.iter().position(|p| *p == path)) {
                (Some(a), Some(b)) => { self.marked = shown[a.min(b)..=a.max(b)].to_vec(); self.selected = Some(path); },
                _ => self.select_only(path),
            }
        } else { self.select_only(path); }
    }
    /// Carries out what was asked, in Browse mode against the selection.
    fn perform(&mut self, ctx: &egui::Context, mut act: Act) -> Option<Event> {
        if let Some(folder) = act.terminal { return Some(Event::Terminal(folder)); }
        if act.undo && self.mode == Mode::Browse && self.transfers.undo.is_some() { return Some(Event::Undo); }
        if act.open { return self.activate(ctx); }
        if self.mode != Mode::Browse { return None; }
        let chosen = self.chosen();
        if act.new_folder && !self.in_trash && !self.loading { self.dialog = Some(OperationDialog::new(DialogKind::NewFolder, String::new())); }
        let files: Vec<PathBuf> = chosen.iter().filter(|p| self.items().iter().any(|e| &e.path == *p && !e.directory)).cloned().collect();
        if act.open_with && !files.is_empty() { self.open_with = Some(crate::file_dialogs::OpenWith::new(files.clone(), ctx)); }
        if let Some(device) = act.send.take() { if !files.is_empty() { return Some(Event::SendTo(device, files)); } }
        if act.compress && !self.in_trash && self.search.is_none() && !chosen.is_empty() { self.compressing = Some(crate::file_dialogs::Compress::new(chosen.clone())); }
        if act.extract && !self.in_trash {
            let archives: Vec<_> = files.iter().filter(|p| crate::file_ops::Archive::of(p).is_some()).cloned().map(|archive| crate::file_ops::Operation::Extract { archive }).collect();
            if !archives.is_empty() { return Some(Event::Operate(archives)); }
        }
        if act.properties {
            if let Some(path) = self.selected_visible().or_else(|| Some(self.path.clone())) {
                let kind = self.items().iter().find(|e| e.path == path).map_or("Folder", kind_of).to_string();
                self.properties = crate::file_dialogs::Properties::new(path, kind, ctx);
            }
        }
        if act.rename && !self.in_trash && chosen.len() > 1 {
            let items = chosen.iter().map(|path| (path.clone(), self.items().iter().find(|e| &e.path == path).and_then(|e| e.modified))).collect();
            self.renaming = Some(crate::file_dialogs::BatchRename::new(items));
        } else if act.rename && !self.in_trash {
            if let Some(source) = self.selected_visible() {
                self.dialog = Some(OperationDialog::new(DialogKind::Rename(source.clone()), source.file_name().unwrap_or_default().to_string_lossy().into_owned()));
            }
        }
        if act.trash && !self.in_trash && !chosen.is_empty() { self.dialog = Some(OperationDialog::new(DialogKind::Trash(chosen.clone()), String::new())); }
        if act.restore && self.in_trash && !chosen.is_empty() {
            return Some(Event::Operate(chosen.into_iter().map(|source| crate::file_ops::Operation::Restore { source }).collect()));
        }
        if (act.copy || act.cut) && !self.in_trash && !chosen.is_empty() { return Some(Event::Clipboard(Clipboard { sources: chosen, cut: act.cut })); }
        if act.paste && !self.in_trash && !self.loading {
            if let Some(clipboard) = &self.clipboard {
                let directory = self.path.clone();
                return Some(Event::Operate(clipboard.sources.iter().cloned().map(|source| if clipboard.cut {
                    crate::file_ops::Operation::Move { source, directory: directory.clone(), conflict: Default::default() }
                } else { crate::file_ops::Operation::Copy { source, directory: directory.clone(), conflict: Default::default() } }).collect()));
            }
        }
        None
    }
    /// Keys for the list and for operations, when nothing else has the keyboard.
    fn keys(&mut self, ui: &mut egui::Ui, act: &mut Act, target: &mut Option<(PathBuf, Navigation)>, close: &mut bool) {
        let shown: Vec<PathBuf> = self.visible().into_iter().map(|i| self.items()[i].path.clone()).collect();
        let pressed = |ui: &mut egui::Ui, modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
        let at = self.selected.as_ref().and_then(|p| shown.iter().position(|s| s == p));
        // egui lets a pattern without Shift or Alt match a key pressed with them, so
        // the modified forms are taken first.
        if pressed(ui, Modifiers::ALT, Key::ArrowUp) { *target = self.path.parent().map(|p| (p.to_path_buf(), Navigation::New)); }
        if pressed(ui, Modifiers::ALT, Key::ArrowLeft) && self.history_at > 0 { *target = Some((self.history[self.history_at - 1].clone(), Navigation::History(self.history_at - 1))); }
        if pressed(ui, Modifiers::ALT, Key::ArrowRight) && self.history_at + 1 < self.history.len() { *target = Some((self.history[self.history_at + 1].clone(), Navigation::History(self.history_at + 1))); }
        let mut step: Option<isize> = None; let mut extend = false;
        let across = if self.view == View::Grid { self.grid_columns.max(1) as isize } else { 1 };
        let mut moves = vec![(Key::ArrowDown, across), (Key::ArrowUp, -across), (Key::PageDown, 12 * across), (Key::PageUp, -12 * across), (Key::End, isize::MAX / 2), (Key::Home, -(isize::MAX / 2))];
        if self.view == View::Grid { moves.extend([(Key::ArrowRight, 1), (Key::ArrowLeft, -1)]); }
        for (key, by) in moves {
            if pressed(ui, Modifiers::SHIFT, key) { step = Some(by); extend = true; }
            else if pressed(ui, Modifiers::NONE, key) { step = Some(by); }
        }
        if let Some(by) = step.filter(|_| !shown.is_empty()) {
            let next = at.map_or(0, |i| (i as isize).saturating_add(by).clamp(0, shown.len() as isize - 1) as usize);
            let path = shown[next].clone();
            if extend { self.click(path, Modifiers::SHIFT, &shown); } else { self.select_only(path); }
            self.reveal = true;
        }
        if pressed(ui, Modifiers::COMMAND, Key::A) { self.marked = shown.clone(); if self.selected.is_none() { self.selected = shown.first().cloned(); } }
        if self.search.is_some() && pressed(ui, Modifiers::COMMAND, Key::Enter) {
            if let Some(found) = self.selected.clone() {
                if let Some(parent) = found.parent() { self.reveal_after = Some(found.clone()); *target = Some((parent.to_path_buf(), Navigation::New)); }
            }
        }
        if pressed(ui, Modifiers::ALT, Key::Enter) { act.properties = true; }
        if pressed(ui, Modifiers::NONE, Key::Enter) { act.open = true; }
        if pressed(ui, Modifiers::NONE, Key::Backspace) {
            *target = self.path.parent().map(|p| (p.to_path_buf(), Navigation::New));
        }
        if pressed(ui, Modifiers::NONE, Key::F5) || pressed(ui, Modifiers::COMMAND, Key::R) { *target = Some((self.path.clone(), Navigation::Refresh)); }
        if pressed(ui, Modifiers::COMMAND, Key::H) { self.hidden = !self.hidden; }
        if pressed(ui, Modifiers::COMMAND, Key::L) { self.editing_path = true; self.focus_path = true; }
        if pressed(ui, Modifiers::COMMAND, Key::F) { self.focus_filter = true; }
        if pressed(ui, Modifiers::NONE, Key::Escape) {
            if self.marked.len() > 1 { self.marked.clear(); } else { *close = true; }
        }
        if self.mode != Mode::Browse { return; }
        if !self.in_trash {
            // Native integrations translate clipboard chords into semantic events
            // instead of Key events. Leave them for text fields and dialogs.
            ui.input_mut(|input| input.events.retain(|event| match event {
                egui::Event::Copy => { act.copy = true; false },
                egui::Event::Cut => { act.cut = true; false },
                egui::Event::Paste(_) => { act.paste = true; self.paste_chord_handled = true; false },
                egui::Event::Key { key: Key::V, pressed: false, modifiers, .. } if modifiers.ctrl || modifiers.command => {
                    if !self.paste_chord_handled { act.paste = true; }
                    self.paste_chord_handled = false; false
                },
                _ => true,
            }));
            act.copy |= pressed(ui, Modifiers::CTRL, Key::C);
            act.cut |= pressed(ui, Modifiers::CTRL, Key::X);
            act.paste |= pressed(ui, Modifiers::CTRL, Key::V);
            act.new_folder |= pressed(ui, Modifiers::CTRL | Modifiers::SHIFT, Key::N);
            act.undo |= pressed(ui, Modifiers::COMMAND, Key::Z);
            act.rename |= pressed(ui, Modifiers::NONE, Key::F2);
            act.trash |= pressed(ui, Modifiers::NONE, Key::Delete);
        }
    }
    fn operation_dialog(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Event> {
        let mut dialog = self.dialog.take()?; let mut event = None; let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("native-file-operation")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(220.0, 440.0));
            let title = match &dialog.kind { DialogKind::NewFolder => "New folder", DialogKind::Rename(_) => "Rename", DialogKind::Trash(_) => "Move to Trash" };
            ui.label(egui::RichText::new(title).font(ui::display_bold(20.0)).color(ui::TEXT));
            ui.add_space(12.0);
            let mut submit;
            match &dialog.kind {
                DialogKind::Trash(sources) => {
                    let what = match sources.as_slice() {
                        [one] => format!("Move “{}” to Trash?", one.file_name().unwrap_or_default().to_string_lossy()),
                        many => format!("Move {} items to Trash?", many.len()),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(what).font(ui::display(14.0)).color(ui::TEXT)).wrap());
                    crate::settings::note(ui, "You can restore them from Trash.");
                    submit = !dialog.pending && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                },
                _ => {
                    let width = ui.available_width();
                    let field = ui.add_enabled_ui(!dialog.pending, |ui| ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(&mut dialog.name).margin(egui::vec2(10.0, 4.0)).vertical_align(egui::Align::Center).char_limit(255).hint_text(ui::hint("Name")))).inner;
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "File or folder name"));
                    if !dialog.focused { field.request_focus(); dialog.focused = true; }
                    submit = !dialog.pending && (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                },
            }
            if let Some(error) = &dialog.error { ui.colored_label(ui::CAUTION, error); }
            if dialog.pending { crate::settings::note(ui, "Working…"); }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let label = match dialog.kind { DialogKind::NewFolder => "CREATE FOLDER", DialogKind::Rename(_) => "RENAME", DialogKind::Trash(_) => "MOVE TO TRASH" };
                let ready = !dialog.pending && (matches!(dialog.kind, DialogKind::Trash(_)) || !dialog.name.is_empty());
                submit |= ui.add_enabled_ui(ready, |ui| ui::button(ui, theme, label, Some("ENTER"), ready)).inner.clicked();
                if dialog.pending {
                    if ui::button(ui, theme, "CANCEL OPERATION", None, false).clicked() { event = Some(Event::CancelOperation); }
                } else { cancel = ui::button(ui, theme, "CANCEL", Some("ESC"), false).clicked(); }
            });
            if submit && !dialog.pending {
                let operations = match &dialog.kind {
                    DialogKind::NewFolder => vec![crate::file_ops::Operation::CreateFolder { parent: self.path.clone(), name: dialog.name.clone() }],
                    DialogKind::Rename(source) => vec![crate::file_ops::Operation::Rename { source: source.clone(), name: dialog.name.clone() }],
                    DialogKind::Trash(sources) => sources.iter().cloned().map(|source| crate::file_ops::Operation::Trash { source }).collect(),
                };
                dialog.pending = true; dialog.error = None; event = Some(Event::Operate(operations));
            }
        });
        if !cancel && (!response.should_close() || dialog.pending) { self.dialog = Some(dialog); }
        event
    }
    pub fn show(&mut self, ui: &mut egui::Ui, theme: &Theme) -> Option<Event> { self.show_with_keyboard(ui, theme, true) }
    pub fn show_with_keyboard(&mut self, ui: &mut egui::Ui, theme: &Theme, active: bool) -> Option<Event> {
        // A side without the keyboard still takes clicks; only its keys are off.
        let owns = active && ui.is_enabled();
        let enabled = ui.is_enabled() && self.dialog.is_none();
        let mut event = ui.add_enabled_ui(enabled, |ui| self.show_inner(ui, theme, owns && self.dialog.is_none())).inner;
        if owns { event = self.operation_dialog(ui.ctx(), theme).or(event); }
        if owns && self.dialog.is_none() { event = self.clash_dialog(ui.ctx(), theme).or(event); }
        if owns {
            if let Some(renaming) = &mut self.renaming {
                match renaming.show(ui.ctx(), theme) {
                    Some(crate::file_dialogs::Outcome::Run(operations)) => { self.renaming = None; event = Some(Event::Operate(operations)); },
                    Some(crate::file_dialogs::Outcome::Cancel) => self.renaming = None,
                    None => {},
                }
            }
            if self.properties.as_mut().is_some_and(|properties| properties.show(ui.ctx(), theme)) { self.properties = None; self.refresh(ui.ctx()); }
            if self.open_with.as_mut().is_some_and(|open_with| open_with.show(ui.ctx(), theme)) { self.open_with = None; }
            if let Some(compressing) = &mut self.compressing {
                match compressing.show(ui.ctx(), theme) {
                    Some(crate::file_dialogs::Outcome::Run(operations)) => { self.compressing = None; event = Some(Event::Operate(operations)); },
                    Some(crate::file_dialogs::Outcome::Cancel) => self.compressing = None,
                    None => {},
                }
            }
        }
        event
    }
    fn show_inner(&mut self, ui: &mut egui::Ui, theme: &Theme, keyboard_active: bool) -> Option<Event> {
        if !self.loaded { self.navigate(ui.ctx(), self.path.clone(), Navigation::Refresh); }
        let answer = self.worker.as_ref().and_then(|w| w.answer.lock().unwrap().take());
        if let Some((generation, result)) = answer { self.accept(generation, result); }
        let mut act = Act::default();
        let mut target = None;
        let mut close = false;
        if let Some(search) = &mut self.search {
            if search.pump() {
                for hit in &search.hits[self.found.len()..] {
                    if let Some(line) = &hit.line { self.lines.insert(hit.path.clone(), line.clone()); }
                    self.found.push(Entry { path: hit.path.clone(), directory: hit.directory, bytes: hit.bytes, modified: hit.modified });
                }
            }
        }
        if let Some(result) = self.mounting.as_ref().and_then(|answer| answer.try_recv().ok()) {
            self.mounting = None;
            match result { Ok(folder) => target = Some((folder, Navigation::New)), Err(error) => self.error = Some(format!("Could not open the drive: {error}")) }
        }
        if let Some(result) = self.ejecting.as_ref().and_then(|(_, answer)| answer.try_recv().ok()) {
            let (folder, _) = self.ejecting.take().unwrap();
            match result {
                Ok(()) => {
                    if folder.is_some_and(|folder| self.path.starts_with(folder)) { target = Some((home(), Navigation::New)); }
                    self.message = Some(("The drive can be unplugged now.".into(), Instant::now()));
                },
                Err(error) => self.error = Some(format!("Could not eject: {error}")),
            }
        }
        let width = ui.available_width();
        let narrow = width < 640.0;
        let typing = self.header(ui, theme, narrow, keyboard_active, &mut target, &mut act, &mut close);
        if keyboard_active && !typing && !self.captures_keyboard() && !self.loading {
            self.keys(ui, &mut act, &mut target, &mut close);
        }
        ui.add_space(10.0);
        let body = ui.available_rect_before_wrap();
        let sidebar = if width >= 760.0 { 196.0 } else { 0.0 };
        let preview = if width >= 1040.0 { 290.0 } else { 0.0 };
        let gap = 16.0;
        let mut menu_drawn = false;
        if sidebar > 0.0 {
            let rect = egui::Rect::from_min_size(body.min, egui::vec2(sidebar, body.height()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
            self.sidebar(&mut child, theme, &mut target);
            ui.painter().vline(rect.right() + gap / 2.0, rect.y_range(), egui::Stroke::new(1.0, ui::LINE));
        }
        if preview > 0.0 {
            let rect = egui::Rect::from_min_max(egui::pos2(body.right() - preview, body.top()), body.max);
            ui.painter().vline(rect.left() - gap / 2.0, rect.y_range(), egui::Stroke::new(1.0, ui::LINE));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
            self.preview_pane(&mut child, theme, &mut act);
        }
        let left = body.left() + if sidebar > 0.0 { sidebar + gap } else { 0.0 };
        let right = body.right() - if preview > 0.0 { preview + gap } else { 0.0 };
        let center = egui::Rect::from_min_max(egui::pos2(left, body.top()), egui::pos2(right.max(left + 1.0), body.bottom()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(center).layout(egui::Layout::top_down(egui::Align::Min)));
        let mut event = self.center(&mut child, theme, &mut act, &mut target, &mut menu_drawn);
        ui.allocate_rect(body, egui::Sense::hover());
        self.menu_open = menu_drawn;
        if close { event = Some(Event::Close); }
        if let Some((path, navigation)) = target { self.navigate(ui.ctx(), path, navigation); }
        if event.is_none() { event = self.perform(ui.ctx(), act); }
        event
    }

    /// Back, forward and up; the path as places to click, or as text to type; the
    /// filter; a terminal here; and the way out. Returns whether a field has the keys.
    #[allow(clippy::too_many_arguments)]
    fn header(&mut self, ui: &mut egui::Ui, theme: &Theme, narrow: bool, keyboard_active: bool, target: &mut Option<(PathBuf, Navigation)>, act: &mut Act, close: &mut bool) -> bool {
        let mut typing = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let can_back = self.history_at > 0 && !self.loading;
            let can_forward = self.history_at + 1 < self.history.len() && !self.loading;
            let can_up = self.path.parent().is_some() && !self.loading;
            if nav_button(ui, theme, Nav::Back, can_back).on_hover_text("Back · Alt+Left").clicked() && can_back { *target = Some((self.history[self.history_at - 1].clone(), Navigation::History(self.history_at - 1))); }
            if nav_button(ui, theme, Nav::Forward, can_forward).on_hover_text("Forward · Alt+Right").clicked() && can_forward { *target = Some((self.history[self.history_at + 1].clone(), Navigation::History(self.history_at + 1))); }
            if nav_button(ui, theme, Nav::Up, can_up).on_hover_text("Up one folder · Backspace").clicked() && can_up { *target = self.path.parent().map(|p| (p.to_path_buf(), Navigation::New)); }
            ui.add_space(4.0);
            let done = if self.mode == Mode::ImagePicker { "CANCEL" } else { "DONE" };
            let tail = if narrow { 0.0 } else { 230.0 + 6.0 } + if self.mode == Mode::Browse { ui::CONTROL_HEIGHT + 6.0 } else { 0.0 } + 92.0;
            let path_width = (ui.available_width() - tail).max(60.0);
            typing |= self.path_bar(ui, theme, path_width, keyboard_active, target);
            if !narrow { typing |= self.filter_field(ui, 230.0); }
            if self.mode == Mode::Browse && nav_button(ui, theme, Nav::Terminal, true).on_hover_text("Open a terminal here").clicked() {
                act.terminal = Some(self.path.clone());
            }
            if ui::button(ui, theme, done, Some("ESC"), false).clicked() { *close = true; }
        });
        if narrow {
            ui.add_space(8.0);
            typing |= self.filter_field(ui, ui.available_width());
        }
        typing
    }
    fn filter_field(&mut self, ui: &mut egui::Ui, width: f32) -> bool {
        let field = ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(&mut self.filter).id(ui.id().with("native-files-filter")).margin(egui::vec2(10.0, 4.0)).vertical_align(egui::Align::Center).char_limit(256).hint_text(ui::hint(if self.mode == Mode::Browse { "Filter · Enter searches inside" } else { "Filter this folder  Ctrl+F" })));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter this folder"));
        if self.focus_filter { field.request_focus(); self.focus_filter = false; }
        // egui lets go of the field on Escape before this sees the key, so the frame
        // it lets go counts too; the key must not reach the shell and close Files.
        if (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if self.filter.is_empty() { field.surrender_focus(); } else { self.filter.clear(); }
            self.end_search();
        }
        if (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) && self.mode == Mode::Browse {
            self.start_search(ui.ctx());
        }
        if field.changed() && self.filter.trim().is_empty() { self.end_search(); }
        field.has_focus() || field.lost_focus()
    }
    /// The folder as a row of places to click; click the bar itself, or press
    /// Ctrl+L, to type a path instead.
    fn path_bar(&mut self, ui: &mut egui::Ui, theme: &Theme, width: f32, keyboard_active: bool, target: &mut Option<(PathBuf, Navigation)>) -> bool {
        if self.editing_path {
            let field = ui.add_sized(egui::vec2(width, ui::CONTROL_HEIGHT), egui::TextEdit::singleline(&mut self.path_text).id(ui.id().with("native-files-path")).margin(egui::vec2(10.0, 4.0)).vertical_align(egui::Align::Center).font(ui::mono(12.0)).char_limit(4096).hint_text(ui::hint("/path/to/folder")));
            field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Folder path"));
            if self.focus_path { field.request_focus(); self.focus_path = false; }
            let typed = field.has_focus() || field.lost_focus();
            if keyboard_active && typed && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                let text = self.path_text.trim();
                let path = match text.strip_prefix("~") { Some(rest) => home().join(rest.trim_start_matches('/')), None => PathBuf::from(text) };
                *target = Some((if path == self.path { self.path.clone() } else { path }, Navigation::New));
                self.editing_path = false;
            } else if typed && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.editing_path = false; self.path_text = self.path.to_string_lossy().into_owned();
            } else if field.lost_focus() {
                self.editing_path = false; self.path_text = self.path.to_string_lossy().into_owned();
            }
            return typed;
        }
        let (bar, response) = ui.allocate_exact_size(egui::vec2(width, ui::CONTROL_HEIGHT), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Type a folder path"));
        let painter = ui.painter_at(bar);
        painter.rect_filled(bar, ui::CONTROL_RADIUS, ui::GROUND);
        painter.rect_stroke(bar, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, if response.hovered() { ui::LINE_STRONG } else { ui::LINE }), egui::StrokeKind::Inside);
        // Home and the folders under it, or the file system from its root.
        let home = home();
        let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
        if self.in_trash { crumbs.push(("Trash".into(), self.path.clone())); }
        else if let Ok(rest) = self.path.strip_prefix(&home) {
            crumbs.push(("Home".into(), home.clone()));
            let mut at = home.clone();
            for part in rest.components() { at.push(part); crumbs.push((part.as_os_str().to_string_lossy().into_owned(), at.clone())); }
        } else {
            crumbs.push(("File system".into(), PathBuf::from("/")));
            let mut at = PathBuf::from("/");
            for part in self.path.components().skip(1) { at.push(part); crumbs.push((part.as_os_str().to_string_lossy().into_owned(), at.clone())); }
        }
        let sep = 18.0; let pad = 8.0;
        let widths: Vec<f32> = crumbs.iter().map(|(name, _)| ui.painter().layout_no_wrap(name.clone(), ui::display(13.5), ui::TEXT).size().x + 2.0 * pad).collect();
        let room = bar.width() - 12.0;
        let mut first = 0;
        let ellipsis = 26.0;
        while first + 1 < crumbs.len() && widths[first..].iter().sum::<f32>() + sep * (crumbs.len() - first - 1) as f32 + if first > 0 { ellipsis } else { 0.0 } > room { first += 1; }
        let mut x = bar.left() + 6.0;
        if first > 0 {
            painter.text(egui::pos2(x + 6.0, bar.center().y), egui::Align2::LEFT_CENTER, "…", ui::display(13.5), ui::LABEL);
            x += ellipsis;
        }
        let last = crumbs.len() - 1;
        for (index, (name, path)) in crumbs.iter().enumerate().skip(first) {
            let w = widths[index].min(bar.right() - 6.0 - x).max(0.0);
            if w < 2.0 * pad { break; }
            let rect = egui::Rect::from_min_size(egui::pos2(x, bar.top() + 3.0), egui::vec2(w, bar.height() - 6.0));
            let crumb = ui.interact(rect, ui.id().with(("crumb", index)), egui::Sense::click());
            if crumb.hovered() { painter.rect_filled(rect, ui::CONTROL_RADIUS - 1.0, ui::TRACK); }
            let color = if index == last { ui::TEXT } else if crumb.hovered() { ui::TEXT } else { ui::SECONDARY };
            let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), ui::display(13.5), color);
            job.wrap = egui::text::TextWrapping::truncate_at_width(w - 2.0 * pad);
            let galley = ui.painter().layout_job(job);
            painter.galley(egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0), galley, color);
            crumb.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.as_str()));
            if crumb.clicked() && index != last { *target = Some((path.clone(), Navigation::New)); }
            x += w;
            if index != last {
                painter.text(egui::pos2(x + sep / 2.0, bar.center().y), egui::Align2::CENTER_CENTER, "/", ui::mono(12.0), ui::LABEL);
                x += sep;
            }
        }
        let _ = theme;
        if response.clicked() { self.editing_path = true; self.focus_path = true; }
        false
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, theme: &Theme, target: &mut Option<(PathBuf, Navigation)>) {
        let width = ui.available_width();
        side_label(ui, "PLACES");
        for (name, path) in self.places.clone() {
            let here = !self.in_trash && self.path == path;
            if place_item(ui, theme, width, name, "", here).clicked() && !here { *target = Some((path, Navigation::New)); }
        }
        let drives = crate::drives::snapshot(ui.ctx());
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(3));
        if !drives.is_empty() {
            ui.add_space(10.0);
            side_label(ui, "DEVICES");
            for drive in drives {
                let here = drive.mountpoint.as_ref().is_some_and(|m| self.path.starts_with(m));
                let busy = self.mounting.is_some() || self.ejecting.is_some();
                let (open, eject) = drive_item(ui, theme, width, &drive, here, busy);
                if open.clicked() && !busy {
                    match &drive.mountpoint {
                        Some(folder) => { if !here || self.path != *folder { *target = Some((folder.clone(), Navigation::New)); } },
                        None => { self.error = None; self.mounting = Some(crate::drives::mount(drive.device.clone(), ui.ctx())); },
                    }
                }
                if eject.is_some_and(|eject| eject.clicked()) && !busy {
                    self.ejecting = Some((drive.mountpoint.clone(), crate::drives::eject(drive.clone(), ui.ctx())));
                }
            }
        }
        ui.add_space(10.0);
        side_label(ui, "ELSEWHERE");
        if self.mode == Mode::Browse {
            if let Some(trash) = crate::file_ops::trash_files() {
                if place_item(ui, theme, width, "Trash", "", self.in_trash).clicked() && !self.in_trash { *target = Some((trash, Navigation::New)); }
            }
        }
        let root = self.path == Path::new("/");
        if place_item(ui, theme, width, "File system", "", root).clicked() && !root { *target = Some((PathBuf::from("/"), Navigation::New)); }
    }

    /// What is selected: a look at it, what it is, and what can be done with it.
    fn preview_pane(&mut self, ui: &mut egui::Ui, theme: &Theme, act: &mut Act) {
        let chosen = self.chosen();
        let width = ui.available_width();
        if chosen.len() > 1 {
            let bytes: u64 = self.items().iter().filter(|e| chosen.contains(&e.path)).map(|e| e.bytes).sum();
            ui.add(egui::Label::new(egui::RichText::new(format!("{} items selected", chosen.len())).font(ui::display_bold(17.0)).color(ui::TEXT)).wrap());
            ui.label(egui::RichText::new(format!("{} in files", file_size(bytes))).font(ui::mono(12.0)).color(ui::SECONDARY));
            ui.add_space(14.0);
            self.actions(ui, theme, act, false);
            return;
        }
        let Some(path) = self.selected_visible() else {
            self.look_path = None; self.look.close();
            let name = if self.in_trash { "Trash".into() } else if self.path == home() { "Home".into() } else { self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "File system".into()) };
            ui.add(egui::Label::new(egui::RichText::new(name).font(ui::display_bold(17.0)).color(ui::TEXT)).wrap());
            ui.label(egui::RichText::new(items(self.visible().len())).font(ui::mono(12.0)).color(ui::SECONDARY));
            ui.add_space(10.0);
            crate::settings::note(ui, "Select something to see it here.");
            return;
        };
        if self.look_path.as_ref() != Some(&path) {
            self.look.open(path.clone(), (0, 1), ui.ctx());
            self.look_path = Some(path.clone());
        }
        let height = (ui.available_height() * 0.42).clamp(120.0, 260.0);
        let (frame, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter().rect_filled(frame, ui::SURFACE_RADIUS, ui::PANEL);
        ui.painter().rect_stroke(frame, ui::SURFACE_RADIUS, egui::Stroke::new(1.0, ui::LINE), egui::StrokeKind::Inside);
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(frame.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Center)));
        inside.set_clip_rect(frame.shrink(2.0));
        self.look.show_inline(&mut inside, theme);
        ui.add_space(12.0);
        let entry = self.items().iter().find(|e| e.path == path).cloned();
        let name = entry.as_ref().map(|e| self.name_of(e)).unwrap_or_default();
        ui.add(egui::Label::new(egui::RichText::new(name).font(ui::display_bold(16.0)).color(ui::TEXT)).wrap());
        if let Some(entry) = &entry {
            let size = if entry.directory { String::new() } else { format!(" · {}", file_size(entry.bytes)) };
            ui.label(egui::RichText::new(format!("{}{size}", kind_of(entry))).font(ui::mono(12.0)).color(ui::SECONDARY));
            let changed = when(entry.modified);
            if !changed.is_empty() { ui.label(egui::RichText::new(format!("Modified {changed}")).font(ui::mono(12.0)).color(ui::SECONDARY)); }
            if let Some(location) = self.location_of(entry) { ui.label(egui::RichText::new(format!("In {location}")).font(ui::mono(12.0)).color(ui::SECONDARY)); }
            if let Some(line) = self.lines.get(&entry.path) {
                ui.add_space(6.0);
                ui.add(egui::Label::new(egui::RichText::new(format!("“{line}”")).font(ui::mono(11.5)).color(ui::TEXT)).wrap());
            }
        }
        ui.add_space(12.0);
        self.actions(ui, theme, act, entry.as_ref().is_some_and(|e| e.directory));
        ui.add_space(12.0);
        if self.mode == Mode::Browse {
            crate::settings::note(ui, "Enter opens · F2 renames · Del moves to Trash · Ctrl+C, X, V");
        }
    }
    fn actions(&mut self, ui: &mut egui::Ui, theme: &Theme, act: &mut Act, folder: bool) {
        let single = self.chosen().len() == 1;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            if self.mode == Mode::ImagePicker {
                if single { act.open |= ui::button(ui, theme, if folder { "OPEN FOLDER" } else { "CHOOSE IMAGE" }, None, true).clicked(); }
                return;
            }
            if self.in_trash { act.restore |= ui::button(ui, theme, "RESTORE", None, false).clicked(); return; }
            if single { act.open |= ui::button(ui, theme, "OPEN", None, true).clicked(); }
            if single && folder { if ui::button(ui, theme, "TERMINAL HERE", None, false).clicked() { act.terminal = self.selected.clone(); } }
            if single && !folder { act.open_with |= ui::button(ui, theme, "OPEN WITH", None, false).clicked(); }
            if single && self.selected.as_deref().and_then(crate::file_ops::Archive::of).is_some() { act.extract |= ui::button(ui, theme, "EXTRACT", None, false).clicked(); }
            if !folder || !single {
                for (id, name) in self.phones.clone() {
                    if ui::button(ui, theme, &format!("SEND TO {}", name.to_uppercase()), None, false).clicked() { act.send = Some(id); }
                }
            }
            if single { act.rename |= ui::button(ui, theme, "RENAME", None, false).clicked(); }
            act.copy |= ui::button(ui, theme, "COPY", None, false).clicked();
            act.cut |= ui::button(ui, theme, "CUT", None, false).clicked();
            act.trash |= ui::button(ui, theme, "TRASH", None, false).clicked();
        });
    }

    /// What a click, drag or right click on one entry does, the same in the list and the grid.
    #[allow(clippy::too_many_arguments)]
    fn entry_events(&mut self, ui: &mut egui::Ui, response: egui::Response, entry: &Entry, name: &str, marked: bool, shown: &[PathBuf], draggable: bool,
        event: &mut Option<Event>, opened: &mut Option<PathBuf>, act: &mut Act, menu_drawn: &mut bool) {
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, marked, format!("{name}, {}", kind_of(entry))));
        // A double click opens only when both clicks were on this entry, unmodified.
        let was_focused = self.selected.as_ref() == Some(&entry.path);
        let modifiers = ui.input(|i| i.modifiers);
        if response.clicked() { self.click(entry.path.clone(), modifiers, shown); }
        if response.secondary_clicked() && !marked { self.select_only(entry.path.clone()); }
        if response.middle_clicked() && entry.directory && self.mode == Mode::Browse { *event = Some(Event::NewTab(entry.path.clone())); }
        if response.drag_started() && draggable && event.is_none() {
            self.select_only(entry.path.clone());
            *event = Some(Event::Drag(entry.path.clone()));
        }
        if response.double_clicked() && was_focused && !(modifiers.ctrl || modifiers.command || modifiers.shift) { *opened = Some(entry.path.clone()); }
        response.context_menu(|ui| {
            *menu_drawn = true;
            ui.set_width(230.0); ui.spacing_mut().item_spacing.y = 0.0;
            let count = self.chosen().len();
            if count > 1 { menu_heading(ui, &format!("{count} ITEMS")); }
            let open = if entry.directory { "Open" } else if self.mode == Mode::ImagePicker { "Choose image" } else { "Open" };
            if count == 1 && menu_item(ui, open, "Enter", entry.directory || self.mode == Mode::Browse || image_path(&entry.path)).clicked() { act.open = true; ui.close_menu(); }
            if self.mode == Mode::Browse {
                if entry.directory && count == 1 && menu_item(ui, "Open in new tab", "Middle click", true).clicked() { *event = Some(Event::NewTab(entry.path.clone())); ui.close_menu(); }
                if entry.directory && count == 1 && menu_item(ui, "Open a terminal here", "", true).clicked() { act.terminal = Some(entry.path.clone()); ui.close_menu(); }
                menu_separator(ui);
                if self.in_trash {
                    if menu_item(ui, "Restore", "", true).clicked() { act.restore = true; ui.close_menu(); }
                } else {
                    if menu_item(ui, "Copy", "Ctrl+C", true).clicked() { act.copy = true; ui.close_menu(); }
                    if menu_item(ui, "Cut", "Ctrl+X", true).clicked() { act.cut = true; ui.close_menu(); }
                    if menu_item(ui, if count == 1 { "Rename" } else { "Rename all…" }, "F2", true).clicked() { act.rename = true; ui.close_menu(); }
                    menu_separator(ui);
                    let chosen = self.chosen();
                    let any_file = chosen.iter().any(|p| self.items().iter().any(|e| &e.path == p && !e.directory));
                    if any_file && menu_item(ui, "Open with…", "", true).clicked() { act.open_with = true; ui.close_menu(); }
                    if self.search.is_none() && menu_item(ui, "Compress…", "", true).clicked() { act.compress = true; ui.close_menu(); }
                    if chosen.iter().all(|p| crate::file_ops::Archive::of(p).is_some()) && menu_item(ui, "Extract here", "", true).clicked() { act.extract = true; ui.close_menu(); }
                    if any_file {
                        for (id, name) in self.phones.clone() {
                            if menu_item(ui, &format!("Send to {name}"), "", true).clicked() { act.send = Some(id); ui.close_menu(); }
                        }
                    }
                    menu_separator(ui);
                    if menu_item(ui, "Copy path", "", true).clicked() {
                        let paths: Vec<String> = self.chosen().iter().map(|p| p.to_string_lossy().into_owned()).collect();
                        ui.ctx().copy_text(paths.join("\n")); ui.close_menu();
                    }
                    if count == 1 && menu_item(ui, "Properties", "Alt+Enter", true).clicked() { act.properties = true; ui.close_menu(); }
                    menu_separator(ui);
                    if menu_item_caution(ui, "Move to Trash", "Del").clicked() { act.trash = true; ui.close_menu(); }
                }
            }
        });
    }

    /// The work queue: what runs, what waits, what was done, and the way back.
    fn transfers_panel(&mut self, ui: &mut egui::Ui, theme: &Theme, undo: &mut bool) -> Option<Event> {
        let mut event = None;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, ui::SURFACE_RADIUS, ui::PANEL);
        ui.painter().rect_stroke(rect, ui::SURFACE_RADIUS, egui::Stroke::new(1.0, ui::LINE), egui::StrokeKind::Inside);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(14.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut inner;
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
        ui.horizontal(|ui| {
            ui::tracked(ui, "TRANSFERS", ui::display_bold(12.0), ui::TEXT, 1.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::button(ui, theme, "HIDE", None, false).clicked() { self.transfers_open = false; }
                if !self.transfers.waiting.is_empty() && ui::button(ui, theme, "CLEAR WAITING", None, false).clicked() { event = Some(Event::ClearTransfers); }
                let paused = self.transfers.paused;
                if (paused || !self.transfers.waiting.is_empty()) && ui::button(ui, theme, if paused { "RESUME" } else { "PAUSE" }, None, paused).clicked() { event = Some(Event::PauseTransfers(!paused)); }
                if let Some(label) = &self.transfers.undo { if ui::button(ui, theme, "UNDO", Some("CTRL Z"), false).on_hover_text(label.as_str()).clicked() { *undo = true; } }
            });
        });
        let line = |ui: &mut egui::Ui, text: &str, color: egui::Color32| {
            let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), ui::mono(11.5), color);
            job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width().max(10.0));
            ui.label(job);
        };
        match (&self.transfers.running, &self.transfers.clash) {
            (_, Some(clash)) => line(ui, &format!("Waiting for you: {} is already there", clash.name), ui::CAUTION),
            (Some(running), None) => {
                ui.horizontal(|ui| {
                    if ui::button(ui, theme, "CANCEL", None, false).clicked() { event = Some(Event::CancelOperation); }
                    let done = if self.transfers.progress == 0 { String::new() } else { format!(" · {}", file_size(self.transfers.progress)) };
                    line(ui, &format!("{running}{done}"), ui::TEXT);
                });
            },
            (None, None) if self.transfers.paused => line(ui, "Paused", ui::CAUTION),
            (None, None) => line(ui, "Nothing running.", ui::SECONDARY),
        }
        for waiting in self.transfers.waiting.iter().take(2) { line(ui, &format!("Waiting · {waiting}"), ui::SECONDARY); }
        if self.transfers.waiting.len() > 2 { line(ui, &format!("and {} more waiting", self.transfers.waiting.len() - 2), ui::LABEL); }
        for (text, ok) in self.transfers.recent.iter().take(3) {
            line(ui, &format!("{} {text}", if *ok { "Done ·" } else { "Failed ·" }), if *ok { ui::LABEL } else { ui::CAUTION });
        }
        event
    }

    /// A name already taken where something goes: what is coming, what is there, and the choices.
    fn clash_dialog(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Event> {
        let clash = self.transfers.clash.clone()?;
        let mut event = None;
        egui::Modal::new(egui::Id::new("native-file-clash")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(240.0, 460.0));
            ui.add(egui::Label::new(egui::RichText::new(format!("{} is already there", clash.name)).font(ui::display_bold(19.0)).color(ui::TEXT)).wrap());
            ui.add_space(12.0);
            // To the second, as a person reads the times; finer differences say nothing.
            let second = |time: Option<std::time::SystemTime>| time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
            let (incoming, existing) = (second(clash.incoming.1), second(clash.existing.1));
            ui.columns(2, |columns| {
                for (column, title, (bytes, when_changed), lit) in [(0, "COMING IN", clash.incoming, incoming > existing), (1, "ALREADY THERE", clash.existing, existing > incoming)] {
                    let ui = &mut columns[column];
                    let label = if lit { format!("{title} · NEWER") } else { title.to_string() };
                    ui.label(egui::RichText::new(label).font(ui::mono(10.5)).color(if lit { theme.main } else { ui::LABEL }));
                    ui.label(egui::RichText::new(file_size(bytes)).font(ui::display(14.0)).color(ui::TEXT));
                    ui.label(egui::RichText::new(when(when_changed)).font(ui::mono(11.5)).color(ui::SECONDARY));
                }
            });
            ui.add_space(12.0);
            let pressed = |ui: &mut egui::Ui, key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                if ui::button(ui, theme, "REPLACE", Some("R"), true).on_hover_text("The one there goes to Trash").clicked() || pressed(ui, Key::R) { event = Some(Event::Resolve(crate::file_ops::Conflict::Replace, self.clash_all)); }
                if ui::button(ui, theme, "KEEP BOTH", Some("B"), false).clicked() || pressed(ui, Key::B) { event = Some(Event::Resolve(crate::file_ops::Conflict::KeepBoth, self.clash_all)); }
                if ui::button(ui, theme, "SKIP", Some("S"), false).clicked() || pressed(ui, Key::S) { event = Some(Event::Resolve(crate::file_ops::Conflict::Skip, self.clash_all)); }
                if ui::button(ui, theme, "STOP", Some("ESC"), false).on_hover_text("Leave this and everything still waiting").clicked() || pressed(ui, Key::Escape) { event = Some(Event::ClearTransfers); }
            });
            if clash.more > 0 {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let toggle = ui::toggle(ui, theme, self.clash_all);
                    toggle.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, self.clash_all, "Do the same for the rest"));
                    if toggle.clicked() { self.clash_all = !self.clash_all; }
                    ui.label(egui::RichText::new(format!("Do the same for the other {}", clash.more)).font(ui::display(13.0)).color(ui::TEXT));
                });
            }
            ui.add_space(6.0);
            crate::settings::note(ui, "Anything replaced goes to Trash, so Undo can bring it back.");
        });
        event
    }

    /// The toolbar, the sortable list and the status line.
    fn center(&mut self, ui: &mut egui::Ui, theme: &Theme, act: &mut Act, target: &mut Option<(PathBuf, Navigation)>, menu_drawn: &mut bool) -> Option<Event> {
        let mut event = None;
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if self.mode == Mode::Browse && !self.in_trash {
                act.new_folder |= ui.add_enabled_ui(!self.loading, |ui| ui::button(ui, theme, "NEW FOLDER", None, false)).inner.on_hover_text("Ctrl+Shift+N").clicked();
                let count = self.clipboard.as_ref().map_or(0, |c| c.sources.len());
                let label = if count > 1 { format!("PASTE {count}") } else { "PASTE".to_string() };
                act.paste |= ui.add_enabled_ui(count > 0 && !self.loading, |ui| ui::button(ui, theme, &label, None, false)).inner.on_hover_text("Ctrl+V").clicked();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let toggle = ui::toggle(ui, theme, self.hidden);
                toggle.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, self.hidden, "Hidden files"));
                if toggle.on_hover_text("Ctrl+H").clicked() { self.hidden = !self.hidden; }
                ui.label(egui::RichText::new("Hidden").font(ui::display(12.5)).color(ui::SECONDARY));
                ui.add_space(8.0);
                crate::settings::segmented(ui, theme, &mut self.view, &[(View::List, "List"), (View::Grid, "Grid")]);
            });
        });
        if let Some(error) = &self.error { ui.add_space(4.0); ui.add(egui::Label::new(egui::RichText::new(error).font(ui::display(12.5)).color(ui::CAUTION)).truncate()); }
        ui.add_space(8.0);

        // Column header: click to sort, again to reverse. The grid has no columns,
        // so it lists the same sorts side by side.
        let columns = Columns::for_width(width);
        let grid = self.view == View::Grid;
        let (head, _) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
        let heads = if grid { sort_labels(head) } else { columns.headers(head, self.search.is_some()) };
        for (key, label, rect, right) in heads {
            let response = ui.interact(rect, ui.id().with(("sort", label)), egui::Sense::click());
            let on = self.sort == key;
            let text = if on { format!("{label} {}", if self.descending { "↓" } else { "↑" }) } else { label.to_string() };
            let color = if on || response.hovered() { ui::TEXT } else { ui::LABEL };
            let anchor = if right { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
            let at = if right { rect.right_center() } else { rect.left_center() };
            ui.painter().text(at, anchor, text, ui::mono(10.5), color);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Sort by {label}")));
            if response.clicked() { if on { self.descending = !self.descending; } else { self.sort = key; self.descending = key == SortKey::Modified; } }
        }
        ui.painter().hline(head.x_range(), head.bottom(), egui::Stroke::new(1.0, ui::LINE));
        ui.add_space(4.0);

        let shown: Vec<PathBuf> = self.visible().into_iter().map(|i| self.items()[i].path.clone()).collect();
        let status_height = 34.0;
        let panel = if self.transfers_open && self.mode == Mode::Browse { 196.0_f32.min((ui.available_height() - status_height - 120.0).max(0.0)) } else { 0.0 };
        let list_height = (ui.available_height() - status_height - panel).max(60.0);
        let list_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, list_height));
        // The empty space of the folder has its own menu, under the rows.
        let background = ui.interact(list_rect, ui.id().with("folder-space"), egui::Sense::click());
        if background.clicked() { self.marked.clear(); self.selected = None; }
        background.context_menu(|ui| {
            *menu_drawn = true;
            ui.set_width(230.0); ui.spacing_mut().item_spacing.y = 0.0;
            if self.mode == Mode::Browse && !self.in_trash {
                if menu_item(ui, "New folder", "Ctrl+Shift+N", true).clicked() { act.new_folder = true; ui.close_menu(); }
                if menu_item(ui, "Paste", "Ctrl+V", self.clipboard.is_some()).clicked() { act.paste = true; ui.close_menu(); }
                if menu_item(ui, "Open a terminal here", "", true).clicked() { act.terminal = Some(self.path.clone()); ui.close_menu(); }
                menu_separator(ui);
            }
            if menu_item(ui, if self.hidden { "Hide hidden files" } else { "Show hidden files" }, "Ctrl+H", true).clicked() { self.hidden = !self.hidden; ui.close_menu(); }
            if menu_item(ui, "Select all", "Ctrl+A", !shown.is_empty()).clicked() { self.marked = shown.clone(); ui.close_menu(); }
            if menu_item(ui, "Copy folder path", "", true).clicked() { ui.ctx().copy_text(self.path.to_string_lossy().into_owned()); ui.close_menu(); }
        });

        // Keep the focused row in view after keys move it.
        let per_row = if grid { ((width + GRID_GAP) / (TILE_W + GRID_GAP)).floor().max(1.0) as usize } else { 1 };
        self.grid_columns = per_row;
        let (row_height, gap) = if grid { (TILE_H, GRID_GAP) } else { (ROW, ROW_GAP) };
        let tile_width = (width - GRID_GAP * (per_row as f32 - 1.0)) / per_row as f32;
        let mut scroll = egui::ScrollArea::vertical().id_salt(("native-files-list", grid)).max_height(list_height).auto_shrink([false, false]);
        if self.reveal {
            if let Some(at) = self.selected.as_ref().and_then(|p| shown.iter().position(|s| s == p)) {
                let top = (at / per_row) as f32 * (row_height + gap); let bottom = top + row_height;
                let mut offset = self.list_offset;
                if top < offset { offset = top; } else if bottom > offset + self.list_height { offset = bottom - self.list_height; }
                scroll = scroll.vertical_scroll_offset(offset.max(0.0));
            }
            self.reveal = false;
        }
        let draggable = self.mode == Mode::Browse && !self.in_trash && self.transfers.running.is_none();
        let mut opened = None;
        ui.spacing_mut().item_spacing.y = gap;
        let output = scroll.show_rows(ui, row_height, shown.len().div_ceil(per_row), |ui, rows| {
            for row in rows {
                if grid {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = GRID_GAP;
                        for at in row * per_row..((row + 1) * per_row).min(shown.len()) {
                            let Some(entry) = self.items().iter().find(|e| e.path == shown[at]).cloned() else { continue };
                            let name = self.name_of(&entry);
                            let marked = self.is_marked(&entry.path);
                            let focused = self.selected.as_ref() == Some(&entry.path) && self.marked.len() > 1;
                            let thumb = (!entry.directory && crate::thumbs::can_thumbnail(&entry.path)).then(|| self.thumbs.get(ui.ctx(), &entry.path, entry.modified)).flatten();
                            let response = grid_tile(ui, theme, &entry, &name, tile_width, marked, focused, thumb.as_ref());
                            self.entry_events(ui, response, &entry, &name, marked, &shown, draggable, &mut event, &mut opened, act, menu_drawn);
                        }
                    });
                } else {
                    let Some(entry) = self.items().iter().find(|e| e.path == shown[row]).cloned() else { continue };
                    let name = self.name_of(&entry);
                    let marked = self.is_marked(&entry.path);
                    let focused = self.selected.as_ref() == Some(&entry.path) && self.marked.len() > 1;
                    let location = self.location_of(&entry);
                    let mut response = entry_row(ui, theme, &entry, &name, location.as_deref(), columns, marked, focused);
                    if let Some(line) = self.lines.get(&entry.path) { response = response.on_hover_text(line.as_str()); }
                    self.entry_events(ui, response, &entry, &name, marked, &shown, draggable, &mut event, &mut opened, act, menu_drawn);
                }
            }
        });
        self.list_offset = output.state.offset.y;
        self.list_height = output.inner_rect.height();
        if let Some(path) = opened { self.select_only(path); act.open = true; }
        if shown.is_empty() && !self.loading {
            let note = if !self.filter.is_empty() { "Nothing here matches the filter." } else if self.mode == Mode::ImagePicker { "No PNG or JPEG images here. Open another folder." } else if self.in_trash { "Trash is empty." } else { "This folder is empty." };
            ui.painter().text(list_rect.center_top() + egui::vec2(0.0, 40.0), egui::Align2::CENTER_TOP, note, ui::display(13.0), ui::LABEL);
        }

        if panel > 0.0 {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, panel), egui::Sense::hover());
            let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(0.0, 6.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            inside.set_clip_rect(rect);
            if let Some(asked) = self.transfers_panel(&mut inside, theme, &mut act.undo) { event = Some(asked); }
        }
        // Status: what is here and selected, the clipboard, work going on.
        let (status, _) = ui.allocate_exact_size(egui::vec2(width, status_height), egui::Sense::hover());
        ui.painter().hline(status.x_range(), status.top(), egui::Stroke::new(1.0, ui::LINE));
        let mut line = ui.new_child(egui::UiBuilder::new().max_rect(status.shrink2(egui::vec2(2.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        line.spacing_mut().item_spacing.x = 10.0;
        if self.mode == Mode::ImagePicker {
            let ready = self.can_choose();
            if line.add_enabled_ui(ready, |ui| ui::button(ui, theme, "CHOOSE IMAGE", None, ready)).inner.clicked() { act.open = true; }
        }
        if self.mode == Mode::Browse {
            let busy = self.transfers.running.is_some() || !self.transfers.waiting.is_empty() || self.transfers.clash.is_some();
            let label = if self.transfers_open { "HIDE TRANSFERS" } else { "TRANSFERS" };
            if (busy || !self.transfers.recent.is_empty()) && ui::button(&mut line, theme, label, None, busy).clicked() { self.transfers_open = !self.transfers_open; }
            if !busy && !self.transfers_open && self.transfers.undo.is_some() && ui::button(&mut line, theme, "UNDO", Some("CTRL Z"), false).on_hover_text(self.transfers.undo.clone().unwrap_or_default()).clicked() { act.undo = true; }
        }
        if let Some(running) = self.transfers.running.clone().filter(|_| !self.transfers_open && width >= 520.0) {
            let done = if self.transfers.progress == 0 { String::new() } else { format!(" · {}", file_size(self.transfers.progress)) };
            let waiting = if self.transfers.waiting.is_empty() { String::new() } else { format!(" · {} waiting", self.transfers.waiting.len()) };
            let mut job = egui::text::LayoutJob::simple_singleline(format!("{running}{done}{waiting}"), ui::mono(11.5), ui::TEXT);
            job.wrap = egui::text::TextWrapping::truncate_at_width((width * 0.45).max(60.0));
            line.label(job);
        } else if let Some(free) = self.free.filter(|_| width >= 420.0 && self.transfers.running.is_none()) {
            line.label(egui::RichText::new(format!("{} free", file_size(free))).font(ui::mono(11.5)).color(ui::LABEL));
        }
        line.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let chosen = self.chosen();
            let mut text = if self.loading { "Loading…".to_string() } else if let Some(search) = &self.search {
                let place = search.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
                match (search.done, search.limited) {
                    (false, _) => format!("Searching {place} for “{}”… {} found", search.query, shown.len()),
                    (true, false) => format!("{} found in {place} · Ctrl+Enter opens its folder", shown.len()),
                    (true, true) => format!("{} found in {place}, stopped early", shown.len()),
                }
            } else { items(shown.len()) };
            if chosen.len() > 1 { text += &format!(" · {} selected", chosen.len()); }
            if self.limited { text += " · limited listing"; }
            if self.mounting.is_some() { text += " · opening the drive…"; }
            if self.ejecting.is_some() { text += " · ejecting…"; }
            if let Some((message, _)) = self.message.as_ref().filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(6)) { text += &format!(" · {message}"); }
            if let Some(clipboard) = self.clipboard.as_ref().filter(|_| self.mode == Mode::Browse) {
                let what = match clipboard.sources.as_slice() { [one] => one.file_name().unwrap_or_default().to_string_lossy().into_owned(), many => format!("{} items", many.len()) };
                text += &format!(" · {} {what} · Ctrl+V pastes here", if clipboard.cut { "Cut" } else { "Copied" });
            }
            ui.add(egui::Label::new(egui::RichText::new(text).font(ui::mono(11.5)).color(ui::SECONDARY)).truncate());
        });
        let _ = target;
        event
    }
}

/// Which columns fit, and where they go.
#[derive(Clone, Copy)]
struct Columns { kind: bool, modified: bool, size: bool }
impl Columns {
    const KIND: f32 = 120.0; const MODIFIED: f32 = 110.0; const SIZE: f32 = 80.0;
    fn for_width(width: f32) -> Self { Columns { kind: width >= 620.0, modified: width >= 480.0, size: width >= 340.0 } }
    /// From the right: size, modified, kind; the name takes the rest.
    fn split(self, rect: egui::Rect) -> (egui::Rect, Option<egui::Rect>, Option<egui::Rect>, Option<egui::Rect>) {
        let mut right = rect.right() - 10.0;
        let mut take = |on: bool, width: f32| on.then(|| { let r = egui::Rect::from_min_max(egui::pos2(right - width, rect.top()), egui::pos2(right, rect.bottom())); right -= width + 14.0; r });
        let size = take(self.size, Self::SIZE);
        let modified = take(self.modified, Self::MODIFIED);
        let kind = take(self.kind, Self::KIND);
        (egui::Rect::from_min_max(egui::pos2(rect.left() + 10.0, rect.top()), egui::pos2(right.max(rect.left() + 20.0), rect.bottom())), kind, modified, size)
    }
    fn headers(self, rect: egui::Rect, searching: bool) -> Vec<(SortKey, &'static str, egui::Rect, bool)> {
        let (name, kind, modified, size) = self.split(rect);
        let mut out = vec![(SortKey::Name, "NAME", egui::Rect::from_min_max(egui::pos2(name.left() + 28.0, name.top()), name.max), false)];
        if let Some(r) = kind { out.push((SortKey::Kind, if searching { "FOLDER" } else { "KIND" }, r, false)); }
        if let Some(r) = modified { out.push((SortKey::Modified, "MODIFIED", r, false)); }
        if let Some(r) = size { out.push((SortKey::Size, "SIZE", r, true)); }
        out
    }
}

fn entry_row(ui: &mut egui::Ui, theme: &Theme, entry: &Entry, name: &str, location: Option<&str>, columns: Columns, marked: bool, focused: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::click_and_drag());
    let painter = ui.painter();
    if marked { painter.rect_filled(rect, ui::CONTROL_RADIUS, theme.main.gamma_multiply(0.16)); }
    else if response.hovered() { painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
    if focused || response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let (name_rect, kind, modified, size) = columns.split(rect);
    let icon = egui::Rect::from_center_size(egui::pos2(name_rect.left() + 9.0, rect.center().y), egui::vec2(18.0, 18.0));
    paint_entry_icon(painter, icon, entry);
    let ink = if entry.directory || marked { ui::TEXT } else { ui::TEXT };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), ui::display(13.5), ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width((name_rect.width() - 28.0).max(8.0));
    let galley = ui.painter().layout_job(job);
    painter.galley(egui::pos2(name_rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    let small = |rect: egui::Rect, text: String, right: bool| {
        let mut job = egui::text::LayoutJob::simple_singleline(text, ui::mono(11.5), ui::SECONDARY);
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
        let galley = ui.painter().layout_job(job);
        let x = if right { rect.right() - galley.size().x } else { rect.left() };
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, ui::SECONDARY);
    };
    if let Some(r) = kind { small(r, location.map_or_else(|| kind_of(entry).to_string(), str::to_string), false); }
    if let Some(r) = modified { small(r, when(entry.modified), false); }
    if let Some(r) = size { small(r, if entry.directory { "—".into() } else { file_size(entry.bytes) }, true); }
    response
}

const TILE_W: f32 = 132.0;
const TILE_H: f32 = 154.0;
const GRID_GAP: f32 = 10.0;

/// The sorts side by side, for the grid, which has no columns to click.
fn sort_labels(rect: egui::Rect) -> Vec<(SortKey, &'static str, egui::Rect, bool)> {
    let mut x = rect.left() + 10.0;
    [(SortKey::Name, "NAME"), (SortKey::Kind, "KIND"), (SortKey::Modified, "MODIFIED"), (SortKey::Size, "SIZE")].into_iter().map(|(key, label)| {
        let width = 26.0 + label.len() as f32 * 7.0;
        let r = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + width, rect.bottom()));
        x += width + 14.0;
        (key, label, r, false)
    }).collect()
}

/// One entry in the grid: its picture or icon, then its name.
#[allow(clippy::too_many_arguments)]
fn grid_tile(ui: &mut egui::Ui, theme: &Theme, entry: &Entry, name: &str, width: f32, marked: bool, focused: bool, thumb: Option<&egui::TextureHandle>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, TILE_H), egui::Sense::click_and_drag());
    let painter = ui.painter();
    if marked { painter.rect_filled(rect, ui::SURFACE_RADIUS, theme.main.gamma_multiply(0.16)); }
    else if response.hovered() { painter.rect_filled(rect, ui::SURFACE_RADIUS, ui::TRACK); }
    if focused || response.has_focus() { painter.rect_stroke(rect, ui::SURFACE_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let picture = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(width - 16.0, TILE_H - 50.0));
    match thumb {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (picture.width() / size.x).min(picture.height() / size.y).min(1.0);
            let shown = egui::Rect::from_center_size(picture.center(), size * scale);
            painter.image(texture.id(), shown, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            painter.rect_stroke(shown, 2.0, egui::Stroke::new(1.0, ui::LINE), egui::StrokeKind::Outside);
        }
        None => {
            let icon = egui::Rect::from_center_size(picture.center(), egui::vec2(18.0, 18.0));
            // The line icon, drawn larger by scaling its frame.
            let big = egui::Rect::from_center_size(icon.center(), egui::vec2(44.0, 44.0));
            paint_entry_icon(painter, big, entry);
        }
    }
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), ui::display(12.5), ui::TEXT);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - 12.0);
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    painter.galley(egui::pos2(rect.center().x, picture.bottom() + 8.0), galley, ui::TEXT);
    let meta = if entry.directory { "Folder".to_string() } else { file_size(entry.bytes) };
    painter.text(egui::pos2(rect.center().x, picture.bottom() + 28.0), egui::Align2::CENTER_TOP, meta, ui::mono(10.5), ui::LABEL);
    response
}

/// A folder, a picture, or a sheet with a turned corner, drawn in lines.
fn paint_entry_icon(painter: &egui::Painter, rect: egui::Rect, entry: &Entry) {
    let stroke = |color| egui::Stroke::new(1.3, color);
    if entry.directory {
        let body = egui::Rect::from_min_max(rect.left_top() + egui::vec2(1.0, 5.0), rect.right_bottom() - egui::vec2(1.0, 2.0));
        painter.rect_stroke(body, 2.0, stroke(ui::TEXT), egui::StrokeKind::Inside);
        painter.line_segment([body.left_top(), body.left_top() + egui::vec2(6.0, -3.0)], stroke(ui::TEXT));
        painter.line_segment([body.left_top() + egui::vec2(6.0, -3.0), body.left_top() + egui::vec2(9.0, 0.0)], stroke(ui::TEXT));
        return;
    }
    let sheet = egui::Rect::from_min_max(rect.left_top() + egui::vec2(3.0, 1.0), rect.right_bottom() - egui::vec2(3.0, 1.0));
    let fold = 5.0;
    let points = vec![sheet.left_top(), sheet.right_top() - egui::vec2(fold, 0.0), sheet.right_top() + egui::vec2(0.0, fold), sheet.right_bottom(), sheet.left_bottom(), sheet.left_top()];
    painter.add(egui::Shape::line(points, stroke(ui::SECONDARY)));
    painter.line_segment([sheet.right_top() - egui::vec2(fold, 0.0), sheet.right_top() + egui::vec2(-fold, fold)], stroke(ui::SECONDARY));
    painter.line_segment([sheet.right_top() + egui::vec2(-fold, fold), sheet.right_top() + egui::vec2(0.0, fold)], stroke(ui::SECONDARY));
    if kind_of(entry) == "Image" {
        let base = sheet.bottom() - 3.0;
        painter.add(egui::Shape::line(vec![egui::pos2(sheet.left() + 2.0, base), egui::pos2(sheet.left() + 5.0, base - 4.0), egui::pos2(sheet.left() + 7.0, base - 2.0), egui::pos2(sheet.right() - 2.0, base - 6.0)], stroke(ui::SECONDARY)));
    }
}

#[derive(Clone, Copy)]
enum Nav { Back, Forward, Up, Terminal }
fn nav_button(ui: &mut egui::Ui, theme: &Theme, kind: Nav, enabled: bool) -> egui::Response {
    let size = ui::CONTROL_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let painter = ui.painter();
    let lit = enabled && (response.hovered() || response.has_focus());
    if lit { painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
    painter.rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, if enabled { ui::LINE_STRONG } else { ui::LINE }), egui::StrokeKind::Inside);
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let color = if !enabled { ui::LINE_STRONG } else if lit { ui::TEXT } else { ui::SECONDARY };
    let stroke = egui::Stroke::new(1.6, color);
    let c = rect.center();
    let line = |points: Vec<egui::Pos2>| painter.add(egui::Shape::line(points, stroke));
    match kind {
        Nav::Back => { line(vec![c + egui::vec2(3.0, -6.0), c + egui::vec2(-3.0, 0.0), c + egui::vec2(3.0, 6.0)]); },
        Nav::Forward => { line(vec![c + egui::vec2(-3.0, -6.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-3.0, 6.0)]); },
        Nav::Up => { line(vec![c + egui::vec2(0.0, 7.0), c + egui::vec2(0.0, -7.0)]); line(vec![c + egui::vec2(-5.0, -2.0), c + egui::vec2(0.0, -7.0), c + egui::vec2(5.0, -2.0)]); },
        Nav::Terminal => { line(vec![c + egui::vec2(-6.0, -5.0), c + egui::vec2(-1.0, 0.0), c + egui::vec2(-6.0, 5.0)]); line(vec![c + egui::vec2(1.0, 6.0), c + egui::vec2(7.0, 6.0)]); },
    }
    let name = match kind { Nav::Back => "Back", Nav::Forward => "Forward", Nav::Up => "Up one folder", Nav::Terminal => "Open a terminal here" };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    response
}

fn side_label(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| { ui.add_space(10.0); ui::tracked(ui, title, ui::mono(10.5), ui::LABEL, 1.4); });
    ui.add_space(2.0);
}
fn place_item(ui: &mut egui::Ui, theme: &Theme, width: f32, title: &str, hint: &str, current: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ui::CONTROL_HEIGHT), egui::Sense::click());
    let lit = response.hovered() || response.has_focus();
    let fill = if current { theme.main.gamma_multiply(0.16) } else if lit { ui::TRACK } else { egui::Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, fill);
    if response.has_focus() { ui.painter().rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let color = if current || lit { ui::TEXT } else { ui::SECONDARY };
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), ui::display(13.0), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - 20.0).max(1.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if !hint.is_empty() { ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, hint, ui::mono(11.0), ui::LABEL); }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, current, title));
    response
}
/// A drive in the sidebar: its name and size, and an eject button while it can be ejected.
fn drive_item(ui: &mut egui::Ui, theme: &Theme, width: f32, drive: &crate::drives::Drive, current: bool, busy: bool) -> (egui::Response, Option<egui::Response>) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ui::CONTROL_HEIGHT), egui::Sense::click());
    let eject_rect = egui::Rect::from_min_size(egui::pos2(rect.right() - ui::CONTROL_HEIGHT, rect.top()), egui::vec2(ui::CONTROL_HEIGHT, ui::CONTROL_HEIGHT));
    let eject = ui.interact(eject_rect, response.id.with("eject"), if busy { egui::Sense::hover() } else { egui::Sense::click() });
    let lit = response.hovered() || response.has_focus();
    let fill = if current { theme.main.gamma_multiply(0.16) } else if lit { ui::TRACK } else { egui::Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, fill);
    if response.has_focus() { ui.painter().rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let color = if current || lit { ui::TEXT } else { ui::SECONDARY };
    let mut job = egui::text::LayoutJob::simple_singleline(drive.name.clone(), ui::display(13.0), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - 20.0 - ui::CONTROL_HEIGHT).max(1.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
    // An eject mark: a triangle over a bar.
    let ink = if eject.hovered() { ui::TEXT } else { ui::LABEL };
    if eject.hovered() { ui.painter().rect_filled(eject_rect.shrink(3.0), ui::CONTROL_RADIUS, ui::LINE); }
    let c = eject_rect.center();
    ui.painter().add(egui::Shape::convex_polygon(vec![c + egui::vec2(-5.0, 1.0), c + egui::vec2(0.0, -5.0), c + egui::vec2(5.0, 1.0)], ink, egui::Stroke::NONE));
    ui.painter().rect_filled(egui::Rect::from_min_size(c + egui::vec2(-5.0, 3.0), egui::vec2(10.0, 2.0)), 0.0, ink);
    let mounted = if drive.mountpoint.is_some() { "open" } else { "not open yet" };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, current, format!("{}, {mounted}", drive.name)));
    eject.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, format!("Eject {}", drive.name)));
    let eject = eject.on_hover_text("Eject");
    (response, Some(eject))
}
fn menu_item_with(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, ink: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(230.0, 28.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if enabled && (response.hovered() || response.has_focus()) { ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
    let color = if enabled { ink } else { ui::LABEL };
    ui.painter().text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, label, ui::display(13.0), color);
    if !shortcut.is_empty() { ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, shortcut, ui::mono(10.5), ui::LABEL); }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}
fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> egui::Response { menu_item_with(ui, label, shortcut, enabled, ui::TEXT) }
fn menu_item_caution(ui: &mut egui::Ui, label: &str, shortcut: &str) -> egui::Response { menu_item_with(ui, label, shortcut, true, ui::CAUTION) }
fn menu_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), egui::Sense::hover());
    ui.painter().hline(rect.x_range().shrink(4.0), rect.center().y, egui::Stroke::new(1.0, ui::LINE));
}
fn menu_heading(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| { ui.add_space(10.0); ui::tracked(ui, text, ui::mono(10.0), ui::LABEL, 1.4); });
}

/// The file manager: browsers in tabs, sharing the clipboard and the work queue.
pub struct Window {
    tabs: Vec<Browser>, active: usize,
    /// A second folder beside the first, and whether it has the keyboard.
    split: Option<Browser>, right_focus: bool,
}
const MAX_FILE_TABS: usize = 12;
impl Window {
    pub fn new(path: PathBuf) -> Self { Window { tabs: vec![Browser::new(path, Mode::Browse)], active: 0, split: None, right_focus: false } }
    pub fn captures_keyboard(&self) -> bool { self.tabs[self.active].captures_keyboard() || self.split.as_ref().is_some_and(Browser::captures_keyboard) }
    pub fn set_phones(&mut self, phones: Vec<(String, String)>) {
        for tab in self.tabs.iter_mut().chain(self.split.as_mut()) { tab.set_phones(phones.clone()); }
    }
    pub fn configure_operations(&mut self, clipboard: Option<Clipboard>, transfers: crate::transfers::View) {
        for tab in self.tabs.iter_mut().chain(self.split.as_mut()) { tab.configure_operations(clipboard.clone(), transfers.clone()); }
    }
    /// The active tab gets the answer; the others look again, as the change may show there.
    pub fn operation_result(&mut self, ctx: &egui::Context, result: Result<&str, &str>) {
        let focused = if self.split.is_some() && self.right_focus { usize::MAX } else { self.active };
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if index == focused { tab.operation_result(ctx, result); } else if result.is_ok() { tab.refresh(ctx); }
        }
        if let Some(split) = &mut self.split {
            if focused == usize::MAX { split.operation_result(ctx, result); } else if result.is_ok() { split.refresh(ctx); }
        }
    }
    fn open_tab(&mut self, path: PathBuf) {
        if self.tabs.len() >= MAX_FILE_TABS { return; }
        self.active += 1;
        self.tabs.insert(self.active, Browser::new(path, Mode::Browse));
    }
    /// Closes a tab; returns true when it was the last one.
    fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() == 1 { return true; }
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() { self.active = self.active.saturating_sub(1); }
        false
    }
    pub fn show_with_keyboard(&mut self, ui: &mut egui::Ui, theme: &Theme, active: bool) -> Option<Event> {
        let keys = active && !self.captures_keyboard();
        let mut close = false;
        if keys {
            let pressed = |ui: &mut egui::Ui, modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
            if pressed(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab) || pressed(ui, Modifiers::COMMAND, Key::PageUp) {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            } else if pressed(ui, Modifiers::COMMAND, Key::Tab) || pressed(ui, Modifiers::COMMAND, Key::PageDown) {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if pressed(ui, Modifiers::COMMAND, Key::T) { let path = self.tabs[self.active].path.clone(); self.open_tab(path); }
            if pressed(ui, Modifiers::COMMAND, Key::W) { close = self.close_tab(self.active); }
            if pressed(ui, Modifiers::NONE, Key::F3) {
                self.split = match self.split.take() { Some(_) => None, None => Some(Browser::new(self.tabs[self.active].path.clone(), Mode::Browse)) };
                self.right_focus = self.split.is_some();
            }
            if let Some(split) = &self.split {
                let copy = pressed(ui, Modifiers::NONE, Key::F5);
                let send = pressed(ui, Modifiers::NONE, Key::F6);
                if copy || send {
                    let (from, to) = if self.right_focus { (split, &self.tabs[self.active]) } else { (&self.tabs[self.active], split) };
                    let directory = to.path.clone();
                    let operations: Vec<crate::file_ops::Operation> = from.chosen().into_iter().filter(|_| !from.in_trash).map(|source| if send {
                        crate::file_ops::Operation::Move { source, directory: directory.clone(), conflict: Default::default() }
                    } else { crate::file_ops::Operation::Copy { source, directory: directory.clone(), conflict: Default::default() } }).collect();
                    if !operations.is_empty() { return Some(Event::Operate(operations)); }
                }
            }
        }
        if close { return Some(Event::Close); }
        // The strip of tabs; with one, just the place for a new one.
        let mut select = None; let mut shut = None; let mut add = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let room = ui.available_width() - ui::CONTROL_HEIGHT - 8.0;
            let width = (room / self.tabs.len() as f32).clamp(60.0, 200.0);
            for (index, tab) in self.tabs.iter().enumerate() {
                let (open, close) = tab_item(ui, theme, width, &tab.title(), index == self.active, self.tabs.len() > 1);
                if open.clicked() { select = Some(index); }
                if open.middle_clicked() || close.is_some_and(|c| c.clicked()) { shut = Some(index); }
            }
            if self.tabs.len() < MAX_FILE_TABS {
                add = new_tab_button(ui, theme).on_hover_text("New tab · Ctrl+T").clicked();
            }
        });
        ui.add_space(8.0);
        if let Some(index) = select { self.active = index; }
        if let Some(index) = shut { if self.close_tab(index) { return Some(Event::Close); } }
        if add { let path = self.tabs[self.active].path.clone(); self.open_tab(path); }
        let event = match &mut self.split {
            None => self.tabs[self.active].show_with_keyboard(ui, theme, active),
            Some(split) => {
                let full = ui.available_rect_before_wrap();
                let gap = 14.0;
                let half = ((full.width() - gap) / 2.0).max(1.0);
                let left = egui::Rect::from_min_size(full.min, egui::vec2(half, full.height()));
                let right = egui::Rect::from_min_size(egui::pos2(full.left() + half + gap, full.top()), egui::vec2(half, full.height()));
                // A press inside a side gives it the keyboard.
                if let Some(at) = ui.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten()) {
                    if left.contains(at) { self.right_focus = false; } else if right.contains(at) { self.right_focus = true; }
                }
                let mut left_ui = ui.new_child(egui::UiBuilder::new().id_salt("files-left").max_rect(left).layout(egui::Layout::top_down(egui::Align::Min)));
                let from_left = self.tabs[self.active].show_with_keyboard(&mut left_ui, theme, active && !self.right_focus);
                let mut right_ui = ui.new_child(egui::UiBuilder::new().id_salt("files-right").max_rect(right).layout(egui::Layout::top_down(egui::Align::Min)));
                let from_right = split.show_with_keyboard(&mut right_ui, theme, active && self.right_focus);
                let focused = if self.right_focus { right } else { left };
                ui.painter().rect_stroke(focused.expand(5.0), ui::SURFACE_RADIUS, egui::Stroke::new(1.0, theme.main.gamma_multiply(0.7)), egui::StrokeKind::Inside);
                ui.painter().vline(full.left() + half + gap / 2.0, full.y_range(), egui::Stroke::new(1.0, ui::LINE));
                ui.allocate_rect(full, egui::Sense::hover());
                from_left.or(from_right)
            },
        };
        match event {
            Some(Event::NewTab(path)) => { self.open_tab(path); None },
            other => other,
        }
    }
}

fn tab_item(ui: &mut egui::Ui, theme: &Theme, width: f32, title: &str, current: bool, closable: bool) -> (egui::Response, Option<egui::Response>) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ui::CONTROL_HEIGHT), egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(20.0, 20.0));
    let close = closable.then(|| ui.interact(close_rect, response.id.with("close"), egui::Sense::click()));
    let lit = response.hovered() || response.has_focus();
    let painter = ui.painter();
    painter.rect_filled(rect, ui::CONTROL_RADIUS, if current { ui::PANEL } else if lit { ui::TRACK } else { egui::Color32::TRANSPARENT });
    if current {
        painter.rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, ui::LINE), egui::StrokeKind::Inside);
        painter.hline(rect.x_range().shrink(6.0), rect.top() + 1.0, egui::Stroke::new(2.0, theme.main));
    }
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let color = if current || lit { ui::TEXT } else { ui::SECONDARY };
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), ui::display(12.5), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - if closable { 40.0 } else { 20.0 }).max(8.0));
    let galley = ui.painter().layout_job(job);
    painter.galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if let Some(close) = &close {
        if current || lit || close.hovered() {
            if close.hovered() { painter.rect_filled(close_rect, 4.0, ui::LINE); }
            let c = close_rect.center(); let ink = if close.hovered() { ui::TEXT } else { ui::LABEL };
            painter.line_segment([c + egui::vec2(-4.0, -4.0), c + egui::vec2(4.0, 4.0)], egui::Stroke::new(1.3, ink));
            painter.line_segment([c + egui::vec2(4.0, -4.0), c + egui::vec2(-4.0, 4.0)], egui::Stroke::new(1.3, ink));
        }
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}")));
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, current, title));
    (response, close)
}

fn new_tab_button(ui: &mut egui::Ui, theme: &Theme) -> egui::Response {
    let size = ui::CONTROL_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let lit = response.hovered() || response.has_focus();
    if lit { ui.painter().rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
    if response.has_focus() { ui.painter().rect_stroke(rect, ui::CONTROL_RADIUS, egui::Stroke::new(1.0, theme.main), egui::StrokeKind::Inside); }
    let c = rect.center(); let ink = if lit { ui::TEXT } else { ui::SECONDARY };
    ui.painter().line_segment([c + egui::vec2(-6.0, 0.0), c + egui::vec2(6.0, 0.0)], egui::Stroke::new(1.6, ink));
    ui.painter().line_segment([c + egui::vec2(0.0, -6.0), c + egui::vec2(0.0, 6.0)], egui::Stroke::new(1.6, ink));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New tab"));
    response
}

pub fn request(ctx: &egui::Context, path: PathBuf) { ctx.data_mut(|data| data.insert_temp(egui::Id::new("open-native-files"), path)); }
pub fn take_request(ctx: &egui::Context) -> Option<PathBuf> { ctx.data_mut(|data| data.remove_temp(egui::Id::new("open-native-files"))) }


/// One application-owned operation can run across navigation or workspace changes.
/// Dropping the application requests cancellation; copy staging stays private until commit.
pub struct Task {
    receiver: std::sync::mpsc::Receiver<Result<crate::file_ops::Completed, String>>,
    pub progress: Arc<AtomicU64>, cancelled: Arc<AtomicBool>, pub operation: crate::file_ops::Operation,
}
static OPERATION_BUSY: AtomicBool = AtomicBool::new(false);
struct OperationLease;
impl Drop for OperationLease { fn drop(&mut self) { OPERATION_BUSY.store(false, Ordering::Release); } }
impl Task {
    pub fn start(operation: crate::file_ops::Operation, ctx: egui::Context) -> Result<Self, String> {
        OPERATION_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).map_err(|_| "Another file operation is still running".to_string())?;
        let lease = OperationLease;
        let (send, receiver) = std::sync::mpsc::sync_channel(1);
        let progress = Arc::new(AtomicU64::new(0)); let cancelled = Arc::new(AtomicBool::new(false));
        let (count, cancel, work) = (progress.clone(), cancelled.clone(), operation.clone());
        std::thread::Builder::new().name("native-file-operation".into()).spawn(move || {
            let _lease = lease;
            let result = crate::file_ops::execute(work, &cancel, &count);
            let _ = send.send(result); ctx.request_repaint();
        }).map_err(|e| format!("Could not start file operation: {e}"))?;
        Ok(Self { receiver, progress, cancelled, operation })
    }
    pub fn poll(&self) -> Option<Result<crate::file_ops::Completed, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("File operation ended unexpectedly. Check the folder before retrying.".into())),
        }
    }
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Release); }
}
impl Drop for Task { fn drop(&mut self) { self.cancel(); } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_files_listing_preserves_paths_and_filters_image_picks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Folder")).unwrap();
        for name in ["image # = 雨.png", "notes.txt", ".hidden.jpg"] { std::fs::write(dir.path().join(name), b"test").unwrap(); }
        let listing = read_directory(dir.path()).unwrap();
        assert!(listing.entries[0].directory);
        let mut browser = Browser::new(dir.path().into(), Mode::ImagePicker);
        browser.entries = listing.entries;
        let visible = browser.visible();
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().any(|&i| browser.entries[i].path == dir.path().join("image # = 雨.png")));
        browser.hidden = true;
        assert_eq!(browser.visible().len(), 3);
        browser.filter = "雨".into();
        assert_eq!(browser.visible().len(), 1);
    }
    #[test]
    fn native_files_failed_or_stale_navigation_keeps_the_current_directory() {
        let mut browser = Browser::new("/tmp/current".into(), Mode::Browse);
        browser.generation = 3;
        browser.pending = Some(("/tmp/latest".into(), Navigation::New));
        browser.loading = true;
        browser.accept(2, Ok(Listing { path: "/tmp/latest".into(), entries: Vec::new(), limited: false, trash: false, labels: Default::default() }));
        assert!(browser.loading);
        assert_eq!(browser.path, PathBuf::from("/tmp/current"));
        browser.accept(3, Err("Permission denied".into()));
        assert!(!browser.loading);
        assert_eq!(browser.path, PathBuf::from("/tmp/current"));
        assert!(browser.error.as_ref().unwrap().contains("Permission denied"));
    }
    #[test]
    fn native_file_operation_failure_preserves_the_dialog_name() {
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.dialog = Some(OperationDialog { kind: DialogKind::NewFolder, name: "keep my name".into(), pending: true, focused: true, error: None });
        browser.operation_result(&egui::Context::default(), Err("Already exists"));
        let dialog = browser.dialog.as_ref().unwrap();
        assert_eq!(dialog.name, "keep my name");
        assert!(!dialog.pending);
        assert_eq!(dialog.error.as_deref(), Some("Already exists"));
    }
    #[test]
    fn native_file_operation_worker_reports_completion_without_blocking_the_ui() {
        let directory = tempfile::tempdir().unwrap();
        let task = Task::start(crate::file_ops::Operation::CreateFolder { parent: directory.path().into(), name: "new folder".into() }, egui::Context::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let result = loop {
            if let Some(result) = task.poll() { break result; }
            assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(result.unwrap().path, directory.path().join("new folder"));
        assert!(directory.path().join("new folder").is_dir());
    }
    #[test]
    fn native_files_filtered_out_selection_cannot_be_activated() {
        let mut browser = Browser::new("/tmp".into(), Mode::Browse);
        browser.entries = vec![Entry { path: "/tmp/hidden-selection".into(), directory: true, bytes: 0, modified: None }];
        browser.selected = Some("/tmp/hidden-selection".into()); browser.filter = "different".into();
        assert!(browser.activate(&egui::Context::default()).is_none());
        assert!(!browser.loading, "Enter must not navigate into a filtered-out selection");
    }
    #[test]
    fn native_files_cannot_pick_a_folder_or_unsupported_image() {
        let mut browser = Browser::new("/tmp".into(), Mode::ImagePicker);
        browser.entries = vec![Entry { path: "/tmp/folder".into(), directory: true, bytes: 0, modified: None }, Entry { path: "/tmp/a.txt".into(), directory: false, bytes: 10, modified: None }, Entry { path: "/tmp/a.PNG".into(), directory: false, bytes: 10, modified: None }];
        browser.selected = Some("/tmp/folder".into());
        assert!(!browser.can_choose());
        browser.selected = Some("/tmp/a.txt".into());
        assert!(!browser.can_choose());
        browser.selected = Some("/tmp/a.PNG".into());
        assert!(browser.can_choose());
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    #[test]
    fn native_clipboard_copy_and_cut_events_select_the_file() {
        for (event, cut) in [(egui::Event::Copy, false), (egui::Event::Cut, true)] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
            browser.entries = vec![Entry { path: "/tmp/selected.png".into(), directory: false, bytes: 10, modified: None }];
            browser.selected = Some("/tmp/selected.png".into());
            render(&mut browser, &ctx, Vec::new(), 700.0);
            let action = render(&mut browser, &ctx, vec![event], 700.0).1;
            assert!(matches!(action, Some(Event::Clipboard(Clipboard { sources, cut: actual })) if sources == vec![PathBuf::from("/tmp/selected.png")] && actual == cut));
        }
    }
    #[test]
    fn native_clipboard_events_stay_with_another_keyboard_owner() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.selected = Some("/tmp/source.txt".into());
        let mut action = None;
        let _ = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 680.0))),
            events: vec![egui::Event::Copy, egui::Event::Cut, egui::Event::Paste("keep".into())],
            ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| action = browser.show_with_keyboard(ui, &Theme::load("signal"), false));
            assert_eq!(ctx.input(|input| input.events.iter().filter(|event| matches!(event, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).count()), 3);
        });
        assert!(action.is_none());
    }

    #[test]
    fn native_clipboard_paste_uses_the_selected_file_clipboard() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp/destination".into(), Mode::Browse); browser.loaded = true;
        browser.configure_operations(Some(Clipboard { sources: vec!["/tmp/source.txt".into()], cut: false }), Default::default());
        render(&mut browser, &ctx, Vec::new(), 700.0);
        let action = render(&mut browser, &ctx, vec![egui::Event::Paste("unrelated system clipboard text".into())], 700.0).1;
        assert!(matches!(action.as_ref().map(|event| match event { Event::Operate(ops) => ops.as_slice(), _ => &[] }), Some([crate::file_ops::Operation::Copy { source, .. }]) if *source == PathBuf::from("/tmp/source.txt")));
    }

    #[test]
    fn paste_shortcut_works_when_the_system_clipboard_has_no_text() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp/destination".into(), Mode::Browse); browser.loaded = true;
        browser.configure_operations(Some(Clipboard { sources: vec!["/tmp/source.txt".into()], cut: false }), Default::default());
        render(&mut browser, &ctx, Vec::new(), 700.0);
        let mut action = None;
        let _ = ctx.run(egui::RawInput { modifiers: Modifiers::CTRL, events: vec![egui::Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: false, repeat: false, modifiers: Modifiers::CTRL }], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| action = browser.show(ui, &Theme::load("signal")));
        });
        assert!(matches!(action.as_ref().map(|event| match event { Event::Operate(ops) => ops.as_slice(), _ => &[] }), Some([crate::file_ops::Operation::Copy { source, .. }]) if *source == PathBuf::from("/tmp/source.txt")));
    }

    #[test]
    fn clipboard_paste_event_and_key_release_do_not_paste_twice() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp/destination".into(), Mode::Browse); browser.loaded = true;
        browser.configure_operations(Some(Clipboard { sources: vec!["/tmp/source.txt".into()], cut: false }), Default::default());
        render(&mut browser, &ctx, Vec::new(), 700.0);
        let mut run = |events| { let mut action = None; let _ = ctx.run(egui::RawInput { modifiers: Modifiers::CTRL, events, ..Default::default() }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| action = browser.show(ui, &Theme::load("signal"))); }); action };
        assert!(matches!(run(vec![egui::Event::Paste("text".into())]), Some(Event::Operate(_))));
        assert!(run(vec![egui::Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: false, repeat: false, modifiers: Modifiers::CTRL }]).is_none());
    }

    #[test]
    fn dragging_an_entry_emits_one_desktop_drag_without_opening_it() {
        for mode in [Mode::Browse, Mode::ImagePicker] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let mut browser = Browser::new("/tmp".into(), mode); browser.loaded = true;
            browser.entries = vec![Entry { path: "/tmp/drag me.png".into(), directory: false, bytes: 10, modified: None }];
            let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
            let at = button(&output, "drag me.png");
            render(&mut browser, &ctx, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton {
                pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE,
            }], 700.0);
            let action = render(&mut browser, &ctx, vec![egui::Event::PointerMoved(at + egui::vec2(30.0, 0.0))], 700.0).1;
            if mode == Mode::Browse {
                assert!(matches!(action, Some(Event::Drag(path)) if path == PathBuf::from("/tmp/drag me.png")));
            } else { assert!(action.is_none(), "image selection must not start an external drag"); }
            assert!(render(&mut browser, &ctx, vec![egui::Event::PointerMoved(at + egui::vec2(50.0, 0.0))], 700.0).1.is_none());
        }
    }
    fn render(browser: &mut Browser, ctx: &egui::Context, events: Vec<egui::Event>, width: f32) -> (egui::FullOutput, Option<Event>) {
        let mut action = None;
        let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 680.0))), events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { action = browser.show(ui, &Theme::load("signal")); });
        });
        (output, action)
    }
    fn button(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output.shapes.iter().find_map(|shape| if let egui::Shape::Text(text) = &shape.shape {
            (text.galley.job.text == label).then_some(text.pos + text.galley.size() / 2.0)
        } else { None }).unwrap_or_else(|| panic!("missing {label}"))
    }
    fn click(browser: &mut Browser, ctx: &egui::Context, at: egui::Pos2) -> Option<Event> {
        let mut action = None;
        for pressed in [true, false] {
            action = render(browser, ctx, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }], 700.0).1.or(action);
        }
        action
    }
    #[test]
    fn native_files_picker_click_selects_then_explicitly_chooses_a_path() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::ImagePicker);
        browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/雨 #.png".into(), directory: false, bytes: 10, modified: None }];
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "雨 #.png")).is_none());
        assert_eq!(browser.selected, Some(PathBuf::from("/tmp/雨 #.png")));
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        match click(&mut browser, &ctx, button(&output, "CHOOSE IMAGE")) { Some(Event::Choose(path)) => assert_eq!(path, PathBuf::from("/tmp/雨 #.png")), _ => panic!("image picker did not return selected path") }
    }
    fn right_click(browser: &mut Browser, ctx: &egui::Context, at: egui::Pos2) {
        for pressed in [true, false] {
            render(browser, ctx, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE }], 900.0);
        }
    }
    fn key(browser: &mut Browser, ctx: &egui::Context, key: Key, modifiers: Modifiers, width: f32) -> Option<Event> {
        render(browser, ctx, vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }], width).1
    }
    #[test]
    fn native_files_context_menu_owns_arrows_and_enter_before_the_list() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/first.txt".into(), directory: false, bytes: 2, modified: None }, Entry { path: "/tmp/second.txt".into(), directory: false, bytes: 2, modified: None }];
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        right_click(&mut browser, &ctx, button(&output, "first.txt"));
        assert_eq!(browser.selected, Some(PathBuf::from("/tmp/first.txt")), "a right click selects what it is on");
        assert!(browser.captures_keyboard(), "the open menu owns the keys");
        for k in [Key::ArrowDown, Key::Enter] {
            let event = key(&mut browser, &ctx, k, Modifiers::NONE, 900.0);
            assert!(!matches!(event, Some(Event::Open(_))), "menu key opened a file behind the menu");
            assert_eq!(browser.selected, Some(PathBuf::from("/tmp/first.txt")), "menu navigation changed the operation target");
        }
    }
    #[test]
    fn native_files_context_menu_keeps_operations_discoverable_and_picker_read_only() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/notes.txt".into(), directory: false, bytes: 2048, modified: None }];
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        right_click(&mut browser, &ctx, button(&output, "notes.txt"));
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        let at = button(&output, "Rename");
        for pressed in [true, false] {
            render(&mut browser, &ctx, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }], 900.0);
        }
        assert!(matches!(browser.dialog.as_ref().map(|dialog| &dialog.kind), Some(DialogKind::Rename(path)) if path == Path::new("/tmp/notes.txt")));

        let mut picker = Browser::new("/tmp".into(), Mode::ImagePicker); picker.loaded = true;
        let picker_ctx = egui::Context::default(); crate::ui::install_fonts(&picker_ctx, None, None);
        let output = render(&mut picker, &picker_ctx, Vec::new(), 320.0).0;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape { assert!(!matches!(text.galley.job.text.as_str(), "NEW FOLDER" | "PASTE" | "Move to Trash" | "TRASH")); }
        }
    }
    #[test]
    fn native_files_path_enter_navigates_without_opening_the_selected_file() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/selected.png".into(), directory: false, bytes: 10, modified: None }];
        browser.selected = Some("/tmp/selected.png".into());
        render(&mut browser, &ctx, Vec::new(), 900.0);
        key(&mut browser, &ctx, Key::L, Modifiers::COMMAND, 900.0);
        assert!(browser.editing_path, "Ctrl+L types a path");
        render(&mut browser, &ctx, Vec::new(), 900.0);
        browser.path_text = "/tmp/typed-folder".into();
        let action = key(&mut browser, &ctx, Key::Enter, Modifiers::NONE, 900.0);
        assert!(action.is_none(), "Enter in the path field must not open an unrelated selection");
        assert_eq!(browser.pending.as_ref().unwrap().0, PathBuf::from("/tmp/typed-folder"));
    }
    #[test]
    fn native_files_select_with_ctrl_and_shift_and_act_on_all_of_them() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = ["a.txt", "b.txt", "c.txt", "d.txt"].map(|n| Entry { path: PathBuf::from("/tmp").join(n), directory: false, bytes: 1, modified: None }).to_vec();
        let shown: Vec<PathBuf> = browser.visible().into_iter().map(|i| browser.entries[i].path.clone()).collect();
        browser.click(shown[0].clone(), Modifiers::NONE, &shown);
        browser.click(shown[2].clone(), Modifiers::SHIFT, &shown);
        assert_eq!(browser.chosen(), shown[0..=2].to_vec());
        browser.click(shown[1].clone(), Modifiers::CTRL, &shown);
        assert_eq!(browser.chosen(), vec![shown[0].clone(), shown[2].clone()]);
        let event = browser.perform(&ctx, Act { copy: true, ..Default::default() });
        assert!(matches!(event, Some(Event::Clipboard(Clipboard { sources, cut: false })) if sources == vec![shown[0].clone(), shown[2].clone()]));
        browser.perform(&ctx, Act { trash: true, ..Default::default() });
        assert!(matches!(browser.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Trash(sources)) if sources.len() == 2));
    }
    #[test]
    fn native_files_ctrl_click_adds_to_the_selection() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = ["a.txt", "b.txt"].map(|n| Entry { path: PathBuf::from("/tmp").join(n), directory: false, bytes: 1, modified: None }).to_vec();
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        let first = button(&output, "a.txt");
        for pressed in [true, false] {
            render(&mut browser, &ctx, vec![egui::Event::PointerMoved(first), egui::Event::PointerButton { pos: first, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }], 900.0);
        }
        assert_eq!(browser.chosen().len(), 1);
        let at = button(&output, "b.txt");
        for pressed in [true, false] {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 680.0))), modifiers: Modifiers::CTRL,
                events: vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::CTRL }], ..Default::default() },
                |ctx| { egui::CentralPanel::default().show(ctx, |ui| { browser.show(ui, &Theme::load("signal")); }); });
        }
        assert_eq!(browser.chosen().len(), 2, "Ctrl+click adds to the selection");
    }
    #[test]
    fn native_files_shift_arrows_extend_and_alt_up_goes_to_the_parent() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp/folder".into(), Mode::Browse); browser.loaded = true;
        browser.entries = ["a.txt", "b.txt", "c.txt"].map(|n| Entry { path: PathBuf::from("/tmp/folder").join(n), directory: false, bytes: 1, modified: None }).to_vec();
        render(&mut browser, &ctx, Vec::new(), 900.0);
        key(&mut browser, &ctx, Key::ArrowDown, Modifiers::NONE, 900.0);
        key(&mut browser, &ctx, Key::ArrowDown, Modifiers::SHIFT, 900.0);
        key(&mut browser, &ctx, Key::ArrowDown, Modifiers::SHIFT, 900.0);
        assert_eq!(browser.chosen().len(), 3, "Shift+Down extends the selection");
        key(&mut browser, &ctx, Key::ArrowUp, Modifiers::ALT, 900.0);
        assert_eq!(browser.pending.as_ref().map(|p| p.0.clone()), Some(PathBuf::from("/tmp")), "Alt+Up opens the parent");
    }
    #[test]
    fn file_tabs_open_close_and_the_last_close_leaves_files() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut window = Window::new("/tmp".into());
        let run = |window: &mut Window, events: Vec<egui::Event>| {
            let mut action = None;
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 700.0))), events, ..Default::default() },
                |ctx| { egui::CentralPanel::default().show(ctx, |ui| action = window.show_with_keyboard(ui, &Theme::load("signal"), true)); });
            action
        };
        let press = |key| vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }];
        run(&mut window, Vec::new());
        assert!(run(&mut window, press(Key::T)).is_none());
        assert_eq!((window.tabs.len(), window.active), (2, 1));
        assert!(run(&mut window, press(Key::W)).is_none());
        assert_eq!(window.tabs.len(), 1);
        assert!(matches!(run(&mut window, press(Key::W)), Some(Event::Close)), "closing the last tab leaves Files");
    }
    #[test]
    fn grid_arrows_move_across_then_down_a_row() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true; browser.view = View::Grid;
        browser.entries = (0..12).map(|n| Entry { path: PathBuf::from(format!("/tmp/{n:02}.txt")), directory: false, bytes: 1, modified: None }).collect();
        render(&mut browser, &ctx, Vec::new(), 900.0);
        let across = browser.grid_columns;
        assert!(across > 1, "a wide grid has several columns");
        key(&mut browser, &ctx, Key::ArrowRight, Modifiers::NONE, 900.0);
        key(&mut browser, &ctx, Key::ArrowRight, Modifiers::NONE, 900.0);
        assert_eq!(browser.selected, Some(PathBuf::from("/tmp/01.txt")));
        key(&mut browser, &ctx, Key::ArrowDown, Modifiers::NONE, 900.0);
        assert_eq!(browser.selected, Some(PathBuf::from(format!("/tmp/{:02}.txt", 1 + across))));
        for width in [320.0, 900.0] {
            let output = render(&mut browser, &ctx, Vec::new(), width).0;
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape { assert!(text.pos.x + text.galley.size().x <= width + 1.0, "grid text spilled at {width}: {}", text.galley.job.text); }
            }
        }
        assert_eq!(Browser::new("/tmp".into(), Mode::ImagePicker).view, View::Grid, "pictures are chosen from a grid");
    }
    #[test]
    fn ctrl_z_asks_for_undo_and_a_clash_asks_before_anything_else() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        render(&mut browser, &ctx, Vec::new(), 900.0);
        assert!(key(&mut browser, &ctx, Key::Z, Modifiers::COMMAND, 900.0).is_none(), "nothing to undo yet");
        browser.configure_operations(None, crate::transfers::View { undo: Some("Move 2 items to Trash".into()), ..Default::default() });
        assert!(matches!(key(&mut browser, &ctx, Key::Z, Modifiers::COMMAND, 900.0), Some(Event::Undo)));
        let clash = crate::transfers::Clash { name: "report.pdf".into(), incoming: (10, None), existing: (9, None), more: 2 };
        browser.configure_operations(None, crate::transfers::View { clash: Some(clash), ..Default::default() });
        assert!(browser.captures_keyboard(), "the question owns the keys");
        render(&mut browser, &ctx, Vec::new(), 900.0);
        assert!(matches!(key(&mut browser, &ctx, Key::B, Modifiers::NONE, 900.0), Some(Event::Resolve(crate::file_ops::Conflict::KeepBoth, true))));
        assert!(matches!(key(&mut browser, &ctx, Key::Escape, Modifiers::NONE, 900.0), Some(Event::ClearTransfers)));
        assert!(browser.transfers_open, "the panel comes up with the question");
    }
    #[test]
    fn enter_in_the_filter_searches_inside_and_clearing_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Work")).unwrap();
        std::fs::write(dir.path().join("Work/budget.ods"), b"x").unwrap();
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new(dir.path().into(), Mode::Browse);
        let deadline = Instant::now() + Duration::from_secs(5);
        while browser.entries.is_empty() { render(&mut browser, &ctx, Vec::new(), 900.0); assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5)); }
        browser.filter = "budget".into();
        browser.focus_filter = true;
        render(&mut browser, &ctx, Vec::new(), 900.0);
        key(&mut browser, &ctx, Key::Enter, Modifiers::NONE, 900.0);
        assert!(browser.search.is_some(), "Enter in the filter searches inside");
        while browser.search.as_ref().is_some_and(|s| !s.done) { render(&mut browser, &ctx, Vec::new(), 900.0); assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5)); }
        render(&mut browser, &ctx, Vec::new(), 900.0);
        assert_eq!(browser.items().len(), 1);
        assert_eq!(browser.location_of(&browser.items()[0].clone()).as_deref(), Some("Work"));
        browser.end_search();
        assert!(browser.search.is_none() && browser.items().len() == 1 && browser.items()[0].directory, "back to the folder");
    }
    #[test]
    fn f2_on_many_renames_them_together_and_alt_enter_shows_properties() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["a.txt", "b.txt"] { std::fs::write(dir.path().join(n), b"x").unwrap(); }
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new(dir.path().into(), Mode::Browse); browser.loaded = true;
        browser.entries = ["a.txt", "b.txt"].map(|n| Entry { path: dir.path().join(n), directory: false, bytes: 1, modified: None }).to_vec();
        render(&mut browser, &ctx, Vec::new(), 900.0);
        key(&mut browser, &ctx, Key::A, Modifiers::COMMAND, 900.0);
        key(&mut browser, &ctx, Key::F2, Modifiers::NONE, 900.0);
        assert!(browser.renaming.is_some(), "F2 on two items renames them together");
        browser.renaming = None;
        key(&mut browser, &ctx, Key::ArrowDown, Modifiers::NONE, 900.0);
        key(&mut browser, &ctx, Key::Enter, Modifiers::ALT, 900.0);
        assert!(browser.properties.is_some(), "Alt+Enter shows properties, and does not open");
    }
    #[test]
    fn f3_splits_and_f5_copies_to_the_other_side() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut window = Window::new("/tmp/left".into());
        let run = |window: &mut Window, events: Vec<egui::Event>| {
            let mut action = None;
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 700.0))), events, ..Default::default() },
                |ctx| { egui::CentralPanel::default().show(ctx, |ui| action = window.show_with_keyboard(ui, &Theme::load("signal"), true)); });
            action
        };
        let press = |key| vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }];
        run(&mut window, press(Key::F3));
        assert!(window.split.is_some() && window.right_focus);
        window.split.as_mut().unwrap().path = "/tmp/right".into();
        window.right_focus = false;
        let left = &mut window.tabs[0];
        left.entries = vec![Entry { path: "/tmp/left/a.txt".into(), directory: false, bytes: 1, modified: None }];
        left.select_only("/tmp/left/a.txt".into());
        let event = run(&mut window, press(Key::F5));
        assert!(matches!(event.as_ref().map(|e| match e { Event::Operate(ops) => ops.as_slice(), _ => &[] }),
            Some([crate::file_ops::Operation::Copy { source, directory, .. }]) if *source == PathBuf::from("/tmp/left/a.txt") && *directory == PathBuf::from("/tmp/right")));
        run(&mut window, press(Key::F3));
        assert!(window.split.is_none());
    }
    #[test]
    fn escape_in_the_filter_clears_it_without_closing_files() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.filter = "budget".into(); browser.focus_filter = true;
        render(&mut browser, &ctx, Vec::new(), 900.0);
        render(&mut browser, &ctx, Vec::new(), 900.0);
        let event = key(&mut browser, &ctx, Key::Escape, Modifiers::NONE, 900.0);
        assert!(event.is_none(), "Escape in the filter must not close Files");
        assert!(browser.filter.is_empty());
    }
    #[test]
    fn the_menu_offers_open_with_compress_extract_and_send_to_a_ready_phone() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/photos.zip".into(), directory: false, bytes: 9, modified: None }];
        browser.set_phones(vec![("tab".into(), "Galaxy Tab".into())]);
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        right_click(&mut browser, &ctx, button(&output, "photos.zip"));
        let output = render(&mut browser, &ctx, Vec::new(), 900.0).0;
        for label in ["Open with…", "Compress…", "Extract here", "Send to Galaxy Tab"] { button(&output, label); }
        let event = browser.perform(&ctx, Act { send: Some("tab".into()), ..Default::default() });
        assert!(matches!(event, Some(Event::SendTo(device, files)) if device == "tab" && files == vec![PathBuf::from("/tmp/photos.zip")]));
        let event = browser.perform(&ctx, Act { extract: true, ..Default::default() });
        assert!(matches!(event.as_ref().map(|e| match e { Event::Operate(ops) => ops.as_slice(), _ => &[] }), Some([crate::file_ops::Operation::Extract { .. }])));
        browser.perform(&ctx, Act { compress: true, ..Default::default() });
        assert!(browser.compressing.is_some());
    }
    #[test]
    fn native_files_sort_keeps_folders_first_and_reverses() {
        let mut browser = Browser::new("/tmp".into(), Mode::Browse);
        let t = |s| Some(std::time::UNIX_EPOCH + Duration::from_secs(s));
        browser.entries = vec![
            Entry { path: "/tmp/b.txt".into(), directory: false, bytes: 50, modified: t(1) },
            Entry { path: "/tmp/z".into(), directory: true, bytes: 0, modified: t(9) },
            Entry { path: "/tmp/a.pdf".into(), directory: false, bytes: 900, modified: t(5) },
        ];
        let names = |b: &Browser| b.visible().into_iter().map(|i| b.entries[i].path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(&browser), ["z", "a.pdf", "b.txt"]);
        browser.sort = SortKey::Size; browser.descending = true;
        assert_eq!(names(&browser), ["z", "a.pdf", "b.txt"]);
        browser.descending = false;
        assert_eq!(names(&browser), ["z", "b.txt", "a.pdf"]);
        browser.sort = SortKey::Modified;
        assert_eq!(names(&browser), ["z", "b.txt", "a.pdf"]);
    }
    #[test]
    fn native_files_does_not_consume_the_scratchpads_enter() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/selected.png".into(), directory: false, bytes: 10, modified: None }];
        browser.selected = Some("/tmp/selected.png".into());
        let mut action = None;
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 600.0))), events: vec![egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE }], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = browser.show_with_keyboard(ui, &Theme::load("signal"), false);
                assert!(ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)), "the active scratchpad still needs the event");
            });
        });
        assert!(action.is_none());
    }
    #[test]
    fn native_files_keyboard_selects_and_names_do_not_overflow_narrow_views() {
        for width in [320.0, 900.0] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
            browser.entries = vec![Entry { path: PathBuf::from(format!("/tmp/{}.png", "long name 雨 ".repeat(15))), directory: false, bytes: 10, modified: None }];
            let output = render(&mut browser, &ctx, Vec::new(), width).0;
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape { assert!(text.pos.x + text.galley.size().x <= width + 1.0, "text spilled at width {width}: {}", text.galley.job.text); }
            }
            render(&mut browser, &ctx, vec![egui::Event::Key { key: Key::ArrowDown, physical_key: Some(Key::ArrowDown), pressed: true, repeat: false, modifiers: Modifiers::NONE }], width);
            let action = render(&mut browser, &ctx, vec![egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE }], width).1;
            assert!(matches!(action, Some(Event::Open(_))));
        }
    }
}
