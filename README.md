# cobserve

*co-observe* — keep watch over a data fleet together: you, the people using it, and the AI
sessions you work with, all in one terminal. It **observes**: nothing in it can change a
ClickHouse query or a table. The one thing it does is cancel a Redash job you pick, after
asking — what Redash's own Cancel button does.

A terminal monitor for on call, as **mission control**: one screen of eight numbered panels —
the ClickHouse fleet, the Redash queue, every node as a tile, **who** holds the chosen node, **every
Airflow DAG's day**, **your Jira tickets**, a tape of everything that changed, and your sessions —
each framed in the colour of its worst finding and a key away from the whole screen. In the same
window Claude Code, OpenCode or a shell, floating over any panel, and a **SQL console on any
server** of the fleet, read-only, that suggests as you type and has Claude or OpenCode write the
query for you. `e` on any number says where it comes from. Around it all, the **day**: your local
time, the prayer times where you are, and a reminder before each.

![The dashboard](docs/screenshots/120x36-dash.png)

**New here?** [GUIDE.md](GUIDE.md) goes from zero to everyday use:

- what you need, and installing it;
- trying it with no fleet (`FAKE=1 cobserve`);
- creating the read-only ClickHouse user and connecting it to your fleet and Redash;
- a tour of every panel and session, recipes for on call, and troubleshooting.

The quick version:

```sh
git clone https://github.com/abduldjafar/cobserve.git && cd cobserve
cargo install --path .                 # Rust 1.88+; puts `cobserve` in ~/.cargo/bin
FAKE=1 cobserve                        # a generated fleet: look around, : for everything, q quits
cobserve --credential ~/.config/cobserve/credentials.yaml    # your fleet (GUIDE.md §5)
```

