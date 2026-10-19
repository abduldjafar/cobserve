# 01 · The screen

Target 120×36, minimum 80×24, richer at 160×48. One screen: a top bar, a grid of framed panels,
a status bar. No view tabs; a panel opens full-screen with `⏎` and `esc` brings the grid back.
Acceptance: `screens/120x36-dash.txt` (below), `screens/80x24-dash.txt`, `screens/160x48-dash.txt`,
and the preset, density and replay variants listed at the end.

## 120×36, preset *all*

```
 ☗ cobserve · prod    04:21 ━━ 05:33 ━━ 11:45 ━━ 14:48 ━●┄ 17:50 in 1h57m ┄┄ 18:59               ● live   15:52:07 WIB
╭ 1 FLEET ───────────────────────── 8 runaway · 1 hot · 9 nodes ─╮╭ 2 REDASH QUEUE ─────────────────── oldest 3m43s ✖ ─╮
│ █ █ ▀▀█ ▀ █  cpu                                               ││ ▄█  █▀▀  waiting                            ▁▂▃▄▅  │
│ ▀▀█ █▀▀  ▄▀  32.8%                                             ││  █  █▀█  6 running                     ▄▆▇▆▇█████  │
│   █ █▄▄ █ ▄  52/160 •••••••••••••••••••••••▃•••••••••••••••••  ││ ▄█▄ █▄█  3 stale                 ▁▁▃▅▆███████████  │
│ 268/640 GiB memory  ███████████████████████•█████████████████  ││ ●●●●●●○ 6/7 worker… ▃▃▄▃▃▄▃▄▄▄▃▄▇████████████████  │
│ 13 queries · 8 ! ·… -60m                    █ mem  • cpu  now  ││ 4 of 4 hold a runa… -60m        waiting jobs  now  │
╰────────────────────────────────────────────────────────────────╯╰────────────────────────────────────────────────────╯
╭ 3 NODES ──────────────────────────────────────────────────────────────────────────────────── sorted by pressure · s ─╮
│ ╭ clickhouse3 ───╮ ╭ clickhouse-bi ─╮ ╭ clickhouse7 ───╮ ╭ clickhouse2 ───╮ ╭ clickhouse5 NEW  ╭ ch4 ───────────╮    │
│ │ ████████░  93% │ │ ██████░░░  67% │ │ █████░░░░  60% │ │ ████░░░░░  42% │ │ ███░░░░░░  38% │ │ ██░░░░░░░  21% │    │
│ │ ▇▇▇▇██████████ │ │ ▆▆▆▆▆▆▆▆▆▆▆▆▆▆ │ │ ▅▅▅▅▅▅▅▅▅▅▅▅▅▅ │ │ ▄▄▄▄▄▄▄▄▄▄▄▄▄▄ │ │ ▃▃▃▃▃▄▃▃▃▃▃▃▃▃ │ │ ▂▂▂▂▂▃▂▂▂▂▂▂▂▂ │    │
│ │ 4q !2    8m31s │ │ 3q !2    6m40s │ │ 2q !2    8m56s │ │ 2q !1    4m38s │ │ 0q           — │ │ 1q !1      51s │    │
│ ╰────────────────╯ ╰────────────────╯ ╰────────────────╯ ╰────────────────╯ ╰────────────────╯ ╰────────────────╯    │
│ ╭ ch6 ───────────╮ ╭ ch8 ───────────╮ ╭ ch9 ───────────╮                                                             │
│ │ ██░░░░░░░  19% │ │ █░░░░░░░░  11% │ │ █░░░░░░░░   8% │                                                             │
│ │ ▂▂▂▂▂▂▂▂▂▂▂▂▂▂ │ │ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │ │ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │                                                             │
│ │ 1q          9s │ │ 0q           — │ │ 0q           — │                                                             │
│ ╰────────────────╯ ╰────────────────╯ ╰────────────────╯                                                             │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ 4 clickhouse3 · 93.1% of 64 GiB · cpu 93.1% ─╮╭ 5 AIRFLOW ───────────── last 24 h ─╮╭ 6 JIRA ───────────────── mine ─╮
│ grigol.ga… ████░░░░░░░░░░  25.5%     8m31s ! ││ 217 runs  ✔ 211  ✖ 3  ▸ 2  ◌ 1  ↻ 1││ 4 › 4 › 1 › 3 done ✖ 1 overdue │
│ j.petrova  ██░░░░░░░░░░░░  12.9%     6m40s ! ││ ▪▪▪▪▪▪▪▪▪✖▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪✖▪✖ ▸━    ││ today 5h10 of 7h30             │
│ airflow    █░░░░░░░░░░░░░   5.5%         18s ││ 18    00    06    12   now         ││ DATA-2207 Product…         11d │
│ haris      ░░░░░░░░░░░░░░   1.9%         24s ││ ✖ test_clickhouse_con…     262d04h ││ DATA-2804 Reload …          8h │
│ server · … ███████░░░░░░░  47.4%             ││ ▸ statement_daily_agg…       2h12m ││ DATA-2850 STATEME…due tomorrow │
│                                              ││ ◌ gateway_transfers_b…       4m12s ││ DATA-2849 airflow…         15h │
│                                              ││ ✖ replication_app_con…       8m06s ││ DATA-2647 Client …  overdue 2d │
╰──────────────────────────────────────────────╯╰────────────────────────────────────╯╰────────────────────────────────╯
╭ 7 TAPE ─────────────────────────── newest first · n note · y export ─╮╭ 8 SESSIONS ─────────────── 4 · \ floats one ─╮
│ ✖ ▁▁▁▁▁▁▁▁▁▁▁▃▁▁▁▁▁▃▃▃  ▲ 15:51 ended m.kairys on clickhouse…        ││ ✻ 7  billing export                ● working │
│ ▲ ▁▁▁▁▁▁▁▁▁▁▁▁▁▃▁▁▃▁▃▃  ✖ 15:51 m.kairys's query on clickhou…        ││ ▣ 8  late partitio…               ! approval │
│                         ▲ 15:50 runaway m.kairys on clickhou…        ││ ▦ 9  sql · clickho…                   ○ idle │
│                         ▲ 15:50 ended m.kairys on clickhouse…        ││ ❯ 10 make totals                      ○ idle │
╰──────────────────────────────────────────────────────────────────────╯╰──────────────────────────────────────────────╯
 1-8 panel   ⏎ open nodes   ←→↑↓ choose   e explain   p layout   : command          all · 9/9 nodes · 0.8 s · read-only
```

