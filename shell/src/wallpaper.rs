//! Bounded asynchronous wallpaper loading and private asset import.

use eframe::egui;
use image::ImageDecoder;
use des_common::appearance::{AppearanceSettings, WallpaperFit};
use std::{path::{Path, PathBuf}, sync::{Arc, Mutex, Condvar, OnceLock, atomic::{AtomicBool, AtomicU64, Ordering}}, collections::VecDeque, io::Read};

const ENCODED_LIMIT: u64 = 32 * 1024 * 1024;
const DECODED_LIMIT: u64 = 64 * 1024 * 1024;

pub fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 || u64::from(width) * u64::from(height) * 4 > DECODED_LIMIT {
        Err("Choose an image no larger than 8192px per side and 16 megapixels".into())
    } else { Ok(()) }
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    // Nonblocking protects the worker from a FIFO substituted for a selected file.
    let file = std::fs::OpenOptions::new().read(true).custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32).open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > ENCODED_LIMIT { return Err("Choose a regular PNG or JPEG file under 32 MiB".into()); }
    let mut bytes = Vec::new();
    file.take(ENCODED_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > ENCODED_LIMIT { return Err("The image exceeds 32 MiB".into()); }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let format = image::guess_format(bytes).map_err(|_| "Choose a valid PNG or JPEG image")?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) { return Err("Only static PNG and JPEG images are supported".into()); }
    // Reject APNG before decode, walking PNG chunks rather than searching pixel bytes.
    if format == image::ImageFormat::Png {
        let mut offset = 8usize;
        while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
            let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
            if &header[4..8] == b"acTL" { return Err("Animated wallpapers are not supported; choose a static image".into()); }
            let Some(next) = offset.checked_add(12).and_then(|n| n.checked_add(len)) else { break; };
            if next > bytes.len() { break; }
            offset = next;
        }
    }
    let dimensions = image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_dimensions().map_err(|e| e.to_string())?;
    check_dimensions(dimensions.0, dimensions.1)?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192); limits.max_image_height = Some(8192); limits.max_alloc = Some(DECODED_LIMIT);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    // Reserve the decoded buffer before allowing decoder scratch allocations,
    // matching ImageReader::decode's allocation accounting.
    limits.reserve(decoder.total_bytes()).map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Drafts own their private imported asset until a successful configuration save.
pub struct Asset { pub path: PathBuf, kept: AtomicBool }
impl Asset { pub fn keep(&self) { self.kept.store(true, Ordering::Relaxed); } }
impl Drop for Asset { fn drop(&mut self) { if !self.kept.load(Ordering::Relaxed) { let _ = std::fs::remove_file(&self.path); } } }
fn data_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME").filter(|p| Path::new(p).is_absolute()).map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share"))).ok_or("No home directory available")?;
    Ok(base.join("des-ui/wallpapers"))
}
fn import(path: &Path, directory: &Path) -> Result<Asset, String> {
    use std::{os::unix::fs::{DirBuilderExt, PermissionsExt}, io::Write};
    let image = decode(&read_bounded(path)?)?;
    // A bounded normalized copy strips source metadata and avoids retaining a huge
    // original solely for a deliberately soft backdrop.
    let normalized = image.thumbnail(2560, 2560).to_rgba8();
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(directory).map_err(|e| e.to_string())?;
    if std::fs::symlink_metadata(directory).map_err(|e| e.to_string())?.file_type().is_symlink() { return Err("Wallpaper data directory cannot be a symbolic link".into()); }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let mut file = tempfile::Builder::new().prefix("wallpaper-").suffix(".png").tempfile_in(directory).map_err(|e| e.to_string())?;
    normalized.write_to(file.as_file_mut(), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    let (_, path) = file.keep().map_err(|e| e.to_string())?;
    let asset = Asset { path, kept: AtomicBool::new(false) };
    std::fs::File::open(directory).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())?;
    Ok(asset)
}
/// Only delete an obsolete asset we generated, never an arbitrary configured path.
pub fn remove_managed(path: &Path) {
    if data_dir().ok().as_deref() == path.parent() && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("wallpaper-")) {
        let _ = std::fs::remove_file(path);
    }
}

