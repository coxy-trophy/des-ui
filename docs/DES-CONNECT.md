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
MPRIS, DES Music and system-volume control now have separate media/volume
permissions, encrypted capability negotiation and guarded runtime dispatch. Other
feature packet adapters still need runtime integration. Remaining capabilities include manual
clipboard/URLs, notification actions, opted-in calls/SMS/contacts,
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
capabilities without reloading actual plugins. At that checkpoint its source fix and regression APK were still being verified;
physical file-transfer success was not claimed. The prior successful physical mirror/ping checks still stand.

## Media integration verification — 2026-11-04

Separate default-off media and system-volume permissions are persisted per peer
and exposed through one native device-permissions group. A single explicit
reconnect applies changed capabilities while retaining trust. Lazy request
adapters handle real MPRIS players, the existing private DES Music socket, and
the default PipeWire output. No player is started and no media is exported while
idle. Authorization is checked again after blocking preparation before every
mutation, and before each TLS metadata reply.

Fresh verification: 87 Rust tests passed; the ignored fake-MPRIS test passed
separately on its disposable bus. All 23 CONNECT tests passed. Both native service
and shell builds passed. Tests cover independent permissions, old-record
migration, locked/disabled/capability-mismatched requests, coalesced request
handling, revocation during lazy setup, and suppression of prepared metadata after
lock. All audio mutations used disposable fake services.

The machine restarted/interrupted since the previous test window; ADB currently
shows no tablet. Physical file-transfer success and Android media interoperability
remain unverified. The previously observed pairing, ping and USB mirror results
are historical checks, not claims about the current session. The Android callback fix now passes its tests and APK build; reinstallation
and the physical retry are still pending. No installed shell/compositor was restarted by this verification.

Final review found no Important/Critical findings in the media integration.
Android verification completed: 155 tests across 25 suites, zero failures, errors
or skipped tests; testDebugUnitTest/assembleDebug succeeded. After adding the
modified-file license notice, the final assembleDebug also succeeded. The callback
fix reloads real plugin handlers before notifying observers, including capability
removal; three regressions cover gain, removal and unchanged identities. Payload
listener acceptance is bounded to 60 seconds, allowing the desktop's 30-second
receive approval window without changing TLS handshake deadlines.

Android source commit: ae067d10. APK: sibling
../des-connect-android/build/outputs/apk/debug/des-connect-android-debug.apk.
SHA256: f87f7d8b1256b70225ef85cdf9cd84f3a6881edabfb7863b15999015b747e0f6.
The tablet was not connected during final verification, so this APK was not
installed in this session. Desktop media source commit: 2ca0df3 on the remote
work/des-connect branch. Installed shell/compositor binaries were not replaced;
install the build and refresh the shell to load the new CONNECT panel.

Remaining device acceptance: reconnect/unlock the authorized tablet, install the
APK, confirm file controls appear and disappear with permission/reconnect, transfer
disposable files in both directions and compare bytes, then test Android media
control against a disposable player before touching actual audio. The rest of the
feature roadmap remains in the design specification; these checks do not imply
full KDE Connect parity.

## Physical tablet verification — 2026-11-04

The authorized Samsung tablet was reconnected and the ae067d10 debug APK installed
successfully using adb --no-streaming. Existing certificate trust reconnected
without re-pairing. The Android Send files control appeared.

A disposable 59-byte desktop-to-tablet file completed and its received bytes
matched SHA256409ba2d66577316ec1a4c5d6867710bc48870449c651594ec84752e5df8d4885.
A separate disposable 59-byte tablet-to-desktop file remained offered at zero
bytes, with no received file, until explicit desktop acceptance. It then completed
and matched SHA2562f52969122b79a1754b290cdea8eb547b42b764d27d4d0dd7f854b007a6d826f.
Disabling file permission and reconnecting removed Send files from the visible
Android device screen, leaving its honest Connected/no-controls state.

Media was tested against a private fake DES Music socket and disposable D-Bus
session, with system-volume permission disabled. Direct adapter discovery
returned DES Music, but the tablet initially showed no players. Android logs
confirmed that the fast startup reply arrived during plugin creation, before the
new incoming-interface routing map was published, and was discarded. This new
Android dispatch/lifecycle race is being reproduced and fixed separately; no
physical media-control success is claimed at this checkpoint. Actual desktop
playback and output volume were not changed.

## Tablet acceptance results — completed current checks (2026-11-04)

The media startup finding was fixed by serializing packet dispatch with plugin
creation/destruction on the same Device monitor. Three concurrency regressions
failed before the fix and passed afterward. The tablet then immediately discovered
the private fake DES Music player. Android Play and Pause produced the expected
toggle commands; player volume produced 50 percent and seek produced 24192 ms in
the fake service's observed state. These actions used only the owned temporary
Unix socket and disposable D-Bus session, not actual desktop playback.

Independent system-volume permission also needed its own Android entrypoint.
The app now reuses the multimedia activity's Devices page without requiring media
permission or adding a duplicate button when media is available. Its pager
contains only loaded plugins' pages. Stable Play=0/Devices=1 IDs exclude unavailable
saved pages and hardware-volume lookup uses those same IDs. Regression coverage
checks all four plugin combinations and actual key-callback routing through the
framework fragment map. The missing button and unsupported-page cases were
reproduced by failing tests before their fixes.

