# Handoff C: compositor motion

Done in the compositor: `compositor/src/motion.rs` (Timeline, Motion, Motions,
window_elements), hooked into `policy.rs::arrange` (show/hide), `switch_workspace`
(slide direction), `forget_app`, `state.rs` (field, per-frame tick), `input.rs::reload_input`
(re-reads the setting) and both render paths, which now list windows themselves
instead of `space_render_elements`.

## For the parent to wire

1. **One setting, not two.** The compositor reads `animate = on|off` from
   `~/.config/des-ui/config` (default on). The shell has `dock_animation` with the
   "Animate dock" switch in Settings → Frame. Unify: rename the switch to
   "Animate" ("Windows, workspaces and the dock; turn off for reduced motion"),
   keep reading `dock_animation` as a fallback in `Config::parse` for old files, write
   `animate` from now on, and pass the one value to the dock. After writing it, send
   `Command::ReloadInput` so the compositor picks it up at once (that is where it
   re-reads; no protocol change needed).
2. **Reduced-motion preference.** If the shell ever reads a desktop reduced-motion
   hint (DESIGN.md mentions one), write it through the same key.
3. **Hardware acceptance** to add to docs/UNTESTED.md:
   - A window materialises (grows from 96%, fades in) on open; split, full screen and
     a move to another display ease over 180 ms; Super+1…4 slides the workspace
     sideways over 220 ms, higher workspaces arriving from the right.
   - Nothing moves while locked or while the screenshot chooser is up.
   - `animate = off` snaps everything; idle power after a motion settles is unchanged
     (no frames are drawn once nothing moves).
   - A client slow to resize is scaled into its new area rather than jumping.

No shell files were touched.
