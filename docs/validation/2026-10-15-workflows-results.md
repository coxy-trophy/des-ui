# Desktop workflows: implementation and validation

All five requested features are implemented in the native Rust desktop. Existing
working-tree changes were preserved; nothing was installed, committed or published.

| Feature | Entry point | Implementation |
|---|---|---|
| Universal palette | Super+Space / Apps | Apps, running windows, settings, desktop actions and bounded local filename search; explicit folders; keyboard and IME |
| Saved session | Settings > Session; palette Save/Restore session | Private atomic snapshot of installed app identities, retained workspace/display slots, terminal folders/layout; explicit restore, missing-item feedback, bounded placement/workspace retries |
| Per-monitor dock | Bottom edge; Super+D on pointer display | Separate native viewport state, shared pins, display-local app groups, secondary shell rendering/input, lock/capture suppression |
| Health | Settings > Health; palette health | Compositor/music status, helper availability, read-only bounded redacted logs, refresh and shared native viewer with failure feedback |
| Terminal search | Ctrl+Shift+F; magnifying glass; palette Find in terminal | Literal Unicode lowercase search across configured history, soft wraps/wide cells, navigation/copy/clear, incremental scanning, bounded matches and cross-frame IME |

## Automatic verification

`cargo build --workspace --locked --offline` succeeds.

`cargo test --workspace --locked --offline --no-fail-fast` reports:

| Crate | Passed | Failed |
|---|---:|---:|
| des-common | 25 | 0 |
| des-comp | 21 | 2 |
| des-music | 19 | 0 |
| des-ui | 155 | 2 |
| Total | 220 | 4 |

All 27 additional tests relative to the previous remediation baseline pass.
The same four pre-existing Unix-socket integration tests fail with `Operation not
permitted` at bind in this sandbox:

- `ipc::tests::refuses_socket_clients_without_the_shell_identity`
- `ipc::tests::serves_state_and_receives_commands`
- `desktop::tests::transport_reconnects_and_retries_unacknowledged_lock`
- `greeter::tests::signs_in_through_the_login_service`

No tests were skipped to conceal these failures. The complete suite still exits
unsuccessfully here. Run it on an unrestricted Linux host before release.

Headless egui tests exercise palette query/selection/IME, terminal find field focus,
Unicode entry, PTY key isolation and cross-frame composition. Unit checks cover
file search bounds/cancellation/exclusions, exact native path arguments and viewer
failure, session privacy/permissions/missing apps/arrival matching, retained inactive
workspace placement, display-local dock grouping, IPC framing, lock/capture policy
and stale retained-slot collisions.

The strict premium static UI audit reports zero errors, warnings or findings; see
`2026-10-15-workflows-ui-audit.json`. `git diff --check` passes. DESIGN frontmatter
parses and token ownership remains `ui.rs` / `theme.rs`. The optional external
`designmd lint` could not run because its npm package is absent from the offline
cache (`ENOTCACHED`). `cargo fmt` is not installed in this environment.

## Review

Independent workflow and display reviews identified and verified fixes for:

- Cross-display keyboard dock clear interference.
- Dock capture overriding screenshot selection and stale overlays after capture.
- Live vs retained workspace slot collisions during restoration.
- Unbounded terminal match/cell allocations and cross-frame IME handling.
- Placement retry expiry under a connected but backpressured compositor.
- Final workspace selection being dropped when the IPC queue was full.

## Runtime limits and remaining acceptance

Physical output topology, scaling/rotation/hotplug, a real IME, actual desktop-app
launch/restore and suspend cannot be certified by headless tests. New acceptance
cases are listed in `../UNTESTED.md`; use `HARDWARE.md` to record evidence. Native GUI
inspection could not run in this socket-restricted environment.

Restore does not replay terminal commands, resurrect processes or recover unsaved
documents; it never enables automatic login launches. Session storage is
`$XDG_STATE_HOME/des-ui-sessions/session.json` (fallback
`~/.local/state/des-ui-sessions/session.json`), separate from existing log permissions.
Filename search does not read contents; filesystem I/O on a stalled network mount
can still delay its worker. Terminal find snapshots the configured VT history, so
an unusually large history has a snapshot-copy cost despite incremental scanning.

Try the build on an unrestricted desktop using `./dev.sh`; do not replace the active
login session until the hardware acceptance checks pass for that machine.
