# cobserve — View 1 · NODES × USERS

**Design + implementation guide.** Written for an implementer with no prior context.
Read all of it before writing code; §5 (the math) and §10 (tests) are the contract.

---

## 0. What this is

A terminal UI (Rust, ratatui) for on-call that answers, on one screen, for a
ClickHouse fleet whose node count keeps growing:

1. **What is every node's resource state right now** — memory and CPU against that node's
   own capacity.
2. **Who is consuming it** — per node, per user, as a **percentage of that node's memory
   and CPU**, with the shared Redash account (`r_redash`) resolved to the real person.
3. **What the Redash queue looks like** — waiting jobs, oldest wait, worker saturation —
   because a "slow dashboard" is usually a full queue, not a slow database.

It is a sibling of FleetLens (the web app); it
reads ClickHouse **directly** so it keeps working when the web app is down. Views 1 and 2 are
in scope for this pass (§2.8 for view 2). Views 3–4 (FLOW, TAPE) are named in the header so
the navigation is stable, but they render a one-line "not built yet" placeholder.

A browser simulation of both views, with the exact keymap and the §5 math running in JS,
is the executable reference: https://claude.ai/code/artifact/9f6896b6-8377-42fc-95f3-5268ddf1c560

**Already decided, do not revisit:** Rust 2024, `ratatui = "0.30"`, `crossterm = "0.29"`,
`color-eyre` (all in `Cargo.toml`). Add: `tokio` (rt-multi-thread, macros, sync, time),
`reqwest` (json, rustls-tls; no default-features), `serde` + `serde_json`, `regex`.

---

## 1. The screen

Target 120×36. Minimum 100×30 (see §7 for what degrades below that).

```
┌ FLEETLENS ──────────────── ● LIVE 2s  12:41:07 · 9 nodes · sort pressure ▼ · [1]NODES [2]QUEUE [3]FLOW [4]TAPE ┐
│ REDASH QUEUE   12 waiting · oldest 1m40s ⚠ · workers 6/6 busy ⚠ · 2 failed/5m                        ⏎ open [2] │
├────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ▾ clickhouse3       mem 58.2 / 64 GB  91% ⚠      cpu 15.1 / 16 cores  94% ⚠      6 running · lag 0s · parts 1.2k│
│      USER → PERSON                MEM %                          CPU %                        QUERIES  LONGEST │
│    ▸ r_redash → grigol.gankava    25.3%  ▇▇▇▇▇░░░░░░░░░░░░░░░   19.4%  ▇▇▇▇░░░░░░░░░░░░░░░░    2      4m35s ✕ │
│    ▸ r_redash → j.petrova         12.8%  ▇▇▇░░░░░░░░░░░░░░░░░    4.4%  ▇░░░░░░░░░░░░░░░░░░░    1      2m44s ✕ │
│    ▸ airflow                       5.5%  ▇░░░░░░░░░░░░░░░░░░░   31.2%  ▇▇▇▇▇▇░░░░░░░░░░░░░░    1        37s   │
│    ▸ haris                         1.9%  ░░░░░░░░░░░░░░░░░░░░    1.8%  ░░░░░░░░░░░░░░░░░░░░    2        12s   │
│      server · caches · merges     45.5%  ▇▇▇▇▇▇▇▇▇░░░░░░░░░░░   37.2%  ▇▇▇▇▇▇▇░░░░░░░░░░░░░    —          —   │
│ ▾ clickhouse-bi     mem 43.5 / 64 GB  68%        cpu 12.0 / 16 cores  75%        9 running · lag 0s · parts 640│
│    ▸ r_redash → j.petrova         12.8%  ▇▇▇░░░░░░░░░░░░░░░░░    4.4%  ▇░░░░░░░░░░░░░░░░░░░    1      2m44s ✕ │
│      server · caches · merges     53.3%  ▇▇▇▇▇▇▇▇▇▇▇░░░░░░░░░   68.8%  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇░░░░░░    —          —   │
│ ▸ clickhouse7       mem 62%   cpu 40%   4 running · lag 12s ⚠                                                    │
│ ▸ clickhouse2       mem 41%   cpu 25%   3 running                                                                │
│ ▸ clickhouse5 NEW   mem 38%   cpu  8%   1 running                                                                │
│ ▸ 4 healthy nodes folded — ch4 ch6 ch8 ch9 (< 35%)                                                  space unfold │
├─ c3e51cb5 · grigol.gankava @ clickhouse3 ──────────────────────────────────────────────────────────────────────┤
│ Redash #7438 · started 12:36:32 on clickhouse3 · running 4m35s · 8.3 GiB · 2.1 cores · read 41.2 GB / 1.9 B rows │
│ WITH BankRecord AS ( SELECT BillOpId, Bank, multiIf( ( AccNr = 'LT48729…' AND Curr…                              │
├────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move   ⏎ expand   u pivot by user   / filter   s sort   p pause   1-4 view   q quit                           │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Regions (top → bottom, fixed unless noted)

| Region | Height | Content |
|---|---|---|
| Header | 1 | app name · live dot + poll interval · UTC clock · node count · current sort · view tabs |
| Queue strip | 1 | Redash queue summary (§6.3). Reads `queue unreachable` when the API fails — never blank. |
| Tree | `Min(8)` | node blocks (§2). Scrolls. |
| Detail drawer | 3 | the selected row's detail (§2.4). Empty drawer keeps its height — the tree must not jump. |
| Footer | 1 | key legend, contextual (changes when a filter or confirm prompt is active) |

Borders: single-line box drawing, one outer frame, hairline separators between regions.
No inner boxes around every widget — the tree is one continuous list.

---

## 2. The tree

The body is a **tree table** with three levels: **node → user → query**.

### 2.1 Node row (level 0)

```
▾ clickhouse3       mem 58.2 / 64 GB  91% ⚠      cpu 15.1 / 16 cores  94% ⚠      6 running · lag 0s · parts 1.2k
```

- `▾` expanded / `▸` collapsed. Name padded to the widest node name in the fleet.
- Memory and CPU **always print the denominator** (`58.2 / 64 GB`, `15.1 / 16 cores`). A
  percentage without its denominator is not allowed anywhere in this UI.
- If a denominator is unknown (see §6.1 fallback chain returns 0), print `mem 58.2 GB / —`
  and no percentage — never a fake `0%`.
- `NEW` badge: a node not present in the first snapshot of this session (§2.6).
- Severity tint on the whole row: worst of mem%/cpu% (thresholds §7).

### 2.2 User rows (level 1) — the point of the screen

Under an expanded node, one row per **initial user**, sorted by mem% desc:

```
▸ r_redash → grigol.gankava    25.3%  ▇▇▇▇▇░░░░░░░░░░░░░░░   19.4%  ▇▇▇▇░░░░░░░░░░░░░░░░    2      4m35s ✕
```

- `USER → PERSON`: the ClickHouse user, and — when the attribution regex (§6.4) finds a
  Redash `Username:` — an arrow to the real person. Plain users show just the user.
- `MEM %` and `CPU %` are **shares of THIS node's capacity** (§5). Bars are 20 cells.
- `QUERIES`: count. `LONGEST`: elapsed of the longest, with `✕` if runaway (§7).
- The **last row of every expanded node is the closing row**:

```
  server · caches · merges     45.5%  ▇▇▇▇▇▇▇▇▇░░░░░░░░░░░   37.2%  ▇▇▇▇▇▇▇░░░░░░░░░░░░░    —          —
