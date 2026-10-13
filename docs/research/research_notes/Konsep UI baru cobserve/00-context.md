# Context for researchers: what cobserve is today (read this first)

cobserve is a Rust + ratatui 0.30 terminal monitor for on-call on a data platform
(Paysera). Repo: this folder's root (`DESIGN.md`, `README.md`, `GUIDE.md`, `src/ui/*.rs`,
`docs/screenshots/*.txt` are the current screens as plain text). ~40k lines of Rust.

It shows, in one terminal, read-only:
- View 1 NODES: every ClickHouse node's memory and CPU against its own capacity, as a tree
  node → user (a Redash query resolved to the real person) → query, with bars, sparklines (4 min
  history), insights ranked worst-first, a detail drawer, and the SQL under the cursor.
- View 2 QUEUE: the Redash RQ queue — running, waiting, stale jobs; the ClickHouse query each
  job became; the one write action in the app: cancel a Redash job (x then y).
- View 3 MAP: the fleet as cards (tiles) with bars + sparklines.
- View 4 TAPE: an event log of what changed (node hot/unreachable, runaways, queue backing up).
- View 5 AIRFLOW: every DAG's last 24 h (running/queued/failed, a timeline per DAG, task logs).
- View 6 JIRA: the user's own tickets in board columns, hours logged chart, time by ticket.
- View 7+ SESSIONS: Claude Code / OpenCode / shell / a read-only SQL console with autocomplete
  and "Claude writes the SQL" (ctrl+k), running in PTYs inside the app, up to 50 sessions.
- View 0 LOCAL: this Mac (Activity Monitor-like) + AI usage (tokens, cost, Claude plan limits).
- Everywhere: masthead with tabs, a "day line" with Islamic prayer times (owner is Indonesian),
  a band (FLEET / REDASH / LOCAL summary lines; cards on wide terminals), footer with keys.
- Desktop app: `desktop/` wraps cobserve in a wry (WebKit) window with xterm.js and its own
  header tabs + status chips. Notifications (macOS) for failed DAGs, red nodes, overdue tickets.
- Theme "Tarum" (batik dyes: soga brown-black ground, mori cream text, tarum indigo accent;
  red/amber/green only for severity). Light and mono variants. 24-bit/256/16-colour.

Hard rules the owner keeps: UI never blocks on network; never panics on resize; colour means
severity only; every percentage prints its denominator; every number traces to a formula
(DESIGN.md §5); read-only on ClickHouse. Target 120×36, works from 80×24, richer at 160×48.

Current NODES screen (120×36), as text:

```
  ❖ cobserve   ✖    1 nodes  2 queue  3 map  4 tape  5 airflow  6 jira  7 sessions  0 local      ● live   15:52:07 WIB
  ━━ Subuh 04:21 ━━━━ Terbit 05:33 ━━━━ Dzuhur 11:45 ━━━━ Ashar 14:48 ━●┄┄ Maghrib 17:50 · in 1h58m ┄┄┄┄ Isya 18:59 ┄┄

  FLEET   9 nodes · 1 hot   mem ━━━╸━━━━━━ 36.4% 280/768 GiB   cpu ━━━╺━━━━━━ 28.7% 55/192 cores   queries 15 · 4 ✕
  REDASH  16 waiting    ▇████ · oldest 3m43s ✖ · 6 running · ●●●●●●○ 6/7 workers busy · 3 stale              [2] queue

   NODE · USER → PERSON            MEM ▼                       GiB  CPU ▼                    cores  QUERIES  LONGEST
  ▌▾ clickhouse3       ▂▃▄▄▅▆▆▇▃▃   93.1%  ━━━━━━━━━━━━━╺  59.6/64   93.4%  ━━━━━━━━━━━━━╺ 14.9/16     4 ✕2    8m31s ✕
   ├─ r_redash → grigol.gankava     25.5%  ━━━╸━━━━━━━━━━     16.3   19.4%  ━━╸━━━━━━━━━━━    3.10        1    8m31s ✕
   ├─ r_redash → j.petrova          12.9%  ━━╺━━━━━━━━━━━     8.23    4.4%  ╸━━━━━━━━━━━━━    0.70        1    6m40s ✕
   ├─ airflow                        5.5%  ━╺━━━━━━━━━━━━     3.53   31.2%  ━━━━╸━━━━━━━━━    5.00        1      18s
   └─ server · caches · merges      47.4%  ━━━━━━╸━━━━━━━     30.3   36.5%  ━━━━━╺━━━━━━━━    5.84
   ▸ clickhouse-bi     ▆▆▃▅▆▆▄▅▆▆   67.2%  ━━━━━━━━━╸━━━━  43.0/64   71.7%  ━━━━━━━━━━╺━━━ 11.5/16     3 ✕1    6m40s ✕
   ▸ 4 healthy nodes folded · ch4 ch6 ch8 ch9 · all ≤ 22%                                                    ␣ unfold

  ─ INSIGHTS · tab to go to one ───────────────────────────────────────────────────────────────────
   ✖ clickhouse3    memory 93.1% · CPU 93.4% · the server itself holds 47%  +1 more
   ▲ clickhouse-bi  memory 67.2% ↗ full in ~18m · j.petrova holds 33%  +1 more
  ─ clickhouse3 · pay-ch-node-1:9000 ──────────────────────────────────────────────────────────────
  MEM 59.6/64 GiB  93.1%  ▂▃▄▅▆▇▇▃ → steady    CPU 14.9/16 cores  93.4%  ▇▇▇▇▇▆▇▇ → steady
   ↑↓  move   ⏎  open   c  SQL on it   ←→  fold   tab  insights   u  pivot   /  filter   s  sort   ␣  healthy
```

What the owner asked (translated from Indonesian): "The project is already pretty. But I want
it more modern and even better. Research in depth how the UI can be better — a different concept
from today, rebuilt from zero, is fine. How to have good visualisation. New cool features. And,
most important, that the user finds it easy to use."

So the research must produce *concrete, actionable* material: named tools to imitate (with what
exactly they do well), named crates with versions and maturity, patterns with ASCII examples
where possible, and trade-offs — not generalities. Everything must be feasible in a Rust/ratatui
terminal app (plus, optionally, its wry/xterm.js desktop shell). Cite sources with URLs.
