# Risk remediation, 15 October 2026

Scope: the current working tree, including pre-existing uncommitted changes. Preserve that work; do not reset, stash, or automatically commit it. The APT binaries are older artifacts and are not rebuilt or published by this change.

## Changes and acceptance criteria

- Send new Wi-Fi credentials to NetworkManager using its D-Bus AddAndActivateConnection method. No password in argv, environment, temporary files or diagnostic output. Preserve nmcli for nonsecret scanning and saved-connection controls. Handle open, WPA and legacy WEP networks; reject invalid input before contacting the service. NetworkManager completes settings from the selected AP. Test payloads and failure handling; real connection testing remains pending.
- Bound music IPC to 16 concurrent clients, 16 KiB per request, 60 seconds per request, 5 seconds per write and 30 seconds per operation. Partial input survives state broadcasts. Reject malformed, oversized and unterminated requests. Release connection slots on every exit. Use an owner-only socket directory and mode 0600 socket, including when XDG_RUNTIME_DIR is absent. Never unlink a regular file or symlink as a stale socket.
- Separate music transport and socket policy from playback. Extract the shell frame update into a focused module without changing behavior. Existing tests remain the regression boundary.
- Keep Unix-socket integration tests strict. Permission-denied failures are environmental failures, never passing checks or silently skipped tests. Add a single local/CI verification entry point and PR website build. Pin the website package manager.
- Collect source identity and local validation results for release review, including untracked source files. Keep real-machine evidence separate from automatic checks.

## Hardware acceptance

Run the full list in ../UNTESTED.md on a disposable test account with recovery access. Record date, source manifest, distribution/kernel, GPU/driver, display topology, steps, expected and actual results, and evidence for each case. Prioritize lock/restart/reconnect, lock during recording, suspend/resume, display rollback, and account operations. Do not run destructive account or boot-manager tests on a primary account.

A clean release requires successful automatic checks and completed hardware evidence for its advertised features. Neither a unit test nor a nested compositor certifies DRM, suspend, or login behavior. Items without evidence remain pending.
