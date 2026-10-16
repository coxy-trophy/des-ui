# Desktop Workflows Implementation Plan

> For agentic workers: use subagent-driven-development for independent modules, with parent-owned integration and final review.

Goal: deliver all five requested desktop features with real entry points and tests.
Architecture: preserve shell/compositor split. Extend native shell modules and typed IPC, reuse Signal controls and atomic config practices.
Tech stack: Rust, egui, Smithay, des-common, serde_json.
Spec: ../specs/2026-10-15-desktop-workflows-design.md

## Global constraints
- Preserve pre-existing edits. Work in current workspace; do not commit, reset, publish or install.
- No secrets, terminal output or arbitrary commands in saved sessions.
- Local search is bounded, asynchronous and stale-safe. No shell execution of queries.
- All new UI uses ui.rs tokens/primitives, explicit clear, keyboard/Escape states, and existing Settings/launcher owners.
- All docks are suppressed while locked. Keep authenticated IPC.

## Tasks
1. Terminal search — term.rs and term/search.rs; test literal/Unicode/history/wrap and input isolation before implementation. Root supplies no changes in these files. Agent exposes any integration API needed.
2. Health — health.rs plus Settings Health section; read-only probes and bounded logs, refresh and open log actions. Agent owns settings.rs for health only; preserve Session controls by coordinating with root.
3. Session restore model/controller — session_restore.rs only; tests serialization/privacy/atomic persistence/missing apps/duplicates and placement retry. Root integrates fields, controls and typed placement command.
4. Palette — launcher.rs and palette.rs; tests ranking, actions, file bounds and stale query behavior. Root integrates action dispatch with existing shell helpers.
5. Per-monitor dock — dock.rs, common IPC, compositor overlay/input and runtime.rs; tests screen-local grouping, protocol and geometry routing. Root owns all shared protocol and runtime integration.
6. Integration — Session controls and palette actions, settings routing, source docs, design/UX records. Run workspace tests/build and UI audit; review all changes and fix findings.

## Review focus
- Lock never leaves an external display dock interactive.
- Restoring cannot run commands from the snapshot or continually duplicate apps.
- Secondary monitor events use its shell and coordinate space.
- Older file queries cannot overwrite a newer result set.
- Search input never reaches the terminal process.

## Progress
- Initial scope and boundaries recorded; baseline has 193 passing tests and four sandbox Unix-bind failures.

- Task 1: implemented — incremental literal/Unicode/soft-wrap terminal search, bounded match storage, IME and PTY isolation; reviewed and fixed memory/composition findings.
- Task 2: implemented — Settings Health, compositor/music state, helpers, bounded redacted logs, refresh and shared native viewer recovery.
- Task 3: implemented — private versioned snapshots, known-app launch planning, retained inactive-workspace slots, terminal-folder reuse, queued placements and confirmed workspace selection with 30-second expiry.
- Task 4: implemented — typed app/window/settings/action/file palette, one bounded stale-safe worker, explicit folder configuration and exact native path arguments.
- Task 5: implemented — display-local dock groups, overlays, pointer/keyboard surface routing, lock/capture suppression and shared pins; reviewed clear/collision/capture findings fixed.
- Task 6: integration and scoped reviews complete; final automatic verification recorded in docs/validation/2026-10-15-workflows-results.md. Native hardware acceptance remains pending.

Ruling: preserve the current working tree rather than create a checkout from HEAD — the user's accepted previous work and extensive uncommitted edits are the input to this task.
Ruling: session restore is explicit, with no automatic launch at login — avoids replaying application launches without a direct user action.
Ruling: snapshots use a private sibling `des-ui-sessions` directory — existing `des-ui` log directories can legitimately be group-writable and must not be silently chmodded.
Ruling: restoration retries placement/workspace traffic within a fixed deadline and waits for authoritative workspace state — full IPC queues must not lose final workspace selection or hold a restore forever.
Ruling: validate native egui with headless interaction tests and code review here — this sandbox does not permit native display/socket integration or physical display certification.
