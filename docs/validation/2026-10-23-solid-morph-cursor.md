# Solid morph cursor validation

Implemented the approved notch-free solid pointer family, archived at docs/design/solid-morph-cursor-reference.png. Actual code-rendered light/dark contact sheet: 2026-10-23-solid-cursor-contact.png.

Named cursors use shared vector silhouettes, 120ms interruptible morphs, a fixed logical hotspot, bounded high-DPI rasters and static reduced-motion states. Pointer/link/text/grab/resize/working styles follow the approved family; precision, copy/move, forbidden and zoom semantics remain distinct. Both display backends and pointer-inclusive capture share the effective resize status. Client cursor surfaces and hidden pointers are preserved. The development window draws the desktop cursor with its host cursor hidden.

The DEs-UI fallback theme exports the same canonical artwork at 24/32/48 for older clients. Installation and Debian packaging include it; launchers provide defaults and a search path including /usr/local/share/icons. Explicit legacy cursor environment choices are preserved. Legacy cursor images change states statically; they cannot use compositor cross-state morphs.

Verification:

- `cargo test --workspace --locked --offline --no-fail-fast -j2`: 681 passed, nine known environment failures (six Unix-socket permission failures, three private-tmux startup failures). The workspace test command exits unsuccessfully; these are not skipped tests.
- `cargo build --workspace --locked --offline -j2`: passed without warnings.
- 15 session tests and six packaging tests passed. Twenty shell scripts passed syntax checks; git diff check passed.
- Thirteen focused cursor tests cover routing, connected silhouettes/contrast, small insertion-stem visibility, fixed hotspots, retargeting, reduced motion, raster/cache identity and bounds, theme scaling and opposite-direction transitions. Three theme tests validate container metadata, aliases and equality with the shared raster.
- System libXcursor loaded all 89 exported cursor files at all three sizes and resolved DEs-UI/left_ptr via XCURSOR_PATH. Export ran headlessly without Wayland.
- Review found a pointer-inclusive capture retargeting mismatch; capture/display now share one effective-status helper and its regression test. Final review found no further material issues.

Logs: /tmp/desui-solid-cursor-tests.log, /tmp/desui-solid-cursor-build.log, /tmp/desui-solid-cursor-session.log, /tmp/desui-solid-cursor-packaging.log. Source snapshot: /tmp/desui-solid-cursor-source.json.

No installation, logout or running-session restart occurred. Native visual acceptance, client interoperability, screenshots, multiple displays and actual compositor CPU/GPU behavior remain open in docs/UNTESTED.md. The user deferred logout; ./dev.sh provides a separate development-window preview where available.
