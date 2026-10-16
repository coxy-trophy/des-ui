# Handoff G: phosphor and night light

Done in the compositor: `compositor/src/phosphor.rs` (settings, Kelvin→RGB, night
window and ramp, the shader, and the per-display off-screen pass), hooked into
`udev.rs::render_udev` and the winit redraw, a `phosphor` field on `DesComp`
re-read in `input.rs::reload_input`, `splash::chosen_accent` made `pub(crate)` for
the glow tint, and `chrono` (clock only, already in the lockfile) for the night
window. `OutputElements` and `TopElements` gained a `Phosphor` variant.

## How it behaves

- With `glow = 0`, `scanlines` off and daylight colours, the pass is skipped and
  frames are drawn exactly as before: the default costs nothing.
- Otherwise each display is drawn into its own off-screen texture (only damage is
  redrawn) and shown through one shader. On the hardware, a frame with no damage and
  unchanged settings is not drawn at all, so an idle desktop stays idle; the night
  light ramp keeps frames coming only while it moves (60 s).
- If the shader fails to compile the pass turns itself off and logs once.
- Screenshots and recordings draw the display afresh without the pass, so captures
  stay clean. (The test-only `--screenshot` of the windowed backend reads the real
  framebuffer and therefore does include it.)

## For the parent to wire (Settings → Appearance)

All keys go in `~/.config/des-ui/config`; after writing any of them send
`Command::ReloadInput`, which is where the compositor re-reads them (as for
`animate`). No protocol change.

- **GLOW** slider 0–100 %: writes `glow = <0.0–1.0>` (a value above 1 is read as a
  percentage; remove the key or write 0 for none).
- **SCANLINES** switch: `scanlines = on`, or remove the key.
- **NIGHT LIGHT**: on/off, a from–to time pair and a WARMTH slider (6500 K = none,
  down to about 2700 K is comfortable; the compositor accepts 1000–6500):
  `night_light = 19:00-07:00 4500`; remove the key to turn it off. A window may run
  past midnight; equal start and end is read as off.
- The theme's accent tints the glow; the compositor reads it from `theme =` and the
  theme file, so a theme change should also send `ReloadInput`. (A custom
  `accent = r, g, b` override is not read yet: if wanted, read it in
  `Phosphor::reload` before falling back to `chosen_accent`.)

## For docs/UNTESTED.md

- [ ] GLOW at 30 % and 100 %: bright text and the accent bleed softly; nothing is
      offset or upside down on a laptop panel, a rotated display and the nested window
- [ ] SCANLINES on a 100 % and a 200 % display: faint, even lines that do not shimmer
- [ ] NIGHT LIGHT across its start and end times: colours warm and return over a
      minute; a window past midnight; idle power with it on and nothing moving
- [ ] All three off: frame time and idle power as before (the pass is skipped)
- [ ] A screenshot and a recording taken with glow on come out clean
- [ ] Frame time with glow on at 1080p and 4K on an integrated GPU (target < 1 ms)
