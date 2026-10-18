# Native desktop interaction contract

Authority: the current user request and `docs/superpowers/specs/2026-10-15-desktop-workflows-design.md`; system boundaries come from `common/src/ipc.rs`. This is a native egui desktop, not a browser document. Queries are transient and never placed in URLs or logs. The interface remains English as in the existing shell; user-entered Unicode and IME composition are preserved.

## Canonical owners

| Capability | Owner | Behavior |
|---|---|---|
| Palette | launcher.rs, palette.rs | Super+Space / Apps; typed result kinds; Up/Down, Enter, Escape; explicit Clear; bounded list |
| File lookup | file_search.rs, launcher worker | Explicit selected folders, names only, no symlink traversal, hidden/build exclusions, stale generation rejection |
| Settings navigation | settings.rs::destinations/open_at | One authoritative section list, including Health and Session |
| Feedback | notify.rs::Center | Existing persistent notification history; failures identify operation and recovery |
| Buttons and visual tokens | ui.rs, theme.rs | Existing egui controls / Signal primitives; keyboard focus and disabled states follow native behavior |
| Terminal find | term.rs, term/search.rs | Ctrl+Shift+F or Find; field owns input; Enter/Shift+Enter navigate; IME confirmation never dispatches a terminal command |
| Session persistence | session_restore.rs, workflows.rs | Explicit Save/Restore; only installed desktop IDs, placement, terminal folders/layout; private atomic file |
| Multi-display overlays | screen_views.rs, dock.rs, compositor/toast.rs | Per-viewport state, display-local coordinates, shared pins, no dock while locked or picking a screenshot |
| Document opening | apps.rs::open_document | Shared asynchronous native viewer, exact OS path arguments, failed-exit feedback and bounded pending requests |
| Health | health.rs | Read-only background probes, bounded excerpts, clear missing/unknown/failure states and Refresh |

## Search states

Palette results distinguish applications, windows, settings, actions and files. Running application results focus an existing window; right-click can pin an installed app. File search waits briefly after typing, works in one background worker, rejects stale results and indicates partial/limited results and unreadable folders. At most eight absolute roots, 20,000 visited entries, depth five below each root, 32 results and 400 ms per lookup. Network-mounted filesystems can still stall kernel I/O; the worker never blocks the interface. Folders are saved only by the explicit Save Folders button.

Terminal find scans configured scrollback in frame budgets and supports literal Unicode lowercase matching, soft wraps and wide cells. It stores at most 10,000 matches and 262,144 matched cells, shows truncation, and offers refinement. Searches snapshot existing VT history for stable coordinates. That snapshot can be expensive with unusually large configured history. Closing find returns focus without forwarding its closing-frame keys to the PTY.

## Session behavior

Save overwrites the explicit snapshot in `$XDG_STATE_HOME/des-ui-sessions/session.json` or `~/.local/state/des-ui-sessions/session.json`. Directory/file permissions are 0700/0600. No automatic session launch is enabled. Restore keeps current terminals, reuses matching folders/windows, launches only recognized installed entries and reports missing apps/folders. It never resurrects running terminal processes, replays commands or restores unsaved document contents. Placement waits at most 30 seconds for arriving windows and sends changes in batches so the IPC queue is not flooded. Disconnected displays fall back to an available display.

## Lock, capture and displays

Each display has independent dock animation, pointer corridor, selection and menu state. Pins are shared. Super+D addresses the pointer's display; selecting Terminal/Apps opens the main shell workflow. The compositor uses the selected backdrop surface and its own display origin for rendering/input. Lock hides applications and secondary backdrops immediately. Screenshot picking clears dock keyboard capture and overlays before taking exclusive input; safe clear messages remain accepted without changing screenshot focus.

## Verification boundary

Headless egui tests exercise find and palette typing, selection and IME. Unit tests cover protocol, restore policy/storage, file bounds, health logs and lock/capture policy. Physical DRM outputs, display scaling/rotation, actual IME engines, launching real desktop apps and suspend still require the hardware acceptance matrix. Static UI audits do not certify those flows.

Terminal links: hover an HTTP/HTTPS URL for its destination and Ctrl+click hint; Ctrl+click opens it in the default browser. Plain drag selects text from the original press position; Ctrl+Shift+C copies. Soft-wrapped visible URLs are recognized; other schemes are not opened.

Theme accent: Settings → Appearance owns a draft opaque sRGB colour using the native egui picker, a hex readout, Apply color and Reset to theme. Editing does not write configuration. Apply persists `accent = r, g, b` before changing the active theme; save failure uses the existing notification center and preserves the active colour. The override survives preset changes and restart. Reset removes it. config.rs::resolved_theme combines the preset with the accent at startup and when changed; terminal ANSI colours and fixed interface greys remain controlled by their existing owners.

Processor heat notifications: Settings → Sound & Screen → Notifications owns the “Processor temperature warnings” switch. Config defaults it on; `processor_temperature_warnings = off` persists the opt-out. SysMon filters the typed CPU alarm before posting and clears pending CPU alarms when disabled; readings and other alarm kinds remain active, and historical notifications are retained. Re-enabling resets the CPU latch so a still-hot processor is reported on the next sample.

Browser links: terminal and scratchpad clicks queue HTTP/HTTPS destinations to the desktop’s asynchronous native browser opener. At most eight browser launchers wait at once; their exit failures use the existing notification center without logging URLs. A launcher that remains alive retains a slot until it exits; launching never blocks the UI.

Startup acknowledgement: session_ready.rs owns the one-shot private marker, emitted at the end of the initialized normal or lock UI update only after the compositor state handshake. The session watchdog owns startup deadlines and fallback; it never imposes a time limit on an acknowledged session.

Live terminal reconnect: common::terminal::Runtime owns a private mode0700 tmux server for one compositor lifetime; shell replacement detaches clients. The first GUI holds live_terminals::Lease so a nested/manual GUI cannot attach the same work. The default-on Keep terminals across shell restarts setting affects new terminals only. Existing managed sessions reconnect automatically even after opt-out. Private atomic mode0600 layout.json stores IDs and pane arrangement, never output, commands or credentials; missing/corrupt metadata recovers discoverable live sessions. Dead sessions are not replayed. At most the existing tab limit reconnects, and a failed/slow helper stops the startup recovery batch while leaving live work alone. Scratchpad reconnects hidden. Explicit shell exit in a tab ends its session; compositor logout closes the private backend, including the hardware fast-exit path. Logout/reboot and terminals created before this feature are outside live restoration. Existing saved desktop session snapshots continue to restore folders/layout independently.

Terminal persistence visibility: tab hover states whether it is live or temporary; Settings → Session shows counts. Missing/older compositor backend raises an explicit temporary-terminal notice. Partial recovery preserves the previous layout file. The backend is a foreground compositor child with parent-death SIGKILL; clients cannot start replacement daemons.
