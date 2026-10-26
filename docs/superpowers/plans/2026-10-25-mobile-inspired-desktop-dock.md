# Mobile-inspired desktop dock

**User direction:** Adopt the dock from des-ui-mobile on desktop. Mobile is read-only because another session owns it. All edits stay inside des-ui.

**Reference inspected:** des-ui-mobile/common/src/com/desui/launcher/AppsPane.java, dock construction and rebuildDock; Ui.java press animation. Reference uses one horizontal strip,68px height,60×56 targets,44px installed icons,12px shared floating material radius, accent badges,94% press scale with90ms press/160ms release, and horizontal overflow. No headings or permanent icon labels.

**Desktop adaptation:** Replace wrapping/vertical dock layout with centered single-row horizontal overflow. Use the mobile geometry/press feedback and calmer indicators. Preserve autohide/keyboard summon, shared material/theme/reduced motion, Terminal/All windows/Applications entries, pins/reordering, running/active/urgent indicators, grouped-window chooser and per-display behavior. Names remain tooltips and accessible labels; selected keyboard item scrolls into view. Add quiet overflow affordances without intercepting clicks.

**Files:** shell/src/dock.rs owns geometry, painting and interactions; DESIGN.md/UX-CONTRACT.md document revised owner; validation/UNTESTED records actual checks. No mobile builds, formatting, generated files, changes or commits.

**Verification:** First assert fixed-height single-row geometry and horizontal overflow, then implement; test narrow/wide screens, keyboard reach to last entry, touchpad/mouse wheel overflow, press feedback and reduced motion. Run dock tests, workspace build and static audit; request independent read-only review. Native installed-session visual acceptance remains explicit.

Completed: adopted one-row mobile geometry/press feel on desktop, preserving existing desktop behaviors. Layout and wheel regressions observed red then repaired;14dock tests and build pass. Independent review approved the repair. Native isolated Xvfb capture inspected; actual dock crop stored in docs/validation/2026-10-25-mobile-inspired-dock.png. Mobile remained read-only throughout. Detailed evidence and hardware limits in matching validation Markdown.
