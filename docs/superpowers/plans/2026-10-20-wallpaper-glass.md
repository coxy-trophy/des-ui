# Wallpaper and Glass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add configurable blurred wallpaper and readable frosted surfaces, then a measured native liquid-glass treatment for floating controls.

**Architecture:** Shell-owned cached wallpaper and shared material primitives provide the first independently usable release. A capability-negotiated compositor material pipeline supplies real application-aware glass after a rendering feasibility gate. Keep foreground text sharp and preserve existing lock, capture and input ownership.

**Tech Stack:** Rust, egui/eframe glow, existing image crate, Smithay GLES, shared des-common settings and IPC.

**Spec:** ../specs/2026-10-20-wallpaper-glass-design.md

## Global constraints

- Existing installations retain Solid and no wallpaper until explicitly changed.
- Static PNG/JPEG only; encoded size ≤32 MiB, each dimension ≤8192, decoded RGBA ≤64 MiB.
- Imports use private 0700 directories, 0600 files and atomic replacement; never delete source files.
- Reduce transparency forces opaque surfaces; reduced motion respects the existing animation switch.
- One image across displays, independently fitted. No idle material animation.
- At most 16 live regions per output; 64 MiB per-output and 128 MiB total material-cache ceilings.
- Low: quarter-resolution blur, maximum 30 live backdrop updates/sec. High: half resolution, bounded by output refresh.
- Preserve unrelated pending work and never install/restart a working desktop during implementation without user authorization.

## Review focus

- Rapid image selection and failed saves must preserve the last successfully applied image.
- Mixed scale/rotation and hotplug must not shift samples or leave old rectangles.
- App switches, shell reconnect and lock must invalidate stale material pixels.
- Moving application content underneath stationary glass must damage the glass correctly.
- No supported live-glass capability or GPU resources must produce a readable Frosted/Solid fallback.

## Task 1: Wallpaper and settings, independently usable

**Files:** create common/src/appearance.rs and shell/src/wallpaper.rs; modify common/src/lib.rs, shell/src/{main,config,settings,runtime,panels,screen_views}.rs and the existing configuration apply handler located from settings::Change. Add module-local tests. Do not add compositor IPC yet.

**Interfaces:** shared AppearanceSettings, SurfaceStyle::{Solid,Frosted,Liquid}, WallpaperFit::{Fill,Fit}, and Quality::{Auto,Low,High}. AppearanceSettings::default() returns no wallpaper/Solid. AppearanceSettings::validate() rejects nonfinite values and enforces spec bounds. WallpaperImport::request(path: PathBuf) -> u64 starts a generation; poll() -> Option<ImportResult> returns a bounded decoded draft or an actionable error. Define ImportResult and ownership in wallpaper.rs; neither method mutates saved configuration.

- [x] Add failing tests for default/backward-compatible configuration, valid ranges, unknown style fallback, and paths containing spaces, Unicode, # and =; use a safely encoded path representation compatible with the existing line-oriented config.
- [x] Add failing import tests for malformed headers, oversized dimensions/allocation, encoded size cap, unsupported/animated formats, stale generation rejection and disappearance of the original source after import.
- [x] Run focused new tests with `cargo test -p des-common --locked --offline appearance` and `cargo test -p des-ui --locked --offline wallpaper`; record expected failures before implementation.
- [x] Implement typed settings and bounded worker import, private normalized managed copy and cache lifecycle. Applying a draft must atomically commit configuration only after import succeeds; retain the prior asset until then, and garbage-collect unreferenced managed assets after success.
- [x] Add Wallpaper controls, Apply appearance/Cancel, thumbnail/loading/error states and accessible labels through existing Settings owners. Reuse picker/document-opening conventions after tracing them; if no file-picker owner exists, provide a labelled local path field and Browse through the existing portal interface where supported.
- [x] Paint cached wallpaper before existing shell content on main and secondary views; implement Fill/Fit and blur/dim defaults from the spec. Keep work surfaces Solid in this task.
- [x] Add headless settings tests for keyboard Apply/Cancel, failed save retaining active appearance, selecting an image preserving a previously chosen style, and Reset affecting only new appearance controls.
- [x] Run the focused tests and build; document missing platform verification rather than calling it passed.

## Task 2: Shared Frosted surfaces and accessibility

**Files:** create shell/src/material.rs; modify shell/src/{ui,main,panels,settings,launcher,dock,screen_views,runtime}.rs and notification/popover painters identified from existing owners. Update DESIGN.md and UX-CONTRACT.md with the implemented subset.

**Interfaces:** material::paint(ui: &egui::Ui, rect: egui::Rect, role: MaterialRole, appearance: &AppearanceSettings, wallpaper: &WallpaperView). MaterialRole covers WorkPanel, FloatingControl and Notification; WallpaperView is a read-only cached sampling view produced by wallpaper.rs. Return no input response: existing widgets retain input ownership.

- [x] Add failing tests for effective material selection: reduced transparency => Solid, terminal opacity >=90%, ordinary frosted tint >=75%, absent wallpaper => readable ground/tint fallback, reduced motion/global animation off => no material animation scheduling.
- [x] Implement wallpaper-sampled frosted fills with cached blur, sharp foregrounds and central role-based tint/radius rules. Remove only the opaque parent fills that would hide these samples; retain explicit fallback fills.
- [x] Add preview and style controls using the same shared painter; Liquid remains unavailable with an explanation until capability support exists.
- [x] Run focused material/settings tests and capture local visual samples if a graphics backend is available. Check dense terminal text, launcher and notifications on bright/busy backgrounds; record contrast checks and unresolved hardware checks.