**On a Mac, as an app of its own**: `./desktop/bundle.sh --install` puts **Cobserve.app** in
`/Applications` — a window with its own icon and menu, a terminal inside it that runs cobserve,
so it opens from the Dock or Spotlight and needs no terminal of yours. See
[the desktop app](#the-desktop-app).

It reads ClickHouse directly over HTTP, read-only, so it keeps working when the web app is
down. `DESIGN.md` is the contract for every number on screen (§5); the additions here sit
on top of it and are listed in `DESIGN.md` §13.

## What is on screen

| Panel | Key | What it answers |
|---|---|---|
| **FLEET** | `1` | The fleet's memory share in big digits with what it is of, the CPU share beside it, queries and runaways, and the history as a graph on a fixed scale. Zoomed: both graphs with ticks, and a row per node. |
| **REDASH QUEUE** | `2` | Jobs waiting in big digits, running and stale, the workers, and how many running jobs hold a runaway ClickHouse query. Zoomed: per queue what **runs**, **waits** and is **stale**; every running job with its person, query, data source and the ClickHouse query it became — live memory, cores, progress; who waits and for how long. `x`, then `y`, cancels a job in Redash. |
| **NODES** | `3` | A tile per node, worst first: memory bar and share, the last minutes, queries, runaways, the longest. Zoomed: bigger tiles with memory and CPU and their denominators. |
| **NODE** | `4` | The chosen node: who holds it — user rows (a Redash query resolved to the person behind it) plus the server's own share, on one scale that adds up to the node (§5.3). Zoomed: GiB and cores per row, the chosen query and its SQL, memory and CPU over time. |
| **AIRFLOW** | `5` | Every DAG's last 24 hours: a strip of the day, what **runs** now (a run going for a day is **stuck**), what **waits**, what **failed** and at which task. Zoomed: the runs, their tasks and logs, and a line per DAG with its day on a timeline; from 140 columns the loom weaves the day. |
| **JIRA** | `6` | Your tickets: the flow across your board's columns, what is overdue, today's logged hours. Zoomed: the board as lanes, each ticket with its priority, time in the column, due date and time logged, the ticket in full, the month by ticket. |
| **TAPE** | `7` | What changed, newest first, beside two strips of `✖` and `▲` a minute: nodes going hot or unreachable and recovering, runaways starting and ending — *"probably killed"* when a query vanished at its memory limit — the queue backing up and draining, your cancels and notes. |
| **SESSIONS** | `8` | Claude Code itself — your own `claude`, signed in with your plan — OpenCode, your own shell, or a query session on a server of the fleet, in as many named sessions as you need. `\` floats one over any panel. |
| **this Mac** | `0` | This machine, as Activity Monitor shows it — CPU, memory and its pressure, every process or program — and what Claude Code and OpenCode used today and this month, with the plan's own limits. Its line joins the status bar from 140 columns. |

Each panel's frame takes the colour of its worst finding and says its gist on its border. `:` is
the palette — every action, and every node, person, query, job, DAG, ticket, session and tape
line, fuzzy — and `e` explains the number under the cursor: its formula, the formula with the
values, and the system table each input came from.

![A node zoomed](docs/screenshots/120x36-zoom-node.png)

### The insights

One short line for each thing in trouble, worst first — a node, the Redash queue, a Redash query
running twice — is what the panels' frames sum up, and what `cobserve --plain` prints under the
panels' summaries. What a line can say: a node without numbers (**unreachable**, **no access**
with the missing grant, **login refused**, **not polled**); memory or CPU hot and whether a
**person** or **the server itself** holds it; memory **climbing** to full (*"↗ full in ~3m30s"*);
a query close to **its own** memory limit; long queries, replica lag; the queue backing up and
whether its workers are stuck on long ClickHouse queries; the same Redash query running twice.

### The Redash queue

The queue reads what the Redash admins' own queue script reads, with the same admin API key:
`/api/admin/queries/rq_status` for the queues and the workers, `/api/users/{id}` for who (name
and address), `/api/queries/{id}` for the query (name and SQL) and `/api/data_sources` for what
it runs on (name and type). The one thing it sends is a cancel you asked for.

- **RUNNING** is a job a live worker holds. Redash's started list is not enough: when a worker
  dies its job stays there, for months without a time limit.
- **STALE** is the rest of that list, each with its reason — *cancelled*, *over a day old*,
  *no worker holds it*. They take no worker; a Redash admin can clear them.
- **IN CLICKHOUSE** follows a running job into ClickHouse by the `Job ID:` Redash writes into
  the comment of every query (and by its query number when there is none): `⏎` goes to the
  query on its node. A job on another database says so — *mysql · not ClickHouse*, *runs inside
  Redash* for Query Results.
- **WAITING** has names and ages only with Redis (`redis_url:`); Redash's API only counts the
  waiting jobs.
- **Cancel** a job when the queue backs up: `x` on it says what a cancel will do, `y` does it —
  `DELETE /api/jobs/<id>`, as Redash's Cancel button and the admins' script do — and any other
  key keeps it. A waiting job leaves its queue; a running one is stopped on its worker, marked
  `⊘` until the worker has let go; a leftover leaves the started list. A query it started in
  ClickHouse **runs on**: Redash's cancel does not reach it, and nothing here sends
  `KILL QUERY` — the question and the notice after it say so, and on which node, and the tape
  keeps a line for every cancel.

![x on a running job: what a cancel will do, before anything is sent](docs/screenshots/120x36-cancel-dialog.png)


### Airflow: every DAG's day

The AIRFLOW panel reads Airflow's stable REST API (`/api/v1`, Airflow 2): every 15 s what runs and waits
and how the scheduler and the triggerer are, every minute the runs of the last 24 hours, every
five minutes the DAGs. It signs in as the web UI does — the login form and its CSRF token, then
the session cookie — because an Airflow whose API takes only its own session (the default)
answers Basic auth with 403. A refused login is not sent again for five minutes, or until `r`:
a wrong password cannot lock the account. Everything after the login is GET.

```text
  ─ RUNNING · 2 · 1 stuck ────────────────────────────────────────────────────────────────────
  ▌ ✖ test_clickhouse_connection         manual · Jan 15     262d04h    0/1 ━━━━━━━━  stuck since Jan 15
    ▸ statement_daily_agg_reload         manual · 13:39        2h12m    4/7 ━━━━╸━━━  ▸ reload_partitions
  ─ FAILED · 3 in the last 24 h ──────────────────────────────────────────────────────────────
    ✖ kyc_onboarding_tables              scheduled · 12:30    10m48s  ended 12:40 · 3h ago  at build_cohorts
  ─ ACTIVITY · 13 DAGs ran in the last 24 h ──────────────────────────────────────────────────
    DAG                                SCHEDULE        RUNS    18    00    06    12     LAST RUN     TOOK
    clickhouse_replication_check       every 15m         96  ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪     7m ago    1m44s
    cbk_accounts_report                :15 every 2h   12 ✖1  ·▪·▪·▪·▪·▪·▪·▪·✖·▪·▪·▪·▪    37m ago    4m00s
    dbt_daily_models                   1 day              1  ········▪━━━············     7h ago    1h57m
```

- **RUNNING**: every run in progress — for how long, how far its tasks are, which task runs and
  which waits for its retry. Past six hours a run is amber; past a day it is red and **stuck
  since** its date: a run whose worker died, left running for months.
- **QUEUED**: what waits to start; amber after an hour.
- **FAILED**: the day's failed runs, when each ended and the task it failed at — or *no task
  failed · 1 skipped* when the scheduler, a person or a timeout failed the run itself.
- **ACTIVITY**: a line per DAG that ran in the last 24 hours, its day on a timeline — a cell an
  hour (half an hour from 160 columns), on the hours of the clock on screen: `▪` runs that went
  well, `✖` a failure, `▸` one running now, `◌` one waiting, `━` a run that went on past its
  cell. Beside it the schedule as Airflow says it, shortened (`every 15m`, `:20 hourly`,
  `Sun 22:00`, `triggered`), the day's runs (`12 ✖1`), when it last ran, how long that took and
  when it runs next — and from 150 columns its owner.

The first read comes in two: what runs now in a few seconds, the day a dozen more after it.

**Detail, here, not in the browser.** `⏎` on a run lists its tasks — state, when each started,
how long it took, its try, operator and worker — and `⏎` on a task opens its log, at its end,
where a failure says why, the traceback in red. `⏎` on a DAG lists its latest runs, and `⏎` on
one of them its tasks. `esc` goes back a page. `o` opens what is under the cursor in Airflow
itself, `y` copies its link, `r` reads the page (or the view) again. Set `AIRFLOW_URL`, `AIRFLOW_USER` and `AIRFLOW_PASSWORD` (the login of its web UI), or
an `airflow:` section in the credential file.

![Airflow zoomed](docs/screenshots/120x36-zoom-airflow.png)



### Jira: your tickets

The JIRA panel is your Jira board, for you alone: the tickets assigned to the token's owner, in the
board's columns — by default **In progress**, **In Review**, **Feedback** and **Done**, the last
only for what was resolved in the last seven days, as the board itself hides older ones. It
reads the REST API of Jira Server and Data Center (`/rest/api/2`) with a personal access token,
every minute, GET only.

```text
    In progress 4  ›  In Review 4  ›  Feedback 1  ›  Done 8 in 7 days     ✖ 1 overdue  ▲ 1 due soon

    KEY        SUMMARY                                                  PRIORITY  IN COLUMN  DUE           LOGGED
  ─ IN REVIEW · 4 ──────────────────────────────────────────────────────────────────────────────────────
    DATA-2647  DE Client account balances as a daily warehouse table   ASAP             4d  overdue 2d     7h12m
    DATA-2773  DE KYC onboarding tables for funnel cohorts             High             2d                    4h
```

- The board's flow on one line, and how many open tickets are **overdue** (red) or due **today
  or tomorrow** (amber).
- Each column under its rule, ranked as the board ranks it — by priority, then the latest
  touched — and Done by when each was finished: the key, the summary (the team's tag, `DE -`,
  drawn faint), the priority (`URGENT` and `ASAP` red), how long it has been in the column —
  from its history, read again only when the ticket changes — its due date and the time logged.
- The drawer has the rest: the whole summary, who reported it, when it was made and updated, its
  parent and labels, and its link.
- **`⏎` shows the ticket in full, here**: its fields, its description — Jira's markup made
  readable, headings, lists and code blocks drawn as such — its sub-tasks and links, and every
  comment. `esc` goes back, `o` opens it in Jira, `y` copies the link, `r` reads again.
- **The month's time**, over the columns: the hours you logged this month on any ticket, in
  total and per working day, and a bar for every day of the month — the hours over it, weekends
  faint, today lit — read every five minutes from your worklogs.

```text
    LOGGED · October 2026   17h10m in 4 working days · 4h17m a working day · today 0h
         14   3           ·   ·
   15h │▇▇▇
       │███ ▅▅▅
          1   2   3   4   5   6   7   8   9  10 …
```
- **`t`: which tickets took the time, day by day.** A page over the board: the month's chart on
  top, and under it a row for every ticket you logged on this month — a bar under each day, in
  the chart's own columns, so a day reads straight down — with its month's sum; then the day
  chosen, its tickets and each one's share of it. `← →` choose the day (its column lit all the
  way down), `↑ ↓` a ticket, `⏎` opens that ticket in full, `esc` goes back.


Set `JIRA_URL` and `JIRA_TOKEN` (in Jira: Profile → Personal Access Tokens), or a `jira:` section
in the credential file. For another board, `JIRA_STATUSES` names its columns, left to right
(the last is where finished tickets go), and `JIRA_DONE_DAYS` how far back that one reaches.

![Jira zoomed: the board](docs/screenshots/120x36-zoom-jira.png)


### The day, and prayer times

The top bar's middle is the day where you are, from Subuh to Isya: every prayer time a
stop on it, the present a point moving between them — the way behind solid, the way ahead
dotted — and the next stop saying how far off it is.

```text
04:21 ━━ 05:33 ━━ 11:45 ━━ 14:48 ━●┄ 17:50 in 1h57m ┄┄ 18:59        (names from 80 free columns)
```

Ten minutes before a prayer its reminder takes the line — `◷ Maghrib in 9:29` — and
is said once beyond the screen: the terminal's bell and a notification of your desktop (through
iTerm2, Ghostty or WezTerm themselves, through macOS or `notify-send` elsewhere). At its time it
says so for five minutes. `d` waves it away.
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


### Claude, OpenCode and a terminal in the monitor

Panel 8 runs programs in a terminal inside the monitor — the official `claude`, `opencode`, or
your own shell — each the same as in a terminal tab of its own: any project, any question, its
own permission prompts.

- **Claude** is Claude Code signed in with your **Pro or Max plan**: install it and sign in once
  (`claude`, then `/login`). `CLAUDE_CMD` runs something else, or the same with arguments:
  `CLAUDE_CMD="claude --model opus"`.
- **OpenCode** signs in its own way: install it (`curl -fsSL https://opencode.ai/install | bash`)
  and sign in once with `opencode auth login`. `OPENCODE_CMD` changes the command.
- **Terminal** is your login shell (`$SHELL`); `SHELL_CMD` runs another one.
- Nothing here uses an API key: `ANTHROPIC_API_KEY` is taken out of every session's
  environment, so Claude Code bills the plan you signed in with even when your shell has a
  key set; so are the monitor's own credentials (`CH_PASSWORD`, `CH_SEED_URLS`,
  `REDASH_ADMIN_API_KEY`, `REDIS_URL`, `AIRFLOW_PASSWORD`, `JIRA_TOKEN`).
