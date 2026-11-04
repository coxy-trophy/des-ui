//! MIT-licensed, independent KDE Connect media interoperability.
//! Caller must authenticate the peer and enforce separate media/system-volume opt-ins.
//! Construction performs no I/O; requests hold a temporary session bus connection.
use serde_json::{Value, json};
use std::{collections::HashMap, fmt, io::{self, Read, Write}, process::{Command, Stdio}, time::{Duration, Instant}, os::fd::AsFd};
use zbus::{blocking::{Connection, Proxy}, zvariant::{OwnedValue, ObjectPath}};

pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";
pub const SYSTEM_VOLUME_REQUEST: &str = "kdeconnect.systemvolume.request";
const MPRIS: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";
const DES_MUSIC: &str = "DES Music";
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
        self.handle_guarded(kind,body,||Ok(()))
    }
    /// The caller's live policy is checked before any I/O and immediately before
    /// every mutation, after lazy bus/socket/sink preparation has completed.
    pub fn handle_guarded(&mut self,kind:&str,body:&Value,mut guard:impl FnMut()->Result<(),MediaError>)->Result<Vec<MediaReply>,MediaError> {
        guard()?;
        if !body.is_object() { return Err(error("expected media object")); }
        match kind {
            MPRIS_REQUEST => media_with_music_path_guarded(body,&music_socket_path(),&mut guard),
            SYSTEM_VOLUME_REQUEST => system_volume_guarded(body,&mut guard),
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
    // Bind to the current unique owner. If the advertised player quits between
    // enumeration and dispatch, its well-known name must not auto-activate it.
    let dbus=zbus::blocking::fdo::DBusProxy::new(c).map_err(error)?;
    let owner=dbus.get_name_owner(name.try_into().map_err(error)?).map_err(error)?;
    zbus::blocking::proxy::Builder::new(c).destination(owner).map_err(error)?.path(PATH).map_err(error)?
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
#[cfg(test)]
fn media_with_music_path(b:&Value,music_path:&std::path::Path)->Result<Vec<MediaReply>,MediaError> {
    media_with_music_path_guarded(b,music_path,&mut ||Ok(()))
}
fn media_with_music_path_guarded(b:&Value,music_path:&std::path::Path,guard:&mut dyn FnMut()->Result<(),MediaError>)->Result<Vec<MediaReply>,MediaError> {
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
    if b.get("player").and_then(Value::as_str) == Some(DES_MUSIC) && !list {
        return Ok(reply("kdeconnect.mpris", des_music_request_guarded(music_path,b,&started,guard)?));
    }
    if !list && !b.get("player").and_then(Value::as_str).is_some_and(valid_player) { return Err(error("invalid MPRIS player")); }
    // DES Music discovery only reads the service's initial state; it never starts it.
    let local_music = list && DesMusic::open(music_path, &started).is_ok();
    let connection = zbus::blocking::connection::Builder::session().map_err(error)
        .and_then(|builder| builder.method_timeout(TIMEOUT).build().map_err(error));
    if list {
        let mut available = connection.as_ref().ok().and_then(|c| names(c, &started).ok()).unwrap_or_default();
        if local_music { available.truncate(15); available.insert(0, DES_MUSIC.to_string()); }
        return Ok(reply("kdeconnect.mpris", json!({"playerList":available,"supportAlbumArtPayload":false})));
    }
    let c = connection?;
    let available = names(&c, &started)?;
    let player = b.get("player").and_then(Value::as_str).filter(|p| valid_player(p)).ok_or_else(|| error("invalid MPRIS player"))?;
    if !available.iter().any(|p| p == player) { return Err(error("unknown MPRIS player")); }
    check_budget(&started)?;let p = proxy(&c, player)?;
    if let Some(v) = vol { guard()?; check_budget(&started)?; p.set_property("Volume", v / 100.0).map_err(error)?; }
    // KDE Connect Seek is already microseconds; SetPosition/pos/length are milliseconds.
    if let Some(offset) = seek { guard()?; check_budget(&started)?; p.call::<_, _, ()>("Seek", &(offset,)).map_err(error)?; }
    if let Some(position) = position {
        check_budget(&started)?;
        let metadata: HashMap<String, OwnedValue> = p.get_property("Metadata").map_err(error)?;
        let track = metadata.get("mpris:trackid").and_then(|v| <&ObjectPath<'_>>::try_from(v).ok())
            .ok_or_else(|| error("player has no track object path"))?;
        guard()?;check_budget(&started)?;
        p.call::<_, _, ()>("SetPosition", &(track, position)).map_err(error)?;
    }
    if let Some(action) = action {
        check_budget(&started)?;
        let method = match action { "Play" => "Play", "Pause" => "Pause", "PlayPause" => "PlayPause", "Next" => "Next", "Previous" => "Previous", "Stop" => "Stop", _ => unreachable!("validated above") };
        guard()?;check_budget(&started)?;p.call::<_, _, ()>(method, &()).map_err(error)?;
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
// Independently follows des_common::music's local socket-address contract.
fn music_socket_path() -> std::path::PathBuf {
    std::env::var_os("DES_MUSIC_SOCKET").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join(format!("des-music-{}", rustix::process::geteuid().as_raw())))
            .join("des-music.sock")
    })
}
const MUSIC_LINE_LIMIT: usize = 64 * 1024;
const MUSIC_TIMEOUT: Duration = Duration::from_millis(500);
struct DesMusic {
    stream: std::os::unix::net::UnixStream,
    started: Instant,
    buffered: Vec<u8>,
    state: Value,
}
impl DesMusic {
    fn open(path: &std::path::Path, request_started: &Instant) -> Result<Self, MediaError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        use rustix::net::{socket_with, connect, AddressFamily, SocketType, SocketFlags, SocketAddrUnix};
        check_budget(request_started)?;
        if !path.is_absolute() { return Err(error("music socket requires an absolute path")); }
        let parent = path.parent().ok_or_else(|| error("music socket has no parent"))?;
        let directory = std::fs::symlink_metadata(parent).map_err(error)?;
        let metadata = std::fs::symlink_metadata(path).map_err(error)?;
        let uid = rustix::process::geteuid();
        if !directory.is_dir() || directory.uid() != uid.as_raw() || directory.mode() & 0o077 != 0
            || !metadata.file_type().is_socket() || metadata.uid() != uid.as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(error("music socket and directory must be private and owned by this user"));
        }
        let started = Instant::now();
        let fd = socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK, None).map_err(error)?;
        // A full UNIX accept backlog fails immediately instead of blocking the actor.
        connect(&fd, &SocketAddrUnix::new(path).map_err(error)?).map_err(error)?;
        let stream = std::os::unix::net::UnixStream::from(fd);
        if rustix::net::sockopt::socket_peercred(&stream).map_err(error)?.uid != uid { return Err(error("wrong music service owner")); }
        let mut session = Self { stream, started, buffered: Vec::new(), state: Value::Null };
        session.state = session.read_state()?;
        Ok(session)
    }
    fn check_deadline(&self) -> Result<(), MediaError> {
        if self.started.elapsed() >= MUSIC_TIMEOUT { Err(error("music socket timed out")) } else { Ok(()) }
    }
    fn read_state(&mut self) -> Result<Value, MediaError> {
        loop {
            self.check_deadline()?;
            if let Some(end) = self.buffered.iter().position(|b| *b == b'\n') {
                let state: Value = serde_json::from_slice(&self.buffered[..end]).map_err(error)?;
                self.buffered.drain(..=end);
                validate_music_state(&state)?;
                return Ok(state);
            }
            if self.buffered.len() >= MUSIC_LINE_LIMIT { return Err(error("music state exceeds 64KiB")); }
            let mut bytes = [0u8; 1024];
            let remaining = (MUSIC_LINE_LIMIT - self.buffered.len()).min(bytes.len());
            match self.stream.read(&mut bytes[..remaining]) {
                Ok(0) => return Err(error("music service disconnected")),
                Ok(n) => self.buffered.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(error(e)),
            }
        }
    }
    fn send(&mut self,command:Value)->Result<(),MediaError>{self.send_guarded(command,&mut ||Ok(()))}
    fn send_guarded(&mut self, command: Value,guard:&mut dyn FnMut()->Result<(),MediaError>) -> Result<(), MediaError> {
        let mut bytes = serde_json::to_vec(&command).map_err(error)?; bytes.push(b'\n');
        let mut offset = 0;
        while offset < bytes.len() {
            self.check_deadline()?;guard()?;
            match self.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(error("music service disconnected")),
                Ok(n) => offset += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(error(e)),
            }
        }
        Ok(())
    }
}
fn validate_music_state(s: &Value) -> Result<(), MediaError> {
    let number = |v: &Value| v.as_u64().is_some_and(|n| n <= u32::MAX as u64);
    if s.get("track").is_none() || s["type"] != "state" || !s["playing"].is_boolean() || !number(&s["position_ms"])
        || !s["volume"].as_u64().is_some_and(|v| v <= 100) { return Err(error("invalid music state")); }
    let track = &s["track"];
    if !track.is_null() && (!track.is_object() || !number(&track["duration_ms"])
        || ["name", "artists", "album"].iter().any(|key| !track[*key].is_string())
        || (!track["cover"].is_null() && !track["cover"].is_string())) { return Err(error("invalid music track")); }
    Ok(())
}
fn music_snapshot(state: &Value) -> Value {
    let track = &state["track"];
    let text = |key: &str| track[key].as_str().unwrap_or("").chars().take(2048).collect::<String>();
    let has_track = track.is_object();
    json!({"player":DES_MUSIC,"title":text("name"),"artist":text("artists"),"album":text("album"),"nowPlaying":text("name"),
        "isPlaying":state["playing"],"pos":state["position_ms"],"length":track["duration_ms"].as_u64().unwrap_or(0),
        "volume":state["volume"],"canPlay":has_track,"canPause":has_track,"canGoNext":has_track,"canGoPrevious":has_track,
        "canSeek":has_track && track["duration_ms"].as_u64().unwrap_or(0)>0,"albumArtUrl":text("cover")})
}
#[cfg(test)]
fn des_music_request(path:&std::path::Path,b:&Value,started:&Instant)->Result<Value,MediaError> {
    des_music_request_guarded(path,b,started,&mut ||Ok(()))
}
fn des_music_request_guarded(path:&std::path::Path,b:&Value,started:&Instant,guard:&mut dyn FnMut()->Result<(),MediaError>)->Result<Value,MediaError> {
    let action = b.get("action").map(|v| v.as_str().ok_or_else(|| error("invalid music action"))).transpose()?;
    if action.is_some_and(|a| !["Play", "Pause", "PlayPause", "Next", "Previous"].contains(&a)) { return Err(error("DES Music does not support this action")); }
    let volume = b.get("setVolume").map(percent).transpose()?;
    let absolute = b.get("SetPosition").map(integer).transpose()?;
    if absolute.is_some_and(|n| n < 0 || n > u32::MAX as i64) { return Err(error("music position outside supported range")); }
    let seek = b.get("Seek").map(integer).transpose()?;
    if absolute.is_some() && seek.is_some() { return Err(error("ambiguous music seek")); }
    let mut music = DesMusic::open(path, started)?;
    let state = &music.state;
    let duration = state["track"]["duration_ms"].as_u64().unwrap_or(0) as i64;
    let target = if let Some(n) = absolute { Some(n) } else if let Some(offset) = seek {
        Some(state["position_ms"].as_i64().unwrap_or(0).checked_add(offset / 1000).ok_or_else(|| error("music seek overflow"))?)
    } else { None };
    if target.is_some_and(|n| n < 0 || n > u32::MAX as i64 || n > duration) { return Err(error("music seek outside current track")); }
    if (action.is_some() || target.is_some()) && state["track"].is_null() { return Err(error("DES Music has no current track")); }
    let command = match action {
        Some("Play") if state["playing"] == false => Some(json!({"cmd":"toggle"})),
        Some("Pause") if state["playing"] == true => Some(json!({"cmd":"toggle"})),
        Some("PlayPause") => Some(json!({"cmd":"toggle"})),
        Some("Next") => Some(json!({"cmd":"next"})),
        Some("Previous") => Some(json!({"cmd":"prev"})),
        _ => None,
    };
    // Android sends one control per packet. Reject combinations before any write,
    // avoiding ambiguous asynchronous state broadcasts between distinct commands.
    let controls = usize::from(volume.is_some()) + usize::from(target.is_some()) + usize::from(action.is_some());
    if controls > 1 { return Err(error("DES Music accepts one control per request")); }
    let changed = command.is_some() || target.is_some() || volume.is_some();
    if let Some(command) = command { music.send_guarded(command,guard)?; }
    if let Some(ms) = target { music.send_guarded(json!({"cmd":"seek","ms":ms}),guard)?; }
    if let Some(percent) = volume { music.send_guarded(json!({"cmd":"volume","percent":percent.round() as u64}),guard)?; }
    if changed {
        music.send(json!({"cmd":"state"}))?;
        music.state = music.read_state()?;
    }
    Ok(music_snapshot(&music.state))
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
fn system_volume_guarded(b:&Value,guard:&mut dyn FnMut()->Result<(),MediaError>)->Result<Vec<MediaReply>,MediaError> {
    system_volume_with_executable(b,wpctl(),guard)
}
fn system_volume_with_executable(b:&Value,executable:Option<&str>,guard:&mut dyn FnMut()->Result<(),MediaError>)->Result<Vec<MediaReply>,MediaError> {
    let started = Instant::now();
    let volume = b.get("volume").map(percent).transpose()?;
    let muted = b.get("muted").map(|v| v.as_bool().ok_or_else(|| error("invalid mute flag"))).transpose()?;
    if b.get("enabled").is_some() { return Err(error("output selection unsupported")); }
    let list = b.get("requestSinks") == Some(&Value::Bool(true));
    if !list && b.get("name").and_then(Value::as_str) != Some(SINK) { return Err(error("unknown system volume sink")); }
    if list && (volume.is_some() || muted.is_some()) { return Err(error("list request cannot mutate volume")); }
    let Some(executable) = executable else {
        return if list { Ok(reply("kdeconnect.systemvolume", json!({"sinkList":[]}))) } else { Err(error("wpctl unavailable")) };
    };
    let run = |args: &[&str]| -> Result<String, MediaError> { check_budget(&started)?; run_wpctl(executable, args) };
    let current = run(&["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|s| parse_sink(&s));
    if list {
        let sinks = current.ok().map(|(volume, muted)| json!({"name":SINK,"description":"Default PipeWire output","volume":volume,"muted":muted,"maxVolume":100,"enabled":true})).into_iter().collect::<Vec<_>>();
        return Ok(reply("kdeconnect.systemvolume", json!({"sinkList":sinks})));
    }
    current?;
    if let Some(volume) = volume { guard()?;run(&["set-volume", "--limit", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{volume}%")])?; }
    if let Some(muted) = muted { guard()?;run(&["set-mute", "@DEFAULT_AUDIO_SINK@", if muted { "1" } else { "0" }])?; }
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
        let list = media_with_music_path(&json!({"requestPlayerList":true}),std::path::Path::new("/nonexistent/des-connect-no-music.sock")).unwrap();
        assert!(list[0].body["playerList"].as_array().unwrap().contains(&json!(fake_name)));
        assert_eq!(list[0].body["supportAlbumArtPayload"], false);
        assert!(!list[0].body["playerList"].as_array().unwrap().contains(&json!(DES_MUSIC)));
        let (music_path, music_worker) = music_fixture();
        let with_music = media_with_music_path(&json!({"requestPlayerList":true}),&music_path).unwrap();
        assert!(with_music[0].body["playerList"].as_array().unwrap().contains(&json!(DES_MUSIC)));
        assert!(music_worker.join().unwrap().is_empty());

        let status = adapter.handle(MPRIS_REQUEST, &json!({"player":fake_name,"setVolume":35,"Seek":10000000})).unwrap();
        assert_eq!(status[0].body["volume"], 35);
        assert_eq!(seek.load(std::sync::atomic::Ordering::SeqCst), 10000000);
        let status = adapter.handle(MPRIS_REQUEST, &json!({"player":fake_name,"SetPosition":1500})).unwrap();
        assert_eq!(position.load(std::sync::atomic::Ordering::SeqCst), 1_500_000);
        assert_eq!(status[0].body["pos"], 1500);
        assert_eq!(status[0].body["length"], 2500);
        assert_eq!(status[0].body["title"], "Fake track");
        assert!(adapter.handle(MPRIS_REQUEST, &json!({"player":"org.mpris.MediaPlayer2.absent_des_test","action":"Play"})).is_err());
        let calls=std::cell::Cell::new(0);let guarded=adapter.handle_guarded(MPRIS_REQUEST,&json!({"player":fake_name,"setVolume":77}),||{calls.set(calls.get()+1);if calls.get()>1{Err(error("revoked after bus preparation"))}else{Ok(())}});
        assert!(guarded.is_err());let connection=Connection::session().unwrap();let player=proxy(&connection,fake_name).unwrap();assert!(player.destination().as_str().starts_with(':'));assert_eq!(player.get_property::<f64>("Volume").unwrap(),0.35);

    }

    fn music_state() -> Value { json!({"type":"state","track":{"name":"Socket song","artists":"Socket artist","album":"Socket album","duration_ms":5000,"cover":"https://example.invalid/known-cover"},"playing":true,"position_ms":2000,"volume":40,"device":null}) }
    fn music_fixture() -> (std::path::PathBuf, std::thread::JoinHandle<Vec<Value>>) {
        use std::os::unix::{net::UnixListener, fs::PermissionsExt};
        use std::io::{BufRead, BufReader, Write};
        let dir = std::env::temp_dir().join(format!("des-connect-fake-music-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&dir).unwrap(); std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700)).unwrap();
        let path=dir.join("music.sock"); let listener=UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker=std::thread::spawn(move || {
            let started=Instant::now();
            let (mut stream,_)=loop {
                match listener.accept() {
                    Ok(pair)=>break pair,
                    Err(e) if e.kind()==io::ErrorKind::WouldBlock && started.elapsed()<Duration::from_secs(2)=>std::thread::sleep(Duration::from_millis(2)),
                    Err(e)=>panic!("fake music accept: {e}"),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut state=music_state(); writeln!(stream,"{state}").unwrap();
            let mut reader=BufReader::new(stream.try_clone().unwrap()); let mut commands=Vec::new();
            loop {
                let mut line=String::new(); if !matches!(reader.read_line(&mut line),Ok(n) if n>0) { break; }
                let command:Value=serde_json::from_str(&line).unwrap(); commands.push(command.clone());
                match command["cmd"].as_str().unwrap() {
                    "toggle"=>state["playing"]=json!(!state["playing"].as_bool().unwrap()),
                    "seek"=>state["position_ms"]=command["ms"].clone(),
                    "volume"=>state["volume"]=command["percent"].clone(),
                    "state"=>if writeln!(stream,"{state}").is_err(){break},
                    "next"|"prev"=>{},
                    _=>panic!("unexpected command"),
                }
            }
            std::fs::remove_dir_all(dir).unwrap(); commands
        }); (path,worker)
    }
    #[test] fn des_music_maps_state_and_checked_commands() {
        for (body,expected) in [
            (json!({"player":"DES Music","action":"Pause"}),json!({"cmd":"toggle"})),
            (json!({"player":"DES Music","action":"Next"}),json!({"cmd":"next"})),
            (json!({"player":"DES Music","Seek":-1000000}),json!({"cmd":"seek","ms":1000})),
            (json!({"player":"DES Music","SetPosition":3000}),json!({"cmd":"seek","ms":3000})),
            (json!({"player":"DES Music","setVolume":25}),json!({"cmd":"volume","percent":25})),
        ] {
            let (path,worker)=music_fixture();
            let snapshot=des_music_request(&path,&body,&Instant::now()).unwrap();
            assert_eq!(snapshot["player"],"DES Music"); assert_eq!(snapshot["title"],"Socket song");
            assert_eq!(snapshot["length"],5000); assert_eq!(snapshot["albumArtUrl"],"https://example.invalid/known-cover");
            let commands=worker.join().unwrap(); assert_eq!(commands,vec![expected,json!({"cmd":"state"})]);
        }
    }
    #[test] fn des_music_idempotent_play_and_no_stop() {
        let (path,worker)=music_fixture();
        let snapshot=des_music_request(&path,&json!({"player":"DES Music","action":"Play"}),&Instant::now()).unwrap();
        assert_eq!(snapshot["isPlaying"],true); assert!(worker.join().unwrap().is_empty());
        let missing=std::path::Path::new("/nonexistent/des-music.sock");
        for body in [json!({"action":"Stop"}),json!({"setVolume":101}),json!({"SetPosition":-1})] { assert!(des_music_request(missing,&body,&Instant::now()).is_err()); }
    }

    #[test] fn des_music_rejects_out_of_track_and_combined_controls_without_writes() {
        assert!(des_music_request(std::path::Path::new("/nonexistent/des-music.sock"),&json!({"Seek":1000,"SetPosition":10}),&Instant::now()).is_err());
        for body in [json!({"Seek":-3000000}),json!({"SetPosition":5001}),json!({"action":"Pause","setVolume":20})] {
            let (path,worker)=music_fixture();
            assert!(des_music_request(&path,&body,&Instant::now()).is_err());
            assert!(worker.join().unwrap().is_empty());
        }
    }
    #[test] fn des_music_bounds_and_validates_socket_state() {
        use std::os::unix::{net::UnixListener,fs::PermissionsExt};
        for response in [Some(b"{bad json}\n".to_vec()),Some(vec![b'x';MUSIC_LINE_LIMIT]),None] {
            let dir=std::env::temp_dir().join(format!("des-connect-fault-music-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            std::fs::create_dir(&dir).unwrap();std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700)).unwrap();
            let path=dir.join("music.sock");let listener=UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
            let worker=std::thread::spawn(move||{
                let(mut stream,_)=listener.accept().unwrap();
                if let Some(bytes)=response {let _=stream.write_all(&bytes);} else {std::thread::sleep(Duration::from_millis(650));}
            });
            let start=Instant::now();assert!(DesMusic::open(&path,&start).is_err());
            assert!(start.elapsed()<Duration::from_secs(1));worker.join().unwrap();
            // A world-readable endpoint is rejected before attempting to connect.
            std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(DesMusic::open(&path,&Instant::now()).is_err());
            std::fs::remove_dir_all(dir).unwrap();
        }
        let mut state=music_state();state["volume"]=json!(101);assert!(validate_music_state(&state).is_err());
        state=music_state();state["position_ms"]=json!(-1);assert!(validate_music_state(&state).is_err());
        state=music_state();state["track"]["cover"]=json!({"url":"file:///etc/passwd"});assert!(validate_music_state(&state).is_err());
    }

    #[test] fn delayed_music_state_rechecks_guard_before_any_control_write(){
        use std::os::unix::{net::UnixListener,fs::PermissionsExt};
        use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
        let dir=tempfile::tempdir().unwrap();std::fs::set_permissions(dir.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=dir.path().join("music.sock");let listener=UnixListener::bind(&path).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        let allowed=Arc::new(AtomicBool::new(true));let revoked=allowed.clone();
        let worker=std::thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();std::thread::sleep(Duration::from_millis(50));revoked.store(false,Ordering::Release);writeln!(stream,"{}",music_state()).unwrap();stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();let mut received=[0;1024];stream.read(&mut received).unwrap_or(0)});
        let mut guard=||if allowed.load(Ordering::Acquire){Ok(())}else{Err(error("revoked during preparation"))};
        assert!(media_with_music_path_guarded(&json!({"player":DES_MUSIC,"action":"Pause"}),&path,&mut guard).is_err());assert_eq!(worker.join().unwrap(),0);
    }
    #[test] fn volume_guard_rechecks_after_dummy_sink_read_before_mutation_exec(){
        use std::os::unix::fs::PermissionsExt;
        let dir=tempfile::tempdir().unwrap();let tool=dir.path().join("wpctl");let marker=dir.path().join("revoked");let changed=dir.path().join("changed");
        let script=format!("#!/bin/sh\nif [ \"$1\" = get-volume ]; then /usr/bin/touch '{}'; printf 'Volume: 0.40\\n'; else /usr/bin/touch '{}'; fi\n",marker.display(),changed.display());std::fs::write(&tool,script).unwrap();std::fs::set_permissions(&tool,std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut guard=||if marker.exists(){Err(error("revoked during sink preparation"))}else{Ok(())};
        assert!(system_volume_with_executable(&json!({"name":SINK,"volume":55}),Some(tool.to_str().unwrap()),&mut guard).is_err());assert!(marker.exists());assert!(!changed.exists());
    }

}
