# Native media bridge

`MediaAdapter::default()` is zero-I/O and holds no connection, worker, or timer.
After pairing authentication and separate per-feature opt-in checks, call
`handle(kind, &body)` and send its `MediaReply` values to the same peer. An
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

Player identifiers must be well-formed `org.mpris.MediaPlayer2.*` names currently
owned on the session bus. Enumeration is capped at 16 players. The fixed object
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
socket protocol. Consequently the bridge discovers only actual MPRIS bus owners.
A music-app MPRIS service is needed before that app appears to the phone.

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
