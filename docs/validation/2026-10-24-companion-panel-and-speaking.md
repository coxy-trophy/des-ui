# Saved key, bottom Chat and speaking cursor —2026-10-24

Requested from screenshot: replace large floating chat with bottom Files/Chat tabs, open Chat directly with Super+J, voice independently with Super+Ctrl+J, remember the API key, and indicate actual speaking with cursor/thought motion.

## Implemented

Chat uses shared bottom-panel geometry. It has a bounded scrolling conversation and compact Enter-to-send composer; no floating chat modal is drawn. Files/Chat selection preserves history/draft. Escape returns keyboard ownership; scoped compositor CompanionChat/CompanionBlur releases the overlay before outside native-app focus or divider routing. Opening other app views only blurs text; explicit voice continues independently.

Secret Service/libsecret stores the key when Remember API key securely is enabled (default). Bounded background operations restore on startup, expose locked/unavailable states, and ignore stale results. Remove clears saved/runtime key and disables future autoload (explicit environment fallback remains). No plaintext credential fallback. Runtime package metadata includes the Secret Service dependencies. Capability introspection verified local Secret typelib/signatures without opening a real keyring or fetching a saved key.

Voice starts only after credential restoration completes, without selecting Chat. Stop cancels a queued start; Keep can be queued until the session is ready. Speaking follows nonempty queued output PCM and returns to listening on drainage. The cursor's configured solid silhouette pulses modestly around its unchanged hotspot, respects reduced motion and expires without shell heartbeats. A separate render-only caption crop is excluded from keyboard/pointer regions. Pointer anchors are shared only on explicit voice invocation or active visual heartbeats, never idle. Pulse supports all outputs; captions currently use the primary shell surface and are suppressed on secondary-pointer anchors.

## Evidence

- New panel geometry/Escape tests failed before panel_ui/chat_selected existed: `/tmp/desui-companion-panel-red.log`.
- Pending-start Stop regression failed before repair: `/tmp/desui-companion-panel-stop-red.log`.
- `cargo test --offline -p des-ui companion:: -- --test-threads=2`: **12 passed**; `/tmp/desui-companion-panel-tests.log`. Includes Enter consumption/draft/focus, credential-ready start once, Stop revocation, no modal, narrow/wide panel, failed worker and queue teardown.
- `PYTHONPATH=companion:. python3 -m unittest discover -s companion`: **56 passed**; `/tmp/desui-companion-panel-python-tests.log`. Fake keyrings/transports/audio only.
- Agent full common/compositor run: **61 common,147 compositor,1 doctest passed**, excluding the two known compositor socket tests whose full runs fail with sandbox bind EPERM; `/tmp/companion-focus-tests.log`. Earlier full failures recorded in `/tmp/companion-cursor-tests.log`. No clean full-workspace-test claim.
- Session integration **15 passed**, packaging unit **6 passed**; `/tmp/desui-companion-panel-session-tests.log`, `/tmp/desui-companion-panel-packaging-tests.log`.
- `cargo build --offline --locked --workspace`: **passed without warnings**,1m16s; `/tmp/desui-companion-panel-build.log`.
- Strict static UI audit zero findings; `/tmp/desui-companion-panel-design-audit.json`. Python compilation, changed shell script syntax and `git diff --check` passed.

Independent review found the outside-native-app keyboard retention bug; its scoped compositor release was implemented and reviewed. The reviewer confirmed key-load waiting, independent voice, error notice, Enter isolation, saved-key privacy, ephemeral indicators and pointer-transparent captions.

## Still untested

Real keyring save/restore/unlock, real API and microphone/playback, installed desktop focus and perceived animation quality, output scaling/secondary captions remain in docs/UNTESTED.md. A locked login keyring may need unlocking once per desktop login. No real saved key was read/written by development tools; no paid call, installation or desktop restart occurred. Workspace remains uncommitted.
