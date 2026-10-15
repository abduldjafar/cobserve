# 06 · Build order, acceptance, tests

Fake data first, as `DESIGN.md` §11: no network until a phase's screens match. Each phase ends
with `cargo test` green, `cargo clippy --all-targets -- -D warnings` clean, and the screens it
names rendered by `TestBackend` and compared to `docs/redesign/screens/`.

**How the comparison works.** `src/ui2/tests.rs` renders the fake fleet (`fake.rs`, frozen at the
same instant the simulation uses: 15:52:07 WIB, the burst 23–26 minutes ago, the same nine nodes)
and diffs the buffer's text against the screen file, ignoring the clock cell and anything the
screen file marks volatile (the ellipsis positions when names truncate differently are not
volatile — match them). The fake generator will need the simulation's numbers added as a fixed
scenario (`FAKE=1 FAKE_SCENARIO=redesign`), one-time work in phase 1. A screen that cannot match
exactly for a good reason gets that reason in the test, not a looser comparison.

## Phase 0 — contracts in the engine (S) — done

- `src/action.rs`, `src/provenance.rs` with an `explain_*` function for every §5 formula (beside
  the type, so `model.rs` stays untouched), `src/ui2/layout.rs`, `src/ui2/state.rs` (focus, zoom,
  preset, density, toasts), `src/caps.rs`.
- The capability probe in `main.rs`; synchronized output around `draw` if the backend lacks it.
- Tests: provenance round-trips on the fake snapshot; layout tiles the screen at the worked sizes
  and at 500 random sizes; the `Action` list has no duplicate keys within a scope.
- No screen changes.

## Phase 1 — shell skeleton (M) — done

`FAKE=1 UI=2 cargo run` shows: top bar with the day line, the grid with FLEET and QUEUE drawn
(big numbers, graphs on the 4-minute history stretched and labelled `-4m`), empty frames for the
other six panels with their titles and severities, the status bar from the `Action` table, focus
and `tab`, zoom frame with an empty body, `esc`, the palette (`tui-input` + `nucleo`), `?`,
toasts, density, presets.
Acceptance: `120x36-dash.txt` rows 0–8 and 35, `80x24-dash.txt` rows 0–5, `160x48-dash.txt` rows
0–10; `120x36-palette*.txt`; `120x36-dash-density-*.txt`; the resize fuzz.

