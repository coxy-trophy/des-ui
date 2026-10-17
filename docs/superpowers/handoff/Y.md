# Handoff Y: resizable splits and layouts

Done on the branch:

- `compositor/src/layout.rs` (new): geometry for every layout, divider positions and the
  8 px grab band, drag-to-share conversion, ratio clamping (a fifth to four fifths, and
  no column narrower than 320 px), the degrade rules, and two pure planners:
  `plan` (where applications go when the layout changes) and `swap_into_master`.
- `compositor/src/policy.rs`: per-display, per-workspace `layout` and `ratios` in
  `ScreenMode` (so they travel with the workspace like split and full screen did);
  `focus_slot` replaces `side_focused`; slots past the second are `Place::Extra(n)`;
  holes left by closed windows are closed up; recency (`App::used`) chooses what fills
  new slots; `set_layout`, `cycle_layout`, `set_ratio`, `nudge_ratio`, `swap_master`,
  `layout_at`, divider drag (`start_resize`/`drag_resize`/`end_resize`), and
  `layout_report`. While a divider is dragged its display's windows are configured at
  most every 100 ms and drawn scaled by the motion system in between.
- `compositor/src/input.rs`: Super+Shift+S, Super+Alt+Left/Right (Ctrl+Alt+Shift in a
  window), Super+Enter; left-button drag on a divider (neither press nor release
  reaches the windows); `compositor/src/handlers/cursor_shape.rs` a double-arrow pointer
  over and while dragging a divider (udev only; nested, the host draws the pointer).
- Protocol (`common/src/ipc.rs`): `Place::Extra(n)` on the wire as 3..5 (0..2 unchanged,
  so a state without further slots is byte-identical); `LayoutKind`; event
  `layouts …` (sent on connect and on change); commands `layout <kind>`, `ratio <share>`,
  `layout-at <w>:<s>:<kind>:<r1>:<r2>`.
- Shell: the SPLIT button is a four-icon layout chooser (`main.rs::top_bar`,
  `ui::Icon::Layout*`); `Desktop::layouts`; `control.rs` reports further slots as
  `slot-2`…; session snapshots are version 2 with `layouts` and `Extra` places, and
  version 1 files still load (`session_restore.rs`); restore sends the layouts before
  the placements (`workflows.rs`).

Nothing for the parent to wire.

Known: an older shell talking to this compositor rejects a `state` line holding a place
of 3..5; shell and compositor ship together, so this only matters across a half-upgrade.
The compositor's `--help` listed Super+Shift+S as a region screenshot and Super+R as
recording, neither of which `input.rs` binds; the Shift+S line is replaced, the R line
was left as it was.

## README (keys table rows)

| <kbd>S</kbd> | Split the workspace between two windows, or make it whole again |
| <kbd>Shift</kbd>+<kbd>S</kbd> | Next layout: one window, two side by side, three columns, a master with up to four stacked beside it |
| <kbd>←</kbd> <kbd>→</kbd> | Give the keyboard to the window on that side |
| <kbd>Alt</kbd>+<kbd>←</kbd> <kbd>→</kbd> | Move the divider beside the window in use |
| <kbd>Enter</kbd> | Make the window in use the master, the first column |

## README paragraph (after the split paragraph)

The workspace can be shared four ways, chosen with the four small icons in the bar
above it or with Super+Shift+S: one application, two side by side, three columns of the
three used most recently, or a master on the left with up to four stacked on the right.
Drag the gap between two windows to move the divider, or use Super+Alt+Left and Right;
Super+Enter makes the window in use the master. Each workspace and display keeps its
own layout and dividers, and a saved session keeps them too. Where three columns would
be narrower than 320 pixels, fewer are shown.

## For docs/UNTESTED.md

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
