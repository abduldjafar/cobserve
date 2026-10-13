# New feature ideas for cobserve, from best-in-class monitoring TUIs and observability products (2024–2026)

Scope: features cobserve does **not** have yet (checked against `00-context.md` and `README.md`:
it already has per-node/per-user memory & CPU, insights, 4-min sparklines, the tape, Redash
queue + cancel, Airflow day, Jira board, embedded Claude/OpenCode/shell/SQL sessions, prayer
times, macOS notifications with `NOTIFY_ON`, a shelf of cards, `/` filter, `u` pivot, `y` copy
link on views 5/6, a desktop shell, Tarum theme in dark/light/mono).

Constraints applied throughout: read-only ClickHouse (every request `readonly=1`), everything
on the laptop, ratatui 0.30, optional wry/xterm.js shell. Writes are flagged **[WRITE]**.

Effort: S = a day or two, M = a week, L = several weeks. Value: how much it helps at 03:00.

---

## 1. Time travel / replay: scrubbing back through a live view, local persistence, "what changed between 02:55 and 03:05"

### Takeaway
No monitoring TUI does true time-travel on a *live dashboard*; the closest precedents are
`hwatch` (every poll kept, arrow keys walk the history, three diff modes, JSON log reloadable)
and `chdig` (`--start/--end` windows over ClickHouse's own `system.*_log`). cobserve already
polls and keeps 4 minutes; keeping the same snapshots in SQLite for hours and adding a scrubber
is the single most "quietly indispensable" feature on this list.

### Cited Findings
- `hwatch` (Rust, since 2018, 895 commits) "records the result of command execution, can display it history and diffs"; history navigated with ↑↓/PgUp/PgDn/Home/End, `Tab` toggles history/watch pane; three diff modes (watch/line/word) via `D`; `Shift+O` shows only lines with differences; `/` filters history by string, `*` by regex; `--logfile` saves JSON and on reopen hwatch "tries to read and reuse its history"; `-A/--aftercommand` runs a command when output changes — [hwatch README](https://github.com/blacknon/hwatch)
- `chdig` supports `--start/--end` time windows and a `--history` flag that includes rotated `system.*_log_*` tables for examining past activity; also "P pauses, D toggles the diff coloring" on flamegraphs; status "pre-alpha" — [chdig README](https://github.com/azat/chdig)
- `grafterm` takes `-s 22h -e 20h`, `-d 48h`, ISO-8601 `-s 2019-05-12T12:32:11+02:00`, refresh `-r 2s` — Grafana-style time range in a terminal dashboard — [grafterm README](https://github.com/slok/grafterm)
- `btop` keeps per-box graph history in memory, graph symbols `braille`/`block`/`tty`, `update_ms` default 2000 — but no scrub-back — [btop README](https://github.com/aristocratos/btop)
- Glances' alert log stores for each event `begin, end, state, type, max, min, avg, sum, count, top` (top-3 process names during the event) and a `history_size` — i.e. per-incident summaries persisted, not raw samples — [Glances Python API](https://glances.readthedocs.io/en/latest/api/python.html)
- `rusqlite` is at 0.40.2 (released 2026-10-12), has a `cache` feature (`CachedStatement`) and optional `serde_json` — [docs.rs rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)
- `duckdb-rs` is "an ergonomic wrapper over the DuckDB C API that exposes an interface modeled on rusqlite", supports the Appender for bulk loading, Arrow interchange, and a `bundled` feature that compiles DuckDB during build — [DuckDB Rust client](https://duckdb.org/docs/current/clients/rust/overview); crate at 1.4.x — [docs.rs duckdb](https://docs.rs/crate/duckdb/1.4.5)
- ClickHouse itself records completed queries in `system.query_log` with duration, `read_rows`, CPU and memory — the server already is a history store for *finished* queries — [query_log](https://clickhouse.com/docs/reference/system-tables/query_log)
- asciinema 3.0 (2025-09-15) is a Rust rewrite with the asciicast v3 format using deltas for event timing and live streaming — [asciinema 3.0 post](https://blog.asciinema.org/post/three-point-o/)

### Inferences
- **Feature A1 — Scrubber (`[`/`]`, `t` to jump to a time).** Store every poll's `NodeSnapshot` + queue snapshot + tape events in `~/.local/state/cobserve/history.sqlite` (rusqlite `bundled`, WAL, one row per poll per node, JSON column via `serde_json`; ~9 nodes × 1 poll/2 s × 12 h ≈ 200k rows ≈ tens of MB; prune by `retention_hours`, default 24). The UI renders a snapshot exactly as live: `App::view(snapshot)` already exists for FAKE mode, so replay is "feed an old snapshot instead of the live one". Masthead changes `● live` → `◷ 03:02:14 · replay` (colour = severity only, so use the glyph). Effort **M**. Value **very high** — reviewing a 03:00 page at 09:00 is the on-call pain point hwatch solves for shell commands.
- **Feature A2 — Diff between two instants (`d` in replay).** hwatch's "only lines with differences" applied to the NODES tree: rows whose memory moved > X GiB or whose state changed; new/vanished queries marked `+`/`−`. Pure in-memory diff of two snapshots; no new data. Effort **S** once A1 exists. Value high.
- **Feature A3 — Longer sparklines from history.** The 4-min sparkline becomes a `1m / 10m / 1h / 6h` selector reading from SQLite (downsample by `avg` per bucket in SQL). Effort **S** after A1.
- SQLite over DuckDB: the write pattern is many tiny inserts every 2 s on battery; DuckDB's `bundled` build is a heavy C++ compile and its strength (columnar scans) is irrelevant at this size. DuckDB only wins if the owner wants to query months of history ad hoc — and then the SQL console (view 7) could open the file with chDB anyway.
- `chdig`'s approach (query the server's `system.query_log` for the past) is complementary and free: see §5 "fingerprint history". Combining both: local SQLite = what the *screen* showed; `query_log` = what the *server* did.
- Recording the terminal (asciinema v3) is **not** a substitute: it replays pixels, not data, and cannot be filtered or diffed.

### Gaps
- Found no TUI that scrubs a live multi-panel dashboard back in time; hwatch is single-command. No benchmark for SQLite insert load from a ratatui app on a laptop (the estimate above is arithmetic, not a measurement).

---

## 2. Incident mode: "what's wrong now", auto-focus, pinning, incident timeline with notes, export of a report (OSC 52, markdown)

### Takeaway
k9s has a dedicated health screen (`:pulses`) and a resource-tree screen (`:xray`); Glances
keeps a per-alert log with begin/end/min/avg/max and the top processes *at the time*; lnav and
many TUIs copy to the clipboard with OSC 52, which works over SSH. cobserve's insights + tape
are 80 % of an incident screen already — what is missing is a focused mode, typed notes, and a
one-key markdown export.

### Cited Findings
- k9s: `:pulses` (or `pu`) "launches the pulses view"; `:xray RESOURCE [NAMESPACE]` launches the XRay view (resource tree); `:screendump`/`sd` views saved resources; `ctrl-a` shows all aliases — [k9s commands](https://k9scli.io/topics/commands/)
- Glances records an alert event with `begin, end, state, type (CPU|LOAD|MEM), max, min, avg, sum, count, top (top 3 processes name during the event period)` — [Glances Python API](https://glances.readthedocs.io/en/latest/api/python.html); default thresholds careful=50, warning=70, critical=90 — [terminal.guide glances](https://terminal.guide/tools/system-monitor/glances)
- OSC 52 is `OSC 52 ; Pc ; Pd BEL` where `Pc` is the clipboard (`c` clipboard, `p` primary…) and `Pd` is base64 data; supported by Windows Terminal, iTerm2, kitty, Alacritty, WezTerm, foot, recent GNOME Terminal — [go-osc52 docs](https://pkg.go.dev/github.com/aymanbagabas/go-osc52)
- lnav's FAQ: if the terminal supports the escape sequence "the selected text will be copied to the clipboard, even if you are on an SSH connection" — [lnav FAQ mirror](https://git.blob42.xyz/Archives/lnav/src/commit/11545f84ca9dd6b5e56f4482f2d2ebeb59952a2e/docs/source/faq.rst)
- crossterm 0.29 includes OSC 52 behind the `osc52` feature; ratatui has no built-in clipboard, apps emit OSC 52 themselves — [ex-ratatui paste & clipboard](https://ex-ratatui.hexdocs.pm/paste_and_clipboard.md)
- Grafana's incident auto-summary "sanitizes and compresses incident timeline data" before sending to the LLM — [Grafana LLM plugin docs](https://grafana.com/docs/grafana-cloud/machine-learning/llm/); Sift investigations attach to incidents in Grafana IRM — [Grafana Sift blog](https://grafana.com/blog/ai-powered-diagnostics-for-incident-response-new-sift-features-in-grafana-irm?pg=blog)
- Datadog Bits Investigation "forms hypotheses, tests them using live telemetry data, and follows promising evidence to a root cause", finishing in ~3–4 minutes — [Datadog: building Bits AI SRE](https://www.datadoghq.com/blog/building-bits-ai-sre/)

### Inferences
- **Feature B1 — Incident view (`!` or `i`).** One screen: the worst insight expanded with its drawer, the tape filtered to the last 30 min, the queue's red line, and the affected nodes' 30-min sparklines. Auto-opens nothing (the owner wants calm); the shelf card turns red and `!` is offered in the footer. Data: all already in memory. Effort **S–M**. Value high (k9s' `:pulses` proves a "health only" screen earns its key).
- **Feature B2 — Pin (`P`).** Pin a node/user/query/job so it stays in the tree top and in the shelf regardless of sort/fold; pins survive restart in a tiny TOML. Effort **S**.
- **Feature B3 — Notes on the tape (`n`).** Type a line; it is appended to the tape with the clock and persisted with A1's SQLite. Glances' *top processes at the time of the alert* generalises to: a note automatically captures the current worst row's numbers. Effort **S**. Value high — this is the raw material of the post-mortem.
- **Feature B4 — Export incident report (`y` on the incident view).** Markdown: header with window, the tape lines (events + notes), a table of nodes with mem/cpu peak (from A1), the queries involved (query_id, person, SQL first line, peak memory, duration), every percentage with its denominator (house rule). Copy with OSC 52 (base64, chunked under ~100 KB for iTerm2's limit; fall back to `pbcopy` on macOS via `arboard`) and also write `~/incidents/2026-10-13T0302.md`. Effort **S**. Value high; pasting into Slack/Jira is the real deliverable of an incident.
- **Feature B5 — "Explain this incident" (AI, see §9)**: B4's markdown is exactly the prompt to pipe into the in-app Claude session.
- Verdict: B3+B4 are *quietly indispensable*; B1 is medium; a Datadog-style autonomous investigator is out of scope for a laptop monitor but B4→Claude gets 70 % of it for free.

### Gaps
- No source found describing lazygit/htop exporting *reports*; htop has no export. k9s `:pulses` behaviour details not on the fetched page beyond the command name.

---

## 3. Alert rules in-app: thresholds, watch expressions, notifications (macOS/Linux, OSC 9/777/99, webhooks), snooze, maintenance windows

### Takeaway
Sampler is the only TUI with user-written trigger expressions (bell / sound / visual / script);
Glances derives alerts from careful/warning/critical thresholds per metric and runs actions;
hwatch's `-A` runs a command on change; btop and k9s have none. Terminal-native notifications
exist (OSC 9 iTerm2, OSC 777 WezTerm/Ghostty, OSC 99 kitty), which matter when cobserve runs
inside tmux/SSH rather than the desktop shell. Chat webhooks are a write to a chat — fine if
opt-in and never automatic.

### Cited Findings
- Sampler triggers: condition + actions `terminal-bell: true`, `sound: true` ("NASA quindar tone"), `visual: false` ("notification with current value on top of the component area"), plus a custom `script`; components runchart/sparkline/barchart/gauge/textbox/asciibox; `rate-ms` default 1000; variables; PTY/`init` for stateful sessions — [Sampler README](https://github.com/sqshq/sampler)
- Glances thresholds careful/warning/critical, defaults 50/70/90, configured per metric in the config file; alert events include min/avg/max and the top processes — [terminal.guide glances](https://terminal.guide/tools/system-monitor/glances); [Glances Python API](https://glances.readthedocs.io/en/latest/api/python.html)
- hwatch `-A, --aftercommand` "Executes the specified command if the output changes" — [hwatch README](https://github.com/blacknon/hwatch)
- Terminal notification escapes: iTerm2 uses OSC 9, kitty OSC 99, WezTerm OSC 777, Ghostty OSC 777 (also rxvt-unicode); progress bars via OSC 9;4 in Windows Terminal, Ghostty 1.2+, iTerm2 3.6.6+ — [soloterm/tnotify docs](https://pkg.go.dev/github.com/soloterm/tnotify@v0.1.6)
- Redash V8+ emails query owners "once per hour if one or more queries failed"; failure emails "run on an independent process from the actual query schedules" and may lag up to an hour — [Redash scheduling help](https://redash.io/help/user-guide/querying/scheduling-a-query/)
- Airflow 3.1 Deadline Alerts: "when a Dag run starts, the system calculates and stores (DeadlineReference + interval)" and the scheduler checks every ~5 s; callbacks run in supervised subprocesses; deadline status is visible in the Grid view — [AIP-86](https://cwiki.apache.org/confluence/spaces/AIRFLOW/pages/323488182/AIP-86+Deadline+Alerts+Formerly+SLA); [SLA to Deadlines howto](https://airflow.apache.org/docs/apache-airflow/3.1.0/howto/sla-to-deadlines.html)

### Inferences
- **Feature C1 — Watch rules in `~/.config/cobserve/rules.toml`.** Sampler's shape, cobserve's vocabulary: `[[rule]] name = "bi hot" when = "node.mem_pct > 85 and node.name == 'clickhouse-bi' for 2m" notify = ["desktop", "tape"]`. Evaluate against the same `NodeSnapshot`/queue structs the insights use (a small expression evaluator: `evalexpr` crate, or hand-rolled since the grammar is tiny). Every rule firing is a tape event, so A1/B4 record it. Effort **M**. Value high, but only if the defaults stay good — rules are for the 10 % the insights do not know.
- **Feature C2 — Snooze and maintenance windows.** `z` on an insight/notification: snooze 15 m / 1 h / until tomorrow Subuh (the day line already knows the times); `maint.toml` with `node = "ch4" from = "2026-10-14T01:00" to = "…"` greys the node and mutes its notifications. Glances has neither; PagerDuty-style snooze is table stakes for a monitor you leave open. Effort **S**. Value high; this is the feature that stops the owner turning notifications off.
- **Feature C3 — Terminal-native notifications.** When not in the desktop shell and `$TERM_PROGRAM`/`$TERM` says iTerm2/WezTerm/Ghostty/kitty, emit OSC 9 / 777 / 99 instead of (or in addition to) `osascript`; this reaches through tmux and SSH where `terminal-notifier` does not. Effort **S**.
- **Feature C4 — Webhook action [WRITE to chat].** A rule may `post = "google-chat"` with the webhook URL from env; body = the insight line + link (§8). Acceptable because it is opt-in per rule, never on by default, and posts only to a channel the owner configures — but it is the second write in the app after the Redash cancel and should be shown in the footer as such. Effort **S**. Value medium (the owner is the one on call; posting to oneself is redundant until a second person reads cobserve's output).
- **Feature C5 — Sound.** Sampler's quindar tone is loved; a single short tone for red events via `rodio` or `afplay` is cheap, and prayer-time integration already proves the owner likes ambient cues. Effort **S**, value low-medium ("wow but shallow" unless it is quiet).
- Airflow 3.1's deadline model (compute the deadline at start, check periodically) is the right model for C1's `for 2m` clauses and for an in-app "DAG X usually finishes by 06:00" rule (see §11).

### Gaps
- Sampler's last release looks like v1.1.0 and the fetched page did not show a last-commit date; maintenance status unverified. No source compared `evalexpr` vs `rhai` for rule expressions.

---

## 4. Comparisons and correlation: two nodes side by side, queue depth vs node load, same query N times, top-K over a window, anomaly flags from local history

### Takeaway
No TUI found does anomaly flags on its own data; the precedents are products: Netdata (18
k-means models per metric, consensus, "anomaly rate" on every chart) and Axiom's CLI (z-score
with ≥30 samples and a 24 h baseline). Both agree on the lesson: a baseline needs hours of
history (→ §1), and a flag is only trustworthy with a consensus/hysteresis step. Side-by-side
node compare and overlaid series are plain ratatui layout work once history exists.

### Cited Findings
- Netdata Anomaly Advisor: "per-metric anomaly detection using k-means clustering (18 models per metric)", each trained on 6-hour windows staggered 3 h; "all machine learning models of a time-series need to agree that a collected sample is an outlier… which removes 99% of the false positives"; anomaly rate exposed on every chart — [Netdata Anomaly Advisor](https://learn.netdata.cloud/docs/netdata-ai/troubleshooting/anomaly-advisor); [Netdata ML](https://learn.netdata.cloud/docs/agent/ml)
- Axiom CLI `detect-anomalies` uses z-score "requiring at least 30 samples per bucket and a 24-hour baseline to be meaningful" — [Axiom skill listing](https://skillselion.com/skills/axiomhq/cli/detect-anomalies)
- chdig has a `--cluster` flag for cluster-wide aggregation and tmux-like split panes — [chdig README](https://github.com/azat/chdig)
- ClickHouse's slow-query guide: "Start by grouping completed initial queries by normalized_query_hash. This separates query patterns that recur from individual slow executions" — [Diagnose slow queries](https://clickhouse.com/docs/guides/clickhouse/performance-and-monitoring/diagnose-slow-queries)
- Honeycomb BubbleUp is "powered by machine learning" to "quickly spot outliers that reveal the cause of hidden issues" (compare a selected region against the rest, by every dimension) — [statsig on Honeycomb](https://statsig.com/perspectives/what-is-honeycomb)
- ratatui 0.30 Canvas gained Quadrant (2×2), Sextant (2×3) and Octant (2×4) markers — [ratatui 0.30 highlights](https://ratatui.rs/highlights/v030/)

### Inferences
- **Feature D1 — Compare two nodes (`=` then pick).** Two drawers side by side, same rows (mem, cpu, users, longest, lag), differences highlighted; on 160 cols two full trees. Effort **S**. Value medium; most useful for replica pairs (one shard, 2 replicas per CLAUDE.md's rig) — "why is replica B at 90 % when A is at 40 %".
- **Feature D2 — Overlay: Redash waiting vs fleet mem/cpu, one chart (`4` tape → `g`).** Braille/Octant Canvas chart of the last hour from A1; the question it answers is "did the queue back up because the nodes were full, or the other way round". Effort **S** after A1. Value high at 03:00.
- **Feature D3 — Anomaly flag `≈`, not a colour.** Per node and per series (mem, cpu, waiting count): rolling mean/stddev over the same hour of the previous 7 days from A1 (seasonality matters: Monday 09:00 is not Sunday 03:00); flag when |z| > 3 for ≥ 3 consecutive polls (hysteresis, Netdata's consensus idea in miniature). Print the baseline next to it (`mem 61 % · usual 35 ± 8 %`) to honour "every number traces to a formula". Colour stays severity-only, so the anomaly is a glyph and an insight line, not red. Effort **M** (the maths is S; the week of history is the cost). Value high — "this is normal for Monday" is exactly what a tired on-call wants to know. Netdata-style k-means is overkill for 9–100 series.
- **Feature D4 — Top-K over a window.** `u` pivot already answers "who is burning the fleet now"; add "over the last hour / today": sum of memory-seconds per person from A1 or from `system.query_log` grouped by `initial_user`/person. Effort **S**.
- **Feature D5 — Same query N times** exists as an insight for Redash (`running 2×`); extend to ClickHouse via `normalized_query_hash` in `system.processes` so a non-Redash duplicate (Airflow retry storms) is caught too. Effort **S**.

### Gaps
- No TUI found with built-in anomaly detection; the claim "no TUI does it" is absence of evidence from ~6 searches, not proof. Honeycomb Query Assistant details were not retrieved (search returned only marketing pages).

---

## 5. Query forensics: EXPLAIN as a tree, flamegraphs from `system.trace_log`, plan diff, fingerprint history from `query_log`, cost hints

### Takeaway
`chdig` is the reference: CPU/real/memory/live flamegraphs from `system.trace_log`, EXPLAIN,
slow/last queries, `--history` over rotated logs, export to speedscope/Perfetto. All of it is
read-only SQL on system tables, so all of it is within cobserve's constraints. ClickHouse's
`EXPLAIN ESTIMATE` returns parts/rows/marks without running the query — the "this would read
1.9 B rows" cost hint is one statement. The docs' own slow-query method is
"group by `normalized_query_hash`" — the fingerprint history.

### Cited Findings
- chdig views: processes, slow queries, last queries, merges, replicas, replication queue, errors, backups, logs; "CPU, real-time, memory, and live flamegraphs sourced from `system.trace_log`" with auto-refresh, `P` pause, `D` diff colouring; EXPLAIN and query details; export to Perfetto and speedscope; share via pastila.nl — [chdig README](https://github.com/azat/chdig)
- `EXPLAIN ESTIMATE` output columns `database, table, parts, rows, marks`; `EXPLAIN PLAN indexes=1` reports per index `"Parts": 4/5, "Granules": 11/12` (selected/total); `EXPLAIN PIPELINE` has `graph`/`compact`; `EXPLAIN ANALYZE` executes and adds runtime metrics (`processors`, `time`) — [ClickHouse EXPLAIN](https://clickhouse.com/docs/sql-reference/statements/explain)
- `system.query_log` rows carry duration, `read_rows` ("Total number of rows read from all tables and table functions… includes usual subqueries, subqueries for IN and JOIN"), CPU, memory — [query_log](https://clickhouse.com/docs/reference/system-tables/query_log)
- "Start by grouping completed initial queries by normalized_query_hash" — [Diagnose slow queries](https://clickhouse.com/docs/guides/clickhouse/performance-and-monitoring/diagnose-slow-queries)
- `flameshow` (Python/textual) renders flamegraphs in the terminal: zoom with Enter/Esc, percentages, mouse; reads pprof and Brendan Gregg collapsed format — [flameshow on PyPI](https://pypi.org/project/flameshow)
- Grafana's LLM plugin powers "flame graph interpretation" among its features — [Grafana LLM plugin](https://grafana.com/docs/grafana-cloud/machine-learning/llm/)

### Inferences
- **Feature E1 — Fingerprint card in the query drawer.** For the query under the cursor: `normalized_query_hash` → `SELECT count(), quantile(0.5)(query_duration_ms), quantile(0.95)(…), max(memory_usage), avg(read_rows) FROM system.query_log WHERE normalized_query_hash = ? AND event_time > now() - INTERVAL 7 DAY AND type = 'QueryFinish'`; render `usually 42 s · p95 1m50 · 6.1 GiB · ran 318× this week` and `✕ 12 failed` (type = ExceptionWhileProcessing). Needs SELECT on `system.query_log` (insights already say which grant is missing, so reuse that pattern). Effort **S**. Value **very high** — "is this slow for *this* query" is the first question on call, and nobody answers it today without Redash + a notebook.
- **Feature E2 — Cost hint before it hurts.** `EXPLAIN ESTIMATE <sql>` for the selected query → `would read 1.9 B rows · 412 parts · 23 k marks`; `EXPLAIN PLAN indexes=1` → "Parts 4/5 · Granules 11/12" shows whether the partition key is doing anything. EXPLAIN does not execute the query and respects `readonly=1`. Effort **S**. Value high for the SQL console (view 7) and for the Redash job before it is cancelled.
- **Feature E3 — Flamegraph (`f` on a running query).** `SELECT arrayStringConcat(arrayReverse(arrayMap(x -> demangle(addressToSymbol(x)), trace)), ';') AS stack, count() FROM system.trace_log WHERE query_id = ? GROUP BY stack` is the collapsed format chdig and flameshow consume. Draw as a ratatui tree of frames with width = share (a flamegraph is a treemap of a tree; a 120-col terminal fits ~6 levels), `⏎` zooms, `d` diffs two refreshes (chdig's `D`). Needs `allow_introspection_functions=1` for `addressToSymbol`/`demangle` — a *setting*, which a read-only profile may refuse; fall back to raw addresses or to exporting the collapsed stacks for `speedscope`/`flameshow`. Effort **M–L**. Value medium for the owner (DataOps, not a CH developer) — "wow", and genuinely useful once a quarter.
- **Feature E4 — Plan diff.** Store `EXPLAIN PLAN` text per fingerprint in A1's SQLite; when the same fingerprint's plan changes (new index, lost projection, analyzer change after an upgrade), say so. Effort **S** on top of E1/E2. Value medium.
- **Feature E5 — Airflow/Redash → query_log join.** `query_log.log_comment`/`http_user_agent`/`client_name` often carry the Redash query id or the Airflow task; linking a DAG task to its ClickHouse queries' history closes the loop between views 5 and 1. Effort **M** (depends on what the fleet's clients send). Flagged as *to verify on a real node*.

### Gaps
- Did not verify whether the fleet's read-only profile allows `allow_introspection_functions`; without it E3 degrades to addresses. Did not find chdig's crate stack (cursive vs ratatui) on the fetched page.

---

## 6. Fleet scale (40–100 nodes): grouping by shard/replica/role, heatmap, "only abnormal" mode, search across everything, bookmarks

### Takeaway
Precedents are k9s (`:xray` resource tree, aliases, filters), btop (tree view with
auto-collapse, filter), chdig (`--cluster`). cobserve already folds healthy nodes and has MAP
cards; what scales to 100 is a heatmap of one glyph per node per minute and grouping by
`system.clusters` shard/replica, which is free metadata.

### Cited Findings
- k9s `:xray RESOURCE [NAMESPACE]` renders a resource tree; `ctrl-a` lists aliases; `:screendump` for saved resources — [k9s commands](https://k9scli.io/topics/commands/)
- btop: "Tree view of processes with auto-collapse functionality", process filtering, presets `box_name:P:G` up to 9 via `p` — [btop README](https://github.com/aristocratos/btop)
- chdig `--cluster` aggregates across the cluster — [chdig README](https://github.com/azat/chdig)
- ratatui 0.30 Canvas high-resolution markers Quadrant/Sextant/Octant; `Rect::layout()`, `Rect::centered()`, `Flex::SpaceEvenly` — [ratatui 0.30 highlights](https://ratatui.rs/highlights/v030/)
- CLAUDE.md (project): the real fleet reports 1 shard, 2 replicas; `system.clusters` names resolve only inside the fleet network.

### Inferences
- **Feature F1 — Group by shard/replica/role (`G`).** `system.clusters` gives `shard_num`, `replica_num`; a role (`bi`, `ingest`, `replica`) comes from name patterns in config. Tree: cluster → shard → replica node; fold at any level; the §5.3 sum invariant holds at every level because it is just addition. Effort **S**. Value high at 40+, nil at 9.
- **Feature F2 — Fleet heatmap (`3` MAP → `h`).** Rows = nodes (or shards), columns = last 60 minutes from A1, one cell = `▁▂▃▄▅▆▇█` by memory %, severity glyph overlaid when red. 100 nodes × 60 cols fits 160×48 with a scroll. Effort **S** after A1. Value high — this is the only honest "whole fleet, last hour" view at 100 nodes.
- **Feature F3 — Only-abnormal mode (`!` doubles as this on NODES).** Already exists as "fold healthy"; extend to hide healthy *users* and *queries* too, and to the queue (only stale/old). Effort **S**.
- **Feature F4 — Global search (`ctrl+f` / `:`).** One prompt searching nodes, people, query ids, SQL text, DAG names, ticket keys, tape lines, session names; results grouped by view; `⏎` jumps. k9s' `:` command palette is the UX model. Effort **M** (indexing is trivial; the palette widget is the work; `tui-input`/`nucleo` crates for fuzzy). Value high and it doubles as a *command palette* for every key the footer cannot show.
- **Feature F5 — Bookmarks.** Named saved states: view + filter + sort + fold + pins (`m` to mark, `'` to recall, vim-style). Effort **S**. Value medium.

### Gaps
- No TUI heatmap-of-fleet-over-time precedent was found in the sources fetched (Grafana's status-history panel is the web precedent; not fetched).

---

## 7. Configurable dashboards and plugins: YAML/TOML layouts, saved layouts per task, Lua/Rhai/WASM scripting

### Takeaway
wtfutil (YAML grid of modules), Sampler (YAML components), grafterm (JSON dashboards),
btop (9 presets) show that *layout* configuration is cheap and liked; k9s (plugins.yaml →
shell command with `$NAME`-style variables) and lazygit (custom commands with prompts and
output modes) show that *shell-out plugins* give 90 % of a plugin system for 5 % of the cost.
Zellij's WASM plugin system is the full-blown precedent and is heavy. For cobserve, k9s-style
plugins fit; Lua/WASM does not pay for itself.

### Cited Findings
- wtfutil: YAML config, grid layout with `top/left/height/width`, per-module `refreshInterval`, focusable modules with shortcuts; the project is "being renamed and moving to a new GitHub org" (`linodians/tessera`) ahead of v1.0 — [wtfutil README](https://github.com/wtfutil/wtf)
- Sampler: six component types, YAML, variables, `rate-ms` — [Sampler README](https://github.com/sqshq/sampler)
- grafterm: JSON dashboards with graph/singlestat/gauge, Prometheus/Graphite, variables `-v env=prod`, datasource aliasing — [grafterm README](https://github.com/slok/grafterm)
- btop: presets `box_name:P:G`, up to 9, `p` cycles; `shown_boxes`; themes as `.theme` files in `$XDG_CONFIG_HOME/btop/themes` — [btop README](https://github.com/aristocratos/btop)
- k9s plugins: `plugins.yaml` with `shortCut, description, scopes, command, background, args`, variables `$NAME $NAMESPACE $CONTEXT $CLUSTER $FILTER $COL-<COLUMN>` injected; example `Ctrl-L` → `kubectl logs -f $NAME -n $NAMESPACE` — [k9s plugins](https://k9scli.io/topics/plugins/)
- lazygit custom commands: `key, context, command, prompts (input/menu/confirm/menuFromCommand), loadingText, output: none|terminal|log|logWithPty|popup`, templates `{{.SelectedCommit.Hash}}`, `{{.Form.X}}`, functions `quote`, `runCommand` — [lazygit Custom_Command_Keybindings.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Custom_Command_Keybindings.md)
- Zellij: WASM/WASI plugins, Rust first-class via `zellij-tile` (0.45) with `ZellijPlugin` trait and `register_plugin!`; "a plugin is a first class citizen… can render a UI, react to application state changes" — [zellij plugins](https://zellij.dev/documentation/plugins); [zellij-tile](https://docs.rs/crate/zellij-tile/0.45.0)

### Inferences
- **Feature G1 — k9s-style plugins (`plugins.toml`).** `[[plugin]] key = "ctrl+l" scope = ["query","job"] command = ["open", "https://redash…/queries/$REDASH_ID"]` or `["sh","-c","clickhouse-client … $QUERY_ID"]`, with `$NODE $HOST $QUERY_ID $PERSON $DAG_ID $TICKET $SQL_FILE` injected and lazygit's `output = "popup" | "session" | "none"` — `session` opens it in a view-7 PTY, which cobserve already has. Effort **S–M**. Value high: it is how the owner wires Grafana, Redash, `kubectl`, runbooks, without cobserve learning each. Note the plugin runs whatever the owner wrote; the app stays read-only, the shell does not — document it.
- **Feature G2 — Layout presets (`p`, like btop).** Per task: `oncall` (NODES + QUEUE + tape strip), `daily` (AIRFLOW + JIRA), `wall` (MAP big). TOML `[preset.oncall] panes = ["nodes:60","queue:40"]`. Effort **M** (cobserve's views are full-screen today; a two-pane layout is new plumbing). Value medium.
- **Feature G3 — Scripting (Lua via `mlua`, Rhai, or WASM).** Not recommended now: the data model is small and typed, G1 covers "run my thing", and C1 covers "my condition". Revisit if two or more people start customising. Effort **L**, value low for a single-owner tool.
- Theme files as data (btop `.theme`) is cheap and belongs to §10.

### Gaps
- Did not retrieve yazi's Lua plugin API or termdash/zenith/gotop READMEs; the comparison relies on k9s/lazygit/zellij/wtfutil only.

---

## 8. Collaboration and remote: TUI over SSH (russh), web twin (ratzilla), tmux/zellij, OSC 8 links, deep links

### Takeaway
Serving a ratatui app over SSH is a solved pattern (russh + per-connection terminal; `sshui`
crate; a real site does it); ratzilla compiles ratatui to WASM with DOM/Canvas/WebGL2 backends.
OSC 8 hyperlinks have a ratatui example and a `hyperrat` widget. The cheapest, highest-value
item is OSC 8 links on every row; SSH serving is a real "wow" with a security cost.

### Cited Findings
- A production example uses "Ratatui to draw the TUI and russh 0.60 in src/serve.rs for the SSH server. The render function is identical whether running locally or over SSH"; it avoids forking a PTY — [hackernoon: website as SSH app](https://hackernoon.com/how-i-turned-my-website-into-an-ssh-terminal-app?source=rss)
- `sshui` (0.1.x) is "a Rust framework for building interactive TUIs that run over SSH, built on top of Ratatui and russh", with client isolation, resizing, custom auth — [docs.rs sshui](https://docs.rs/sshui/latest/sshui/)
- ratzilla: "terminal-themed web applications with Rust and WebAssembly. Powered by Ratatui"; DOM, Canvas and WebGL2 backends; port by wrapping rendering in `terminal.draw_web()` plus an `index.html`; 1.5k stars, 209 commits; real sites built with it — [ratzilla README](https://github.com/orhun/ratzilla)
- ratatui hyperlink example renders clickable OSC 8 links — [ratatui hyperlink example](https://ratatui.rs/examples/apps/hyperlink/); `hyperrat` is "a Link widget for ratatui that renders clickable hyperlinks using the OSC 8 escape sequence" — [docs.rs hyperrat](https://docs.rs/hyperrat)
- asciinema 3.0 adds terminal live streaming — [asciinema 3.0](https://blog.asciinema.org/post/three-point-o/)
- chdig shares flamegraphs/logs via pastila.nl — [chdig README](https://github.com/azat/chdig)

### Inferences
- **Feature H1 — OSC 8 links everywhere.** Redash job → its query page, person → Redash user, DAG/run → Airflow, ticket → Jira, node → Grafana dashboard with `var-host=`; `y` already copies on views 5/6, so this is "make it clickable too". xterm.js in the desktop shell supports OSC 8 with its web-links addon (verify). Effort **S**. Value high, zero risk.
- **Feature H2 — `cobserve://` deep links.** Register the URL scheme in the desktop app's `Info.plist`; `cobserve://node/clickhouse3`, `cobserve://query/<id>`, `cobserve://replay/2026-10-13T03:02`. B4's exported report then contains links back into the replay. Effort **S** (desktop) + routing. Value medium-high with A1.
- **Feature H3 — Share a live view over SSH (`cobserve serve --ssh :2222`)** [opens a network listener]. Second on-call sees the same frames; read-only viewer role (no `x`); russh with key auth only. Effort **M–L**; the hard part is a second `Terminal` with its own size per client, which the hackernoon example shows is feasible. Value medium — "wow"; real value only if a second person ever shares the pager. Also exposes the owner's credentials-backed session to the network: keep off by default.
- **Feature H4 — Web twin via ratzilla.** The desktop shell already *is* a web view; ratzilla would replace xterm.js+PTY with native rendering, but loses the PTY sessions (view 7) which are the point of the shell. Not recommended; "wow but shallow" here.
- **Feature H5 — Live stream via asciinema 3** (`asciinema stream`) is a zero-code way to let a colleague watch; mention in GUIDE.md rather than build.

### Gaps
- Did not verify xterm.js OSC 8 support in the version used by `desktop/`. No source fetched on tmux/zellij integration specifics (zellij plugin API only).

---

## 9. AI in the loop: explain the incident, natural-language filter, drafted messages, query rewrite — what works and what is gimmicky

### Takeaway
Vendors converge on four things that work: summarise an incident timeline (Grafana, Datadog),
explain an artefact in front of the user (flame graph, log line — Grafana), NL → query with
schema context (ClickHouse Ask AI), and hypothesis-driven investigation over the same telemetry
the human sees (Datadog Bits). What is gimmicky: chat boxes without context. cobserve's edge is
that it already has Claude *in the terminal*, on the owner's plan, read-only — so every feature
here is "assemble a precise prompt from on-screen numbers and open a session", not a new model.

### Cited Findings
- ClickHouse Cloud Ask AI "responds with generated queries, visualizations, or summaries, and can incorporate context like active tabs, saved queries, schema details, and dashboards"; a Docs AI sub-agent answers doc questions — [ClickHouse Ask AI docs](https://clickhouse.com/docs/products/cloud/features/ai-ml/ask-ai); public beta of the Ask AI agent and a remote MCP server; "ClickHouse Agents, launched in beta during OpenHouse in June 2026… powered by Claude" used for "query performance analysis" — [ClickHouse blog](https://clickhouse.com/blog/agentic-analytics-ask-ai-agent-and-remote-mcp-server-beta-launch)
- Grafana LLM plugin: proxy to OpenAI/Anthropic/Azure/vLLM/Ollama/LiteLLM; powers "flame graph interpretation, incident auto-summary, panel title generation, Sift log explanations, and natural language panel descriptions"; incident timeline data is sanitised and compressed before the LLM — [Grafana LLM plugin](https://grafana.com/docs/grafana-cloud/machine-learning/llm/)
- Datadog Bits Investigation: triggers on the alert, "reads the same telemetry data as your team… follows your runbooks", ~3–4 min, tested in 2,000+ customer environments — [Datadog building Bits AI SRE](https://www.datadoghq.com/blog/building-bits-ai-sre/); [Better Stack comparison](https://betterstack.com/community/comparisons/better-stack-ai-sre-vs-datadog-bits-ai-sre/)
- cobserve already has "Claude writes the SQL" (`ctrl+k`) in the SQL console — README (local).

### Inferences
- **Feature I1 — "Explain this" (`e`) on anything.** Builds a prompt from the drawer's numbers + the last 30 tape lines + (for a query) the SQL, E1's fingerprint stats and E2's estimate, then opens/reuses a Claude session (view 7) with it pasted. The prompt is text the owner can read before sending — Grafana's "sanitise the timeline" principle; strip hostnames/people if `AI_REDACT=1`. Effort **S** (the session plumbing exists). Value **high**; this is Grafana's auto-summary and flame-graph interpretation on the owner's own plan.
- **Feature I2 — Natural-language filter (`/` then `?`).** "j.petrova over 5 GiB on bi" → Claude returns a filter expression in C1's grammar, cobserve applies it and *shows the expression* so it is learnable and reproducible. Effort **S**. Value medium; the typed filter is usually faster, but the NL path teaches the grammar.
- **Feature I3 — Draft the message.** From B4's report: "draft a Google Chat message to #data-oncall / a Jira comment for DATA-xxxx". Output goes to the clipboard (OSC 52), never posted automatically [posting would be a WRITE]. Effort **S**. Value high at 03:00 when words are hard.
- **Feature I4 — Query rewrite suggestion** for a runaway Redash query: prompt = SQL + E2 estimate + E1 history + the project's ClickHouse best-practice rules; result shown as a diff; `y` copies. Effort **S**. Value medium-high (ties into the owner's existing optimisation work).
- **Feature I5 — Investigator loop (Datadog-style)**: a Claude session with an MCP/CLI that can run read-only SQL against the fleet (the SQL console's `readonly=1` path) and read A1's SQLite; `!` → "investigate". Effort **M**; value high but risk of token burn — the LOCAL view already shows plan usage, so the cost is visible. Keep it manual-trigger only.
- Gimmicks to avoid: an always-on chat pane, AI-generated panel titles, "AI insights" that duplicate the deterministic insights (§5 formulas must stay the source of truth).

### Gaps
- Honeycomb Query Assistant, Warp AI and OpenCode TUI behaviour were not retrieved (searches returned marketing/aggregator pages only).

---

## 10. Quality of life: themes gallery, big-number wall mode, screenshot/record, hot-reload, profiles switcher, session persistence, macros

### Takeaway
btop's `.theme` files + bundled gallery and `p` presets are the model for themes/layouts;
`tui-big-text` gives an 8×8 bitmap font with eight pixel sizes for a wall display; asciinema 3
(Rust) records and streams; k9s has contexts (`:ctx`) as the model for fleet profiles. These
are all S-effort and mostly "wow" except hot-reload and profiles, which are daily conveniences.

### Cited Findings
- btop: bundled themes in `/themes`, `.theme` format compatible with bpytop/bashtop, user themes in `$XDG_CONFIG_HOME/btop/themes`, `color_theme`, `truecolor`, presets via `p`, full mouse — [btop README](https://github.com/aristocratos/btop)
- `tui-big-text`: "oversized pixel text using glyphs from the font8x8 crate"; `PixelSize::{Full, HalfHeight, HalfWidth, Quadrant, ThirdHeight, Sextant, QuarterHeight, Octant}`; alignment; `cargo add ratatui tui-big-text` — [tui-big-text](https://github.com/joshka/tui-widgets/tree/main/tui-big-text)
- asciinema 3.0: Rust, asciicast v3 (delta timing, easier editing), live streaming, static binary — [asciinema 3.0](https://blog.asciinema.org/post/three-point-o/)
- k9s `:ctx` switches contexts, `:ns` namespaces — [k9s commands](https://k9scli.io/topics/commands/)
- ratatui 0.30: `ratatui::run()`, six new dashed `BorderType`s, border merging, `LineGauge::filled_symbol()/unfilled_symbol()`, List highlight as styled `Line` — [ratatui 0.30 highlights](https://ratatui.rs/highlights/v030/)
- hwatch reloads `--logfile` history on restart — [hwatch README](https://github.com/blacknon/hwatch)

### Inferences
- **Feature J1 — Theme files.** Keep Tarum as the default and *identity*; add `~/.config/cobserve/themes/*.toml` with the same slots (ground, text, accent, severity red/amber/green) so Catppuccin etc. are a file, not code; severity colours remain mandatory slots to keep "colour means severity". Effort **S**. Value low-medium ("wow"; also what people screenshot).
- **Feature J2 — Wall mode (`W`).** `tui-big-text` Quadrant/Sextant: fleet mem %, cpu %, waiting count, oldest wait, each with its denominator in normal text beneath; MAP cards below. For a monitor in the room or a tmux pane at 50 % zoom. Effort **S**. Value medium; it is also the best demo screen.
- **Feature J3 — Hot-reload of config/rules/themes/plugins** via `notify` crate on `~/.config/cobserve/`; no restart to tweak a C1 rule at 03:00. Effort **S**. Value high (quietly).
- **Feature J4 — Profiles (`:ctx`-like).** `~/.config/cobserve/profiles/{prod,rig,fr13}.toml` each with CH/Redash/Airflow/Jira endpoints; `ctrl+p` switches, masthead shows the profile name; credentials stay in the existing per-server YAML. Effort **S–M**. Value high once FR13 (new DC, per the owner's Grafana skill) comes alive.
- **Feature J5 — Record/screenshot.** `S` writes the frame as text (the `dump_screen` test does this) and as `.ans` with colours; a GIF/`.cast` is better done by `asciinema rec`/`vhs` externally — document, do not build. Effort **S**.
- **Feature J6 — Macros.** vim-style `q<key>` record / `@<key>` play of keystrokes, saved to TOML; a bookmark (F5) covers 90 % of the need. Effort **S**, value low.
- Session persistence across restarts (view 7) is already partly done; A1 extends it to data.

### Gaps
- No source on how many bundled themes btop ships (count not in the fetched text). vhs not fetched.

---

## 11. Redash / Airflow / Jira extras: scheduled-query health, SLA misses and duration anomalies, "due this week", time logging

### Takeaway
Redash exposes a consecutive-failure counter per query (`schedule_failures`) and only emails
hourly, so a live "failing/overdue schedules" list in cobserve beats Redash's own alerting;
Airflow 2 has an `SlaMiss` table (and the UI's Browse → SLA Misses) while Airflow 3.1 replaced
SLAs with Deadline Alerts visible in the Grid — cobserve must handle both; "took 3× its usual"
is computable from run history cobserve already reads. Jira time logging is a write.

### Cited Findings
- Redash `schedule_failures` "tracks consecutive failures for scheduled queries… reset to 0 when execution succeeds"; failing queries can be found "where schedule_failures != 0"; failure emails are hourly and may lag an hour — [Redash discussion #5929](https://github.com/getredash/redash/discussions/5929); [Redash scheduling help](https://redash.io/help/user-guide/querying/scheduling-a-query/)
- Airflow 2 `SlaMiss` model: "stores a history of the SLA that have been missed… to avoid double triggering alert emails"; fields `task_id, dag_id, execution_date, email_sent, timestamp, description, notification_sent` — [SlaMiss (2.5.2)](https://airflow.apache.org/docs/apache-airflow/2.5.2/_api/airflow/models/slamiss/index.html)
- "The old SLA feature is removed in Airflow 3.0 and in 3.1 it will be replaced with Deadline Alerts"; SLA ambiguity about when counting starts; Deadline = reference + interval, checked every ~5 s, "visible in the UI Grid view and DAG run overview", named deadlines per stakeholder — [AIP-86](https://cwiki.apache.org/confluence/spaces/AIRFLOW/pages/323488182/AIP-86+Deadline+Alerts+Formerly+SLA); [SLA to Deadlines](https://airflow.apache.org/docs/apache-airflow/3.1.0/howto/sla-to-deadlines.html)
- Axiom's z-score guidance: ≥30 samples and a 24 h baseline — [Axiom detect-anomalies](https://skillselion.com/skills/axiomhq/cli/detect-anomalies)

### Inferences
- **Feature K1 — Redash schedule health panel (view 2 → `h`).** Via the Redash API (`GET /api/queries?…`, which returns `schedule` and `schedule_failures` per query — verify field presence in the API JSON vs the DB column), list: failing schedules (`schedule_failures > 0`, sorted by count), overdue (`now - retrieved_at > schedule.interval × 1.5`), and the heaviest schedules by `runtime`. Effort **S–M**. Value **high**: cobserve already has the admin API key for the queue; Redash's own hourly email is slower than this.
- **Feature K2 — Airflow "took longer than usual" and deadlines.** Per DAG, keep the last 30 run durations (from the runs cobserve already reads) in A1's SQLite; mark a run `▲ 2.8× usual (median 12m)` when > 2× the median and ≥ 5 samples. Airflow 2: read `/api/v1/…` SLA misses if exposed (gap below) or the UI table; Airflow 3.1: read deadline status from the run. Effort **S** (durations) / **M** (SLA/deadline sources). Value high — "is this DAG late or just slow" is the daily morning question.
- **Feature K3 — Jira "this week" planner (view 6 → `w`).** Due-this-week tickets grouped by day across the day line's week, with logged vs estimated hours; nothing new to fetch beyond what view 6 reads (due dates, worklogs). Effort **S**. Value medium.
- **Feature K4 — Log time from the app [WRITE to Jira].** `POST /rest/api/2/issue/{key}/worklog` with `timeSpentSeconds` and a comment; needs the existing PAT; confirm with `y` like the Redash cancel. Optional, off by default. Effort **S**. Value medium for the owner personally; it is a third write.
- **Feature K5 — Redash failing query → ClickHouse error** join: a `schedule_failures > 0` query's last error text from Redash plus `system.query_log` `exception` for the matching `query_id` (via E5's linkage). Effort **M**.

### Gaps
- Could not confirm from the fetched pages whether Airflow 2's stable REST API exposes SLA misses (the `SlaMiss` model and the UI report are documented; an endpoint was not found). Could not confirm that Redash's `/api/queries` JSON includes `schedule_failures` (documented as a DB column). The Jira worklog endpoint was not fetched in this session (owner's memory file notes Jira Server/DC with PAT; verify path).

---

## 12. Prioritised shortlist: quietly indispensable vs wow-but-shallow

### Takeaway
Build the history store first; a third of the ideas above (replay, diff, long sparklines,
anomaly baselines, heatmap, "took longer than usual", plan diff, incident report) are one SQLite
file away. Then the two on-call rituals: incident notes + markdown export, and query
fingerprint history. Everything AI is a prompt assembled from those.

### Cited Findings
- hwatch keeps history and diffs; chdig reads the server's own logs for the past; Glances persists alert events with min/avg/max/top; Netdata/Axiom need hours of baseline — sources cited in §1, §2, §4 above.
- k9s/lazygit shell-out plugins and btop presets/themes — §7, §10.
- OSC 8/52/9/777/99 are supported across iTerm2, kitty, WezTerm, Ghostty, Alacritty — §2, §3, §8.

### Inferences
**Quietly indispensable (do these):**
1. **A1 history in SQLite + scrubber** (M) — unlocks A2, A3, D2, D3, F2, K2, E4, B4.
2. **E1 query fingerprint history from `system.query_log`** (S) — "is this slow for this query".
3. **B3 notes + B4 incident report to clipboard/markdown** (S) — the post-mortem writes itself.
4. **C2 snooze / maintenance windows** (S) — keeps notifications on.
5. **K1 Redash schedule health** (S–M) and **K2 "took longer than usual"** (S).
6. **H1 OSC 8 links on every row** (S); **G1 k9s-style plugins** (S–M); **J3 hot-reload** (S); **J4 profiles** (S–M).
7. **E2 EXPLAIN ESTIMATE cost hint** (S) in the SQL console and the queue.
8. **I1 "explain this" into the existing Claude session** (S) — Grafana/Datadog's best idea, free here.
9. **D3 anomaly glyph with the printed baseline** (M) — only after a week of A1.
10. **F4 global search / command palette** (M).

**Wow, worth it as demos or for the room:** J2 wall mode (S), E3 flamegraph (M–L), F2 fleet heatmap (S after A1), J1 theme files (S), C5 a quiet tone (S), D1 node compare (S).

**Wow but shallow / not now:** G3 Lua/WASM scripting (L), H4 ratzilla web twin, H3 SSH serving (M–L, security cost; only if a second on-call appears), J6 macros, an always-on AI pane, auto-posting webhooks.

**Writes to flag:** C4 webhook post, K4 Jira worklog, H3 (network listener), G1 (user-authored shell commands). Everything else stays read-only.

### Gaps
- Effort estimates are judgement, not measured; the project is ~40k lines and the owner's velocity is unknown to this researcher.
