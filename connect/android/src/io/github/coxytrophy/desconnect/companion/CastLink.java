// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

import android.app.KeyguardManager;
import android.content.Context;
import android.graphics.Point;
import android.hardware.display.DisplayManager;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;
import android.util.Base64;
import android.view.Display;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.JSONException;
import org.json.JSONObject;

/** Handles cast messages only after Link has authenticated and paired the desktop. */
final class CastLink {
    static final String KIND = "des.connect.cast.v1";

    private static final class InputGeometry {
        final CastSession.Token token;
        final long id;
        final CastGeometry.Shape shape;
        InputGeometry(CastSession.Token token, long id, CastGeometry.Shape shape) {
            this.token = token;
            this.id = id;
            this.shape = shape;
        }
    }

    private final Context context;
    private final CastSession session;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final AtomicBoolean inputPending = new AtomicBoolean();
    private volatile boolean foreground;
    private volatile InputGeometry inputGeometry;
    private long lastInput = -1_000; // main thread only

    CastLink(Context context, Runnable changed) {
        this.context = context;
        this.session = new CastSession(changed);
    }

    static long now() { return SystemClock.elapsedRealtime(); }

    static boolean unlocked(Context context) {
        KeyguardManager lock = context.getSystemService(KeyguardManager.class);
        PowerManager power = context.getSystemService(PowerManager.class);
        return lock != null && power != null && power.isInteractive()
                && !lock.isKeyguardLocked() && !lock.isDeviceLocked();
    }

    static CastGeometry.Shape displayShape(Context context) {
        DisplayManager manager = context.getSystemService(DisplayManager.class);
        Display display = manager == null ? null : manager.getDisplay(Display.DEFAULT_DISPLAY);
        if (display == null) return null;
        Point size = new Point();
        display.getRealSize(size);
        return size.x > 0 && size.y > 0 ? new CastGeometry.Shape(size.x, size.y, display.getRotation()) : null;
    }

    void foreground(boolean visible) { foreground = visible; }
    CastSession.View view() { return session.view(); }

    boolean approve(CastSession.Token token, boolean control) {
        return foreground && unlocked(context) && (!control || CastControlService.available())
                && session.approve(token, control, now());
    }

    boolean canStart(CastSession.Token token) {
        return unlocked(context) && session.canStart(token, now());
    }

    boolean activate(CastSession.Token token) {
        return unlocked(context) && session.activate(token, now());
    }

    boolean capturing(CastSession.Token token) {
        if (!unlocked(context)) {
            stop(token);
            return false;
        }
        return session.captures(token, now());
    }

    long geometry(CastSession.Token token, CastGeometry.Shape shape, boolean control) {
        long id = session.geometry(token, now(), control && shape != null);
        if (id != 0) inputGeometry = control && shape != null ? new InputGeometry(token, id, shape) : null;
        return id;
    }

    void frame(CastSession.Token token, long geometry, byte[] jpeg) {
        if (capturing(token)) session.frame(token, geometry, jpeg, now());
    }

    void stop() { session.stop(); }
    void stop(CastSession.Token token) { session.stop(token); }

    void received(JSONObject body) {
        Object id = body.opt("id");
        Object op = body.opt("op");
        if (!(id instanceof String) || !(op instanceof String)) return;
        switch ((String) op) {
            case "start":
                if (foreground && unlocked(context)) session.request((String) id, now());
                break;
            case "lease":
                if (!unlocked(context)) stop();
                else session.lease((String) id, now());
                break;
            case "stop":
                session.stop((String) id);
                break;
            case "input":
                input((String) id, body);
                break;
            default:
                break;
        }
    }

    JSONObject poll() throws JSONException {
        if (session.view() != null && !unlocked(context)) stop();
        CastSession.Outbound item = session.next(now());
        if (item == null) return null;
        JSONObject body = new JSONObject().put("id", item.token.id);
        if (item.phase != null) {
            if (!session.current(item, now())) return null;
            body.put("op", "status").put("phase", item.phase).put("control", item.control);
            if ("live".equals(item.phase)) body.put("geometry", item.geometry);
            return body;
        }
        String jpeg = Base64.encodeToString(item.jpeg, Base64.NO_WRAP);
        int escapedLength = jpeg.length();
        for (int index = 0; index < jpeg.length(); index++) {
            if (jpeg.charAt(index) == '/') escapedLength++;
        }
        if (escapedLength > Frames.MAX - 256 || !capturing(item.token) || !session.current(item, now())) return null;
        return body.put("op", "frame").put("geometry", item.geometry).put("jpeg", jpeg);
    }

    private void input(String id, JSONObject body) {
        InputGeometry geometry = inputGeometry;
        Object supplied = body.opt("geometry");
        Object action = body.opt("action");
        if (geometry == null || !geometry.token.id.equals(id) || !(supplied instanceof Long)
                || ((Long) supplied).longValue() != geometry.id || !(action instanceof String)
                || !unlocked(context) || !session.input(geometry.token, geometry.id, now())
                || !CastControlService.available()) return;
        String command = (String) action;
        int x = coordinate(body, "x"), y = coordinate(body, "y");
        int toX = coordinate(body, "to_x"), toY = coordinate(body, "to_y");
        switch (command) {
            case "tap": if (x < 0 || y < 0) return; break;
            case "swipe": if (x < 0 || y < 0 || toX < 0 || toY < 0) return; break;
            case "back": case "home": case "recents": break;
            default: return;
        }
        if (!inputPending.compareAndSet(false, true)) return;
        long received = now();
        main.post(() -> {
            try {
                long tick = now();
                if (tick - received > 400 || tick - lastInput < 100 || inputGeometry != geometry
                        || !geometry.shape.same(displayShape(context))) return;
                session.input(geometry.token, geometry.id, tick, () -> {
                    if (!unlocked(context)) return;
                    lastInput = tick;
                    CastControlService.dispatch(command,
                            pixel(x, geometry.shape.width), pixel(y, geometry.shape.height),
                            pixel(toX, geometry.shape.width), pixel(toY, geometry.shape.height));
                });
            } finally {
                inputPending.set(false);
            }
        });
    }

    private static int coordinate(JSONObject body, String key) {
        Object value = body.opt(key);
        if (!(value instanceof Long)) return -1;
        long number = (Long) value;
        return number >= 0 && number <= 10_000 ? (int) number : -1;
    }

    private static float pixel(int normalized, int axis) {
        return Math.max(0, normalized) * (axis - 1) / 10_000f;
    }
}
