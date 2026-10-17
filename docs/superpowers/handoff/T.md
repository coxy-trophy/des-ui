# Handoff T: wlr-layer-shell

Done in the compositor: `compositor/src/handlers/layer_shell.rs` (the handler, the
placement and reservation rules, hit-testing, keyboard policy, drawing, frame
callbacks), with small hooks in `state.rs` (three fields, frames, hit-testing),
`handlers/compositor.rs` (commit), `policy.rs` (`workspace_of`, the keyboard in
`arrange`, clicks in `focus_at`), `motion.rs` (`window_elements` takes what goes under
the windows), and both render paths (`udev.rs`, `winit.rs`, a `Cropped` element in
`OutputElements` and `TopElements`).

## What should work

- `swaybg` or `swww`: a wallpaper on secondary displays and inside the main display's
  workspace, behind applications. The frame is never covered.
- `waybar` and other bars: drawn above applications; their exclusive zone keeps
  applications clear on that display.
- `wofi`, `rofi -show drun` (wayland build), `fuzzel`, `tofi`: exclusive keyboard
  while open, and the keyboard goes back when they close.
- `mako`, `dunst` (wayland), `swaync`: notifications in the overlay or top layer,
  clickable.
- Popups (menus) of layer surfaces, e.g. waybar's tray menus, placed within the
  display and drawn with their parent.

## The rules

- Overlay and top layers: above every application, beneath the shell's toasts, dock
  and drop-down.
- Bottom and background layers: beneath every application and above the shell's own
  window, cut to the workspace on the main display; whole-display on the others.
- Locked: no layer surface is drawn, hit, or has the keyboard.
- Screenshot chooser up: layers stay in view, take no pointer or keyboard input.
- Exclusive zones: per display, the sum of each edge's reservations (zone plus that
  edge's margin, counted only for a surface anchored to one edge alone or with both
  perpendicular edges). Applications' area = their usual area ∩ what remains. On the
  main display the frame never moves, so a bar that lies wholly over the frame's top
  bar changes nothing; one reaching into the workspace pushes applications down.
  Nothing is ever shrunk to zero: an impossible reservation leaves the area as it was.
- Keyboard: `exclusive` on top/overlay takes it while mapped; `on_demand` (and
  `exclusive` on bottom/background) on a click; a click elsewhere takes an on-demand
  focus back. The dock and the shell's overlay still win while they are up.

## Known limits

- Screenshots and recordings go through `space_render_elements`, which draws
  layers in smithay's own order: background layers end up behind the shell's
  window and so are not in the picture; bars and notifications are.
- Without a display named by the client, a layer surface goes on the display in
  use at the time it is created.
- `--split` (two displays in one window) cuts beneath-layers to the main workspace
  only.

## For docs/UNTESTED.md

```
## Layer surfaces (other desktops' tools)

- [ ] `swaybg -i <picture>`: the picture shows in the main workspace and on the other
      displays, behind applications, never over the frame or the lock screen
- [ ] `waybar` at the top and at the bottom: applications stay clear of it on that
      display; quitting it gives the room back
- [ ] `wofi --show drun` / `fuzzel`: takes the keyboard at once, gives it back on close
- [ ] `mako` + `notify-send`: the notification shows above applications and can be clicked
- [ ] A waybar tray menu opens within the display
- [ ] Lock with all of the above running: none of them shows or takes keys until unlocked
- [ ] Screenshot chooser with waybar up: the bar stays visible, clicks go to the chooser
```
