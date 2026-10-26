# Desktop bug fixes and performance

Goal: fix the four confirmed defects and reduce measured shell CPU saturation without interrupting the user's session.

Authorization: user requested fixes and optimization. Execute inline in this dirty checkout to preserve the current right-panel implementation. Do not install, restart, kill user applications or change credentials.

- [x] Capture geometry: reproduce the 175% mismatch against Smithay output geometry; use consistent rounding; test fractional scales.
- [x] Capture completion: add correlated failure events through compositor, protocol and shell; test stale IDs, failed writes/grabs, locked requests and color picking.
- [x] Region selection: reproduce original press lost on first drag motion; retain press origin and test native egui events.
- [x] Hand helper: test help/unknown options without accessing a real camera; validate before models/camera; fail camera-only errors.
- [x] Performance: sample live CPU and isolate frame work; reproduce and benchmark the expensive path; preserve input responsiveness and animation settings.
- [x] Files clipboard: accept native Copy/Cut/Paste events only when Files owns input; preserve text-field/dialog ownership.
- [x] Unit discovery: include inactive loaded and installed unloaded user services without duplicate active records or bare templates.
- [x] Documentation and final verification: reconcile obsolete README claims; run full verification with constrained build concurrency and record limitations.

Review focus: mixed-version IPC, cancellation races, invalid arguments, secondary viewport repainting, resource cleanup. Review all new changes against the pre-existing diff; leave unrelated edits intact.

Final review: the independent reviewer identified capture chooser ownership and Files-rail commands lost under a covering app; both were reproduced RED and repaired GREEN. No remaining actionable review findings. Native hardware acceptance and activation remain outside the completed source/build verification.
