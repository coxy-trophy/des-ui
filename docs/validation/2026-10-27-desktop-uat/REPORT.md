# Desktop user test — 2026-10-27

> Captures and logs that show private content are kept on the test machine only, so some
> evidence links below open nothing here.

**Combined result: 37 scenarios, 26 passed, 9 findings, 2 inconclusive.** First pass: 28 scenarios, 19 passed, 8 findings, 1 inconclusive. No implementation fixes were made during this pass. Interactive testing stopped at 7% battery while the charger question was pending. This is not a claim that the entire desktop has been tested.

## Scope and method

Used the installed desktop through real pointer/keyboard input, with native screenshots. The native computer-control surface was unavailable; command-line input automation had already been explicitly authorized by the user. The CLI was used to open the initial Health page, capture evidence, and verify state; subsequent workflows used user-facing controls. Disposable fixture creation and byte verification used file tools. No private browser pages were explored, and no real account/library mutations were performed.

The installed shell includes the revised Apps shelf, but its binary does **not** contain the newer Spotify library actions or visible Music volume component. Their absence is a version gap, not a new implementation finding. Binary hashes are recorded in baseline.json.

Screenshots/logs are local and private (directory 0700, files 0600). Some evidence includes account labels or a terminal view revealed by Music closing. Do not publish raw captures. Capture itself replaces the clipboard with a PNG and posts a notification; these are test side effects, not independent notification tests.

## Findings

### F-01 — P2: Volume slider ignores keys after mouse focus

**Reproduce:** Sound & screen → click volume around 71% → press End, then repeat with Right.
**Observed:** The value and PipeWire level stayed at 71%. Mouse movement restored it to 100%.
**Expected / next check:** Keyboard adjustment should work after the intended slider is focused. Tab-focused behavior needs a separate test.
Evidence: [07-volume-keyboard-immediate.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/07-volume-keyboard-immediate.png), [08-volume-arrow.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/08-volume-arrow.png)

### F-02 — P2: Files fields have inconsistent, undersized geometry

**Reproduce:** Open Files; compare path/search fields and New folder/Rename name fields with nearby buttons.
**Observed:** Fields are visibly about 18–22px tall while buttons are about 30px.
**Expected / next check:** Use comfortable shared control geometry; text-area exceptions should be intentional.
Evidence: [15-files-test-folder.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/15-files-test-folder.png), [16-new-folder-dialog.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/16-new-folder-dialog.png), [25-files-rename-dialog.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/25-files-rename-dialog.png)

### F-03 — P2: Files Ctrl+V fails while Paste button succeeds

**Reproduce:** Select disposable alpha.txt → Ctrl+C → navigate to another disposable directory → Ctrl+V. Retry Ctrl+V, then click Paste.
**Observed:** Internal clipboard status and Paste button become available; Ctrl+V leaves destination empty. Paste button creates an exact byte-for-byte copy.
**Expected / next check:** Keyboard Paste should invoke the same operation as the button.
Evidence: [21-files-keyboard-copy-paste.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/21-files-keyboard-copy-paste.png), [22-files-paste-shortcut-retry.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/22-files-paste-shortcut-retry.png), [23-files-paste-button.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/23-files-paste-button.png)

### F-04 — P3: Keyboard-owner status is misleading

**Reproduce:** Leave Files open, click Apps search, type brave.
**Observed:** App list filters and the Files surface dims, but footer still says FILES HAS THE KEYBOARD.
**Expected / next check:** Footer should identify the actual input owner or omit a misleading claim.
Evidence: [27-apps-search.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/27-apps-search.png)

### F-05 — P2: Installed Music volume popup spills below workspace

**Reproduce:** Open Music → More → click the volume slider near the bottom edge.
**Observed:** Slider appears below the main rounded workspace and clicking it did not change the service volume (100%).
**Expected / next check:** Keep popup controls inside an interactive area. The new visible volume component exists in source but is not installed: retest after activation before another fix.
Evidence: [31-music-more-volume.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/31-music-more-volume.png), [32-music-volume-adjust.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/32-music-volume-adjust.png)

