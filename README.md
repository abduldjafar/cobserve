# cobserve

A terminal monitor for on call: the ClickHouse fleet and the Redash queue on one screen,
**who** is using each node, and **what it means**: ranked insights, trends and forecasts, a
fleet map, a tape of everything that changed — and in the same window Claude Code, OpenCode or
a shell, and a **SQL console on any server** of the fleet, read-only, that suggests as you type
and has Claude or OpenCode write the query for you. Around it all, the **day**: your local
time, the prayer times where you are, and a reminder ten minutes before each.

![NODES view](docs/screenshots/120x36-nodes.png)

It reads ClickHouse directly over HTTP, read-only, so it keeps working when the web app is
down. `DESIGN.md` is the contract for every number on screen (§5); the additions here sit
on top of it and are listed in `DESIGN.md` §13.

## What is on screen

| View | Key | What it answers |
|---|---|---|
| **NODES** | `1` | Every node's memory and CPU against its own capacity, and who holds it: user rows (a query from Redash resolved to the person behind it) plus the server's own share always add up to the node (§5.3). Below the tree, **insights**; below them, the **drawer** for the selected row. |
| **QUEUE** | `2` | Redash: per queue what **runs**, what **waits** and what is **stale**, and its workers. Every running job with its person, query and data source, and the ClickHouse query it became — live memory, cores and progress; who is waiting, and for how long against the 3-minute red line; and what RQ's started list holds although no worker runs it. The cursor opens the job's SQL. |
| **MAP** | `3` | The whole fleet as cards, marked by their severity, with bars and history. A forty-node fleet on one screen. |
| **TAPE** | `4` | What changed, newest first: nodes going hot or unreachable and recovering, runaways starting and ending — *"probably killed"* when a query vanished at its memory limit — the queue backing up and draining. |
| **SESSIONS** | `5` | Claude Code itself — your own `claude`, signed in with your Pro or Max plan — OpenCode, your own shell, or a query session on a server of the fleet, in as many named sessions as you need, while the fleet and the Redash queue stay in sight above them. |

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

### The day, and prayer times

The line under the masthead is the day where you are, from Subuh to Isya: every prayer time a
stop on it, the present a point moving between them — the way behind solid, the way ahead
dotted — and the next stop saying how far off it is.

```text
━ Subuh 04:21 ━━━━━━━ Terbit 05:33 ━━━━━━━ Dzuhur 11:45 ━━━━━━━ Ashar 14:48 ━━━●┄┄┄┄ Maghrib 17:50 · in 1h58m ┄┄┄┄ Isya 18:59 ┄  Jakarta
```

Ten minutes before a prayer its reminder takes the line — `◷ Maghrib in 9:29 · 17:50 WIB` — and
is said once beyond the screen: the terminal's bell and a notification of your desktop (through
iTerm2, Ghostty or WezTerm themselves, through macOS or `notify-send` elsewhere). At its time it
says so for five minutes. `d` waves it away (`ctrl+\ d` in a session), as does a click on ✕.
The terminal's own title carries the next prayer, for a tab in the background.

- The times are computed here, from the sun, by **Kemenag**'s criteria — Subuh at 20°, Isya at
  18°, Maghrib 1° below the horizon, Ashar Shafi'i, two minutes of *ihtiyat* — and match its
  published tables to the minute; on a Friday the noon prayer is **Jumat**. Nothing is asked of
  a service.
- **Where**: `PRAYER_CITY=Bandung` (most Indonesian cities by name, and Kuala Lumpur, Singapore,
  Mecca, Medina), or `PRAYER_AT=-6.91,107.61` for anywhere; without either, the city of your
  machine's time zone (Jakarta for WIB, Makassar for WITA, Jayapura for WIT), followed when the
  zone changes. Set it: a city 300 km from the zone's is minutes off.
- `PRAYER_REMIND=15` for another lead (`0` for none), `NOTIFY=bell` for the bell alone,
  `NOTIFY=off` for the screen alone, `PRAYER=off` for no prayer times.
- **The clock** is your local time, named WIB, WITA or WIT in Indonesia, and follows the
  machine's zone as it changes; `z` or a click on it shows UTC — the servers' own — and back.
  `TIME=utc` starts on UTC. The tape and the drawer follow the clock; prayer times stay local.

![The reminder ten minutes before Maghrib](docs/screenshots/120x36-reminder.png)

### Claude, OpenCode and a terminal in the monitor

