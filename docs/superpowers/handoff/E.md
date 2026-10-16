# Handoff E: palette extensions

Nothing needs wiring in main.rs or runtime.rs beyond the `mod commands;` line already
added: dispatch lives in workflows.rs (`palette_action`, `run_argv`) and background
outcomes are drained by the existing `pump_documents` call each frame.

What the parent may want to do:

- README: a short paragraph under "Command palette" for `ssh` hosts (from
  `~/.ssh/config`), the calculator (`2+2`, `10 km in mi`, `255 in hex`),
  `kill <name>` / `ps <name>`, `start|stop|restart <unit>`, and
  `~/.config/des-ui/commands/*.command` files (`name`, `exec`, `terminal`, `keywords`).
- docs/UNTESTED.md: real `systemctl` start/stop/restart of a user and a system unit
  (the latter is expected to be refused without polkit), SIGTERM of a real process,
  `ssh` host opening in a tab and the tab closing when ssh exits.
- Settings: none. The commands directory is fixed; the worker re-reads it every 30 s.

Boundaries kept: queries never reach a shell; `exec` is split like .desktop lines;
processes are signalled only when `/proc/<pid>/status` still says they are this
user's and not root's; `systemctl` runs with `--no-ask-password` and a deadline.
