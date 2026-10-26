# Not yet tried on real hardware

Hardware testing began on 26 October 2026. See [the machine validation record](validation/2026-10-26-machine/RESULTS.md) for actual passes, failures and pending cases. A checked item applies only to the recorded hardware/binaries. This is an incomplete run; TV and gesture follow-ups were deferred by the user.

Things that are built and pass their automatic tests, but that nobody has yet
watched work on a real machine. Tick them off, or note what went wrong.

## Hand control (Settings → Mouse & touchpad → Hand control, Super+H)

- [ ] Super+H turns it on: the status bar says CAMERA ON, and CAMERA ON  HAND once a
      hand is seen; Super+H or a click on it turns it off, and the camera light goes out
- [ ] An open hand moves the pointer smoothly, gliding between camera frames, on
      each display; a fist lifts the hand without moving the pointer
- [ ] A quick pinch clicks where the pointer was; pinch and move drags a window or
      selects text; a pinch held still opens a context menu
- [ ] Two fingers up scroll a page, the way the touchpad's natural scrolling goes;
      a quick sideways sweep changes workspace
- [ ] The setting turns it on at the start of the next session; the speed slider
      changes how far the pointer goes
- [ ] Without a camera, or with the camera in use elsewhere, the status bar says
      HAND CONTROL STOPPED and clicking it tries again
- [ ] Logging out, or the session ending, turns the camera off

## Sound

- [ ] With a TV on HDMI, Settings → Sound and the quick settings list it by name;
      choosing it plays through the TV, and choosing Speakers goes back

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
- [ ] A TV on HDMI: deep blacks and no washed-out grey (full-range colour), the
      whole picture visible with no edges cut off, and crisp text (its PC or
      graphics mode); the log says "connector set for a sharper picture"
- [ ] A TV plugged in for the first time, with nothing saved, starts at 1080p
      120 Hz or another smooth mode, not 4096×2160 at 24 Hz
- [ ] A monitor never set up here starts at a scale that suits it: a 55" TV at 4K
      at 200%, a 27" 4K monitor at 150%, a 1080p laptop at 100%
- [ ] The mode list leaves out shapes the screen does not have (4096×2160 on a
      16:9 TV) but keeps the mode in use
- [ ] Movement on a 24 or 30 Hz display is even, and no worse on the laptop; the
      log's once-a-minute "frames" lines show fps close to each display's rate

## Screenshots and recording

- [ ] FULL SCREEN, AREA and WINDOW each save the right picture
- [ ] The letter keys in the chooser (F, A, W, R, Enter, Escape)
- [x] The keyboard goes back to the application after the chooser closes
- [ ] Alt+Print and Shift+Print
- [x] Copy to the clipboard (needs `wl-clipboard`)
- [x] A real recording (needs `ffmpeg`): starts, shows `REC` in the status strip,
      stops, and plays back with the pointer in it
- [x] The toast when `ffmpeg` is missing
- [x] Locking during a recording stops it; no locked screen is captured
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
- [x] A `.command` file with `terminal = yes` runs in a tab; `exec` with quoted arguments

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
- [x] While locked, `launcher` is refused and `lock` is a no-op; `ping` still answers
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
- [x] A program that hangs shows "no answer" and the others keep updating
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
- [x] Locked: a portal request is refused at once and nothing is captured
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

## Layer shell (bars, wallpapers, launchers)

- [ ] `swaybg -i <picture>`: the picture shows in the main workspace and on the other
      displays, behind applications, never over the frame or the lock screen
- [ ] `waybar` at the top and at the bottom: applications stay clear of it on that
      display; quitting it gives the room back
- [ ] `wofi --show drun` / `fuzzel`: takes the keyboard at once, gives it back on close
- [ ] `mako` + `notify-send`: the notification shows above applications and can be clicked
- [ ] A waybar tray menu opens within the display
- [ ] Lock with all of the above running: none of them shows or takes keys until unlocked
- [ ] Screenshot chooser with waybar up: the bar stays visible, clicks go to the chooser

## Standard protocols (scaling, middle-click, activation)

- [ ] At 150%: Firefox, a GTK4 application and a Qt 6 application are sharp, not blurry; the
      shell itself is sharp; moving a window between a 100% and a 150% display changes it
