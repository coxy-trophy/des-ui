# Native desktop interaction contract

Authority: the current user request and `docs/superpowers/specs/2026-10-15-desktop-workflows-design.md`; system boundaries come from `common/src/ipc.rs`. This is a native egui desktop, not a browser document. Queries are transient and never placed in URLs or logs. The interface remains English as in the existing shell; user-entered Unicode and IME composition are preserved.

## Canonical owners

| Capability | Owner | Behavior |
|---|---|---|
| Launcher / palette | launcher.rs, palette.rs, app_icons.rs | Super+Space / Apps; responsive app grid; typed result kinds; arrows, Enter, Escape; explicit Clear; bounded results |
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

World View: globe::Explorer owns one ephemeral camera, zoom and pinned Place shared
by panel and workspace. Drag rotates, hovered scroll zooms between 1× and 2.5×,
and marker hover reveals location with its arc highlighted. Picking a visible
marker or destination row focuses that place and pauses automatic rotation. HOME
resets orientation/zoom and clears selection. RESUME clears the pin. Hovering the
globe or expanded workspace pauses ambient movement; automatic rotation requires
both animation and globe rotation settings. Controls provide non-drag alternatives.
Double-clicking the compact globe brings the terminal workspace forward, closes sibling workflows, and focuses
the labelled local destination search. CLEAR immediately resets search and returns
focus to it. CLOSE/Escape return to the existing terminal; switching workflows closes
the explorer. Lock prevents rendering/input. Selection survives disappearing links
and is explicitly marked no longer current; IP locations remain approximate.
Lookup-off, unavailable and no-match states give inline directions. No extra network
requests, packet simulation, traffic attribution, saved camera state or configuration
writes are introduced. Motion uses the existing approximately 15 Hz idle globe cadence,
with no animation repaint loop while paused or reduced motion is selected.

Terminal creation resilience: a failed managed New Tab falls back to a fresh direct
interactive shell in the requested working directory, with a Temporary terminal
opened notice that states it closes on shell restart. Existing sessions remain
untouched. Explicit terminal command launches and reconnecting sessions do not
use this fallback, so commands/work are never replayed. tmux helper metadata uses
printable pipe separators and clients explicitly request UTF-8, avoiding locale
sanitization of tab separators and captured history.

Launcher grid: blank search shows apps; typing preserves all universal search
providers and ranking. Tiles use installed icons with the canonical fallback,
centered names of at most two lines and full-name/detail tooltips. Non-app search
results retain a compact kind label and accessible kind/detail metadata. No
executable paths, verbose provider explanations or APP badges in the browse grid.
Grid columns adapt to width (one on narrow workspaces), virtual scrolling keeps
rendering bounded, and keyboard selection scrolls by grid row. Left/Right edit
the search caret until Up/Down starts grid navigation; query changes or clicking
search return to editing. Enter opens the selection; Escape closes. Tab-focused
tiles update selection. Existing app pin, clipboard and unit menus are retained.
Search folders and close use shared labelled icon controls; Clear is always
allocated, enabled for non-empty text and restores search focus. File-search
progress/error text has reserved height to keep tiles stable.

Managed terminal scrolling: the private tmux server enables mouse reporting. Root
wheel bindings forward mouse events to mouse-aware programs or an existing copy
mode, send Up/Down to alternate-screen programs without mouse support, and enter
backend history at a normal prompt. The private copy-mode table scrolls one line
per wheel tick; Escape cancels, and other keys cancel then re-inject the original
key so typing is not swallowed. Configure this before a new pane and during
reattachment in the existing bounded dimension-query queue. Direct PTY wheel
routing and the globe's hovered-only zoom remain unchanged.

Lifted-region delivery: Desktop retains the latest Toast/Overlay rectangles,
including explicit clears, in coalesced state slots outside the bounded window
action queue. The worker retries a full wire buffer and re-enqueues current state
after a ready handshake/reconnect. Queued geometry advances only after successful
enqueue; locked or pending-unlock state holds presentation until acknowledgement.
Window-index actions remain bounded and are never replayed across disconnection.
This prevents stale rectangular copies of the shell remaining above an app after
switching away from the terminal.


