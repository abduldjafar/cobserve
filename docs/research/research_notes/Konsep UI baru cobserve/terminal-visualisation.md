# Data visualisation in terminal UIs — a catalogue for cobserve (Rust + ratatui 0.30)

Scope: which chart forms work in a text grid, how the best tools draw them, what ratatui 0.30
and its ecosystem give for free, and what degrades where. Each key question below ends with
ASCII mock-ups sized for cobserve's data (per-node memory/CPU over time, per-user share of a
node, queue depth over time, a DAG's day, hours logged per day, an event log, a fleet of 9–40
nodes). Mock-ups are my designs (Inferences), the facts behind them are in Cited Findings.

Research date: 2026-10-13. Current screens read from `docs/screenshots/*.txt`: bars `━╸╺`,
sparklines `▂▃▄`, Airflow timeline `▪·✖▸◌━`, worker dots `●●○`, fold markers `▾▸`.

---

## KQ0. The cell primitives: what resolution each gives and where it renders

### Takeaway
Five families of "pseudo-pixel" exist, in rising resolution and falling font support:
block elements (1×1 or 1×8 vertical levels, universal), half-blocks (1×2), quadrants (2×2),
Braille (2×4 dots, near-universal but visibly gappy), sextants (2×3, Unicode 13) and octants
(2×4, Unicode 16 — same resolution as Braille but solid; only in a few fonts, though Ghostty
draws them itself). ratatui 0.30 exposes all of these as `Marker` variants, so the choice can be
a runtime setting, exactly as btop's `graph_symbol` is.

