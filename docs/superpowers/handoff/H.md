# Handoff H: window rules, picture in picture, the shell overlay

Done in the compositor and the protocol:

- `compositor/src/rules.rs`: `~/.config/des-ui/rules`, one rule per line,
  `<app_id or X11 class glob> [workspace=<1-4>] [display=<n>] [side]`, `#` comments,
  first match wins, case-insensitive `*`/`?` globs, bad lines skipped. Applied once per
  window: in `add_window` when the identity is already known (X11), otherwise at the
  window's first commit (`handlers/compositor.rs`), before its initial configure, since
  Wayland clients set their app ID after the toplevel exists. Read afresh each time,
  so an edit counts for the next window. What a moved window had opened on top of gets
  its place back.
- Picture in picture: `Command::Pin(n)` / `Command::Unpin` (`pin <n>`, `unpin`) and
  Super+P (pins the application in use, or unpins). One pinned application at a time:
  a quarter of the workspace wide, at most half as high, its shape kept, in the bottom
  right corner clear of the dock strip, above every application on every workspace of
  its display. A click on it gives it the keyboard (Super+Q and Super+O then act on
  it); a click elsewhere takes the keyboard back. Unpinning returns it to the place it
  was pinned from on its own workspace. Hidden while locked.
- `AppInfo::pinned`. On the wire it is a `:p` after the saved place; the saved place is
  written whenever anything is pinned, so the field order stays unambiguous. Nothing
  pinned and nothing retained gives exactly the old line.
- `Command::Overlay(Option<(x, y, w, h)>)` (`overlay <x y w h|none>`): a third lifted
  rectangle of the shell's window on the main display, drawn above every window and
  beneath the toasts and docks, with pointer input going to the shell there and the
  keyboard going to the shell for as long as it shows. Refused while a screenshot is
  being chosen; cleared on lock, on a shell restart and when the shell disconnects.

## For the parent to wire in the shell

1. **Overlay rect (the drop-down scratchpad).** Mirror the dock: keep
   `overlay_area: Option<Option<(i32, i32, i32, i32)>>` on `App`, compute the
   rectangle in main-display logical points from the scratchpad's egui rect each frame
   (`rect.min.x.round() as i32` etc., as `runtime.rs` does for `dock_area`), and send
   `Command::Overlay(area)` only when it changes. Send `Command::Overlay(None)` when
   the scratchpad closes, on lock, and reset the cached value to `None` (unknown) on
   reconnect, as with `dock_area`, so the compositor is told again.
2. **Reading `pinned`.** `DesktopState.apps[i].pinned`. The dock may badge it and offer
   "Unpin" (send `Command::Unpin`) or "Pin" (`Command::Pin(i)`) in its context menu;
   `des-ui-ctl` could gain `pin`/`unpin`. A pinned app reports `place: Hidden`, so code
   that treats Hidden as "not visible" should also check `pinned`.
3. **USAGE / README keys:** Super+P is listed in `des-comp --help`; add it to the README
   key table ("Keep the current application in a corner above everything, or put it
   back") and document `~/.config/des-ui/rules` with an example:

   ```
   # ~/.config/des-ui/rules
   firefox        workspace=2
   org.gnome.*    display=2 side
   mpv            side
   ```

## For docs/UNTESTED.md

- [ ] A rule for a Wayland app (Firefox) and an X11 app (xterm) sends each to its
      workspace and display; `side` opens it beside what is there; an edited rules
      file counts for the next window
- [ ] Super+P pins a video player in the corner; it stays in view through Super+1…4;
      Super+P again puts it back in its half
- [ ] Clicking the pinned window types into it; clicking elsewhere takes the keyboard
      back; Super+Q closes it while it has the keyboard
- [ ] Locking hides the pinned window, and it comes back after unlocking
- [ ] The overlay (once the scratchpad exists): drawn above applications and below
      notifications, keys go to it, it disappears on lock and during screenshot picking
