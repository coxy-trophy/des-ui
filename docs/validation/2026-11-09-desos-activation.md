# DesOS candidate activation — 9 November 2026

The user explicitly approved installing the verified candidate and beginning
controlled logout/login and suspend/resume tests. Candidate code commit:
`d462986a5100b0c75401b7710c7642efd32ee44f`.

## Installation observed

- All eight staged executable hashes were checked against the candidate manifest.
- The existing source installer ran in a separate user terminal; administrator
  authentication stayed in that terminal. Installer exit status: 0.
- All eight installed executables matched the candidate hashes, were root-owned
  and mode 0755. The installed virtual-display helper matched its source bytes.
- The previous shell/compositor files matched the pre-installation hashes and
  were root-owned, mode 0755. The previous shell contains the initialized-startup
  readiness protocol required by fallback.
- The display manager was active and running. The current graphical session was
  active Wayland on seat0. Its control socket answered ping and state.

Evidence: `target/stabilization/2026-11-09/activation-install.log`,
`activation-check.log`, `activation-installed.json` and `activation-before.json`.

## Fresh-login handoff

The active agent terminal is a descendant of the current desktop's managed
terminal server. Ending this graphical session can also end this agent process.
The user's login password must be entered directly at the login screen.

A private handoff under
`target/stabilization/2026-11-09/hardware-activation/` records only session identity,
boot identity, bounded non-title desktop state, candidate identity and recovery
hashes. No raw application titles, screenshots or credentials are recorded.

The prepared one-shot logout helper refuses to act if the recorded session,
boot identity, candidate binaries or recovery pair changed. It targets only the
recorded graphical session, never the whole user account. Its receipt records a
request/result, not proof that greeter return or login succeeded.

After the approved logout, sign back into DEs-UI and return to this conversation.
The next agent checks the receipt and fresh session, matches running shell and
compositor hashes to `candidate.json`, checks startup/readiness and then proceeds
with lock/unlock and supervised suspend/resume. All those hardware outcomes
remain Pending until observed. Do not rerun the installer merely to resume this
handoff: installation is already verified and the recovery pair is preserved.

## Fresh login and lock/unlock observed

The owner returned after signing in. The one-shot logout receipt returned exit 0.
The old graphical session ended and a new Wayland session initialized on the
same boot. Both running executable hashes matched the installed candidate.
The compositor used the DRM backend; the shell's initialized-startup marker had
the expected contents, owner, mode 0600 and private parent mode 0700. This is
logout/login evidence, not a cold-boot pass.

An actual lock request was acknowledged. While the desktop remained locked,
ping succeeded and launcher/workspace mutations returned `error locked`.
The owner's normal unlock was observed. The owner explicitly confirmed that
the lock screen fully covered the desktop and typing worked normally afterward.

Evidence: private `fresh-login.json`, `lock-events.jsonl`, `lock-check.log` and
`lock-result.json`. Only bounded non-title state and control outcomes are saved.

## Suspend/resume preparation

The laptop was on AC power at 98% battery and charging. The kernel advertised
suspend with deep sleep selected. The candidate held its DEs-UI sleep delay
inhibitor, and logind allowed suspend from the test's user-service context.
The configured lock-on-sleep setting used its enabled source default.

The prepared observer subscribes to logind PrepareForSleep, samples boot-time
versus monotonic clock differences and bounded desktop lock state, and checks
that the same candidate processes survive. It performs only one approved
suspend. Waking and entering the lock-screen password remain owner-operated.
Suspend/resume and post-wake visual acceptance are Pending until observed.

## Suspend/resume observed

The approved suspend request returned exit 0. The observer received logind's
PrepareForSleep true/false pair and measured a 7.641-second increase in the
boot-time versus monotonic-clock offset, confirming actual sleep on this boot.
The desktop was observed locked before sleep and after resume. The same shell
and compositor PIDs survived and still matched the candidate hashes.

The bounded observer ended while the owner had not yet unlocked. A later sample
after the owner returned verified the same session and candidate processes,
connected compositor state and a normal unlocked desktop. The original observer
result is retained unchanged; `post-resume.json` supplies the later unlock.

