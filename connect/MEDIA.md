# Native media bridge

`MediaAdapter::default()` is zero-I/O and holds no connection, worker, or timer.
After pairing authentication and separate per-feature opt-in checks, call
`handle_guarded(kind, &body, live_policy_guard)` and send its `MediaReply` values
to the same peer only after another policy check. An
adapter error is local diagnostic information, not a successful protocol reply.
Do not call this module for unauthenticated discovery or disabled features.

Supported requests:

- `kdeconnect.mpris.request`: `requestPlayerList:true`; or a listed `player`
  bus name with `requestNowPlaying`, `requestVolume`, `action` (`Play`, `Pause`,
  `PlayPause`, `Next`, `Previous`, `Stop`), `Seek`, `SetPosition`, `setVolume`.
- `kdeconnect.systemvolume.request`: `requestSinks:true`; or `name:"default-output"`
  with `volume` and/or Boolean `muted`.

Responses use `kdeconnect.mpris` (`playerList`, `supportAlbumArtPayload:false`,
then player metadata/playback/capabilities/volume) and `kdeconnect.systemvolume`
(`sinkList`, then sink updates). The list is request-driven; there are no
spontaneous updates. Phone UIs can request a fresh snapshot. Currently no
loop/shuffle controls, remote-phone MPRIS export, or album-art payload transfer.
Metadata URLs are forwarded as metadata only, never fetched or opened. Controlled
album-art transport can be added through the separately authorized file bridge.

External player identifiers must be well-formed `org.mpris.MediaPlayer2.*` names currently
owned on the session bus. Enumeration is capped at 16 players including the synthetic DES Music entry. The fixed object
path is `/org/mpris/MediaPlayer2`; the interface is
`org.mpris.MediaPlayer2.Player`. A temporary zbus session connection is created
only for an explicit request and dropped afterward. Calls have a one-second
method timeout. Each request reserves the final second for an in-flight
operation and starts no further operation after one elapsed second, limiting
sequential D-Bus/subprocess work to about two seconds. Exhaustion returns an
error; an earlier successful mutation may already have taken effect. Local
D-Bus connection setup uses zbus and is not separately interruptible by this
blocking adapter. Metadata text is capped at 2,048 characters per field. There
is no additional resident D-Bus connection/thread memory while idle.

MPRIS `Volume` is the actual player's 0..1 property. It is independent of the
OS output volume. Incoming KDE volume numbers must be finite and 0..100;
boosts are rejected. `Seek` is signed microseconds, while KDE `SetPosition`,
`pos`, and `length` are milliseconds. Absolute positions are checked before
conversion and use the current player's `mpris:trackid` object path, never a
peer-provided path. MPRIS actions are restricted to six fixed method names.

System volume requires an installed `/usr/bin/wpctl` or `/bin/wpctl` and a
successful `get-volume @DEFAULT_AUDIO_SINK@`. Otherwise an empty `sinkList`
is returned. Only the current default PipeWire sink is exposed, with a stable
logical name. It is never replaced by a selected alternate output; `enabled`
requests are rejected. An external change to the default output affects which
sink subsequent requests control. A sink already above 100% is currently
unavailable through this bridge rather than misreported. Subprocesses use
fixed executable/argv, null stdin/stderr, nonblocking bounded stdout (4 KiB),
and a one-second deadline. Only the owned child is killed and reaped.

Protocol compatibility was independently implemented from the field names and
units observed in the Android fork's `MprisPlugin.kt`, `MprisReceiverPlugin.kt`,
`MprisReceiverPlayer.kt`, `MprisNowPlayingFragment.kt` (default seek interval
10,000,000 microseconds), `SystemVolumePlugin.kt`, and `Sink.kt`. No GPL source
implementation was copied. At implementation time this checkout contains no
`shell/src/mpris.rs` or music MPRIS service; its music service exposes its own
socket protocol. The bridge therefore offers DES Music through the synthetic
adapter below and uses actual MPRIS bus owners for other applications.

Pure tests cover destination validation, volume boundaries/types, seek units and
overflow, strict bounded sink parsing, and command rejection before any I/O.
They do not touch desktop media, volume, Android, or input. Runtime RSS and
end-to-end Android interoperability should be measured by the integrating daemon
with both opt-ins disabled and with explicit feature requests separately.

An ignored integration test serves an isolated fake MPRIS player. To run it
without touching existing desktop media:

```sh
dbus-run-session -- env DES_CONNECT_FAKE_MPRIS=1 cargo test -p des-connect media::tests::fake_mpris -- --ignored
```

The test checks player discovery, unsupported album-art payload advertisement,
actual fake-player Volume updates, microsecond Seek, millisecond SetPosition
and metadata conversion using the current track ID, and unknown-player rejection.


## DES Music socket adapter

An exact `player:"DES Music"` request uses the existing music service socket
instead of D-Bus. Discovery adds that entry only after the already-running
service returns valid state. It never starts the player. The local address
contract matches `des_common::music`: `DES_MUSIC_SOCKET` if set, otherwise
`$XDG_RUNTIME_DIR/des-music.sock`, otherwise
`$TMPDIR/des-music-<uid>/des-music.sock`. The peer cannot supply a socket path.
The endpoint must be an owned private socket in an owned private directory;
Linux peer credentials must match the daemon's effective UID.

