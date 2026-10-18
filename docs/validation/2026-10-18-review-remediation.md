# Review remediation, 18 October 2026

## Changes

- Protect critical PIDs before process ownership checks or signals; replace the unsafe real-PID-1 test with pure guard checks and controlled-child termination.
- Preserve the cached-peer-credential startup deadlock correction. Deny privileged input-method registration until a trusted launch mechanism exists and release keyboard grabs on lock.
- Add a Python 3 startup watchdog with private per-attempt readiness, a 15-second startup deadline, process-group cleanup and fallback. Shell acknowledgement requires initialized normal/lock UI and compositor state handshake. Package/install/uninstall the helper; verification runs its tests.
- Propagate portal Close/disconnection/expiry through request-scoped cancellation; tag compositor shots so late results and local hotkey captures cannot be mistaken for the cancelled request. Bound portal answer lines.
- Refuse unsupported SSH options explicitly instead of probing a different connection; show SSH OPTIONS UNSUPPORTED. Apply one deadline to child execution and bounded nonblocking output collection; clean up its process group.
- Add Settings → Sound & Screen → Notifications → Processor temperature warnings. Defaults on. `processor_temperature_warnings = off` survives restart, removes pending CPU alarms, preserves sensor readings and other alert kinds. Existing notification history remains. Re-enabling reports a still-hot CPU on its next sample.
- Route terminal/scratchpad links through a bounded asynchronous native browser opener and show launcher failures in the existing notification center.

## Verification

`cargo test --workspace --locked --offline --no-fail-fast -j 2`:

- common: 33 pass
- compositor: 90 pass, 2 fail
- music: 19 pass
- portal: 7 pass
- shell: 341 pass, 4 fail
- **490 pass, six failures, no excluded or ignored tests.**

The six failures are Unix-listener bind `PermissionDenied / Operation not permitted` in this sandbox: compositor IPC's `refuses_socket_clients_without_the_shell_identity` and `serves_state_and_receives_commands`; shell control's `a_client_is_answered_after_the_interface_acts` and `an_unanswered_request_is_told_so`; shell desktop's `transport_reconnects_and_retries_unacknowledged_lock`; greeter's `signs_in_through_the_login_service`. They are not counted as passing.

`cargo build --workspace --locked --offline -j 2`: succeeds. Existing four dead-code warnings remain.

`python3 -m unittest discover -s session/tests -v`: ten pass, including startup timeout, initialized session surviving past the deadline, clean/error exits, private exact readiness, symlink rejection, descendant cleanup and actual launcher fallback with stubbed services.

Two standalone Rust readiness tests pass. New configuration/CPU suppression, literal browser argument/failure, inherited-pipe deadlines, cancellation correlation, and critical-PID regressions pass in the workspace run. Fourteen shell scripts pass `sh -n`; `git diff --check` passes. Native UI strict static audit reports no findings. Independent final code review found no concrete regressions.

Logs: `/tmp/desui-remediation-tests.log`, `/tmp/desui-remediation-build.log`, `/tmp/desui-remediation-startup-tests.log`, `/tmp/desui-remediation-ui-audit.json`.

## Limits and decisions

No installation, service restart, commit, publication, or system preference change was performed. Existing uncommitted startup correction and recovery installer were preserved. Physical DRM login/recovery, IME compatibility, native D-Bus integration and actual default-browser behavior still require a real desktop session.

Privileged external Wayland IMEs are disabled until trusted launch/identity exists; text-input and the shell keyboard remain. Legacy previous shells lacking readiness support are skipped, with cage retained as final fallback. Unsupported explicit SSH transport options are refused; ordinary SSH configuration still applies. Process-group cleanup cannot reach descendants that intentionally create separate sessions. Browser launchers retain one of eight slots until they exit; they never block rendering.

A temporary artifact-cache copy filled `/tmp` during build isolation; the copy was removed and free space restored before verification. No project source or user files were removed.
