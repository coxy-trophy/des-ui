---
version: alpha
colors:
  ground: '#06080b'
  panel: '#0a0e12'
  track: '#141b22'
  line: '#1a232b'
  text: '#dce7ec'
  secondary: '#8fa3b0'
typography:
  display:
    fontFamily: 'Chakra Petch'
  utility:
    fontFamily: 'JetBrains Mono'
rounded:
  dock: '12px'
omitted:
  - section: spacing
    reason: 'Existing native egui geometry and shared UI primitives remain authoritative.'
---
## Overview
DEs-UI is a terminal-first Linux desktop. Preserve the existing three-panel instrument layout, globe, telemetry and files. This is incremental improvement, not the previously explored glass redesign.

## Colors
Runtime palette is defined by shell/src/ui.rs. Theme accent belongs to Theme.main in shell/src/theme.rs; do not hardcode an accent in the dock or launcher. Caution remains the shared caution color. Settings → Appearance uses egui’s opaque sRGB picker for a draft accent; Apply color persists it and Reset to theme removes the override. Config::resolved_theme is the canonical adapter for preset plus custom accent; terminal ANSI colours and interface greys retain their existing owners.

## Typography
Use ui::display for interface labels and ui::mono for readings/terminal text, installed by ui::install_fonts. Installed application icons replace letter avatars, with one generic icon fallback.

## Layout
Graphical app switching lives in an autohiding bottom dock. Terminal tabs, splits and workspace controls stay at the top. Each display owns a dock; they share pins and overlay content without resizing windows. Its width grows with its icons; at screen width it wraps into another row, with no dock scrollbar. Pins use desktop IDs, grouping uses compositor app IDs or X11 class. Three panel geometry stays owned by des-common.

## Elevation & Depth
Flat matte surfaces and fine strokes continue. The dock is lifted above app content through the compositor overlay mechanism; notifications remain a separate overlay. No glassmorphism, wallpaper or new permanent sidebar.

## Shapes
Existing controls keep their shapes. The dock alone uses a compact 12px rounded container and small hover/selection surfaces, derived from the same palette.

## Components
The command palette extends the existing launcher in the workspace, with explicit APP/WINDOW/SETTING/ACTION/FILE kinds and the same matte rows. Settings owns Health and Session controls. Terminal find is a compact pane-local bar that wraps controls on narrow panes. All use shared ui.rs typefaces and palette; native egui widgets inherit apply_style. ScrollArea owns bounded result/log scrolling; no new document or browser scrollbar system is introduced. The behavioral owners and state contracts are in UX-CONTRACT.md.

Shared native icons: ui::Icon and paint_icon. Installed app artwork: app_icons::IconCache. Shared action/name tooltips and accessible button labels. Dock context menus use egui's existing menu pattern; no external/native dialogs. Short eased vertical motion; desktop reduced-motion preference suppresses animation when available.

## Do's and Don'ts
Keep numeric status readings and shortcut explanations. Preserve current terminals and panels. Never remove unknown pins automatically. Do not steal focus on pointer hover or show dock over lock. No fake app logos, avatar initials, or title-based app matching.

Notification preferences reuse the existing Settings → Sound & Screen switch rows and configuration owner. Processor temperature warnings have an independent persistent switch. Browser-opening errors reuse the notification center and do not change terminal selection or layout.

Settings → Session distinguishes LIVE TERMINALS (automatic process reconnection) from SAVED SESSION (explicit folder/application layout snapshots). Reuse switch_row, existing text styles and notification center for recovery results. Reconnected scratchpad remains hidden and terminal output is not persisted in the layout file.
