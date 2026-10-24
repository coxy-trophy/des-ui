# Cursor companion validation —2026-10-23

Approved design: `docs/superpowers/specs/2026-10-23-cursor-companion-design.md`.
Implementation plan: `docs/superpowers/plans/2026-10-23-cursor-companion.md`.

## Automated evidence

- `python3 -m unittest discover -s companion`: **40 passed** (22 service,18 voice); `/tmp/desui-companion-python-tests.log`.
- `cargo test --offline -p des-ui companion:: -- --test-threads=2`: **6 passed**, after final native status wording; `/tmp/desui-companion-native-tests.log`.
- `cargo test --offline --workspace --no-fail-fast -- --test-threads=2`: **710 unit tests plus 1 doctest passed, 9 failed** because sandbox denies the existing socket/tmux operations; `/tmp/desui-companion-workspace-tests.log`. No companion failures. This is not a clean full-suite pass.
- Existing terminal `stalled_ui_backpressures_noisy_shell` passed in this run; earlier timeout is timing-sensitive rather than established fixed by this feature.
- `python3 -m unittest discover -s session/tests`: **15 passed** after isolated rerun; `/tmp/desui-companion-session-tests.log`. Packaging fixture now stages and checks companion helpers/dependencies. Initial missing-assets fixture failure was repaired. Startup tests with 150ms deadlines failed under concurrent build load and passed after compilation; production startup policy was unchanged.
- `python3 -m unittest discover -s packaging/tests`: **6 passed**; `/tmp/desui-companion-packaging-tests.log`.
- Python compilation, shell syntax for changed install/package/verify scripts, and `git diff --check` passed.

The nine blocked Rust tests are compositor IPC identity/state socket tests; shell control client/timeout, desktop reconnect and greeter service tests; and three private-tmux terminal tests. Their errors are PermissionDenied or private backend startup denial. Existing restrictions were not bypassed and tests were not marked skipped.

## Behavior covered

Private persistence and history bounds; symlink rejection; role/schema separation; key redaction/nonpersistence; provider/refusal/incomplete handling; cancellation/stale replies; identity rollback; memory/clear separation; social suppression, throttling and unanswered persistence; daily caps/reservations/cumulative final usage including closure beyond the local deadline; exact caption spacing/turn boundaries; shutdown despite storage failure. Voice tests cover ack-before-capture, documented framing/configuration, kept hard deadline, follow-up timing, mute/stop, blocked callbacks/sends, bounded audio queues, stale sessions, conservative missing-usage finalization, final-transcript FIFO and parent-death audio cleanup. Native tests cover Escape isolation, narrow/wide bounds, no-focus social pointer interaction, direct voice command ordering and saturated queue teardown.

An independent reviewer verified the repaired backend/native findings. Social bubble routing was checked against compositor shell mapping rather than adding a focus-stealing overlay.

## Acceptance still open

No actual API request, key/account access check, microphone, live playback, native compositor session, installation or restart was performed. Mocks establish lifecycle/protocol handling, not provider acceptance or voice quality. See the cursor AI companion checklist in `docs/UNTESTED.md`. Spoken Keep command, local wake detector, explicit attachments, token streaming and per-companion audio device selectors remain deferred. No arbitrary desktop tools or ambient context collection exist.

Final `cargo build --offline --locked --workspace` passed without warnings after the last UI changes (`/tmp/desui-companion-build.log`, 1m09s). No dependencies were downloaded. The workspace was left uncommitted and the running desktop was not replaced.