- [ ] Middle-click paste from a Wayland application to an X11 one (xterm) and back
- [ ] Ctrl+click on a link in the terminal brings the running browser forward; a link
      opened by a background program marks the browser urgent instead of taking the keyboard
- [ ] Nothing is brought forward over the lock or the screenshot chooser
- [ ] Video in mpv and a browser plays without judder; `mpv --video-sync=display-resample`
      reports the display's refresh
- [ ] The I-beam over text fields and the hand over links in GTK4 and Qt 6 applications;
      the hotspots point where they should
- [ ] A Flatpak application's file chooser stays attached to its window

## Administrator password (polkit)

- [ ] `pkexec true` in the terminal: the dialog shows the action and your name; the right
      password succeeds, and `pkexec` exits 0
- [ ] GNOME Software (or `pkcon install`): installing a package asks here and proceeds
- [ ] A wrong password three times: each refusal says so, the third fails the request
- [ ] CANCEL and Escape: the program reports the request was dismissed
- [ ] A request made while the screen is locked (e.g. `sleep 5; pkexec true` then lock): nothing
      shows over the lock screen; the dialog appears after unlocking
- [ ] A user who is not an administrator: the dialog offers the administrators to choose from
- [ ] A fingerprint or security-key PAM stack: the helper's info lines show in the dialog

## Autostart

- [ ] A real login starts the entries in `~/.config/autostart` and `/etc/xdg/autostart`
      (e.g. Nextcloud, a GNOME keyring entry marked `OnlyShowIn=GNOME` stays out)
- [ ] Super+Shift+R does not start them a second time; logging out and in does
- [ ] An entry with `X-GNOME-Autostart-Delay=10` starts about ten seconds in
- [ ] Turning a system entry off and on in Settings → Session leaves no file behind
- [ ] ADD with a quoted argument (`foo --name "a b"`) starts exactly that next session
- [ ] polkit-gnome or polkit-kde installed: their agent is not started

## System tray

- [ ] Discord, Steam, Nextcloud, Telegram and `nm-applet --indicator` each show an icon,
      with the right picture and tooltip
- [ ] Left click opens or raises the application; right click opens its menu; entries
      with submenus, check marks and radio buttons work and close the menu
- [ ] An item that changes its icon or status (Nextcloud syncing, a new Telegram
      message) updates within a second; a Passive one disappears
- [ ] Quitting an application removes its icon; starting it again brings it back
- [ ] Restarting the shell (Super+Shift+R) brings every icon back without restarting
      the applications
