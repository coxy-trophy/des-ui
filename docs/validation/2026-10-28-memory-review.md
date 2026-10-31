# DEs-UI memory review — 2026-10-28

Read-only snapshot of the installed desktop on this machine, with its existing apps and terminals left running. Values are approximate MiB from `/proc/<pid>/smaps_rollup`; PSS apportions shared pages between processes, while RSS counts shared pages in each process. This is a snapshot, not a leak or workload benchmark. The installed binaries may differ from the current working tree.

| Component | RSS | PSS | Private dirty |
| --- | ---: | ---: | ---: |
| Shell | 147 | 94 | 61 |
| Compositor | 106 | 50 | 24 |
| AI companion | 49 | 29 | 24 |
| Music service | 24 | 17 | 8 |
| Portal | 4 | 3 | <1 |
| Session startup and wrapper | 17 | 9 | 7 |

These core processes total approximately 202 MiB PSS. Xwayland, terminal sessions and other applications are separate; graphics allocations and kernel memory may not be fully represented. The host had approximately 8.2 GiB available and no swap in use at the time of the snapshot. The Codex application itself had several larger processes, but those are application memory, not shell allocations.

## Findings

- The music artwork cache in the shell held every texture requested for the life of the shell. An RGBA cover at its maximum 320 × 320 resolution is 400 KiB before renderer overhead; 1,000 such covers can therefore represent roughly 391 MiB of pixel data. This is a calculated risk, not an observed measurement of the current cache. The working tree now limits it to 128 recently used entries, including pending and failed requests; evicted covers can be loaded again from the existing disk cache. Renderer copies and transient in-flight images are additional.
- Wallpaper maintains original and blurred textures, with bounded input and output dimensions. Terminal scrollback defaults to 5,000 lines and is configurable, so large histories or many tabs can increase shell and tmux memory. Neither has been established as a leak by this snapshot; reducing either without profiling could degrade the experience.
- Music list data also grows as distinct sources are visited: `Shared.lists`, associated metadata and the list of requested keys persist until invalidation, and additional pages extend lists. The active list and playlists are cloned for rendering. This is a separate unbounded growth and allocation risk; safe eviction needs to coordinate with the request bookkeeping and pagination so an evicted list can be fetched again. Its current footprint has not been measured.
- Terminal search clones the terminal screen and its history while an incremental scan is in progress. The copy is released after the scan; large configured histories can cause temporary spikes. Persistent terminal sessions also maintain history in tmux, separately from the shell's VT history. Investigate with a disposable terminal workload before changing search or retention behavior.
- The companion and music service each run separately, which has a baseline cost. Their process memory is modest in this snapshot. Measure idle versus active voice, image attachment, long music browsing, and many terminal tabs before deciding whether architectural changes are justified.

## Follow-up measurements

Repeat PSS and private-dirty readings after a fresh session, after browsing many unique covers, and after returning to idle. Include process lifetime, screen size and workload, and compare the same installed build. Profile heap and GPU allocations separately if memory continues to grow. Avoid terminating existing user sessions to establish a baseline.
