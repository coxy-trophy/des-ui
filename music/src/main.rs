//! The DEs-UI music service: signs in to Spotify, plays through librespot, and serves
//! the shell's music player over a Unix socket.
//!
//! It is a program of its own so that music keeps playing when the shell restarts.
//! The shell speaks to it in lines of JSON (see `protocol`); `des-music send` does the
//! same from a command line, which is how the keyboard's media keys reach it.

mod protocol;
mod web;

use librespot_connect::{ConnectConfig, LoadRequest, LoadRequestOptions, PlayingTrack, Spirc};
use librespot_core::{
    authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
};
use librespot_metadata::audio::UniqueFields;
use librespot_playback::{
    audio_backend,
    config::{AudioFormat, PlayerConfig},
    mixer::{self, MixerConfig},
    player::{Player, PlayerEvent},
};
use protocol::{Request, State, Track};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

/// What Spotify is asked to allow: playing here, and reading the user's library.
const SCOPES: [&str; 5] = [
    "streaming",
    "user-read-private",
    "user-library-read",
    "playlist-read-private",
    "playlist-read-collaborative",
];
/// Where the browser hands the sign-in back to this program.
const REDIRECT: &str = "http://127.0.0.1:8898/login";
/// The program that turns raw samples on its input into sound, through PipeWire.
const OUTPUT: &str = "pw-cat --playback --raw --format s16 --rate 44100 --channels 2 -";

const USAGE: &str = "\
Usage: des-music [COMMAND]

  (none)         Run the music service
  login          Sign in to Spotify, in your browser; needed once
  library [ID]   Connect your library, with the client ID of your own Spotify app
  logout         Forget the sign-in
  send <WORD>    Tell a running service to: toggle, next, prev";

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("des-ui/music")
}

fn open_cache() -> Result<Cache, String> {
    let dir = cache_dir();
    // Credentials and the volume are kept; audio is not, to spare the disk.
    Cache::new(Some(&dir), Some(&dir), None, None).map_err(|e| format!("cannot use {}: {e}", dir.display()))
}

