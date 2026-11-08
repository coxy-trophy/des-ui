# First-party DES Connect Android companion

Status: approved. This replaces the Android fork decision in the November 3
design. The native desktop service and CONNECT panel remain the foundation.

## Goal

Ship a standalone Android app written and maintained by DES UI. It should work
alongside the Android launcher and the current companion during migration. It
gets a separate package and fresh device keys, with no code or assets copied
from the KDE Connect fork. After the replacement passes the cutover checks, the
fork stops being the app we develop, document, or distribute for DES Connect.

## Approaches

1. **New app, existing protocol first (recommended).** Implement the desktop's
   existing wire contract independently for discovery, pairing, ping, battery,
   files, and media. Negotiate versioned DES capabilities separately for new
   features. This keeps the deployed desktop and its proven pairing flow useful
   while we replace the app. The cost is maintaining a small compatibility
   layer until we decide whether it remains worthwhile.
2. **New app and a new protocol from day one.** This gives us full control of the
   wire contract immediately, but adds a second discovery, trust, and migration
   project before users can try the new companion. Only revisit this if a
   measured requirement cannot fit a separately negotiated DES capability.

The desktop remains the owner of pairing policy, feature grants, lock state,
and capture revocation. The Android app owns Android permissions and session
lifecycles. Start with a small native Activity and platform APIs; add a library
only when a feature needs it. Use DES UI's existing visual language and keep
the app separate from the default launcher. Pairing does not require a cloud
account, root, or a change to the Android home screen.

Test the first APK by direct install on the Android 15 emulator and the Android
14 realme and tablet. Store distribution and support for additional OS versions
are separate release decisions. The baseline is the desktop service at `7f8b7d1`;
new DES capabilities may require coordinated app and desktop upgrades.

Start with a connection while the app is open. Add optional background
availability before cutover, with a visible connection notification when the
platform requires one. If the app is unavailable, CONNECT should ask the user
to open it on the phone. Do not promise availability after force-stop or reboot,
or launch a capture permission prompt automatically in the background.

## Delivery order

| Milestone | What should work before moving on |
| --- | --- |
| 1. Independent app | Install a separate APK. Connect to the desktop, show the same verification code on both sides, pair, reconnect, report battery, ping in both directions, and unpair. Start with an isolated daemon and emulator; verify ordinary LAN discovery on the realme and Samsung tablet. |
| 2. Wireless phone preview | Request sharing from CONNECT, let the user approve Android's screen-sharing prompt on the phone, and display live frames in the existing panel. Start view-only and local-network-only, with the app or full screen chosen through Android where supported. Keep USB as a fallback. |
| 3. Daily-use parity and handoff | Restore the fork's verified file-transfer and media/volume workflows. Add Share → DES Connect for files and links, Send to phone in DES Files, and read-only browsing of user-selected phone folders. Start with explicit clipboard actions. |
| 4. Tablet and camera | Make Extend desktop a usable display workflow with reconnect and display arrangement. Add touch input after the viewer is stable. Add Scan to this folder, then webcam mode. |
| 5. Optional control | Evaluate input control, cursor crossing, and individual Android app shortcuts separately on supported devices. Do not require accessibility access for mirroring or pairing. |

## Safety and resource boundaries

- Store the app's private key in Android Keystore. Verify the desktop's identity
  and certificate before authorizing actions; keep the existing code comparison
  and pinned reconnect model. Refuse changed keys until the user re-pairs.
- Treat wireless video as a separate capability. Authenticate the selected peer
  and bind the stream to that pairing and the current sharing request. Do not
  publish a stream URL on the network or attach video to the command socket.
- Use Android MediaProjection consent on each capture start. Stop and clear the
  preview on desktop lock, permission revocation, loss of the peer, or Stop.
  Reconnecting never starts another capture. Start with a native viewer and
  bounded buffers. Compare transports after measuring the first capture path.
- Measure idle memory, CPU, and wakeups against the fork on the same device.
  Video workers run only during sharing. File listings and thumbnails load on
  demand. Keep the desktop panel compact.

## Cutover

Install both apps side by side during validation, with the new app labeled
DES Connect Preview. Validate switching between apps, not simultaneous activity
from both apps on one phone. The new app pairs separately; do not copy old
private keys or overwrite the old app's data. Verify Android
reconnect, both file directions, media and volume opt-ins, and capture start,
stop, lock and disconnect on real hardware. Then make the new APK the documented
default and retain the old build as a rollback option until migration is stable.
Device-specific capabilities must be visible and tested before advertising them.

For the initial tablet display workflow, recover windows to the desktop when
the tablet disconnects. A new extension session requires an explicit start;
preserving window placement across a short interruption is future work.