struct Pixels { original: egui::ColorImage, soft: egui::ColorImage, peak: [u8; 3], asset: Option<Arc<Asset>> }
type Answer = (u64, Result<Pixels, String>);
struct Request { client: u64, generation: u64, path: PathBuf, blur: f32, importing: bool, ctx: egui::Context, answer: Arc<Mutex<Option<Answer>>>, current: Arc<AtomicU64> }
struct Queue { pending: Mutex<VecDeque<Request>>, ready: Condvar }
fn queue() -> Result<&'static Arc<Queue>, String> {
    static QUEUE: OnceLock<Result<Arc<Queue>, String>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let queue = Arc::new(Queue { pending: Mutex::new(VecDeque::new()), ready: Condvar::new() });
        let worker = queue.clone();
        std::thread::Builder::new().name("wallpaper-images".into()).spawn(move || loop {
            let request = {
                let mut pending = worker.pending.lock().unwrap();
                while pending.is_empty() { pending = worker.ready.wait(pending).unwrap(); }
                pending.pop_front().unwrap()
            };
            if request.current.load(Ordering::Acquire) != request.generation { continue; }
            let result = prepare(&request);
            let mut answer = request.answer.lock().unwrap();
            if request.current.load(Ordering::Acquire) == request.generation {
                *answer = Some((request.generation, result));
                request.ctx.request_repaint();
            }
        }).map_err(|e| format!("Could not start the image loader: {e}"))?;
        Ok(queue)
    }).as_ref().map_err(Clone::clone)
}
fn prepare(request: &Request) -> Result<Pixels, String> {
    let asset = if request.importing { Some(Arc::new(import(&request.path, &data_dir()?)?)) } else { None };
    let path = asset.as_ref().map_or(request.path.as_path(), |a| a.path.as_path());
    let image = decode(&read_bounded(path)?)?.thumbnail(1600, 1600).to_rgba8();
    let peak = image.pixels().fold([0; 3], |peak, pixel| [peak[0].max(pixel[0]), peak[1].max(pixel[1]), peak[2].max(pixel[2])]);
    let small = image::imageops::thumbnail(&image, 640, 640);
    // Resolution-independent strength; the UI labels this as strength, not pixels.
    let sigma = request.blur * small.width().max(small.height()) as f32 / 1920.0;
    let soft = if sigma > 0.0 { image::imageops::blur(&small, sigma) } else { small };
    let color = |image: image::RgbaImage| egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
    Ok(Pixels { original: color(image), soft: color(soft), peak, asset })
}
static CLIENT: AtomicU64 = AtomicU64::new(1);
pub struct Loader {
    client: u64, generation: u64, current: Arc<AtomicU64>, answer: Arc<Mutex<Option<Answer>>>,
    pub loading: bool, pub requested_blur: f32, pub error: Option<String>, pub view: Option<WallpaperView>, pub asset: Option<Arc<Asset>>,
}
impl Default for Loader {
    fn default() -> Self { Self { client: CLIENT.fetch_add(1, Ordering::Relaxed), generation: 0, current: Arc::new(AtomicU64::new(0)), answer: Default::default(), loading: false, requested_blur: 18.0, error: None, view: None, asset: None } }
}
impl Loader {
    fn is_current(&self, generation: u64) -> bool { self.generation == generation }
    pub fn request(&mut self, ctx: &egui::Context, path: &Path, blur: f32, importing: bool) {
        self.requested_blur = blur;
        self.generation += 1; self.current.store(self.generation, Ordering::Release); self.loading = true; self.error = None;
        self.answer.lock().unwrap().take();
        // Clear old preview so controls cannot apply the previous selection while loading.
        self.view = None;
        if importing || self.asset.as_ref().is_some_and(|asset| asset.path != path) { self.asset = None; }
        let queue = match queue() { Ok(queue) => queue, Err(error) => { self.loading = false; self.error = Some(error); return; } };
        let mut pending = queue.pending.lock().unwrap();
        pending.retain(|r| r.client != self.client);
        if pending.len() >= 4 { self.loading = false; self.error = Some("Image loader is busy; try again".into()); return; }
        pending.push_back(Request { client: self.client, generation: self.generation, path: path.into(), blur, importing, ctx: ctx.clone(), answer: self.answer.clone(), current: self.current.clone() });
        queue.ready.notify_one();
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let answer = self.answer.lock().unwrap().take();
        if let Some((generation, result)) = answer {
            if !self.is_current(generation) { return; }
            self.loading = false;
            match result {
                Ok(pixels) => {
                    if pixels.asset.is_some() { self.asset = pixels.asset; }
                    self.view = Some(WallpaperView {
                        peak: pixels.peak,
                        original: ctx.load_texture(format!("wallpaper-{}-{generation}", self.client), pixels.original, egui::TextureOptions::LINEAR),
                        soft: ctx.load_texture(format!("wallpaper-soft-{}-{generation}", self.client), pixels.soft, egui::TextureOptions::LINEAR),
                    });
                },
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub fn clear(&mut self) {
        // Hold the answer lock across invalidation so an in-flight worker cannot
        // publish a cancelled imported asset after the result has been cleared.
        let mut answer = self.answer.lock().unwrap();
        self.generation += 1; self.current.store(self.generation, Ordering::Release);
        answer.take(); self.loading = false; self.error = None; self.view = None; self.asset = None;
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        // Do not initialize a worker simply to dispose an unused loader.
        self.clear();
    }
}
#[derive(Clone)]
pub struct WallpaperView { peak: [u8; 3], original: egui::TextureHandle, soft: egui::TextureHandle }
pub fn geometry(screen: egui::Rect, image: egui::Vec2, fit: WallpaperFit) -> (egui::Rect, egui::Rect) {
    let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    if !screen.is_positive() || image.x <= 0.0 || image.y <= 0.0 { return (screen, whole); }
    let ratios = screen.size() / image;
    match fit {
        WallpaperFit::Fit => (egui::Rect::from_center_size(screen.center(), image * ratios.x.min(ratios.y)), whole),
        WallpaperFit::Fill => {
            let visible = screen.size() / (image * ratios.x.max(ratios.y));
            (screen, egui::Rect::from_center_size(egui::pos2(0.5, 0.5), visible))
        }
    }
}
impl WallpaperView {
    pub fn peak(&self) -> [u8; 3] { self.peak }
    pub fn texture(&self, blurred: bool) -> &egui::TextureHandle { if blurred { &self.soft } else { &self.original } }

}
#[derive(Default)]
pub struct Wallpaper { pub loader: Loader, key: Option<(PathBuf, u32)> }
impl Wallpaper {
    pub fn adopt(&mut self, appearance: &AppearanceSettings, view: WallpaperView) {
        self.loader.clear(); self.loader.view = Some(view);
        self.key = appearance.wallpaper.as_ref().map(|path| (path.clone(), appearance.blur.to_bits()));
    }
    pub fn update(&mut self, ctx: &egui::Context, appearance: &AppearanceSettings) -> Option<String> {
        let key = appearance.wallpaper.as_ref().map(|p| (p.clone(), appearance.blur.to_bits()));
        if key != self.key {
            self.key = key;
            if let Some((path, _)) = &self.key { self.loader.request(ctx, path, appearance.blur, false); }
            else { self.loader.clear(); }
        }
        self.loader.poll(ctx);
        self.loader.error.take()
    }
    pub fn publish(&self, ctx: &egui::Context, appearance: &AppearanceSettings) {
        crate::material::publish(ctx, appearance.clone(), self.loader.view.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn wallpaper_decode_rejects_bad_and_unsupported_inputs() {
        assert!(decode(b"not an image").is_err());
        assert!(decode(b"GIF89a").is_err());
        let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert_eq!(decode(bytes.get_ref()).unwrap().dimensions(), (8, 4));
        assert!(check_dimensions(8193, 1).is_err());
        assert!(check_dimensions(8192, 8192).is_err());
        assert!(check_dimensions(4096, 4096).is_ok());
        let directory = tempfile::tempdir().unwrap();
        let large = directory.path().join("large.png");
        std::fs::File::create(&large).unwrap().set_len(ENCODED_LIMIT + 1).unwrap();
        assert!(read_bounded(&large).is_err());
        let mut apng = bytes.into_inner();
        let mut animation = Vec::from(8u32.to_be_bytes());
        animation.extend_from_slice(b"acTL"); animation.extend_from_slice(&[0; 12]);
        apng.splice(8..8, animation);
        assert!(decode(&apng).unwrap_err().contains("Animated"));
    }
    #[test]
    fn wallpaper_jpeg_import_respects_photo_orientation() {
        let image = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 50, 20]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Jpeg).unwrap();
        let mut jpeg = bytes.into_inner();
        assert_eq!(decode(&jpeg).unwrap().dimensions(), (8, 4));
        // EXIF orientation 6: phone photo stored sideways, displayed upright.
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut segment = vec![0xff, 0xe1];
        segment.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        segment.extend_from_slice(exif);
        jpeg.splice(2..2, segment);
        assert_eq!(decode(&jpeg).unwrap().dimensions(), (4, 8));
    }
    #[test]
    fn wallpaper_import_is_private_and_survives_source_removal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source # = 雨.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
        let asset = import(&source, &dir.path().join("managed")).unwrap();
        std::fs::remove_file(source).unwrap();
        assert!(decode(&read_bounded(&asset.path).unwrap()).is_ok());
        assert_eq!(std::fs::metadata(&asset.path).unwrap().permissions().mode() & 0o777, 0o600);
        let path = asset.path.clone();
        asset.keep();
        drop(asset);
        assert!(path.exists(), "successfully saved imports must survive editor/loader shutdown");
    }
    #[test]
    fn wallpaper_geometry_preserves_aspect_and_crops_without_stretching() {
        use des_common::appearance::WallpaperFit;
        let screen = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 200.0));
        let (dest, uv) = geometry(screen, egui::vec2(400.0, 200.0), WallpaperFit::Fill);
        assert_eq!(dest, screen);
        assert_eq!(uv.width(), 0.5);
        let (dest, uv) = geometry(screen, egui::vec2(400.0, 200.0), WallpaperFit::Fit);
        assert_eq!(dest.size(), egui::vec2(200.0, 100.0));
        assert_eq!(dest.center(), screen.center());
        assert_eq!(uv.width(), 1.0);
    }
    #[test]
    fn wallpaper_latest_result_cannot_replace_a_newer_request() {
        let ctx = egui::Context::default();
        let mut loader = Loader::default();
        loader.generation = 3; loader.loading = true;
        let pixels = || Pixels { original: egui::ColorImage::new([2, 2], egui::Color32::RED), soft: egui::ColorImage::new([2, 2], egui::Color32::RED), peak: [255, 0, 0], asset: None };
        *loader.answer.lock().unwrap() = Some((2, Ok(pixels())));
        loader.poll(&ctx);
        assert!(loader.view.is_none());
        assert!(loader.loading);
        *loader.answer.lock().unwrap() = Some((3, Ok(pixels())));
        loader.poll(&ctx);
        assert!(loader.view.is_some());
        assert!(!loader.loading);
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    #[test]
    fn wallpaper_cancel_releases_pending_assets_without_another_poll() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).save(&source).unwrap();
        let asset = Arc::new(import(&source, &dir.path().join("managed")).unwrap());
        let path = asset.path.clone();
        let mut loader = Loader::default();
        *loader.answer.lock().unwrap() = Some((0, Ok(Pixels { original: egui::ColorImage::new([2, 2], egui::Color32::RED), soft: egui::ColorImage::new([2, 2], egui::Color32::RED), peak: [255, 0, 0], asset: Some(asset) })));
        loader.clear();
        assert!(!path.exists(), "Cancel must release an unconsumed imported result");
    }
}
