# Terminal wheel routing — 2026-10-19

User reports wheel failure inside programs such as less/Vim in an installed
desktop. The persistence server had mouse reporting off. Enabling it alone would
still leave alternate-screen applications without mouse support unhandled:
tmux's default wheel binding sends mouse events to alternate screens, while
[its input encoder drops them when no mouse mode is enabled](https://github.com/tmux/tmux/blob/3.7c/input-keys.c).

Configure the private server's mouse flag and wheel bindings at creation and
reconnect: forward real mouse to programs that requested it; send Up/Down to
alternate-screen programs otherwise; enter backend scrollback at the prompt.
The existing bounded dimension query also applies the reconnect setup, so no
extra UI-blocking helper process is added. The private copy table retains wheel
and Escape; ordinary keys cancel copy mode and forward the original event via
[send-keys with no arguments](https://github.com/tmux/tmux/blob/3.7c/cmd-send-keys.c).
This config affects only the compositor's private backend. Existing desktop
work, direct PTY input, UI selection and other user tmux servers are untouched.

## Verification

- `cargo test -p des-ui --locked --offline -j 2 term::`: 53 pass, three fail,
  zero ignored (`/tmp/desui-terminal-scroll-final-tests.log`).
- Existing direct scrollback, fractional wheel, alternate-screen fallback, mouse
  encodings, grid coordinates, unfocused-hover and selection regressions pass.
- New real-backend regressions cover history scrolling with no shell input,
  first-key retention when typing afterwards, and both wheel directions to
  full-screen raw-TTY programs with and without SGR mouse. All three real tmux
  tests, including the previous reconnect regression, fail at server startup
  under the sandbox's Unix-socket restrictions. This is a verification limit,
  not a successful end-to-end reproduction or pass. Tests remain enabled.
- Workspace offline build passes without warnings (`/tmp/desui-terminal-scroll-build.log`).
- `git diff --check` passes. No installation, restart, preference change, live
  backend command, commit or push was performed.

Run the native acceptance checklist in docs/UNTESTED.md after updating the installed
shell. A successful local build alone does not update that installed executable.
