# Window shelf, collapsed clock and drag destinations

Implemented in the workspace; not installed and no running desktop was restarted.

The dock distinguishes pinned, running and active apps, includes All windows, and opens a live chooser for grouped windows. The chooser uses stable window identities and nine previews per page. Older compositors use a native window list. Large dock collections scroll within bounded geometry; keyboard selection remains reachable. The collapsed left clock uses regional formatting.

Confirmed native desktop drags open destination previews. An uninterrupted 450ms hover reveals the actual window and continues the original drag; previews do not accept drops. Escape, source destruction, lock, capture and output changes clear the drag presentation safely. Files supports outbound entry drags; image selection and Trash do not. Settings → Frame → Dock offers a persisted opt-out. Browser tabs require browser support for native desktop dragging.

## Automated verification

- `cargo test --workspace --locked --offline --no-fail-fast -j2`: 607 passed, nine known environment failures. Six socket tests fail with `Operation not permitted`; three private-tmux tests cannot start their server in this sandbox. These failures are not a passing workspace result.
- New protocol, chooser selection/layout, bounded dock scrolling, regional clock, uninterrupted dwell/reset, handoff geometry, preference persistence/error handling and Files drag interaction tests pass.
- `cargo build --workspace --locked --offline -j2`: passed without warnings.
- `git diff --check`: passed.

Logs: `/tmp/desui-dock-tests.log`, `/tmp/desui-dock-focused.log`, `/tmp/desui-dock-build.log`. Final selected source hashes: `/tmp/desui-dock-final-source.sha256`.

## Native acceptance still required

Install matching shell and compositor builds before assessing global behavior. Verify animation and glass appearance, multi-output/DPI and fullscreen transitions, actual offer delivery and copy/move behavior, source/target closure and cancellation, reduced motion/transparency, browser native tab drags and Xwayland separately. Internal browser tab reordering may not expose a desktop drag. The installed-session checklist is in `docs/UNTESTED.md`; automated checks do not establish visual or browser interoperability acceptance.