Wallpaper and surface style: common::appearance::AppearanceSettings owns a versioned single-value config entry, preserving arbitrary Unix path bytes through hex encoding. Old/malformed configurations use no wallpaper/Solid. Settings::appearance_editor owns a local draft; Apply appearance commits it once through config::save_appearance before changing active state, and save errors use the existing notification center. A post-rename directory-sync failure is reconciled against the saved config, retains its managed asset and reports a durability warning. An import completion retains its rendered blur generation so edits made in flight trigger a fresh preview before Apply. Cancel changes, closing Settings and reopening discard unapplied edits. Reset appearance resets only new appearance controls. Choosing a wallpaper selects the Frosted preset when the previous style is Solid, so its backdrop is immediately visible in preview and after Apply. This is part of the wallpaper action; the user may explicitly select Opaque afterward. Version 2 persistence preserves that explicit choice; version 1 wallpaper-plus-Solid defaults migrate to Frosted with a 62% tint, at least 18 blur and at most 35% dim. Reduce transparency continues to override surface effects.

wallpaper.rs owns one background image worker, a latest request per client and bounded pending work. Local PNG/JPEG imports reject animated/invalid/oversized inputs, make a private normalized managed copy, and remove cancelled assets. Source images are never modified/deleted. Saved imported copies survive moving/deleting the source. No per-frame file reads, decodes or blur. A failed active image load gives one actionable notice and Solid fallback. Main and secondary shell viewports share cached textures but compute independent Fill/Fit geometry.

material.rs owns sharp-foreground wallpaper-based frosted surfaces for panels, terminal, dock and notifications. Existing input, focus, terminal mouse/wheel handling, overlay bounds and lock ownership remain unchanged. Reduce transparency forces opaque work/control surfaces; Reduce motion combines with the existing animate switch. Live application-aware glass is unavailable in this build; existing compositor sampling/capture paths are unchanged. Native lock, output switching/scaling, screenshots and installed-session verification remain required.


Files: files::Browser owns folder/history/filter/selection state. One bounded shared directory worker resolves paths and lists entries (10,000 entry and 500ms traversal bounds); generations reject stale results. Errors preserve the current listing. The image picker shares navigation but permits only PNG/JPEG selection. Closing or hiding Settings dismisses its picker without applying its draft. File browsing consumes keyboard input only while it owns the workspace; scratchpad, authentication and quick settings take precedence.

File operations: the application owns a single background Task across browser closure. Create folder and rename use named dialogs; Move to Trash requires confirmation. Copy/Cut set an in-app single-item clipboard; Paste copies/moves into the current directory. Existing destinations are never overwritten. Copies use private staging and atomic publication, preserve symlinks without following them, and clean staging on error/cancellation. Cancellation is cooperative and cannot undo a completed atomic commit. Cross-filesystem Move/Trash returns an error preserving the source. Copy does not preserve all metadata (xattrs, times and hard-link identity).

Trash uses the home XDG Trash with private metadata and original-name display; Restore requires its original parent and an unoccupied destination. Trash storage and directories containing it cannot be trashed. Trash view disables operations which would invalidate recovery metadata. No permanent deletion, multi-selection or per-volume Trash is implemented. Failed operations retain dialog input and report errors; completed operations refresh the current listing and report through existing notifications.


Terminal wallpaper continuity: material.rs owns the whole terminal workspace, including padding/split gaps, per-pane background and gutter. All sample one screen-aligned cached backdrop. Default terminal-cell backgrounds are transparent; explicit ANSI/application backgrounds and reverse video remain opaque as required by terminal semantics. Blur affects only wallpaper. No framebuffer blur is applied to foreground text. Normal bright text uses a separate contrast floor from faint chrome labels, which retain denser control surfaces. Visual acceptance requires native screenshots, not just painter-state unit tests.


