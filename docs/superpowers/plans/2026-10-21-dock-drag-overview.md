# Dock and drag overview implementation

Approved specification: ../specs/2026-10-20-dock-drag-overview-design.md

1. Extend bounded common protocol with window chooser and drag overview modes and compositor capability announcement. Preserve existing overview formats.
2. Replace dock cycling with explicit compositor window previews, add All windows, and distinguish running/pinned state. Preserve existing pins and overlays.
3. Add regional vertical clock to left collapsed rail.
4. Add compositor-owned drag session/dwell state, native Wayland and shell-file lifecycle hooks, cancellation and safe real-surface handoff. Reuse live preview rendering without replacing data grabs.
5. Add persistent opt-out in Settings and update design/interaction documentation.
6. Verify protocol, grouping, layout, timing, cancellation and regressions; run offline workspace tests/build. Record native compositor/browser acceptance limits.

Execute inline in the existing workspace, preserving all prior changes. No installation or session restart.
