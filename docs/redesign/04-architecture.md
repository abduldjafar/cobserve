# 04 · Architecture: what stays, what is new

## What stays, untouched

| Module | Role | Why it is untouched |
|---|---|---|
| `model.rs` | types and the §5 math | the contract; its tests are the proof the numbers did not change |
| `sources/clickhouse.rs`, `redash.rs`, `airflow.rs`, `jira.rs`, `local.rs`, `usage.rs`, `limits.rs` | the pollers | UI-agnostic; they send events on the channel |
| `history.rs`, `insight.rs`, `tape.rs`, `severity.rs`, `attrib.rs`, `fmt.rs` | derived numbers | they already compute what the panels print |
| `pty.rs`, `claude.rs`, `console.rs`, `complete.rs`, `assist.rs`, `saved.rs`, `conversations.rs`, `folders.rs` | the sessions | panel 8 and the floating pane draw their screens; nothing about running them changes |
| `prayer.rs`, `clock.rs`, `alerts.rs`, `theme.rs`, `config.rs`, `fake.rs`, `desktop.rs` | the day, notifications, the look, configuration, the fake fleet, the desktop shell protocol | |
| `main.rs` | runtime, terminal, event loop | gains the `UI=2` switch and the capability probe only |
| `app.rs` | state + `update()` | gains the fields of `ShellState` below; its view enum and key handling move out once the old shell is removed |

What goes, at the end of phase 6: `src/ui/*` (the old shell) and the view-specific key handling
in `app.rs`.

## The new modules

```
src/ui2/
  mod.rs          draw(&App, &mut Frame): chrome + grid or zoom + overlays + toasts; the only entry point
  layout.rs       Layout::compute(W, H, preset, density, nodes) -> Grid { rects per PanelId }   (01-screen.md)
  chrome.rs       top bar (day line, clock, path), status bar (hints from Action, poll line)
  panel.rs        trait Panel, PanelId, frame(): the rounded box with severity colour and chip title
  panels/
    fleet.rs queue.rs nodes.rs node.rs airflow.rs jira.rs tape.rs sessions.rs local.rs
  glyph/
    graph.rs      braille / block / ascii tiers, fixed scale, severity per column, the • series
    big.rs        the three-row digit table
    bar.rs        share bar, bullet bar, stacked bar with largest-remainder rounding
    strip.rs      tape density, the Airflow day, Gantt spans, the hours calendar
    tile.rs       the node tile
  overlay/
    palette.rs explain.rs float.rs wizard.rs dialog.rs toast.rs tour.rs note.rs
  tests.rs        TestBackend renders against docs/redesign/screens/*.txt (see 06)
src/action.rs     the Action table (below)
src/provenance.rs Provenance on every §5 metric (below)
```

## Contract 1 — `Action`

One table feeds the status bar, `?` and the palette, so they cannot disagree.

```rust
pub struct Action {
    pub id: &'static str,            // "explain", "zoom", "cancel-job", "setup-redash"
    pub label: &'static str,         // "explain this number"
    pub detail: &'static str,        // "the value under the cursor · its §5 formula"
    pub keys: &'static [&'static str], // ["e"]; empty for palette-only actions
    pub scope: Scope,                // Everywhere | Panel(PanelId) | Zoomed(PanelId) | Overlay(..)
    pub rank: u8,                    // status-bar order; the five lowest ranks of the scope show
    pub shown: &'static str,         // how the status bar writes its keys: "1-8", "←→↑↓"
    pub enabled: fn(&App) -> bool,   // "cancel job" only on a queue row; "back to live" only in replay
    pub run: fn(&mut App, &str),     // the key pressed: one action answers several ("1"…"8")
}
```

`ACTIONS: &[Action]` is a static list; `actions_for(app) -> Vec<&Action>` filters by scope and
`enabled`. The status bar prints the first five by rank with their first key; `?` lists the whole
scope, filterable; the palette lists every enabled action plus the object channels. Tests: every
key the key handler accepts has an `Action` (no key does something unlisted), and every listed
key is accepted — the rule of `DESIGN.md` §3 ("a listed key that does nothing is a bug") made
mechanical.

## Contract 2 — `Provenance`