Desktop consistency pass: shared native style uses display typography for UI copy, 8px spacing and 30px interaction height, while explicit paths/readings and terminal cells remain mono. Custom controls have disabled treatment, hover/pressed states and visible keyboard focus. Settings heading/description rows wrap and expand vertically; native field focus/persistence behavior is retained. Files keeps New folder and Paste visible and places Rename/Copy/Cut/Trash/Restore under More with existing shortcuts; picker mode never exposes mutation actions. Image/file identity remains native PathBuf and display metadata never replaces it.

Shared sliders consume adjustment keys only when focused and enabled, preserve unfocused terminal/navigation input, clamp values to their range and expose semantic accessible labels. Existing application actions remain their owners: no direct mixer, brightness, seek or settings side effects are introduced in the shared painter. Authentication/login remain opaque and preserve secure-focus ownership.

Files menu ownership: an open or newly activated More menu owns Arrow/Enter and operation shortcuts before the list. Opening Rename/Trash cannot also activate the selected document with the same Enter. Escape dismisses the menu before the workspace; scratchpad and Quick Settings retain precedence over hidden Files controls.


Keyboard mapping contract: common::shortcuts::Bindings owns one versioned config value (`shortcuts = v1;id:Chord;…`), storing overrides and empty values for disabled actions. Unknown/malformed/conflicting data falls back atomically to defaults. Shell and full config parsing use the last shortcuts entry consistently. Standard copy/paste/interrupt, lock, recovery, hardware and fixed application-local controls remain protected or explicitly contextual. Desktop shortcuts require Super or the development Ctrl+Alt alias; shell/terminal shortcuts require Ctrl. Conflicts include effective Ctrl+Alt aliases. The existing split-focus/terminal-prompt Up/Down pairs are deliberate context variants; other conflicting mappings are rejected.

Settings emits a proposed configuration and applies it only after atomic persistence succeeds. A post-rename directory-sync error reconciles the committed file and reports durability uncertainty. Failed writes preserve active mappings. The shell updates its cache immediately; compositor reload requests retry after disconnect/queue pressure. No shell/session restart is needed for live remapping once matching binaries are installed. Global compositor support requires the updated compositor binary, as well as the shell.

The reference is modal: background raw events are held until the reference renders, preventing launcher/file-panel handlers from consuming its Escape/Arrow/Enter. It lifts the whole display above applications, preserves their workspace/process state, and closes for lock/authentication. Scratchpad focus is suspended while the reference is shown. Configure opens Settings → Shortcuts; Escape cancels reference, recording, or pending reset before closing its parent. Hidden editors never retain shortcut capture. Manual draft entry and explicit recording are separate modes; recording cannot fire shell shortcuts behind Settings.


Window shelf: dock.rs owns grouped application selection, height-budgeted pages and stable keyboard focus; compositor overview.rs owns stable window identities, nine-preview pages and actual activation. Escape returns from the chooser to its dock item. Capability negotiation selects previews or the native list fallback. Regional formatting owns the collapsed rail clock.

Drag destinations: compositor dnd.rs owns session, cancellation, 450ms dwell and real-surface handoff. Client callbacks only record state; frame/input processing handles pointer operations outside grab callbacks. Lock, capture, output changes and dead sources cancel safely. Native offers retain copy/move negotiation; previews never accept a drop. Returning to the bottom edge reopens destinations. Files emits outbound drags only from its normal browser; the wallpaper picker and Trash do not. The persisted drag_overview preference disables presentation while preserving ordinary native dragging.


Comfortable hand control: hands::gestures owns palm-relative relaxed pointing and debounced index/middle taps. Default index tap clicks immediately; a second pinch and movement grabs, relaxed fingers keep carrying, and another pinch drops. Two taps without movement remain a double-click. With release-pinch dragging off, index pinch holds the left button until release. Middle pinch right-clicks without timed holding. Fist, lost/clipped tracking and incompatible poses release held input; neutral recognition rearms after returning. Clicks use the visible pointer position rather than finishing an unseen camera glide.

InputSettings owns finite normalized pinch/scroll preferences and drag mode. Reload releases process-held buttons before starting with matching preferences and preserves temporary Super+H on/off choices. Hand feedback is compositor-owned, theme-colored, static and noninteractive, with stable damage identities on both backends. Hardware comfort and recognition quality still require user camera testing.


