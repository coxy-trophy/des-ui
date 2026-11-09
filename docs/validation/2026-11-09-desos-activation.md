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
