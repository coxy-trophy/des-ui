//! Short-lived cast state for a single authenticated phone. The caller owns
//! transport, live lock checks, and admission to the shared screen slot.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::{io::Cursor, time::{Duration, Instant}};
use zune_jpeg::JpegDecoder;

pub(crate) const CAPABILITY: &str = "des.connect.cast.v1";
pub(crate) const LEASE: Duration = Duration::from_secs(3);
const CONSENT: Duration = Duration::from_secs(60);
const VIDEO_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_JPEG: usize = 44 * 1024;
const MAX_AXIS: u16 = 960;

pub(crate) struct Cast {
    pub peer: String,
    pub epoch: u64,
    pub id: String,
    created: Instant,
    viewer: Instant,
    geometry: Option<u64>,
    video_deadline: Option<Instant>,
    control: bool,
    latest: Option<Frame>,
    last_frame_packet: Option<Instant>,
    last_input: Option<Instant>,
}

struct Frame {
    received: Instant,
    jpeg: Vec<u8>,
}

pub(crate) enum Update { Ignored, Stop }

impl Cast {
    pub fn new(peer: String, epoch: u64, id: String, now: Instant) -> Self {
        Self { peer, epoch, id, created: now, viewer: now, geometry: None, video_deadline: None, control: false, latest: None, last_frame_packet: None, last_input: None }
    }

    pub fn matches(&self, peer: &str, id: &str) -> bool { self.peer == peer && self.id == id }
    pub fn owned_by(&self, peer: &str, epoch: u64) -> bool { self.peer == peer && self.epoch == epoch }

    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.viewer) >= LEASE
            || (self.geometry.is_none() && now.duration_since(self.created) >= CONSENT)
            || self.video_deadline.is_some_and(|deadline| now >= deadline)
    }

    pub fn renew(&mut self, now: Instant) -> bool {
        if self.expired(now) { return false; }
        self.viewer = now;
        true
    }

    pub fn snapshot(&self) -> Value {
        let live = self.geometry.is_some();
        json!({
            "mode": "wireless", "device_id": self.peer, "serial": self.id, "running": true, "control": self.control,
            "message": if !live { "Approve casting on the phone." } else if self.control { "Wireless cast · Control enabled" } else { "Wireless cast · View only" }
        })
    }

    pub fn receive(&mut self, body: &Map<String, Value>, now: Instant) -> Update {
        if body.get("id").and_then(Value::as_str) != Some(&self.id) || self.expired(now) { return Update::Ignored; }
        match body.get("op").and_then(Value::as_str) {
            Some("status") => match body.get("phase").and_then(Value::as_str) {
                Some("stopped") => return Update::Stop,
                Some("live") => {
                    let (Some(control), Some(geometry)) = (body.get("control").and_then(Value::as_bool), geometry(body.get("geometry"))) else { return Update::Ignored; };
                    if self.geometry.is_some_and(|previous| geometry < previous) { return Update::Ignored; }
                    if self.geometry != Some(geometry) { self.video_deadline = Some(now + VIDEO_TIMEOUT); }
                    self.geometry = Some(geometry);
                    self.control = control;
                    self.latest = None;
                    self.last_frame_packet = None;
                }
                _ => {}
            },
            Some("frame") => {
                if self.geometry.is_none() || geometry(body.get("geometry")) != self.geometry { return Update::Ignored; }
                if self.last_frame_packet.is_some_and(|last| now.duration_since(last) < Duration::from_millis(100)) { return Update::Ignored; }
                self.last_frame_packet = Some(now);
                self.latest = body.get("jpeg").and_then(Value::as_str).and_then(decode_jpeg).map(|jpeg| {
                    self.video_deadline = Some(now + VIDEO_TIMEOUT);
                    Frame { received: now, jpeg }
                });
            }
            _ => {}
        }
        Update::Ignored
    }

    pub fn frame(&self, now: Instant) -> Option<(u64, Vec<u8>)> {
        let geometry = self.geometry?;
        let frame = self.latest.as_ref().filter(|f| now.duration_since(f.received) < LEASE)?;
        let mut bytes = format!("JPEG {} {geometry}\n", frame.jpeg.len()).into_bytes();
        bytes.extend_from_slice(&frame.jpeg);
        Some((geometry, bytes))
    }

    pub fn can_release(&self, generation: Option<u64>, now: Instant) -> bool {
        !self.expired(now) && generation.is_none_or(|g| {
            self.geometry == Some(g) && self.latest.as_ref().is_some_and(|frame| now.duration_since(frame.received) < LEASE)
        })
    }

    pub fn input_allowed(&self, geometry: u64, now: Instant) -> bool {
        self.control && self.can_release(Some(geometry), now)
    }

    pub fn input(&mut self, args: &std::collections::BTreeMap<String, Value>, now: Instant) -> Result<Value, &'static str> {
        let geometry = geometry(args.get("geometry")).ok_or("missing cast geometry")?;
        if !self.input_allowed(geometry, now) { return Err("remote control is not authorized for the current preview"); }
        if self.last_input.is_some_and(|last| now.duration_since(last) < Duration::from_millis(40)) { return Err("remote input is arriving too quickly"); }
        let action = args.get("action").and_then(Value::as_str).ok_or("missing remote input action")?;
        let mut body = json!({"op":"input","id":self.id,"action":action,"geometry":geometry});
        let fields: &[&str] = match action {
            "tap" => &["x", "y"],
            "swipe" => &["x", "y", "to_x", "to_y"],
            "back" | "home" | "recents" => &[],
            _ => return Err("unsupported remote input action"),
        };
        for &field in fields {
            let point = args.get(field).and_then(Value::as_u64).filter(|&n| n <= 10_000).ok_or("remote input lies outside the preview")?;
            body[field] = json!(point);
        }
        self.last_input = Some(now);
        Ok(body)
    }
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn new_id() -> Result<String, openssl::error::ErrorStack> {
    let mut random = [0u8; 16];
    openssl::rand::rand_bytes(&mut random)?;
    Ok(random.iter().map(|b| format!("{b:02x}")).collect())
}

