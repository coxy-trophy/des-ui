# Performance investigation — 29 October 2026

## Active-session sample

This is a read-only sample of the existing installed session, not a fresh boot or controlled idle benchmark. The user had applications and terminal sessions open; background activity and displays were not held constant. A 10.01-second interval on kernel `7.1.5+kali-amd64` sampled process CPU ticks and `/proc/<pid>/smaps_rollup`. CPU is percentage of one logical core, not percentage of the whole machine. Memory is MiB (KiB divided by 1,024). AC was connected and the battery was charging; CPU package temperature was approximately 84°C before sampling. Process memory does not represent all GPU or kernel allocations.

| Installed component | PSS MiB | RSS MiB | Private dirty MiB | CPU % of one core | Threads |
| --- | ---: | ---: | ---: | ---: | ---: |
| Shell | 105.3 | 162.2 | 75.2 | 43.17 | 53 |
| Compositor | 55.9 | 114.8 | 32.7 | 9.69 | 8 |
| Companion | 29.1 | 49.2 | 24.3 | 0.10 | 6 |
| Music | 18.1 | 25.3 | 10.5 | 0.00 | 10 |
| Portal | 2.6 | 4.3 | 0.4 | 0.00 | 4 |

The five PSS values total approximately 211 MiB. Startup wrapper, Xwayland, tmux, terminals and applications are excluded. Values are observations, not a clean-start cost, desktop-wide total, leak finding, or GNOME/KDE comparison. Ten seconds is too short to establish idle behavior.

Installed identities (SHA-256): shell `5767d765dee2fc76cb8717335aa66590a1cc9cfb124848cc678d65dcfadbc861`; compositor `f9f133e942a675553768a0ee9f1a31cac1d4a0d57c24f3390df99585c0f71506`; music `88c4c6ab3c5c1f9e4893f9d7fae9e88cf15a5aff5161a308b832f02d12925cb2`; companion script `eb311d192c7344516add802c0f519f87fa9a9f1d1245605df70dcf285d5c6b69`. Workspace HEAD was `dc42aa1`, with substantial uncommitted changes; it is not assumed to match the installed binaries.

## Source inventory

- The shell constructs `Music` at startup. That starts a connection worker and three artwork workers; the connection worker may launch the music service. Changing this needs to preserve playback controls, external playback status, and restart/recovery behavior.
- The companion's `social_tick` connects its Python worker when the bridge is absent, even before checking whether social suggestions are enabled. On-demand startup needs an independent way to honor an explicitly enabled social setting and should preserve setup and voice shortcuts.
- The shell schedules a repaint at 66 ms while the globe is rotating and 500 ms otherwise, in addition to event-driven and component-specific redraws. This establishes a source of periodic work, not that it explains the measured CPU. Profile the installed build with the same scene and controlled display activity before changing pacing.
- The core desktop starts other services and threads for power, clipboard, widgets, tray, authorization, calendar and configured autostart applications. They cannot be treated as optional without testing their associated desktop functions.

## Next checks

1. Run a clean-session comparison, with matching binaries, display setup and workload; record startup timing, idle and active samples, power draw and GPU data if available. Do not log out the user's live sessions to do so.
2. Profile shell CPU and redraws with globe visible/hidden and animation enabled/disabled. Compare native GPU operation rather than software-rendering previews, which have different cost.
3. Design and test on-demand startup for music and companion, preserving existing features and user settings; measure before and after.
4. Repeat sustained music browsing, terminal scrollback/search, voice and image attachment workloads, followed by idle measurements. See the earlier [memory review](2026-10-28-memory-review.md).
5. Compare other desktops only under equivalent hardware and configuration. Avoid claiming a lightweight target or specific savings until measured.

## Follow-up — 30 October

The working tree now defers the three music artwork workers until the first actual cover request. Constructing the music UI or requesting no cover starts none; music connection and playback behavior are unchanged. The regression test for lazy startup and the focused cover tests passed (4 tests). This source change has not been installed, and the RAM/CPU saving has not been measured in a comparable session.

A separate five-second active-session per-thread sample showed two unnamed shell threads at approximately 13.8% and 7.6% of one core; the device inventory and clipboard threads used approximately 1.0% and 0.8%. The rest of the visible sample was lower. Thread names alone do not identify the work in the unnamed threads, and this sample cannot establish idle CPU use or the effect of the globe. System profiling support was not available through `perf` in this environment. A controlled comparison remains necessary.

## Installed-session follow-up

After the user installed the update, `/proc/<pid>/exe` hashes matched the new installed shell and compositor. In three consecutive 10-second active-session samples, the shell main thread used 29.1%, 31.3%, and 29.8% of one core; a second unnamed thread used 5.3%, 4.6%, and 4.6%. These are not a clean idle workload.

A separate 12-second process-CPU sample before, during, and after toggling the right panel showed 37.8%, 40.6%, and 41.1%, respectively. The toggle was restored. The test did not independently establish the initial panel/globe visibility or control concurrent work, so it does not establish the effect of globe rendering; it provides no evidence that toggling the panel alone materially reduced CPU. `perf`, `gdb`, and `strace` were not available, and kernel perf access was restricted. Capture a controlled profile with appropriate tools and known visual state before assigning root cause.

## Known panel-state follow-up

The control service reported the right panel open before testing. Three 15-second shell process samples were taken: panel open 43.2%, panel closed 37.5%, and panel restored open 41.9%, each as a percentage of one CPU core. The panel state was queried before and during the test and restored afterwards. The approximately 5 percentage-point difference is an uncontrolled active-session observation, not a measurement of the globe in isolation: other desktop and application work continued. Most of the observed shell CPU remained with the right panel closed. Do not reduce globe functionality on this evidence alone.
