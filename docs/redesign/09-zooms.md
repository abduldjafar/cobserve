# The four zooms, designed from zero

**Status: built (October 2026, branch `zoom-redesign`).** The zoomed REDASH QUEUE, AIRFLOW, JIRA
and SESSIONS are drawn as below (`src/ui2/views/{queue,airflow,board,claude}.rs`, the shared
pieces in `views/kit.rs`); their screens in `screens/*-zoom-{queue,airflow,jira,sessions}*.txt`
are rendered from the code and held by `src/ui2/tests.rs`. Where the build differs from this text
it says so in a *Built:* note. The mockups below are the design; the screen files are the truth.

**Scope.** The zoomed REDASH QUEUE (`2 ⏎`), AIRFLOW (`5 ⏎`), JIRA (`6 ⏎`) and SESSIONS (`8 ⏎`).
Today they are the old shell's tables drawn inside the zoom frame (`src/ui2/views/{queue,airflow,
loom,jira,board,claude}.rs` plus the 4-line drawer of `views/drawer.rs`). This spec replaces their
*drawing* only. The engine, the state in `App` (`queue_selection`, `airflow_selected`,
`jira_selected`, `claude.active`, the viewports, the pages, the cancel dialog), every key of
`src/action.rs` and `docs/redesign/05-keys.md`, and the dashboard panels stay as they are. Where the
design needs one more field or one more read, §7 of each zoom says exactly which.

**House rules this design was checked against** (and §4 of each zoom re-checks): colour means
severity only (red crit, amber warn, green good news); accent marks structure, selection and the
ordinary fill of a bar or graph; identity is never a hue (people are `theme.person`, Claude's mark
`theme.claude`, as today). Every percentage prints its denominator. Nothing blocks on the network.
Every size from 80×24 renders without panic; the layouts below say what goes first. Read-only but
the Redash cancel (`x` then `y`), unchanged. Redraw on `Event::Tick` (1 s) and on every data
event: "dynamic" below always means per second or per poll, never faster.

The mockups were generated while designing and width-checked: every row is exactly 120, 80 or
160 cells in `unicode-width` terms (every glyph used is one cell: box drawing, blocks, braille,
`✖ ▲ ● ○ ◌ ◐◓◑◒ ✔ ! ▸ ↻ ✻ ▣ ▦ ❯ ⎇ ⌕ ┃ ▌ ▎ ━ ╸ ╺ ░ ▒ ▓`). They use the fake fleet's real names and
the numbers `cargo test ui2::tests::dump` prints at the simulation's instant (15:52:07 WIB), with
three exceptions that are said where they occur (the Gantt's task times, the sessions' screens —
and, for the twenty and fifty sessions of §4.2, the sessions themselves — and the usage strip).

---

## 0 · Shared design language

The dashboard's idiom is *framed cards with one big fact each* (`03-glyphs.md`). The zooms take
the same idiom one level down: **inside the zoom frame, rounded cards for the facts, one flat
list for the rows, one card at the bottom for the row under the cursor.** No zoom draws a table
header band any more except where a column needs a name.

### 0.1 Cards

`frame::rounded` boxes, title at column 1 of the top border (` title `), an optional note at the
right end of the top border and another in the bottom border (`╰─ … ─ note ╯`). Three uses:

| Card | Where | Frame colour | Title style |
|---|---|---|---|
| **hero card** (a fact: *waiting*, *queues*, *waiting · 4 min*) | the top of the queue zoom | `theme.border()` | lower-case, `theme.muted()` |
| **item card** (a run, a session) | the airflow NOW rows, the sessions grid | the item's severity (`frame::frame_style`), **accent when it is the cursor's** | mark + name, `theme.strong()`; the cursor's card gets the chip (`theme.tab_active()`) — reversed+bold in mono, so the cursor is visible without colour |
| **detail card** (the row under the cursor) | the bottom of queue, jira, airflow (its Gantt) | `theme.border()` | the row's words, `theme.strong()`; the keys that act on it at the right end, `theme.muted()` |

The detail card replaces the drawer for these zooms: `views::draw` hands the four zooms the whole
inside (`drawer::draw` keeps serving TAPE and LOCAL). A detail card is drawn only when the inside
is at least 30 rows; between 20 and 29 rows it shrinks to the sizes given per zoom; under 20 it
goes, and the status bar's hints are all the help there is.

### 0.2 Lists and the cursor

Rows are one line, in the exact order the `App` walks them (`queue_rows()`, `Sections::keys()`,
`Board::rows()`, `Sessions::order()`). The cursor's row: `▌` (accent) in the gutter and
`theme.selected()` as a band from the frame's left side to its right (`Painter::band`, as the
fleet zoom does). Sections are `widgets::rule` lines (`─ RUNNING · 6 on a worker ───`), the title
in `theme.section()`, the count in `theme.muted()`, a problem count in its severity. A list that
does not fit scrolls under its first row with `scroll_into_view` and the zoom's existing
viewport cell; the label above a cut list says `· N shown · ↓ M more`.

### 0.3 Marks, spinners, live numbers

- States keep `03-glyphs.md`'s marks: `✖` critical · `▲` warning · `✔` done · `▸` running ·
  `◌` queued · `↻` retrying · `!` runaway · `⊘` cancelled · `◐` working.
- **Spinner**: a thing that is running *right now* (a job on a worker, a live task, a session
  that printed within 3 s, a query session's query) shows `◐ ◓ ◑ ◒`, one frame per tick:
  `frame = app.clock.duration_since(UNIX_EPOCH).as_secs() % 4`. It is a mark, drawn in the
  row's text colour (accent on cards' titles), never a severity colour. In `GRAPH=ascii` / `TERM=dumb`
  it is `| / - \`.
- **Live durations**: everything that is *now − start* is recomputed every tick from `app.now()`
  (Airflow `run.took(now)`, `Task.start`, sessions' quiet time). A Redash job's age is poll-bound,
  so it is shown as `job.age_s + (app.clock − queue.taken_at)`, capped at +60 s; past that the
  freshness mark below says the numbers are old instead of the age pretending to tick.
- **Freshness** (`read 27s ago` at the right of a zoom's first line, from `jira::freshness` as
  today): `theme.text2()` under 10 s, `theme.muted()` under 60 s, `theme.faint()` under 5 min,
  then `◌ stale 6m` in amber bold — old data is a warning, so the colour rule holds.
- **Bars**: `widgets::thin_bar` (`━━━╸━━━`, half-cell) for progress and "how far to a
  threshold"; `glyph::bar::share` (`████░░`) is not used in these zooms (no shares of a node here).
  A bar's fill is `theme.bar_fill(sev)`: accent when nothing is wrong, the severity otherwise.
- **Graphs**: `glyph::graph::area` in the terminal's tier, fixed scale, `100% · 50% · 0% ┤`
  ticks on a zoom (`03-glyphs.md`), the window printed (`-4m`), `now` at the right.
- **The now line**: timelines and Gantts end in a `┃` column (accent) that is *now*; the axis row
  prints the clock next to it (`┃ now 15:52:07`, ticking). Cells left of it are the past; nothing
  is ever drawn right of it.

### 0.4 Colour, in one table

| Element | Colour | Why it is allowed |
|---|---|---|
| frame of the zoom | the panel's severity, as today | severity |
| a card's frame | the item's severity; accent for the cursor's; border otherwise | severity / structure |
| `✖ ▲ !` marks, stuck/failed/overdue words, a wait ≥ 60 s / ≥ 180 s, a bar past its threshold | red / amber | §7 thresholds |
| `✔`, `● live`, scheduler `●`, `◐ working` | green | good news |
| `▸ ◌ ↻ ◐` spinners and marks, `→ node`, keys `DATA-2207`, the `┃` now line, the cursor band, graph and bar fill | accent | structure |
| people (`grigol.gankava`, `ana.k`, `Jurgita Petrova`) | `theme.person` | one colour for all people, as today |
| `✻` and Claude's name | `theme.claude` | its own mark, as today |
| everything else | text / text2 / muted / faint | neutral |

### 0.5 Empty, error, not-configured

One shape for all four, centred in the upper third of the inside (`claude::message` does this
today), three lines: a mark and a sentence, the reason, the key that fixes it:

```
                    ◌  Redash is not configured
                       the queue needs REDASH_URL and REDASH_ADMIN_API_KEY (Redis names the waiting jobs)
                       :setup redash · r reads again
```

`✖` and amber for *unreachable / refused* (`✖ Redash refused (HTTP 401)`), `◌` muted for *not
configured* and *connecting…*. The sessions zoom's empty state is a ghost card (§4.2).

### 0.6 Widths

Everything is placed in cells by `Painter::put`/`putr` or by `widgets::Cells` columns; strings are
cut with `…` at the column's edge (`paint::tr`), never allowed across a card's side. A column that
does not fit is dropped whole (each zoom says which go first).

---

## 1 · REDASH QUEUE (`2 ⏎`)

### 1.1 Concept

The first question is *why is the queue full and who is holding it?* — so the top of the zoom is
three hero cards that answer it in one look: the jobs **waiting** (big digits, the oldest wait
against the 3-minute red line, how many Redis could name), the **queues** (a worker board per
queue: `●●●● 4/4 busy`), and the **waiting line over the last 4 minutes** on the fixed 100-job
scale with its 30/60 thresholds printed. Under them the jobs are one flat list — RUNNING with a
spinner, a live age and the stitch into ClickHouse drawn as a progress bar with memory and cores;
WAITING with each wait drawn as a bar *to the red line*; STALE last. The row under the cursor
opens as a **detail card at the bottom** with the facts of the job and its SQL (`J K` scroll), so
the list never jumps when the cursor moves — today the SQL opens under the row and shoves
everything below it.

### 1.2 Mockups

120×36:

```
 ☗ cobserve · prod    dashboard › queue                                               esc back   ● live   15:52:07 WIB  
