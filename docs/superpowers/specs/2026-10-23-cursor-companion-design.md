# Cursor companion

Status: approved for implementation on2026-10-23. OpenAI-first text and GPT-Live voice; no paid API calls made during development.

## User decisions

The cursor should become an interactive companion with continuity and learning. Claude is available; the user is now exploring obtaining an OpenAI API key for more natural live voice. The user approved OpenAI Responses with GPT-Live voice and client-owned memory; Claude remains a possible later provider. Its personality must develop through interaction, without a predefined temperament. The user selected Social initiative and both typed and spoken conversation. The existing simple solid shape-morph cursor remains the visual foundation.

## Proposed experience

A configurable shortcut opens a small conversation bubble near the cursor. It stays clear of the pointer's hotspot and expands into a bounded conversation surface for longer replies. After opening, it remains anchored while the pointer moves; there is no moving text field to chase. Text and voice share one conversation, transcript and memory. Escape closes the surface and restores the prior focus. Ordinary pointing and clicking always remain immediate.

Social mode lets the companion initiate conversations, ask questions, suggest topics and follow up on prior discussions. Local events such as returning to the desktop or resuming a conversation offer opportunities; the model can choose whether to initiate and what to say. No API request is tied to mouse motion. There is at most one unanswered approach; ignored bubbles fade without repeated reminders. Focus, fullscreen, lock, capture and active drags suppress unsolicited approaches. Quiet/Snooze controls and remembered feedback adjust frequency. These are interaction boundaries, not a prescribed personality.

Proposed first-version voice behavior: explicitly start a visible voice session, with a push-to-talk option and a clear microphone indicator. With GPT-Live, enable full-duplex conversation within that session; local transcription mode retains transcript review before sending. Spoken replies accompany voice conversations, with a visible transcript and immediate Stop/Mute. Proactive conversation begins as a text bubble; unprompted spoken approaches require a separate opt-in. No always-on microphone or wake-word mode in the first version.

## Evolving identity and memory

The companion can choose a name, self-description, conversational style, interests and habits through an editable identity journal. This begins without a hardcoded personality profile. Identity evolves gradually from conversation, with prior versions available for rollback. A self-description expresses the companion's style; it must not invent access, capabilities or shared history.

Store conversation history, user-approved facts/preferences, identity and follow-up notes separately in private local storage. Mark user statements separately from model inferences. Provide inspect, correct, forget, clear conversation and reset identity controls. Only relevant bounded context goes into a provider request; local storage is not a promise that selected memory will never be sent to the API. Permission settings are separate from self-authored identity and cannot be rewritten by it.

## Proposed technical approach

Keep the compositor responsible only for local cursor appearance/input and overlay ownership. A separate companion service handles provider requests, streaming, cancellation, memory and speech work; no network/model processing runs in the compositor or UI paint loop. Use bounded queues and generation IDs to discard stale replies. A reconnectable local authenticated interface ties the service to the shell. Shell restart preserves conversation through durable state; disconnect does not leave microphone capture or speech running.

Use OpenAI Responses for text conversation and structured, schema-validated memory updates. Screenshots, selected text and files are shared only through an explicit context attachment. First version is a conversational companion, with no arbitrary desktop execution, terminal commands or unrestricted file tools. API errors and offline status remain visible and retryable. Credentials use the system secret service where available or a runtime environment credential; never the ordinary settings file, identity journal or logs.

Preferred voice candidate for evaluation: PipeWire capture/playback with OpenAI GPT-Live, keeping the identity/memory/backend service independent. Client delegation can connect Claude, while Responses delegation can use an OpenAI backend. Text and voice draw from the same application-owned identity and memories. Local alternative: PipeWire capture → whisper.cpp transcription → Claude conversation → Piper synthesis → playback; these local engines/models are not installed here. Voice setup must report actual provider/model access or missing local components. Bound audio buffers and queues; Stop closes recording, playback and network sessions. Muting is distinct from closing a billable session.

Settings owns enable/pause, API setup/model choice, voice setup, microphone/output device, push-to-talk mapping, speech playback, social frequency, quiet periods, memory and an API usage limit. Visible listening/thinking/speaking/offline states use existing shared typography, material, theme and reduced-motion owners. No extra decorative cursor appendages or constant idle animation.

## Verification and delivery

