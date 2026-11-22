# Phone as a key

A paired phone can lock the session when it walks away, answer a lock-screen
unlock request, and approve an administrator action. Each is a separate switch
per paired device and each is **off by default**. The password always works and
is never seen by this code. The first sign-in after boot always needs it.

The full design, wire protocol and threat model are in
`desos/phonekey/README.md` (the DesOS recipe tree). This file covers what is in
this repository. Nothing here has been run against a real phone.

## What is implemented

| Part | Where |
|---|---|
| Signed message, key and signature checks, single-use requests, limits, lockout, audit, walk-away rule, unlock policy | `src/phonekey.rs` |
| Capability `des.phonekey.v1`, packets, presence heartbeat, walk-away lock request, IPC | `src/service.rs` |
| Per-device settings and the enrolled key | `src/identity.rs` (`PeerRecord.walkaway`, `walkaway_seconds`, `phonekey`) |
| Commands | `src/bin/des-connect-ctl.rs` (`phonekey-*`) |
| Tests | `src/phonekey.rs`, `tests/phonekey_session.rs`, `tests/phonekey_vectors.rs` |

## Settings

Stored per paired device in the daemon's owner-only `peers.json`; changed only
through the daemon and only while the session is unlocked. All default to off.

```
des-connect-ctl phonekey-status
des-connect-ctl phonekey-enrol DEVICE_ID                   # the phone creates and proves its signing key
des-connect-ctl phonekey-set DEVICE_ID walkaway on [SECONDS]   # 30-3600, default 120
des-connect-ctl phonekey-set DEVICE_ID unlock on
des-connect-ctl phonekey-set DEVICE_ID approve on
des-connect-ctl phonekey-revoke DEVICE_ID
des-connect-ctl phonekey-presence                          # exit 0 only if the walk-away phone is present
des-connect-ctl phonekey-approve ACTION TEXT...            # exit 0 only on a verified signature
des-connect-ctl phonekey-unlock-check                      # prints the decision; unlocks nothing
```

Unpairing deletes the key and every switch at once. A re-enrolled key starts
with `unlock` and `approve` off. The audit trail is
`~/.local/state/des-ui/connect/phonekey-audit.log`.

Administrator approval additionally needs the DesOS `phonekey` component and
`sudo desos-phonekey enable --user NAME`; without it `approve` only serves
`des-connect-ctl phonekey-approve`.

## Walk-away lock

The daemon asks the walk-away phone "are you there?" every 10 seconds over the
paired TLS session. When the phone was seen during the current unlocked period
and then has not answered for the grace period, the daemon sends `lock` to the
shell's control socket: the request `des-ui-ctl lock` makes. That is the only
thing presence can cause. The daemon never sends the shell anything but `state`
and `lock`, and the shell has no unlock verb.

## The lock-screen hook that is not wired

The unlock request, the phone's answer and its verification are implemented and
tested, but **the lock screen does not act on them**. `Lock::show` in
`shell/src/lock.rs` still releases only on a correct password. A second release
path changes the lock screen's guarantee, and its drawing code could not be run
or seen while this was written, so it was left for a reviewed change:

1. `Lock` owns a `phonekey::UnlockPolicy`. Call `password_accepted(now)` where
   `show` handles `Ok(true)`, and at start-up only when the session was entered
   with a password at the greeter (not when started with `--locked`).
2. Add an explicit "Use phone" control. Do not ask the phone automatically.
3. On that control, if `policy.allowed(now)` is `Ok`, on a worker thread:
   make `Challenge::new(Purpose::Unlock, user, "Unlock this DES UI session", wall, 30)`
   **in the shell**; send it with `service::request_within` as
   `phonekey_request` carrying the challenge's own `request_id`, `nonce`,
   `issued_at`, `expires_at`; then check the returned `signature` with
   `phonekey::verify_answer` against a `Binding` and `PublicKey` the shell read
   **itself** from `identity.json` / `peers.json` (this needs a read-only loader;
   `Store::open` creates an identity when none exists). Ignore the key and
   certificates in the daemon's reply.
4. On `Ok(())` take exactly the path `Ok(true)` from the password check takes.
   On anything else call `policy.phone_failed()` and say the password is needed.

Done that way the lock releases on "the password, or a signature by the enrolled
key over a challenge the shell made", and `des-connectd`, which listens on the
network, stays outside the lock screen's trusted base.