/// Signs in through the browser and keeps the result for the service to use.
async fn login() -> Result<(), String> {
    let cache = open_cache()?;
    let config = SessionConfig::default();
    println!("Opening Spotify's sign-in page in your browser…");
    let client = librespot_oauth::OAuthClientBuilder::new(&config.client_id, REDIRECT, SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("Signed in. You can close this page and go back to DEs-UI.")
        .build()
        .map_err(|e| format!("cannot start the sign-in: {e}"))?;
    let token = client
        .get_access_token_async()
        .await
        .map_err(|e| format!("the sign-in did not complete: {e}"))?;
    // Connecting once turns the short-lived token into credentials that last.
    let session = Session::new(config, Some(cache));
    session
        .connect(Credentials::with_access_token(token.access_token), true)
        .await
        .map_err(|e| format!("Spotify did not accept the sign-in: {e}"))?;
    println!("Signed in as {}.", session.username());
    Ok(())
}

/// The cover to show for a track: the one closest to what the player draws.
fn pick_cover(covers: &[librespot_metadata::audio::item::CoverImage]) -> Option<String> {
    covers
        .iter()
        .min_by_key(|cover| (cover.width - 300).abs())
        .map(|cover| cover.url.clone())
}

struct Service {
    spirc: Spirc,
    session: Session,
    state: Mutex<(State, Instant)>,
    updates: broadcast::Sender<String>,
}

impl Service {
    /// The state as of now, with the position moved on by the time since it was set.
    fn snapshot(&self) -> State {
        let guard = self.state.lock().unwrap();
        let (state, since) = (&guard.0, guard.1);
        let mut now = state.clone();
        if now.playing {
            let length = now.track.as_ref().map_or(u32::MAX, |track| track.duration_ms);
            now.position_ms = now.position_ms.saturating_add(since.elapsed().as_millis() as u32).min(length);
        }
        now
    }

    fn publish(&self) {
        let _ = self.updates.send(self.snapshot().to_line());
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        {
            let mut guard = self.state.lock().unwrap();
            // Bring the position up to date before changing anything that it depends on.
            if guard.0.playing {
                guard.0.position_ms = guard.0.position_ms.saturating_add(guard.1.elapsed().as_millis() as u32);
            }
            guard.1 = Instant::now();
            change(&mut guard.0);
        }
        self.publish();
    }

    fn on_event(&self, event: PlayerEvent) {
        match event {
            PlayerEvent::TrackChanged { audio_item } => {
                let (artists, album) = match &audio_item.unique_fields {
                    UniqueFields::Track { artists, album, .. } => {
                        (artists.iter().map(|artist| artist.name.clone()).collect::<Vec<_>>().join(", "), album.clone())
                    }
                    UniqueFields::Episode { show_name, .. } => (show_name.clone(), String::new()),
                    UniqueFields::Local { artists, album, .. } => {
                        (artists.clone().unwrap_or_default(), album.clone().unwrap_or_default())
                    }
                };
                let track = Track {
                    uri: audio_item.uri.clone(),
                    name: audio_item.name.clone(),
                    artists,
                    album,
                    duration_ms: audio_item.duration_ms,
                    cover: pick_cover(&audio_item.covers),
                };
                self.update(|state| {
                    state.track = Some(track);
                    state.position_ms = 0;
                });
            }
            PlayerEvent::Playing { position_ms, .. } => self.update(|state| {
                state.playing = true;
                state.position_ms = position_ms;
            }),
            PlayerEvent::Paused { position_ms, .. } => self.update(|state| {
                state.playing = false;
                state.position_ms = position_ms;
            }),
            PlayerEvent::Seeked { position_ms, .. }
            | PlayerEvent::PositionCorrection { position_ms, .. }
            | PlayerEvent::PositionChanged { position_ms, .. } => self.update(|state| state.position_ms = position_ms),
            PlayerEvent::Stopped { .. } => self.update(|state| state.playing = false),
            PlayerEvent::VolumeChanged { volume } => {
                self.update(|state| state.volume = (volume as u32 * 100 / u16::MAX as u32) as u8)
            }
            PlayerEvent::ShuffleChanged { shuffle } => self.update(|state| state.shuffle = shuffle),
            _ => {}
        }
    }

    /// Carries out one request. Returns what to send back to the one who asked.
    async fn handle(&self, request: Request) -> Option<String> {
        let done = |outcome: Result<(), librespot_core::Error>| {
            if let Err(e) = outcome {
                eprintln!("des-music: {e}");
            }
            None
        };
        match request {
            Request::State => Some(self.snapshot().to_line()),
            Request::Toggle => done(self.spirc.play_pause()),
            Request::Next => done(self.spirc.next()),
            Request::Previous => done(self.spirc.prev()),
            Request::Seek(ms) => done(self.spirc.set_position_ms(ms)),
            Request::Volume(percent) => {
                done(self.spirc.set_volume((percent.min(100) as u32 * u16::MAX as u32 / 100) as u16))
            }
            Request::Shuffle(on) => done(self.spirc.shuffle(on)),
            Request::Play { context, track } => {
                let options = LoadRequestOptions {
                    start_playing: true,
                    playing_track: track.clone().filter(|_| context.is_some()).map(PlayingTrack::Uri),
                    ..Default::default()
                };
                let load = match (context, track) {
                    // The user's liked songs are a list only they have a name for.
                    (Some(context), _) if context == "liked" => LoadRequest::from_context_uri(
                        format!("spotify:user:{}:collection", self.session.username()),
                        options,
                    ),
                    (Some(context), _) => LoadRequest::from_context_uri(context, options),
                    (None, Some(track)) => LoadRequest::from_tracks(vec![track], options),
                    (None, None) => return None,
                };
                // Take over from whichever device was playing, then start.
                let _ = self.spirc.activate();
                done(self.spirc.load(load))
            }
            Request::Search(query) => Some(web::search(&self.session, &query).await),
            Request::Playlists => Some(web::playlists(&self.session).await),
            Request::Tracks(playlist) => Some(web::tracks(&self.session, &playlist).await),
            Request::Liked => Some(web::liked(&self.session).await),
            Request::Queue => Some(web::queue(&self.session).await),
            Request::Enqueue(track) => Some(web::enqueue(&self.session, &track).await),
        }
    }
}

async fn serve_client(service: Arc<Service>, stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut updates = service.updates.subscribe();
    let mut lines = BufReader::new(reader).lines();
    if writer.write_all(service.snapshot().to_line().as_bytes()).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Some(request) = Request::parse(&line) else { continue };
                if let Some(reply) = service.handle(request).await {
                    if writer.write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
            update = updates.recv() => {
                match update {
                    Ok(line) => {
                        if writer.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    // Fell behind: the next update carries everything anyway.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn serve() -> Result<(), String> {
    let path = protocol::socket_path();
    // One service at a time: a second would take the socket from under the first.
    if UnixStream::connect(&path).await.is_ok() {
        return Err("the music service is already running".to_string());
    }
    let cache = open_cache()?;
    let credentials = cache.credentials().ok_or("not signed in: run `des-music login` first")?;
    let session = Session::new(SessionConfig::default(), Some(cache));

    let mixer_builder = mixer::find(None).ok_or("no volume control is built in")?;
    let mixer = mixer_builder(MixerConfig::default()).map_err(|e| format!("cannot set up the volume: {e}"))?;
    let sink_builder = audio_backend::find(Some("subprocess".to_string())).ok_or("no audio output is built in")?;
    let player = Player::new(PlayerConfig::default(), session.clone(), mixer.get_soft_volume(), move || {
        sink_builder(Some(OUTPUT.to_string()), AudioFormat::S16)
    });
    let mut events = player.get_player_event_channel();

    let connect = ConnectConfig {
        name: "DEs-UI".to_string(),
        ..Default::default()
    };
    let (spirc, spirc_task) = Spirc::new(connect, session.clone(), credentials, player, mixer)
        .await
        .map_err(|e| format!("cannot connect to Spotify: {e}"))?;
    eprintln!("des-music: signed in as {}", session.username());

    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    let (updates, _) = broadcast::channel(32);
    let service = Arc::new(Service {
        spirc,
        session,
        state: Mutex::new((State::default(), Instant::now())),
        updates,
    });

    let for_events = service.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            for_events.on_event(event);
        }
    });
    let for_clients = service.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_client(for_clients.clone(), stream));
        }
    });

    // The service lasts as long as its connection to Spotify, or until told to stop.
    tokio::select! {
        () = spirc_task => eprintln!("des-music: the connection to Spotify ended"),
        _ = tokio::signal::ctrl_c() => {}
    }
    let _ = service.spirc.shutdown();
    let _ = std::fs::remove_file(&path);
    Ok(())
}