### F-06 — P2: Escape closes Music instead of only its popup

**Reproduce:** Open Music → More → press Escape once.
**Observed:** Entire player closes and the terminal is revealed. No subsequent keys were sent to that terminal.
**Expected / next check:** Dismiss the active popup first; another Escape may close the player.
Evidence: [31-music-more-volume.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/31-music-more-volume.png), [33-music-popup-escape.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/33-music-popup-escape.png)

### F-07 — P3: Music Clear button wraps its label

**Reproduce:** Type a non-empty search in the Music rail.
**Observed:** Clear displays as Cle / ar on two lines and increases the search row height.
**Expected / next check:** Provide a usable consistent clear target without label wrapping.
Evidence: [34-music-search-no-results.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/34-music-search-no-results.png), [35-music-search-settled.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/35-music-search-settled.png)

### F-08 — P3: Health logs expose raw ANSI formatting

**Reproduce:** Settings → System health → Recent logs.
**Observed:** ANSI escape fragments appear as visible text in log lines.
**Expected / next check:** Strip or render terminal formatting in the diagnostic display.
Evidence: [01-settings-health.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/01-settings-health.png)

## Scenario log

| ID | Result | Action / observation | Evidence |
|---|---|---|---|
| SET-01 | pass | Health page opens and reports compositor/music connected | [01-settings-health.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/01-settings-health.png) |
| SET-02 | pass | Settings search volume opens Sound & screen | [02-settings-search-volume.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/02-settings-search-volume.png) |
| SET-03 | pass | Unmatched settings query gives recovery guidance | [10-settings-no-results-focused.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/10-settings-no-results-focused.png) |
| SET-04 | pass | Mouse output volume changed to 71%, verified with wpctl, then restored to 100% | [04-volume-click.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/04-volume-click.png) |
| SET-05 | finding | Clicked volume slider does not respond to End/Right | [07-volume-keyboard-immediate.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/07-volume-keyboard-immediate.png) |
| NAV-01 | pass | Super+Space opens application launcher | [11-launcher-open.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/11-launcher-open.png) |
| NAV-02 | pass | Unmatched launcher query gives empty state | [12-launcher-no-results.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/12-launcher-no-results.png) |
| NAV-03 | pass | Files search result opens with Enter | [14-files-open.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/14-files-open.png) |
| FILES-01 | pass | Typed absolute directory path navigates on Enter | [15-files-test-folder.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/15-files-test-folder.png) |
| FILES-02 | pass | Hidden files toggle exposes synthetic hidden file | [20-files-hidden-toggle.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/20-files-hidden-toggle.png) |
| FILES-03 | pass | New folder created through dialog | [18-new-folder-created.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/18-new-folder-created.png) |
| FILES-04 | pass | Invalid path-like name rejected; draft/error retained and outside path absent | [19-new-folder-invalid-submit.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/19-new-folder-invalid-submit.png) |
| FILES-05 | pass | Ctrl+C sets Files internal clipboard | [21-files-keyboard-copy-paste.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/21-files-keyboard-copy-paste.png) |
| FILES-06 | finding | Ctrl+V does not paste despite clipboard and enabled Paste button | [22-files-paste-shortcut-retry.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/22-files-paste-shortcut-retry.png) |
| FILES-07 | pass | Paste button copies exact fixture bytes | [23-files-paste-button.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/23-files-paste-button.png) |
| FILES-08 | pass | Existing destination refused without overwrite | [24-files-copy-conflict.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/24-files-copy-conflict.png) |
| FILES-09 | pass | F2 rename opens and Enter commits filename | [26-files-rename-enter.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/26-files-rename-enter.png) |
| FILES-10 | finding | Path/search/name fields undersized relative to buttons | [16-new-folder-dialog.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/16-new-folder-dialog.png) |
| APPS-01 | pass | App search filters group correctly | [27-apps-search.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/27-apps-search.png) |
| APPS-02 | pass | Group expands into readable left-aligned window rows | [28-apps-expanded.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/28-apps-expanded.png) |
| APPS-03 | pass | Search clear restores app groups | [29-apps-clear.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/29-apps-clear.png) |
| APPS-04 | finding | Status claims Files has keyboard while Apps search owns input | [27-apps-search.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/27-apps-search.png) |
| MUSIC-01 | pass | Player opens on existing paused track and lyrics | [30-music-open.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/30-music-open.png) |
| MUSIC-02 | finding | More-menu volume extends below work area and click does not change level | [32-music-volume-adjust.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/32-music-volume-adjust.png) |
| MUSIC-03 | finding | Escape with More open closes whole player | [33-music-popup-escape.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/33-music-popup-escape.png) |
| MUSIC-04 | finding | Search Clear label wraps across two lines | [34-music-search-no-results.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/34-music-search-no-results.png) |
| MUSIC-05 | inconclusive | Gibberish query returned unrelated results; needs upstream/cache comparison | [35-music-search-settled.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/35-music-search-settled.png) |
| HEALTH-01 | finding | Recent logs display raw ANSI sequences | [01-settings-health.png](/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat/01-settings-health.png) |

