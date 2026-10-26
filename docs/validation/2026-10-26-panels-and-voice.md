# Panel spacing and companion voice — 26 October 2026

## Implemented

- The right panel starts on System. Apps selection remains explicit through its tab or Super+D; existing panel visibility settings are retained.
- Shared `common::frame::Layout` defines primary-display side cards/rails, center/header, Files/Chat and the native workspace. Frame gaps and corner radii are 8 logical pixels. A 4px workspace rim keeps rectangular native windows within the rounded center. Fullscreen and secondary-display window placement remain unchanged.
- Shared material painting retains themes, wallpaper alignment and reduced transparency. Existing startup progress draws rounded outlines without connecting across gutters. No new animation timer or per-frame image processing was added.
- The companion's server-owned bearer WebSocket omits websocket-client's synthetic Origin header. Before the fix, an authenticated connection using the stored local key returned HTTP403 with the default Origin; the same request with Origin suppressed was accepted. The fixed `_connect` helper was separately verified to establish the authenticated handshake.
- Provider rejection and audio-device startup errors now provide safe actionable messages rather than being collapsed into a generic connection failure. Partially initialized audio devices are stopped on failure.

## Verification

`CARGO_BUILD_JOBS=1 nice -n 10 ./scripts/verify.sh` passed: script syntax, 80 Python tests (15 session, 6 packaging, 59 companion), 757 Rust unit tests, 3 CLI integration tests, 1 doctest and locked debug workspace build. No compiler warnings. `git diff --check` passed.

Three frame tests cover channel widths, center/native insets, collapse versus hide, and all panel combinations at five small/fractional/standard display sizes. Voice transport and error regression tests were observed failing before their repairs.

A private Xvfb native-shell preview with simulated compositor state was captured and inspected at 1280×900. System is visibly selected, all four frame regions fit, and the new gaps/corners remain consistent. Preview files stay local, alongside the excluded raw machine-testing captures. This proves shell rendering, not installed DRM/native-client acceptance.

The strict frontend design audit reports zero findings. Independent read-only review confirmed native rectangular corners fit within the 4px rim and preserved unrelated startup code; an obsolete `area.halves()` reference found during integration was corrected to the new canonical workspace.

## Scope and activation

The authenticated voice check sent no `session.start`, microphone audio, or conversation content, and did not request a paid conversation. Real microphone/playback quality and a complete spoken conversation remain untested. No key value, authorization header or provider body was printed or committed.

No installed binaries/helpers were replaced and no running desktop, application or terminal was restarted. Use the existing installer to rebuild/install and activate matching shell/compositor builds at the next login. Do not hot-reload only the new shell onto the old compositor geometry. The existing shell-only on-screen keyboard scope is unchanged; no new native-application OSK reservation protocol was introduced.

Official protocol reference: [OpenAI voice WebSockets](https://developers.openai.com/api/docs/guides/voice-websockets). The Origin policy diagnosis above is based on the controlled authenticated comparison, not an assertion that every account will return the same refusal.
