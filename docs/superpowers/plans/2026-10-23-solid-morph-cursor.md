# Solid morph cursor implementation plan

**Goal:** Implement the user's approved solid, notch-free pointer family and shape-only interaction states.

**Spec:** User-approved comparison archived at docs/design/solid-morph-cursor-reference.png. Narrow convex triangle for pointing, fuller rounded wedge for links, slender insertion needle for text, compact lozenge for grab, directional spindle for resize, slowly turning seed for working. Flat off-white with dark contrast outline. No decorative accents, trails, halos or notches. A fixed hotspot anchors transitions.

**Architecture:** Compositor cursor-shape owns routing, code-drawn geometry, bounded high-DPI raster cache and interruptible 120ms morphs. Both hardware and development-window render paths consume this owner. Application-supplied cursor surfaces and hidden cursors retain their semantics. Existing animation/Reduce motion preferences suppress morphs and busy rotation. No new dependencies or raster runtime assets.

**Scope rulings:** Preserve copy, forbidden, precision, direction-specific resize and zoom meanings rather than replacing every protocol name with the default. Clients supplying their own cursor images retain them; no guessing interaction meaning from client image pixels. No installation or restart while the user cannot log out. Preserve all earlier uncommitted audit and gesture changes.

- [x] Implement shapes, routing, morph state, bounded raster cache; observe regression tests fail then pass.
- [x] Integrate both backends and capture; hide duplicate host cursor in development window, preserve custom/hidden cursor paths and coordinate transforms.
- [x] Check small-size/high-DPI silhouettes and contrast with exported contact sheet; update DESIGN, UX and native acceptance checklist.
- [x] Run focused/workspace checks and build, review final changes, record sandbox and native acceptance limits.
