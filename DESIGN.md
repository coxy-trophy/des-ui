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
  control: '6px'
  surface: '10px'
  dock: '12px'
omitted:
  - section: spacing
    reason: 'Existing native egui geometry and shared UI primitives remain authoritative.'
---
## Overview
DEs-UI is a modern, futuristic desktop for people who work in terminals and value visual craft. Every feature must belong to the same coherent desktop: deliberate hierarchy, restrained accent, precise spacing, usable keyboard flows and shared materials. Preserve the instrument layout, globe and telemetry while making their presentation feel integrated rather than assembled from unrelated panels. A feature is not visually accepted on build/test results alone; inspect its native rendered states before calling its UI finished.

## Colors
Runtime palette is defined by shell/src/ui.rs. Theme accent belongs to Theme.main in shell/src/theme.rs; do not hardcode an accent in the dock or launcher. Caution remains the shared caution color. Settings → Appearance uses egui’s opaque sRGB picker for a draft accent; Apply color persists it and Reset to theme removes the override. Config::resolved_theme is the canonical adapter for preset plus custom accent; terminal ANSI colours and interface greys retain their existing owners.

## Typography
Use ui::display for interface labels, body copy and native control text (13px baseline), and ui::mono for readings, paths and terminal text, installed by ui::install_fonts. Section labels use restrained tracking; descriptive copy wraps rather than disappearing behind ellipses. Terminal font preferences remain independent. Installed application icons replace letter avatars, with one generic icon fallback.

## Layout
Graphical app switching lives in the existing right panel. System is the default tab; Apps retains the application shelf. Running applications appear first in a vertical layout with height-budgeted pages with compact icons, readable names and restrained focus/urgency indicators. Expand groups for window titles, close and workspace controls; context menus provide launch, pin ordering and previews. The bottom dock and its reserved band are removed. Super+D opens Apps and takes temporary keyboard ownership; secondary displays expose a transient right rail. Primary Apps includes all displays and workspaces. Pins retain desktop IDs, grouping uses compositor app IDs or X11 class. Three panel geometry stays owned by des-common.

## Elevation & Depth
Solid matte surfaces remain the default. Settings → Appearance can add a private static wallpaper and wallpaper-backed Frosted surfaces; fine strokes and sharp foreground text continue. material.rs is the shared role-based painter, wallpaper.rs owns image work, and appearance_editor.rs owns the draft. The dock is lifted above app content through the existing compositor crop mechanism; its Frosted material samples wallpaper. Live application-aware Liquid glass remains unavailable pending its compositor prototype and native acceptance. Use the existing right panel rather than adding a sidebar.

## Shapes
Controls share a restrained 6px corner radius; content-sized floating surfaces use 10px and the dock 12px. Workspace boundaries, terminal cells, monitor diagrams and image artwork retain their semantic geometry. ui.rs owns SPACE (8px), CONTROL_HEIGHT (30px), CONTROL_RADIUS and SURFACE_RADIUS; main.rs::apply_style adapts these to native egui controls. No screen should invent another routine button style.

Terminal tab selection uses a short expanding underline when terminal tab animation and desktop animation are enabled. Reduce motion or disabling either animation setting makes it immediate. The terminal settings retain the existing font, size, line height, cursor, copy and scrollback controls.

The terminal workbench adds a dismissible session rail in sufficiently wide terminal workspaces; narrow workspaces keep the header tabs. The rail uses actual terminal sessions and command navigation, keeps the nine-session limit visible, and must not cover graphical applications or consume space in their workspace. History, file and job drawers and contextual completion from the concept are separate work and must not be represented by sample data in the desktop.

The default pointer is a compact dark triangle with a soft blue halo, inspired by the supplied reference. The click hotspot stays at its tip; semantic cursor shapes and user-selected size and colour remain available. The compositor renderer and exported Xcursor theme share the same artwork.

