// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.content.Intent;
import android.graphics.Path;
import android.util.Log;
import android.view.accessibility.AccessibilityEvent;

/** Gesture-only system bridge. No nodes, windows or accessibility events are inspected. */
public final class CastControlService extends AccessibilityService {
    private static volatile CastControlService connected;

    static boolean available() { return connected != null; }

    @Override protected void onServiceConnected() { connected = this; }

    @Override public void onAccessibilityEvent(AccessibilityEvent event) {}
    @Override public void onInterrupt() {}

    @Override public boolean onUnbind(Intent intent) {
        if (connected == this) connected = null;
        return super.onUnbind(intent);
    }

    @Override public void onDestroy() {
        if (connected == this) connected = null;
        super.onDestroy();
    }

    static void dispatch(String action, float x, float y, float toX, float toY) {
        CastControlService service = connected;
        if (service == null) return;
        try {
            service.perform(action, x, y, toX, toY);
        } catch (RuntimeException unavailable) {
            Log.w("DESConnectPreview", "Android declined a cast input");
        }
    }

    private void perform(String action, float x, float y, float toX, float toY) {
        switch (action) {
            case "back": performGlobalAction(GLOBAL_ACTION_BACK); return;
            case "home": performGlobalAction(GLOBAL_ACTION_HOME); return;
            case "recents": performGlobalAction(GLOBAL_ACTION_RECENTS); return;
            case "tap":
            case "swipe":
                Path path = new Path();
                path.moveTo(x, y);
                boolean swipe = "swipe".equals(action);
                if (swipe) path.lineTo(toX, toY);
                GestureDescription gesture = new GestureDescription.Builder()
                        .addStroke(new GestureDescription.StrokeDescription(path, 0, swipe ? 260 : 55))
                        .build();
                dispatchGesture(gesture, null, null);
                return;
            default:
        }
    }
}
