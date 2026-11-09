# Disposable startup recovery — 9 November 2026

Status: early failure, startup timeout and final Cage fallback passed in an
isolated, headless environment. No production code or installed files changed.
This supports beginning a DesOS VM prototype. Physical display-manager/DRM
recovery and target-distribution compatibility remain separate acceptance cases.

## Method and identities

The user approved proceeding with disposable failed-startup recovery testing.
The existing `des-ui-session` launcher and `des-ui-startup` watchdog were copied
unchanged from frozen candidate `d462986a5100b0c75401b7710c7642efd32ee44f`.
The watchdog retained its production 15-second deadline and process-group cleanup.

Bubblewrap provided private user, process, mount, network, IPC and UTS namespaces,
a fresh home/runtime directory, basic synthetic devices and a private D-Bus.
The host home, host session sockets, system bus, DRM/input devices and systemd
manager were unavailable. The candidate/previous executables and system libraries
were mounted read-only; only the dedicated evidence directory was host-writable.
Six namespace identities were explicitly compared with the host in the final
fallback case. This isolated the launcher's user-wide portal/music cleanup from
the owner's actual desktop processes.

Headless Cage supplied a Wayland display with software rendering. Test adapters
selected `winit` instead of physical DRM for successful attempts and injected
failures in the other attempts. The real previous shell/compositor were used
for recovery, and the real candidate shell/compositor for final Cage fallback.
The adapters are test fixtures, not replacement installed executables.

| Executable | SHA-256 |
|---|---|
| Candidate shell | `cbe1114cf7182a3f5e4edba33ef325c1174b27a8c777fdc4a66a0f9b0598947b` |
| Candidate compositor | `ed5cf9046ecfe0c8011b37938453d8e8a42f0c197c9272c1c2ca81f2d9242e57` |
| Previous shell | `5dcf3db89480e5014fd4d6d6c9046ec60b27b5ccee1950a80e890f00cbfdfa0e` |

The full previous compositor identity, commands, fixture sources, output logs,
result JSON and evidence hashes are retained privately under
`target/stabilization/2026-11-09/disposable-recovery/summary.json`.

## Observed cases

| Case | Observed result |
|---|---|
| Previous-build baseline | Real previous desktop acknowledged initialized readiness at 4.63s, answered connected state and exited 0 at 18.29s, beyond the startup deadline |
| Early candidate crash | Injected exit 42 led directly to the previous build; its real shell acknowledged readiness at 4.53s; connected state and a rendered desktop were observed; clean exit 0 at 18.58s; no Cage attempt |
| Candidate startup hang | Real candidate compositor ran a shell fixture that never acknowledged readiness; watchdog reported timeout after the full 15s, terminated both SIGTERM-resistant shell processes and selected the previous build; real previous shell acknowledged readiness at 19.20s; clean exit 0 at 34.58s |
| Both native attempts fail | Injected candidate exit 42 and previous exit 43 led to exactly one Cage attempt; real candidate shell hash, connected/unlocked state and a rendered desktop were observed; clean exit 0 at 18.60s |

Native-attempt acknowledgements had the expected `ready\n` contents, mode 0600
and private parent mode 0700. The final Cage path intentionally has no startup
watchdog or readiness acknowledgement; its evidence is the actual shell identity,
connected control response and captured frame. A screenshot timer on the real
compositor ended successful disposable sessions cleanly after 18 seconds.
Recovered previous and final Cage frames were visually inspected.

The existing session test suite also ran: `python3 -m unittest discover -s
session/tests -v` — 21 tests passed in 6.008 seconds. No Rust/runtime input changed,
so the previously verified release binaries were reused.

## Probe corrections and limits

Two setup failures are retained with the evidence: the first sandbox omitted
`/etc/alternatives`, which this host's graphics-library symlinks require, and
the first fault-injection wrapper contained a quoting error. A subsequent Cage
probe incorrectly required the acknowledgement used only by native attempts.
That probe was corrected to check the documented untimed Cage behavior. These
were test-environment/probe corrections; no production recovery repair was needed.
Expected unavailable-service messages and software-renderer warnings remain in
the logs.

Host continuity checks confirmed the same boot and original shell/compositor PIDs
with matching hashes, a connected/unlocked desktop, and all nine original
containers still running with unchanged restart policies. No host logout,
shutdown, installation, authentication change or device takeover occurred.

The tests establish launcher ordering, real previous-build startup, timeout
cleanup and a working software-rendered Cage route. They do not exercise DRM
seat release/reacquisition, VT switching, greetd return, login-loop behavior under
a display manager, or Ubuntu library compatibility. Those checks belong in the
DesOS guest and later controlled hardware acceptance before replacing the host.
