# Cursor color and size

Implemented in the working copy; no installation, logout or running-session restart occurred.

Settings → Mouse & touchpad now exposes size16–64px (default24), an opaque RGB draft picker, Apply/Reset color, Reset size, and light/dark previews using shared vector geometry. Preferences persist through InputSettings. Native named cursors update on ReloadInput, preserving the current morph and exact scaled click anchor. Dark fills retain contrasting outlines. Reload delivery shares the compositor reconnect retry path.

Legacy cursor publication runs on a bounded latest-request worker. It publishes complete private cache generations atomically, preserves the last good theme on failure, retries failed preferences on later reload, and retains bounded prior generations. Startup prepares matching fallback artwork before clients launch. Configured images carry1×–4× density tags for the inherited nominal request, with cropped padding/adjusted hotspots and linked aliases. Installer defaults remain separate. Older clients may need reopening because they cache cursor images; custom app artwork remains respected.

Verification:

- Locked offline workspace build passed without warnings.
- Final workspace tests, limited to two test threads: 703 passed, ten failed. Nine are the known Unix-socket/private-tmux sandbox failures. The pre-existing terminal `stalled_ui_backpressures_noisy_shell` test additionally exceeded its backpressure-release deadline, also reproduced in isolation. Terminal source was not changed; its timeout remains unresolved and is not counted as an environment skip.
- All15 session tests and six packaging tests passed. Twenty shell scripts passed syntax checks; git diff check passed.
- Focused checks cover config parsing, draft/apply/reset, narrow layouts, black preview contrast, exact size anchors, RGB/ARGB validity, morph/cache continuity, worker coalescing/retry, cache trust boundaries, atomic failure recovery, repeated exports and HiDPI metadata.
- System libXcursor loaded configured40px artwork in two custom colors, preserved logical bounds/hotspots at1×/2×/3×/4×, and accepted repeat exports to the same directory. Debug exports took2.09s and2.10s.
- Raster optimization retains exact reference pixels and four-sample antialiasing. A same-process debug64px-arrow4× sample improved from1.874s to63.1ms; this is a focused raster measurement, not a hardware compositor performance claim.
- Final review found no further material feature issues after the HiDPI and repeat-export corrections.

Logs: /tmp/desui-cursor-settings-final-tests.log, /tmp/desui-cursor-settings-final-build.log, /tmp/desui-cursor-settings-final-session.log, /tmp/desui-cursor-settings-final-packaging.log, /tmp/desui-cursor-settings-terminal-check.log, /tmp/desui-cursor-settings-libxcursor.log.

Native installed-session behavior and actual desktop appearance remain open in docs/UNTESTED.md. The user cannot log out now.
