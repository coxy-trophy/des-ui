# Handoff O: clipboard history

Done on the branch: `shell/src/clipboard.rs` (in-memory history of 50 text entries,
64 KiB each, pins, password-manager hint skipped, `wl-paste --watch` with a
2-second fallback poll only while the palette is open), the `CLIP` kind in
`palette.rs` (`clip`, `clipboard`, `clip <text>`), its rows and Pin/Remove/Clear
menu in `launcher.rs`, and their dispatch in `workflows.rs`. The `Copied` notice now
shows only short single-line text, so clipboard entries are not echoed into the
notification history.

## To wire in the shell

The palette holds a `clipboard::Handle` instead of the brief's `Catalog`, because
the providers worker replaces the `Catalog` wholesale on each query and the history
has to outlive that.

1. `App` gets a field `clipboard: clipboard::Clipboard`. In `App::new`:
   ```rust
   let clipboard = clipboard::Clipboard::start(ctx.clone());
   launcher.set_clipboard(clipboard.handle());   // or after Launcher::default()
   ```
   (`set_clipboard` is the one unused-method warning until this is done.)
2. Each frame in `runtime.rs`, near `self.pump_documents()`:
   ```rust
   self.clipboard.poll(self.launcher.open);
   ```
   With the watcher running this does nothing; without it, it looks every two
   seconds only while the palette is open.
3. Locking: nothing. The history is never shown on the lock screen and lives only
   as long as the shell process.

## README paragraph (under the palette)

Type `clip` (or `clipboard`) in the palette for what you copied lately, newest first;
`clip <words>` narrows it. Enter copies an entry back; right-click pins it, removes it,
or clears the history. Up to 50 entries of text are kept, each up to 64 KB, in memory
only: nothing is written to disk and all of it is gone at logout. What a password
manager copies is never kept. It needs `wl-clipboard` (`wl-paste`).

## For docs/UNTESTED.md

- [ ] Copy from a Wayland app and from an X11 app (through Xwayland); both appear
- [ ] A password copied from KeePassXC or another manager that sets the hint does not appear
- [ ] Without `wl-clipboard` the palette says so under `clip` and nothing else changes
- [ ] A pin survives 50 further copies; Clear history empties it

## Note on the test suite

`term::tests::stalled_ui_backpressures_noisy_shell` failed here three times (10 s
deadline, 4 MiB through the debug parser) with a load average near 30 on 4 cores from
the other workers' builds. term.rs is unchanged on this branch; rerun it on a quiet
machine before treating it as a regression.
