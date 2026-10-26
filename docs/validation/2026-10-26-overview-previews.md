# Workspace overview previews — 26 October 2026

## Repairs and demonstrated causes

The blank native preview was reproduced in the built-in screenshot path. A controlled Chromium Wayland window on a hidden workspace contributed 105,297 green fixture pixels to a live display capture and zero to the built-in screenshot. The offscreen capture path rendered only ordinary Space elements, which exclude hidden workspace windows and do not include the compositor's overview preview layer. Capture now includes that same overview renderer and suppresses the ordinary desktop scene when the overview covers the display. This also applies to recordings using the same capture path.

Workspace content now has 16px padding where space permits before packing previews. The size budget includes the title label; the selected outline has room to expand without crossing the cell border. Geometry tests cover one to twelve previews per workspace at 1920×1080, 1280×900 and 640×480.

The overview ground previously shared Foreground order with separately painted frame outlines. It also used an Area with sizing/fade behavior. It now paints directly into a screen-clipped layer above frame chrome, preserving the shared opaque material and wallpaper. Switcher and window chooser grounds remain bounded to their strips. Tests check first-frame, reopen and resize coverage above a raised frame layer.

## Validation

- `CARGO_BUILD_JOBS=1 nice -n 10 ./scripts/verify.sh` passed: 80 Python tests, 765 Rust unit/integration tests, one doctest and workspace debug build.
- New geometry regression failed before padding; the native capture regression failed before capture composition was repaired.
- `python3 scripts/check-overview-capture.py`: hidden-workspace Wayland at 1920×1080 passed, 90,786 native content pixels.
- `python3 scripts/check-overview-capture.py --x11`: hidden-workspace Xwayland at 1920×1080 passed, 90,786 native content pixels.
- `python3 scripts/check-overview-capture.py --width 1280`: two-row overview passed, 112,968 native content pixels.
- `python3 scripts/check-overview-capture.py --current`: current-workspace overview passed, 90,786 native content pixels.
- Captured wide and narrow native states were inspected. The integration fixture checks app bounds and that the collapsed rail outline does not leak through. It waits boundedly for shell presentation because control replies acknowledge enqueueing, not a Wayland buffer commit.
- Strict design audit and diff checks passed; independent source review found no actionable regression.

The native fixture uses a private Xvfb display, HOME/configuration, Chromium profile, runtime sockets and invalid D-Bus address. It opens only synthetic local content. Raw captures and logs remain in private temporary folders and are not committed. No live compositor, user app or login session was restarted. These are source/debug verification results, not installation into the active DRM desktop. Activate matching shell/compositor builds together at the next login after installation.
