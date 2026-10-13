# 02 · The panels

Eight panels, numbered as the keys that focus them. Each has a **dashboard** form (its box in the
grid) and a **zoomed** form (the whole screen, `⏎`). Every number is listed with the formula it
comes from: `§5.x` means `DESIGN.md`, `LOCAL` means the arithmetic of `DESIGN.md` §13 *LOCAL*.
Acceptance screens: `screens/<W>x<H>-dash.txt` and `screens/<W>x<H>-zoom-<panel>.txt`.

Common rules:

- The frame's colour is the panel's **severity**: red when its worst finding is critical, amber
  when warning, dim otherwise (§7 thresholds). The title is `N NAME`; it is drawn as a chip
  (accent background) when the panel has the **focus**. The right end of the top border carries
  the panel's one-line summary (`8 runaway · 1 hot · 9 nodes`, `oldest 3m43s ✖`).
- The focused panel takes `←→↑↓`, `⏎`, `e`, and its own letters. `tab` moves the focus to the
  next panel. Digits `1`–`8` focus a panel; while zoomed they switch the zoom.
- A list inside a panel shows its first rows and scrolls with the cursor; a zoomed panel shows
  everything with a cursor, `▌` in the gutter and the selection background.
- Text never crosses a frame: every string is clipped to the inner width. A column that does not
  fit is dropped whole, never cut to a meaningless stub (the 80×24 screens show which).

## 1 · FLEET

Dashboard (`r1` rows high, 55 % wide):

| Element | Value | From |
|---|---|---|
| big number | fleet memory share, `Σ mem_used ÷ Σ mem_total` | §5.1 denominators, summed over reachable nodes |
| beside it | `cpu 32.8%` = `Σ cpu_busy_cores ÷ Σ cores`, `52/160 c` | §5.2 |
| under it | `268/640 GiB memory`, `13 queries · 8 ! · lag ≤ 12 s` | counts of `system.processes` rows, runaways §5.4, `max(lag_s)` |
| graph | last 60 min of fleet memory share, one column a slice, filled area; `•` dots are the CPU share on the same scale | `history.rs` (4 min today; 60 min needs feature A1 of `07-features.md`, until then the last 4 min stretched with the window printed) |
| graph labels | `-60m` left, `█ mem  • cpu  now` right | window printed, never implied |
| frame | red if any node is red, amber if any amber | §7 |

Under 60 inner columns or 4 inner rows the big number gives way to three plain lines
(`mem 41.8% 268/640 GiB`, `cpu …`, `13 q · 8 !`), see `80x24-dash.txt`.

Zoomed: two full-width graphs, memory then CPU, each with `100% · 50% · 0%` ticks, then one row
per node (`name  bar  pct  used/total GiB  cpu pct`) with the cursor; `⏎` opens that node (panel 4
zoomed). `[` `]` move the replay cursor, drawn as a highlighted column on the graphs.

## 2 · REDASH QUEUE

| Element | Value | From |
|---|---|---|
| big number | jobs waiting across the queues | §6.3 `Σ queued` |
| beside it | `6 running`, `3 stale` | live-worker rule and STALE rule of `DESIGN.md` §13 |
| under | `●●●●●●○ 6/7 workers busy`, `4 of 4 hold a runaway` | workers[].state; the stitch to §5.4 runaways |
| graph | waiting jobs over 60 min | history of `Σ queued` |
| frame | red when oldest wait ≥ 180 s or all workers busy with runaways; amber ≥ 60 s | §7 |
| title right | `oldest 3m43s ✖` | `now − min(enqueued_at)` (Redis), `—` without Redis |

