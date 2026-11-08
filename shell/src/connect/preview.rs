//! One bounded on-demand frame request; no socket work occurs in egui callbacks.
use super::{connect_local, socket_path, IO_TIMEOUT};
use eframe::egui;
use image::ImageDecoder;
use std::{io::{BufRead, BufReader, Read, Write}, sync::mpsc::{self, Receiver}, time::{Duration, Instant}};

const INTERVAL: Duration = Duration::from_millis(100);
const CAST_INTERVAL: Duration = Duration::from_millis(200);
const CAST_FRESHNESS: Duration = Duration::from_millis(800);
const MAX_AXIS: usize = 960;
const MAX_JPEG: usize = 44 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Session { pub device_id: String, pub serial: String, pub wireless: bool }

enum Frame { Pixels { size: [usize; 2], rgb: Vec<u8>, geometry: Option<u64> }, Wait, Stopped, Locked, Error }

fn line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader.take(65).read_until(b'\n', &mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 64 || !bytes.ends_with(b"\n") { return Err("Invalid preview header".into()); }
    String::from_utf8(bytes).map_err(|_| "Invalid preview header".into())
}

fn read_frame(reader: &mut impl BufRead) -> Result<Frame, String> {
    let header = line(reader)?;
    match header.as_str() {
        "WAIT\n" => return Ok(Frame::Wait),
        "STOPPED\n" => return Ok(Frame::Stopped),
        "LOCKED\n" => return Ok(Frame::Locked),
        "ERROR\n" => return Ok(Frame::Error),
        "P6\n" => {},
        _ if header.starts_with("JPEG ") => return read_cast_frame(reader, &header),
        _ => return Err("Invalid preview response".into()),
    }
    let dimensions = line(reader)?;
    let size = dimensions.split_whitespace().map(str::parse::<usize>).collect::<Result<Vec<_>, _>>().map_err(|_| "Invalid preview dimensions")?;
    if size.len() != 2 || size.iter().any(|n| *n == 0 || *n > MAX_AXIS) { return Err("Preview dimensions exceed bounds".into()); }
    if line(reader)? != "255\n" { return Err("Invalid preview pixels".into()); }
    let mut rgb = vec![0; size[0] * size[1] * 3];
    reader.read_exact(&mut rgb).map_err(|_| "Incomplete preview frame")?;
    Ok(Frame::Pixels { size: [size[0], size[1]], rgb, geometry: None })
}

fn read_cast_frame(reader: &mut impl Read, header: &str) -> Result<Frame, String> {
    let mut parts = header.trim_end_matches('\n').split(' ').skip(1);
    let length = parts.next().and_then(|v| v.parse::<usize>().ok()).filter(|n| (1..=MAX_JPEG).contains(n)).ok_or("Invalid cast frame size")?;
    let geometry = parts.next().and_then(|v| v.parse::<u64>().ok()).filter(|n| *n != 0).ok_or("Invalid cast geometry")?;
    if parts.next().is_some() { return Err("Invalid cast frame header".into()); }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|_| "Incomplete cast frame")?;
    if !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]) { return Err("Incomplete cast JPEG".into()); }
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes)).map_err(|_| "Invalid cast JPEG")?;
    let (width, height) = decoder.dimensions();
    if [width, height].iter().any(|axis| *axis == 0 || *axis > MAX_AXIS as u32) { return Err("Cast dimensions exceed bounds".into()); }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_AXIS as u32);
    limits.max_image_height = Some(MAX_AXIS as u32);
    limits.max_alloc = Some(16 * 1024 * 1024);
    decoder.set_limits(limits).map_err(|_| "Invalid cast limits")?;
    let rgb = image::DynamicImage::from_decoder(decoder).map_err(|_| "Could not decode cast JPEG")?.into_rgb8().into_raw();
    Ok(Frame::Pixels { size: [width as usize, height as usize], rgb, geometry: Some(geometry) })
}

