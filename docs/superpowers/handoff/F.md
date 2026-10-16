# Handoff F: control socket and des-ui-ctl

What landed: `shell/src/control.rs` (the socket, `Request`, `Answer`, `Reply`,
`Control`, `state_json`), `shell/src/bin/des-ui-ctl.rs` (the CLI), the socket path in
`common/src/control.rs` (shared by both programs, like the music one), and the new
binary in install.sh, uninstall.sh and packaging/build-deb.sh. shell/Cargo.toml gained
the rustix `net` feature (SO_PEERCRED) and explicit `[[bin]]` entries.

## To wire in the shell

1. `App` gets a field `control: Option<control::Control>`. In `App::new`, after the
   notification centre exists:
   ```rust
   let control = match control::Control::start(ctx.clone()) {
       Ok(control) => Some(control),
       Err(error) => { eprintln!("des-ui: no control socket: {error}"); None }
   };
   ```
   Do not start it when `--write-boot-splash` or another one-shot mode runs.

2. In `runtime.rs`'s `update`, early (after `desktop.pump()`), drain and answer:
   ```rust
   let requests = self.control.as_mut().map(|c| c.take()).unwrap_or_default();
   for (request, reply) in requests {
       use control::{Answer, Request};
       let answer = match request {
           Request::Ping => Answer::Ok,
           Request::State => Answer::Json(control::state_json(
               self.desktop.as_ref().filter(|d| d.connected()).map(|d| &d.state), self.lock.showing())),
           Request::Lock => { self.lock_now(); Answer::Ok }
           Request::Workspace(n) => { self.send(Command::Workspace(n)); Answer::Ok }
           Request::Toast(text) => { self.notices.post("des-ui-ctl", &text, false); Answer::Ok }
           Request::Dock => { self.dock.reveal_keyboard(); Answer::Ok }
           Request::Launcher => { if !self.launcher.open { self.toggle_launcher(); } Answer::Ok }
           Request::Settings(None) => { if !self.settings.open { self.toggle_settings(); } Answer::Ok }
           Request::Settings(Some(section)) => {
               self.launcher.open = false; self.music.open = false; self.notices.open = false; self.power_menu.open = false;
               self.settings.open_at(&self.config, &section); self.send(Command::Terminal); Answer::Ok
           }
           Request::Music(word) => { self.music.command(word.word()); Answer::Ok }
           Request::Panel(panel) => { self.send(Command::TogglePanel(panel)); Answer::Ok }
           Request::Screenshot(mode) => { self.send(Command::Shot(mode)); Answer::Ok }
       };
       reply.send(answer);
   }
   ```
   - `self.lock.showing()`: use whatever `Lock` exposes for "the lock screen is up"
     (the field `reconcile_lock` reads); `false` is acceptable for a first cut.
   - `self.music.command(word)`: `Music::send` is private and takes a JSON value;
     add `pub fn command(&mut self, word: &str) { self.send(json!({ "cmd": word })); }`
     beside it. The words are exactly `toggle`, `next`, `prev`.
   - While locked, answer `Request::Lock`, `Ping` and `State` only; give the others
     `Answer::Error("locked".into())` so a script cannot open the launcher over the
     lock screen. Also refuse `Screenshot` and `Dock` while `self.capture.marking()`.
   - Without a compositor (`--windowed`), `send` is a no-op; `Workspace`, `Panel` and
     `Screenshot` may answer `Answer::Error("no compositor".into())` when
     `self.desktop.is_none()`.

3. Nothing else: the socket thread already calls `ctx.request_repaint()` when a
   request arrives, and an unanswered `Reply` (dropped) tells the client
   `error the shell did not answer`, so a forgotten arm fails safe.

## Note for every worker
The shared `CARGO_TARGET_DIR` gives wrong builds across worktrees: path crates share
one fingerprint slot, so a worktree whose edits are older than another worker's last
build is taken as "Fresh" and links against the other tree's `des-common`. Each
worktree must build into its own `target/`.
