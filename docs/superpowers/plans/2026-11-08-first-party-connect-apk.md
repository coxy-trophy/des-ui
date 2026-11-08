# First-party DES Connect APK Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development for the independent Android transport and app tasks. Verify the real app against an isolated daemon before promoting it.

**Goal:** Deliver the first separately installable DES Connect APK with live discovery, pairing, persistent reconnect, battery and ping.

**Architecture:** A small native Android Activity owns an opt-in, foreground-only connection to the existing Rust daemon. Android Keystore owns the device key; the connection worker owns the TLS socket and pending pairing state. The desktop wire and installed fork stay unchanged.

**Tech Stack:** Java and Android platform APIs; Android SDK tools for packaging; existing Rust `des-connectd` for integration tests.

**Spec:** [Approved design](../specs/2026-11-08-des-connect-companion-design.md).

## Global constraints

Separate package `io.github.coxytrophy.desconnect.companion`; app label DES Connect Preview; no fork code/assets, capture, launcher replacement, privileged permissions, or idle background connection. Initial minimum API 26; compile/target API 36 for this directly installed preview. Phone initiates TCP to the desktop and acts as TLS server after sending its plaintext discovery identity. Only verified mutual TLS and explicit, current pairing approvals can authorize actions. Existing forks and live desktop pins remain untouched during tests.

## Review focus

Changed certificates, unsolicited acceptance, stale approvals, truncated frames, socket teardown, and foreground/background transitions must fail closed. Offline/empty discovery must allow explicit address entry. No test should reuse the live daemon identity or replace an existing APK.

### 1. Authenticated foreground link

- [x] Implement `Link` and identity/pairing helpers under `connect/android/src/io/github/coxytrophy/desconnect/companion/`.
- [x] Reuse the Rust daemon's verified wire format: bounded JSON lines, protocol 8, TLS 1.2, cert CN bound to secure identity, timestamp/public-key verification code. Keep a single desktop pin in app-private storage and support both pairing directions.
- [x] Add targeted regression checks for stale approvals and unsolicited pair acceptance. Validate paired battery, ping, unpair and reconnect in the real app.

### 2. Native app and reproducible APK

- [x] Implement `MainActivity`, app resources, manifest and `connect/android/build.sh`. Draw readable device/verification states, explicit pairing actions and a manual address fallback. Close sockets and discovery when the app is stopped.
- [x] Sign with a local debug key outside the tracked tree, verify the APK, and inspect its UI at phone widths.

### 3. Discovery and acceptance

- [x] Implement bounded, foreground-only reception of the existing desktop broadcast. Discovery is an untrusted hint until TLS and user confirmation. Keep manual address available when UDP discovery is unavailable.
- [x] Exercise both pairing directions, persistence across app restart, battery/ping, unpair and failure on changed trust. The emulator stopped before installation, so use the connected realme (Android 14) with an isolated daemon via ADB reverse for these checks.
- [x] On realme, verify UDP discovery using an explicit announcement on the Wi-Fi interface and connect to the discovered desktop over LAN without creating a live pairing. The Samsung tablet and emulator still need a run before broader release.
- [x] Document the build, working flows, release limitations and cutover status.

On realme, a sign-only Keystore RSA key needed `NoPadding` authorization for
Conscrypt's TLS signing upcall. The installed APK also rejected a different
certificate with the same saved desktop ID and recovered through offline Forget.
On the desktop, a TLS regression demonstrated that simultaneous timestamped
requests could be mistaken for approval. The fixed daemon requires a
timestamp-free acknowledgement; the full `des-connect` test suite passed.
