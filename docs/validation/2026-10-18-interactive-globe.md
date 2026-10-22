# Interactive globe validation — 2026-10-18

Scope: the approved first set of globe enhancements: drag rotation, bounded zoom,
Home/pause/resume, hover details and arc emphasis, pinned destination selection,
and an expanded searchable workspace. Keep existing land/wire aesthetics and
geo lookup policy. Day/night, animated traffic and richer process/network
attribution are deferred.

## Ownership and integration

`globe::Explorer` owns shared ephemeral camera and pinned Place state. Existing
`geo::Snapshot` remains the data source; the feature adds no network calls.
The panel opens the canonical workspace replacement, sending Terminal to the
compositor; sibling workflows close it. It is hidden behind lock, uses existing
keyboard routing and returns to terminal input on Close/Escape. Shared buttons,
TextEdit, selectable labels, ScrollArea, fonts and theme tokens remain authoritative.
Search stays above scrolling content, wraps controls on narrow workspaces, and
uses two body columns only when space permits. Data is labelled approximate or
manual; disappeared destinations retain the pin with a no-longer-current status.
Automatic motion requires both existing animation and globe rotation settings.

## Verification

- Red: new camera tests failed to compile before Explorer/project_view existed,
  `/tmp/desui-globe-red.log`.
- Green: all eight globe tests pass, `/tmp/desui-globe-gesture-tests.log`.
  Coverage includes actual egui pointer press/release selection, drag rotation,
  hovered wheel zoom vs wheel outside the canvas, initial search focus and typing,
  motion pause/reduced motion, camera bounds/Home, selection across disconnection,
  original geographic/projection/arc invariants, empty/no-match/lookup-off and
  long-name rendering at 240px and 800px with a width-overflow assertion.
- Full Rust workspace tests: 515 passed, seven failed, zero ignored,
  `/tmp/desui-globe-workspace-tests.log`. Six existing Unix IPC/control/greeter
  tests fail on socket permissions; the live tmux test also cannot start its
  socket server in this sandbox. These remain failed checks, not skips.
- Full workspace offline build passes without warnings, `/tmp/desui-globe-build.log`.
- Strict frontend static audit: zero findings, `/tmp/desui-globe-ui-audit.json`.
- `git diff --check` passes. rustfmt is not installed locally.

Native desktop visual inspection, physical trackpad/touch, compositor focus
transitions and lock/unlock still require the checklist in `docs/UNTESTED.md`.
Headless egui coverage and static audit do not certify these real desktop flows.
No installation, restart, user configuration mutation, commit or push performed.
Earlier live-terminal recovery fixes in the working tree were preserved.
