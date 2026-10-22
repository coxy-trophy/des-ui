# Wallpaper and glass appearance design

Status: proposed, researched 2026-10-20. Planning only; no runtime change or installation.

## Direction

Give DEs-UI a softly blurred personal wallpaper, dark frosted work surfaces, and restrained liquid glass on floating navigation. Retain the terminal-first three-panel layout, Chakra Petch headings, JetBrains Mono utility text, accent picker, dotted globe and launcher grid. The signature is a quiet, edge-lit instrument dock above a muted photograph.

The user's current request supersedes DESIGN.md's earlier exclusion of wallpaper and glass. Update that contract when implementing the approved design; do not silently treat this proposal as already shipped.

## Apple research and interpretation

Apple distinguishes Liquid Glass for a floating functional layer from standard materials used within content. Its regular variant adapts for legibility; clear is reserved for appropriate rich backgrounds. Our default should be dark frosted surfaces, with clearer glass restricted to small controls. [Apple HIG: Materials](https://developer.apple.com/design/human-interface-guidelines/materials).

Apple describes lensing, light-responsive interaction and flexible motion. Translate that into restrained edge refraction, a soft rim highlight and a brief response to hover or press. Keep text and icons on a sharp foreground layer. These are our rendering choices inspired by public behavior; Apple's complete optical implementation is not publicly specified. [Meet Liquid Glass, WWDC25](https://developer.apple.com/videos/play/wwdc2025/219/).

Apple's custom-view APIs group nearby glass effects, support coordinated transitions, and caution against excessive effect containers. We should share backdrop captures and limit simultaneous glass regions. SwiftUI's APIs are Apple-platform facilities; this project requires its own Rust/GLES implementation. [Applying Liquid Glass to custom views](https://developer.apple.com/documentation/SwiftUI/Applying-Liquid-Glass-to-custom-views).

## Visual rules

Preserve ground #06080b, panel #0a0e12, line #1a232b, text #dce7ec and secondary #8fa3b0 as fallbacks and tint bases. Preserve Theme.main as the accent owner. Wallpaper must never recolor ANSI terminal output or replace the user's chosen accent.

| Surface | Proposed treatment |
|---|---|
| Desktop background, including secondary displays | Same static image, individually cropped, blurred and dimmed |
| Terminal, system panels, expanded globe | Strong dark frosted fill; sharp text, charts and cursor |
| Settings and launcher | Frosted parent surface; ordinary rows and app tiles, no nested glass on every item |
| Dock, small popovers, navigation controls | Optional liquid edge and responsive highlight |
| Notification cards | Readable frosted material; preserve existing priority and input behavior |
| External applications | Their content and window opacity remain owned by those applications |
| Lock and login | Existing appearance remains separate; never sample an unlocked scene |

Keep the dock's existing 12px radius. Use one shared 12px radius for floating popovers where adopted; retain existing work-panel geometry. Do not turn every panel into a pill. Hover/press light response lasts at most 180ms; material effects never change hit targets or text position. No idle shimmer, wallpaper animation or parallax in this scope. Shape merging is deferred until ordinary glass is visually and operationally proven.

## Settings → Appearance

Place Wallpaper and Surface style before existing accent/palette/phosphor controls. Reuse canonical settings rows, sliders, focus behavior and error reporting.

Wallpaper: None or Choose image; thumbnail and filename; Fill/Fit; blur 0–40 logical px; dim 0–80%; Remove wallpaper. Proposed initial values after selecting the first image: Fill, blur 18px, dim 35%. Fit fills letterboxed space with ground color. One image across displays, independently fitted; per-display images and spanning are deferred.

Surface style: Solid / Frosted / Liquid. Proposed Frosted tint opacity 88%, adjustable 75–100%; terminal fill remains at least 90%. Liquid adds edge refraction to eligible floating controls, initially at a conservative fixed strength. Advanced quality: Auto / Low / High. Auto initially uses the bounded Low implementation; High is explicit until measurements support an automatic upgrade.

Accessibility: Reduce transparency forces opaque materials while retaining wallpaper; Reduce motion disables material movement and respects the existing global animation switch. Neither setting changes external applications. A small local preview is temporary; Apply appearance saves and activates the complete draft atomically, Cancel restores the saved draft. The desktop does not change before Apply. These controls do not alter the existing accent picker's independent Apply behavior.

Existing installations retain Solid and no wallpaper until explicitly changed. Selecting a wallpaper may offer the proposed Frosted preset, but must not silently change a previously chosen style. Reset appearance restores these new controls to Solid/no wallpaper; existing theme and accent have their own reset.

## Existing architecture and ownership

- shell/src/config.rs owns current appearance preferences; settings.rs::appearance emits changes; runtime/application coordination applies them. Preserve this division.
- settings.rs::Faces provides precedent for asynchronous image thumbnails, but its unbounded full decode is not suitable to copy into wallpaper import. Reuse image support and picker conventions, with explicit decode limits.
- shell/src/runtime.rs, panels.rs and screen_views.rs currently paint opaque backgrounds. Wallpaper must be drawn before panel content in every applicable viewport; lowering panel opacity alone cannot reveal it through an opaque CentralPanel.
- compositor/src/phosphor.rs postprocesses the final scene and deliberately excludes those cosmetic effects from captures. It must not become a full-screen backdrop blur.
- compositor/src/toast.rs lifts cropped shell buffers above applications. A shell buffer already flattened over wallpaper cannot provide genuine glass over an app. Live glass requires a separate alpha foreground and compositor material, synchronized with that foreground's surface commit.

New common/src/appearance.rs should own typed, versioned settings and bounds; shell/src/wallpaper.rs should own import/cache loading; shell/src/material.rs should own shared shell material painting. New compositor/src/material.rs should own live backdrop processing only after the feasibility gate below. Avoid duplicating settings parsing between the shell and compositor.

## Image lifecycle

Support static PNG and JPEG initially, using existing dependencies. Explicitly reject animated formats and unsupported inputs. Open as a regular file without blocking the UI; cap encoded size at 32 MiB, dimensions at 8192 per side and decoded RGBA allocation at 64 MiB. Validate headers and decoder allocation limits before full decode. Worker concurrency is one with a replaceable latest request; results carry generation IDs so rapid selections cannot apply stale images.

Import a normalized private copy under the application's XDG data directory, with directory/file permissions 0700/0600 and atomic replacement. Do not depend on the source remaining in Pictures. Reuse existing config atomic-write behavior. A failed decode, copy or configuration write leaves the previous appearance active. Clean abandoned temporary files and superseded managed assets; never delete the source image.

Cache display-sized wallpaper textures and a downsampled blur pyramid; regenerate only on image, blur, scale, transform or output-size changes. Dim/tint can be cheap uniforms. Preserve aspect ratio and clamp sampling at edges. Release caches on output removal. Do not run an image decode or CPU blur per frame. If an imported asset becomes unreadable, fall back to Solid and show one actionable error.

## Rendering stages

1. Wallpaper and Solid: shell-owned background behind existing content, including secondary viewports. Works in an installed session and standalone development window.
2. Frosted: reuse cached blurred wallpaper under shared tinted panel surfaces. This is wallpaper-based frosted glass, not application-aware backdrop glass. Sharp foregrounds remain egui-drawn.
3. Live Liquid: first prove one dock region on both winit and DRM. Capture only scene content below the region, blur a padded crop, apply rounded signed-distance masking and conservative edge-normal UV displacement, then composite the alpha-only shell foreground above it. Sample no foreground from the same material, avoiding feedback/double images. Apply existing phosphor/night-light effects afterward, once.

A genuine live-glass implementation must solve foreground/background separation for lifted shell regions. Evaluate a dedicated transparent shell overlay surface before extending the current crop mechanism; do not remove opaque fills and assume the old path suffices. The feasibility milestone produces a measured prototype and a documented surface/protocol decision before enabling Liquid in Settings.

Material metadata must be bounded, versioned and surface-commit synchronized: output identity, stable region ID, surface generation, logical rectangle, radius and style. Accept it only from the registered shell. Limit to 16 regions per output, reject nonfinite/out-of-bounds geometry, and support an explicit empty snapshot. Reuse the latest-state/coalescing delivery pattern recently added to desktop.rs. An older compositor negotiates no live-glass capability and the shell uses Frosted.

Capture policy: wallpaper and material styling are part of the desktop's appearance and should appear in screenshots/recordings; existing phosphor exclusions remain. Share the canonical scene composition path across screen rendering and capture. Screenshot selection controls remain excluded as today. Lock immediately clears material descriptors, captures and cached application pixels; unlock rebuilds from current scene. A shell reconnect, display removal or lost surface invalidates its material generation and stale regions.

## Performance and fallback

Low uses quarter-resolution backdrop blur and at most 30 updates/sec for changing glass backdrops; High may use half resolution up to output refresh. Use expanded damage to cover blur/refraction padding; a changed source under a glass region invalidates it even if the shell foreground is unchanged. Moving a region damages old and new bounds. Damage-free scenes must not trigger an effect repaint loop.

Cache once per affected output/frame and share textures between eligible regions. Estimate target allocations before allocating; a 64 MiB per-output material-cache budget is the ceiling, with a 128 MiB total budget across outputs. Lower resolution or fall back to Frosted when allocation or shader setup fails. Report an effective fallback once without notification spam. Keep the saved preference so a future session can retry.

Because processor heat has already mattered in this project, measure CPU and GPU time and power behavior against the same scene in Solid. Target less than 2ms additional p95 GPU time at 1080p on the test machine, zero continuous material redraw while idle, and no repeated CPU image work. These are acceptance targets, not current benchmark results. Do not couple rendering quality to the notification opt-out; temperature warnings remain independent.

## Acceptance and exclusions

Verify bright/busy/dark wallpapers, terminal ANSI output, selection, links, IME, less/Vim wheel input, launcher keyboard navigation and settings errors. Require readable small text (4.5:1 where applicable), clear focus and 3:1 control boundaries, with opacity increased when necessary. Validate both blur=0 and max blur, file deletion after import, malformed/oversized images, rapid selection and failed saves.

Installed-session checks must cover app↔terminal switching without leftover regions, shell restart with multiple live terminals, lock/unlock, screenshot/recording, fullscreen/direct-scanout eligibility, mixed DPI, rotation, multiple displays and unplug/replug. Glass invalidates direct scanout only when visible composition requires it. GPU failure keeps the desktop usable. Sandbox/headless tests cannot certify these visual paths.

No video wallpapers, downloads, remote wallpaper URLs, automatic wallpaper rotation, per-app transparency, glass terminal glyphs, login-screen redesign or Apple-exact shader claim in this scope.


## Implementation record — 2026-10-20

Stages 1–2 are implemented locally, with native visual acceptance pending. The cache uses normalized blur strength (UI explicitly labels it), 2560px managed copies and shared 1600px/640px source/soft textures; exact per-output logical-pixel blur is deferred to compositor work. Existing FileChooser support was absent, so this release supplies a bounded local Pictures list and full-path input. Material opacity can increase for conservative small-label contrast. The preview uses the same material painter as the desktop. Future Quality controls remain unavailable alongside Liquid. These choices and their costs are recorded in the validation document and execution ledger. Stages 3–4 remain gated; this is not an implemented Apple-equivalent material renderer.
