# Window shelf, vertical clock and drag overview

Status: approved for implementation on 2026-10-21. Implemented; native desktop acceptance pending.

## Intent and success

Make running applications and their individual windows easy to discover, choose and drag content into. Keep DEs-UI's terminal-first workspace and futuristic, restrained glass design. The user requested an enhanced dock, a vertical clock on the collapsed left rail, and an animated overview of open apps as soon as a drag begins, including browser tabs where the browser exposes a desktop drag.

Success means a user can identify running versus pinned apps, select a specific window without blind cycling, see time with the left panel collapsed, and carry a supported drag into another app without losing the payload. Native rendered acceptance is required; passing Rust tests alone does not establish visual or interoperability success.

## Chosen approach and alternatives

Extend the existing autohiding dock into a window shelf, with an explicit window chooser and All windows entry. Reuse compositor-rendered window buffers and shared shell materials. Give dragging its own overview mode and lifecycle.

A conventional icon-only dock preserves the existing ambiguity around multiple windows. A permanent full-width taskbar exposes more titles but occupies workspace and weakens the instrument layout. The chosen shelf preserves compact geometry while making window selection explicit.

## Dock behavior

- Keep Terminal and Applications discoverable. Add an accessible All windows control opening the existing workspace overview across all four workspaces on the current display.
- Keep pinned application order and desktop-ID identity. Place unpinned running applications after a quiet separator. A pinned running app appears once, with its running state on that item.
- Show an active underline, a separate running marker and a count for multiple windows. Keep attention indication restrained and independent of selection. Never infer application identity from titles.
- Clicking an app with one window activates it. Clicking an app with multiple windows opens its window chooser; it does not cycle blindly. Clicking an inactive pin launches it through the existing launch path.
- The chooser contains individually selectable live window previews, short titles and workspace/display metadata. Full titles remain accessible through tooltips. Include every window of that app across workspaces on this display. Existing context actions retain pin, new window, close and move-display behavior.
- Keyboard dock navigation retains the configured shortcut. Arrows traverse dock items; Enter opens/activates. Within a chooser, arrows select a window, Enter activates and Escape returns focus to its dock item. A second Escape dismisses the dock and restores prior application focus.
- Hover highlights and describes items; it does not focus another application or open a chooser unexpectedly. A chooser remains open while the pointer crosses between its app item and preview surface. Clicking outside dismisses it without activating an underlying shell control.
- Preserve per-display docks and bottom-edge activation over fullscreen apps. The shelf overlays content without resizing applications or changing terminal geometry. At narrow widths, wrap dock slots and bound large collections to 45% of display height with scrolling. The preview chooser shows nine windows per page with Page Up/Down and wheel navigation rather than extending outside the display. Unknown or unavailable apps retain generic installed-icon fallback and removable pins.
- Preserve the current hide delay and short eased reveal unless native testing identifies a usability problem. Never hide while keyboard navigation, a chooser or a context menu owns input.

## Visual contract

Use DESIGN.md, material.rs and ui.rs as owners: Floating material, screen-aligned blurred wallpaper, 12px dock radius, shared surface/control radii, 8px spacing grid, Chakra Petch labels and JetBrains Mono metadata. Icons and text stay sharp. Use the configured accent for selection/focus, never a new neon palette. Separate pins and running items with spacing and a fine stroke, not labelled containers.

Preview geometry reserves title and metadata space. Selected, hovered, keyboard-focused, unavailable and urgent states must be distinguishable without relying solely on color. Use installed icons; no invented logos. Avoid idle magnification, bouncing icons or perpetual animation. Reuse compositor easing and approximately 180–200ms transitions. Both global animation-off and Appearance Reduce motion suppress movement; Reduce transparency uses opaque shared surfaces.

## Collapsed left clock

Render hours above minutes as two upright two-digit lines, centered in the existing left rail below its expand control. Use the configured regional time format; show a small AM/PM marker when applicable. Do not rotate letters or include constantly changing seconds. Retain the existing rail width, expand click behavior, tooltip and keyboard accessibility. The expanded panel retains its existing full clock/calendar. On insufficient rail height, preserve expansion and clock before decorative content. Repaint at the next minute boundary through the existing clock timing path; no new continuous timer.

## Drag overview behavior

This is a visually modal destination chooser owned by the compositor, not an egui modal that steals the drag grab.

