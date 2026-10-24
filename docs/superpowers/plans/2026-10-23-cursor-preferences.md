# Cursor color and size implementation

**Goal:** Finish the approved pointer with persisted color and size controls before discussing the user's next idea.

**Design:** Settings → Mouse & touchpad owns Cursor size (16–64px, default24), an opaque RGB picker with Apply/Reset and exact shared-arrow previews on light/dark backgrounds. Named cursors update live, keeping the click anchor and in-progress morph. Contrasting outlines protect visibility, including black/white fills. Configured fallback images update off the rendering thread; older applications may need reopening because they cache images.

**Architecture:** InputSettings owns bounded size and RGB; cursor_shape Preferences adapts it to geometry/raster/cache. A coalesced background theme worker publishes complete private cache generations atomically while preserving the previous usable generation on failure. Installer/package themes remain default, multi-size artwork. Configured per-user themes label1×–4× images using the inherited legacy nominal request, while rendering the chosen logical size at each density. Transparent padding is cropped with hotspot adjustment and aliases share image files. This preserves size for clients that retain an old nominal request and apply an output buffer scale. Launchers prepend the private cache only for DEs-UI; explicit other themes remain untouched.

- [x] Config and native Settings controls, draft/apply/reset and narrow layouts; regression checks.
- [x] Color/size-aware raster, exact anchors, bounded cache and morph continuity; regression checks.
- [x] Startup/reload delivery, coalesced private fallback publication, launcher paths; regression checks.
- [x] Full checks/build and final review; keep installed-session acceptance open because the user cannot log out. No installation or restart.
