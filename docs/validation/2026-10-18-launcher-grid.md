# Application launcher grid validation — 2026-10-18

User request: replace the application launcher's list with a grid and reduce text.
The default view now shows application icons and centered two-line names only.
Details and paths remain in hover descriptions and accessibility labels. Typed
queries preserve the existing universal providers and action handlers; non-app
results retain compact kind labels. No applications are launched by the tests.

The grid reuses app_icons::IconCache (48px installed artwork or native fallback),
shared ui fonts/palette/icon buttons and virtual ScrollArea rows. Columns adapt
to width, with a single column in narrow workspaces. Left/Right move across items
and Up/Down move by grid row, with keyboard scroll positioning. Query editing
keeps caret arrows until grid navigation starts; clicking/typing in search returns
to editing. IME guards, calculator copy, app focus and existing context menus stay
in place. Search progress has reserved height; search folders/close/clear are
labelled icon controls instead of extra header words.

## Evidence

- Red: added keyboard grid navigation test failed against the list implementation
  (`/tmp/desui-launcher-grid-red.log`).
- Focused launcher checks passed after implementation, including 2D arrow
  movement/Enter, ordinary search caret edits, IME execution guard, calculator
  copy and accessible result-kind labels (`/tmp/desui-launcher-grid-interactions.log`).
- Final full workspace tests: **519 passed, seven failed, zero ignored**
  (`/tmp/desui-launcher-grid-workspace-tests.log`). All six launcher tests pass,
  including actual egui pointer click dispatch, app-only blank search, hidden
  executable details, and tile bounds at 240px/900px with long names/missing icons.
  The seven failures are the existing six Unix listener IPC/control/greeter tests
  plus the real tmux startup/reconnect test blocked by this environment's socket
  restrictions. No checks were skipped or reported as fully passing.
- Full workspace offline build passes without warnings (`/tmp/desui-launcher-grid-build.log`).
- Strict frontend static audit: zero findings (`/tmp/desui-launcher-grid-ui-audit.json`).
- `git diff --check` passes. Local rustfmt is unavailable.

DESIGN.md and UX-CONTRACT.md document the changed presentation and navigation.
Native desktop icon/theme appearance, compositor app launching, right-click menus,
physical scrolling and screenshots still need the launcher checklist in
docs/UNTESTED.md; headless egui checks do not replace this acceptance.
No installed files, running session, configuration, commit or remote was changed.
Earlier globe and terminal fixes in the working tree remain intact.
