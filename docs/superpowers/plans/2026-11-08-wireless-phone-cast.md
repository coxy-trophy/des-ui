# Wireless Phone Cast Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development for the Android capture, daemon, and shell components. Validate the combined build before installation.

**Goal:** Cast a paired Android phone into the CONNECT tab and use clicks, drags, and Android navigation from the PC after explicit phone approval.

**Architecture:** Reuse the existing pinned TLS connection and inline CONNECT preview. Android MediaProjection produces bounded JPEG frames only for an approved cast; a separately enabled, gesture-only accessibility service handles authorized input.

**Tech Stack:** Android Java/platform APIs; Rust DES Connect daemon; native egui CONNECT panel.

**Spec:** [Approved companion design](../specs/2026-11-08-des-connect-companion-design.md), milestone 2 as clarified to include opt-in phone control.

## Global constraints

LAN only, existing protocol-8 TLS and pins, no new listening socket, no clipboard, keyboard injection, or capture persistence. Capture and remote input are separate grants. Close on either lock, unpair, explicit stop, peer or lease loss. Never resume a revoked cast automatically. Keep the existing USB path. Bound all media and commands; use one latest frame rather than a video queue. Do not replace the installed fork or restart the live shell during isolated tests.

## Wire and UI contract

Secure identities advertise `des.connect.cast.v1` in both incoming and outgoing capabilities. Same packet type in either direction; every body contains `op` and random session `id` (32 lowercase hex chars). Desktop sends `start`, `lease` (about once per second, including while awaiting consent), `stop`, or `input`. Phone sends `status` with `phase: "live" | "stopped"` and `control: bool`, or `frame` with `jpeg` (standard padded Base64 of JPEG, max 44 KiB decoded, axis max 960; target 5 fps). Every live status and frame includes `geometry`, a positive integer that increases on resize/rotation. Clear cached frames on a geometry change. A lease expires after about 3 seconds once active; request consent expires after 60 seconds. Local input contains `action: "tap" | "swipe" | "back" | "home" | "recents"`, `geometry`, `x,y` for a tap or `x,y,to_x,to_y` for a swipe, integers normalized to 0–10000 within the preview image. Recheck matching geometry and control permission before dispatch. Control requires a full display capture that matches current display geometry.

Local RPC: `cast_phone` with `device_id`; `stop_screen` remains shared; `cast_input` adds `device_id, serial` (the cast ID) and the input fields above. Snapshot devices add `wireless_cast` bool; screens use `mode: "wireless"`, `serial` as the opaque cast ID, `running: true` during consent/live, `control` true only after approval. `screen_frame` uses existing device and serial selector; new binary reply is `JPEG <length> <geometry>\n` followed by JPEG, or an existing status line. The shell worker verifies decoded dimensions and carries geometry alongside the texture; pointer mapping uses the actual letterboxed image rectangle. Polling renews a local viewer lease; a lost viewer stops the cast. Reuse existing buttons, pending status and Stop affordance.

## Review focus

- No unpaired, unrequested, mismatched or stale cast can yield pixels or input.
- Consent is explicit on the phone; granting view never grants remote touch.
- Lock, link loss, Stop and rotation clear or invalidate old pixels/coordinates.
- Oversize and malformed data cannot trigger unbounded allocation or queued input.
- Existing USB screen actions and side-by-side companion install remain available.

## Tasks

- [x] Android: add protocol and lifecycle tests, then implement MediaProjection foreground service, permission flow, bounded latest frame, Android input service and lease/session revocation. Build and sign the APK.
- [x] Daemon: add isolated TLS integration tests, then implement cast session, capability negotiation, lock/peer/lease checks, local frame and input IPC. Run the Connect suite.
- [x] Shell: test bounded frame decoding and click/drag mapping, then integrate wireless cast UI and shared preview. Run targeted shell checks.
- [x] Use an isolated daemon and a disposable emulator to demonstrate visible motion, a remote click, rotation, Stop and lock revocation. Record the physical realme's verification separately.

Verification: the Android 15 emulator shared its live Settings screen into a
native CONNECT panel. A PC click opened Network & internet. Rotation replaced
the old geometry and restored control. The desktop Stop button, a simulated
desktop lock, and Android screen-off each stopped the cast; Android unlock did
not resume it. A separate native input fixture verified drag mapping and refusal
in view-only mode and outside the picture. The physical Android 14 realme
paired over Wi-Fi, showed Calculator in the native CONNECT panel, accepted a
desktop click that entered `7`, and stopped from the panel. During device
testing, avoid `uiautomator dump` while control is live: Android's default
[UiAutomation flags](https://developer.android.com/reference/android/app/UiAutomation#FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
suppress accessibility services, correctly revoking the control session.
Four JVM suites, 99 desktop service tests and 32 focused shell tests passed.
The desktop binaries and signed APK built.
