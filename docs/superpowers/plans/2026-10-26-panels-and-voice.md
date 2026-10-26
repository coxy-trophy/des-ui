# Panels and companion voice

Goal: show System on login, introduce small rounded panel gaps matching the user's reference, and diagnose voice failure.

- [x] Add shared frame Layout with explicit side, center, bottom, header and native workspace rectangles; regression-test gaps, collapse/hide, narrow bounds and fullscreen independence.
- [x] Wire shell surfaces and compositor placement to this owner. Set initial right_apps=false; Super+D continues selecting Apps. Retain existing materials and add only a subtle outline.
- [x] Diagnose voice against official API documentation and actual error. Repair demonstrated diagnostics/audio lifecycle issues with failing tests; no unverified success claim.
- [x] Verify workspace + companion tests, native preview, design contract and diff. Preserve live sessions.
- [x] Commit and push completed changes, keeping raw machine evidence excluded.
