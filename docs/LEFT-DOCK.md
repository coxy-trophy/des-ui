# Collapsed left application dock

The left panel defaults to collapsed in new configurations. Existing explicit
`panels` preferences remain unchanged. In Collapse mode its rail shows grouped
running applications across workspaces/displays. A single-window app activates
on click; a group opens a window chooser. Right-click opens the same window list
and an Open new window action when its installed application can be resolved.
The launcher remains available at the bottom. Overflow uses a bounded chooser.

Hovering an app tile, launcher or overflow control never arms system expansion.
Unused rail space and the top expand control open the monitoring panel after
220ms. The panel stays open while the pointer is inside it, and closes after a
250ms grace period when leaving or entering an app tile. The hover panel appears
beside the fixed dock as an overlay, so native windows are not resized. Clicking
the top control retains the existing explicit full-panel expansion behavior.

The existing dock grouping, application discovery and bounded shared icon cache
are reused. Hover timers repaint only while pending; there is no new polling
worker or persistent animation. Right-side Apps and Super+D retain their existing
management and keyboard behavior.

The peek and menus use the compositor's existing lifted Dock region. Only one
lifted dock surface is selected at a time: disjoint left/right rectangles must
never be combined into a rectangle covering the native workspace. Lock, capture
picking and modal surfaces suppress the peek.

Regression coverage includes icon exclusion, delayed opening, panel retention,
leave grace, modal reset, grouped icon hit ownership and preserving saved panel
preferences. Existing dock/window-management and frame tests remain applicable.

## Validation and preview

`cargo check -p des-ui --locked -j2` and `cargo build -p des-ui --locked -j2`
passed. The full shell suite with `--test-threads=2` passed 550 tests with one
ignored. An initial parallel run hit the existing remote descendant-timeout
fixture before its pidfile was written; its isolated rerun and full rerun passed.

Run `python3 scripts/left-dock-demo.py` from a running Wayland desktop to open
a disposable nested compositor with two real Thunar windows. This requires
Thunar and dbus-run-session. It uses private runtime/config directories and a
separate application bus, and does not install or restart the login session.
Close the preview window to stop its processes and remove temporary state.
The native preview confirmed two real windows grouped in the collapsed dock;
hover exclusion/delay are covered by egui/state tests. Interactive hover over
native windows still needs visual confirmation in the preview.
