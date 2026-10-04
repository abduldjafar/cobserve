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
| **NODES** | `1` | Every node's memory and CPU against its own capacity, and who holds it: user rows (a query from Redash resolved to the person behind it) plus the server's own share always add up to the node (§5.3). Below the tree, **insights**; below them, the **drawer** for the selected row. |
| **QUEUE** | `2` | Redash: per queue what **runs**, what **waits** and what is **stale**, and its workers. Every running job with its person, query and data source, and the ClickHouse query it became — live memory, cores and progress; who is waiting, and for how long against the 3-minute red line; and what RQ's started list holds although no worker runs it. The cursor opens the job's SQL. |
| **MAP** | `3` | The whole fleet as tiles framed in their severity colour, with bars and history. A forty-node fleet on one screen. |
| **TAPE** | `4` | What changed, newest first: nodes going hot or unreachable and recovering, runaways starting and ending — *"probably killed"* when a query vanished at its memory limit — the queue backing up and draining. |
| **SESSIONS** | `5` | Claude Code itself — your own `claude`, signed in with your Pro or Max plan — OpenCode, or your own shell, in as many named sessions as you need, for any work at all, while the fleet and the Redash queue stay in sight above them. |

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

### The Redash queue

View 2 reads what the Redash admins' own queue script reads, with the same admin API key and
nothing that could cancel a job: `/api/admin/queries/rq_status` for the queues and the
workers, `/api/users/{id}` for who (name and address), `/api/queries/{id}` for the query (name
and SQL) and `/api/data_sources` for what it runs on (name and type).

- **RUNNING** is a job a live worker holds. Redash's started list is not enough: when a worker
  dies its job stays there, for months without a time limit.
- **STALE** is the rest of that list, each with its reason — *cancelled*, *over a day old*,
  *no worker holds it*. They take no worker; a Redash admin can clear them.
- **IN CLICKHOUSE** follows a running job into ClickHouse by the `Job ID:` Redash writes into
  the comment of every query (and by its query number when there is none): `⏎` goes to the
  query on view 1. A job on another database says so — *mysql · not ClickHouse*, *runs inside
  Redash* for Query Results.
- **WAITING** has names and ages only with Redis (`redis_url:`); Redash's API only counts the
  waiting jobs.

![Redash with leftovers in its started list](docs/screenshots/120x36-queue-leftovers.png)

### Claude, OpenCode and a terminal in the monitor

View 5 runs programs in a terminal inside the monitor — the official `claude`, `opencode`, or
your own shell: the header, the FLEET and REDASH lines and the worst thing in the fleet stay on
top, the session gets the rest. Each is the same as in a terminal tab of its own — any
project, any question, its own permission prompts.

- **Claude** is Claude Code signed in with your **Pro or Max plan**: install it and sign in once
  (`claude`, then `/login`). `CLAUDE_CMD` runs something else, or the same with arguments:
  `CLAUDE_CMD="claude --model opus"`.
- **OpenCode** signs in its own way: install it (`curl -fsSL https://opencode.ai/install | bash`)
  and sign in once with `opencode auth login`. `OPENCODE_CMD` changes the command.
- **Terminal** is your login shell (`$SHELL`); `SHELL_CMD` runs another one.
- Nothing here uses an API key: `ANTHROPIC_API_KEY` is taken out of every session's
  environment, so Claude Code bills the plan you signed in with even when your shell has a
  key set; so are the monitor's own credentials (`CH_PASSWORD`, `CH_SEED_URLS`,
  `REDASH_ADMIN_API_KEY`, `REDIS_URL`).
- Each session works in a folder of its own — the first in the one the monitor was started
  from. What runs there can read files there, so keep the credential file somewhere else.
  `--claude` opens on view 5.

Up to five sessions run side by side, each its own program with its own conversation, listed
beside it like a terminal's tabs, the one on screen framed: what it runs (`✻` Claude, `▣`
OpenCode, `❯` a terminal), its name — the one you gave it, what the program says it is working
on, or else its folder — and number, and under them the folder and its git branch. A session
that rings while you are elsewhere is marked `●` there and on `5 SESSIONS`; one that ended,
`✕`. The sessions are numbered on from the views: `1`–`4` are the monitor, `5`–`9` the
sessions.

