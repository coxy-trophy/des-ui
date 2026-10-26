# Collapsed rails and native frame placement — 26 October 2026

The collapsed side cards now provide 44 logical pixels of width, from a shared 56px reservation. The 8px channels and 4px native-client inset remain owned by common::frame::Layout. The left clock rotates the complete leading-zero regional HH:MM string counterclockwise by 90 degrees: hours begin at the bottom and minutes end at the top, with glyphs facing right. The 12-hour period is retained; short rails scale the clock to fit. Each rail has an independent interaction identity.

## App overlap diagnosis

Read-only checks established that the live compositor executable differed from the newly installed compositor, while the shell had already restarted on the newer build. This combined old window placement with the newer frame. No live compositor or user application was restarted. The installer now explicitly reminds users to activate frame changes by logging out and back in, switching shell and compositor together.

A private Xvfb nested desktop ran the matching debug compositor and shell with a real GTK3 Wayland client and client header bar. At 1280×900, the app occupied x=64..1215 with both side cards collapsed (1152px width), and x=248..1031 with both side panels open (784px width). Both retained the 4px center inset and 8px panel channels. Native screenshots were inspected; raw captures remain local. This verifies native placement in the nested backend, not a replacement of the active DRM session.

## Validation

- Added regression coverage for minimum collapsed-card width, preserved channels/inset, and independent clicking of simultaneously visible rails.
- Existing regional clock tests retain leading zeros, midnight and the 12-hour period. A native 1280×200 preview confirmed the scaled 12-hour clock fits its 156px rail.
- `CARGO_BUILD_JOBS=1 nice -n 10 ./scripts/verify.sh` passed: 80 Python tests, workspace tests and debug build. Final `cargo test --workspace --locked --no-fail-fast` after review passed 762 Rust unit/integration tests and one doctest.
- Strict design audit: zero findings. Installer shell syntax and git diff checks pass.
- Source review caught insufficient height for a rotated 12-hour clock; the clock now measures and scales its actual text width. Follow-up review found no further issues.

Activation: run the installer, then log out and in when ready. Restarting only the shell leaves an older compositor using its previous placement rules.
