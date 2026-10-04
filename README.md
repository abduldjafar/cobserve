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

One short line for each thing in trouble, worst first: a node, the Redash queue, a Redash
query running twice. A node with several problems is still one line — its worst, then
`+N more`. When all is well there is one line that says so.

```
✖ clickhouse5     no access · monitor needs SELECT on system.asynchronous_metrics
✖ clickhouse3     memory 91.2% · CPU 93.5% · the server itself holds 46%  +1 more
▲ clickhouse-bi   j.petrova's query at 88% of its memory limit  +2 more
▲ Redash          queries: 16 waiting · oldest 1m49s · 2 of 6 workers on long queries
▲ Redash #8585    running 2× · j.petrova
```

`tab` picks a line and the drawer shows the numbers behind it — who holds what, the forecast,
the free memory against one more query, every finding about that node — and `⏎` goes to the
row. What a line can say:

- a node without numbers: **unreachable**, **no access** (it answered, but the login lacks a
  grant — the grant is named), **login refused**, or **not polled** (no login for it);
- memory or CPU hot, and whether a **person** or **the server itself** holds it;
- memory **climbing** to full — *"↗ full in ~3m30s"*;
- a query close to **its own** memory limit, and when ClickHouse will kill it;
- long queries, replica lag;
- the Redash queue backing up, and whether its workers are stuck on long ClickHouse queries;
- the same Redash query running twice.

The heaviest user, slow polls and new nodes are on screen elsewhere (`u`, the drawer, the tape)
and are not repeated here.

### Reading the screen

| | |
|---|---|
| `✖` `▲` | critical (red) · warning (amber), by `DESIGN.md` §7 |
| `✕` | runaway query: ≥ 30 s, or ≥ 80% of its own memory limit |
| `↯` | node without numbers — unreachable, no access, login refused or not polled; unknown, never zero |
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

cargo run --release -- --credential credentials.yaml     # the fleet, a login per server

CH_SEED_URLS=http://ch-node-1:8123 CH_CLUSTER=ch_cluster \
CH_USER=monitor CH_PASSWORD=… cargo run --release        # the fleet, one login for all
```

### A login per server: `--credential`

When the servers do not share a login, write each one's into a YAML file and name the file
on the command line. [`credentials.example.yaml`](credentials.example.yaml) is the template:

```yaml
clickhouse:
  cluster: ch_cluster
  servers:
    - url: http://clickhouse1.example.net:8123
      user: monitor_ch1
      password: "…"
    - url: http://clickhouse2.example.net:8123
      user: monitor_ch2
      password: "…"
```

```sh
cp credentials.example.yaml credentials.yaml     # gitignored; fill it in
chmod 600 credentials.yaml
pay_monitoring --credential credentials.yaml     # or: cargo run --release -- --credential credentials.yaml
```

- Each server logs in as its own user, and a password is only ever sent to its own server. A
  host discovery finds that is not in the file uses `default_login` (or `CH_USER` /
  `CH_PASSWORD`) when there is one; when there is not, it is shown as **not polled**, with a
  note, rather than sent somebody else's password.
- Passwords are read exactly as written — `:` `@` `/` `,` `%` and leading zeros included. Only
  YAML's own rules apply: quote one that starts with a quote, `#`, `{`, `[`, `&`, `*`, `!`, `|`,
  `>`, `%` or `@`, or holds ` #`.
- What the file says wins over the environment; `CH_SEED_URLS` adds servers to its list. An
  optional `redash:` section holds `url`, `api_key`, `redis_url` and `email_domain`.
- A wrong password shows on its node as *login refused for user monitor_ch2* — the user,
  never the password. A file other users can read gets a warning in the footer. A mistake in
  the file is reported with its key and line, never with the value in it.
- Without a file, a login can also ride in the seed URL:
  `CH_SEED_URLS=http://user:password@host:8123,…` — with `/`, `,` and `%` in a password
  written `%2F`, `%2C` and `%25`.

`./dev/local-rig.sh credentials` writes such a file for the local rig, whose two nodes each
have a user the other one does not know.

| Variable | Meaning | Default |
|---|---|---|
| `CH_SEED_URLS` | comma-separated `http://host:8123` seeds, optionally `http://user:password@host:8123`; the rest is discovered from `system.clusters` | required, unless the `--credential` file lists servers |
| `CH_CLUSTER` | cluster name for discovery | required (or `cluster:` in the file) |
| `CH_USER` / `CH_PASSWORD` | a **read-only** ClickHouse user, for every server without a login of its own | required unless every server has its own |
| `CH_HTTP_PORT` | HTTP port for discovered hosts | `8123` |
| `REDASH_URL` / `REDASH_ADMIN_API_KEY` | Redash admin API | optional — the strip says *not configured* |
| `REDIS_URL` | Redash's RQ Redis (read-only), for the names of **waiting** jobs | optional |
| `EMAIL_DOMAIN` | your organisation's e-mail domain: people there are shown by name alone (`grigol.gankava`), everyone else by the whole address | unset — every address whole |
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
