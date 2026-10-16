//! Installed application icons. Filesystem work and image decoding stay off the UI thread.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};
use eframe::egui::{self, Color32, ColorImage, Rect, Stroke, TextureHandle, Ui, vec2};

enum Cached {
    Pending,
    Missing,
    Ready(TextureHandle),
}
struct Request {
    name: String,
    context: egui::Context,
}
pub struct IconCache {
    entries: HashMap<String, Cached>,
    requests: SyncSender<Request>,
    results: Receiver<(String, Option<ColorImage>)>,
}
impl Default for IconCache {
    fn default() -> Self { Self::new() }
}
impl IconCache {
    pub fn new() -> Self {
        let (requests, jobs) = mpsc::sync_channel::<Request>(32);
        let (results, completed) = mpsc::channel();
        std::thread::spawn(move || {
            let roots = icon_roots();
            for request in jobs {
                let image = resolve(&request.name, &roots).and_then(|path| decode(&path));
                if results.send((request.name, image)).is_err() { break; }
                request.context.request_repaint();
            }
        });
        Self { entries: HashMap::new(), requests, results: completed }
    }
    /// Retry cached misses after applications or themes change.
    pub fn clear(&mut self) {
        self.entries.retain(|_, entry| matches!(entry, Cached::Pending));
    }
    pub fn paint(&mut self, ui: &Ui, rect: Rect, icon: Option<&str>) {
        while let Ok((name, image)) = self.results.try_recv() {
            let entry = image.map(|image| Cached::Ready(ui.ctx().load_texture(
                format!("app-icon:{name}"), image, egui::TextureOptions::LINEAR,
            ))).unwrap_or(Cached::Missing);
            self.entries.insert(name, entry);
        }
        if let Some(name) = icon.filter(|name| !name.is_empty()) {
            if !self.entries.contains_key(name) && self.entries.len() < 1024 {
                if self.requests.try_send(Request { name: name.to_owned(), context: ui.ctx().clone() }).is_ok() {
                    self.entries.insert(name.to_owned(), Cached::Pending);
                } else { ui.ctx().request_repaint_after(Duration::from_millis(50)); }
            }
            if let Some(Cached::Ready(texture)) = self.entries.get(name) {
                let size = texture.size_vec2();
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let target = Rect::from_center_size(rect.center(), size * scale);
                ui.painter().image(texture.id(), target, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                return;
            }
        }
        // A small native application window, recognizable without a letter or a font glyph.
        let painter = ui.painter();
        let body = rect.shrink(rect.width() * 0.1);
        painter.rect_filled(body, 4.0, crate::ui::TRACK);
        painter.rect_stroke(body, 4.0, Stroke::new(1.2, crate::ui::SECONDARY), egui::StrokeKind::Inside);
        let line = body.top() + body.height() * 0.28;
        painter.line_segment([egui::pos2(body.left()+2.0,line),egui::pos2(body.right()-2.0,line)], Stroke::new(1.0, crate::ui::SECONDARY));
        painter.rect_filled(Rect::from_center_size(body.center()+vec2(0.0,3.0),body.size()*0.28),1.0,crate::ui::LABEL);
    }
}
fn icon_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let mut roots = vec![home.join(".icons"), data.join("icons"), data.join("pixmaps")];
    for root in std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into()).split(':').filter(|root| !root.is_empty()) {
        roots.push(PathBuf::from(root).join("icons"));
        roots.push(PathBuf::from(root).join("pixmaps"));
    }
    roots
}
fn resolve(icon: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() { return path.is_file().then(|| path.to_owned()); }
    if icon.contains('/') { return None; }
    for root in roots {
        let mut budget = 30000;
        if let Some(path) = find(root, icon, 0, &mut budget) { return Some(path); }
    }
    None
}
fn find(dir: &Path, icon: &str, depth: usize, budget: &mut usize) -> Option<PathBuf> {
    if depth > 7 || *budget == 0 { return None; }
    // Prefer ordinary 48px assets, then scalable assets, with theme fallbacks.
    for extension in ["", ".png", ".svg", ".xpm", ".jpg", ".jpeg", ".gif"] {
        let candidate = dir.join(format!("{icon}{extension}"));
        if candidate.is_file() { return Some(candidate); }
    }
    let mut children: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    children.sort_by_key(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let priority = match name.as_str() { "hicolor" => 0, "Adwaita" => 1, "Yaru" => 2, "48x48" => 0, "64x64" => 1, "scalable" => 2, _ => 3 };
        (priority, name)
    });
    for entry in children {
        if *budget == 0 { break; }
        *budget -= 1;
        // Do not follow directory symlinks (themes can contain cycles).
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if let Some(path) = find(&entry.path(), icon, depth+1, budget) { return Some(path); }
        }
    }
    None
}
fn decode(path: &Path) -> Option<ColorImage> {
    if std::fs::metadata(path).ok()?.len() > 8 * 1024 * 1024 { return None; }
    let raster = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "svg" | "xpm" => {
            let output = tempfile::Builder::new().suffix(".png").tempfile().ok()?;
            let mut child = std::process::Command::new("/usr/bin/convert")
                .args(["-limit", "memory", "32MiB", "-limit", "map", "64MiB", "-background", "none"])
                .arg(path).args(["-thumbnail", "128x128"])
                .arg(output.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok()?;
            let started = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break,
                    Ok(Some(_)) | Err(_) => return None,
                    Ok(None) if started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(10)),
                    _ => { let _ = child.kill(); let _ = child.wait(); return None; }
                }
            }
            image::open(output.path()).ok()?
        }
        _ => {
            let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            reader.decode().ok()?
        }
    };
    let raster = if raster.width() > 128 || raster.height() > 128 {
        raster.thumbnail(128, 128)
    } else { raster }.into_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([raster.width() as usize, raster.height() as usize], raster.as_raw()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_real_pixels_and_rejects_invalid_files() {
        let fixture = tempfile::tempdir().unwrap();
        let png = fixture.path().join("icon.png");
        image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 255])).save(&png).unwrap();
        let icon = decode(&png).unwrap();
        assert_eq!(icon.size, [2, 3]);
        assert_eq!(icon.pixels[0], Color32::from_rgb(12, 34, 56));
        let broken = fixture.path().join("broken.png");
        std::fs::write(&broken, b"invalid").unwrap();
        assert!(decode(&broken).is_none());
    }
    #[test]
    fn resolves_svg_and_pixmap_filenames() {
        let fixture = tempfile::tempdir().unwrap();
        let theme = fixture.path().join("Adwaita/scalable/apps");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("editor.svg"), []).unwrap();
        std::fs::write(fixture.path().join("legacy.xpm"), []).unwrap();
        let roots = [fixture.path().to_owned()];
        assert_eq!(resolve("editor", &roots), Some(theme.join("editor.svg")));
        assert_eq!(resolve("legacy.xpm", &roots), Some(fixture.path().join("legacy.xpm")));
        assert!(resolve("../outside", &roots).is_none());
    }
    #[test]
    fn finds_theme_icons_and_prefers_user_root() {
        let fixture = tempfile::tempdir().unwrap();
        let user = fixture.path().join("user");
        let system = fixture.path().join("system");
        for root in [&user, &system] {
            std::fs::create_dir_all(root.join("hicolor/48x48/apps")).unwrap();
            std::fs::write(root.join("hicolor/48x48/apps/editor.png"), []).unwrap();
        }
        assert_eq!(resolve("editor", &[user.clone(), system]), Some(user.join("hicolor/48x48/apps/editor.png")));
        let absolute = user.join("hicolor/48x48/apps/editor.png");
        assert_eq!(resolve(absolute.to_str().unwrap(), &[]), Some(absolute));
        assert_eq!(resolve("missing", &[user]), None);
    }
}
