# Cursor Companion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Use focused ownership and review the integrated behavior.

**Goal:** A native social cursor companion with persistent shared text/voice context, self-authored identity and on-demand five-minute voice sessions.
**Architecture:** Shell overlay and configurable desktop shortcut; private stdin/stdout JSON worker process; Python standard-library persistence/HTTPS plus maintained websocket-client transport and PipeWire audio. No compositor networking.
**Tech Stack:** Rust/egui, Python3/sqlite3, OpenAI Responses and GPT-Live, websocket-client, PipeWire.
**Spec:** docs/superpowers/specs/2026-10-23-cursor-companion-design.md (approved by user's Proceed).

## Global Constraints

Preserve existing dirty work. Work in current checkout; no install/restart/commit. No paid API tests or credential disclosure. No predefined character, ambient screen reads, arbitrary tools or silent voice reconnect. App-owned identity cannot alter permissions. Bounded queues/history/audio; monotonic hard cap300seconds and explicit resume. Closing overlay/lock/disconnect stops microphone/playback. Mock provider tests; document hardware/account checks honestly.

## Review Focus

Focus isolation, stale callbacks, bounded shutdown, credentials outside ordinary config, private persistence, interruption and truthful usage/failure states.

## Interfaces

Worker: `python3 companion/service.py` (installed path beside shared des-ui assets). JSON-lines commands: `status`, `configure` (model, enabled, social, runtime-only api_key), `send` (text), `cancel`, `voice_start`, `voice_keep`, `voice_stop`, `voice_mute` (muted bool), `identity` (text), `remember` (text), `forget` (id), `clear`, `reset_identity`, `social` (suppressed bool), `shutdown`. Every command carries `id` integer. Events are JSON objects: `state` (snapshot fields), `error` (message), `voice` (state, remaining, muted, kept, finalized), `reply` (text, proactive bool). Snapshot includes messages[{role,text}], identity string, memories[{id,text}], enabled, social, model, configured bool, busy bool. Credentials runtime only and redacted from all events/errors. Environment OPENAI_API_KEY supported. Single active operation; cancel discards late results. Voice implementation callback receives transcript(role,text) + event(state fields); supplies start(context,instructions,api_key,model), keep(), mute(bool), stop(reason). `LiveVoice` in companion/voice.py owns all audio/network lifecycle. Service context is bounded and immutable per request.

## Tasks

1. Core worker: red tests for private memory/history, cancellation and malformed provider output; implement Responses adapter, private SQLite persistence, self-authored versioned identity, memory management, bounded social opportunities and JSON protocol. Owner: root.
2. Voice: red lifecycle tests with fake clock/transport/audio; implement documented GPT-Live transport, PipeWire audio, startup/error/interruption, quick follow-up, five-minute cap and explicit restart. Owner: focused voice agent, only companion/voice.py + voice tests.
3. Native integration: shell bridge/overlay, shared shortcuts/settings, packaging assets/dependency checks. Owner: focused shell agent. No worker internals.
4. Integrate protocol, exercise mocked worker end-to-end, targeted Rust/Python tests and build; independent review, repair findings, update DESIGN/UX contract/UNTESTED and validation record.

## Execution ledger

- Ruling: use the established Rust desktop and a Python helper rather than a new web agent framework; existing stack and installed Python/PipeWire make it coherent.
- Ruling: use Responses delegation for Live with the same bounded local identity/history; no desktop tools. Typed identity updates remain separately schema validated.
- Ruling: credentials may be supplied for this worker lifetime in the companion setup surface; environment fallback. No disk credential file.

### Completed implementation and review

- Core: separate private worker, Responses adapter, schema validation, runtime-only credentials, bounded persistent history, self-authored versioned identity and approved memory controls. Social unanswered state survives restart. Forty mocked Python tests cover core and voice.
- Voice: maintained websocket-client transport, documented GPT-Live protocol, default PipeWire devices, bounded audio, independent monotonic watchdog, explicit keep/mute/stop/resume, graceful finalization and Linux parent-death audio cleanup.
- Native: anchored shared-material modal, Settings entry, configurable text/voice shortcuts, focus isolation, live captions, opted-in non-focus social bubble, usage/setup/memory controls. Six native headless/control tests pass.
- Review findings repaired: invalid/missing terminal usage could refund allowance; accepted transcript fragments could be lost on closure; captions could merge into prior typed turns; setup could erase a key on model change; Quiet could no-op; social initiation could block direct voice summon; Send could discard input during active voice. Bounded control queue failure now tears down the worker.
- Ruling: social bubble uses exposed shell input, without lifting a keyboard overlay. Reviewer verified full-screen shell mapping and withdrew its initial routing concern.
- Ruling: spoken keep-command recognition, local wake detector, explicit screenshot/file attachments, and text token streaming are deferred and documented. Explicit Keep/voice shortcuts work without these; no ambient context is read.
- Ruling: default PipeWire source/sink follows desktop Sound controls. A separate companion device selector is deferred.
- Verification: 710 Rust unit tests plus one doctest passed; nine existing socket/tmux tests are blocked by sandbox permissions. All six companion tests pass again after final UI wording changes. The unrelated terminal backpressure timeout passed in this workspace run. Fifteen session/packaging integration tests and six packaging unit tests pass. Final build and detailed evidence in docs/validation/2026-10-23-cursor-companion.md.
