# Stale shell patch after app switching — 2026-10-19

The user clarified that the screenshot artifact persists after switching from the
terminal to an application. The initial notification-layout explanation did not
establish the cause of this transition bug; no notification layout change was
made for this fix.

## Confirmed failure path

The compositor intentionally copies nominated rectangles of the shell surface
above application windows for toasts and overlays. App::update cached a clear
as sent before Desktop::send accepted it. The shared bounded action channel
could reject it; the unchanged cached rectangle then prevented a retry. The
transport also discards ordinary actions while reconnecting. A stale nominated
rectangle can therefore keep exposing whatever shell UI occupies that position
after an app becomes foreground. This explains a persistent patch; it is a
confirmed delivery bug rather than a reproduced native screenshot in this sandbox.

Desktop now retains latest presentation intent separately from window actions.
Toast and Overlay updates coalesce, explicit None clears survive queue saturation
and disconnection, failed wire-buffer enqueue stays pending, and Link::reset
invalidates queued geometry so ready reconnects resend the latest state. Geometry
waits for stable unlocked acknowledgement. Existing security intent handling and
non-replay of stale window actions remain unchanged.

## Verification

- Red: `overlay_clears_are_retained_when_the_action_queue_is_full` failed against
  the original send path (`/tmp/desui-overlay-clear-red.log`).
- Focused desktop tests: eight pass, one Unix-socket integration failure in the
  sandbox (`/tmp/desui-overlay-clear-green.log`).
- Added coverage for latest-state coalescing, offline clears, a full wire buffer,
  retry after draining, resend after reset/handshake, duplicate suppression, and
  waiting for unlock acknowledgement. No external service or real session is
  modified by these tests.
- Final workspace tests: 522 passed, nine environment-blocked failures, zero ignored (`/tmp/desui-switch-overlay-tests.log`). The failures are six Unix listener IPC/control/greeter tests and three live tmux tests. All new retry/coalescing/unlock regressions pass.
- Workspace offline build passes without warnings (`/tmp/desui-switch-overlay-build.log`).
- `git diff --check` passes. Physical desktop app-switch verification remains
  required using the checklist in docs/UNTESTED.md.

Installed executables and the running desktop are unchanged. Previous launcher,
globe and terminal changes in the working tree were preserved. No commit or push.
