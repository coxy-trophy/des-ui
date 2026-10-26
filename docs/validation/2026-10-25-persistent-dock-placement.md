# Persistent dock positioning —2026-10-25

User correction: adopt mobile positioning too; no automatic hiding. Mobile is a read-only reference owned by another session.

The dock is permanently placed below Files/Chat and above status, centered in the content column rather than the whole output. Shared68px strip/88px band constants reserve work space. Files/Chat and Quick controls end above the lane; on-screen keyboard shifts the lane upward. Primary fullscreen reserves dock/status and secondary displays reserve dock space. Lock and screenshot picking retain suppression.

Visibility is independent from keyboard ownership. Super+D focuses the visible strip. Escape, activation, native outside clicks (including already-focused app and divider resize) release ownership using explicit per-display DockKeyboard and DockDismissed messages. Crops remain present. Pointer grabs, other overlays, locked/picking cases and other-display ownership are guarded. Idle dock has no permanent60fps repaint loop.

Review repaired two regressions: fullscreen flags had compared against raw output rather than new reserved unframed area; keyboard release had depended on clearing dock crop toNone. A further divider early-return bypass was repaired by moving guarded release before resize handling. Each compositor regression was exercised red/green and independently reviewed.

Evidence:

- `/tmp/desui-persistent-dock-red.log`: new persistent-startup test failed before implementation.
- `cargo test --offline -p des-ui dock:: -- --test-threads=2`:14passed; `/tmp/desui-persistent-dock-tests.log`.
- `cargo test --offline --locked --workspace --no-fail-fast -- --test-threads=2`: **741unit tests and1doctest passed**, no failures/skips; `/tmp/desui-persistent-dock-workspace-tests.log`. Includes64common,152compositor and463shell tests.
- `cargo build --offline --locked --workspace`: **passed without warnings**,1m08s; `/tmp/desui-persistent-dock-build.log`.
- Strict UI static audit zero findings; `/tmp/desui-persistent-dock-audit.json`. `git diff --check` passed.
- Isolated eframe/Xvfb capture1100×760, with asymmetric left/bottom panels, temporary HOME/config/state/runtime, private D-Bus, keyring disabled and no provider credentials or host compositor. No dock shortcut or edge hover was sent. Fresh screenshot timestamps verified. Dock is visible, centered in column beneath Files/Chat, with no content overlap. `2026-10-25-persistent-dock.png` is a crop of this actual native frame; `/tmp/desui-persistent-dock-native.log` has expected unavailable desktop integration notices.

No mobile code/build/generated files changed. No running desktop restart, installation, commit or push. Real native-app/per-output/hardware/input acceptance remains in docs/UNTESTED.md; the isolated shell capture does not certify installed compositor behavior.
