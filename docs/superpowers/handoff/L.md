# Handoff L: the screenshot portal

What landed:
- `portal/` (`des-portal`): a session-bus service, `org.freedesktop.impl.portal.desktop.des-ui`,
  serving `org.freedesktop.impl.portal.Screenshot` version 2 (`Screenshot` and `PickColor`)
  at `/org/freedesktop/portal/desktop`. For each call it exports an
  `org.freedesktop.impl.portal.Request` at the handle (`Close` cancels the call) and
  asks the shell one line over the control socket:
  - `Screenshot` with `interactive = true` sends `screenshot-file region`; otherwise
    `screenshot-file screen`;
  - `PickColor` sends `pick-color`.

  The answer becomes the portal's: `ok {"uri":"file:///..."}` → response 0 with
  `uri`; `ok {"r":..,"g":..,"b":..}` (0–255) → response 0 with `color (ddd)` in 0–1;
  `error cancelled` → 1; anything else, or no answer within 120 s → 2.
- `shell/src/control.rs`: `Request::ScreenshotFile(ShotMode)` and `Request::PickColor`, plus
  `Request::deadline()`. These two get 120 s before the client is told the shell did not
  answer; everything else keeps 10 s. `des-ui-ctl` waits up to 130 s for them.
- `session/des-ui.portal`, `session/org.freedesktop.impl.portal.desktop.des-ui.service`
  (D-Bus activation, so nothing has to start the portal), `des-ui-portals.conf`
  (`Screenshot=des-ui;gtk`, default still `gtk`), a `pkill des-portal` on logout in
  `des-ui-session`, and entries in install.sh, uninstall.sh and packaging/build-deb.sh
  (the package's service file is rewritten to `/usr/bin/`).

## For the parent to wire

Both requests are answered late: the `Reply` is held until the user has finished.
So that the shell compiles meanwhile, `answer_control` in runtime.rs has one interim
arm, added on this branch: it answers both with `error not available yet`, and the
portal turns that into response 2 at once. Replace that arm with the following.

1. **Fields on `App`:**
   `pending_shot_replies: Vec<control::Reply>` and `pending_color: Option<control::Reply>`.
   Hold at most one of each kind; a second request while one waits gets
   `Answer::Error("busy".into())`.

2. **`Request::ScreenshotFile(mode)`:** refuse when `!compositor` or `self.capture.marking()`.
   Otherwise push the reply onto `pending_shot_replies` and
   `self.send(Command::Shot(mode))`. Do not `reply.send` yet.

3. **Completing it, in `capture.rs`:** `Capture::keep` saves on a thread and sends
   `Result<(PathBuf, bool), String>` down `self.saved`, which `Capture::report(&mut notices)`
   drains each frame. Make `report` return what it drained (or add
   `pub fn take_saved(&mut self) -> Vec<Result<PathBuf, String>>` beside it, keeping the
   notices). Then, where runtime.rs calls `report`:
   - for `Ok(path)`, pop one held reply and send
     `Answer::Json(json!({"uri": file_uri(&path)}).to_string())`. `control::file_uri`
     (on this branch, tested) percent-encodes every byte outside `A–Z a–z 0–9 - . _ ~ /`,
     so that spaces and non-ASCII names survive; the portal refuses anything not
     starting with `file:///`;
   - for `Err(why)`, send `Answer::Error(why)`.
   - When marking ends in `Choice::Cancel` (Escape, a click beside the choices, Print again),
     answer every held shot reply with `Answer::Error("cancelled".into())`. `mark()` returns
     `false` for both a cancel and a kept piece today. Return an enum
     (`Marked::Kept | Marked::Cancelled | Marked::Record | Marked::Busy`) or record the
     cancel in a flag that runtime.rs reads.
   - If the shell is locked while a reply is held, answer it `Answer::Error("cancelled".into())`
     in `finish_lock`.

4. **`Request::PickColor`:** refuse while locked, marking, or without a compositor. Otherwise
   hold the reply in `pending_color` and send `Command::Shot(ShotMode::Region)`. That
   freezes the display into `Capture::marking` with `choosing == false`, which goes
   straight to dragging. Then:
   - add a `colour: bool` flag to `Marking`; set it through a new
     `Capture::pick_colour_next()` called before sending the shot;
   - in `mark()`, when `colour` is set, a primary click (no drag) reads one pixel of
     `marking.picture.rgba` at `(at(x - screen.left()), at(y - screen.top()))`, clamped
     to the picture, and finishes with a new `Choice::Colour([r, g, b])`. Show a cross-hair
     and a small swatch of the colour under the pointer instead of the selection rectangle.
     Escape cancels.
   - runtime.rs answers `pending_color` with
     `Answer::Json(json!({"r": r, "g": g, "b": b}).to_string())`, or
     `Answer::Error("cancelled".into())`.

5. **README**, under "Screenshots and recordings":

   > Applications that ask the desktop for a screenshot, such as a browser's
   > screen-capture extension or a colour picker in a design tool, go through the same
   > chooser: the `des-portal` program answers xdg-desktop-portal's Screenshot portal by
   > asking the shell. An interactive request lets you mark an area; a colour request asks
   > for a click. `des-ui-ctl screenshot-file region` and `des-ui-ctl pick-color` do the
   > same from a terminal and print the file or the colour as JSON.

6. **docs/UNTESTED.md:**

   ```
   ## Screenshot portal

   - [ ] Firefox: a screenshot through the portal (e.g. `flatpak run` an app that asks, or
         `gdbus call --session --dest org.freedesktop.portal.Desktop --object-path
         /org/freedesktop/portal/desktop --method org.freedesktop.portal.Screenshot.Screenshot
         "" "{'interactive': <true>}"`), the chooser appears and the file is returned
   - [ ] Chromium and a Flatpak app the same way; GIMP's or Inkscape's colour picker via PickColor
   - [ ] Escape in the chooser makes the application see "cancelled", not a failure
   - [ ] `journalctl --user -u xdg-desktop-portal` shows des-ui chosen for Screenshot and
         gtk for the rest; no 25-second waits on start
   - [ ] Locked: a portal request is refused at once and nothing is captured
   - [ ] A file name with spaces or non-ASCII letters arrives intact as a URI
   ```

## Out of scope: ScreenCast

Screen sharing (`org.freedesktop.impl.portal.ScreenCast`, used by browsers for WebRTC
and by OBS) needs a PipeWire video stream, not a file:
- the compositor would export frames as DMA-BUFs, or SHM as a fallback, into a PipeWire
  node it creates, with the `pipewire` crate and its own loop thread;
- it would need damage-driven frame pacing and cursor metadata;
- the portal would need `CreateSession`, `SelectSources` (an output or window chooser drawn
  by the shell), `Start` returning the node id, and the `org.freedesktop.impl.portal.Session`
  object.

That is mostly compositor work (render to an offscreen buffer per stream, which
`capture.rs` in the compositor already does for recording) plus perhaps 600–900 lines.
It belongs in its own task once recording through ffmpeg has been validated on hardware.
