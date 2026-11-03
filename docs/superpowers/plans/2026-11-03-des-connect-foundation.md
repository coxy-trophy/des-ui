# DES Connect Foundation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish the Android fork and a tested native desktop protocol foundation without claiming device features before they work.

**Architecture:** Keep the GPL Android fork in its own repository. A separate MIT Rust workspace crate owns bounded KDE-compatible packet framing, identity/capability validation and pairing state transitions. Later transport integration must authenticate TLS peers before authorizing those transitions; this stage exposes no network listener or remote commands.

**Tech Stack:** Kotlin/Gradle Android app; Rust, serde and serde_json desktop foundation.

**Spec:** docs/superpowers/specs/2026-11-03-des-connect-design.md

## Global Constraints

- Android is the initial mobile platform.
- Keep desktop memory use a first-class constraint.
- Do not relabel upstream GPL code as MIT.
- Never disturb the owner's active terminals or replace the login session without reporting restart requirements.
- Commit and push coherent verified changes per the user's preference.

## Review Focus

- Oversized/malformed packet frames must fail without unbounded buffering.
- Unpaired peers cannot activate application capabilities.
- Disconnect/reconnect must not imply trust or accept stale pairing decisions.
- Identity and capability input is untrusted and must be bounded.
- Android branding must retain license notices and avoid overwriting the installed upstream app.

## Task 1: Android fork, inventory and build

Files: independent des-connect-android repository, its build.gradle.kts, app resources and DES-CONNECT.md; desktop docs/DES-CONNECT.md.

- [ ] Fork KDE/kdeconnect-android to coxy-trophy/des-connect-android; add upstream and record exact base revision.
- [ ] Read build, manifest, resource and license structure. Record file-level license patterns and dependency inventory limits.
- [ ] Add a separate application ID and DES Connect label for a co-installable fork; preserve notices and license files.
- [ ] Run upstream unit/build checks that tooling permits. If tooling fails, reproduce exact cause and record it; do not claim a built APK.
- [ ] Commit/push Android customization and desktop inventory.

## Task 2: Desktop protocol and pairing state foundation

Files: connect/Cargo.toml, connect/src/lib.rs, connect/src/packet.rs, connect/src/pairing.rs; workspace Cargo.toml/Cargo.lock.

Interfaces: Packet encoding/decoding consumes newline-delimited JSON with a 64KiB frame bound. Identity validation consumes a decoded identity packet and returns a checked identity. Pairing state only consumes an explicitly authenticated peer identity/fingerprint from a future TLS adapter, never raw network claims.

- [ ] Inspect actual upstream identity, pairing and protocol-version source before fixing model semantics.
- [ ] Write tests for frame boundary, malformed/oversized JSON, identity validation, unpaired access, explicit acceptance, mismatch and stale/disconnected requests.
- [ ] Run tests and observe failure before implementation.
- [ ] Implement the bounded pure protocol/state modules; do not create an unauthenticated LAN service or persist false trust.
- [ ] Run focused tests, clippy and workspace metadata validation.
- [ ] Document the TLS/session adapter and real-device verification still required. Commit/push.

## Task 3: Review and handoff

- [ ] Review both diffs, build results and license/branding boundaries independently.
- [ ] Record exact completed checks and unresolved Android toolchain/device requirements.
- [ ] Mark stage truthfully: no pairing, streaming or right-panel feature is working until integration and real-device verification exist.

The remaining stages in the design (authenticated transport, native tab, feature plugins and screen modes) follow separate implementation plans after the foundational contracts are established; this plan does not silently narrow their agreed target scope.

## Execution ledger

- Approved direction: user explicitly chose Android and authorized native Rust desktop + Android fork on 2026-11-03.
- Task 1: GitHub fork created, upstream base recorded, separate package/branding and runtime identity fixes implemented. Static checks pass; Android Gradle build is in progress with an existing full JDK21. No Android device is connected.
- Task 2: initial tests failed with missing modules; nine implementation tests then passed. Independent review reproduced two stale-attempt failures; per-attempt tokens fixed them. Added exact frame and identity boundary tests: 13/13 pass. Native crate build and workspace metadata pass. fmt/clippy unavailable on this machine.
- Task 3: independent review completed. Important findings fixed (attempt identity and Android FileProvider authority). Framing errors documented connection-fatal; exact-boundary tests added. No finished connectivity or deployment claim.