fn geometry(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).filter(|&v| (1..=i64::MAX as u64).contains(&v))
}

fn decode_jpeg(encoded: &str) -> Option<Vec<u8>> {
    if encoded.len() > MAX_JPEG.div_ceil(3) * 4 { return None; }
    let jpeg = STANDARD.decode(encoded).ok()?;
    if jpeg.len() > MAX_JPEG || !jpeg.starts_with(&[0xff, 0xd8]) || !jpeg.ends_with(&[0xff, 0xd9]) { return None; }
    let mut decoder = JpegDecoder::new(Cursor::new(jpeg.as_slice()));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    ((1..=MAX_AXIS).contains(&info.width) && (1..=MAX_AXIS).contains(&info.height)).then_some(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_polls_cannot_extend_phone_consent_forever() {
        let start = Instant::now();
        let mut cast = Cast::new("phone".into(), 1, "cast".into(), start);
        for second in (2..60).step_by(2) {
            assert!(cast.renew(start + Duration::from_secs(second)));
        }
        assert!(!cast.renew(start + CONSENT));
    }

    #[test]
    fn live_cast_without_video_ends_even_if_status_and_viewer_still_poll() {
        let start = Instant::now();
        let mut cast = Cast::new("phone".into(), 1, "cast".into(), start);
        let status = json!({"op":"status","id":"cast","phase":"live","control":false,"geometry":1});
        for second in [0, 2, 4, 6] {
            let now = start + Duration::from_secs(second);
            assert!(cast.renew(now));
            cast.receive(status.as_object().unwrap(), now);
        }
        assert!(!cast.renew(start + VIDEO_TIMEOUT));
    }
}