## Qualifications and follow-up

- The first raw SET-02 entry prematurely assumed End restored volume. Later verification contradicted it; SET-02-correction and F-01 supersede that assumption. Final PipeWire output is confirmed at 100%.
- During Files copying, the clipboard-status row moved the directory rows. The destination entered was created-by-ui, not nested. Both belong to the disposable fixture; screenshots and the final byte comparison establish the actual outcome.
- Music search for a gibberish term eventually returned unrelated songs/artists. This needs a controlled repeat and comparison with Spotify before assigning an application bug.
- Closing Music revealed the terminal rather than the originally focused native application. Record this as a focus-restoration observation requiring a dedicated reproduction; no terminal commands or text were sent.
- Two identically labelled Brave entries were visible in the launcher grid. They may be distinct desktop entries; this is an unclassified discoverability observation.

## End state and retained fixtures

- Restored workspace 0, original panel visibility, ChatGPT as the focused native app, and collapsed the expanded Brave group.
- Protected tmux panes retained the same IDs, PIDs and commands: `%1 / 149409 / zsh`, `%0 / 1973 / nano`. No shell/compositor/music restart, logout, lock, power action or terminal cleanup was performed. Process identity checks are in final-state.json.
- System output volume is 100%; des-music remains paused at volume 100 and position 192323 ms. Playback/likes/queue were not changed.
- The closed Files browser retains the disposable directory and Hidden files enabled. Its internal clipboard still references the synthetic alpha.txt. Screenshots also replaced the OS clipboard; the original clipboard was not preserved.
- Retained only synthetic fixtures under `/tmp/desui-user-test-20261027` for reproduction. No outside escape-probe directory was created.
- Input devices and the automation socket were closed. Generated screenshot/device notifications remain in history; no global clear was used because that would remove the user’s existing history.

## Not tested in the first pass (see continuation below)

Quick Settings interactions, notification-center management, app launch/close using owned fixture windows, window layouts/moving between workspaces, full keyboard Tab navigation, multi-monitor behavior (only one output connected), gestures (previously deferred), lock/unlock and power/session recovery. These remain pending; battery and the protected terminal constraint limited the pass.

## Raw evidence

- `interaction-log.jsonl`: timestamped input actions and screenshot requests.
- `actions.jsonl`: native CLI commands/responses.
- `results.jsonl`: incremental notes, including explicit corrections.
- `cases.json`: final scenario classifications.
- `baseline.json`, `final-state.json`: binary hashes, state and protected process checks.

## Continuation and confirmed overlay finding