```

  It is node usage minus the sum of the user rows (§5.3). **User rows + closing row = the
  node's percentage.** If they do not, the numbers are wrong, not the row.

### 2.3 Query rows (level 2)

`⏎` on a user row expands their queries, one per line, indented one more level:

```
      c3e51cb5   4m35s ✕   8.3 GiB  2.1c   WITH BankRecord AS (SELECT BillOpId, Bank, mul…
```

SQL is collapsed to one line (all whitespace runs → single space) and truncated to fit.

### 2.4 Detail drawer

Shows the **selected row**:

- Node selected → `name · host:port · shard/replica · version · uptime · 9 running · lag · parts`.
- User selected → `person (user) · N queries · Σ mem · Σ cores · longest query_id`.
- Query selected → line 1: `query_id · person · Redash #id (if any) · started HH:MM:SS on node
  · running Xs · mem · cores · read bytes / rows`; line 2: the collapsed SQL.

### 2.5 Default state, sorting, folding

- Sort key for nodes = **pressure** = `max(mem_pct, cpu_pct)`, then runaway count desc,
  then name. `s` cycles: pressure → mem → cpu → name.
- On first snapshot: the top node is **expanded**, all others collapsed.
- **Folding:** nodes with `mem_pct < 35 && cpu_pct < 35 && runaway == 0 && lag_s < 10`
  collapse into one line `▸ N healthy nodes folded — names (< 35%)`. `space` toggles the
  fold. This is what keeps 40 nodes on one screen; it must exist from the first version.
- Expansion state is keyed by **node name**, so a re-sort does not lose what you opened.
- Selection is keyed by row identity (node name / user / query_id), not by index, so a
  refresh under the cursor keeps the cursor on the same thing. If the selected row
  disappears (query finished), selection moves to its parent.

### 2.6 Dynamic node count

- The node list is **discovered**, not configured (§6.1). It may grow while running.
- A node first seen after the session's first snapshot gets `NEW` for the rest of the
  session.
- A node missing from a poll (timeout / connection refused) stays in the list with its
  numbers replaced by `?` and a `↯ unreachable 12s` note; it is dropped only after it has
  been unreachable for 5 minutes AND it is absent from `system.clusters`.

### 2.7 Pivot (`u`)

`u` flips the tree to **user → node → query** using the same snapshot. Level-0 rows become
users (across the fleet), level-1 rows become the nodes they run on, each with that node's
mem%/cpu% for this user. Sort: Σ memory bytes desc. This answers "who is burning the fleet"
without a second data path. `u` again flips back.

### 2.8 View 2 — QUEUE: the people behind the counts

The strip on view 1 is deliberately one line. `2` (or `⏎` on the strip) opens the people:

```
┌ FLEETLENS ──────────────── ● LIVE 3s  12:41:07 · redash.example.net · [1]NODES [2]QUEUE [3]FLOW [4]TAPE ┐
│ queries              12 waiting   oldest 1m40s ▲   workers 6/6 busy ▲   failed/5m 2                       │
│ scheduled_queries     3 waiting   oldest   22s     workers 2/2 busy     failed/5m 0                       │
│ periodic              0 waiting   oldest    —      workers 0/1 idle     failed/5m 0                       │
│ WAITING                       #   WAIT      USER → PERSON              DATA SOURCE     QUERY               │
│    1   1m40s ▲  r_redash → r.simonyte      clickhouse-bi    #8091 July close pack · by product            │
│    2   1m12s ▲  r_redash → j.petrova       clickhouse-bi    #8113 AML dashboard · by country              │
│    3     58s    r_redash → m.kairys        clickhouse2      #7711 FX exposure · intraday                  │
│ RUNNING · on a worker         WORKER  RUNNING   USER → PERSON     DATA SOURCE   QUERY          → CLICKHOUSE│
│    1   4m35s ✕  r_redash → grigol.gankava  clickhouse3      #7438 Gateway transfers  → clickhouse3 · c3e5…│
│    2   2m44s ✕  r_redash → j.petrova       clickhouse-bi    #8585 AML dashboard      → clickhouse-bi · 8a1…│
├─ queue · queries ────────────────────────────────────────────────────────────────────────────────────────┤
│ 6/6 workers busy · 3 of them hold runaway ClickHouse queries (clickhouse3, clickhouse-bi) → why it's full │
├──────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ move   ⏎ jump to the ClickHouse query   / filter   p pause   1 back to nodes   ? help                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- Three queue rows first (counts, oldest wait, worker saturation, failures), then **WAITING**
  — every queued job with its person, data source, Redash query number and name, sorted by
  wait — then **RUNNING**, every job on a worker with the **ClickHouse node and `query_id` it
  became**. That arrow is the stitch: a Redash job that has started IS a `system.processes`
  row somewhere, and `⏎` jumps to it on view 1 (expanding its node and user).
- A **waiting** job has not reached ClickHouse yet: there is nothing to kill, the wait *is*
  the queue. The drawer says so, plus how many jobs are ahead of it.
- The drawer on a queue row answers the question the DA actually has: *"workers are all
  busy because N of them hold runaway ClickHouse queries on these nodes"* — or "workers are
  keeping up" when they are.
- Same severity rules as everywhere: wait ≥ 60 s amber, ≥ 180 s red; a running job inherits
  the runaway state of its ClickHouse query.

---

## 3. Keys

| Key | Action |
|---|---|
| `↑` `↓` / `j` `k` | move selection |
| `⏎` | expand / collapse the selected node or user |
| `←` `→` | collapse / expand (vim-style tree) |
| `space` | toggle the healthy fold |
| `u` | pivot node↔user |
| `s` | cycle sort |
| `/` | filter (substring over node, user, person, SQL); `Esc` clears |
| `p` | pause polling (header dot goes hollow, clock freezes) |
| `1` … `4` | view tabs (2–4 are placeholders this pass) |
| `?` | help overlay |
| `q` / `Ctrl-C` | quit |

Not in this pass: `k` kill, `a` ask Klikas, mouse. Leave them **out of the footer** —
a listed key that does nothing is a bug.

---

## 4. Data model (Rust)

```rust
pub struct FleetSnapshot {
    pub taken_at: std::time::SystemTime,
    pub nodes: Vec<NodeSnapshot>,      // discovered order; UI sorts
}

pub struct NodeSnapshot {
    pub name: String,                  // host_name from system.clusters
    pub host: String, pub port: u16,
    pub shard: u32, pub replica: u32,
    pub version: String,
    pub reachable: bool,               // false → numbers below are stale/unknown
    pub mem_total: Option<u64>,        // bytes; None when the fallback chain gave 0
    pub mem_used: u64,                 // MemoryResident
    pub cores: Option<f64>,            // None when unknown
    pub cpu_busy_cores: Option<f64>,   // node-wide busy cores (see §5.2)
    pub running: u32,
    pub lag_s: u64,                    // max absolute_delay over system.replicas
    pub parts: u64,                    // active parts
    pub queries: Vec<QueryRow>,
}

pub struct QueryRow {
    pub query_id: String,
    pub user: String,                  // initial_user
    pub person: Option<String>,        // from Username: in the SQL (§6.4)
    pub redash_query_id: Option<u64>,
    pub elapsed_s: f64,
    pub memory_bytes: u64,
    pub cores: f64,                    // §5.2
    pub read_rows: u64, pub read_bytes: u64,
    pub sql: String,                   // raw; collapse at render time
    pub cpu_time_us: u64,              // cumulative counter, kept for the delta
}

/// Derived per node, never stored: §5.
pub struct UserSlice {
    pub user: String, pub person: Option<String>,
    pub mem_pct: Option<f64>, pub cpu_pct: Option<f64>,
    pub queries: Vec<QueryRow>,        // sorted elapsed desc
    pub longest_s: f64, pub runaway: bool,
}

pub struct QueueStatus {
    pub reachable: bool,
    pub queues: Vec<QueueRow>,          // queries / scheduled_queries / periodic
    pub jobs: Vec<Job>,                 // waiting + running, with people (§6.3)
    pub names_available: bool,          // false → WAITING shows counts only
}
pub struct QueueRow { pub name: String, pub waiting: u32, pub oldest_wait_s: Option<u64>, pub workers_busy: u32, pub workers_total: u32, pub failed_5m: u32 }
pub enum JobState { Queued, Started }
pub struct Job {
    pub id: String, pub state: JobState, pub queue: String,
    pub person: Option<String>,         // resolved user email
    pub redash_query_id: Option<u64>, pub query_name: Option<String>, pub data_source: Option<String>,
    pub age_s: u64,                     // waiting time, or running time
    pub ch_node: Option<String>, pub ch_query_id: Option<String>,   // the stitch, for Started
}
```

`model.rs` holds these plus **all the math in §5 as pure functions** with unit tests.
Nothing in `model.rs` does I/O.

---

## 5. The math — this is the contract

### 5.1 Memory share

```
user.mem_pct   = Σ memory_usage(queries of user on node) / node.mem_total * 100
```

- Denominator = `node.mem_total` from the fallback chain in §6.1. If `None`, `mem_pct` is
  `None` and renders as `—`; never divide by a guess.
- Group by **`initial_user`**, and only rows with `is_initial_query = 1` (the SQL in §6.1
  already filters). A distributed query fans out to sub-queries on other nodes that appear
  in *their* `system.processes` under the same user — counting those would charge the user
  twice. This is the single most likely way to get the percentages wrong.

### 5.2 CPU share

Per query, cores in use:

```
cores(q) = Δ cpu_time_us(q) / Δ wall_us               -- between two consecutive polls
fallback  = cpu_time_us(q) / (elapsed_s * 1e6)         -- first sighting of a query
```

where `cpu_time_us = greatest(OSCPUVirtualTimeMicroseconds, UserTime+SystemTime)` from
`ProfileEvents` (§6.1 SQL returns it). The delta form is "right now"; the fallback is the
average since the query started (what FleetLens shows) — use it only until a second sample
exists. Clamp to `[0, node.cores]`.

```
user.cpu_pct   = Σ cores(q) / node.cores * 100          -- None if node.cores is None
node.cpu_pct   = node.cpu_busy_cores / node.cores * 100
```

`node.cpu_busy_cores` comes from the `*Normalized` async metrics (§6.1, `server_cpu_percent`)
× `cores`; when a node reports neither family, difference `server_cpu_time_us` between
polls instead (same delta trick, node-wide).

### 5.3 The closing row

```
server.mem_pct = node.mem_used / node.mem_total * 100  −  Σ user.mem_pct
server.cpu_pct = node.cpu_pct                           −  Σ user.cpu_pct
```

Clamp each to `≥ 0` (query memory can momentarily exceed `MemoryResident` accounting).
**Invariant, unit-tested:** `Σ user.mem_pct + server.mem_pct == node mem%` (±0.1) and the
same for CPU. If the invariant cannot hold because a denominator is `None`, the whole column
renders `—` for that node — a partial column is a lie.

### 5.4 Runaway

A query is **runaway** when `elapsed_s ≥ 30` **or** `memory_bytes ≥ 0.8 × per-query limit`
where the limit is `max_memory_usage` from `system.settings` on that node (fallback 9 GiB —
the fleet's known ceiling — if unreadable). Runaway rows carry `✕` and the red tint.

---

## 6. Data sources

All ClickHouse access over the **HTTP interface**, `POST` body = SQL, `default_format=JSONEachRow`,
query settings `readonly=1&max_execution_time=2`, credentials via `X-ClickHouse-User` /
`X-ClickHouse-Key` headers. Per-request timeout 1.5 s. Poll every node **concurrently**; a
slow node must not delay the others. `POLL_MS` default 2000.

### 6.1 Per node, every poll — two statements

**Capacity + node-wide load** (lifted from FleetLens `fleetMetrics.ts`; keep the fallback
chains — they exist because real nodes in this fleet lack some metrics):

```sql
SELECT
  coalesce(
    (SELECT value FROM system.asynchronous_metrics WHERE metric = 'OSMemoryTotal' LIMIT 1),
    (SELECT value FROM system.asynchronous_metrics
       WHERE metric = 'CGroupMemoryTotal' AND value > 0 AND value < pow(2, 50) LIMIT 1),
    (SELECT toFloat64(toUInt64OrZero(value)) FROM system.server_settings
       WHERE name = 'max_server_memory_usage'
         AND toUInt64OrZero(value) > 0 AND toUInt64OrZero(value) < pow(2, 50) LIMIT 1),
    0
  ) AS server_memory_total_bytes,
  (SELECT value FROM system.asynchronous_metrics WHERE metric = 'MemoryResident' LIMIT 1)
    AS server_memory_used_bytes,
  (SELECT if(cpu_busy IS NULL, NULL, round(100 * least(1.0, greatest(0.0, cpu_busy)), 2))
   FROM (
     SELECT coalesce(
       if(countIf(metric IN ('OSUserTimeNormalized','OSSystemTimeNormalized','OSNiceTimeNormalized')) > 0,
          sumIf(value, metric IN ('OSUserTimeNormalized','OSSystemTimeNormalized','OSNiceTimeNormalized')), NULL),
       if(countIf(metric IN ('CGroupUserTimeNormalized','CGroupSystemTimeNormalized')) > 0,
          sumIf(value, metric IN ('CGroupUserTimeNormalized','CGroupSystemTimeNormalized')), NULL)
     ) AS cpu_busy FROM system.asynchronous_metrics
   )) AS server_cpu_percent,
  (SELECT coalesce(
     nullIf(countIf(metric LIKE 'OSUserTimeCPU%'), 0),
     nullIf(countIf(metric LIKE 'CGroupUserTimeCPU%'), 0),
     nullIf(countIf(metric LIKE 'CPUFrequencyMHz\\_%'), 0),
     nullIf(toUInt64(ceil(maxIf(value, metric = 'CGroupMaxCPU' AND value > 0 AND value < 4096))), 0)
   ) FROM system.asynchronous_metrics) AS server_cpu_cores,
  (SELECT greatest(
     sumIf(value, event = 'OSCPUVirtualTimeMicroseconds'),
     sumIf(value, event = 'UserTimeMicroseconds') + sumIf(value, event = 'SystemTimeMicroseconds')
   ) FROM system.events) AS server_cpu_time_us,
  (SELECT count() FROM system.processes) AS active_queries,
  (SELECT max(absolute_delay) FROM system.replicas) AS replica_lag_s,
  (SELECT count() FROM system.parts WHERE active) AS active_parts,
  (SELECT toUInt64OrZero(value) FROM system.settings WHERE name = 'max_memory_usage') AS max_memory_usage,
  version() AS version,
  uptime() AS uptime_s
```

**Running queries** (lifted from FleetLens `liveQueriesFleet.ts` `PER_NODE_SQL`):

```sql
SELECT
  query_id,
  initial_user AS user,
  query,
  elapsed AS elapsed_s,
  memory_usage,
  read_rows, read_bytes,
  greatest(
    ProfileEvents['OSCPUVirtualTimeMicroseconds'],
    ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']
  ) AS cpu_time_us,
  trim(extract(query, 'Username:\\s*([^,]+)')) AS redash_user,
  extract(query, 'query_id:\\s*(\\d+)')        AS redash_query_id
FROM system.processes
WHERE is_initial_query = 1
  AND query NOT LIKE '%FROM system.processes%'
  AND query NOT LIKE 'KILL QUERY%'
ORDER BY elapsed DESC
```

(`extract` runs server-side; keep the Rust regex in §6.4 too, for fake mode and tests.)

### 6.2 Discovery — every 60 s and at start

```sql
SELECT cluster, shard_num, replica_num, host_name, host_address, port
FROM system.clusters
WHERE cluster = {cluster:String}
ORDER BY shard_num, replica_num
```

Run against **every seed** in `CH_SEED_URLS`; union the results by `host_name`. Seeds that are
not in any cluster are still polled (a standalone node is a node). The fleet is
`seeds ∪ discovered`. The HTTP port from `system.clusters` is the native port; assume HTTP =
`CH_HTTP_PORT` (default 8123) unless the seed URL for that host says otherwise.

### 6.3 Redash queue — every 3 s

`GET {REDASH_URL}/api/admin/queries/rq_status` with header `Authorization: Key {REDASH_ADMIN_API_KEY}`.

Shape as known (Redash ≥ 10, RQ) — **verify against the live instance first thing, and
capture one real response into `tests/fixtures/rq_status.json`**:

```json
{
  "queues":  { "queries": { "name": "queries", "queued": 12, "started": [ { "id": "...",
               "origin": "queries", "enqueued_at": "...", "started_at": "...",
               "meta": { "query_id": 7438, "user_id": 42, "data_source_id": 3 } } ] },
               "scheduled_queries": { ... }, "periodic": { ... } },
  "workers": [ { "name": "...", "state": "busy", "current_job": "...", "queues": "queries" } ]
}
```

Derive: `waiting = Σ queued`, `oldest_wait_s` = now − min(`enqueued_at`) over queued jobs if
the API lists them (if it only gives counts, show `oldest —`), `workers_busy/total` from
`workers[].state`, `failed_5m` from `/api/admin/queries/rq_status` if present else omit the
segment. If Redash is **< 10** the queue is Celery and the endpoint is
`/api/admin/queries/tasks` with a different shape — branch on `/api/config` → `version`.

**Who is running — from the API.** `started[].meta` carries `query_id` and `user_id`;
resolve names through `GET /api/users/{id}` and `GET /api/queries/{id}` (name, data_source_id
→ `GET /api/data_sources`) with an in-memory cache keyed by id — these do not change
mid-session. That is enough for the RUNNING half of view 2.

**Who is waiting — probably NOT from the API.** `rq_status` reports `queued` as a *count*;
job details for queued jobs live in Redis under RQ's keys (`rq:queue:<name>` → job ids,
`rq:job:<id>` → hash with `data`/`meta`, where `meta` again has `query_id` and `user_id`).
Verify on the live instance first: if `rq_status` does list queued jobs, use it; if it
does not, read Redis directly with `REDIS_URL` (read-only commands `LRANGE` + `HGETALL`,
crate `redis`). Without Redis access the WAITING list degrades to `12 waiting · names
unavailable (no REDIS_URL)` — a count, never an invented name. `enqueued_at` in the job
hash gives the real wait time; without it, show `—`.

**Fail soft:** any error → `QueueStatus { reachable: false, .. }` and the strip prints
`REDASH QUEUE  unreachable (HTTP 401)`. The app must never exit because Redash is down.

### 6.4 Attribution

Redash writes a comment into every query it runs. Two regexes (same as FleetLens):

```
person          : Username:\s*([^,]+)        → trim → "grigol.gankava@example.net"
redash_query_id : query_id:\s*(\d+)
```

Display the person as the local part before `@` when the domain is the organisation's own
(`EMAIL_DOMAIN`), full address otherwise. A user that is not `r_redash` and has no `Username:` shows only the user.

---

## 7. Rendering rules

**Colour carries severity and nothing else.** Users are distinguished by rows, never by hue.

| Signal | none | warn (amber) | crit (red) |
|---|---|---|---|
| node mem % / cpu % | < 75 | ≥ 75 | ≥ 90 |
| query elapsed | < 5 s | ≥ 5 s | ≥ 30 s (runaway) |
| user mem % of a node | < 20 | ≥ 20 | ≥ 40 |
| replica lag | < 10 s | ≥ 10 s | ≥ 60 s |
| queue waiting / oldest | < 60 s | ≥ 60 s or workers all busy | ≥ 180 s |

- Bars: 20 cells, `▇` filled, `░` empty, filled = `round(pct / 5)`. Bar colour follows the
  row's severity; otherwise the default foreground at reduced intensity.
- Numbers: bytes → `1.2 GiB` (one decimal, binary units); percent → `25.3%`; durations →
  `12s`, `4m35s`, `1h02m`. Right-align numeric columns.
- Clock in the header is **UTC**, `HH:MM:SS`, refreshed every second even while paused.
- Truncate with `…`, never wrap, never let a row exceed one line.
- **Width < 100:** drop the CPU bar (keep the number), then `LONGEST`, then `QUERIES`.
  **Height < 30:** detail drawer shrinks to 1 line, then disappears. Never panic on resize.

---

## 8. Architecture

```
src/
  main.rs        tokio runtime, terminal setup/teardown (color-eyre panic hook restores it),
                 the event loop: select over key events, ticks, and source channels
  app.rs         App state + `update(&mut self, Event)`: pure state machine, no I/O
  ui.rs          `draw(&App, &mut Frame)`: layout + widgets, reads App only
  model.rs       types (§4) + math (§5) as pure functions — 100% unit-tested
  tree.rs        flattening (node→user→query, and the `u` pivot) into rows; selection by id
  attrib.rs      the two regexes + person display rule
  sources/
    clickhouse.rs  discovery + per-node polling (reqwest, concurrent, timeouts)
    redash.rs      rq_status polling, fail-soft
  fake.rs        `FAKE=1` → a generator that emits believable snapshots (9 nodes, ~30
                 queries, one runaway, one NEW node appearing after 20 s, queue backed up)
```

Event flow: sources run as tokio tasks and send `Event::Snapshot(FleetSnapshot)` /
`Event::Queue(QueueStatus)` on an `mpsc` channel; the loop applies them to `App` and redraws.
Key events come from `crossterm::event::EventStream`. One redraw per event, plus a 1 s tick
for the clock. **The UI never blocks on the network.**

State in `App`: current snapshot, previous snapshot (for CPU deltas), queue status, first-seen
set (for `NEW`), expansion set, selection id, sort, pivot, filter, paused, view.

---

## 9. Configuration — environment only, nothing on disk

| Variable | Meaning | Default |
|---|---|---|
| `CH_SEED_URLS` | comma-separated `http://host:8123` seeds | required |
| `CH_CLUSTER` | cluster name for `system.clusters` discovery | required |
| `CH_USER` / `CH_PASSWORD` | a **read-only** ClickHouse user | required |
| `CH_HTTP_PORT` | HTTP port for discovered hosts | `8123` |
| `REDASH_URL` / `REDASH_ADMIN_API_KEY` | Redash admin API | optional — strip says so if absent |
| `REDIS_URL` | Redash's RQ Redis, read-only — names of **waiting** jobs (§6.3) | optional — WAITING shows counts only |
| `POLL_MS` | ClickHouse poll interval | `2000` |
| `FAKE` | `1` → no network, generated data | unset |

Never print credentials, not even in error messages (redact the `X-ClickHouse-Key` header
if you log requests).

Since v1, a server can have a login of its own — from a credential file named on the command
line, or in its seed URL. See §13.

---

## 10. Tests — what must exist before this is done

`model.rs` (plain `#[test]`, no terminal):
- percentages close: `Σ user + server == node` for mem and cpu, on a fixture with 3 users.
- a node with `mem_total: None` yields `mem_pct: None` for every row, never `0`.
- cores: delta form when two samples exist, average form on first sighting, clamp to `cores`.
- runaway: elapsed ≥ 30 s; memory ≥ 0.8 × limit; fallback limit 9 GiB when unreadable.
- fold rule: exactly the nodes below every threshold with no runaway are folded.
- sort: pressure desc, runaway count desc, name asc — stable.
- `NEW`: a node absent from the first snapshot and present later is flagged; one present
  from the start never is.

`tree.rs`:
- selection survives a refresh that reorders nodes; a vanished query moves selection to
  its user; the pivot round-trips (node→user→node yields the original rows).

`attrib.rs`: the two regexes on real-shaped Redash comments, including a `Username:` with a
trailing comma and one with no `query_id:`.

`sources/redash.rs`: parse `tests/fixtures/rq_status.json` (captured from the real
instance); a 401 and a connection refused both yield `reachable: false`.

`ui.rs` with `ratatui::backend::TestBackend`: the fake snapshot renders at 120×36, 100×30 and
80×24 without panicking; the header, queue strip, one node row, one user row and the closing
row appear in the buffer.

---

## 11. Build order

1. `FAKE=1 cargo run` shows the full screen from generated data — layout, tree, keys,
   fold, pivot, drawer. **No network until this looks right.** Reuse the ASCII in §1 as the
   acceptance target.
2. `model.rs` math + tests (§5, §10). Do this before touching a real cluster.
3. `sources/clickhouse.rs` against one seed, no discovery. Compare a user's mem% with the
   FleetLens web page's Consumption view for the same moment — they must agree.
4. Discovery via `system.clusters`; start the app with one seed and watch all nodes appear.
5. `sources/redash.rs` — capture a real `rq_status` first, then parse it. Then view 2:
   RUNNING from the API; WAITING from Redis if `REDIS_URL` is given, counts otherwise.
6. Attribution end-to-end: a Redash query shows `r_redash → person`.
7. Resize degradation, `?` help overlay, `p` pause, filter.

**Definition of done:** steps 1–7, `cargo test` green, `cargo clippy -- -D warnings` clean,
and a 30-second screen recording (or three screenshots: 120×36, 100×30, pivot view) of it
running against the real fleet. The percentages on screen must be explainable by §5 — if a
number cannot be traced to a formula here, it is a bug.

---

## 12. Gotchas — read before step 3

- `is_initial_query = 1` is the dedup. Remove it and every distributed query counts N times.
- `ProfileEvents` are **cumulative counters** for the life of the query; the delta needs the
  previous snapshot keyed by `query_id`.
- `system.asynchronous_metrics` refreshes about once a second; polling faster than 1 s buys
  nothing.
- The HTTP interface truncates long `GET ?query=` URLs — always `POST` the SQL as the body.
- `max_server_memory_usage = 0` means "auto", which is why the fallback chain rejects 0.
- Some nodes in this fleet expose **no** `OS*` CPU families at all; `server_cpu_time_us`
  from `system.events` is the only CPU signal there — hence the delta path in §5.2.
- Terminal restore on panic: install the color-eyre hook **before** entering raw mode, and
  make the hook leave raw mode / the alternate screen, or a crash leaves the user's shell
  unusable.

---

## 13. After v1: what this version adds, and where it departs from the text above

Nothing here changes a §5 number. Every addition is derived from what §5 already computes,
or from a slope over the last minutes of it.

### Added

- **Insights** (`src/insight.rs`), under the tree on view 1: one short line per subject in
  trouble — a node, the Redash queue, a Redash query running twice — worst first, its subject in
  a column of its own. A node's line is its worst finding (down, memory or CPU and who holds
  it, a forecast, a query near its own limit, long queries, lag) with `+N more` for the rest; no
  query is named twice. The drawer shows every finding of the chosen line with its numbers.
  Merely interesting things (the heaviest user, slow polls, new nodes) are left to the screen
  elsewhere, and a quiet fleet is one line. `tab` focuses them, `⏎` jumps to the row. CPU is
  judged over 10 s, not one poll.
