# Cast an Android screen into CONNECT

Wireless cast uses the standalone **DES Connect Preview** app and the updated
desktop service and shell. The phone and desktop must be reachable on the same
local network. The app must be open and paired when you start.

1. Select the phone in **CONNECT**, then start a wireless cast.
2. On Android, choose view-only sharing or sharing with control. Confirm
   Android's screen sharing prompt. Where Android offers an app-only share, use
   that for view-only mode; control requires the full display.
3. With control enabled, click or drag on the image. Use **Back**, **Home** and
   **Recents** below the preview for Android navigation. Tap **Stop** on the
   desktop, **Stop sharing** in the phone app, or use Android's system sharing
   controls to end it. Android may hide the service notification in the drawer.

Control needs Android's **DES Connect** accessibility service. Enable it from
Android Accessibility settings if you want to use the PC as a pointer. The
service dispatches gestures and navigation; it does not read the app hierarchy,
type text or access the clipboard. Keep it disabled for view-only sharing if
you do not need it. Sideloaded apps may require an extra confirmation under
**App info → Allow restricted settings**. See
[Android's restricted settings instructions](https://support.google.com/android/answer/12623953).
If the request expires while you're in Settings, return to DES Connect,
reconnect if needed, and start another cast from the PC. If a device offers an
app-only option during the control flow, choose the full display to use control.

The capture service runs only during a share and needs a fresh Android approval
each time. Frames and input travel through the existing pinned TLS connection.
The inline viewer keeps only a recent frame, and uses a short lease to end the
session if it closes. Either screen locking, a lost connection or a revoked
grant ends the session. Reconnect and unlock do not resume it. No frame is
written to disk in normal operation.

The first cast path is a low frame rate preview for navigation, without audio,
desktop keyboard input, or multitouch. USB mirroring remains available through
the existing [USB setup](SCREEN-BRIDGE.md). This feature is separate from
extending your Linux desktop onto a tablet.