Test streaming/cancellation/reconnect, memory boundaries and retention, prompt/tool-result separation, focus and event isolation, ignored proactive turns, quiet modes, credential handling, speech lifecycle/error states, missing engines/models, bounded work and narrow/wide layouts. Use mocked API responses for automated checks; real Claude integration requires user-provided local credentials and explicit live testing. Voice quality, latency, CPU use and native visual acceptance require hardware checks.

Suggested implementation order: service and typed conversations; self-authored identity and editable memory; bounded Social initiative; speech capture/transcription/playback; full native integration and acceptance. Both input modes remain part of the first complete companion release, rather than describing incomplete voice support as finished.

Existing installed-session cursor/gesture checks and the unrelated terminal backpressure test timeout remain open. No installation or session restart is implied by this design review.

## Sources

- Claude vision: https://platform.claude.com/docs/en/build-with-claude/vision
- Claude tool use: https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview
- Client-managed memory: https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool
- Local transcription: https://github.com/ggml-org/whisper.cpp
- Local speech synthesis: https://github.com/OHF-Voice/piper1-gpl


## OpenAI live-voice exploration — 2026-10-23

The user asked to explore an OpenAI API key and more natural spoken conversation. No key was requested in chat, no paid call was made and no provider implementation has started.

Official OpenAI documentation currently recommends GPT-Live for full-duplex conversation with a separate reasoning/backend service. Realtime is an alternative for speech, reasoning and tools in one model/session. A chained transcription/text/synthesis pipeline offers stage-by-stage control.

Options for this project:

1. GPT-Live + OpenAI Responses backend: one provider/account and managed conversation delegation; app retains permissions, identity/memory and its tools.
2. GPT-Live + Claude through client delegation: keeps the existing Claude option and gives us direct control of backend context/results, with additional provider integration work.
3. Claude + local whisper.cpp/Piper: retains the original speech pipeline as an optional local speech path; actual quality/latency depends on hardware and installed models.

Recommendation: evaluate GPT-Live as the preferred spoken interaction layer while preserving a provider-independent companion core. Confirm account access and test real interruption/latency/voice behavior before selecting the final release path. Design for one coherent identity across text, voice and backend choices; do not build separate companion personalities for separate providers.

Cost snapshot checked in official docs: gpt-live-1 is listed at $0.05 per minute, billed per second, with backend model/tool usage separate. An active30-minute session is $1.50 for voice alone; one hour is $3. Silence and backend work count as active time, and muting does not close billing. WebRTC session creation initially bills15seconds credited toward a running session. Close idle sessions and provide usage/budget controls; do not keep a voice session open throughout a working day just to support social text bubbles. Recheck pricing/access before implementation and live testing.

Sources:
- https://developers.openai.com/api/docs/guides/live
- https://developers.openai.com/api/docs/guides/voice-agents
- https://developers.openai.com/api/docs/models/gpt-live-1
- https://developers.openai.com/api/docs/guides/voice-latency-cost


## On-demand voice and five-minute cap

User direction: summon the companion for a quick exchange, optionally keep the voice conversation running, and enforce a five-minute session cap with explicit restart to continue. This is an application limit, not a stated provider limit.

- Quick summon: configurable shortcut, cursor action or deliberate gesture starts a voice connection. Show Connecting/Listening state. Buffer only explicit summon audio locally until the session is ready, then deliver it once. After the reply and a short follow-up opportunity, close the voice session.
- Keep talking: an explicit control or recognized conversation command keeps the current voice session open until Stop or the same five-minute cap. It does not reset the timer or bypass the cap.
- Five-minute limit: maintain a monotonic app deadline for the active voice session, with a visible countdown and a quiet approaching-limit indicator. At the deadline stop capture/playback and request session closure. Finalize transport/usage with a bounded cleanup timeout; do not claim provider billing ends at exactly the local deadline when finalization is unconfirmed.
- Restart is explicit: never silently open a replacement paid session at expiry. Show Resume voice, starting a new connection and timer only on user action.
- Preserve continuity: store identity, relevant text history and conversation state independently of the voice connection. Restarting voice can continue the same topic; clearing conversation or resetting identity are separate explicit actions. Mark cutoff replies/partial transcripts honestly and reject late audio/results from expired sessions.
- Closed voice sessions cannot hear a summon. Shortcuts/gestures work locally. Optional spoken wake requires a separate local wake detector and visible microphone controls; it is not achieved by leaving GPT-Live connected while described as idle.
- Account for connection startup delay and session initialization charges when choosing the brief quick-mode follow-up window. Muting does not close a billable voice session.

Official lifecycle source: https://developers.openai.com/api/docs/guides/live-conversations
