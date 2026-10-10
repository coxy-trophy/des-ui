# DEs-UI

A sci-fi terminal desktop for Linux, written in Rust: a Wayland compositor and a
shell that run as their own login session. It began as a rewrite of
[eDEX-UI](https://github.com/GitSquared/edex-ui).

It has three programs:

- **des-ui** (`shell/`) — the interface: terminal, system panels, file browser.
- **des-comp** (`compositor/`) — a Wayland compositor that shows the shell
  fullscreen and places other applications in the middle of its frame.
- **des-music** (`music/`) — a Spotify player with no window of its own; the
  shell is its interface.

## Try it in a window

    cargo build
    ./target/debug/des-comp                      # the whole desktop
    ./target/debug/des-comp -- --theme blade     # arguments after -- go to the shell
    ./target/debug/des-ui --windowed             # the shell on its own

Open applications with the launcher, or by starting them from the terminal. Hold
Super (or Ctrl+Alt, which works when running in a window):

- **Space** — open the launcher: type to search, Enter to open, Esc to cancel
- **,** — open the settings
- **L** — lock the screen
- **Tab** — switch between the terminal and the applications on this display
- **S** — split the workspace between two windows, or make it whole again
- **Left / Right** — give the keyboard to that half of a split
- **F** — full screen: the applications on this display cover all of it
- **[  ]  \\** — collapse or open the left, right and bottom panels
- **Q** — close the current application
- **O** — move the current application to the next display

The bar above the workspace lists the terminal tabs and every open application;
click one to switch to it. The application in use shows an arrow, to send it to
the next display, and a cross, to close it. `SPLIT`, `FULL` and `APPS` do what the
keys do, and the three small switches in the status strip collapse the panels.

A split keeps the window in view on the left and puts the application used before
it on the right. With one application open, that one goes right and the terminal
stays on the left.

With several displays, modes and arrangement follow what you chose in the settings,
or else the layout saved in GNOME's display settings (`~/.config/monitors.xml`)
when it covers the connected monitors.
Otherwise the frame is on a laptop's own panel and the others extend to its right.
Each further display shows its applications at full size, split or not.
Applications open on the display the pointer is on.

`des-ui --list-apps` prints what the launcher offers.

The globe marks this machine and the places it has connections open to. Positions
come from looking addresses up at get.geojs.io, which tells that service the public
addresses this machine talks to; each is asked about once and kept in
`~/.cache/des-ui/places.tsv`. Your own position is guessed from your address
unless you set it.

## The terminal

Ctrl+Shift+T opens a tab and Ctrl+Tab moves between them. The terminal can also be
split in two, each half showing a tab of its own:

- **Ctrl+Shift+E** — split side by side; again to make it whole
- **Ctrl+Shift+O** — split one above the other
- **Ctrl+Shift+arrow** — give the keyboard to the pane in that direction, as does a
  click in it
- **Ctrl+Tab** — change the tab in the pane in use

The small divided-frame button beside `+` does the same as Ctrl+Shift+E. Both tabs
stay open when the split ends. This is separate from Super+S, which shares the
workspace with an application.

## Settings

**Super+,** or `SETTINGS` in the status strip opens them in place of the terminal;
Esc closes. Changes apply at once and are saved.

- **Appearance** — theme
- **Terminal** — font, text size, line height, cursor shape and blink, bold in
  bright colours, copy on select, scrollback
- **Frame** — which panels are open, now and at every start
- **Displays** — each monitor's resolution and refresh rate, their order left to
  right, and which one is the main display
- **Network** — Wi-Fi on or off, and joining, leaving and forgetting networks
- **Bluetooth** — on or off, and pairing, connecting and forgetting devices
- **Keyboard** — layout, and how held keys repeat
- **Mouse & touchpad** — natural scrolling, tap to click, pointer speed, left-handed
- **Globe** — rotation, online lookups, and your location
- **Sound & screen** — volume and brightness
- **Session** — where the logs are, and logging out

A display change is tried for 15 seconds and kept only if you confirm it; otherwise
it goes back by itself, so a mode your monitor cannot show does not strand you.
Confirmed layouts are saved in `~/.config/des-ui/displays`, one per set of
monitors, and come before GNOME's saved layout.

The rest are kept in `~/.config/des-ui/config` as `key = value` lines, which can also
be edited by hand. The compositor reads the keyboard, mouse and touchpad
settings from the same file. Network and Bluetooth go through `nmcli` and
`bluetoothctl`; a Wi-Fi password is handed to `nmcli` on its command line. Two can be set from a terminal:

    des-ui --set-location 48.8566,2.3522,Paris   # or: --set-location auto
    des-ui --geo off                             # no lookups at all; on to resume

X11 applications run through Xwayland, which `des-comp` starts and manages; the
clipboard is shared between them and Wayland applications.

The look is called Signal (`shell/src/ui.rs`): fixed greys, two typefaces, and one
accent colour. A theme sets that accent and the terminal's colours, in the JSON
format eDEX-UI uses, so its theme files work here. `themes/` holds the built-in
one; your own go in `~/.config/des-ui/themes/`.

## Install as a login session

    sudo apt install libudev-dev libinput-dev libgbm-dev libseat-dev cage
    ./install.sh

Then log out and choose **DEs-UI** from the session menu on the login screen.
Exiting the last shell, or Ctrl+Shift+Q, ends the session. `./uninstall.sh`
removes it again.

In the session, `des-comp` drives the displays and input devices itself (one GPU,
every monitor connected to it, including ones plugged in later). If that fails in the first seconds it falls back to running inside
`cage`, and the reason is in `~/.local/state/des-ui/session.log`. What applications started
from the launcher print goes to `apps.log` next to it.

Ways out if something goes wrong: **Ctrl+Alt+Backspace** ends the session, and
**Ctrl+Alt+F1…F12** switch virtual terminals.

## Locking

**Super+L**, or `LOCK NOW` under Session in the settings, hides everything behind
your password; what was running keeps running. While locked the compositor shows
nothing but the lock screen and sends it every key, so no shortcut reaches what is
behind it. The password is checked with the system's own helper, `unix_chkpwd`.

It also locks by itself:

- after ten minutes without a key or the pointer being touched (Session in the
  settings changes how long, or turns it off), though not while an application is
  playing something to watch;
- before the machine sleeps, so that closing the lid and opening it again shows the
  lock screen (this too can be turned off there);
- when the system asks, as `loginctl lock-session` does.

If the lock screen ever cannot be unlocked, press Ctrl+Alt+F3, sign in on the text
console, and run `des-ui --unlock`.

## Music

**Super+M**, or `MUSIC` in the status strip, opens a Spotify player in the
workspace: search, your playlists and your liked songs, with the track playing and
its controls along the bottom. `UP NEXT` shows what is queued, and `+ QUEUE` on a
track under the pointer adds it. While something plays, the right column shows it
too, and the keyboard's play, next and previous keys work from anywhere.

It needs Spotify Premium, and two things done once, both from the music view:

1. **Sign in** (or `des-music login`), on Spotify's own page in your browser.
   This is what plays.
2. **Connect your library** (or `des-music library <client ID>`). Spotify turns
   away library requests made in a player's name, so reading playlists, liked songs
   and search needs an app of your own: at
   [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard) create
   an app with the Redirect URI `http://127.0.0.1:8898/login` and the Web API ticked,
   and paste its Client ID into the music view.

`des-music logout` forgets both. Nothing else of Spotify needs to be installed or open.

The playing is done by `des-music`, built on
[librespot](https://github.com/librespot-org/librespot): it appears in Spotify's
device list as "DEs-UI", so a phone can control it as well, and it keeps playing
while the shell restarts. Sound goes out through PipeWire (`pw-cat`).

## Notifications

The shell is the session's notification service, so what applications announce (a
message in the browser, a finished download) shows as a toast at the top right of
the workspace, above whatever is open, and is kept in a list. The desktop adds its
own: battery low, network lost and found, a display connected or removed.

A toast stays a few seconds, longer under the pointer; an urgent one stays until
seen to. Clicking it does what its sender offered (usually: show the application),
and the cross puts it away for good.

**Super+N**, or `NOTIFICATIONS` in the status strip, opens the list. `DO NOT
DISTURB` there keeps all but the urgent ones off the screen; they still reach the
list. Nothing is shown over the lock screen.

## Its own login screen

Optional, and separate from the above, because it changes how the machine boots:

    ./install-login.sh

That installs greetd, a small login manager, and makes the DEs-UI login screen the
machine's own in place of GDM, from the next boot. `./uninstall-login.sh` puts GDM
back. To see the screen without any of that: `des-ui --greeter --windowed`.

The screen shows a picture, a name and a password line. The picture is a pattern
made from the name unless you give it one: `des-ui --set-avatar <file>`. F1 changes
the session, Tab the account when there are several, and the three marks at the
bottom right suspend, restart and shut down, each on a second click.

The screen is shown by `des-comp` itself; its log is
`/var/lib/des-ui/greeter/login.log`. If it cannot start, a text login takes its
place. If the machine ever comes
up with no usable login at all, press Ctrl+Alt+F3, sign in on the text console, and
run `sudo systemctl disable greetd && sudo systemctl enable gdm3 && sudo reboot`.

## Working inside it while changing it

- `./dev.sh` builds the working copy and runs it in a window inside the running
  session. Nothing installed is touched.
- `./install.sh` can be run from inside the session. **Super+Shift+R** then restarts
  the shell on the new build: its terminal tabs close, applications stay open. A new
  compositor takes effect at the next login.
- If the shell crashes it is started again and applications stay open. Only quitting
  it (Ctrl+Shift+Q, or closing the last tab) ends the session.
- `install.sh` keeps the build it replaces. If a new build cannot start at login, the
  session tries that one, then `cage`.
- Anything that must survive a shell restart should not run in the shell's own
  terminal tabs: use a terminal application such as `ptyxis`, or `tmux`.

## Layout

- `shell/src/term.rs` — shell on a PTY, VT parsing, grid rendering, key encoding
- `shell/src/sysmon.rs` — CPU, memory, process and network sampling
- `shell/src/ui.rs` — the interface language: colours, typefaces, shared parts
- `shell/src/panels.rs` — side columns, file browser, status strip
- `shell/src/globe.rs`, `geo.rs` — the rotating globe and the places on it
- `shell/src/settings.rs`, `config.rs` — the settings screen and the file behind it
- `shell/src/radio.rs` — Wi-Fi and Bluetooth
- `shell/src/greeter.rs`, `lock.rs` — the login screen and the lock screen
- `shell/src/power.rs` — locking before sleep, and what holds off the idle lock
- `common/src/input.rs` — keyboard, mouse and touchpad settings
- `shell/src/apps.rs`, `launcher.rs` — installed applications and the launcher
- `shell/src/theme.rs` — eDEX-UI theme loader
- `shell/src/music.rs` — the music view and what is playing
- `shell/src/notify.rs`, `compositor/src/toast.rs` — notifications, and how their
  toasts get above the applications
- `music/src/main.rs`, `web.rs`, `protocol.rs` — the Spotify player, the library
  lookups, and the lines it exchanges with the shell
- `compositor/src/policy.rs` — which window is shown on which display
- `compositor/src/udev.rs`, `winit.rs` — the hardware and windowed backends
- `compositor/src/displays.rs`, `monitors.rs` — display settings and saved layouts
- `compositor/src/xwayland.rs`, `focus.rs` — X11 applications
- `compositor/src/ipc.rs`, `shell/src/desktop.rs` — the socket the two talk over
- `common/` — frame geometry and that socket's protocol, shared by both
- `session/` — login session entry and launcher

## Not done yet

- Mirroring displays
- Keyboard layout setting; display scaling for high-density screens
- Dialogs at their natural size; sharing the middle-click selection with X11 applications
- Turning a display off; placing displays above one another; scaling
- Wired and VPN connections in the settings; Bluetooth devices that need a code
- In the music player: removing from or reordering the queue (Spotify offers outside
  apps no way to), albums and artists, more than the first page of a long playlist
- In notifications: pictures and application icons, sounds
- Turning the screen off after a while
- On the login screen: accounts that need more than a password
- From the original: on-screen keyboard, sound effects, mouse reporting to
  terminal apps

## Licence

MIT; see `LICENSE`. `compositor/` is derived from Smithay's `smallvil` example, also
MIT. The bundled typefaces, Chakra Petch and JetBrains Mono (`shell/assets/fonts/`),
are under the SIL Open Font License.

eDEX-UI's own theme files are GPL-3.0 and are not part of this repository. They
work as themes here: copy them into `~/.config/des-ui/themes/`.
