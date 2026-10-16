# Apex: the most beautiful and the most powerful terminal desktop

User request: do all fifteen directions from the 16 October brainstorm. Preserve the
Signal visual language (DESIGN.md), the owners in UX-CONTRACT.md, authenticated IPC
and lock behaviour. Native Rust; no web redesign; no glassmorphism or wallpaper.

## Beauty — what only a compositor owner can do
1. **Motion system.** One easing (cubic out), one duration scale (120–220 ms),
   honoured by the compositor (windows materialise, workspaces slide, split and
   fullscreen changes animate geometry) and the shell (dock, panels). A reduced-motion
   preference disables all of it; nothing animates while locked or picking a screenshot.
2. **Phosphor.** Compositor post-processing: accent glow, optional scanlines, slight
   bloom on bright text, one `glow` setting from 0 to 1. CRT, not frosted glass.
3. **Boot-to-desktop continuity.** Plymouth splash, compositor splash and the shell's
   first frame form one sequence: the glitch resolves into the frame, panels power up.
4. **Panels as instruments.** Per-core bars, sparklines with history, network
   waterfall, disk I/O, temperatures, fans, GPU; the globe shows live connections.
5. **One accent, whole palette.** The terminal palette, caution and muted colours are
   derived from one accent in OKLCH; the user picks a colour and everything agrees.

## Power — what only a shell that owns the terminal can do
6. **Command blocks.** OSC 133 prompt marks from bash, zsh and fish: prompt gutter
   marks, exit status, jump between prompts, select or copy a command's output.
7. **The frame follows the machine you are in.** When the active terminal is an SSH
   session, the panels show that machine's CPU, memory, processes and connections.
8. **The palette as the OS surface.** SSH hosts, calculator and unit conversion,
   processes, systemd units, user commands from `~/.config/des-ui/commands/`,
   clipboard history. Queries are never interpreted by a shell.
9. **Panel widgets from programs.** A program prints JSON lines; the panel draws it in
   Signal style. Declared in `~/.config/des-ui/widgets/`.
10. **Window rules, scratchpad and picture-in-picture.** Per-app-ID rules (workspace,
    display, side), a drop-down scratchpad terminal, a window pinned above the frame.
11. **Monitoring with alerts.** Thresholds with hysteresis raise notifications through
    the existing centre: temperature, disk full, failed units, memory.
12. **Portals.** Screenshot and screencast portals so browsers and OBS share the
    screen; a colour picker; night light through DRM gamma.

## Foundations
13. Hardware acceptance stays the user's: docs/UNTESTED.md and HARDWARE.md.
14. **`des-ui-ctl`.** A CLI that talks to the shell over a private socket with an
    allow-listed vocabulary; the compositor still refuses everyone but the shell.
15. **Accessibility.** AccessKit through egui; an on-screen keyboard.

## Boundaries
- No secrets in logs or sockets; every new socket is 0600, peer-checked, bounded.
- All new sampling is off the UI thread and bounded in time and memory.
- New configuration uses the existing `key = value` file through config.rs.
- Every piece lands with tests; hardware behaviour is listed in docs/UNTESTED.md.
