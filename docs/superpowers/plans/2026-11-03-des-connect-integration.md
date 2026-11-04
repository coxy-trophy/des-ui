# DES Connect Integration Implementation Plan

> Implement inline with independent Android design and shell-panel tasks under the dispatching-parallel-agents skill.

**Goal:** Match the Android app to DES UI, establish real authenticated Android/desktop pairing, and expose actual state/actions in the right panel; then extend verified capabilities without advertising unfinished ones.

**Architecture:** Android GPL fork owns shared platform themes. Native Rust daemon owns TLS identity, persisted peer pins, bounded discovery/connections and private UID-checked IPC. Shell egui CONNECT consumes asynchronous IPC snapshots and never starts networking on shell boot. Unsupported features stay out of the action list until their backend exists.

**Spec:** ../specs/2026-11-03-des-connect-design.md

## Constraints and checks

- Preserve existing Apps/window management until the left dock offers replacements.
- Never modify the owner's active terminal sessions or restart the installed compositor for preview verification.
- Identity/capability claims before TLS never authorize features. Require protocol8 encrypted identity exchange, certificate/key possession and identity binding, explicit pair verification and pinned reconnect.
- Private identity/pin files and IPC are owner-only; bound frames, connections, timeouts and state queues.
- Defaults use event-driven/blocked I/O and visible-only shell refresh; no decorative animation workers, capture/encoding only while sharing.
- Android shared tokens cover Compose and XML; retain accessible Android48dp targets, responsive phone/tablet layout, font/license and upstream notices.

## Tasks

- [x] Android shared Signal colors, fonts, shapes, control density and device hierarchy; build/unit tests and real-tablet visual verification.
- [x] Native private certificate identity/pin persistence, TLS1.2 interoperability and v8 verification code; tests for file permissions/cert mismatch and transcript/geometry limits.
- [x] Bounded LAN discovery/connection lifecycle, per-session/attempt pair callbacks, real ping and battery; private IPC/CLI and authorization tests.
- [x] Native CONNECT tab/controller with service unavailable/empty/pairing/trusted/error states and explicit actions. Build and verify via isolated shell preview.
- [ ] Actual tablet pairing with verification code comparison, bidirectional ping/battery, restart/reconnect and unpair checks.
- [ ] Add remaining sharing/control/mirroring capabilities in tested increments, with Android consent and locked-session protection; carry any outstanding stages explicitly in docs.
- [ ] Independent security/correctness review, focused fixes, final test/build, source commits/pushes and honest verification record.

## Progress ledger — 2026-11-03

Core transport, Signal Android redesign, native panel and view-only USB screen
bridge implemented. Real pairing, bidirectional ping, battery, trusted daemon
restart/reconnect and visible Android mirror verified. Real unpair not exercised
on the tablet; loopback TLS lifecycle covers it. See docs/DES-CONNECT.md for
commands, evidence, outstanding capabilities and runtime limitations.

Ruling: Keep per-feature adapters unadvertised until live permission checks and
panel dispatch exist. This avoids prompting Android permissions for controls the
desktop cannot yet perform. Transfer/media/feature adapters are checkpointed as
tested library work, not shipped functionality.

Security review fixes and focused regressions passed; complete capability-set
review remains due after the remaining runtime integrations. Preserve existing
terminal sessions; the compositor has not been restarted.

### File-sharing increment

- Integrate payload manager behind persisted per-device file permission, default
  off. Derive encrypted identity capabilities from authenticated peer policy.
- Explicit reconnect applies capabilities; discovery never advertises private
  opt-ins. Cancel pending/active work on lock, disconnect, unpair, disable or stop.
- Native panel uses the existing Files panel for locating a path; no duplicate
  file browser. Require explicit send confirmation and receive acceptance.
- Extend Android listener acceptance to60s to allow a person to review the
  desktop's30s offer. TLS deadlines stay unchanged.
- Keep DES Music's new socket adapter lazy and unadvertised until its separate
  runtime opt-in and media dispatch are integrated.
- Ruling: installer includes service/CLI/helper, but never starts networking on
  boot. Existing installed compositor and user terminals are not replaced during
  automated verification.

### Media permissions increment

Files checkpoint f46f647 passed82Rusttests and21paneltests, with physicalretry
pendingAndroidcallbackfix. Continueinline/parallelwithinapprovedscope: add
separatemedia/systemvolume opt-ins, encryptedcapabilities and unlockedruntime
dispatch. Phoneownsplayercontrols; nativepanelownsonlypermissions/reconnect.
No realaudiochangesinautomatedtests; adaptersremainslazyandnocapturestarts.

### Resume verification — 2026-11-04

Media service, lazy adapter guards and native permission controls are implemented.
Fresh commands: cargo test -p des-connect --locked -j 2 => 87 passed/1 ignored;
disposable dbus-run-session fake_mpris => 1 passed; cargo test -p des-ui
connect::tests --locked -j 2 => 23 passed/1 fixture ignored; service and shell
builds passed. Final independent media review found no Important/Critical issues. Android
155-test verification and final APK build passed; source ae067d10 is pushed.
No ADB device is connected; physical file/media checks cannot be claimed complete.

### Tablet acceptance follow-through — 2026-11-04

Both physical file directions passed exact-byte checks with disposable59-byte
files; incoming offer stayedat0bytes until explicitAccept. Filepermission
revocation removedtheAndroidcontrol. Androidstartupreplyrace reproduced/fixed
withsharedDevice lifecyclemonitor; physicalfake-playerPlay/Pause/seek/volume
passed. Volume-onlyUI entryanddynamicstablepager fixed/verifiedphysically;
161Androidtests across26suites pass, finalAPKinstalled, source047378a7pushed.
Rust87tests rerunpass aftercapabilitygatinglegacybatterypolls, trustedreconnect
verified. Fakeaudio/testdaemon/files/UI snapshots cleaned; normaldaemonrunning
withoriginalpermissions andpins. ActiveDesktop/terminals not restarted.
Remaining stages arethebroaderroadmap, not unverifiedfile/mediabatchclaims.
