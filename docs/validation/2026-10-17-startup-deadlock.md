# Login startup deadlock, 17 October 2026

The installed session log at `~/.local/state/des-ui/session.log` stops at
13:15:39 UTC after starting `/usr/local/bin/des-ui` and presenting the first
frame on eDP-1. No shell window connects. The offline system journal records a
successful greetd login followed by a power-key shutdown, without a compositor
crash. The installed `/usr/local/bin/des-comp` matches the original working
copy's `target/release/des-comp` (SHA-256
`477fa067720cb7d10045c49a0cbbbcbbfbcac793c0ab3936b6f727f4b1dc1ff2`).

The virtual keyboard global's visibility filter calls `Client::get_credentials`
through the display handle. Wayland registry advertisement invokes this filter
while holding the backend mutex. Credential lookup attempts to acquire that same
mutex, deadlocking the event loop before a client can finish initialization. This
explains a black first frame with an immobile pointer. Session fallback requires
the compositor to exit, so it cannot recover a blocked event loop.

The fix obtains kernel peer credentials from the accepted Unix stream before
inserting the client into the display, and stores its PID in `ClientState`.
The visibility filter reads only this cached data. It still permits only the
current shell PID, rejects missing credentials, and rejects Xwayland clients,
which do not have this client data.

## Validation

- An isolated reproduction of the original credential lookup inside
  `backend_handle().with_all_clients` blocks until a three-second process timeout
  (exit 124).
- `virtual_keyboard_filter_works_while_backend_is_locked` passes. It executes the
  fixed filter with the backend mutex held and checks current-shell, old-shell,
  absent-shell and missing-credential cases.
- `cargo test -p des-comp --locked --offline`: **90 passed, 2 failed**.
  `ipc::tests::refuses_socket_clients_without_the_shell_identity` and
  `ipc::tests::serves_state_and_receives_commands` fail on Unix listener binding
  with `Operation not permitted` in this sandbox. They are not passing tests.
- `git diff --check` and the recovery installer's shell syntax check pass.

The live USB has older glibc and no native Rust toolchain. Tests use the mounted
installation's Rust 1.95.0 and libraries through temporary wrappers. The optimized
recovery compositor is compiled from the changed source against the existing
release dependency artifacts identified by the original Cargo fingerprints.
Its compiler arguments are saved alongside it in `target/recovery`.

`scripts/repair-mounted-session.sh` installs the prepared binary into the mounted
offline root's `/usr/local/bin`, keeps a timestamped backup, and restores a
read-only mount after installation if it was read-only initially. It requires a
manual sudo invocation from the live terminal because the mounted root is outside
the agent's writable workspace. The older packaged login-screen compositor in
`/usr/bin` is not replaced.

Successful login and physical input still require verification after installing
the replacement and rebooting; the sandbox cannot run a graphical session.
