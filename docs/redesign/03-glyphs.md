# 03 · Glyphs: how things are drawn

Everything below is reproduced exactly by the simulation (`docs/research/simulasi-konsep-ui.html`),
so the simulation's functions are named where they are the reference.

## Colour

Six foregrounds and five backgrounds, all Tarum tokens (`theme.rs`): text (mori cream), dim,
accent (tarum indigo), red (mengkudu), amber (kunyit), green (leaf), person (lilac, as today for
people); backgrounds ground, panel, selection, shadow, chip (accent ground with ground text).
Rules:

- **Red and amber mean severity and nothing else**, by the thresholds of `DESIGN.md` §7. Green
  means *good news* (`✔`, `● live`, `● working`). Accent marks structure and the ordinary fill of
  a bar or graph; it never ranks anything.
- A graph column, a bar, a tile frame, a panel frame take the severity of the value they show.
- Identity is never a hue: people are told apart by rows (and the `person` colour is one colour
  for all of them), bar segments by texture `█ ▓ ▒ ░ ·`.
- 16 colours: the three severities on the ANSI slots, painted backgrounds off, frames carry the
  structure. Mono / `NO_COLOR`: glyphs alone — `✖ ▲ ● ✔ ! ↯`, textures, big digits.

## Frames

Rounded single-line boxes `╭─╮ │ ╰─╯` for panels, tiles, overlays. A panel's title sits in the top
border at column 2 as ` N NAME ` (bold; chip when focused), the summary right-aligned at the
top border's end. Overlays add a shadow: the cells one right and one below the box take the
shadow background (`box()` in the simulation). Nothing else is boxed.

## Graphs (`graph()` in the simulation)

One sample a column, newest at the right, scaled to a **fixed** maximum (100 % of the
denominator; never auto-scaled — "a graph that rescales lies"). Height `h` rows; a column of
value `v` fills `round(v / 100 × h × 8)` eighths from the bottom:

| Tier | Glyphs | When |
|---|---|---|
| braille (default) | ratatui `Canvas` with `Marker::Braille`, two samples a column as btop does | the terminal is not `TERM=dumb` and `GRAPH` is unset or `braille` |
| block | `▁▂▃▄▅▆▇█` for the partial top cell, `█` below it (the simulation) | `GRAPH=block`, or the start-up width check finds braille doubled |
| ascii | `.:-=+*#` | `GRAPH=ascii`, `TERM=dumb` |

Each column is coloured by its own value's severity (red ≥ 90, amber ≥ 75 on node and fleet
memory/CPU; the queue's ≥ 60 and ≥ 30 waiting). A second series on the same axes is a dot `•`
per column (text colour, bold), as the CPU over the memory area. Labels: the window at the left of
the baseline (`-60m`), the legend and `now` at the right; zoomed graphs add `100% · 50% · 0% ┤`
ticks. The replay cursor is a selection-background column.

Sparklines in tiles and rows: `▁`–`█` on 8 levels, each cell the **worst** sample of its slice,
a gap left as a space (`history.rs` already does this).

## Big digits (`BIG` in the simulation)

Three rows of `▀ ▄ █`, three cells a digit, one cell between digits, `%` three cells:

```
█▀█ ▄█  ▀▀█ ▀▀█ █ █ █▀▀ █▀▀ ▀▀█ █▀█ █▀█ ▀ █
█ █  █  █▀▀ ▀▀█ ▀▀█ ▀▀█ █▀█   █ █▀█ ▀▀█  ▄▀
█▄█ ▄█▄ █▄▄ ▄▄█   █ ▄▄█ █▄█   █ █▄█ ▄▄█ █ ▄
 0   1   2   3   4   5   6   7   8   9   %
```

Used only for the two headline numbers (fleet memory share, jobs waiting), coloured by their
severity, with the denominator in normal text under them. From 4 inner rows and 60 inner
columns; plain text below that. `tui-big-text` can replace the table in Rust if its `Quadrant`
size is preferred; the glyphs above are the contract either way.

## Bars

- **Share bar** (tiles, who-holds-it): `█` filled, `░` empty, width fixed per column so the rows
  of one node compare; the percentage printed after it. The rows of panel 4 use **one scale**
  (the node), so their filled cells add up to the node's.
- **Bullet bar** (today's `━━╸━━┊━┊`): kept for the band of the desktop app's chips and anywhere a
  threshold tick helps; the simulation's `bullet()` draws `┊` at 70 % and 90 %.
- **Stacked share** (an option for panel 4, from the report): `█ ▓ ▒ ░ ·` segments with
  largest-remainder rounding so they sum exactly to the width.

## Strips and timelines

- **Tape density**: two rows, `✖` red and `▲` amber, one column a minute, `▁`–`█` by count.
- **Airflow day**: one glyph an hour (`▪` ran, `✖` failed, `▸` running, `◌` queued, `━` a run
  that went on), half-hours from 160 columns, quarter-hours in the zoomed Gantt.
- **Gantt**: `━` spans positioned by start/end, `✔ ✖ ▸ ◌ ↻` at the end of a span.
- **Hours calendar** (Jira zoomed, from the report): five shades ` ░▒▓█` against the daily norm.

## Marks

`✖` critical · `▲` warning · `●` fine / live · `○` paused / idle · `✔` done · `!` runaway (replaces
today's `✕`, too close to `✖`) · `↯` unreachable · `◷` replay / snoozed · `☾` the next prayer ·
`▸ ◌ ↻` running / queued / retrying · `✻ ▣ ▦ ❯` Claude / OpenCode / SQL / terminal · `⊘` a
cancelled job · `✎` a note · `❖` the kawung mark.

## Widths and fonts

Everything is one cell wide: box drawing, block elements, braille, the marks above. Avoid any
glyph that terminals draw two cells wide (`⚠`, emoji). Block elements `▁`–`█` are East Asian
*Ambiguous*: on a terminal set to wide-ambiguous they double. Phase 0 adds a start-up self-check
(print `▂`, ask the cursor position with `CSI 6n`, fall back to braille or ascii if it moved two
cells). Octants (Unicode 16) are not used.

## Terminal support (from the report; `?` unverified)

| | Ghostty | kitty | WezTerm | iTerm2 | Apple Terminal | VS Code | tmux | wry + xterm.js |
|---|---|---|---|---|---|---|---|---|
| truecolor | ✓ | ✓ | ✓ | ✓ | ? (256) | ✓ | pass-through | ✓ |
| kitty keyboard (`ctrl+\`, `⇧⏎`) | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ | ? | ✓ |
| OSC 8 links | ✓ | ✓ | ✓ | ✓ | ? | ? | ? | ? check `desktop/` |
| OSC 52 copy | ? | ✓ | ✓ | ✓ | ? | ? | ? | ? |
| sync output (DEC 2026) | ✓ | ✓ | ✓ | ✓ | ? | ? | ignored | ? |
| braille in the default font | ✓ | ✓ | ✓ | ✓ | ✓ | font | font | Rec Mono: check |
