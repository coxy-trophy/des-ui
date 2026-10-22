# Desktop UI and interaction consistency

## User standard

A modern, futuristic terminal-first desktop with one consistent visual language across terminal, Settings, Files, panels and supporting desktop surfaces. Shared material depth, restrained accent, clear typography, responsive controls and keyboard usability govern this pass; native visual acceptance is required separately from compilation.

## Implemented

- Canonical geometry and typography in ui.rs/main.rs::apply_style: 8px spacing, 30px native control height, 6px control radius, 10px floating radius, display UI copy and monospace readings/paths/terminal text.
- Shared custom controls: quiet selected tint, hover treatment, disabled opacity, visible keyboard-focus outlines; named sliders with focused arrows, Shift steps, Home/End and clamping, leaving unfocused terminal input untouched.
- Terminal: restrained tab surfaces/underline, preserved cell rendering and terminal semantics, shared glass search toolbar with space for larger controls, existing continuous wallpaper background and gutter. Scratchpad uses terminal material.
- Settings: wrapped descriptions and dynamic switch-row height, adaptive chip/location layouts, native fields with shared focus geometry, display copy and mono paths/readings. Persistence and wallpaper draft behavior retained.
- Files: folder hierarchy, compact New folder/Paste/More toolbar, clearable search, distinct vector folder/file glyphs, responsive secondary metadata, accessible path identity. Operations, native paths and picker restrictions retained.
- Shared material integration in panel tiles, launcher, dock/tooltips, calendar, Quick Settings, OSD, preview, notifications, on-screen keyboard and music surfaces. Secure authentication/login, photo content and intentional editable/data insets retain appropriate opacity.
- Quick Settings scrolls within its bounded panel; sound controls wrap at narrow widths. Music selected rows/playback controls use shared tint/geometry and accessibility labels.

## Evidence and review

Observed red tests established native control font/geometry drift, missing icon focus outlines, pointer-only slider behavior, and Files list selection changing behind an open More menu. Read-only review identified the menu/Enter ownership issue; Files now reserves menu/dialog input and root shortcuts respect that ownership. Runtime evidence exposed an egui 0.31 nuance: a visible menu invokes its content closure while returning inner=None. Ownership is therefore tracked from closure execution, with background controls/list inert while the menu owns input. Two existing test fixtures were updated to install the same custom fonts the real application installs.

The premium static audit reports zero findings but does not inspect Rust native rendering. Native installed-session screenshots, DPI/output checks, real compositor performance and full visual acceptance remain pending in docs/UNTESTED.md. No install, session restart or commit is performed. Final test/build results follow below.

Final workspace tests: **576 passed, 9 failed**. The nine failures are unchanged sandbox restrictions: two compositor IPC, four shell service/login transports and three private tmux socket tests. All six added regressions pass. Final read-only review confirms the Files menu ownership issue is resolved and found no further concrete Important/Critical issue in that delta. `git diff --check` passes. Source snapshot: `/tmp/desui-cohesion-source.json`. Build result follows after completion.

`cargo build --workspace --locked --offline -j2` passed without warnings. Strict premium static audit reports zero findings (not native visual evidence). No source changed after the final test snapshot. Native installed-desktop visual acceptance remains outstanding.
