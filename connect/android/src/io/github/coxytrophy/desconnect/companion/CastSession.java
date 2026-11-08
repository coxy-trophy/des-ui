// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

/** One consent, capture and input gate. Shared by the socket, capture and UI threads. */
final class CastSession {
    static final int MAX_JPEG = 44 * 1024;
    static final long LEASE_MS = 3_000L;
    private static final long CONSENT_MS = 60_000L;

    enum Phase { REQUESTED, CONSENT, LIVE }

    static final class Token {
        final String id;
        Token(String id) { this.id = id; }
    }

    static final class View {
        final Token token;
        final Phase phase;
        final boolean control;
        View(Token token, Phase phase, boolean control) {
            this.token = token;
            this.phase = phase;
            this.control = control;
        }
    }

    static final class Outbound {
        final Token token;
        final String phase;
        final boolean control;
        final long geometry;
        final byte[] jpeg;
        Outbound(Token token, String phase, boolean control, long geometry, byte[] jpeg) {
            this.token = token;
            this.phase = phase;
            this.control = control;
            this.geometry = geometry;
            this.jpeg = jpeg;
        }
    }

    private final Runnable changed;
    private String previousId;
    private Token token;
    private Phase phase;
    private boolean approvedControl;
    private boolean control;
    private long expires;
    private long lease;
    private long geometry;
    private Outbound status;
    private Outbound frame;
    private boolean frameDirty;
    private long lastSent;

    CastSession() { this(() -> {}); }

    CastSession(Runnable changed) { this.changed = changed; }

    synchronized Token request(String id, long now) {
        expire(now);
        if (token != null || status != null || id == null || !id.matches("[a-f0-9]{32}")
                || id.equals(previousId)) return null;
        token = new Token(id);
        previousId = id;
        phase = Phase.REQUESTED;
        expires = now + CONSENT_MS;
        lease = now;
        geometry = 0;
        changed.run();
        return token;
    }

    synchronized View view() {
        return token == null ? null : new View(token, phase, control);
    }

    synchronized void lease(String id, long now) {
        expire(now);
        if (token != null && token.id.equals(id)) lease = now;
    }

    synchronized boolean approve(Token expected, boolean allowControl, long now) {
        expire(now);
        if (expected == null || expected != token || phase != Phase.REQUESTED) return false;
        phase = Phase.CONSENT;
        approvedControl = allowControl;
        changed.run();
        return true;
    }

    synchronized boolean activate(Token expected, long now) {
        expire(now);
        if (expected == null || expected != token || phase != Phase.CONSENT) return false;
        if (now - lease >= LEASE_MS) {
            stop(expected);
            return false;
        }
        phase = Phase.LIVE;
        status = new Outbound(token, "live", false, ++geometry, null);
        changed.run();
        return true;
    }

    synchronized boolean captures(Token expected, long now) {
        expire(now);
        return expected != null && expected == token && phase == Phase.LIVE;
    }

    synchronized boolean canStart(Token expected, long now) {
        expire(now);
        return expected != null && expected == token && phase == Phase.CONSENT && now - lease < LEASE_MS;
    }

    synchronized long geometry(Token expected, long now, boolean defaultDisplay) {
        if (!captures(expected, now)) return 0;
        if (geometry == Integer.MAX_VALUE) {
            stop(expected);
            return 0;
        }
        geometry++;
        frame = null;
        frameDirty = false;
        boolean previous = control;
        control = approvedControl && defaultDisplay;
        status = new Outbound(token, "live", control, geometry, null);
        if (previous != control) changed.run();
        return geometry;
    }

    synchronized boolean input(Token expected, long generation, long now) {
        return captures(expected, now) && control && generation > 0 && generation == geometry;
    }

    synchronized boolean input(Token expected, long generation, long now, Runnable action) {
        if (!input(expected, generation, now)) return false;
        action.run();
        return true;
    }

    synchronized void frame(Token expected, long generation, byte[] jpeg, long now) {
        if (!captures(expected, now) || generation <= 0 || generation != geometry
                || jpeg == null || jpeg.length == 0 || jpeg.length > MAX_JPEG) return;
        frame = new Outbound(token, null, false, geometry, jpeg);
        frameDirty = true;
    }

    synchronized boolean current(Outbound packet, long now) {
        if ("stopped".equals(packet.phase)) return true;
        return captures(packet.token, now) && packet.geometry == geometry;
    }

    synchronized Outbound next(long now) {
        expire(now);
        if (status != null) {
            Outbound result = status;
            status = null;
            return result;
        }
        if (frame != null && (frameDirty || now - lastSent >= 1_000)) {
            frameDirty = false;
            lastSent = now;
            return frame;
        }
        return null;
    }

    synchronized void stop(String id) {
        if (token != null && token.id.equals(id)) stop(token);
    }

    synchronized void stop(Token expected) {
        if (expected == null || expected != token) return;
        status = new Outbound(token, "stopped", false, 0, null);
        token = null;
        phase = null;
        frame = null;
        frameDirty = false;
        approvedControl = false;
        control = false;
        changed.run();
    }

    synchronized void stop() { stop(token); }

    private void expire(long now) {
        if (token == null) return;
        if (phase == Phase.LIVE ? now - lease >= LEASE_MS : now >= expires) stop(token);
    }
}