/// Sends one word to a running service, for scripts and the media keys.
async fn send(word: &str) -> Result<(), String> {
    let request = match word {
        "toggle" | "next" | "prev" => json!({ "cmd": word }),
        other => return Err(format!("unknown word '{other}'\n\n{USAGE}")),
    };
    let mut stream = UnixStream::connect(protocol::socket_path())
        .await
        .map_err(|_| "the music service is not running".to_string())?;
    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => serve().await,
        ["login"] => login().await,
        ["library", client] => connect_library(client).await,
        // Again, as the app already connected: for when more needs approving.
        ["library"] => match saved_client() {
            Some(client) => connect_library(&client).await,
            None => Err("give the client ID of your Spotify app: des-music library <ID>".to_string()),
        },
        ["logout"] => {
            let _ = std::fs::remove_file(web_token_file());
            std::fs::remove_file(cache_dir().join("credentials.json"))
                .map_err(|e| format!("there was no sign-in to forget: {e}"))
        }
        ["send", word] => send(word).await,
        ["-h" | "--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("unrecognised arguments\n\n{USAGE}")),
    };
    if let Err(why) = outcome {
        eprintln!("des-music: {why}");
        // Lets the shell tell "not signed in" from other failures.
        std::process::exit(if why.starts_with("not signed in") { 3 } else { 1 });
    }
}

