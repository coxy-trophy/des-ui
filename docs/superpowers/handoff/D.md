# Handoff D: instruments and alerts

Branch: `worktree-agent-a55745660549219fe`. Files: `shell/src/sensors.rs` (new),
`shell/src/sysmon.rs`, `shell/src/panels.rs`, additions at the end of `shell/src/ui.rs`
(`bar_grid_layout`, `bar_grid`, `waterfall`), and `mod sensors;` in main.rs.

## What the parent wires

One call per frame in `runtime.rs`, right after `self.mon.tick()`:

```rust
for alert in self.mon.take_alerts() {
    self.notices.post(&alert.title, &alert.body, alert.urgent);
}
```

`notify::Center::post(title, body, urgent)` already exists and is what the dock uses for
"Application unavailable". Until this is wired, `take_alerts` is reported unused.

## What the alerts are

`sysmon::Alarms` evaluates on every applied sample, with hysteresis so nothing nags:

| Alert | Raised at | Cleared below | Urgent |
|---|---|---|---|
| Processor running hot | 90 °C | 80 °C | yes |
| Memory nearly full | 95 % | 85 % | yes |
| Disk nearly full (per mount) | 95 % | 90 % | no |
| A service failed (per unit) | newly listed by `systemctl --failed` | no longer listed | no |

At most 16 alerts wait between frames; older ones are dropped.

## Readings now on `SysMon`

`core_hist`, `mem_hist`, `load`, `temps` (hottest first, labelled CPU/GPU/NVME/BOARD/WIFI…),
`fans`, `gpu` + `gpu_hist`, `disk_read_hist`/`disk_write_hist` and totals, `filesystems`
(fullest first), `battery_health`, `failed_units`, `cpu_temperature()`. All sampled on
the existing worker; `systemctl` runs at most every 30 s and `nvidia-smi` every 5 s, each
under a timeout and killed if it overruns.

## Panels

Left column: load average in the PROCESSOR heading, one bar per core (any count, hover
reads it), a GPU meter when there is one, memory history, then THERMAL and STORAGE
sections that appear only when the column has room for them and the process list and
companion (`sections_that_fit`). Failed units show in the SYSTEM heading in caution.
Right column: the two traffic tiles keep their rates and totals; one waterfall below
them shows download rising and upload hanging from a shared axis. Battery hover text
adds wear.

## Not done here

Settings has no thresholds; the lines are constants on `Alarms`. The secondary-display
backdrop is unchanged. Hardware cases to add to UNTESTED.md: real hwmon labels on AMD
and Intel, an NVIDIA machine, a machine with no sensors at all.