## Chrome

**Top bar** (row 0; absent in *focus* density): `☗ cobserve · <profile>` at the left, the clock at
the right (`● live   15:52:07 WIB`; `○ paused …`; `◷ 15:40 · replay −12m` in amber while
replaying; a click toggles local ↔ UTC). Between them the day line when at least 60 columns are
free (`04:21 ━━ 05:33 ━━ 11:45 ━━ 14:48 ━●┄ 17:50 in 1h57m ┄┄ 18:59`, prayer names added from 80
free columns), else the chip `☾ Maghrib in 1h57m`. While zoomed the middle shows the path
`dashboard › node › clickhouse3` and `esc back`. The prayer reminder and the notifications are
toasts.

**Status bar** (last row): at most five hints for the focused panel or the zoom, rendered from
the `Action` table (`04-architecture.md`), then at the right `<preset> · 9/9 nodes · 0.8 s ·
read-only` (the poll line of today's footer). A filter in progress replaces the right part with
`/ query · esc clears`. The LOCAL line (`this Mac · cpu 31% · mem 12/32 GiB`) joins the right part
from 140 columns.

## The grid (`layout()` in the simulation — port it as is)

Inputs: `W`, `H`, the preset (`all`, `fleet`, `ops`), the density, the node count `n`.

```
top     = density == focus ? 0 : 1
r1      = H ≥ 40 ? 9 : H ≥ 30 ? 7 : 5                      FLEET | QUEUE row
tileW   = W ≥ 150 ? 20 : W ≥ 100 ? 19 : 16
tileH   = H ≥ 30 ? 5 : 4
perRow  = floor((W − 4) / tileW)
tiles   = preset == ops ? 0 : min(2, ceil(n / perRow)) × tileH + 2
bottom  = H ≥ 40 ? 7 : H ≥ 30 ? 6 : 3;  preset == fleet → 3
mid     = H − 1 − top − r1 − tiles − bottom
if mid < 6 and the tiles take two rows → one row of tiles, recompute mid
if mid < 6 → bottom = 3, recompute mid
```

Rows, top to bottom: FLEET (55 % of W) | QUEUE (the rest); NODES (full width); the middle row —
*all*: NODE 40 % | AIRFLOW 32 % | JIRA 28 %; *fleet*: NODE full width; *ops*: AIRFLOW 50 % |
JIRA 50 % — then the bottom row: TAPE 60 % | SESSIONS 40 % when `W ≥ 100` and `bottom ≥ 5`,
else SESSIONS alone. Widths are rounded so the panels tile the row exactly; a frame is part of its
panel (two columns, two rows of every box are border).

Worked sizes: 120×36 → 1 + 7 + 12 + 9 + 6 + 1. 80×24 → 1 + 5 + 6 (one tile row) + 8 + 3 + 1.
160×48 → 1 + 9 + 12 (two rows of 7 tiles) + 18 + 7 + 1 — as `screens/160x48-dash.txt` draws it
(an earlier draft of this line said one row; the screen and `layout()` agree on two).

Rules that follow: a panel whose inner height is under 3 rows is not drawn (its key still zooms
it); the NODE panel's graph needs 8 inner rows; the FLEET and QUEUE big numbers need 4 inner rows
and 60 / 40 inner columns; a tile's fourth line needs `tileH = 5`.

## Presets (`p`)

`all` (above) → `fleet` (NODES and NODE take the middle; AIRFLOW and JIRA leave the grid, their
keys still zoom them; the bottom row shrinks to 3) → `ops` (no tiles; AIRFLOW and JIRA split the
middle; the NODE panel leaves) → `all`. Kept in the config as `layout = "all"`. Screens:
`*-dash-preset-fleet.txt`, `*-dash-preset-ops.txt`.

## Density (`+`)

`comfortable` (the day line) → `compact` (the chip instead; nothing else changes) → `focus` (no
top bar at all; the status bar stays as it is, as the screen draws it — the clock is gone with the
bar). Screens:
`120x36-dash-density-compact.txt`, `120x36-dash-density-focus.txt`.

## Focus and zoom

Exactly one panel has the focus (chip title); it starts on NODES. `1`–`8` and `tab` move it, a
click on a panel takes it. `⏎` zooms the focused panel: the top bar shows the path, the panel's
zoomed form takes rows 1..H−2 inside one frame, the status bar shows the zoom's actions. While
zoomed: `1`–`8` and `tab` switch the zoom directly; `esc` (or `⌫`, or `f`) returns to the grid
with the focus on that panel; the grid's state (selection, scroll) is kept.

Inside a panel or a zoom the arrows move its own cursor: the tile grid in NODES (`←→` within a
row, `↑↓` between rows), rows elsewhere, lanes and tickets in the Jira board. A choice in NODES
or the zoomed FLEET list sets *the chosen node*, which the NODE panel and the explain popover
follow.

## Replay (`[` `]` `t`)

Each `[` steps one minute back (up to 59), `]` forward, `t` back to live. While replaying: the
clock turns `◷ HH:MM · replay −Nm` in amber, every number comes from the stored minute (the
simulation scales the live snapshot by the node's history; the real thing reads feature A1's
store), graphs show the minute as a highlighted column, polling continues underneath so `t` is
instant. Screen: `120x36-dash-replay.txt`.

## Resizing

Every size from 80×24 up must render without panicking (the resize fuzz test of today's shell
is extended to the new one: random sizes 20..250 × 8..80, every panel, every zoom, every overlay).
Below 80×24 the grid degrades to the FLEET and QUEUE plain lines plus the NODES tiles and the
status bar; below 40×12 a single line `cobserve · too small (40×12 at least)`.