1. Detect an actual supported desktop data drag. Immediately begin the overview transition once drag initiation is confirmed. Ordinary pointer movement, divider resizing, text selection and internal shell reordering do not qualify.
2. Keep the source's data offer, pointer grab, serials and negotiated copy/move behavior intact. Render live previews of open destination windows on the current display across its four workspaces, with workspace labels. Show the source for orientation but do not preselect it as a destination.
3. Hovering a destination gives a visible focus outline and a 450ms dwell indication. Moving away resets the dwell. On completion, reveal that actual window/workspace, close the overview, and continue the same drag over the real app surface.
4. Releasing on a preview before dwell completes cancels that drag without delivering a synthetic drop. The overlay must explain briefly: “Hold over a window to open it, then drop inside.” Do not claim a destination accepts the payload before its actual surface negotiates it.
5. After handoff, retain a small dock-edge affordance: dragging back to the dock opens the destination overview again for the same drag. Do not reopen immediately after every handoff.
6. Releasing inside the real destination follows its native drag protocol. Only the target app decides whether to accept and process the content. Unsupported payloads retain normal rejection behavior.
7. Escape cancels the drag and removes the overview without delivering a drop. Source destruction, drag cancellation/completion, display removal, lock and screenshot selection clear state and overlays. No stale preview or pending dwell may activate a later window.

For zero eligible destination windows, leave the normal drag available and show a compact nonblocking “No other windows open” status; do not trap the user in an empty overlay. Window changes during overview update the target list and invalidate dwell for removed targets. Use stable compositor window identity internally rather than an IPC app index that can change. Multi-display handoff resets dwell and rebuilds targets for the pointer's display; no window is moved between displays by this feature.

Files originating in DEs-UI use the existing server drag path. Native Wayland client drags use compositor data-device hooks. Browser tab handling applies only when the browser starts a system data drag: internal tab reordering and browser-specific detach gestures cannot be assumed to expose one. Cross-browser tab transfer is not promised. Xwayland drag initiation, transfer and cancellation require explicit interoperability verification before being described as supported. Unsupported sources retain their existing behavior.

## State and ownership

- `shell/src/dock.rs`: grouping, shelf presentation, keyboard selection, chooser requests and pin actions. Extract focused layout/state helpers if needed rather than expanding one render function indefinitely.
- `shell/src/panels.rs`: minute-granularity collapsed clock presentation using existing regional formatting.
- `compositor/src/overview.rs` and shared preview rendering: reuse packing and live-buffer drawing for dock previews; normal workspace overview and Alt+Tab retain their own modes.
- A focused compositor drag-overview state machine owns source identity, active drag generation, target identity, dwell deadline, display and handoff. `dnd.rs`, data-device handlers and `input.rs` connect lifecycle and routing. Ordinary overview pointer handling must not intercept an active data drag.
- `common/src/ipc.rs`: explicit bounded preview requests/layout state where shell chrome requires it. Treat decoded requests as untrusted: validate target identities, display, lock state, mode and rectangle bounds. A stale chooser request must not activate another window after list reordering.
- Existing shell/compositor crop delivery owns lifted regions, including explicit clears and reconnects. Preview presentation cannot reuse a full-screen input region that blocks the underlying drag destination after handoff.
- No payload contents are copied into shell IPC, persisted or inspected merely to show the overview. Existing data-source transfer remains responsible for payload delivery.

The current empty `ClientDndGrabHandler` and ordinary overview input interception are integration points requiring a local dependency/API check before implementation. Do not replace the active grab with normal overview grabs to approximate the interaction.

## Settings and failure behavior

Extend Settings' existing Dock group with a persistent “Show windows while dragging” switch, enabled by default for supported drag paths. Keep animation governed by existing preferences; do not duplicate an animation switch. A disabled drag-overview switch leaves native drag behavior intact. Use the existing atomic config writer and notification center; a save failure retains the prior active setting. Manual All windows access remains available regardless of the drag preference.

Disconnected or mismatched compositor capability means the dock remains usable with a labelled window list fallback. Do not display blank pretend thumbnails. Failed preview rendering must preserve window activation through that fallback. Older compositor sessions must not advertise automatic drag overview as active.

## Implementation sequence and acceptance

Implement in dependency order: shared state/protocol and dock selection; compositor previews; collapsed clock; drag lifecycle and handoff; settings and cross-mode integration; native visual/interoperability acceptance. Preserve all existing uncommitted project work. No installation, session restart or push is part of this design approval.

Meaningful automated coverage must include grouping/deduplication, hidden-workspace windows, chooser selection and dismissal, stale identity rejection, protocol bounds, narrow layout containment, minute/time-format clock output, dwell entry/reset/handoff, release-before-handoff, successful completion/cancellation, source/target destruction, lock/display loss, reduced motion and failed preference saves. Regress existing overview, Alt+Tab, shortcuts, Files drags and overlay clearing. Run offline locked workspace tests/build and diff checks, reporting sandbox-blocked failures separately.

Native acceptance must inspect wide/narrow displays, bright/dark wallpapers, opaque/frosted surfaces, mouse/keyboard focus, fullscreen, multiple windows/workspaces/displays, empty state and unavailable icons. Exercise Files-to-browser upload, native app-to-app drag, browser tab system drag versus internal reorder, Xwayland sources, early release, Escape and lock during drag. Verify no source payload loss, accidental synthetic drop, stuck grab, stale overlay or idle repaint loop. Until these run, report native acceptance as pending rather than declaring the interaction finished.
