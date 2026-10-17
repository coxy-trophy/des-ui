# Not yet tried on real hardware

Things that are built and pass their automatic tests, but that nobody has yet
watched work on a real machine. Tick them off, or note what went wrong.

## Display options (Settings → Displays)

- [ ] Scale: 125%, 150%, 175%, 200% on the laptop panel
- [ ] Scale on an external monitor
- [ ] Turn: on its right side, upside down, on its left side — and whether right
      and left are the right way round
- [ ] Mirror the main display on a second monitor (same shape, and a different one)
- [ ] Turn a second monitor off, and back on
- [ ] The 15-second "keep this layout?" step undoes each of the above when left alone
- [ ] A kept layout comes back after logging out and in, and after unplugging and
      plugging the monitor
- [ ] The pointer, toasts and the screenshot chooser sit in the right place on a
      scaled or turned display
- [ ] A monitor turned off: whether its backlight goes dark too

## Screenshots and recording

- [ ] FULL SCREEN, AREA and WINDOW each save the right picture
- [ ] The letter keys in the chooser (F, A, W, R, Enter, Escape)
- [ ] The keyboard goes back to the application after the chooser closes
- [ ] Alt+Print and Shift+Print
- [ ] Copy to the clipboard (needs `wl-clipboard`)
- [ ] A real recording (needs `ffmpeg`): starts, shows `REC` in the status strip,
      stops, and plays back with the pointer in it
- [ ] The toast when `ffmpeg` is missing
- [ ] Locking during a recording stops it; no locked screen is captured
- [ ] Screenshots and recordings of a second display

## Power and battery

- [ ] The power menu: Super+Escape and the power key
- [ ] Each action in it: lock, sleep, log out, restart, shut down
- [ ] The power key puts the machine to sleep when the screen is locked
- [ ] Turning the screen off when left alone, and back on at a key or the pointer
- [ ] Battery warnings at 15% and 8%
- [ ] Sleep at 4% after the one-minute warning
- [ ] Time left in the status strip, charging and discharging

## Earlier work with gaps

- [ ] Wireless data-control clipboard protocol (used by `wl-copy`)
- [ ] Media keys (play, next, previous) driving the music player

## Network and Bluetooth (Settings)

- [ ] Wired: a cable plugged in shows as connected; DISCONNECT and CONNECT work
- [ ] VPN: ADD brings in a WireGuard `.conf` and an OpenVPN `.ovpn` file
- [ ] VPN: CONNECT, DISCONNECT and FORGET
- [ ] Bluetooth: pairing a device that needs no code still works
- [ ] Bluetooth: a keyboard that shows a code to type on it
- [ ] Bluetooth: a phone that asks to confirm a code
- [ ] Bluetooth: a device that wants a PIN typed here
- [ ] Bluetooth: CANCEL while pairing

## Notifications: pictures and sounds

- [ ] The sound itself: one when a notification shows, a different one when urgent
- [ ] Several notifications at once make one sound, not several
- [ ] No sound with do not disturb on, or with sounds switched off in the settings
- [ ] Pictures from real applications: a browser's message with a sender's
      picture, a music player's cover

## Music: albums, artists, long lists

- [ ] Scrolling to the end of the liked songs, or of a long playlist, loads the rest
- [ ] ALBUMS in the sidebar; opening one; playing a track from it plays on through the album
- [ ] Clicking a track's artist, and its album (the album shows only when the view is wide)
- [ ] Albums and artists among search results
- [ ] BACK, through several lists
- [ ] The Spotify sign-in carried over from the old `edex-rs` folder still works

## Your picture (Settings → Session)

- [x] Clicking a picture sets it; it then shows in the frame above, on the lock
      screen, and on the login screen
- [x] USE with a typed path, including one starting with `~/`
- [x] REMOVE goes back to the pattern
- [x] A file that is no picture gives a "Picture not changed" notice

## Music: other devices, covers, repeat, like