╭ 2 REDASH QUEUE · redash.example.net · 10.1.0 ───────────────────────────────────────── 7 workers, 6 busy · esc back ─╮
│ ╭ waiting ─────────────────╮╭ queues ────────────────────────────────────────────────╮╭ waiting · 4 min ───────────╮ │
│ │ ▄█  ▀▀█  jobs waiting    ││ QUEUE              RUN  WAIT  OLDEST   WORKERS    STALE││100 ┤                       │ │
│ │  █  █▀▀  oldest 2m13s ▲  ││ queries              4     9  2m13s ▲  ●●●● 4/4 busy  3││ 50 ┤                       │ │
│ │ ▄█▄ █▄▄  7 named of 12   ││ scheduled_queries    2     3  1m01s ▲  ●● 2/2 busy    ·││  0 ┤⣀⣀⣠⣤⣤⣄⣀⣀⣠⣤⣤⣄⣀⣀⣠⣤⣤⣄⣀⣀⣠⣤⣤│ │
│ │ 3 of 4 hold a runaway !  ││ default · emails · periodic · schemas  idle            ││    -4m  ▲ 30 · ✖ 60   now  │ │
│ ╰──────────────────────────╯╰────────────────────────────────────────────────────────╯╰────────────────────────────╯ │
│ ─ RUNNING · 6 on a worker ────────────────────────────────────────────────────────────────────────────────────────── │
│        FOR     WHO             QUERY                                     IN CLICKHOUSE    progress  done  mem   cor… │
│ ▌ ◐  5m08s !  grigol.gankava  #7438 Gateway transfers                   → clickhouse3    ━━━╺━━━━  37%  16.3G  3.1c  │
│   ◐  4m08s !  j.petrova       #8585 AML dashboard                       → clickhouse-bi  ━━━━╺━━━  53%  21.2G  1.9c  │
│   ◐  3m08s    m.kairys        #7711 FX exposure                         → clickhouse2    ━━━━━━╺━  79%  15.4G  1.2c  │
│   ◐  2m53s !  r.simonyte      #8091 July close pack                     → clickhouse7    ━━━━╺━━━  51%  51.3G  2.2c  │
│   ◐  1m21s    d.zaleckas      #6120 Gateway refunds · open              gateway-mysql · mysql · not ClickHouse       │
│   ◐    51s    a.vaitkus       #8470 Merchant risk · rollup              Query Results · runs inside Redash           │
│                                                                                                                      │
│ ─ WAITING · 12 in the queues · 7 named ───────────────────────────────────────────────────────────────────────────── │
│       WAIT     WHO             QUERY                                     TO THE RED LINE · 3 min       SOURCE        │
│   ◌  2m13s ▲  r.simonyte      #8093 July close pack · by country        ━━━━━━━━╸━━━  74% of 3 min  clickhouse-bi    │
│   ◌  1m55s ▲  j.petrova       #8113 AML dashboard · by country          ━━━━━━━━╺━━━  64% of 3 min  clickhouse3      │
│   ◌  1m37s ▲  m.kairys        #7712 FX exposure · intraday              ━━━━━━╸━━━━━  54% of 3 min  clickhouse-bi    │
│   ◌  1m19s ▲  grigol.gankava  #8092 Gateway transfers · hourly          ━━━━━╺━━━━━━  44% of 3 min  clickhouse3      │
│   ◌  1m01s ▲  d.zaleckas      #8120 Chargebacks · weekly                ━━━━╺━━━━━━━  34% of 3 min  clickhouse-bi    │
│   ◌  1m01s ▲  d.zaleckas      Nightly settlement                        ━━━━╺━━━━━━━  34% of 3 min  clickhouse-bi    │
│   ◌    52s ▲  a.vaitkus       Refresh merchant risk                     ━━━╸━━━━━━━━  29% of 3 min  clickhouse-bi    │
│ ╭ #7438 Gateway transfers · grigol.gankava · queries · job run-1 ────────── SQL as ClickHouse runs it · J K scroll ╮ │
│ │ running 5m08s → clickhouse3 · c3e51f3b ! runaway · 16.3G of 24.0G limit · 3.1 cores · 37% done · ⏎ node · x canc…│ │
│ │ 6/7 workers busy · 3 of them hold a runaway ClickHouse query (clickhouse3, clickhouse-bi, clickhouse7) → why it'…│ │
│ │ ▎ WITH BankRecord AS (                                                                                          ┃│ │
│ │ ▎   SELECT BillOpId, Bank,                                                                                      ││ │
│ │ ▎     multiIf(AccNr LIKE 'LT%', 'LT', Bank = 'SEB', 'LV', 'OTHER') AS region,                                   ││ │
│ │ ▎ lines 1–3 of 10 · J K or shift ↑↓ to scroll                                                                    │ │
│ ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ move   ⏎ the node   x cancel job   J K scroll SQL                          9/9 nodes · 1.1 s · read-only 
```

The chosen row has the selection band (not visible in text); `◐` steps every second; `5m08s`
ticks; `!` is red; `▲` amber; the hero's `12` is amber bold (the oldest wait is ≥ 60 s); the
`3 of 4 hold a runaway !` line is red. The RUNNING header is the one column-name band kept, on
`theme.table_head()`, because the stitch's five numbers need naming once.

80×24:

```
 ☗ cobserve · prod    dashboard › queue       esc back   ● live   15:52:07 WIB  
╭ 2 REDASH QUEUE · redash.example.net · 10.1.0 ─ 7 workers, 6 busy · esc back ─╮
│ 12 waiting · oldest 2m13s ▲ · 7 named  ●●●●●●○ 6/7 busy · 3 stale · 3 of 4 ! │
│ queries ●●●● 4/4 · 9 ◌ · 2m13s ▲   scheduled_queries ●● 2/2 · 3 ◌ · 1m01s ▲  │
│ ─ RUNNING · 6 on a worker ────────────────────────────────────────────────── │
│ ▌ ◐  5m08s !  grigol.gankava  #7438 Gateway transfers   → clickhouse3   37%  │
│   ◐  4m08s !  j.petrova       #8585 AML dashboard       → clickhouse-bi 53%  │
│   ◐  3m08s    m.kairys        #7711 FX exposure         → clickhouse2   79%  │
│   ◐  2m53s !  r.simonyte      #8091 July close pack     → clickhouse7   51%  │
│   ◐  1m21s    d.zaleckas      #6120 Gateway refunds ·…  mysql · not ClickHo… │
│   ◐    51s    a.vaitkus       #8470 Merchant risk · r…  runs inside Redash   │
│                                                                              │
│ ─ WAITING · 12 in the queues · 7 named ───────────────────────────────────── │
│   ◌  2m13s ▲  r.simonyte      #8093 July close pack ·…  ━━━━━╸━━ 74% of 3m   │
│   ◌  1m55s ▲  j.petrova       #8113 AML dashboard · b…  ━━━━━╺━━ 64% of 3m   │
│   ◌  1m37s ▲  m.kairys        #7712 FX exposure · int…  ━━━━╺━━━ 54% of 3m   │
│   ◌  1m19s ▲  grigol.gankava  #8092 Gateway transfers…  ━━━╸━━━━ 44% of 3m   │
│   ◌  1m01s ▲  d.zaleckas      #8120 Chargebacks · wee…  ━━━╺━━━━ 34% of 3m   │
│ ╭ #7438 Gateway transfers · grigol.gankava ───────────────────── SQL · J K ╮ │
│ │ running 5m08s → clickhouse3 ! runaway · 16.3G of 24.0G · 3.1c · 37% done │ │
│ │ ▎ WITH BankRecord AS (                                   … 10 lines      │ │
│ ╰──────────────────────────────────────────────────────────────────────────╯ │
╰──────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ move   ⏎ the node   x cancel job   9/9 nodes · 1.1 s · read-only 
```

≥ 160 wide (160×48): the hero cards keep their widths (28 + 58 + 30) and the rest of the row
holds a fourth card, **`oldest wait · 4 min`**, the `history.queue_oldest` series on a fixed 300 s
scale with the 60 s and 180 s thresholds as ticks (`180 ✖ ┤`, `60 ▲ ┤`, `0 ┤`). The QUERY column
grows to 60; from 150 columns a QUEUE column (18) comes back between WHO and QUERY when the jobs
are on more than one queue (`Columns::of` already decides this at `QUEUE_COLUMN_FROM`). At 48
rows the detail card is 10 rows (6 SQL lines) and the STALE section is on screen without
scrolling.

### 1.3 What is dynamic

| Element | Changes | Computed |
|---|---|---|
| `◐` on every running row | per tick | `spinner_frame(app.clock)` (§0.3) |
| `5m08s` ages, running and waiting | per tick | `live_age(job, queue, clock) = age_s + min(60, clock − taken_at)` |
| `━━━━━━━━╸━━━  74% of 3 min` | per tick | `live_age / 180 s`, fill `bar_fill(severity::wait(live_age, saturated))` |
| `━━━╺━━━━  37%  16.3G  3.1c` | per poll (ClickHouse, 1 s) | `QueryStat::progress`, `memory_bytes`, `cores` of the stitched query |
| big `12`, `oldest 2m13s ▲` | per queue poll | `total_waiting()`, `max(oldest_wait_s)` |
| the graph | per queue poll | `history.queue_waiting.buckets(now, WINDOW_S, cols × per_column)` |
| `read 27s ago` | per tick, fading (§0.3) | `queue.age(clock)` |
| `⊘ cancelling…` on a row | per event | `app.cancelling(id)` as today |

### 1.4 Colour

| Element | Colour | Rule |
|---|---|---|
| big digits | `severity::wait(oldest, saturated)` → amber/red, else `strong` | the oldest wait's §7 severity |
| `oldest 2m13s ▲` | same severity | §7 |
| `3 of 4 hold a runaway !` | red when > 0, else muted | runaways are §5.4 critical |
| worker dots `●●●●` | `bar_fill(Warn)` when saturated *and* jobs wait, else `bar_fill(None)` (accent); `○` faint | as `widgets::dots` today |
| `✖`/`▲` after OLDEST | severity | §7 |
| graph columns | red ≥ 60, amber ≥ 30, accent below | `panels/queue.rs` thresholds, printed in the legend |
| running row age | `Crit` when its query is a runaway, else `severity::elapsed(age)` | §2.8 inherits the runaway |
| `!` | red | runaway |
| `→ clickhouse3` | accent bold | structure |
| progress bar | accent fill | a share of the query's own work, not a severity |
| `16.3G` | amber from 80 % of the query's limit, red from 95 % (`severity::limit_share`) | §5.4 |
| waiting bar | `bar_fill(severity::wait)` | §7 |
| person | `theme.person` | identity, one hue |
| source chip, `mysql · not ClickHouse` | faint | neutral |
| detail card frame | border | structure |

### 1.5 Layout

Inside = `frame::inner` of the zoom (116 × 32 at 120×36). Regions, top to bottom:

1. **Hero** (6 rows, cards of 28 · 58 · 30 cells; needs ≥ 100 columns and ≥ 26 rows). Under
   100 columns or 26 rows it is two plain lines (the 80×24 mockup): line 1 the waiting number,
   oldest, named, workers, stale, runaway holders; line 2 one chip per queue that is not idle
   (`queries ●●●● 4/4 · 9 ◌ · 2m13s ▲`). Under 8 rows the hero is one line.
2. **The list**, everything between the hero and the detail card, scrolling with
   `app.viewport.queue` (the selected row kept whole on screen; the SQL no longer takes rows
   here, so `scroll_into_view(offset, Some(line), height, len)` only).
   Columns (116): lead 14 `▌ ◐  5m08s !  ` · WHO 14 · 2 · QUERY flex (min 14) · 2 · TAIL 44.
   Drops as it narrows: the stitch's cores and memory (TAIL 44 → 20: `→ clickhouse3   37%`) under
   100 columns; the WAITING bar shrinks 12 → 8; the column-name rows go under 100 columns; under
   60 columns WHO goes (the detail card and the title still say who).
3. **Detail card** (8 rows ≥ 30 inside rows; 4 rows between 20 and 29: title, facts, one SQL line
   ending in `… N lines`; none under 20). Title: `query label · person · queue · job id`, right
   end `SQL as ClickHouse runs it | SQL as saved in Redash | Redash keeps no text for ad-hoc
   queries` and `J K scroll` when the SQL is cut. Row 1 the job's state line (today's drawer line
   2, plus the §5.4 numbers of its ClickHouse query: `16.3G of 24.0G limit · 3.1 cores · 37%
   done`). Row 2 the queue explanation (`app.queue_explanation()`), kept as it is. Rows 3+ the
   SQL via `widgets::code_block(sql, " ", scroll, room, width − 2, theme)`.

Hit regions: `Hit::Job(i)` on each job row (full inner width, 1 row), as today. No hit on the
cards. The cancel dialog is unchanged (`overlay::dialog`).

### 1.6 Keys and selection

Unchanged, all through `App::on_queue_key` (`views::pass`): `↑↓ j k PgUp PgDn Home End` move over
`queue_rows()` (running by age desc, then waiting, then stale); `⏎` follows the running job to its
node (`activate_queue_row` → `views::follow`); `x` asks, `y` sends; `J K` / `⇧↑↓` scroll the SQL in
the detail card (`sql_scroll_for(job_sql_key(job))`, `viewport.sql_max` set by the card's
`code_block`). The status bar's five hints are those of `Scope::Zoomed(PanelId::Queue)` already in
`action.rs`. No key is added.

### 1.7 Implementation notes

- New module `src/ui2/views/queue.rs` rewritten around `Painter` (like `panels/fleet.rs
  draw_zoomed`), not `Paragraph` lines: `pub fn draw(frame, app, theme, inside: Rect)` takes the
  whole inside; `views::draw` no longer carves a drawer for `PanelId::Queue`.
- Reuse: `panels::queue::{runaway_holders, worker_dots, GRAPH_MAX}`, `panels::fleet::ticked_graph`
  (give it a `max` and tick labels parameter: `ticked_graph_on(p, app, theme, r, series, max,
  ticks: &[(f64, &str)])`), `glyph::big::draw`, `widgets::{thin_bar, code_block, dots}`,
  `queue.rs::{short_bytes, clickhouse_cell}` (the cost lookup moves to a helper returning a
  struct), `app.queue_explanation()`, `app.job_sql`, `severity::{wait, elapsed, limit_share}`.
- New helpers (signatures):
  - `pub fn spinner(clock: SystemTime, tier: GraphTier) -> char` in `ui2/glyph/mod.rs`.
  - `pub fn live_age(job: &Job, queue: &QueueStatus, clock: SystemTime) -> u64` in `views/queue.rs`.
  - `pub fn freshness_style(age: Option<Duration>, theme: &Theme) -> (String, Style)` in
    `views/widgets.rs` (shared by the four zooms).
  - `pub fn hero(p, app, theme, r: Rect)`, `pub fn list(p, app, theme, r) -> Vec<(Rect, Hit)>`,
    `pub fn detail(p, app, theme, r)` in `views/queue.rs`.
  - `pub fn stitch(app: &App, job: &Job) -> Option<Stitch { node, memory_bytes, limit, cores,
    progress, runaway }>` in `views/queue.rs` (what `clickhouse_cell` computes today, as data).
- The dashboard panel `panels/queue.rs` is untouched.
- Tests to change (`src/ui2/views/tests.rs`): `the_queue_view_lists_people_and_the_stitch_to_clickhouse`
  keeps every assertion but `"▎ WITH BankRecord AS ("` (now inside the detail card, still with
  the `▎` gutter — the assertion holds as written) and `"4/4 busy"` (now `4/4 busy` in the queues
  card — holds). `x_then_y_cancels_a_job…`: `"⊘ cancelling… · → clickhouse3"` becomes
  `"⊘ cancelling…"` on the row and `"→ clickhouse3"` in the tail (assert both, not adjacent).
  `j_and_k_scroll_the_sql_under_a_job`: at 120×30 the detail card has 2 SQL rows → `"lines 1–1 of
  10"`; assert `contains("lines 1–")` as today (holds) and `"lines 2–"` after `J` (holds).
  `a_click_puts_the_cursor_on_a_row…` holds (`Hit::Job`). `src/ui2/tests.rs`
  `the_zoomed_queue_and_the_cancel_dialog`: regenerate `docs/redesign/screens/120x36-zoom-queue.txt`
  from the new render (`same_frames` compares card corners, so the file must be the new screen);
  replace the parts list with `["╭ waiting ", "╭ queues ", "4/4 busy", "─ RUNNING · 6 on a worker ─",
  "→ clickhouse3", "─ WAITING · ", "╭ #7438 Gateway transfers · grigol.gankava"]`; the title row
  and the four hints are unchanged. The cancel-dialog assertions hold (the dialog is unchanged).

---

## 2 · AIRFLOW (`5 ⏎`)

### 2.1 Concept

The first question is *what needs a look right now?* — so the top is a row of **item cards**, one
per run that is running, queued or failed today, in the order the cursor walks them: mark and DAG
in the title, the state and a live duration, who and when, and what its tasks are doing (a
spinner on the live task, a progress bar from `Progress`). Under them **the day as a loom**: one
thread per DAG that ran in the last 24 h, a cell a half hour (an hour at 80 columns, a quarter
from 160), hours of the clock over the cells, and a `┃` **now line** that every running thread
grows into. The bottom is a **Gantt card** of the run under the cursor on its own time axis, task
bars ending in `✔ ✖ ◐ ◌`, the running one reaching the now line. The three pieces answer, in
order: what is wrong, what the day looked like, what this run is doing.

### 2.2 Mockups

120×36 (the cursor on the first card, `test_clickhouse_connection`; its chip title and accent
frame are not visible in text):

```
 ☗ cobserve · prod    dashboard › airflow                                             esc back   ● live   15:52:07 WIB  
╭ 5 AIRFLOW · airflow.example.net · 2.10.2 ─────────────────────────────── scheduler ● 2s · triggerer ● 4s · esc back ─╮
│ last 24 h   217 runs  ✔ 211  ✖ 3   ▸ 2 running  ◌ 1 queued  ↻ 1 retrying   13 of 13 DAGs ran · 3 paused  read 0s ago │
│ NOW · 2 running · 1 stuck · 1 queued · 3 failed today                                                                │
│ ╭ ✖ test_clickhouse_connection ──────╮ ╭ ▸ statement_daily_agg_reload ──────╮ ╭ ◌ gateway_transfers_backfill ──────╮ │
│ │ stuck · running 262d04h            │ │ running 2h12m · 4 of 7 ━━━━╸━━━    │ │ queued · waiting 4m12s             │ │
│ │ manual · Jan 24 10:52 · airflow    │ │ manual · 13:39 · data-platform     │ │ manual · 15:47 · tomas.r           │ │
│ │ ↻ ping_clickhouse waits for a retry│ │ ◐ reload_partitions · 41m          │ │ no task has started                │ │
│ ╰────────────────────────────────────╯ ╰────────────────────────────────────╯ ╰────────────────────────────────────╯ │
│ ╭ ✖ replication_app_consistency_che…─╮ ╭ ✖ kyc_onboarding_tables ───────────╮ ╭ ✖ cbk_accounts_report ─────────────╮ │
│ │ failed at check_consistency        │ │ failed at build_cohorts            │ │ failed at upload_report            │ │
│ │ scheduled · 13:00 → 13:08 · 8m06s  │ │ scheduled · 12:30 → 12:40 · 10m48s │ │ scheduled · 07:15 → 07:18 · 3m36s  │ │
│ │ 2h ago · j.petrova                 │ │ 3h ago · ana.k                     │ │ 8h ago · ana.k · ✔ 4 runs since    │ │
│ ╰────────────────────────────────────╯ ╰────────────────────────────────────╯ ╰────────────────────────────────────╯ │
│ THE DAY · 13 DAGs ran · a cell a half hour · 10 shown · ↓ 3 more                                                     │
│                                         18    21    00    03    06    09    12    15┃ now 15:52:07                   │
│   gateway_transfers_backfill        ···············································◌┃ triggered · queued             │
│   clickhouse_replication_check      ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ every 15m · in 7m              │
│   postgres_blacklist_etl            ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ every 30m · in 7m              │
│   posthog_warehouse_refresh         ·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪┃ :20 hourly · in 27m            │
│   cbk_accounts_report               ··▪···▪···▪···▪···▪···▪···▪···✖···▪···▪···▪···▪·┃ ✖1 · :15 every 2h · in 1h      │
│   single_table_reconcile            ·······▪·······▪·······▪·······▪·······▪·······▪┃ :10 every 4h · in 3h           │
│   redash_scheduler_monitoring       ▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·▪·┃ 1h · in 7m                     │
│   statement_daily_agg_reload        ···········································▸━━━━┃ triggered · running 2h12m      │
│   replication_app_consistency_che…  ··········································✖·····┃ ✖1 · 06:00 · in 21h            │
│   kyc_onboarding_tables             ·········································✖······┃ ✖1 · 05:30 · in 20h            │
│ ╭ statement_daily_agg_reload · manual 13:39 · running 2h12m · 4 of 7 tasks ───────────── ⏎ task log · o in Airflow ╮ │
│ │  task                           14:00       14:30        15:00       15:30     ┃ now 15:52:07                    │ │
│ │  build_partitions              ━━━━━━━━━━━━━━━━━✔                              ┃ done 14:40 · 42m                │ │
│ │  validate_counts                                 ━━━━✔                         ┃ done 14:52 · 12m                │ │
│ │  publish_counts                                       ━━━━━━━✔                 ┃ done 15:11 · 19m                │ │
│ │  reload_partitions                                            ◐━━━━━━━━━━━━━━━━┃ running 41m · try 1 of 1        │ │
│ │  refresh_views                                                                 ┃ ◌ queued                        │ │
│ ╰─────────────────────────────────────────────────────────────────── 7 tasks · ↑ extract_statements ✔ · ↓ notify ◌ ╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ move   ⏎ task log   o in Airflow   y copy link   r read now                9/9 nodes · 1.1 s · read-only 
```

The Gantt's task names and times are *not* the fake's (`fake_tasks` spaces every task 40 s
apart, which is a dot on a two-hour axis); see §2.7 for the change that makes the fake read like
this. Everything else is the fake's.

80×24 (no Gantt under 30 inside rows; the cards are one line each under 100 columns):

```
 ☗ cobserve · prod    dashboard › airflow     esc back   ● live   15:52:07 WIB  
╭ 5 AIRFLOW · airflow.example.net · 2.10.2 ─ scheduler ● 2s · triggerer ● 4s …─╮
│ last 24 h  217 runs  ✔ 211  ✖ 3   ▸ 2  ◌ 1  ↻ 1   13 of 13 DAGs  read 0s ago │
│ NOW · 2 running · 1 stuck · 1 queued · 3 failed today                        │
│ ▌✖ test_clickhouse_connection      stuck 262d04h · ↻ ping_clickhouse to ret… │
│  ▸ statement_daily_agg_reload      running 2h12m · 4 of 7 ━━━━╸━━━ · ◐ relo… │
│  ◌ gateway_transfers_backfill      queued · waiting 4m12s · manual 15:47     │
│  ✖ replication_app_consistency_c…  failed at check_consistency · 2h ago      │
│  ✖ kyc_onboarding_tables           failed at build_cohorts · 3h ago          │
│  ✖ cbk_accounts_report             failed at upload_report · 8h ago          │
│ THE DAY · 13 DAGs ran · a cell an hour · 10 shown · ↓ 3 more                 │
│                                   18    00    06    12  ┃ now 15:52          │
│   gateway_transfers_backfill    ·······················◌┃ triggered · queued │
│   clickhouse_replication_check  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ every 15m · in 7m  │
│   postgres_blacklist_etl        ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ every 30m · in 7m  │
│   posthog_warehouse_refresh     ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ :20 hourly · in 2… │
│   cbk_accounts_report           ·▪·▪·▪·▪·▪·▪·▪·✖·▪·▪·▪·▪┃ ✖1 · :15 every 2h  │
│   single_table_reconcile        ···▪···▪···▪···▪···▪···▪┃ :10 every 4h · in… │
│   redash_scheduler_monitoring   ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪┃ 1h · in 7m         │
│   statement_daily_agg_reload    ·····················▸━━┃ triggered · runni… │
│   replication_app_consistency…  ·····················✖··┃ ✖1 · 06:00 · in 2… │
│   kyc_onboarding_tables         ····················✖···┃ ✖1 · 05:30 · in 2… │
╰──────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ move   ⏎ task log   o in Airflow   9/9 nodes · 1.1 s · read-only 
```

≥ 160 wide (160×48): four cards a row (38 cells each, 4 × 38 + 3 = 155, centred in 156); the loom
gets 96 quarter-hour cells with a label every two hours and room for an OWNER column (16) after
the name, as `Grid::OWNER_FROM` gives it today; the Gantt card is 12 rows (10 tasks) and its axis
labels every 15 minutes. At 48 rows all 13 threads fit without scrolling.

### 2.3 What is dynamic

| Element | Changes | Computed |
|---|---|---|
| `running 2h12m`, `stuck · running 262d04h`, `waiting 4m12s` | per tick | `run.took(now)`, `run.waited(now)` with `now = app.now()` |
| `◐ reload_partitions · 41m` | per tick | spinner + `now − Task.start` of the live task (`activity.tasks_of(run)`) |
| `4 of 7 ━━━━╸━━━` | per Airflow read | `Progress { done, total }` |
| `┃ now 15:52:07` | per tick | the clock; the loom's last cell is the one holding now (`Axis::new`) |
| `▸━━━━` growing into the now line | per read (cells) · the glyph `▸` is static, the thread's tail says `running 2h12m` ticking | `airflow::timeline` |
| Gantt bars | per task read; the running bar's end per tick | `TaskRun.start/end`, `now` for a running task |
| `in 7m` (next run) | per tick, minute resolution | `dag.next − now` via `jira::age` |
| `read 0s ago` | per tick, fading | `activity.age(clock)` |
| `reading…` in the first line | per event | `app.is_reading(Feed::Airflow)` |

### 2.4 Colour

| Element | Colour | Rule |
|---|---|---|
| card frame + title mark | `run.severity(now)`: red failed/stuck, amber > 6 h or queued > 1 h; accent for the cursor's card; border otherwise | §7 / `airflow.rs` thresholds |
| `stuck`, `failed at check_consistency` | red | severity |
| `▸`, `◐`, progress fill | accent | structure |
| `◌ queued`, `no task has started` | muted | neutral |
| `↻ … waits for a retry` | amber | retrying is a warning, as today |
| `✔ 4 runs since` | green | good news |
| owner (`ana.k`) | person | identity |
| loom cells | `▪` accent (`bar_fill(None)`), `✖` red bold, `▸` accent bold, `◌` muted, `━` the run's colour | as `timeline_spans` today |
| thread name | red when its day has a failure (`DagLine::severity`), else text | severity |
| thread tail `✖1` | red bold | severity |
| `┃` now line, axis clock | accent | structure |
| Gantt `✔` green, `✖` red, `◐` accent, `◌` muted; bars accent, a failed task's bar red | severity / structure |
| health `scheduler ● 2s` in the frame title | green dot, as today | good news |

### 2.5 Layout

Inside = 116 × 32 at 120×36. Regions:

1. **Day line** (row 0): `last 24 h  217 runs ✔ 211 ✖ 3  ▸ 2 running ◌ 1 queued ↻ 1 retrying  13 of
   13 DAGs ran · 3 paused`, freshness at the right (today's `day_line` + `freshness`). Before
   `day_read` it says `reading the last 24 h of runs…` and the loom region says the same.
2. **NOW label** (row 1): `NOW · 2 running · 1 stuck · 1 queued · 3 failed today`; when nothing
   runs, waits or failed: `NOW · nothing runs, nothing waits · the day went well ✔`.
3. **Cards**: `per_row = max(1, (width + 1) / 39)` cards of 38 × 5 (3 inner lines) in reading
   order running → queued → failed; rows of cards until all are placed, **at most two rows**
   (ten more knots say `+4 more below` in the NOW label and continue as one-line rows); under
   100 columns every knot is a one-line row (`▌✖ name  state · …`), as in the 80×24 mockup. The
   cursor's card is kept in view: when the cards alone exceed the room, they scroll with the
   same offset as the loom (one `viewport.airflow` over the concatenated row list, as `loom.rs`
   does with `spots`).
4. **THE DAY label + axis** (2 rows): label with the DAG count, cell size and `shown / ↓ more`;
   the axis row carries the hour labels (`Axis::labels(offset_s)`) and `┃ now HH:MM:SS`.
5. **Threads**: name 32 (28 under 100 columns, cut with `…`) · 2 · cells · `┃` · 1 · tail 30 (18
   under 100). `slots = airflow::slots_for(width − name − 2 − 1 − 1 − tail)`, so 48 at 120, 24 at
   80, 96 at 160. Tail: `✖N · schedule · in Xm | paused | running Xm | queued`.
6. **Gantt card** (8 rows ≥ 30 inside rows; 6 rows between 24 and 29 with 3 task rows; none
   below 24). Title: `dag · run_label · state took · done of total tasks`, right `⏎ task log · o in
   Airflow`. Axis: name 22 · 56 cells (`inside.width − 2 − 22 − 2 − 32`) · `┃` · status 32. The
   axis runs from `run.start` (or `queued`) to `now` for a live run, to `run.end` for a finished
   one; labels at round clock times at least 8 cells apart. Tasks: the rows around the cursor's
   interest — the running/failed task centred, done tasks above, queued below; the bottom border
   says what is cut (`7 tasks · ↑ extract_statements ✔ · ↓ notify ◌`). When the tasks are not
   read yet the card shows one row: `◐ 4 of 7 done · ▸ reload_partitions · reading its tasks…`,
   from `Progress` and `tasks_of`.

Drops as it shrinks: Gantt first (height), then the cards' third line (height < 24 → 4-row
cards? no: cards become one-line rows when the inside is under 24 rows *or* 100 columns), then
the loom's tail (width < 90), then the NEXT part of the tail. The loom never goes: at 80×24 it is
the body of the zoom.

Hit regions: `Hit::AirflowRow(i)` with `i` the index in `Sections::keys()` — each card's full
rect, each thread's row. Clicks inside the Gantt card: none (its page has its own hits once
opened).

### 2.6 Keys and selection

Unchanged: `↑↓ j k PgUp PgDn Home End` through `App::on_airflow_key` (`move_airflow` over
`airflow_rows()` = cards then threads); `⏎ → l` opens the run's tasks or the DAG's runs as a page
(`detail::draw` keeps drawing pages inside the zoom, unchanged, with the drawer-less inside); `o`
in Airflow, `y` copy, `r` read now. `←` on a card is a page key only (unchanged). No key added.
Cursor identity stays `RowKey`, so a re-read under the cursor keeps it (`resolve_lists`).

### 2.7 Implementation notes

- `src/ui2/views/airflow.rs` rewritten with `Painter`; `loom.rs` is removed (its knot/thread
  code becomes the cards and threads here); `detail.rs` untouched.
- Reuse: `airflow::{sections, counts, timeline, Axis, slots_for, short_schedule, run_label,
  when}`, `views/airflow.rs::{timeline_spans → cell_glyph(cell) -> (char, Style)}`,
  `widgets::thin_bar`, `jira::freshness`, `fmt::{dur, ago}`, `jira::age` (for `in 7m`).
- **Task reads for the Gantt** (the one new read, asynchronous): `App` gains
  `airflow_tasks: HashMap<(String, String), (Vec<TaskRun>, SystemTime)>`. When the cursor is on a
  run row and the map has no entry younger than 30 s, `views::airflow::draw` pushes
  `Ask::Tasks { dag, run }` through the existing `detail_asks` channel via a new
  `App::peek(ask: Ask)` (same plumbing as `open_page`, no page); `App::on_detail` stores
  `Body::Tasks` into the map as well as into pages. `main.rs` already answers asks
  (`sources/airflow.rs::tasks`, `fake::detail`). The drawing never waits: §2.5 (6) says what it
  draws meanwhile. Entries older than 10 min are dropped on each read.
- `fake::fake_tasks` spreads tasks across the run: `step = (end.unwrap_or(now) − start) / count`
  instead of 40 s, so the fake Gantt has bars (the mockup's names are the fake's `extract …`
  list once spread; the 120×36 mockup used descriptive names for legibility — the screen file is
  whatever the fake renders).
- New helpers: `pub fn knot_card(p, theme, r: Rect, run: &Run, activity: &Activity, app, now,
  chosen: bool)`, `pub fn one_line_knot(…) -> Cells`, `pub fn gantt(p, theme, r, run, tasks:
  Option<&[TaskRun]>, progress: Option<&Progress>, now)`, `pub fn time_axis(start: i64, end: i64,
  cells: usize, offset_s: i64) -> Vec<(usize, String)>` (labels at round times), `pub fn
  span_cells(start, end, axis) -> (usize, usize)`.
- Tests to change. `views/tests.rs::airflow_shows_what_runs_waits_and_failed_and_every_dag_s_day`:
  `row("Airflow 2.10.2")` is now the zoom's title row (holds: host + `scheduler ●`); `row("last 24
  h").contains("✖ 3")` holds; `row("RUNNING").contains("1 stuck")` → `row("NOW ·").contains("1
  stuck")`; `row("test_clickhouse_connection").contains("262d")` → the card's second row
  (`row("stuck · running")`); `row("statement_daily_agg_reload").contains("4/7")` → `"4 of 7"` on
  the card's first inner row; `row("QUEUED")`/`row("FAILED")` → `row("NOW ·")` contains `1 queued`
  and `3 failed today`; `row("▌").contains("test_clickhouse_connection")` → the cursor's card title
  contains it (assert the title row contains `✖ test_clickhouse_connection` and that
  `app.airflow_selection()` is the first key); `"stuck: running for more than a day"` → `"stuck ·
  running 262d"`; the SCHEDULE/RUNS header assertion → the axis row contains `18`, `00`, `06`,
  `12` and `┃ now`; `kyc` row: contains `✖` and `✖1` and `05:30`; `fifteen`: `▪▪▪▪` and `every
  15m` hold. `wide_airflow_tells_its_knots_and_weaves_the_day` (172×48): `" now  "` → `"NOW ·"`
  with `2 running · 1 stuck · 1 queued`; the stuck/running assertions move to card rows; `failed
  today` → `3 failed today`; `at check_consistency` holds (card row 1); the hours row assertion
  holds minus `SCHEDULE`; the click on `Hit::AirflowRow(3)` opening `Ask::Tasks` holds.
  `ui2/tests.rs::the_zoomed_airflow_jira_and_tape_are_today_s_views`: regenerate
  `120x36-zoom-airflow.txt`; parts `["NOW ·", "test_clickhouse_connection", "THE DAY",
  "replication_app_consistency_check", "╭ statement_daily_agg_reload"]`; title and hints unchanged.

---

## 3 · JIRA (`6 ⏎`)

### 3.1 Concept

The first question is *what is on my board, and what is late?* — so the zoom **is the board at
every width from 80 columns** (today only from 140): the columns as lanes side by side, each
ticket a 4-line block — key and priority, the summary in two lines, then its age in the column,
its due date and the time logged — the lane holding the cursor underlined in accent. Above the
lanes the month's time stays, as the proven bar chart, with today's hours and the norm in words.
The bottom is a **detail card** for the ticket under the cursor (summary in full, reporter, made,
updated, labels, link), in place of the drawer. Overdue and due-soon counts sit in the lane
headers where the tickets are, not only in the frame title.

### 3.2 Mockups

120×36:

```
 ☗ cobserve · prod    dashboard › jira                                                esc back   ● live   15:52:07 WIB  
╭ 6 JIRA · jira.example.net · 9.12.1 · Sam Example ──────────────────────────── ✖ 1 overdue · ▲ 1 due soon · esc back ─╮
│ LOGGED · October 2026   66h30m in 9 working days · 7h23m a working day · today 2h30m       t by ticket · read 0s ago │
│       8½  8       7½  8 8½  8 7½        8 2½                                                                         │
│   9h │▇▇ ▆▆       ▅▅ ▆▆ ▇▇ ▆▆ ▅▅       ▆▆                                                                            │
│      │██ ██       ██ ██ ██ ██ ██       ██ ▄▄                                                                         │
│        1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31                   │
│                                                                                                                      │
│ IN PROGRESS 4 ▲ 1 due soon   IN REVIEW 4 ✖ 1 overdue      FEEDBACK 1                   DONE 8 in 7 days              │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━  ───────────────────────────  ───────────────────────────  ───────────────────────────   │
│ ▌DATA-2207             High   DATA-2647             ASAP   DATA-1289             High   DATA-2638             ✔ 2h   │
│ ▌Product metrics              Client account balances      Daily per-covenantee         Enrich                       │
│ ▌integration — PostHog + C…   as a daily warehouse tabl…   history of statement vs a…   accounting_monitoring.sta…   │
│ ▌11d in column                4d · overdue 2d · 7h12m      12d · 4h logged              2h logged                    │
│                                                                                                                      │
│  DATA-2804           Medium   DATA-1817             High                                DATA-2737             ✔ 2d   │
│  Reload                       Replicate                                                 Google Chat failure alerts   │
│  mv_statement_daily_agg fo…   gateway.bank_transfer_dat…                                for the 10 highest-impact…   │
│  8h · 1h30m logged            2d · 2h logged                                            2h logged                    │
│                                                                                                                      │
│  DATA-2850                —   DATA-2788             High                                DATA-2650             ✔ 3d   │
│  STATEMENT_DAILY_AGG:         cbk_accounts_report: close                                Add all                      │
│  upstream moves drifted 13…   the duplicate-upload race…                                gateway.bank_account colu…   │
│  12h · due tomorrow           2d · 3h logged                                            30m logged                   │
│                                                                                                                      │
│  DATA-2849                —   DATA-2773             High                                DATA-2653             ✔ 3d   │
│  airflow-dags: extract        KYC onboarding tables for                                 Replicate CreditOnline       │
│  duplicated Google auth an…   funnel cohorts and questi…                                tables to ClickHouse BI      │
│  15h · 30m logged             2d · 4h logged                                            30m logged                   │
│ ╭ DATA-2207 · Story · High · In progress for 11d ──────────────── ⏎ in full · o in Jira · y copy link · r read now ╮ │
│ │ DE - Product metrics integration — PostHog + ClickHouse → portal (PoC)                                           │ │
│ │ from Jurgita Petrova · made Sep 29 · updated 10d ago · posthog, clickhouse, poc                                  │ │
│ │ https://jira.example.net/browse/DATA-2207                                                                        │ │
│ ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ←→ a lane   ↑↓ a ticket   ⏎ ticket in full   o in Jira   y copy link          9/9 nodes · 1.1 s · read-only 
```

80×24 (3-line blocks, one-line summaries, one LOGGED line, no detail card):

```
 ☗ cobserve · prod    dashboard › jira        esc back   ● live   15:52:07 WIB  
╭ 6 JIRA · jira.example.net · 9.12.1 · Sam Example ─ ✖ 1 overdue · ▲ 1 due so…─╮
│ LOGGED · October · 66h30m in 9 working days · today 2h30m        t by ticket │
│ IN PROGRESS 4 ▲ 1  IN REVIEW 4 ✖ 1    FEEDBACK 1         DONE 8 in 7 days    │
│ ━━━━━━━━━━━━━━━━━━ ────────────────── ────────────────── ──────────────────  │
│ ▌DATA-2207    High  DATA-2647    ASAP  DATA-1289    High  DATA-2638    ✔ 2h  │
│ ▌Product metrics …  Client account b…  Daily per-covena…  Enrich accountin…  │
│ ▌11d in column      4d · overdue 2d    12d · 4h           2h logged          │
│                                                                              │
│  DATA-2804  Medium  DATA-1817    High                     DATA-2737    ✔ 2d  │
│  Reload mv_statem…  Replicate gatewa…                     Google Chat fail…  │
│  8h · 1h30m         2d · 2h                               2h logged          │
│                                                                              │
│  DATA-2850       —  DATA-2788    High                     DATA-2650    ✔ 3d  │
│  STATEMENT_DAILY_…  cbk_accounts_rep…                     Add all gateway.…  │
│  12h · due tomorr…  2d · 3h                               30m logged         │
│                                                                              │
│  DATA-2849       —  DATA-2773    High                     DATA-2653    ✔ 3d  │
│  airflow-dags: ex…  KYC onboarding t…                     Replicate Credit…  │
│  15h · 30m          2d · 4h                               30m logged         │
│                                                                              │
│                                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
 esc back   ←→ a lane   ↑↓ a ticket   ⏎ in full   9/9 nodes · 1.1 s · read-only 
```

≥ 160 wide (160×48): lanes of 37 (`(156 − 6) / 4`), the month chart three bars tall with
three-cell columns (as `month_chart` already does from 40 rows), the summaries wrap less, the
detail card is 7 rows and adds the description's first two lines once the ticket's page body is
known (`app.jira_page` is not opened for that; the card reads it only if a page was opened before
and its body is cached — otherwise the three lines of the 120×36 card). A fifth lane, `moved on`,
appears whenever `board.others()` is not empty (today's `board::lanes` already appends it).

### 3.3 What is dynamic

| Element | Changes | Computed |
|---|---|---|
| `11d in column`, `4d` | per tick, coarse units (`jira::age`) | `ticket.in_status(now)` |
| `today 2h30m` and today's bar | per worklog read (every few minutes) | `Month::per_day` |
| `overdue 2d`, `due tomorrow` | per day roll-over | `jira::today(now, offset)` |
| `read 0s ago` | per tick, fading | `board.age(clock)` |
| `reading…` | per event | `app.is_reading(Feed::Jira)` |
| the lane underline `━━━━` | per key | the lane holding `jira_selection()` |

Nothing in Jira runs *now*, so there is no spinner here; honesty over motion.

### 3.4 Colour

| Element | Colour | Rule |
|---|---|---|
| `✖ 1 overdue` in a lane header, `overdue 2d` in a block | red bold | `jira::due_severity` Crit |
| `▲ 1 due soon`, `due tomorrow`, `due today` | amber bold | Warn |
| priority `ASAP`/`URGENT` | red bold (`priority_severity`) · `High` strong · others text2 · `—` faint | as today |
| `✔ 2h` on a finished block | `✔` green, the age muted | good news |
| key `DATA-2207` | accent bold (muted when finished) | structure |
| lane underline | accent under the cursor's lane, rule elsewhere | structure |
| `▌` and the block's band | accent / selection | cursor |
| reporter | person | identity |
| month bars | `bar_fill(None)`; today's bar accent | structure |
| detail card frame | border | structure |

### 3.5 Layout

Inside = 116 × 32 at 120×36. Regions:

1. **LOGGED** (5 rows ≥ 26 inside rows: header, values, two bar rows, days; 1 row below that:
   `LOGGED · October · 66h30m in 9 working days · today 2h30m` with `t by ticket` at the right).
   Drawn by today's `month_chart` (bars = 2 under 40 rows, 3 from 40). Not drawn before
   `worklogs_read` (the region collapses to the one-line form saying `reading this month's
   worklogs…`).
2. One blank row (only when the chart is drawn).
3. **Lane headers** (2 rows): `STATUS N` + `✖ n overdue` or `▲ n due soon` (open lanes) or `in 7
   days` (the finished lane), cut to the lane; under it the underline row (`━` accent for the lane
   with the cursor, `─` rule for the others).
4. **Lanes**: `count = lanes.len()`, `lane_w = (width − 2 × (count − 1)) / count` (gap 2; gap 1
   and `lane_w = (width − (count − 1)) / count` under 100 columns). Blocks of 4 lines + 1 blank
   (3 + 1 under 100 columns or under 26 rows: the summary is one line). All lanes share one
   vertical offset (`viewport.jira`), the cursor's block kept whole on screen as `board.rs` does.
   An empty lane says `nothing here` faint.
5. **Detail card** (5 rows ≥ 30 inside rows; none below). Title `KEY · kind · priority · status
   for age`, right `⏎ in full · o in Jira · y copy link · r read now`; rows: the summary in full
   (tag kept, as the drawer does), the facts line (`from reporter · made · updated · finished ·
   due (label) · logged · under parent · labels`, today's drawer line 2), the link.

Block lines (width `lane_w`): line 1 `gutter + key` left, priority right (`✔ age` for finished);
lines 2–3 `board::wrap(words, lane_w − 1, 2)`; line 4 the pieces joined by ` · `: `Nd in column`
when it is alone, else `Nd · due … · Nh logged` (the word `logged` is dropped first, then
`in column`, when the line is cut).

Hit regions: `Hit::Ticket(i)` with `i` the index in `board.rows()` (columns flattened, then
`others()`), each block's rect (lane x, 4 rows), as `board.rs` registers them. `Hit::TimeDay` /
`Hit::TimeTicket` stay on the `t` page.

### 3.6 Keys and selection

Unchanged through `App::on_jira_key`: `←→ h l` cross lanes (`cross_lane`, which needs
`viewport.jira_board` — now set `true` at every width, since the lanes are always lanes), `↑↓ j
k PgUp PgDn Home End` walk `board.rows()` (down a lane and on into the next), `⏎` the ticket's page,
`o`, `y`, `r`, `t` the month's time (`draw_time` unchanged, drawn in the whole inside). No key
added. `esc` closes the page or the time page before the zoom (`views::has_inner`).

### 3.7 Implementation notes

- `src/ui2/views/jira.rs` keeps `month_chart`, `freshness`, `draw_time`, `half_hours`; its
  narrow table (`Widths`, `header`, `column_rule`, `ticket_line`) goes. `board.rs` becomes the
  only board, drawn with `Painter`: `pub fn draw(p, app, theme, inside)` laying out the regions
  above; `lanes()` and `wrap()` stay as they are (`views::board_lanes` is used by `cross_lane`).
- Reuse: `jira::{due_severity, due_label, age, logged, split_tag, today}`, `Board::{columns,
  others, rows, link}`, `widgets::Cells` for the block lines (then `Painter::put` per line).
- New helpers: `pub fn block_lines(ticket, finished, lane_w, now, today, theme) -> Vec<Cells>`
  (4 or 3 lines), `pub fn lane_header(lane: &Lane, board, today, theme, w) -> Cells`, `pub fn
  detail(p, app, theme, r, ticket)`.
- `app.viewport.jira_board.set(true)` unconditionally in `draw` (the field can go in a later
  cleanup together with the `if self.viewport.jira_board.get()` guards in `on_jira_key`).
- Tests to change. `views/tests.rs::jira_shows_your_tickets_in_the_board_s_columns`:
  `row("jira.example.net").contains("Sam Example")` holds (the zoom's title); the `flow` line
  assertions (`In progress 4 … Done 8 … ✖ 1 overdue … ▲ 1 due soon`) move to the lane header row:
  assert it contains `IN PROGRESS 4`, `IN REVIEW 4`, `FEEDBACK 1`, `DONE 8 in 7 days`, `✖ 1
  overdue`, `▲ 1 due soon`; `row("─ IN REVIEW")` → `row("IN REVIEW 4")`; `overdue` assertions
  (`ASAP`, `overdue 2d`, `7h12m`) split over block lines 1 and 4: assert `row("DATA-2647")`
  contains `ASAP` and the screen contains `4d · overdue 2d · 7h12m`; `!overdue.contains("DE - ")`
  holds for the block (the tag is split off) — the detail card's summary line *keeps* the tag, so
  assert on the block line; `row("DATA-2850").contains("due tomorrow")` → `screen.contains("12h ·
  due tomorrow")`; `row("▌").contains("DATA-2207")` holds; the URL holds (detail card). The size
  loop holds. `wide_jira_is_a_board_of_lanes_and_the_arrows_cross_them`: `in progress 4`-style
  lower-case titles → `IN PROGRESS 4`, `IN REVIEW 4 ✖ 1 overdue`, `FEEDBACK 1`, `DONE 8 in 7 days`;
  `▌ DATA-2207` → `▌DATA-2207` (no space after the gutter); `"4d · overdue 2d · 7h12m logged"` →
  without `logged` when the lane is narrower than 32, so assert `4d · overdue 2d · 7h12m`; the
  `narrow` (120×36) assertion `contains("─ IN REVIEW")` → `contains("IN REVIEW 4")`; the arrow and
  click assertions hold. `t_shows_which_tickets_took_the_time_day_by_day` and
  `jira_charts_the_hours_logged_this_month_day_by_day` hold (the chart and the time page are
  unchanged). `ui2/tests.rs`: regenerate `120x36-zoom-jira.txt`; parts `["DATA-2207", "DATA-2647",
  "IN PROGRESS 4"]`; `⏎` then `esc` assertions hold.

---

## 4 · SESSIONS (`8 ⏎`)

### 4.1 Concept

The first question is *what are my sessions doing?* — and the honest answer depends on how many
there are. Four sessions can each show their own screen; twenty cannot, and a grid that shows
four of twenty answers the question for a fifth of them. So the zoom has **two layouts, chosen by
the count** against what the screen can hold (the exact rule is §4.5):

- **Few — the grid of live thumbnails.** Every session a card whose body is the bottom of its own
  screen, drawn from the same `vt100` screen the full view uses, cropped (never resized) to the
  card. A glance shows the Claude that is running tests, the OpenCode waiting for a permission,
  the shell at its prompt, the query session's last answer. The title carries the kind's mark, the
  number, the name and a state that moves (`◐ working` spinning while output arrives, `! needs
  you` amber on a bell, `○ idle`, `✕ ended`); the line under it says where it works and what it
  used today.
- **More than fit — the list and the big preview.** Every session a line, grouped by project
  (`Sessions::groups()` — the order `↑↓` already walk), each line a state mark that moves, the
  kind's mark, the number, the name, what it used today and, where the list is wide, the last line
  its program printed; beside the list the chosen session's screen, large — a crop of the same
  `vt100` screen, nearly whole. The list answers the question for all of them (every mark is
  live), the preview for the one under the cursor. This is `02-panels.md`'s zoomed shape (the
  list left, the screen right), a line a session as today's grouped list is — kept, because it is
  the shape that holds twenty.

One strip at the top says what Claude Code and OpenCode cost today, how near the plan's limits
are and — across every session, in both layouts — how many there are, how many are working, how
many need you. `⏎` opens the chosen session full — then the pane takes the whole inside with a
one-line tab strip over it, every key the program's, `ctrl+\` back to the zoom.

### 4.2 Mockups

**(1) 120×36, four sessions** (`with_sessions`; 4 ≤ the capacity of 6, §4.5): the grid. The
screens inside the cards are what the fake programs would show and are illustrative — tests run no
program, so their cards show the `starting claude in ~/work/billing …` message of §4.5; the usage
strip is the simulation's numbers and is drawn only when `app.usage.read`:

```
 ☗ cobserve · prod    dashboard › sessions                                            esc back   ● live   15:52:07 WIB  
╭ 8 SESSIONS · 4 ────────────────────────────────────────────────────────────── n new · / find · x x close · esc back ─╮
│ ✻ 287.0M today · ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86                 4 sessions · 1 needs you │
│ n new session · ↻ past conversations · / find one · x x close · \ float it · ⏎ open it full · r rename it            │
│ ╭ ✻ 7 · billing export ────────────────────── ◐ working ╮  ╭ ❯ 9 · make totals ──────────────────────────── ○ idle ╮ │
│ │ ~/work/billing ⎇ main · 2.1M today · claude           │  │ ~/work/reports ⎇ main · terminal                      │ │
│ │ › make the invoice export stream its rows instead of  │  │                                                       │ │
│ │   building one big Vec                                │  │                                                       │ │
│ │                                                       │  │                                                       │ │
│ │ ● I'll read the exporter first.                       │  │                                                       │ │
│ │                                                       │  │                                                       │ │
│ │ ● Read(src/export/invoice.rs)                         │  │ $ make totals                                         │ │
│ │   └ 214 lines                                         │  │ python3 totals.py --month 2026-10                     │ │
│ │                                                       │  │ reading 31 files… done                                │ │
│ │ ● Update(src/export/invoice.rs)                       │  │ writing totals-2026-10.csv (12,418 rows)              │ │
│ │   └ 18 lines added, 31 removed                        │  │ $ wc -l totals-2026-10.csv                            │ │
│ │                                                       │  │ 12419 totals-2026-10.csv                              │ │
│ │ ✻ Running the tests… (esc to interrupt)               │  │ $ █                                                   │ │
│ ╰───────── ⏎ opens it full · \ floats it · r renames it ╯  ╰───────────────────────────────────────────────────────╯ │
│ ╭ ▣ 8 · late partitions ─────────────────── ! needs you ╮  ╭ ▦ 10 · clickhouse3 ────────────────────────── ○ ready ╮ │
│ │ ~/work/pipelines ⎇ fix/late-parts · 0.9M today · open…│  │ SQL on clickhouse3 · read-only · 4 rows last          │ │
│ │ ▣ opencode · ~/work/pipelines                         │  │                                                       │ │
│ │                                                       │  │                                                       │ │
│ │ > find why the 03:00 partitions land late             │  │ SELECT user, sum(memory_usage) AS mem                 │ │
│ │                                                       │  │ FROM system.processes                                 │ │
│ │ I checked the scheduler log: the merge of             │  │ GROUP BY user ORDER BY mem DESC                       │ │
│ │ 2026-10-12 waited on parts_to_delay_insert.           │  │                                                       │ │
│ │                                                       │  │ ✔ 4 rows · 0.3 s · read 12 rows                       │ │
│ │ Permission needed to run:                             │  │ user        mem                                       │ │
│ │   clickhouse-client --query "SELECT …"                │  │ r_redash    37.5 GiB                                  │ │
│ │                                                       │  │ airflow     3.5 GiB                                   │ │
│ │   [a]llow  [d]eny  [e]dit                             │  │ haris       1.2 GiB                                   │ │
│ │                                                       │  │ default     0.9 GiB                                   │ │
│ ╰───────────────────────────────────────────────────────╯  ╰───────────────────────────────────────────────────────╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ a session   ⏎ open it full   \ float it   n new   / find one               9/9 nodes · 1.1 s · read-only 
```

Column-major order: the first column holds sessions 7 and 8, the second 9 and 10, so `↓` is the
card *below* and the next in `Sessions::order()` at once (`↓` from 7 goes to 8; `→` from 7 goes
to 9). The chosen card (7) has the accent frame and the chip title, and its thumbnail is drawn in
the program's own colours; the other thumbnails are drawn with `Modifier::DIM` so the eye lands on
the chosen one. With five or six sessions the cards are 57 × 10 (seven screen rows); with one or
two, 30 rows tall (§4.5).

**(2) 120×36, twenty sessions on seven groups** (20 > 6): the list and the preview. The sessions
were opened in number order (7 the first); `Sessions::groups()` puts the projects in the order
their first session was opened and the lone ones (`make totals`, `weekly notes`) under `other` at
the end — so the numbers are the open order and the lines the grouped one, exactly as `↑↓` walk
them. Illustrative: the names, states, tokens, the chosen session's screen (a Claude Code session
in `~/work/airflow-dags`), the branch names. The twenty fit in the 30 rows without scrolling; the
strip counts every session, not only those on screen:

```
 ☗ cobserve · prod    dashboard › sessions                                            esc back   ● live   15:52:07 WIB  
╭ 8 SESSIONS · 20 ───────────────────────────────────────────────────────────── n new · / find · x x close · esc back ─╮
│ ✻ 287.0M today · ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86    20 sessions · 3 working · 1 needs you │
│ n new session · ↻ past conversations · / find one · x x close · \ float it · ⏎ open it full · r rename it            │
│   billing 3  ⎇ main                     ╭ ✻ 14 · airflow-dq ──────────────────────────────────────────── ◐ working ╮ │
│  ◐ ✻  7 billing export            2.1M  │ ~/work/airflow-dags ⎇ docs-move · 1.9M today · claude                    │ │
│  ◐ ✻ 18 invoice tests             0.8M  │ › the airflow-dq DAG fails its row-count check on the day a refund       │ │
│  ○ ❯ 19 shell                           │   is reversed — make the check idempotent                                │ │
│   pipelines 4  ! 1                      │                                                                          │ │
│  ! ▣  8 late partitions           0.9M  │ ● I'll look at how the check reads the table first.                      │ │
│  ○ ✻ 20 backfill 03:00            1.4M  │                                                                          │ │
│  ○ ✻ 21 dq checks                 312k  │ ● Read(dags/airflow_dq.py)                                               │ │
│  ○ ❯ 22 shell                           │   └ 312 lines                                                            │ │
│   servers 2                             │                                                                          │ │
│  ○ ▦ 10 clickhouse3                     │ ● Grep("INSERT INTO metrics.daily_atomic")                               │ │
│  ○ ▦ 24 clickhouse-bi                   │   └ 2 matches: dags/metrics_atomic.py, sql/daily_atomic.sql              │ │
│   airflow-dags 4  ⎇ docs-move           │                                                                          │ │
│  ○ ▣ 11 airflow-agent             118k  │ ● The load is INSERT … SELECT over the whole day with no partition       │ │
│  ○ ✻ 12 MetricsAtomic             4.2M  │   drop first, so a rerun adds a second copy of every row: the refund     │ │
│ ▌◐ ✻ 14 airflow-dq                1.9M  │   rows are not double counted, they are loaded twice.                    │ │
│  ✕ ✻ 23 docs-move                 0.6M  │                                                                          │ │
│   apitap-lib 3  ⎇ main                  │ ● Update(sql/daily_atomic.sql)                                           │ │
│  ○ ▣ 13 apitap-opencode            87k  │   └ 6 lines added, 1 removed                                             │ │
│  ✕ ✻ 15 apitap-lib-qa             2.3M  │                                                                          │ │
│  ○ ❯ 17 shell                           │ ● Bash(pytest tests/dq -q)                                               │ │
│   cobserve 2  ⎇ zoom-redesign           │   └ 14 passed in 3.2s                                                    │ │
│  ○ ✻ 16 integrate-jira-api        1.2M  │                                                                          │ │
│  ○ ✻ 25 zoom redesign             3.0M  │ ✻ Checking yesterday's partition against the source… (esc to interrupt)  │ │
│   other 2                               │                                                                          │ │
│  ○ ❯  9 make totals                     │ ╭──────────────────────────────────────────────────────────────────────╮ │ │
│  ○ ✻ 26 weekly notes               41k  │ │ ›                                                                    │ │ │
│                                         │ ╰──────────────────────────────────────────────────────────────────────╯ │ │
│                                         │   ? for shortcuts                                  shift+tab accept edits│ │
│                                         ╰──────────────────────────── ⏎ opens it full · \ floats it · r renames it ╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ a session   ⏎ open it full   \ float it   n new   / find one               9/9 nodes · 1.1 s · read-only 
```

A line: the gutter (`▌` on the chosen line, which also carries the selection band), the state
mark (`◐◓◑◒` spinning = output within 3 s, `!` needs you, `○` idle, `↻` kept and not started,
`✕` ended), the kind's mark, the number, the name, the tokens it used today at the right end. A
group's header: the project, how many, `! n` when some need you, `⎇ branch` when every one of
them is on the same branch. The preview is the chosen session's screen — 74 of its 116 columns,
the bottom 27 of its 31 rows — in the program's own colours, with the card's info row over it and
the keys that act on it in the bottom border.

**(3) 120×36, fifty sessions** (twelve projects of four and two lone ones: 63 lines in 30 rows):
the list scrolls under the strip as a **window that follows the chosen line**, an end line at each
end saying how many *sessions* are past it. When the window starts inside a group, the `↑` line
carries that group's header at its right end, so the lines under it keep their project. The
cursor is on 33, which needs you, so the preview shows what it is asking. With fifty open, the
hint line's first words become `50 of 50 · x x closes one` and the frame's right end loses
`n new` (as today's list says it):

```
 ☗ cobserve · prod    dashboard › sessions                                            esc back   ● live   15:52:07 WIB  
╭ 8 SESSIONS · 50 ───────────────────────────────────────────────────────────────────── x x close · / find · esc back ─╮
│ ✻ 287.0M today · ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86    50 sessions · 7 working · 2 need you  │
│ 50 of 50 · x x closes one · ↻ past conversations · / find one · \ float it · ⏎ open it full · r rename it            │
│   ↑ 4 more          pipelines 4  ! 1    ╭ ✻ 33 · FX exposure ───────────────────────────────────────── ! needs you ╮ │
│  ! ▣ 11 late partitions           0.9M  │ ~/work/redash-queries · 533k today · claude                              │ │
│  ○ ✻ 12 backfill 03:00            1.4M  │ › the FX exposure query (#7711) takes 15 GiB on clickhouse2 — can the    │ │
│  ○ ✻ 13 dq checks                 312k  │   intraday rollup be read instead of the raw transfers?                  │ │
│  ○ ❯ 14 shell                           │                                                                          │ │
│   airflow-dags 4  ⎇ docs-move           │ ● I'll compare the two tables' columns first.                            │ │
│  ○ ▣ 15 airflow-agent             118k  │                                                                          │ │
│  ○ ✻ 16 MetricsAtomic             4.2M  │ ● Read(queries/fx_exposure.sql)                                          │ │
│  ◐ ✻ 17 airflow-dq                1.9M  │   └ 88 lines                                                             │ │
│  ✕ ✻ 18 docs-move                 0.6M  │                                                                          │ │
│   apitap-lib 4  ⎇ main                  │ ● Bash(clickhouse-client --query "DESCRIBE final.fx_exposure_intraday")  │ │
│  ○ ▣ 19 apitap-opencode            87k  │                                                                          │ │
│  ✕ ✻ 20 apitap-lib-qa             2.3M  │   Claude needs your permission to run this command                       │ │
│  ○ ✻ 21 retry budget              640k  │                                                                          │ │
│  ○ ❯ 22 shell                           │   clickhouse-client --query "DESCRIBE final.fx_exposure_intraday"        │ │
│   cobserve 4  ⎇ zoom-redesign           │                                                                          │ │
│  ○ ✻ 23 integrate-jira-api        1.2M  │   Do you want to proceed?                                                │ │
│  ◐ ✻ 24 zoom redesign             3.0M  │   ❯ 1. Yes                                                               │ │
│  ○ ✻ 25 local rig                 905k  │     2. Yes, and don't ask again for clickhouse-client commands in        │ │
│  ○ ❯ 26 shell                           │        ~/work/redash-queries                                             │ │
│   fleetlens 4  ⎇ main                   │     3. No, and tell Claude what to do differently (esc)                  │ │
│  ○ ✻ 27 fleet metrics port        1.6M  │                                                                          │ │
│  ○ ▣ 28 live queries              402k  │                                                                          │ │
│  ✕ ✻ 29 tests                      77k  │                                                                          │ │
│  ○ ❯ 30 shell                           │                                                                          │ │
│   redash-queries 4  ! 1                 │ ╭──────────────────────────────────────────────────────────────────────╮ │ │
│  ◐ ✻ 31 AML dashboard             2.7M  │ │ ›                                                                    │ │ │
│  ○ ✻ 32 July close pack           1.1M  │ ╰──────────────────────────────────────────────────────────────────────╯ │ │
│ ▌! ✻ 33 FX exposure               533k  │   ? for shortcuts                                  shift+tab accept edits│ │
│   ↓ 23 more                             ╰──────────────────────────── ⏎ opens it full · \ floats it · r renames it ╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ a session   ⏎ open it full   \ float it   n new   / find one               9/9 nodes · 1.1 s · read-only 
```

*Why a window and not folded projects:* folding would hide lines that `↓` must still reach
(`Sessions::order()` is unchanged, so the cursor would land on a hidden line or the walk would
skip sessions), it is state the person must manage (which projects are folded) that no poll can
set, and the cut-off groups would lose their live marks — the one thing the list is for. The
window is what today's grouped list does, and what `fifty_sessions_fit_the_list…` tests; `←→`
(§4.6) jump a group at a time, so fifty is twelve presses, and `/` reaches a far one by name.

**(4) 80×24, twenty sessions**: the preview needs 48 cells beside a list of 32 and the inside is
76, so the preview goes and **the list is the zoom** — wide enough for the last line each program
printed (faint), which is the preview's job done a line at a time. The usage strip keeps Claude
only, then loses the plan's limits before the counts; the list has 18 rows for 27 lines, so it is
the window again. The cursor is on 13 here, so both end lines show:

```
 ☗ cobserve · prod    dashboard › sessions    esc back   ● live   15:52:07 WIB  
╭ 8 SESSIONS · 20 ───────────────────────────────────────── / find · esc back ─╮
│ ✻ 287.0M today · ≈ $191.17             20 sessions · 3 working · 1 needs you │
│ n new · ↻ resume · / find · x x close · \ float · ⏎ full · r rename          │
│   ↑ 2 more                                               billing 3  ⎇ main   │
│  ○ ❯ 19 shell                    $                                           │
│   pipelines 4  ! 1                                                           │
│  ! ▣  8 late partitions          [a]llow  [d]eny  [e]dit                0.9M │
│  ○ ✻ 20 backfill 03:00           ● The backfill needs the 02:00 part…   1.4M │
│  ○ ✻ 21 dq checks                ›                                      312k │
│  ○ ❯ 22 shell                    $ airflow dags list | grep late             │
│   servers 2                                                                  │
│  ○ ▦ 10 clickhouse3              ✔ 4 rows · 0.3 s                            │
│  ○ ▦ 24 clickhouse-bi            read-only SQL                               │
│   airflow-dags 4  ⎇ docs-move                                                │
│  ○ ▣ 11 airflow-agent            >                                      118k │
│  ○ ✻ 12 MetricsAtomic            ● Done. The daily table is idempote…   4.2M │
│  ◐ ✻ 14 airflow-dq               ● Bash(pytest tests/dq -q)             1.9M │
│  ✕ ✻ 23 docs-move                claude — it exited                     0.6M │
│   apitap-lib 3  ⎇ main                                                       │
│ ▌○ ▣ 13 apitap-opencode          Written src/client.rs (41 lines)        87k │
│   ↓ 6 more                                                                   │
╰──────────────────────────────────────────────────────────────────────────────╯
 esc back   ↑↓ a session   ⏎ open it full   \ float it    9/9 nodes · read-only 
```

Four sessions at 80×24 are still the grid (four cards of 37 × 9, six screen rows each — the
80×24 mockup of the previous draft holds, with the strip and hint lines of this one), because
the capacity there is four (§4.5).

**(5) The finder, 120×36, twenty sessions** (`/`, then `shell`): the hint line is the search
field, both layouts show only the matching sessions, and the list is **flat, in `matching()`'s
order** (the number order, which is what `↑↓` step through while finding — `App::on_finding_key`
moves `at` over `matching()`), each line keeping its project at the right end so the grouping is
not lost; the preview follows the match under the cursor. The layout itself does not change while
finding (it is chosen by the total, §4.5), so typing never reflows the screen:

```
 ☗ cobserve · prod    dashboard › sessions                                            esc back   ● live   15:52:07 WIB  
╭ 8 SESSIONS · 20 ───────────────────────────────────────────────────────────── n new · / find · x x close · esc back ─╮
│ ✻ 287.0M today · ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86    20 sessions · 3 working · 1 needs you │
│ ⌕ /shell▏  1 of 3 · ↑↓ choose · ⏎ goes there · esc back                                                              │
│ ▌○ ❯ 17 shell               apitap-lib  ╭ ❯ 17 · shell ──────────────────────────────────────────────────── ○ idle ╮ │
│  ○ ❯ 19 shell                  billing  │ ~/code/apitap-lib ⎇ main · terminal                                      │ │
│  ○ ❯ 22 shell                pipelines  │                                                                          │ │
│                                         │                                                                          │ │
│          …                              │          …                                                               │ │
│                                         │ $ cargo test -q                                                          │ │
│                                         │                                                                          │ │
│                                         │ running 212 tests                                                        │ │
│                                         │ ............................................................             │ │
│                                         │ ............................................................             │ │
│                                         │ ............................................................             │ │
│                                         │ ................................                                         │ │
│                                         │ test result: ok. 212 passed; 0 failed; 0 ignored; 0 measured             │ │
│                                         │                                                                          │ │
│                                         │ $ █                                                                      │ │
│                                         ╰─────────────────────────────────────────── ⏎ opens it full · \ floats it ╯ │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 ↑↓ choose   ⏎ go there   esc back                                                        9/9 nodes · 1.1 s · read-only 
```

(The two `…` rows stand for the sixteen empty rows cut from the frame here; the generated
`mock-sessions_120_finding.txt` is the full 36.)

**(6) 160×48** (inside 156 × 44): three columns, so the grid holds **nine** — cards of 50 × 14
with eleven screen rows (two of 77 × 42 when there are two, three of 50 × 42 for three, 50 × 21
for four to six); from ten sessions the list is 70 cells wide, so it carries the last printed
line of every session as the 80×24 list does, and the preview is 84 × 42 — 82 columns and 39 rows
of the session's 156 × 43 screen, which is nearly all of it. The strip has room for everything.

### 4.3 What is dynamic

| Element | Changes | Computed |
|---|---|---|
| thumbnails, the preview | per `Event::Pane` of that session (the same redraw as today's pane), per console event | `session.pane.screen()` rows; `console::draw` for a query session |
| `◐ working` / the `◐` mark | per tick (spinner) while output arrived within 3 s | new `ClaudePane::last_output: Option<SystemTime>`, set by `App::update` on `Event::Pane` from `app.clock` (so the tests' clock drives it); working = `clock − last_output < 3 s` |
| `○ idle` | per tick | running, no output for 3 s |
| `! needs you` | per event | `pane.attention` (a bell while not on screen), as today |
| `↻ kept`, `◌ starting`, `✕ ended` | per event | `Session::waiting()`; `PaneState::Starting`; `PaneState::Exited/Failed` |
| the last printed line (list ≥ 60 cells) | per `Event::Pane` | the last row with content at or above the cursor, trimmed (`console_line` for a query session) |
| `2.1M today` on a card or a line | per usage read | `usage::by_session` for the session's conversation id, as today's list |
| usage strip | per usage / limits read | `usage::sidebar_lines`, re-flowed into one line |
| `20 sessions · 3 working · 1 needs you` | per tick | `claude.list.len()`, the sessions whose state is working, those with `pane.attention`; a zero count is left out, `need you` from two |
| the window's `↑ N more` / `↓ M more` | per key, per open/close | `scroll_into_view` over the list's lines from `claude.list_scroll` |

### 4.4 Colour

| Element | Colour | Rule |
|---|---|---|
| a card's frame | accent for the chosen; amber when `pane.attention`; border otherwise | cursor / warning |
| the chosen line | `▌` accent in the gutter, `theme.selected()` band across the list (`Painter::band`); its name `theme.strong()` | structure, as every list of §0.2 |
| the preview's frame and title | accent frame, chip title (`theme.tab_active()`) — it is the chosen one's card | cursor |
| `✻` and Claude's name | `theme.claude` | its own mark, as today |
| `▣ ❯ ▦` | accent (`panels::sessions::kind_style`) | structure |
| `◐ working` and the spinning `◐◓◑◒` mark of a session | green (`Severity::Ok`) | good news, as the dashboard's `● running` and §0.4's `◐ working` — green is not a severity (`03-glyphs.md`: red and amber are), so §0.3's rule that a spinner never takes a severity colour holds; a *job's* spinner in the queue is a fact and stays in the text colour |
| `! needs you`, the `!` mark, a header's `! 2`, `2 need you` in the strip | amber bold | warning |
| `3 working` in the strip | green | good news |
| `○ idle`, `○ ready`, `◌ starting`, `✕ ended` and their marks, `↑ N more` | muted | neutral |
| `↻ kept` and its mark | accent | structure, as today's list |
| a group's header | the project `theme.section()`, the count and branch `theme.faint()` | structure, as the rules of §0.2 |
| a line's number, tokens today, the last printed line | muted / faint / faint | neutral |
| plan `session 11% · week 70%` | `severity::node(percent)`: amber ≥ 75, red ≥ 90 | §7, as `usage.rs` colours the limits |
| thumbnail and preview text | the program's own colours (chosen / preview); the same with DIM (the other cards); a program that ended: DIM, as `screen(…, live=false)` | not a severity: the program's |
| ghost cards | faint | neutral |

### 4.5 Layout

Inside = 116 × 32 at 120×36, 76 × 20 at 80×24, 156 × 44 at 160×48. Rows 0 and 1 are the same
in both layouts, so switching moves nothing at the top:

1. **Usage strip** (row 0): `usage::sidebar_lines` re-flowed into one line — `✻ 287.0M today ·
   ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86` (each program only when it has
   replies today; the plan's two limits via `usage::limit_glance`, else `5h 1h26m left`) — and at
   the right end the counts across every session: `N sessions · W working · A need you` (a zero
   count left out). What goes when the width is short, in this order: OpenCode's half, the plan's
   limits, the money; the counts never, with at least one cell before them. Without `usage.read`
   the strip is the counts alone.
2. **Hint line** (row 1, muted): `n new session · ↻ past conversations · / find one · x x close ·
   \ float it · ⏎ open it full · r rename it` (at 80: `n new · ↻ resume · / find · x x close · \
   float · ⏎ full · r rename`); with `MAX_SESSIONS` open its first words are `50 of 50 · x x
   closes one`. The words `n new session` and `↻ past conversations` are `Hit::NewSession` and
   `Hit::Resume` in both layouts. While finding (`Mode::Finding`) this row is the search field:
   `⌕ /ject07▏  1 of 1 · ↑↓ choose · ⏎ goes there · esc back` (`none · no session is called
   that`). Dropped when the inside is under 16 rows.
3. **The body** (the rest, `body_h = height − 2`): the grid or the list, by the rule below.

**The rule.** The grid's capacity at a size is the number of cards it can draw at a readable
size — at least 36 cells wide and 9 rows tall (a title, an info row, six screen rows, a bottom):

```
cols(w)       = 1 if w < 76, else clamp(w / 48, 2, 4)        two columns from 76, three from 144, four from 192
rows_max(h)   = clamp(body_h / 9, 1, 3)
cap(w, h)     = cols × rows_max, or 0 when w < 36 or body_h < 9 (no readable card at all)
layout        = Grid if n ≤ cap, else List        n = claude.list.len() — every session, ended ones included
```

| terminal | inside | cols × rows_max | **cap** | cards when few | list / preview when many |
|---|---|---|---|---|---|
| 80×24 | 76 × 20 | 2 × 2 | **4** | 37 × 9 | 76 / — (the list is the zoom) |
| 100×30 | 96 × 26 | 2 × 2 | **4** | 47 × 12 | 32 / 62 |
| 120×36 | 116 × 32 | 2 × 3 | **6** | 57 × 10 (5–6) · 57 × 15 (3–4) · 57 × 30 (2) · 116 × 30 (1) | 38 / 76 |
| 140×40 | 136 × 36 | 2 × 3 | **6** | 67 × 11 | 58 / 76 |
| 160×48 | 156 × 44 | 3 × 3 | **9** | 50 × 14 | 70 / 84 |
| 200×60 | 196 × 56 | 4 × 3 | **12** | 47 × 18 | 70 / 124 |

The switch has **no hysteresis**: the layout is a pure function of `(n, inside)`, so the same
sessions at the same size always draw the same screen, and a test can say which. It cannot
flicker: `n` changes only by the person's hand (`n`, a picked conversation, `x x` — a program
that ends stays in the list as `✕ ended` until it is closed), never by a poll or a tick; the size
only by a resize; the finder does not change `n` (it filters what is shown, not what is counted).
A session opened at `n = cap` turns the grid into the list and closing it turns it back — once
each, on the key that did it. On a resize the next frame re-evaluates `cap`; a switch keeps
`claude.active` and the list window is recomputed from `claude.list_scroll` by `scroll_into_view`,
so the chosen session stays on screen; nothing is sent to the PTYs (below). There is no key to
force a layout: the count decides, so the screen is predictable.

**The grid** (`n ≤ cap`): `cols = min(cols(w), max(n, 1))` (one session takes the whole width,
two take two columns), `card_w = (w − 2 × (cols − 1)) / cols`, `rows = clamp(ceil(n / cols), 1,
rows_max)`, `card_h = body_h / rows`; screen rows = `card_h − 3`. Cells are filled column-major
from `Sessions::order()`; a free cell of the last row holds a **ghost card** (`theme.faint()`
frame, `+ new session` title, `n · claude, opencode, a terminal or a query session` inside, a click
on it is `Hit::NewSession`), the next free one `↻ past conversations` (`Hit::Resume`). Zero sessions
are the two ghosts alone (§0.5's empty state). A card: title ` mark number · label `, right ` state `;
inner row 1 = `dir ⎇ branch · tokens today · kind` (`views/claude.rs::place` for the path, cut
from the left); rows 2.. = the thumbnail (below). The chosen card's bottom border says `⏎ opens
it full · \ floats it · r renames it`.

**The list** (`n > cap`): the body is the list on the left and the preview on the right.

- Widths: `list_w = clamp(w − 78, 32, 70)` and `preview_w = w − list_w − 2`, so the preview keeps
  76 cells wherever there is room and the list grows from 32 to 70; the preview is drawn only when
  `preview_w ≥ 48` (from an inside of 82 cells — 86 columns), else the list takes the whole width.
  From `list_w ≥ 60` each line carries the last line its program printed (faint), the name cut to
  24; narrower, the name has the room and the tokens stand alone at the right end.
- Lines, in `Sessions::groups()`' order — which *is* `Sessions::order()` (`order()` is
  `groups()` flattened), so `↑↓` are the line above and below: a header a group (`  project n  ! a
  ⎇ branch`: the branch only when all its sessions share one, `! a` only when some need you), then a
  line a session: gutter (`▌` on the chosen), the state mark, the kind's mark, the number
  (right-aligned in 2), the name, [the last printed line], the tokens today. No blank line between
  groups (the header separates) and no foot lines (the hint line has `n new session` and `↻ past
  conversations`), so twenty sessions on up to ten groups — the most twenty can make — are 30 lines
  and fit at 120×36 without a window.
- The window: when the lines outnumber the rows, `window = rows − (end lines needed)` and `first =
  scroll_into_view(claude.list_scroll, chosen line, window, lines)` (`claude.list_scroll` kept by
  the draw, as today), `↑ N more` above and `↓ M more` below counting sessions, not lines; when
  the first shown line is not a header, the `↑` line ends with the cut group's header. The chosen
  line is never off screen.
- While finding: the lines are the matching sessions in `matching()`'s order, flat, each with its
  project at the right end in place of the tokens, the `at`-th carrying the cursor; the preview
  follows it. While naming (`Mode::Naming`) the chosen line's name is `name▏` on `theme.keycap()`
  (the chosen card's title, in the grid). After one `x` (`claude.closing`) the preview's info row —
  or, without a preview, the chosen line's tail — says `x again closes it` in amber bold.
- The preview: a card `preview_w × body_h`, accent frame, chip title ` mark number · label ` with
  the state at the right, inner row 1 the info row of a card, rows 2.. the thumbnail of the chosen
  session (live colours), the bottom border `⏎ opens it full · \ floats it · r renames it`.

**A thumbnail** (a card's body, the preview's): the pane's screen rows `[last + 1 − n ..= last]`
where `last` is the later of the cursor's row and the last row with content, columns `0..inner_w`
— a crop, cut at the edge without `…` (so a shell shows its prompt, Claude Code the bottom of its
screen with its prompt box). Chosen / preview: the program's own colours (`screen_rows(…, live,
dim = false)`); the other cards: `Modifier::DIM`; a program that ended: DIM and, in the chosen
card or the preview, the bottom row's band `claude — it exited · ⏎ start it again` of today's
`pane()`. A query session's thumbnail is `console::draw` into the rect, read-only (the hits the
console registers inside it are dropped; the whole card is `Hit::Session(i)`). Idle, starting and
failed panes show the message lines of today's `pane()` (`starting claude in ~/work/billing …`,
the how-to-get text), wrapped to the card.

**Full mode** (`shell.sessions_full`): row 0 the session bar (today's `session_bar`, tabs round
the chosen one), rows 1.. the pane (`claude::pane`) edge to edge; the picker (`Mode::Opening`)
also takes the whole inside, exactly as today (`picker()` unchanged).

**PTY sizing**: `claude.want_size` is set from the **full-mode pane size** (`inside.height − 1`,
`inside.width`) in every layout, so a thumbnail and the preview are crops of the real screen and
opening a session full reflows nothing; switching layouts sends nothing to the PTYs. (Today
`pane()` sets it from whatever rect it draws into; the grid and the preview must not.)

**Hit regions**: `Hit::Session(i)` on each card and on each session line (a header's line is its
first session's; the `↑`/`↓` lines are the first hidden session past them, as today); the preview
is `Hit::Session(active)`; `Hit::NewSession` / `Hit::Resume` on the ghost cards and on the hint
line's words; `Hit::Pane` only in full mode. A click chooses; a click on the chosen one (its card,
its line, the preview) opens it full. The wheel over the zoom steps the chosen session (`↑↓`), the
list following it.

**Drops as it shrinks**: the strip's OpenCode half, then the plan's limits, then the money (width);
the hint line (height < 16); a card's info row (card_h < 8); ghost cards (no free cell); the
preview (`preview_w < 48`); the last printed line (list < 60). Under `MIN_W × MIN_H` the shell
draws nothing anyway.

### 4.6 Keys and selection

Unchanged: `↑↓ j k` (`nav::choose` → `claude.step` over `order()`, round the ends as today),
`Home End` (the first / last of `order()`), `⏎` open full (`sessions::open_full`), `\` float, `n`
new (the picker), `/` find, `x x` close, `ctrl+\` back from full, the prefix keys in full mode as
today, `esc` back to the grid. In both layouts the keys walk `Sessions::order()`; the grouped list
is that order drawn, the grid that order column-major.

Changed or added, all in `Scope::Zoomed(PanelId::Sessions)` (a scoped action wins over an
`Everywhere` one on the same key, `action::for_key`):

- `←→ h l` — `sessions-choose` gains `"Left", "Right", "h", "l"` in its `keys` (no conflict: no
  other `Zoomed(Sessions)` action lists them; the global `choose` already routes them to
  `nav::choose`, which ignores them for Sessions today). In the grid: the card beside, `order[at ∓
  rows]` clamped. In the list: **the previous / next group's first session** — the one motion that
  reads the structure the list draws, so fifty sessions on twelve projects are twelve presses;
  nothing at the ends, nothing while finding (the list is flat) or with a single group. `shown`
  stays `↑↓` (the status bar reads `↑↓ a session`; `?` lists `←→` too).
- `PgUp PgDn` — a page, not one (today they step one): `order[at ∓ page]` clamped, where `page`
  is the window's session lines in the list and `cols × rows` in the grid.
- `r` — new `sessions-rename` (rank 7): `claude.mode = Mode::Naming(the current name or "")`,
  drawn on the chosen line or card title, `⏎` keeps, `esc` cancels, as the float's `ctrl+\ r` does
  today. It shadows the global `refresh` in this zoom only, exactly as `airflow-read` and
  `jira-read` shadow it in theirs; `:read now` stays on the palette.

`nav::choose` for `PanelId::Sessions` reads a new `shell.sessions_shape: Cell<SessionsShape>`
(`column: u16` — the rows of a grid column, 0 in the list — and `page: u16`) that the draw sets,
like `zoom_tiles_per_row`; the group jump uses `Sessions::groups()`.

### 4.7 Implementation notes

- `src/ui2/views/claude.rs`: `sidebar`, `grouped_rows`, `card_row`, `GROUPED_FROM`, `Density`
  and `mark` go; `pane`, `screen`, `style_of`, `picker`, `rows_and_note`, `pick_line`,
  `session_bar`, `message`, `how_to_get`, `scrolled_note`, `console_line`, `place`,
  `shorten_left` stay. New `pub fn draw(frame, app, theme, inside)` branches: picker →
  `picker(...)`; `sessions_full` → bar + `pane`; else `strip` + `hints` + (`grid(...)` or
  `list(...)` + `preview(...)`) by `shape`.
- New helpers: `pub fn shape(width: u16, height: u16, n: usize) -> Shape` (`enum Shape { Grid
  { cols, rows, card_w, card_h }, List { list_w, preview_w: Option<u16> } }`) and `pub fn cap(width,
  height) -> usize` — pure, unit-tested against the table of §4.5; `thumbnail(frame, pane, theme,
  r, live, dim)` (a crop — refactor today's `screen()` into `screen_rows(pane, rows: Range, cols:
  Range, live, dim) -> Vec<Line>`); `last_line(pane) -> Option<String>`; `pub fn state_word(s:
  &Session, clock) -> (word, mark, Severity, spinning)` (replaces `panels::sessions::state` and is
  used by the dashboard panel too, so both say `◐ working`); `pub fn usage_strip(app, theme, width,
  counts) -> Line`; `group_lines(app, theme, list_w, tails) -> Vec<(Line, Option<usize>)>` and the
  window over them (today's `grouped_rows` logic, kept).
- `ClaudePane` gains `pub last_output: Option<SystemTime>`; `App::update` sets it on `Event::Pane`
  next to `feed()` from `self.clock` (the one `app.rs` line); `claude.rs` is otherwise untouched
  (the house rule: the sessions' running code does not change).
- `src/ui2/state.rs`: `sessions_shape: Cell<SessionsShape>`; `src/ui2/nav.rs`: the Sessions arm
  of `choose` as §4.6; `src/ui2/mouse.rs`: in the sessions zoom (not full) the wheel is
  `nav::choose` `Up`/`Down` and a left click on `Hit::Session(i)` with `i == claude.active` is
  `sessions::open_full`, before the pass-through; `src/action.rs`: `sessions-choose.keys +=
  ["Left", "Right", "h", "l"]`, new `sessions-rename`.
- `panels::sessions::state` → delegates to `state_word` with the clock, so the dashboard row reads
  `◐ working` / `○ idle` / `! needs you` / `✕ ended` consistently (the panel's test strings
  `● working` in `02-panels.md` become `◐ working`; the dashboard test `the_sessions_panel_and_zoom`
  asserts glyph and number only, so it holds).
- Tests to change (`views/tests.rs`; `sessions()` opens one session, `app_with_claude` one at
  140×40 — those sizes are the grid, 50 sessions at 140×40 the list, cap 6):
  `sessions_are_a_list_beside_claude_and_can_be_renamed` — `row("✻  cobserve").contains("● 7")` →
  the card title `✻ 7 · cobserve` and its right end `! needs you`; the `'▎'` lit checks → assert
  `app.claude.active` and that the chosen card's title cell carries `theme.tab_active()` (the
  TestBackend buffer keeps styles); `"the second session"` visible and `"the first session"` not →
  both thumbnails are visible now: assert both; `"+  new session"` → the ghost card `+ new session`
  and the hint line `n new session`; `infra▏`, `keep the name`, the toast `x again closes the
  session` hold.
  `opencode_and_a_terminal_sit_in_the_list_beside_claude` — `row("▣  notes").contains('8')` →
  `▣ 8 · notes` (and `❯ 9 · scripts`, `✻ 7 · cobserve`); the failed pane's card says `✕ ended`
  with the reason in its thumbnail; the `opencode auth login` / `OPENCODE_CMD` assertions move after
  `⏎` (a 50-cell card wraps that sentence; the full pane does not); the picker assertions hold.
  `a_narrow_terminal_puts_the_sessions_on_a_bar` (96×30): the bar exists only in full mode now →
  press `⏎` first, then assert the bar; the `!SESSIONS` assertion goes.
  `claude_not_installed_says_how_to_get_it`, `a_claude_that_ended_says_so_and_offers_to_start_again`
  (one session at 120×36: one card of 116 × 30): hold — the message and the exit band are in the
  thumbnail.
  `sessions_of_the_last_run_wait_marked_and_a_conversation_can_be_taken_up` — `row("▣  pipelines")
  .contains("↻ 8")` → the card `▣ 8 · pipelines` with `↻ kept` at its right end; the started one's
  card says `◌ starting`; `"↻  past conversations"` → `↻ past conversations` (the hint line);
  `click_on(Hit::Resume)` holds.
  `a_shell_scrolled_back_says_how_far` — `Hit::Pane` exists only in full mode: press `⏎` before the
  wheel; the rest holds.
  `five_sessions_open_still_leave_room_for_a_new_one` — `"+  new session"` → `+ new session` (the
  one free cell's ghost); `past conversations` (the hint line) and `click_on(Hit::NewSession)` hold.
  `fifty_sessions_fit_the_list…` (the list + preview; fifty lone folders make one `other` group,
  51 lines): `SESSIONS  50` → the strip's `50 sessions`; `▣  project49` lit (`▎`) → the line
  `▣ 56 project49` carries `▌`; `↑ … more` holds (the window's top end line); `!contains("project01")`
  holds; `50 of 50 · x closes one` → the hint line's `50 of 50 · x x closes one`; after 30 × `↑`
  the line `project19` carries `▌` and `↓ … more` is there; `/ject07▏` and `1 of 1` → the finder
  row (`row("⌕ /")`), `project07` the line with `▌`; `⏎` → active 7 holds (engine); `‹ 40 ›` on the
  narrow bar → full mode only: `⏎` first.
  `many_sessions_sit_by_project_a_line_each…` — render at **120×36** (eight sessions are a grid at
  160×48, cap 9, and the list at 120×36, cap 6): `SESSIONS  …8` → the strip's `8 sessions · 1
  needs you`; `⌕ find one  ctrl+\ /` → the hint line's `/ find one`; `airflow-dags 3` header
  contains `! 1  ⎇ docs-move`, `apitap-lib 3` contains `⎇ main`, `other 2` exists; the lit line
  `integrate-jira-api` carries `▌`, and the preview's info row contains `~/code/cobserve ⎇ main`
  (the `lines[at + 1]` assertion goes); `dq − agent == 2` holds; the `↓` order assertions hold
  (`order()` is unchanged); the sizes loop holds.
  `the_session_list_says_what_each_session_used_today` — `✻ 4.2M today` and `▣ 51k today` hold
  (the strip is `sidebar_lines` re-flowed); `integrate-jira-api` row contains `1.2M ` → the card's
  info row `row("~/work/cobserve").contains("1.2M today")`.
  `the_claude_plan_s_limits_show_where_its_usage_does` — `list.contains("✻ 2.9M today · ≈ $3.68 ·
  session")` holds (strip).
  `a_query_session_is_opened_on_a_server_chosen_from_the_fleet` — `● clickhouse-bi ▾` holds (the
  query session's card is `console::draw`).
  New: `twenty_sessions_are_a_list_with_a_preview_and_six_a_grid` — `with_many_sessions(app, 20)`
  (the seven-group set of mockup 2 as a fixture: dirs, kinds, names, branches; no program runs) at
  120×36 asserts the list (every name on screen, no `↑`/`↓` line, the headers' text, the strip's
  counts, `▌` on the chosen line, the preview's title `✻ 14 · airflow-dq`), `←` / `→` land on the
  previous / next group's first session, `PgDn` moves a page, `r` then `⏎` renames; closing down to
  six (`x x`) draws the grid, opening the seventh the list again — the switch, both ways, on the
  key alone; at 80×24 no preview and every line carries its tail; `shape()` and `cap()` against the
  table of §4.5.
- `ui2/tests.rs::the_sessions_panel_and_zoom`: regenerate `120x36-zoom-sessions.txt` with
  `with_sessions` (the dump test needs `SESSIONS=4` to call `with_sessions`, `SESSIONS=20` / `50`
  `with_many_sessions` — add that env switch to `dump`); the title cells, the six hints, `billing
  export`, `↓ → active 1`, `⏎ → sessions_full`, `ctrl+\` back, `esc` hold. `every_size_renders_
  without_a_panic`: rounds with 0, 4, 20 and 50 sessions, so both layouts, the window and the
  no-preview form are fuzzed. `export_screens`: `120x36-zoom-sessions-list` and
  `80x24-zoom-sessions-list` (twenty sessions) beside the grid's.

---

## 5 · Cross-cutting changes

### 5.1 `views::draw`

```rust
pub fn draw(frame, app, theme, zoom: PanelId, inside: Rect) {
    match zoom {
        PanelId::Queue    => queue::draw(frame, app, theme, inside),          // detail card inside
        PanelId::Airflow  => match app.page() { Some(p) => detail::draw(.., body_with_drawer(inside), p), None => airflow::draw(frame, app, theme, inside) },
        PanelId::Jira     => match (app.page(), app.jira_time) { (Some(p), _) => detail::draw(..), (None, Some(t)) => jira::draw_time(frame, app, theme, inside, t), _ => board::draw(frame, app, theme, inside) },
        PanelId::Sessions => claude::draw(frame, app, theme, inside),
        PanelId::Tape     => { tape::draw(.., body); drawer::draw(.., drawer) }   // unchanged
        _ => {}
    }
}
```

Pages (`detail::draw`) and the month's time keep the 4-line drawer under them (`drawer::page`,
`drawer::time`): they are unchanged and their drawer is their detail. Only the four zoom bodies
lose it.

### 5.2 Screens, docs, tests

- Regenerate `docs/redesign/screens/{80x24,120x36,160x48}-zoom-{queue,airflow,jira,sessions}.txt`
  from the new renders (`dump` with `SIZE`, `KEYS`, and `SESSIONS=4`), add
  `{80x24,120x36}-zoom-sessions-list.txt` (twenty sessions: the list, with and without the
  preview — `SESSIONS=20`), and `docs/screenshots/*`
  via `dev/screenshots.sh`. The simulation (`docs/research/simulasi-konsep-ui.html`) is not
  updated; `08-simulation.md` gets a line saying the four zooms left it.
- `docs/redesign/02-panels.md`: replace the *Zoomed* paragraphs of panels 2, 5, 6, 8 with one
  sentence each pointing at this spec's regions (panel 8's says: a grid of thumbnails up to the
  size's capacity, §4.5, the grouped list beside a big preview past it; and `● working` →
  `◐ working` in its dashboard list). `06-build-order.md`: a *Phase 7 — the zooms* entry listing the four acceptance
  screens.
- `src/ui2/tests.rs::dump`: honour `SESSIONS=4` (calls `with_sessions`) and `SESSIONS=20` / `50`
  (`with_many_sessions`, the fixture of §4.7) so a sessions frame of either layout can be dumped.
- The resize fuzz (`five_hundred_sizes…` in `layout.rs` is the grid's; the zooms' is in
  `ui2/tests.rs` — extend its size loop to render each of the four zooms with the cursor on the
  first, the last and a middle row, 20..250 × 8..80 — the sessions zoom with 0, 4, 20 and 50
  sessions, so the grid, the list, its window and the no-preview form are all hit).
- `action.rs`: `sessions-choose.keys += ["Left", "Right", "h", "l"]` and a new `sessions-rename`
  on `r` in `Zoomed(Sessions)` (§4.6); the uniqueness test holds (no other `Zoomed(Sessions)`
  action lists them; `r` is in that scope once, as it is in Airflow's and Jira's).
- `src/ui2/state.rs`: `sessions_shape`; `src/ui2/nav.rs` and `src/ui2/mouse.rs`: the sessions
  arms of §4.6 and §4.5 (hit regions).

### 5.3 Build order (small steps, each leaving `cargo test` green)

1. Shared: `glyph::spinner`, `widgets::freshness_style`, `frame::card` (a thin wrapper over
   `rounded` + title + right note + bottom note), `views::draw` dispatch with a per-zoom
   `drawer` flag — ship with the old bodies.
2. QUEUE (hero, list, detail card) + its screens and tests.
3. JIRA (lanes everywhere, detail card) — the smallest change, mostly `board.rs`.
4. AIRFLOW (cards, loom, Gantt with the task cache) — the `App::peek`/cache is the one engine
   touch; land it with a test that a cursor on a run row produces one `Ask::Tasks` and that the
   answer lands in the map without a page.
5. SESSIONS — in two steps: the grid for few (`shape`/`cap`, thumbnails, `last_output`,
   `state_word`, `←→`), then the list and the preview for many (`group_lines`, the window, `PgUp
   PgDn`, the group jump, `r`), each with its screens and tests; the dashboard panel's words with
   the first.
6. Docs and the `02-panels.md` paragraphs.

---

## 6 · Built: where the code went beyond or beside this text

- **QUEUE.** The job card is 6 rows from 20 inside rows, 9 from 30, 12 from 40. From 152 cells of
  hero a fourth card graphs the oldest wait (`history.queue_oldest`, fixed 5-minute scale,
  amber at 1 min, red at 3). The ages and the oldest wait tick for a minute past the read
  (`live_age`), and the cards' oldest wait does too, so the two agree.
- **AIRFLOW.** With a DAG's thread under the cursor the Gantt's place holds that DAG: its runs of
  the day, the last one's end and length, the next, its description and tags. An unhealthy DAG
  processor or metadatabase, which the zoom's title does not carry, is said on the day line. The
  Gantt's axis labels months on a run that has been going for months.
- **JIRA.** A block that is overdue or due soon carries a `▎` down its left edge in that severity
  (the cursor's `▌` wins). The ticket card's third line is the link alone: its keys are in its
  title.
- **SESSIONS.** With the list and the preview on screen, `⏎` opens the chosen session *in the
  preview*: its program takes every key (its PTY sized to the preview) while the list stays in
  sight, the hint line saying where the keys go; `ctrl+\` comes back to the list. `f` opens it
  edge to edge under the bar of tabs, as `⏎` does on the grid (which has no preview to type in).
  The owner asked for it: working beside the list, not only full. A session opened from the
  picker opens the same way at once — it is the one to work in.
  The dashboard's panel 8 and the floating pane's title say a session's state in the zoom's words
  (`◐ working`, `! needs you`, `○ idle`, `✕ ended`, `↻ kept`, `◌ starting`).
- **Tests.** `src/ui2/tests.rs::the_zooms_render_at_every_size_without_a_panic` renders the four
  at 160 random sizes each with 0, 4, 20 and 50 sessions; `dump` takes `SESSIONS=4|20|50` and
  answers the Gantt's reads as the loop does.