On the tablet, enabling system volume with media disabled showed a System volume
button. It opened and retained an activity containing only Devices and the actual
Default PipeWire output. No unavailable Play page or MPRIS settings menu appeared.
This was a read-only UI check: output volume was 1.00 before and after, and no
actual audio setter was used. The successful file-transfer/hash and explicit
approval checks above remain the physical transfer evidence.

Final Android source: 047378a7; installed debug APK SHA256:
aac6170fd40cddbd3091cf16b644c7f00bc7fbddd19da30ebdeebfa3751b2120.
Full Android test suite: 161 tests across 26 suites, zero failures/errors/skips;
final testDebugUnitTest/assembleDebug passed. The Rust service suite was rerun
after gating legacy battery polls on the authenticated incoming capability:
87 passed, 1 explicit isolated-media test ignored (that media test previously
passed separately). Trusted tablet reconnect passed without unsupported legacy
battery-poll warnings. No native shell source changed after its 23-test/build pass.

Cleanup removed only the content-verified disposable transfer files and owned
tablet UI snapshots. The temporary fake player and private test daemon were stopped.
A normal daemon was started with the existing identity/pins and no test socket or
private-bus override; the tablet reconnected with its original file=true,
media=false and volume=false permissions. No autostart service was installed.
The installed shell/compositor and active terminal sessions were not restarted;
the new desktop CONNECT panel still requires installation and a shell refresh.
Full KDE Connect parity and production screen transport remain future stages.


## Compact CONNECT and embedded mirroring — 2026-11-04

Production CONNECT now uses the device-first layout study: selected name and
status/battery, a real in-panel preview, short actions and compact transfers.
Permissions, USB setup, pairing/service details and diagnostics are contextual.
Stop stays outside the content scroller and also appears beside SYSTEM while
sharing may still be active. Passive state refresh does not disable editors;
focused CONNECT fields keep keyboard ownership away from terminal input.
Expand uses the existing native workspace and the same preview texture, with
Minimize returning to the tab. It does not alter compositor output geometry.

The embedded source uses the pinned official scrcpy 5.0.1 server and ffmpeg,
640px maximum axis and 10fps, over a private ADB Unix forward. No browser runtime,
external scrcpy window, capture file, FIFO recording or LAN frame listener is
used. One bounded latest RGB frame and one reusable native texture are retained.
Control/audio/clipboard synchronization/power-on are disabled. Frame requests on
the existing same-user Unix socket bind the expected peer and USB serial; live
lock/pairing/connection/client checks run around acquisition. EOF, Stop, lock and
disconnect clear the source; hidden/context views discard UI textures and late
worker replies. Reconnection requires explicit capture start.

Connected Samsung SM-P585 acceptance: real 640×400 P6 (768015 bytes including
header), visibly rendered inside the native CONNECT view; first frame 8.71s.
Wrong owner/USB requests yielded no pixels. A test lock proxy delegated to the
real desktop state and could only force additional locking; it stopped capture
and denied frames, and clearing that forced lock did not restart sharing.
Explicit restart and Stop passed. Both the local capture/decoder and remote
scrcpy process were gone after bounded teardown. The normal daemon and existing
trusted identity/policies were restored. No tablet input, pairing or permissions
were changed. The physical screen was not locked during this check.

Native screenshot checks covered 180/220/300/620px, active/idle, settings,
pairing and transfer states. The diagnostic fixture gradient is test-only;
acceptance also rendered actual tablet pixels. The native fixture waits for a
real texture before taking a live capture. Private device screenshots remain
outside git. Independent review found and corrected editor routing, stale owner
races, Stop availability after errors and expanded-view exit paths.

Static UI audit: 33 existing findings, all in the older
`docs/prototypes/terminal-workbench/index.html`; none in this change. This static
audit cannot validate native egui behavior. Runtime tests and source bounds are
the evidence for this implementation. The tablet's first raw encoded byte took
about eight seconds; decode added around 20ms. Raw transport removes the roughly
four seconds of additional Matroska buffering observed in the rejected spike.


Validation commands for this update: full des-connect Cargo suite: 95 passed,
2 ignored fixtures; native shell suite: 576 passed with 2 ignored fixtures, including
real pointer/text focus during an outstanding passive read. An additional parallel
rerun hit the existing bash-completion test's 20-second prompt deadline; its isolated
rerun passed in 0.41s. Final sequential shell run: 576 passed, 0 failed, 2 ignored (68.90s).
Release shell/daemon build: passed (8m06s). cargo fmt/rustfmt are not installed in this environment; the same
formatter command fails in the unchanged baseline. git diff --check is clean.


Final input-height follow-up: both path and USB fields use ui::CONTROL_HEIGHT
(30px). All 29 CONNECT tests passed; the final release shell rebuild passed
(6m39s). The full release shell was launched under an isolated X server/private
D-Bus with its own settings, runtime socket and disposable /bin/sh. CONNECT and
Send file were opened by pointer input; an absolute test path and Enter stayed
inside CONNECT, while the terminal remained at an empty prompt. The final 30px
field was visually inspected. Both live managed desktop terminal IDs remained
unchanged; the real compositor/graphical shell was never restarted.

Deployment: source is committed/pushed and the main checkout is fast-forwarded.
`sudo -n install` was refused because an administrator password is required.
The installed shell has a different digest from the tested release; therefore
this verification does not claim that the user's live desktop has loaded the
new UI. Install target/release/des-ui and target/release/des-connectd into
/usr/local/bin with normal sudo authentication, then Super+Shift+R loads the
shell while managed terminal processes reconnect. No logout is required.