Pointer ownership: compositor handlers/cursor_shape.rs owns semantic routing, geometry, morphs and the bounded raster cache. DesComp shares that cache across display, capture and both backends; udev::effective_cursor_status owns the divider resize override for all consumers. Named shape morphs keep a fixed logical hotspot and retarget from the current silhouette; opposite directions retain visible area. Hidden and client-supplied cursor surfaces suspend the compositor morph. Dead cursor surfaces revert to default before any backend draws. The development window hides its host cursor and draws its desktop cursor.

Legacy theme: cursor_theme.rs exports valid DEs-UI Xcursor files at 24/32/48 sizes from the same geometry. Session and dev launchers set theme/search-path defaults while preserving explicit cursor environment choices. Installation and Debian packaging include the fallback theme; uninstall removes its installed directory. Legacy images are static and application custom bitmaps can override them. No client image pixels are inspected or replaced to guess semantics.


Cursor preference ownership: InputSettings persists bounded cursor_size and six-digit RGB cursor_color. Cursor-shape Preferences normalizes them; configure invalidates cached pixels only when appearance changes and preserves the current morph and exact scaled click anchor. Settings color is an explicit draft; Apply and Reset use persisted readback. Saved cursor changes share the existing ReloadInput reconnect retry path.

Configured legacy theme updates are coalesced off the render thread into private user cache generations. A complete theme becomes visible through atomic publication; failure retains the last usable generation. Configured theme density variants account for the inherited legacy nominal request and output buffer scale so logical size stays consistent. Package/installer defaults remain separate. Startup prepares the configured theme before shell/Xwayland clients load it; session/dev launchers include its private cache path for DEs-UI. Older applications may cache cursor images until reopened; app-supplied custom artwork remains outside these controls.

Cursor companion: Settings → Companion opens an anchored native conversation modal. Super+J / Ctrl+Shift+J open text; Super+Shift+J / Ctrl+Shift+U summon voice, all configurable. Only an enabled companion with a runtime/environment API key can request OpenAI. Keys never enter ordinary configuration or SQLite, and changing models preserves an untouched entered key. Opening a surface alone never requests social text before an explicit voice start. The Python worker uses owned pipes and bounded queues; queue failure tears it down so closing cannot leave recording active. Audio children have Linux parent-death protection.

Voice starts after the provider startup acknowledgement, uses visible countdown/mute/keep/stop controls, and stops locally at300seconds independently of UI, network or callback progress. Keep never extends this deadline. Quick mode uses PCM inactivity and estimated playback duration; it is not a provider turn-completed signal. Disconnected sessions have no wake listener. No reconnect occurs without explicit resume. Final usage is cumulative; missing authoritative terminal usage retains the conservative five-minute daily allowance. Provider billing finalization is shown separately from local microphone shutdown. Typed Send is disabled while voice is active. Accepted transcript fragments retain spacing and turn boundaries and persist into the shared history.

The private0700/0600 SQLite store separates conversation, approved facts, versioned identity, non-secret settings and daily counters. Requests select bounded recent history and approved memories. Identity cannot change permission or tools; no desktop execution/context-reading tools exist. Clear conversation and Reset identity are separately confirmed; Forget memory does not erase the same fact from a conversation. An unanswered social approach survives shell restart and suppresses repeated approaches until the user replies or clears conversation. Social opportunities are locally throttled and suppressed under lock, capture, authentication, app focus/covering, drags or other overlays. Quiet disables social initiative. Spoken Keep talking recognition and local wake detection are deferred; use the explicit controls.

Companion setup revision: Settings → Companion is the sole configuration owner, with visible key-presence status (distinct from verified provider access), safe actionable text/voice diagnostics, and explicit worker reconnect. Worker teardown/reconnect clears stale listening/busy/key-available state, preserving stored conversation while allowing setup again. The voice transport allows10seconds for TLS/WebSocket handshake, then polls established reads at1second beneath the independent15second startup watchdog. A failure before attempting session.start releases its local voice reservation; any ambiguous send/later loss retains the conservative reservation until confirmed final usage. No automatic voice reconnect or paid diagnostic probe is introduced.

