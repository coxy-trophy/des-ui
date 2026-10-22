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


## Follow-up: first-start recovery errors

A desktop screenshot showed `no current target` during enumeration and `invalid
pane dimensions` while preparing a fresh attachment. Source investigation found:

- `list-panes -a` still resolves a current window before executing, so it fails
  on the intentionally empty foreground server. Use target-free `list-sessions`
  and each session's active-pane format defaults instead.
- A pane target written `=session-name` does not mark the session part exact: it
  is parsed as a pane/window target. The dimension/history queries now use
  `=session-name:`; the attach-session command retains its session target.
  `display-message` allows target lookup failure, which explains the empty fields
  and misleading dimension parse error.

Confirmed against upstream tmux 3.7c [target lookup](https://github.com/tmux/tmux/blob/3.7c/cmd-find.c)
and [session enumeration](https://github.com/tmux/tmux/blob/3.7c/cmd-list-sessions.c).
The real backend regression now exercises the GUI's own attachment preparation,
checks that its live viewport appears, verifies stable PID across two detaches,
and enumerates both before creation and after the final terminal exits. Earlier
coverage attached directly, bypassing the failing dimension/history queries.

Run the integration check outside the restricted sandbox:

```sh
cargo test -p des-ui --locked private_session_survives_client_detach_and_literal_arguments -- --nocapture
```

No installed binaries, running terminals, or desktop services were changed.

Follow-up verification: workspace offline build passed without warnings
(`/tmp/desui-recovery-fix-build.log`). Focused persistent-terminal tests: nine
passed, one real-backend test failed at server startup under socket restrictions,
zero ignored (`/tmp/desui-recovery-fix-tests.log`). `git diff --check` passed.
The corrected reconnect path still requires the unrestricted integration run above.


## Follow-up: new-tab failure at 10:37

The screenshot reports `Terminal not opened: invalid pane dimensions`. The installed
`/usr/local/bin/des-ui` was last written at 10:09 and contains the old list-panes
query; the working debug binary was rebuilt later. The running process is not
visible in this tool PID namespace, so this alone does not identify which binary
produced the screenshot. Do not equate a successful build with installation.

Additional confirmed failure mode: upstream tmux's non-UTF8
[client printer](https://github.com/tmux/tmux/blob/3.7c/server-client.c) calls
[utf8_sanitize](https://github.com/tmux/tmux/blob/3.7c/utf8.c), which maps tab
metadata separators to underscores. A regression using the actual format strings
and that sanitization rule reproduced `invalid pane dimensions` before the fix
(`/tmp/desui-terminal-locale-red.log`). The source now uses printable pipe separators
for session/PID and dimensions, plus explicit UTF-8 clients for captured display
bytes. The regression passes; focused backend tests report ten passes and one
real tmux server-start failure under the sandbox socket restriction
(`/tmp/desui-terminal-locale-green.log`).

New Tab now falls back to a fresh direct interactive shell on managed creation
failure and explicitly reports that the tab is temporary. Other managed sessions
are left alone; direct command launches and reconnects never use this fallback.
The immediate workaround in an older running build is Settings → Session → Keep
terminals across shell restarts → off; new tabs then use the direct PTY path.
No installed binary, user setting, or running terminal was changed during this fix.

The user confirmed the failing window is the installed desktop session. Final
workspace build passes without warnings (`/tmp/desui-terminal-fallback-build.log`),
and the existing direct PTY output regression passes
(`/tmp/desui-terminal-direct-fallback-tests.log`). `git diff --check` passes. The
new-tab fallback still needs native desktop acceptance; no automated GUI fallback
claim is made.