The owner explicitly confirmed that the lock screen appeared before desktop
content after wake and that display, keyboard, touchpad and Wi-Fi worked normally.
NetworkManager subsequently reported connected/full. PipeWire and WirePlumber
were active, and the default output/input levels were readable. This does not
certify audible playback or microphone capture; neither was exercised.

Evidence: private `suspend-result.json`, `suspend-events.jsonl`,
`suspend-check.log`, `post-resume.json` and `post-resume-services.json`.

## Cold-boot preflight (before authorization)

At this checkpoint, cold boot, failed-startup recovery, authenticated VT recovery, lid behavior and
the broader application/device/performance cases remain unverified for this
candidate. Successful logout/login and suspend/resume do not close those cases.

Preflight found kernel `7.1.5+kali-amd64`, with that kernel and the older
`6.19.14+kali-amd64` image present. GRUB's default index was 0 and its timeout
5 seconds. No kernel or bootloader configuration was changed. Logind reported
power-off available. The only ordinary login session belonged to the owner.

There were **nine running Docker containers** at this checkpoint. A full shutdown
would stop those containers as well as current applications and terminal jobs.
No cold-boot shutdown had yet been scheduled or requested by the test helpers.

The proposed next test, when the owner is ready, is one normal system shutdown,
physical power-on, sign-in to DEs-UI, and verification of a changed boot ID,
running candidate hashes, readiness acknowledgement and service recovery. Keep
the recorded previous shell/compositor pair available; if normal startup fails,
use the existing fallback or authenticated text-console recovery. Do not install
or rebuild the candidate again merely to perform that boot test.

## Cold-boot shutdown authorized

After being told that a shutdown would stop nine running Docker containers and
current applications, the owner explicitly instructed "do it". Preflight again
verified all installed candidate hashes, the previous core recovery hashes and
readiness support, the active display manager and the owner's graphical session.
Logind allowed normal power-off from the test's user-service context.

The private `cold-boot-handoff.json` records the old boot/session identity,
candidate and recovery identities, and container IDs/restart policies without
container names, environment variables or credentials. No container policy is
changed. The guarded one-shot helper uses normal `systemctl poweroff`, leaving
service shutdown and filesystem flushing to the operating system.

After the machine fully powers off, the owner physically powers it on, signs into
DEs-UI and returns. Verify a new boot ID, kernel selection, candidate process
hashes, initialized readiness, network/audio services and container recovery
against the recorded policies before claiming a cold-boot pass. A shutdown
request receipt alone is not cold-boot evidence. Do not repeat shutdown on resume.

## Cold boot and service recovery observed

The owner returned after the approved power-off/power-on and login handoff.
The normal shutdown request returned exit 0. Boot identity changed from
`232e5817-1736-4a6f-abd9-2e2bcf9f0e8b` to
`0b582d7e-281b-46b6-8415-046b51eb8590`; the selected kernel remained
`7.1.5+kali-amd64`.

The active seat0 session was Wayland, with the candidate compositor using DRM.
Running shell/compositor hashes and all eight installed executable hashes
matched the frozen candidate. The previous recovery pair also retained its
recorded hashes. The initialized-startup acknowledgement had the expected
contents, owner and private file/directory permissions. The control socket
answered ping, with a connected, unlocked desktop and one screen.

The display manager and Docker were active; Docker remained enabled.
NetworkManager reported connected/full, and PipeWire and WirePlumber were active.
All nine original container IDs were running with unchanged `unless-stopped`
policies and restart counts of zero. All four configured health checks reported
healthy; the other five containers had no health check configured. This verifies
process/service recovery, without certifying application transactions or audible
playback.

Evidence: private `cold-boot-result.json`, alongside the pre-shutdown handoff and
shutdown receipt. No additional visual boot acceptance answer was collected.
This closes the cold-boot/login case on this laptop with this kernel and
candidate; failed-startup recovery, authenticated VT recovery, lid behavior and
the broader device/application/performance cases remain pending.
