# Music, applications, panels, devices and keyboard

User scope: strengthen Spotify-style music browsing/playback, launcher and right-panel Apps; cache icons instead of reloading on each open; remove System globe controls and resume rotation after touch; prevent side-panel scrolling/overflow and keep the pet visible; research and improve connected-device visibility beyond webcams with external camera choice; align the on-screen keyboard with the separated frame. Preserve DEs-UI theme/material/type and existing account/backend behavior.

## Design and implementation

- Music: artwork-led Home/library navigation and collection cards with persistent playback bar. Compact playback has an explicit height so System can budget it. Preserve Spotify operations and honest authentication/offline/loading states.
- Apps: context-shared, bounded icon cache with warm disk thumbnails, source invalidation and background decoding. Launcher search remains keyboard-driven; right Apps uses finite pages fitting its available height.
- System/left panels: reserve fixed footer rectangles before laying out instruments; fit optional telemetry to remaining space, keep the pet and controls visible. No vertical side-panel ScrollArea. Remove panel globe control row; pointer/touch interaction temporarily pauses rotation, leaving resumes. Expanded globe remains accessible by double click.
- Devices: research Linux device APIs. Background inventory reports cameras, audio, input, storage, USB, displays and connected Bluetooth where supported; include hotplug feedback and Settings details. Camera selection validates stable device identity and never starts a capture merely on discovery. Preserve explicit hand-control activation.
- Keyboard: one canonical frame layout reserves a rounded keyboard card in the center column while side cards retain full height. Share reservation with compositor through bounded IPC so native windows stay clear. Scale keys to available width/height and preserve shell text focus.

## Verification and delivery

Regression-test cache lifecycle, panel height budgets, globe resume, camera selection/disconnect, inventory diffs and keyboard placement/protocol. Run workspace checks with one nice compiler. Inspect representative native previews at wide and short dimensions, playback on/off, pet enabled, Apps and Devices, OSK open. Review changes, update DESIGN/UX and reviewed validation notes, commit/push authorized source changes. Raw machine captures stay local. Do not restart the live desktop, camera, microphone or account session.

## Additional user steering

- External monitor enlargement: inspect saved mirror/scaling and reconnect behavior; correct demonstrated source defects without changing the live display. Stabilize preferred scale ownership on independent outputs and revalidate modes/physical dimensions after hotplug. Photo-specific hardware reproduction remains separate from regression coverage.
- Pointer: replace the prominent opaque outline with configured fill and a restrained translucent definition shadow; keep semantic cursor shapes/hotspots. Forward allowlisted cursor lookup environment into Xwayland, add standard legacy aliases, and update the Settings preview. Preserve app-owned custom cursor surfaces.
- Settings organization: research official desktop documentation; group existing controls into Desktop, Hardware, Connections, Personal and System. Add familiar-word search with explicit results, friendly section titles, compact navigation and unchanged protocol destinations. Preserve Devices and all settings actions/drafts.

Completed: implementation, research, regression tests, reviewed fixes and native preview checks passed. Source/debug validation is recorded in docs/validation/2026-10-26-desktop-experience.md. Physical monitor reconnect and release installation remain activation checks; raw machine evidence stays local.
