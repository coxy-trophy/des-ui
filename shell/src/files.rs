//! Shared native folder browser and file-picker foundation.

use eframe::egui::{self, Key, Modifiers};
use std::{path::{Path, PathBuf}, sync::{Arc, Mutex, Condvar, atomic::{AtomicBool, AtomicU64, Ordering}, OnceLock}, time::{Duration, Instant}};
use crate::ui;

#[derive(Clone)]
struct Entry { path: PathBuf, directory: bool, bytes: u64 }
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
        let bytes = if file_type.is_file() { item.metadata().map_or(0, |m| m.len()) } else { 0 };
        if trash {
            if let Some(name) = crate::file_ops::trash_original_name(&item.path()) {
                use std::os::unix::ffi::OsStrExt;
                if name.as_bytes().len() <= 1024 { labels.insert(item.path(), name.to_string_lossy().into_owned()); }
            }
        }
        entries.push(Entry { path: item.path(), directory, bytes });
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
pub struct Clipboard { pub source: PathBuf, pub cut: bool }
pub enum Event { Open(PathBuf), Choose(PathBuf), Drag(PathBuf), Close, Clipboard(Clipboard), Operate(crate::file_ops::Operation), CancelOperation }
enum DialogKind { NewFolder, Rename(PathBuf), Trash(PathBuf) }
struct OperationDialog { kind: DialogKind, name: String, pending: bool, focused: bool, error: Option<String> }
impl OperationDialog { fn new(kind: DialogKind, name: String) -> Self { Self { kind, name, pending: false, focused: false, error: None } } }
pub struct Browser {
    path: PathBuf, path_text: String, entries: Vec<Entry>, selected: Option<PathBuf>,
    mode: Mode, filter: String, hidden: bool, history: Vec<PathBuf>, history_at: usize,
    worker: Option<Worker>, generation: u64, pending: Option<(PathBuf, Navigation)>,
    loading: bool, loaded: bool, error: Option<String>, limited: bool, in_trash: bool,
    clipboard: Option<Clipboard>, operation_progress: Option<u64>, dialog: Option<OperationDialog>, menu_open: bool, labels: std::collections::HashMap<PathBuf, String>,
}
pub fn home() -> PathBuf { std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/")) }
fn image_path(path: &Path) -> bool { matches!(path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg") }
fn file_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{bytes} B") }
    else if bytes < 1_048_576 { format!("{:.1} KiB", bytes as f64 / 1024.0) }
    else if bytes < 1_073_741_824 { format!("{:.1} MiB", bytes as f64 / 1_048_576.0) }
    else { format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0) }
}
impl Browser {
    pub fn new(path: PathBuf, mode: Mode) -> Self {
        let path = if path.is_absolute() { path } else { home() };
        Self { path_text: path.to_string_lossy().into_owned(), history: vec![path.clone()], path, entries: Vec::new(), selected: None,
            mode, filter: String::new(), hidden: false, history_at: 0, worker: None, generation: 0, pending: None,
            loading: false, loaded: false, error: None, limited: false, in_trash: false,
            clipboard: None, operation_progress: None, dialog: None, menu_open: false, labels: Default::default() }
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
                match navigation {
                    Navigation::New if path != self.path => {
                        self.history.truncate(self.history_at + 1); self.history.push(path.clone());
                        if self.history.len() > 64 { self.history.remove(0); }
                        self.history_at = self.history.len() - 1;
                    },
                    Navigation::History(index) => self.history_at = index,
                    _ => {},
                }
                self.path = path; self.path_text = self.path.to_string_lossy().into_owned();
                self.entries = listing.entries; self.limited = listing.limited; self.selected = None; self.filter.clear(); self.error = None;
            },
            Err(error) => { self.pending = None; self.error = Some(error); self.path_text = self.path.to_string_lossy().into_owned(); },
        }
    }
    fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.entries.iter().enumerate().filter(|(_, e)| {
            let name = self.labels.get(&e.path).map(std::borrow::Cow::from).unwrap_or_else(|| e.path.file_name().unwrap_or_default().to_string_lossy());
            (self.hidden || !name.starts_with('.')) && name.to_lowercase().contains(&needle)
                && (self.mode == Mode::Browse || e.directory || image_path(&e.path))
        }).map(|(i, _)| i).collect()
    }
    fn can_choose(&self) -> bool {
        self.selected.as_ref().is_some_and(|path| self.entries.iter().any(|e| &e.path == path && !e.directory && (self.mode == Mode::Browse || image_path(path))))
    }
    fn activate(&mut self, ctx: &egui::Context) -> Option<Event> {
        let index = self.entries.iter().position(|e| Some(&e.path) == self.selected.as_ref())?;
        if !self.visible().contains(&index) { return None; }
        let entry = self.entries[index].clone();
        if entry.directory { self.navigate(ctx, entry.path, Navigation::New); None }
        else if self.can_choose() { Some(if self.mode == Mode::ImagePicker { Event::Choose(entry.path) } else { Event::Open(entry.path) }) }
        else { None }
    }
    pub fn captures_keyboard(&self) -> bool { self.dialog.is_some() || self.menu_open }
    pub fn configure_operations(&mut self, clipboard: Option<Clipboard>, progress: Option<u64>) {
        self.clipboard = clipboard; self.operation_progress = progress;
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
        self.selected.as_ref().filter(|path| self.visible().iter().any(|&i| &self.entries[i].path == *path)).cloned()
    }
    fn operations(&mut self, ui: &mut egui::Ui, keyboard: bool, typing: bool) -> Option<Event> {
        if self.mode != Mode::Browse { return None; }
        let busy = self.operation_progress.is_some(); let source = self.selected_visible();
        let mut new_folder = false; let mut rename = false; let mut copy = false; let mut cut = false; let mut paste = false; let mut trash = false; let mut restore = false;
        ui.horizontal_wrapped(|ui| {
            new_folder = ui.add_enabled(!busy && !self.in_trash && !self.loading && !self.menu_open, egui::Button::new("New folder")).clicked();
            paste = ui.add_enabled(!busy && !self.in_trash && self.clipboard.is_some() && !self.loading && !self.menu_open, egui::Button::new("Paste")).clicked();
            let mut menu_drawn = false;
            let menu = ui.menu_button("More", |ui| {
                menu_drawn = true;
                rename = ui.add_enabled(!busy && !self.in_trash && source.is_some(), egui::Button::new("Rename")).clicked();
                copy = ui.add_enabled(!busy && !self.in_trash && source.is_some(), egui::Button::new("Copy")).clicked();
                cut = ui.add_enabled(!busy && !self.in_trash && source.is_some(), egui::Button::new("Cut")).clicked();
                ui.separator();
                if self.in_trash { restore = ui.add_enabled(!busy && source.is_some(), egui::Button::new("Restore")).clicked(); }
                else { trash = ui.add_enabled(!busy && source.is_some(), egui::Button::new("Move to Trash")).clicked(); }
                if rename || copy || cut || restore || trash { ui.close_menu(); }
            });
            // Native menu buttons do not consume Enter; retain ownership for
            // this frame even when an action closes the menu or opens a dialog.
            // egui 0.31 returns inner=None on an ordinary open menu frame;
            // closure execution, rather than inner, is the open-state signal.
            self.menu_open = menu_drawn || menu.response.clicked();
            menu.response.on_hover_text(if self.in_trash { "Restore selected item" } else { "Rename · Copy · Cut · Trash" });
        });
        if keyboard && !typing && !busy && self.dialog.is_none() && !self.menu_open {
            if !self.in_trash {
                copy |= ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::C));
                cut |= ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::X));
                paste |= ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::V));
                new_folder |= ui.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::N));
                rename |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2));
                trash |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete));
            }
        }
        if let Some(progress) = self.operation_progress {
            ui.horizontal_wrapped(|ui| {
                ui.label(if progress == 0 { "Working…".into() } else { format!("Working · {:.1} MiB copied", progress as f64 / 1_048_576.0) });
                if ui.button("Cancel operation").clicked() { self.error = Some("Cancellation requested…".into()); }
            });
            if self.error.as_deref() == Some("Cancellation requested…") { return Some(Event::CancelOperation); }
        } else if let Some(clipboard) = &self.clipboard {
            let verb = if clipboard.cut { "Move" } else { "Copy" };
            ui.add(egui::Label::new(format!("{verb}: {} · open a destination and Paste", clipboard.source.file_name().unwrap_or_default().to_string_lossy())).truncate());
        }
        if busy { return None; }
        if new_folder && !self.in_trash && !self.loading { self.dialog = Some(OperationDialog::new(DialogKind::NewFolder, String::new())); }
        if let Some(source) = source {
            if rename && !self.in_trash { self.dialog = Some(OperationDialog::new(DialogKind::Rename(source.clone()), source.file_name().unwrap_or_default().to_string_lossy().into_owned())); }
            if trash && !self.in_trash { self.dialog = Some(OperationDialog::new(DialogKind::Trash(source.clone()), String::new())); }
            if restore && self.in_trash { return Some(Event::Operate(crate::file_ops::Operation::Restore { source })); }
            if (copy || cut) && !self.in_trash { return Some(Event::Clipboard(Clipboard { source, cut })); }
        }
        if paste && !self.in_trash && !self.loading {
            if let Some(clipboard) = &self.clipboard {
                return Some(Event::Operate(if clipboard.cut { crate::file_ops::Operation::Move { source: clipboard.source.clone(), directory: self.path.clone() } }
                    else { crate::file_ops::Operation::Copy { source: clipboard.source.clone(), directory: self.path.clone() } }));
            }
        }
        None
    }
    fn operation_dialog(&mut self, ctx: &egui::Context) -> Option<Event> {
        let mut dialog = self.dialog.take()?; let mut event = None; let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("native-file-operation")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).clamp(220.0, 440.0));
            let title = match &dialog.kind { DialogKind::NewFolder => "New folder", DialogKind::Rename(_) => "Rename item", DialogKind::Trash(_) => "Move to Trash" };
            ui.label(egui::RichText::new(title).font(ui::display_bold(20.0)));
            ui.add_space(12.0);
            let mut submit = false;
            match &dialog.kind {
                DialogKind::Trash(source) => { ui.label(format!("Move “{}” to Trash?", source.file_name().unwrap_or_default().to_string_lossy())); ui.label("You can restore it from Trash."); },
                _ => {
                    let field = ui.add_enabled(!dialog.pending, egui::TextEdit::singleline(&mut dialog.name).char_limit(255).desired_width(ui.available_width()).hint_text("Name"));
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "File or folder name"));
                    if !dialog.focused { field.request_focus(); dialog.focused = true; }
                    submit = !dialog.pending && (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                },
            }
            if let Some(error) = &dialog.error { ui.colored_label(ui::CAUTION, error); }
            if dialog.pending { ui.label("Working…"); }
            ui.horizontal_wrapped(|ui| {
                let label = match dialog.kind { DialogKind::NewFolder => "Create folder", DialogKind::Rename(_) => "Rename", DialogKind::Trash(_) => "Move to Trash" };
                submit |= ui.add_enabled(!dialog.pending && (matches!(dialog.kind, DialogKind::Trash(_)) || !dialog.name.is_empty()), egui::Button::new(label)).clicked();
                if dialog.pending {
                    if ui.button("Cancel operation").clicked() { event = Some(Event::CancelOperation); }
                } else { cancel = ui.button("Cancel").clicked(); }
            });
            if submit && !dialog.pending {
                let operation = match &dialog.kind {
                    DialogKind::NewFolder => crate::file_ops::Operation::CreateFolder { parent: self.path.clone(), name: dialog.name.clone() },
                    DialogKind::Rename(source) => crate::file_ops::Operation::Rename { source: source.clone(), name: dialog.name.clone() },
                    DialogKind::Trash(source) => crate::file_ops::Operation::Trash { source: source.clone() },
                };
                dialog.pending = true; dialog.error = None; event = Some(Event::Operate(operation));
            }
        });
        if !cancel && (!response.should_close() || dialog.pending) { self.dialog = Some(dialog); }
        event
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Event> { self.show_with_keyboard(ui, true) }
    pub fn show_with_keyboard(&mut self, ui: &mut egui::Ui, active: bool) -> Option<Event> {
        let owns = active && ui.is_enabled();
        let base_enabled = owns && self.dialog.is_none();
        let mut event = ui.add_enabled_ui(base_enabled, |ui| self.show_inner(ui, base_enabled)).inner;
        if owns { event = self.operation_dialog(ui.ctx()).or(event); }
        event
    }
    fn show_inner(&mut self, ui: &mut egui::Ui, keyboard_active: bool) -> Option<Event> {
        if !self.loaded { self.navigate(ui.ctx(), self.path.clone(), Navigation::Refresh); }
        let answer = self.worker.as_ref().and_then(|w| w.answer.lock().unwrap().take());
        if let Some((generation, result)) = answer { self.accept(generation, result); }
        let mut event = None; let mut target = None;
        let controls_active = keyboard_active && !self.menu_open;
        let text_input_active = ui.add_enabled_ui(!self.menu_open, |ui| {
        let folder_name = if self.in_trash { "Trash".into() } else if self.path == home() { "Home".into() }
            else { self.path.file_name().map(|name| name.to_string_lossy()).unwrap_or_else(|| "File system".into()) };
        ui.add(egui::Label::new(egui::RichText::new(folder_name).font(ui::display_bold(22.0)).color(ui::TEXT)).truncate());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.history_at > 0 && !self.loading, egui::Button::new("Back")).clicked() { target = Some((self.history[self.history_at - 1].clone(), Navigation::History(self.history_at - 1))); }
            if ui.add_enabled(self.history_at + 1 < self.history.len() && !self.loading, egui::Button::new("Forward")).clicked() { target = Some((self.history[self.history_at + 1].clone(), Navigation::History(self.history_at + 1))); }
            if ui.add_enabled(self.path.parent().is_some() && !self.loading, egui::Button::new("Up")).clicked() { target = self.path.parent().map(|p| (p.to_path_buf(), Navigation::New)); }
            if ui.add_enabled(!self.loading, egui::Button::new("Refresh")).clicked() { target = Some((self.path.clone(), Navigation::Refresh)); }
            if ui.button(if self.mode == Mode::ImagePicker { "Cancel" } else { "Close Files" }).clicked() { event = Some(Event::Close); }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for (name, path) in [("Home", home()), ("Documents", home().join("Documents")), ("Downloads", home().join("Downloads")), ("Pictures", home().join("Pictures"))] {
                if ui.selectable_label(self.path == path, name).clicked() { target = Some((path, Navigation::New)); }
            }
            if self.mode == Mode::Browse {
                if let Some(trash) = crate::file_ops::trash_files() { if ui.selectable_label(self.in_trash, "Trash").clicked() { target = Some((trash, Navigation::New)); } }
            }
        });
        let path_field = ui.add(egui::TextEdit::singleline(&mut self.path_text).font(ui::mono(12.0)).char_limit(4096).desired_width(ui.available_width()).hint_text("Folder path"));
        path_field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Folder path"));
        if controls_active && (path_field.has_focus() || path_field.lost_focus()) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { target = Some((if self.path_text == self.path.to_string_lossy() { self.path.clone() } else { PathBuf::from(&self.path_text) }, Navigation::New)); }
        let search_field = ui.horizontal(|ui| {
            let clear_width = 48.0 + ui.spacing().item_spacing.x;
            let field = ui.add(egui::TextEdit::singleline(&mut self.filter).char_limit(256).desired_width((ui.available_width() - clear_width).max(48.0)).hint_text("Search this folder"));
            if ui.add_enabled(!self.filter.is_empty(), egui::Button::new("Clear")).clicked() { self.filter.clear(); field.request_focus(); }
            field
        }).inner;
        search_field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search this folder"));
        let text_input_active = path_field.has_focus() || path_field.lost_focus() || search_field.has_focus() || search_field.lost_focus();
        if controls_active && (search_field.has_focus() || search_field.lost_focus()) { ui.input_mut(|i| { i.consume_key(Modifiers::NONE, Key::Enter); }); }
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.hidden, "Hidden files");
            if self.loading { ui.label("Loading folder…"); }
            else { ui.label(egui::RichText::new(format!("{} items{}", self.visible().len(), if self.limited { " · limited listing" } else { "" })).font(ui::mono(11.0)).color(ui::SECONDARY)); }
        });
        if let Some(error) = &self.error { ui.colored_label(ui::CAUTION, error); }
        text_input_active
        }).inner;
        if let Some((path, navigation)) = target { self.navigate(ui.ctx(), path, navigation); }
        event = self.operations(ui, keyboard_active, text_input_active).or(event);
        let visible = self.visible();
        let mut scroll_selection = None;
        let mut list_enter = false;
        if keyboard_active && !self.loading && !text_input_active && !self.captures_keyboard() {
            let step = if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) { 1 } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { -1 } else { 0 };
            if step != 0 && !visible.is_empty() {
                let at = visible.iter().position(|&i| Some(&self.entries[i].path) == self.selected.as_ref());
                let next = at.map_or(0, |i| (i as isize + step).clamp(0, visible.len() as isize - 1) as usize);
                self.selected = Some(self.entries[visible[next]].path.clone());
                scroll_selection = Some(next);
            }
            if event.is_none() { list_enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)); }
        }
        // Action stays above the scroll region, reachable on short/narrow windows.
        let selected_visible = self.selected.as_ref().is_some_and(|path| visible.iter().any(|&i| &self.entries[i].path == path));
        let selected_directory = self.selected.as_ref().is_some_and(|path| self.entries.iter().any(|e| &e.path == path && e.directory));
        let label = if selected_directory { "Open folder" } else if self.mode == Mode::ImagePicker { "Choose image" } else { "Open file" };
        if ui.add_enabled(!self.loading && !self.captures_keyboard() && selected_visible && (selected_directory || self.can_choose()), egui::Button::new(label)).clicked() { event = self.activate(ui.ctx()).or(event); }
        if visible.is_empty() && !self.loading {
            ui.label(if !self.filter.is_empty() { "No matching files" } else if self.mode == Mode::ImagePicker { "No PNG or JPEG images here. Open another folder." } else if self.in_trash { "Trash is empty" } else { "This folder is empty" });
            if !self.filter.is_empty() && ui.button("Clear search").clicked() { self.filter.clear(); }
        }
        let height = ui.available_height().max(80.0);
        let mut activate = None;
        let mut list = egui::ScrollArea::vertical().id_salt("native-files-list").max_height(height).auto_shrink([false, false]);
        if let Some(row) = scroll_selection { list = list.vertical_scroll_offset(row as f32 * (32.0 + ui.spacing().item_spacing.y)); }
        list.show_rows(ui, 32.0, visible.len(), |ui, rows| {
            ui.add_enabled_ui(!self.loading && !self.captures_keyboard(), |ui| {
                for row in rows {
                    let entry = &self.entries[visible[row]];
                    let name = self.labels.get(&entry.path).map(std::borrow::Cow::from).unwrap_or_else(|| entry.path.file_name().unwrap_or_default().to_string_lossy());
                    let metadata = if entry.directory { "Folder".into() } else { file_size(entry.bytes) };
                    let show_metadata = ui.available_width() >= 480.0;
                    let response = ui.horizontal(|ui| {
                        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 30.0), egui::Sense::hover());
                        let ink = if entry.directory { ui::TEXT } else { ui::SECONDARY };
                        if entry.directory {
                            let rect = egui::Rect::from_center_size(icon_rect.center(), egui::vec2(16.0, 11.0));
                            ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ink), egui::StrokeKind::Inside);
                            ui.painter().line_segment([rect.left_top(), rect.left_top() + egui::vec2(7.0, -3.0)], egui::Stroke::new(1.0, ink));
                        } else {
                            ui::paint_icon(ui.painter(), icon_rect.shrink2(egui::vec2(2.0, 7.0)), ui::Icon::Copy, ink, false);
                        }
                        let metadata_width = if show_metadata { 88.0 + ui.spacing().item_spacing.x } else { 0.0 };
                        let response = ui.add_sized(egui::vec2((ui.available_width() - metadata_width).max(32.0), 30.0), egui::Button::new(egui::RichText::new(name.as_ref()).font(ui::display(13.0))).selected(self.selected.as_ref() == Some(&entry.path)).sense(egui::Sense::click_and_drag()).truncate());
                        if show_metadata { ui.add_sized(egui::vec2(88.0, 30.0), egui::Label::new(egui::RichText::new(&metadata).font(ui::mono(11.0)).color(ui::SECONDARY)).truncate()); }
                        response
                    }).inner;
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !self.loading, format!("{}, {metadata}, {}", name, entry.path.display())));
                    response.clone().on_hover_text(format!("{}{}", entry.path.display(), if entry.directory { String::new() } else { format!("\n{} bytes", entry.bytes) }));
                    if response.clicked() { self.selected = Some(entry.path.clone()); }
                    if response.drag_started() && self.mode == Mode::Browse && !self.in_trash && self.operation_progress.is_none() && event.is_none() {
                        self.selected = Some(entry.path.clone());
                        event = Some(Event::Drag(entry.path.clone()));
                    }
                    if response.double_clicked() || (list_enter && response.has_focus()) { activate = Some(entry.path.clone()); }
                    if self.selected.as_ref() == Some(&entry.path) && response.has_focus() { response.scroll_to_me(Some(egui::Align::Center)); }
                }
            });
        });
        if let Some(path) = activate { self.selected = Some(path); event = self.activate(ui.ctx()).or(event); }
        else if list_enter { event = self.activate(ui.ctx()).or(event); }
        event
    }
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
        browser.entries = vec![Entry { path: "/tmp/hidden-selection".into(), directory: true, bytes: 0 }];
        browser.selected = Some("/tmp/hidden-selection".into()); browser.filter = "different".into();
        assert!(browser.activate(&egui::Context::default()).is_none());
        assert!(!browser.loading, "Enter must not navigate into a filtered-out selection");
    }
    #[test]
    fn native_files_cannot_pick_a_folder_or_unsupported_image() {
        let mut browser = Browser::new("/tmp".into(), Mode::ImagePicker);
        browser.entries = vec![Entry { path: "/tmp/folder".into(), directory: true, bytes: 0 }, Entry { path: "/tmp/a.txt".into(), directory: false, bytes: 10 }, Entry { path: "/tmp/a.PNG".into(), directory: false, bytes: 10 }];
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
    fn dragging_an_entry_emits_one_desktop_drag_without_opening_it() {
        for mode in [Mode::Browse, Mode::ImagePicker] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let mut browser = Browser::new("/tmp".into(), mode); browser.loaded = true;
            browser.entries = vec![Entry { path: "/tmp/drag me.png".into(), directory: false, bytes: 10 }];
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
            egui::CentralPanel::default().show(ctx, |ui| { action = browser.show(ui); });
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
        browser.entries = vec![Entry { path: "/tmp/雨 #.png".into(), directory: false, bytes: 10 }];
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "雨 #.png")).is_none());
        assert_eq!(browser.selected, Some(PathBuf::from("/tmp/雨 #.png")));
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        match click(&mut browser, &ctx, button(&output, "Choose image")) { Some(Event::Choose(path)) => assert_eq!(path, PathBuf::from("/tmp/雨 #.png")), _ => panic!("image picker did not return selected path") }
    }
    #[test]
    fn native_files_open_menu_owns_arrows_and_enter_before_the_list() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/first.txt".into(), directory: false, bytes: 2 }, Entry { path: "/tmp/second.txt".into(), directory: false, bytes: 2 }];
        browser.selected = Some("/tmp/first.txt".into());
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "More")).is_none());
        for key in [Key::ArrowDown, Key::Enter] {
            let event = render(&mut browser, &ctx, vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }], 700.0).1;
            assert!(!matches!(event, Some(Event::Open(_))), "menu key opened a file behind the menu");
            assert_eq!(browser.selected, Some(PathBuf::from("/tmp/first.txt")), "menu navigation changed the operation target");
        }
    }

    #[test]
    fn native_files_more_menu_keeps_operations_discoverable_and_picker_read_only() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/notes.txt".into(), directory: false, bytes: 2048 }];
        browser.selected = Some("/tmp/notes.txt".into());
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "More")).is_none());
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "Rename")).is_none());
        assert!(matches!(browser.dialog.as_ref().map(|dialog| &dialog.kind), Some(DialogKind::Rename(path)) if path == Path::new("/tmp/notes.txt")));

        let mut picker = Browser::new("/tmp".into(), Mode::ImagePicker); picker.loaded = true;
        let picker_ctx = egui::Context::default(); crate::ui::install_fonts(&picker_ctx, None, None);
        let output = render(&mut picker, &picker_ctx, Vec::new(), 320.0).0;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape { assert!(!matches!(text.galley.job.text.as_str(), "New folder" | "Paste" | "More" | "Move to Trash")); }
        }
    }
    #[test]
    fn native_files_does_not_consume_the_scratchpads_enter() {
        let ctx = egui::Context::default();
        crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/selected.png".into(), directory: false, bytes: 10 }];
        browser.selected = Some("/tmp/selected.png".into());
        let mut action = None;
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 600.0))), events: vec![egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE }], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = browser.show_with_keyboard(ui, false);
                assert!(ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)), "the active scratchpad still needs the event");
            });
        });
        assert!(action.is_none());
    }
    #[test]
    fn native_files_path_enter_navigates_without_opening_the_selected_file() {
        let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
        let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
        browser.entries = vec![Entry { path: "/tmp/selected.png".into(), directory: false, bytes: 10 }];
        browser.selected = Some("/tmp/selected.png".into());
        let output = render(&mut browser, &ctx, Vec::new(), 700.0).0;
        assert!(click(&mut browser, &ctx, button(&output, "/tmp")).is_none());
        browser.path_text = "/tmp/typed-folder".into();
        let action = render(&mut browser, &ctx, vec![egui::Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE }], 700.0).1;
        assert!(action.is_none(), "Enter in the path field must not open an unrelated selection");
        assert_eq!(browser.pending.as_ref().unwrap().0, PathBuf::from("/tmp/typed-folder"));
    }
    #[test]
    fn native_files_keyboard_selects_and_names_do_not_overflow_narrow_views() {
        for width in [320.0, 900.0] {
            let ctx = egui::Context::default(); crate::ui::install_fonts(&ctx, None, None);
            let mut browser = Browser::new("/tmp".into(), Mode::Browse); browser.loaded = true;
            browser.entries = vec![Entry { path: PathBuf::from(format!("/tmp/{}.png", "long name 雨 ".repeat(15))), directory: false, bytes: 10 }];
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