## Components
The application launcher uses an icon grid in the workspace: 48px installed icons, centered two-line names, matte square selection tiles, and responsive columns. Blank search shows applications only; typed queries retain the universal palette providers, with small result-kind labels for non-apps. Descriptions and paths stay in hover details and accessibility labels. Settings owns Health and Session controls. Terminal find is a compact pane-local bar that wraps controls on narrow panes. All use shared ui.rs typefaces and palette; native egui widgets inherit apply_style. ScrollArea owns bounded result/log scrolling; no new document or browser scrollbar system is introduced. The behavioral owners and state contracts are in UX-CONTRACT.md.

Shared native icons: ui::Icon and paint_icon. Installed app artwork: app_icons::IconCache. Shared action/name tooltips and accessible button labels. Dock context menus use egui's existing menu pattern; no external/native dialogs. The Apps panel stays in place; hover/press feedback respects the appearance Reduce motion setting and existing global animation preference. Reduce transparency makes work/control surfaces opaque while retaining wallpaper.

## Do's and Don'ts
Keep numeric status readings and shortcut explanations. Preserve current terminals and panels. Never remove unknown pins automatically. Do not steal focus on pointer hover or show dock over lock. No fake app logos, avatar initials, or title-based app matching.

Notification preferences reuse the existing Settings → Sound & Screen switch rows and configuration owner. Processor temperature warnings have an independent persistent switch. Browser-opening errors reuse the notification center and do not change terminal selection or layout.

Settings → Session distinguishes LIVE TERMINALS (automatic process reconnection) from SAVED SESSION (explicit folder/application layout snapshots). Reuse switch_row, existing text styles and notification center for recovery results. Reconnected scratchpad remains hidden and terminal output is not persisted in the layout file.

World View keeps the dotted land, wire sphere and accent connection arcs. The
compact panel and expanded workspace share globe::Explorer camera/selection state.
The compact panel is canvas-only and resumes ambient rotation after interaction.
Double-click opens the expanded view, which uses the canonical workspace replacement
pattern and retains explicit exploration controls, not a floating modal. Keep search visible
above scrolling content; use two columns on wide workspaces and stack at narrow
widths. Shared ui::button, native egui TextEdit/selection/ScrollArea and theme fonts
own control behavior. Selected destination arcs and rings use existing TEXT ink.


## Wallpaper and frosted materials

AppearanceSettings in des-common is the typed persisted owner. Default is no wallpaper/Solid. Wallpaper supports static PNG/JPEG, Fill/Fit, normalized blur strength 0–40 and dim 0–80%; blur is a visual strength rather than a promised screen-pixel radius. The image worker caches 1600px source and 640px soft textures shared across outputs; no image processing runs per frame. Imported copies are normalized to at most 2560px per side. Choosing wallpaper establishes Frosted surfaces with a 62% tint by default. Terminal tint is adjustable from 35–100% and raised only as necessary for 4.5:1 normal bright terminal-text contrast against the cached image channel peaks; faint chrome-label contrast must not force the whole terminal opaque. Control surfaces keep a stronger minimum 75% tint and notifications 90%, with small-label contrast protection. Workspace padding, split gaps and terminal gutters sample the same screen-aligned blurred wallpaper, so no exposed bright picture frame surrounds a black terminal. Explicit Opaque and Reduce transparency remain available. Text, ANSI backgrounds, selections, icons and cursor stay sharp. Dock keeps its 12px radius. No new idle material motion.

The appearance editor uses native egui controls and the existing Apply/Reset vocabulary. Choose image uses the shared native Files browser in image-selection mode. Folder navigation, search, hidden files and typed paths use the same controls as the Files workspace; mutation controls are absent from the picker. The editor's preview is local until Apply appearance. Cancel changes and Reset appearance affect only this group; theme/accent retain their independent controls. Liquid and its future quality options must not advertise functionality before the compositor gate passes.


## Settings and Files