Connections use nonblocking Unix sockets and one shared 500 ms deadline for
connect/read/write and response processing. An unavailable/full accept backlog
fails immediately. JSON state lines are capped at 64 KiB. The adapter holds no
socket after the request. It requires valid playback, position, volume, and
track fields before advertising the service. Metadata text and known cover URLs
are forwarded with the same 2,048-character caps; no cover file or URL is read.

`Play` and `Pause` use the service's fixed `toggle` command only when the observed
state requires it; `PlayPause`, `Next`, and `Previous` map to fixed commands.
Because the socket has no atomic explicit play/pause operation, another client
changing playback between snapshot and toggle can race this translation. `Stop`
is rejected because the music socket offers no stop command. There is no
stop-capability advertisement. Track-dependent controls are unavailable when
there is no current track.

Relative `Seek` microseconds are converted to milliseconds and added to the
observed actual position; `SetPosition` is already milliseconds. Positions must
fit the music service's `u32` field and the current track duration. Volume
accepts 0..100 and rounds to the integer percent supported by the socket.
One control per DES Music packet is accepted; ambiguous combined controls are
rejected before any write. After a command the adapter asks for state and
returns the next actual state response/broadcast. Service commands have no
acknowledgement, so the returned observed state can precede asynchronous
playback completion. It does not invent a successful resulting state. As with
the music app itself, controls affect the service's current playing destination,
which can be another Spotify device; no device transfer command is exposed.

Fake private socket tests cover state mapping, idempotent Play, controls and seek
units, out-of-track and combined-command rejection without writes, malformed
and oversized state, stalled peers, endpoint permissions, and invalid volume or
metadata. The private D-Bus test also verifies that the synthetic list entry is
absent for a missing socket and present only for its fake responding service.
Tests never connect to the real music socket or control desktop playback.

## Integrated daemon permissions and transport

The native daemon persists two independent per-peer permissions:
`media_enabled` and `volume_enabled`, both defaulting to false for existing
records. Updating either preserves the certificate pin, file permission and the
other media permission; remembering a peer again also preserves all three.
Media permission covers player listing, metadata, playback and the player's own
volume. System-volume permission separately controls the desktop's default output.

Private IPC uses `set_media {device_id, enabled:boolean}` and
`set_volume {device_id, enabled:boolean}`. Both require an unlocked session and a
trusted paired peer. A trusted offline peer can be updated for its next connection.
Device snapshots expose these booleans plus `media_reconnect_needed` and
`volume_reconnect_needed`. Changes to an existing connection's advertised policy
use the existing explicit `reconnect {device_id}` action; it preserves pairing and
requests a single new connection rather than replaying any old media request.

Public discovery/plaintext identities retain the core-only capability list.
Only an encrypted identity for the opted-in, currently pinned peer includes:

| Permission | Incoming request | Outgoing response |
| --- | --- | --- |
| Media | `kdeconnect.mpris.request` | `kdeconnect.mpris` |
| System volume | `kdeconnect.systemvolume.request` | `kdeconnect.systemvolume` |

The actor retains the peer's secure capability lists and its own actual advertised
policy across socket replacement. The peer must advertise the matching outgoing
request and incoming reply types. A media permission or capability change does not
invalidate an otherwise unchanged, authorized file-transfer link.

Every incoming media request verifies current paired/connected state, the current
stored certificate pin, its specific saved and advertised permission, both secure
capability directions and current desktop lock status. Disabled, locked or
unsupported requests retain the control connection and publish a bounded local
device error. Adapter errors use a generic bounded message rather than caching
private request or player content. Media payload transfer is unsupported.

`MediaAdapter::handle_guarded(kind, body, guard)` rechecks the live authorization
callback after lazy preparation and immediately before **each** actual mutation:
MPRIS property/method writes, DES Music control writes and `wpctl` setter execution.
The compatibility `handle` entrypoint supplies an always-allow guard for callers
that already own policy; the daemon uses the guarded entrypoint. A permission or
lock check only before opening a bus/socket is insufficient. The daemon also
rechecks the same policy immediately before each TLS response, so metadata prepared
while the session locks is discarded rather than returned or saved in snapshots.
MPRIS dispatch targets the enumerated player's current unique bus owner; an owner
that disappears cannot be started again through its well-known name.

The adapter remains a local temporary value per authorized request. There is no
resident adapter connection, media cache, player polling, event subscription,
background export or music-service startup. Player information is returned only
on the current authenticated TLS connection. After one potentially blocking media
request the actor yields to local commands and revocation checks; at most one
already-read 4 KiB of raw input is retained before processing the next request.
Coalesced player/volume startup requests are preserved, not silently dropped.
The adapter's existing OS/D-Bus connection and filesystem limitations still apply;
this integration does not make blocking kernel or bus setup interruptible.

Additional regression tests use disposable TLS identities, temporary daemon homes,
a private fake shell and a private fake DES Music service. They cover old-policy
migration, independent permissions, encrypted capability gates, explicit reconnect,
unknown peers and missing peer capability directions. A fake service delays its
initial state until the fake shell locks: a playback mutation must produce no
control command, and a separate read-only request must return no prepared metadata.
Coalesced requests both receive replies, and malformed volume requests are refused
before any real system-volume command. A dummy `wpctl` script checks that revocation
after sink lookup prevents setter execution. The disposable private-D-Bus test
also checks that a revoked guard leaves its fake player's Volume unchanged.
None of these fixtures reads or changes the actual desktop's playback or audio.