- **History** (`src/history.rs`): four minutes of every node, user row, query and queue.
  Sparklines (each cell the worst moment of its slice, gaps left as gaps), trend arrows from a
  least-squares slope over 60 s, forecasts only once 30 s of history exist.
- **TAPE**, view 4 (`src/tape.rs`): what changed. Severity changes need two polls in a row and
  a margin below the line before they are called over (3 points for memory, 6 for CPU), so a
  node at 75% does not write a line a poll. A query's end is listed only when it was a runaway
  or near its limit; vanishing at ≥ 95% of its limit reads *probably killed*.
- **MAP**, view 3: the fleet as severity-framed tiles. It takes the slot §0 named FLOW; FLOW
  remains free for a Redash → ClickHouse flow view if one is wanted.
- **Progress and ETA** from `system.processes.total_rows_approx`, plus `written_rows`,
  `peak_memory_usage`, `query_kind` and the query's own `Settings['max_memory_usage']`. All of
  them exist on 24.10 (checked on the local rig); a server that refuses them gets §6.1's
  statement for the rest of the session.
- **Poll latency** per node, in the drawer and the insights.
- **Theme** (`src/theme.rs`): 24-bit, 256- and 16-colour renderings of one palette, `THEME=light`,
  `THEME=mono` and `NO_COLOR`.