Settings uses a 196px grouped section rail on wide workspaces and a section selector below 720px. Appearance has persistent Wallpaper, Theme and Effects tabs, independent scroll positions, a preview before controls and stacked slider labels on narrow screens. Content respects the available workspace width.

Files is a native workspace opened through the bottom FILES control or launcher search “Open Files”. A folder title, editable path and clearable local search establish hierarchy. Keep navigation and compact New folder/Paste/More operation controls above its bounded scrolling entries. Rows pair distinct folder/file glyphs with names and secondary metadata, hiding decorative metadata at narrow widths. Native buttons, TextEdit and Modal own focus and keyboard behavior. Long names truncate visually with full path details available. Trash shows original names while retaining the actual stored path as identity. Use shared theme typography and surfaces.


## Visual acceptance standard

The signature is a continuous atmospheric wallpaper behind precise, sharp instruments. Blur belongs to the backdrop, never glyphs, selections or icons. Material depth distinguishes work, controls and floating surfaces without introducing unrelated card styles. Keep theme accents restrained and functional; avoid decorative neon, oversized borders and dense explanatory copy. Shared material.rs and ui.rs owners apply this across terminal, Settings, Files, launcher and dock. Native review must include the entire workspace, narrow/wide layouts, bright/dark wallpaper, focused/unfocused panes, terminal applications, keyboard/hover/disabled/error states, reduced transparency and reduced motion. Performance and functional checks accompany visual review; neither replaces the other. Deferred effects must not occupy the normal interface as disabled promotional controls.


## Shared interaction and surface ownership

Native widgets inherit main.rs::apply_style. ui.rs custom buttons, chips, icons and switches use the same radius, hover tint, disabled opacity and visible accent focus outline. Selections use quiet tint with bright text, avoiding large accent-filled slabs. Shared sliders accept focused Arrow keys, Shift for larger steps and Home/End; their accessible names identify the controlled value. Terminal arrows remain untouched when the slider is unfocused.

material.rs::surface owns content-sized native frames, and material.rs::paint owns fixed geometry. Tiles, calendar, Quick Settings, volume/brightness OSD, preview, scratchpad, dock/tooltips, notifications and music work surfaces share screen-aligned wallpaper materials. Launcher resting tiles leave the underlying workspace visible. Account passwords/authentication, lock/login, screenshot/photo content, explicit terminal ANSI backgrounds and small editable insets can remain opaque for their semantic purpose; their surrounding controls still follow the common geometry and typography. No changes to secure input, capture behavior or compositor overlay delivery are implied by visual migration.


## Shortcut reference and mappings

The right telemetry panel contains no keyboard cheat sheet. A searchable native modal owns the complete shortcut reference, using shared floating materials, grouped action labels and current bindings. Default entry is Super+/ in the desktop and Ctrl+Shift+H in the shell; the launcher also offers Keyboard shortcuts. Settings → Shortcuts uses the same registry, clearable search, compact binding rows, inline validation and reset/disable controls. Reset all requires a second explicit confirmation. Desktop Super combinations use explicit text entry because the Linux UI input adapter does not expose Super for recording; shell/terminal Ctrl combinations have a separate Record action. Manual editing must retain paste, Backspace and caret navigation.

common/src/shortcuts.rs owns definitions, normalized chords, defaults, validation and persisted overrides. shell/src/shortcuts.rs owns the reference/editor, live UI cache and dynamic hints. Shell and compositor routing must use that registry; new actions must be added there before advertising a binding. Application-local and protected hardware/security controls are identified separately. Tooltips must read the active mapping or omit key names, never duplicate a configurable default.


Window shelf: running markers, active underlines and counts distinguish application state. Multiple windows open compositor previews, nine per page, rather than cycling; old compositors receive a native window-list fallback. All windows opens the workspace overview. Collapsed side cards are 44 logical pixels wide (56px reservation including surrounding frame spacing). The collapsed left rail rotates the complete regional HH:MM clock counterclockwise by 90 degrees: glyphs face right, hours start at the bottom and minutes finish at the top. Preserve leading zeros and the 12-hour period. Shared floating materials and reduced-motion preferences apply throughout.

