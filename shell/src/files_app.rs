//! Files as an application of its own: `des-ui --files [FOLDER|FILE|URI...]` opens a
//! window with the shell's Files view, its tabs, split view and file work, so other
//! applications can show a folder ("Show in folder") and it has a place in the dock.
use crate::file_ops::Operation;
use crate::files::{self, Event};
use crate::theme::Theme;
use crate::transfers;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Where to start: the first argument that names something that exists, as a folder
/// and, when it named a file, that file to select. Home otherwise.
pub fn start_at(arguments: &[String]) -> (PathBuf, Option<PathBuf>) {
    for argument in arguments {
        let path = match argument.strip_prefix("file://") {
            Some(rest) => PathBuf::from(percent_decode(rest.trim_start_matches("localhost"))),
            None => PathBuf::from(argument),
        };
        let path = if path.is_relative() { std::env::current_dir().map(|dir| dir.join(&path)).unwrap_or(path) } else { path };
        if path.is_dir() { return (path, None); }
        if path.exists() {
            if let Some(parent) = path.parent() { return (parent.to_path_buf(), Some(path)); }
        }
    }
    (std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")), None)
}

/// `%20` and the like back into bytes; anything malformed is kept as written.
fn percent_decode(text: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    std::ffi::OsString::from_vec(out)
}

pub struct FilesApp {
    window: files::Window,
    theme: Theme,
    clipboard: Option<files::Clipboard>,
    work: transfers::Transfers,
    job: Option<files::Task>,
    batch: u64,
    openers: Vec<Receiver<Result<(), String>>>,
    /// A word in the corner: what went wrong opening a file, or a terminal asked for.
    message: Option<(String, Instant)>,
    screenshot: Option<PathBuf>,
    frames: usize,
}

impl FilesApp {
    pub fn new(folder: PathBuf, select: Option<PathBuf>, theme: Theme, screenshot: Option<PathBuf>) -> Self {
        let mut window = files::Window::new(folder);
        if let Some(entry) = select { window.reveal(entry); }
        FilesApp { window, theme, clipboard: None, work: Default::default(), job: None, batch: 0, openers: Vec::new(), message: None, screenshot, frames: 0 }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.job.as_ref().and_then(files::Task::poll) {
            let task = self.job.take().unwrap();
            self.work.finished(self.batch, &task.operation, &result);
            match result {
                Ok(completed) => {
                    // What was moved, renamed or binned is no longer where the clipboard says.
                    let source = match &task.operation {
                        Operation::Rename { source, .. } | Operation::Copy { source, .. } | Operation::Move { source, .. }
                        | Operation::Trash { source } | Operation::Restore { source } => Some(source.clone()),
                        _ => None,
                    };
                    if let (Some(source), Some(clip), false) = (source, self.clipboard.as_mut(), completed.skipped) {
                        if let Some(at) = clip.sources.iter().position(|s| *s == source) {
                            match task.operation {
                                Operation::Rename { .. } | Operation::Restore { .. } => clip.sources[at] = completed.path.clone(),
                                Operation::Move { .. } | Operation::Trash { .. } => { clip.sources.remove(at); }
                                _ => {}
                            }
                            if clip.sources.is_empty() { self.clipboard = None; }
                        }
                    }
                    self.window.operation_result(ctx, Ok(&completed.message));
                }
                Err(error) => self.window.operation_result(ctx, Err(&error)),
            }
        }
        while self.job.is_none() {
            let transfers::Next::Start(batch, operation) = self.work.next() else { break };
            match files::Task::start(operation.clone(), ctx.clone()) {
                Ok(task) => { self.job = Some(task); self.batch = batch; }
                Err(error) => {
                    self.window.operation_result(ctx, Err(&error));
                    self.work.finished(batch, &operation, &Err(error));
                }
            }
        }
        if self.job.is_some() { ctx.request_repaint_after(Duration::from_millis(100)); }
        self.openers.retain(|rx| match rx.try_recv() {
            Ok(Err(error)) => { self.message = Some((error, Instant::now())); false }
            Ok(Ok(())) => false,
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
    }

    fn handle(&mut self, ctx: &egui::Context, event: Event) {
        match event {
            Event::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Event::Clipboard(clipboard) => self.clipboard = Some(clipboard),
            Event::Operate(operations) => { self.work.enqueue(operations); }
            Event::Undo => { let _ = self.work.undo(); }
            Event::Resolve(choice, all) => { self.work.resolve(choice, all); }
            Event::PauseTransfers(paused) => { self.work.set_paused(paused); }
            Event::ClearTransfers => { self.work.clear(); }
            Event::CancelOperation => { if let Some(task) = &self.job { task.cancel(); } }
            Event::Open(path) => {
                if self.openers.len() < 32 { self.openers.push(crate::apps::open_document(path, ctx.clone())); }
            }
            Event::Terminal(folder) => self.terminal(ctx, &folder),
            Event::SendTo(..) => self.message = Some(("Sending to a phone is in the desktop's Files.".into(), Instant::now())),
            // Dragging out and tabs of choosers belong to the desktop.
            _ => {}
        }
    }

    /// A terminal belongs to the desktop: the way there goes on the clipboard.
    fn terminal(&mut self, ctx: &egui::Context, folder: &Path) {
        let quoted = format!("'{}'", folder.display().to_string().replace('\'', "'\\''"));
        ctx.copy_text(format!("cd {quoted}"));
        self.message = Some(("Copied “cd” to this folder: paste it in a terminal tab.".into(), Instant::now()));
    }
}

impl eframe::App for FilesApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.pump(ctx);
        let progress = self.job.as_ref().map(|task| task.progress.load(std::sync::atomic::Ordering::Relaxed));
        self.window.configure_operations(self.clipboard.clone(), self.work.view(progress.unwrap_or(0)));
        let frame = egui::Frame::NONE.fill(crate::ui::GROUND).inner_margin(egui::Margin::symmetric(18, 14));
        let event = egui::CentralPanel::default().frame(frame).show(ctx, |ui| self.window.show_with_keyboard(ui, &self.theme, true)).inner;
        if let Some(event) = event { self.handle(ctx, event); }
        if let Some((text, at)) = &self.message {
            if at.elapsed() < Duration::from_secs(6) {
                egui::Area::new(egui::Id::new("files-app-message")).anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -14.0]).show(ctx, |ui| {
                    egui::Frame::NONE.fill(crate::ui::TRACK).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.label(egui::RichText::new(text).font(crate::ui::mono(12.0)).color(crate::ui::TEXT));
                    });
                });
                ctx.request_repaint_after(Duration::from_millis(500));
            } else {
                self.message = None;
            }
        }
        // `--screenshot`: once the folder has been read, one frame to a PPM, then quit.
        if let Some(path) = &self.screenshot {
            self.frames += 1;
            if self.frames == 40 { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let mut ppm = format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
                    ppm.extend(image.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b()]));
                    let _ = std::fs::write(path, ppm);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_at_a_folder_a_file_or_a_uri() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("my photos");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("a b.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(start_at(&[folder.display().to_string()]), (folder.clone(), None));
        assert_eq!(start_at(&[file.display().to_string()]), (folder.clone(), Some(file.clone())));
        let uri = format!("file://{}", folder.display().to_string().replace(' ', "%20"));
        assert_eq!(start_at(&[uri]), (folder.clone(), None));
        // What does not exist is passed over for the next.
        assert_eq!(start_at(&["/no/such/place".into(), folder.display().to_string()]), (folder, None));
        assert_eq!(start_at(&[]).1, None);
    }
}
