# cobserve

A terminal monitor for on call: the ClickHouse fleet and the Redash queue on one screen,
**who** is using each node, and — new in this version — **what it means**: ranked insights,
trends and forecasts, a fleet map, and a tape of everything that changed.

![NODES view](docs/screenshots/120x36-nodes.png)

It reads ClickHouse directly over HTTP, read-only, so it keeps working when the web app is
down. `DESIGN.md` is the contract for every number on screen (§5); the additions here sit
on top of it and are listed in `DESIGN.md` §13.

## What is on screen

| View | Key | What it answers |
|---|---|---|
| **NODES** | `1` | Every node's memory and CPU against its own capacity, and who holds it: user rows (Redash's `r_redash` resolved to the person) plus the server's own share always add up to the node (§5.3). Below the tree, **insights**; below them, the **drawer** for the selected row. |
| **QUEUE** | `2` | The Redash queues: workers as slots, depth over the last minutes, who is **waiting** (and for how long, against the 3-minute red line), and every job **running** with the ClickHouse query it became — live memory, cores and progress. |
| **MAP** | `3` | The whole fleet as tiles framed in their severity colour, with bars and history. A forty-node fleet on one screen. |
| **TAPE** | `4` | What changed, newest first: nodes going hot or unreachable and recovering, runaways starting and ending — *"probably killed"* when a query vanished at its memory limit — the queue backing up and draining. |

![Query detail](docs/screenshots/140x40-query.png)

### Insights

Each poll the screen is read out loud, worst first, each line pointing at its row (`tab`,
then `⏎` jumps there):

- a hot node, and whether a **person** or the **server itself** (caches, merges) holds it,
  with how much memory is left against one more query's limit;
- memory **climbing** steadily enough to run out — *"full in ~3m30s at this rate"*;
- a query close to **its own** memory limit (read from its `Settings`, not the monitor's) —
  *"+43 MiB/s → killed in ~42s"*;
- the same **Redash query running twice** at once;
- a **full Redash queue explained**: how many workers are stuck on runaway ClickHouse queries;
- replica lag, unreachable or slow nodes, nodes that joined, the heaviest user.

### Reading the screen

| | |
|---|---|
| `✖` `▲` | critical (red) · warning (amber), by `DESIGN.md` §7 |
| `✕` | runaway query: ≥ 30 s, or ≥ 80% of its own memory limit |
| `↯` | node not answering — its numbers are unknown, never zero |
| `↗ ↑ ↘ ↓` | rising · rising fast · falling, over the last minute |
| `▁▂▃▅▇` | the last 4 minutes, each cell its worst moment, coloured by severity |
| `NEW` | joined the fleet during this session |

Bars have eighth-cell resolution and share one scale per column, so a node's bar is the sum of
the bars under it.

## Running it

```sh
FAKE=1 cargo run --release            # a generated, moving fleet — no network at all

./dev/local-rig.sh up                 # two ClickHouse 24.10 nodes + Keeper in Docker
eval "$(./dev/local-rig.sh env)" && cargo run --release

CH_SEED_URLS=http://ch-node-1:8123 CH_CLUSTER=ch_paysera \
CH_USER=monitor CH_PASSWORD=… cargo run --release        # the fleet
```

| Variable | Meaning | Default |
|---|---|---|
| `CH_SEED_URLS` | comma-separated `http://host:8123` seeds; the rest is discovered from `system.clusters` | required |
| `CH_CLUSTER` | cluster name for discovery | required |
| `CH_USER` / `CH_PASSWORD` | a **read-only** ClickHouse user | required |
| `CH_HTTP_PORT` | HTTP port for discovered hosts | `8123` |
| `REDASH_URL` / `REDASH_ADMIN_API_KEY` | Redash admin API | optional — the strip says *not configured* |
| `REDIS_URL` | Redash's RQ Redis (read-only), for the names of **waiting** jobs | optional |
| `POLL_MS` | ClickHouse poll interval | `2000` |
| `THEME` | `dark`, `light` or `mono` | `dark` |
| `NO_COLOR` | any value: no colour at all (glyphs still carry severity) | unset |

Colour depth follows the terminal: 24-bit when `COLORTERM` says so, the 256-colour palette
otherwise, the 16 ANSI colours (no painted background) on anything older.

![Light theme](docs/screenshots/120x36-light.png)

## Keys

| Key | |
|---|---|
| `↑ ↓` `j k` | move · `PgUp PgDn Home End` jump |
| `⏎` | open / close a node or user · on an insight, a queue job, a tile or a tape line: go there |
| `← →` `h l` | collapse / expand |
| `tab` | move between the tree and the insights |
| `space` | fold / unfold the healthy nodes |
| `u` | pivot node ↔ user: who is burning the fleet |
| `s` | sort: pressure, memory, CPU, name |
| `/` | filter by node, user, person, SQL or query id · `esc` clears |
| `1 2 3 4` | views |
| `p` | pause (the numbers stop, the clock does not) |
| `?` | help · `q` quit |

Nothing here can kill or change a query: every request is `readonly=1`, and also capped at
2 threads, 6 GB and 2 seconds when the server lets a session set those (a read-only user's
own profile carries its caps otherwise).

## Developing

```sh
cargo test                            # model, insights, tape, history, every view at every size
cargo clippy --all-targets -- -D warnings
./dev/screenshots.sh                  # docs/screenshots/: text for every view, PNGs for these
```

| | |
|---|---|
| ![QUEUE](docs/screenshots/120x36-queue.png) | ![MAP](docs/screenshots/120x36-map.png) |
| ![TAPE](docs/screenshots/120x36-tape.png) | |
