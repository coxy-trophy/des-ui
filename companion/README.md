# Cursor companion

Configure the companion in **Settings → Companion**. Select the bottom panel’s Chat tab with **Super+J** (desktop) / **Ctrl+Shift+J** (shell), or switch between Files and Chat using its tabs. For direct voice summon use **Super+Ctrl+J** (desktop) / **Ctrl+Shift+U** (shell). All mappings are configurable in Keyboard settings. Enter your OpenAI API key in Settings, apply setup, and enable the companion. With Remember API key securely enabled, the key is stored in the system Secret Service and restored automatically after shell restart. It is never written into ordinary settings or the journal. A locked login keyring may need unlocking. Alternatively, inherit `OPENAI_API_KEY` when starting the shell. Do not paste credentials into the conversation.

The default Responses model is `gpt-5.6-luna`, editable for account access. Voice uses `gpt-live-1` with Responses delegation to that same backend. Both use local conversation, identity and approved memories. The identity begins empty; the model chooses how to describe itself through typed exchanges. Voice history also contributes to future context. The journal is conversational data and cannot grant permissions. There are no desktop execution tools, ambient screen reads, clipboard reads, file searches or continuous microphone monitoring.

Use **Start / resume voice** to connect and listen. Quick mode closes after a reply and a brief follow-up opportunity. **Keep talking** keeps it open, with the same hard five-minute deadline from connection startup. **Mute** stops local capture but leaves the billable connection open. **Stop** closes it. Resume is always explicit; context survives a new connection. This version uses the Keep talking button; recognizing the spoken phrase as a control is deferred. There is no spoken wake detector while disconnected.

Voice requires Python3, `websocket-client` (`python3-websocket` on Debian/Ubuntu), and PipeWire's `pw-record` / `pw-play`. Text uses Python's standard library. Audio uses default PipeWire source/sink at mono PCM16 24kHz. Select devices in the desktop Sound controls. Raw recordings are not stored. Linux audio helpers die with the worker, including an unexpected worker crash.

Daily controls count typed requests and voice starts; delegated backend responses have separate token costs. A five-minute voice allowance is reserved before each connection and adjusted only after confirmed provider finalization. Interrupted/unconfirmed sessions conservatively retain that allowance. These limits are application controls, not an exact dollar budget or a guarantee of provider billing cutoff. The interface reports connection state and finalization. A Live session may bill while silent or muted.

Conversation, approved memories, identity versions, usage counters and non-secret settings live under `$XDG_STATE_HOME/des-ui-companion/companion.sqlite3` (fallback `~/.local/state`). The directory is mode 0700 and database mode 0600. History is capped at 200 messages and 96,000 characters; each request selects at most 32 recent messages  /24,000 characters plus bounded identity and approved memories. Memory is capped at 128 entries  /32,000 characters. Only selected context is sent to OpenAI. Forgetting a memory does not remove a separately stored conversation message containing that information: use Clear conversation too when desired. Reset identity removes its journal versions; Clear conversation preserves identity and approved memories.

Tests use mocked providers, transports and audio. No actual paid API, live microphone, installed-session focus, acoustic echo/interruption quality, visual acceptance or latency check has been completed in this environment. Primary WebSocket output has no turn-completed event: quick mode estimates inactivity from audio and delegated work. Headset testing is recommended before judging conversational quality.

Official references (checked 2026-10-23):

- https://developers.openai.com/api/docs/guides/voice-websockets
- https://developers.openai.com/api/docs/guides/live-conversations
- https://developers.openai.com/api/docs/guides/live-delegation
- https://developers.openai.com/api/docs/guides/structured-outputs
- https://developers.openai.com/api/docs/models/gpt-5.6-luna

Connection recovery: voice now reports safe authentication/access, handshake, timeout, dependency, DNS and TLS errors. Reconnect clears stale listening state. The handshake has a ten-second timeout; failures before attempting session.start release the local reserved voice allowance. Account access and live voice still require a real test.

Speaking indication: voice is independent of text Chat. Actual queued output PCM controls a restrained solid-cursor pulse and a pointer-transparent thought caption; reduced motion removes the pulse. Keep is Super+Shift+J; Stop, mute and Quiet retain configurable mappings in Settings. The caption currently uses the primary display; cursor pulsing supports all outputs.
