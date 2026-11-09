# DesOS desktop stabilization candidate — 9 November 2026

Status: automatic stabilization checks passed; candidate ready for a controlled
installation and hardware acceptance session. Physical hardware checks, Ubuntu
compatibility and DesOS image creation remain Pending.

## Scope and provenance

The user approved stabilizing DEs-UI before preparing a personal DesOS image.
Desktop baseline: `50794d0eabf9dca7d2a4a1859a1222716f7b37b7`.
Code repairs: `b733ad7` (packaging/viewer) and `a2b8c57` (startup test/recovery).
Recovery formatting: `e37bb2e`; remote cleanup test stabilization: `d462986`.
Frozen executable-source candidate: `d462986a5100b0c75401b7710c7642efd32ee44f`.
Website baseline: `25f6fba`; corrected recovery copy: `0c1079e` and `309ba64`.

Work is performed in isolated desktop and website worktrees. The build host is
Kali GNU/Linux Rolling 2026.3, x86_64, kernel `7.1.5+kali-amd64`. A package built
here carries this host's native library dependencies. Build and validate again
in the selected Ubuntu LTS environment before using it in an Ubuntu-based image.

The local evidence bundle is retained at `target/stabilization/2026-11-09/`:
full verification/build logs, fault-injection harness/results, source snapshot,
release binary/archive hashes and Debian archive inspection. Private user
configuration, device keys and desktop screenshots are not inputs to that bundle.

The immutable source inventory is `frozen-source.json`: 505 tracked files from
the frozen commit, SHA-256
`9e99873b91d2b830bc63754ec7b733f6fa2382f450d4ee3ea4123dc0baad30e5`.
Later plan/result edits are documentation only; the build/runtime inputs remain
identical to that commit. Eight release binaries were copied out of the shared
Cargo cache into this worktree's own `target/release` before packaging. Their
hashes are checked against `release-inputs.json` and `candidate.json`.

## Repairs and focused verification

- Desktop deb now includes `des-connectd`, `des-connect-ctl` and the Python
  virtual-display helper. Native dependency discovery includes both Connect
  binaries. No Connect autostart service or permission change is introduced.
- The service resolves the viewer under both `/usr/local/share/des-ui` and
  `/usr/share/des-ui`, preserving explicit-override and development precedence.
- Dependency scanning failures abort desktop/music packaging even when the
  scanner emits plausible partial output before failing.
- Source uninstall removes the Connect executables, without removing user state.
- `/proc` inspection in the startup cleanup regression accepts ENOENT and ESRCH.
  A deterministic injected ESRCH failed before the fix and passed afterwards;
  injected permission denial still produces an error rather than a false pass.
- Recovery documentation removes the rejected `des-ui --unlock` command.
  Source inspection also established Ctrl+Alt+Backspace is ignored while locked.
  The documented fallback uses authenticated text-console/loginctl recovery,
  identifies the affected graphical session, and explains unsaved-work loss.
- The first full gate exposed a separate fixture-startup race in
  `remote::tests::timeout_kills_descendants_holding_the_pipes`: under suite load,
  the 150ms budget expired before the fixture wrote its PID. A deliberate 250ms
  setup delay reproduced the failure. The cleanup test now allows 2s, while its
  separate short-deadline regression and the production timeout are unchanged.
  The full Rust suite then passed, including this delayed-startup case.

Focused checks already observed: 7 packaging tests passed; Connect tests 101
passed and 2 ignored; session tests 21 passed; website production build passed.
Five new package tests first failed with seven assertions covering missing tools,
helper, swallowed dependency errors and incomplete uninstall.

Complete test results: 1,050 Rust tests passed, 8 ignored, zero failures; Python
suites passed 21 session, 6 packaging and 75 companion tests. The first failed
gate is retained separately rather than omitted from the evidence.

## Complete candidate checks

| Check | Result |
|---|---|
| `./scripts/verify.sh` — script syntax, all Python/Rust tests, debug workspace build | Passed, exit 0; 1,050 Rust / 102 Python passed, 8 Rust ignored |
| Locked release workspace build | Passed, exit 0; optimized build completed in 15m 10s |
| Actual Debian archive build and inspection | Passed, exit 0; four archives; binary/script payloads, helper, owners, executable modes and no Connect autostart verified |
| Release CLI help checks, without starting sessions/services | Passed, six programs, all exit 0 |
| Website production build | Passed, exit 0, at website `309ba64` |
| Fresh independent code review | No blocking findings; reviewed ranges and subsequent checks described below |