## Task 3: Live-glass feasibility gate

**Files:** prototype compositor/src/material.rs; inspect and minimally adapt compositor/src/{toast,winit,udev,splash,state}.rs, shell/src/{screen_views,dock,material}.rs and common/src/ipc.rs. Record the chosen design in docs/validation/2026-10-20-glass-prototype.md before broader integration.

**Consumes:** task 2's material roles and fallback. **Produces:** a documented surface/commit synchronization contract and one dock prototype on both render backends. Final protocol signatures follow the proved surface architecture; task 4 must not proceed with an unresolved contract.

- [ ] Prove whether a dedicated transparent overlay surface can retain existing dock input regions and per-output ownership while allowing the compositor to insert a material under its alpha foreground. Compare against a synchronized foreground-only crop; choose one using evidence, including notification overlap and old-compositor behavior.
- [ ] Prototype a padded below-layer capture, downsampled blur, rounded mask and bounded edge refraction for one dock. No text or foreground is included in the source sample.
- [ ] Verify that visible wallpaper/materials survive screenshot and recording composition while phosphor remains excluded, identifying the canonical capture render path from code.
- [ ] Measure idle redraw, moving-content damage, memory and GPU time in winit and installed DRM. Test opaque/Solid fallback on shader failure. If native access is unavailable, mark this gate pending and keep Liquid unavailable; do not guess at benchmark results.
- [ ] Freeze exact shared descriptor/capability signatures and surface lifecycle in the prototype document, and adjust task 4's file map to the proved design before implementing it.

## Task 4: Production live Liquid and bounded state delivery

**Files:** common/src/{appearance,ipc}.rs; shell/src/{desktop,material,dock,screen_views,settings}.rs; compositor/src/{material,state,toast,winit,udev}.rs; canonical capture and protocol handlers identified in task 3. Preserve shell/src/desktop.rs's pending coalesced presentation fix.

**Consumes:** the task 3 contract. **Produces:** negotiated, bounded surface-commit-synchronized region snapshots; compositor material output; effective-style fallback visible in settings.

- [ ] Add failing protocol/state tests for malformed/nonfinite geometry, >16 regions, foreign sender rejection, stale surface generation, explicit empty snapshot, queued clear under backpressure, shell reconnect and old compositor capability fallback.
- [ ] Implement latest-state delivery and capability negotiation. Tie region activation to the intended surface commit; never combine new geometry with an unrelated old foreground frame.
- [ ] Implement shared per-output source caches, blur padding, old/new bounds damage, sampling clamps, output scale/rotation transforms and allocation ceilings. Add tests for moving content under stationary glass and mixed-output resource accounting.
- [ ] Add lock/unlock, output removal and shell-disconnect invalidation; add tests showing no pre-lock source texture or stale descriptor is reused.
- [ ] Add bounded hover/press highlights and edge response on approved floating controls only, reusing existing animation scheduling. Respect accessibility switches immediately.
- [ ] Enable Liquid only when the capability is ready; display effective fallback in Settings. Exercise capture/recording parity through the canonical scene path and ensure external applications remain unmodified.

## Task 5: Verification and reviewable release

**Files:** DESIGN.md, UX-CONTRACT.md, docs/UNTESTED.md and docs/validation/2026-10-20-wallpaper-glass.md.

- [x] Run `cargo test --workspace --locked --offline --no-fail-fast -j2`, `cargo build --workspace --locked --offline -j2` and `git diff --check`. Distinguish actual regressions from known sandbox Unix-socket failures; require all new runnable tests to pass.
- [ ] Perform the spec's installed-session matrix: switching terminal/apps, multiple live terminals across shell restart, less/Vim scrolling, links, IME, launcher keyboard input, lock, capture, multiple outputs, scaling/rotation, hotplug and fullscreen scanout.
- [ ] Benchmark identical Solid/Frosted/Liquid scenes: idle, terminal output, scrolling browser and moving windows at 1080p and 4K where supported. Record machine, CPU/GPU, p50/p95 GPU time, repaint count, texture allocation and sampling method. Compare against the spec's <2ms added p95 GPU target at 1080p; lower defaults or keep Liquid experimental if unmet.
- [x] Record verified behavior and limitations, update design/interaction contracts to match shipped stages, and produce a clean scoped diff. Do not claim installation or native visual success from a build alone.

## Recommended delivery

Ship tasks 1–2 as the first reviewable feature. Complete tasks 3–5 before enabling application-aware Liquid. This keeps wallpaper and frosted panels useful while the compositor work receives its own visual/performance acceptance. Implementation is the next action after the user reviews this proposal; no source changes are part of this planning task.


Execution status (2026-10-20): tasks 1–2 delivered locally; 19 new tests pass, workspace 541 pass / 9 sandbox integration failures, build warning-free. Task 3 requires a native rendering prototype and acceptance, so task 4 remains gated. Task 5's hardware/benchmark checks are pending. See docs/validation/2026-10-20-wallpaper-glass.md and the prototype gate document. No installation or full-plan completion is claimed.