Drag overview: a confirmed native desktop drag opens destination previews. A 450ms hover reveals the real destination while retaining the original data offer and pointer grab; the user drops inside that application. Settings → Frame → Dock owns the persisted opt-out. Browser support depends on a browser exposing a native desktop drag; internal tab rearrangement is not guaranteed.


Hand control: keep the webcam indicator visible while active. Mouse & touchpad owns native pointer speed, pinch sensitivity (Firm–Gentle), independent scroll speed and release-pinch dragging controls. A short static accent marker beside the pointer confirms hand clicks; a steady center mark identifies a held drag. Markers never take input, animate continuously or appear over lock, screenshot selection or startup splash. Sharp geometry and shared theme accents remain authoritative.


Solid morph pointer: the approved family is a filled off-white silhouette with a dark contrast keyline. Point is a narrow convex triangle without a notch; links use a fuller rounded wedge, text an insertion needle, grab a compact slanted form and resize a directional spindle. Cursor-shape owns vector outlines, 120ms interruptible morphs and bounded high-DPI rasters. One hotspot anchors changes; Reduce motion and animation-off snap to static states. No decorative accent pieces or trails. Precise, forbidden, copy, move and zoom meanings remain distinct. Client-supplied cursor artwork and hidden cursors retain their behavior. The DEs-UI Xcursor fallback theme uses the same canonical raster artwork for clients without the shape protocol; it changes states statically.


Cursor preferences live in Settings → Mouse & touchpad, before mouse/hand controls. Reuse shared sliders, opaque RGB picker, Apply/Reset and native buttons. Size ranges16–64px (default24); default color is F7F6F2. Light/dark previews use the same shared triangle and base-size constants as the compositor; dark fills receive a light keyline with a fine dark outer hint. No alpha control or new decorative effects. Preview color is a draft until Apply, while size updates directly. Native cursors update live; the interface explains that some apps update when reopened.

## Cursor companion

The cursor companion uses the shared Floating material, display typography and theme controls. Its conversation surface anchors once near the pointer and stays still while typing. It does not add a face, decorative tail or perpetual cursor animation. Text and explicit voice summons have separate configurable mappings. A full modal owns keyboard events; Escape never reaches the terminal/launcher beneath it. Closing restores prior egui focus and stops speech/capture. Social text bubbles appear only on the exposed shell, do not focus themselves, fade after15seconds, and offer Talk/Quiet.

The companion's character is self-authored conversational data rather than a preset temperament. A blank identity journal evolves through typed exchanges; shared voice history provides continuity. Setup and private memory remain inside an explicit disclosure, keeping the primary surface focused on conversation. Capture, lock and authentication take precedence over the companion. No ambient desktop context or execution tools are attached.

Companion revision: connection/key, model, social preferences, limits, identity and memory now live directly in Settings → Companion. Summoned conversation contains no configuration form. Thought bubbles anchor once near the pointer, use the shared Floating material and theme accent, and have a brief cubic opacity entrance/exit with a small rising thought trail. Hit targets stay fixed, focus is never taken, and reduced motion removes transitions. Configurable global Keep/Mute/Stop/Quiet operations supplement text/voice summons; Quiet always disables social approaches.

## Bottom-panel companion —2026-10-24

The current conversation owner is the bottom panel, with Files and Chat tabs; the earlier floating conversation modal is retired. Super+J reveals/selects Chat and focuses its compact composer. Message history scrolls within shared bottom geometry; Escape releases typing without erasing the draft, and clicking a native app returns keyboard ownership while Chat stays visible. Settings owns credentials, memory and usage controls.

Voice is independent of Chat. Super+Ctrl+J summons it without opening text UI. A pointer-transparent thought caption indicates listening/speaking, while the existing solid cursor pulses gently only during output playback. The configured color, shape and hotspot remain intact. Captions have a separate render-only compositor crop and cannot steal clicks or keyboard focus. Reduced motion retains a steady indicator. Cursor pulse works across outputs; the first caption implementation is limited to the primary shell surface.

