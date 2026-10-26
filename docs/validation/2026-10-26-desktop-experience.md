# Desktop experience — 26 October 2026

Implemented the requested music, launcher, Apps/System panels, pet, device inventory, keyboard placement, cursor and Settings improvements. Additional monitor work addresses two demonstrated source defects; the exact physical-display failure shown in the photograph remains unconfirmed.

## Behavior and evidence

Music Home presents actual playlists and album artwork, responsive navigation and persistent playback. Compact playback has a fixed 136px budget. Account-free native previews used a synthetic local music service and locally cached fixture artwork; no Spotify account or real playback was exercised. Existing service/authentication operations are retained.

Launcher, Apps and tray share a bounded 512-entry texture cache. Reopening the launcher no longer clears artwork, nor does the shelf’s periodic application refresh. Background source/theme checks preserve ready textures; decoded thumbnails persist under the XDG cache directory. Empty launcher browsing includes all installed applications; typed results remain bounded. The Apps panel pages both applications and expanded window rows, keeping keyboard selection on the visible page.

Side panels do not scroll. Playback, local readings and the pet receive independent reservations before optional content. Short left panels use compact telemetry; the configured crab was visible in inspected native previews. The compact globe has no control strip, resumes rotation after pointer/touch departure and opens its explorer on double-click. Disabled animation remains respected. Native visual checks caught clipped display tiles/readings; explicit heading, row and tile geometry repaired those reservations.

The keyboard is a rounded center-column card, rather than a full-width strip shrinking both side panels. A bounded keyboard-height command gives shell and compositor the same rounded reservation. Keys and labels fit narrow cards. A real GTK client at 1280×720 received a 784×368 workspace above the keyboard while side panels stayed full-height. Keyboard typing retains its existing shell-controls/terminal scope; this is not a virtual-keyboard implementation for other apps.

Settings now has Desktop, Hardware, Connections, Personal and System categories, seventeen preserved destinations, friendly names, descriptions and familiar-word search. Search requires an explicit result choice and Escape first clears a nonempty query. Narrow navigation uses selectors. Native search for “camera” found Connected devices and Mouse & touchpad.

Devices enumerates cameras, audio endpoints, input, storage, USB, displays and Bluetooth through a background worker. Initial discovery is quiet; later changes use existing notifications. Metadata nodes are excluded from hand-camera choices. Saved camera identity survives video-number changes and fails closed when unavailable. Select a camera in Hardware → Connected devices and restart hand control to apply it. Other applications retain their own camera choices. Inventory never streams a camera/microphone, pairs hardware or mounts storage. Current-machine read-only checks found one usable capture node among two video nodes and parsed two PipeWire audio endpoints. Bluetooth was unavailable in the check; its parser and stale-service handling were fixture-tested.

Monitor reconnect previously retained stale connector capabilities, and mirrored outputs could become a window’s preferred client-scale source through arbitrary output iteration. Reconnect now revalidates modes/physical dimensions; client scale follows independent-output overlap with stable primary ownership. Saved mirror scaling was intentional, so no global scale reset or live display change was made. Physical HDMI reconnect/overscan still needs verification on the affected hardware.

Pointer artwork uses the configured fill without an opaque outline, plus a contrasting translucent offset shadow. Semantic shapes, hotspots, size and custom client-supplied cursor surfaces remain intact. Xwayland’s cleared environment now receives an allowlisted set of cursor lookup variables. The exported theme includes standard legacy aliases and XDG lookup roots. Existing applications may retain cached cursors until reopened. Settings previews match the raster parameters.

## Research applied

[KDE’s System Settings guide](https://docs.kde.org/stable5/en/systemsettings/systemsettings/systemsettings.pdf) describes categorized modules and keyword topics. [Apple’s settings search](https://support.apple.com/en-ie/guide/mac-help/mchl8d10839d/mac) supports familiar terms and explicit matching results. [Windows settings documentation](https://support.microsoft.com/en-us/windows/experience/system-configuration-tools-in-windows) describes category organization; [GNOME’s keyboard settings](https://help.gnome.org/gnome-help/keyboard-shortcuts-set.html) provides a categorized desktop control pattern. The implementation adopts these navigation principles while retaining existing Linux capabilities.

[Linux V4L2 capabilities](https://docs.kernel.org/userspace-api/media/v4l/vidioc-querycap.html) distinguish usable device functions; inventory uses udev’s cached capture metadata without opening a stream. [PipeWire’s JSON inventory](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-dump_1.html) informs audio endpoint discovery. [BlueZ device states](https://bluez.readthedocs.io/en/latest/device-api/) distinguish pairing from connection. [UDisks filesystem behavior](https://storaged.org/doc/udisks2-api/2.7.6/gdbus-org.freedesktop.UDisks2.Filesystem.html) keeps mounting an explicit operation, which this inventory does not perform. [Linux KMS documentation](https://docs.kernel.org/gpu/drm-kms.html) informs connector/mode revalidation.

## Validation and activation

`CARGO_BUILD_JOBS=1 nice -n 10 ./scripts/verify.sh` passed: 81 Python tests, 797 Rust unit/integration tests, one doctest and workspace debug build. Coverage includes icon sharing/warm cache/invalidation, paging and activation, panel reservations, pet rendering, keyboard bounds/protocol, device parsing/identity, Settings search/layout, mirror scale ownership, connector changes, cursor raster/background contrast and export/environment behavior. A pre-existing `/proc` reaping race in the startup test was corrected to recognize successful process disappearance.

Wide and short native previews were inspected for Music, System, Apps, launcher, Devices, Settings search and keyboard/native-window placement. Static design audit and diff checks passed. Independent reviews found issues that were repaired; follow-up reviews found no further actionable regressions. Music/desktop previews used private Xvfb/configuration/runtime directories. Raw captures, inventories and logs remain local and are not committed.

No installed binary, live desktop, monitor configuration, camera or microphone was changed. Install the source update, then log out and back in to activate matching shell, compositor and cursor-session environment. Release installation and physical monitor reconnect are not claimed as completed by these source/debug checks.

## System panel spacing follow-up

The supplied screenshot exposed an unused middle band: the flexible instrument reservation consumed all remaining card height, while Displays and Controls were anchored to the floor. Instrument rendering now reports its actual content extent; Displays and Controls follow it with an 8px gap. Music keeps its protected bottom reservation. No new scrolling or live-session changes were introduced.

Fresh verification: all 499 shell tests passed. A rendered egui regression checks 192px and 292px columns at 480px, 720px and 1000px heights, both with and without playback, for the globe-to-display gap and local-reading clearance above playback. Existing footer and height-budget tests also passed; git diff --check passed. This follow-up was verified through headless rendering geometry, not an installed live-session preview.