- [ ] Controls while another device plays: pause, next, previous, seek, volume, shuffle
- [ ] Clicking a track while another device plays: it plays there
- [ ] DEVICES: moving the music to this machine, and from it to a phone
- [ ] Playing here after playing elsewhere: the `ON <DEVICE>` line goes away
- [ ] REPEAT in its three states, here and on another device
- [ ] LIKE: the approval it asks for the first time, then liking and unliking
- [ ] The large cover of what is playing on another device
- [ ] Scrolling a long list: covers appear as rows come into view

## The splash

- [ ] How the glitch reads in motion: how often, how hard
- [ ] It shows briefly on Super+Shift+R
- [ ] With two displays: it shows on each
- [x] After the password, the black before the splash is now about a second shorter
      (the compositor leaves without switching the panel off)
- [ ] Logging out still returns to the login screen, now that the compositor leaves at once
- [ ] `./install-boot-splash.sh`: the splash shows while booting and shutting down
- [ ] The boot splash's bar follows the boot, and the name glitches
- [ ] `./uninstall-boot-splash.sh` brings the old boot screen back

## Sound (Settings → Sound & Screen)

- [x] Clicking another output moves the sound there (speakers, Bluetooth, HDMI)
- [x] Clicking another input changes the microphone in use
- [x] The level sliders and MUTE, for the output, the input and each application
- [x] A device plugged in or connected while the page is open appears within a few seconds
- [x] Music that is playing follows a change of output

## Music on the lock screen

- [x] The card shows when something plays, here or on another device, and not otherwise
- [x] Previous, play/pause and next work with the pointer while locked
- [x] The media keys work while locked
- [x] On the real display the card sits clear of the password line

## Lyrics

- [x] LYRICS with a real track playing: the words appear and follow the singing
- [x] Clicking a line jumps to it
- [x] Scrolling by hand, then the view following again at the next line
- [x] A track whose words are not timed, and one with none
- [x] Words for a track playing on another device

## Workspaces

- [ ] Super+1 to 4 on the real keyboard, and Super+Shift+1 to 4 to send an application
- [ ] Clicking the numbered squares, and Shift+click
- [ ] Split and full screen kept apart per workspace
- [ ] A notification's click bringing an application on another workspace into view
- [ ] With two displays: each workspace's applications on both

## Users (Settings → Users)

