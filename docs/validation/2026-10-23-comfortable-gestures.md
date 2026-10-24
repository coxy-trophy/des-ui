# Comfortable hand gestures

Implemented in the working copy; no binaries installed and no desktop session restarted.

Relaxed pointing, immediate index-pinch click, distinct middle-pinch right-click, default release-pinch dragging and optional conventional grip mode replace the overloaded timed pinch. A second pinch with movement grabs; relaxing the fingers carries the drag and another pinch drops. Two taps without movement remain a double-click. Lost/clipped tracking, fist and incompatible poses release held input; returning pinched cannot click until neutral recognition rearms it.

Mouse & touchpad exposes finite persisted pinch sensitivity, independent scroll speed and drag mode. Reload preserves temporary Super+H choices. Clicks use the visible pointer position, discarding unrendered glide. Both renderers show sharp static accent feedback with stable damage identity and output/DPI coordinates; lock, capture and splash hide it.

Verification:

- Locked offline workspace build passed without warnings (`/tmp/desui-comfort-final-build.log`).
- Workspace tests: 666 passed, nine known environment failures: six socket tests report Operation not permitted and three private-tmux tests cannot start their server (`/tmp/desui-comfort-final-tests.log`). The workspace test command therefore exits unsuccessfully.
- All 13 session tests and six packaging tests passed.
- Gesture regressions include double-click, both drag modes, long holds, tracking loss/reacquisition, jitter and accidental right-click suppression when leaving a held left gesture. Preference parsing, process arguments/reload, narrow/wide settings and feedback timing/DPI checks pass.
- Script syntax and git diff checks passed. Final read-only review found no further material issues.

Native acceptance remains pending: actual camera recognition, physical comfort, small target accuracy, application drag/drop negotiation, reduced motion, multiple displays and rendered visual quality. See docs/UNTESTED.md. Synthetic hands and headless layouts do not establish hardware usability.
