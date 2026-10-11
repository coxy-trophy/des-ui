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
