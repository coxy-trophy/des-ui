# Handoff W: XDG autostart

Done on the branch: `shell/src/autostart.rs` (scan with user-over-system precedence,
the rules, the skip list, delays and phases, the once-per-session marker, turning
entries off and on, adding one), Settings → Session → AUTOSTART, two `Change`
variants, and their two arms in `App::apply` (main.rs) — the arms were needed for the
shell to compile, so they are already in; nothing else in main.rs changed beyond
`mod autostart;`.

## For the parent to wire

1. **Start once, in a real session.** In `App::new`, after the `App` value is built
   (or at the end of `main` before `eframe::run_native`), and only when the shell is
   connected to the compositor:

   ```rust
   if desktop_present {
       autostart::start();
   }
   ```

   `desktop_present` is the `Desktop::connect(..).is_some()` flag already computed in
   `App::new`. With `--windowed` nothing starts. `start()` returns at once: it claims
   `$XDG_RUNTIME_DIR/des-ui/autostart-done-<XDG_SESSION_ID or boot id>` and launches
   on its own thread, so Super+Shift+R does not start everything a second time.

2. Nothing else. The Settings list re-reads the directories every 3 seconds and
   straight after a change, on the UI thread; the directories are small.

## Behaviour to know

- `XDG_CURRENT_DESKTOP` is `des-ui` (session/des-ui-session), so `OnlyShowIn=des-ui;`
  and `NotShowIn=des-ui;` are what count.
- Entries with `Terminal=true` are listed as "Needs a terminal" and not started: the
  session start has no terminal tab to give them.
- Other desktops' polkit agents, notification daemons, panels and tray bridges are
  never started (`OTHER_DESKTOPS` in autostart.rs, with the reason); the shell is each
  of those itself, or will be once wave 3's agent and tray land.
- Output goes to `~/.local/state/des-ui/apps.log` through `apps::launch`, like the
  launcher's.

## README paragraph (under Settings → Session)

> **Autostart** — the programs that start with every session: those in
> `~/.config/autostart` and the system's `/etc/xdg/autostart`, following the usual
> rules (`Hidden`, `OnlyShowIn`/`NotShowIn` with `des-ui`, `TryExec`, GNOME's
> enabled switch and delay). Each has a switch; turning a system entry off writes
> your own copy with `Hidden=true`, and turning it back on removes that copy again.
> ADD takes a command line, run as written without a shell, from the next session.
> Other desktops' authentication agents, notification daemons and panels are left
> out, since this desktop is each of those itself. Restarting the shell does not
> start them again.

## For docs/UNTESTED.md

- [ ] A real login starts the entries in `~/.config/autostart` and `/etc/xdg/autostart`
      (e.g. Nextcloud, a GNOME keyring entry marked `OnlyShowIn=GNOME` stays out)
- [ ] Super+Shift+R does not start them a second time; logging out and in does
- [ ] An entry with `X-GNOME-Autostart-Delay=10` starts about ten seconds in
- [ ] Turning a system entry off and on in Settings → Session leaves no file behind
- [ ] ADD with a quoted argument (`foo --name "a b"`) starts exactly that next session
- [ ] polkit-gnome or polkit-kde installed: their agent is not started
