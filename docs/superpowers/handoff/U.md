# Handoff U: polkit authentication agent

Done: `shell/src/polkit.rs` and `mod polkit;` in main.rs. It serves
`org.freedesktop.PolicyKit1.AuthenticationAgent` at `/org/desui/PolicyKit1/AuthenticationAgent`
on the system bus and registers it with polkit for this session (`unix-session` with
`XDG_SESSION_ID`, else `unix-process` with pid and start-time). The cookie and the password reach
`polkit-agent-helper-1` on stdin only; the password's bytes are overwritten once written. Three
refusals fail the request (`org.freedesktop.PolicyKit1.Error.Failed`), Cancel or polkit's
`CancelAuthentication` answer `...Error.Cancelled`, and a request with no answer for two minutes
fails. shell/Cargo.toml gained `async-channel = "2"` (already in the lockfile through zbus) so the
D-Bus method can await the dialog without blocking zbus's thread.

## For the parent to wire

1. `App` gets `polkit: polkit::Polkit`; in `App::new`: `let polkit = polkit::Polkit::start(ctx.clone());`
   before `ctx` is moved into the struct. Without a system bus or polkit it logs once
   (`des-ui: no polkit agent: …`) and stays inert.
2. In `runtime.rs` `update`:
   - inside the `if self.lock.locked { … }` branch, before its `return`:
     `self.polkit.show(ctx, &theme, true);` — this only pumps: a dialog in progress is put
     back to wait and nothing is drawn over the lock screen;
   - after the lock block, at the end of `update` (so it is drawn above everything else):
     `self.polkit.show(ctx, &self.theme.clone(), false);`.
3. Keyboard: while `self.polkit.active()`, the dialog's password field takes focus on its own
   (it requests focus when the helper prompts). Keep keys from the terminal and from shell
   shortcuts while it is up: early in `take_shortcuts`, `if self.polkit.active() { return Vec::new(); }`,
   and pass `focused = false` to the panes' `handle_events` / `show` for that frame (the same way
   the scratchpad keeps keys from the panes). Escape cancels the dialog, Enter answers.
4. Session: `session/des-ui-session` starts no polkit agent of its own (no `polkit-gnome`,
   `lxpolkit` or similar), so this one is the only agent in the session; nothing to remove.
   polkit allows one agent per session: if a user runs another, whichever registered first wins
   and the other logs the refusal.

## For docs/UNTESTED.md

## Polkit authentication

- [ ] `pkexec true` in the terminal: the dialog shows the action and your name; the right
      password succeeds, and `pkexec` exits 0
- [ ] GNOME Software (or `pkcon install`): installing a package asks here and proceeds
- [ ] A wrong password three times: each refusal says so, the third fails the request
- [ ] CANCEL and Escape: the program reports the request was dismissed
- [ ] A request made while the screen is locked (e.g. `sleep 5; pkexec true` then lock): nothing
      shows over the lock screen; the dialog appears after unlocking
- [ ] A user who is not an administrator: the dialog offers the administrators to choose from
- [ ] A fingerprint or security-key PAM stack: the helper's info lines show in the dialog
