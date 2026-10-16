//! The DEs-UI music service: signs in to Spotify, plays through librespot, and serves
//! the shell's music player over a Unix socket.
//!
//! It is a program of its own so that music keeps playing when the shell restarts.
//! The shell speaks to it in lines of JSON (see `protocol`); `des-music send` does the
//! same from a command line, which is how the keyboard's media keys reach it.

mod protocol;
mod ipc;
mod socket;
mod service;
use service::serve;
mod web;

use librespot_core::{
    authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

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

/// The name this player goes by among the user's devices: forty letters and digits,
/// the same at every start, so that it can be told apart from the others when
/// Spotify lists them. A service being tried out beside the real one gets its own.
fn device_id() -> String {
    let fresh = || {
        let mut bytes = [0u8; 20];
        let read = std::fs::File::open("/dev/urandom").and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes));
        if read.is_err() {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            bytes[..16].copy_from_slice(&nanos.to_le_bytes());
            bytes[16..].copy_from_slice(&std::process::id().to_le_bytes());
        }
        bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
    };
    if std::env::var_os("DES_MUSIC_SOCKET").is_some() {
        return fresh();
    }
    let path = cache_dir().join("device-id");
    let kept = std::fs::read_to_string(&path).ok().map(|id| id.trim().to_string());
    match kept.filter(|id| id.len() == 40 && id.chars().all(|c| c.is_ascii_hexdigit())) {
        Some(id) => id,
        None => {
            let id = fresh();
            let _ = std::fs::write(&path, &id);
            id
        }
    }
}

pub const NO_LIKING: &str = "Liking needs one more approval from Spotify.";

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

/// What the Web API is asked to allow: reading the user's library and liking tracks
/// in it, reading and steering what plays on any device, and adding to what plays next.
const LIBRARY_SCOPES: [&str; 7] = [
    "user-library-modify",
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