View 5 runs programs in a terminal inside the monitor — the official `claude`, `opencode`, or
your own shell: the shelf — the FLEET and REDASH lines and a card for every node — stays on
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
beside it like a terminal's tabs, the one on screen raised: what it runs (`✻` Claude, `▣`
OpenCode, `❯` a terminal), its name — the one you gave it, what the program says it is working
on, or else its folder — and number, and under them the folder and its git branch. A session
that rings while you are elsewhere is marked `●` there and on `5 sessions`; one that ended,
`✕`. The sessions are numbered on from the views: `1`–`4` are the monitor, `5`–`9` the
sessions.

The fleet stays in sight above the sessions, a card for every node — the worst first, marked
`✖` or `▲` (a quiet one `●`), with its lag when it is behind — and on every card its memory and
its CPU, a thin bar and the share each:

```text
      ✖ clickhouse3          ▲ clickhouse7 lag 12s  ● clickhouse-bi        +6 more
 mem  ━━━━━━━━━━━━━━╸━  91%  ━━━━━━━━━━╺━━━━━  62%  ━━━━━━━━━━━╺━━━━  67%  ≤ 42%
 cpu  ━━━━━━━━━━━━━━━╺  95%  ━━━━━╸━━━━━━━━━━  36%  ━━━━━━━━━━━━╺━━━  72%  ≤ 29%
```

A node that does not answer says why (`↯ no access`). As many cards as the width holds, then
how many more and how high the rest go; a click on a card opens its node on view 1. On a
terminal under 28 rows the cards give way to one line on the shelf, each node in a few words:
`✖ clickhouse3 mem 91% cpu 95%`.

