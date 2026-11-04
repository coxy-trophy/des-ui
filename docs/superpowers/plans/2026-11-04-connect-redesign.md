# CONNECT redesign implementation plan

**Goal:** Replace verbose CONNECT with the requested compact native device panel and real in-panel USB mirroring.
**Spec:** docs/prototypes/connect-panel/README.md and the user’s implementation request.
**Architecture:** Reuse egui/theme/shared controls and the daemon’s authenticated same-user Unix control socket. A bounded on-demand frame pipeline supplies the native preview, with no network listener or disk recording. Keep the current pairing, permissions, cancellation and lock policy authoritative.
**Constraints:** Preserve unrelated work and all live terminals. No automatic trust, permission grants or mirror restart. Keep capture view-only. Stop and clear frames on lock, peer loss and USB loss. No idle decoder. Use shared control sizes; no browser runtime. Preserve explicit USB serial selection.

## Task 1 — Embedded frame source
- [x] Implement a bounded scrcpy/ffmpeg frame source owned by ScreenBridge, with process teardown and frame invalidation.
- [x] Focused tests: frame bounds/partial data, stop/EOF, guard rejection and frame cleanup.

## Task 2 — Compact native CONNECT
- [x] Selected device header with status/battery; small chooser and settings disclosure.
- [x] Primary real mirror canvas and always reachable Stop; concise file actions and transfers.
- [x] Preserve pairing verification, explicit USB selection, file consent and permission controls in contextual views.
- [x] Background bounded frame reader and one reusable texture; clear stale results on hide/lock/disconnect.
- [x] Native rendering and interaction tests at narrow and regular widths.

## Task 3 — Integrate and verify
- [x] Secure screen_frame Unix IPC with live lock/peer checks and no frame in general state.
- [x] Build shell/daemon, run full affected suites and inspect native renders.
- [x] Verify a real tablet frame in the panel, stop and reconnect behavior; report limits.
- [x] Update DESIGN/UX/bridge docs, commit and push. Install only through normal administrator authentication; preserve running terminals.

## Review focus
Late frame after lock; USB mismatch; decoder EOF leaving a stale frame; Stop while a start request is pending; long names and narrow widths.


Execution notes: native workspace expansion was chosen to reuse existing geometry;
the compact mirror remains directly in CONNECT. Raw scrcpy was chosen after MKV
buffering added four seconds. The tablet itself needs about eight seconds for the
first encoded frame. No changes to the compositor or Android app are needed.

Review fixed: editor input ownership; passive refresh focus loss; frame owner/USB
binding across concurrent session changes; Stop retained after IPC failure;
Escape/terminal-search exit from expanded view; stale frame clearing on EOF.

Validation evidence is recorded in docs/DES-CONNECT.md. Privileged installation
is a separate OS-authenticated step; no graphical session or terminals were
restarted by this update.
