# Design language of modern TUIs (2024–2026) and the ratatui 0.30 ecosystem that reproduces it

Researcher notes for the cobserve redesign. Context read first: `00-context.md` (cobserve today, the
owner's hard rules: UI never blocks on network, never panics on resize, colour = severity only,
every percentage prints its denominator, every number traces to DESIGN.md §5, read-only on
ClickHouse; target 120×36, works at 80×24, richer at 160×48) and the current screens in
`docs/screenshots/*.txt` (nodes tree, map tiles, queue, airflow).

Dates: all crate versions and "updated" dates below were read from the crates.io API on
2026-10-13. Anything marked **(stale?)** is older than ~12 months or could not be re-verified.

---

## KQ1 — Exemplar TUIs: what each one does well, device by device

### Takeaway
The apps that read as "modern" in 2024–2026 share a small vocabulary: one border language (usually
rounded, often *no* borders on secondary regions), a persistent bottom key-hint bar that changes
with focus, a command palette or `:` command mode, which-key style popups for chords, a
list→detail (or Miller-column) drill-down, sparklines/braille graphs instead of numbers-only, and a
named theme system with semantic colour roles (primary/accent/surface/panel/success/warning/error).
Nerd Font icons are always *optional*; plain Unicode fallbacks are the norm.

### Cited Findings

**Monitors (btop, bottom)**