**Sessions are kept.** Quit — or close the terminal — and the next start lists them again,
marked `↻`; each takes its conversation up where it was when you open it (`claude --resume`,
`opencode --session`). **Past conversations** (`ctrl+\ p`, or the entry under the sessions)
lists the conversations Claude Code and OpenCode had anywhere — in another terminal, in an
earlier run — by title or first prompt, folder and age, newest first; one opens in a new
session in its own folder, as a copy when it is still open in the other terminal. The picker of
a new session lists the folder's own under **Open the session here**. Only the names of
conversations are read, and nothing leaves the machine; the list of sessions is kept in
`~/.local/state/cobserve/sessions.json`.

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
| `ctrl+\` then `c` · `o` · `t` · `q` | a new Claude, OpenCode, terminal or query session, the same way — a query session's picker lists the servers |
| `ctrl+\` then `p` | past conversations, from any folder or terminal, to take up in a new session |
| `ctrl+\` then `r` | rename the one on screen (an empty name gives the default back) |
| `ctrl+\` then `x` `x` | close it — its conversation is still in **past conversations** |
| `ctrl+\` then `z` · `d` | the clock local ↔ UTC · a prayer's reminder away |
| `ctrl+\` then `esc` | back to the session |
| `ctrl+\` `ctrl+\` | back to the monitor, where you came from |
| `F1`…`F9` | the same tabs from anywhere, without `ctrl+\` |
| a click | a tab in the masthead, the clock, a session, **+ new session**, **past conversations**, a folder, what to run, a node's card, the reminder's ✕ |
| the wheel | over Claude, its page up and down; over a shell, back through what went past (a key comes back down), and in `less` or `vim` the arrow keys; over OpenCode, what it asked for; elsewhere, the cursor |

From the monitor, `5`…`9` or `ctrl+\` go to the sessions. `ctrl+z` is not passed on to Claude
or OpenCode — there is no shell around them to bring them back — but it is to a terminal,
whose shell suspends its own jobs. With the mouse on, selecting text takes the terminal's
modifier — Shift in most, ⌥ in iTerm2 and Terminal — and `MOUSE=0` leaves the mouse to the
terminal altogether.

![Claude, OpenCode and a terminal in view 5](docs/screenshots/160x48-claude.png)

![A new session's folder, picked with clicks](docs/screenshots/160x48-claude-new.png)

### SQL on any server: query sessions

A query session is a SQL console on one server of the fleet, beside the other sessions on
view 5: `c` on a node in view 1 (or a tile in view 3) opens one there, `ctrl+\ q` asks which
server, and so does **▦ Query** in a new session's picker. Everything it runs is read-only —
`readonly=1`, 30 seconds at most, the first 1000 rows — and stopped on the server when you stop
it (`ctrl+c`) or close the session. The server is a chip at its top: a click, or `ctrl+o`,
switches it to any other server of the fleet, each with the login the monitor has for it.

- **Suggestions as you type**, from the server's own lists (its databases, tables, columns and
  functions, read from its system tables when the session first runs there, and again a
  quarter of an hour later): tables after `FROM` and `JOIN` — `log` finds `system.query_log` —
  a database's tables after `db.`, the columns of the tables the statement reads (by their
  aliases too, wherever the cursor is in it), functions with their parentheses, keywords, and
  formats after `FORMAT`. `tab` takes one, `↑` `↓` choose (then `⏎` takes it too), `esc`
  closes them, `ctrl+space` asks for them anywhere.
- **Claude or OpenCode writes it**: say what you want in a `--` comment and press `ctrl+g`.
  Claude Code answers signed in with your Pro or Max plan (`claude -p`, no tools, no API key,
  nothing saved), OpenCode as you signed it in (`opencode run`, every permission denied); both
  run in an empty folder of their own, without the monitor's secrets. They get the server's name
  and version, your text, the tables its words point at with their columns and the names of
  the others — no rows, no logins. On a query that failed, `ctrl+g` sends what the server said,
  to put it right. What comes back takes the text's place — read it, then `⏎` runs it;
  `ctrl+z` puts back what was there. `ctrl+t` or a click on the chip switches between Claude
  and OpenCode; `ASSISTANT=opencode` starts with OpenCode.
- **The answer** is a table under it — numbers on the right, `NULL` dimmed, as many columns as
  fit — with how long it took and what the server read. `shift+tab` goes into it: the arrows
  move through it, `y` copies the row and `Y` all of it as tab-separated text (through the
  terminal, and `pbcopy` on a Mac).
- What ran is kept: `↑` on the first line goes back through it, and the session — its server,
  its text and what ran — comes back with the next start.

![A query session: what Claude wrote for a comment, and its answer](docs/screenshots/140x40-query-session.png)

![Suggestions as it is typed](docs/screenshots/140x40-query-suggest.png)

| In a query session | |
|---|---|
| `⏎` | a new line — or runs the statement once it ends with `;` |
| `ctrl+r` | run what is there |
| `tab` · `↑` `↓` · `esc` | take a suggestion · choose one · close them (`ctrl+space` asks) |
| `ctrl+g` · `ctrl+t` | Claude (or OpenCode) writes it, or puts it right · the other one |
| `ctrl+z` | put back what was there before Claude wrote, or before a clear |
| `ctrl+c` | stop the query or the writing · clear the text |
| `ctrl+o` | run on another server |
| `shift+tab` | the answer: the arrows move, `y` `Y` copy, `tab` comes back |
| `↑` on the first line | what ran before |
| `ctrl+\` | the session bar, as everywhere on view 5 |

### Reading the screen

| | |
|---|---|
| `✖` `▲` | critical (red) · warning (amber), by `DESIGN.md` §7 |
| `✕` | runaway query: ≥ 30 s, or ≥ 80% of its own memory limit |
| `↯` | node without numbers — unreachable, no access, login refused or not polled; unknown, never zero |
| `↗ ↑ ↘ ↓` | rising · rising fast · falling, over the last minute |
| `▁▂▃▅▇` | the last 4 minutes, each cell its worst moment, coloured by severity |
| `NEW` | joined the fleet during this session |
| `━━●┄┄` | the day: behind, now, ahead — prayer times at the place |
| `↻` | a session kept from the last run, or a conversation to take up |

Bars are thin lines with half-cell resolution and share one scale per column, so a node's bar
is the sum of the bars under it.

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
cobserve --credential credentials.yaml     # or: cargo run --release -- --credential credentials.yaml
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
| `c` | a query session on the node under the cursor — SQL, read-only (also on view 3) |
| `s` | sort: pressure, memory, CPU, name |
| `/` | filter by node, user, person, SQL or query id · `esc` clears |
| `1 2 3 4` | views · `5`…`9` the sessions |
| `F1`…`F9` | the same, from anywhere — Claude's screen too |
| `ctrl+\` | the sessions (view 5); there, the key before a number — see *Claude, OpenCode and a terminal in the monitor* |
| a click · the wheel | a row, an insight, a job, a tile or a tape line puts the cursor there, a second click opens it; a tab, a session · the wheel moves the cursor (on view 5, the session's page up and down) |
| `p` | pause (the numbers stop, the clock does not) |
| `?` | help · `q` quit |

Nothing here can kill or change a query: every request is `readonly=1`, and also capped at
2 threads, 6 GB and 2 seconds when the server lets a session set those (a read-only user's
own profile carries its caps otherwise). A query session's queries are `readonly=1` too, with
30 seconds and 1000 rows when the server allows them; a write is refused by the server itself.

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