The key is remembered through the system Secret Service by default. Settings reports loading/saved/locked/unavailable states; it never falls back to plaintext configuration. Explicit Remove saved key disables subsequent autoload. A locked login keyring may need unlocking; it does not require re-entering the API key.


## Separated frame panels

The login default is System in the right panel. Apps remains an explicit tab and Super+D selects it. This selection is ephemeral; existing panel visibility preferences are unchanged.

`des_common::frame::Layout` owns the primary display rectangles for the shell and compositor. Frame cards use 8 logical-pixel gaps, an 8px corner radius and a subtle shared LINE outline. The center keeps a 4px rim around native application/terminal content, so native rectangular surfaces do not cover the rounded outer corners. Fullscreen and secondary-display application rectangles remain edge-to-edge. Side panels, collapsed rails, and Files/Chat reuse the same frame surface; inner controls keep their existing 6px radius. Wallpaper sampling remains screen-aligned and static. Use the existing startup progress for outlines; add no new idle repaint loop.

Workspace overview owns an opaque screen-sized presentation above desktop frame outlines. Preserve the shared wallpaper material while covering the underlying side cards, rails and center rim. Pack native previews into a 16px-padded content rectangle below each workspace heading; include labels and selection strokes in the fitting budget. Built-in screenshots and recordings must include the same native preview layer as the visible overview. Switcher and window chooser backgrounds remain bounded to their strips.

Desktop experience refinements: keep the instrument theme and shared Chakra/JetBrains typography. Music Home uses real library artwork and collection cards, with responsive navigation and a persistent playback transport; the compact playing card reserves 136px. Side panels never scroll: reserve the crab/animal footer and playback/local readings before optional instruments, reduce dense telemetry at short heights, and page application/window rows. The compact globe has no control strip; drag/touch pauses while interacting and rotation resumes on leaving. Double-click opens the existing workspace explorer. Respect disabled animation and reduced motion.

Artwork ownership: app_icons::IconCache shares one bounded cache per egui context across launcher, Apps and tray. Ready textures survive reopening and refresh. Background source/theme checks retain ready artwork while validating; decoded PNG thumbnails live under XDG_CACHE_HOME (HOME fallback), bounded to 512 entries. Source metadata invalidates stale artwork. Launcher application discovery runs off the UI thread and empty-query browsing includes every installed application.

The keyboard is a separate rounded card in the center column above the status strip. Side cards remain full height. common::frame::Layout::with_keyboard owns both the drawn reservation and native workspace placement; a bounded keyboard-height command keeps the compositor in sync, using the same rounded logical height. Keys and labels fit the actual card. Devices belongs in Settings, using background inventory and native section/button patterns; it reports real availability and offers hand-camera selection without opening streams merely to detect hardware.

Settings navigation follows a shared catalog: Desktop, Hardware, Connections, Personal and System. Keep search visible, accept familiar keywords, show explicit section summaries, and preserve stable routing IDs while using friendly display names. Narrow layouts use category/section selectors rather than seventeen stacked destinations. Research informs organization, while existing Linux capabilities and controls remain authoritative.

Pointer artwork uses the configured solid fill without a heavy opaque keyline, with a small translucent offset shadow for definition. Settings, named compositor cursors and exported legacy theme use the same silhouette/preferences. Forward only cursor lookup variables to Xwayland and keep standard aliases; app-provided custom cursor artwork remains app-owned. Mirrored scanouts never own a window's preferred client scale; independent outputs use greatest overlap and stable primary tie-breaking.

System panel spacing: Network and World View use only their painted height. Displays and local Controls follow immediately with an 8px section gap; unused height must not stretch the globe reservation into a blank middle band. Playback retains its protected bottom allocation, and the same height budget still prevents clipping on short panels.
