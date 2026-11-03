# DES Connect: Android-first development

The approved target is a native Rust service, a right-panel CONNECT tab, and a
custom Android companion with KDE Connect capabilities plus screen sharing.
This is an incremental implementation; the entire capability set is not finished.

## Repositories and licensing

Desktop: this MIT workspace. Android: https://github.com/coxy-trophy/des-connect-android,
local sibling ../des-connect-android. Its upstream base is KDE/kdeconnect-android
3242aeff092bba165a64d63a690d31627064644d (protocol8). The separate des-ui-mobile
project is untouched. Retain upstream COPYING/LICENSES, copyrights and dependency
license generation. Android source predominantly declares GPL-2.0-only OR
GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL with Apache third-party files. This is
not a complete dependency-license audit. No GPL implementation was copied into
Rust. Bundled Chakra Petch and JetBrains Mono retain OFL notices.

The Android app ID is io.github.coxytrophy.desconnect (.debug for debug builds).
Provider authorities and self-notification filtering follow the runtime ID.
The app now shares DES UI's Signal palette, local fonts, outlined matte surfaces
and shared XML/Compose theme tokens. Touch targets remain at least48dp; existing
light/dark/system preferences, translations, Android permissions and upstream
credits remain. See the Android DESIGN.md and UX-CONTRACT.md.

## Working integration

- Native protocol8 discovery, private certificate/key persistence, TLS pairing,
  verification-code display and persisted pinned reconnect.
- Bidirectional ping and Android battery/charging status.
- Per-device file-sharing opt-in, encrypted capability negotiation, explicit
  reconnect, send confirmation, incoming acceptance, progress and cancellation.
- Native CONNECT tab with explicit start/stop, discover, code-checked accept,
  reject, ping, unpair confirmation and honest unavailable/offline states.
  Existing SYSTEM/APPS functionality is preserved.
- Explicit view-only USB Android-to-desktop mirroring through pinned scrcpy5.0.1.
  Starting/stopping, lock checks, disconnect cleanup and capability reporting are
  wired. No capture starts on boot or on pairing.
- Existing extended-output helper can be invoked only when the current compositor
  exposes its opted-in virtual-output socket. It is unavailable in the current
  installed session. Physical-desktop-to-Android mirroring is unavailable.

Run/build instructions and security boundaries are in ../connect/README.md;
USB requirements and limitations are in ../connect/SCREEN-BRIDGE.md.

## Device verification — 2026-11-03

The connected Samsung SM-P585, Android14, was used with the owner's permission.
Real verification codes matched, pairing succeeded, battery/charging state came
from the tablet, and ping was confirmed in both directions. A native CONNECT
preview rendered the live paired state in isolation without restarting the
installed desktop or touching existing terminals. A daemon stop/start retained
trust, reconnected without approval and returned battery state; Android Send ping
then updated the restarted daemon's ping timestamp.

USB mirroring visibly displayed the Android screen in a real desktop window.
The viewer was stopped successfully. Hardware decoding reported initialization
errors before software fallback displayed frames. Sustained performance, hardware
decode and motion/latency measurements are still required. Private screenshots
and device keys are not repository artifacts.

The Android redesign was checked on the tablet in landscape, portrait and a
400dp layout; its original size/density/rotation were restored. An initial
capability-aliasing bug exposed unsupported plugin controls/permissions. The fix
filters plugins on initial identity and later explicit capability updates, honors
empty sets, preserves missing-vector legacy behavior, and removes stale modules.
Six failing regression assertions became green. The installed rebuilt APK now
shows Connected with an honest no-controls state and retains Send ping in its
menu instead of advertising unavailable desktop features. Notification-listener,
SMS, contacts and accessibility permissions were not granted through automation.
No real SMS/call or message to another person was sent.

## Automated verification

- Rust service suite:67 passed,1 ignored; isolated fake-MPRIS integration also
  passed on its disposable bus. Includes real loopback TLS sessions/payloads,
  file collision/symlink protection, explicit transfer acceptance/cancellation,
  stale approval callbacks, reconnect/replacement, absolute handshake deadlines,
  lock refusal and screen-process cleanup.
- cargo build -p des-connect passed.
- Native panel/controller tests:15 passed,1 fixture ignored by default. Isolated
  native fixtures were visually checked at narrow widths and with live IPC.
- The shell build passed. The earlier full shell run had a timing-sensitive
  terminal backpressure test exceed its existing10s deadline under load; the
  isolated rerun passed. Terminal code/deadlines were not changed for this work.
- Android testDebugUnitTest/assembleDebug passed:152 tests across25 suites,
  zero failures/errors/skips. APK installed with adb --no-streaming because the
  streamed installation path fails with splice EINVAL on this machine.
- Inherited JAVA_HOME is invalid; command-scoped JDK21 at
  /home/coxy/.local/jdk-21 and ANDROID_HOME=/home/coxy/Android/Sdk were used.
- Rust format/clippy components are absent; those checks are not claimed passing.
- Static UI audit reports33 existing findings in the old HTML terminal prototype;
  no new native UI findings were reported by that audit.

Security review findings corrected with focused tests: approval tokens reused
across new actors, slow-handshake slot exhaustion, lock/trust changes during USB
preparation, and starts surviving an IPC caller disconnect. The same-certificate
TLS replacement race observed against the tablet was also reproduced and fixed.
This is not an independent penetration audit.

## Memory and remaining work

A preliminary daemon sample with two unpaired candidates measured11804KiB RSS,
5586KiB PSS,3 threads and0.2% average CPU. Android foreground debug PSS was about
160MiB. These are samples, not release/background/video budgets. Keep visible-only
panel polling, lazy services, bounded caches and no capture while idle.

File transfers are integrated with per-device permission and lock/loss cleanup.
MPRIS/volume and other feature packet adapters remain library modules pending
live permissions, daemon dispatch and panel forms. DES Music now has a tested
private-socket adapter in that library. Remaining capabilities include manual
clipboard/URLs, notification actions, opted-in calls/SMS/contacts, media controls,
remote input, command allowlists and production screen transports.
Do not confuse unit-tested contracts with end-to-end Android functionality.
No session restart is required for the daemon or Android update. Loading the new
native panel into the installed shell requires a shell update; compositor virtual
output configuration may require a session restart, which has not been performed.

## File-sharing integration checkpoint

The service and panel now support default-off per-device file sharing, explicit
capability reconnect, confirmed sends, approved incoming offers and cancellation.
Encrypted peer identity carries the opted-in capability; plaintext discovery does
not. Latest Rust suite: 82 passed, one explicit media fixture ignored; that
fixture also passed separately on a disposable bus. Native CONNECT tests: 21
passed, one visual fixture ignored by default; narrow fixtures were inspected.
The installer now includes both Connect binaries and the existing viewer helper
without starting networking at boot.

Review fixes have regression coverage: expired/disconnected IPC callers cannot
start content work; source-file preparation occurs outside the global transfer
mutex; the lock watchdog stops screen sharing before consulting transfers and
uses a nonblocking manager lock; shutdown stops screens before joining file
workers. OS-stalled filesystem operations are not claimed to be interruptible.
Healthy authenticated same-address control-socket replacements preserve file
jobs; changed address/capability/policy, real disconnect, unpair and lock revoke
sharing. TLS fixtures test both continuity and revocation.

The first physical file attempt exposed Android's reused-link callback updating
capabilities without reloading actual plugins. Its source fix and regression APK
are being verified separately; this checkpoint does not claim physical file
transfer success. The prior successful physical mirror/ping checks still stand.
