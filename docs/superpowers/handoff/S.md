# Handoff S: input protocols

All in `compositor/src/handlers/input_protocols.rs`, with small hooks elsewhere:
`state.rs` (one field, `init`, the shell's pid for the virtual keyboard), `policy.rs`
(`set_locked` lets go of the pointer), `capture.rs` (so does the screenshot chooser),
`handlers/compositor.rs` (places an input-method popup on commit), `input.rs`
(shortcut inhibition, relative motion, held/confined movement).

## Added

- **text-input-v3 + input-method-v2.** fcitx5 or ibus (with its Wayland frontend)
  composes for the window with the keyboard; smithay moves text-input focus with the
  keyboard focus. The candidate list is tracked in the `PopupManager`, so it is drawn
  with its window, and placed by `ime_popup_offset`: under the text cursor, above it
  near the bottom of the display, pulled back inside the display sideways.
  X11 applications keep using XIM through Xwayland; that is the IME's business.
- **pointer-constraints + relative-pointer.** A constraint activates once the pointer is
  on its window and inside the region it named. Locked: the pointer stays still and only
  relative motion is sent. Confined: the pointer moves only where the window (and its
  region) is. Every compositor shortcut, the lock and the screenshot chooser let go of
  the pointer (`release_pointer_constraint`); losing pointer focus does too (smithay).
  A locked pointer's cursor-position hint is honoured while the lock lasts.
- **keyboard-shortcuts-inhibit.** Granted to any application that asks. While the window
  with the keyboard holds an active inhibitor, compositor shortcuts go to it, except
  those `input::inhibitable` keeps: lock (Super+L), end session (Ctrl+Alt+Backspace),
  VT switching, the power key, and the hardware volume/brightness/media keys. Nothing is
  inhibited while locked or picking a screenshot.
- **virtual-keyboard-v1.** Only the running shell's process may use it (checked by
  client pid against `SHELL_PID`, set when the shell is spawned).

## Skipped

Nothing from the brief. No env vars are forced in `session/des-ui-session`.

## For the shell's on-screen keyboard

It can now type into applications: connect to `zwp_virtual_keyboard_manager_v1` from the
shell (e.g. with `wayland-client` + `wayland-protocols-misc`), send a keymap once (the
user's layout), then `key`/`modifiers` for each press. The compositor routes those keys
to the focused window like real ones, so they never reach the lock screen's password
field from anywhere but the shell itself.

## IME setup for users (README material)

fcitx5: install `fcitx5` and a frontend for GTK/Qt, start `fcitx5 -d` (an autostart entry
works once XDG autostart lands). On Wayland the toolkits use text-input; set
`XMODIFIERS=@im=fcitx` only for X11 applications. Do not set `GTK_IM_MODULE` (GTK4 then
skips text-input). ibus: `ibus-daemon -drx --panel disable`.

## For docs/UNTESTED.md

- [ ] fcitx5 Pinyin in Firefox and in a GTK4 app (gnome-text-editor): preedit shows in
      place, the candidate list sits under the cursor, above it near the screen's foot
- [ ] The candidate list on a second display and on a scaled display
- [ ] A game or a 3D viewer (e.g. a Wayland-native Minecraft launcher, Blender's
      continuous grab) captures the mouse; relative motion is smooth
- [ ] Super+1…4, Super+L and Print release a captured pointer
- [ ] A VM (virt-manager / GNOME Boxes) or a remote desktop grabbing the keyboard: Super+Tab
      and Super+Space reach the guest; Super+L still locks, Ctrl+Alt+Backspace still ends
      the session, Ctrl+Alt+F3 still switches VT
- [ ] Locking while a VM holds the shortcuts: the lock screen gets every key