### Cited Findings
- ratatui `Marker` variants: `Dot` (•), `Block` (█), `Bar` (▄), `Braille` (2×4, "Support for this marker is limited to terminals and fonts that support Unicode Braille Patterns"), `HalfBlock` (█ ▄ ▀, double vertical resolution), `Quadrant` (2×2, "densely packed pixels without visible bands"), `Sextant` (2×3, Symbols for Legacy Computing), `Octant` (2×4, "same resolution as Braille but with better visual density"; the block "is a relatively recent addition to unicode that is less broadly supported than Braille dots"), `Custom(char)` — [ratatui Marker docs](https://docs.rs/ratatui/latest/ratatui/symbols/enum.Marker.html)
- `Marker::Quadrant/Sextant/Octant` landed in ratatui 0.30.0 (PR #2235: "The octant marker is an alternative to the Braille marker with the same resolution"); 0.30.0 also made Charts and Canvas "render Braille over Blocks" so text and block symbols can be stacked under a Braille layer (PR #2165); `BarChart::grouped()` arrived in 0.30.0 (PR #1513); 0.30.1 added `FilledLine` for Canvas and `GraphType::Area` + `Dataset::fill_to_y()` for filled area charts (PR #2426) and a `Fill` widget that "paints every cell within its area with a single repeated symbol" (PR #2520) — [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)
- The docs.rs widget index is for ratatui 0.30.2 (released 2026-09-10): `BarChart` ("multiple datasets as bars with optional grouping", horizontal direction, value labels), `Chart` (lines or scatter), `Gauge` (block characters, `use_unicode` fractional blocks), `LineGauge` (one line), `Sparkline`, `Canvas`, `Calendar` (feature `widget-calendar`) — [ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)
- Microsoft's reasoning for adding octants to Cascadia Code (2404.23): "Braille is not the best solution because the symbols are using discrete dots, which makes the on-and-off pseudo-pixels unbalanced"; octants "will provide the highest resolution that can be achieved currently without using graphical extensions such as Sixels or ReGIS". The release added quadrants, sextants (U+1FB00–U+1FB3B), all 256 octants, eighths, checkerboards, diagonals, segmented digits; Windows Terminal ships the font — [Cascadia Code 2404.23](https://devblogs.microsoft.com/commandline/cascadia-code-2404-23/)
- Ghostty renders octants (code points 0x1CD00–0x1CDE5, i.e. the Unicode 16 Symbols for Legacy Computing *Supplement*) and separated sextants in its own built-in sprite font, with octant name tables parsed at compile time — so no installed font is needed — [Ghostty sprite Face.zig (mirror)](https://git.uoc.run.place/Applied-Software/ghostty/src/commit/2a29f71b2cf6f4db0d18da9fe2fb75564f3709ea/src/font/sprite/Face.zig)
- btop's `graph_symbol` option: "braille" offers the highest resolution but might not be included in all fonts. "block" has half the resolution of braille but uses more common characters. "tty" uses only 3 different symbols but will work with most fonts and should work in a real TTY. btop lists the font requirement explicitly: Braille Patterns U+2800–U+28FF (not needed in tty/block mode), Geometric Shapes U+25A0–U+25FF, Box Drawing + Block Elements U+2500–U+259F — [btop README](https://github.com/aristocratos/btop)
- Block elements such as U+2581 LOWER ONE EIGHTH BLOCK have East Asian Width = **Ambiguous** — [codepoints.net U+2581](https://codepoints.net/U+2581?lang=pl); U+2588 FULL BLOCK likewise — [codepoints.net U+2588](https://codepoints.net/U+2588)
- Unicode's Braille Patterns block is U+2800–U+28FF — [Unicode chart U2800](https://www.unicode.org/charts/PDF/U2800.pdf)

### Inferences
- A **symbol-set setting with three rungs** (btop's model) is the right abstraction for cobserve: `octant` (Ghostty, Windows Terminal/Cascadia, kitty with a capable font) → `braille` (everything with a Unicode font: iTerm2, Apple Terminal, VS Code, tmux) → `block` (▁▂▃▄▅▆▇█; universal) → `ascii` (`_.-=#` or btop's 3-symbol tty). Default to `braille` and let the owner opt into octants; octants in a font that lacks them render as tofu boxes, which is worse than any lower rung. Detecting support is not possible from inside the app (no terminal query exists for glyph coverage), so it must be a config/`?`-menu toggle.
- The East Asian Ambiguous width of ▁–█ is a real pitfall for cobserve's sparklines: a terminal set to treat ambiguous characters as wide (a common option in CJK locales, e.g. iTerm2 "Treat ambiguous-width characters as double width", Windows Terminal/WezTerm `treat_east_asian_ambiguous_width_as_wide`) will make every ▂▃▄ take two cells and shear the whole layout. Braille (U+28xx) and box drawing used in a plain way are Narrow/Neutral, so a Braille sparkline is actually *safer* than a block sparkline in that configuration. Mitigation: a self-check on start-up (print ▂, query cursor position via CSI 6n, compare) and fall back to Braille or ASCII if it moved two columns. (The sparkline widths in `120x36-map.txt` today assume width 1.)
- Octants give exactly Braille's 2×4 grid, so any code that computes Braille dot masks can emit octants from the same mask with a 256-entry table — one renderer, two glyph sets. ratatui's `Marker::Octant` already does that inside Canvas/Chart.

### Gaps
- No source found measuring which *macOS* fonts ship the Legacy Computing Supplement (JetBrains Mono, Iosevka, SF Mono, Menlo); the search for JetBrains Mono / Iosevka octant coverage returned nothing explicit. Treat octants as "Ghostty / Windows Terminal / Cascadia only" until verified by hand (`printf '\U1CD00'` in each terminal).
- Apple Terminal's and iTerm2's fallback behaviour for U+1CDxx (tofu vs. font fallback to a system font) was not verified.

---

## KQ1. Time series — Braille line charts, filled areas, legends, compact axes, zoom levels, sparklines

### Takeaway
The gold standard is btop's technique: two samples per column packed into one Braille/block
glyph, scaled to a fixed `max_value` (100 % for utilisation), coloured per column by a
gradient indexed on the larger of the two values — so time runs right-to-left with newest at
the right edge, and colour is severity. ratatui gives the pieces (Chart + `Marker::Braille`,
`GraphType::Area` since 0.30.1, Sparkline with absent values), but the axis labelling and the
"last 4 min / 1 h / 24 h" windows must be hand-rolled. Sparklines should be downsampled with
min/max or LTTB, never with means, or spikes vanish.

### Cited Findings
- btop packs **two consecutive data points per character column**: `_create` iterates the data, maps each of the pair (`last`, `data_value`) to 0–4 against per-row height thresholds, and indexes a 25-entry table per row: `graph_symbol.at(result[0] * 5 + result[1])`; the tables are `Symbols::graph_symbols["braille_up"|"braille_down"|"block_up"|"block_down"|"tty_up"|"tty_down"]` (5×5 each) — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp)
- btop scales values before drawing: `data_value = clamp((data_value + offset) * 100 / max_value, 0, 100)`; `offset` exists so negative-baseline series (temperature) still fit — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp)
- btop colours each column from a 101-step gradient indexed by the **max of the two packed values**: `out += Theme::g(color_gradient).at(clamp(max(last, data_value), 0, 100))` when a gradient is set; `invert` swaps the `_up`/`_down` symbol tables and reverses row order so a graph can hang from the top (used for download/upload pairs) — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp)
- btop theme gradients are three-stop: `cpu_start`, `cpu_mid`, `cpu_end` (likewise `mem_*`, `temp_*`, and `download`/`upload`), blended into the 101-step ramp; 24-bit colour recommended, 256- and 16-colour fallbacks exist — [btop README](https://github.com/aristocratos/btop)
- ratatui `Sparkline` accepts `u64`, `Option<u64>` or `SparklineBar`; `None` is drawn with `absent_value_symbol` (default `symbols::shade::EMPTY`) and `absent_value_style`; `max()` fixes the scale (defaults to the data max); `direction()` is `LeftToRight` or `RightToLeft`; `bar_set()` is `NINE_LEVELS` by default or `THREE_LEVELS`; per-bar styles via `SparklineBar::style()` — [ratatui Sparkline](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Sparkline.html)
- ratatui 0.30.1 added filled area charts: `GraphType::Area` with `Dataset::fill_to_y()` in `Chart`, and `FilledLine` for `Canvas` (PR #2426) — [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)
- `tui-bar-graph` 0.3.6 (2026-09-24, part of `tui-widgets`) draws vertical bar graphs in `BarStyle::Braille`, `Solid`, `Quadrant` or `Octant`, with `ColorMode::Solid` or `VerticalGradient` and any `colorgrad` preset (turbo, magma, viridis, inferno, plasma, sinebow, rainbow): `BarGraph::new(data).with_gradient(colorgrad::preset::turbo()).with_bar_style(BarStyle::Braille).with_color_mode(ColorMode::VerticalGradient)` — [tui-bar-graph](https://docs.rs/tui-bar-graph); the `tui-widgets` workspace also carries tui-big-text, tui-box-text, tui-cards, tui-equalizer, tui-popup, tui-prompts, tui-qrcode, tui-scrollbar, tui-scrollview — [tui-widgets](https://docs.rs/tui-widgets)
- `trazo` 0.0.1 (2026-08-10, ratatui ^0.30) promises "stacked and grouped bars, candlesticks, heatmaps, area plots" and "richer axes with auto-tick and date formatting", widgets implementing `ratatui::widgets::Widget`; a search snippet additionally lists backends "ascii, ascii+color, half-block, octants, and braille" — [trazo on docs.rs](https://docs.rs/trazo); backend list from [trazo 0.0.1 crate page](https://docs.rs/crate/trazo/0.0.1) (snippet only; not confirmed on the rendered docs page)
- Downsampling: "Naive downsampling (take every N-th point) can miss sharp spikes and troughs"; ClickHouse's `largestTriangleThreeBuckets(N)(x, y)` implements LTTB, which "selects N representative points that preserve the visual appearance of the original series" — [oneuptime on ClickHouse LTTB](https://oneuptime.com/blog/post/2026-03-31-clickhouse-largesttrianglebuckets/markdown); MinMaxLTTB pre-selects minima and maxima per bucket before LTTB — [MinMaxLTTB, IEEE VIS 2023](https://content.ieeevis.org/year/2023/paper_v-short-1092.html); Chart.js documents that min/max decimation "will preserve peaks in your data but could require up to 4 points for each pixel" and suits "a very noisy signal where you need to see data peaks" — [Chart.js decimation](https://chartjs.org/docs/next/configuration/decimation.html)
- Tufte: sparklines are "data-intense, design-simple, word-sized graphics" that "can be everywhere a word or number can be: embedded in a sentence, table, headline, map, spreadsheet, graphic" — [Tufte, Sparkline theory and practice](https://www.edwardtufte.com/notebook/sparkline-theory-and-practice-edward-tufte/); term introduced 2006 — [Wikipedia: Sparkline](https://en.wikipedia.org/wiki/Sparkline)
- sampler (Go) is the reference "YAML dashboard" with exactly these component types: runchart, sparkline, barchart, gauge, textbox, asciibox — [sampler README](https://raw.githubusercontent.com/sqshq/sampler/master/README.md)
- trippy (ratatui) shows "per hop round-trip-time (RTT) history and frequency distributing charts" next to a stats table — [trippy features](https://trippy.cli.rs/start/features)
- Plotters: since 0.3 "all backends have been hosted as separate crates" and third-party backends plug in by implementing `DrawingBackend` — [plotters-backend](https://docs.rs/plotters-backend/); **no `plotters-ratatui-backend` crate was found on docs.rs/crates.io in this search** (two searches); only a Python `plotui-ratatui` surfaced — [plotui on PyPI](https://pypi.org/project/plotui/)

### Inferences
- **Fixed scale, not auto-scale.** btop's `max_value=100` for utilisation is the right rule for cobserve: a node's memory sparkline must be drawn against *its own capacity* (cobserve already prints `59.6/64 GiB`), so that a flat ▇▇▇ at 93 % and a flat ▂▂▂ at 20 % read differently across nodes. ratatui `Sparkline::max()` does this. Auto-scaled sparklines in a fleet grid lie (every node looks volatile).
- **Worst-of-slice, not mean.** When 240 one-second samples (4 min) fold into 24 columns, take max per column for memory/CPU (the question on call is "did it spike?") and keep min as a second, dim series if two rows are available. LTTB is the principled version when the series is long (24 h) and shape matters; for ≤ 1000 points and ≤ 160 columns, min/max per bucket is enough and is O(n).
- **Annotate min/max** on any sparkline wider than ~20 cells: Tufte's convention is a dot on the high and low point; in cells, print the two numbers at the ends (`▂▃▆▇▇▃  lo 41 % · hi 93 %`). This is what makes a sparkline "trace to a formula" (§5): the peak printed is the peak drawn.
- **Zoom conventions.** bottom/btop both keep newest at the right and show the window length as a single left-edge label. For cobserve: `4m ⟵ ... now` with `[`/`]` or `-`/`+` stepping 4 min → 1 h → 24 h; the label is the window, not tick marks (a Braille chart 60 wide has no room for ticks). The data per window must already exist (ring buffers at 1 s / 15 s / 6 min) — no interpolation.
- **Multi-series with legend:** ratatui `Chart` supports several `Dataset`s with their own `Marker`/style and a legend; but with "colour = severity only", series identity must come from **glyph**, not hue: memory as solid area (`GraphType::Area`), CPU as a line (`Braille`), both in the cream text colour, with the severity ramp applied only where a value crosses a threshold. 0.30.0's Braille-over-blocks rendering makes exactly this overlay possible (area in blocks, line in Braille on top).
- **Filled vs line:** fill the series that is a *level* (memory used; queue depth), line the series that is a *rate* (CPU cores, queries/s). This matches btop (mem/CPU filled, net line).

Mock-ups (cobserve data). Braille/area line chart, 60 wide, compact axes, two series by glyph:

```
 clickhouse3 · last 4 min · mem GiB of 64 · cpu cores of 16
 64 ┤                                              ▗▄▟████████
    │                                       ▗▄▄▟███████████████     mem 59.6 ▲ hi 59.6
 32 ┤ ⠀⠀⢀⣀⣀⣀⣀⣀⣀⠤⠤⠤⠒⠒⠒⠉⠉⠉⠑⠒⠒⠤⠤⣀⣀⣀⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⠤⠒⠉⠉⠉⠉⠉⠉⠉⠉⠉   cpu 14.9 ─ hi 15.1
    │▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄
  0 ┼──────────────────────────────────────────────────────────
    -4m        -3m         -2m         -1m          now   [ ] zoom
```
(axis: three labels only — 0, half, full capacity — because every % prints its denominator in
the header; x axis four labels; the right margin carries current value + window peak.)

Sparkline in a table row, with worst-of-slice folding and peak annotation:

```
 ▸ clickhouse-bi   ▆▆▃▅▆▆▄▅▆▆▇▇  67.2%  43.0/64   hi 71.4% at -1m
                   ^ 240 samples → 12 columns, each column = max of 20 s
```

Queue depth (a level) as a 2-row filled area with absent samples:

```
 queries · waiting · last 1 h       max 21
 ▂▃▅▇██▇▆▅▃▂▁▁▂▃▅▆▇██░░▇▆▄▃▂▂▃▄▆▇█▇▆▅▄▄▃▂▂▁▁
 ▁▁▂▃▄▅▄▃▂▁▁▁▁▁▂▃▄▅▆▆░░▅▄▃▂▁▁▁▂▃▄▅▅▄▃▂▂▂▁▁▁▁       ░ = no poll (Sparkline absent value)
```

### Gaps
- bottom's documentation pages (CPU widget: time axis labels, zoom keys, autohide axis) returned 404 at two URLs, so its exact axis convention could not be quoted; the claim that it labels the left edge with the window length is from memory and is **not sourced** — verify in the bottom repo (`docs/content/usage/widgets/cpu.md`) before relying on it.
- No Rust crate implementing LTTB for ratatui was identified; the algorithm is ~40 lines and ClickHouse can also do it server-side (`largestTriangleThreeBuckets`) if 24 h history is ever read from `system.metric_log`.
- trazo's backend list (ascii/half-block/octants/braille) comes from a crate-page snippet, not the rendered docs; at 0.0.1 and two months old it is a watch item, not a dependency.

---

## KQ2. Composition and share — stacked bars that sum to 100 %, treemaps, band charts

### Takeaway
A single-row **stacked horizontal bar** is the honest terminal form for "who holds this node's
memory" (cobserve's §5.3 invariant: user rows add up to the node): segments in eighth-block
resolution, identity by **glyph texture** (█ ▓ ▒ ░ and ▞) rather than hue, each segment's
percentage printed. Treemaps do exist in terminals (diskbloom, diskonaut, darya, leaves,
treemonger) and all correct for the ~2:1 cell aspect; they pay off only above ~10 items with a
hierarchy (fleet → node → user), not for 4 users of one node.

### Cited Findings
- dust draws a directory's share as a bar of block characters where "the shades of grey are used to indicate which parent folder a subfolder belongs to", with a percentage column; "The largest subdirectories will be colored"; `-b` drops bars, `-B` moves bars right, `-c` is monochrome, `--dim` dims bars for dark terminals; output is limited to terminal height − 1 rows — [dust README](https://github.com/bootandy/dust)
- diskbloom uses "the squarified treemap algorithm" with "cell-aspect correction" because terminal cells are "~2× taller than wide"; rounded borders are "fused into single lines"; area = size on disk, colour = file type (hues spread by the golden angle), brightness = share of parent, nesting three levels deep; Go + Bubble Tea v2 — [diskbloom](https://github.com/Zingzy/diskbloom)
- diskonaut (Rust) "displays directory sizes in an interactive treemap visualization" — [diskonaut man page](https://linuxcommandlibrary.com/man/diskonaut); darya has "a live color-coded treemap view" — [darya on Terminal Trove](https://terminaltrove.com/darya/); treemonger "uses the squarify subdivision rules, shows the directory hierarchy with thin but nonzero margins, and uses color for depth with filename labels" — [treemonger](https://github.com/alanbernstein/treemonger); Leaves is "a text-mode disk usage treemap visualizer inspired by WinDirStat" — [Leaves](https://zeli.app/story/48936389)
- ratatui `BarChart::grouped()` (0.30.0, PR #1513) draws grouped bars; `BarChart` supports horizontal direction and value labels — [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md), [ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)
- `tui-piechart` 1.0.3 (2026-10-10, ratatui ^0.30): slices with colour, labels, percentages, configurable fill char (e.g. `█`), legend position/alignment/markers — [tui-piechart](https://docs.rs/tui-piechart)
- Stephen Few's rule for qualitative ranges on a bullet graph: use "a single hue in progressively darker intensities" — [Bullet Graph Design Spec (PDF)](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf)

### Inferences
- **ratatui has no stacked-bar widget**; one is ~60 lines: for a row of width W, compute each segment's width in eighths (`round(share × W × 8)`), emit `█` cells and a boundary cell from `▏▎▍▌▋▊▉` so adjacent segments meet at eighth precision — the current gauge glyphs `━╸╺` already do this at half-cell precision. Fix the rounding so the segments **sum exactly to W×8** (largest-remainder rounding), otherwise the §5.3 invariant is visibly broken by a stray empty cell.
- **Identity without hue:** cobserve forbids colour for identity, so segments need texture: `█` queries of user A, `▓` user B, `▒` user C, `░` "server · caches · merges", `·` free. With more than four users, fold the tail into "others" (as the NODES tree already folds healthy nodes). Severity colour still applies to the *whole bar* (node ≥ 90 % → red), not to segments.
- **Pie charts: no.** tui-piechart exists but a pie in a 2:1 cell grid distorts angles, and it answers nothing a one-line stacked bar does not; it also needs hue for slices, which the house rule forbids.
- **Treemap: fleet level only.** A fleet treemap (area = node capacity, inner tiles = users, brightness = pressure) would be a genuine MAP view for 20–40 nodes; for 9 nodes the current card grid is clearer. If built, copy diskbloom's rules: squarify on a grid scaled x:2 to correct aspect, minimum tile 6×2 cells for a label, fused single-line borders. Rust reference: diskonaut.
- **Band chart** (shares over time) is a stacked area: with 0.30.1's `GraphType::Area` + `fill_to_y` it can be done by drawing cumulative series bottom-up; readable only for ≤ 3 bands.

Mock-ups. One-row stacked share bar for a node (§5.3 — segments sum to the node):

```
 clickhouse3   mem 59.6/64 GiB  93.1%
 █████████████▓▓▓▓▓▓▓▒▒▒░░░░░░░░░░░░░░░░░░░░░░░░░··
 █ grigol.gankava 25.5% 16.3   ▓ j.petrova 12.9% 8.23   ▒ airflow 5.5% 3.53
 ░ server · caches · merges 47.4% 30.3                   · free 6.9% 4.4 GiB
```

Per-user share as a column inside the existing tree (keeps the table, adds the whole):

```
   NODE · USER → PERSON          SHARE OF NODE (64 GiB)            GiB
  ▾ clickhouse3                  █████████████▓▓▓▓▓▓▒▒▒░░░░░░░░░░░░░░░░░░··  59.6
   ├─ r_redash → grigol.gankava  █████████████                               16.3
   ├─ r_redash → j.petrova                     ▓▓▓▓▓▓▓                        8.23
   ├─ airflow                                         ▒▒▒                     3.53
   └─ server · caches · merges                           ░░░░░░░░░░░░░░░░░░  30.3
```
(each child's segment sits at the offset where it lives in the parent bar — the sum is visible
without reading numbers.)

Fleet treemap sketch, 40 nodes, 60×12 cells, aspect-corrected:

```
 ╭────────────────────╮╭──────────╮╭──────────╮╭─────╮╭─────╮
 │ clickhouse3  93% ✖ ││ ch-bi 67%││ ch7  60% ││ch2  ││ch5  │
 │ ████▓▓▒░░░░░░░░░░  ││ ██▓▓░░░  ││ █▓░░░    ││ ▓░  ││ ░   │
 ╰────────────────────╯╰──────────╯╰──────────╯╰─────╯╰─────╯
 ╭────╮╭────╮╭────╮╭────╮╭───╮╭───╮╭───╮╭───╮╭──╮╭──╮╭──╮╭──╮
 │ch4 ││ch6 ││ch8 ││ch9 ││   ││   ││   ││   ││  ││  ││  ││  │
 ╰────╯╰────╯╰────╯╰────╯╰───╯╰───╯╰───╯╰───╯╰──╯╰──╯╰──╯╰──╯
```

### Gaps
- No terminal tool was found that draws a *100 % stacked* bar per row as a first-class widget (dust's bars show nesting, not a sum); the stacked bar above is a design, not a documented pattern.
- diskbloom does not publish its exact aspect ratio constant; "~2×" is its own wording.

---

## KQ3. Heatmaps, matrices, run timelines, calendar heatmaps

### Takeaway
A heatmap in cells is one glyph per (row, column) with a 5-step ramp — either a shade ramp
(` ░▒▓█`) or a colour ramp (GitHub's five greens, lnav's green→yellow→red spectrogram).
For node × minute it is the densest honest form on screen: 40 nodes × 60 minutes fits in
40×60 cells. Run timelines in cells (cobserve's `▪·✖▸◌━` row) are the terminal Gantt; the
only Airflow TUI found (flowrs) does not draw one. Calendar heatmaps (git-cal, heatmap_cli)
are a solved pattern with 5 levels per day.

### Cited Findings
- lnav's SPECTRO view is "a 'three'-dimensional display of data points of a log field or a SQL query column", time on the Y axis, value ranges on the X axis, colour intensity for frequency, legend "green having the fewest number of data points, yellow the middle, and red the most" — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)
- lnav's HIST view is "a stacked bar chart of messages over time classified by their log level and whether they've been bookmarked", toggled with `i` / `Shift+i`, zoomed with `z` / `Shift+z` — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)
- git-cal: "each block in the graph corresponds to a day and is shaded with one of the 5 possible colors representing relative number of commits on that day" — [git-cal](https://github.com/CurbaiCode/git-cal); checkgit (Rust) prints "a GitHub-style contribution heatmap ... directly in the terminal" — [checkgit](https://docs.rs/checkgit); `heatmap_cli` (Python) and `calendarheatmap` (Go) are the same pattern — [heatmap_cli](https://pypi.org/project/heatmap_cli/0.13.3), [calendarheatmap](https://beta.pkg.go.dev/github.com/nikolaydubina/calendarheatmap)
- trazo 0.0.1 lists heatmaps among its chart types for ratatui ^0.30 — [trazo](https://docs.rs/trazo)
- ratatui `Calendar` widget (feature `widget-calendar`) "displays a single month" with per-date styling — [ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)
- flowrs (Rust, ratatui) is "a TUI for Apache Airflow" to "monitor, inspect, manage and trigger" DAGs, with filter/list/trigger/pause of DAG runs, task management, open-in-browser, multiple Airflow servers, and managed providers (Conveyor, MWAA, Cloud Composer, Astronomer) — [flowrs](https://github.com/jvanbuel/flowrs), [Terminal Trove: flowrs](https://terminaltrove.com/flowrs/). Its README demo GIFs are not described in text; no Gantt/timeline is mentioned.
- Airflow 2.x's own web UI is the reference for grid (run × task state matrix) and Gantt views; nothing in the sources above reproduces them in a terminal.
- ratatui 0.30.1 `Fill` widget paints an area with one repeated symbol (PR #2520) — [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)

### Inferences
- **Node × minute heatmap** is the strongest new form for cobserve's fleet. Per cell: max utilisation in that minute → one of five glyphs ` ░▒▓█` (works in mono) with the severity colour applied *only* above thresholds (amber ≥ 70 %, red ≥ 90 %), so colour stays severity and texture carries magnitude. Rows sorted worst-first; a cursor on a cell shows node/minute/value in the drawer. 40 nodes × 60 min = 40 rows × 60 cols, fits 160×48; at 120×36 show 30 min or fold healthy rows (as today).
- Use **5 levels, not 8**: ▁–█ eighths in a heatmap read as height, not intensity, and sit on the baseline (the top of the cell stays empty), which makes a row look like a bar chart. Shade characters fill the cell uniformly. GitHub, git-cal and lnav all stop at 4–5 levels because more are indistinguishable at a glance.
- **Airflow timeline (Gantt) in cells** — cobserve's ACTIVITY row already does the "one glyph per slot" form. The improvement is a true Gantt for the *selected* DAG: one row per task, `━` for the running span placed by start/end minute, `▸` current, `✖` failed, `◌` queued, `↻` retry, with the hour grid `18 00 06 12` reused. 24 h in 96 columns = 15 min per cell; a task shorter than a cell still gets one `▪`.
- **Hours logged**: the Jira "Time logged" view already draws a day bar chart. A GitHub-style **month heatmap** (7 rows = weekdays × 5 columns = weeks, 5 shade levels against the 7h30 norm) is smaller and shows weekly rhythm; keep the bar chart for exact hours (it prints them), add the heatmap as the compact band form. ratatui `Calendar` can host it directly via per-date styles.

Mock-ups. Node × minute heatmap (severity colour only on ▓/█ cells over threshold):

```
 FLEET · max memory per minute · last 60 min · ░ <40% ▒ <70% ▓ <90% █ ≥90%
 node          -60m                                              now
 clickhouse3   ▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓███████████████████████████████████  93%
 clickhouse-bi ░░░▒▒▒▒▒▒▒▒▓▓▓▓▓▓▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▒▒▒▒▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▒▒▒▒  67%
 clickhouse7   ░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░  60%
 clickhouse2   ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▒▒▒▒▒░░░░░  42%
 ch4 ch6 ch8 ch9  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  all ≤ 22%
```

Gantt for one DAG's day (one row per task, 15 min per cell):

```
 statement_daily_agg_reload · 13:39 → running 2h12m · 4/7 tasks
 task                 13     14     15     16
 extract_statements   ━━━━✔
 build_partitions         ━━━━━━━✔
 validate_counts                 ━━✔
 reload_partitions                 ━━━━━━━━━━▸        running 1h05m
 refresh_views                               ◌        queued
 notify                                      ◌
 ping_clickhouse      ✖↻ ✖↻ ✖                         3 tries, failed
```

Calendar heatmap of hours logged (5 levels against 7h30):

```
 October 2026 · 120h in 16 working days      ░ <2h ▒ <5h ▓ <7h30 █ ≥7h30
        W40  W41  W42  W43  W44
 Mon         █    █    ▓    █
 Tue         ▓    █    █    ▒
 Wed         ·    ▓    █
 Thu    █    ▒    █    █
 Fri    █    ▓    ░    ▓
```

### Gaps
- No Airflow terminal tool with a Gantt or grid view was found (flowrs' screenshots are GIFs, undocumented in text). The Gantt above is a design derived from Airflow's web UI, not from a terminal precedent.
- No source gives a measured legibility limit for shade ramps (how many of ` ░▒▓█` a user distinguishes at 10–12 pt); the "5 levels" rule rests on GitHub's and git-cal's practice, not a study.

---

## KQ4. Gauges, dials, bullet graphs

### Takeaway
Few's bullet graph — a thick measure bar over 2–5 single-hue qualitative bands with a thin
comparison tick — is a one-row widget that answers "percentage of a denominator with a
threshold", which is every bar cobserve draws. ratatui's `Gauge`/`LineGauge` give the
measure bar; the bands and the target tick are a few extra cells. Ring/arc gauges in Braille
have no crate and no reason to exist in a monitor.

### Cited Findings
- Few's bullet graph components: label, quantitative scale, featured measure bar, comparative measure marker ("typically a thin line"), 2–5 qualitative ranges as background bands; the measure bar is "noticeably thicker than the comparative marker"; ranges use "a single hue in progressively darker intensities" with darker = poor; horizontal is the normal orientation; Few positions bullet graphs as the replacement for dashboard gauges and meters — [Bullet Graph Design Spec (PDF)](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf)
- The bullet graph "was developed to replace the meters and gauges that are often used on dashboards" because its "linear and no-frills design" fits "a small space, which is essential on a dashboard" — [FusionCharts chart primer](https://www.fusioncharts.com/resources/chart-primers/bullet-graph)
- ratatui `Gauge` renders a percentage "using block characters" with `use_unicode` fractional blocks; `LineGauge` renders progress "as a line" (one row) — [ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)
- cobserve's current bars already are one-row `LineGauge`-style tracks (`━━━╸━━━━━━ 36.4% 280/768 GiB`) — [docs/screenshots/120x36-nodes.txt](../../screenshots/120x36-nodes.txt)

### Inferences
- **Turn the existing bar into a bullet.** Keep the track `━━━╸━━━━━━`, add (a) the threshold band boundaries as `┊` at 70 % and 90 % of the track (the §5.4 thresholds), (b) a comparison tick `╷`/`▏` for "4 min ago" or "this time yesterday", (c) the number with denominator, which is already there. The qualitative bands as *background shade* (Few's single-hue intensities) cannot be done on a one-row glyph track without colour, so use the two boundary ticks instead; on 2-row cards, draw a `░▒▓` band row under the bar.
- **Thresholds in colour are allowed** (colour = severity): the measure bar turns amber past the first tick, red past the second. That is Few's dark-is-bad rule turned into cobserve's rule.
- **No dials.** An arc gauge in Braille needs ≥ 7×4 cells for a legible arc, encodes one number, and reads worse than a 12-cell bar with its number. Nothing in the surveyed tools (btop, bottom, k9s, sampler) draws dials; sampler's "gauge" is a horizontal bar.
- Worker occupancy `●●●●●●○ 6/7` is already the right "unit chart" for small integers — keep it; above ~12 units switch to a bar.

Mock-ups. Bullet bar replacing a plain gauge (one row, mono-safe):

```
 mem  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸━━━━━━━━━━┊━━━━━┊━━  59.6/64 GiB  93.1%
                                ▲ 4 min ago 52.1          70%   90%
```
Compact variant for table rows (ticks only, threshold crossing in colour):

```
 93.1%  ━━━━━━━━━━━━━┊━━┊╸  59.6/64      ← red: past the 90 % tick
 25.5%  ━━━╸━━━━━━━━━┊━━┊━  16.3         ← cream: under both
```

### Gaps
- No ratatui crate for arc/ring gauges was found (searches for Braille ring gauges returned nothing relevant); this is reported as absence, and also as a recommendation not to build one.

---

## KQ5. Distributions — histograms, box plots, log-scale bars, flame graphs

### Takeaway
Latency histograms and frequency charts are standard (trippy draws RTT frequency per hop in
ratatui; lnav stacks message counts per level); a text box-plot is a one-row glyph string
(`├──┤█┃█├────┤`). Flame graphs in the terminal are proven by flamelens (ratatui 0.29), which
chdig embeds for ClickHouse `system.trace_log` — each stack frame is a row of cells whose width
is proportional to samples, which is directly portable to cobserve's query-time breakdown.

### Cited Findings
- chdig provides "Flamegraphs (CPU/Real/Memory/Live) in TUI (thanks to flamelens)", with auto-refresh while a query runs and highlighting of changes since the previous update (`P` pauses, `D` toggles difference colouring); it also renders query pipelines with viz.js and offers split panes and cluster aggregation; status "pre-alpha with core features daily-usable" — [chdig](https://github.com/azat/chdig), [Terminal Trove: chdig](https://terminaltrove.com/chdig/)
- flamelens is "an interactive flamegraph viewer in the terminal" for folded stacks, with hjkl navigation, Enter to zoom into a frame, regex search with highlighting, live mode (`z` freezes); version 0.4.0 depends on ratatui 0.29.0 and crossterm 0.28.1 (optional py-spy for Python live profiling) — [flamelens](https://github.com/YS-L/flamelens), [flamelens Cargo.toml](https://raw.githubusercontent.com/YS-L/flamelens/main/Cargo.toml)
- trippy shows "per hop round-trip-time (RTT) history and frequency distributing charts" alongside sent/received/loss/avg/best/worst/stddev/jitter — [trippy features](https://trippy.cli.rs/start/features)
- lnav HIST: "a stacked bar chart of messages over time classified by their log level" — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)
- ratatui `BarChart` supports grouped datasets, horizontal direction and value labels — [ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)

### Inferences
- **Query-duration histogram per node** (buckets 0–1 s, 1–10 s, 10–60 s, 1–10 min, > 10 min) is a 5-bar horizontal `BarChart` with counts printed; bucket edges are log-spaced, so label the edges and never draw a log *axis* — in 5 bars nobody reads a log scale, they read the labels.
- **Log-scale bars:** when values span 3+ orders of magnitude (memory 10 MiB … 50 GiB across queries), a linear bar makes all but one row empty. Draw `log10(x/min)` scaled, print the value, and mark the bar `log` in its header; never mix linear and log bars in one column.
- **Text box plot** for latency per user/day: `min ├──┤ q1 █ median ┃ q3 ├──┤ max` in one row of ~30 cells; with the five numbers printed underneath it meets the trace-to-a-formula rule. trippy's stats table (best/avg/worst/stddev) is the tabular equivalent and may be better at 80 columns.
- **Flame graph for a query** (`system.trace_log` sampled while the query runs, read-only) is a stretch feature: flamelens shows the drawing technique (rows of blocks, label truncated to width, zoom on Enter) and is MIT/ratatui, but it is on ratatui 0.29, so vendor the renderer rather than depend on it. It is also the one place where hue-for-identity is conventional (warm random hues per frame); under cobserve's rule, use a single hue with intensity by self-time and reserve red for frames in the current search match.

Mock-ups. Latency histogram (horizontal, log-spaced buckets, counts printed):

```
 clickhouse3 · 4 running + 38 finished in 4 min · by duration
 < 1s     ████████████████████████████████████  31
 1–10s    ███████  6
 10–60s   ███  3
 1–10m    █  1   (j.petrova)
 > 10m    █  1 ✕ (grigol.gankava 8m31s)
```

Text box plot row:

```
 user              latency last 24 h (s)   min  q1   med  q3   max
 j.petrova         ├────┤██┃█████├──────────┤   0.4  2.1  3.0  9.8  61.2
 grigol.gankava    ├┤█┃██├─────────────────────┤ 0.2  0.9  1.4  3.1  511
```

Flame graph for one query (rows = depth, width ∝ samples):

```
 c3ee64bf · 1 200 samples · 2.4 s · CPU
 ████████████████████████████████████████████████████████████  Query::execute
 ████████████████████████████████████████████▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒  Aggregating ·· Sorting
 ██████████████████████████▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒░░░░░░░░░░▒▒▒▒▒  Hash ····· Merge · Spill
```

### Gaps
- flamelens' README does not describe its rendering internals (cell mapping, minimum frame width, colour scheme); only its dependencies were verified. Read `src/` before porting.
- No source was found for a text box-plot convention in a monitoring TUI; the glyph string above is a design.

---

## KQ6. Event streams — how lnav, tig, k9s, btop, trippy draw dense logs with severity

### Takeaway
The surveyed log viewers agree on three devices: a **severity gutter** (one glyph/colour per
row at the left), a **density strip** (lnav's histogram/spectrogram, time on one axis, count as
shade), and **structure highlights** (identifiers, IPs, SQL keywords in a consistent style).
cobserve's TAPE already has the gutter (`▲ ✖ ✔ ●`); the missing pieces are the density strip
over time and zoom.

### Cited Findings
- lnav LOG view: "Errors will be colored in red; warnings will be yellow"; search hits in reverse video; IP addresses, SQL keywords, XML tags highlighted; "identifiers" get random colours; the breadcrumb bar (backtick) shows "the semantic location of the focused line"; PRETTY view (`Shift+P`) pretty-prints the focused structured text — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)
- lnav HIST view: stacked bars by log level over time, `i` toggles, `z`/`Shift+z` zoom; SPECTRO view: time × value-range with green/yellow/red frequency — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)
- btop graphs can be inverted (`invert` swaps `_up`/`_down` symbol tables) to hang from the top — the pattern behind its mirrored download/upload pair — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp)
- trippy pairs a per-hop stats table with RTT history and frequency charts in ratatui — [trippy features](https://trippy.cli.rs/start/features)
- cobserve's TAPE (29 events, `✖ ▲ ✔ ●` gutter, KIND column, newest first) — [docs/screenshots/120x36-tape.txt](../../screenshots/120x36-tape.txt)

### Inferences
- **Add lnav's histogram as a 2-row strip** above the TAPE list: one column per minute (or per 5 min at 24 h), stacked `✖` count in red over `▲` count in amber as block heights, so "when did it get bad" is visible before reading rows; `z` zooms the strip and the list together. This also fixes the TAPE's one visible weakness today — repeated runaway/ended pairs every 60 s flood the list; the strip shows the burst as one tall column.
- **Collapse repeats** (lnav-style): the m.kairys runaway→95 %→ended cycle recurring every minute should fold into one row `✖ 15:26–15:29  QUERY  m.kairys on clickhouse-bi · 4× runaway → killed at 99 % · 8.9 GiB peak  ▸ 12 events`, expandable.
- **Identifier highlighting without hue:** lnav colours identifiers randomly, which the house rule forbids; use bold/underline for the node name and the person, dim for the hash and table — structure by weight, not colour.
- **Severity glyphs are already correct** (`✖ ▲ ✔ ●`); keep them so mono terminals lose nothing.

Mock-up. TAPE with a density strip and folded repeats:

```
 TAPE · 29 events · 8 ✖ · 18 ▲ · last 1 h          ✖ ▇  ▲ ▃
 15:00        15:15        15:30        15:45     now
 ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▂▃▇█▇▂▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁   ✖ per minute
 ▁▁▁▁▂▁▁▁▁▁▁▁▂▁▁▁▁▁▁▁▂▄▆▆▆▄▂▁▁▁▁▂▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁   ▲ per minute
 ▌✖ 15:26–15:29  QUERY  m.kairys on clickhouse-bi · 4× runaway → killed at 99 % · peak 8.9 GiB   ▸ 12
  ✖ 15:27:09     QUEUE  Redash queries backed up: 9 waiting, oldest 3m01s
  ✔ 15:28:18     NODE   ch6 answering again · was unreachable 30 s (timed out after 1.5 s)
```

### Gaps
- tig, k9s and gh-dash were not fetched in this pass (budget); their event/column conventions are not sourced here. k9s' event view and gh-dash's PR table are table-first designs with a status glyph column — the same pattern as TAPE — but that is unverified in this note.

---

## KQ7. Fleet overview — tile grids, small multiples, space-filling layouts for 40+ nodes

### Takeaway
For 9–40 nodes the proven terminal forms are, in order of density: a **card grid** (what MAP
does now; ~28×4 cells per card → 16 cards at 120×36, 30 at 160×48), **small multiples of
sparklines** (one row per node: ~60 fit at 160×48), and a **node × time heatmap** (KQ3).
Tufte's small-multiples principle — same scale, same design, repeated — is what makes any of
them comparable. Hilbert/space-filling datacenter maps had no terminal precedent in this
search.

### Cited Findings
- Tufte: small multiples "are the best design solution to present a wide range of problems"; sparklines combine naturally with them for "direct visual comparisons" — [Wikipedia: Sparkline (citing Tufte)](https://en.wikipedia.org/wiki/Sparkline), [Tufte, Sparkline theory and practice](https://www.edwardtufte.com/notebook/sparkline-theory-and-practice-edward-tufte/)
- cobserve MAP today: 4 cards per row at 120 columns, each card = name + queries, mem bar, cpu bar, sparkline (4 rows), sorted by pressure, with a drawer for the selected node — [docs/screenshots/120x36-map.txt](../../screenshots/120x36-map.txt)
- ratatui `Sparkline::max()` fixes the scale so multiples share one scale; `RightToLeft` direction keeps newest at the edge — [ratatui Sparkline](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Sparkline.html)
- diskbloom's treemap applies aspect correction and fused borders for a tile layout — [diskbloom](https://github.com/Zingzy/diskbloom)

### Inferences
- **Scale by terminal size, not one design.** Three densities of the same data, chosen by what fits: ≥ 4 rows per node → cards (current); 1 row per node → small multiples table (name · mem bar · cpu bar · 20-cell sparkline · queries); < 1 row per node → heatmap rows of 1 cell/node/minute, or fold healthy nodes (as NODES does). At 40 nodes and 120×36 the cards no longer fit (10 rows of cards = 40 rows), so the second density is mandatory for the real fleet growth.
- **Same scale everywhere**: every sparkline in a grid against the node's capacity, every bar with the same track width; a legend printed once in the header, not per card.
- **Space-filling order**: sorting worst-first (current) already does what a Hilbert layout would (spatial locality of pressure). A treemap sized by capacity (KQ2) is the only space-filling form worth considering, and only above ~20 nodes.

Mock-up. Small multiples, one row per node, 120 columns (fits 40 nodes at 160×48 with the band):

```
 node           mem ━━━━━━━━━━━━ GiB      cpu ━━━━━━━━━━━━ cores   last 4 min (mem)        q   lag
 clickhouse3    ━━━━━━━━━━━┊╸ 93% 59.6/64  ━━━━━━━━━━━┊╸ 93% 14.9/16  ▂▃▄▄▅▆▆▇▇▇▇▇▇▇▇▇▇▇▇█  4 ✕2  0s
 clickhouse-bi  ━━━━━━━━┊━━━━ 67% 43.0/64  ━━━━━━━━━┊━━━ 72% 11.5/16  ▆▆▃▅▆▆▄▅▆▆▆▆▅▅▆▆▆▆▆▆  3 ✕1  0s
 clickhouse7    ━━━━━━━┊━━━━━ 60% 38.3/64  ━━━━━┊━━━━━━━ 44%  7.1/16  ▅▄▃▄▆▆▆▅▃▄▅▆▆▆▄▃▄▅▆▆  2 ✕1 12s
 clickhouse2    ━━━━━┊━━━━━━━ 42% 26.8/64  ━━━┊━━━━━━━━━ 29%  4.6/16  ▃▃▄▆▆▆▅▄▃▄▅▆▆▆▄▃▃▅▅▅  2     0s
 ▸ 5 healthy nodes folded · ch4 ch5 ch6 ch8 ch9 · all ≤ 38%                                   ␣ unfold
```

### Gaps
- No terminal tool using Hilbert curves or space-filling layouts for host fleets was found (searches for datacenter maps in TUIs returned nothing); treat it as unexplored rather than ruled out.
- hwatch/nerdctl were not investigated; neither is a fleet visualiser by description.

---

## KQ8. Inline raster graphics — ratatui-image, Plotters → PNG, the 2026 terminal matrix

### Takeaway
ratatui-image 11.1 (ratatui 0.30.1+) makes inline images practical where a protocol exists —
Kitty (kitty, Ghostty), iTerm2 (iTerm2, WezTerm, Rio), Sixel (xterm, foot, mlterm, VS Code with
a setting) — and falls back to half-blocks everywhere else, including Apple Terminal, Alacritty
and inside tmux. For a monitor whose values must be readable as text, a raster plot buys
anti-aliased lines at the cost of a second rendering path, a thread for encoding, and three
terminals where it silently degrades; it is justified only for one-off artefacts (a flame graph
PNG, a 24 h chart export), not for the live screens.

### Cited Findings
- ratatui-image 11.1.0 (2026-09-17, MIT) renders via Sixel, Kitty, iTerm2 or Unicode half-blocks; it detects the protocol from environment variables first, then terminal queries; it measures font size in pixels; `Image` is stateless/non-blocking, `StatefulImage` fits the area but "requires offloading resize/encoding to separate threads to prevent UI blocking"; depends on `image` 0.25.6+, ratatui 0.30.1+, `icy_sixel` 0.5 — [ratatui-image on docs.rs](https://docs.rs/ratatui-image/latest/ratatui_image/)
- ratatui-image compatibility matrix: Xterm Sixel ✔ ("Run with `-ti 340`"), Foot Sixel ✔, Kitty ✔ (≥ 0.28.0), WezTerm iTerm2 ✔ ("Also would support Sixel and Kitty, but only iTerm2 actually works bug-free"), Ghostty Kitty ✔ ("Implements Kitty with unicode placeholders"), iTerm2 ✔, Rio iTerm2 ✔ (Sixel glitches), mlterm Sixel ✔ ("Quite slow but no glitches"), Alacritty ❌ (sixel fork "probably never getting merged, and does not clear graphics"), Konsole ❌, Warp ❌ (clearing issues); "Halfblocks should work in all terminals, even if the font size could not be detected, with a 4:8 pixel ratio" — [ratatui-image README](https://github.com/ratatui/ratatui-image)
- VS Code 1.80 added Sixel and iTerm inline-image support in the integrated terminal, behind `terminal.integrated.enableImages` (off by default) plus `terminal.integrated.gpuAcceleration`; Linux and macOS only — [Petri on VS Code 1.80](https://petri.com/visual-studio-code-1-80/), [Ultralytics guide](https://docs.ultralytics.com/guides/view-results-in-terminal)
- Apple Terminal.app "does not support Sixel or image/Terminal Graphics Protocol" — [Terminal Trove: kitty vs Terminal.app](https://terminaltrove.com/compare/terminals/kitty-vs-mac-terminal/)
- The kitty graphics protocol specification (reference implementation kitty) — [kitty graphics-protocol.rst](https://git.wessel.gg/mirrors/kitty/src/tag/v0.48.0/docs/graphics-protocol.rst); a search summary lists kitty, WezTerm, Ghostty (full), Konsole ≥ 22.08 and foot ≥ 1.17 (partial) as implementers — [linuxjunkies guide](https://linuxjunkies.org/guides/terminal-true-color-and-images)
- Plotters' architecture allows any `DrawingBackend` crate; no ratatui backend crate was found (see KQ1) — [plotters-backend](https://docs.rs/plotters-backend/)

### Inferences
- **Terminal matrix for cobserve's owner (macOS):** Ghostty → Kitty protocol, also octants native; kitty → Kitty; WezTerm → iTerm2 protocol (per ratatui-image, the only bug-free one there); iTerm2 → iTerm2; Apple Terminal → half-blocks only; VS Code → Sixel/iTerm2 only if the user flips a setting, else half-blocks; **tmux** → the README gives no row for tmux and the only "works" reports are via writing directly to the client tty, so assume half-blocks inside tmux; the **desktop shell (wry + xterm.js)** → xterm.js has no graphics protocol without an addon (xterm-addon-image exists for Sixel/iTerm2 but is not confirmed in this note), so assume half-blocks there too. That is: the owner's own wrapper would be the first place the raster path fails.
- **Half-block fallback is a 2-colour-per-cell raster** (fg/bg), i.e. the same thing a `Marker::HalfBlock` Canvas draws — so "real plot via PNG" degrades to exactly what ratatui can draw natively, minus the text.
- **Verdict**: not for the live screens. Reserve ratatui-image for (a) a flame graph or 24 h chart rendered to PNG on demand (`g` key), shown in a popup, and (b) the Jira ticket's attached screenshots. Keep Braille/octant Canvas as the primary renderer; it is also what makes `dev/screenshots.sh` text captures possible.
- Rendering a Plotters chart to an in-memory RGB buffer (via `plotters-bitmap`) and handing it to ratatui-image is feasible without a ratatui backend crate; this is the only sensible "Plotters in ratatui" route today.

### Gaps
- No row for tmux, Windows Terminal, Apple Terminal or xterm.js in ratatui-image's matrix; their behaviour is inferred (half-blocks), not quoted.
- Performance numbers (encode time, flicker on redraw) for Kitty vs Sixel in ratatui-image were not found.

---

## KQ9. Colour — severity ramps, perceptual uniformity, colour-blind safety, 16-colour/mono

### Takeaway
btop's practice (a three-stop gradient `start/mid/end` expanded to 101 steps, indexed by the
value, 24-bit with 256/16 fallbacks) is the template for a severity ramp; `colorgrad` gives
perceptually uniform presets (viridis, magma, turbo) in Rust. For red/green deficiency the
safe pairs come from Okabe–Ito (vermillion `#D55E00`/orange `#E69F00` vs bluish green
`#009E73`/sky blue `#56B4E9`); cobserve's "colour = severity only" plus glyph redundancy
(`✖ ▲ ✔`) already satisfies the mono case, so the ramp only has to be *ordinal*, never
*identifying*.

### Cited Findings
- btop: per-column gradient colour `Theme::g(gradient).at(clamp(max(last, data_value), 0, 100))` — a 101-entry ramp — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp); themes define `cpu_start/cpu_mid/cpu_end`, `mem_*`, `temp_*`, `download`, `upload`; 24-bit recommended, 256-colour and 16-colour TTY fallbacks exist — [btop README](https://github.com/aristocratos/btop)
- lnav's spectrogram ramp is green (fewest) → yellow → red (most) — [lnav UI docs](https://docs.lnav.org/en/stable/ui.html); its log view uses red for errors, yellow for warnings — same source
- `tui-bar-graph` applies `colorgrad` presets (turbo, magma, viridis, inferno, plasma, sinebow, rainbow) as vertical gradients — [tui-bar-graph](https://docs.rs/tui-bar-graph)
- Okabe–Ito palette (8 colours): black `#000000`, orange `#E69F00`, sky blue `#56B4E9`, bluish green `#009E73`, yellow `#F0E442`, blue `#0072B2`, vermillion `#D55E00`, reddish purple `#CC79A7`; "distinguishable for people with all common types of color vision deficiency (CVD), including protanopia, deuteranopia, and tritanopia"; ~8 % of men and 0.5 % of women have a CVD — [Okabe-Ito reference](https://dev.conceptviz.app/fr/blog/okabe-ito-palette-hex-codes-complete-reference), [Coloring in R's Blind Spot (arXiv 2303.04918)](https://arxiv.org/pdf/2303.04918)
- Few: qualitative ranges should use "a single hue in progressively darker intensities", darker = poorer — [Bullet Graph Design Spec](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf)
- dust offers `--dim` to reduce bar brightness on dark terminals and `-c` monochrome — [dust README](https://github.com/bootandy/dust)

### Inferences
- **Keep the ramp stepped, not continuous.** A 101-step gradient (btop) is pretty but it makes "orange-ish" mean nothing precise. For cobserve the ramp should have exactly the §5.4 steps: cream (< 70 %), amber (70–90 %), red (≥ 90 %), with *texture* carrying magnitude inside a step (shade levels, bar length). This keeps every colour traceable to a threshold in §5 — the house rule in colour form.
- **Colour-blind safety by construction:** since colour never distinguishes two simultaneous things (only "how bad"), and every severity has a glyph (`✖ ▲ ✔ ●`), a deuteranope loses nothing but the pre-attentive pop. To keep that pop too, pick the Tarum palette's red and green from the Okabe–Ito side of the wheel (vermillion rather than pure red, bluish-green rather than pure green): they stay apart in both protan and deutan simulations, and red-vs-amber is separated by lightness as well as hue.
- **16-colour mode:** map cream → default fg, amber → yellow (bold), red → red (bold), indigo accent → blue; shade heatmaps and Braille charts need no colour at all. **Mono:** already covered by glyphs; the one thing to add is that the heatmap and stacked bar use `░▒▓█` so they survive mono too.
- **Dark/light:** the ramp's lightness ordering must hold in both themes — on a cream background the "bad" colours must be *darker* than text (Few), on soga they must be *brighter*. Test by converting both themes to greyscale; if the three severity steps are not monotonic in L*, the ramp fails for someone.
- Perceptually uniform presets (viridis/magma via `colorgrad`) are for *continuous* quantities such as a 24 h heatmap of a non-severity measure (queries per minute); they must not be reused for severity, or colour stops meaning severity.

### Gaps
- No authoritative source on *which* ramp bottom or k9s ship by default was fetched (bottom docs unreachable; k9s not fetched).
- No study was found on legibility of 24-bit ramps at terminal text sizes; the "stepped, not continuous" rule is argued, not measured.

---

## KQ10. Tufte and Few applied to a text grid; what the ratatui docs say about charts

### Takeaway
Three Tufte/Few principles translate directly to cells: sparklines belong *inside* the text
(word-sized, in table rows — which cobserve already does), small multiples demand one shared
scale, and data-ink ratio in a terminal means deleting borders and axes, not data — every
`│┤┼` that is not a threshold or a boundary between two datasets is chartjunk. Few's bullet
graph is the terminal gauge. ratatui's own docs add the practical limits: Braille and the
Legacy-Computing markers are font-dependent; Sparkline wants a fixed `max`; Chart overlays
Braille on blocks since 0.30.

### Cited Findings
- Tufte: sparklines are "small, high resolution graphics embedded in a context of words, numbers, images"; "graphics are no longer cartoonish special occasions with captions and boxes" — [Tufte, Sparkline theory and practice](https://www.edwardtufte.com/notebook/sparkline-theory-and-practice-edward-tufte/); data-ink ratio and small multiples are his core principles — [Wikipedia: Sparkline](https://en.wikipedia.org/wiki/Sparkline), [Lessons from Edward Tufte](https://www.antoinebuteau.com/lessons-from-edward-tufte.md)
- Few: bullet graphs replace gauges; label + scale + measure + comparison + 2–5 ranges; single-hue intensities — [Bullet Graph Design Spec](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf)
- ratatui docs on markers' font dependence, Sparkline `max`, Braille-over-blocks, `GraphType::Area`/`fill_to_y` — [ratatui Marker](https://docs.rs/ratatui/latest/ratatui/symbols/enum.Marker.html), [ratatui Sparkline](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Sparkline.html), [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)
- btop's 2-samples-per-column packing doubles time resolution for free — [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp)

### Inferences — a checklist for any cobserve chart
1. **Fixed denominator, printed.** Scale to capacity/limit, print `value/denominator %`. (ratatui `Sparkline::max`, `Gauge` ratio.)
2. **Newest at the right edge, window length as one label** (`last 4 min`), no ticks under 40 columns.
3. **Fold by worst-of-slice** (or LTTB for long windows); print the window peak.
4. **Identity by glyph/texture, severity by colour** — never the reverse. Multi-series = area vs line, or `█▓▒░`.
5. **Three symbol rungs** (octant/braille/block/ascii) selectable; Braille default; self-check ambiguous width on start.
6. **Thresholds as ticks on the bar** (bullet), colour flips exactly at the §5.4 thresholds.
7. **Delete borders that are not data**; the only `─` lines are section titles (as today) and the baseline of a chart.
8. **Same scale across multiples**; one legend per screen, in the header.
9. **Heatmaps use 5 shade levels** and colour only above thresholds.
10. **Raster only for artefacts**, never for live screens; test in Apple Terminal and the wry shell, where it will fall back.

### Gaps
- Tufte's and Few's texts were not fetched in full (sparkline page summary and the bullet spec PDF were); page-level quotes on data-ink ratio are from secondary summaries.

---

## Appendix — crates and tools named in this note, with versions as found (2026-10)

| Thing | Version / date | Role for cobserve | Source |
|---|---|---|---|
| ratatui | 0.30.2 (2026-09-10) | Chart (Braille/Octant/Area), Sparkline (absent values), BarChart grouped/horizontal, Gauge/LineGauge, Canvas, Calendar, Fill | [docs.rs](https://docs.rs/ratatui/latest/ratatui/widgets/index.html), [CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md) |
| tui-widgets / tui-bar-graph | 0.3.6 (2026-09-24) | Braille/octant bar graphs with colorgrad gradients; big-text, popup, scrollview | [docs.rs](https://docs.rs/tui-bar-graph) |
| trazo | 0.0.1 (2026-08-10), ratatui ^0.30 | stacked/grouped bars, heatmaps, area, date axes — watch, too young | [docs.rs](https://docs.rs/trazo) |
| tui-piechart | 1.0.3 (2026-10-10), ratatui ^0.30 | pies — not recommended under colour=severity | [docs.rs](https://docs.rs/tui-piechart) |
| ratatui-image | 11.1.0 (2026-09-17), ratatui ≥ 0.30.1 | Kitty/iTerm2/Sixel/half-block images; artefacts only | [docs.rs](https://docs.rs/ratatui-image/latest/ratatui_image/), [README matrix](https://github.com/ratatui/ratatui-image) |
| flamelens | 0.4.0, ratatui 0.29 | flame graph renderer to port (chdig uses it) | [GitHub](https://github.com/YS-L/flamelens) |
| chdig | 26.4.3 (Homebrew) | ClickHouse TUI with live flamegraphs; reference, not dependency | [GitHub](https://github.com/azat/chdig) |
| btop | main | graph packing + gradient technique; `graph_symbol` three rungs | [GitHub](https://github.com/aristocratos/btop) |
| lnav | 0.13.x docs | severity colours, HIST stacked bars, SPECTRO heatmap | [docs](https://docs.lnav.org/en/stable/ui.html) |
| flowrs | — | only Airflow TUI found; no timeline | [GitHub](https://github.com/jvanbuel/flowrs) |
| diskbloom / diskonaut / darya / treemonger | — | terminal treemaps with aspect correction | [diskbloom](https://github.com/Zingzy/diskbloom) |
| dust | — | nested-share bars with shade for parentage | [GitHub](https://github.com/bootandy/dust) |
| sampler | 1.0.2 | YAML dashboard: runchart/sparkline/barchart/gauge | [README](https://raw.githubusercontent.com/sqshq/sampler/master/README.md) |
| trippy | — | per-hop RTT history + frequency charts in ratatui | [features](https://trippy.cli.rs/start/features) |
| Cascadia Code | 2404.23 | first mainstream font with all 256 octants | [blog](https://devblogs.microsoft.com/commandline/cascadia-code-2404-23/) |
| Ghostty | 2025 commits | draws octants/sextants from a built-in sprite font; Kitty graphics | [Face.zig mirror](https://git.uoc.run.place/Applied-Software/ghostty/src/commit/2a29f71b2cf6f4db0d18da9fe2fb75564f3709ea/src/font/sprite/Face.zig) |
| plotters-ratatui-backend | **not found** | — | searched docs.rs/crates.io twice |
