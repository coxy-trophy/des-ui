# Handoff Z: quick settings, OSD, urgent marks, keyboard icon, accent glow

Everything is wired on the branch; nothing more is needed in main.rs or runtime.rs.

## What landed
- `shell/src/quick.rs`: the quick settings pull-down at the right of the status strip
  (Wi-Fi, Bluetooth, volume + mute + output picker, brightness, night light, Do not
  disturb, on-screen keyboard, battery line, LOCK / SETTINGS / POWER). Opens with
  **Super+A** (compositor key, new `quick` event), the sliders icon in the status strip,
  a click on the status readings, or `des-ui-ctl quick`. Escape or a click outside
  closes it; clicks on the status strip do not. It sits above the on-screen keyboard
  when that is open. While open it takes the compositor overlay; a scratchpad that is
  also down stays down but under applications until the quick settings close.
  Closed on lock. Keys do not reach the terminal while it is open.
- `shell/src/osd.rs`: a volume/brightness meter for changes from outside the shell's
  sliders (media keys, wpctl, apps). It hangs at the top right of the workspace where
  toasts begin, so it rides the toast rectangle (toasts move down below it): no new
  overlay. Quiet while Settings or the quick settings are open; the shell's own
  Volume/Brightness/Mute changes hold it off for 1.5 s. Fades with the motion easing
  unless Animate is off. (Deviation: top right rather than centred low, so the lifted
  area stays one rectangle with the toasts.)
- Urgent windows: a breathing accent dot on the dock icon (steady without Animate);
  palette window results tagged `WANTS ATTENTION` and sorted right after CALC/CLIP.
- `Icon::Keyboard` and `Icon::Quick`; a keyboard toggle in the status strip.
- `panels::status` now returns a `StatusClicks` struct.
- The phosphor glow uses the custom `accent = r, g, b` when set (`phosphor::custom_accent`),
  and an accent change sends `ReloadInput`.
- `radio.rs` gained `wifi_glance`/`toggle_wifi`/`bluetooth_glance`/`toggle_bluetooth`;
  `sound.rs` gained `quick_outputs` (outside the brief's list, needed for the panel).

## README

> **Quick settings** — **Super+A**, the sliders icon at the right of the status strip,
> or a click on its readings: Wi-Fi, Bluetooth, volume and the output device,
> brightness, the night light, Do not disturb and the on-screen keyboard, with the
> battery and buttons to lock, open the settings or the power menu. Escape or a click
> elsewhere closes it. When the volume or brightness changes from the keyboard's keys
> or another program, a small meter shows it for a moment at the top right. A window
> that asks for attention gets a pulsing dot on its dock icon and comes first among
> the palette's windows.

## For docs/UNTESTED.md

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
