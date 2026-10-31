//! The music player: what is playing in the right column, and a full view of the
//! user's Spotify library in the workspace.
//!
//! The playing is done by `des-music`, a service of its own; this is its remote
//! control. The shell starts the service when it is not running, and offers the
//! sign-in when the service says nobody is signed in.

use crate::theme::Theme;
use crate::ui;
use des_common::music::socket_path;
use eframe::egui::{
    self, Align, Align2, Key, Layout, Modifiers, Rect, Response, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, TextEdit, TextureHandle, Ui, UiBuilder, pos2, vec2,
};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u32,
    pub cover: Option<String>,
}

/// What the service last said is playing, and when it said so.
#[derive(Debug, Clone, Default)]
pub struct Now {
    pub track: Option<Track>,
    pub playing: bool,
    position_ms: u32,
    pub volume: u8,
    pub shuffle: bool,
    /// `off`, `all` or `one`.
    pub repeat: String,
    /// The device it is playing on, when that is not this one.
    pub device: Option<String>,
    /// Whether the track is among the liked songs, once known.
    pub liked: Option<bool>,
    at: Option<Instant>,
}

impl Now {
    /// Where in the track playback is at this moment.
    pub fn position_ms(&self) -> u32 {
        let length = self.track.as_ref().map_or(0, |track| track.duration_ms);
        let moved = match (self.playing, self.at) {
            (true, Some(at)) => at.elapsed().as_millis() as u32,
            _ => 0,
        };
        self.position_ms.saturating_add(moved).min(length)
    }

    pub fn parse(value: &Value) -> Now {
        let track = &value["track"];
        Now {
            track: track["uri"].as_str().map(|uri| Track {
                uri: uri.to_string(),
                name: track["name"].as_str().unwrap_or_default().to_string(),
                artists: track["artists"].as_str().unwrap_or_default().to_string(),
                album: track["album"].as_str().unwrap_or_default().to_string(),
                duration_ms: track["duration_ms"].as_u64().unwrap_or(0) as u32,
                cover: track["cover"].as_str().map(str::to_string),
            }),
            playing: value["playing"].as_bool().unwrap_or(false),
            position_ms: value["position_ms"].as_u64().unwrap_or(0) as u32,
            volume: value["volume"].as_u64().unwrap_or(0).min(100) as u8,
            shuffle: value["shuffle"].as_bool().unwrap_or(false),
            repeat: value["repeat"].as_str().unwrap_or("off").to_string(),
            device: value["device"].as_str().map(str::to_string),
            liked: value["liked"].as_bool(),
            at: Some(Instant::now()),
        }
    }
}

/// What a row in a list is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Track,
    Playlist,
    Album,
    Artist,
    /// Something that can play: this machine, a phone, a speaker.
    Device,
    Show,
    Episode,
    Audiobook,
    Chapter,
}

/// A row in a list: a track, or a playlist, album or artist to open.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub uri: String,
    /// The id of a playlist, album or artist, for asking what is in it.
    pub id: String,
    pub name: String,
    /// A track's or album's artists, or a playlist's owner.
    pub by: String,
    /// The album a track is on, or the year an album came out.
    pub album: String,
    pub duration_ms: u32,
    /// For a track: the ids of its album and of its first artist, for going to them.
    pub album_id: String,
    pub artist_id: String,
    /// The address of its picture.
    pub cover: Option<String>,
    /// For a device: whether it is the one playing, and whether it is this machine.
    pub active: bool,
    pub here: bool,
    pub position: Option<u32>,
    pub editable: bool,
    pub external_url: String,
}

