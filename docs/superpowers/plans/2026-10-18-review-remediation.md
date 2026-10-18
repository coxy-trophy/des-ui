# Review remediation and processor warning preference

Goal: fix the six findings from the 18 October review and add a persistent opt-out for processor heat notifications, preserving other alerts.

Architecture: preserve the current native Rust/egui desktop and existing uncommitted credential-cache fix. Reuse typed IPC, config persistence and notification center. Fail closed on privileged input-method access; do not copy arbitrary SSH options that could execute local commands. No installation, publication, commits or service changes.

Execution: subagent-driven development with disjoint file owners, parent integration and final review. Existing dirty state remains intact; work in the shared checkout because its uncommitted deadlock correction is a prerequisite, and no branch integration is authorized.

- [x] Process/remote worker: protect PID 1 and current process before signalling; test using controlled children rather than real system PIDs. Preserve safe SSH transport settings or explicitly refuse unsupported connections. Bound process and pipe collection together, kill/reap process groups on timeout. Own commands.rs, remote.rs.
- [x] Portal worker: propagate Close promptly to shell with scoped cancellation; bound answer reads. Own portal/, common/src/control.rs, shell/src/control.rs, portal handling portions of runtime.rs and capture.rs. Coordinate parent on runtime.rs before edits.
- [x] Compositor/session worker: preserve cached-credential startup correction; restrict IME registration to explicit trusted process identity, revoke grabs across lock; add a bounded startup readiness watchdog with fallback. Own compositor/ and session/des-ui-session plus new watchdog helper/tests/docs as necessary. Avoid blocking or reentrant credential queries.
- [x] Parent: persistent processor_temperature_warnings preference in Settings → Notifications; suppress only CPU heat alerts at evaluation time. Add browser-open error feedback using bounded async operations. Own config.rs, settings.rs, sysmon.rs, term.rs, apps.rs, main.rs, workflows.rs, UI contracts and validation docs.
- [x] Regression tests first for each behavior; focused tests, full workspace tests and build, shell syntax, diff check, native UI static audit. Never rerun the unsafe original terminate(1) test before replacement.
- [x] Independent final code review and repairs. Record sandbox/hardware limitations explicitly.

Rulings: CPU alerts remain enabled by default for backward compatibility. Disabling does not disable sensor readings, memory/disk/service alerts, or remove historical notifications. Source fixes are authorized; installing the repaired session is outside this change.

Completion: all planned source changes and independent review completed. 490 workspace tests pass, six socket-bind tests sandbox-fail (none excluded), Python watchdog ten pass, readiness two pass, strict UI audit and syntax/diff checks pass. Workspace build succeeds (four existing dead-code warnings); result recorded in validation doc.
