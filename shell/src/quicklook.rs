//! Quick-look: a look at a file from the file panel without opening it. Pictures,
//! text, the first page of a PDF, what an archive holds, how big a folder is, or at
//! least what a file is. Everything is read on a worker thread within bounds, and
//! nothing is ever run but the fixed helpers named here, with the file as an argument.

use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, ColorImage, Order, Rect, ScrollArea, Stroke, StrokeKind, TextureHandle, vec2};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

/// Larger files are described, never read.
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
pub const MAX_LINES: usize = 2_000;
pub const MAX_ENTRIES: usize = 500;
/// A folder's size is added up over at most this many entries.
pub const MAX_WALK: usize = 20_000;
const HELPER_TIME: Duration = Duration::from_secs(2);
/// Enough of a text file for its first lines, however long they are.
const TEXT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Image,
    Text,
    Pdf,
    Zip,
    Tar,
    Folder,
    Other,
}

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "log", "csv", "tsv", "json", "toml", "yaml", "yml", "ini", "cfg", "conf", "xml",
    "html", "htm", "css", "js", "mjs", "ts", "tsx", "jsx", "rs", "c", "h", "cc", "cpp", "hpp", "py", "rb", "go",
    "java", "kt", "swift", "sh", "bash", "zsh", "fish", "lua", "pl", "php", "sql", "tex", "diff", "patch", "desktop",
    "service", "lock", "gitignore", "env", "nix", "zig", "hs", "ml", "ex", "exs", "erl", "dart", "vue", "svelte",
];

/// What a file is, by its name first and then by its first bytes.
pub fn kind_of(path: &Path, head: &[u8], is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_string()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" => return Kind::Image,
        "pdf" => return Kind::Pdf,
        "zip" | "jar" | "apk" | "epub" | "docx" | "xlsx" | "odt" => return Kind::Zip,
        "tar" | "tgz" | "txz" | "tbz2" => return Kind::Tar,
        "gz" | "xz" | "bz2" | "zst" if name.contains(".tar.") => return Kind::Tar,
        ext if TEXT_EXTENSIONS.contains(&ext) => return Kind::Text,
        _ => {}
    }
    if head.starts_with(b"\x89PNG") || head.starts_with(&[0xFF, 0xD8, 0xFF]) || head.starts_with(b"GIF8") {
        Kind::Image
    } else if head.starts_with(b"%PDF") {
        Kind::Pdf
    } else if head.starts_with(b"PK\x03\x04") {
        Kind::Zip
    } else if head.get(257..262) == Some(b"ustar") {
        Kind::Tar
    } else if !head.is_empty() && looks_like_text(head) {
        Kind::Text
    } else {
        Kind::Other
    }
}

/// No NUL bytes and mostly printable: what `file` would call text, near enough.
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    let text = String::from_utf8_lossy(head);
    let odd = text.chars().filter(|c| *c == '\u{FFFD}' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c'))).count();
    odd * 20 <= text.chars().count().max(1)
}

/// The first `max` lines of `bytes`, read as UTF-8 come what may, and whether more
/// were left out.
pub fn text_lines(bytes: &[u8], max: usize) -> (Vec<String>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<String> = text.lines().take(max + 1).map(|line| line.replace('\t', "    ")).collect();
    let more = lines.len() > max;
    lines.truncate(max);
    (lines, more)
}

/// An archive's listing, one entry a line, at most `max` of them.
pub fn listing(output: &str, max: usize) -> (Vec<String>, bool) {
    let mut entries: Vec<String> =
        output.lines().map(str::trim_end).filter(|line| !line.is_empty()).take(max + 1).map(str::to_string).collect();
    let more = entries.len() > max;
    entries.truncate(max);
    (entries, more)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FolderCount {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    /// The walk stopped at its bound; the numbers are at least these.
    pub partial: bool,
}

/// Counts what a folder holds, without following links, over at most `max` entries.
pub fn count_folder(root: &Path, max: usize) -> FolderCount {
    let mut count = FolderCount::default();
    let mut pending = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > max {
                count.partial = true;
                return count;
            }
            let Ok(meta) = entry.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                count.folders += 1;
                pending.push(entry.path());
            } else {
                count.files += 1;
                count.bytes += meta.len();
            }
        }
    }
    count
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// What the preview shows, once the worker has read it.
#[derive(Debug, Clone)]
pub enum Preview {
    Loading,
    Image { picture: ColorImage, width: u32, height: u32, note: Option<&'static str> },
    Text { lines: Vec<String>, more: bool },
    Listing { entries: Vec<String>, more: bool },
    Folder(FolderCount),
    Info { mime: String, size: u64, modified: Option<SystemTime> },
    Note(String),
}

fn program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|dir| Path::new(dir).join(name)).find(|path| path.is_file())
}

