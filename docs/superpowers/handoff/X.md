# Handoff X: overview, Alt+Tab, gestures

## Done on the branch

- `compositor/src/overview.rs`: the overview's layout (`cells`, `pack`), filtering,
  arrow navigation, the Super tap, its state, drawing (`overview::elements`) and its
  key/pointer handling on `DesComp`.
- `compositor/src/switcher.rs`: recency order, the switcher's strip layout.
- `compositor/src/gestures.rs`: three/four-finger swipe decisions.
- Hooks: `input.rs` (keys, pointer, gestures), `state.rs` (fields, refresh before each
  frame, the pointer-gestures global), `handlers/mod.rs` (recency on focus),
  `handlers/buffers.rs` (gesture delegate), `policy.rs` (`Command::Overview`, close on
  lock, recency on close, `window_title`/`window_app_id` made `pub(crate)`), `motion.rs`
  (`window_at` made `pub(crate)`), `udev.rs` and `winit.rs` (drawing).
- Protocol: `Event::Overview(Option<OverviewLayout>)` and `Command::Overview`, tested.
- Shell: `shell/src/overview.rs` draws the names and selection; `desktop.rs` keeps the
  layout; `runtime.rs` draws it; `des-ui-ctl overview`.

## Deviations from the brief, and why

- **No `Command::Overlay` for the labels.** The shell's window is opaque, so an overlay
  covering the screen would hide the pictures. Instead the compositor draws the shell's
  window *beneath* the pictures (overview) or a cropped strip of it beneath them
  (switcher), and the shell paints the ground and the labels there.
- **Swipes do not follow the finger yet.** A swipe switches workspace when the fingers
  lift, with the usual 220 ms slide. Following the finger needs the slide driven by a
  progress value rather than a timeline; `Motions::slide` is the place to add it.
- **Other displays get no labels.** The shell draws into its main window only; on
  another display the overview is pictures on the plain ground, the switcher a plain
  panel. `screen_views.rs` could draw them in the backdrop viewports later.
- Pinch and hold gestures are always forwarded to applications.

## For the parent

Nothing to wire beyond merging; `runtime.rs` already draws the layout.

## For docs/UNTESTED.md

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
