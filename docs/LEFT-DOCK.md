# Collapsed left application dock

New configurations start with the left panel collapsed. Explicit saved panel
preferences remain unchanged. In Collapse mode the rail shows running apps
grouped across workspaces/displays, using the existing dock model and icon cache.
Click a single-window app to activate it, or a group to choose a window.
Right-click also offers Open new window when an installed app is resolved.
The launcher stays at the bottom; overflow uses a bounded window chooser.

The vertical clock and top expand button open the full system panel on click.
The existing panel shortcut also remains available. Hover never expands the
panel: the temporary peek, hover timers and overlay were removed. Ordinary app
tooltips and control feedback remain. The clock follows regional time format
and theme, updates on minute boundaries, and shares the legacy rail painter.

App popups use the existing lifted Dock region. Only one lifted dock surface
is selected at a time; never union disjoint left/right popup rectangles across
native windows. Right Apps and Super+D keep their detailed management workflow.

Regression tests cover full expansion through both controls, app click ownership
and sustained hover over the button, clock, apps and blank space without expansion.

Run `python3 scripts/left-dock-demo.py` in a Wayland session for a disposable
nested desktop with two grouped Thunar windows (requires Thunar and
dbus-run-session). It uses private runtime/config directories and a separate
application bus. Close the preview to clean up its owned processes and state.
It does not install or restart the login session.

Validation after removing hover expansion: full shell suite passed 547 tests
with one ignored; `cargo build -p des-ui --locked -j2` passed. Preview helper
Python compilation and `git diff --check` passed.
