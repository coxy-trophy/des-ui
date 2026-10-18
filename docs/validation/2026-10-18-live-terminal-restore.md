# Live terminal restore validation — 2026-10-18

Implemented preservation of running terminal processes across graphical-shell restarts, scoped to the compositor lifetime. New terminals use a private tmux >=3.3 server; the GUI reconnects tabs, active/split layout and the hidden scratchpad without replaying commands. Existing terminals created before this feature cannot be converted. Both compositor and GUI require the updated build.

## Automated checks

- Full workspace build: `cargo build --workspace --locked --offline -j 2` — passed (four existing dead-code warnings), `/tmp/desui-live-terminals-final-build.log`.

- Full Rust tests: `cargo test --workspace --locked --offline --no-fail-fast -j 2` — **510 passed, 7 failed, 0 ignored**. Log: `/tmp/desui-live-terminals-final-tests.log`.
- Six existing IPC/control/greeter Unix-listener tests fail with `Operation not permitted` under the sandbox. The new real tmux detach/reconnect integration test also fails because the private server cannot bind its Unix socket here. Its server exits with status 1; the earlier explicit attempt reported socket `Operation not permitted`. This test remains enabled, and CI installs tmux. No successful real-backend reconnect is claimed.
- All five common terminal-lifetime tests pass, including parent-death signal across exec and controlled foreground-child reaping. The exec check runs in the child main thread because Linux parent-death signal state is per thread.
- All ten Python startup/watchdog tests pass: `/tmp/desui-live-final-python.log`.
- Strict frontend audit: zero findings, `/tmp/desui-live-final-audit.json`.
- POSIX install/package syntax, Bash/Zsh integration syntax, and `git diff --check` pass. Fish is unavailable locally; its runtime integration checks were not exercised.

## Review repairs

Set tmux defaults before pane creation, correct the history-limit option scope, roll back failed fresh sessions, and preserve existing work on reconnect failures. Use a compositor-owned foreground backend with parent-death SIGKILL, bounded readiness/shutdown, and `-N` clients that cannot start replacement daemons. Explicitly stop the backend before the compositor's hardware fast exit. Keep prior layout metadata when enumeration or recovery is incomplete. A private GUI lease prevents a second GUI from stealing live terminals. Helper output and execution times are bounded; only explicit terminal integration variables enter command arguments.

## Remaining hardware acceptance

Follow the live-terminal checklist in `docs/UNTESTED.md` in a fresh development desktop: verify identical shell PIDs and jobs across restart, multiple tabs/splits, scratchpad, explicit exit, opt-out, second-GUI lease, failure retry and desktop logout cleanup. No install, logout, shell restart or user preference change was performed during this work. Logout/reboot are outside live preservation scope.