Under them, at the bottom of the list, the fleet stays in sight: a line a node in view 1's
order — its state, its name, and what it uses of its memory and CPU (with bars when the list
is wide enough, a one-cell gauge when not), red and amber as on view 1. A node that does not
answer says why instead. A click on one opens it on view 1; when there are more nodes than
room, the last line says how many more there are.

A new session's folder is picked as in a file explorer, no typing needed: it starts in the
folder the session on screen works in, a click on a folder goes into it, a click on a step of
the path above (`~ › work › cobserve`) goes back up to it, and **Open the session here** (or
`⏎`) starts it there. Above them, a click chooses what it runs — Claude, OpenCode, Terminal —
or `shift+tab` does. Typing searches the folders below by name — up to six levels down,
nearest first, `node_modules` and the like left out — and a path (`~/`, `/`, `../`) is
completed like a shell does. A repository shows its branch.

| On view 5 | |
|---|---|
| any key | goes to the session — `q`, the digits and `ctrl+c` too |
| `ctrl+\` then `1`…`4` | that view of the monitor |
| `ctrl+\` then `5`…`9` | that session |
| `ctrl+\` then `n` | a new session, of the kind on screen: pick its folder — `↑` `↓` choose, `⏎` opens it there, `→` goes in, `←` up, typing searches, `shift+tab` changes what it runs, `esc` clears the search or gives up |
| `ctrl+\` then `c` · `o` · `t` | a new Claude, OpenCode or terminal session, the same way |
| `ctrl+\` then `r` | rename the one on screen (an empty name gives the default back) |
| `ctrl+\` then `x` `x` | close it — `claude --resume` finds the conversation later |
| `ctrl+\` then `esc` | back to the session |
| `ctrl+\` `ctrl+\` | back to the monitor, where you came from |
| `F1`…`F9` | the same tabs from anywhere, without `ctrl+\` |
| a click | a tab in the header, a session, **+ new session**, a folder, what to run |
| the wheel | over Claude, its page up and down; over a shell, back through what went past (a key comes back down), and in `less` or `vim` the arrow keys; over OpenCode, what it asked for; elsewhere, the cursor |

From the monitor, `5`…`9` or `ctrl+\` go to the sessions. `ctrl+z` is not passed on to Claude
or OpenCode — there is no shell around them to bring them back — but it is to a terminal,
whose shell suspends its own jobs. With the mouse on, selecting text takes the terminal's
modifier — Shift in most, ⌥ in iTerm2 and Terminal — and `MOUSE=0` leaves the mouse to the
terminal altogether.

![Claude, OpenCode and a terminal in view 5](docs/screenshots/160x48-claude.png)

![A new session's folder, picked with clicks](docs/screenshots/160x48-claude-new.png)

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
| `CLAUDE_CMD` | what a Claude session runs, with its arguments | `claude` |
| `OPENCODE_CMD` | what an OpenCode session runs | `opencode` |
| `SHELL_CMD` | what a terminal session runs | `$SHELL`, else `/bin/sh` |
| `MOUSE` | `0` leaves the mouse to the terminal: no clicks, but selection without a modifier | on |
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
| `J K` `shift ↑↓` | scroll the SQL of the selected query or Redash job (it opens right under its row) |
| `tab` | move between the tree and the insights |
| `space` | fold / unfold the healthy nodes |
| `u` | pivot node ↔ user: who is burning the fleet |
| `s` | sort: pressure, memory, CPU, name |
| `/` | filter by node, user, person, SQL or query id · `esc` clears |
| `1 2 3 4` | views · `5`…`9` the sessions |
| `F1`…`F9` | the same, from anywhere — Claude's screen too |
| `ctrl+\` | the sessions (view 5); there, the key before a number — see *Claude, OpenCode and a terminal in the monitor* |
| a click · the wheel | a tab, a session · the cursor (on view 5, the session's page up and down) |
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
