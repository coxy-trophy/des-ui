# DES Connect — Android-first design draft

Status: proposed architecture, pending user review. No connectivity feature is
implemented by this document.

## Intent and agreed scope

Build our own device-integration experience, exposed as a native CONNECT tab in
the existing right panel. Android is the initial mobile platform. Target the KDE
Connect feature set plus screen mirroring. Keep desktop memory use a first-class
constraint. The left dock now supplies everyday app switching; moving remaining
right Apps functions must precede deleting their only UI entry points.

## Approaches

1. Fork both KDE Connect desktop and Android. This reuses the most behavior but
   retains a Qt/KDE desktop service and its maintenance/dependency footprint.
2. Native Rust desktop service plus a KDE Connect Android fork (recommended).
   Reuse the Android starting point and baseline protocol; implement desktop
   capabilities behind a private local IPC API, with DES UI owning the interface.
   This avoids embedding Qt widgets in the shell but requires a substantial new
   compatible desktop backend. Compatibility is a test target, not assumed.
3. New desktop and Android apps from scratch. Most control, but reproduces years
   of permission, lifecycle, notification, transfer and messaging work.

The recommendation is the builder's proposal, not an approved user choice.
Before copying upstream code, inventory file-level SPDX licenses and dependency
licenses. The existing desktop workspace is MIT; do not relabel upstream GPL
code as MIT. Keep a fork's notices and applicable source-distribution obligations.
A process boundary alone is not a blanket licensing determination.

## Desktop and Android responsibilities

A separate desktop service owns discovery, paired identities, authenticated
connections, capability negotiation, transfers and device state. The egui shell
uses a private, versioned local IPC connection and renders state/events without
networking or video encoding on its UI thread. Do not reuse the unrelated Python
AI companion directory as the Android app.

The Android companion owns notification access, battery reports, clipboard
operations permitted by Android, SMS/call permissions, remote input features,
media projection and background connection lifecycle. Request permissions when
an associated feature is enabled. Missing permissions or unsupported capabilities
must produce explicit per-feature states rather than pretend success.

Baseline protocol compatibility and DES-specific screen extensions must be
negotiated separately. Never infer screen support from ordinary KDE Connect
pairing. Handle unknown packets, bounded message sizes, peer disconnect, version
mismatch and reconnect without replaying commands or duplicating transfers.

## Target capabilities

- Device discovery, explicit pairing/verification, unpairing, reconnect and battery.
- Files, URLs, transfer progress/cancel, phone file browsing and text clipboard.
- Android notification sync, supported inline replies, SMS, contacts and call alerts.
- Desktop media/volume control, find phone and presentation remote.
- Phone touchpad/keyboard and explicitly configured desktop commands.
- Phone-to-desktop screen viewing, optional authorized control, desktop-to-phone
  mirroring and genuine extended desktop output.

These are target capabilities, not a promise of parity on every Android version,
vendor build or app-store distribution. SMS/call access, background execution and
clipboard reads require verification against current platform/distribution rules.
Protected media may not be capturable. Phone viewing and phone control require
different permissions and implementations.

## Screen architecture

Phone to desktop: assess scrcpy as a USB developer/validation path. Its ADB
transport requires an authorized debugging device and is not a substitute for a
consumer companion's permission-based screen-sharing flow. The Android app's
production capture path should use approved platform screen-capture APIs and a
negotiated encoded stream; investigate control separately.

Desktop to phone: capture a selected physical output through the compositor.
Extended display: reuse the real virtual-output lifecycle and geometry already
in compositor/src/virtual_display.rs. Existing scripts/virtual-display-viewer.py
is an experimental USB browser viewer with raw RGBA frames, not a production
wireless backend. Add efficient encoding and a maintained transport only after
measuring available hardware, latency, bandwidth and dependencies. WebRTC is a
candidate to evaluate, not a committed implementation.

Start USB screen validation before LAN streaming. Require authenticated pairing,
explicit screen-sharing start/stop and visible active-session state. Refuse
capture while locked; clear pending buffers and blank the receiver. Already
received pixels cannot be revoked. Revoke input/control on lock, disconnect and
session end. Test reconnect, output removal and window recovery independently.

## Right-panel experience

SYSTEM remains available. CONNECT contains a device selector, connection/battery
header, a short set of contextual actions, notification list and transfer status.
Screen sharing has three named modes; an active preview can open a larger viewer.
Do not show inactive capability placeholders as working controls. Provide clear
not-installed, discovering, no-device, unpaired, disconnected, permission-denied,
unsupported, transferring, sharing and failure states.

Before removing APPS, preserve window activation, close, workspace/output moves,
pins, launch-new-window and keyboard navigation through the left dock/chooser,
launcher or overview. Audit Super+D, secondary screens and accessibility as part
of that migration. No unrelated feature is removed in the first backend step.

## Lightweight behavior

No video decoder/encoder until an explicit screen session. No video thumbnails
while idle. Use event-driven state updates, bounded transfer queues/caches,
backpressure and bounded reconnect backoff. An optional connection toggle can
stop discovery/service work when disabled. Measure actual idle RSS/PSS, wakeups,
CPU and active-transfer/video usage before setting numeric budgets; do not claim
Rust alone guarantees lower memory than the existing daemon.

## Delivery and acceptance gates

1. Pin upstream revisions, license inventory and Android toolchain/device check.
   Prototype basic authenticated pairing and reconnect; demonstrate two-way ping.
2. Native tab with actual device/battery state; file/link sharing and cancellation.
3. Clipboard, notifications/replies, media, find phone and permission handling.
4. Messaging/calls, file browsing, commands and compositor remote-input integration.
5. USB phone mirroring; permission-based companion capture; desktop mirroring and
   extended output; then authenticated wireless screen sessions.
6. Complete right Apps migration, remove duplicate tab, and profile/soak-test.

Each stage needs implementation, focused tests, real-device verification and
honest unsupported states before being marked complete. Cover spoofed/unpaired
peers, malformed packets, transfer traversal/overwrite handling, disconnects,
locked capture/input, service crashes and UI responsiveness. Never disturb the
owner's active terminals or replace the login session without reporting restart
requirements. Commit and push coherent verified changes per the user's preference.

## Sources inspected

- https://github.com/KDE/kdeconnect-kde/blob/master/core/daemon.h
- https://github.com/KDE/kdeconnect-kde/blob/master/core/device.h
- https://github.com/KDE/kdeconnect-android/blob/master/COPYING
- https://github.com/KDE/kdeconnect-kde/blob/master/plugins/mousepad/waylandremoteinput.cpp
- https://github.com/Genymobile/scrcpy
- docs/VIRTUAL-DISPLAY.md and compositor/src/virtual_display.rs

## Open decisions

Choose the backend approach before implementation. Recommended: option 2, with
upstream-compatible baseline device integration and separately negotiated DES
screen features. Confirm a physical Android test device and available Android
build tooling during stage 1. Full UI composition follows the chosen service
contract and shared DESIGN.md tokens.
