# DesOS Desktop Stabilization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair confirmed delivery defects and produce a verified desktop candidate for subsequent DesOS hardware acceptance.

**Architecture:** Keep the current Rust services and shell unchanged except for installed viewer discovery. Repair existing packaging/uninstall scripts and their integration tests; correct the startup test's process-exit race and the website recovery copy.

**Tech Stack:** Rust, POSIX shell, Python unittest, Debian packaging, React/Vite.

**Spec:** `docs/superpowers/specs/2026-11-09-desos-stabilization-design.md`

## Global Constraints

- Baseline desktop `50794d0`, website `25f6fba`; preserve source changes outside this work.
- No installation, publication, session restart, power transition, real phone/capture/provider operation or kernel changes.
- Use real Debian archives in temporary packaging tests; no real administrator commands.
- Preserve explicit Connect startup and existing permissions.
- Hardware acceptance remains Pending without actual candidate-specific evidence.

## Review Focus

- Clean package-only installation without a source checkout must find the viewer.
- Native dependency discovery must include Connect and must fail the build if discovery fails.
- Uninstall must remove new executables without deleting user state.
- `/proc` disappearance must pass cleanup while permission/read errors still fail.
- Recovery instructions must preserve password authentication and warn when ending a session loses work.

### Task 1: Repair package/source installation parity

**Files:** `packaging/build-deb.sh`, `uninstall.sh`, `session/tests/test_packaging.py`, `connect/src/screens.rs`.

**Interfaces:** Package installs Connect tools into `/usr/bin` and helper into `/usr/share/des-ui`; service supports both installed prefixes. Dependency discovery emits the native libraries or fails the package build.

- [ ] Add temporary-tree package regressions using real `dpkg-deb`, fake release executables and controlled `dpkg-shlibdeps`; assert archive contents, dependencies, executable modes and no autostart unit. Add missing-input/dependency-failure cases and uninstall assertions.
- [ ] Run `python3 -m unittest discover -s session/tests -p test_packaging.py -v`; confirm missing Connect tools and dependency-failure assertions fail.
- [ ] Add Connect tools/helper and linked binaries to package staging, propagate dependency errors, and complete uninstall cleanup.
- [ ] Add Rust viewer-path regression with a temporary `/usr`-style prefix and no checkout or `/usr/local` helper; verify failure before implementing installed data-directory lookup.
- [ ] Run Python package regressions and `cargo test -p des-connect --locked`; expect zero failures.
- [ ] Commit this repair and its regression tests.

### Task 2: Repair verification race and recovery guidance

**Files:** `session/tests/test_startup.py`; sibling website `src/lib/data.ts`.

**Interfaces:** Test cleanup accepts ENOENT/ESRCH only. Website recovery offers no unlock bypass and explains the consequences of ending a session.

- [ ] Fault-inject ESRCH only for `/proc/<pid>/stat` in the existing cleanup integration test; observe failure and retain command/output.
- [ ] Catch `ProcessLookupError` alongside `FileNotFoundError`; rerun the injection and normal session suite; expect zero failures. Confirm permission errors still fail using the same injection harness.
- [ ] Replace `--unlock` guidance with password-based recovery and an explicit session-ending fallback warning about unsaved work.
- [ ] Run the website production build; expect exit zero. Commit the repairs in their respective repositories.

### Task 3: Verify and record the candidate

**Files:** `docs/validation/2026-11-09-desos-stabilization.md`; ignored build artifacts/evidence.

**Interfaces:** Record exact source identity, commands, exit statuses and pending hardware cases for the candidate; do not certify target-distribution compatibility from a Kali-host build.

- [ ] Run `./scripts/verify.sh` and a release workspace build; expect exit zero, retaining full logs.
- [ ] Build actual deb archives from that release, inspect binaries/helper/dependencies and run CLI help smoke checks without starting services.
- [ ] Record the candidate snapshot and artifact hashes plus focused boot/recovery, lock/suspend, application/focus, device and performance acceptance cases.
- [ ] Obtain a fresh review of the changes and fix important findings with focused regressions; rerun affected checks.
- [ ] Commit the acceptance record; keep installation and real hardware transitions pending for the next controlled session.
