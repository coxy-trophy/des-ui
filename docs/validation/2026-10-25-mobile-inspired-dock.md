# Mobile-inspired desktop dock —2026-10-25

Reference inspected read-only: des-ui-mobile AppsPane.java dock construction/rebuildDock and Ui.java press feedback. All code edits are in desktop shell/src/dock.rs. No mobile source, build, formatting, deployment or generated artifact was touched by this task; another session owns that project.

Desktop adaptation: one centered68px-high strip,60×56 targets,44px installed icons, shared12px floating material radius; horizontal overflow with quiet chevrons. Press-in scales artwork to94% with90ms press/160ms release while hit areas remain fixed. Reduced motion disables animation. Existing autohide, keyboard launch, running/active/urgent/group indicators, context menus, pins/reordering, per-display grouping and grouped-window chooser remain.

Evidence:

- New mobile-layout test failed first on the old52px targets: `/tmp/desui-mobile-dock-red.log`.
- Manual wheel regression failed before repair (first item20→20): `/tmp/desui-mobile-dock-scroll-red.log`. Reveal is now gated to opening/navigation/viewport change, and dock-local one-axis wheel translation is enabled explicitly.
- `cargo test --offline -p des-ui dock:: -- --test-threads=2`: **14passed**, including fixed-height single row, overflow, manual wheel after keyboard opening, far-right keyboard reach, grouping/display identity and keyboard event isolation; `/tmp/desui-mobile-dock-tests.log`.
- `cargo build --offline --locked -p des-ui`: **passed**,50.60s; `/tmp/desui-mobile-dock-build.log`.
- Strict static UI audit **zero findings**, `/tmp/desui-mobile-dock-design-audit.json`; `git diff --check` passed.
- Real native eframe screenshot captured at1100×760 in isolated Xvfb/private D-Bus, with temporary HOME/config/state/runtime, keyring disabled, no provider keys and no host compositor connection. Ctrl+Shift+D opened the dock. Preview: `2026-10-25-mobile-inspired-dock.png` is a crop of that actual rendered frame. Fixture log has expected unavailable desktop-polkit integration; native capture completed. Initial harness focus attempt selected an unmapped X window; waiting for a visible window fixed the harness without changing application code.
- Independent read-only review found continuous keyboard recentering overrode manual scrolling; that repair, explicit wheel translation and hovered/keyboard tooltip anchors were verified. No remaining important finding in reviewed scope.

Installed compositor/hardware scaling, real multi-display input routing, live window preview/context interactions and wallpaper/reduced-motion acceptance remain in docs/UNTESTED.md. No user's desktop restart, app launch through the dock, installation, commit or push was performed.