As built: the palette is our own (`ui2/overlay/palette.rs`, the simulation's `fz` scoring) rather
than `tui-input` + `nucleo` — two crates for a one-line input and a subsequence match over a few
hundred rows. The screens are compared by their frames (every corner and side in place), their
chrome and their labels; the numbers inside come from the fake fleet, which is not the
simulation's, and are checked against the engine instead (`ui2/tests.rs` says why). The queue's
graph is on a fixed scale of 100 jobs waiting.

## Phase 2 — NODES and NODE (M) — done

Tiles with the grid cursor, the chosen node, the NODE panel with the one-scale bars, the sparkline
and the 60-minute (for now 4-minute) graph, the zoomed NODES and NODE, `e` with the provenance
popover, `s` sort, `/` filter on tiles.
Acceptance: `*-dash.txt` complete for the NODES and NODE panels, `*-zoom-nodes.txt`,
`*-zoom-node.txt`, `120x36-explain-node.txt`, `120x36-explain-user.txt`.

As built: the zoomed FLEET came with it (the node rows and both graphs). Where the screens differ,
`ui2/tests.rs` says why: NODE's bars use largest-remainder rounding so their cells sum to the
node's exactly (the simulation rounds each row on its own); a tile name too long for its tile is
cut (`clickhouse5 N…`) instead of written over the corner; the explain popover lists the inputs
of its formula only (the simulation adds the node's CPU under a memory share); the zoomed NODE's
last line says lag, parts and free memory and nothing that §5 does not trace ("= 0.2× a 24 GiB
query"). "usually 42 s (318× / 7 d)" needs the query-log fingerprints of feature A2.

## Phase 3 — QUEUE, AIRFLOW, JIRA, TAPE (M) — done

Zoomed QUEUE from today's view 2 drawing code (moved, not rewritten), the dialog; AIRFLOW panel
and zoom with the Gantt; JIRA panel and the board zoom (from `ui/board.rs`); TAPE panel with the
density strips and the folded repeats, the note.
Acceptance: `*-zoom-queue.txt`, `120x36-cancel-dialog.txt`, `*-zoom-airflow.txt`,
`*-zoom-jira.txt`, `*-zoom-tape.txt`, `120x36-note.txt`.

As built: the four zooms are today's views drawn inside the zoom's frame (`ui2/views.rs`), their
state still keyed by `app.view`, which the shell keeps on the focused panel; their keys are
listed as `Zoomed` actions that hand the key to the view's own handler, so the status bar, `?`
and the palette name them. `⏎` on a running job or a tape line follows the view to the panel that
shows it (the node zoomed, its person chosen). The view's own first lines (its title, the drawer's
"view 1" wording) are today's until phase 6 moves the code. The dashboard panels are new.

## Phase 4 — SESSIONS and the floating pane (S–M) — done

Panel 8, its zoom with the session's screen beside the list, the floating pane over any panel
with the prefix and the which-key box, keyboard enhancement flags with the `F1`–`F10` fallback and
its toast, `c` for a SQL session, the Claude hand-off from `e` and from a person row.
Acceptance: `*-zoom-sessions.txt`, `120x36-float-session.txt`, `120x36-float-session-prefix.txt`;
a `--ignored` test that drives a real `claude -p` is not needed — the PTY code is unchanged.

As built (`ui2/sessions.rs`): the zoom is today's sessions view (list beside the screen); `⏎`
opens the session full and `ctrl+\` comes back to the list. The floating pane hands its keys to
today's view 7 handling, so a running program gets every key; after `ctrl+\` the prefix takes
`1`–`6` (a panel — `7`–`9` are the sessions), `7`–`9` `↑↓`, `f`, `/`, `n`, `c o t q`, `r`, `x x`,
`esc`; the which-key box comes 250 ms later (the loop is woken for it). `F1`–`F6` focus a panel,
`F9` the next session, `F10` hides — said once in a toast where the kitty keyboard protocol is
missing. `e`'s `⏎` and `⏎`/`\` on a person in NODE type a prompt into a Claude session (started
if need be, the prompt typed once it is up, never sent). The mouse came with it: a click focuses,
a second opens, the wheel moves the cursor, clicks inside a zoomed view or the pane are theirs.

## Phase 5 — wizard, tour, replay stub, `--plain` (M) — done

`:setup` with the five steps writing `~/.config/cobserve/config.toml` and secrets to the Keychain
(`keyring`), with the announced `0600` fallback and env vars still winning; `:tour`; `[` `]` `t`
over the 4-minute history (the full hour comes with feature A1); `cobserve --plain` printing the
panel summaries as text every poll; `:local`.
Acceptance: `120x36-wizard.txt`, `120x36-tour.txt`, `120x36-dash-replay.txt`.

As built: `setup.rs` keeps the variables themselves (`NAME = "value"`, so the file is what one
would export) and the four secrets in the Keychain through `security-framework` — the crate
`keyring` itself wraps on macOS — or, where that fails, in `secrets.toml` at 0600, which the toast
names. `Config::load` asks the environment first, then the store; what `:setup` keeps is used from
the next start. The live checks run in `sources/setup_check.rs` (read-only: `SELECT version()`,
Redash's queue status, Airflow's `/health`, Jira's `myself`; the day is computed). `d` is `ctrl+d`,
since `d` is a letter of a field. The replay moves the clock, the FLEET numbers and graph cursor,
and the tiles; the other panels stay live until feature A1 stores their minutes. `:local` (and `0`
on the grid) zooms today's view 0; its line joins the status bar from 140 columns. `:theme` and
`:prayer` came with the palette's settings.

## Phase 6 — real data, parity, the swap (S–M) — done

Run both shells against `dev/local-rig.sh` and then the fleet; the numbers must be identical (same
engine). Move `dev/screenshots.sh` to the new screens, delete `src/ui/`, the view enum and the
`UI=2` flag; write the new `DESIGN.md` §1, §3, §7 and §11 from this folder, keeping §4, §5, §6,
§9 (now "environment-first"), §12 and §13. Update `README.md` and `GUIDE.md` §6–§8.

As built: the views the zooms show moved to `src/ui2/views/` (moved, not rewritten — the
"view 1" wording in them went with the move), the rest of `src/ui/` (the masthead, the band and
its cards, the day rail, the tree, the map, the pivot, the insights panel, the help overlay, the
footer) was deleted with its tests, and the old shell's tests of the views that stayed were
ported to `src/ui2/views/tests.rs`. `UI=2` and `--shell` are gone. `app::View` stays as the
selector of the views' state (`CLAUDE.md` says why); the key handling only the old shell used —
the digit views, the tree, the map, the insights, help — went from `app.rs`. The desktop app's
tabs are the panels (`⌘1`–`⌘9`, the F-keys). Parity needs no second run: both shells drew the
same `App`, and the panels' numbers are held to the engine by the tests.

## The zooms, redrawn (October 2026) — done

REDASH QUEUE, AIRFLOW, JIRA and SESSIONS zoomed were still today's views inside the zoom's frame;
they are redrawn from zero as `09-zooms.md` designs them: cards for the facts, one list for the
rows, the row under the cursor in a card, and what runs now moving every second. The only engine
touches are the Airflow tasks read for the Gantt without a page (`App::airflow_tasks`),
`ClaudePane::last_output`, and the fake's tasks spread over their run.

Acceptance: `*-zoom-queue.txt`, `120x36-cancel-dialog.txt`, `*-zoom-airflow.txt`,
`*-zoom-jira.txt`, `*-zoom-sessions.txt`, `*-zoom-sessions-list.txt`, `120x36-zoom-sessions-typing.txt`; the zooms' resize fuzz.

## Phase 7 — feature wave 1 · Phase 8 — wave 2

`07-features.md`.

## Definition of done for the shell (end of phase 6)

- Every screen in `docs/redesign/screens/` has a passing test or a written reason.
- `cargo test`, clippy clean, the resize fuzz at 500 sizes for the grid, every zoom and every
  overlay.
- The §5 unit tests untouched and green; the provenance of every metric on the fake fleet
  reproduces its value.
- Three recordings or screenshots against the real fleet at 120×36, 80×24 and 160×48, in
  `docs/screenshots/`.
- `GUIDE.md` §6–§8 rewritten for the new screen; `CLAUDE.md` loses its redesign section.
