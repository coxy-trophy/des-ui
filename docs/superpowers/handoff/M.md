# Handoff M: the power-up

Done on the branch:
- `shell/src/powerup.rs`: `PowerUp` (the timeline), `Stage` (per-part progress,
  `alpha`, `offset`, `apply`, `swept*`), `progress_for`, `stage(ctx)`, and
  `frame_lines`, which draws the frame's one-pixel lines sweeping out from the middle
  of the workspace.
- `common/src/splash.rs`: the splash's `FULL_FOR` and `FADE` are now shared. The
  compositor's splash reads them from there, and the shell starts its sequence
  `FULL_FOR` after its first frame, which is when the splash begins to fade.
- `shell/src/panels.rs`: `left`, `right`, `status` and `FsView::show` read the stage
  from the egui context and fade and slide themselves in. The instruments (CPU and
  memory sparklines, per-core bars, GPU, memory bar, disk and network waterfalls,
  filesystem meters) sweep from zero. No signature changed.
- `shell/src/term.rs`: the grid's painter draws at the terminal part's alpha until
  the sequence is done.

How it is carried: the stage lives in egui's temporary data, keyed
`des-ui-power-up`. It is published once a frame and removed when the sequence
finishes. A context without one reads as finished, so tests, the secondary displays
and every frame after the first second behave exactly as before. `backdrop` is not
touched.

## For the parent to wire

1. `App` gets `powerup: powerup::PowerUp`, built with `powerup::PowerUp::under_splash()`
   in `App::new`. Use `PowerUp::new(Duration::ZERO)` when `self.desktop.is_none()`:
   with `--windowed` there is no splash to wait for.
2. In `runtime.rs` `update`, right after the lock block returns (so it never runs
   behind the lock) and before the `CentralPanel`:
   ```rust
   let stage = self.powerup.publish(ctx, self.config.animate, self.lock.locked);
   ```
   Also call `self.powerup.publish(ctx, self.config.animate, true);` inside the
   `if self.lock.locked { … }` branch before its `return`. That makes a shell that
   starts locked (`--locked`, or a restart while locked) finish the sequence at once.
3. Replace the "one-pixel lines" block in `update`, from `lines.hline(screen.x_range(), status_top, line);`
   through the bottom-panel `hline`, with:
   ```rust
   powerup::frame_lines(&lines, screen, workspace, status_top, open,
       self.config.side_panel_behavior == des_common::PanelBehavior::Collapse, line, &stage);
   ```
4. Restart: Super+Shift+R `exec`s a new shell, which starts with a fresh `PowerUp`,
   so nothing else is needed. If the shell ever restarts in-process, call
   `self.powerup.restart()`.
5. Dock (optional): `dock.show` can multiply its alpha by `stage.alpha(Part::Status)`.
   The dock is hidden at session start anyway.

## Plymouth → splash → shell

`session/boot/des-ui.script` already matches the compositor splash. The ground is
`0.024, 0.031, 0.043` in both. The name is 321×63 in both (the compositor draws its
642×126 picture at half size). Both place the name at the centre minus half of
`63 + 44 + 6`. Both bars are 20 blocks on a 16 px step (316 wide), 44 px below the
name. No constants needed changing.

One gap remains: Plymouth's bar follows boot progress, but the compositor's bar starts
again from its own clock. At the hand-over the bar can drop from roughly 18 blocks to
a few. Closing it would need the compositor to read how far Plymouth got, for example
by having the session script pass `DES_SPLASH_FILLED`. That is left for later.

## For docs/UNTESTED.md

- A real boot, Plymouth → compositor splash → power-up: no black frame between any two
  of them, and the name and bar do not jump at the Plymouth hand-over.
- The bar's drop at the Plymouth hand-over (the known gap above): how noticeable it is.
- The frame lines sweep out as the splash fades; the columns, file panel, terminal
  text and status strip follow, and the instruments rise from zero, all within a
  second.
- Super+Shift+R: the splash, then the power-up again.
- Starting locked, and Settings → Frame → Animate off: everything appears at once.