```rust
pub struct Provenance {
    pub formula: &'static str,                 // "node.mem_pct = node.mem_used ÷ node.mem_total × 100"
    pub reference: &'static str,               // "DESIGN.md §5.3"
    pub inputs: Vec<Input>,                    // each with name, value, unit, source
    pub at: SystemTime,                        // the poll the inputs came from
    pub note: Option<String>,                  // "delta form (2nd poll on)", "clamped ≥ 0", "fallback 9 GiB"
}
pub struct Input { pub name: &'static str, pub value: f64, pub unit: Unit, pub source: &'static str }
pub struct Metric<T> { pub value: T, pub prov: Provenance }
```

`model.rs` keeps returning plain numbers for the hot path and is not touched; a parallel set of
`explain_*(…) -> Provenance` functions in `provenance.rs`, one per §5 formula, rebuilds the
provenance from the same inputs on demand (the popover is opened by a person, not every poll),
and carries the formula as a function as well as a string. Tested: for every metric on the fake
snapshot, evaluating the provenance's inputs with its formula reproduces the value within 0.1.

## Contract 3 — `Layout`

`layout.rs` is a pure function of `(W, H, Preset, Density, node_count)` returning the rects of
`01-screen.md`; tested against the worked sizes and against the invariant that the rects tile the
screen without overlap and leave exactly the top bar and the status bar.

## Contract 4 — `Panel`

```rust
pub trait Panel {
    fn id(&self) -> PanelId;
    fn title(&self, app: &App) -> Title;           // name, severity, right-hand summary
    fn draw(&self, app: &App, buf: &mut Buffer, inner: Rect, focused: bool);
    fn draw_zoomed(&self, app: &App, buf: &mut Buffer, inner: Rect);
    fn cursor(&self, app: &App) -> Option<CursorId>;      // what ↑↓ move; identity, not index
    fn provenance(&self, app: &App) -> Option<Provenance>; // for `e`
    fn hits(&self) -> &[Hit];                              // click targets registered while drawing
}
```

Cursors are keyed by identity (node name, person, job id, DAG id, ticket key, session number) so
a refresh under the cursor keeps it, as `tree.rs` does today.

## State

Added to `App` (or a `ShellState` inside it): `focus: PanelId`, `zoom: Option<PanelId>`,
`preset`, `density`, `chosen_node: Option<String>`, `chosen_person`, per-panel cursors, `replay:
Option<Minute>`, `palette: Option<PaletteState>`, `explain: Option<Provenance>`, `float:
FloatState { open, session, full, prefix }`, `wizard`, `dialog`, `note`, `tour`, `toasts`,
`filter`. All pure state; the sources never look at it.

## Render pipeline

```
draw():  layout → chrome → for each panel in the grid: frame + panel.draw(inner)
         or: chrome(path) + zoom frame + panel.draw_zoomed(inner)
         then overlays in order: float → explain → palette → dialog → wizard → note → tour → toasts
```

Overlays paint their own background (panel token) and shadow; nothing underneath is cleared.
Drawing is wrapped in `BeginSynchronizedUpdate … EndSynchronizedUpdate` (crossterm) unless the
ratatui backend is found to do it already (phase 0 checks).

## Events

Unchanged: sources → `Event::*` on the mpsc channel → `app.update()` → redraw. Keys route in this
order: tour → wizard → palette → dialog → explain → floating pane → note → filter → the zoom or the
focused panel → global. The floating pane forwards every key to its PTY except `ctrl+\`, which
starts the prefix (`05-keys.md`).

## Capability probe (phase 0, `main.rs`)

Before the event loop: colour depth (`termprofile` or the existing detection), kitty keyboard
enhancement (`supports_keyboard_enhancement`, called before `poll`), the block-width self-check of
`03-glyphs.md`, `$TERM_PROGRAM` for OSC 8/9 heuristics. Results live in `App::caps` and pick the
graph tier, the painted backgrounds, the link style.

## The flag

`UI=2` (env) or `--shell next` selects `ui2::draw` and the new key router; everything else is
shared. Both shells run against the same fake fleet and the same local rig, which is how parity
is checked in phase 6. When the old shell is deleted the flag goes with it.