- [ ] With another watcher running (e.g. `snixembed` or a KDE session's), the shell
      shows the same icons as a host

## File panel: quick-look and dragging files out

- [ ] Space on a PNG, a large JPEG, a GIF, a source file, a 10,000-line log, a PDF
      (with and without poppler-utils), a zip, a .tar.xz, a folder of 50,000 files and a
      binary; ← → through a folder of mixed files; Escape returns keys to the terminal
- [ ] The preview over a workspace showing an application: the terminal comes into
      view under it
- [ ] Drag a file from the panel into Firefox (an upload field), Nautilus, a GTK4 text
      editor and a Qt application; the drop arrives as the file, not as text
- [ ] Drag a file named with spaces and non-ASCII letters
- [ ] Drag onto an X11 application (expected: no drop) and onto the shell itself
- [ ] Locking while dragging: the drag is cancelled, nothing is dropped
- [ ] Clicking a file then typing in the terminal after a click there: keys reach the
      terminal again

## Calendar and notifications

- [ ] Click the clock: the calendar opens below it, today in the accent; arrows change
      month; a day with events has a dot; Escape and a click elsewhere close it
- [ ] With an application open, the pop-up shows above it (lifted like the toasts)
- [ ] A Google or Nextcloud calendar's `.ics` address in `calendars.list`: events show
      after the first fetch, and again after 30 minutes offline from the cache
- [ ] A weekly meeting with an exception and a monthly one on the 31st look right
- [ ] An event 14 minutes away is announced once
- [ ] Locale `en_US` starts weeks on Sunday, `en_GB` on Monday
- [ ] A chat application with inline reply (e.g. a KDE Connect SMS, Telegram Desktop):
      REPLY, type, SEND; the application receives the text
- [ ] Notifications from three applications: grouped, folding a group, CLEAR ALL
- [ ] Quiet hours across midnight: non-urgent notifications stay in the list, urgent
      ones still show; turning the switch off ends it at once

## Overview, Alt+Tab and gestures

- [ ] Super pressed and let go opens the overview; Super+key still does its shortcut
      and opens nothing; Super+W toggles it; `des-ui-ctl overview` too
- [ ] The overview on a wide, a narrow (2×2) and a turned display; pictures update
      live (a playing video moves)
- [ ] Click a picture, click an empty workspace, drag a picture to another workspace;
      Escape; arrows and Enter; typing narrows and Backspace widens
- [ ] Super+L while the overview is open locks at once and nothing of it shows after
      unlocking; Print closes it before the chooser
- [ ] Alt+Tab, Alt+Shift+Tab and letting go of Alt; Alt+` with two windows of one app;
      a VM holding the keyboard keeps Alt+Tab for itself
- [ ] Three- and four-finger swipes left, right, up, down on a real touchpad; a
      two-finger scroll and a pinch-to-zoom still reach the browser
- [ ] The opening and closing animation with Animate on and off

## Layouts and dividers

- [ ] Drag the gap between two windows: the divider follows, the windows resize without
      tearing and settle crisply when let go; the double-arrow pointer shows over the gap
- [ ] Super+Shift+S round all four layouts with 1, 3 and 6 applications open; the ones
      used most recently fill new slots; the window in use keeps the keyboard
- [ ] Super+Alt+Left/Right in each layout; Super+Enter in three columns and in master
      and stack; beside the terminal, Super+Enter covers the terminal
- [ ] Three columns on a 1366-wide laptop panel with both side panels open: fewer
      columns, nothing narrower than 320 px
- [ ] Different layouts on workspaces 1 and 2 and on a second display, kept across
      switching back and forth
- [ ] Save a session with three columns and master and stack, restore it after logging
      in again: layouts, dividers and positions come back
- [ ] Closing the middle of three columns closes the gap; closing the master promotes
      nothing silently (the terminal or backdrop shows in the first slot)
- [ ] An X11 application in each slot

## Quick settings, the meter and attention marks

- [ ] Super+A over a full-screen application: the panel shows above it and closes on
      Escape and on a click elsewhere; the application keeps the keyboard afterwards
- [ ] Wi-Fi and Bluetooth switches turn the radios off and on; the connected network
      and devices are named
- [ ] Volume and brightness sliders and mute; choosing another output device
- [ ] Night light and Do not disturb from the panel match Settings afterwards
- [ ] Media keys show the volume meter; brightness keys the brightness meter; dragging
      the slider in Settings or the panel shows none
- [ ] The meter and a notification together: both above an application, no gap of
      terminal showing between them
- [ ] A window demanding attention (e.g. a terminal bell in an unfocused app, or a
      browser opened from a link): the dock dot pulses, the palette lists it first
- [ ] The keyboard button in the status strip shows and hides the on-screen keyboard
- [ ] A custom accent tints the phosphor glow straight after Apply color


## Live terminals across graphical shell restart

Use an updated compositor and GUI with tmux >=3.3. Existing terminals from before
this feature cannot be converted; save their work before the first desktop update.

- [ ] In a fresh development desktop, open three terminals, record `echo $$`, change
      directories, leave a long-running command and a background job active. Split
      two tabs, focus the second pane, and open then hide the scratchpad.
- [ ] Restart the graphical shell (Super+Shift+R): same shell PIDs, running jobs,
      directories, tab order, split orientation/focus, and hidden scratchpad return.
- [ ] Exit one terminal shell explicitly, restart again: that terminal stays closed.
- [ ] Turn off Settings → Session → Keep terminals across shell restarts: existing
      live tabs reconnect, new tabs say Temporary and close when the shell restarts.
- [ ] Start another GUI against the same desktop: it cannot steal live terminals.
- [ ] Log out: the desktop's private tmux backend ends; personal tmux servers remain.
- [ ] Force one reconnect/enumeration failure: processes and original layout remain
      available for another restart, with a visible recovery warning.


## Interactive World View

- [ ] Drag the compact globe horizontally and vertically; rotation follows the
      pointer, stops on release, and HOME returns to the local marker.
- [ ] Scroll over the sphere to zoom; scroll over a different panel without moving
      the camera. Check trackpad and touch input on the actual desktop.
- [ ] Hover/click near-side markers: names/coordinates and highlighted arcs; no
      selection of markers behind the planet.
- [ ] EXPAND brings the workspace forward with search focused. Type a destination,
      clear search, select a row, and close with CLOSE/Escape. Existing terminals
      and running commands remain available. Check switching to Settings/Launcher.
- [ ] Test narrow workspace, split application layout, theme/accent changes and
      long destination names; controls remain visible and results scroll.
- [ ] Disable animation/rotation, lock/unlock, pause/resume, turn location lookup
      off, and disconnect a selected destination. No idle animated repaint while
      motion is disabled; disconnected selection is labelled no longer current.


## Application launcher grid

- [ ] Open Apps/Super+Space: installed icons and short app names form a grid.
- [ ] Click an app; focus a running app; right-click and pin an app to the dock.
- [ ] Use Left/Right and Up/Down, Enter, Escape, and Tab/Space. Scroll to a lower
      row and verify keyboard selection remains visible.
- [ ] Type/search/clear, edit the caret with Left/Right, then navigate results.
      Settings, files, windows, calculator, clipboard and unit actions still work.
- [ ] Check a narrow/split workspace, long names, missing icons and theme changes.
      Names stay inside tiles; complete names/details appear on hover.


## Managed terminal wheel regression

- [ ] In a preserved tab, print several hundred lines and wheel up/down at the
      prompt. Type afterwards: return to live output without losing the first key.
- [ ] Run `less` on a large file and Vim with mouse disabled, then enabled. Wheel
      in both directions operates the program, without entering backend history.
- [ ] Restart the graphical shell with work saved: scrolling still works in
      reattached tabs. Test a temporary tab as well.
- [ ] Hover another panel/globe while scrolling: terminal stays put; hover an
      unfocused split terminal pane: that pane receives the wheel.


## Terminal-to-app stale patch regression

- [ ] Switch repeatedly from terminal/workspace views to a browser through the
      dock and Super+Tab. No old UI patch remains above browser content.
- [ ] Repeat while toast/OSD/calendar rectangles expire or close, and while
      scratchpad/quick settings hide. Only visible overlays stay above the app.
- [ ] Interrupt/reconnect compositor IPC or fill the action queue in a development
      session, close overlays, and confirm clears recover after readiness.
- [ ] Lock/unlock with overlays present: no shell patch appears over the lock,
      and the current cleared state is applied after unlocking.


## Wallpaper and frosted surfaces (2026-10-20)

- [ ] Settings → Appearance: choose PNG/JPEG, preview, Apply appearance, Cancel changes, Reset appearance
- [ ] Source image moved/deleted after Apply; wallpaper survives shell restart
- [ ] Fill/Fit, blur 0–40, dim and surface opacity on bright/busy/dark images
- [ ] Solid/Frosted, Reduce transparency and Reduce motion; dense terminal ANSI, links and less/Vim wheel input
- [ ] Switch terminal↔apps repeatedly: no stale dock/notification rectangles
- [ ] Lock/unlock, screenshot/recording, shell restart preserving multiple terminals
- [ ] Mixed DPI, rotated/external displays, unplug/replug; independently fitted shared image
- [ ] Measure GPU/CPU and idle repaint behavior; no performance results are claimed
- [ ] Live Liquid compositor gate: alpha foreground separation, source damage/capture parity and actual DRM measurements (option remains unavailable)


## Settings and native Files (2026-10-20)

- [ ] Installed desktop: Settings at narrow/wide widths; section selector/rail, all Appearance tabs, scrolling, keyboard focus and accent controls.
- [ ] Open Files from FILES and launcher search; Back/Forward/Up, typed path Enter, local search, hidden files, long/native-byte names and document opening.
- [ ] In a disposable folder: new folder, rename, copy/paste, cut/paste, conflict handling, cancellation during a large copy, Trash and Restore. Confirm source/destination contents.
- [ ] Cross-filesystem move/Trash errors preserve the source; permissions and unavailable/removable mount failures remain actionable.
- [ ] Trash original names, restore conflict, missing original parent and interoperability with other desktop Trash implementations.
- [ ] Wallpaper picker Escape/cancel, closing Settings, switching to scratchpad/quick settings, lock/unlock and returning: no hidden keyboard capture.
- [ ] Real desktop performance for large/slow folders and copy progress. Worker bounds do not interrupt a blocked filesystem syscall.


## Wallpaper continuity correction

- [ ] Existing v1 wallpaper + Solid configuration loads into the visible Frosted preset; explicit v2 Opaque and Reduce transparency remain opaque.
- [ ] No bright/grey frame around the terminal. Wallpaper forms one aligned blurred backdrop through outer padding, split gaps and gutters at all supported scaling factors.
- [ ] Bright/dark/busy wallpapers at low/default/high tint; glyphs stay crisp, selection/cursor/find highlights remain clear and terminal apps retain explicit ANSI backgrounds.
- [ ] Compare native Settings, Files, launcher, dock and terminal for consistent depth; do not mark the visual work accepted from unit tests alone.


## Desktop visual consistency pass

- [ ] Installed desktop, bright/dark wallpaper and Opaque/Reduce transparency: compare terminal, Settings, Files, launcher, panels, music, dock, Quick Settings, calendar, notifications and OSD side by side.
- [ ] Terminal tabs, split focus marker, find toolbar, scratchpad, less/Vim scroll/input, links and cursor/selection retain behavior; no stale glass rectangles after app switching.
- [ ] Tab through buttons/toggles/chips/icons/sliders; focus is visible without pointer hover. Slider arrows/Shift/Home/End affect only the focused control.
- [ ] Narrow/wide Settings fields, wrapped descriptions, Files metadata/search/More menu and image picker; no clipped controls or unrelated keyboard capture.
- [ ] Verify screen-sized and floating materials across DPI/output changes, modal/authentication/lock transitions, reduced motion, media and screenshot previews.
- [ ] Native screenshots and compositor CPU/GPU checks are required; static painter and interaction tests are not aesthetic acceptance.


## Configurable shortcuts and reference

- [ ] Install matching compositor and shell builds. Super+/ opens/closes the reference over terminals and applications; Ctrl+Shift+H opens it in a development shell. Search Keyboard shortcuts in the launcher.
- [ ] Right panel has no KEYS list; reference search/clear, groups, disabled labels and Configure destination match current mappings. Inspect at320/360/wide widths and short displays.
- [ ] Settings → Shortcuts: manual Super entry, explicit Ctrl recording, conflict/reserved-key errors, disable, reset one, confirm/cancel reset all. Backspace/arrows/paste work in the manual field.
- [ ] Remap shell/desktop/terminal-find actions. Old keys cease acting, new keys work; mappings survive shell restart and compositor reload. Confirm Ctrl+Alt development aliases and shifted punctuation on actual XKB layouts.
- [ ] Reference Escape/Arrow/Enter never affect an underlying launcher/file panel/terminal; scratchpad resumes afterward. Lock/authentication/capture and shortcut-inhibited applications retain priority.
- [ ] Simulate config save failure, compositor disconnect and busy IPC queue: old mappings remain after failed writes; committed mappings are reloaded on reconnect. Native global routing and overlay acceptance remain unverified here.


## Window shelf and drag destinations

- [ ] Install matching compositor and shell; inspect running markers, counts, pins, All windows, narrow displays, large pin collections and per-display shelves.
- [ ] Choose among multiple live windows, including more than nine: arrows, Page Up/Down, Enter, Escape back to the dock; check the older-compositor list fallback.
- [ ] Check collapsed left clock in 12/24-hour formats, minute rollover and short displays.
- [ ] Drag from Files and native Wayland apps; dwell over previews, drop inside the revealed app, cancel with Escape, switch displays, close the source and lock mid-drag. Confirm negotiated copy/move and payload delivery.
- [ ] Test actual browsers exposing native tab drags, including Xwayland separately. Internal browser reorder gestures may never create a desktop drag.
- [ ] Toggle Settings Dock drag overview; check persistence, ordinary dragging while disabled, animation-off, Reduce motion and Reduce transparency.
- [ ] Review animation, preview labels, wallpaper continuity and compositor CPU/GPU use in the installed session. Automated tests do not establish native visual acceptance.


## Comfortable hand gestures

Status: awaiting installed-session testing. On 2026-10-23 the user deferred logout, so the new gesture mappings, comfort settings and pointer feedback have not been tested in their desktop session. Keep the checks below open until matching binaries are installed and a fresh session can be tested.

- [ ] Try relaxed fingers and small palm movements seated at the actual camera; verify no click merely by entering the view pinched.
- [ ] Light index pinch clicks once and holding never right-clicks; middle pinch right-clicks once. Confirm aim stays on small targets.
- [ ] Two quick taps double-click. Tap then pinch again and move grabs; relax fingers to carry and pinch to drop. Repeat with hold-pinch mode disabled/enabled.
- [ ] Fist, lower hand, tracking loss and picture edges release drag once; returning reanchors without a jump.
- [ ] Adjust sensitivity and scroll speed, including while temporarily enabled/paused by Super+H; verify saved preferences and no stuck grab on reload.
- [ ] Confirm click/grab markers in applications and terminals across DPI/output changes, fullscreen, reduced motion, lock and capture. Review physical comfort and actual camera jitter; automated synthetic hands cannot establish these.


## Solid morph cursor

Status: implemented, awaiting fresh installed-session acceptance. The user cannot log out now; no running session was changed.

- [ ] Check pointer/link/text/grab/resize/busy morphs in shell, settings, Files and native applications; clicking remains at the same point while changing shape.
- [ ] Check all resize directions, copy/move/forbidden/precision/zoom semantics, rapid retargeting and drag-overview interactions.
- [ ] Inspect 1×, fractional and high-DPI outputs, monitor crossings and small-size contrast on bright/dark wallpaper; confirm no duplicate host cursor in ./dev.sh.
- [ ] Confirm animation-off and Reduce motion snap/freeze; custom cursor images and deliberate hidden pointers remain intact; close an app supplying a cursor surface.
- [ ] Confirm pointer-inclusive recording/capture shows the same cursor and does not reset resize morphs. Lock, screenshot selection and startup splash retain ownership.
- [ ] Verify DEs-UI fallback theme in older Wayland clients and Xwayland; explicit cursor theme/size/path choices remain respected. Theme files are static endpoints, not cross-state morphs.
- [ ] Assess idle and busy cursor CPU/GPU use in the installed compositor. Automated geometry/cache tests and libXcursor loading are not native visual acceptance.


## Cursor color and size

Status: implemented with automated cursor checks complete; installed-session acceptance deferred because the user cannot log out. Full workspace checks also retain the terminal backpressure timeout documented in docs/validation/2026-10-23-cursor-preferences.md. No installed binaries or desktop settings were changed by the agent.

- [ ] Mouse & touchpad: adjust size16–64px, Apply/Reset opaque cursor colors, Reset size; verify persistence and exact previews on light/dark grounds, including black and white.
- [ ] Verify live native changes during morphs, app switching and dragging; pointer click point stays fixed at all sizes and display scales.
- [ ] Reopen older apps after changing size/color; check logical size on1×/2×/3×/4× outputs, custom theme overrides, and newly launched apps after reconnect.
- [ ] Simulate an unwritable/full cursor cache; named cursor stays usable, prior legacy generation remains available, and later reload retries publication. Check no render stalls during rapid slider changes.

## Cursor AI companion —2026-10-23

Implementation and mocked tests are present; no paid OpenAI request, real microphone or installed-session activation has been performed. Existing gesture/cursor/session checks above remain open. No shell restart or installation was performed.

- [ ] Settings → Companion setup with a locally entered key; text reply, failure/quota feedback, model change retaining the key, worker/shell restart requiring re-entry unless environment configured.
- [ ] Global and shell text/voice mappings, Escape/focus restoration against terminal and native apps, no key leakage into another app; visual acceptance at actual scaling and multiple displays.
- [ ] Real GPT-Live access, connection latency, microphone/output defaults, speaker echo vs headset, natural interruption and playback quality. Default PipeWire source/sink are used.
- [ ] Quick summon follow-up auto-close, Keep talking, mute vs stop, five-minute cutoff while speaking, explicit resume preserving context, no automatic reconnect.
- [ ] Stop/lock/authentication/screenshot/shell restart ends mic and audio immediately. Test worker crash while microphone is live.
- [ ] Finalized and unconfirmed usage presentation against actual provider accounting; daily limits include backend costs separately from connection minutes.
- [ ] Social opt-in bubble on exposed desktop, Talk/Quiet hit targets, ignored approach after restart, app focus/fullscreen/drag/recording suppression and cadence.
- [ ] Self-authored identity evolution across text/voice context; edit, restore, reset, approved memory/forget/clear and history limits.

Deferred controls: spoken “keep talking” recognition, disconnected local wake detector, provider alternatives, per-companion audio device selectors, and explicit screenshot/file attachments. No ambient screen/file access exists. A five-minute cap is an application limit, not a verified exact provider billing cutoff. Text replies are completed responses; token-by-token text streaming is not implemented.

### Companion setup and thought-bubble revision

The reported “Voice connection failed” did not identify its underlying cause. Longer handshake allowance, safe status-specific diagnostics and reconnect-state repair are implemented and mock-tested; a successful real voice connection is still unverified. Retest from Settings → Companion and use the displayed diagnostic without sharing the key. Check animated thought-trail/fade near all screen edges, reduced motion, social Talk/Quiet click targets, new global control mappings and Settings navigation in the installed session. No shell restart/installation or paid test was performed during this revision.

### Bottom chat, saved key and speaking cursor —2026-10-24

- [ ] Save a real key into the system keyring; restart shell and confirm automatic restoration, locked/unavailable feedback, explicit removal, and no unexpected unlock prompt on idle startup. Development tests use fake keyrings and never inspect real saved credentials.
- [ ] Super+J opens the bottom panel's Chat tab from collapsed state over native apps. Switch Files/Chat; Enter sends once; Escape/clicking terminal/native apps returns keyboard correctly. Draft/history survive tab switches; text controls explain disabled/setup/loading states.
- [ ] Super+Ctrl+J starts voice without opening Chat. Cursor pulse and speaking caption follow actual output audio, stop/drain/mute/lock behavior and reduced motion. Pointer-transparent caption never blocks clicks or steals native app focus.
- [ ] Actual pulse quality at different cursor sizes/colors/output scales. Cursor pulse supports all outputs; captions currently use the primary shell surface and are suppressed for secondary-pointer anchors.
- [ ] Keep/Stop during key restoration, voice timeout/failure notices, five-minute cutoff and explicit continuation in the installed session.

No installation, restart, real API/keyring operation or microphone test was performed for these changes. Secret Service runtime dependencies are included in package metadata; a login keyring may need unlocking once per desktop login. Existing hardware acceptance items remain open.

## Right-panel Apps —2026-10-25

- [ ] Installed session: Apps/System tab switching, open applications, all-workspace/display groups, previews, close and workspace operations.
- [ ] Super+D reveals a collapsed right panel; Escape and native outside clicks return keyboard ownership. Repeat on secondary displays, fullscreen apps and mixed display scales.
- [ ] Pin/reorder persistence, long window titles, many running apps, wallpaper contrast and reduced motion.

Previous bottom-dock acceptance items are superseded. Mobile remains read-only. No installed session restart or installation performed.

Automated evidence and the isolated native preview: [right-panel validation](validation/2026-10-25-right-panel-apps.md).


## Separated frame panels and voice transport — 2026-10-26

Automated tests and a private native-shell preview pass. System starts selected; primary frame cards share 8px gaps/radius and a 4px native-workspace rim. Matching compositor and shell builds must be installed and activated together. Check native apps, panel collapse/hide, fractional DPI, fullscreen and Files/Chat at the next controlled login. Existing OSK/native reservation limitations remain unchanged.

Authenticated voice handshake now succeeds after suppressing the server client's synthetic Origin header (the previous request returned403). No real microphone or paid spoken conversation was started. Verify end-to-end speech, playback, interruption and lock/stop after installing the updated companion helper. See [validation](validation/2026-10-26-panels-and-voice.md).
