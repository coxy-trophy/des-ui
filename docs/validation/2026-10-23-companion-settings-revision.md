# Companion Settings and thought bubbles validation

User correction: configuration stays in Settings; shortcuts trigger interaction; thought bubbles need polished motion. Reported failure was generic “Voice connection failed” with unconfirmed usage. No actual API connection was attempted by development tools.

Verified implementation defects repaired: one-second connection handshake allowance; stale listening/busy/key state after worker teardown/reconnect; empty provider diagnostic suppressing normal close explanations; indistinguishable HTTP/network failures. Voice handshake now allows10seconds then polls reads at1second, beneath independent startup/active deadlines. Confirmed pre-start failures release local reserved allowance; uncertain post-send failures do not.

Configuration resides directly in Settings → Companion. Text/voice surfaces own conversation and status only. Thought bubbles anchor once near the cursor, use shared material/theme/typography, fade in/out with a small rising thought trail, preserve stationary hit targets, expire after15seconds, and disable motion under reduced motion. New configurable global Keep/Mute/Stop/Quiet actions use typed bounded IPC. Quiet disables social consistently.

Evidence:

- `/tmp/desui-companion-revision-red.log`: new settings/motion tests failed before implementation.
- `/tmp/desui-companion-reconnect-red.log`: stale voice recovery test failed before repair.
- `python3 -m unittest discover -s companion`:50passed; `/tmp/desui-companion-revision-python.log`.
- `cargo test --offline --workspace companion -- --test-threads=2`:12passed (3common,9native); `/tmp/desui-companion-revision-workspace-tests.log`. This is a focused suite, not a full workspace pass.
- Full common suite:57tests plus1doctest passed (shortcut/IPC agent).
- Session15 and packaging6 Python tests passed (diagnostics agent).
- Strict UI static audit:zero findings; `/tmp/desui-companion-revision-design-audit.json`. Native Rust application; browser audits do not certify installed compositor behavior.
- Python compilation, changed shell script syntax and `git diff --check` passed.
- Independent reviewer verified Settings separation, recovery repairs, safe messages and pre-start accounting; Quiet toggle finding repaired to disable-only.

Real provider/account access, microphone/playback, installed Settings/shortcut focus, animations at actual output scaling and perceived voice quality remain untested. User must retry with the updated version to identify any remaining account/network failure using the improved message. No credentials were read or printed; no paid requests, installation or desktop restart occurred.

Final `cargo build --offline --locked --workspace` passed without warnings in1m16s; `/tmp/desui-companion-revision-build.log`. Running desktop was not replaced.
