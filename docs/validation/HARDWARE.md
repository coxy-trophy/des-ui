# Hardware validation record

Copy this template per source snapshot and hardware configuration. A checked box
in UNTESTED.md requires an actual result, not an automated test of a policy function.

- Date and tester:
- Source snapshot: attach `python3 scripts/validation-snapshot.py` output
- Automatic checks: attach complete `./scripts/verify.sh` output and exit status
- Distribution and kernel:
- GPU and driver:
- Displays: model, connector, resolution, scale, orientation, arrangement
- Backend: DRM / nested (nested does not validate hardware)
- Recovery: known working login session and virtual-terminal access

| Case | Steps | Expected | Actual | Evidence | Result |
|---|---|---|---|---|---|
| Lock and shell restart | Lock; restart shell; attempt ordinary app keys; authenticate | Applications remain hidden and unfocused until password succeeds | | | Pending |
| Recording and lock | Start recording; lock; inspect saved recording | Capture stops before any locked frame | | | Pending |
| Suspend and resume | Suspend from menu and lid; resume | Lock appears before application content | | | Pending |
| Display rollback | Change scale, orientation, mirror/off; leave confirmation unanswered | Prior usable layout returns within 15 seconds | | | Pending |
| Saved display layout | Confirm layout; relogin; unplug/replug | Confirmed topology restored; input coordinates match display | | | Pending |
| Wi-Fi secrets | Join new open, WPA2, WPA3, WEP networks; retry wrong password | Correct connection or actionable error; no secret in process arguments/logs | | | Pending |
| Music IPC | Play; remain idle over two minutes; restart shell | Playback continues; state reconnects without repeated sign-in | | | Pending |
| Workspaces and X11 | Move/split/focus Wayland and X11 apps across displays/workspaces | Correct app receives keys; clipboard works | | | Pending |
| Accounts | On a disposable machine, create/edit/remove a disposable account | Only requested account changes; wrong admin password refuses operation | | | Pending |
| Recovery | Fail a test build at startup; then normal logout | Previous build/Cage fallback; logout returns to greeter | | | Pending |

Also run every applicable case in [UNTESTED.md](../UNTESTED.md). Record failures
with logs and reproduction steps; leave untested items Pending. Do account, power,
boot-splash and greeter changes on a disposable machine/account with recovery access.
Never include real passwords, tokens, or personal files in evidence.

Release criterion: automated checks pass on an unrestricted Linux host, and every
advertised hardware feature has evidence for the supported configuration. Source
changes invalidate the source snapshot; rerun affected hardware cases. Package
binaries need a fresh build and validation against that same snapshot before publishing.
