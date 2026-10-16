//! Playback state and service lifecycle, independent of CLI and authentication setup.

use crate::{device_id, ipc, open_cache, pick_cover, protocol, socket, web, NO_LIKING, OUTPUT};
use crate::protocol::{Request, State, Track};
use librespot_connect::{ConnectConfig, LoadRequest, LoadRequestOptions, PlayingTrack, Spirc};
use librespot_core::{config::SessionConfig, session::Session};
use librespot_metadata::audio::UniqueFields;
use librespot_playback::{
    audio_backend,
    config::{AudioFormat, PlayerConfig},
    mixer::{self, MixerConfig},
    player::{Player, PlayerEvent},
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

struct Service {
    spirc: Spirc,
    session: Session,
    state: Mutex<(State, Instant)>,
    /// What another device is playing, as last heard, while one is: this then is what
    /// clients are shown, and what the controls act on.
    remote: Mutex<Option<(State, Instant)>>,
    /// The track last asked about, and whether it is among the liked songs.
    liked: Mutex<(String, Option<bool>)>,
    /// Woken to look at once at what is playing elsewhere.
    look: tokio::sync::Notify,
    updates: broadcast::Sender<String>,
}

/// How often what other devices are playing is looked at.
const LOOK_EVERY: Duration = Duration::from_secs(5);


impl Service {
    /// The state as of now, with the position moved on by the time since it was set.
    fn snapshot(&self) -> State {
        let (mut now, since) = match &*self.remote.lock().unwrap() {
            Some((state, since)) => (state.clone(), *since),
            None => {
                let guard = self.state.lock().unwrap();
                (guard.0.clone(), guard.1)
            }
        };
        if now.playing {
            let length = now.track.as_ref().map_or(u32::MAX, |track| track.duration_ms);
            now.position_ms = now.position_ms.saturating_add(since.elapsed().as_millis() as u32).min(length);
        }
        let liked = self.liked.lock().unwrap();
        now.liked = liked.1.filter(|_| now.track.as_ref().is_some_and(|track| track.uri == liked.0));
        now
    }

    /// Whether another device is the one playing.
    fn elsewhere(&self) -> bool {
        self.remote.lock().unwrap().is_some()
    }

    /// Changes what is shown of the other device at once, ahead of hearing it confirmed.
    fn update_remote(&self, change: impl FnOnce(&mut State)) {
        if let Some((state, since)) = self.remote.lock().unwrap().as_mut() {
            if state.playing {
                state.position_ms = state.position_ms.saturating_add(since.elapsed().as_millis() as u32);
            }
            *since = Instant::now();
            change(state);
        }
        self.publish();
    }

    /// Tells the device that is playing something, through the Web API. What goes
    /// wrong is said to the clients.
    async fn tell(&self, method: http::Method, path: &str, body: Option<serde_json::Value>) {
        if let Err(why) = web::send(&self.session, method, path, body).await {
            let _ = self.updates.send(protocol::problem_line(&why));
        }
        self.look.notify_one();
    }

    /// Finds out whether the track playing is liked, when the track changes.
    async fn check_liked(&self) {
        let Some(uri) = self.snapshot().track.map(|track| track.uri) else { return };
        if self.liked.lock().unwrap().0 == uri {
            return;
        }
        *self.liked.lock().unwrap() = (uri.clone(), None);
        let liked = web::is_liked(&self.session, &uri).await;
        let mut held = self.liked.lock().unwrap();
        if held.0 == uri {
            held.1 = liked;
            drop(held);
            self.publish();
        }
    }

    /// Looks at what is playing on whichever device, and takes it up when that is
    /// not this one.
    async fn look_elsewhere(&self) -> Result<(), String> {
        let found = web::player(&self.session).await?;
        let here = self.session.device_id().to_string();
        let remote = found.filter(|(_, device)| *device != here).map(|(state, _)| (state, Instant::now()));
        let changed = {
            let mut held = self.remote.lock().unwrap();
            let changed = held.as_ref().map(|(state, _)| state) != remote.as_ref().map(|(state, _)| state) || remote.is_some();
            *held = remote;
            changed
        };
        if changed {
            self.publish();
        }
        self.check_liked().await;
        Ok(())
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
            PlayerEvent::Playing { position_ms, .. } => {
                // Playing here: whatever another device was doing is over.
                *self.remote.lock().unwrap() = None;
                self.look.notify_one();
                self.update(|state| {
                    state.playing = true;
                    state.position_ms = position_ms;
                })
            }
            PlayerEvent::RepeatChanged { context, track } => self.update(|state| {
                state.repeat = if track { "one" } else if context { "all" } else { "off" }.to_string();
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
        use http::Method;
        // While another device plays, the controls are for it.
        if self.elsewhere() {
            match &request {
                Request::Toggle => {
                    let playing = self.snapshot().playing;
                    self.update_remote(|state| state.playing = !playing);
                    self.tell(Method::PUT, if playing { "/me/player/pause" } else { "/me/player/play" }, None).await;
                    return None;
                }
                Request::Next => {
                    self.tell(Method::POST, "/me/player/next", None).await;
                    return None;
                }
                Request::Previous => {
                    self.tell(Method::POST, "/me/player/previous", None).await;
                    return None;
                }
                Request::Seek(ms) => {
                    let ms = *ms;
                    self.update_remote(|state| state.position_ms = ms);
                    self.tell(Method::PUT, &format!("/me/player/seek?position_ms={ms}"), None).await;
                    return None;
                }
                Request::Volume(percent) => {
                    let percent = (*percent).min(100);
                    self.update_remote(|state| state.volume = percent);
                    self.tell(Method::PUT, &format!("/me/player/volume?volume_percent={percent}"), None).await;
                    return None;
                }
                Request::Shuffle(on) => {
                    let on = *on;
                    self.update_remote(|state| state.shuffle = on);
                    self.tell(Method::PUT, &format!("/me/player/shuffle?state={on}"), None).await;
                    return None;
                }
                Request::Repeat(mode) => {
                    let word = match mode.as_str() {
                        "all" => "context",
                        "one" => "track",
                        _ => "off",
                    };
                    let mode = mode.clone();
                    self.update_remote(|state| state.repeat = mode);
                    self.tell(Method::PUT, &format!("/me/player/repeat?state={word}"), None).await;
                    return None;
                }
                Request::Play { context, track } => {
                    let context = context.as_ref().map(|context| match context.as_str() {
                        "liked" => format!("spotify:user:{}:collection", self.session.username()),
                        other => other.to_string(),
                    });
                    let body = match (context, track) {
                        (Some(context), Some(track)) => json!({ "context_uri": context, "offset": { "uri": track } }),
                        (Some(context), None) => json!({ "context_uri": context }),
                        (None, Some(track)) => json!({ "uris": [track] }),
                        (None, None) => return None,
                    };
                    self.tell(Method::PUT, "/me/player/play", Some(body)).await;
                    return None;
                }
                _ => {}
            }
        }
        match request {
            Request::State => Some(self.snapshot().to_line()),
            Request::Repeat(mode) => {
                let _ = self.spirc.repeat(mode == "all");
                done(self.spirc.repeat_track(mode == "one"))
            }
            Request::Like(on) => {
                let Some(uri) = self.snapshot().track.map(|track| track.uri) else { return None };
                match web::set_liked(&self.session, &uri, on).await {
                    Ok(()) => {
                        *self.liked.lock().unwrap() = (uri, Some(on));
                        self.publish();
                        None
                    }
                    Err(why) if ["401", "403", "scope"].iter().any(|sign| why.contains(sign)) => Some(protocol::problem_line(NO_LIKING)),
                    Err(why) => Some(protocol::problem_line(&why)),
                }
            }
            Request::Lyrics(track) => Some(web::lyrics(&self.session, &track).await),
            Request::Devices => Some(web::devices(&self.session, self.session.device_id()).await),
            Request::Transfer(device) => {
                self.tell(Method::PUT, "/me/player", Some(json!({ "device_ids": [device], "play": true }))).await;
                // The devices take a moment to agree on who is playing.
                tokio::time::sleep(Duration::from_millis(900)).await;
                let _ = self.look_elsewhere().await;
                Some(web::devices(&self.session, self.session.device_id()).await)
            }
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
            Request::Tracks { playlist, offset } => Some(web::tracks(&self.session, &playlist, offset).await),
            Request::Liked { offset } => Some(web::liked(&self.session, offset).await),
            Request::Albums { offset } => Some(web::albums(&self.session, offset).await),
            Request::Album(id) => Some(web::album(&self.session, &id).await),
            Request::Artist(id) => Some(web::artist(&self.session, &id).await),
            Request::Queue => Some(web::queue(&self.session).await),
            Request::Enqueue(track) => Some(web::enqueue(&self.session, &track).await),
        }
    }
}

pub(super) async fn serve() -> Result<(), String> {
    let path = protocol::socket_path();
    socket::prepare_parent(&path).map_err(|e| format!("cannot prepare music socket: {e}"))?;
    let _socket_lock = socket::lock(&path).map_err(|_| "another music service owns this endpoint, or its lock is unavailable")?;
    // Only a refused or absent endpoint is stale. Permission and other failures
    // must never cause a running service's socket to be unlinked.
    match UnixStream::connect(&path).await {
        Ok(_) => return Err("the music service is already running".to_string()),
        Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {},
        Err(error) => return Err(format!("cannot check music socket: {error}")),
    }
    let cache = open_cache()?;
    let credentials = cache.credentials().ok_or("not signed in: run `des-music login` first")?;
    let session = Session::new(SessionConfig { device_id: device_id(), ..Default::default() }, Some(cache));

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

    // A listening socket is the shell's readiness signal. Keep the endpoint lock
    // while signing in, but bind only after playback is ready to accept clients.
    socket::remove_stale(&path).map_err(|e| format!("cannot recover music socket: {e}"))?;
    let listener = UnixListener::bind(&path).map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    socket::restrict(&path).map_err(|e| format!("cannot restrict music socket: {e}"))?;

    let (updates, _) = broadcast::channel(32);
    let service = Arc::new(Service {
        spirc,
        session,
        state: Mutex::new((State::default(), Instant::now())),
        remote: Mutex::new(None),
        liked: Mutex::new((String::new(), None)),
        look: tokio::sync::Notify::new(),
        updates,
    });

    // What other devices are playing is asked for now and then, while anyone is
    // there to be shown it.
    let for_looking = service.clone();
    tokio::spawn(async move {
        let mut wait = LOOK_EVERY;
        loop {
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                // Something was just changed: give it a moment to take, then look.
                () = for_looking.look.notified() => tokio::time::sleep(Duration::from_millis(700)).await,
            }
            if for_looking.updates.receiver_count() == 0 {
                continue;
            }
            // Without a library connection, or with Spotify out of reach, ask less often.
            wait = if for_looking.look_elsewhere().await.is_ok() { LOOK_EVERY } else { LOOK_EVERY * 6 };
        }
    });

    let for_events = service.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            for_events.on_event(event);
        }
    });
    let for_clients = service.clone();
    let slots = ipc::Clients::new();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            // Reject excess clients rather than creating an unbounded waiter queue.
            let Some(permit) = slots.admit() else { continue };
            let service = for_clients.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let (reader, writer) = stream.into_split();
                let initial = service.snapshot().to_line();
                let updates = service.updates.subscribe();
                ipc::serve_client(reader, writer, initial, updates, move |request| {
                    let service = service.clone();
                    async move { service.handle(request).await }
                }).await;
            });
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

