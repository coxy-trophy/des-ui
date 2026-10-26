# Collapsed rails and app boundaries

The user wants wider collapsed side panels, the clock rotated as a whole with glyphs facing right and hours at the bottom, and native applications clear of the rounded frame.

1. Confirm running shell/compositor versions and inspect shared placement. Verify a matching pair with a real native client in an isolated nested desktop; preserve live sessions.
2. Regression-test that collapsed cards provide at least 44 logical pixels of width and preserve the 8px channel and app inset. Increase the shared rail reservation from 36 to 56px.
3. Rotate the entire leading-zero regional clock counterclockwise by 90 degrees, matching the supplied bottom-to-top 18:05 example. Retain 12-hour period and minute updates. Inspect the native result.
4. Update design context, run workspace verification, review, commit and push. Keep captures local.

Completed: version mismatch confirmed read-only; matched native open/collapsed/short previews inspected; regression tests and workspace checks passed; review resolved clock fit; raw evidence kept local.