Thought bubbles use a once-captured pointer anchor, bounded15second lifetime, cubic fade and subtle trail rise. Their hit rectangle is stationary throughout the animation; reduced motion has no animation. Shortcut defaults: Super+Ctrl+J Keep, Super+Alt+J Mute, Super+Ctrl+Shift+J Stop, Super+Alt+Shift+J Quiet. They share typed IPC controls and the existing shortcut editor/conflict checks. Quiet disables social; re-enable it explicitly in Settings.

Companion panel revision2026-10-24: Super+J reveals the bottom panel and selects/focuses Chat; Files/Chat are peer tabs. No floating chat modal is drawn. Enter submits and retains editor focus; Escape releases keyboard with draft/tab preserved. Chat keyboard capture lifts only its bottom rectangle. CompanionChat marker scopes outside native-app click dismissal, emitting CompanionBlur before app focus routing; other overlays retain their existing policy. Changing tabs/apps does not stop explicit voice; lock, capture/authentication and Stop still do.

Voice default is now Super+Ctrl+J; Keep uses Super+Shift+J (all registry configurable). Voice startup waits for initial saved-key restoration, starts once, and Stop revokes a pending request before autoload completes. Voice errors have a brief pointer-transparent thought notice. Speaking state derives from queued output PCM and ends when playback drains, independently of microphone mute. Shell sends bounded500ms visual heartbeats only while indicators are present; compositor expires them after2seconds and clears on lock/capture/IPC loss. Caption crops are excluded from all pointer/keyboard input regions. Pointer anchors come only from explicit voice invocation or active-indicator heartbeats; no idle pointer tracking is added.

Remember API key securely defaults on. libsecret operations run on one bounded background worker with cancellation/timeouts; state exposes storage status without key contents. Apply saves to Secret Service; new workers restore without paid requests or microphone activity. Environment key retains precedence. Remove clears saved/runtime key (explicit environment fallback remains), disables autoload and ignores stale queued results. Keys never enter ordinary config, conversation, SQLite or logs. Missing/locked keyrings surface safe errors, never a plaintext fallback.

## Right-panel applications —2026-10-25

System is the default right-panel tab; Apps remains available on demand. No bottom app strip or reserved dock band remains. Apps groups running windows across workspaces/displays and exposes activation, launch, close, pin ordering and window workspace controls. Super+D opens and focuses Apps. Escape/activation/outside native clicks release keyboard ownership without hiding the normal panel. Secondary displays show a transient right rail for keyboard switching. Existing Dock IPC names are retained internally for overlay crop and explicit keyboard ownership.


Separated frame geometry: common::frame::Layout is the canonical source for primary side panels/rails, center/header, Files/Chat and the inset native workspace. The compositor uses exactly its workspace rectangle; the shell uses all surface rectangles. New shell/compositor builds must be activated together for geometry changes. Fullscreen and secondary outputs retain their existing ownership. Initial right-panel selection is System; Super+D still explicitly selects Apps and handles keyboard/focus as before. No persistent tab-selection field is introduced.

The on-screen keyboard types into shell controls and terminals. A bounded keyboard-height IPC reservation now keeps native application placement clear of the center-column keyboard card; it does not add virtual-keyboard typing into native applications. The same rounded height and canonical frame layout are used on both sides, including reconnect/lock reset.

Collapsed side cards keep a 44px-wide click target, an 8px channel beside the center, and a whole rotated regional clock on the left. Its glyphs face right and HH:MM reads from bottom to top. Shared frame geometry owns the 56px rail reservation in shell and compositor.

Overview presentation: the compositor owns preview geometry, window activation and drag input; shell paint has no independent hit targets or fade/sizing delay. The full-screen ground sits above frame chrome. Captures render native overview elements before the shell background, including hidden-workspace apps; ordinary Space rendering alone is insufficient. Thumbnail content is inset 16px where space permits, with labels and selection borders remaining inside workspace cells.