- Each session works in a folder of its own — the first in the one the monitor was started
  from. What runs there can read files there, so keep the credential file somewhere else.
  `--claude` starts with a Claude session floating.

**Three ways to see one.** Panel 8 lists them: what it runs (`✻` Claude, `▣` OpenCode, `❯` a
terminal, `▦` SQL), its number, its name — the one you gave it, what the program says it is
working on, or its folder — its folder and branch, and its state (`● running`, `! attention`
when it rang while out of sight, `↻ kept`, `✕ ended`). `⏎` zooms it: the list — by project once
there are many — beside the chosen session's screen, with what Claude Code and OpenCode used today
over it; `⏎` again opens the session full, every key the program's, and `ctrl+\` comes back to the
list. And `\` **floats** the chosen session over whatever panel you are on — the fleet stays in
sight around it — `f` in the prefix makes it full, `ctrl+\` twice hides it.

**Sessions are kept.** Quit — or close the terminal — and the next start lists them again,
marked `↻`; each takes its conversation up where it was when you open it (`claude --resume`,
`opencode --session`). **Past conversations** (the entry under the sessions in the zoom) lists the
conversations Claude Code and OpenCode had anywhere — in another terminal, in an earlier run — by
title or first prompt, folder and age, newest first; one opens in a new session in its own folder.
Only the names of conversations are read, and nothing leaves the machine; the list of sessions
is kept in `~/.local/state/cobserve/sessions.json`.

A new session's folder is picked as in a file explorer, no typing needed: it starts in the
folder the session on screen works in, a click on a folder goes into it, a click on a step of
the path above (`~ › work › cobserve`) goes back up to it, and **Open the session here** (or
`⏎`) starts it there. Above them, a click chooses what it runs — Claude, OpenCode, Terminal —
or `shift+tab` does. Typing searches the folders below by name, and a path (`~/`, `/`, `../`) is
completed like a shell does. A repository shows its branch.

| In a floating session | |
|---|---|
| any key | goes to the session — `q`, the digits and `ctrl+c` too |
| `⇧⏎` · `ctrl+j` | a new line in Claude's or OpenCode's prompt. `⇧⏎` where the terminal tells it from `⏎` (the kitty keyboard protocol), `ctrl+j`, `⌥⏎` or `\` then `⏎` in any |
| `ctrl+\` then `1`…`6` | a panel, the pane hidden |
| `ctrl+\` then `7`…`9` · `↑` `↓` | another session: the first three by number, or the one before or after |
| `ctrl+\` then `/` | find one: part of its name, folder, kind, server or number |
| `ctrl+\` then `n` · `c` `o` `t` `q` | a new session of the kind on screen · a new Claude, OpenCode, terminal or query session |
| `ctrl+\` then `f` · `r` · `x` `x` | full ↔ floating · rename it · close it (its conversation stays in **past conversations**) |
| `ctrl+\` then `esc` · `ctrl+\` `ctrl+\` | back to the session · hide the pane |
| `F1`…`F8` · `F9` · `F10` | a panel · the next session · this Mac — from anywhere, without `ctrl+\` |

![A session floating over the dashboard](docs/screenshots/120x36-float-session.png)

![The sessions zoomed](docs/screenshots/120x36-zoom-sessions.png)


### SQL on any server: query sessions

A query session is a SQL console on one server of the fleet, beside the other sessions:
`c` opens one on the chosen node, floating, `ctrl+\ q` in a floating session asks which
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
- **Quick to change**: drag over the text with the mouse, or `shift` with the arrows, to select
  — `ctrl+a` selects all of it. `⌫` then takes all that out at once and typing replaces it;
  `ctrl+c` copies it, `ctrl+x` cuts it, `ctrl+r` runs only what is selected. `ctrl+w` takes the
  word before the cursor, `ctrl+u` the whole text (`ctrl+z` brings it back).
- **Claude or OpenCode writes it**: `ctrl+k` (or a click on *asks Claude* under the text;
  `ctrl+g` does the same where nothing outside the terminal takes it first — Gemini in Chrome
  does on a Mac) opens a line to say what it should do, in any language — *top 10 users by
  memory*, *make it faster*, *why does it fail* — and `⏎` asks; with nothing typed it writes
  what the text's `--` comments say. With part of the text selected it is asked about that
  part alone, the rest given for context, and what it writes takes that part's place — marked,
  so you see what changed, while `ctrl+r` still runs all of it.
  Claude Code answers signed in with your Pro or Max plan (`claude -p`, no tools, no API key,
  nothing saved), OpenCode as you signed it in (`opencode run`, every permission denied); both
  run in an empty folder of their own, without the monitor's secrets. They get the server's name
  and version, today's date (so *last week* means something), your text, the tables its words
  point at with their columns — words of another language too: *transaksi* points at
  transactions, transfers and payments — and every other table of the server by name, with
  about how many rows it holds and its comment. When those are not the right ones it answers
  `-- need: db.table, …` instead, and is asked again with their columns: a second round you see
  only as a longer *writing…*. Metadata only — never a row of a table, never a login. On a query that failed, `ctrl+k` sends what the server said,
  to put it right. What comes back takes the text's place (or the selected part's) — read it,
  then `⏎` runs it; `ctrl+z` puts back what was there. `ctrl+t` or a click on the chip switches between Claude
  and OpenCode; `ASSISTANT=opencode` starts with OpenCode.
- **The answer** is a table under it — numbers on the right, `NULL` dimmed, as many columns as
  fit — with how long it took and what the server read. `shift+tab` goes into it: the arrows
  move through it, `y` copies the row and `Y` all of it as tab-separated text (through the
  terminal, and `pbcopy` on a Mac).
- What ran is kept: `↑` on the first line goes back through it, and the session — its server,
  its text and what ran — comes back with the next start.




| In a query session | |
|---|---|
| `⏎` | a new line — or runs the statement once it ends with `;` |
| `⇧⏎` · `ctrl+⏎` `⌘⏎` | always a new line · run it, as in Redash (where the terminal tells them from `⏎`) |
| `ctrl+r` | run what is there — what is selected, when something is |
| a drag · `shift`+arrows · `ctrl+a` | select · select all — then `⌫` deletes it, typing replaces it, `ctrl+c` copies, `ctrl+x` cuts, `esc` lets go |
| `ctrl+w` · `ctrl+u` | delete the word before the cursor · clear the whole text |
| `tab` · `↑` `↓` · `esc` | take a suggestion · choose one · close them — `tab` after `FROM ` lists the tables |
| `ctrl+k` · `ctrl+t` | say what Claude (or OpenCode) should write, then `⏎` — on a selection, about that part alone · the other one |
| `ctrl+z` | put back what was there before Claude wrote, or before a clear or a deletion |
| `ctrl+c` | stop the query or the writing · clear the text |
| `ctrl+o` | run on another server |
| `shift+tab` | the answer: the arrows move, `y` `Y` copy, `tab` comes back |
| `↑` on the first line | what ran before |
| `ctrl+\` | the prefix, as in every floating session |

### Reading the screen

| | |
|---|---|
| `✖` `▲` | critical (red) · warning (amber), by `DESIGN.md` §7 |
| `!` | runaway query: ≥ 30 s, or ≥ 80% of its own memory limit |
| `↯` | node without numbers — unreachable, no access, login refused or not polled; unknown, never zero |
| `▁▂▃▅▇` `⣀⣤⣶⣿` | the last 4 minutes on a fixed scale, each cell its worst moment, coloured by severity (braille by default, `GRAPH=block` or `ascii`) |
| `NEW` | joined the fleet during this session |
| `━━●┄┄` | the day: behind, now, ahead — prayer times at the place |
| `↻` | a session kept from the last run, or a conversation to take up |
| `◷` | replaying a minute (`[` `]`, `t` back to live) |

In the NODE panel the bars share one scale, so a node's people and its server fill exactly the
node's share between them (§5.3).

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
  optional `redash:` section holds `url`, `api_key`, `redis_url` and `email_domain`; `airflow:`
  holds `url`, `user` and `password`; `jira:` holds `url`, `token`, and optionally `statuses`
  and `done_days`.
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
| `AIRFLOW_URL` / `AIRFLOW_USER` / `AIRFLOW_PASSWORD` | Airflow 2 for panel 5, and the login of its web UI | optional — panel 5 says how to set it |
| `JIRA_URL` / `JIRA_TOKEN` | Jira Server or Data Center for panel 6, and a personal access token | optional — panel 6 says how to set it |
| `JIRA_STATUSES` | the board's columns, left to right, comma-separated; the last is where finished tickets go | `In progress,In Review,Feedback,Done` |
| `JIRA_DONE_DAYS` | how many days of finished tickets the last column shows | `7` |
| `CLAUDE_CMD` | what a Claude session runs, with its arguments | `claude` |
| `OPENCODE_CMD` | what an OpenCode session runs | `opencode` |
| `SHELL_CMD` | what a terminal session runs | `$SHELL`, else `/bin/sh` |
| `TELEGRAM_BOT_TOKEN` / `TELEGRAM_TOKEN_FILE` | the Telegram bot's token, or a file it is in (BotFather's message as it came will do) | optional — no bot |
| `TELEGRAM_CHAT_ID` | the chats the bot answers, comma-separated; without it, it answers anyone with their chat's id only | unset |
| `MOUSE` | `0` leaves the mouse to the terminal: no clicks, but selection without a modifier | on |
| `POLL_MS` | ClickHouse poll interval | `2000` |
| `CH_TIMEOUT_MS` | how long a node has to answer one request before it reads `↯ unreachable` — over a VPN, `5000` | `1500` |
| `THEME` | `dark` (or `soga`), `light` (or `mori`) or `mono` — Cobserve.app follows it too | `dark` |
| `NO_COLOR` | any value: no colour at all (glyphs still carry severity) | unset |

Colour depth follows the terminal: 24-bit when `COLORTERM` says so, the 256-colour palette
otherwise, the 16 ANSI colours (no painted background) on anything older.

The palette is cobserve's own, **Tarum**, after the dyes of hand-made batik: the dark ground is
cloth dyed in soga — the icon's warm brown, soft enough to sit in front of all day — the text
the cream of mori, the undyed cotton, and the accent the indigo of tarum leaves. Severity keeps
to the dyes that mean it — mengkudu red, kunyit yellow, a cool leaf green that stays apart from
the red for a red-green blind eye — and nothing else is coloured. `THEME=light` (or `mori`) is
the same cloth before the dye: a mori ground, soga ink, a deep tarum. In Cobserve.app the frame
round the terminal is a step lighter than the screen (darker on mori), the live mark breathes
while all is well, and the tab underline slides to the view on screen.

![Light theme](docs/screenshots/120x36-light.png)

## Keys

| Key | |
|---|---|
| `1`…`8` · `tab` | focus a panel (its title turns into a chip) · while zoomed, switch the zoom |
| `⏎` · `esc` | the focused panel on the whole screen · back; inside a zoom `⏎` is the panel's own open — a node, a job's query, a run's tasks, a ticket in full, a tape line's subject |
| `↑ ↓ ← →` `h j k l` | the focused panel's cursor · `PgUp PgDn Home End` jump |
| `e` | explain the number under the cursor: formula, values, sources · `⏎` asks Claude, `y` copies |
| `:` · `?` | the palette: every action and object, fuzzy, `tab` for the channels · the keys of this screen |
| `/` · `s` | filter the tiles · sort them: pressure, memory, CPU, name |
| `c` | a query session on the chosen node — SQL, read-only, floating |
| `\\` · `ctrl+\\` | float a session · its prefix (see above) |
| `x` then `y` | in the queue: cancel the Redash job under the cursor — any other key keeps it |
| `o` · `y` · `r` | open the row in the browser · copy its link · read Airflow and Jira again now |
| `J K` | scroll the SQL under a Redash job |
| `n` | a note on the tape |
| `[` `]` `t` | replay a minute back · forward · back to live |
| `p` · `+` | layout: all → fleet → ops · density: comfortable → compact → focus |
| `F1`…`F8` · `F10` | a panel zoomed · this Mac — from anywhere, a session's screen too |
| `0` | this Mac |
| a click · the wheel | a panel takes the focus, a second click opens it; inside a zoom a row puts the cursor there, a second click opens it · the wheel moves the cursor |
| `space` | pause (the numbers stop, the clock does not) |
| `q` | quit |

Nothing here can kill or change a ClickHouse query: every request is `readonly=1`, and also capped at
2 threads, 6 GB and 2 seconds when the server lets a session set those (a read-only user's
own profile carries its caps otherwise). A query session's queries are `readonly=1` too, with
30 seconds and 1000 rows when the server allows them; a write is refused by the server itself.
The only request that changes anything goes to Redash, and only after `x` and `y` in the queue:
the cancel of that one job.

## On Telegram

With a bot token, cobserve — the screen, the desktop app or `--plain` — is a Telegram bot as
well, for the chats `TELEGRAM_CHAT_ID` names (`src/telegram/`):

- **Ask anything**, in any language: Claude Code (`claude -p`, signed in as for the sessions, no
  tools) answers from a snapshot of what the screen knows — the nodes, the queries running, the
  Redash queue, Airflow, Jira, the tape — and remembers the conversation (`/forget`). The SQL of
  queries never goes to Claude.
- **Stop something**: asked to, it sends a card — a query's node, person, time, memory against
  its limit and its node, the first words of its SQL — and `KILL QUERY` (or a Redash cancel)
  runs only when its button is pressed, within five minutes. The card then says what happened,
  and when the query has left the node. `/kill <node> <query_id>` and `/cancel <job>` make a
  card without Claude. Another user's query needs the KILL QUERY grant on cobserve's login.
- **`/status`**: an overview and a tab a panel — Fleet, Redash, Airflow, Jira — in one message.
- **Unasked**, it tells what has been wrong for ten minutes — a node, a query, a Redash queue
  (the screen's insights), a DAG whose last run failed or a run stuck — once, with **Explain**,
  and again when it clears. What clears within ten minutes is never told; a gap of under 90 s
  does not restart the clock (`/alerts red | all | off`).

## The desktop app

`desktop/` is cobserve in a window of its own: a window (`tao`) with a web view (`wry`) holding a
terminal (xterm.js, vendored — nothing is fetched), and in it `cobserve` itself in a
pseudo-terminal: the same program, keys, mouse, colours and sessions included. Block and box
glyphs are drawn to the cell, so bars and rules have no seams.

It looks like cobserve: Tarum for the terminal's colours, the header and the chips, and
**Maple Mono** for the type — rounded and soft, with a quiet 600 bold, and it draws cobserve's
frames, bars and braille graphs itself, so the screen is one face; the name stays in Recursive's
casual, like a line drawn with a canting. Both ship inside the app (SIL Open Font License,
`desktop/assets/fonts`). `COBSERVE_ITERM_PROFILE=<name>` takes an iTerm2 profile's font and
colours instead. The terminal is drawn with WebGL, by the GPU — a fifth of the energy of the web
view's own text while sessions talk; `COBSERVE_RENDERER=dom` draws it as the system's text.

```sh
./desktop/bundle.sh             # dist/Cobserve.app
./desktop/bundle.sh --install   # and copy it to /Applications
```

An app opened from the Dock has none of your terminal's environment, so:

- cobserve runs through your **login shell** (`$SHELL -l -c`) for your `PATH` — not an
  interactive one, whose `.zshrc` may wait for a terminal that is not there;
- **`~/.config/cobserve/credentials.yaml`** is passed as `--credential` when it exists: put the
  servers there, and the `redash:`, `airflow:` and `jira:` sections;
- **`~/.config/cobserve/env`** sets anything else, one `KEY=value` a line, read as written — no
  shell, so a password with `&` or `$` in it is fine:

```sh
PRAYER_CITY=Jakarta
JIRA_URL=https://jira.example.net
JIRA_TOKEN=personal-access-token
```

The window has a header and a status bar of its own, drawn by the app, not in the terminal:

- **the panels as tabs** — Fleet to Sessions and Local, `⌘1` … `⌘9` or a click (it sends the
  panel's F-key, which zooms it from anywhere); Sessions shows how many there are, and rings when
  one calls while out of sight;
- **the clock and the next prayer** on the right, and the fleet's worst as the dot by the name;
- **a chip per thing watched** along the bottom — ClickHouse, Redash, Airflow, Jira, the hours
  logged and this Mac — coloured by severity, a click opens its panel.

cobserve tells the app what to show with a private escape sequence (OSC 7700, JSON, only when
`TERM_PROGRAM` is `cobserve-desktop` and only when it changed). The numbers are the screen's own (`src/desktop.rs`).

Files dropped on the window are typed in as their paths, quoted for the shell — for Claude Code or
OpenCode in a session. Programs in `~/.opencode/bin`, `~/.claude/local`, `~/.local/bin`,
`~/.bun/bin`, `~/.cargo/bin`, Homebrew and the like are found even when only `.zshrc` names them.

`⌘C` `⌘V` copy and paste, `⌘+` `⌘−` `⌘0` change the type's size (kept for next time), `⌥` and a
drag selects text while cobserve has the mouse, and the window's title is cobserve's. `q` closes
the window; if cobserve stops with an error, the window stays and says why. `chmod 600` both
files: they hold logins.

## Developing

```sh
cargo test                            # model, insights, tape, history, the screens of docs/redesign
cargo clippy --all-targets -- -D warnings
./dev/screenshots.sh                  # docs/screenshots/: text for every screen, PNGs for these
cobserve --plain                      # no screen: the panels' summaries every poll
cargo test -p cobserve-desktop        # the desktop app's own tests
cargo test live_airflow_and_jira -- --ignored --nocapture   # Airflow and Jira against your own
                                      # Airflow and Jira, read once and drawn (AIRFLOW_*, JIRA_* set)
```

| | |
|---|---|
| ![The queue zoomed](docs/screenshots/120x36-zoom-queue.png) | ![The tiles zoomed](docs/screenshots/120x36-zoom-nodes.png) |
| ![The palette](docs/screenshots/120x36-palette.png) | ![Explain this number](docs/screenshots/120x36-explain-node.png) |
| ![80×24](docs/screenshots/80x24-dash.png) | ![160×48](docs/screenshots/160x48-dash.png) |