/// The file's type as `file` names it, else nothing.
fn mime_of(path: &Path) -> String {
    program("file")
        .and_then(|file| crate::sensors::run_bounded(&file, &["--brief", "--mime-type", "--", &path.to_string_lossy()], HELPER_TIME))
        .map(|out| out.trim().to_string())
        .filter(|mime| !mime.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn decode_image(bytes: &[u8]) -> Option<(ColorImage, u32, u32)> {
    let image = image::load_from_memory(bytes).ok()?;
    let (width, height) = (image.width(), image.height());
    // A texture larger than this gains nothing on any display.
    let image = if width > 2048 || height > 2048 { image.thumbnail(2048, 2048) } else { image };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some((ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), width, height))
}

fn first_pdf_page(path: &Path) -> Preview {
    let Some(pdftoppm) = program("pdftoppm") else {
        return Preview::Note("A PDF. Install poppler-utils to see its first page here.".into());
    };
    let Ok(dir) = tempfile::tempdir() else { return Preview::Note("No room to draw the page.".into()) };
    let out = dir.path().join("page");
    let file = path.to_string_lossy().into_owned();
    let out_arg = out.to_string_lossy().into_owned();
    crate::sensors::run_bounded(&pdftoppm, &["-png", "-r", "72", "-f", "1", "-l", "1", "-singlefile", "--", &file, &out_arg], HELPER_TIME);
    match std::fs::read(dir.path().join("page.png")).ok().and_then(|bytes| decode_image(&bytes)) {
        Some((picture, width, height)) => Preview::Image { picture, width, height, note: Some("FIRST PAGE") },
        None => Preview::Note("The first page could not be drawn.".into()),
    }
}

fn archive(kind: Kind, path: &Path) -> Preview {
    let file = path.to_string_lossy().into_owned();
    let output = match kind {
        Kind::Zip => program("unzip").and_then(|unzip| crate::sensors::run_bounded(&unzip, &["-Z1", "--", &file], HELPER_TIME)),
        _ => program("tar").and_then(|tar| crate::sensors::run_bounded(&tar, &["-tf", &file], HELPER_TIME)),
    };
    match output {
        Some(output) => {
            let (entries, more) = listing(&output, MAX_ENTRIES);
            Preview::Listing { entries, more }
        }
        None => Preview::Note(if kind == Kind::Zip { "Install unzip to list what it holds.".into() } else { "Its contents could not be listed.".into() }),
    }
}

/// Reads one preview. Runs on the worker; never on the interface thread.
pub fn read_preview(path: &Path) -> Preview {
    let Ok(meta) = std::fs::metadata(path) else { return Preview::Note("It can no longer be read.".into()) };
    if meta.is_dir() {
        return Preview::Folder(count_folder(path, MAX_WALK));
    }
    let mut head = [0u8; 512];
    let read = std::fs::File::open(path).and_then(|mut file| file.read(&mut head)).unwrap_or(0);
    let kind = kind_of(path, &head[..read], false);
    let info = || Preview::Info { mime: mime_of(path), size: meta.len(), modified: meta.modified().ok() };
    if meta.len() > MAX_FILE && kind != Kind::Zip && kind != Kind::Tar {
        return info();
    }
    match kind {
        Kind::Image => match std::fs::read(path).ok().and_then(|bytes| decode_image(&bytes)) {
            Some((picture, width, height)) => Preview::Image { picture, width, height, note: None },
            None => info(),
        },
        Kind::Text => {
            let mut bytes = Vec::new();
            let _ = std::fs::File::open(path).and_then(|file| file.take(TEXT_BYTES).read_to_end(&mut bytes));
            let (lines, more) = text_lines(&bytes, MAX_LINES);
            Preview::Text { lines, more: more || meta.len() > TEXT_BYTES }
        }
        Kind::Pdf => first_pdf_page(path),
        Kind::Zip | Kind::Tar => archive(kind, path),
        Kind::Folder | Kind::Other => info(),
    }
}

/// The preview over the workspace, and the worker that reads it.
#[derive(Default)]
pub struct QuickLook {
    path: Option<PathBuf>,
    preview: Option<Preview>,
    texture: Option<TextureHandle>,
    pending: Option<(Receiver<Preview>, Instant)>,
    /// Where it is in the folder, as `n / of`.
    place: (usize, usize),
}

impl QuickLook {
    pub fn is_open(&self) -> bool {
        self.path.is_some()
    }

    #[cfg(test)]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn close(&mut self) {
        *self = QuickLook::default();
    }

    /// Shows `path`, reading it afresh on the worker; an older answer still on its way is
    /// dropped with its channel.
    pub fn open(&mut self, path: PathBuf, place: (usize, usize), ctx: &egui::Context) {
        let (send, receive) = channel();
        let (target, wake) = (path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = send.send(read_preview(&target));
            wake.request_repaint();
        });
        self.path = Some(path);
        self.place = place;
        self.preview = Some(Preview::Loading);
        self.texture = None;
        self.pending = Some((receive, Instant::now()));
    }

    fn pump(&mut self, ctx: &egui::Context) {
        let Some((receive, _)) = &self.pending else { return };
        if let Ok(preview) = receive.try_recv() {
            if let Preview::Image { picture, .. } = &preview {
                self.texture = Some(ctx.load_texture("quicklook", picture.clone(), egui::TextureOptions::LINEAR));
            }
            self.preview = Some(preview);
            self.pending = None;
        }
    }

    /// Draws it over `area`, above the frame.
    pub fn show(&mut self, ctx: &egui::Context, area: Rect, theme: &Theme) {
        if !self.is_open() {
            return;
        }
        self.pump(ctx);
        let rect = area.shrink(24.0);
        if rect.width() < 120.0 || rect.height() < 80.0 {
            return;
        }
        let name = self.path.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let slow = self.pending.as_ref().is_some_and(|(_, since)| since.elapsed() > HELPER_TIME);
        egui::Area::new(egui::Id::new("quicklook")).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, ui::PANEL);
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme.main), StrokeKind::Inside);
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
            painter.text(header.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, &name, ui::display(15.0), ui::TEXT);
            let (n, of) = self.place;
            let mut right = format!("{} / {}   SPACE OR ESC CLOSES · ← → NEXT", n + 1, of.max(1));
            if let Some(Preview::Image { width, height, note, .. }) = &self.preview {
                right = format!("{}{width} × {height}   {right}", note.map(|note| format!("{note} · ")).unwrap_or_default());
            }
            painter.text(header.right_center() - vec2(16.0, 0.0), Align2::RIGHT_CENTER, right, ui::mono(11.0), ui::LABEL);
            painter.hline(rect.x_range(), header.bottom(), Stroke::new(1.0, ui::LINE));
            let body = Rect::from_min_max(header.left_bottom() + vec2(16.0, 12.0), rect.max - vec2(16.0, 12.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
            body_view(&mut child, self.preview.as_ref(), self.texture.as_ref(), slow, theme);
        });
    }
}

