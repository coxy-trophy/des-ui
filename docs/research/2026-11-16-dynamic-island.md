# Dynamic Island: what Apple does, and what DEs-UI's status island should adopt

Research done 16 Nov 2026. Nothing below is built yet; it is the plan for later.

## Apple's model

**Sizes**
- **Compact**: used when one activity is running. It is split into a leading part and a trailing part, one on each side of the camera, and the two read as one piece of information. Keep it as narrow as the content allows.
- **Minimal**: used when two or three activities are running. One stays attached to the island and the others sit beside it, detached, as circles or ovals.
  - Show live data even at this size, not just a logo (the Timer app shows the time left).
  - On iPhone 18 Pro, two activities show side by side as narrower compact views instead.
  - A narrow presentation must fit a 28 pt circle.
- **Expanded**: opens on touch-and-hold. It is the compact layout made larger: elements keep their relative places and grow. The height follows the content, between 84 and 160 pt.

**Gestures**
- Tap opens the app at the relevant screen.
- Hold expands.
- Tapping outside collapses it.
- Swiping toward the centre hides an activity: a call leaves an icon behind, music disappears. Swiping outward brings it back, and swiping at the edges changes which activity is in front.

**Opening on its own.** The island expands by itself only for an "alerting update", which also lights the screen and plays a sound. Apps should send these rarely, and should not send a notification for the same event.

**Lifetime**
- An activity is removed from the island as soon as it ends.
- An activity can run for at most 8 hours.
- A summary can stay on the lock screen for up to 4 hours after it ends.
- `staleDate` marks data as out of date, and the activity then says so.
- When several activities compete for the island, a relevance score decides which one shows.

**Look**
- The background is opaque black.
- The outline (Apple's "key line") is tinted to match the content.
- Content uses bold brand colours and text of medium weight or heavier.
- Margins are even, and inner corners are concentric with the outline, so nothing touches the edge.
- Prefer a single control per activity.
- Show a harmless summary instead of sensitive content.
- No ads.

**Motion**
- Animations last at most 2 s.
- Move existing elements to their new place rather than removing and re-adding them.
- Numbers roll when they change, and text changes cross-fade with a blur.
- Apple does not publish its spring settings. Third-party copies overshoot slightly on expand and settle without overshoot on collapse.

**What the system itself puts there:** calls, timers, Face ID, AirDrop, charging with the percentage, the silent switch, AirPods connecting with their battery, and Focus changes. A camera or microphone dot sits next to the island while either is in use.

**Other platforms**
- Mac notch apps (NotchNook, Alcove) expand on hover and have a file shelf you can drop files onto.
- Android 16 Live Updates is a status-bar chip in the app's colour that expands when tapped.
- Samsung's Now Bar shows media, timers and the stopwatch.

## Our island compared with Apple's

| Behaviour | Ours (island.rs) | Plan |
|---|---|---|
| Tap vs hold | A click toggles the card | Hover expands, click opens the app (Spotify, or the app that sent the notice), and long-press on touch expands |
| Click while recording | Stops the recording at once | Click shows the card instead; the Stop button lives in the card |
| Hover holds a notice | Done | Keep |
| Opens by itself only for alerts | Every notice takes over for 4 s | Add an urgent flag. Only urgent notices expand; the others pulse quietly in compact |
| Several activities | Music plus a recording bubble | Two narrower compact views side by side, and a third as a bubble |
| Swipe to hide | None | Touch: swipe inward to hide and outward to restore. Mouse: the scroll wheel does the same |
| Removed when it ends | Music stays while paused | Hide music after about a minute paused |
| Stale state | None | Dim the content and show a "not updating" mark when data stops arriving |
| Outline colour | Always the grey line colour | Album colour for music, red for recording |
| Spring | Critically damped both ways | Expand with damping about 0.8, collapse critically damped |
| Sensitive content | Shows the notice body | Optional sender-only mode, and hide the body automatically while the screen is shared |
| System events | Volume and brightness levels | Bluetooth device and battery (BlueZ Battery1), charging %, Do Not Disturb, camera/mic dot (PipeWire) |
| App progress | None | Read the Unity LauncherEntry progress (Chrome, Firefox, file copies) and KDE JobView |
| Timers, calls | None | Phase 3: local timers, and calls through DES Connect |
| App API | None | A D-Bus service where an app starts, updates and ends an activity with a relevance score and an urgent flag |
| File shelf | None | Maybe: drop a file on the island to hold it, or send it to the phone |

**Not adopted:**
- Wrapping content around a camera cutout (we have no camera cutout).
- Hold as the only way to expand.
- Ignoring the animations apps ask for.
- The 8-hour cap.

## Order

1. **Behaviour, in `island.rs` only:**
   - hover to expand, click to open the app, no one-click stop;
   - the spring;
   - the tinted outline;
   - the stale state;
   - removing music after a pause;
   - the privacy option.
2. **System activities, from sources the system already has:** Bluetooth, charging, Do Not Disturb, the camera/mic dot, download and copy progress, and two activities side by side.
3. **New plumbing:** swipe gestures, the D-Bus island API, timers, calls through DES Connect, and possibly the file shelf.

## Sources

- https://developer.apple.com/design/human-interface-guidelines/live-activities
- https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities
- https://nonstrict.eu/wwdcindex/wwdc2026/223/
- https://sixcolors.com/post/2022/10/using-gestures-inside-the-dynamic-island/
- https://www.macrumors.com/2022/09/27/iphone-14-pro-dynamic-island-two-live-activities/
- https://www.macupdate.com/comparisons/notchnook-vs-alcove-whats-the-best-macbook-notch-app-in-2026
- https://www.androidauthority.com/android-16-live-notifications-3518375
