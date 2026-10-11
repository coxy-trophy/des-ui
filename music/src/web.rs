//! The user's library and the catalogue, read from Spotify's Web API.
//!
//! Every function answers with a finished `list` line: on failure the list is empty
//! and says why, so that the player always has something to show.

use crate::protocol::{list_line, page_line};
use bytes::Bytes;
use http::{Method, Request};
use http_body_util::BodyExt;
use librespot_core::session::Session;
use serde_json::{Value, json};

const API: &str = "https://api.spotify.com/v1";

/// Makes text safe to put in an address.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

async fn get(session: &Session, path: &str) -> Result<Value, String> {
    call(session, Method::GET, path).await
}

async fn call(session: &Session, method: Method, path: &str) -> Result<Value, String> {
    let token = crate::web_token().await?;
    let request = Request::builder()
        .method(method)
        .uri(format!("{API}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Bytes::new())
        .map_err(|e| e.to_string())?;
    let response = session
        .http_client()
        .request(request)
        .await
        .map_err(|e| format!("Spotify could not be reached: {e}"))?;
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("Spotify's answer was cut short: {e}"))?
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = value["error"]["message"].as_str().unwrap_or("no reason given");
        return Err(format!("Spotify refused ({}): {detail}", status.as_u16()));
    }
    Ok(value)
}

/// One row for the player from a track as the Web API describes it.
pub fn track_item(track: &Value) -> Option<Value> {
    let uri = track["uri"].as_str()?;
    // Podcasts and local files come through the same lists; only tracks are shown.
    if !uri.starts_with("spotify:track:") {
        return None;
    }
    let artists: Vec<&str> = track["artists"]
        .as_array()
        .map(|artists| artists.iter().filter_map(|artist| artist["name"].as_str()).collect())
        .unwrap_or_default();
    Some(json!({
        "kind": "track",
        "uri": uri,
        "name": track["name"].as_str().unwrap_or("Untitled"),
        "artists": artists.join(", "),
        "album": track["album"]["name"].as_str().unwrap_or_default(),
        "duration_ms": track["duration_ms"].as_u64().unwrap_or(0),
        // For going from a track to where it comes from.
        "album_id": track["album"]["id"].as_str(),
        "artist_id": track["artists"][0]["id"].as_str(),
    }))
}

pub fn album_item(album: &Value) -> Option<Value> {
    let id = album["id"].as_str()?;
    let artists: Vec<&str> = album["artists"]
        .as_array()
        .map(|artists| artists.iter().filter_map(|artist| artist["name"].as_str()).collect())
        .unwrap_or_default();
    Some(json!({
        "kind": "album",
        "id": id,
        "uri": album["uri"].as_str().map(str::to_string).unwrap_or_else(|| format!("spotify:album:{id}")),
        "name": album["name"].as_str().unwrap_or("Untitled"),
        "artists": artists.join(", "),
        // The year is all of the date that is always there.
        "album": album["release_date"].as_str().and_then(|date| date.get(..4)).unwrap_or_default(),
        "count": album["total_tracks"].as_u64(),
    }))
}

pub fn artist_item(artist: &Value) -> Option<Value> {
    let id = artist["id"].as_str()?;
    Some(json!({
        "kind": "artist",
        "id": id,
        "uri": artist["uri"].as_str().map(str::to_string).unwrap_or_else(|| format!("spotify:artist:{id}")),
        "name": artist["name"].as_str().unwrap_or("Unknown"),
    }))
}

/// Whether a page says there is more after it.
fn has_more(page: &Value) -> bool {
    page["next"].as_str().is_some_and(|next| !next.is_empty())
}

pub fn playlist_item(playlist: &Value) -> Option<Value> {
    let id = playlist["id"].as_str()?;
    // The count moved from `tracks` to `items` in 2026; read whichever is there.
    let count = playlist["items"]["total"].as_u64().or_else(|| playlist["tracks"]["total"].as_u64());
    Some(json!({
        "kind": "playlist",
        "id": id,
        "uri": playlist["uri"].as_str().map(str::to_string).unwrap_or_else(|| format!("spotify:playlist:{id}")),
        "name": playlist["name"].as_str().unwrap_or("Untitled"),
        "owner": playlist["owner"]["display_name"].as_str().unwrap_or_default(),
        "count": count,
    }))
}

/// The tracks in a page of a playlist or of the liked songs, where each entry wraps
/// its track under `item` (since 2026) or `track` (before).
pub fn wrapped_tracks(page: &Value) -> Vec<Value> {
    page["items"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| track_item(if entry["item"].is_object() { &entry["item"] } else { &entry["track"] }))
                .collect()
        })
        .unwrap_or_default()
}

