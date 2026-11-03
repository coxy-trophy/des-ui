# DES Connect: Android-first development

Approved direction: native Rust desktop service, custom right-panel CONNECT tab,
and a separate GPL Android fork. All KDE Connect-style capabilities plus screen
mirroring remain the target; this first change establishes the fork and protocol
foundation only.

## Repositories and licensing

- Desktop: this MIT workspace, new `connect` library crate.
- Android: https://github.com/coxy-trophy/des-connect-android
- Android checkout: sibling `../des-connect-android`; the existing
  `../des-ui-mobile` launcher project remains untouched.
- Android upstream: KDE/kdeconnect-android at
  `3242aeff092bba165a64d63a690d31627064644d` (protocol 8).

The Android source inventory finds predominantly
`GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL` declarations and
Apache-2.0 third-party files. This is a source inventory, not a completed
transitive dependency-license review. Upstream COPYING/LICENSES, copyright
headers and Gradle dependency-license generation are retained. No GPL source
implementation was copied into the MIT Rust library. Screen extensions will
require their own dependency/license review before distribution.

The Android fork has a DES Connect launcher label, independent application ID
`io.github.coxytrophy.desconnect` (debug suffix `.debug`), runtime-aware
self-notification filtering and matching received-file FileProvider authority.
Internal namespaces, existing artwork and most upstream screens remain intact;
this is not yet the planned Android UI redesign or screen-sharing extension.

## Native foundation

`connect/src/packet.rs` implements bounded newline-delimited JSON framing and
identity validation. `connect/src/pairing.rs` models trust transitions with opaque
connection and per-attempt tokens, incoming/outgoing timeouts, changed-key
rejection and explicit approval/send-success requirements. See connect/README.md
for limits and the mandatory TLS-adapter contract.

The crate does not run a network service. Discovery/TLS, certificate identity
binding, verification-code display, persistent key/pin storage, private IPC and
the CONNECT tab remain unimplemented. Unit tests are not an Android pairing test.
No remote commands or network listener have been enabled by these foundations.

## Verification

- `cargo test -p des-connect --locked`: 13 passed.
- `cargo build -p des-connect --locked`: passed.
- `cargo metadata --no-deps --locked --format-version 1`: passed.
- `git diff --check`: passed for both repositories.
- Android static checks: distinct app ID/label, dynamic manifest provider
  authority, no hard-coded upstream provider use, retained license files.
- The first Java invocation failed because inherited JAVA_HOME pointed to a
  missing JDK17. System Java25 then failed because it lacks JAVA_COMPILER.
  Existing full JDK21 at /home/coxy/.local/jdk-21 is used command-scoped instead.
- Rust formatting/clippy commands were attempted but those cargo components are
  not installed; they are not reported as passing.
- Independent review found stale same-connection pairing callbacks and an Android
  file-provider authority mismatch. Both were reproduced and corrected; the
  pairing regressions now pass with per-attempt tokens. Frame error handling is
  documented connection-fatal and exact-limit tests were added.

Android Gradle build/unit-test result is recorded below after completion.
`adb devices -l` currently reports no connected device. Real pairing,
received-file opening (including coexistence with official KDE Connect), Android
permissions and screen capture/control still require device verification.

## Next integration boundary

Implement authenticated protocol-8 transport and durable private identity/pin
storage, then demonstrate two-way ping with the Android fork. Only then expose
real device status and pairing controls in the right panel. Preserve the right
Apps tab's management actions until they have working replacements. Production
video must replace the experimental raw-RGBA USB viewer with measured, efficient
encoding and an authenticated transport; do not expose its existing local helper
on the LAN.