F-09 (P2): Quick Settings appears behind native client windows. Screenshots 50–51 reproduce this with a disposable X11 fixture. The user independently confirmed that the popup was partly hidden on the actual desktop, ruling out a screenshot-only artifact. This should be prioritized before further Quick Settings interaction tests.

Quick Settings mouse volume changed the system level to 50%; Sound & screen restored it to 100%, verified through PipeWire. DND state and suppression were not established, so the notification attempts are inconclusive. No notification history was globally cleared. The four earlier synthetic notification IDs belong to the previous login and must not be reused for cleanup in the new login.

X11 text input and local copy/paste passed (54). Copying synthetic text from X11 to a Wayland entry also passed (55); the saved screenshot establishes the result even though the old temporary process logs disappeared.

The machine restarted between the earlier continuation and this resumed turn, as shown by uptime, missing test processes, and changed terminal socket. The cause was not established. Earlier protected-process continuity claims apply only to their recorded test interval. This resumed turn did not restart the desktop or any user application.

Fresh owned Wayland windows were launched on workspace 3 using the CLI for setup. Layout buttons were then exercised with actual pointer input: single (56), split (57), columns (58), master (59). Fullscreen removed the panels (60), and Super+F restored them (61). The grid button opened Applications (62); this was not a successful workspace-overview test and is not counted as one.

| ID | Result | Observation |
|---|---|---|
| QUICK-01 | finding | Quick Settings is partly hidden behind native windows; user independently confirmed live overlap. |
| QUICK-02 | inconclusive | DND toggle and notification suppression could not be verified reliably because of popup overlap. |
| NATIVE-01 | pass | X11 entry typing and in-window copy/paste of synthetic text succeeded. |
| NATIVE-02 | pass | Synthetic text copied from X11 entry pasted into Wayland entry. |
| LAYOUT-01 | pass | Single mode shows focused owned window; second window hidden. |
| LAYOUT-02 | pass | Split button displays both owned windows at equal widths. |
| LAYOUT-03 | pass | Columns button displays two owned windows in columns. |
| LAYOUT-04 | pass | Master button gives focused window larger share of workspace. |
| LAYOUT-05 | pass | Fullscreen removes shell panels and Super+F restores them while keeping both windows. |

A possible focus/workspace jump during earlier Quick Settings attempts remains unconfirmed. CLI acknowledgements can precede settled compositor state; those observations are not counted as another bug.

Final cleanup: both fresh disposable clients were stopped through their own execution sessions; the automation server closed its input devices and socket. Workspace 1 and ChatGPT focus were restored, preserving this login’s left/right panel visibility. Current terminal pane identity was unchanged from restart-baseline.json; earlier pre-restart pane PIDs cannot be compared across boots. Volume remains 100%. Screenshots changed the OS clipboard and generated notification history. Source implementation files were not edited during this UAT phase.

Remaining coverage: notification-center management, verified DND suppression, moving windows across workspaces, actual workspace overview, full Tab navigation, multi-monitor recovery, and previously deferred gestures. No power/session action was attempted. The fresh pass was bounded because the battery was again at 10%.

## Fix work after testing

Source changes address the shared slider focus, Files control height and paste handling with non-text clipboards, Apps keyboard-owner label, Music popup Escape and search Clear label, and ANSI log formatting. The Music volume control was already moved out of its popup in the current source before this UAT; the installed player tested above did not include that change.

Quick Settings now takes overlay priority over an open companion panel, including disabling companion-specific outside-click handling while Quick Settings is active. This removes one conflicting overlay state; the original reproduction had the bottom panel closed, so the overlap is not yet established as fixed. A live retest with the newly built shell is required, and further compositor investigation may be necessary.

Shell tests: 514 passed and 1 ignored; the subsequent focused click-to-keyboard slider regression test passed. These are source-level tests. The running shell was not replaced or restarted during fix work, so the deployed desktop has not yet been retested with these changes.
