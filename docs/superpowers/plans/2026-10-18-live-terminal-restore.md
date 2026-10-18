# Live terminal restore implementation plan

Goal: preserve running terminal processes across graphical-shell restarts; spec ../specs/2026-10-18-live-terminal-restore.md.

Execution: subagent-driven development, disjoint owners and parent integration. Preserve existing dirty remediation changes; no installs, commits or publication.

- [x] Backend worker: term.rs + term/persistent.rs. Managed spawn/reattach with private tmux server, live ref enumeration, bounded capture/history, actual pane PID/cwd/foreground tracking, direct PTY fallback compatibility. Root GUI drops detach, terminal exit ends server session. Tests first; no unconditional tmux spawn in existing direct PTY unit tests.
- [x] Lifetime worker: common/src/terminal.rs public ENV, validated directory(), Runtime::new/path/Drop; compositor state lifetime passes runtime dir to shell replacements and kills only owned server on teardown. Install/package tmux>=3.3 integration. No mutation to existing startup/capture fixes.
- [x] Integration worker: shell integration assets + integration.rs tests preserve OSC133/7 through managed tmux passthrough, no changes to unrelated user hooks.
- [x] Parent: runtime live layout snapshot module; App reconnects live refs and checkpoints changes; persistent preference and Session UI; new-tab, commands, saved-folder restore and scratchpad integration. Private bounded metadata, missing sessions handled, recovery feedback.
- [x] Parent runs focused/full tests and build, syntax/static UI checks; independent final review and repairs, document real-session limits.

Initial API contracts coordinated between workers in messages; public managed interface finalized before parent integration. Existing direct TermTab::spawn remains available and nonpersistent to preserve current tests.

Review repairs: configure tmux defaults before creating panes; roll back failed fresh sessions; explicitly shut down before hardware fast exit; retain foreground backend with parent-death signal and prevent replacement daemons; preserve layout on failed or partial enumeration/reconnect. Tests remain enabled in CI, including the real tmux detach test. Local Unix-socket restrictions prevent that integration check; see validation record.
