//! MIT-licensed, independent KDE Connect media interoperability.
//! Caller must authenticate the peer and enforce separate media/system-volume opt-ins.
//! Construction performs no I/O; requests hold a temporary session bus connection.
use serde_json::{Value, json};
use std::{collections::HashMap, fmt, io::{self, Read}, process::{Command, Stdio}, time::{Duration, Instant}, os::fd::AsFd};
use zbus::{blocking::{Connection, Proxy}, zvariant::{OwnedValue, ObjectPath}};

pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";
pub const SYSTEM_VOLUME_REQUEST: &str = "kdeconnect.systemvolume.request";
const MPRIS: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";
const SINK: &str = "default-output";
const TIMEOUT: Duration = Duration::from_secs(1);
#[derive(Debug)]
pub struct MediaError(pub String);
impl fmt::Display for MediaError { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for MediaError {}
fn error(s: impl ToString) -> MediaError { MediaError(s.to_string()) }
#[derive(Debug, Clone)]
pub struct MediaReply { pub kind: &'static str, pub body: Value }
#[derive(Default)]
pub struct MediaAdapter;
impl MediaAdapter {
    /// No event subscriptions or polling. Invoke only for an authorized explicit request.
    pub fn handle(&mut self, kind: &str, body: &Value) -> Result<Vec<MediaReply>, MediaError> {
        if !body.is_object() { return Err(error("expected media object")); }
        match kind {
            MPRIS_REQUEST => media(body),
            SYSTEM_VOLUME_REQUEST => system_volume(body),
            _ => Err(error("unsupported media packet")),
        }
    }
}
fn reply(kind: &'static str, body: Value) -> Vec<MediaReply> { vec![MediaReply { kind, body }] }
fn valid_player(s: &str) -> bool {
    s.starts_with("org.mpris.MediaPlayer2.") && s.len() <= 255
        && s.split('.').all(|p| !p.is_empty() && !p.as_bytes()[0].is_ascii_digit()
            && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}
fn percent(v: &Value) -> Result<f64, MediaError> {
    let n = v.as_f64().ok_or_else(|| error("volume must be numeric"))?;
    if n.is_finite() && (0.0..=100.0).contains(&n) { Ok(n) } else { Err(error("volume must be 0..100")) }
}
fn integer(v: &Value) -> Result<i64, MediaError> { v.as_i64().ok_or_else(|| error("expected signed integer")) }
fn milliseconds_to_microseconds(n: i64) -> Result<i64, MediaError> { n.checked_mul(1000).ok_or_else(|| error("position overflow")) }
fn proxy<'a>(c: &'a Connection, name: &'a str) -> Result<Proxy<'a>, MediaError> {
    zbus::blocking::proxy::Builder::new(c).destination(name).map_err(error)?.path(PATH).map_err(error)?
        .interface(MPRIS).map_err(error)?.cache_properties(zbus::proxy::CacheProperties::No).build().map_err(error)
}
fn names(c: &Connection, started: &Instant) -> Result<Vec<String>, MediaError> {
    check_budget(started)?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(c).map_err(error)?;
    let mut names: Vec<String> = dbus.list_names().map_err(error)?.into_iter()
        .map(|n| n.to_string()).filter(|n| valid_player(n)).take(16).collect();
    names.sort(); Ok(names)
}
fn check_budget(started: &Instant) -> Result<(), MediaError> {
    // Reserve a complete one-second call window. No operation may start after
    // the first second, so sequential calls cannot accumulate beyond two seconds.
    if started.elapsed() >= TIMEOUT { Err(error("media request budget exhausted")) } else { Ok(()) }
}
fn media(b: &Value) -> Result<Vec<MediaReply>, MediaError> {
    let started = Instant::now();
    // Validate all mutations before any connection or side effect.
    let action = b.get("action").map(|v| v.as_str().ok_or_else(|| error("invalid action"))).transpose()?;
    if action.is_some_and(|s| !["Play", "Pause", "PlayPause", "Next", "Previous", "Stop"].contains(&s)) { return Err(error("unsupported media action")); }
    let vol = b.get("setVolume").map(percent).transpose()?;
    let seek = b.get("Seek").map(integer).transpose()?;
    let position = b.get("SetPosition").map(integer).transpose()?.map(|n| {
        if n < 0 { Err(error("negative absolute position")) } else { milliseconds_to_microseconds(n) }
    }).transpose()?;
    if b.get("albumArtUrl").is_some() { return Err(error("album art payload is unsupported")); }
    if b.get("setLoopStatus").is_some() || b.get("setShuffle").is_some() { return Err(error("loop/shuffle control unsupported")); }
    let list = b.get("requestPlayerList") == Some(&Value::Bool(true));
    if list && (action.is_some() || vol.is_some() || seek.is_some() || position.is_some()) { return Err(error("player list request cannot mutate playback")); }
    if !list && !b.get("player").and_then(Value::as_str).is_some_and(valid_player) { return Err(error("invalid MPRIS player")); }
    let c = zbus::blocking::connection::Builder::session().map_err(error)?.method_timeout(TIMEOUT).build().map_err(error)?;
    let available = names(&c, &started)?;
    if list {
        return Ok(reply("kdeconnect.mpris", json!({"playerList":available,"supportAlbumArtPayload":false})));
    }
    let player = b.get("player").and_then(Value::as_str).filter(|p| valid_player(p)).ok_or_else(|| error("invalid MPRIS player"))?;
    if !available.iter().any(|p| p == player) { return Err(error("unknown MPRIS player")); }
    let p = proxy(&c, player)?;
    if let Some(v) = vol { check_budget(&started)?; p.set_property("Volume", v / 100.0).map_err(error)?; }
    // KDE Connect Seek is already microseconds; SetPosition/pos/length are milliseconds.
    if let Some(offset) = seek { check_budget(&started)?; p.call::<_, _, ()>("Seek", &(offset,)).map_err(error)?; }
    if let Some(position) = position {
        check_budget(&started)?;
        let metadata: HashMap<String, OwnedValue> = p.get_property("Metadata").map_err(error)?;
        let track = metadata.get("mpris:trackid").and_then(|v| <&ObjectPath<'_>>::try_from(v).ok())
            .ok_or_else(|| error("player has no track object path"))?;
        check_budget(&started)?;
        p.call::<_, _, ()>("SetPosition", &(track, position)).map_err(error)?;
    }
    if let Some(action) = action {
        check_budget(&started)?;
        let method = match action { "Play" => "Play", "Pause" => "Pause", "PlayPause" => "PlayPause", "Next" => "Next", "Previous" => "Previous", "Stop" => "Stop", _ => unreachable!("validated above") };
        p.call::<_, _, ()>(method, &()).map_err(error)?;
    }
    Ok(reply("kdeconnect.mpris", snapshot(player, &p, &started)?))
}
fn snapshot(player: &str, p: &Proxy<'_>, started: &Instant) -> Result<Value, MediaError> {
    check_budget(started)?;
    let properties = Proxy::new(p.connection(), p.destination().clone(), PATH, "org.freedesktop.DBus.Properties").map_err(error)?;
    let props: HashMap<String, OwnedValue> = properties.call("GetAll", &(MPRIS,)).map_err(error)?;
    let m: HashMap<String, OwnedValue> = props.get("Metadata").and_then(|v| v.try_clone().ok()).and_then(|v| HashMap::try_from(v).ok()).unwrap_or_default();
    let text = |key: &str| m.get(key).and_then(|v| <&str>::try_from(v).ok()).unwrap_or("").chars().take(2048).collect::<String>();
    let artists = m.get("xesam:artist").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default().into_iter().take(16).collect::<Vec<_>>().join(", ");
    let length = m.get("mpris:length").and_then(|v| i64::try_from(v).ok()).unwrap_or(0).max(0) / 1000;
    let status = props.get("PlaybackStatus").and_then(|v| <&str>::try_from(v).ok()).unwrap_or("");
    let position = props.get("Position").and_then(|v| i64::try_from(v).ok()).unwrap_or(0);
    let volume = props.get("Volume").and_then(|v| f64::try_from(v).ok()).filter(|v| v.is_finite());
    let can_control = props.get("CanControl").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
    let can = |name| can_control && props.get(name).and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
    Ok(json!({"player":player,"title":text("xesam:title"),"artist":artists.chars().take(2048).collect::<String>(),"album":text("xesam:album"),"nowPlaying":text("xesam:title"),"isPlaying":status == "Playing","pos":position.max(0)/1000,"length":length,"volume":volume.map(|v| (v*100.0).clamp(0.0,100.0).round() as i64).unwrap_or(-1),"canPlay":can("CanPlay"),"canPause":can("CanPause"),"canGoNext":can("CanGoNext"),"canGoPrevious":can("CanGoPrevious"),"canSeek":can("CanSeek"),"albumArtUrl":text("mpris:artUrl")}))
}
fn wpctl() -> Option<&'static str> { ["/usr/bin/wpctl", "/bin/wpctl"].into_iter().find(|p| std::path::Path::new(p).is_file()) }
/// Fixed executable/argv only, bounded stdout and wall time, kill/wait only our child.
fn run_wpctl(executable: &str, args: &[&str]) -> Result<String, MediaError> {
    let mut child = Command::new(executable).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(error)?;
    let mut stdout = child.stdout.take().ok_or_else(|| error("missing wpctl stdout"))?;
    let flags = match rustix::fs::fcntl_getfl(stdout.as_fd()) {
        Ok(flags) => flags,
        Err(e) => { let _ = child.kill(); let _ = child.wait(); return Err(error(e)); }
    };
    if let Err(e) = rustix::fs::fcntl_setfl(stdout.as_fd(), flags | rustix::fs::OFlags::NONBLOCK) {
        let _ = child.kill(); let _ = child.wait(); return Err(error(e));
    }
    let mut out = Vec::new();
    let start = Instant::now();
    let result = loop {
        let mut buf = [0u8; 512];
        match stdout.read(&mut buf) {
            Ok(n) => {
                if out.len() + n > 4096 { break Err(error("wpctl output too large")); }
                out.extend_from_slice(&buf[..n]);
                if n == 0 {
                    match child.try_wait() {
                        Ok(Some(status)) => break if status.success() { Ok(()) } else { Err(error("wpctl failed")) },
                        Ok(None) => {},
                        Err(e) => break Err(error(e)),
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {},
            Err(e) => break Err(error(e)),
        }
        if start.elapsed() >= TIMEOUT { break Err(error("wpctl timed out")); }
        std::thread::sleep(Duration::from_millis(5));
    };
    if result.is_err() { let _ = child.kill(); }
    let _ = child.wait();
    result?;
    String::from_utf8(out).map_err(error)
}
fn parse_sink(s: &str) -> Result<(f64, bool), MediaError> {
    if s.len() > 128 { return Err(error("oversized sink status")); }
    let mut words = s.split_whitespace();
    if words.next() != Some("Volume:") { return Err(error("unexpected sink status")); }
    let n: f64 = words.next().ok_or_else(|| error("missing sink volume"))?.parse().map_err(error)?;
    if !n.is_finite() || !(0.0..=1.0).contains(&n) { return Err(error("sink volume outside supported 0..100 range")); }
    let muted = match words.next() { None => false, Some("[MUTED]") => true, _ => return Err(error("unexpected sink mute status")) };
    if words.next().is_some() { return Err(error("unexpected sink status suffix")); }
    Ok(((n * 100.0).round(), muted))
}
fn system_volume(b: &Value) -> Result<Vec<MediaReply>, MediaError> {
    let started = Instant::now();
    let volume = b.get("volume").map(percent).transpose()?;
    let muted = b.get("muted").map(|v| v.as_bool().ok_or_else(|| error("invalid mute flag"))).transpose()?;
    if b.get("enabled").is_some() { return Err(error("output selection unsupported")); }
    let list = b.get("requestSinks") == Some(&Value::Bool(true));
    if !list && b.get("name").and_then(Value::as_str) != Some(SINK) { return Err(error("unknown system volume sink")); }
    if list && (volume.is_some() || muted.is_some()) { return Err(error("list request cannot mutate volume")); }
    let Some(executable) = wpctl() else {
        return if list { Ok(reply("kdeconnect.systemvolume", json!({"sinkList":[]}))) } else { Err(error("wpctl unavailable")) };
    };
    let run = |args: &[&str]| -> Result<String, MediaError> { check_budget(&started)?; run_wpctl(executable, args) };
    let current = run(&["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|s| parse_sink(&s));
    if list {
        let sinks = current.ok().map(|(volume, muted)| json!({"name":SINK,"description":"Default PipeWire output","volume":volume,"muted":muted,"maxVolume":100,"enabled":true})).into_iter().collect::<Vec<_>>();
        return Ok(reply("kdeconnect.systemvolume", json!({"sinkList":sinks})));
    }
    current?;
    if let Some(volume) = volume { run(&["set-volume", "--limit", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{volume}%")])?; }
    if let Some(muted) = muted { run(&["set-mute", "@DEFAULT_AUDIO_SINK@", if muted { "1" } else { "0" }])?; }
    let (volume, muted) = parse_sink(&run(&["get-volume", "@DEFAULT_AUDIO_SINK@"] )?)?;
    Ok(reply("kdeconnect.systemvolume", json!({"name":SINK,"volume":volume,"muted":muted,"enabled":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn operation_budget_reserves_last_timeout() {
        assert!(check_budget(&Instant::now()).is_ok());
        assert!(check_budget(&(Instant::now() - Duration::from_secs(1))).is_err());
    }
    #[test] fn constructor_does_not_connect() { let _adapter = MediaAdapter::default(); }
    #[test] fn subprocess_timeout_and_output_are_bounded() {
        if std::path::Path::new("/usr/bin/sleep").is_file() {
            let start = Instant::now();
            assert!(run_wpctl("/usr/bin/sleep", &["5"]).is_err());
            assert!(start.elapsed() < Duration::from_secs(3));
        }
        if std::path::Path::new("/usr/bin/yes").is_file() {
            assert!(run_wpctl("/usr/bin/yes", &[]).is_err());
        }
    }
    #[test] fn validates_destination_names() {
        assert!(valid_player("org.mpris.MediaPlayer2.des_music"));
        for n in ["org.freedesktop.DBus", "org.mpris.MediaPlayer2.", "org.mpris.MediaPlayer2.a/b", "org.mpris.MediaPlayer2.a;id", "org.mpris.MediaPlayer2.1bad"] { assert!(!valid_player(n)); }
    }
    #[test] fn volume_limits_and_types() {
        for v in [json!(-1), json!(101), json!("50"), Value::Null] { assert!(percent(&v).is_err()); }
        assert_eq!(percent(&json!(50.5)).unwrap(), 50.5);
    }
    #[test] fn seek_units_and_overflow() { assert_eq!(milliseconds_to_microseconds(1500).unwrap(), 1_500_000); assert!(milliseconds_to_microseconds(i64::MAX).is_err()); }
    #[test] fn sink_parser_is_strict() {
        assert_eq!(parse_sink("Volume: 0.42 [MUTED]\n").unwrap(), (42.0,true));
        assert_eq!(parse_sink("Volume: 1.00\n").unwrap(), (100.0,false));
        for s in ["Volume: NaN", "Volume: inf", "Volume: -0.1", "Volume: 1.5", "Volume: 0.5 garbage", "Volume: 0.5 [MUTED] extra", "junk"] { assert!(parse_sink(s).is_err()); }
        assert!(parse_sink(&"x".repeat(129)).is_err());
    }
    #[test] fn invalid_commands_never_access_bus_or_volume() {
        let mut a = MediaAdapter;
        for b in [json!({"action":"OpenUri"}),json!({"setVolume":101}),json!({"SetPosition":-1}),json!({"albumArtUrl":"file:///etc/passwd"})] { assert!(a.handle(MPRIS_REQUEST,&b).is_err()); }
        for b in [json!({"name":"other","volume":50}),json!({"name":SINK,"volume":101}),json!({"name":SINK,"muted":1}),json!({"name":SINK,"enabled":true})] { assert!(a.handle(SYSTEM_VOLUME_REQUEST,&b).is_err()); }
    }
    struct FakePlayer { volume: f64, seek: std::sync::Arc<std::sync::atomic::AtomicI64>, position: std::sync::Arc<std::sync::atomic::AtomicI64> }
    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        #[zbus(property)] fn metadata(&self) -> HashMap<String, OwnedValue> {
            HashMap::from([
                ("mpris:trackid".into(), OwnedValue::from(ObjectPath::try_from("/org/mpris/MediaPlayer2/track_1").unwrap())),
                ("mpris:length".into(), OwnedValue::from(2_500_000i64)),
                ("xesam:title".into(), OwnedValue::from(zbus::zvariant::Str::from("Fake track"))),
            ])
        }
        #[zbus(property)] fn position(&self) -> i64 { self.position.load(std::sync::atomic::Ordering::SeqCst) }
        fn set_position(&self, track: zbus::zvariant::OwnedObjectPath, position: i64) -> zbus::fdo::Result<()> {
            if track.as_str() != "/org/mpris/MediaPlayer2/track_1" { return Err(zbus::fdo::Error::InvalidArgs("wrong track".into())); }
            self.position.store(position, std::sync::atomic::Ordering::SeqCst); Ok(())
        }
        #[zbus(property)] fn volume(&self) -> f64 { self.volume }
        #[zbus(property)] fn set_volume(&mut self, value: f64) { self.volume = value; }
        fn seek(&self, offset: i64) { self.seek.store(offset, std::sync::atomic::Ordering::SeqCst); }
    }
    /// Run only in a disposable private bus:
    /// dbus-run-session -- env DES_CONNECT_FAKE_MPRIS=1 cargo test -p des-connect media::tests::fake_mpris -- --ignored
    #[test]
    #[ignore = "requires disposable dbus-run-session and DES_CONNECT_FAKE_MPRIS=1"]
    fn fake_mpris() {
        assert_eq!(std::env::var("DES_CONNECT_FAKE_MPRIS").as_deref(), Ok("1"));
        let seek = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
        let position = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
        let fake_name = "org.mpris.MediaPlayer2.des_connect_test";
        let _server = zbus::blocking::connection::Builder::session().unwrap()
            .name(fake_name).unwrap().serve_at(PATH, FakePlayer { volume: 0.5, seek: seek.clone(), position: position.clone() }).unwrap().build().unwrap();
        let mut adapter = MediaAdapter::default();
        let list = adapter.handle(MPRIS_REQUEST, &json!({"requestPlayerList":true})).unwrap();
        assert!(list[0].body["playerList"].as_array().unwrap().contains(&json!(fake_name)));
        assert_eq!(list[0].body["supportAlbumArtPayload"], false);
        let status = adapter.handle(MPRIS_REQUEST, &json!({"player":fake_name,"setVolume":35,"Seek":10000000})).unwrap();
        assert_eq!(status[0].body["volume"], 35);
        assert_eq!(seek.load(std::sync::atomic::Ordering::SeqCst), 10000000);
        let status = adapter.handle(MPRIS_REQUEST, &json!({"player":fake_name,"SetPosition":1500})).unwrap();
        assert_eq!(position.load(std::sync::atomic::Ordering::SeqCst), 1_500_000);
        assert_eq!(status[0].body["pos"], 1500);
        assert_eq!(status[0].body["length"], 2500);
        assert_eq!(status[0].body["title"], "Fake track");
        assert!(adapter.handle(MPRIS_REQUEST, &json!({"player":"org.mpris.MediaPlayer2.absent_des_test","action":"Play"})).is_err());
    }

}
