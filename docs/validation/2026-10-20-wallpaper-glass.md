# Wallpaper and Frosted verification — 2026-10-20

## Scope

Implemented locally on feature/wallpaper-glass: typed/versioned appearance settings; private bounded static-image import; local Pictures browser/path chooser; temporary preview; atomic Apply/Cancel/Reset; Fill/Fit, blur strength, dim and opacity; Solid/Frosted; Reduce transparency and Reduce motion. Shared wallpaper-backed backgrounds cover panels, terminal, dock, notifications and secondary displays. Existing ANSI backgrounds and all foreground/input behavior remain owned by their current widgets. No installation, shell restart, logout or preference change was performed.

Liquid is unavailable until the compositor feasibility gate passes. See 2026-10-20-glass-prototype.md. There is no app-aware refraction, live application sampling or new material IPC in this change. Quality controls belong to that later stage.

## Automated evidence

`cargo test --workspace --locked --offline --no-fail-fast -j2`: **541 passed, 9 failed, 0 ignored**. All **19 new appearance tests pass**, including upright JPEG/EXIF handling, animated/oversized input rejection, pointer/keyboard drafts, both review regressions, cancellation, image persistence, contrast and compositor reduced motion. Log: `/tmp/desui-wallpaper-verified-tests.log`. Full-suite exit is 101 because the nine sandbox integration checks fail. Earlier full workspace runs passed all new appearance tests and reproduced nine existing sandbox integration failures: two compositor IPC sockets; shell control sockets (two), desktop transport, greeter socket; three tmux-backed terminal tests. Failures report permission denied / private server startup in this environment. They are not counted as passing.

Focused red/green evidence: shared settings (defaults, safe path round trip, malformed values); wallpaper decode/import/geometry; cancelled result cleanup; compositor reduced motion; import/blur generation; post-rename directory-sync error reconciliation. Headless editor tests cover pointer and Tab/Enter Apply/Cancel and draft-only reset. Material tests cover opacity floors, opaque missing-image fallback and conservative 4.5:1 small-label contrast over a white background.

One independent read-only review found the import/blur race and post-rename asset-ownership edge; both have regression coverage and fixes. Shared preview painting, duplicate secondary background drawing and keyboard coverage were addressed too.

`cargo build --workspace --locked --offline -j2`: **passed without warnings**. Log: `/tmp/desui-wallpaper-verified-build.log`. `git diff --check`: passed. Rust-source SHA-256 snapshot matched after verification (`/tmp/desui-wallpaper-source.json`).

## Native boundary

An attempted temporary Xvfb display failed to bind local sockets. No native screenshot, shader behavior, installed DRM output, actual graphics-memory/GPU-time measurement or hardware accessibility result is claimed. Run the wallpaper section in docs/UNTESTED.md on the installed desktop before calling the visual paths verified.

Keep particular attention on bright/busy backgrounds, terminal/app switches without stale lifted rectangles, screenshots/recordings, lock, shell restart with live tabs, rotated/mixed-scale displays, hotplug and fullscreen apps. Native frame-time/power comparison must use the same scene and machine across Solid/Frosted; this change adds no independent idle material animation or per-frame image processing.

## Implementation decisions

- Blur is normalized visual strength, not an exact output-pixel radius. Cached textures are shared across displays with independent Fill/Fit mapping.
- Imported copies normalize to 2560px; displayed source/soft textures cap at 1600px/640px. Large unblurred displays can appear softer than the original.
- Local Pictures listing and a full-path field replace a system file dialog; there was no existing FileChooser owner to reuse. PNG and JPEG only.
- Effective opacity may exceed the requested value for text contrast; the editor explains this. Preview and desktop use the same painter.
- A post-rename durability warning retains both new and previous managed images, so either config reference survives possible crash rollback. Ordinary successful replacements remove only obsolete managed copies.
- No new commits or remote pushes are part of this implementation turn; prior pending fixes remain in the feature branch's working tree.