pub async fn search(session: &Session, query: &str) -> String {
    // Ten is the most a personal app may ask for at a time.
    let path = format!("/search?q={}&type=track,album,artist,playlist&limit=10", encode(query));
    match get(session, &path).await {
        Ok(found) => {
            let list = |key: &str| found[key]["items"].as_array().cloned().unwrap_or_default();
            let mut items: Vec<Value> = list("tracks").iter().filter_map(track_item).collect();
            items.extend(list("artists").iter().take(4).filter_map(artist_item));
            items.extend(list("albums").iter().filter_map(album_item));
            items.extend(list("playlists").iter().filter_map(playlist_item));
            list_line("search", query, items, None)
        }
        Err(why) => list_line("search", query, Vec::new(), Some(why)),
    }
}

/// Every playlist in the library, however many pages that takes (within reason).
pub async fn playlists(session: &Session) -> String {
    let mut items = Vec::new();
    for page_number in 0..20 {
        match get(session, &format!("/me/playlists?limit=50&offset={}", page_number * 50)).await {
            Ok(page) => {
                items.extend(page["items"].as_array().into_iter().flatten().filter_map(playlist_item));
                if !has_more(&page) {
                    break;
                }
            }
            // The first pages are worth having even when a later one fails.
            Err(_) if !items.is_empty() => break,
            Err(why) => return list_line("playlists", "", Vec::new(), Some(why)),
        }
    }
    list_line("playlists", "", items, None)
}

pub async fn tracks(session: &Session, playlist: &str, offset: u32) -> String {
    let id = encode(playlist);
    // The address was renamed in 2026; fall back to the old one for older accounts.
    let mut page = get(session, &format!("/playlists/{id}/items?limit=100&offset={offset}")).await;
    if page.is_err() {
        page = get(session, &format!("/playlists/{id}/tracks?limit=100&offset={offset}")).await;
    }
    match page {
        Ok(page) => page_line("tracks", playlist, wrapped_tracks(&page), None, offset, has_more(&page)),
        Err(why) => page_line("tracks", playlist, Vec::new(), Some(why), offset, false),
    }
}

/// The albums saved to the library.
pub async fn albums(session: &Session, offset: u32) -> String {
    match get(session, &format!("/me/albums?limit=50&offset={offset}")).await {
        Ok(page) => {
            let items = page["items"].as_array().into_iter().flatten().filter_map(|entry| album_item(&entry["album"])).collect();
            page_line("albums", "", items, None, offset, has_more(&page))
        }
        Err(why) => page_line("albums", "", Vec::new(), Some(why), offset, false),
    }
}

/// The tracks of an album, in order. Its tracks do not say which album they are on,
/// so that is filled in from the album itself.
pub async fn album(session: &Session, id: &str) -> String {
    let found = match get(session, &format!("/albums/{id}")).await {
        Ok(found) => found,
        Err(why) => return list_line("album", id, Vec::new(), Some(why)),
    };
    let on = json!({ "name": found["name"], "id": found["id"] });
    let fill = |track: &Value| {
        let mut track = track.clone();
        track["album"] = on.clone();
        track_item(&track)
    };
    let mut items: Vec<Value> = found["tracks"]["items"].as_array().into_iter().flatten().filter_map(fill).collect();
    // A long album comes in several pages.
    let mut more = has_more(&found["tracks"]);
    while more && items.len() < 500 {
        match get(session, &format!("/albums/{id}/tracks?limit=50&offset={}", items.len())).await {
            Ok(page) => {
                let before = items.len();
                items.extend(page["items"].as_array().into_iter().flatten().filter_map(fill));
                more = has_more(&page) && items.len() > before;
            }
            Err(_) => break,
        }
    }
    list_line("album", id, items, None)
}

/// An artist's albums and singles, newest first as Spotify gives them.
pub async fn artist(session: &Session, id: &str) -> String {
    let mut items: Vec<Value> = Vec::new();
    // Ten at a time is the most a personal app may ask for here.
    for page_number in 0..6 {
        let path = format!("/artists/{id}/albums?include_groups=album,single&limit=10&offset={}", page_number * 10);
        match get(session, &path).await {
            Ok(page) => {
                for item in page["items"].as_array().into_iter().flatten().filter_map(album_item) {
                    // The same album is often listed once for each market.
                    if !items.iter().any(|other| other["name"] == item["name"] && other["album"] == item["album"]) {
                        items.push(item);
                    }
                }
                if !has_more(&page) {
                    break;
                }
            }
            Err(_) if !items.is_empty() => break,
            Err(why) => return list_line("artist", id, Vec::new(), Some(why)),
        }
    }
    list_line("artist", id, items, None)
}

