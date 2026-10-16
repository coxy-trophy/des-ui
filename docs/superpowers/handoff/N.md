# Handoff N: accessibility and the on-screen keyboard

Done on the branch:

- **AccessKit.** eframe's `accesskit` feature is on in shell/Cargo.toml (it is in
  eframe's defaults, which the shell turns off). The custom widgets now name
  themselves: `ui::button` and `ui::chip` (label, selected), `ui::toggle` (a check box,
  "Switch"), `ui::slider` (value, "Level"), `ui::keycap` (the key), `settings::switch_row`
  (the switch takes its row's name), launcher rows ("<label>, <KIND>: <detail>",
  selected), and the terminal tab bar (`main.rs::tab`, name and running program,
  selected). `ui::icon_button` and dock icons were already named.
- **On-screen keyboard.** `shell/src/osk.rs`: `Osk { open, .. }`, `Osk::height()`,
  `Osk::inject(ctx)`, `Osk::show(ui, theme, rect)`. Letters and a symbols layer;
  Shift, Ctrl and Alt latch for one key; keys are 44 px and not focusable, and the
  field that had the keyboard gets it back. A press becomes `Event::Text` (or
  `Event::Key` with modifiers for named keys and Ctrl/Alt chords) in the next frame.
- **Setting.** `osk = on|off` in config.rs (`Config::osk`, default off); Settings →
  Keyboard → ON SCREEN has the switch. (The `self.keyboard(..)` call in
  `Settings::show` gained the `config` argument for it.)

## For the parent to wire

1. `App` gets `osk: osk::Osk`, with `osk.open = config.osk` in `App::new`, and again in
   `apply`'s `Change::Set` arm when `key == "osk"`.
2. In `runtime.rs::update`, call `self.osk.inject(ctx)` **first**, before
   `take_shortcuts` and anything else that reads input; that is how presses reach the
   terminal and text fields.
3. Room: after `let status_top = screen.bottom() - des_common::STATUS_HEIGHT;` add
   `let osk_top = status_top - self.osk.height();` and use `osk_top` instead of
   `status_top` as the bottom of the left and right columns, the file panel and the
   terminal/workspace region; then draw
   `self.osk.show(ui, &theme, Rect::from_min_max(pos2(screen.left(), osk_top), pos2(screen.right(), status_top)))`
   inside the CentralPanel. The compositor's workspace is unchanged, so an
   application in view covers the keys; that is right, since they cannot type into it.
4. Toggle: a status-strip icon beside SETTINGS (needs an `Icon::Keyboard` in ui.rs and
   a sixth return value from `panels::status`), and **Super+K** — the compositor does
   not know K, so with the shell focused take `Ctrl+Shift+K` in `take_shortcuts` and,
   if wanted, add `K` to the compositor's Super map as an event the shell answers.
   Both should flip `self.osk.open` and save `osk`.

## README

Under Settings → Keyboard: "On-screen keyboard — keys along the foot of the
desktop for a touchscreen; types into the terminal and the desktop's own fields.
Shift, Ctrl and Alt hold for one key; 123 switches to symbols."
A short Accessibility paragraph: the shell exposes its controls to screen readers
through AccessKit (Orca on GNOME's AT-SPI bus).

## docs/UNTESTED.md

- Orca reads the launcher's results with their kind, the settings switches with
  their row name and state, the tab bar and the dock.
- The on-screen keyboard on a real touchscreen: every key, Shift/Ctrl latching,
  Ctrl+C in the terminal, typing into the launcher and a settings field, and that a
  key never takes the keyboard away from the field.
- The keyboard with an application in view (it is covered, by design).
