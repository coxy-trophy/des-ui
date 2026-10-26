# Persistent mobile-position dock

User correction: adopt mobile positioning too; dock must not hide. Mobile remains read-only, owned by another session.

Place desktop dock permanently at the bottom of the central app column, below Files/Chat and above the status strip, with the same10px gap as the mobile layout. Reserve an88px band (68px dock plus two10px gaps) in shared app/terminal geometry so content never sits behind it. Secondary displays reserve their bottom88px without primary status; primary fullscreen keeps the dock/status band too. Files/Chat end above this band, and on-screen keyboard shifts the dock above itself.

Remove reveal dwell, hide timers, slide-out and obsolete edge-only input. Super+D selects/focuses the visible dock; Escape, activation and outside click release keyboard selection without hiding. Keep hover/press/reduced-motion, horizontal overflow, preview/pin/context behaviors and lock/capture suppression. Idle dock must not force60fps repainting.

Root owns shell dock/runtime and docs. Geometry agent owns common/lib.rs and compositor policy/toast. Tests: persistent without hover/on selection dismissal; centered in supplied column; common workspace/files/dock separation, fullscreen/secondary reservation, input region removal. Review, build and isolated native screenshot. No mobile edits/builds, installation, restart or commit.

Completed: permanent content-column placement; shared reserved band on primary/secondary/fullscreen; Files/Chat/Quick separation; no hover/hide timers or slide animation. Keyboard focus is independent from visible crop via DockKeyboard/DockDismissed, including native divider paths. Reviewed fullscreen and focus regressions repaired. Full741unit+1doc tests pass; workspace build passes; isolated native startup capture inspected without any dock reveal action. Mobile untouched. Validation: docs/validation/2026-10-25-persistent-dock-placement.md.