pub fn parse_items(value: &Value) -> Vec<Item> {
    value["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = match item["kind"].as_str() {
                        Some("playlist") => Kind::Playlist,
                        Some("album") => Kind::Album,
                        Some("artist") => Kind::Artist,
                        Some("device") => Kind::Device,
                        Some("show") => Kind::Show,
                        Some("episode") => Kind::Episode,
                        Some("audiobook") => Kind::Audiobook,
                        Some("chapter") => Kind::Chapter,
                        _ => Kind::Track,
                    };
                    let text = |key: &str| item[key].as_str().unwrap_or_default().to_string();
                    Some(Item {
                        kind,
                        uri: item["uri"].as_str()?.to_string(),
                        id: text("id"),
                        name: text("name"),
                        by: if kind == Kind::Playlist { text("owner") } else { text("artists") },
                        album: text("album"),
                        duration_ms: item["duration_ms"].as_u64().unwrap_or(0) as u32,
                        album_id: text("album_id"),
                        artist_id: text("artist_id"),
                        cover: item["cover"].as_str().map(str::to_string),
                        active: item["active"] == true,
                        here: item["here"] == true,
                        position: item["position"].as_u64().map(|n|n as u32),
                        editable: item["editable"]==true,
                        external_url: text("external_url"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A track's words, as the service gave them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lyrics {
    /// The track they belong to.
    pub track: String,
    /// Whether each line carries the moment it is sung.
    pub synced: bool,
    /// Each line, with that moment in milliseconds.
    pub lines: Vec<(u32, String)>,
    /// Why there are none, when there are none.
    pub error: Option<String>,
}

impl Lyrics {
    pub fn parse(value: &Value) -> Lyrics {
        Lyrics {
            track: value["track"].as_str().unwrap_or_default().to_string(),
            synced: value["synced"] == true,
            lines: value["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|line| (line["ms"].as_u64().unwrap_or(0) as u32, line["text"].as_str().unwrap_or_default().to_string()))
                .collect(),
            error: value["error"].as_str().map(str::to_string),
        }
    }

    /// The line being sung `position` milliseconds in: the last one that has begun.
    pub fn current(&self, position: u32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        self.lines.iter().rposition(|(at, _)| *at <= position)
    }
}

/// Minutes and seconds, as on a track listing.
pub fn clock(ms: u32) -> String {
    format!("{}:{:02}", ms / 60_000, (ms / 1000) % 60)
}

#[derive(Debug, Clone, PartialEq)]
enum Link {
    Starting,
    Connected,
    NotSignedIn,
    SigningIn,
    /// The service cannot be run at all, and why.
    Unavailable(String),
}

struct Shared {
    link: Link,
    now: Now,
    /// Lists received, by `kind:key`, each with the reason it is empty if it failed.
    lists: HashMap<String, (Vec<Item>, Option<String>)>,
    /// The lists, by the same names, that have more after what was received.
    more: std::collections::HashSet<String>,
    next: HashMap<String, Value>,
    snapshots: HashMap<String,String>,
    edits: HashMap<String,Result<(),String>>,
    saved: HashMap<String, bool>,
    details: HashMap<String, Value>,
    /// The words last received.
    lyrics: Option<Lyrics>,
    /// What last went wrong with something asked of the service, and when.
    problem: Option<(String, Instant)>,
    /// Set to ask the connection thread to run a sign-in: the service's arguments.
    sign_in: Option<Vec<String>>,
    /// Set when a sign-in went through, so that lists are asked for afresh.
    refresh: bool,
    /// Set to call off a sign-in that is waiting on the browser.
    cancel: bool,
}

impl Shared {
    fn receive_edit(&mut self, value: &Value) {
        if let Some(operation)=value["operation"].as_str() {
            let result=if value["type"]=="changed"{Ok(())}else{Err(value["text"].as_str().unwrap_or("Spotify did not confirm this edit.").to_string())};
            if self.edits.len()>=64{self.edits.clear();}
            self.edits.insert(operation.to_string(),result);
        }
    }
    fn invalidate(&mut self) {
        self.lists.clear(); self.more.clear(); self.next.clear(); self.snapshots.clear();
        self.saved.clear(); self.details.clear(); self.refresh=true;
    }

    fn receive_list(&mut self, value: &Value) {
                        let key = format!("{}:{}", value["kind"].as_str().unwrap_or_default(), value["key"].as_str().unwrap_or_default());
                        let error = value["error"].as_str().map(str::to_string);
                        let offset = value["offset"].as_u64().unwrap_or(0) as usize;
                        let expected=self.next.get(&key).and_then(|v|v["offset"].as_u64()).unwrap_or(0) as usize;
                        if offset>0 && offset!=expected {return;}
                        if let Some(snapshot)=value["snapshot"].as_str() {self.snapshots.insert(key.clone(),snapshot.to_string());}
                        else if offset==0 {self.snapshots.remove(&key);}
                        if value["more"] == true {
                            self.more.insert(key.clone());
                            self.next.insert(key.clone(),json!({"offset":value["next_offset"].as_u64().unwrap_or(offset as u64+value["items"].as_array().map_or(0,Vec::len) as u64),"cursor":value["cursor"]}));
                        } else {
                            self.more.remove(&key);
                            self.next.remove(&key);
                        }
                        match self.lists.get_mut(&key) {
                            // A further part goes after what is there, if it is the part that follows.
                            Some((items, why)) if offset > 0 => {
                                if error.is_none() { items.extend(parse_items(value)); }
                                else { *why=error; }
                            }
                            _ if offset > 0 => {}
                            _ => {
                                self.lists.insert(key, (parse_items(value), error));
                            }
                        }
    }
}

/// The service's program: the one installed beside this one.
fn service_program() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("des-music")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("des-music"))
}

/// Keeps a connection to the service for as long as the shell runs: starts the
/// service when it is not there, reads what it says, and passes on what `commands`
/// brings.
fn connection(shared: Arc<Mutex<Shared>>, commands: Receiver<String>, ctx: egui::Context) {
    let set = |link: Link| {
        shared.lock().unwrap().link = link;
        ctx.request_repaint();
    };
    loop {
        let wants_sign_in = shared.lock().unwrap().sign_in.take();
        if let Some(arguments) = wants_sign_in {
            set(Link::SigningIn);
            // The sign-in waits on the browser for as long as it takes, so it can be
            // called off from here.
            let mut signed_in = false;
            if let Ok(mut child) = Command::new(service_program()).args(&arguments).stdin(Stdio::null()).spawn() {
                signed_in = loop {
                    match child.try_wait() {
                        Ok(None) if std::mem::take(&mut shared.lock().unwrap().cancel) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            break false;
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                        Ok(Some(status)) => break status.success(),
                        Err(_) => break false,
                    }
                };
            }
            if signed_in {
                let mut shared = shared.lock().unwrap();
                shared.invalidate();
            }
            // Connecting the library fails without undoing the sign-in itself.
            set(if signed_in || arguments[0] != "login" { Link::Starting } else { Link::NotSignedIn });
            continue;
        }
        if shared.lock().unwrap().link == Link::NotSignedIn {
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }

        let Ok(stream) = UnixStream::connect(socket_path()) else {
            // Not running: start it, in a group of its own so that it outlives the shell.
            let spawned = Command::new(service_program())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn();
            match spawned {
                Err(e) => {
                    set(Link::Unavailable(format!("The music service could not be started: {e}")));
                    std::thread::sleep(Duration::from_secs(10));
                }
                Ok(mut child) => {
                    // It says at once if nobody is signed in; otherwise it takes a
                    // while to reach Spotify and start listening. Wait for whichever
                    // comes, rather than starting a second one beside it.
                    let started = Instant::now();
                    while started.elapsed() < Duration::from_secs(30)
                        && matches!(child.try_wait(), Ok(None))
                        && UnixStream::connect(socket_path()).is_err()
                    {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    match child.try_wait() {
                        Ok(Some(status)) if status.code() == Some(3) => set(Link::NotSignedIn),
                        Ok(Some(_)) => {
                            set(Link::Unavailable("The music service stopped as soon as it started. Is this machine online?".to_string()));
                            std::thread::sleep(Duration::from_secs(10));
                            set(Link::Starting);
                        }
                        _ => {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                    }
                }
            }
            continue;
        };

        set(Link::Connected);
        let Ok(mut writer) = stream.try_clone() else {
            continue;
        };
        if writer.set_write_timeout(Some(Duration::from_secs(5))).is_err() { continue; }
        let reading = shared.clone();
        let repaint = ctx.clone();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let mut shared = reading.lock().unwrap();
                shared.receive_edit(&value);
                match value["type"].as_str() {
                    Some("state") => shared.now = Now::parse(&value),
                    Some("lyrics") => shared.lyrics = Some(Lyrics::parse(&value)),
                    Some("problem") => {
                        shared.problem = value["text"].as_str().map(|text| (text.to_string(), Instant::now()));
                    }
                    Some("changed") => {
                        shared.invalidate();
                    }
                    Some("saved") => {if let (Some(uri),Some(saved))=(value["uri"].as_str(),value["saved"].as_bool()){shared.saved.insert(uri.to_string(),saved);}}
                    Some("playlist_details") => {if let Some(id)=value["id"].as_str(){shared.details.insert(id.to_string(),value.clone());}}
                    Some("list") => {
                        shared.receive_list(&value);
                    }
                    _ => {}
                }
                drop(shared);
                repaint.request_repaint();
            }
        });
        // Keep an idle subscription within the service's request deadline.
        let mut heartbeat = Instant::now();
        // Pass commands on until the service goes away.
        while !reader.is_finished() {
            if heartbeat.elapsed() >= Duration::from_secs(20) {
                if writer.write_all(b"{\"cmd\":\"state\"}\n").is_err() { break; }
                heartbeat = Instant::now();
            }
            match commands.recv_timeout(Duration::from_millis(250)) {
                Ok(command) => {
                    if writer.write_all(command.as_bytes()).is_err() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if shared.lock().unwrap().sign_in.is_some() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
        let _ = reader.join();
        set(Link::Starting);
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Which list the view is showing.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    Home,
    Liked,
    /// What plays next.
    Queue,
    Playlist { id: String, name: String, uri: String },
    /// The albums saved to the library.
    Albums,
    Album { id: String, name: String, uri: String },
    /// An artist's albums.
    Artist { id: String, name: String },
    /// What can play, to choose among.
    Devices,
    /// The words of the track playing.
    Lyrics,
    Search(String),
    Browse {kind: String, key: String, name: String, range: String},
}

impl Source {
    fn key(&self) -> String {
        match self {
            Source::Home => "playlists:".to_string(),
            Source::Liked => "liked:".to_string(),
            Source::Queue => "queue:".to_string(),
            Source::Playlist { id, .. } => format!("tracks:{id}"),
            Source::Albums => "albums:".to_string(),
            Source::Album { id, .. } => format!("album:{id}"),
            Source::Artist { id, .. } => format!("artist:{id}"),
            Source::Devices => "devices:".to_string(),
            Source::Lyrics => "lyrics:".to_string(),
            Source::Search(query) => format!("search:{query}"),
            Source::Browse{kind,key,range,..}=>format!("{kind}:{}",if kind.starts_with("top_"){range}else{key}),
        }
    }

    /// How to ask for the part of this list from `offset` on, for the lists that
    /// come a part at a time.
    fn further(&self, offset: usize) -> Option<Value> {
        let mut request = match self {
            Source::Liked | Source::Playlist { .. } | Source::Albums | Source::Search(_) | Source::Browse{..} => self.request(),
            _ => return None,
        };
        request["offset"] = json!(offset);
        Some(request)
    }

    fn request(&self) -> Value {
        match self {
            Source::Home => json!({ "cmd": "playlists" }),
            Source::Liked => json!({ "cmd": "liked" }),
            Source::Queue => json!({ "cmd": "queue" }),
            Source::Playlist { id, .. } => json!({ "cmd": "tracks", "playlist": id }),
            Source::Albums => json!({ "cmd": "albums" }),
            Source::Album { id, .. } => json!({ "cmd": "album", "album": id }),
            Source::Artist { id, .. } => json!({ "cmd": "artist", "artist": id }),
            Source::Devices => json!({ "cmd": "devices" }),
            // Asked for by track, as each begins.
            Source::Lyrics => json!({ "cmd": "state" }),
            Source::Search(query) => json!({ "cmd": "search", "q": query }),
            Source::Browse{kind,key,range,..}=>json!({"cmd":"browse","kind":kind,"key":key,"range":range}),
        }
    }

    /// What to tell the service so that a track plays as part of this list.
    fn context(&self) -> Option<String> {
        match self {
            Source::Liked => Some("liked".to_string()),
            Source::Playlist { uri, .. } | Source::Album { uri, .. } => Some(uri.clone()),
            _ => None,
        }
    }
}

/// The pictures shown beside tracks, albums and the rest: fetched and made small in
/// the background, a few at a time, and kept on disk so that each is fetched once.
struct Covers {
    wanted: Sender<String>,
    ready: Receiver<(String, Option<egui::ColorImage>)>,
    pending_workers: Option<(Receiver<String>, Sender<(String, Option<egui::ColorImage>)>)>,
    /// Those asked for: `None` while on their way, and for good if they cannot be had.
    held: HashMap<String, Option<TextureHandle>>,
    recent: VecDeque<String>,
}

// The disk cache can be revisited cheaply; GPU textures must not grow forever.
const COVER_CACHE_LIMIT: usize = 128;

impl Covers {
    fn new(_ctx: &egui::Context) -> Covers {
        let (wanted, queue) = channel::<String>();
        let (done, ready) = channel();
        Covers { wanted, ready, pending_workers: Some((queue, done)), held: HashMap::new(), recent: VecDeque::new() }
    }

    fn start_workers(&mut self, ctx: &egui::Context) {
        let Some((queue, done)) = self.pending_workers.take() else { return; };
        let queue = Arc::new(Mutex::new(queue));
        for _ in 0..3 {
            let (queue, done, ctx) = (queue.clone(), done.clone(), ctx.clone());
            std::thread::spawn(move || {
                loop {
                    let next = queue.lock().unwrap().recv();
                    let Ok(url) = next else { return };
                    let pixels = fetch_cover(&url);
                    if done.send((url, pixels)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
    }

    fn touch(&mut self, url: &str) {
        self.recent.retain(|entry| entry != url);
        self.recent.push_back(url.to_owned());
        while self.recent.len() > COVER_CACHE_LIMIT {
            if let Some(old) = self.recent.pop_front() { self.held.remove(&old); }
        }
    }

    fn get(&mut self, ctx: &egui::Context, url: Option<&str>) -> Option<TextureHandle> {
        while let Ok((url, pixels)) = self.ready.try_recv() {
            // A request may have been evicted while the worker was fetching it.
            if self.held.contains_key(&url) {
                let texture = pixels.map(|pixels| ctx.load_texture(format!("cover-{url}"), pixels, egui::TextureOptions::LINEAR));
                self.held.insert(url, texture);
            }
        }
        let url = url?;
        if !self.held.contains_key(url) {
            self.start_workers(ctx);
            self.held.insert(url.to_string(), None);
            let _ = self.wanted.send(url.to_string());
        }
        self.touch(url);
        self.held.get(url).cloned().flatten()
    }
}

/// Fetches a cover, or finds it where it was kept, and reads it.
fn fetch_cover(url: &str) -> Option<egui::ColorImage> {
    // Kept on disk under a name made from the address.
    let name: String = url.chars().rev().take(48).filter(|c| c.is_ascii_alphanumeric()).collect();
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("des-ui/music/covers");
    let path = dir.join(name);
    let fetched = path.exists()
        || (std::fs::create_dir_all(&dir).is_ok()
            && url.starts_with("https://")
            && Command::new("curl")
                .args(["-fsS", "-m", "10", "--proto", "=https", "-o"])
                .arg(&path)
                .arg(url)
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success()));
    if !fetched {
        // Whatever part arrived is no picture.
        let _ = std::fs::remove_file(&path);
        return None;
    }
    // The file's name says nothing of its kind, so that is read from its contents.
    let image = image::ImageReader::open(&path).and_then(|reader| reader.with_guessed_format()).ok()?.decode().ok()?;
    let image = image.thumbnail(320, 320).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}

/// A transport control, drawn rather than lettered.
#[derive(Clone, Copy, PartialEq)]
enum Control {
    Previous,
    Play,
    Pause,
    Next,
}

fn control(ui: &mut Ui, theme: &Theme, which: Control, size: f32, lit: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let label = match which { Control::Previous => "Previous track", Control::Play => "Play", Control::Pause => "Pause", Control::Next => "Next track" };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let response = response.on_hover_text(label);
    let ink = if lit { ui::TEXT } else if response.hovered() { ui::TEXT } else { ui::SECONDARY };
    let painter = ui.painter();
    if lit || response.hovered() { painter.rect_filled(rect, ui::CONTROL_RADIUS, if lit { theme.tint() } else { ui::TRACK }); }
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
    let c = rect.center();
    let unit = size / 6.0;
    let triangle = |tip_x: f32, back_x: f32| {
        Shape::convex_polygon(
            vec![pos2(c.x + back_x, c.y - unit * 1.4), pos2(c.x + tip_x, c.y), pos2(c.x + back_x, c.y + unit * 1.4)],
            ink,
            Stroke::NONE,
        )
    };
    let bar = |x: f32| Rect::from_center_size(pos2(c.x + x, c.y), vec2(unit * 0.5, unit * 2.8));
    match which {
        Control::Play => {
            painter.add(triangle(unit * 1.3, -unit * 0.9));
        }
        Control::Pause => {
            painter.rect_filled(bar(-unit * 0.7), 0.0, ink);
            painter.rect_filled(bar(unit * 0.7), 0.0, ink);
        }
        Control::Next => {
            painter.add(triangle(unit * 0.8, -unit * 1.2));
            painter.rect_filled(bar(unit * 1.2), 0.0, ink);
        }
        Control::Previous => {
            painter.add(triangle(-unit * 0.8, unit * 1.2));
            painter.rect_filled(bar(-unit * 1.2), 0.0, ink);
        }
    }
    response
}

#[derive(Default)]
struct PlaylistEditor { id: String, name: String, description: String, public: bool, cover_path: String, picker: Option<crate::files::Browser>, error: Option<String>, pending: Option<String>, pending_since: Option<Instant> }

impl PlaylistEditor {
    fn begin(&mut self, shared: &mut Shared, kind: &str, now: Instant) -> u64 {
        static NEXT: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let request_id=NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let operation=format!("{kind}:{}:{request_id}",self.id);
        shared.edits.remove(&operation);self.pending=Some(operation);self.pending_since=Some(now);self.error=None;
        request_id
    }
    /// Close only after this submission succeeds; a failed draft stays editable.
    fn finish_pending(&mut self, shared: &mut Shared, now: Instant) -> bool {
        let Some(operation)=self.pending.as_ref() else{return false};
        match shared.edits.remove(operation) {
            Some(Ok(()))=>true,
            Some(Err(error))=>{self.pending=None;self.pending_since=None;self.error=Some(error);false},
            None if self.pending_since.is_some_and(|at|now.saturating_duration_since(at)>Duration::from_secs(35))=>{
                self.pending=None;self.pending_since=None;
                self.error=Some("Spotify did not confirm the change. Check the playlist before trying again.".into());false
            }
            None=>false,
        }
    }
}

pub struct Music {
    pub open: bool,
    shared: Arc<Mutex<Shared>>,
    commands: Sender<String>,
    source: Source,
    query: String,
    browser_open: Option<Receiver<Result<(),String>>>,
    editor: Option<PlaylistEditor>,
    add_to: Option<String>,
    /// The track that was playing when the queue was last asked for.
    queue_as_of: Option<String>,
    /// The track last added to the queue, and when: to say so on its row for a moment.
    queued: Option<(String, Instant)>,
    /// The client ID being typed, while the library is not connected.
    client_id: String,
    /// Lists already asked for, so that each is asked for once.
    asked: Vec<String>,
    /// The further part of a list last asked for: the list, and from where.
    asked_further: Option<(String, usize)>,
    /// The lists looked at before this one, for going back.
    trail: Vec<Source>,
    covers: Covers,
    /// When the devices were last asked for, while they are being looked at.
    devices_asked: Option<Instant>,
    /// The track whose words were last asked for.
    lyrics_asked: Option<String>,
    /// The line last brought into view, so that it is done once per line.
    lyrics_at: Option<usize>,
}

impl Music {
    pub fn connection_status(&self)->&'static str {
        match &self.shared.lock().unwrap().link {
            Link::Starting=>"starting",Link::Connected=>"connected",Link::NotSignedIn=>"not signed in",
            Link::SigningIn=>"signing in",Link::Unavailable(_)=>"unavailable — open Music for details",
        }
    }

    pub fn new(ctx: egui::Context) -> Music {
        let shared = Arc::new(Mutex::new(Shared {
            link: Link::Starting,
            now: Now::default(),
            lists: HashMap::new(),
            more: std::collections::HashSet::new(),
            next: HashMap::new(), snapshots: HashMap::new(), edits: HashMap::new(), saved: HashMap::new(), details: HashMap::new(),
            lyrics: None,
            problem: None,
            sign_in: None,
            refresh: false,
            cancel: false,
        }));
        let (commands, receiver) = channel();
        let for_thread = shared.clone();
        let covers = Covers::new(&ctx);
        std::thread::spawn(move || connection(for_thread, receiver, ctx));
        Music {
            open: false,
            shared,
            commands,
            source: Source::Home,
            query: String::new(),
            browser_open: None, editor: None, add_to: None,
            client_id: String::new(),
            queue_as_of: None,
            queued: None,
            asked: Vec::new(),
            asked_further: None,
            trail: Vec::new(),
            covers,
            devices_asked: None,
            lyrics_asked: None,
            lyrics_at: None,
        }
    }

    fn sign_in(&self, arguments: Vec<String>) {
        let mut shared = self.shared.lock().unwrap();
        shared.cancel = false;
        shared.sign_in = Some(arguments);
    }

    /// One of the words `des-music send` takes: `toggle`, `next`, `prev`.
    pub fn command(&mut self, word: &str) {
        self.send(json!({ "cmd": word }));
    }

    fn send(&self, request: Value) {
        let _ = self.commands.send(format!("{request}\n"));
    }

    pub fn now(&self) -> Now {
        self.shared.lock().unwrap().now.clone()
    }

    /// Starts on the words of what is playing.
    pub fn show_lyrics(&mut self) {
        self.source = Source::Lyrics;
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Shows another list. Going into something from a list leaves a way back to it.
    fn go(&mut self, to: Source, deeper: bool) {
        if to == self.source {
            return;
        }
        if deeper {
            self.trail.push(self.source.clone());
            if self.trail.len() > 12 {
                self.trail.remove(0);
            }
        } else {
            self.trail.clear();
        }
        self.source = to;
    }

    /// Asks for a list unless it was asked for already.
    fn want(&mut self, key: String, request: Value) {
        if !self.asked.contains(&key) {
            self.asked.push(key);
            self.send(request);
        }
    }

    fn load_more(&mut self, source: &Source, loaded: usize) {
        if let Some(mut request)=source.further(loaded) {
            let token=self.page_token(source,loaded);
            if self.asked_further.as_ref()==Some(&token){return;}
            self.asked_further=Some(token);
            if let Some(next)=self.shared.lock().unwrap().next.get(&source.key()) {
                request["offset"]=next["offset"].clone(); request["cursor"]=next["cursor"].clone();
            }
            if let Some(snapshot)=self.shared.lock().unwrap().snapshots.get(&source.key()) {request["snapshot"]=json!(snapshot);}
            self.send(request);
        }
    }

    fn page_token(&self, source: &Source, loaded: usize) -> (String,usize) {
        let offset=self.shared.lock().unwrap().next.get(&source.key()).and_then(|v|v["offset"].as_u64()).map_or(loaded,|n|n as usize);
        (source.key(),offset)
    }

    /// Draws a square for the cover: the picture, or a quiet placeholder.
    fn draw_cover(&mut self, ui: &mut Ui, size: f32, url: Option<&str>) {
        let texture = self.covers.get(ui.ctx(), url);
        let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, ui::PANEL);
        match texture {
            Some(texture) => {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture.id(), rect, uv, egui::Color32::WHITE);
            }
            None => {
                ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
            }
        }
    }

    /// The right column's section: what is playing, and the three controls. Returns
    /// true when the title was clicked, to open the full view.
    pub fn preferred_panel_height(&self) -> f32 {
        if self.now().track.is_some() { PANEL_HEIGHT } else { 0.0 }
    }

    pub fn panel(&mut self, ui: &mut Ui, theme: &Theme) -> bool {
        let now = self.now();
        let Some(track) = now.track.as_ref() else { return false; };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PANEL_HEIGHT), Sense::hover());
        let mut inner = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
        inner.set_clip_rect(rect.intersect(ui.clip_rect()));
        let ui = &mut inner;
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        ui::tracked(ui, if now.playing { "NOW PLAYING" } else { "PAUSED" }, ui::display_bold(10.0), ui::SECONDARY, 1.5);
        let mut open = false;
        ui.horizontal(|ui| {
            self.draw_cover(ui, 36.0, track.cover.as_deref());
            ui.vertical(|ui| {
                let fit = (ui.available_width().max(0.0) / 8.0) as usize;
                let title = ui.add(egui::Label::new(egui::RichText::new(crate::panels::truncate(&track.name, fit)).font(ui::display_bold(13.0)).color(ui::TEXT)).truncate().sense(Sense::click()));
                open = title.on_hover_text(format!("Open Music — {}", track.name)).clicked();
                ui::text(ui, &crate::panels::truncate(&track.artists, fit), ui::display(11.0), ui::SECONDARY).on_hover_text(&track.artists);
            });
        });
        ui.horizontal(|ui| {
            if control(ui, theme, Control::Previous, ui::CONTROL_HEIGHT, false).clicked() { self.send(json!({ "cmd": "prev" })); }
            if control(ui, theme, if now.playing { Control::Pause } else { Control::Play }, ui::CONTROL_HEIGHT, true).clicked() { self.send(json!({ "cmd": "toggle" })); }
            if control(ui, theme, Control::Next, ui::CONTROL_HEIGHT, false).clicked() { self.send(json!({ "cmd": "next" })); }
            ui::text(ui, &clock(now.position_ms()), ui::mono(10.0), ui::SECONDARY);
        });
        let frac = now.position_ms() as f32 / track.duration_ms.max(1) as f32;
        ui::bar(ui, theme, 3.0, frac.min(CAUTION_FREE), 0.0);
        self.volume_control(ui, theme, &now);
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
        open
    }

    /// What is playing, for the lock screen: the cover, the track, how far along it
    /// is, and the three controls. Nothing when nothing is. It sits at the bottom of
    /// `within`, in the middle.
    pub fn lock_card(&mut self, ui: &mut Ui, theme: &Theme, within: Rect) {
        let now = self.now();
        let Some(track) = now.track.clone() else {
            return;
        };
        let size = vec2(460.0_f32.min(within.width()), 116.0);
        let mut card = Rect::from_center_size(pos2(within.center().x, within.bottom() - 40.0 - size.y / 2.0), size);
        // On a short screen the middle is taken by the password line: the corner then.
        if card.top() < within.center().y + 236.0 {
            card = Rect::from_min_size(within.right_bottom() - size, size);
        }
        crate::material::paint(ui.ctx(), ui.painter(), card, ui::CONTROL_RADIUS, crate::material::Role::Panel);
        ui.painter().rect_stroke(card, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);

        let mut inner = ui.new_child(UiBuilder::new().max_rect(card.shrink(14.0)).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut inner;
        ui.spacing_mut().item_spacing.x = 16.0;
        self.draw_cover(ui, 88.0, track.cover.as_deref());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let fit = (ui.available_width() / 8.6) as usize;
            let state = match (&now.device, now.playing) {
                (Some(device), true) => format!("PLAYING ON {}", device.to_uppercase()),
                (None, true) => "NOW PLAYING".to_string(),
                (_, false) => "PAUSED".to_string(),
            };
            ui::tracked(ui, &crate::panels::truncate(&state, fit), ui::display_bold(10.0), if now.playing { theme.main } else { ui::LABEL }, 2.0);
            ui::text(ui, &crate::panels::truncate(&track.name, fit), ui::display_bold(16.0), ui::TEXT);
            ui::text(ui, &crate::panels::truncate(&track.artists, fit + 4), ui::display(11.0), ui::SECONDARY);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if control(ui, theme, Control::Previous, 26.0, false).clicked() {
                    self.send(json!({ "cmd": "prev" }));
                }
                let which = if now.playing { Control::Pause } else { Control::Play };
                if control(ui, theme, which, 30.0, true).clicked() {
                    self.send(json!({ "cmd": "toggle" }));
                }
                if control(ui, theme, Control::Next, 26.0, false).clicked() {
                    self.send(json!({ "cmd": "next" }));
                }
                ui.add_space(10.0);
                let position = now.position_ms();
                ui::text(ui, &format!("{} / {}", clock(position), clock(track.duration_ms)), ui::mono(11.0), ui::LABEL);
            });
            let frac = now.position_ms() as f32 / track.duration_ms.max(1) as f32;
            ui::bar(ui, theme, 3.0, frac.min(CAUTION_FREE), 0.0);
        });
        // The position keeps moving while it plays.
        if now.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The full view, in place of the terminal.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        if let Some(receiver)=&self.browser_open {
            match receiver.try_recv() {
                Ok(result)=>{self.browser_open=None;if let Err(error)=result{self.shared.lock().unwrap().problem=Some((error,Instant::now()));}},
                Err(std::sync::mpsc::TryRecvError::Disconnected)=>{self.browser_open=None;self.shared.lock().unwrap().problem=Some(("The browser launcher stopped without an answer.".into(),Instant::now()));},
                Err(std::sync::mpsc::TryRecvError::Empty)=>{},
            }
        }
        if self.editor.is_none() && self.add_to.is_none() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            if ui.memory(|memory| memory.any_popup_open()) {
                ui.memory_mut(|memory| memory.close_popup());
            } else {
                self.open = false;
                return;
            }
        }
        let (link, now) = {
            let shared = self.shared.lock().unwrap();
            (shared.link.clone(), shared.now.clone())
        };
        let margin = (rect.width() * 0.05).clamp(16.0, 56.0);
        let area = Rect::from_min_max(rect.min + vec2(margin, 28.0), rect.max - vec2(margin, 14.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                ui::tracked(ui, "MUSIC", ui::display_bold(22.0), ui::TEXT, 3.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui::button(ui, theme, "DONE", Some("ESC"), false).clicked() {
                        self.open = false;
                    }
                });
            });
            ui.add_space(16.0);

            match link {
                Link::Connected => self.library(ui, theme, &now),
                Link::Starting => {
                    ui::text(ui, "Starting the music service…", ui::display(13.0), ui::SECONDARY);
                }
                Link::SigningIn => {
                    ui::text(ui, "Spotify's page is open in your browser. Finish there and come back.", ui::display(13.0), ui::TEXT);
                    ui.add_space(18.0);
                    if ui::button(ui, theme, "CANCEL", None, false).clicked() {
                        self.shared.lock().unwrap().cancel = true;
                    }
                }
                Link::Unavailable(why) => {
                    ui::text(ui, &why, ui::display(13.0), ui::CAUTION);
                }
                Link::NotSignedIn => {
                    ui::text(ui, "Play your Spotify library here, without opening Spotify.", ui::display(18.0), ui::TEXT);
                    ui.add_space(8.0);
                    ui::text(ui, "Signing in happens once, on Spotify's own page in your browser. It needs Spotify Premium.", ui::display(12.0), ui::SECONDARY);
                    ui.add_space(18.0);
                    if ui::button(ui, theme, "SIGN IN TO SPOTIFY", None, true).clicked() {
                        self.sign_in(vec!["login".to_string()]);
                    }
                }
            }
        });
        self.playlist_dialogs(ui.ctx(),theme);
    }

    /// Shown in place of the library until it is connected: what to do, and where
    /// to put the client ID.
    fn connect_library(&mut self, ui: &mut Ui, theme: &Theme) {
        ui::text(ui, "One more step: connect your library.", ui::display(18.0), ui::TEXT);
        ui.add_space(8.0);
        for line in [
            "You are signed in. Spotify only lets an app of your own read your playlists,",
            "liked songs and search, so it takes a free one, made once:",
            "",
            "1  Open developer.spotify.com/dashboard and choose Create app.",
            "2  Give it any name. As its Redirect URI enter exactly:  http://127.0.0.1:8898/login",
            "3  Tick Web API, save, and copy the app's Client ID here.",
        ] {
            ui::text(ui, line, ui::display(12.0), if line.starts_with(char::is_numeric) { ui::TEXT } else { ui::SECONDARY });
        }
        ui.add_space(14.0);
        let (field, _) = ui.allocate_exact_size(vec2(ui.available_width().min(340.0), 36.0), Sense::hover());
        ui.painter().rect_filled(field, ui::CONTROL_RADIUS, ui::PANEL);
        ui.painter().rect_stroke(field, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
        ui.put(
            field.shrink2(vec2(10.0, 7.0)),
            TextEdit::singleline(&mut self.client_id).frame(false).font(ui::mono(13.0)).text_color(ui::TEXT).hint_text("client ID"),
        );
        ui.add_space(10.0);
        let id = self.client_id.trim().to_string();
        let valid = id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit());
        if !id.is_empty() && !valid {
            ui::text(ui, "A client ID is 32 letters and digits.", ui::display(11.0), ui::CAUTION);
        }
        if ui::button(ui, theme, "CONNECT LIBRARY", None, valid).clicked() && valid {
            self.sign_in(vec!["library".to_string(), id]);
        }
    }

    fn browse_menu(&mut self, ui: &mut Ui) {
        ui.menu_button("Browse library", |ui| {
            for (name,kind) in [("Recently played","recent"),("Top tracks","top_tracks"),("Top artists","top_artists"),("Followed artists","following"),("Podcasts","shows"),("Saved episodes","episodes"),("Audiobooks","audiobooks")] {
                if ui.button(name).clicked() {self.go(Source::Browse{kind:kind.into(),key:String::new(),name:name.into(),range:"medium_term".into()},false);ui.close_menu();}
            }
        });
    }

    fn library_actions(&mut self, ui: &mut Ui, theme: &Theme, source: &Source) {
        ui.horizontal_wrapped(|ui| {
            self.browse_menu(ui);
            ui.menu_button("Library actions", |ui| {
                if ui.button("New playlist").clicked(){self.editor=Some(PlaylistEditor::default());ui.close_menu();}
                if ui.button("Refresh").clicked(){self.asked.clear();self.asked_further=None;self.shared.lock().unwrap().invalidate();ui.close_menu();}
                if ui.button("Reconnect / approve features").clicked(){self.sign_in(vec!["library".into()]);ui.close_menu();}
            });
            let uri=match source {Source::Playlist{uri,..}|Source::Album{uri,..}=>Some(uri.clone()),Source::Artist{id,..}=>Some(format!("spotify:artist:{id}")),Source::Browse{kind,key,..} if kind=="show"||kind=="audiobook"=>Some(format!("spotify:{kind}:{key}")),_=>None};
            if let Some(uri)=uri {
                self.want(format!("saved:{uri}"),json!({"cmd":"library","uri":uri}));
                let saved=self.shared.lock().unwrap().saved.get(&uri).copied();
                let label=if uri.starts_with("spotify:artist:"){if saved==Some(true){"Unfollow"}else{"Follow"}}else if saved==Some(true){"Remove from library"}else{"Save to library"};
                if ui.add_enabled_ui(saved.is_some(),|ui|ui::button(ui,theme,label,None,saved==Some(true))).inner.clicked(){self.send(json!({"cmd":"library","uri":uri,"save":saved!=Some(true)}));}
            }
            if let Source::Playlist{id,..}=source {
                self.want(format!("details:{id}"),json!({"cmd":"playlist_details","id":id}));
                let details=self.shared.lock().unwrap().details.get(id).cloned();
                if ui.add_enabled_ui(details.as_ref().is_some_and(|v|v["owner"]==true),|ui|ui::button(ui,theme,"Edit playlist",None,false)).inner.clicked(){
                    let v=details.unwrap();self.editor=Some(PlaylistEditor{id:id.clone(),name:v["name"].as_str().unwrap_or_default().into(),description:v["description"].as_str().unwrap_or_default().into(),public:v["public"].as_bool().unwrap_or(false),..Default::default()});
                }
            }
        });
        if let Source::Browse{kind,key,name,range}=source {
            if kind.starts_with("top_") {
                ui.horizontal_wrapped(|ui| {for (label,value) in [("4 weeks","short_term"),("6 months","medium_term"),("1 year","long_term")] {
                    if ui.selectable_label(range==value,label).clicked(){self.go(Source::Browse{kind:kind.clone(),key:key.clone(),name:name.clone(),range:value.into()},false);}
                }});
            }
        }
    }

    fn playlist_dialogs(&mut self, ctx: &egui::Context, theme: &Theme) {
        if let Some(uri)=self.add_to.clone() {
            let playlists=self.shared.lock().unwrap().lists.get("playlists:").cloned();
            let mut close=false;
            let modal=egui::Modal::new(egui::Id::new("music-add-to-playlist")).show(ctx,|ui| {
                ui.set_width((ctx.screen_rect().width()-64.0).clamp(200.0,420.0));
                ui::text(ui,"Add to playlist",ui::display_bold(18.0),ui::TEXT);
                ScrollArea::vertical().max_height(360.0).show(ui,|ui| {
                    match &playlists {
                        Some((items,None))=>{if !items.iter().any(|p|p.editable){ui.label("No editable playlists. Create one through Library actions first.");}for playlist in items.iter().filter(|p|p.editable) {if ui.button(&playlist.name).clicked(){self.send(json!({"cmd":"playlist_item","id":playlist.id,"uri":uri,"remove":false}));close=true;}}},
                        Some((_,Some(why)))=>{ui.label(why);},_=>{ui.label("Loading playlists…");}
                    }
                });
                if ui::button(ui,theme,"Cancel",None,false).clicked(){close=true;}
            });
            if close||modal.should_close(){self.add_to=None;}
        }
        let Some(mut editor)=self.editor.take() else{return};
        if editor.finish_pending(&mut self.shared.lock().unwrap(),Instant::now()){return;}
        let mut close=false;
        let modal=egui::Modal::new(egui::Id::new("music-playlist-editor")).show(ctx,|ui| {
            ui.set_width((ctx.screen_rect().width()-64.0).clamp(200.0,480.0));
            if let Some(picker)=editor.picker.as_mut() {
                ui.set_height((ctx.screen_rect().height()-120.0).clamp(180.0,480.0));
                match picker.show(ui) {Some(crate::files::Event::Choose(path))=>{editor.cover_path=path.to_string_lossy().into_owned();editor.picker=None;},Some(crate::files::Event::Close)=>editor.picker=None,_=>{}}
                return;
            }
            ui::text(ui,if editor.id.is_empty(){"New playlist"}else{"Edit playlist"},ui::display_bold(18.0),ui::TEXT);
            ui.add_enabled_ui(editor.pending.is_none(),|ui| {
            ui.label("Name");ui.add_sized(vec2(ui.available_width(),ui::CONTROL_HEIGHT),TextEdit::singleline(&mut editor.name).char_limit(200));
            ui.label("Description");ui.add_sized(vec2(ui.available_width(),72.0),TextEdit::multiline(&mut editor.description).char_limit(300));
            ui.checkbox(&mut editor.public,"Public playlist");
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(editor.pending.is_none() && !editor.name.trim().is_empty(),egui::Button::new("Save playlist")).clicked(){let request_id=editor.begin(&mut self.shared.lock().unwrap(),"playlist_save",Instant::now());self.send(json!({"cmd":"playlist_save","id":editor.id,"name":editor.name.trim(),"description":editor.description,"public":editor.public,"request_id":request_id}));}
                if ui::button(ui,theme,if editor.pending.is_some(){"Close"}else{"Cancel"},None,false).clicked(){close=true;}
            });
            if !editor.id.is_empty() {
                ui.separator();
                if ui.add_enabled_ui(editor.pending.is_none(),|ui|ui::button(ui,theme,"Choose cover image",None,false)).inner.clicked(){editor.picker=Some(crate::files::Browser::new(crate::files::home().join("Pictures"),crate::files::Mode::ImagePicker));}
                if !editor.cover_path.is_empty(){ui.label(&editor.cover_path);if ui.add_enabled_ui(editor.pending.is_none(),|ui|ui::button(ui,theme,"Upload cover",None,false)).inner.clicked(){match playlist_cover(&editor.cover_path){Ok(jpeg)=>{let request_id=editor.begin(&mut self.shared.lock().unwrap(),"playlist_cover",Instant::now());self.send(json!({"cmd":"playlist_cover","id":editor.id,"jpeg":jpeg,"request_id":request_id}));},Err(error)=>{editor.error=Some(error);}}}}
            }
            if editor.pending.is_some(){ui.label("Waiting for Spotify to confirm…");ctx.request_repaint_after(Duration::from_secs(1));}
            if let Some(error)=&editor.error{ui.colored_label(ui::CAUTION,error);}
        });
        if !close && !modal.should_close(){self.editor=Some(editor);}
    }

    fn library(&mut self, ui: &mut Ui, theme: &Theme, now: &Now) {
        if std::mem::take(&mut self.shared.lock().unwrap().refresh) {
            self.asked.clear();
            self.asked_further = None;
        }
        self.want("playlists:".to_string(), json!({ "cmd": "playlists" }));
        let source = self.source.clone();
        self.want(source.key(), source.request());
        // The queue moves on with every track, so it is asked for again each time.
        let playing_uri = now.track.as_ref().map(|track| track.uri.clone());
        if source == Source::Queue && self.queue_as_of != playing_uri {
            self.queue_as_of = playing_uri;
            self.send(json!({ "cmd": "queue" }));
        }
        // Devices come and go, so while they are looked at they are asked for again.
        if source == Source::Devices && self.devices_asked.is_none_or(|at| at.elapsed() > Duration::from_secs(6)) {
            self.devices_asked = Some(Instant::now());
            self.send(json!({ "cmd": "devices" }));
            ui.ctx().request_repaint_after(Duration::from_secs(6));
        }
        let (playlists, list, more, problem) = {
            let mut shared = self.shared.lock().unwrap();
            if shared.problem.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(12)) {
                shared.problem = None;
            }
            (
                shared.lists.get("playlists:").cloned(),
                shared.lists.get(&source.key()).cloned(),
                shared.more.contains(&source.key()),
                shared.problem.as_ref().map(|(text, _)| text.clone()),
            )
        };

        let transport_height = if ui.available_width() < 480.0 { 246.0 } else { 200.0 };
        let problem_height = if problem.is_some() { 40.0 } else { 0.0 };
        let body_height = (ui.available_height() - transport_height - 12.0 - problem_height).max(80.0);
        let unconnected = matches!(&playlists, Some((_, Some(why))) if why.starts_with("Your library is not connected"));
        if unconnected {
            let (body, _) = ui.allocate_exact_size(vec2(ui.available_width(), body_height), Sense::hover());
            let mut inner = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
            inner.set_clip_rect(body.intersect(ui.clip_rect()));
            ScrollArea::vertical().max_height(body_height).id_salt("connect-library").show(&mut inner, |ui| {
                self.connect_library(ui, theme);
            });
            ui.add_space(12.0);
            self.transport(ui, theme, now, transport_height);
            return;
        }
        let rail_width = (ui.available_width() * 0.23).clamp(152.0, 224.0);
        let wide = ui.available_width() >= 660.0;
        let navigation_top = ui.cursor().min.y;
        if !wide { self.compact_navigation(ui, theme); }
        let navigation_height = ui.cursor().min.y - navigation_top;
        let body_height = (body_height - navigation_height).max(32.0);
        let (body, _) = ui.allocate_exact_size(vec2(ui.available_width(), body_height), Sense::hover());
        let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::left_to_right(Align::Min)));
        body_ui.set_clip_rect(body.intersect(ui.clip_rect()));
        body_ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            // Left: search, then the library.
            if wide { ui.allocate_ui_with_layout(vec2(rail_width, body_height), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let (field, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                ui.painter().rect_filled(field, ui::CONTROL_RADIUS, ui::PANEL);
                ui.painter().rect_stroke(field, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE_STRONG), StrokeKind::Inside);
                let edit = ui.put(
                    Rect::from_min_max(field.min + vec2(10.0, 7.0), field.max - vec2(48.0, 7.0)),
                    TextEdit::singleline(&mut self.query)
                        .frame(false)
                        .font(ui::display(13.0))
                        .text_color(ui::TEXT)
                        .hint_text("search Spotify"),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.query.trim().is_empty() {
                    self.go(Source::Search(self.query.trim().to_string()), false);
                }
                if !self.query.is_empty() && ui.put(Rect::from_min_size(field.right_top() - vec2(44.0, 0.0), vec2(44.0, 36.0)), egui::Button::new("Clear").wrap_mode(egui::TextWrapMode::Extend)).on_hover_text("Clear search").clicked() {
                    self.query.clear();
                    self.go(Source::Home, false);
                    edit.request_focus();
                }
                ui.add_space(10.0);
                if side_row(ui, theme, "HOME", self.source == Source::Home) { self.go(Source::Home, false); }
                if side_row(ui, theme, "LIKED SONGS", self.source == Source::Liked) {
                    self.go(Source::Liked, false);
                }
                if side_row(ui, theme, "ALBUMS", self.source == Source::Albums) {
                    self.go(Source::Albums, false);
                }
                if side_row(ui, theme, "UP NEXT", self.source == Source::Queue) {
                    self.go(Source::Queue, false);
                    self.queue_as_of = None;
                }
                if side_row(ui, theme, "LYRICS", self.source == Source::Lyrics) {
                    self.go(Source::Lyrics, false);
                    self.lyrics_at = None;
                }
                if side_row(ui, theme, "DEVICES", self.source == Source::Devices) {
                    self.go(Source::Devices, false);
                    self.devices_asked = None;
                }
                ui.add_space(8.0);
                ui::tracked(ui, "PLAYLISTS", ui::display_bold(11.0), ui::LABEL, 3.0);
                match playlists {
                    None => {
                        ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL);
                    }
                    Some((_, Some(why))) => {
                        ui::text(ui, &why, ui::display(11.0), ui::CAUTION);
                    }
                    Some((items, None)) => {
                        ScrollArea::vertical().max_height((body_height - 300.0).max(24.0)).id_salt("playlists").auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for playlist in &items {
                                let chosen = matches!(&self.source, Source::Playlist { id, .. } if *id == playlist.id);
                                if side_row(ui, theme, &playlist.name, chosen) {
                                    let to = Source::Playlist {
                                        id: playlist.id.clone(),
                                        name: playlist.name.clone(),
                                        uri: playlist.uri.clone(),
                                    };
                                    self.go(to, false);
                                }
                            }
                        });
                    }
                }
            });

            }
            // Right: the list chosen.
            ui.allocate_ui_with_layout(vec2(ui.available_width(), body_height), Layout::top_down(Align::Min), |ui| {
                let title = match &source {
                    Source::Home => "YOUR MUSIC".to_string(),
                    Source::Liked => "LIKED SONGS".to_string(),
                    Source::Queue => "UP NEXT".to_string(),
                    Source::Playlist { name, .. } | Source::Album { name, .. } => name.to_uppercase(),
                    Source::Albums => "ALBUMS".to_string(),
                    Source::Artist { name, .. } => format!("ALBUMS BY {}", name.to_uppercase()),
                    Source::Devices => "PLAY ON".to_string(),
                    Source::Lyrics => "LYRICS".to_string(),
                    Source::Search(query) => format!("RESULTS FOR “{}”", query.to_uppercase()),
                    Source::Browse{name,..}=>name.to_uppercase(),
                };
                let mut back = false;
                ui::section(ui, &crate::panels::truncate(&title, 48), |ui| {
                    if !self.trail.is_empty() {
                        back = ui::button(ui, theme, "BACK", None, false).clicked();
                    }
                    if let Some((items, None)) = &list {
                        let count = if more { format!("{} SO FAR", items.len()) } else { format!("{} SHOWN", items.len()) };
                        ui::meta(ui, &count, ui::LABEL);
                    }
                });
                if back {
                    if let Some(to) = self.trail.pop() {
                        self.source = to;
                    }
                    return;
                }
                self.library_actions(ui,theme,&source);
                if source == Source::Lyrics {
                    self.lyrics(ui, theme, now);
                    return;
                }
                match list {
                    None => {
                        ui::text(ui, "Loading…", ui::display(12.0), ui::LABEL);
                    }
                    Some((_, Some(why))) if why.starts_with("The queue needs") => {
                        ui::text(ui, "The queue needs one more approval from Spotify, on its page in your browser.", ui::display(12.0), ui::TEXT);
                        ui.add_space(12.0);
                        if ui::button(ui, theme, "APPROVE", None, true).clicked() {
                            self.sign_in(vec!["library".to_string()]);
                        }
                    }
                    Some((_, Some(why))) => {
                        ui::text(ui, &why, ui::display(12.0), ui::CAUTION);
                        if ui::button(ui, theme, "TRY AGAIN", None, false).clicked() {
                            self.asked.retain(|key| key != &source.key());
                            self.asked_further=None;
                            self.shared.lock().unwrap().lists.remove(&source.key());
                        }
                    }
                    Some((items, None)) if items.is_empty() => {
                        if more {
                            ui::text(ui,"No available items on this page.",ui::display(12.0),ui::SECONDARY);
                            if ui::button(ui,theme,"LOAD MORE",None,false).clicked(){self.load_more(&source,items.len());}
                            return;
                        }
                        let empty = match source {
                            Source::Queue => "Nothing is waiting to play.",
                            Source::Devices => "No device is online. Open Spotify on one and it shows up here.",
                            Source::Home => "Your playlists will appear here. Add a playlist in Spotify, or open Liked songs.",
                            Source::Liked => "No liked songs yet. Search for a track, play it and choose Like.",
                            Source::Albums => "No saved albums yet. Albums saved in Spotify appear here.",
                            Source::Search(_) => "No results. Try another song, artist or album.",
                            _ => "This collection is empty.",
                        };
                        ui::text(ui, empty, ui::display(12.0), ui::SECONDARY);
                        if source == Source::Home {
                            ui.add_space(12.0);
                            ui.horizontal_wrapped(|ui| {
                                if ui::button(ui, theme, "LIKED SONGS", None, false).clicked() { self.go(Source::Liked, false); }
                                if ui::button(ui, theme, "SAVED ALBUMS", None, false).clicked() { self.go(Source::Albums, false); }
                            });
                        }
                    }
                    Some((items, None)) if matches!(source, Source::Home | Source::Albums | Source::Artist { .. }) => {
                        ScrollArea::vertical().max_height((body_height - 132.0).max(24.0)).id_salt(source.key()).auto_shrink([false, false]).show(ui, |ui| {
                            if source == Source::Home {
                                ui::text(ui, "Made yours", ui::display_bold(26.0), ui::TEXT);
                                ui::text(ui, "Your playlists, saved albums and favourite songs.", ui::display(13.0), ui::SECONDARY);
                                ui.add_space(12.0);
                                ui.horizontal_wrapped(|ui| {
                                    if ui::button(ui, theme, "LIKED SONGS", None, false).clicked() { self.go(Source::Liked, false); }
                                    if ui::button(ui, theme, "SAVED ALBUMS", None, false).clicked() { self.go(Source::Albums, false); }
                                });
                                ui.add_space(18.0);
                                ui::tracked(ui, "YOUR PLAYLISTS", ui::display_bold(11.0), ui::SECONDARY, 1.5);
                                ui.add_space(8.0);
                            }
                            self.collection_cards(ui, theme, &items);
                            if more && ui::button(ui, theme, "LOAD MORE", None, false).clicked() {
                                self.load_more(&source,items.len());
                            }
                        });
                    }
                    Some((items, None)) => {
                        let playing = now.track.as_ref().map(|track| track.uri.as_str());
                        ScrollArea::vertical().max_height((body_height - 132.0).max(24.0)).id_salt(source.key()).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let in_queue = source == Source::Queue;
                            for (index, item) in items.iter().enumerate() {
                                let just_queued = self.queued.as_ref().is_some_and(|(uri, at)| *uri == item.uri && at.elapsed() < Duration::from_secs(2));
                                // In the queue a track may come up twice, and none of them is the one playing.
                                let lit = (!in_queue && playing == Some(item.uri.as_str())) || item.active;
                                // Only the rows in view have their pictures fetched.
                                let place = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), ROW));
                                let cover = if ui.is_rect_visible(place) { self.covers.get(ui.ctx(), item.cover.as_deref()) } else { None };
                                let picked = ui.push_id(index, |ui| track_row(ui, theme, item, cover.as_ref(), lit, !in_queue, just_queued, matches!(&source,Source::Playlist{id,..} if self.shared.lock().unwrap().details.get(id).is_some_and(|v|v["editable"]==true)), playlist_move_target(&items,index,true).is_some(), playlist_move_target(&items,index,false).is_some())).inner;
                                match picked {
                                    Picked::Save|Picked::RemoveSaved=>self.send(json!({"cmd":"library","uri":item.uri,"save":picked==Picked::Save})),
                                    Picked::AddToPlaylist=>self.add_to=Some(item.uri.clone()),
                                    Picked::RemoveFromPlaylist|Picked::MoveUp|Picked::MoveDown=>{
                                        if let Source::Playlist{id,..}=&source {
                                            let snapshot=self.shared.lock().unwrap().snapshots.get(&source.key()).cloned();
                                            if let (Some(position),Some(snapshot))=(item.position,snapshot) {
                                                if picked==Picked::RemoveFromPlaylist {self.send(json!({"cmd":"playlist_item","id":id,"uri":item.uri,"position":position,"remove":true,"snapshot":snapshot}));}
                                                else if let Some(before)=playlist_move_target(&items,index,picked==Picked::MoveUp) {
                                                    self.send(json!({"cmd":"playlist_reorder","id":id,"from":position,"before":before,"snapshot":snapshot}));
                                                }
                                            }
                                        }
                                    },_=>{}
                                }
                                if picked == Picked::Queue {
                                    self.queued = Some((item.uri.clone(), Instant::now()));
                                    self.send(json!({ "cmd": "enqueue", "track": item.uri }));
                                    ui.ctx().request_repaint_after(Duration::from_secs(2));
                                }
                                // From a track to the album it is on, or to who made it.
                                if picked == Picked::Album && !item.album_id.is_empty() {
                                    let uri = format!("spotify:album:{}", item.album_id);
                                    self.go(Source::Album { id: item.album_id.clone(), name: item.album.clone(), uri }, true);
                                }
                                if picked == Picked::Artist && !item.artist_id.is_empty() {
                                    // Only the first of several artists can be gone to.
                                    let name = item.by.split(", ").next().unwrap_or_default().to_string();
                                    self.go(Source::Artist { id: item.artist_id.clone(), name }, true);
                                }
                                if picked != Picked::Row {
                                    continue;
                                }
                                let named = (item.id.clone(), item.name.clone(), item.uri.clone());
                                match item.kind {
                                    // Spotify offers no way to jump into the queue but one step at a time.
                                    Kind::Track | Kind::Episode if in_queue => {
                                        for _ in 0..=index {
                                            self.send(json!({ "cmd": "next" }));
                                        }
                                    }
                                    Kind::Track | Kind::Episode => self.send(json!({ "cmd": "play", "context": source.context(), "track": item.uri })),
                                    Kind::Playlist => self.go(Source::Playlist { id: named.0, name: named.1, uri: named.2 }, true),
                                    Kind::Album => self.go(Source::Album { id: named.0, name: named.1, uri: named.2 }, true),
                                    Kind::Artist => self.go(Source::Artist { id: named.0, name: named.1 }, true),
                                    Kind::Show|Kind::Audiobook=>self.go(Source::Browse{kind:if item.kind==Kind::Show{"show"}else{"audiobook"}.into(),key:item.id.clone(),name:item.name.clone(),range:"medium_term".into()},true),
                                    Kind::Chapter=>{if self.browser_open.is_none(){if let Some(url)=spotify_item_url(item){self.browser_open=Some(crate::apps::open_url(url,ui.ctx().clone()));}else{self.shared.lock().unwrap().problem=Some(("Spotify did not provide a valid chapter link.".into(),Instant::now()));}}}
                                    Kind::Device => {
                                        if !item.active {
                                            self.send(json!({ "cmd": "transfer", "device": item.id }));
                                        }
                                    }
                                }
                            }
                            // The rest of a long list is fetched as its end comes into view.
                            if more {
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                                ui.painter().text(rect.left_center() + vec2(26.0, 0.0), Align2::LEFT_CENTER, "Loading more…", ui::display(12.0), ui::LABEL);
                                let wanted = self.page_token(&source,items.len());
                                if ui.is_rect_visible(rect) && self.asked_further.as_ref() != Some(&wanted) {
                                    if let Some(request) = source.further(items.len()) {
                                        let _=request; self.load_more(&source,items.len());
                                        self.asked_further = Some(wanted);
                                    }
                                }
                            }
                        });
                    }
                }
            });
        });

        if let Some(problem) = problem {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), problem_height), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui::text(ui, &crate::panels::truncate(&problem, 110), ui::display(12.0), ui::CAUTION);
                // One more permission is asked for on Spotify's page, as for the library.
                if problem.contains("needs one more approval") && ui::button(ui, theme, "APPROVE", None, true).clicked() {
                    self.shared.lock().unwrap().problem = None;
                    self.sign_in(vec!["library".to_string()]);
                }
            });
        }
        ui.add_space(12.0);
        self.transport(ui, theme, now, transport_height);
    }

    /// Narrow workspaces retain every destination without squeezing a permanent rail.
    fn compact_navigation(&mut self, ui: &mut Ui, theme: &Theme) {
        ui.horizontal(|ui| {
            let edit = ui.add_sized(vec2((ui.available_width() - 76.0).max(48.0), 30.0), TextEdit::singleline(&mut self.query).font(ui::display(13.0)).hint_text("Search Spotify · Enter"));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.query.trim().is_empty() {
                self.go(Source::Search(self.query.trim().to_string()), false);
            }
            if !self.query.is_empty() && ui::button(ui, theme, "CLEAR", None, false).clicked() {
                self.query.clear(); self.go(Source::Home, false); edit.request_focus();
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (label, source) in [("Home", Source::Home), ("Liked", Source::Liked), ("Albums", Source::Albums), ("Queue", Source::Queue), ("Lyrics", Source::Lyrics), ("Devices", Source::Devices)] {
                if ui.selectable_label(self.source == source, egui::RichText::new(label).font(ui::display(12.0))).clicked() { self.go(source, false); }
            }
        });
        ui.add_space(8.0);
    }

    /// Art leads browsing; selecting a collection still uses the existing service route.
    fn collection_cards(&mut self, ui: &mut Ui, theme: &Theme, items: &[Item]) {
        let width = ui.available_width();
        let columns = ((width + 12.0) / 164.0).floor().max(1.0) as usize;
        let tile = ((width - 12.0 * columns.saturating_sub(1) as f32) / columns as f32).max(24.0);
        for chunk in items.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for item in chunk {
                    ui.push_id(&item.uri, |ui| {
                        let (rect, response) = ui.allocate_exact_size(vec2(tile, tile + 58.0), Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Open {} — {}", item.name, described(item))));
                        let response = response.on_hover_text(format!("{}
{}", item.name, described(item)));
                        let painter = ui.painter_at(rect);
                        crate::material::paint(ui.ctx(), &painter, rect, ui::CONTROL_RADIUS, crate::material::Role::Panel);
                        if response.hovered() { painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK); }
                        if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
                        let art = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2((tile - 16.0).max(8.0), (tile - 16.0).max(8.0)));
                        let cover = if ui.is_rect_visible(rect) { self.covers.get(ui.ctx(), item.cover.as_deref()) } else { None };
                        if let Some(texture) = cover {
                            painter.image(texture.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
                        } else {
                            painter.rect_filled(art, 0.0, ui::TRACK);
                            ui::paint_icon(&painter, Rect::from_center_size(art.center(), vec2(24.0, 24.0)), ui::Icon::Music, ui::SECONDARY, false);
                        }
                        let chars = ((tile - 16.0) / 7.8) as usize;
                        painter.text(pos2(rect.left() + 8.0, art.bottom() + 12.0), Align2::LEFT_TOP, crate::panels::truncate(&item.name, chars), ui::display_bold(13.0), ui::TEXT);
                        painter.text(pos2(rect.left() + 8.0, art.bottom() + 34.0), Align2::LEFT_TOP, crate::panels::truncate(&described(item), chars), ui::display(11.0), ui::SECONDARY);
                        if response.clicked() {
                            match item.kind {
                                Kind::Playlist => self.go(Source::Playlist { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
                                Kind::Album => self.go(Source::Album { id: item.id.clone(), name: item.name.clone(), uri: item.uri.clone() }, true),
                                Kind::Artist => self.go(Source::Artist { id: item.id.clone(), name: item.name.clone() }, true),
                                _ => {}
                            }
                        }
                    });
                }
            });
            ui.add_space(12.0);
        }
    }

    /// The words of the track playing: the line being sung in the accent and kept in
    /// the middle, the rest around it. A click on a line goes to it.
    fn lyrics(&mut self, ui: &mut Ui, theme: &Theme, now: &Now) {
        let Some(track) = &now.track else {
            ui::text(ui, "Nothing is playing.", ui::display(12.0), ui::LABEL);
            return;
        };
        if self.lyrics_asked.as_deref() != Some(track.uri.as_str()) {
            self.lyrics_asked = Some(track.uri.clone());
            self.lyrics_at = None;
            self.send(json!({ "cmd": "lyrics", "track": track.uri }));
        }
        let lyrics = self.shared.lock().unwrap().lyrics.clone().filter(|lyrics| lyrics.track == track.uri);
        let Some(lyrics) = lyrics else {
            ui::text(ui, "Looking for the words…", ui::display(12.0), ui::LABEL);
            return;
        };
        if let Some(why) = &lyrics.error {
            ui::text(ui, why, ui::display(12.0), ui::LABEL);
            return;
        }
        let current = lyrics.current(now.position_ms());
        if !lyrics.synced {
            ui::text(ui, "These words are not timed to the music.", ui::display(11.0), ui::LABEL);
            ui.add_space(8.0);
        }
        let moved = current != self.lyrics_at;
        self.lyrics_at = current;
        let mut seek = None;
        ScrollArea::vertical().id_salt("lyrics").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.add_space(ui.available_height() * 0.3);
            for (index, (at, text)) in lyrics.lines.iter().enumerate() {
                let sung = current == Some(index);
                let (font, ink) = match (lyrics.synced, sung, current.is_some_and(|current| index < current)) {
                    (false, _, _) => (ui::display(17.0), ui::TEXT),
                    (true, true, _) => (ui::display_bold(22.0), theme.main),
                    (true, false, true) => (ui::display(17.0), ui::LABEL),
                    (true, false, false) => (ui::display(17.0), ui::SECONDARY),
                };
                // A line with no words is a stretch of music.
                let shown = if text.is_empty() { "· · ·" } else { text.as_str() };
                let galley = ui.painter().layout(shown.to_string(), font, ink, ui.available_width() - 12.0);
                let (rect, response) = ui.allocate_exact_size(galley.size(), if lyrics.synced { Sense::click() } else { Sense::hover() });
                let ink = if response.hovered() && !sung { ui::TEXT } else { ink };
                ui.painter().galley(rect.min, galley, ink);
                if response.clicked() {
                    seek = Some(*at);
                }
                // Follow the singing, but only as it moves on: scrolling by hand is let be until then.
                if sung && moved {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
            }
            ui.add_space(ui.available_height().max(120.0) * 0.5);
        });
        if let Some(ms) = seek {
            self.send(json!({ "cmd": "seek", "ms": ms }));
        }
        // The line changes with the music, not with the pointer.
        if now.playing && lyrics.synced {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Music's software volume is independent of the desktop output volume.
    /// Keep it visible in both players so a quiet source is easy to identify.
    fn volume_control(&self, ui: &mut Ui, theme: &Theme, now: &Now) {
        ui.horizontal(|ui| {
            ui::text(ui, "Music", ui::display(12.0), ui::SECONDARY)
                .on_hover_text("des-music volume; separate from system volume");
            let width = (ui.available_width() - 44.0 - ui.spacing().item_spacing.x).max(20.0);
            if let Some(to) = ui::slider(ui, theme, width, now.volume as f32 / 100.0, "Music volume") {
                self.send(json!({ "cmd": "volume", "percent": (to * 100.0).round() as u8 }));
            }
            ui::text(ui, &format!("{}%", now.volume), ui::mono(11.0), ui::TEXT);
        });
    }

    /// The strip along the bottom of the view: the track, the controls, the position.
    fn transport(&mut self, ui: &mut Ui, theme: &Theme, now: &Now, height: f32) {
        let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        crate::material::paint(ui.ctx(), ui.painter(), strip, ui::CONTROL_RADIUS, crate::material::Role::Panel);
        ui.painter().rect_stroke(strip, ui::CONTROL_RADIUS, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
        let mut inner = ui.new_child(UiBuilder::new().max_rect(strip.shrink(10.0)).layout(Layout::top_down(Align::Min)));
        inner.set_clip_rect(strip.intersect(ui.clip_rect()));
        let ui = &mut inner;
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        let Some(track) = &now.track else {
            ui::text(ui, "Your next listen starts here", ui::display_bold(16.0), ui::TEXT);
            ui::text(ui, "Choose a playlist, album or song to start playback.", ui::display(12.0), ui::SECONDARY);
            return;
        };
        ui.horizontal(|ui| {
            self.draw_cover(ui, 44.0, track.cover.as_deref());
            ui.vertical(|ui| {
                let fit = (ui.available_width() / 8.5).max(0.0) as usize;
                ui::text(ui, &crate::panels::truncate(&track.name, fit), ui::display_bold(15.0), ui::TEXT).on_hover_text(&track.name);
                ui::text(ui, &crate::panels::truncate(&track.artists, fit), ui::display(12.0), ui::SECONDARY).on_hover_text(&track.artists);
            });
        });
        ui.horizontal_wrapped(|ui| {
            if control(ui, theme, Control::Previous, 30.0, false).clicked() { self.send(json!({ "cmd": "prev" })); }
            if control(ui, theme, if now.playing { Control::Pause } else { Control::Play }, 32.0, true).clicked() { self.send(json!({ "cmd": "toggle" })); }
            if control(ui, theme, Control::Next, 30.0, false).clicked() { self.send(json!({ "cmd": "next" })); }
            let state = now.device.as_deref().map(|device| format!("On {device}")).unwrap_or_else(|| if now.playing { "Playing on this desktop" } else { "Paused" }.to_string());
            if ui.add(egui::Button::new(egui::RichText::new(crate::panels::truncate(&state, ((ui.available_width() / 7.8) as usize).max(4))).font(ui::display(11.0))).frame(false)).on_hover_text("Choose playback device").clicked() {
                self.go(Source::Devices, false); self.devices_asked = None;
            }
        });
        ui.horizontal(|ui| {
            ui::text(ui, &clock(now.position_ms()), ui::mono(11.0), ui::SECONDARY);
            let width = (ui.available_width() - 54.0).max(12.0);
            if let Some(to) = ui::slider(ui, theme, width, now.position_ms() as f32 / track.duration_ms.max(1) as f32, "Playback position") {
                if ui.input(|i| i.pointer.any_released()) || !ui.input(|i| i.pointer.any_down()) { self.send(json!({ "cmd": "seek", "ms": (to * track.duration_ms as f32) as u32 })); }
            }
            ui::text(ui, &clock(track.duration_ms), ui::mono(11.0), ui::SECONDARY);
        });
        ui.horizontal_wrapped(|ui| {
            if ui::chip(ui, theme, "SHUFFLE", None, now.shuffle).clicked() { self.send(json!({ "cmd": "shuffle", "on": !now.shuffle })); }
            let (label, next) = match now.repeat.as_str() { "all" => ("REPEAT", "one"), "one" => ("REPEAT 1", "off"), _ => ("REPEAT", "all") };
            if ui::chip(ui, theme, label, None, now.repeat != "off").clicked() { self.send(json!({ "cmd": "repeat", "mode": next })); }
            let liked = now.liked == Some(true);
            if ui::chip(ui, theme, if liked { "LIKED" } else { "LIKE" }, None, liked).clicked() { self.send(json!({ "cmd": "like", "on": !liked })); }
            ui.menu_button(egui::RichText::new("MORE").font(ui::display(11.0)), |ui| {
                if ui.button("Lyrics").clicked() { self.go(Source::Lyrics, true); self.lyrics_at = None; ui.close_menu(); }
                if ui.button("Devices").clicked() { self.go(Source::Devices, true); self.devices_asked = None; ui.close_menu(); }

            });
        });
        self.volume_control(ui, theme, now);
        if now.playing { ui.ctx().request_repaint_after(Duration::from_millis(500)); }
    }

}

/// Read only the explicitly selected image; encode a bounded JPEG for Spotify.
fn playlist_cover(path: &str) -> Result<String,String> {
    use base64::Engine;
    let file=std::fs::File::from(rustix::fs::open(path,rustix::fs::OFlags::RDONLY|rustix::fs::OFlags::NONBLOCK|rustix::fs::OFlags::CLOEXEC,rustix::fs::Mode::empty()).map_err(|e|e.to_string())?);
    let metadata=file.metadata().map_err(|e|e.to_string())?;
    if !metadata.is_file() || metadata.len()>20*1024*1024{return Err("Choose an image smaller than 20 MB.".into());}
    let mut reader=image::ImageReader::new(std::io::BufReader::new(file)).with_guessed_format().map_err(|e|e.to_string())?;
    let mut limits=image::Limits::default();limits.max_image_width=Some(8192);limits.max_image_height=Some(8192);limits.max_alloc=Some(128*1024*1024);reader.limits(limits);
    let image=reader.decode().map_err(|e|format!("Cannot read image: {e}"))?.thumbnail(512,512).to_rgb8();
    let mut jpeg=Vec::new();image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg,80).encode_image(&image).map_err(|e|e.to_string())?;
    let encoded=base64::engine::general_purpose::STANDARD.encode(jpeg);
    if encoded.len()>256*1024{return Err("This cover is too detailed; choose a simpler or smaller image.".into());}
    Ok(encoded)
}

/// Finite right-panel playback reservation, including its internal spacing.
pub const PANEL_HEIGHT: f32 = 136.0;

/// A track near its end is no warning, so progress never shows the caution colour.
const CAUTION_FREE: f32 = ui::CAUTION_ABOVE;

/// A row in the left list. Returns true when clicked.
fn side_row(ui: &mut Ui, theme: &Theme, name: &str, chosen: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, ui.is_enabled(), name));
    let painter = ui.painter_at(rect);
    if chosen {
        painter.rect_filled(rect, ui::CONTROL_RADIUS, theme.tint());
        painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK);
    }
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
    let (font, ink) = if chosen { (ui::display_bold(13.0), ui::TEXT) } else { (ui::display(13.0), ui::TEXT) };
    painter.text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, crate::panels::truncate(name, 26), font, ink);
    response.clicked()
}

