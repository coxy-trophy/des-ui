# Native screensavers implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for independent native drawing, settings, and compositor tasks; verify the assembled flow in isolation.

**Goal:** Bring the seven chosen gallery scenes to the real desktop.

**Architecture:** The existing lock protocol stays authoritative. The compositor distinguishes idle lock requests, approves saver display only after lock while still idle, and swallows the wake input. The shell shares the same native renderer between the locked outputs and a separate Settings preview.

**Spec:** [Gallery and idle policy study](../../prototypes/idle-saver/README.md). Ship Silk Fold · Ink, Signal Contour, Terminal Weave, Night Transit, Rising Lines, Event Horizon and Polar Veil. Keep current lock/display-off/suspend settings and default the new preference to Off. A saver never authenticates or unlocks.

- [x] Implement bounded native scenes in shell/src/screensaver.rs using the boot wordmark. Draw only when visible; schedule at up to 24 fps, slower on battery, and use a static scene with motion disabled.
- [x] Store the scene under screensaver in the shell configuration. Use the existing Session settings controls and add Preview. Explain idle-lock and display-off interactions.
- [x] Extend common IPC with IdleLockRequested, ShowSaver, HideSaver and Saver(bool). Compositor: accept a saver only after an outstanding idle lock and confirmed lock, dismiss on local input/display-off/unlock, and swallow the wake input sequence.
- [x] In the shell, request ShowSaver after idle lock confirmation; clear stale intent on manual lock, sleep and disconnect. Draw the approved saver on the primary lock surface and dedicated render-only secondary surfaces. Preview is local, exits on input, and returns to Settings.
- [x] Verify scene output visually on an isolated display, run targeted lock/IPC and configuration tests and compile shell/compositor. Keep the live session running. Commit and push to the existing work branch.
