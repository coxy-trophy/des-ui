# Handoff R: the protocols applications assume

Each protocol is in its own file under `compositor/src/handlers/`; state.rs gains three
fields (`primary_selection_state`, `xdg_activation_state`, `xdg_foreign_state`) and the
globals that keep nothing.

## Added

| Protocol | File | What it does here |
|---|---|---|
| fractional-scale-v1 | `scale.rs` | Each window is told its display's exact scale (1.25, 1.5…) once a frame, from `before_frame`; smithay only sends what changed. Whole-number `wl_surface.preferred_buffer_scale` is the scale rounded up. |
| viewporter | `buffers.rs` | Global only: smithay's renderer already honours viewport source and destination through `on_commit_buffer_handler`, which the commit handler calls. |
| single-pixel-buffer | `buffers.rs` | Global only: the GLES renderer imports these buffers itself. |
| presentation-time | `presentation.rs` | On the hardware, feedback is gathered as each frame is queued (the DRM output's user data is now that feedback) and answered at its page flip with the kernel's monotonic time, sequence and the monitor's refresh. In a window, frames are answered as shown when handed to the host. |
| cursor-shape | `cursor_shape.rs` | Named pointers: an I-beam for text, a hand for links, the arrow for everything else. The three pictures (and the arrow from udev.rs) now live there, with hotspots. In a window the host draws the pointer, as before. |
| primary-selection | `primary_selection.rs`, `xwayland.rs` | Select, then middle-click, between Wayland applications, and with X11 applications through the X server's PRIMARY, the way the clipboard already was. |
| xdg-activation-v1 | `xdg_activation.rs` | A token made under recent input by the window with the keyboard (no older than 10 s, used once) brings its window forward through the existing `activate` path. Any other request, or one while the screenshot chooser is up, marks the window `urgent` instead. While locked, requests are ignored. |
| xdg-foreign-v2 | `xdg_foreign.rs` | Smithay keeps the imported parent; dialogs of sandboxed applications follow their window. |
| xdg-output | already there | `OutputManagerState::new_with_xdg_output` was in place. |

`urgent` is new on the compositor's `App` and on `AppInfo`: it is sent as the mark `u`
beside the pin's `p` (`:u`, `:p`, `:pu`), so a state with nothing pinned or urgent is
exactly as before. It is cleared whenever the application is activated.

## Skipped

- **tearing-control**: not in smithay 0.7.
- Presentation feedback for windows in the middle of a motion is answered on the
  display the window is placed on, which is where it is drawn once it settles.

## For the shell

- `AppInfo::urgent` is true for an application that asked to come forward and was not
  let. Show it: the dock icon in the accent with a small dot, the same mark beside its
  entry in the window list of the palette, and optionally a quiet notification
  ("Firefox wants attention"). Activating it clears the flag in the compositor.
- Shell literals of `AppInfo` gained `urgent: false` (dock, palette, control and
  session-restore tests); nothing else in the shell changed.

## For docs/UNTESTED.md

- [ ] At 150%: Firefox, a GTK4 application and a Qt 6 application are sharp, not blurry; the
      shell itself is sharp; moving a window between a 100% and a 150% display changes it
- [ ] Middle-click paste from a Wayland application to an X11 one (xterm) and back
- [ ] Ctrl+click on a link in the terminal brings the running browser forward; a link
      opened by a background program marks the browser urgent instead of taking the keyboard
- [ ] Nothing is brought forward over the lock or the screenshot chooser
- [ ] Video in mpv and a browser plays without judder; `mpv --video-sync=display-resample`
      reports the display's refresh
- [ ] The I-beam over text fields and the hand over links in GTK4 and Qt 6 applications;
      the hotspots point where they should
- [ ] A Flatpak application's file chooser stays attached to its window
