# Experimental USB virtual display

The compositor can now create `DES-VIRTUAL-1` without a DRM connector. It is a
Wayland output mapped into the desktop Space and appended to its screen geometry,
so existing window placement and pointer crossing apply. Physical output changes
recompute its position to the right of the rightmost physical screen.

This is an experimental foundation, not a finished display-settings feature.
The installed compositor is unchanged. Current limitations:

- The virtual output is not yet represented/configurable in the normal hardware
  display-settings report. Resolution and rate are viewer launch options.
- Streaming uses uncompressed RGBA request/response frames. The configured FPS is
  a ceiling, not a performance guarantee. This favors simple USB debugging over
  bandwidth efficiency; high resolutions can use substantial CPU and bandwidth.
- The physical desktop supplies mouse and keyboard input. Tablet touch input is
  not implemented.
- Reconnecting recreates the output, but applications recovered to the primary
  display are not automatically moved back.

## Run alongside the installed desktop

Use a private runtime directory. This example does not replace the login session:

```sh
cargo build -p des-comp
mkdir -p "$XDG_RUNTIME_DIR/des-virtual-test"
chmod 700 "$XDG_RUNTIME_DIR/des-virtual-test"
# Supply the actual absolute socket of the parent desktop here:
WAYLAND_DISPLAY=/run/user/1000/wayland-1 \
XDG_RUNTIME_DIR=/run/user/1000/des-virtual-test \
DES_UI_CONTROL_SOCKET=/run/user/1000/des-virtual-test/control.sock \
DES_VIRTUAL_SOCKET=/run/user/1000/des-virtual-test/virtual.sock \
./target/debug/des-comp --backend winit --no-xwayland
```

In another terminal, with an authorized USB Android device connected:

```sh
python3 scripts/virtual-display-viewer.py \
  --socket /run/user/1000/des-virtual-test/virtual.sock \
  --serial DEVICE_SERIAL --width 960 --height 600 --fps 15
```

The viewer opens Chrome via ADB. Ctrl+C stops the viewer and virtual display.
Closing the page or losing the connection removes the output after five seconds.
The browser retries and recreates it when the connection returns. Use Super+O
inside the nested compositor to move its current application between displays.

## Transport and protection

The compositor listens only on a Unix socket, mode 0600, in a mode-0700 directory.
It checks peer credentials against the socket owner. `DES_VIRTUAL_SOCKET` is
opt-in; an existing socket is never replaced. The helper listens on IPv4 loopback
only. A random 256-bit token protects its HTTP routes. ADB reverse carries the
connection over USB to the already-authorized tablet; no LAN listener is exposed.

Frames reuse the compositor's off-screen GLES capture. Capture is refused while
locked. Pending userspace image responses are discarded on the next locked tick;
the viewer clears its canvas upon lock or connection failure. As with any display,
pixels already received by the viewer cannot be revoked.

The control protocol is one newline-terminated command per Unix connection:
`start WIDTH HEIGHT FPS`, `stop`, `move` (existing move-current-window action),
or `frame`. Image replies are `RGBA WIDTH HEIGHT\n` followed by tightly packed
RGBA bytes. Other replies are short text lines. Clients, request lengths, frame
sizes, and stalled connection lifetimes are bounded.

## Verification on 2 November 2026

- Built `des-comp` and exercised a nested compositor; the installed session was not restarted.
- On the USB Samsung SM-P585, visually inspected a Chrome screenshot showing
  the virtual backdrop, then a separate DEs-UI Settings application moved to screen 1.
- Shell state reported the application on screen 1 and two screens.
- Locked only the nested session: `frame` returned `LOCKED`, move was refused,
  and the tablet screenshot showed a cleared canvas with “Session locked”.
- Abruptly killed the viewer: after six seconds state reported one screen and
  the application's recovery to screen 0, hidden.
- Unit tests cover bounds, geometry with negative physical positions, private
  socket permissions/removal, and refusing replacement of a live socket.

Remaining verification: physical DRM hotplug, physical desktop pointer/key use on
the virtual output, reconnect after an actual USB unplug/replug, and end-to-end
integration with normal display settings. Do not treat this as production-ready.
