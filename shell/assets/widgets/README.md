# Widgets

A widget is a file in `~/.config/des-ui/widgets/` ending in `.widget`, with
`key = value` lines:

| Key | Meaning |
|---|---|
| `name` | The label above the tile |
| `exec` | The program and its arguments, split like a desktop entry's `Exec`; never a shell |
| `every` | How often it runs: `5s`, `1m`, `2h`, or seconds; at least `1s`, `10s` by default |
| `panel` | `left` or `right` (the default) |
| `kind` | `text` (the default), `meter`, `series` or `list` |

The program's last line of standard output is the reading. Plain text is shown as it
is. A JSON object may give any of:

```json
{"text": "87%", "value": 0.87, "label": "root", "series": [1, 2, 3],
 "state": "ok", "lines": ["one", "two"], "tooltip": "what this is"}
```

`value` is a fraction from 0 to 1 for a meter; `series` draws a sparkline, or a
`series` widget keeps a history of single `value`s; `state` is `ok`, `caution`
(drawn in the caution colour) or `off`; `lines` fill a list. Text is cut at 120
characters, lines at 16 of 80, series at 120 points.

Programs run one at a time, for at most three seconds, with 64 KiB of output read.
One that fails or runs out of time shows "no answer" until it next succeeds.

The three files here are examples that work on any Linux: copy them into the
widgets directory to try them.
