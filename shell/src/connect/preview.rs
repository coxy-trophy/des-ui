//! One bounded on-demand frame request; no socket work occurs in egui callbacks.
use super::{connect_local, socket_path, IO_TIMEOUT};
use eframe::egui;
use std::{io::{BufRead, BufReader, Read, Write}, sync::mpsc::{self, Receiver}, time::{Duration, Instant}};

const INTERVAL: Duration = Duration::from_millis(100);
const MAX_AXIS: usize = 960;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Session { pub device_id: String, pub serial: String }

pub(super) enum Frame { Pixels { size: [usize; 2], rgb: Vec<u8> }, Wait, Stopped, Locked, Error }

fn line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader.take(65).read_until(b'\n', &mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 64 || !bytes.ends_with(b"\n") { return Err("Invalid preview header".into()); }
    String::from_utf8(bytes).map_err(|_| "Invalid preview header".into())
}

fn read_frame(reader: &mut impl BufRead) -> Result<Frame, String> {
    match line(reader)?.as_str() {
        "WAIT\n" => return Ok(Frame::Wait),
        "STOPPED\n" => return Ok(Frame::Stopped),
        "LOCKED\n" => return Ok(Frame::Locked),
        "ERROR\n" => return Ok(Frame::Error),
        "P6\n" => {},
        _ => return Err("Invalid preview response".into()),
    }
    let dimensions = line(reader)?;
    let size = dimensions.split_whitespace().map(str::parse::<usize>).collect::<Result<Vec<_>, _>>().map_err(|_| "Invalid preview dimensions")?;
    if size.len() != 2 || size.iter().any(|n| *n == 0 || *n > MAX_AXIS) { return Err("Preview dimensions exceed bounds".into()); }
    if line(reader)? != "255\n" { return Err("Invalid preview pixels".into()); }
    let mut rgb = vec![0; size[0] * size[1] * 3];
    reader.read_exact(&mut rgb).map_err(|_| "Incomplete preview frame")?;
    Ok(Frame::Pixels { size: [size[0], size[1]], rgb })
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
    pub blocked: bool,
    #[cfg(test)]
    pub fixture: bool,
}
impl Preview {
    pub fn clear(&mut self) {
        self.texture = None;
        self.message = None;
        self.session = None;
        self.generation = self.generation.wrapping_add(1);
        // Retain the in-flight receiver until completion: hide/show cannot create concurrent polls.
    }
    pub fn tick(&mut self, ctx: &egui::Context, session: Option<Session>) {
        #[cfg(test)]
        if self.fixture { return; }
        if session.is_none() { self.texture = None; }
        if session != self.session { self.clear(); self.session = session; }
        if let Some((generation, rx)) = &self.pending {
            let received = rx.try_recv();
            if let Ok(result) = received {
                let valid = *generation == self.generation && self.session.is_some() && !self.blocked;
                self.pending = None;
                if valid {
                    match result {
                        Ok(Frame::Pixels { size, rgb }) => {
                            let image = egui::ColorImage::from_rgb(size, &rgb);
                            if let Some(texture) = &mut self.texture { texture.set(image, egui::TextureOptions::LINEAR); }
                            else { self.texture = Some(ctx.load_texture("connect-live-preview", image, egui::TextureOptions::LINEAR)); }
                            self.message = None;
                        }
                        Ok(Frame::Wait) => { self.texture = None; self.message = Some("Waiting for device screen…"); }
                        Ok(Frame::Locked) => { self.clear(); self.blocked = true; self.message = Some("Session locked · sharing stopped"); }
                        Ok(Frame::Stopped) => { self.clear(); self.blocked = true; self.message = Some("Sharing stopped"); }
                        _ => { self.texture = None; self.message = Some("Preview unavailable · stop and retry"); }
                    }
                }
            } else if matches!(received, Err(mpsc::TryRecvError::Disconnected)) {
                self.pending = None;
                self.texture = None;
            }
        }
        if self.session.is_some() && !self.blocked && self.pending.is_none() && self.last.is_none_or(|at| at.elapsed() >= INTERVAL) {
            let (tx, rx) = mpsc::sync_channel(1);
            let context = ctx.clone();
            let session = self.session.clone().unwrap();
            std::thread::spawn(move || { let _ = tx.send(exchange_frame(&session)); context.request_repaint(); });
            self.pending = Some((self.generation, rx));
            self.last = Some(Instant::now());
        }
        if self.session.is_some() { ctx.request_repaint_after(INTERVAL); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_parser_rejects_hostile_headers_and_partial_frames() {
        for bytes in [b"P6\n961 1\n255\n".as_slice(), b"P6\n0 3\n255\n", b"P6\n1 1\n255\n\0\0", b"P6\n1 1 1\n255\n"] {
            assert!(read_frame(&mut std::io::Cursor::new(bytes)).is_err());
        }
        assert!(read_frame(&mut std::io::Cursor::new(vec![b'A'; 100])).is_err());
        match read_frame(&mut std::io::Cursor::new(b"P6\n1 1\n255\n\0\x80\xff")).unwrap() {
            Frame::Pixels { size, rgb } => { assert_eq!(size, [1, 1]); assert_eq!(rgb, [0,128,255]); }, _ => panic!(),
        }
    }
    #[test]
    fn hidden_then_visible_discards_late_pixels_and_keeps_one_pending_poll() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut preview = Preview { session: Some(Session { device_id:"phone".into(), serial:"USB".into() }), pending: Some((0, rx)), last: Some(Instant::now()), ..Default::default() };
        preview.clear();
        preview.tick(&ctx, Some(Session { device_id:"phone".into(), serial:"USB".into() }));
        assert!(preview.pending.is_some());
        tx.send(Ok(Frame::Pixels { size: [1,1], rgb: vec![1,2,3] })).unwrap();
        preview.tick(&ctx, Some(Session { device_id:"phone".into(), serial:"USB".into() }));
        assert!(preview.texture.is_none());
    }
}