/// Where the sign-in's own token is kept: it is what reads the library.
fn web_token_file() -> PathBuf {
    cache_dir().join("web-token.json")
}

/// What the Web API is asked to allow: reading the user's library, and reading and
/// adding to what plays next.
const LIBRARY_SCOPES: [&str; 6] = [
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-private",
    "user-library-read",
    "playlist-read-private",
    "playlist-read-collaborative",
];

/// The client ID the library was last connected with.
fn saved_client() -> Option<String> {
    let text = std::fs::read_to_string(web_token_file()).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?["client"].as_str().map(str::to_string)
}

fn save_web_token(client: &str, token: &librespot_oauth::OAuthToken) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let left = token.expires_at.saturating_duration_since(Instant::now());
    let expires = (SystemTime::now() + left).duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let text = json!({ "client": client, "access": token.access_token, "refresh": token.refresh_token, "expires": expires })
        .to_string();
    let path = web_token_file();
    // Written under another name first, readable by the user alone.
    let fresh = path.with_extension("new");
    let _ = std::fs::remove_file(&fresh);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&fresh)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
        .and_then(|()| std::fs::rename(&fresh, &path))
        .map_err(|e| format!("cannot keep the sign-in in {}: {e}", path.display()))
}

/// Signs in to the Web API as the user's own Spotify app. Spotify turns away library
/// requests made in the player's name, so reading the library needs an app of the
/// user's own; playing does not.
async fn connect_library(client: &str) -> Result<(), String> {
    if client.len() != 32 || !client.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("a client ID is 32 letters and digits, from your app's page on developer.spotify.com".to_string());
    }
    std::fs::create_dir_all(cache_dir()).map_err(|e| format!("cannot use {}: {e}", cache_dir().display()))?;
    println!("Opening Spotify's page in your browser, to let your app read your library…");
    let token = librespot_oauth::OAuthClientBuilder::new(client, REDIRECT, LIBRARY_SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("Library connected. You can close this page and go back to DEs-UI.")
        .build()
        .map_err(|e| format!("cannot start the sign-in: {e}"))?
        .get_access_token_async()
        .await
        .map_err(|e| format!("the sign-in did not complete: {e}"))?;
    save_web_token(client, &token)?;
    println!("Library connected.");
    Ok(())
}

pub const NO_QUEUE: &str = "The queue needs one more approval from Spotify. Run `des-music library` again.";

pub const NO_LIBRARY: &str = "Your library is not connected yet. Run `des-music library <client ID>` once; the README says where the ID comes from.";

/// A token for the Web API: the one from `library`, renewed when it runs out.
pub async fn web_token() -> Result<String, String> {
    let saved = std::fs::read_to_string(web_token_file())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .ok_or(NO_LIBRARY)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Some(access) = saved["access"].as_str().filter(|_| saved["expires"].as_u64().unwrap_or(0) > now + 60) {
        return Ok(access.to_string());
    }
    let (Some(client), Some(refresh)) = (saved["client"].as_str(), saved["refresh"].as_str()) else {
        return Err(NO_LIBRARY.to_string());
    };
    let token = librespot_oauth::OAuthClientBuilder::new(client, REDIRECT, LIBRARY_SCOPES.to_vec())
        .build()
        .map_err(|e| format!("cannot renew the library connection: {e}"))?
        .refresh_token_async(refresh)
        .await
        .map_err(|e| format!("Spotify would not renew the library connection ({e}). Run `des-music library` again."))?;
    // Spotify does not always send a new renewal token; the old one then still stands.
    let mut token = token;
    if token.refresh_token.is_empty() {
        token.refresh_token = refresh.to_string();
    }
    save_web_token(client, &token)?;
    Ok(token.access_token)
}
