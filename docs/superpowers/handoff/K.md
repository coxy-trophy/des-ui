# Handoff K: panel widgets from programs

Done on the branch: `shell/src/widgets.rs` (declarations, output parsing, a pure
`Schedule`, the `Widgets` worker and `widgets::show`), examples and a format note in
`shell/assets/widgets/`, and `mod widgets;` in main.rs. Nothing is mounted yet.

## To wire

1. **App field.** In `struct App` (main.rs): `widgets: widgets::Widgets,`; in
   `App::new`, `widgets: widgets::Widgets::new(ctx.clone()),`. The worker thread starts
   there; the directory is read on the first pump.

2. **Pump.** In `runtime.rs` `update`, beside `self.mon.tick()`:
   `self.widgets.pump();` — it re-reads `~/.config/des-ui/widgets/` every 30 s, takes
   in answers, and queues due runs without blocking.

3. **Mount.** `widgets::show(ui, theme, widgets, Panel::Left | Panel::Right)` draws the
   tiles for one side and nothing when none are declared.
   - **Left column:** `panels::left` (panels.rs), after the THERMAL/STORAGE block and
     before `pet.show(ui)`. Give `left` a `widgets: &widgets::Widgets` parameter and
     count the tiles into the `sections_that_fit` reserve, so the companion keeps its
     room. About 52 px per text tile, 58 for meter and list rows, 70 for a series.
   - **Right column:** `panels::right`, immediately before
     `globe::show(...)`, same extra parameter.
   - Callers in runtime.rs pass `&self.widgets`. `backdrop` (secondary displays) needs
     nothing.

## README paragraph (under "What is in it")

> **Widgets.** A file in `~/.config/des-ui/widgets/` ending in `.widget` puts a
> program's reading in a side panel: `name`, `exec` (split like a desktop entry, never
> through a shell), `every = 5s`, `panel = left|right`, `kind = text|meter|series|list`.
> The program's last line is shown as text, or read as JSON (`text`, `value`, `label`,
> `series`, `state`, `lines`, `tooltip`). Programs run one at a time, for at most three
> seconds; one that fails says "no answer". Examples are in `shell/assets/widgets/`.

## For docs/UNTESTED.md

- [ ] The three example widgets copied into `~/.config/des-ui/widgets/` show on the
      real panels, left and right, and the companion keeps its room
- [ ] A widget added, changed or removed shows within 30 seconds
- [ ] A program that hangs shows "no answer" and the others keep updating
- [ ] A `meter` with `"state":"caution"` and a `series` filling its history