- [ ] Adding a user, with and without administrator, then signing in as them
- [ ] Changing a full name, and a password (yours and another's)
- [ ] Making someone an administrator, and stopping that
- [ ] Removing a user, keeping files and with files
- [ ] A wrong password of your own is refused with a clear message

## Date and region (Settings → Date & region)

- [ ] The network switch: turning it on and off, and the line under it saying
      whether the clock agrees with the time servers
- [ ] Setting the clock by hand with the network off, and that it is not offered
      with the network on
- [ ] Finding a time zone by typing, choosing it, and every clock following at once
- [ ] 12 HOURS and 24 HOURS on the desktop, the lock screen and in notifications
- [ ] Changing the language, and its showing after signing in again
- [ ] Formats from another country than the language, and SAME AS THE LANGUAGE
- [ ] A language that is not on the machine yet being made first (Ubuntu, or
      Debian without `locales-all`)
- [ ] A wrong password of your own is refused with a clear message

## The companion (Settings → Frame)

- [ ] It shows at the foot of the left column on the real display, clear of the
      process list, and not at all where there is no room
- [ ] It sits, lies, walks and runs to both sides, and turns round at each
- [ ] It paws when the pointer is over it
- [ ] Each animal, and NONE
- [ ] How much the processor is used with it showing and the globe still

## Desktop workflow expansion (15 October 2026)

- [ ] Palette: focus existing apps, settings navigation, file opening and explicit folder selection with real input/IME
- [ ] Session: save on each supported distribution; restore existing/new apps across four workspaces and several displays
- [ ] Session: missing app/folder, disconnected display, 30-second expiry, repeated restore without duplicate app instances
- [ ] Docks: pointer and Super+D on each physical display, mixed scale/rotation, context menus, shared pins, hotplug
- [ ] Lock and screenshot picking suppress every dock and return keyboard focus correctly
- [ ] Health: real helper paths, fresh/missing logs, open full log, refresh and failure states
- [ ] Terminal find: large scrollback, wide/combining text, a real IME, two panes, live output and keyboard focus

## Motion (compositor)

- [ ] A window materialises (grows from 96%, fades in) when it opens; split, full
      screen and a move to another display ease over 180 ms; Super+1…4 slides the
      workspace sideways over 220 ms, higher workspaces arriving from the right
- [ ] Nothing moves while locked or while the screenshot chooser is up
- [ ] Settings → Frame → Animate off snaps everything, for windows and the dock alike
- [ ] Once nothing moves no frames are drawn: idle power is as before
- [ ] A client slow to resize is scaled into its new area rather than jumping

## Palette: hosts, processes, units and commands

- [ ] An `ssh` host from `~/.ssh/config` opens in a tab, and the tab closes when ssh exits
- [ ] `kill <name>` ends a real process of your own, and never one of root's
- [ ] `start`, `stop` and `restart` on a user unit; a system unit is refused without polkit
- [ ] A `.command` file with `terminal = yes` runs in a tab; `exec` with quoted arguments

## Instruments (the side panels)

- [ ] Temperatures and fans carry the right labels on an AMD and on an Intel machine
- [ ] The GPU meter on amdgpu, on an NVIDIA machine (`nvidia-smi`), and absent otherwise
- [ ] A machine with no sensors shows no THERMAL section and nothing else moves
- [ ] The alerts: a hot CPU, a disk past 95%, memory past 95%, a unit that fails;
      each once, and again only after it has cleared
- [ ] The per-core bars on 2, 8 and 32 cores, and the companion still has its room

## Command blocks (the terminal)

- [ ] Prompt marks in a real bash, zsh and fish session; a user whose `.zshenv` sets
      `ZDOTDIR`; starship and powerlevel10k prompts
- [ ] Marks stay in place through `yes | head -100000` once the scrollback is full,
      and after the window is resized
- [ ] Ctrl+Shift+Up/Down inside `less` or `vim`: no prompts, no movement
- [ ] A failed command's square is amber and hover shows its status

## des-ui-ctl

- [ ] Each word from a terminal inside the session, and `state` printing the open apps
- [ ] While locked, `launcher` is refused and `lock` is a no-op; `ping` still answers
- [ ] A second user on the machine is refused at the socket
- [ ] The socket comes back after the shell restarts (Super+Shift+R)

## Clipboard history (the palette)

- [ ] Copy from a Wayland app and from an X11 app (through Xwayland); both appear
- [ ] A password copied from KeePassXC or another manager that sets the hint does not appear
- [ ] Without `wl-clipboard` the palette says so under `clip` and nothing else changes
- [ ] A pin survives 50 further copies; Clear history empties it

## Phosphor and night light (Settings → Appearance)

- [ ] GLOW at 30 % and 100 %: bright text and the accent bleed softly; nothing is
      offset or upside down on a laptop panel, a rotated display and the nested window
- [ ] SCANLINES on a 100 % and a 200 % display: faint, even lines that do not shimmer
- [ ] NIGHT LIGHT across its start and end times: colours warm and return over a
      minute; a window past midnight; idle power with it on and nothing moving
- [ ] All three off: frame time and idle power as before (the pass is skipped)
- [ ] A screenshot and a recording taken with glow on come out clean
- [ ] Frame time with glow on at 1080p and 4K on an integrated GPU (target < 1 ms)

## Power-up (boot and shell restart)

- [ ] A real boot, Plymouth → compositor splash → power-up: no black frame between any two
      of them, and the name and bar do not jump at the Plymouth hand-over.
- [ ] The bar's drop at the Plymouth hand-over (the known gap above): how noticeable it is.
- [ ] The frame lines sweep out as the splash fades; the columns, file panel, terminal
      text and status strip follow, and the instruments rise from zero, all within a
      second.
- [ ] Super+Shift+R: the splash, then the power-up again.
- [ ] Starting locked, and Settings → Frame → Animate off: everything appears at once.

## Panel widgets

- [ ] The three example widgets copied into `~/.config/des-ui/widgets/` show on the
      real panels, left and right, and the companion keeps its room
- [ ] A widget added, changed or removed shows within 30 seconds
- [ ] A program that hangs shows "no answer" and the others keep updating
- [ ] A `meter` with `"state":"caution"` and a `series` filling its history

## Window rules and picture in picture

- [ ] A rule for a Wayland app (Firefox) and an X11 app (xterm) sends each to its
      workspace and display; `side` opens it beside what is there; an edited rules
      file counts for the next window
- [ ] Super+P pins a video player in the corner; it stays in view through Super+1…4;
      Super+P again puts it back in its half
- [ ] Clicking the pinned window types into it; clicking elsewhere takes the keyboard
      back; Super+Q closes it while it has the keyboard
- [ ] Locking hides the pinned window, and it comes back after unlocking
- [ ] The overlay (once the scratchpad exists): drawn above applications and below
      notifications, keys go to it, it disappears on lock and during screenshot picking

## The frame follows ssh

- [ ] `ssh` to a Linux host with a key or an agent: REMOTE heading, its processes, memory
      and network within a few seconds; back to this machine when ssh exits
- [ ] A password-only host shows NEEDS A KEY and the panels stay local; no password prompt
      appears anywhere
- [ ] A host that is down shows UNREACHABLE and is asked again only every 10 s
- [ ] A non-Linux host (macOS, a router) shows NOT LINUX
- [ ] Two tabs on two hosts: switching tabs switches the panels
- [ ] The globe draws the remote machine's public connections
- [ ] `ssh -N -L …` and `ssh -W` are not followed
- [ ] No `ssh-*` control sockets are left in `$XDG_RUNTIME_DIR/des-ui/` a minute after
      leaving

## Accessibility and the on-screen keyboard

- Orca reads the launcher's results with their kind, the settings switches with
  their row name and state, the tab bar and the dock.
- The on-screen keyboard on a real touchscreen: every key, Shift/Ctrl latching,
  Ctrl+C in the terminal, typing into the launcher and a settings field, and that a
  key never takes the keyboard away from the field.
- The keyboard with an application in view (it is covered, by design).

## Scratchpad terminal

- [ ] Ctrl+Shift+` over a full-screen application: the scratchpad slides down above it
      and the application keeps its size
- [ ] Keys go to the scratchpad and not to the application, Escape included; the same
      shortcut puts it away and the application has the keyboard again
- [ ] Locking puts it away and nothing of it shows on the lock screen
- [ ] Super+Shift+R (shell restart) with it down: the compositor drops the overlay and
      the new shell starts with it up
- [ ] `exit` in it puts it away; the next Ctrl+Shift+` starts a fresh shell

## Screenshot portal

- [ ] Firefox: a screenshot through the portal (e.g. `flatpak run` an app that asks, or
      `gdbus call --session --dest org.freedesktop.portal.Desktop --object-path
      /org/freedesktop/portal/desktop --method org.freedesktop.portal.Screenshot.Screenshot
      "" "{'interactive': <true>}"`), the chooser appears and the file is returned
- [ ] Chromium and a Flatpak app the same way; GIMP's or Inkscape's colour picker via PickColor
- [ ] Escape in the chooser makes the application see "cancelled", not a failure
- [ ] `journalctl --user -u xdg-desktop-portal` shows des-ui chosen for Screenshot and
      gtk for the rest; no 25-second waits on start
- [ ] Locked: a portal request is refused at once and nothing is captured
- [ ] A file name with spaces or non-ASCII letters arrives intact as a URI

## Input methods, pointer capture and shortcut grabs

- [ ] fcitx5 Pinyin in Firefox and in a GTK4 app (gnome-text-editor): preedit shows in
      place, the candidate list sits under the cursor, above it near the screen's foot
- [ ] The candidate list on a second display and on a scaled display
- [ ] A game or a 3D viewer (e.g. a Wayland-native Minecraft launcher, Blender's
      continuous grab) captures the mouse; relative motion is smooth
- [ ] Super+1…4, Super+L and Print release a captured pointer
- [ ] A VM (virt-manager / GNOME Boxes) or a remote desktop grabbing the keyboard: Super+Tab
      and Super+Space reach the guest; Super+L still locks, Ctrl+Alt+Backspace still ends
      the session, Ctrl+Alt+F3 still switches VT
- [ ] Locking while a VM holds the shortcuts: the lock screen gets every key
