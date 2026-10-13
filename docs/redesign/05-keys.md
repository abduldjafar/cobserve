# 05 · Keys, mouse, palette

Every key here is an `Action` (`04-architecture.md`). The status bar shows five per scope; `?`
shows the scope's whole list; `:` everything.

## Everywhere (grid or zoom)

| Key | Action |
|---|---|
| `1`…`8` | focus a panel (grid) · switch the zoom (zoomed) |
| `0` | the grid (from a zoom) · `:local` zoom from the grid |
| `tab` / `shift+tab` | next / previous panel |
| `⏎` | zoom the focused panel · inside a zoom: the panel's own *open* (`02-panels.md`) |
| `esc` `⌫` | close the top-most thing: overlay, filter, zoom; never quits |
| `f` | zoom ↔ grid (same as `⏎` / `esc` on a panel) |
| `←→↑↓` `hjkl` | the focused panel's cursor; `PgUp PgDn Home End` jump |
| `e` | explain this number (the focused panel's cursor) |
| `E` | EXPLAIN ESTIMATE of the chosen query (read-only; toast with rows · parts · marks) |
| `:` | the palette · `?` the keys of this scope |
| `/` | filter the focused list (substring over its rows; `esc` clears) |
| `\` or `ctrl+\` | float the chosen session; again: prefix mode (below) |
| `c` | a SQL session on the chosen node (floats it) |
| `p` | layout preset all → fleet → ops |
| `+` | density comfortable → compact → focus |
| `s` | sort the nodes: pressure → memory → CPU → name |
| `[` `]` `t` | replay one minute back / forward · back to live |
| `space` | pause polling (the clock goes on) |
| `r` | read every source again now |
| `n` | a note on the tape · `y` copy the row's link (OSC 8 where it works) · `z` snooze notifications 1 h |
| `x` | on a queue row: the cancel dialog, then `y` cancels, anything else keeps |
| `-` | back to where you came from |
| `W` | wall mode: the big numbers alone (feature J2) |
| `q` `ctrl+c` | quit |

Not here any more: the digit views, `u` pivot (the NODE panel is the pivot's answer; a `people`
channel in the palette lists everyone across the fleet), `tab` between tree and insights
(insights are the summaries on the frames and the panel titles).

## In the floating session or a zoomed session

Every key goes to the program, as today, except `ctrl+\` (and `F1`–`F10` as the fallback for
terminals that keep `ctrl+\`): then `1`–`8` a panel, `7`–`9` and `↑↓` another session, `f` full
↔ floating, `/` find a session, `n` new, `c o t q` new Claude / OpenCode / terminal / query, `r`
rename, `x x` close, `esc` back to the program, `ctrl+\` again hides the pane. The which-key box
appears 250 ms after the prefix; a fast hand never sees it. `⇧⏎` / `ctrl+j` for a new line as
today; the pane's bottom line names the key that works in this terminal.

## The palette

`:` opens it empty on the **actions** channel: every enabled action of the current scope first,
then the global ones, each with its key at the right. Typing filters by fuzzy subsequence
(`nucleo`), matched letters bold; `↑↓` choose, `⏎` runs, `esc` closes. `tab` cycles the
channels: `actions · nodes · people · queries · jobs · dags · tickets · sessions · tape`; an object
chosen there opens its panel with the cursor on it. `?` is the same box opened on the actions
channel with the scope's keys only. Settings live here, not on keys: `:theme light`, `:clock
utc`, `:prayer off`, `:setup redash`, `:layout ops`, `:tour`, `:local`.

Screens: `120x36-palette.txt`, `-palette-typed.txt`, `-palette-help.txt`.

## Mouse

A click on a panel's frame or body focuses it; a second click where the cursor already is does
what `⏎` does. A click on a tile, a row, a lane chooses it; on the top bar's clock toggles UTC;
on `❖ cobserve` goes to the grid; on a status-bar hint runs it; on a palette row runs it; on the
dialog's buttons. The wheel moves the focused cursor; over the floating pane it is the program's.
`MOUSE=0` as today.

## Overlays' own keys

- Explain: `⏎` ask Claude about it (floats the session with the prompt), `y` copy, anything else
  closes.
- Wizard: `⏎` next step (after the live check), `tab` next field, `esc` skip the step, `d` a
  generated fleet instead; typing edits the field, `⌫` deletes.
- Dialog: `y` cancels the job, `esc`/`n` keeps it.
- Note: `⏎` keeps it, `esc` drops it.
- Tour: any key advances, `esc` skips.
