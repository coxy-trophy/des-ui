# Right-panel Apps validation —2026-10-25

The bottom dock and its reserved band are replaced by the existing right panel. Apps is selected by default; System preserves the instruments. Super+D opens/focuses Apps. Secondary displays use a transient right rail. Mobile was not edited or built.

## Automated evidence

- Workspace tests: 741 unit tests and one doctest passed (65 common, 152 compositor, 36 hands, 19 music, 7 portal, 462 shell).
- Final shell tests after integration adjustments: 462 passed.
- Workspace build and final shell build: passed without warnings.
- Strict design audit: zero findings; git diff whitespace check passed.
- Popup regression reproduced Enter activation behind a context menu, then passed after popup ownership was fixed. Narrow 192/240/340px layouts, cross-display grouping, expanded window navigation, Escape and the last of 128 rows are covered.

## Native preview

[1280×900 native preview](2026-10-25-right-panel-apps.png) captured using the built shell in Xvfb, private configuration and a private simulated compositor socket. The four example windows are synthetic; this verifies native placement and rendering, not real client operations. The right panel is visible, grouped app rows fit, running apps precede pins, Files/Chat reaches the status strip, and no bottom dock remains.

No installed desktop restart, installation or real application activation occurred. Native client focus, previews, display scaling and window operations remain in docs/UNTESTED.md for installed-session acceptance.