The independent review covered desktop `50794d0..e37bb2e` and website
`25f6fba..0c1079e`. The later change to the remote cleanup test was reproduced
RED then passed within the complete suite. The website's general recovery
statement was also narrowed to "While unlocked", then its production build
reran successfully. Review disposition is retained in `review.txt`.

Native dependency discovery emitted warnings about this host's libc loader
diversion and analyzing binaries outside the Debian staging directory. These
warnings are retained in `package-build.log`; they are not a clean target-distro
dependency certification. CLI checks passed on this host. Rebuild and validate
native dependencies in the selected Ubuntu environment before image use.

The four archives retain the project's existing version `0.1.1` for local
inspection. They have not been installed or published. Assign a distinct release
version and rebuild on the target base before distributing an update through apt.

| Local archive | SHA-256 prefix (full value in `candidate.json`) |
|---|---|
| `des-ui_0.1.1_amd64.deb` | `fccce255409b` |
| `des-ui-music_0.1.1_amd64.deb` | `8783f0be7853` |
| `des-ui-greeter_0.1.1_amd64.deb` | `8fc99afe0afb` |
| `des-ui-plymouth_0.1.1_amd64.deb` | `8bd061a895b0` |

## Focused hardware acceptance gate

Test one exact candidate on this laptop first. Record source/binary hashes,
kernel, GPU driver, display setup and observed outcomes. Preserve a known-working
boot/login option. Schedule operations that end or suspend sessions only after
the owner saves work and identifies sessions that must be preserved.

| Case | Acceptance condition | Status |
|---|---|---|
| Cold boot, login and logout | Desktop initializes; logout returns to greeter; a subsequent login works | Partial: actual logout/login and initialized candidate startup passed; cold boot pending |
| Failed startup and update recovery | Failed candidate reaches the known-working fallback; no login loop | Pending |
| Lock and shell failure/restart | Applications remain hidden and unfocused until password authentication | Partial: actual lock/unlock and locked control policy passed; forced shell failure/restart pending |
| Suspend/resume and lid | Lock precedes exposed application content; display/input recover | Partial: actual suspend/resume passed with lock observed before/after sleep and owner visual confirmation; lid pending |
| Authenticated VT recovery | Text-console access works; only the selected disposable graphical session ends; greeter returns | Pending |
| Terminals and daily applications | Shell restart preserves required terminal jobs; browser/editor and Wayland/X11 focus work | Pending |
| Overlays, clipboard and Files | Open/dismiss operations return focus; keyboard copy/paste and file operations preserve expected data | Pending |
| Wi-Fi, Bluetooth and audio | Required connect/reconnect, device changes and controls work on actual devices | Partial: Wi-Fi recovered after sleep; audio services active; Bluetooth and audible playback/device changes pending |
| External displays | Required scale, hotplug, mode rollback and saved layout work with correct input coordinates | Pending |
| Performance and endurance | Record controlled idle/active CPU, memory, battery and repeated-use behavior on matching binaries | Pending |

Current candidate observations and receipts are described in
[the activation record](2026-11-09-desos-activation.md). Installation, logout/login,
lock/unlock and one supervised suspend/resume have now been observed; the owner
confirmed normal display/input/Wi-Fi and lock-first wake behavior. The broader
gate is not complete. No cold-boot shutdown is scheduled: nine running containers
were found, so a separate owner-ready shutdown window is needed.

Existing machine records are historical evidence for their own binaries, not
passes for this candidate. The older performance samples are active-session
observations, not a controlled idle baseline. Optional music, AI, gestures and
phone features can be validated separately from the essential desktop gate.

## Next decision

The activation sequence below was subsequently carried out for installation,
logout/login, lock/unlock and one suspend/resume; see the activation record.
Cold boot and the remaining acceptance cases are still pending.

The next proposed action is activation through the existing source installer
from the pinned worktree, using the staged release binaries. It requires the
owner's administrator authentication. First save or transfer active work and
identify terminal jobs to preserve. After installation, a full logout/login is
needed to activate the matching shell and compositor. Compare installed binary
hashes with `candidate.json`, then exercise login, lock/unlock, applications,
and a supervised suspend/resume. Authentication remains user-operated. Deliberate
startup-failure testing needs a separate disposable/recovery arrangement.

The current running shell/compositor hashes are recorded in
`installed-before.json`; they matched their on-disk installed files at the time
of inspection. No current session was restarted or ended.

DesOS live-image work follows the essential hardware gate;
upstream-kernel testing follows the established base-system baseline. The installed
desktop has now been activated through the separately approved test session;
the kernel, display-manager service choice and boot configuration remain unchanged.
