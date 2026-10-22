# Shortcut reference and configuration

Removed the static KEYS section from the right panel. Added a searchable shared-material reference (Super+/ desktop, Ctrl+Shift+H shell), a launcher entry and Settings → Shortcuts. The shared registry drives persisted overrides, desktop/shell/terminal-find/prompt routing, reference rows and tooltip hints. Users can enter mappings, explicitly record Ctrl combinations, disable/reset actions and confirm a bulk reset. Reserved controls and actual alias conflicts are validated.

Linux Super recording is not falsely advertised: the UI toolkit omits that modifier, so desktop chords use explicit entry. Manual entry preserves editing/paste keys. Background raw events are suspended before drawing other workspaces while the reference owns input. Lock/authentication and screenshot marking take priority. Compositor reload retries after disconnection/queue pressure.

Observed failing regressions established shell routing replacement, narrow reference layout and modal Escape reaching the underlying launcher. Additional checks cover manual editing and recording conflicts. Native Area first frames are invisible sizing passes; layout tests settle those passes and assert the real modal Area bounds at320/360/900 pixels, including vertical bounds. Persistence tests use temporary files and distinguish failed writes from post-rename durability warnings. Shifted punctuation/navigation keys share normalization.

Read-only review found no further concrete Important/Critical issue in parent routing/persistence; native global compositor input, installed overlay rendering and visual acceptance remain pending. No installed binaries or session were replaced/restarted. Matching updated shell and compositor are required for global Super bindings; the shell fallback remains available independently.

Final verification results follow below.

Final workspace suite: **596 passed, 9 failed**. The nine unchanged sandbox failures are two compositor IPC, four shell service/login transports and three private tmux socket tests. The new IPC doctest passes. All shortcut/model/UI/routing/persistence regressions pass. Source snapshot: `/tmp/desui-shortcuts-source.json`; strict static UI audit and `git diff --check` pass. The static auditor does not inspect native Rust rendering.

Final build: `cargo build --workspace --locked --offline -j2` completed successfully without warnings. Source hashes remained unchanged after verification; `git diff --check` passed.
