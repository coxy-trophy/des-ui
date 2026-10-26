# Overview preview repairs

User requirements: show native app content in workspace overview, keep pictures and selection outlines inside workspace columns with padding, and prevent underlying frame panels from intruding. Push verified changes to GitHub; preserve the live session and keep raw captures local.

1. Reproduce active and hidden native-window previews in a private nested compositor and inspect render ordering and thumbnail transforms.
2. Add geometry regressions for previews plus labels/selection inside workspace cells. Introduce one padded content rectangle used by the compositor.
3. Repair the demonstrated preview/render-layer cause, preserving native client buffers, fullscreen/switcher behavior, window activation and drag interactions.
4. Give the full workspace overview its own screen-sized opaque backing and clip; keep switcher/window shelves bounded.
5. Verify workspace tests, real native previews including hidden workspace content and narrow layouts, review, update design/validation notes, commit and push.

Completed: all three demonstrated causes repaired; geometry and native capture regressions verified; workspace checks, four native cases, design audit and independent source review passed. Raw evidence remains local; matching binaries require installation and a new login to replace the active compositor.
