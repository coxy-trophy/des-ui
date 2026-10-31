# HeyClicky research for the DEs-UI companion

Research date: 2026-10-28. These are product claims from published material, not hands-on verification. Proposed ideas below are not implementation decisions.

## Identity and scope

Clicky was renamed HeyClicky. Current [HeyClicky](https://www.heyclicky.com/) is available for macOS and Windows. Its [changelog](https://www.heyclicky.com/changelog) describes voice questions about the screen, on-screen drawing and guided lessons, background agents, computer use, integrations, dictation and routines. It distinguishes conversational help from longer agent work. Capability availability varies by release and platform. The [original Clicky repository](https://github.com/farzaa/clicky) is MIT-licensed; its maintainer says subsequent work is private. The independently developed [OpenClicky](https://github.com/prasanthsasikumar/openclicky) describes an early Linux client using X11, not Wayland. Do not assume the original repository contains current HeyClicky features.

## DEs-UI today

[The companion README](../../companion/README.md) documents typed chat, explicitly started voice, captions, local conversation and approved memories, identity, social mode, usage limits, and secure key storage. It explicitly says there are no desktop execution tools, ambient screen reads, clipboard reads, file searches or continuous microphone monitoring. The README also says actual paid-provider, live microphone, native visual, acoustic and latency acceptance tests remain incomplete; test rather than assume those work in production.

## Ideas worth considering

1. **Ask about this**: explicitly attach a display, window, selected region, text or file to a question. Show the attachment and provider destination, let the user remove it, and respect lock and private windows. Distinguish one-shot capture from continuous observation and never silently retain screenshots as long-term memory.
2. **Point and teach**: a non-interactive, temporary highlight or annotation points to a control; a guided lesson waits for the user and supports next, repeat, skip and dismiss. Start with a single display and verify coordinates, scaling, screenshots and user input before adding richer drawing. Prefer accessibility coordinates or deterministic app APIs where available; mark uncertain visual targets.
3. **Desktop actions**: expose small typed tools for opening apps, settings, music and volume. Show activity/results, preserve user input, and use scoped consent and review for actions with external effects. Prefer existing desktop APIs over emulated clicks. Never type into the user's terminal by default.
4. **Longer jobs**: hand off research or other work to a visible background job with status, cancellation, progress, results and follow-up. Keep ordinary conversation responsive; do not launch agents merely because a model suggests one.
5. **Dictation and voice**: explore push-to-talk and explicit insertion into the focused field, a review step for uncertain text, and clear microphone/listening state. Protect terminals from accidental execution. First run real microphone, interruption, echo, latency, device-switch, and session cleanup checks for the current implementation.
6. **Conversation and memory**: improve the composer, formatting, copy/retry/stop, searchable history and larger view; let people inspect, edit and forget memories and understand what was sent to the provider.
7. **Selective suggestions**: if added, require an explicit user setting, explain what inputs are used, allow mute and dismissal, and keep suggestions inactive while locked, sharing or interrupted. HeyClicky's changelog says it removed proactive activity tracking and accessibility collection, and later paused suggestions; do not assume historical features are current.

## Lessons from HeyClicky's changes

Its [changelog](https://www.heyclicky.com/changelog) reports that overlays interfered with screenshots and sharing, caused focus problems and sometimes appeared on the wrong display; it also describes scope-limited computer-use consent, user takeover, dictation protections for terminals, and fixes for audio device changes. These are useful test cases for our existing compositor overlay and protected terminal sessions. The [privacy page](https://www.heyclicky.com/trust) says screenshots are requested via a hotkey and not retained raw, but prompts and generated screen analyses are retained and processing is cloud-based. These are vendor statements; do not equate no raw screenshot storage with no retained sensitive information.

## Suggested sequence

- Improve chat usability and perform real voice acceptance checks.
- Add explicit one-shot screen/region attachment with clear provider and retention disclosure.
- Add pointer-transparent guidance and verify input, multiple displays, screenshots, lock and focus.
- Add narrowly scoped desktop actions and a reviewable job model, then consider connected services.
- Revisit optional proactive help and local models after the earlier controls and user tests are reliable.

No paid calls, downloads, external account changes, or desktop actions were performed for this research.

## Implementation notes — 2026-10-28

The typed companion now has an explicit screen attachment with a removable preview; image bytes go only to that request and are not retained in the conversation database. The cursor voice card and social bubble use updated shared visual controls, and chat supports multiline text, copying replies and stopping a pending reply. Approved memories can be edited. Explicit voice screen sharing is implemented through Responses delegation with mocked protocol tests; live-provider acceptance, target annotations and AI-driven actions remain future work.

A compositor screen shot uses its focused display, but the existing Shot event does not include the display identity or its logical geometry. Screen-pointing coordinates must therefore not be interpreted against the primary display until the capture protocol records which display and geometry were captured and handles changes during a request.

GPT-Live voice image integration: [official OpenAI documentation](https://developers.openai.com/api/docs/guides/live-delegation) says the Live audio frontend does not accept images directly. With Responses delegation, queue the image as a Responses input item and then create or resume the backend response; do not add it to the Live `session.input` text history. The implementation must bound the image queue, handle a backend already in flight, clear pending images on stop/lock, surface provider errors, and verify no captured image is sent until the user explicitly shares it.
