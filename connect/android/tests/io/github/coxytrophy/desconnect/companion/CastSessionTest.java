// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

public final class CastSessionTest {
    private static final String ID = "0123456789abcdef0123456789abcdef";
    private static final String OTHER = "abcdef0123456789abcdef0123456789";

    public static void main(String[] args) {
        approvalsAndGeometryFenceInput();
        expiryAndStopClearFrames();
        consentTimesOutDespiteLeases();
        rotationAndFreshLeasesCannotReplayFrames();
        rotationCorrelatesIndependentGeometryEvents();
        System.out.println("Cast session regression checks passed");
    }

    private static void approvalsAndGeometryFenceInput() {
        CastSession session = new CastSession();
        require(session.request("invalid", 0) == null, "invalid session ID");
        CastSession.Token token = session.request(ID, 100);
        require(token != null && session.request(OTHER, 101) == null, "one current request");
        require(!session.activate(token, 101), "capture without local approval");
        require(session.approve(token, false, 102) && session.activate(token, 103), "local view approval");
        long geometry = session.geometry(token, 104, true);
        require(geometry > 0 && !session.input(token, geometry, 105), "viewing never grants control");
        session.stop(token);
        session.next(105);
        CastSession.Token fresh = session.request(OTHER, 106);
        require(session.approve(fresh, true, 107) && session.activate(fresh, 108), "local control approval");
        long full = session.geometry(fresh, 109, true);
        require(session.input(fresh, full, 110), "approved default-display input");
        long resized = session.geometry(fresh, 111, false);
        require(resized != full && !session.input(fresh, full, 112) && !session.input(fresh, resized, 112),
                "old geometry and app-only content cannot accept input");
        require(!session.input(token, full, 112), "old session cannot accept input");
    }

    private static void expiryAndStopClearFrames() {
        CastSession session = new CastSession();
        CastSession.Token token = session.request(ID, 100);
        require(session.approve(token, true, 101) && session.activate(token, 102), "capture starts");
        long geometry = session.geometry(token, 103, true);
        session.frame(token, geometry, new byte[]{1}, 104);
        session.frame(token, geometry, new byte[]{2}, 105);
        require("live".equals(session.next(106).phase), "announce status before pixels");
        require(session.next(106).jpeg[0] == 2, "latest frame only");
        session.frame(token, geometry, new byte[44 * 1024 + 1], 107);
        require(session.next(107) == null, "oversize frame rejected");
        session.lease(OTHER, 3098);
        require(!session.input(token, geometry, 3100), "wrong-session lease cannot retain control");
        require("stopped".equals(session.next(3100).phase), "lease expiry reports stop");
        require(session.next(4000) == null, "expired frame discarded");
        require(session.request(ID, 4010) == null, "duplicate start does not reopen consent");
    }

    private static void consentTimesOutDespiteLeases() {
        CastSession session = new CastSession();
        CastSession.Token token = session.request(ID, 100);
        session.lease(ID, 59_999);
        require(!session.approve(token, true, 60_100), "leases cannot prolong request consent");
        require("stopped".equals(session.next(60_100).phase), "expired request reports stop");
    }

    private static void rotationAndFreshLeasesCannotReplayFrames() {
        CastSession session = new CastSession();
        CastSession.Token token = session.request(ID, 1);
        require(session.approve(token, true, 2) && session.activate(token, 3), "capture starts");
        long first = session.geometry(token, 4, true);
        session.frame(token, first, new byte[]{1}, 5);
        CastSession.Outbound ready = session.next(6);
        require(ready.control && ready.geometry == first, "live status identifies current geometry");
        CastSession.Outbound old = session.next(6);
        session.lease(ID, 2_999);
        require(session.input(token, first, 3_100), "fresh lease retains control");
        long next = session.geometry(token, 3_101, false);
        require(next > first && !session.current(old, 3_102), "rotation invalidates queued frames");
        session.frame(token, first, new byte[]{2}, 3_102);
        require(session.next(3_102).geometry == next, "rotation status uses the new geometry");
        require(session.next(3_102) == null, "old surface cannot publish to new geometry");
        session.stop(token);
        require(!session.input(token, next, 3_103), "stop cancels queued input");
    }

    private static void rotationCorrelatesIndependentGeometryEvents() {
        CastGeometry.Shape portrait = new CastGeometry.Shape(720, 1600, 0);
        CastGeometry.Shape landscape = new CastGeometry.Shape(1600, 720, 1);
        CastGeometry modern = new CastGeometry(false);
        require(!modern.display(portrait).full, "no control before the initial reported capture");
        require(modern.capture(portrait, 720, 1600).full, "matching portrait permits control");
        CastGeometry.Target earlyCapture = modern.capture(portrait, 1600, 720);
        require(earlyCapture.width == 1600 && earlyCapture.height == 720 && !earlyCapture.full,
                "capture-first rotation keeps the new bounds and suspends input");
        require(modern.display(landscape).full, "a later display event re-enables matching control");
        CastGeometry.Target earlyDisplay = modern.display(portrait);
        require(earlyDisplay.width == 1600 && earlyDisplay.height == 720 && !earlyDisplay.full,
                "display-first rotation waits for capture without substituting its own size");
        require(modern.capture(portrait, 720, 1600).full, "a later capture event re-enables matching control");
        CastGeometry.Target appOnly = modern.capture(portrait, 580, 900);
        require(!appOnly.full, "an app-only crop cannot authorize full-display control");
        CastGeometry.Target appOnlyRotation = modern.display(landscape);
        require(appOnlyRotation.width == 580 && appOnlyRotation.height == 900 && !appOnlyRotation.full,
                "a physical rotation cannot resize an app-only surface");
        require(modern.capture(landscape, 580, 900) == null, "repeated unchanged callbacks do not recreate surfaces");

        CastGeometry legacy = new CastGeometry(true);
        require(legacy.display(portrait).full && legacy.display(landscape).width == 1600,
                "legacy whole-display projection follows actual display dimensions");
    }

    private static void require(boolean condition, String detail) {
        if (!condition) throw new AssertionError(detail);
    }
}
