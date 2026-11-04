# CONNECT layout study

Throwaway visual proposal requested on 2026-11-04. Not an approved implementation
and not connected to the daemon or tablet. Product source is unchanged.

The same panel is shown at 300px and 620px. Remove APPS from right-panel navigation;
keep SYSTEM and CONNECT. Selected device and battery form one compact header.
Mirroring owns the primary content area; Stop stays visible and Expand widens the
panel in place. Send file and Media are short secondary actions. Transfers use
one compact row. Permissions move behind the device gear; secondary service and
pairing workflows need contextual views rather than default explanatory copy.

The prototype borrows the existing Signal palette and local Chakra Petch /
JetBrains Mono fonts. Its device screen is an SVG illustration, not a capture or
live stream. Buttons only change local demo state. Connected, mirroring, locked,
offline-device, portrait shape, settings and expanded views are inspectable.

Decision to make: whether starting a mirror automatically widens the right panel,
or whether it stays compact until Expand is selected. The proposed regular and
expanded views share one component, rather than becoming separate destinations.

Implementation constraint: the current ScreenBridge launches scrcpy in its own
window. A genuine embedded viewer needs a frame/decoder integration; a visual
restyle alone does not put that window into an egui tab. Permission, session-lock,
disconnect and capture-stop protections must carry through. No decoder runs while
the mirror is stopped or the session is locked.

Before removing the production APPS destination, migrate any remaining window
management/keyboard actions to the existing left dock/overview. The user's current
request overrides the older DESIGN.md prose retaining the APPS tab; no normative
design or runtime code is changed while this proposal awaits feedback.

Verified in an isolated Chromium renderer at 1500×1120 and 390×844. Shared-panel
layout, start/stop, SYSTEM capture-stop access, settings/Escape, cancellation,
locked/cleared preview, no auto-resume, offline gating, portrait and expand/restore
were exercised. Node syntax check passed. The strict project audit reports no
findings in this proposal and 33 existing findings in the older terminal prototype.
No product build or device test applies to this visual-only spike.
