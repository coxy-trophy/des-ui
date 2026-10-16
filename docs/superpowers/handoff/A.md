# Handoff A: palette from one accent

Done on the branch: `shell/src/oklch.rs` (sRGB↔OKLCH, contrast, `legible`),
`Theme::derived()`, two new `Theme` fields (`caution`, `glow`), config key
`palette = derived` (`Config::palette`, applied in `resolved_theme` after the
accent), and the PALETTE section of Settings → Appearance with swatches.

What the parent must wire in `main.rs`:

1. In `App::apply`, the `Change::Set(key, value)` arm reloads the config but keeps
   `self.theme` as it was. After the reload add:

   ```rust
   if key == "palette" {
       self.theme = self.config.resolved_theme();
       apply_style(ctx, &self.theme);
   }
   ```

   No new `Change` variant was added, so nothing else changes shape.

2. Optional, for a later pass: `ui::CAUTION` is still the fixed constant everywhere
   meters and readings turn amber. With a derived palette `theme.caution` is the
   colour that agrees with the accent; switching those call sites in `ui.rs`,
   `panels.rs` and `sysmon`-driven bars to `theme.caution` completes the effect.
   `theme.glow` is for the compositor's phosphor pass (wave 2); the shell can hand it
   over with the accent wherever it already tells the compositor the theme.
