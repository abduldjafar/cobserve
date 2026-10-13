# cobserve redesign — the knowledge base

**Status: concept approved by the owner (2026-10-13); phases 0 (the contracts), 1 (the shell
skeleton, `UI=2`) and 2 (NODES and NODE) built — next is phase 3 (`06-build-order.md`).** This folder is
everything the next agent needs to build the new shell: the decisions, the screen as text, the
panels, the glyphs, the architecture, the keys, the build order and its tests. Read it in order;
it is written for an implementer with no prior context, like `DESIGN.md` was for v1.

| File | What it holds |
|---|---|
| [01-screen.md](01-screen.md) | The screen: chrome, the panel grid, the layout algorithm per size and preset, density, zoom |
| [02-panels.md](02-panels.md) | Every panel, on the dashboard and zoomed: content, numbers and the §5 formula each comes from, keys |
| [03-glyphs.md](03-glyphs.md) | How things are drawn: graphs, big digits, tiles, bars, strips; colour rules; terminal support |
| [04-architecture.md](04-architecture.md) | What stays, what is new: the `Action`, `Provenance`, `Layout` and `Panel` contracts, modules, the `UI=2` flag |
| [05-keys.md](05-keys.md) | The complete key map, the mouse, the palette's actions and channels, the overlays |
| [06-build-order.md](06-build-order.md) | Phases, each with its acceptance screens and tests; the definition of done |
| [07-features.md](07-features.md) | The feature waves after the shell (history, fingerprints, health row, …), from the report |
| [08-simulation.md](08-simulation.md) | The browser simulation: what it is, how to regenerate the screens, what it fakes |
| [screens/](screens/) | 47 screens as text, generated from the simulation: the acceptance targets |

## Where this came from

1. The research report, in Bahasa Indonesia, with five research notes behind it:
   [`docs/research/reports/Konsep UI baru cobserve.md`](../research/reports/Konsep%20UI%20baru%20cobserve.md).
   Its visualisation catalogue, feature table, usability findings, crate stack and terminal matrix
   all still apply; its ASCII mock-ups describe an earlier *evolution* concept that the owner
   rejected as too close to today's screens.
2. The browser simulation, the executable reference of the approved concept:
   https://claude.ai/artifact/WkRxfkk2VHU2narocc4pE9 — source
   [`docs/research/simulasi-konsep-ui.html`](../research/simulasi-konsep-ui.html). The rejected
   evolution concept is kept as [`simulasi-evolusi.html`](../research/simulasi-evolusi.html).
3. The screens in `screens/` are rendered from the simulation by `dev/sim-screens.js`.

## Decisions (do not reopen without the owner)

- **Mission control, not evolution.** One screen of framed panels in the manner of btop: big
  numbers and graphs for the fleet and the queue, a tile per node, who-holds-the-node, Airflow,
  Jira, the tape, the sessions. Every panel opens full-screen with `⏎`. No view tabs.
- **The engine stays; the shell is rewritten.** `model.rs` (§5), `sources/*`, `history.rs`,
  `insight.rs`, `tape.rs`, `pty.rs`, `claude.rs`, `console.rs`, `prayer.rs`, `theme.rs`,
  `fake.rs`, `config.rs`, `alerts.rs` and their tests are kept as they are. The new shell lives in
  `src/ui2/` behind `UI=2` until it reaches parity on the local rig, then replaces `src/ui/`.
- **Two engine contracts come first**: an `Action` table (one source for the status bar, `?`
  and the palette) and a `Provenance` on every §5 metric (powers *explain this number*).
- **The house rules are unchanged**: colour means severity and nothing else; every percentage
  prints its denominator; every number traces to a formula in `DESIGN.md` §5 (or to the arithmetic
  named in `02-panels.md`); the UI never blocks on the network; never panics on resize; ClickHouse
  stays read-only; the one write is the Redash cancel, `x` then `y`.
- **Graphs are braille by default** in the terminal (ratatui `Marker::Braille`), blocks where
  braille is unavailable or asked for (`GRAPH=block`), ascii below that; the simulation draws
  blocks because browser fonts do not promise braille widths.
- **The Tarum palette stays**, as semantic tokens: ground, text, dim, accent, panel, selection,
  and the three severities.
- **Sessions are panel 8**, beside the tape, and `\` (`ctrl+\`) floats the chosen session over
  any panel; inside the pane every key belongs to the program, as today.
- **LOCAL (this Mac) has no panel.** It is a line in the status bar from 140 columns and the
  zoom `:local` (or `0`); the AI-usage panel of today moves there unchanged.
- **The day line (prayer times) stays** in the top bar; a chip (`☾ Maghrib in 1h57m`) when the
  bar is narrow or the density is compact; gone in focus density.

## How to start

```bash
FAKE=1 cargo run                 # today's shell, for comparison
FAKE=1 UI=2 cargo run            # the new shell, once phase 1 exists
node dev/sim-screens.js          # regenerate docs/redesign/screens/ from the simulation
cargo test ui2::                 # the new shell's tests, once phase 1 exists
```

Work on the branch `ui-redesign`, from `main`. Commit author and language rules are in
`CLAUDE.md`. Fake data first, as `DESIGN.md` §11 says: no network until a phase's screens match.
