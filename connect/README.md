# DES Connect native service

The MIT Rust service implements LAN discovery, protocol-8 mutual TLS, explicit
pairing, durable certificate pins, battery/ping, explicit file transfers, owner-only local IPC and an opt-in
screen bridge. It does not start automatically with the desktop. The standalone
[DES Connect Preview](android/README.md) supports foreground pairing, battery and
ping. The separate GPL fork at `../des-connect-android` remains available for
features the preview does not yet offer. The native crate and preview contain no
implementation copied from the fork.

```sh
cargo build -p des-connect
./target/debug/des-connectd
# In a separate terminal:
./target/debug/des-connect-ctl state
./target/debug/des-connect-ctl discover
./target/debug/des-connect-ctl pair DEVICE_ID
./target/debug/des-connect-ctl accept DEVICE_ID ATTEMPT
./target/debug/des-connect-ctl ping DEVICE_ID
./target/debug/des-connect-ctl stop_screen
./target/debug/des-connect-ctl stop
```

Compare the eight-character code on both devices before accepting. The native
CONNECT panel uses this same IPC and requires the current approval token. Reject,
unpair and stop actions are supported. Pairing persists across daemon restarts;
unknown devices never gain feature access from discovery or identity claims.
Use --help for CLI paths and --loopback for isolated integration tests.

## Boundaries

The control socket defaults to $XDG_RUNTIME_DIR/des-connect/control.sock and
requires the same UID. DES_CONNECT_SOCKET overrides its path. Identity and pins
live in $XDG_STATE_HOME/des-ui/connect (normally ~/.local/state/des-ui/connect),
with private files/directories and symlink/type/owner checks. Never commit these
files. Discovery listens on UDP1716 and TLS uses TCP1716..1764. No video stream is
exposed on the LAN.

Mutual TLS1.2 establishes key possession; peer CN/device identity, certificate
validity/self-signature/key strength and stored SHA256 pins are checked. Only the
encrypted protocol-8 identity is used after the handshake. Pair requests use
monotonic deadlines, timestamp-bound verification codes and unique IPC approval
tokens. Per-IP/global admission limits, absolute handshake deadlines, bounded
frames and unpaired expiry limit resource use. Same-certificate socket replacement
preserves an active pairing attempt; a different key does not replace it.
Legacy battery polling is sent only when the encrypted peer identity advertises
that request type; current Android peers publish battery updates directly.

Discovery advertises battery and ping. A trusted peer with file sharing enabled
receives the file capability through encrypted identity after an explicit reconnect. Screen sharing is a
separate explicit IPC action requiring a trusted connected peer and an unlocked
desktop. Capture stops on lock, peer loss or explicit stop and does not auto-resume.
See SCREEN-BRIDGE.md for USB prerequisites, supported directions and limitations.

The transfer adapter is dispatched by the service and native panel: file sharing
is off by default per device; send requires an explicit selected path, receive
requires explicit acceptance. See TRANSFERS.md for limits and filesystem caveats.
The installer includes the service, CLI and viewer helper, without enabling
autostart.

Media controls and system volume have independent default-off per-device
permissions in CONNECT. After enabling one, reconnect the phone to apply its
encrypted capability list. Android then controls actual MPRIS players, the running
DES Music service, or the default PipeWire output. The adapter opens services only
on request, checks live policy before mutations and replies, and starts no player.
See MEDIA.md for supported operations and limits.

`features.rs` contains tested default-disabled URL, clipboard, notification, calls,
SMS and contacts packet contracts, but their runtime dispatch and panel controls
remain unfinished. Input, command allowlists, physical-desktop mirroring, wireless
video and release packaging also remain outstanding. Do not advertise unsupported
capabilities before their runtime permission/lock checks and UI exist.

## Verification

```sh
cargo test -p des-connect --locked
# This uses a disposable bus and only controls its own fake media player:
dbus-run-session -- env DES_CONNECT_FAKE_MPRIS=1 cargo test -p des-connect media::tests::fake_mpris -- --ignored
```

See ../docs/DES-CONNECT.md for the real-device verification record.