- **A login per server** (`src/config.rs`): `--credential FILE` names a YAML file with the
  servers and a user and password for each (`credentials.example.yaml`), plus optionally the
  cluster, a `default_login` and the Redash settings; `http://user:password@host:8123` in
  `CH_SEED_URLS` does the same without a file. A password is only ever sent to its own server:
  a host discovery finds uses the default login (the file's, or `CH_USER` / `CH_PASSWORD`) and,
  without one, is shown as *not polled* with a note instead of being asked. The file wins over
  the environment, and `CH_SEED_URLS` adds to its servers. Errors in it name the key and the
  line, never a value — a password in the wrong place is not echoed — and a file other users
  can read is flagged in the footer.
- **Redash, read the way its admins' queue script reads it** (`src/sources/redash.rs`): per
  queue what runs, what waits and what is stale; each job's person by name and address
  (`/api/users/{id}`), its data source by name and **type** (one `/api/data_sources` call for
  all of them), its saved query by name and **SQL** (`/api/queries/{id}`, read again after ten
  minutes, since queries get edited). The version comes from `/api/config`, asked once. All of
  it read-only — but for the one cancel below, asked for and confirmed on view 2.
- **Cancelling a Redash job** (`src/sources/redash.rs`, `src/app.rs`, `src/ui/drawer.rs`): what
  the admins' script does, one job at a time. `x` on a job of view 2 puts the question in the
  drawer — about that job by its id, as the list moves under the cursor — with what a cancel
  does to it; `y` sends `DELETE /api/jobs/{id}` with the admin key from the task that polls
  Redash, which reads the queue again at once; any other key (or a click) keeps it, and does
  nothing else. A waiting job leaves its queue, a running one is stopped on its worker (its row
  marked `⊘` until the worker has let go, ten minutes at most), a leftover leaves RQ's started
  list. The house rule is the script's: a job on ClickHouse is cancelled only when asked of it,
  and the question and the notice say that its query runs on in ClickHouse — on which node, when
  the stitch found it — until a `KILL QUERY` there, which nothing here sends: ClickHouse stays
  read-only. Every cancel Redash took is a line on the tape (amber when its query runs on). A job id that is not RQ's (letters, digits, `-`, `_`) is not put into a path;
  Redash's 500 for a job it no longer has reads *it may have just finished*.
- **STALE**, on view 2: what RQ's started list holds although no worker runs it — a job whose
  worker died stays in that list, and without a time limit it stays for months (the screenshot
  this was built from had five such jobs, 163 to 177 days old, under RUNNING). Each says why:
  *cancelled*, *over a day old*, or *no worker holds it*. One that ClickHouse still runs is
  marked so, and `⏎` goes to it.
- **SESSIONS**, view 7 (`src/claude.rs`, `src/pty.rs`): a program in a pseudo-terminal — the
  official `claude`, `opencode`, or the user's shell — its screen emulated (`vt100`) and drawn
  in the well under the shelf, a card for every node on the shelf. Each signed in
  its own way, Claude with the user's own plan; no API is called from here, and
  `ANTHROPIC_API_KEY` and the monitor's credentials are taken out of every program's
  environment. Up to fifty sessions, numbered on from 7 after the views' 1 to 6 — a digit picks
  the first three, `ctrl+\` then the arrows walk through all of them, `ctrl+\ /` finds one by its
  name, folder, kind, server or number — each its own
  program, conversation and working directory, listed beside the pane like a terminal's tabs,
  the one on screen raised and marked — what it runs, name (the user's, the title its program sets unless
  that is only the program's name, else the folder),
  directory and git branch (read from `.git/HEAD`, a worktree's `.git` file followed, every few
  seconds) — on a narrow terminal a bar of tabs over it; a session that rings while it is not
  on screen is marked. The more there are the closer the list packs them — two lines and a gap
  each, then two lines, then a line each with the one on screen keeping its folder — and it
  follows the one on screen, saying how many more are above and below (the bar shows the tabs
  round it and how many either side). A kept session's program starts only when it is first
  shown, so fifty listed are not fifty running. On the shelf, a card for every node — those in trouble first, the
  rest in view 1's order: its mark (§7's colours), name and lag when it is behind, then its
  memory and its CPU each as a thin bar (`━`, half-cell steps) and its share, under a legend
  at the left. The cards are as wide as the widest needs and the width allows, all alike so
  their bars compare; as many as fit, then one with how many more and how high they go. A
  click on a card opens its node on view 1. Under 28 rows the cards give way to one line on
  the shelf, each node in a few words (`✖ clickhouse3 mem 91% cpu 95%`). A new session's folder is chosen in a picker shaped like a file
  explorer (`folders.rs`): clickable path, the folders here, a click goes in, a search below by
  name (breadth-first, six levels, bounded in folders read and in time, package and build
  folders skipped, on a thread of its own and dropped when a newer one starts) or a typed path
  completed as a shell does; above it, what the session runs, and under the way to open it
  here the conversations had in that folder and the way to every folder's. Every PTY is sized to the pane,
  so switching never shows a layout for another size. The terminal queries a full-screen
  program makes (cursor position, device attributes, default colours) are answered, and the
  rest (OpenCode asks for many more) left to time out as an older terminal would; `ctrl+z` is
  held back from Claude and OpenCode, since nothing in the pane could resume them — a shell
  gets it, for its own jobs. Keys reach the program as an xterm sends them. Where the terminal
  speaks the kitty keyboard protocol — asked once at the start, given back on the way out, its
  escape codes made unambiguous and nothing more — Enter with shift, ⌘ or ctrl comes through as
  itself, as it does for Claude Code run on its own; that, and ⌥⏎, reaches Claude and OpenCode
  as `ctrl+j`, the line feed both take as a new line in the prompt, while a shell gets Enter
  as an xterm would send it. The footer names the key for a new line: `⇧⏎`, or `ctrl+j` where
  the terminal cannot tell.
- **Sessions kept, and conversations taken up** (`src/saved.rs`, `src/conversations.rs`): the
  list of sessions — what each runs, where, its name and the conversation it is in — is kept in
  `$XDG_STATE_HOME/cobserve/sessions.json` (`~/.local/state/cobserve/` without it, mode 600)
  as it changes, and on the way out, a closed terminal (SIGHUP) or a SIGTERM included. The next
  run lists them again, marked `↻`, and each takes its conversation up when it is first shown:
  `claude --resume <id>` (started under `--session-id`, so its id is known from the start, and
  read back from `~/.claude/sessions/<pid>.json` while it runs — a `/clear` moves it on), and
  `opencode --session <id>` once `opencode session list` has named it, else `--continue`. A
  conversation never typed into is started again under its id. **Past conversations**
  (`ctrl+\ p`, or the sidebar's entry, or the picker's row) lists Claude Code's from
  `~/.claude/projects` — its title (`/rename`) or first prompt, its folder, how long ago — and
  OpenCode's from `opencode db`, newest first; one is taken up in a new session in its own
  folder, as a copy (`--fork-session`, `--fork`) when it is still open in another terminal.
  Only what names a conversation is read, nothing is written there, and none of it leaves the
  machine. The arguments are added to `claude` and `opencode` themselves only, never to a
  command of the user's (`CLAUDE_CMD`) that may not take them.
- **The day line and prayer times** (`src/prayer.rs`, `src/ui/day.rs`): under the masthead on
  every view, the day at a place from Subuh to Isya — each time a stop, evenly spaced (a stretch
  of the day as it is lived, not of the clock), the present a point that moves with the clock
  within its stretch, the way behind solid and the way ahead dotted, the next stop with how far
  off it is. Computed here from the sun by Kemenag's criteria — Subuh at 20° below the horizon,
  Isya at 18°, Terbit and Maghrib at 1°, Ashar Shafi'i, Dzuhur once the disc has crossed the
  meridian, two minutes of ihtiyat rounded up (Terbit off, rounded down) — and held by the tests
  to Kemenag's tables for Jakarta and Medan; Jumat on a Friday. The place is `PRAYER_CITY` (a
  list of Indonesian and a few other cities) or `PRAYER_AT=lat,lon`, else the city of the
  machine's time zone (`zone1970.tab`), followed as the zone changes; `PRAYER=off` leaves it
  out. `PRAYER_REMIND` minutes before a prayer (10) its reminder takes the line — `◷ Maghrib in
  9:48 · 17:50 WIB` on a warning's tint, then `Time for Maghrib` on good news' for five minutes
  — and is said once beyond the screen: the terminal's bell, and the desktop's notification
  through the terminal where it takes OSC 9 (iTerm2, Ghostty, WezTerm), through the system where
  it does not (`osascript`, `notify-send`); `NOTIFY=bell` or `off` says less. `d` (`ctrl+\ d` in
  a session) or a click on ✕ waves it away. The terminal's title says the fleet's mark and the
  next prayer, for a tab in the background.
- **Local time** (`src/clock.rs`): the clock and every time on screen — the tape, the drawer —
  in the machine's zone (`TZ`, else the system's; looked at again every half minute), named the
  way Indonesia names its zones (WIB, WITA, WIT) and by offset elsewhere; `z`, a click on the
  clock, or `TIME=utc` shows UTC instead. Prayer times are always the place's own.
- **The look** (`src/ui/mod.rs`, `src/theme.rs`): no frame round the screen. The chrome is a
  surface of its own — the masthead and the day line, the shelf with the band (and on view 7
  the cards), the footer, and on view 7 the list of sessions — and the work is drawn in the
  well it leaves. The masthead has the name, the fleet's mark on its tint, the tabs (the one
  open underlined) and how fresh the numbers are with the clock; a click on a tab opens it.
  Bars are thin (`━`, half-cell steps) everywhere; the map's tiles are cards on the surface;
  the session on screen and the tile under the cursor are raised, a bar of the accent at their
  left. Without painted backgrounds (16 colours, mono) lines take the surfaces' place.
- **Query sessions** (`src/console.rs`, `src/ui/console.rs`, `src/sources/clickhouse.rs`): a
  session of view 7 that is a SQL console on one server of the fleet — `c` on a node of view 1
  or a tile of view 3, `ctrl+\ q`, or *Query* in the picker, which then lists the servers. Its
  queries go over HTTP with the login the monitor has for that server, `readonly=1` always,
  with `max_execution_time=30`, `max_result_rows=1000` (`break`), `wait_end_of_query=1` so the
  summary header says what was read, and `cancel_http_readonly_queries_on_client_close=1`: a
  query stopped with `ctrl+c`, or whose session closes, is let go and the server stops it. A
  login whose read-only profile refuses those settings is asked again with `readonly=1` alone.
  The answer is read as `JSONCompactEachRowWithNamesAndTypes`, at most 1000 rows and 8 MiB; a
  query with a `FORMAT` of its own is shown as the text it sent. ClickHouse's errors are shown
  in its words (`Code 164 · … (READONLY)`), taken out of the answer's format when the server
  wrote them there, without its version and never with the password. The logins stay in
  `main.rs` with the code that sends them, refreshed after every discovery; `App` knows the
  servers by name only. The text, what ran (`↑` on the first line) and the server are kept with
  the session for the next start. `FAKE=1` answers from the made-up fleet. The text is selected
  as in an editor — a drag of the mouse (past the field's edge it scrolls along), `shift` with
  the arrows, `ctrl+a` — and what is selected is deleted, typed over, copied (`ctrl+c`), cut
  (`ctrl+x`) or run alone (`ctrl+r`); `ctrl+w` takes a word, `ctrl+u` everything, `ctrl+z` gives
  back the last of these. Where the terminal tells them apart, `⇧⏎` is always a new line and
  `ctrl+⏎` or `⌘⏎` runs, as in Redash.
- **Suggestions** (`src/complete.rs`): as a word is typed in a query session, what the cursor's
  place in the statement calls for — tables after `FROM`/`JOIN`/`INTO`/`DESCRIBE`, a database's
  tables after `db.`, the columns of the tables the statement reads (by name or alias, the whole
  statement read, not only what is before the cursor), functions (with their parentheses),
  keywords in the case typed, formats after `FORMAT`; nothing in a string or a comment, after
  `AS`, `LIMIT` or a number. A word matches at its start or at the start of a part of a name
  (`log` → `query_log`, `hour` → `toStartOfHour`). After a value a keyword ranks first; after
  `SELECT`, a comma or an operator a column does. The lists are the server's own —
  `system.databases`, `system.tables`, `system.columns`, `system.functions`, read-only and
  bounded — read when a session first runs on it and again after fifteen minutes; until then,
  the system tables and common functions.
- **A helper for the SQL** (`src/assist.rs`): `ctrl+k` (or a click on *asks Claude* under the
  text; `ctrl+g` as well, where a global shortcut does not take it first) opens a line under the
  text to say what to do, in any language, and `⏎` asks Claude Code — `claude -p` with
  `--tools ""`, `--strict-mcp-config`, `--no-session-persistence` and the rules appended to its
  own system prompt, so it answers on the user's Pro or Max plan — or OpenCode — `opencode run`
  with `OPENCODE_PERMISSION` denying every tool — for that; with nothing typed, to write what
  the text's `--` comments ask for, or to put right what failed with what the server said. With
  part of the text selected it is told the whole text and asked about that part alone; its
  answer takes that part's place if the text is as it was when asked, and is marked (Claude's
  tint) until it is changed or run — not selected, so `ctrl+r` runs all of it. Both run in an empty folder of
  their own (`~/.cache/cobserve/assist`), without `ANTHROPIC_API_KEY` or the monitor's
  credentials, for two minutes at most; `ctrl+c` kills them. They are told the server, its
  version, the text, the error, and the tables the text names or its words point at with their
  columns (the others by name) — never a row or a login. The answer, out of its fences and its
  long lines broken before their clauses, takes the text's place — the question kept on top as
  a `--` comment when the answer does not carry it; `ctrl+z` puts the text back; nothing runs
  until asked. OpenCode's conversations from here are left out of *past
  conversations*. `ASSISTANT=opencode` chooses OpenCode first; `ctrl+t` or the chip switches.
- **Every row a click**: on view 1 a row of the tree or an insight, on view 2 a job, on view 3 a
  tile, on view 4 a line of the tape — a click puts the cursor there, a second does what `⏎`
  does.
- **The name**: the crate, the binary, the masthead and the terminal's title say *cobserve*, as
  the repository does; the sessions kept under the name the screen had before are still read.
- **The job's SQL on view 2**: the cursor on a job opens its SQL under the row, as on view 1 —
  what ClickHouse runs when the stitch found it, the query as saved in Redash otherwise (an
  ad-hoc query outside ClickHouse has none: Redash's API keeps no text for it). `J` `K` scroll.
- **AIRFLOW**, view 5 (`src/airflow.rs`, `src/sources/airflow.rs`, `src/ui/airflow.rs`): what
  every DAG did over the last day, from Airflow 2's stable REST API (`/api/v1`). Signed into as
  its web UI is — `GET /login/` for the form's CSRF token and the session it lives in, the form
  posted back, its session cookie kept — since an Airflow whose `auth_backends` is the default
  session answers Basic auth with 403; signed in again when the session runs out, and a refused
  login is not sent again for five minutes (or until `r`), so a wrong password cannot lock the
  account. Everything after the login is GET. Every 15 s: `/health`, the runs that are running
  or queued (`/dags/~/dagRuns`), the live task instances (`/dags/~/dagRuns/~/taskInstances`:
  running, queued, up for retry or reschedule, deferred, restarting) and the tasks of each
  running run, counted; every minute — and at once when a run that was live has finished — the
  runs that ended in the last 24 h, the first page then the rest six at a time; every five
  minutes the active DAGs and the import errors; once, each failed run's tasks. The first read
  is sent in two: what runs, within seconds; the day, when it is in. The screen: the health of
  the scheduler and the triggerer with their heartbeat's age, the day in numbers, then
  RUNNING (a run past `LONG_RUN_S` = 6 h amber, past `STUCK_RUN_S` = 24 h red and *stuck since*
  its date: a run whose worker died stays running), QUEUED (amber past `LONG_QUEUE_S` = 1 h),
  FAILED (the day's, with the task each failed at, or what its tasks were when none did) and
  ACTIVITY: a line per DAG that ran, its day on a timeline whose cells start on the hours of the
  clock shown (`clock.rs` `offset_s`), 24 of them at 120 columns and 48 from 160 — in a cell
  several runs share, a failure shows over a run, a run over a wait, a wait over a success —
  beside Airflow's schedule shortened, the day's runs, the last one's age and length, and the
  next. A run is labelled by when it began, not by its logical date, which for a daily DAG is
  the day before. `⏎` opens a page in the view's place (`src/detail.rs`, `src/ui/detail.rs`): on
  a run its tasks (`/dags/{d}/dagRuns/{r}/taskInstances`), on a task the log of its last try
  (`…/logs/{try}`, asked as `text/plain`, its last 3000 lines, opened at the end, errors red),
  on a DAG its last 50 runs; `esc` goes back a page. A page is read by a task of its own on the
  source's session, so it never waits for a poll. `o` opens the row or the page in the browser
  (`open`, `xdg-open`), `y` copies the link, `r` reads again. `AIRFLOW_URL`, `AIRFLOW_USER`, `AIRFLOW_PASSWORD`, or
  `airflow:` in the credential file.
- **JIRA**, view 6 (`src/jira.rs`, `src/sources/jira.rs`, `src/ui/jira.rs`): the tickets
  assigned to the token's owner, in the board's columns, from Jira Server's REST API
  (`/rest/api/2`) with a personal access token (`Authorization: Bearer`), every minute, GET
  only. The search is `assignee = currentUser() AND (status in (…) OR (status = <last> AND
  (resolved >= -Nd OR resolution is EMPTY)))`: the last column is where finished tickets go and
  shows the last `JIRA_DONE_DAYS` (7) days of them, as the board hides older ones (its
  `oldDoneIssuesCutoff`). The columns are `JIRA_STATUSES`, by default the Data platform board's
  `In progress, In Review, Feedback, Done`. When a ticket moved into its column comes from its
  history (`expand=changelog`), read for a ticket only when it is new or changed. Columns are
  ranked as the board ranks them — by priority in Jira's own order (`/rest/api/2/priority`),
  then the latest touched — and the last by when each was finished. On a row: the key, the
  summary with the team's tag (`DE -`) faint, the priority (`URGENT`, `ASAP`, `Highest`,
  `Blocker` red), the time in the column, the due date — red when past, amber today and
  tomorrow, nothing once finished — and the time logged. The first line is the flow in counts,
  with how many open tickets are overdue or due soon. Under it, LOGGED: your worklogs of the
  month on any ticket (`worklogAuthor = currentUser() AND worklogDate >= startOfMonth(-1d)`, each
  ticket's worklogs read whole when the search cuts them, kept by author), every five minutes
  and after `r`, sent after the board so the board does not wait for them; a day is the date the
  time was logged for, as written. The total, its average over the working days so far and
  today's, and a bar a day, eighths of a cell, up to eight hours or the longest day — counts, so
  no colour but today's. `t` opens the month by ticket over the board: the chart, then a row per
  ticket logged on (the most first) with a one-row bar under each day — a full cell eight hours —
  in the chart's columns (the key, then the summary when it has twelve cells, then a column a
  day of at least three, else two with only the 1st, every fifth, today and the chosen day
  numbered), and the chosen day's tickets with their share of it. `← →` a day, its column lit
  through chart and rows; `↑ ↓` a ticket; `⏎` the ticket in full over the page; `esc` back. `⏎` shows the ticket in full in the view's place — fields,
  description with Jira's wiki markup turned into headings, items, code and text, sub-tasks,
  links, comments; `o` opens it in Jira, `y` copies its link, `r` reads again. `JIRA_URL`, `JIRA_TOKEN`, `JIRA_STATUSES`, `JIRA_DONE_DAYS`, or `jira:`
  in the credential file. Both views keep what they last read when a read fails, and say so.
- **LOCAL**, view 0 (`src/local.rs`, `src/sources/local.rs`, `src/ui/local.rs`): the machine
  cobserve runs on, as Activity Monitor shows it, read every `POLL_MS` with no configuration and
  nothing on the network — `ps -axo pid,ppid,user,pcpu,rss,time,comm`, `vm_stat`, one `sysctl`
  call, and the kernel's CPU ticks (`host_statistics(HOST_CPU_LOAD_INFO)`; `/proc` on Linux).
  The arithmetic, which is this view's §5:
  - machine CPU `= Δ(user + system + nice) / Δ(all ticks)` of `hw.ncpu` cores; busy cores is
    that share of them. `ps` cannot be summed for it: `kernel_task` is not in it.
  - a process's cores `= Δ cpu time / Δ wall` between two reads, the same pid and command — the
    delta form of §5.2 — and `%cpu / 100` (ps's decaying average) for one seen once.
  - **the rest** `= busy cores − Σ process cores`, at least 0: the kernel and what started and
    ended between two reads. It closes the list as §5.3's row closes a node, so the rows add up
    to the bar.
  - memory used `= app + wired + compressed`, Activity Monitor's sum, with `app = anonymous −
    purgeable` and `cached = file-backed + purgeable`, of `hw.memsize`. A process's memory is its
    RSS — not Activity Monitor's footprint, which counts its compressed pages too, so an app whose
    memory is compressed shows less here.
  - memory pressure is the kernel's: `kern.memorystatus_vm_pressure_level` (1 normal, 2 warning,
    4 critical) is the severity, `kern.memorystatus_level` the share it counts free. A Mac uses
    most of its memory by design, so the memory bar takes its colour from the pressure, never
    from the used share; CPU takes §7's 75 and 90.
  On screen: the machine's name, cores, memory, uptime and load; CPU and memory as bars with
  their denominators, user and system, app · wired · compressed · cached, four minutes of each;
  the pressure and the swap; then every process — or every program, `g`, the outermost `.app` it
  runs from (all of Chrome's helpers are Google Chrome) — by CPU or by memory, `s`, each with its
  cores of the machine's and its RSS of the machine's. The cursor follows its process while the
  list re-sorts; the drawer has the rest. With 20 rows, the band says the machine on a third
  line, `LOCAL`, on every view, and the desktop app has a chip for it. Read-only: nothing here
  signals a process. With `FAKE=1` the machine is made up too, so screenshots carry no real
  process.

### Departures

- **§3 the tabs**: the monitor has six views, so the sessions are numbered on from 7, and a
  digit picks the first three of them (`ctrl+\` and the arrows, or `/`, reach the rest). This
  machine is `0` (and F10), the last key of the row, drawn after the sessions, so they keep 7–9.
  Airflow and Jira are where they are because a digit there is what makes a glance cheap.
- **§5.4 runaway by memory** uses the query's own `max_memory_usage` from its `Settings`.
  `system.settings` answers for the monitoring session — 6 GB on the rig, the monitor's own
  cap — not for the person running the query; it is now only the fallback before 9 GiB.
- **§7 marks** are `▲` (amber) and `✖` (red) instead of `⚠`, which several terminals draw as a
  two-cell emoji and so shift the row. Colour also marks structure (node names, persons, keys)
  — still never to tell two users apart, and red and amber still mean only severity.
- **§2.4 the SQL is no longer in the drawer**: the cursor on a query opens its SQL right under
  its row — Redash's comment gone, a one-line query broken before its clauses, wrapped to the
  screen and coloured — on up to 45% of the tree's height, with a scrollbar and `J` `K` (or
  shift ↑↓) when it is longer. The drawer keeps the query's numbers.
- **§1 layout**: a fleet summary line above the queue strip, one column header above the tree
  instead of one under every open node, bars on node rows on the same scale as the user rows
  under them, insights between the tree and the drawer. The drawer and the §7 degradation order
  are as specified. The header is the masthead with the day line under it; the poll counts and
  *read-only* that sat on the bottom border are at the footer's right, where there is room, and
  a notice comes before the footer's last keys.
- **§7 the clock** is local time, not UTC (see *Local time*); `TIME=utc` keeps §7's.
- **§6.4 the organisation's domain** is configuration — `EMAIL_DOMAIN`, or `email_domain:`
  under `redash:` in the credential file — rather than written into the code. Without it every
  person is shown by the whole address; FAKE=1 uses its own `example.net`.
- **§9 nothing on disk** has these exceptions: the credential file, read only when the command
  line names it; view 7's list of sessions, kept in the state directory; the names of past
  conversations, read from Claude Code's and OpenCode's own files; and the system's list of
  time zones, for where prayer times are for. `CH_USER` / `CH_PASSWORD` are required only when some server has no login of
  its own, and one of the two is never completed with a guess for the other.
- **§6.2 seeds** are de-duplicated by URL, not by host: two ports on one machine are two
  servers.
- **§6.2 discovery matches each seed to its own row** of `system.clusters` before adding
  anyone: by `is_local` (the row that is the server answering), else by the server's own
  `hostName()`, else by the seed's URL naming the host or its address, else by the seed's
  name resolving to the row's address — and a binding, once made, is kept. Matching by name
  alone made a seed reached as `clickhouse1.example.net` and listed by its cluster as
  `pay-ch-node-1.example.lan` two nodes, the second one unreachable from a laptop that cannot
  resolve `.lan`. A seed keeps the name it was typed with (a bare IP takes the cluster's);
  the cluster's name is in the drawer. A host no seed is — a real other node — is reached by
  name when that resolves here, by its listed address when it does not.
- **A seed that refuses the discovery query** — a wrong password — is still matched to its row,
  by the name its server gives in `X-ClickHouse-Server-Display-Name` (its host name unless
  configured otherwise), which ClickHouse sends even with a refused login. While a seed neither
  answers nor matches, rows nobody has a login for are not added: they may be that seed, and
  would only say "add this host" about a host that is already there.
- **Unreachable reasons** are said plainly (*name does not resolve from here (DNS)*,
  *connection refused*, *no answer within 1.5 s*) instead of reqwest's `error sending request
  for url (…)` or ClickHouse's paragraph. A server that answers is not called unreachable: it
  reads **no access** with the grant it is missing (*monitor needs SELECT on
  system.asynchronous_metrics*), **login refused**, or **not polled** when there is no login
  for it.
- **A query's own limit** that it is already more than 10% past is not quoted ("1279% of its
  limit"): the server is evidently holding it to something else, so no forecast is made from
  it.
- **§6 concurrency**: the per-node requests now really run together; the previous `join_all`
  awaited them one after another.
- **§3 keys on view 7**: every key goes to the session — `q`, the digits, `ctrl+c` — except
  `ctrl+\`, after which a number goes to that tab (`1`–`6` a view, `7`–`9` a session), `n` `r`
  `x x` open, rename and close sessions (`c` `o` `t` open a Claude, OpenCode or terminal one),
  `esc` goes back to the session and `ctrl+\` again to the monitor (and `p` past
  conversations, `z` the clock, `d` a prayer's reminder away); and `F1`–`F9`, the same
  tabs with no key before them, for a terminal that keeps `ctrl+\`. From the monitor, `7`–`9`
  or `ctrl+\` open the sessions.
- **§3 the mouse**, which v1 left out: a click opens a tab of the header, a session or a new
  one, and in the folder picker goes into a folder or back up the path; the wheel moves the
  cursor, and over a session is what it would be in a terminal of its own — the reports
  themselves when the program asked for mouse reports; else Claude's page up and down, the
  arrow keys for a full-screen program, and for a shell its own past (a thousand lines are
  kept; a key comes back down). The terminal's own selection then needs its modifier key;
  `MOUSE=0` gives the mouse back to the terminal. With bracketed paste on, a paste is one
  event: into Claude as a paste, into `/` or the picker's search as text, and nowhere else —
  before, a paste was a stream of keys, and a `q` in it quit.
- **Read-only** still describes everything the monitor does. View 5 is the user's own Claude
  Code, OpenCode or shell with its own permissions, as it would be in another terminal tab;
  the monitor only draws it and carries its keys.
- **§2.8 view 2's layout**: the queues first, idle ones folded into one line; then RUNNING —
  the table already says how full the queue is, the running jobs are why — then WAITING (only
  when something waits), then STALE. No `failed/5m` column: the endpoint has no failure count,
  and a column of zeros read as "no failures". OLDEST only where Redis can name the waiting
  jobs, instead of a column of dashes. WHO is the person alone — the account was the same
  `r_redash →` on every line. Columns are as wide as what is in them and never run into each
  other; QUEUE appears when a list spans several queues on a terminal 150 wide or more. A job
  on another database says so (*mysql · not ClickHouse*, *runs inside Redash* for Query
  Results) instead of *not in ClickHouse yet*.
- **§6.3 running is not "in the started list"**: a job counts as running when a live worker
  holds it (`workers[].current_job`). Otherwise it is stale: cancelled (`meta.cancelled`), over
  a day old, or held by no worker although every busy worker of its queue names its job (judged
  only past 60 s, and a job a worker let go of since the last poll gets one more poll — it is
  finishing). Workers that do not say what they hold leave only the first two tests.
- **§6.3 a live worker** is one heard from within 480 s, RQ's own lifetime for a worker's key:
  an idle RQ worker beats once every 405 s, so 60 s counted most idle workers as gone. A
  heartbeat from the future is a clock running ahead, not a dead worker. The band and the
  view's header count every worker once; a queue's row counts the workers serving it, so a
  worker shared by two queues is in both rows.
- **§6.3 waiting jobs from Redis** are read from RQ's `description` — the call written out,
  `execute_query('…', 3, {'query_id': 7438, …}, user_id=42, …)`: RQ pickles `data` and
  `meta`, which the previous reading expected to be JSON and so never named a waiting job.
- **§6.4 attribution** is by the comment, whatever the account: the account Redash connects
  with is a setting of each data source, and a fleet where it is not `r_redash` showed nobody
  behind its queries and left the queue nothing to stitch to. An account whose SQL has no
  comment is still never renamed. `query_id` is matched case-insensitively with a space or an
  underscore, so `Query ID: N` — what older Redash writes for scheduled runs — counts too.
- **§2.8 the stitch** uses `Job ID:`, which Redash also writes into the comment: the RQ job's
  id, an exact match where the Redash number is not (one query can run twice at once). The
  number is the fallback, for running jobs on ClickHouse data sources only — a leftover from
  last spring does not claim today's run of the same query — and a data source names a node
  when its name holds the node's first label (`clickhouse-bi (prod)` → `clickhouse-bi.…`).

### Fixed on the way

- View 2 listed every waiting job younger than the oldest running one under RUNNING.
- The stitch ran only on ClickHouse polls; it now also runs on each queue poll, prefers the
  node the job's data source names, and never hands one ClickHouse query to two jobs.
- The pivot keyed rows by account alone, so every person behind `r_redash` opened and closed
  together and the cursor could not reach the second one.
- The queue explanation counted visible tree rows instead of runaway queries.
- A momentarily negative `memory_usage` (it is Int64) dropped the query from the screen.
- The tree never scrolled to follow the cursor.
- A line printed to stderr at startup was drawn over the screen and stayed in the cells the
  next frame did not change.
- `dev/local-rig.sh` on Linux: the umask for the password file leaked onto the rendered
  configs, and Keeper's raft port needed `enable_ipv6=false` on a host without IPv6.
- A Redash timestamp with a zone (`+03:00`) was read as if it were UTC.
- The queue explanation said *why it is full* when nothing was waiting.
- A Redash user, query or data source that could not be read was asked for again on every
  poll; an answer that says "no" is now remembered, a connection that failed is not.