/// Why the queue cannot be read or added to, in words the player can act on.
fn queue_trouble(why: String) -> String {
    let unapproved = ["401", "403", "ermission", "scope"].iter().any(|sign| why.contains(sign));
    if unapproved { crate::NO_QUEUE.to_string() } else { why }
}

/// What plays next, as Spotify has it: the tracks queued by hand, then those that
/// follow in the list being played.
pub async fn queue(session: &Session) -> String {
    match get(session, "/me/player/queue").await {
        Ok(found) => {
            let items = found["queue"].as_array().map(|all| all.iter().filter_map(track_item).collect());
            list_line("queue", "", items.unwrap_or_default(), None)
        }
        Err(why) => list_line("queue", "", Vec::new(), Some(queue_trouble(why))),
    }
}

/// Adds a track to what plays next, and answers with the queue as it then stands.
pub async fn enqueue(session: &Session, track: &str) -> String {
    match call(session, Method::POST, &format!("/me/player/queue?uri={}", encode(track))).await {
        Ok(_) => {
            // Spotify takes a moment to show what it was just given.
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            queue(session).await
        }
        Err(why) => list_line("queue", "", Vec::new(), Some(queue_trouble(why))),
    }
}

pub async fn liked(session: &Session, offset: u32) -> String {
    match get(session, &format!("/me/tracks?limit=50&offset={offset}")).await {
        Ok(page) => page_line("liked", "", wrapped_tracks(&page), None, offset, has_more(&page)),
        Err(why) => page_line("liked", "", Vec::new(), Some(why), offset, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_for_addresses() {
        assert_eq!(encode("miles davis & co"), "miles%20davis%20%26%20co");
        assert_eq!(encode("abc-_.~123"), "abc-_.~123");
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn reads_tracks_in_both_shapes() {
        let track = json!({"uri": "spotify:track:1", "name": "So What", "duration_ms": 562000,
            "artists": [{"name": "Miles Davis"}, {"name": "John Coltrane"}], "album": {"name": "Kind of Blue"}});
        let item = track_item(&track).unwrap();
        assert_eq!(item["artists"], "Miles Davis, John Coltrane");
        assert_eq!(item["album"], "Kind of Blue");
        assert_eq!(track_item(&json!({"uri": "spotify:episode:9", "name": "A podcast"})), None);

        let new_shape = json!({"items": [{"item": track.clone()}, {"item": null}]});
        let old_shape = json!({"items": [{"track": track.clone()}, {"track": {"uri": "spotify:local:x"}}]});
        assert_eq!(wrapped_tracks(&new_shape).len(), 1);
        assert_eq!(wrapped_tracks(&old_shape).len(), 1);
        assert!(wrapped_tracks(&json!({})).is_empty());
    }

    #[test]
    fn reads_albums_and_artists() {
        let album = json!({"id": "al1", "name": "Kind of Blue", "release_date": "1959-08-17", "total_tracks": 5,
            "artists": [{"name": "Miles Davis", "id": "ar1"}]});
        let item = album_item(&album).unwrap();
        assert_eq!((item["kind"].as_str(), item["uri"].as_str()), (Some("album"), Some("spotify:album:al1")));
        assert_eq!((item["artists"].as_str(), item["album"].as_str()), (Some("Miles Davis"), Some("1959")));
        assert_eq!(album_item(&json!({"name": "no id"})), None);
        let item = artist_item(&json!({"id": "ar1", "name": "Miles Davis"})).unwrap();
        assert_eq!((item["kind"].as_str(), item["uri"].as_str()), (Some("artist"), Some("spotify:artist:ar1")));
        // A track says where it comes from, for going there.
        let track = json!({"uri": "spotify:track:1", "name": "So What", "artists": [{"name": "Miles Davis", "id": "ar1"}], "album": {"name": "Kind of Blue", "id": "al1"}});
        let item = track_item(&track).unwrap();
        assert_eq!((item["album_id"].as_str(), item["artist_id"].as_str()), (Some("al1"), Some("ar1")));
        assert!(has_more(&json!({"next": "https://api.spotify.com/v1/me/tracks?offset=50"})));
        assert!(!has_more(&json!({"next": null})));
    }

    #[test]
    fn reads_playlists_in_both_shapes() {
        let new_shape = json!({"id": "abc", "name": "Focus", "owner": {"display_name": "ada"}, "items": {"total": 42}});
        let old_shape = json!({"id": "abc", "name": "Focus", "tracks": {"total": 7}});
        assert_eq!(playlist_item(&new_shape).unwrap()["count"], 42);
        assert_eq!(playlist_item(&new_shape).unwrap()["uri"], "spotify:playlist:abc");
        assert_eq!(playlist_item(&old_shape).unwrap()["count"], 7);
        assert_eq!(playlist_item(&json!({"name": "no id"})), None);
    }
}