Music Home is a presentation of actual library playlists; collection artwork, loading, unavailable and sign-in states reflect the existing service. Full music content has a bounded body and reserved transport; compact playback has a 136px allocation. Side-panel playback and local readings cannot be displaced by the globe, display tiles or custom widgets. Left telemetry yields to the selected pet's fixed footer. Optional instruments/custom widgets appear only where they fit; there is no side-panel scrolling.

Applications: empty launcher search includes every installed app; typed palette results remain bounded. The launcher, shelf and tray share cached textures and background source invalidation; opening a surface does not clear artwork. Right-panel paging includes expanded window rows. Page buttons and keyboard selection move together so Enter always targets a visible row. Installed-app scans, icon resolution and decoding stay off the UI thread.

Devices: initialize the inventory worker while Settings is closed; initial enumeration is quiet, subsequent hardware/status changes use existing notifications. Show cameras, audio endpoints, USB, input, storage, displays and connected/paired Bluetooth with honest status and stale-service handling. Inventory reads metadata and service snapshots; it never streams a camera/microphone, pairs devices or mounts media. Hand control uses a privately saved physical camera identity, resolves current video numbering only when enabled, and fails closed if that choice is missing. Camera changes take effect when hand control is restarted; other applications retain their own camera choices.

Settings uses five category groups and one search catalog for names, descriptions and synonyms. Search never switches pages while typing; choosing a result clears the query and opens that existing section. Escape clears a nonempty search before closing Settings. Legacy uppercase section IDs and external settings destinations remain supported. Current section selection survives clicking its already active category. Cursor previews show the actual filled/shadowed style.

Display reconnect revalidates advertised modes and physical dimensions before reusing outputs, while keeping valid current configurations. Mirrored outputs preserve primary client scale ownership instead of arbitrary output iteration; independent-output movement follows overlap. Cursor export forwards a strict theme/size/path/HOME/XDG allowlist into Xwayland, honors explicit alternate themes and custom cursor surfaces, and includes standard legacy aliases. No live monitor reconfiguration or application restart is implied by source verification.

System panel spacing: Network and World View use only their painted height. Displays and local Controls follow immediately with an 8px section gap; unused height must not stretch the globe reservation into a blank middle band. Playback retains its protected bottom allocation, and the same height budget still prevents clipping on short panels.


## Spotify library extensions

The music service owns Spotify requests; the shell sends validated JSON commands.
Library writes and saved checks use URI-based `/me/library` endpoints. The music
volume is independent of system output volume and is visible in both players.
Browse library owns history, top items/ranges, followed artists and spoken content;
Library actions owns refresh, permission approval and playlist creation. Collection
pages expose save/follow and permitted playlist editing. Row context menus own
playlist insertion/removal/reordering. Removal matches all occurrences of the URI;
reordering preserves raw playlist positions even when unavailable entries are hidden.
Cover selection reuses native Files; encoding is a bounded JPEG before upload.
Mutations invalidate subscribers' cached lists only after Spotify confirms success.
Pagination uses API offsets/cursors, not the number of displayed rows. Chapter
playback opens Spotify; it must not claim native audiobook playback. New OAuth
permissions need explicit approval on Spotify's page. Restricted Spotify endpoints
remain unavailable. Native preview fixtures must never start a music service or use
account credentials.

Spotify bug-review refinements: playlist pages retain their own snapshot IDs;
version changes during/between fetches require refresh rather than mixing positions.
Visible row moves use neighbours' raw positions. Playlist details/privacy editing
belongs to owners; collaborators retain item editing. Empty filtered pages retain
Load more, and in-flight requests are keyed by server offset. Refresh invalidates
saved/permission caches. Form submissions carry distinct operation IDs, retain drafts
on errors and recover after missing confirmations. Canonical Spotify URLs own chapter
opening (chapters can use episode URIs); launch failures are surfaced in the music UI.

Terminal local completion uses a small built-in list and is shown only at a
single-line bash or zsh prompt identified by shell integration and a matching
foreground process. Full-screen programs, commands in progress, and unverified
prompts retain their normal input. Tab or Enter inserts the chosen suffix without
running it; Escape dismisses it, and the Terminal settings can turn it off.
Right-clicking a completed command with retained output offers a local copy action.
Neither feature sends terminal text to a network service or saves separate history.
