# Settings and native Files validation

Implemented responsive Settings navigation, Appearance tabs, shared native image picker and Files workspace. Files supports folder navigation/search, document opening, new folder, rename, copy, cut/move, paste, home Trash and Restore, asynchronous progress and cooperative cancellation.

Safety: destinations are never overwritten, copy stages privately, symlinks are preserved without traversal, failed cross-filesystem moves retain the source, Trash recovery metadata precedes movement and service directories cannot be trashed. Filesystem tests use temporary directories. Home Trash follows the [Freedesktop Trash layout](https://specifications.freedesktop.org/trash/latest/), with deliberately stricter private metadata validation; per-volume Trash and arbitrary external metadata compatibility are not claimed.

Workspace run before final narrow-layout regression: 566 passed, 9 failed. The nine failures are the existing sandbox socket/private-tmux restrictions (two compositor IPC, four shell service transport, three tmux tests). The added narrow-layout regression passed separately. Final verification results are recorded below after the final build/test run.

Native compositor/installed-session visual verification is unavailable here. See docs/UNTESTED.md for acceptance checks. No installation or shell restart performed.

Final verification: **567 passed, 9 failed**; failures remain the nine sandbox-only cases listed above. `cargo build --workspace --locked --offline -j2` passed without warnings. `git diff --check` passed. Final source hashes are saved in `/tmp/desui-settings-files-source.json`.
