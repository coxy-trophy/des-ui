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