Zoomed: the queue table (`QUEUE · RUNNING · WAITING · OLDEST · WORKERS · STALE · last 4 min`),
then RUNNING / WAITING / STALE as today's view 2 (same columns, same stitch `→ clickhouse3 16.3G
3.1c 57%`), a drawer with the explanation line, and `x` → dialog → `y` for the cancel. `⏎` on a
running job opens its node (panel 4) with that person selected.

## 3 · NODES

A **tile** per node, worst first (sort key pressure, `s` cycles pressure → memory → CPU →
name). Tile = `tileW × tileH` cells, `19×5` at 120 columns, `20×5` from 150, `16×4` under 100 or
under 30 rows. Inside the tile:

```
╭ clickhouse3 ───╮   name, NEW badge; the tile's frame in the node's severity
│ ████████░  93% │   memory share of the node, §5: mem_used ÷ mem_total
│ ▇▇▇▇██████████ │   the last 4 minutes, worst of each slice (history.rs)
│ 4q !2    8m31s │   queries, runaways (§5.4), the longest query's elapsed (`lag 12s` when behind and idle)
╰────────────────╯
```

As many tiles as fit in two rows; the last cell says `+N more · ≤ 42%` for the rest. The chosen
tile has the selection background and a chip title; `←→↑↓` move across the grid; the NODE panel
follows the choice. An unreachable node's tile prints `↯` and the reason in place of the bar.

Zoomed: `28×8` tiles with memory and CPU bars with denominators, the sparkline and the top
person; `⏎` opens the node.

## 4 · NODE (the chosen node)

Title: `4 clickhouse3 · 93.1% of 64 GiB · cpu 93.1%`. Rows, one per person then the server:

```
grigol.gankava  ████░░░░░░░░░░  25.5%     8m31s !     user.mem_pct §5.1 · longest query §5.4
server · …      ███████░░░░░░░  47.4%                 server.mem_pct §5.3
```

The bars share one scale (the node), so the filled cells of all rows add up to the node's bar:
§5.3 made visible. With 8 or more inner rows, a two-row memory graph of the last 60 min closes
the panel with `memory · 4 people + the server = 93.1%`.

Zoomed: left, WHO HOLDS IT with `pct · GiB · cores` per row and, under the chosen row, the query
(`id · Redash #… · elapsed · usually 42 s (318× / 7 d)`, read bytes/rows, progress, `E estimate`,
the SQL); right, memory and CPU over 60 min with ticks, then `lag · parts · free GiB = N× a 24 GiB
query`. `↑↓` choose a person; `e` explains the row; `c` opens a SQL session on the node; `\`
floats Claude with the row in its prompt (`enter()` on a person seeds that prompt).

## 5 · AIRFLOW

Dashboard: the day in numbers (`217 runs ✔ 211 ✖ 3 ▸ 2 ◌ 1 ↻ 1`), a 24-hour strip of all DAGs
(`▪` ran, `✖` a failure, `▸━` running now), the hours of the clock under it, then the rows of
today's view 5 in order: running (stuck ones red, with the age), queued, failed, then the DAGs.
Zoomed: today's view 5 (RUNNING, QUEUED, FAILED, ACTIVITY with the hourly timeline) plus, under
it, the **Gantt** of the run under the cursor, one row per task (`━` span, `✔ ✖ ▸ ◌ ↻`), with
`usually 1h48m (median of 30)` in its title; `⏎` on a task opens its log as today.

## 6 · JIRA

Dashboard: the flow (`4 › 4 › 1 › 3 done`), `✖ 1 overdue`, today's hours against the norm with the
month's mini bars, then the tickets with key, summary and due/age. Zoomed: LOGGED (the month's
bars as today) then the **board**: the columns as lanes side by side, each ticket a 4-row block
(key and priority, the summary in two lines, time in column · due · logged); `←→` cross lanes,
`↑↓` walk a lane, `⏎` the ticket in full (today's page).

## 7 · TAPE

Dashboard: two density rows (`✖` and `▲` events per minute over the last hour, `▁`–`█`) beside the
newest events. Zoomed: the strips with their counts and the burst label, then the whole tape with
repeats folded (`m.kairys on clickhouse-bi · 4× runaway → killed at 99% ▸ 12 events`), a drawer
for the chosen line, `n` a note, `y` the incident report (feature B3/B4).

## 8 · SESSIONS

Dashboard: one row per session: kind glyph (`✻` Claude, `▣` OpenCode, `▦` SQL, `❯` terminal),
number, name, folder ⎇ branch, state (`● working` green, `! approval` amber, `○ idle` dim,
`✕ ended`). `↑↓` choose; `⏎` zooms; `\` floats the chosen session.

Zoomed: the list on the left (two rows a session, then `+ new session` and `↻ past
conversations`), the chosen session's screen on the right (read-only until opened); `⏎` opens it
full: the PTY takes the body and every key goes to the program, `ctrl+\` as today.

## Overlays (drawn over whichever screen is under them)

- **Palette** `:` and **keys** `?` — `05-keys.md`.
- **Explain** `e` — `screens/120x36-explain-node.txt`, `-explain-user.txt`: formula, the same
  formula with the values, inputs with their source tables, the poll time, the `DESIGN.md`
  reference; `⏎` sends it to the floating Claude, `y` copies it.
- **Floating session** `\` — 80 % × 72 % of the body, title with the session and the others, a
  prompt line; `ctrl+\` once shows the prefix hints, after 250 ms the which-key box.
- **Setup wizard** `:setup` — five steps (ClickHouse, Redash, Airflow, Jira, the day), one
  question per step with the reason under it, a live check after `⏎`, secrets to the Keychain.
- **Cancel dialog** `x` on a queue row — title, three facts, `[y] cancel the job  [esc] keep it`.
- **Note** `n` — one input line on the status bar, kept on the tape.
- **Tour** `:tour` — six boxes over the live screen, each highlighting a row.
- **Toasts** — bottom-right, at most three, 5 s (10 s for errors).
