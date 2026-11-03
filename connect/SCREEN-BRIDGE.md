# USB screen bridge

`src/screens.rs` owns explicit, single-device screen sessions. It opens no network
listener itself. The service must require a paired peer, explicit USB serial
selection, and an unlocked current desktop before calling either start method.
Pairing a network peer does **not** prove that a selected USB serial is that peer:
the user must select the physically connected device deliberately. Do not infer
that association from an Android model or a display name.

## Supported directions

- **Android screen → desktop:** an owned scrcpy window, over authorized USB ADB.
  Viewing defaults to no device control. Control is a separate explicit option.
  Audio and clipboard autosync are disabled; starting does not wake the phone.
  Capture is capped at 1920 pixels on the longest side and 30 fps. Older Android
  versions may not enforce the FPS ceiling; scrcpy documents official FPS support
  from Android 10. Secure Android surfaces may remain black.
- **Extra desktop output → Android:** the existing experimental
  `scripts/virtual-display-viewer.py` creates a genuine compositor output. It
  transports uncompressed frames to an Android browser through a random-token
  loopback HTTP endpoint and ADB reverse. It requires an opt-in, private
  `DES_VIRTUAL_SOCKET` owned by the *current compositor*. This bridge checks both
  socket owners and Linux peer PID equality against `DES_COMP_SOCKET` (or
  `$XDG_RUNTIME_DIR/des-comp-$WAYLAND_DISPLAY.sock`). It refuses an existing output,
  caps configuration at 1920×1200 and 30 fps, and does not forward tablet touch.
- **Physical desktop screen → Android:** unavailable. The virtual helper captures
  its extra output; it does not mirror an existing physical desktop. The current
  installed session has no opt-in virtual socket. Starting a different nested
  compositor would display that different session, and is not a substitute for
  extending the current desktop.

See [the virtual display implementation notes](../docs/VIRTUAL-DISPLAY.md) for its
bandwidth, reconnect, lock, and display-settings limitations.

## Service API and lifecycle

```rust
let mut screens = ScreenBridge::new();
let capabilities = screens.capabilities();
let state = screens.start_phone_guarded(selected_usb_serial, false, check_current_peer_and_lock)?;
// Or: screens.start_extended_guarded(selected_usb_serial, 960, 600, 15, check_current_peer_and_lock)?;
let state = screens.status();
let state = screens.stop()?;
```

The service supplies `check_current_peer_and_lock` as an `FnOnce() ->
Result<(), String>` callback that rechecks the selected peer's live pairing and
connection state and the current desktop's lock state. Both guarded methods invoke
it after all USB/helper/socket preparation, immediately before spawning the viewer.
Rejecting the guard starts no viewer and removes any prepared output lease;
no reverse mapping or virtual output has been created at that stage. A failed
spawn likewise releases only the lease. The older `start_phone` / `start_extended`
methods remain compatibility entrypoints with caller-only policy checks; services
must use the guarded variants to avoid stale checks during USB preparation.

Starts return `Result<ScreenStatus, String>`. `status()` returns `ScreenStatus`;
`capabilities()` returns `ScreenCapabilities`. Both reports implement `Serialize`.
`running` means the owned viewer process is alive, **not** that a frame has been
confirmed on screen. Startup failures and bounded tool diagnostics appear through
status polling. The bridge rejects a second session until the first stops.

Poll while the service is active, including while its settings panel is closed.
Stop immediately on desktop lock or paired-peer disconnection/unpairing. Never
restart capture automatically after unlock/reconnect. Independent USB probes at
most every two seconds also stop a session when USB disappears or stops responding;
scrcpy normally exits itself on unplug. Probes have a 500 ms bound during polling.
Neither probing nor capability reporting starts capture.

Stopping or dropping the bridge terminates only its owned viewer process group,
reaps its child, removes its own reverse-port mapping, and stops its owned virtual
output if the compositor peer PID still matches. A bounded TERM grace period is
followed by KILL when necessary. It does not kill the shared ADB server. It does
not close unrelated terminals or restart the desktop. On a service crash, the
virtual compositor times out after five seconds without frame requests; an ADB
reverse mapping or a `*.des-connect-lease` file can remain. Check for an existing
viewer before manually removing a stale lease. After cleanup, reconnect explicitly.

The helper chooses an unused local ephemeral port. There is a small bind race:
if another process takes it first, helper startup fails and the session is cleaned
up. External manual viewers should not be started concurrently with a bridge
session; they do not participate in its private lease.

## Tools and installation

ADB and Python 3 resolve to absolute executable paths from the service's PATH.
The scrcpy lookup is:

1. Absolute `DES_CONNECT_SCRCPY`, when provided by the local user.
2. `$HOME/.local/share/des-connect/tools/scrcpy-linux-x86_64-v5.0.1/scrcpy`.
3. An absolute scrcpy executable in PATH.

For the virtual helper, an absolute `DES_CONNECT_VIEWER` overrides the repository
helper compiled into the build location. An installed deployment can also place
it at `/usr/local/share/des-ui/virtual-display-viewer.py`. Environment overrides
are local administrator/user configuration, never peer-provided packet fields.
Keep the portable scrcpy directory intact: its sibling `scrcpy-server` is needed.
The selected ADB path overrides the portable bundle's ADB choice.

On this x86_64 host, official scrcpy **v5.0.1** was installed privately under the
second path above. No system package or global PATH was changed. The archive was
downloaded only from the official pinned release and verified **before extraction**
against the checksum published in the same pinned upstream Linux instructions:

```text
https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-linux-x86_64-v5.0.1.tar.gz
SHA-256 9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e
```

For another machine, download that archive, check its SHA-256 using `sha256sum`,
then extract into `$HOME/.local/share/des-connect/tools/`. Do not use that x86_64
archive on ARM or assume the checksum applies to another release. The bridge does
not download programs or run installers at runtime.

Primary references:
[official pinned Linux download/checksum](https://github.com/Genymobile/scrcpy/blob/v5.0.1/doc/linux.md),
[read-only and clipboard flags](https://github.com/Genymobile/scrcpy/blob/v5.0.1/doc/control.md),
[no-power-on flag](https://github.com/Genymobile/scrcpy/blob/v5.0.1/doc/device.md).
The installed binary's `--help` also confirmed all emitted flags.

## Verification

Unit tests exercise hostile serial rejection, safe viewing arguments, tool timeout,
bounded diagnostics, nonzero exit reporting, natural viewer exit without restart,
owned-process stop/drop, survival of an unrelated process, and local Unix socket
peer credentials. A delayed disposable fake ADB changes the policy state during
preparation: the final guard rejects and a fake viewer's launch marker is never
created. Guard rejection also releases a prepared extended lease without invoking
ADB cleanup or launching a helper. They use disposable
`/bin/sleep` and `/bin/sh` processes; none records a screen or operates a device.
The host binary's `--version` reports scrcpy 5.0.1. Actual Android capture,
keyboard/mouse control, unplug/reconnect, and extended-output streaming require
coordinated manual verification after pairing; unit tests do not establish those
end-to-end behaviors.