fn exchange_frame(session: &Session) -> Result<Frame, String> {
    let mut stream = connect_local(&socket_path())?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut stream, &serde_json::json!({ "op":"screen_frame", "device_id":session.device_id, "serial":session.serial })).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    read_frame(&mut BufReader::new(stream))
}

#[derive(Default)]
pub(super) struct Preview {
    pub texture: Option<egui::TextureHandle>,
    pub message: Option<&'static str>,
    pending: Option<(u64, Receiver<Result<Frame, String>>)>,
    generation: u64,
    last: Option<Instant>,
    session: Option<Session>,
    cast_frame: Option<(u64, Instant)>,
    pub blocked: bool,
    #[cfg(test)]
    pub fixture: bool,
}
impl Preview {
    pub fn cast_geometry(&self, session: &Session) -> Option<u64> {
        if self.blocked || self.session.as_ref() != Some(session) || self.texture.is_none() { return None; }
        self.cast_frame.filter(|(_, at)| at.elapsed() < CAST_FRESHNESS).map(|(geometry, _)| geometry)
    }

    fn clear_pixels(&mut self) {
        self.texture = None;
        self.cast_frame = None;
    }

    pub fn clear(&mut self) {
        self.clear_pixels();
        self.message = None;
        self.session = None;
        self.generation = self.generation.wrapping_add(1);
        // Retain the in-flight receiver until completion: hide/show cannot create concurrent polls.
    }
    pub fn tick(&mut self, ctx: &egui::Context, session: Option<Session>) {
        #[cfg(test)]
        if self.fixture { return; }
        if session.is_none() { self.clear_pixels(); }
        if session != self.session { self.clear(); self.session = session; }
        if self.cast_frame.is_some_and(|(_, at)| at.elapsed() >= CAST_FRESHNESS) { self.clear_pixels(); }
        if let Some((generation, rx)) = &self.pending {
            let received = rx.try_recv();
            if let Ok(result) = received {
                let valid = *generation == self.generation && self.session.is_some() && !self.blocked;
                self.pending = None;
                if valid {
                    match result {
                        Ok(Frame::Pixels { size, rgb, geometry })
                            if self.session.as_ref().is_some_and(|s| s.wireless == geometry.is_some()) => {
                            let image = egui::ColorImage::from_rgb(size, &rgb);
                            if let Some(texture) = &mut self.texture { texture.set(image, egui::TextureOptions::LINEAR); }
                            else { self.texture = Some(ctx.load_texture("connect-live-preview", image, egui::TextureOptions::LINEAR)); }
                            self.cast_frame = geometry.map(|geometry| (geometry, Instant::now()));
                            self.message = None;
                        }
                        Ok(Frame::Wait) => { self.clear_pixels(); self.message = None; }
                        Ok(Frame::Locked) => { self.clear(); self.blocked = true; self.message = Some("Session locked · sharing stopped"); }
                        Ok(Frame::Stopped) => { self.clear(); self.blocked = true; self.message = Some("Sharing stopped"); }
                        _ => { self.clear_pixels(); self.message = Some("Preview unavailable · stop and retry"); }
                    }
                }
            } else if matches!(received, Err(mpsc::TryRecvError::Disconnected)) {
                self.pending = None;
                self.clear_pixels();
            }
        }
        let interval = if self.session.as_ref().is_some_and(|s| s.wireless) { CAST_INTERVAL } else { INTERVAL };
        if self.session.is_some() && !self.blocked && self.pending.is_none() && self.last.is_none_or(|at| at.elapsed() >= interval) {
            let (tx, rx) = mpsc::sync_channel(1);
            let context = ctx.clone();
            let session = self.session.clone().unwrap();
            std::thread::spawn(move || { let _ = tx.send(exchange_frame(&session)); context.request_repaint(); });
            self.pending = Some((self.generation, rx));
            self.last = Some(Instant::now());
        }
        if self.session.is_some() { ctx.request_repaint_after(interval); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn jpeg_fixture(width: u32, height: u32) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&vec![80; (width * height * 3) as usize], width, height, image::ExtendedColorType::Rgb8).unwrap();
        jpeg
    }
    #[test]
    fn bounded_jpeg_frame_accepts_real_pixels_and_rejects_invalid_data() {
        let jpeg = jpeg_fixture(8, 4);
        let mut frame = format!("JPEG {} 7\n", jpeg.len()).into_bytes();
        frame.extend_from_slice(&jpeg);
        assert!(matches!(read_frame(&mut std::io::Cursor::new(&frame)), Ok(Frame::Pixels { size: [8, 4], geometry: Some(7), .. })));
        assert!(read_frame(&mut std::io::Cursor::new(&frame[..frame.len()-1])).is_err());
        for header in ["JPEG 0 7\n", "JPEG 45057 7\n", "JPEG 3 0\n", "JPEG 3 7 extra\n", "JPEG 3 7\nbad"] {
            assert!(read_frame(&mut std::io::Cursor::new(header)).is_err());
        }
        let oversized = jpeg_fixture(961, 1);
        let mut frame = format!("JPEG {} 7\n", oversized.len()).into_bytes();
        frame.extend(oversized);
        assert!(read_frame(&mut std::io::Cursor::new(frame)).is_err());
    }
    #[test]
    fn bounded_parser_rejects_hostile_headers_and_partial_frames() {
        for bytes in [b"P6\n961 1\n255\n".as_slice(), b"P6\n0 3\n255\n", b"P6\n1 1\n255\n\0\0", b"P6\n1 1 1\n255\n"] {
            assert!(read_frame(&mut std::io::Cursor::new(bytes)).is_err());
        }
        assert!(read_frame(&mut std::io::Cursor::new(vec![b'A'; 100])).is_err());
        match read_frame(&mut std::io::Cursor::new(b"P6\n1 1\n255\n\0\x80\xff")).unwrap() {
            Frame::Pixels { size, rgb, geometry } => { assert_eq!(size, [1, 1]); assert_eq!(rgb, [0,128,255]); assert_eq!(geometry, None); }, _ => panic!(),
        }
    }
    #[test]
    fn hidden_then_visible_discards_late_pixels_and_keeps_one_pending_poll() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::sync_channel(1);
        let session = Session { device_id:"phone".into(), serial:"USB".into(), wireless: false };
        let mut preview = Preview { session: Some(session.clone()), pending: Some((0, rx)), last: Some(Instant::now()), ..Default::default() };
        preview.clear();
        preview.tick(&ctx, Some(session.clone()));
        assert!(preview.pending.is_some());
        tx.send(Ok(Frame::Pixels { size: [1,1], rgb: vec![1,2,3], geometry: None })).unwrap();
        preview.tick(&ctx, Some(session));
        assert!(preview.texture.is_none());
    }

    #[test]
    fn input_geometry_requires_fresh_pixels_from_the_current_session() {
        let ctx = egui::Context::default();
        let session = Session { device_id: "phone".into(), serial: "first".into(), wireless: true };
        let foreign = Session { serial: "second".into(), ..session.clone() };
        let mut preview = Preview {
            session: Some(session.clone()),
            texture: Some(ctx.load_texture("test pixel", egui::ColorImage::from_rgb([1, 1], &[1, 2, 3]), egui::TextureOptions::LINEAR)),
            cast_frame: Some((15, Instant::now())),
            ..Default::default()
        };
        assert_eq!(preview.cast_geometry(&session), Some(15));
        assert_eq!(preview.cast_geometry(&foreign), None);
        preview.cast_frame = Some((15, Instant::now() - CAST_FRESHNESS));
        assert_eq!(preview.cast_geometry(&session), None);
        preview.clear();
        assert!(preview.texture.is_none());
    }
}
