# App dock and installed icons

Requested behavior: retain existing terminal-first desktop and three-panel frame. Move graphical application switching from top tabs into an always-autohiding bottom dock. Terminal tabs and workspace controls stay at the top. Use installed application icons in launcher and dock.

Dock overlays existing content without changing workspace geometry. It grows with its icons, wrapping at the available screen width without a dock scrollbar. A four-logical-pixel bottom-edge trigger reveals it, including over full-screen applications. It rests above the status strip, slides vertically with a short eased transition, stays open across the edge-to-dock corridor and its context menu, and hides after pointer exit. Locked sessions suppress rendering and input.

A persistent terminal shortcut and launcher shortcut surround pinned applications and grouped running applications. Click a running group to activate a window; context menu lists windows and offers open another, close, pin/unpin. Pin by desktop entry ID, never by a changing window title. Missing desktop entries keep removable pins with a generic icon and unavailable state. Running windows use Wayland app_id or X11 class matched against desktop ID, StartupWMClass or executable. Unknown apps remain switchable with generic icons.

Icons resolve from desktop Icon metadata, absolute files, XDG icon roots and pixmaps; decode and rasterization happen off the UI thread with bounded caching. Invalid or missing icons use the same generic application symbol in launcher and dock. Pin saves use existing atomic config writer and report errors in the notification center.

Shared geometry and protocol tests cover edge activation, app identity framing, independent dock overlay and lock suppression. Shell tests cover grouped windows, pin ordering/deduplication/persistence, timed reveal/hide, edge corridor and interaction. Run full workspace tests/build, preserve existing uncommitted work. Native display/socket access is sandbox-blocked, so report that runtime limit explicitly.
