# Wallpaper continuity correction

## Brief and evidence

The user requires a cohesive modern, futuristic desktop, with blurred wallpaper visible throughout the terminal background and sharp foreground content. The supplied screenshot (2026-10-20 17-01-51) shows an opaque black terminal surrounded by a broad grey border. Read-only inspection of the installed appearance entry confirmed a wallpaper combined with v1 Solid, blur 7, dim 50 and opacity 91.

The implementation previously kept Solid after image selection, forced terminal tint to at least 90%, applied faint chrome-label contrast to the whole terminal, left workspace padding on the un-tinted wallpaper and painted an opaque gutter. These mechanisms prevented the intended continuous material.

## Correction

- Image selection establishes Frosted with a 62% tint when previously Solid.
- Version 2 persistence distinguishes explicit Opaque choices. Legacy v1 wallpaper/Solid migrates to Frosted, tint 62, blur at least 18 and dim at most 35. No home configuration file was modified by development tools.
- Terminal tint supports 35–100%; normal bright terminal text uses a conservative 4.5:1 contrast floor separate from faint control-label ink. Terminal apps retain explicit ANSI backgrounds, reverse video, selection and cursor behavior. Dim/custom ANSI colours are not given a universal contrast guarantee.
- Entire terminal workspace, pane padding, split gaps and gutter share screen-aligned backdrop sampling. Text is never blurred.
- Reduce transparency and explicit Opaque remain available. Controls and notifications retain stronger tint for legibility. The unavailable Liquid glass control was removed from the ordinary editor.
- DESIGN.md now carries the user's visual standard and requires native rendered acceptance for future features.

## Verification

Observed failing regression: terminal alpha was 0.9137 despite a requested 55% tint. Observed failing migration regression: legacy wallpaper remained Solid. Automated final results are recorded below. The premium UI static audit reported zero findings; it does not inspect Rust native rendering and is not visual evidence. Native installed-session visual review remains pending in docs/UNTESTED.md. Build/tests cannot establish that the result meets the user's aesthetic standard.

Final verification: `cargo test --workspace --locked --offline --no-fail-fast -j2`: **570 passed, 9 failed**. Failures are unchanged sandbox restrictions: two compositor IPC, four shell transport/login, three private tmux socket tests. All three new regressions passed. `cargo build --workspace --locked --offline -j2` passed without warnings; `git diff --check` passed. Source snapshot `/tmp/desui-glass-correction-source.json` matches the tested source. Installed binaries/session were not replaced or restarted.
