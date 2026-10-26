# Bug fixes and performance — 26 October 2026

Scope: six confirmed desktop defects from the project analysis and installed-machine testing, plus measured rendering overhead. Existing right-panel/dock edits were preserved. No installed binary was replaced; no user application, terminal, compositor or desktop session was restarted.

## Repairs

- Match Smithay's ceiling conversion for logical display dimensions in DRM and nested backends. The 1920×1080 / 175% case now uses 1098×618 consistently, rather than 1097×617 in one path.
- Send bounded, correlated capture failures from the compositor to the shell and waiting control/portal clients. Handle failed grabs, writes, rejected locked/picking requests and full/disconnected command queues. Color-picker errors complete their held replies too. Stale results do not complete new requests or dismiss unrelated local selection.
- Start screenshot-region selection at the original press, retaining the movement that crossed egui's drag threshold.
- Handle hand-helper help and invalid arguments before loading models or accessing a camera. Camera-only errors now return a failing status.
- Accept native egui Copy/Cut/Paste events in Files while retaining text-field, modal and other keyboard ownership.
- Include inactive loaded and installed unloaded user services in the palette. Preserve live records and omit duplicate identities and bare templates.

## Rendering

The installed shell consumed 103–111% of one CPU core in repeated process samples. Hiding/restoring the left panel did not lower that load. Memory pressure was zero during investigation, with approximately 4.6 GiB available; occupied swap alone did not establish active swap thrashing.

Blank terminal cells previously created one text-layout section each: 9,600 sections for a blank 160×60 grid on every repaint. They now create zero sections, while backgrounds, underlined blanks, glyph positions, wide characters, combining characters and cursor rendering remain covered by tests. Empty text rows are not submitted to the font layouter.

The shell continues pumping every PTY, but skips terminal text layout, focus acquisition and cursor blinking when a native application covers it. Explicit commands chosen in the visible Files rail are still delivered once. Terminal/app splits remain visible.

Instrumented debug builds in a private Xvfb preview measured central UI work at roughly 16–26 ms/frame before and 6–8 ms/frame after blank-cell elimination. A paired five-second sample with a simulated covering native app measured UI-thread CPU at 20.0% before and 5.4% after; two-display samples were 15.6% and 10.2%. Total-process CPU remained noisy/high under software OpenGL, so these figures are diagnostic and are not a measured improvement of the installed release session. Temporary profiling instrumentation was removed.

## Verification

All six defects were reproduced with failing regression tests before their fixes. Review additionally reproduced capture chooser ownership and dropped Files-rail input behind a covering app; both received regression tests and repairs.

An isolated native-shell IPC test injected a compositor capture failure. `screenshot-file screen` returned `error fixture capture failure` in 187 ms; `pick-color` returned the same error in 202 ms. A repeat on the clean final debug build returned both errors in 313/304 ms. No real screen image or camera was requested.

`CARGO_BUILD_JOBS=1 nice -n 10 ./scripts/verify.sh` passed: shell-script syntax checks, 77 Python tests (15 session, 6 packaging, 56 companion), 754 Rust unit tests, 3 CLI integration tests, 1 doctest and the locked workspace build. No warnings were emitted. `git diff --check` passed.

The initial optimized release build was interrupted by SIGTERM after verification completed. The retry passed: `CARGO_BUILD_JOBS=1 nice -n 10 cargo build --workspace --locked --release`, exit 0, optimized build completed in 8m 15s. Full-output release CLI smoke checks passed for shell help, hand-helper help and unknown-option rejection. No camera was opened. Exact source and release-binary hashes are recorded in [the evidence JSON](2026-10-26-bugfix-performance.json).

The website’s obsolete keyboard and application-location claims were corrected without changing components or dependencies. Its production build passed.

## Deployment and remaining acceptance

Both compositor and shell must be updated for capture-error propagation. An older compositor cannot emit the new failure event. Install with the existing `install.sh`; its administrator authentication is interactive. The new compositor takes effect at the next login. Managed terminal reconnection is supported, but active sessions were deliberately left running during this work.

Real DRM/GPU performance after activation, fractional-scale capture on the actual monitors, Files shortcuts against native window focus, and the existing hardware/account acceptance backlog remain unverified by these isolated previews. This work does not assert that every possible project bug has been found or that all hardware checklist entries have passed.