/// What was clicked on a row: the row itself, or its way into the queue.
#[derive(Clone, Copy, PartialEq)]
enum Picked {
    Nothing,
    Row,
    Queue,
    /// The album a track is on.
    Album,
    /// Who a track is by.
    Artist,
    Save,
    RemoveSaved,
    AddToPlaylist,
    RemoveFromPlaylist,
    MoveUp,
    MoveDown,
}

/// What a row says in its second column: who a track is by, or what else the row is.
fn described(item: &Item) -> String {
    match item.kind {
        Kind::Track => item.by.clone(),
        Kind::Playlist if item.by.is_empty() => "playlist".to_string(),
        Kind::Playlist => format!("playlist by {}", item.by),
        Kind::Album if item.by.is_empty() => "album".to_string(),
        Kind::Album => format!("album by {}", item.by),
        Kind::Artist => "artist".to_string(),
        Kind::Device if item.here => format!("this machine · {}", item.by.to_lowercase()),
        Kind::Device => item.by.to_lowercase(),
        Kind::Show=>"podcast".into(), Kind::Episode=>format!("episode · {}",item.by),
        Kind::Audiobook=>format!("audiobook · {}",item.by),Kind::Chapter=>"chapter".into(),
    }
}

/// How tall a row of a list is.
const ROW: f32 = 44.0;

