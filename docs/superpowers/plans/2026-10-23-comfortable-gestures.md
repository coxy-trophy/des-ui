# Comfortable gestures implementation plan

**Goal:** Make webcam pointing and clicking easier with less sustained finger tension.

**Authorization:** User approved proceeding after discussion of relaxed pointing, gentle clicks, separate right-click, optional drag locking, adjustable sensitivity and clear feedback. Preserve previous audit fixes in this workspace. No installation or session restart.

**Architecture:** The hands gesture state machine owns recognition and button lifetime; common InputSettings owns persisted preferences; compositor owns process configuration, native pointer delivery and sharp noninteractive feedback. Settings reuses shared controls, materials and typography.

**Behavior:** Relaxed non-fist pointing; thumb–index tap clicks, thumb–middle tap right-clicks without a hold timer. In release-pinch mode, tap then pinch again and move to grab; relax fingers to carry the drag and pinch again to drop. Two taps without movement remain a double-click. Optional hold-pinch mode uses conventional press/release. Aim steadies during clicks. Lowering/loss, fist, unsafe poses and tracking edges release held input and reset anchors; reacquiring an already-pinched hand never clicks. Two-finger scrolling keeps smoothing and has independent speed. Preferences are finite, bounded and reload without leaking grabs.

- [x] Gesture state machine: red/green tests for click/right-click, double-click/drag lock, hold mode, tracking safety and jitter.
- [x] Preferences and Settings: persisted pinch sensitivity, scroll speed and drag lock; finite parsing and existing native controls.
- [x] Integration: pass matching preferences to des-hands, restart safely on changes, scale scroll independently, refresh instructions/recordings.
- [x] Feedback: static accent confirmation near pointer, held marker, cleared on rest/stop/lock, both render backends and output coordinates.
- [x] Verification: focused regressions, workspace tests/build, session and packaging tests, script syntax, diff check and review. Record native camera/UI acceptance limits.