fn row(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new(egui::RichText::new(name).font(ui::mono(12.0)).color(ui::LABEL)));
        ui.label(egui::RichText::new(value).font(ui::mono(12.0)).color(ui::TEXT));
    });
}

fn body_view(ui: &mut egui::Ui, preview: Option<&Preview>, texture: Option<&TextureHandle>, slow: bool, theme: &Theme) {
    let mono = |text: &str, color| egui::RichText::new(text).font(ui::mono(12.0)).color(color);
    match preview {
        None | Some(Preview::Loading) => {
            ui.label(mono(if slow { "Still reading…" } else { "Reading…" }, ui::LABEL));
        }
        Some(Preview::Note(note)) => {
            ui.label(mono(note, ui::SECONDARY));
        }
        Some(Preview::Image { .. }) => {
            if let Some(texture) = texture {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(ui.available_height() / size.y).min(1.0);
                ui.centered_and_justified(|ui| ui.image((texture.id(), size * scale)));
            }
        }
        Some(Preview::Text { lines, more }) => {
            ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                for line in lines {
                    ui.label(mono(line, ui::TEXT));
                }
                if *more {
                    ui.label(mono(&format!("… only the first {MAX_LINES} lines are shown"), ui::LABEL));
                }
            });
        }
        Some(Preview::Listing { entries, more }) => {
            ui.label(mono(&format!("{} ENTRIES{}", entries.len(), if *more { "+" } else { "" }), theme.main));
            ui.add_space(6.0);
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for entry in entries {
                    ui.label(mono(entry, ui::TEXT));
                }
            });
        }
        Some(Preview::Folder(count)) => {
            let at_least = if count.partial { "at least " } else { "" };
            row(ui, "FOLDERS", &format!("{at_least}{}", count.folders));
            row(ui, "FILES", &format!("{at_least}{}", count.files));
            row(ui, "SIZE", &format!("{at_least}{}", human_size(count.bytes)));
        }
        Some(Preview::Info { mime, size, modified }) => {
            row(ui, "TYPE", mime);
            row(ui, "SIZE", &human_size(*size));
            if let Some(modified) = modified {
                let when: chrono::DateTime<chrono::Local> = (*modified).into();
                row(ui, "MODIFIED", &when.format("%Y-%m-%d %H:%M").to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name_then_by_first_bytes() {
        let p = Path::new;
        assert_eq!(kind_of(p("a.PNG"), b"", false), Kind::Image);
        assert_eq!(kind_of(p("notes.rs"), b"", false), Kind::Text);
        assert_eq!(kind_of(p("x.tar.gz"), b"", false), Kind::Tar);
        assert_eq!(kind_of(p("x.gz"), b"\x1f\x8b", false), Kind::Other);
        assert_eq!(kind_of(p("doc.pdf"), b"", false), Kind::Pdf);
        assert_eq!(kind_of(p("book.epub"), b"", false), Kind::Zip);
        assert_eq!(kind_of(p("noext"), b"\x89PNG\r\n", false), Kind::Image);
        assert_eq!(kind_of(p("noext"), b"%PDF-1.7", false), Kind::Pdf);
        assert_eq!(kind_of(p("noext"), b"PK\x03\x04rest", false), Kind::Zip);
        let mut tar = vec![0u8; 300];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(kind_of(p("noext"), &tar, false), Kind::Tar);
        assert_eq!(kind_of(p("README"), b"plain words\nand more\n", false), Kind::Text);
        assert_eq!(kind_of(p("blob"), b"\x00\x01\x02binary", false), Kind::Other);
        assert_eq!(kind_of(p("anything"), b"", true), Kind::Folder);
    }

    #[test]
    fn text_is_cut_at_its_line_bound() {
        let bytes = (0..10).map(|n| format!("line {n}\n")).collect::<String>();
        let (lines, more) = text_lines(bytes.as_bytes(), 4);
        assert_eq!(lines, vec!["line 0", "line 1", "line 2", "line 3"]);
        assert!(more);
        let (lines, more) = text_lines(b"a\tb\n\xffc", 10);
        assert_eq!(lines, vec!["a    b", "\u{FFFD}c"]);
        assert!(!more);
    }

    #[test]
    fn archive_listings_are_bounded() {
        let (entries, more) = listing("a/\na/b.txt\n\nc\n", 10);
        assert_eq!(entries, vec!["a/", "a/b.txt", "c"]);
        assert!(!more);
        let long = (0..20).map(|n| format!("f{n}\n")).collect::<String>();
        let (entries, more) = listing(&long, 5);
        assert_eq!(entries.len(), 5);
        assert!(more);
    }

    #[test]
    fn folder_counts_stop_at_their_bound_and_skip_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a"), b"12345").unwrap();
        std::fs::write(dir.path().join("b"), b"123").unwrap();
        std::os::unix::fs::symlink("/", dir.path().join("root")).unwrap();
        // The link counts as itself, a small file; what it points at is not walked.
        let link = dir.path().join("root").symlink_metadata().unwrap().len();
        let count = count_folder(dir.path(), 100);
        assert_eq!((count.folders, count.files, count.bytes, count.partial), (1, 3, 8 + link, false));
        assert!(count_folder(dir.path(), 2).partial);
    }

    #[test]
    fn previews_of_real_files() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello\nworld\n").unwrap();
        assert!(matches!(read_preview(&text), Preview::Text { lines, more: false } if lines == vec!["hello", "world"]));
        assert!(matches!(read_preview(dir.path()), Preview::Folder(FolderCount { files: 1, .. })));
        assert!(matches!(read_preview(&dir.path().join("gone")), Preview::Note(_)));
        let mut png = Vec::new();
        image::RgbaImage::new(3, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let picture = dir.path().join("dot.png");
        std::fs::write(&picture, png).unwrap();
        assert!(matches!(read_preview(&picture), Preview::Image { width: 3, height: 2, .. }));
    }

    #[test]
    fn sizes_read_like_a_person_would_say_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }
}
