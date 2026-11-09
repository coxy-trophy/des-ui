The login launcher requires Python 3 and installs `des-ui-startup` next to
`des-ui-session`. Each native compositor attempt runs in a separate process group
with a fresh private readiness directory. The shell acknowledges only after its
UI initialized and it received compositor state; binding sockets and drawing the
compositor splash do not count. An unready attempt has 15 seconds, then the helper
terminates its process group (including descendants), escalating to SIGKILL.
An acknowledged session has no runtime deadline. Clean exit remains logout.

The previous build is tried only if its shell binary contains the readiness
protocol environment name `DES_UI_SESSION_READY`. Older builds are skipped: they
cannot distinguish a healthy desktop from a startup hang. Cage remains the final
fallback when installed. Cage fallback is deliberately not timed: a nested
compositor failure must not repeatedly restart the login session.

The privileged input-method-v2 global is currently denied to all clients. This
also prevents IME keyboard grabs and surrounding-text access, including across
lock. Text-input remains available; external IME candidate/composition integration
is unavailable until a compositor-owned trusted launch/identity mechanism exists.
The shell virtual keyboard remains restricted to the current shell's cached PID.
Keyboard grabs are explicitly released when locking.

Run focused watchdog regressions with:

    python3 -m unittest discover -s session/tests -v

These tests launch only controlled children and never install files or modify
services. Actual DRM/display-manager recovery still needs hardware verification.

## Recovering a session that will not unlock

Normal unlock requires the account password. `des-ui --unlock` is deliberately
rejected and is not a recovery command. Ctrl+Alt+Backspace ends an unlocked
desktop, but the current input filter does not handle it while locked.

For recovery, press Ctrl+Alt+F3 (or another available text console), sign in with
your own account, and run `loginctl list-sessions`. Identify your affected
graphical session by its seat, TTY and type; `loginctl show-session SESSION_ID
-p Name -p Type -p TTY -p Seat` shows those details. If ending that session is
necessary, run `loginctl terminate-session SESSION_ID` with its actual ID.
This ends the session and its applications; unsaved work may be lost. Do not
terminate unrelated sessions or use this to test recovery while work is active.

VT switching, display-manager return and the next login must still be verified
on the exact installed candidate. These instructions do not bypass authentication
or certify that hardware recovery has already passed.
