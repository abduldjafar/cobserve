# 07 · Features after the shell

The full table with sources, effort and the write flags is in the report
(`docs/research/reports/Konsep UI baru cobserve.md`, section *Fitur baru*); the research behind
each is in `docs/research/research_notes/Konsep UI baru cobserve/new-feature-ideas.md` and
`clickhouse-observability-features.md`. This is the order and what each needs from the shell.

## Wave 1 (phase 7)

| # | Feature | Needs | Note |
|---|---|---|---|
| A1 | **History store + replay**: every poll into `~/.local/state/cobserve/history.sqlite` (rusqlite, bundled, WAL, 24 h), `[` `]` over the real hour, graphs over 60 min | the replay keys of phase 5 | unlocks A2, A3, F2, D2, D3, K2 |
| A2 | Diff two minutes in replay (`d`): rows that moved more than X GiB, queries `+`/`−` | A1 | |
| A3 | Graph windows `4m / 1h / 6h / 24h` with the window printed | A1 | |
| E1 | **Query fingerprint**: `usually 42 s · p95 1m50 · ran 318× / 7 d` on the NODE panel's query line, from `system.query_log` by `normalized_query_hash`, polled every 30–60 s | a second poll class in `sources/clickhouse.rs` | the simulation already prints the line |
| H0 | **Health row** per node: parts per partition against `parts_to_delay_insert` / `parts_to_throw_insert`, delayed and rejected inserts, read-only replicas, Keeper session, disk free with *full in ~X*, uptime < 3 min | `system.asynchronous_metrics`, `system.metrics`, `system.disks`, `system.merge_tree_settings` | a fourth tile line or the NODE panel's closing line |
| H1 | Merges and mutations by name, splitting *server · caches · merges* | `system.merges`, `system.mutations` | |
| H2 | Failed queries panel: exception code, who, which Redash query | `query_log` type Exception*, `system.error_log` (24.8+) | a zoom under `:failed` until it earns a panel |
| B3+B4 | **Notes on the tape** (`n`, done in phase 3) and the **incident report** (`y` on the tape): markdown with every percentage and its denominator → OSC 52 and `~/incidents/` | — | |
| C2 | Snooze / maintenance window (`z`): 15 m / 1 h / until Subuh tomorrow | the day line knows the time | |
| C3 | Native terminal notifications (OSC 9 / 777 / 99) outside the desktop shell | `$TERM_PROGRAM` | |
| — | OSC 8 links on every row (job → Redash, DAG → Airflow, ticket → Jira, node → Grafana); `y` stays the fallback | the capability probe | |
| J3 | Hot-reload of `config.toml`, rules and themes (`notify`) | the wizard's config of phase 5 | |
| J4 | Profiles `prod / rig / fr13` (`:ctx`), the name in the top bar | config | |
| E2 | Cost hint `E`: `EXPLAIN ESTIMATE` → rows · parts · marks (runs under `readonly=1`) | — | |
| I1 | **Explain this → Claude**: `⏎` in the popover floats the session with the formula, inputs, 30 tape lines, the SQL, E1 and E2 in the prompt, readable before it is sent | phase 4's hand-off | |
| K1 | Redash schedule health: failing (`schedule_failures > 0`), overdue by the `2^failures` back-off | `/api/admin/queries/outdated`, `/api/queries` | verify the fields on the real instance first |
| K2 | Airflow *took longer than usual* (`▲ 2.8× usual (median 12m)`) from the last 30 run durations; Airflow 3.1 deadline alerts | A1 or the runs already read | |

## Wave 2 (phase 8)

| # | Feature | Note |
|---|---|---|
| D3 | Anomaly glyph `≈` with the printed baseline (`mem 61 % · usual 35 ± 8 %`), z > 3 for ≥ 3 polls against the same hour of the last 7 days | after a week of A1; colour stays severity-only |
| F2 | Fleet heatmap node × minute as a NODES zoom variant | A1 |
| D1 | Compare two nodes side by side (replica pairs) | |
| D2 | Queue waiting overlaid on fleet memory/CPU, one hour | A1 |
| J1 | Theme files `~/.config/cobserve/themes/*.toml` with the severity slots mandatory | |
| J2 | Wall mode `W`: the big numbers alone, denominators in normal text under them | |
| G1 | k9s-style plugins `plugins.toml` (`$NODE $QUERY_ID $PERSON $DAG_ID $TICKET`, output `popup \| session \| none`) | runs the user's shell: not read-only by nature, say so in the docs |
| E3 | Flame graph from `system.trace_log` | needs `allow_introspection_functions`, which a `readonly=1` user cannot set: check the profile first; otherwise export collapsed stacks |
| C1 | Watch rules `rules.toml` (`when = "node.mem_pct > 85 for 2m"`) | Sampler as the model |

## Not now, and why

Chat webhooks (a write to a chat; opt-in per rule if ever), Jira worklog from the app (a write),
`cobserve serve --ssh` (a network listener), Lua/WASM scripting (plugins cover it), a ratzilla web
twin (loses the PTY sessions), an always-on AI pane (the formulas are the source of truth).
