# Handoff B: command blocks

Everything in the terminal works on its own: marks, gutter, Ctrl+Shift+Up/Down,
gutter click, hover status, shell integration for bash, zsh and fish. Two things
are for the parent to wire; nothing else is needed.

## To wire

- `TermTab::copy_last_output(&mut self) -> Option<String>` — the output of the
  last command that finished (bounded to 10,000 rows / 1 MiB). Suggested: a
  palette action "Copy last command output" next to "Find in terminal"
  (`workflows.rs`, dispatch on the focused tab, `ctx.copy_text(..)`), and a
  toolbar icon in `main.rs::top_bar` beside the find button. Until wired it is an
  unused-method warning.
- `TermTab::jump_prompt(&mut self, previous: bool)` is public too, should the
  palette want "Previous/Next prompt" actions; the keys already work.

## Behaviour to know

- The grid is 10 px narrower: `term::GUTTER` is carved off the left of the pane
  for the marks. Nothing outside term.rs needs to change for that.
- bash is no longer started with `-l` when integration is on: it gets
  `--rcfile` with a file that reads `/etc/profile` and the first of
  `~/.bash_profile`, `~/.bash_login`, `~/.profile`, as a login shell would, then
  the integration. `shopt login_shell` is therefore off in the terminal's bash.
  zsh and fish keep `-l`.
- Files go to `$XDG_RUNTIME_DIR/des-ui/shell-integration/` (0700), written only
  when they change. `DES_UI_SHELL_INTEGRATION=0` in the desktop's environment
  turns the whole thing off; set in a shell, it turns the script off.
- A user's own DEBUG trap in bash is left alone; prompts are then marked but not
  command starts or exit statuses. Another OSC 133 integration already present
  (`133;` in PS1/PROMPT_COMMAND) makes ours step aside.

## For docs/UNTESTED.md

- Prompt marks with a real bash, zsh and fish session, including a user whose
  `.zshenv` sets `ZDOTDIR`, and one with starship or powerlevel10k.
- Marks staying in place across a long `yes | head -100000` once the scrollback
  is full, and after a window resize.
- Ctrl+Shift+Up/Down inside `less` or `vim` (alternate screen: no prompts, no
  movement).
