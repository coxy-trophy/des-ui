//! Thumbnails for the file manager's grid: pictures made small on two background
//! threads, kept as textures while their folder is open.

use eframe::egui::{self, ColorImage, TextureHandle};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// The longest side of a thumbnail, in pixels.
pub const SIZE: u32 = 192;
/// Larger pictures are not read for a thumbnail.
const MAX_BYTES: u64 = 40 * 1024 * 1024;

pub fn can_thumbnail(path: &Path) -> bool {
    matches!(path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif")
}

/// A picture shrunk to fit `SIZE`, or nothing when it cannot be read.
pub fn make(path: &Path) -> Option<ColorImage> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES { return None; }
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.thumbnail(SIZE, SIZE).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}

struct Job { path: PathBuf, modified: Option<SystemTime>, reply: Sender<Done>, ctx: egui::Context }
type Done = (PathBuf, Option<SystemTime>, Option<ColorImage>);
struct Queue { jobs: Mutex<VecDeque<Job>>, ready: Condvar }

fn queue() -> &'static Queue {
    static QUEUE: OnceLock<&'static Queue> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let queue: &'static Queue = Box::leak(Box::new(Queue { jobs: Mutex::new(VecDeque::new()), ready: Condvar::new() }));
        for n in 0..2 {
            let _ = std::thread::Builder::new().name(format!("thumbnails-{n}")).spawn(move || loop {
                let job = {
                    let mut jobs = queue.jobs.lock().unwrap();
                    while jobs.is_empty() { jobs = queue.ready.wait(jobs).unwrap(); }
                    jobs.pop_front().unwrap()
                };
                let image = make(&job.path);
                if job.reply.send((job.path, job.modified, image)).is_ok() { job.ctx.request_repaint(); }
            });
        }
        queue
    })
}

/// One folder view's thumbnails.
pub struct Thumbs {
    ready: HashMap<PathBuf, (Option<SystemTime>, Option<TextureHandle>)>,
    asked: HashMap<PathBuf, Instant>,
    reply: Sender<Done>,
    answers: Receiver<Done>,
}

impl Default for Thumbs {
    fn default() -> Self {
        let (reply, answers) = channel();
        Thumbs { ready: HashMap::new(), asked: HashMap::new(), reply, answers }
    }
}

impl Thumbs {
    /// The thumbnail of `path`, asking for it when there is none yet. `None` while it
    /// is being made, and for good when the picture cannot be read.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path, modified: Option<SystemTime>) -> Option<TextureHandle> {
        while let Ok((done, when, image)) = self.answers.try_recv() {
            self.asked.remove(&done);
            let texture = image.map(|image| ctx.load_texture(format!("thumb:{}", done.display()), image, egui::TextureOptions::LINEAR));
            self.ready.insert(done, (when, texture));
        }
        if let Some((when, texture)) = self.ready.get(path) {
            if *when == modified { return texture.clone(); }
        }
        let waiting = self.asked.get(path).is_some_and(|at| at.elapsed() < Duration::from_secs(10));
        if !waiting {
            self.asked.insert(path.to_path_buf(), Instant::now());
            let queue = queue();
            let mut jobs = queue.jobs.lock().unwrap();
            // Pictures scrolled past are worth less than the ones on screen now.
            while jobs.len() >= 48 { jobs.pop_front(); }
            jobs.push_back(Job { path: path.to_path_buf(), modified, reply: self.reply.clone(), ctx: ctx.clone() });
            queue.ready.notify_one();
        }
        None
    }

    /// Lets go of every texture, as when the folder changes.
    pub fn clear(&mut self) {
        self.ready.clear();
        self.asked.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnails_fit_the_size_and_unreadable_files_have_none() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("wide.png");
        image::RgbaImage::from_pixel(800, 400, image::Rgba([10, 200, 90, 255])).save(&picture).unwrap();
        let thumb = make(&picture).unwrap();
        assert_eq!(thumb.size, [SIZE as usize, SIZE as usize / 2]);
        let broken = dir.path().join("broken.jpg");
        std::fs::write(&broken, b"not a picture").unwrap();
        assert!(make(&broken).is_none());
        assert!(can_thumbnail(Path::new("a.JPG")) && !can_thumbnail(Path::new("a.txt")));
    }
}