- btop lays the screen out as boxes — "Available values are 'cpu mem net proc' and 'gpu0' through
  'gpu5'" — with nine layout **presets** in the form `box_name:P:G` (position + graph symbol),
  three graph styles (**braille** = highest resolution, **block**, **tty** = ASCII-only), a
  `rounded_corners` option (disabled in TTY mode), `theme_background` (transparent background),
  `truecolor` with "256 color conversion" fallback, `terminal_sync` ("flicker reduction through
  synchronized output"), `proc_tree` (hierarchical process view), `proc_colors` (CPU-gradient
  colouring of process rows), on-screen menu with key hints, mouse: "all buttons with a highlighted
  key are clickable and mouse scrolling works in process list and menu boxes"; Enter opens a
  detailed process panel. Themes are external `.theme` files compatible with bashtop/bpytop —
  [btop README](https://github.com/aristocratos/btop)
- bottom (ratatui) has a **customisable grid layout** in TOML: `[[row]]` → `[[row.child]]` →
  `[[row.child.child]]`, each with `ratio` (default 1); widget types `cpu, mem, net, proc, temp,
  temp_graph, disk, disk_io_graph, batt, empty`; widgets can be duplicated —
  [bottom layout docs](https://clementtsang.github.io/bottom/nightly/configuration/config-file/layout/)
- bottom: "htop-inspired basic mode" vs default dashboard; `e` expands one widget to fill the
  screen; braille or dot-marker graphs; themes gruvbox, nord, default-light; mouse support; `?`
  help — [bottom README](https://github.com/ClementTsang/bottom)

  ```
  btop/bottom "overview → drill-down" (zoom one box with `e`):
  ┌ cpu ───────────────────────┐┌ mem ─────────┐
  │ ⣿⣿⣷⣄⣀⣀⣠⣾⣿⣿⣿⣷⣄   43%     ││ ▇▇▆▅  61%   │
  └────────────────────────────┘└──────────────┘
  ┌ net ────────────┐┌ proc ────────────────────┐
  │ ⢀⣀⣤⣶⣿ ↓1.2M    ││ pid  name   cpu  mem  ▲  │
  └─────────────────┘└──────────────────────────┘
  ```

**Kubernetes / containers (k9s, lazydocker)**

- k9s is a **command-driven modal interface**: `:` opens command mode, "singular, plural,
  short-name or alias" resolves a resource (`:pod`/`:pods`); a **crumbs bar** shows navigation
  history, `Esc` goes back, `[`/`]` jump through command history "like how 'cd -' works";
  per-resource drill-down with single keys (`y` YAML, `l` logs, `d` describe); `f` fullscreen for
  logs/YAML/describe; `/filter⏎` regex, `/-l` label selector, `/-f` fuzzy; `?` shows active
  mnemonics, `Ctrl-A` all aliases; `Ctrl-W` toggles wide columns, `Ctrl-E` toggles the header
  ("headless"); `:pulses` is a "monitoring dashboard showing cluster health", `:xray` a dependency
  tree; YAML **skins** per context, colours by name or hex; hotkeys.yaml maps e.g. Shift-0 to a
  view; plugins add key-triggered commands with `$NAME`/`$NAMESPACE` substitution —
  [k9s README](https://github.com/derailed/k9s)

  ```
  k9s model: header (context + logo + key menu) / crumbs / resource table / drill-down:
   Context: prod   Cluster: pay   User: abdul    <0> all  <1> default   ?help  :cmd
   ───────────────────────────────────────────────────────────────────────────────
    pods(default)[42]                                                   ← crumbs
    NAME↑          READY  STATUS   RESTARTS  CPU  MEM   AGE
    ch-0           1/1    Running  0         12   3400  4d
   ───────────────────────────────────────────────────────────────────────────────
    <pods> <deploy> <ch-0>                                             ← history
  ```

- lazydocker: left column Project / Containers / Images / Volumes / Networks, right panel with
  tabs logs / stats / config / top / env; "ascii graphs of your containers' metrics"; "one keypress
  away (or one click away!)" with mouse support and on-screen key hints; custom commands; built on
  gocui — [lazydocker README](https://github.com/jesseduffield/lazydocker)

**Git (lazygit, gitui)**

- lazygit layout: left stack Status / Files / Branches / Commits / Stash, right main panel; panels
  reached by number keys, Tab, `h`/`l`; bottom key-hint line; `?` full keybindings; screen modes
  normal / half / full cycled with `+`/`_` — [lazygit README](https://github.com/jesseduffield/lazygit)
- lazygit gui config (defaults): `sidePanelWidth: 0.3333`; `expandFocusedSidePanel: false` ("increase
  the height of the focused side window; creating an accordion effect"); `mainPanelSplitMode:
  flexible` (horizontal|vertical|flexible); `border: rounded` (rounded|single|double|hidden|bold);
  `nerdFontsVersion: ""` ("2"|"3"); `showFileTree: true`; `showBottomLine: true` ("the bottom line
  that contains keybinding info and useful buttons"); `showPanelJumps: true` ("show jump-to-window
  keybindings in window titles"); `showCommandLog: true`; `screenMode: normal`; `statusPanelView:
  dashboard`; `filterMode: substring|fuzzy`; `spinner.rate: 180` ms; theme keys
  `activeBorderColor`, `inactiveBorderColor`, `selectedLineBgColor` —
  [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)

  ```
  lazygit: stacked side panels + one main panel; focused panel border coloured; jump keys in titles
  ╭─[1] Status ─────────╮╭─ Diff ────────────────────────────────────╮
  │ repo → main ↑1      ││ @@ -1,4 +1,6 @@                           │
  ╭─[2] Files ──────────╮│ -old                                      │
  │ M src/ui/nodes.rs   ││ +new                                      │
  ╭─[3] Branches ───────╮│                                           │
  │ * main              ││                                           │
  ╭─[4] Commits ────────╮│                                           │
  │ 6035574 Merge #36   ││                                           │
  ╰─────────────────────╯╰───────────────────────────────────────────╯
   ↑↓ navigate  ⏎ open  ? keys  + screen mode                       ← bottom line
  ```

- gitui (ratatui): tabs status / log / files / stashing / stashes; "context based help" so users
  "don't need to memorize extensive keyboard shortcuts"; key-hint bar at the bottom; `.ron` theme
  files; adapts to dark and light backgrounds; vim-key config; benchmark on the Linux repo: 24 s /
  0.17 GB vs lazygit 57 s / 2.6 GB — [gitui README](https://github.com/gitui-org/gitui)

**File managers (yazi, superfile)**

- yazi (ratatui) uses **Miller columns** via `ratio = [1, 4, 3]` → "1/8 width for parent, 4/8
  width for current, 3/8 width for preview"; a panel can be hidden with ratio 0; `linemode`
  (size/mtime/permissions/owner on the row); image preview with scaling filters (nearest, triangle,
  Catmull-Rom, Lanczos3); a `[which]` section configures the which-key popup (candidate sorting by
  key / description / reverse) — [yazi.toml docs](https://yazi-rs.github.io/docs/configuration/yazi)
- yazi image protocols, in detection priority order: Kitty unicode placeholders (Kitty ≥ 0.28,
  Ghostty, Windows Terminal ≥ 1.22), Kitty old protocol (Konsole), iTerm2 inline images (iTerm2,
  WezTerm, Warp, Tabby, VSCode, Rio, Bobcat), Sixel (foot, Windows Terminal, st+patch, Black Box),
  Überzug++ (X11/Wayland), Chafa ASCII fallback. "Zellij's KGP implementation currently has a bug
  that prevents image previews from working correctly"; tmux needs three options and
  `--enable-sixel` — [yazi image preview](https://yazi-rs.github.io/docs/image-preview)

  ```
  Miller columns (yazi): parent │ current │ preview, ratio 1:4:3
  ╭─ ~ ──────╮╭─ ~/code ───────────────────────╮╭─ cobserve/ ────────────╮
  │ code  ▸  ││ ▸ cobserve        12 items    ││  src/  docs/  DESIGN.md │
  │ work     ││   fleetlens       3d ago      ││  README.md …            │
  ╰──────────╯╰────────────────────────────────╯╰────────────────────────╯
   ~/code/cobserve   3/12   ↑↓ move  l open  h back  : cmd            ← status bar
  ```

- superfile (Go, Bubble Tea): "Every pixel obsessively crafted. superfile looks as good as your
  favorite apps—but lives entirely in your terminal"; multiple file panels, preview, "adjustable
  borders and color schemes", "20+ built-in themes including Catppuccin, Nord, Tokyo Night,
  Dracula, Gruvbox, and Rose Pine", remappable hotkeys, plugins — [superfile.dev](https://superfile.dev/)

**Finders and pickers (television, atuin)**

- television (ratatui): input + results list + preview pane; "channels" (TOML cable files with
  source, preview command, keybindings); editor plugins for Neovim/Vim/VSCode/Zed/JetBrains —
  [television README](https://github.com/alexpasmantier/television)
- television `[ui]` config: `ui_scale` ("How much space to allocate for the UI (in percentage of
  the screen)"), `orientation` landscape|portrait, `theme`; `[ui.input_bar] position` top|bottom,
  `prompt`, `border_type` **none|plain|rounded|thick** (ratatui BorderType), `padding`;
  `[ui.status_bar] hidden`; `[ui.results_panel] border_type, padding`; `[ui.preview_panel] size,
  scrollbar, border_type, hidden`; `[ui.help_panel] show_categories`; feature toggles preview
  ctrl-o, help ctrl-h, remote control ctrl-t, status bar F12, layout ctrl-l —
  [television default config.toml](https://github.com/alexpasmantier/television/blob/main/.config/config.toml)
- atuin TUI options: `style` "which display style to use" auto|full|compact; `inline_height` "the
  maximum number of lines the interface should take up; set it to 0 to always go full screen";
  `invert` "put the search bar at the top"; `show_preview`, `max_preview_height`; `show_help`
  (help row incl. version/update notice); `show_tabs` (search / inspect); `keymap_mode` auto/vim;
  `enter_accept: true`; `prefers_reduced_motion` "Atuin will minimize motion in the UI - timers
  will not update live, etc."; `[theme] name` built-ins "default", "autumn", "marine" —
  [atuin config.toml](https://github.com/atuinsh/atuin/blob/main/crates/atuin-client/config.toml)

**API / SQL clients (posting, Harlequin, rainfrog)**

- posting: three panels — left collection sidebar (folders), centre request editor, right response
  viewer; request tabs Auth/Headers/Body/Path/Query/Options; response tabs Body/Headers/Cookies/
  Trace; **jump mode** `Ctrl+O` ("Labels appear across the screen; users press corresponding
  letters" — e.g. "press W to jump to the request's Body tab"); **command palette** `Ctrl+P`
  (incl. theme and layout switching); footer shows "the most useful keys for whatever is focused";
  `Ctrl+N` new tab, `Ctrl+G` search collection, `F1` shortcuts — [posting guide](https://posting.sh/guide/)
- posting theme roles: Primary "the main colour: GET labels, JSON keys, dialog borders"; Secondary
  "smaller highlights"; Accent "used to show what has focus"; Background "the screen's background";
  Surface "inputs, text areas and tables"; Panel "raised surfaces, a step above `surface`"; Text;
  Success "successful responses, variables that have a value"; Warning "redirects, unsaved
  changes"; Error. 37 built-in themes (29 dark incl. galaxy, dracula, nord, gruvbox; 8 light incl.
  catppuccin-latte, rose-pine-dawn); custom YAML themes need only `name` and `primary`, the rest
  inherits; live preview in the picker; syntax colours derive from the roles —
  [posting themes](https://posting.sh/guide/themes/)
- posting 3 "is built with Terma" (Posting 2 was Textual); "37 built-in themes, plus user-defined
  themes"; requests stored as YAML "so they're easy to read and version control" —
  [posting README](https://github.com/darrenburns/posting)
- Harlequin (Textual) "has 5 main components: The Data Catalog on the left-hand pane … in an
  interactive tree, the Query Editor as the top pane on the right side … and the Results Viewer
  under the Query Editor"; "press F10 to view the Editor or Results in full-screen mode"; results
  viewer handles "1M+ results in an interactive table"; F1 in-app help; theme + keymap config
  profiles — [harlequin.sh usage](https://harlequin.sh/docs/getting-started/usage) (via search
  snippet; the page itself rate-limited), [Harlequin README](https://github.com/tconbeer/harlequin)
- rainfrog (ratatui): sections menu (schemas/tables), editor, results, history, favorites; focus
  via `Alt+1..5` or `Ctrl+k/j/h/g/m`, Tab/Shift+Tab cycle, `Alt+-`/`Alt++` resize sections;
  vim-like editor (hjkl, v/V, yank, undo, gg/G); results: `v` field, `V` row, PageUp/Down; mouse
  for focus + scroll but "the editor doesn't support mouse selection"; `Alt+e` external editor,
  `P` CSV export; inline SQL completions with debounce; macOS needs "use option as meta key" —
  [rainfrog README](https://github.com/achristmascarl/rainfrog)

**Multiplexers and editors (zellij, Helix)**

- zellij options: `pane_frames` (default true), `pane_frame_style` **titles** ("single line above
  panes", default) | **full** | **none**; `rounded_corners`; `hide_session_name`;
  `simplified_ui` ("without arrow fonts", i.e. no powerline glyphs); `default_layout` default|
  compact; `theme`, `theme_dark`/`theme_light` switch on terminal background; `styled_underlines`;
  `mouse_mode`, `mouse_hover_effects` ("such as pane frame highlight"), `mouse_hover_tips`;
  `stacked_resize`; `show_startup_tips` — [zellij options](https://zellij.dev/documentation/options.html)
- zellij README: "floating and stacked panes", modal keybindings (pane/tab/resize/scroll/session
  modes) with mode-specific hints, KDL layouts, WASM plugins, session manager —
  [zellij README](https://github.com/zellij-org/zellij)

  ```
  zellij status bar: mode indicator + which-key hints that change per mode
   Zellij (cobserve)  Tab #1  Tab #2
   ─────────────────────────────────────────────────────────────────────
   <Ctrl g> LOCK  <p> PANE  <t> TAB  <n> RESIZE  <h> MOVE  <s> SCROLL  <o> SESSION  <q> QUIT
   PANE: <←↓↑→> move focus  <n> new  <d> down  <r> right  <x> close  <f> fullscreen  <w> floating
  ```

- Helix: statusline `left = ["mode", "spinner", "file-name", "read-only-indicator",
  "file-modification-indicator"]`, `right = ["diagnostics", "selections", "register", "position",
  "file-encoding"]`, `separator = "│"`; `bufferline` always|never|multiple (default never);
  `popup-border` none|popup|menu|all (default **none**); `cursorline` false; `color-modes` false;
  `auto-info: true` "Whether to display info boxes"; `idle-timeout 250` ms; `true-color` and
  `undercurl` override detection; `[editor.lsp] display-messages` "below statusline";
  `[editor.inline-diagnostics]`; mode texts NOR / INS / SEL —
  [Helix editor config](https://docs.helix-editor.com/editor.html)
- Helix minor modes: space mode ("a kludge of mappings, mostly pickers": `f` file picker, `b`
  buffer picker, `s` symbols, `d` diagnostics, `?` command palette), goto `g`, view `z`/`Z`, window
  `Ctrl-w`, match `m`; they "typically switch back to normal mode after a command" —
  [Helix keymap](https://docs.helix-editor.com/keymap.html)

  ```
  which-key / Helix infobox after a prefix key (auto-info):
                                        ╭─ space ─────────────────────╮
                                        │ f  file picker               │
                                        │ b  buffer picker             │
                                        │ s  symbol picker             │
                                        │ d  diagnostics               │
                                        │ ?  command palette           │
                                        ╰──────────────────────────────╯
   NOR  main.rs ●              ⚠ 2  1 sel  12:4  utf-8          ← statusline
  ```

**AI coding agents (Crush, OpenCode, Claude Code)**

- Crush: "Session management across projects", "Multi-window functionality including chat and
  file contexts", themes "Charmtone Panther (default) and Gruvbox Dark" customisable live,
  "Command palette accessible via Ctrl+P", "Permission dialogs for tool execution control",
  "compact and standard modes" — [Crush README](https://github.com/charmbracelet/crush)
- Crush "is built on bubbletea v2, lipgloss v2, bubbles v2, glamour v2, and ultraviolet, with 10+
  composed components, real-time LLM streaming, dialog overlays, and a full agent loop" — [agent-skills
  charm-crush SKILL.md (third party)](https://git.secluded.site/agent-skills/blob/main/vendor/forge/skills/charm-crush/SKILL.md?raw=1);
  "Charm understands terminal product design better than most teams shipping agent tools into the
  shell" — [The New Stack review (search snippet; article body not retrievable)](https://thenewstack.io/terminal-user-interfaces-review-of-crush-ex-opencode-al)
- Bubble Tea (v2 README): "a high-performance cell-based renderer, built-in color downsampling,
  declarative views, high-fidelity keyboard and mouse handling, native clipboard support" —
  [bubbletea README](https://github.com/charmbracelet/bubbletea)
- Lip Gloss borders: Normal, Rounded, Thick, Double, Hidden, Block, half-block variants, ASCII
  (markdown tables); per-side borders; border fg/bg incl. gradients across several colours; CSS-like
  padding/margin shorthand; `LightDark` adaptive colours; `Complete` colour per profile (ANSI16 /
  ANSI256 / TrueColor / ASCII); Table / List / Tree sub-packages (tree with rounded connectors) —
  [lipgloss README](https://github.com/charmbracelet/lipgloss)
- OpenCode TUI: slash commands (`/connect`, `/compact`, `/editor`, `/export`, `/models`, `/new`,
  `/sessions`, `/share`, `/themes`, `/undo`, `/redo`, `/thinking`); leader key `ctrl+x` with
  `leader_timeout` 2000 ms; `tui.json` with `scroll_speed`, `diff_style` auto|stacked, `cursor`,
  `mouse`, `attention` (desktop notifications + sounds) — [opencode.ai/docs/tui](https://opencode.ai/docs/tui/)
- OpenTUI (OpenCode's stack): "You arrange boxes and text with flexbox. You add selects, inputs,
  and scroll boxes with keyboard and mouse controls. You can play sounds, show images, and render
  3D graphics"; Zig core + TypeScript, React and Solid renderers, Yoga flexbox; "OpenCode uses
  OpenTUI in production for millions of users" — [OpenTUI README](https://github.com/sst/opentui)
- Claude Code renders with a custom React/Ink: "custom implementation of the React-based Ink
  framework (over 4,100 lines)", Yoga flexbox, "differential rendering", "~16ms frame budget" —
  [wikidocs Part 12 (secondary source)](https://wikidocs.net/342647); the extracted renderer
  `ink-terminal` lists: double buffering, "Hardware Scrolling: Uses DECSTBM terminal-side row
  shifting", "Damage-Tracked Diffing", "Atomic Output: DEC 2026 synchronized updates prevent visual
  tearing", virtual lists (1000+ items), streaming markdown renderer, bracketed paste, terminal
  focus detection, text selection, FPS counter, spinners, progress bars, terminal hyperlinks,
  "tab status indicators (iTerm2, Kitty)" — [ink-terminal README](https://cdn.jsdelivr.net/npm/ink-terminal@0.1.0-alpha.1/README.md)

**Ratatui showcase (what the project itself puts forward)**

- 33 apps incl. bottom, bandwhich, trippy, dua, yazi, joshuto, xplr, gitui, slumber, csvlens,
  rainfrog, yozefu, atuin, taskwarrior-tui, television, codex, oatmeal, steer, gpg-tui, binsider,
  oha, oxker, mdfried ("Markdown viewer rendering headers with larger text"), material —
  [ratatui.rs/showcase/apps](https://ratatui.rs/showcase/apps/)

### Inferences
- Across every exemplar the *discoverability stack* is identical and layered: (1) a one-line
  key-hint bar that re-renders per focused region (lazygit, gitui, posting, Helix statusline,
  zellij), (2) `?`/F1 full help, (3) a searchable command palette (posting, Crush, Helix `space ?`,
  k9s `:`), (4) which-key popups for chords (yazi, Helix, zellij). cobserve already has (1) and
  (2); (3) and (4) are the missing layers.
- "Modern" chrome in 2024–2026 = rounded corners *or* no frame at all, with the focused region
  signalled by border colour (lazygit `activeBorderColor`) or a title line only (zellij
  `pane_frame_style: titles`, Helix `popup-border: none`). Full boxes everywhere (btop) now read as
  "classic monitor", not modern.
- Jump mode (posting) and `showPanelJumps` (lazygit) solve the same problem cobserve has with many
  regions: label every region with a one-key jump target.
- Themes are *semantic roles*, not palettes: posting/Textual's primary/accent/surface/panel/
  success/warning/error maps cleanly onto cobserve's Tarum (indigo = primary/accent, soga/mori =
  background/foreground, red/amber/green = severity-only).

### Gaps
- Crush's actual sidebar layout and dialog chrome could not be read (README is marketing-level;
  the New Stack article body did not come through). Only the stack (bubbletea v2 + lipgloss v2 +
  ultraviolet) and the Ctrl+P palette / permission dialogs are confirmed.
- k9s `:pulses` layout (tile grid with sparklines) is only confirmed as "monitoring dashboard
  showing cluster health"; the docs page 404'd.
- Claude Code's TUI specifics come from a secondary source and the ink-terminal README, not from
  Anthropic documentation.
- Harlequin's command palette key and footer design were not readable (429); only the 3-pane layout
  and F10 full-screen are sourced.

---

## KQ2 — Layout concepts and which fit a fleet monitor (9–40 nodes + queues + DAGs + tickets + PTYs)

### Takeaway
Six reusable layout concepts recur: overview→zoom (btop/bottom `e`), resource list + drill-down
with crumbs (k9s), stacked side panels + main (lazygit/lazydocker), Miller columns (yazi), picker
with preview (television/atuin), and modal/floating overlays with a which-key status bar (zellij/
Helix). For cobserve the natural fit is **k9s's list→detail with crumbs** for the fleet (nodes →
node → user → query), **lazygit's stacked-side + inspector** at 160×48, **btop's zoom** for any
panel, and a **persistent command bar** (`:`/`Ctrl+P`) as the single navigation entry point.

### Cited Findings
- Overview→zoom: bottom's `e` "to enlarge a specific widget" and "basic mode" —
  [bottom README](https://github.com/ClementTsang/bottom); k9s `f` fullscreen for logs/YAML/describe
  — [k9s README](https://github.com/derailed/k9s); Harlequin F10 full-screen editor/results —
  [harlequin.sh](https://harlequin.sh/docs/getting-started/usage)
- List + detail with history: k9s crumbs, `Esc` back, `[`/`]` history — [k9s README](https://github.com/derailed/k9s)
- Stacked side + main, accordion focus: lazygit `expandFocusedSidePanel` ("accordion effect"),
  `mainPanelSplitMode: flexible`, `screenMode normal|half|full` — [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- Miller columns with hideable panes: yazi `ratio`, pane hidden with 0 — [yazi docs](https://yazi-rs.github.io/docs/configuration/yazi)
- Picker + preview with configurable orientation and `ui_scale`: television — [config.toml](https://github.com/alexpasmantier/television/blob/main/.config/config.toml);
  atuin `inline_height` (non-fullscreen inline UI) and `invert` — [atuin config](https://github.com/atuinsh/atuin/blob/main/crates/atuin-client/config.toml)
- Floating/stacked panes + modal hint bar: zellij — [zellij README](https://github.com/zellij-org/zellij)
- Grid of tiles declared as rows/children/ratios: bottom — [layout docs](https://clementtsang.github.io/bottom/nightly/configuration/config-file/layout/)
- Tabs as top-level sections with per-tab panels: gitui (status/log/files/stashing/stashes) —
  [gitui README](https://github.com/gitui-org/gitui); posting request/response tabs — [posting guide](https://posting.sh/guide/)
- Ratatui layout primitives that implement all of the above: `Constraint::{Length, Percentage,
  Ratio, Min, Max, Fill}`, `Flex::{Legacy, Start, End, Center, SpaceBetween, SpaceAround,
  SpaceEvenly}`, `.spacing(u16)`, nesting — [ratatui layout concepts](https://ratatui.rs/concepts/layout/);
  0.30 adds `Rect::layout()`, `Rect::centered(Constraint, Constraint)`, `centered_horizontally/
  vertically`, `Rect::outer()` — [ratatui 0.30 highlights](https://ratatui.rs/highlights/v030/)
- Popups: `frame.render_widget(Clear, area); frame.render_widget(popup, area)` with
  `frame.area().centered(...)` — [center a widget recipe](https://ratatui.rs/recipes/layout/center-a-widget/)

### Inferences (design sketches for cobserve, derived from the above)

1. **Shell = k9s + lazygit hybrid.** One masthead line (context + clock + live dot), a *crumbs
   line* that doubles as the command bar (`:` or `Ctrl+P` turns the crumbs into an input), a main
   region, an inspector that is a right column at ≥140 cols and a bottom drawer at 120×36, and a
   one-line key-hint bar. The tab digits 1–0 stay as k9s-style hotkeys, but the *primary* way in is
   `:nodes`, `:queue`, `:dag gateway_*`, `:ticket DATA-8743`, `:sql clickhouse3`.

   ```
   120×36 — fleet view, inspector as bottom drawer
    ❖ cobserve  prod  ● live 15:52 WIB                            ━ Dzuhur 11:45 ─●─ Ashar 14:48 ━
    fleet › nodes › clickhouse3                                    :  type a command   ?  help
    ─────────────────────────────────────────────────────────────────────────────────────────────
    NODE            MEM ──────────── GiB   CPU ─────────── cores  Q   LONGEST   4 min
    ▌clickhouse3    93% ━━━━━━━━━━━╸ 59.6/64  93% ━━━━━━━━━━╸ 14.9/16  4 ✕2  8m31s  ▂▃▄▅▆▇▇▃
     clickhouse-bi  67% ━━━━━━━╸━━━━ 43/64    72% ━━━━━━━╺━━━ 11.5/16  3 ✕1  6m40s  ▆▆▃▅▆▆▄▅
     4 healthy folded · ch4 ch6 ch8 ch9 · all ≤ 22%                                       ␣
    ─ clickhouse3 · pay-ch-node-1:9000 ───────────────────────────────── inspector ─ f zoom ──
     r_redash → grigol.gankava   16.3 GiB 25.5%   3.1 cores   8m31s ✕   #7438 Gateway transfers
     ▎ SELECT toStartOfHour(ts) … FROM open_banking.ais WHERE ts >= now() - INTERVAL 6 HOUR
    ─────────────────────────────────────────────────────────────────────────────────────────────
     ↑↓ move  ⏎ drill  ⌫ back  f zoom  : command  / filter  s sort  tab inspector
   ```

   ```
   160×48 — same data, lazygit-style right inspector + stacked side summary
   ╭─[1] fleet ───────────╮  NODE · USER → PERSON        MEM            CPU        ╭─ clickhouse3 ────────╮
   │ 9 nodes · 1 hot      │ ▌▾ clickhouse3   93% ━━━━━━╸ 59.6/64  93% ━━━╸ 14.9/16 │ MEM 59.6/64  ▂▃▄▅▆▇▇▃ │
   │ mem 36% 280/768 GiB  │   ├ r_redash → grigol   16.3 25%  3.1c  8m31s ✕       │ CPU 14.9/16  ▇▇▇▇▇▆▇▇ │
   ╭─[2] redash ──────────╮   ├ airflow             3.5   5%  5.0c    18s          │ free 4.4 GiB = 0.2×   │
   │ 16 waiting · 6/7 busy│   └ server · caches     30.3 47%  5.8c                 │ ─ SQL ──────────────  │
   ╭─[3] airflow ─────────╮  ▸ clickhouse-bi   67% …                              │ SELECT …              │
   │ 2 running · 3 failed │                                                        │                       │
   ╭─[4] jira ────────────╮                                                        │ ─ insights ─────────  │
   │ 5 open · 1 overdue   │                                                        │ ✖ memory 93%          │
   ╰──────────────────────╯                                                        ╰───────────────────────╯
   ```

2. **Zoom everything** (btop/bottom/k9s/Harlequin): `f` on any region makes it the whole main area
   with the crumbs updated (`fleet › nodes › clickhouse3 › sql`). This is how 80×24 stays usable:
   the small terminal simply *is* always zoomed to one region.

3. **Tiles (MAP) stay, but as k9s-pulses-style cards**: card = name + two LineGauges + a Sparkline
   + a status glyph; Enter drills into the list view with the node pre-selected (already the case).
   bottom's rows/children/ratio grammar is a good model if the owner wants user-arrangeable tiles.

4. **Queues, DAGs, tickets are "resources"** in the k9s sense: each gets a table view with the same
   column grammar, the same `/` filter, the same inspector, the same crumbs. One mental model
   instead of six bespoke screens.

5. **PTY sessions = zellij-style stacked/floating panes** inside view 7+: a floating session over
   the monitor (`Ctrl+\``?) is more useful than a separate tab when watching a node while writing
   SQL. tui-term already provides the widget (see KQ4).

6. **Picker + preview** (television/atuin) for every "choose one" interaction: pick a node, pick a
   DAG, pick a ticket, pick a saved query — one widget, fuzzy input on top, preview on the right.

### Gaps
- No published usability study comparing list+detail vs tiles for fleet monitors was found; the
  recommendation rests on the convergent practice of k9s/lazygit/bottom, not on measured evidence.

---

## KQ3 — ratatui 0.30: what is new and what it enables

### Takeaway
0.30 (current 0.30.2, 2026-06-19 on crates.io) is the modular release: `ratatui-core` 0.1.2 /
`ratatui-widgets` 0.3.2 / `ratatui-macros` 0.7.2 / `ratatui-crossterm` 0.1.2, Rust 2024 edition,
MSRV 1.86. For a redesign the relevant additions are border merging (`MergeStrategy`), dashed and
quadrant `BorderType`s, `Block::shadow()`, `Rect::centered/layout/outer`, `Flex::SpaceEvenly`,
Canvas `Quadrant/Sextant/Octant` markers, `LineGauge` custom symbols, `Text += Text`, and
`ratatui::run()`.

### Cited Findings
- Workspace split: "ratatui-core: Fundamental types, traits, and utilities with slower evolution;
  ratatui-widgets: Widget implementations; ratatui-crossterm, ratatui-termion, ratatui-termwiz;
  ratatui-macros"; widget authors should depend on `ratatui_core::{widgets::{Widget,
  StatefulWidget}, buffer::Buffer, layout::Rect}` — [0.30 highlights](https://ratatui.rs/highlights/v030/)
- `no_std` with `default-features = false` (ESP32, STM32H7, UEFI) — same source
- `ratatui::run(|terminal| { … })` wraps init/restore — same source
- Block: "Border merging: `Cell::new("┘").merge_symbol("┏", MergeStrategy::Exact)`", "New
  `BorderType` variants with dashed patterns", "Removed `Block::title()` in favor of `Line`-based
  approach" (`title_top`/`title_bottom`, `Line::centered()` etc.), `BlockExt` exported — same source
- Block docs 0.30.2: `title_top()`/`title_bottom()`, "multiple titles … rendered with spaces
  separating titles that share the same position or alignment", `Padding` (uniform / horizontal /
  vertical / directional), `merge_borders(MergeStrategy)` "collapses overlapping borders for
  seamless layouts", `shadow()` "Adds background shadow effects", `inner(area)` —
  [Block docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Block.html)
- BorderType variants 0.30.2: Plain `┌─┐│└┘`, Rounded `╭─╮│╰╯`, Double `╔═╗║╚╝`, Thick `┏━┓┃┗┛`,
  Light/Heavy Double/Triple/Quadruple-Dashed (`╌ ╍ ┄ ┅ ┈ ┉`), QuadrantInside `▗▄▄▖▐ ▌▝▀▀▘`,
  QuadrantOutside `▛▀▀▜▌ ▐▙▄▄▟` — [BorderType docs](https://docs.rs/ratatui/latest/ratatui/widgets/enum.BorderType.html)
- Canvas markers 0.30.2: Dot, Block, Bar (1×1), Braille (2×4), HalfBlock (2×2), Quadrant (2×2),
  Sextant (2×3), Octant (2×4, "same 2x4 resolution as Braille characters but display densely
  packed and regularly spaced pseudo-pixels"); Braille/Sextant/Octant "may display replacement
  characters in unsupported terminals" — [Marker docs](https://docs.rs/ratatui/latest/ratatui/symbols/enum.Marker.html);
  "Braille characters render over block symbols in charts" — [0.30 highlights](https://ratatui.rs/highlights/v030/)
- LineGauge: `filled_symbol()` / `unfilled_symbol()`; BarChart: `BarChart::new`, `Bar::new`,
  `BarGroup::new`, labels `Into<Line>`; Scrollbar `ScrollbarState::get_position()`; List
  `highlight_symbol` `Into<Line>`; Tabs `width()`; `RatatuiMascot` widget — same source
- Table 0.30.2: widths as `Constraint`, `column_spacing`, `flex`, header/footer rows, row heights,
  `row_highlight_style` / `column_highlight_style` / `cell_highlight_style` (applied Row → Column →
  Cell), `highlight_symbol`, `highlight_spacing` Always|WhenSelected|Never, `TableState` with
  offset / selected row / column / cell — [Table docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Table.html)
- Layout: `Rect::layout()` (compile-time constraint count), `try_layout`, `layout_vec`,
  `centered(Constraint, Constraint)`, `centered_vertically/horizontally`, `Rect::outer()`,
  `VerticalAlignment`, `Flex::SpaceEvenly` (old SpaceAround) and new `Flex::SpaceAround` (edge
  spacing = ½ between-item spacing), `Alignment` → `HorizontalAlignment`, `Rect::from(size)`;
  layout cache now opt-in `features = ["layout-cache"]` — [0.30 highlights](https://ratatui.rs/highlights/v030/)
- Style: `anstyle` conversions (feature), `Color::from([255,0,0])`, `Styled` for primitives,
  `const MY_STYLE: Style = Style::new().blue().on_black();` — same source
- Text: `Text += Text`, control characters no longer rendered, `UnicodeWidthStr` implemented;
  VS16 emoji cleared properly; `Cell::symbol` is `Option<CompactString>` — same source
- Backend: `FromCrossterm`/`IntoCrossterm` traits instead of blanket `From`; `Backend::Error`
  associated type; required `clear_region()`; feature flags `crossterm_0_28` / `crossterm_0_29`;
  `StatefulWidget::State: ?Sized`; `WidgetRef` blanket reversed; `FrameExt` behind
  `unstable-widget-ref`; MSRV 1.86, Rust 2024 — same source
- Hyperlinks: the official example writes OSC 8 into cell symbols —
  `format!("\x1B]8;;{}\x07{}\x1B]8;;\x07", url, text)` — in **2-character chunks** to dodge
  "a ratatui bug (issue #902) [that] causes incorrect ANSI escape sequence width calculations" —
  [hyperlink example](https://ratatui.rs/examples/apps/hyperlink/); `hyperrat` 0.1.3 packages a
  `Link` widget on `ratatui-core ^0.1.2` — crates.io

### Inferences
- `merge_borders` + dashed `BorderType` + `shadow()` are exactly the devices for "borders that
  don't look like 2015": a card grid whose shared edges merge into a single rule, a dashed
  separator for secondary groups, a shadow under a floating palette.
- Octant/Sextant markers let a 4-minute history sparkline carry 2×4 pixels per cell with
  *regularly spaced* dots (Braille's dots are visually uneven); but they need a 2024+ font
  (Unicode 16 "Symbols for Legacy Computing Supplement"); fall back to Braille → `▁▂▃▄▅▆▇█`.
- `Rect::layout()` with const generics removes most of the `let [a,b,c] = Layout::…areas()`
  boilerplate; `centered()` + `Clear` is the whole popup recipe.

### Gaps
- Whether ratatui's crossterm backend wraps each `draw()` in DEC 2026 synchronized output
  automatically was **not** confirmed; crossterm exposes `BeginSynchronizedUpdate` /
  `EndSynchronizedUpdate` ("instructs the terminal emulator to begin a synchronized frame") —
  [crossterm terminal docs](https://docs.rs/crossterm/latest/crossterm/terminal/index.html) — so
  the app can emit them itself around `terminal.draw`. Needs a check in `ratatui-crossterm` source.

---

## KQ4 — Third-party ratatui crates: what each gives, maturity, pitfalls

### Takeaway
Nearly the whole ecosystem has moved to ratatui 0.30 (checked via crates.io dependency metadata
on 2026-10-13). The one notable laggard is `tui-textarea` 0.7.0 (ratatui ^0.29, last release
2024-10-22); its maintained successor is `ratatui-textarea` 0.9.3 (org-owned). The
`ratatui/tui-widgets` monorepo (popup, scrollview, big-text, prompts, bar-graph, scrollbar, …)
released in lockstep on 2026-09-24.

### Cited Findings
Format: crate — version (updated, downloads) — depends on — what it gives — note. Source for
version/date/deps: crates.io API (`/api/v1/crates/<name>` and `/dependencies`), 2026-10-13.

| crate | version (updated) | ratatui dep | gives | pitfalls / notes |
|---|---|---|---|---|
| **tachyonfx** | 0.25.2 (2026-09-06, 472k dl) | ratatui ^0.30.2 | "shader-like effects": `fade_from/to`, `fade_*_fg`, `hsl_shift`, `paint*`, `lighten/darken/saturate`, `dissolve/coalesce`, `evolve*`, `explode`, `slide_in/out`, `sweep_in/out`, `delay/sleep`, `never_complete/repeating`, `repeat/ping_pong`, `with_duration/freeze_at`, `prolong_start/end`, `translate/expand/stretch`, `parallel/sequence`, `effect_fn/effect_fn_buf`, `dispatch_event`, `offscreen_buffer`, `dynamic_area`; `CellFilter` targets cells (e.g. `CellFilter::FgColor`); `EffectTimer` — [docs.rs fx](https://docs.rs/tachyonfx/latest/tachyonfx/fx/index.html), [README](https://github.com/junkdog/tachyonfx) | Effects are post-processing on the `Buffer`, so they need a redraw per frame while active (a timer tick, not just data ticks). The raw README fetched showed an old 0.7.0 badge — treat README snippets as stale, docs.rs as current. |
| **ratatui-image** | 11.1.0 (2026-10-05, 1.07M dl) | ratatui ^0.30.1, crossterm ^0.29, image ^0.25.6, icy_sixel | Sixel, Kitty, iTerm2, Unicode half-blocks; `Picker` queries the terminal for protocol and font size; `Image` (fixed, non-blocking) vs `StatefulImage` (fits area, needs a resize thread) — [README](https://github.com/benjajaja/ratatui-image) | "Sixel rendering on terminal's final line causes scrolling"; SSH/tmux need care; font-size detection failure falls back to 4:8; weak/unsupported: Alacritty, Konsole, Contour, Warp. |
| **ratzilla** | 0.3.1 (2026-06-08, 514k dl) | ratatui ^0.30.1 | ratatui in the browser via WASM: `DomBackend`, `CanvasBackend`, `WebGl2Backend` (font atlas config); `on_key_event`, `draw_web`; hyperlinks via backend options; mouse events; `SelectionMode`; build with trunk — [docs.rs](https://docs.rs/ratzilla/latest/ratzilla/), [README](https://github.com/orhun/ratzilla) | No crossterm events / stdin; the event model differs (callbacks, not a poll loop), so the app core must be split from the terminal loop. FOSDEM 2025 talk "Bringing terminal aesthetics to the Web with Rust (and vice versa)" — [FOSDEM](https://archive.fosdem.org/2025/schedule/speaker/orhun_parmaksiz/) |
| **tui-term** | 0.3.4 (2026-04-07, 1.39M dl) | ratatui ^0.30, portable-pty ^0.9, vt100 ^0.16.2 (opt) | `PseudoTerminal` widget drawing a vt100 screen — crates.io deps | README not fetchable (404 on main); cobserve already embeds PTYs — compare its own parser with vt100 0.16 before switching. |
| **tui-tree-widget** | 0.24.1 (2026-08-09, 1.85M dl) | ratatui ^0.30 | stateful tree with open/close, identifiers per node — crates.io | The obvious base for node → user → query; cobserve's hand-rolled tree can be measured against it. |
| **tui-scrollview** | 0.6.8 (2026-09-24) | ratatui ^0.30 | render into an oversized buffer then show a window with scrollbars — crates.io | Part of `ratatui/tui-widgets`; good for the inspector/SQL pane. |
| **tui-popup** | 0.7.7 (2026-09-24, 943k dl) | ratatui ^0.30, crossterm opt | centred popup with title/border, optional mouse-drag — crates.io | Same monorepo. |
| **tui-big-text** | 0.8.10 (2026-09-24, 540k dl) | ratatui ^0.30 | font8x8 big glyphs (full/half/quadrant/sextant pixel sizes) — crates.io, [awesome-ratatui](https://github.com/ratatui/awesome-ratatui) | Use for a splash or a huge "93%" in zoom mode, nothing else. |
| **tui-prompts** | 0.6.8 (2026-09-24) | ratatui ^0.30 | text/password/confirm prompts with state — crates.io | Same monorepo. |
| **tui-bar-graph** | 0.3.6 (2026-09-24) | ratatui ^0.30 | "pretty bar graphs" with gradient colouring — crates.io | Colour gradients conflict with "colour = severity only" unless gradient is monochrome. |
| **tui-scrollbar** | 0.2.8 (2026-09-24, 1.55M dl) | ratatui ^0.30 | standalone scrollbar — crates.io | Ratatui has `Scrollbar` built in; this is the monorepo's variant. |
| **tui-logger** | 0.18.3 (2026-07-04, 2.52M dl) | ratatui ^0.30, crossterm ^0.29 | `log`/`tracing` sink + smart log viewer widget with per-target level filtering — crates.io | Could back the TAPE view's debug mode. |
| **throbber-widgets-tui** | 0.11.1 (2026-06-19, 987k dl) | ratatui ^0.30 | spinner sets (braille, dots, arrows…) as a stateful widget — crates.io | |
| **tui-input** | 0.15.5 (2026-09-26, 2.22M dl) | ratatui ^0.30.2 opt, crossterm ^0.29 opt | headless single-line input (cursor, history-friendly) — crates.io | Right size for `:` command bar and `/` filter. |
| **ratatui-textarea** | 0.9.3 (2026-10-05, 810k dl) | ratatui-core ^0.1.1, ratatui-widgets ^0.3.1 | multi-line editor (fork of tui-textarea under the ratatui org) — crates.io | Prefer over **tui-textarea** 0.7.0 (2024-10-22, ratatui ^0.29 — **stale**, does not compile against 0.30 without patching). |
| **edtui** | 0.11.7 (2026-08-16, 343k dl) | ratatui ^0.30, syntect opt | vim-inspired editor widget with syntax highlighting — crates.io | Heavier than ratatui-textarea; relevant only if the SQL console wants modal editing. |
| **modalkit** | 0.0.31 (2026-10-07, 49k dl) | crossterm ^0.29 (no direct ratatui dep listed in filter) | vim/emacs-style modal input framework (used by iamb) — crates.io | Pre-1.0 (0.0.x) API churn; big dependency for a monitor. |
| **tui-markdown** | 0.3.11 (2026-10-10, 530k dl) | ratatui ^0.30, pulldown-cmark ^0.13, syntect opt | Markdown → `Text` — crates.io | For Jira descriptions / Claude answers in sessions. |
| **ratskin** | 0.3.1 (2026-01-19, 9.5k dl) | ratatui ^0.30, termimad ^0.34 | termimad-rendered markdown as ratatui widgets (from mdfried) — crates.io | Small user base. |
| **ratatui-explorer** | 0.3.0 (2026-03-06) | ratatui ^0.30 | file explorer widget — crates.io | |
| **ratatui-splash-screen** | 0.1.5 (2026-02-25) | ratatui >=0.25 | image → splash with fade-in — crates.io | |
| **tuirealm** | 4.1.0 (2026-05-02, 247k dl) | ratatui ^0.30 | Elm/React-like component framework + `tui-realm-stdlib` 4.1.0 — crates.io | A framework, not a widget; adopting it is a rewrite. |
| **tui-widget-list** | 0.15.3 (2026-07-18) | ratatui ^0.30 | list of arbitrary widgets with variable heights, scroll — crates.io | Fits "rows that are cards". |
| **ratatui-interact** | 0.5.3 (2026-04-02) | ratatui ^0.30, crossterm ^0.29 | focus management + mouse hit-testing for components — crates.io | |
| **tui-overlay** 0.1.2, **tui-skeleton** 0.3.0, **tui-shimmer** 0.1.3, **tui-tabs** 0.1.1 | 2026-02…04, tiny download counts | ratatui ^0.30 | drawers/modals/toasts; "placeholder widgets with pulse, sweep, shimmer effects"; shimmer text; "tab navigation with bordered boxes and rounded corners" — [awesome-ratatui](https://github.com/ratatui/awesome-ratatui), crates.io | Young (0.1–0.3, <5k downloads); copy ideas, don't depend. |
| **ratatui-toaster** 0.1.4 / **ratatui-comfy-toaster** 0.6.2 / **ratatui-comfy-tabs** 0.5.12 | 2026-06/07 | ratatui ^0.30 | toast queues; "feature-rich tab navigation" — awesome-ratatui | Small. |
| **opaline** | 0.4.2 (2026-09-02, 30k dl) | ratatui ^0.30 | "Token-based theme engine with gradients, 20 builtin themes, user theme discovery" — awesome-ratatui | A ready model for semantic tokens if Tarum is to become a theme *system*. |
| **catppuccin** | 2.8.0 (2026-04-26, 123k dl) | ratatui ^0.30 (feature `ratatui`) | 4 flavors × 26 colours, `ratatui` feature converts to `ratatui::style::Color`, ANSI mapping — [docs.rs](https://docs.rs/catppuccin/latest/catppuccin/) | |
| **rat-widget** 3.2.1 / **rat-salsa** 4.0.3 | 2026-03 | ratatui-core ^0.1, ratatui-widgets ^0.3 | data-input widgets, table, file dialog, menubar with submenus; event-queue with tasks/timers/focus/dialogs — awesome-ratatui | A full app framework; same caveat as tuirealm. |
| **hyperrat** | 0.1.3 (2026-06-28) | ratatui-core ^0.1.2 | `Link` widget for OSC 8 with hover style and fallback suffix — [docs.rs](https://docs.rs/hyperrat) | Wraps the 2-char-chunk trick; terminal must support OSC 8. |
| **tui-syntax-highlight** | 0.2.0 (2025-12-26) | ratatui ^0.30, syntect ^5 | code blocks → styled `Text` — crates.io | For the SQL pane; syntect pulls in onig/fancy-regex. |
| **malevich** | 1.24.1 (2026-10-03) | ratatui ^0.30 | "line, scatter, bar, histogram, heatmap, box plot, and violin charts" — awesome-ratatui | Heatmap is the one chart type ratatui lacks (node × minute heat strip). |
| **ratagrid** | 0.3.1 (2026-10-11, 89 dl) | — | "Typed data grid with sorting, search, and selection" — awesome-ratatui | Brand new. |
| **ansi-to-tui** | 8.0.1 (2026-01-10, 6.68M dl) | — | ANSI text → ratatui `Text` — crates.io | For Airflow task logs with colour codes. |
| **termprofile** | 0.2.4 (2026-05-02) | — | "Detect and handle terminal color/styling support" — awesome-ratatui | Replaces hand-rolled truecolor/256/16 detection. |
| **tui-pantry** | — | — | "Component-driven development tool for ratatui widgets, similar to Storybook" — awesome-ratatui | Useful for the screenshot pipeline in `dev/`. |
| **mousefood** / **ratatui-wgpu** / **soft_ratatui** / **egui_ratatui** | — | — | embedded-graphics, wgpu, software and egui backends — awesome-ratatui | A wgpu/soft backend is an alternative to xterm.js for the desktop shell (render cells natively instead of emulating a terminal). |

### Inferences
- A 0.30 redesign can rely on the `ratatui/tui-widgets` monorepo (popup, scrollview, prompts,
  big-text, bar-graph, scrollbar) plus tui-tree-widget, tui-input, tachyonfx, ratatui-image,
  tui-term, tui-markdown, catppuccin/opaline, hyperrat — all on 0.30 as of 2026-10. The frameworks
  (tuirealm, rat-salsa, ratatui-kit) are not needed for an app of cobserve's size and would force a
  rewrite of event handling.
- Avoid `tui-textarea`; use `ratatui-textarea`. Avoid depending on the 0.1.x toast/overlay/skeleton
  crates — they are 50–200 lines each and better re-implemented inside cobserve's theme.
- ratzilla gives the desktop shell a second option: instead of xterm.js running cobserve in a PTY,
  compile the *draw* side to WASM and feed it snapshots over a websocket. It needs the polling
  core separated from the UI (which the "UI never blocks on network" rule already implies).

### Gaps
- tui-term's README (limits on alternate screen, scrollback, mouse passthrough) was not readable
  (404 on `main`; the default branch may be `development`). Only the dependency set is confirmed.
- No crate was found that renders a *heat-map strip per row* in 1-cell height besides malevich's
  heatmap; cobserve's `▂▃▄` sparkline is the usual approach.
- modalkit's ratatui dependency did not appear in the filtered dependency list — unverified whether
  it pins 0.30.

---

## KQ5 — Chrome with borders vs borderless; painted surfaces vs rules

### Takeaway
The 2024–2026 convention is: borders carry *meaning* (focus, modality), not grouping. Textual
formalised the alternative — `background` → `surface` → `panel` → `boost` layers — and both
Posting and Harlequin are built on it; Helix ships popups without borders by default; zellij's
default frame is a title line only; lazygit and television let the user pick `rounded`/`hidden`/
`none`. No controlled readability study of TUI borders was found — the evidence is practitioner
consensus and defaults.

### Cited Findings
- Textual design variables: `$background` "A color used for the background, where there is no
  content"; `$surface` "The default background color of widgets, typically sitting on top of
  `$background`"; `$panel` "A color used to differentiate a part of the UI form the main content.
  Used sparingly in Textual itself"; `$boost` "A color with alpha that can be used to create
  _layers_ on a background"; `$primary` "Typically used for titles, and backgrounds for strong
  emphasis"; `$accent`, `$warning`, `$error`, `$success`; automatic `-lighten-1..3` /
  `-darken-1..3` shades; muted variants blend with background at 70 % — [Textual design guide](https://textual.textualize.io/guide/design/)
- Posting's roles repeat the ladder: Surface "inputs, text areas and tables", Panel "raised
  surfaces, a step above `surface`", Accent "used to show what has focus" — [posting themes](https://posting.sh/guide/themes/)
- Helix: `popup-border` default `none`; `cursorline` default false — [Helix editor config](https://docs.helix-editor.com/editor.html)
- zellij: `pane_frame_style` default `titles` = "single line above panes"; `full` and `none`
  exist; `mouse_hover_effects` "pane frame highlight" — [zellij options](https://zellij.dev/documentation/options.html)
- lazygit: `border: rounded` default with `hidden` available; focus by `activeBorderColor` vs
  `inactiveBorderColor`; "jump-to-window keybindings in window titles" — [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- television: `border_type` none|plain|rounded|thick per panel, plus `padding` — [config.toml](https://github.com/alexpasmantier/television/blob/main/.config/config.toml)
- btop: `rounded_corners` on, `theme_background` toggles painting the background —
  [btop README](https://github.com/aristocratos/btop)
- ratatui tooling for both styles: `Block::bordered().border_type(BorderType::Rounded)`,
  `Borders` bitflags for one-sided rules, `Padding`, `merge_borders(MergeStrategy)`, `shadow()`,
  `title_top/title_bottom` with `Line::centered()` — [Block docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Block.html)
- Practitioner guidance (LLM-skill documents on skills.sh — low authority, 2026): "Reserve full
  borders for important regions; use whitespace or subtle separators for secondary grouping";
  "Create hierarchy with, in order: placement, spacing, grouping, text labels, alignment, weight,
  then color"; "one border language, one spacing scale, one heading system, and semantic color
  roles"; "High-intensity color, heavy borders, inverse video, animation, and icons should
  highlight state changes or actions, not decorate every element" —
  [tui-visual-design](https://www.skills.sh/josiahsiegel/claude-plugin-marketplace/tui-visual-design),
  [tui-design](https://www.skills.sh/marcioaltoe/skills/tui-design)
- Will McGugan (Textual): modern terminals are GPU-accelerated and "We had 60 frames per second
  animation moving across the screen, and it looked silky smooth"; emoji and some symbols "occupy
  two terminal cells, with cross-platform rendering disagreements" —
  [Talk Python #380 transcript](https://talkpython.fm/episodes/transcript/380/7-lessons-from-building-a-modern-tui-framework)

  ```
  Three chrome styles for the same inspector, cobserve-sized:

  (a) full rounded box, focus by border colour (lazygit)
  ╭─ clickhouse3 · pay-ch-node-1 ───────────── f zoom ─╮
  │ MEM 59.6/64 GiB 93.1%  ▂▃▄▅▆▇▇▃ → steady          │
  ╰────────────────────────────────────────────────────╯

  (b) title-rule only (zellij `titles`, cobserve today)
  ─ clickhouse3 · pay-ch-node-1 ──────────────── f zoom ─
    MEM 59.6/64 GiB 93.1%  ▂▃▄▅▆▇▇▃ → steady

  (c) painted surface, no rule (Textual surface/panel) — ▒ = panel bg one step above surface
  ▒ clickhouse3 · pay-ch-node-1                  f zoom ▒
  ▒ MEM 59.6/64 GiB 93.1%  ▂▃▄▅▆▇▇▃ → steady            ▒
  ```

### Inferences
- Painted surfaces (c) need truecolor or a carefully chosen 256-colour step and do nothing on the
  owner's "mono" variant; rules (b) and boxes (a) degrade to 16 colours. A redesign that must still
  work in 16-colour mode should keep (b) as the base and add (c) as an *enhancement* when the
  theme has ≥256 colours — i.e. a `panel` token that is `Color::Reset` in mono.
- Severity-only colour (owner's rule) *favours* borderless/painted chrome: a coloured border would
  compete with the red/amber/green of the data. Focus should then be shown by a brighter *title*
  (bold indigo) or a `▌` left gutter marker, which cobserve already uses on the selected row.
- `merge_borders` makes a card grid viable without the "double rule" artefact that makes bordered
  tiles look dated.

### Gaps
- No empirical readability study (eye-tracking, task time) comparing bordered vs borderless TUIs
  was found; everything above is defaults and practitioner opinion.
- The "7 things I've learned building a modern TUI framework" blog post itself was 404; the
  podcast transcript substitutes.

---

## KQ6 — Published design guidance for TUIs

### Takeaway
Formal guidance is thin. The reusable sources are Textual's design guide (colour roles, layers),
clig.dev (colour with intention, NO_COLOR, <100 ms responsiveness, no animation when not a TTY,
confirm dangerous actions), Charm's lipgloss model (CSS-like padding/borders, colour profiles with
downsampling), and Helix/zellij/lazygit defaults as de-facto standards. Ratatui's own site has
layout/recipes, not a design guide. Textualize the company closed in 2025 but Textual remains
maintained by McGugan.

### Cited Findings
- clig.dev: "Human-readable output is paramount. Humans come first, machines second"; "Use color
  with intention … Don't overuse it"; disable colour when not a TTY, when `NO_COLOR` is set,
  `TERM=dumb`, or `--no-color`; "If `stdout` is not an interactive terminal, don't display any
  animations"; "Use symbols and emoji where it makes things clearer"; "Show progress if something
  takes a long time"; "Responsive is more important than fast. Print something to the user in
  <100ms"; "Confirm before doing anything dangerous" — [clig.dev](https://clig.dev/)
- Textual design guide (roles, shades, muted variants) — [textual.textualize.io/guide/design](https://textual.textualize.io/guide/design/)
- Textualize closing: "a business needs a product. Textual has always been a solution in search of
  a problem"; Textual and Rich continue under McGugan — [The future of Textualize, 2025-05-07](https://textual.textualize.io/blog/2025/05/07/the-future-of-textualize/)
- Lip Gloss: CSS-style padding/margin, border per side, `LightDark`, `Complete` colour per profile,
  "complements rather than replaces Bubble Tea", Glamour for markdown — [lipgloss README](https://github.com/charmbracelet/lipgloss)
- Ratatui docs cover layout (`Constraint`, `Flex`, spacing, nesting) and recipes (centering,
  popups) — [layout concepts](https://ratatui.rs/concepts/layout/), [center recipe](https://ratatui.rs/recipes/layout/center-a-widget/)
- Orhun Parmaksız talks: FOSDEM 2024 "Introducing Ratatui", FOSDEM 2025 "Bringing terminal
  aesthetics to the Web with Rust (and vice versa)", Rust Forge 2025 "Becoming a terminal chef
  with Ratatui", Tokyo Rust Oct 2025 "Building Pocket-Sized Terminal UIs with Rust" (no_std on an
  ESP32 T-Display) — [FOSDEM 2024](https://fosdem.org/schedule/event/fosdem-2024-1934-introducing-ratatui-a-rust-library-to-cook-up-terminal-user-interfaces),
  [FOSDEM 2025](https://archive.fosdem.org/2025/schedule/speaker/orhun_parmaksiz/),
  [Rust Forge 2025](https://pretalx.com/rust-forge-2025/talk/N9UDZV), [Tokyo Rust](https://www.meetup.com/ja-JP/tokyo-rust-meetup/events/310899137/)
- PyCon India 2024 talk "TUIs Simplified": help hints "ranging from simple cycling to
  context-specific, priority-ranked hints based on usage frequency"; responsive layouts, themes,
  hotkey remapping, in-app tutorials; Zulip terminal as the example —
  [PyCon India CFP](https://in.pycon.org/cfp/2024/proposals/tuis-simplified-the-transformation-of-tuis-into-user-friendly-tools~dwp2m/)
- Textual lessons (McGugan): terminals can do 60 fps; use caching and immutability for layout;
  fractions not floats for splitting space ("ensuring exact screen space division without
  off-by-one issues"); wide-char/emoji width disagreements — [Talk Python #380](https://talkpython.fm/episodes/transcript/380/7-lessons-from-building-a-modern-tui-framework)
- A 2026 crop of "TUI design" skill files exists (skills.sh, tessl.io); they are LLM prompt
  material, not peer-reviewed, but they summarise the consensus quoted in KQ5 —
  [tui-design (tessl)](https://tessl.io/registry/skills/github/administrakt0r/pro-skills-repo-administraktor/tui-design)

### Inferences
- cobserve's existing rules already match clig.dev (colour with intention, progress/"● live",
  confirm the one write action). The gaps the guidance exposes are: a reduced-motion switch (atuin
  `prefers_reduced_motion`), NO_COLOR honouring, and a "usage-ranked" hint bar (PyCon talk) instead
  of a static one.
- "Fractions not floats" is moot in ratatui (integer Cassowary solver), but the lesson transfers:
  never compute column widths in the view code — let `Constraint`s do it so 80/120/160 columns all
  round consistently.

### Gaps
- No Charm "design notes" document exists publicly beyond READMEs; the "glamorous" vocabulary is
  marketing, not a written style guide.
- No Ratatui design guide (as opposed to API docs) exists.

---

## KQ7 — Terminal-protocol constraints for each device (what needs which terminal)

### Takeaway
Everything in KQ1–KQ5 is feasible in Rust + ratatui 0.30 + crossterm 0.29, but four devices depend
on the terminal: images (kitty/sixel/iTerm2), OSC 8 links, DEC 2026 synchronized output, and the
kitty keyboard protocol (for Shift/Ctrl-disambiguation and key-release). The wry/xterm.js desktop
shell determines which of these cobserve.app gets.

### Cited Findings
- Images: protocol × terminal matrix (Kitty ≥ 0.28 / Ghostty / WT ≥ 1.22 → kitty placeholders;
  Konsole → old kitty; iTerm2/WezTerm/Warp/Tabby/VSCode/Rio → iTerm2 inline; foot/WT/st → Sixel);
  zellij KGP "bug"; tmux needs `--enable-sixel` — [yazi image preview](https://yazi-rs.github.io/docs/image-preview);
  ratatui-image: Alacritty/Konsole/Contour/Warp "limited or unsupported", last-line Sixel scroll
  bug, 4:8 fallback cell ratio — [ratatui-image README](https://github.com/benjajaja/ratatui-image)
- OSC 8: ratatui example embeds `\x1B]8;;URL\x07…\x1B]8;;\x07` in cell symbols, 2-char chunks
  because of width bug #902; "Not all terminals recognize OSC 8" — [hyperlink example](https://ratatui.rs/examples/apps/hyperlink/)
- Synchronized output: crossterm `BeginSynchronizedUpdate` / `EndSynchronizedUpdate` —
  [crossterm terminal](https://docs.rs/crossterm/latest/crossterm/terminal/index.html); btop's
  `terminal_sync` "flicker reduction through synchronized output" — [btop README](https://github.com/aristocratos/btop);
  ink-terminal: "DEC 2026 synchronized updates prevent visual tearing" — [ink-terminal README](https://cdn.jsdelivr.net/npm/ink-terminal@0.1.0-alpha.1/README.md);
  mode 2026 supported by "Ghostty, Kitty, Alacritty, WezTerm, iTerm2, Windows Terminal, etc.",
  terminals without it "just ignore the escapes" — [ratto docs (secondary)](https://docs.rs/ratto/0.7.0/rat/)
- Kitty keyboard protocol in crossterm: `KeyboardEnhancementFlags` = `DISAMBIGUATE_ESCAPE_CODES`
  ("Represent Escape and modified keys using CSI-u sequences, so they can be unambiguously read"),
  `REPORT_EVENT_TYPES` (Repeat/Release events), `REPORT_ALTERNATE_KEYS`,
  `REPORT_ALL_KEYS_AS_ESCAPE_CODES`; "Alternate keys and Unicode codepoints are not yet supported
  by crossterm"; `supports_keyboard_enhancement()` probes the terminal —
  [KeyboardEnhancementFlags](https://docs.rs/crossterm/latest/crossterm/event/struct.KeyboardEnhancementFlags.html),
  [crossterm terminal](https://docs.rs/crossterm/latest/crossterm/terminal/index.html)
- Truecolor fallbacks: btop "24-bit truecolor" with "256 color conversion" — [btop README](https://github.com/aristocratos/btop);
  lipgloss `Complete` per profile (ANSI16/256/TrueColor/ASCII) — [lipgloss README](https://github.com/charmbracelet/lipgloss);
  Helix `true-color`/`undercurl` override detection — [Helix config](https://docs.helix-editor.com/editor.html);
  zellij `styled_underlines` — [zellij options](https://zellij.dev/documentation/options.html)
- Nerd Fonts are optional everywhere: lazygit `nerdFontsVersion` "" | "2" | "3" —
  [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md); zellij
  `simplified_ui` "without arrow fonts" — [zellij options](https://zellij.dev/documentation/options.html)
- Canvas Octant/Sextant "may display replacement characters in unsupported terminals" —
  [Marker docs](https://docs.rs/ratatui/latest/ratatui/symbols/enum.Marker.html)
- Alternatives to xterm.js for the desktop shell: ratzilla (DOM/Canvas/WebGL2 backends, mouse,
  hyperlinks) — [ratzilla docs](https://docs.rs/ratzilla/latest/ratzilla/); ratatui-wgpu /
  soft_ratatui / egui_ratatui backends — [awesome-ratatui](https://github.com/ratatui/awesome-ratatui)

### Inferences
- A capability probe at start-up (termprofile for colour depth, `supports_keyboard_enhancement`,
  ratatui-image `Picker` for graphics, `$TERM_PROGRAM` for OSC 8 heuristics) lets the same binary
  pick: painted panels vs rules, Octant vs Braille vs `▂▃▄`, inline Airflow Gantt as a kitty image
  vs cells, clickable Jira/Redash links vs `y` copy-link. Everything must degrade; the current `y
  copies its link` is the right fallback for OSC 8.
- xterm.js (desktop shell) supports truecolor, OSC 8 and mode 2026 in recent versions but **not**
  kitty graphics or Sixel out of the box, so image-based devices would only appear in a kitty/
  Ghostty/WezTerm terminal — *this xterm.js capability claim was not verified in this research and
  should be checked against the xterm.js version bundled in `desktop/`.*
- Kitty keyboard protocol is what makes `Shift+Tab`, `Ctrl+Enter`, `Ctrl+,` reliably distinct;
  without it the key map must stay within the plain-ASCII-safe set cobserve uses today.

### Gaps
- xterm.js and WebKit (wry) support for OSC 8 / mode 2026 / kitty graphics was not checked.
- Whether ratatui 0.30's `CrosstermBackend` already brackets frames in mode 2026 was not
  confirmed (see KQ3 gap).
- Ghostty/Kitty "tab status indicators" (mentioned by ink-terminal) — no primary spec was fetched.
