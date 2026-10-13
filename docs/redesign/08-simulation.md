# 08 · The simulation

`docs/research/simulasi-konsep-ui.html` is one HTML page with no dependency but Google Fonts. It
draws the concept into a `W × H` cell buffer (`class Buf`, a tiny ratatui: `put`, `putr`, `seg`,
`bg`, `clear`, `hit`) and serialises the buffer to a `<pre>` of spans, one span per run of equal
style. Published at https://claude.ai/artifact/WkRxfkk2VHU2narocc4pE9 (private until shared from
the page's Share menu).

## Reading it as a spec

The file is in sections, each a comment line `// ───────── … ─────────`:

| Section | What to port |
|---|---|
| `data` | the fake fleet (`FLEET`, `HIST`, `SPARK`, `TAPE`, `QUEUE`, `AF`, `JIRA`, `SESS`): the scenario `fake.rs` must reproduce for the screen tests |
| `Buf` and helpers | `tr rp lp f1 pctS sevNode sevUser bullet shares sparkStr heatCh rule box` — the same helpers exist in `fmt.rs`, `severity.rs`, `ui/widgets.rs` |
| `mission control: state, layout, graphs, big digits, panels` | `S` (the state), `layout()`, `graph()`, `BIG`/`big()`, `panel()`, `tile()`, `panelFleet … panelSessions`, `dash()` — `01-screen.md` and `02-panels.md` are written from these |
| `zoomed panels` | `zoomFrame()`, `zoomFleet … zoomSessions` |
| `overlays` | `actionList()` (the `Action` table's content), `channelItems()`, `fz()` (the fuzzy matcher), the palette, `provenance()` (what the popover shows for each cursor type), the floating pane, the wizard (`WIZ` holds the five steps' copy), the dialog, the note, the tour |
| `keys, mouse, render loop` | `onKey()` is the key router of `05-keys.md`; `render()` the pipeline of `04-architecture.md` |

Two things are deliberately simpler than the real thing: graphs are blocks, not braille (browser
fonts), and *replay* scales the live snapshot by the node's minute history instead of reading a
store.

## Regenerating the screens

```bash
node dev/sim-screens.js      # → docs/redesign/screens/<W>x<H>-<screen>.txt (47 files)
```

The script loads the page's script with a stub `document`, sets `W`, `H` and the state for each
scenario, calls `render()` and dumps the buffer's characters. Add a scenario by adding a `shot()`
line. The screens are the acceptance targets of `06-build-order.md`; regenerate them whenever the
simulation changes and commit both together.

## What the simulation fakes

Everything: the fleet (nine nodes, the numbers of `docs/screenshots/120x36-nodes.txt`), the queue,
the DAGs, the tickets, the sessions and their screens, Claude's answers (`floatAsk` appends canned
lines), the wizard's live checks (a timer), the notifications. Nothing it does reaches a network.
