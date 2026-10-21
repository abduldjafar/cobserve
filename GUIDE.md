# cobserve from zero

This guide goes from nothing installed to using cobserve every day. It covers what you need,
installing it, trying it without a fleet, pointing it at yours, and a tour of every screen.
[README.md](README.md) is the reference for each feature. [DESIGN.md](DESIGN.md) is the
contract for every number on screen.

1. [What it is](#1-what-it-is)
2. [What you need](#2-what-you-need)
3. [Install](#3-install)
4. [Try it without a fleet](#4-try-it-without-a-fleet)
5. [Connect it to your fleet](#5-connect-it-to-your-fleet)
6. [The screen at a glance](#6-the-screen-at-a-glance)
7. [The eight panels](#7-the-eight-panels)
8. [Sessions](#8-sessions)
9. [Recipes for on call](#9-recipes-for-on-call)
10. [Configuration](#10-configuration)
11. [What it keeps on your machine](#11-what-it-keeps-on-your-machine)
12. [Troubleshooting](#12-troubleshooting)
13. [Updating and removing](#13-updating-and-removing)

---

## 1. What it is

cobserve is a terminal app you run on your own machine. Nothing is deployed and nothing is
installed on the servers. It shows:

- **the ClickHouse fleet**: every node's memory and CPU, and who is using them. It asks each
  node's system tables over HTTP every 2 seconds.
- **the Redash queue**: what runs, what waits, what is stuck, and which ClickHouse query each
  running job became. It uses Redash's admin API every 3 seconds, and optionally Redis.
- **what it means**: insights ranked worst first, trends and forecasts, and a tape of
  everything that changed.
- **Airflow**: what every DAG did over the last day — what runs now, what waits, what failed
  and where, and each DAG's day on a timeline. It uses Airflow's REST API every 15 seconds.
- **your Jira tickets**: in your board's columns, In progress to Done, with their age and due
  date. It uses Jira's REST API every minute.
- **your work beside it**: Claude Code, OpenCode, your shell, and a SQL console on any server,
  in up to fifty sessions.

What it does **not** do:

- **It never changes anything in ClickHouse.** Every request carries `readonly=1` and goes in
  as a read-only user. It cannot kill a query or touch a table.
- **The one thing it changes is a Redash job you pick.** In the queue (panel 2, zoomed), `x` on a job and then `y`
  cancels it, exactly what Redash's own Cancel button does. Nothing is sent without that
  second key.
- **It keeps passwords to itself.** It never shows a password, not even in an error. It only
  sends a server's password to that server, and it takes its own credentials out of the
  environment of every session it starts.

![The dashboard](docs/screenshots/120x36-dash.png)

---

## 2. What you need

| What | Why | Needed? |
|---|---|---|
| **macOS or Linux** | cobserve runs there. Windows is not tested. | yes |
| **A terminal** | Any modern one: iTerm2, Ghostty, WezTerm, kitty, Terminal, GNOME Terminal, the VS Code terminal. It works from 80×24, but at 120×36 or bigger you see everything. 24-bit colour, mouse and the kitty keyboard protocol are used when the terminal has them. | yes |
| **Rust 1.88 or newer** | To build it, with `cargo`. [rustup](https://rustup.rs) installs it. | yes, to build |
| **A C linker** | Rust needs one to link. On macOS: `xcode-select --install`. On Debian/Ubuntu: `sudo apt install build-essential`. | yes, to build |
| **git** | To get the code. | yes |
| **A read-only ClickHouse user** | What it logs in as. [5.2](#52-a-read-only-clickhouse-user) shows how to create one. | yes, for a real fleet |
| **Network access to every ClickHouse node** | It talks to each node directly, on its HTTP port (8123 by default). It uses the names in `system.clusters`, or their addresses when a name does not resolve. On a VPN, connect first. | yes, for a real fleet |
| **Redash URL and an admin's API key** | For the REDASH QUEUE panel. Without them that line says *not configured* and everything else works. | optional |
| **Redash's Redis URL** | For the names and ages of *waiting* jobs, and to cancel them one by one. Without it, the waiting jobs are only a count. | optional |
| **An Airflow 2 login** | For the AIRFLOW panel: the URL of Airflow, and the user and password of its web UI (a user whose role can read DAGs). | optional |
| **A Jira personal access token** | For the JIRA panel: Jira Server or Data Center's URL, and a token made under Profile → Personal Access Tokens. | optional |
| **Claude Code**, signed in | For Claude sessions, and for `ctrl+k` in a query session. Needs a Pro, Max, Team or Enterprise account. | optional |
| **OpenCode**, signed in | For OpenCode sessions, or as the `ctrl+k` helper instead of Claude. | optional |
| **Docker and Python 3** | Only for the local test rig in `dev/`. | optional |
| `pbcopy` (macOS), `wl-copy` or `xclip` (Linux) | Copying answers to the clipboard. Terminals that support OSC 52 copy without them. | optional |

---

## 3. Install

```sh
# Rust, once, if you do not have it (it puts ~/.cargo/bin on your PATH)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
#   macOS: xcode-select --install        Debian/Ubuntu: sudo apt install build-essential git

# The code
git clone https://github.com/abduldjafar/cobserve.git
cd cobserve

# Build it and put `cobserve` on your PATH (in ~/.cargo/bin)
cargo install --path .
cobserve --version
```

You can also build without installing. `cargo build --release` leaves the program at
`./target/release/cobserve`. The rest of this guide writes `cobserve` for either one.

### Claude Code and OpenCode (optional)

These are only needed for the sessions and for `ctrl+k` in a query session.

```sh
# Claude Code (or: brew install --cask claude-code · npm install -g @anthropic-ai/claude-code)
curl -fsSL https://claude.ai/install.sh | bash
claude            # once, in a normal terminal: sign in in the browser it opens, then quit

# OpenCode (or: npm install -g opencode-ai)
curl -fsSL https://opencode.ai/install | bash
opencode auth login
```

Sign in once in a normal terminal, before you use them inside cobserve. cobserve starts the
same `claude` and `opencode` you just installed. It does not log them in, and it never passes
them an API key. See the official
[Claude Code setup](https://code.claude.com/docs/en/setup) for other platforms.

---

## 4. Try it without a fleet

```sh
FAKE=1 cobserve
```

This shows a generated fleet: eight nodes and a ninth that joins after 20 seconds, a busy
Redash queue, and runaway queries. Everything works on it: every view, the keys, the mouse, query
sessions (they answer from the made-up fleet), and cancelling Redash jobs (in the made-up
queue). Claude, OpenCode and shell sessions run the real programs. Press `?` for the keys and
`q` to quit.

### A real ClickHouse on your laptop (optional)

`dev/local-rig.sh` starts a real ClickHouse in Docker: two nodes and a Keeper. The passwords
are made up on the spot and kept in `dev/local/.env`. It never contacts your fleet.

```sh
./dev/local-rig.sh up                         # two ClickHouse 24.10 nodes + Keeper
eval "$(./dev/local-rig.sh env)" && cobserve  # the app against them
./dev/local-rig.sh load                       # some long queries, so there is something to see
./dev/local-rig.sh credentials                # a --credential file with a login per node
./dev/local-rig.sh down                       # stop and remove it all
```

---

## 5. Connect it to your fleet

You need four things:

- a **seed**: the HTTP address of any one node, such as `http://clickhouse1.example.net:8123`;
- the **cluster** name;
- a **read-only login**;
- optionally, **Redash**.

cobserve finds the rest of the nodes itself in `system.clusters`, and looks again every minute.

### 5.1 The cluster's name

Run this on any node:

```sql
SELECT DISTINCT cluster FROM system.clusters;
```

### 5.2 A read-only ClickHouse user

A ClickHouse admin creates this once. It is a profile that is read-only and small, and a user
that can read the system tables:

```sql
CREATE SETTINGS PROFILE IF NOT EXISTS monitor ON CLUSTER ch_cluster
    SETTINGS readonly = 1, max_threads = 2, max_memory_usage = 6000000000, max_execution_time = 30;

CREATE USER IF NOT EXISTS monitor ON CLUSTER ch_cluster
    IDENTIFIED WITH sha256_password BY 'a long random password'
    SETTINGS PROFILE 'monitor';

GRANT ON CLUSTER ch_cluster SELECT ON system.* TO monitor;
```

- **What it reads**: `system.processes`, `system.asynchronous_metrics`, `system.events`,
  `system.settings`, `system.server_settings`, `system.replicas`, `system.parts` and
  `system.clusters`. For a query session's suggestions it also reads `system.databases`,
  `tables`, `columns` and `functions`.
- **A missing grant is named.** If one is missing, that node says so on screen, for example
  *no access · monitor needs SELECT on system.asynchronous_metrics*.
- **SQL on your own tables** in a query session needs them granted too, such as
  `GRANT SELECT ON analytics.* TO monitor`. Without that, a query session only sees the system
  tables. Whatever the grants, everything it runs stays read-only.
- **Users in `users.xml`** instead of SQL? [`dev/local/configs/users.xml`](dev/local/configs/users.xml)
  shows the same setup as XML.
- **The settings are a choice.** With `readonly = 1` in the profile, cobserve cannot lower the
  limits itself, so the profile's limits are the ones that apply. That is why they are small.

### 5.3 One login for every server

```sh
export CH_SEED_URLS=http://clickhouse1.example.net:8123   # a comma-separated list is fine too
export CH_CLUSTER=ch_cluster
export CH_USER=monitor
read -rs CH_PASSWORD && export CH_PASSWORD                # type it; it stays out of your history
cobserve
```

### 5.4 Or a login per server: the credential file

If the servers do not share a login, or you would rather keep secrets out of your shell, use a
file:

```sh
mkdir -p ~/.config/cobserve
cp credentials.example.yaml ~/.config/cobserve/credentials.yaml
chmod 600 ~/.config/cobserve/credentials.yaml      # cobserve warns when others can read it
$EDITOR ~/.config/cobserve/credentials.yaml
cobserve --credential ~/.config/cobserve/credentials.yaml
```

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
  # default_login: { user: monitor, password: "…" }   # for nodes discovery finds that are not listed

redash:                                   # optional
  url: https://redash.example.net
  api_key: "…"                            # an admin's API key
  redis_url: "redis://:password@redis-host:6379/0"
  email_domain: example.net
```

- **Keep the file outside your projects.** Sessions run programs in folders of your
  choice, and those programs can read files there. `~/.config/cobserve/` is a good place.
- **A password is only sent to its own server.** A node discovery finds that is not in the file
  uses `default_login` if there is one. Otherwise it shows *not polled*, rather than being sent
  another server's password.
- **Passwords are read exactly as written.** Quote one that starts with a special character
  (`"` `'` `#` `{` `[` `&` `*` `!` `|` `>` `%` `@`). [README.md](README.md#a-login-per-server---credential)
  has the details.

### 5.5 Redash and Redis

- **URL and key**: set `REDASH_URL` and `REDASH_ADMIN_API_KEY`, or the `redash:` section of the
  file. The key is the **API key on an admin's profile page** in Redash. A non-admin key gets
  *HTTP 403 · the API key has to be an admin's*.
- **Redis** (`REDIS_URL` or `redis_url:`): add it to see who is waiting and for how long, and to
  cancel a waiting job. cobserve only reads from Redis. The one write, a cancel, goes through
  Redash's API.
- **`EMAIL_DOMAIN`**: shows the people of your organisation by name alone (`grigol.gankava`)
  instead of their whole address.
- **Redash versions**: Redash 10 and newer (RQ) is what it is built against. Older, Celery-based
  versions are read through their own endpoint.

### 5.6 Airflow and Jira

- **Airflow** (panel 5): set `AIRFLOW_URL` (its web address, `https://airflow.example.net`),
  `AIRFLOW_USER` and `AIRFLOW_PASSWORD`, or an `airflow:` section in the file with `url`, `user`
  and `password`. It is the login of Airflow's web UI: its API is signed into the same way, with
  the login form, because an Airflow whose API takes only its own session (the default) refuses
  Basic auth. A login that is refused is not tried again for five minutes, or until `r`.
- **Jira** (panel 6): set `JIRA_URL` (`https://jira.example.net`) and `JIRA_TOKEN`, or a `jira:`
  section with `url` and `token`. The token is a **personal access token** of Jira Server or
  Data Center: Profile → Personal Access Tokens → Create token. The JIRA panel shows the tickets
  assigned to the token's owner.
- **Another board**: the columns are `In progress, In Review, Feedback, Done` unless
  `JIRA_STATUSES` (or `statuses:` in the file) names others, left to right; the last is where
  finished tickets go, and shows those resolved in the last `JIRA_DONE_DAYS` days (7).
- Both only read: Airflow and Jira are asked with GET, and nothing here can change a run or a
  ticket. Their secrets are taken out of every session's environment, like the others.

### 5.7 A launcher, so you type one word

On a Mac, the launcher can be an app: `./desktop/bundle.sh --install` makes **Cobserve.app**, with
its own window and icon. It reads `~/.config/cobserve/credentials.yaml` by itself, and
`~/.config/cobserve/env` for other variables (`KEY=value` lines) — an app from the Dock has no
terminal environment. [README.md](README.md#the-desktop-app) has the details.

In a terminal, a script does it:

Put this in `~/bin/cobserve-fleet` and run `chmod +x` on it. The secrets stay in the credential
file:

```sh
#!/usr/bin/env bash
export PRAYER_CITY=Jakarta          # or PRAYER=off
exec cobserve --credential ~/.config/cobserve/credentials.yaml "$@"
```

### 5.8 Is it working?

- **Footer**: the right side says *N nodes polled · slowest … ms · read-only*.
- **Band**: the **FLEET** line counts nodes, memory and CPU. The **REDASH** line counts what
  waits and runs, or says *not configured*.
- **A node in trouble** says why, in its row and in the insights: *unreachable*, *no access*
  (with the missing grant), *login refused for user …*, or *not polled*.
  [12](#12-troubleshooting) has the fix for each.
- **Airflow and Jira**: zoomed, the first line says the host, its version and *read 4s ago*; without a
  configuration they say what to set, and when a read fails they say why.

---

## 6. The screen at a glance

```
 ☗ cobserve · ch_paysera   04:21 ━━ 05:33 ━━ 11:45 ━━ 14:48 ━●┄ 17:50 in 1h57m ┄┄ 18:59    ● live   15:52:07 WIB  ← top bar
╭ 1 FLEET ─────────────── 8 runaway · 1 hot · 9 nodes ─╮╭ 2 REDASH QUEUE ────────── oldest 3m43s ✖ ─╮
│  big memory share · cpu · the hour as a graph         ││  jobs waiting · workers · the graph       │
╭ 3 NODES ─────────────────────────────────────────────────────────────── sorted by pressure · s ─╮
│  a tile per node, worst first                                                                    │
╭ 4 clickhouse3 · 93.1% of 64 GiB ─╮╭ 5 AIRFLOW ── last 24 h ─╮╭ 6 JIRA ──────────── mine ─╮
│  who holds the chosen node        ││  the day, the runs       ││  the flow, the tickets    │
╭ 7 TAPE ───────────── newest first ─╮╭ 8 SESSIONS ──────────── 4 · \ floats one ─╮
 1-8 panel   ⏎ open nodes   ←→↑↓ choose   e explain   p layout   : command      all · 9/9 nodes · 0.8 s · read-only   ← status bar
```

One screen of eight numbered panels. Each frame takes the colour of its worst finding — red,
amber, or dim when all is well — and says its gist at the right of its top border.

- **Focus and zoom**: `1`–`8` (or `tab`, or a click) puts the focus on a panel — its title turns
  into a chip. `⏎` (or a second click) opens it on the whole screen; `esc` comes back. While
  zoomed, `1`–`8` switch the zoom. `F1`–`F8` zoom a panel from anywhere, a session included.
- **The top bar**: the cluster, the day line — the prayer times where you are, the present a
  point on it, the next one counting down (a chip when the screen is narrow) — and the clock:
  `● live`, `○ paused` (`space`), or `◷ replay −2m` while you look back (`[` `]`, `t` for live).
- **The status bar**: the keys that work right now, from the same table as `?` and the palette,
  and how the last poll went. From 140 columns, this Mac too.
- **`e`** on any number explains it: the formula, the formula with the values, each input with
  the system table it came from. `⏎` there asks Claude about it, `y` copies it.
- **`:`** opens the palette: every action, fuzzy (`exp` finds *explain*), and with `tab` the
  objects — nodes, people, queries, jobs, DAGs, tickets, sessions, the tape. `?` lists the keys
  of the screen you are on. Settings live there too: `setup`, `tour`, `theme`, `clock`,
  `prayer line`, `local`.
- **Layout**: `p` cycles all → fleet (the nodes take the room) → ops (Airflow and Jira do);
  `+` cycles comfortable → compact → focus (no top bar).

**Colours and marks**: `✖` red is critical and `▲` amber is a warning (by DESIGN.md §7), green
is good news. `!` is a runaway query: 30 s or longer, or at 80 % of its own memory limit. `↯`
means no numbers, so the value is unknown rather than zero. `NEW` marks a node that just
joined. Graphs are on a fixed scale and print their window (`-4m`).

---

## 7. The eight panels

### 7.1 FLEET (`1`)

The fleet's memory share in big digits, with what it is of (`268/640 GiB memory`); the CPU share
beside it (`52/160 c`); queries, runaways and the worst replica lag under it; and the history as
a graph — memory the filled area, CPU the dots, both on the same 0–100 % scale. Zoomed: both
graphs with their ticks, then a row per node; `↑` `↓` choose one, `⏎` opens it.

### 7.2 REDASH QUEUE (`2`)

The jobs waiting in big digits, the running and the stale beside them, the workers as dots and
how many of the running jobs hold a runaway ClickHouse query. Zoomed it is the queue in full:

- **The table on top**: each queue's running, waiting, oldest wait and workers.
- **RUNNING**: every job a live worker holds — who, which query, on what, and, under **IN
  CLICKHOUSE**, the node and query it became, with live memory, cores and progress.
- **WAITING**: who has been waiting and for how long, against the 3-minute red line (needs Redis).
- **STALE**: entries Redash's started list still holds although no worker runs them, with why.

| Key | |
|---|---|
| `↑` `↓` | move. The job's SQL opens under it; `J` `K` scroll it |
| `⏎` | the job's query in ClickHouse: the node opens with that person chosen |
| `x`, then `y` | **cancel the job in Redash.** `x` opens a dialog that says what will happen; `y` does it, anything else keeps the job |

What a cancel does: **a waiting job** leaves the queue and never runs; **a running job** is
stopped on its worker (the row shows `⊘` until the worker lets go); **a stale entry** leaves
the started list. **A job on ClickHouse: its query keeps running in ClickHouse.** Redash's
cancel does not reach it, and cobserve never sends `KILL QUERY` — the dialog, the notice and the
tape say so and name the node. Every cancel Redash accepts is also a line on the tape.

### 7.3 NODES (`3`)

A tile per node, worst first: its memory bar and share, the last minutes as a sparkline, its
queries and runaways, its longest query. `←→↑↓` move across the tiles; the NODE panel follows.
`s` sorts (pressure, memory, CPU, name), `/` filters by name. Zoomed: bigger tiles with memory
and CPU and their denominators, and the person who holds the most. An unreachable node says
`↯` and why.

### 7.4 NODE (`4`)

The chosen node: one row a person, then `server · caches · merges`, each a bar of the node's
memory. The bars share one scale, so their filled cells add up to the node's (§5.3 made visible).
Zoomed: **WHO HOLDS IT** with GiB and cores per row; under the chosen row its longest query —
Redash number, elapsed, read, progress — and its SQL; at the right, memory and CPU over the
history, then lag, parts and free memory. `↑` `↓` choose a person, `e` explains the row, `⏎` or
`\` asks Claude about it (the row's numbers and SQL typed into its prompt), `c` opens a SQL
session on the node, `y` copies the query id.

### 7.5 AIRFLOW (`5`)

The day in numbers, a 24-hour strip of every DAG's runs (`▪` went well, `✖` failed, `▸` runs now)
on the clock's hours, then the runs that run (the stuck ones red), wait and failed, then the DAGs.
Zoomed it is the whole of it, read every 15 seconds:

- **RUNNING**: each run in progress, how long and how far its tasks are (`4/7 ━━━━╸━━`), amber
  past six hours, red past a day and **stuck since** its date.
- **QUEUED**: what waits to start, amber after an hour. **FAILED**: the day's failures, when and
  at which task.
- **ACTIVITY**: a line per DAG on a timeline, with its schedule, runs, last run, how long it took
  and when it runs next; from 140 columns the loom weaves the day.

`⏎` on a run lists its tasks and `⏎` on a task opens its log at its end; `⏎` on a DAG lists its
latest runs; `esc` goes back. `o` opens it in Airflow, `y` copies its link, `r` reads again now.

### 7.6 JIRA (`6`)

The flow across your board's columns (`4 › 4 › 1 › 3 done`), what is overdue, today's logged
hours against your working-day average, then your tickets with their due date or age. Zoomed:
**LOGGED** charts the hours of the month, then the board — the columns as lanes, a block per
ticket with its key, priority, summary, time in the column, due date and time logged. `←→` cross
the lanes, `↑↓` walk one, `⏎` shows the ticket in full here, `o` opens it in Jira, `y` copies its
link, `t` the month by ticket, `r` reads again.

### 7.7 TAPE (`7`)

Two strips — `✖` and `▲` events a minute over the last hour — beside the newest lines: a node
going hot or unreachable and recovering, runaways starting and ending (*probably killed* when
one vanished at its limit), the queue backing up and draining, your Redash cancels and your
notes. Zoomed: the whole tape with its drawer; `⏎` goes to what a line is about, `n` writes a
note onto it.

### 7.8 SESSIONS (`8`)

A row a session: what it runs (`✻` Claude, `▣` OpenCode, `▦` SQL, `❯` a terminal), its number, its
name, its folder and branch, and its state. See [8](#8-sessions).

### This Mac (`0`)

This machine has no panel: its CPU and memory join the status bar from 140 columns, and `0` (or
`F10`, or *local* in the palette) opens it — processes, memory pressure, and what Claude Code and
OpenCode used today and this month, with your plan's limits.

---

## 8. Sessions

cobserve runs programs inside itself — your own Claude Code, OpenCode, a shell, or a SQL
console — each the real program in a real terminal, with its own folder, conversation and
permission prompts.

- **Panel 8** lists them; `↑` `↓` choose one.
- **`⏎` on panel 8** zooms the sessions: the list beside the chosen session's screen. `⏎` again
  opens it full — every key is then the program's — and `ctrl+\` comes back to the list.
- **`\`** floats the chosen session over whatever panel you are on (a first Claude starts when
  there is none). Every key goes to the program; `ctrl+\` twice hides it again.

### 8.1 The prefix: `ctrl+\`

Inside a floating session every key belongs to the program, `q`, the digits and `ctrl+c`
included. The one exception is `ctrl+\`, which starts the prefix; the pane's bottom border says
what follows, and a box lists it if you wait a moment. After it:

| Key | |
|---|---|
| `1`…`6` | a panel, the pane hidden |
| `7`…`9` · `↑` `↓` | a session: the first three by number, or walk through all of them |
| `/` | find a session by name, folder, kind, server or number |
| `n` | a new session of the kind on screen, in a folder you pick |
| `c` · `o` · `t` · `q` | a new **C**laude, **O**penCode, **t**erminal or **q**uery (SQL) session |
| `f` | full screen ↔ floating |
| `r` | rename the session on screen |
| `x` `x` | close it (its conversation stays in *past conversations*) |
| `esc` | back to the session |
| `ctrl+\` | hide the pane |

`F1`…`F8` (a panel), `F9` (the next session) and `F10` (this Mac) work from anywhere, for a
terminal that keeps `ctrl+\` to itself.

**A new session's folder** is picked in a small file explorer: `↑` `↓` choose, `⏎` opens the
session there, `→` goes into a folder and `←` goes up, typing searches, `shift+tab` changes what
the session runs. The folder's own past conversations are listed too.

### 8.2 Claude Code

This is your own `claude`, signed in with your plan.

- **What it cannot see**: cobserve takes `ANTHROPIC_API_KEY` and its own credentials out of the
  session's environment.
- **A new line in the prompt**: `⇧⏎` where the terminal reports shift (the footer then says
  `⇧⏎ new line`). Otherwise use `ctrl+j`, `⌥⏎` (with ⌥ as Meta), or `\` then `⏎`. All of
  these work in any terminal.
- **The mouse wheel** pages Claude's screen up and down.
- **`ctrl+z`** is not passed on. Nothing inside the pane could bring a suspended Claude back.
- **Selecting text with the mouse**: hold `shift` (or `⌥` in iTerm2 and Terminal). `MOUSE=0`
  gives the mouse back to the terminal entirely.
- **`CLAUDE_CMD`** runs something else, or `claude` with arguments, for example
  `CLAUDE_CMD="claude --model opus"`.

### 8.3 OpenCode and a terminal

- **OpenCode**: your own `opencode`, signed in with `opencode auth login`. New lines work the
  same way as in Claude.
- **Terminal**: your login shell (`$SHELL`, or `SHELL_CMD`). Here `ctrl+z` does go through, for
  the shell's own jobs.

### 8.4 Query sessions: SQL on any server



**Open one** in any of three ways:

- `c` anywhere: a SQL session on the chosen node, floating;
- `ctrl+\` then `q`, which asks which server;
- **▦ Query** in the new-session picker.

**What it runs** is always read-only, stopped after 30 s, and capped at 1000 rows. It is
stopped on the server when you press `ctrl+c` or close the session. The server is the chip at
the top: click it, or press `ctrl+o`, to run on another server.

| Key | |
|---|---|
| typing | suggestions follow you: tables after `FROM`, columns of the tables you read, functions, keywords, formats |
| `tab` · `↑` `↓` · `esc` · `ctrl+space` | take a suggestion · choose · close them · ask for them anywhere |
| `⏎` | a new line, or runs the statement when it ends with `;` |
| `ctrl+r` | run it (only what is selected, when something is) |
| `⇧⏎` · `ctrl+⏎` / `⌘⏎` | always a new line · run it, as in Redash. These need a terminal that reports shift, ctrl and ⌘ with Enter |
| drag · `shift`+arrows · `ctrl+a` | select · select all. Then `⌫` deletes it, typing replaces it, `ctrl+c` copies, `ctrl+x` cuts |
| `ctrl+w` · `ctrl+u` · `ctrl+z` | delete a word · clear everything · undo the last big change |
| `↑` on the first line | what you ran before |
| `shift+tab` | into the answer: the arrows move, `y` copies the row, `Y` copies everything, `tab` comes back |

**Have Claude write it.** Press `ctrl+k`, say what you want in any language (for example *top
10 users by memory*, *make it faster*, or *why does it fail*), then press `⏎`.

- **What Claude is given**: the server's name and version, your text, and the tables your words
  point at, with their columns. No rows and no logins.
- **With part of the text selected**, Claude is asked about that part alone. Its answer replaces
  only that part, and is marked so you see what changed.
- **Nothing runs until you run it.** `ctrl+z` puts the old text back.
- **The helper**: `ctrl+t` switches between Claude and OpenCode, and `ASSISTANT=opencode` starts
  with OpenCode.
- **A query that failed**: `ctrl+k` then `⏎` sends Claude what the server said, so it can put
  the query right.

### 8.5 Sessions are kept

- **Across runs**: quit, or close the terminal, and the next start lists the sessions again,
  marked `↻`. Each takes its conversation up again when you open it.
- **Past conversations** (**↻ past conversations** in the sessions zoom): the conversations
  Claude Code and OpenCode had anywhere, newest first, to open in a new session.

---

## 9. Recipes for on call

**The Redash queue is backing up**

1. Panel 2 says it at a glance: how many wait, the oldest wait, and how many of the running
   jobs hold a runaway ClickHouse query. `⏎` zooms it; the drawer says why the workers are busy.
2. Look at **RUNNING**: the oldest and the `!` runaways are usually the cause.
3. `⏎` on one opens its node with that person chosen: who, how much memory, and how far along.
4. To free the worker, `x` and then `y`. If the query should stop in ClickHouse too, run
   `KILL QUERY WHERE query_id = '…'` there yourself.
5. Watch **WAITING** drain. The tape keeps the record.

**A node is hot**

1. Its tile is first in panel 3, red or amber; choose it and panel 4 says who holds it — the
   people and the server itself (merges, caches), on one scale that adds up to the node.
2. `4` then `⏎`: the chosen person's longest query, its SQL, and the node's memory over time.
3. `e` on a row: the formula and the system tables behind the number.

**Who is burning the fleet?** `:` then `tab` to *people*: everyone across the fleet, the most
memory first; `⏎` opens the node they hold most of.

**I need a quick diagnostic query.**

1. `c` (a SQL session on the chosen node, floating).
2. `ctrl+k`, then type *slowest queries in the last hour*, then `⏎`.
3. Read what Claude wrote, then `ctrl+r`.

**I want Claude Code on a project, with the fleet in sight.**

1. `\` floats a session; `ctrl+\` then `c` opens a new Claude.
2. Pick the project's folder.
3. Work as usual. `ctrl+\` twice hides the pane and the panels are all there; `\` brings it back.

---

## 10. Configuration

Everything is set with environment variables. The `--credential` file holds the logins, and
can also hold the cluster and Redash settings. `setup` in the palette (`:`) asks for the same
settings one step at a time, checks each against the service, and keeps them in
`~/.config/cobserve/config.toml` with the secrets in the macOS Keychain; the environment wins
over what it keeps.

| Command line | |
|---|---|
| `--credential FILE` | the servers with a login each, and optionally `cluster`, `redash:`, `airflow:` and `jira:` ([5.4](#54-or-a-login-per-server-the-credential-file)) |
| `--claude` | start with a Claude session floating |
| `--plain` | no screen: the panels' summaries as text, a block every poll |
| `-h` · `-V` | help · version |

**ClickHouse**

| Variable | Meaning | Default |
|---|---|---|
| `CH_SEED_URLS` | comma-separated `http://host:8123` seeds, optionally `http://user:password@host:8123` | required, unless the file lists servers |
| `CH_CLUSTER` | the cluster to discover the rest from | required, unless the file sets `cluster:` |
| `CH_USER` / `CH_PASSWORD` | the read-only login for every server without one of its own | required, unless every server has its own |
| `CH_HTTP_PORT` | the HTTP port of discovered nodes | `8123` |
| `POLL_MS` | how often the nodes are polled | `2000` |

**Redash**

| Variable | Meaning | Default |
|---|---|---|
| `REDASH_URL` / `REDASH_ADMIN_API_KEY` | the Redash admin API | unset: *not configured* |
| `REDIS_URL` | Redash's Redis (read-only), for the waiting jobs | unset: counts only |
| `EMAIL_DOMAIN` | your organisation's domain: its people are shown by name alone | unset |

**Airflow and Jira**

| Variable | Meaning | Default |
|---|---|---|
| `AIRFLOW_URL` / `AIRFLOW_USER` / `AIRFLOW_PASSWORD` | Airflow 2 for panel 5, and the login of its web UI | unset: panel 5 says how |
| `JIRA_URL` / `JIRA_TOKEN` | Jira Server or Data Center for panel 6, and a personal access token | unset: panel 6 says how |
| `JIRA_STATUSES` | the board's columns, left to right, comma-separated | `In progress,In Review,Feedback,Done` |
| `JIRA_DONE_DAYS` | how many days of finished tickets the last column shows | `7` |
| `JIRA_STRUCTURE` / `JIRA_TEAM_PREFIX` | the team's board (`d` in panel 6): a Jira Structure's number, and the first word of the team's summaries; `m` moves a ticket after `y` | unset (off) / `DE` |

**Sessions**

| Variable | Meaning | Default |
|---|---|---|
| `CLAUDE_CMD` · `OPENCODE_CMD` · `SHELL_CMD` | what a Claude, OpenCode or terminal session runs | `claude` · `opencode` · `$SHELL` |
| `ASSISTANT` | `opencode` to have OpenCode answer `ctrl+k` | Claude |
| `TELEGRAM_BOT_TOKEN` · `TELEGRAM_TOKEN_FILE` | the Telegram bot's token, or a file it is in | no bot |
| `TELEGRAM_CHAT_ID` | the chats the bot answers | unset — it answers only with the chat's id |

**The day, the clock and alerts**

| Variable | Meaning | Default |
|---|---|---|
| `PRAYER_CITY` / `PRAYER_AT` | where the prayer times are for: a city (`Bandung`, `Medan`, `Kuala Lumpur`…) or `lat,lon` | the city of your time zone |
| `PRAYER_REMIND` | minutes of warning before a prayer (`0` for none) | `10` |
| `PRAYER` | `off` hides the prayer times | on |
| `NOTIFY` | `bell` for the terminal's bell only, `off` for the screen only | desktop notification + bell |
| `TIME` | `utc` starts the clock on UTC | local time |

**The look**

| Variable | Meaning | Default |
|---|---|---|
| `THEME` | `dark` (or `soga`), `light` (or `mori`) or `mono` — Cobserve.app follows it too | `dark` |
| `CHROME_WARN_PCT` / `CHROME_CRIT_PCT` | when Chrome (or another Chromium browser) holds too much of the Mac's memory, % of RAM by RSS; `CHROME_WARN_PCT=off` for never | `40` / `60` |
| `NO_COLOR` | any value: no colour at all (the marks still say how bad) | unset |
| `MOUSE` | `0` leaves the mouse to the terminal | on |
| `FAKE` | `1`: a generated fleet, no network | unset |

---

## 11. What it keeps on your machine

- `~/.local/state/cobserve/sessions.json` (or under `$XDG_STATE_HOME`, mode 600): the list of
  sessions, each with its kind, folder, name and conversation id, so the next start shows them
  again. It holds no passwords.
- `~/.cache/cobserve/assist/`: the empty folder the `ctrl+k` helper runs in.
- Your credential file, wherever you put it.

That is all. There is no telemetry, and nothing leaves your machine except the requests to your
own ClickHouse, Redash and Redis, and what Claude Code or OpenCode send when you use them.

---

## 12. Troubleshooting

**Starting cobserve**

| You see | Why | Do |
|---|---|---|
| `CH_SEED_URLS is required` (or `CH_CLUSTER`, `CH_USER`) and it exits | nothing says where the fleet is | [5.3](#53-one-login-for-every-server) or [5.4](#54-or-a-login-per-server-the-credential-file), or `FAKE=1` to try it |
| the screen is garbled after a crash | the terminal was left in raw mode | type `reset` and press `⏎` |

**ClickHouse nodes**

| You see | Why | Do |
|---|---|---|
| a node: *unreachable* / *connection refused* | your machine cannot reach it on that port | check the VPN, `curl http://host:8123/ping`, and `CH_HTTP_PORT` |
| a node: *no access · monitor needs SELECT on system.…* | the user lacks that grant | `GRANT SELECT ON system.* TO monitor` ([5.2](#52-a-read-only-clickhouse-user)) |
| a node: *login refused for user …* | wrong password for that server | fix it in the file or `CH_PASSWORD` |
| a node: *not polled* | discovery found it, but there is no login for it | add it to the file, or set `default_login` / `CH_USER` |

**Redash**

| You see | Why | Do |
|---|---|---|
| REDASH: *not configured* | no Redash settings | [5.5](#55-redash-and-redis) |
| REDASH: *HTTP 401/403 · the API key has to be an admin's* | the key is not an admin's | use an admin's API key |
| REDASH: *a web page, not JSON* | the URL is not Redash's own, or the key is wrong | check `REDASH_URL` |
| WAITING shows only a count | Redis is not set | add `REDIS_URL` |
| a cancel: *Redash no longer has it* | the job finished just before | nothing to do |

**Airflow and Jira**

| You see | Why | Do |
|---|---|---|
| Airflow: *login refused* | wrong Airflow user or password | fix `AIRFLOW_USER` / `AIRFLOW_PASSWORD`, then `r` (otherwise it asks again in 5 minutes) |
| Airflow: *signed in, but this login may not read the DAGs* | the user's role lacks DAG read access | ask an Airflow admin for a role that can read DAGs |
| Airflow or Jira: *a web page, not JSON* · *HTTP 404* | the URL is not the service's own address | the address you open in the browser, without a path |
| Jira: *HTTP 401 · the token was refused* | the token is wrong or expired | make a new one under Profile → Personal Access Tokens |
| Jira: *The value 'X' does not exist for the field 'status'* | `JIRA_STATUSES` names a status Jira does not have | use the names your board's columns show |
| a row says *◌ read 3m ago · …* | the last read failed; what is on screen is the read before | the reason follows; `r` tries again |

**Sessions**

| You see | Why | Do |
|---|---|---|
| a Claude session asks you to log in, or says it cannot | `claude` was never signed in | run `claude` once in a normal terminal and sign in |
| `⇧⏎` sends the message instead of a new line | the terminal does not report shift with Enter | `ctrl+j` (the footer names the key that works) |
| `ctrl+\` does nothing | the terminal keeps it for itself | `F1`…`F9`, or click the tabs |
| `ctrl+k` opens something else (a browser shortcut) | something outside the terminal takes it | click *asks Claude* under the text, or `ctrl+g` |
| cannot select text with the mouse | cobserve has the mouse | hold `shift` (`⌥` in iTerm2 and Terminal), or `MOUSE=0` |

**The look**

| You see | Why | Do |
|---|---|---|
| colours look wrong or washed out | the terminal is not 24-bit | `COLORTERM=truecolor` if it is, `THEME=light` on a light background, `NO_COLOR=1` for none |
| prayer times are a few minutes off | the city is guessed from the time zone | `PRAYER_CITY=YourCity`, or `PRAYER_AT=lat,lon` |

---

## 13. Updating and removing

```sh
cd cobserve && git pull && cargo install --path .     # update

cargo uninstall cobserve                                # remove the program
rm -rf ~/.local/state/cobserve ~/.cache/cobserve        # and what it kept
```
