# DesOS desktop stabilization

## Intent and authority

Prepare a reproducible DEs-UI candidate for the owner's laptop before building a DesOS live image. The user approved the stabilization-first recommendation on 9 November 2026. Baseline: desktop `50794d0eabf9dca7d2a4a1859a1222716f7b37b7`; website `25f6fbad1d293283e0c39d7c6c0098db54e65c32`.

## Approaches and decision

The discussion considered building an OS image immediately, stabilizing the existing desktop first, and independently assembling the entire OS. The user selected stabilization first, reusing distribution user-space packages and eventually unmodified upstream Linux. This work repairs verified desktop-delivery defects and records acceptance evidence; OS/image and kernel work follows separately.

## Scope

- Include the Connect daemon, CLI and virtual-display viewer in the desktop deb, with the daemon's native dependencies discovered by the existing dependency mechanism. Preserve explicit service start and feature permissions; add no autostart.
- Resolve the viewer at both `/usr/local/share/des-ui` and `/usr/share/des-ui`, preserving the existing explicit override and development fallback.
- Remove Connect executables during source uninstall.
- Make the process-cleanup regression handle both ENOENT and ESRCH when a dying process disappears during `/proc` inspection. Other errors remain failures.
- Correct the website's obsolete `des-ui --unlock` recovery instruction. Recovery must use password authentication or explicitly end the session; never introduce an unlock bypass.
- Verify the complete automatic gate, website build and actual package contents; capture source/binary identity and a focused hardware acceptance checklist.

## Constraints and boundaries

Use isolated checkouts; preserve user edits and the installed desktop. Do not install packages, publish, restart sessions, alter boot/login settings, suspend, reboot, start capture/voice or send messages. Packaging tests use temporary trees and mocked native dependency discovery, with real Debian archive inspection. No UI redesign, kernel replacement or broad refactor. Hardware results are Pending until witnessed on the exact candidate.

## Acceptance

Focused regressions fail before their repairs and pass afterwards. `./scripts/verify.sh`, the website production build and package archive inspection pass. The record separates automated, previously observed and still-pending machine behavior; it does not claim the image or laptop acceptance is complete.