/// Chapters can use episode URIs. Prefer Spotify's canonical link, not a guessed chapter path.
fn spotify_item_url(item: &Item) -> Option<String> {
    if item.external_url.starts_with("https://open.spotify.com/") && item.external_url.len()<16*1024 && !item.external_url.chars().any(|c|c.is_whitespace()||c.is_control()) {
        return Some(item.external_url.clone());
    }
    let parts:Vec<_>=item.uri.split(':').collect();
    if parts.len()==3 && parts[0]=="spotify" && matches!(parts[1],"track"|"episode"|"audiobook"|"chapter") && !parts[2].is_empty() && parts[2].bytes().all(|b|b.is_ascii_alphanumeric()) {
        Some(format!("https://open.spotify.com/{}/{}",parts[1],parts[2]))
    }else{None}
}

/// Use visible neighbours' original positions; hidden entries need not be adjacent.
fn playlist_move_target(items: &[Item], index: usize, up: bool) -> Option<u32> {
    items.get(index)?.position?;
    if up {items.get(index.checked_sub(1)?)?.position}
    else {items.get(index+1)?.position?.checked_add(1)}
}

/// A track, or a playlist, album or artist to open. `queueable` rows carry a way to add
/// them to what plays next, and say `queued` for a moment after.
fn track_row(ui: &mut Ui, theme: &Theme, item: &Item, cover: Option<&TextureHandle>, playing: bool, queueable: bool, queued: bool, playlist: bool, move_up: bool, move_down: bool) -> Picked {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("{} — {}", item.name, described(item))));
    let mut menu_pick=None;
    response.context_menu(|ui| {
        if !matches!(item.kind,Kind::Device|Kind::Chapter) {
            if ui.button("Save / follow").clicked(){menu_pick=Some(Picked::Save);ui.close_menu();}
            if ui.button("Remove / unfollow").clicked(){menu_pick=Some(Picked::RemoveSaved);ui.close_menu();}
        }
        if matches!(item.kind,Kind::Track|Kind::Episode) && ui.button("Add to playlist…").clicked(){menu_pick=Some(Picked::AddToPlaylist);ui.close_menu();}
        if playlist && item.position.is_some() {
            if ui.button("Remove matching items from playlist").clicked(){menu_pick=Some(Picked::RemoveFromPlaylist);ui.close_menu();}
            if ui.add_enabled(move_up,egui::Button::new("Move up")).clicked(){menu_pick=Some(Picked::MoveUp);ui.close_menu();}
            if ui.add_enabled(move_down,egui::Button::new("Move down")).clicked(){menu_pick=Some(Picked::MoveDown);ui.close_menu();}
        }
    });
    if let Some(picked)=menu_pick{return picked;}
    // Under the pointer, the row offers to queue the track where its length is.
    let mut add = None;
    let track = matches!(item.kind, Kind::Track | Kind::Episode);
    if queueable && track && (queued || response.has_focus() || ui.memory(|memory| memory.has_focus(ui.id().with("queue"))) || ui.rect_contains_pointer(rect)) {
        let spot = Rect::from_min_max(pos2(rect.right() - 92.0, rect.top()), rect.right_bottom());
        let adding = ui.interact(spot, ui.id().with("queue"), Sense::click());
        adding.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Add to queue"));
        add = Some((spot, adding));
    }
    let painter = ui.painter_at(rect);
    if response.hovered() {
        painter.rect_filled(rect, ui::CONTROL_RADIUS, ui::TRACK);
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ui::TRACK));
    let ink = ui::TEXT;
    if playing { painter.rect_filled(rect, ui::CONTROL_RADIUS, theme.tint()); }
    let mark = Rect::from_center_size(rect.left_center() + vec2(10.0, 0.0), vec2(6.0, 6.0));
    if playing {
        painter.rect_filled(mark, 0.0, theme.main);
    }
    if response.has_focus() { painter.rect_stroke(rect, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
    // Its picture, where it has one; a device has none, and none is drawn for it.
    let lead = if item.kind == Kind::Device { 26.0 } else { 64.0 };
    if item.kind != Kind::Device {
        let square = Rect::from_center_size(rect.left_center() + vec2(40.0, 0.0), vec2(32.0, 32.0));
        match cover {
            Some(texture) => {
                painter.image(texture.id(), square, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => {
                painter.rect_filled(square, 0.0, ui::PANEL);
                painter.rect_stroke(square, 0.0, Stroke::new(1.0, ui::LINE), StrokeKind::Inside);
            }
        }
    }
    let width = rect.width();
    // How many characters fit in a column this many points wide.
    let chars = |points: f32| ((points - 14.0).max(0.0) / 8.0) as usize;
    let with_album = width > 620.0;
    let by_width = if with_album { width * 0.26 } else { width * 0.54 - 64.0 };
    painter.text(rect.left_center() + vec2(lead, 0.0), Align2::LEFT_CENTER, crate::panels::truncate(&item.name, chars(width * 0.46 - lead)), ui::display(13.0), ink);
    let by = described(item);
    // A track's artist and album are ways to go to them.
    let mut went = None;
    let mut link = |ui: &mut Ui, at: f32, text: String, quiet: egui::Color32, to: Picked, id: &str| {
        let from = rect.left_center() + vec2(at, 0.0);
        let accessible_name = format!("Open {id}: {text}");
        let galley = painter.layout_no_wrap(text, ui::display(12.0), quiet);
        let spot = Rect::from_min_size(from - vec2(0.0, galley.size().y / 2.0), galley.size()).expand2(vec2(2.0, 6.0));
        let over = ui.interact(spot, ui.id().with(id), Sense::click());
        over.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), accessible_name.clone()));
        if over.has_focus() { painter.rect_stroke(spot, ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
        if over.hovered() {
            painter.hline(spot.shrink2(vec2(2.0, 0.0)).x_range(), spot.bottom() - 5.0, Stroke::new(1.0, ui::TEXT));
        }
        painter.galley(from - vec2(0.0, galley.size().y / 2.0), galley, if over.hovered() { ui::TEXT } else { quiet });
        if over.clicked() {
            went = Some(to);
        }
    };
    let by_text = crate::panels::truncate(&by, chars(by_width));
    if track && !item.artist_id.is_empty() {
        link(ui, width * 0.46, by_text, ui::SECONDARY, Picked::Artist, "artist");
    } else {
        painter.text(rect.left_center() + vec2(width * 0.46, 0.0), Align2::LEFT_CENTER, by_text, ui::display(12.0), ui::SECONDARY);
    }
    if with_album {
        let album_text = crate::panels::truncate(&item.album, chars(width * 0.28 - 64.0));
        if track && !item.album_id.is_empty() {
            link(ui, width * 0.72, album_text, ui::LABEL, Picked::Album, "album");
        } else {
            painter.text(rect.left_center() + vec2(width * 0.72, 0.0), Align2::LEFT_CENTER, album_text, ui::display(12.0), ui::LABEL);
        }
    }
    if let Some(went) = went {
        return went;
    }
    if let Some((spot, adding)) = add {
        let (label, ink) = match (queued, adding.hovered()) {
            (true, _) => ("QUEUED", theme.main),
            (false, true) => ("+ QUEUE", ui::TEXT),
            (false, false) => ("+ QUEUE", ui::SECONDARY),
        };
        painter.text(spot.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, label, ui::display_bold(11.0), ink);
        if adding.has_focus() { painter.rect_stroke(spot.shrink(1.0), ui::CONTROL_RADIUS, Stroke::new(1.0, theme.main), StrokeKind::Inside); }
        if adding.clicked() {
            return Picked::Queue;
        }
        return if response.clicked() { Picked::Row } else { Picked::Nothing };
    }
    let end = match item.kind {
        Kind::Track | Kind::Episode => clock(item.duration_ms),
        Kind::Chapter=>"SPOTIFY ↗".into(),
        Kind::Device if item.active => "PLAYING".to_string(),
        Kind::Device => "SWITCH TO".to_string(),
        _ => "OPEN".to_string(),
    };
    painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, end, if track { ui::mono(12.0) } else { ui::display(12.0) }, ui::SECONDARY);
    if response.clicked() { Picked::Row } else { Picked::Nothing }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_workers_start_only_after_an_artwork_request() {
        let ctx = egui::Context::default();
        let mut covers = Covers::new(&ctx);
        assert!(covers.pending_workers.is_some());
        covers.get(&ctx, None);
        assert!(covers.pending_workers.is_some());
        covers.get(&ctx, Some("fixture-without-network"));
        assert!(covers.pending_workers.is_none());
    }

    #[test]
    fn cover_cache_evicts_least_recently_used_and_ignores_stale_results() {
        let (wanted, _requests) = channel();
        let (done, ready) = channel();
        let mut covers = Covers { wanted, ready, pending_workers: None, held: HashMap::new(), recent: VecDeque::new() };
        let ctx = egui::Context::default();
        for index in 0..COVER_CACHE_LIMIT {
            covers.get(&ctx, Some(&format!("cover-{index}")));
        }
        covers.get(&ctx, Some("cover-0"));
        covers.get(&ctx, Some("extra"));
        assert_eq!(covers.held.len(), COVER_CACHE_LIMIT);
        assert!(covers.held.contains_key("cover-0"));
        assert!(!covers.held.contains_key("cover-1"));
        done.send(("cover-1".into(), Some(egui::ColorImage::new([1, 1], egui::Color32::WHITE)))).unwrap();
        covers.get(&ctx, None);
        assert!(!covers.held.contains_key("cover-1"));
        assert_eq!(covers.held.len(), COVER_CACHE_LIMIT);
    }

    /// UI fixtures never start the service, fetch artwork, or touch a Spotify account.
    fn fixture(now: Now) -> Music {
        let (commands, _) = channel();
        let (wanted, _) = channel();
        let (_, ready) = channel();
        Music {
            open: true,
            shared: Arc::new(Mutex::new(Shared {
                link: Link::Connected, now, lists: HashMap::from([("playlists:".into(), (Vec::new(), None))]),
                more: Default::default(), next: HashMap::new(), snapshots: HashMap::new(), edits: HashMap::new(), saved: HashMap::new(), details: HashMap::new(), lyrics: None, problem: None, sign_in: None, refresh: false, cancel: false,
            })),
            commands, source: Source::Home, query: String::new(), browser_open: None, editor: None, add_to: None, client_id: String::new(),
            queue_as_of: None, queued: None, asked: Vec::new(), asked_further: None, trail: Vec::new(),
            covers: Covers { wanted, ready, pending_workers: None, held: HashMap::new(), recent: VecDeque::new() }, devices_asked: None, lyrics_asked: None, lyrics_at: None,
        }
    }

    #[test]
    fn escape_closes_an_open_popup_before_closing_music() {
        let ctx = egui::Context::default(); ui::install_fonts(&ctx, None, None);
        let theme = Theme::load("signal"); let mut music = fixture(Now::parse(&json!({})));
        ctx.memory_mut(|memory| memory.open_popup(egui::Id::new("test-music-popup")));
        let key = || egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0));
        let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), events: vec![key()], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| music.show(ui, screen, &theme));
        });
        assert!(music.open); assert!(!ctx.memory(|memory| memory.any_popup_open()));
        let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), events: vec![key()], ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| music.show(ui, screen, &theme));
        });
        assert!(!music.open);
    }

    #[test]
    fn compact_playback_reserves_finite_height_at_narrow_panel_widths() {
        let theme = Theme::parse(include_str!("../../themes/signal.json")).unwrap();
        for width in [192.0, 240.0, 292.0] {
            for playing in [false, true] {
                let now = Now::parse(&json!({"playing": playing, "track": {
                    "uri": "fixture", "name": "A very long track title that should stay inside the panel",
                    "artists": "An artist with a long display name", "duration_ms": 200000
                }}));
                let mut music = fixture(now);
                assert_eq!(music.preferred_panel_height(), PANEL_HEIGHT);
                let ctx = egui::Context::default();
                ui::install_fonts(&ctx, None, None);
                ctx.enable_accesskit();
                let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0));
                let rendered = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        // CentralPanel expands its min_rect to the whole screen before
                        // our callback. Measure a fresh content child, as the real
                        // right-panel reservation does, rather than that 600px frame.
                        let mut probe = ui.new_child(UiBuilder::new().id_salt("playback-budget-probe").max_rect(screen).layout(Layout::top_down(Align::Min)));
                        let start = probe.cursor().min;
                        music.panel(&mut probe, &theme);
                        assert!(probe.min_rect().width() <= width + 0.5, "panel expanded at {width}px: {:?}", probe.min_rect());
                        assert!(probe.min_rect().bottom() - start.y <= PANEL_HEIGHT + 0.5, "panel exceeded its reservation: {:?}", probe.min_rect());
                    });
                });
                let tree = rendered.platform_output.accesskit_update.expect("music accessibility tree");
                let bounds = tree.nodes.iter().find_map(|(_, node)|
                    (node.label() == Some("Music volume")).then(|| node.bounds()).flatten())
                    .expect("music volume stays visible outside menus");
                assert!(bounds.x0 >= 0.0 && bounds.x1 <= width as f64);
                assert!(bounds.y1 <= PANEL_HEIGHT as f64, "volume must fit the reserved panel");
            }
        }
        let mut empty = fixture(Now::default());
        assert_eq!(empty.preferred_panel_height(), 0.0);
        empty.source = Source::Home;
        assert_eq!(empty.source.key(), "playlists:");
    }

    #[test]
    fn reads_what_the_service_says() {
        let now = Now::parse(&json!({"type": "state", "playing": false, "position_ms": 61000, "volume": 80, "shuffle": true,
            "track": {"uri": "spotify:track:1", "name": "So What", "artists": "Miles Davis", "album": "Kind of Blue", "duration_ms": 562000, "cover": null}}));
        assert_eq!(now.track.as_ref().unwrap().name, "So What");
        assert_eq!(now.position_ms(), 61000, "paused: the position stands still");
        assert!(now.shuffle);
        assert_eq!(Now::parse(&json!({"type": "state", "track": null})).track, None);

        let items = parse_items(&json!({"items": [
            {"kind": "track", "uri": "spotify:track:1", "name": "So What", "artists": "Miles Davis", "album": "Kind of Blue", "duration_ms": 562000},
            {"kind": "playlist", "uri": "spotify:playlist:p", "id": "p", "name": "Focus", "owner": "ada"},
            {"kind": "track", "name": "no address"}]}));
        assert_eq!(items.len(), 2);
        assert!(items[0].kind == Kind::Track && items[0].by == "Miles Davis");
        assert!(items[1].kind == Kind::Playlist && items[1].by == "ada" && items[1].id == "p");
        let more = parse_items(&json!({"items": [
            {"kind": "album", "uri": "spotify:album:a", "id": "a", "name": "Kind of Blue", "artists": "Miles Davis", "album": "1959"},
            {"kind": "artist", "uri": "spotify:artist:r", "id": "r", "name": "Miles Davis"},
            {"kind": "track", "uri": "spotify:track:t", "name": "So What", "artists": "Miles Davis", "album_id": "a", "artist_id": "r"},
        ]}));
        assert_eq!(described(&more[0]), "album by Miles Davis");
        assert_eq!(described(&more[1]), "artist");
        assert_eq!((more[2].album_id.as_str(), more[2].artist_id.as_str()), ("a", "r"));
        assert_eq!(Source::Liked.further(50), Some(json!({"cmd": "liked", "offset": 50})));
        assert_eq!(Source::Album { id: "a".into(), name: "n".into(), uri: "u".into() }.further(50), None, "an album comes whole");
        assert_eq!(Source::Album { id: "a".into(), name: "n".into(), uri: "spotify:album:a".into() }.context(), Some("spotify:album:a".to_string()));
    }

    #[test]
    fn finds_the_line_being_sung() {
        let lyrics = Lyrics::parse(&json!({"type": "lyrics", "track": "spotify:track:t", "synced": true, "error": null,
            "lines": [{"ms": 1200, "text": "So what"}, {"ms": 5400, "text": ""}, {"ms": 9000, "text": "and again"}]}));
        assert_eq!(lyrics.lines.len(), 3);
        assert_eq!(lyrics.current(0), None, "before the first line");
        assert_eq!(lyrics.current(1200), Some(0));
        assert_eq!(lyrics.current(8999), Some(1));
        assert_eq!(lyrics.current(600_000), Some(2));
        let plain = Lyrics { synced: false, ..lyrics.clone() };
        assert_eq!(plain.current(5000), None, "untimed words have no line being sung");
        let none = Lyrics::parse(&json!({"type": "lyrics", "track": "t", "synced": false, "lines": [], "error": "Spotify has no words for this track."}));
        assert!(none.lines.is_empty() && none.error.is_some());
    }

    #[test]
    fn a_playing_track_moves_on_but_not_past_its_end() {
        let mut now = Now::parse(&json!({"playing": true, "position_ms": 1000,
            "track": {"uri": "u", "name": "n", "duration_ms": 1200}}));
        now.at = Some(Instant::now() - Duration::from_secs(5));
        assert_eq!(now.position_ms(), 1200);
    }

    #[test]
    fn formats_times_and_names_lists() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(61_000), "1:01");
        assert_eq!(clock(562_000), "9:22");
        assert_eq!(Source::Liked.context().as_deref(), Some("liked"));
        assert_eq!(Source::Search("x".into()).context(), None);
        assert_eq!(Source::Playlist { id: "p".into(), name: "n".into(), uri: "spotify:playlist:p".into() }.key(), "tracks:p");
    }

    #[test]
    fn search_pages_use_server_offsets_and_ignore_duplicate_pages() {
        let music=fixture(Now::default());let mut shared=music.shared.lock().unwrap();
        let item=|id:&str|json!({"kind":"track","uri":format!("spotify:track:{id}"),"name":id});
        shared.receive_list(&json!({"kind":"search","key":"sample","items":[item("a"),item("b")],"offset":0,"more":true,"next_offset":10}));
        shared.receive_list(&json!({"kind":"search","key":"sample","items":[item("c")],"offset":10,"more":true,"next_offset":20}));
        shared.receive_list(&json!({"kind":"search","key":"sample","items":[item("c")],"offset":10,"more":true,"next_offset":20}));
        assert_eq!(shared.lists["search:sample"].0.len(),3);
        assert_eq!(shared.next["search:sample"]["offset"],20);
        shared.receive_list(&json!({"kind":"search","key":"sample","items":[],"offset":20,"error":"Spotify rate limit","more":false}));
        assert_eq!(shared.lists["search:sample"].0.len(),3);
        assert!(shared.lists["search:sample"].1.is_some());
    }

    #[test]
    fn new_destinations_preserve_spoken_content_and_range_identity() {
        let rows=parse_items(&json!({"items":[{"kind":"episode","uri":"spotify:episode:a"},{"kind":"audiobook","uri":"spotify:audiobook:b"},{"kind":"chapter","uri":"spotify:chapter:c"}]}));
        assert_eq!(rows.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![Kind::Episode,Kind::Audiobook,Kind::Chapter]);
        let source=Source::Browse{kind:"top_tracks".into(),key:String::new(),name:"Top tracks".into(),range:"short_term".into()};
        assert_eq!(source.key(),"top_tracks:short_term");assert_eq!(source.request()["range"],"short_term");assert!(source.further(50).is_some());
    }

    #[test]
    fn cover_upload_converts_png_to_bounded_jpeg() {
        use base64::Engine;
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("cover.png");
        image::RgbImage::from_pixel(800,400,image::Rgb([40,100,180])).save(&path).unwrap();
        let encoded=playlist_cover(path.to_str().unwrap()).unwrap();assert!(encoded.len()<=256*1024);
        let bytes=base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(),image::ImageFormat::Jpeg);
        let decoded=image::load_from_memory(&bytes).unwrap();assert_eq!((decoded.width(),decoded.height()),(512,256));
        assert!(playlist_cover(directory.path().to_str().unwrap()).is_err());
    }


    #[test]
    fn unavailable_pages_advance_using_server_offsets_and_preserve_snapshot() {
        let mut music=fixture(Now::default());let source=Source::Playlist{id:"abc".into(),name:"Playlist".into(),uri:"spotify:playlist:abc".into()};
        let (commands,received)=channel();music.commands=commands;
        music.shared.lock().unwrap().receive_list(&json!({"kind":"tracks","key":"abc","items":[],"offset":0,"more":true,"next_offset":100,"snapshot":"version1"}));
        let first=music.page_token(&source,0);assert_eq!(first.1,100);
        music.load_more(&source,0);
        let request:Value=serde_json::from_str(&received.recv().unwrap()).unwrap();assert_eq!(request["offset"],100);assert_eq!(request["snapshot"],"version1");
        music.load_more(&source,0);assert!(received.try_recv().is_err(),"an in-flight page must not be requested twice");
        music.shared.lock().unwrap().receive_list(&json!({"kind":"tracks","key":"abc","items":[],"offset":100,"more":true,"next_offset":200,"snapshot":"version1"}));
        assert_ne!(first,music.page_token(&source,0));
    }

    #[test]
    fn empty_filtered_page_keeps_load_more_visible() {
        let mut music=fixture(Now::default());music.source=Source::Liked;
        music.shared.lock().unwrap().receive_list(&json!({"kind":"liked","key":"","items":[],"offset":0,"more":true,"next_offset":50}));
        let ctx=egui::Context::default();ui::install_fonts(&ctx,None,None);ctx.enable_accesskit();
        let output=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.0,0.0),vec2(1000.0,800.0))),..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{music.library(ui,&Theme::load("signal"),&Now::default());});
        });
        let tree=output.platform_output.accesskit_update.unwrap();
        assert!(tree.nodes.iter().any(|(_,node)|node.label()==Some("LOAD MORE")),"filtered pages must not hide later available entries");
    }

    #[test]
    fn chapter_links_follow_spotify_episode_identity_and_canonical_urls() {
        let items=parse_items(&json!({"items":[{"kind":"chapter","uri":"spotify:episode:abc","external_url":"https://open.spotify.com/episode/canonical"},{"kind":"chapter","uri":"spotify:episode:def","external_url":"https://open.spotify.com.evil.test/steal"}]}));
        assert_eq!(spotify_item_url(&items[0]).as_deref(),Some("https://open.spotify.com/episode/canonical"));
        assert_eq!(spotify_item_url(&items[1]).as_deref(),Some("https://open.spotify.com/episode/def"));
    }

    #[test]
    fn reorder_targets_visible_neighbours_across_unavailable_items() {
        let items=parse_items(&json!({"items":[{"uri":"spotify:track:a","position":2},{"uri":"spotify:track:b","position":5},{"uri":"spotify:track:c","position":9}]}));
        assert_eq!(playlist_move_target(&items,1,true),Some(2));assert_eq!(playlist_move_target(&items,1,false),Some(10));
        assert_eq!(playlist_move_target(&items,0,true),None);assert_eq!(playlist_move_target(&items,2,false),None);
    }

    #[test]
    fn failed_and_timed_out_edits_preserve_draft_and_ignore_other_submissions() {
        let music=fixture(Now::default());let mut shared=music.shared.lock().unwrap();
        let now=Instant::now();let mut editor=PlaylistEditor{name:"Keep my draft".into(),..Default::default()};
        editor.begin(&mut shared,"playlist_save",now);let old=editor.pending.clone().unwrap();
        shared.receive_edit(&json!({"type":"problem","operation":old,"text":"Permission denied"}));
        assert!(!editor.finish_pending(&mut shared,now));assert_eq!(editor.name,"Keep my draft");assert_eq!(editor.error.as_deref(),Some("Permission denied"));assert!(editor.pending.is_none());
        editor.begin(&mut shared,"playlist_save",now);let current=editor.pending.clone().unwrap();
        shared.receive_edit(&json!({"type":"changed","operation":old}));assert!(!editor.finish_pending(&mut shared,now));assert!(editor.pending.is_some());
        shared.receive_edit(&json!({"type":"changed","operation":current}));assert!(editor.finish_pending(&mut shared,now));
        editor.begin(&mut shared,"playlist_save",now);assert!(!editor.finish_pending(&mut shared,now+Duration::from_secs(36)));assert!(editor.pending.is_none());assert_eq!(editor.name,"Keep my draft");
    }

    #[test]
    fn refresh_clears_permissions_and_pagination_without_losing_edit_results() {
        let music=fixture(Now::default());let mut shared=music.shared.lock().unwrap();
        shared.saved.insert("spotify:album:a".into(),true);shared.details.insert("abc".into(),json!({"owner":true}));shared.snapshots.insert("tracks:abc".into(),"old".into());
        shared.receive_edit(&json!({"type":"changed","operation":"playlist_save:abc:1"}));shared.invalidate();
        assert!(shared.saved.is_empty()&&shared.details.is_empty()&&shared.snapshots.is_empty());assert_eq!(shared.edits["playlist_save:abc:1"],Ok(()));
    }

    #[test]
    fn cover_picker_rejects_a_named_pipe_without_waiting_for_a_writer() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("cover.png");
        rustix::fs::mknodat(rustix::fs::CWD,&path,rustix::fs::FileType::Fifo,rustix::fs::Mode::RUSR|rustix::fs::Mode::WUSR,0).unwrap();
        assert!(playlist_cover(path.to_str().unwrap()).is_err());
    }

    #[test]
    #[ignore = "renders isolated native fixtures under Xvfb; needs DESUI_MUSIC_PREVIEW"]
    fn native_music_preview() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        struct Preview { music: Music, frames: usize, output: PathBuf, mode: String }
        impl eframe::App for Preview {
            fn update(&mut self,ctx:&egui::Context,_:&mut eframe::Frame) {
                for event in ctx.input(|i|i.events.clone()) {if let egui::Event::Screenshot{image,..}=event {
                    let bytes:Vec<u8>=image.pixels.iter().flat_map(|p|p.to_array()).collect();
                    image::save_buffer(&self.output,&bytes,image.size[0] as u32,image.size[1] as u32,image::ColorType::Rgba8).unwrap();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }}
                let theme=Theme::load("signal");
                egui::CentralPanel::default().show(ctx,|ui| {
                    if self.mode=="compact"{self.music.panel(ui,&theme);}else{let rect=ui.available_rect_before_wrap();self.music.show(ui,rect,&theme);}
                });
                self.frames+=1;
                if self.frames==4 {ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));}
                ctx.request_repaint();
            }
        }
        let output=PathBuf::from(std::env::var("DESUI_MUSIC_PREVIEW").expect("preview output"));
        let mode=std::env::var("DESUI_MUSIC_PREVIEW_MODE").unwrap_or_default();
        let width=std::env::var("DESUI_MUSIC_PREVIEW_WIDTH").ok().and_then(|s|s.parse::<f32>().ok()).unwrap_or(1100.0);
        let mut music=fixture(Now::parse(&json!({"volume":75,"playing":false,"track":{"uri":"spotify:track:fixture","name":"Sample track","artists":"Sample artist","duration_ms":240000}})));
        if mode.starts_with("editor") {music.editor=Some(PlaylistEditor{name:"Evening favourites".into(),description:"A personal playlist".into(),..Default::default()});if mode=="editor-error"{music.editor.as_mut().unwrap().error=Some("Spotify refused this edit. Your draft is preserved; reconnect the library to approve permissions.".into());}else if mode=="editor-pending"{music.editor.as_mut().unwrap().begin(&mut music.shared.lock().unwrap(),"playlist_save",Instant::now());}}
        else if mode!="compact" {music.source=Source::Browse{kind:"shows".into(),key:String::new(),name:"Podcasts".into(),range:"medium_term".into()};music.shared.lock().unwrap().lists.insert("shows:".into(),(parse_items(&json!({"items":[{"kind":"show","uri":"spotify:show:fixture","id":"fixture","name":"A sample podcast","artists":"Sample publisher"}]})),None));}
        let options=eframe::NativeOptions{viewport:egui::ViewportBuilder::default().with_inner_size([width,800.0]),event_loop_builder:Some(Box::new(|builder|{builder.with_x11().with_any_thread(true);})),..Default::default()};
        eframe::run_native("DEs-UI music fixture",options,Box::new(move |cc|{ui::install_fonts(&cc.egui_ctx,None,None);crate::apply_style(&cc.egui_ctx,&Theme::load("signal"));Ok(Box::new(Preview{music,frames:0,output,mode}))})).unwrap();
    }

}
