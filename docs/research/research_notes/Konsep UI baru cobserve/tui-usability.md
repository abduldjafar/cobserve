# TUI usability and ease-of-use: patterns for a cobserve redesign

Scope: what makes a terminal UI easy for a new user to pick up and fast for a daily user,
with the exact behaviour of the best tools, how to do each in Rust + ratatui 0.30 + crossterm,
and the trade-offs. Written 2026-10-13. Everything marked *Inference* is my reading of the
cited behaviour applied to cobserve; everything in *Cited Findings* has a URL.

## What is hard in cobserve today (read from GUIDE.md §6–8 and docs/screenshots/120x36-help.txt)

### Takeaway
cobserve already has the three discoverability primitives the good tools have (a `?` overlay,
a context footer, consistent `⏎`/`esc`) but it spends them badly: the `?` screen is one
30-line wall for *all* views at once, the footer carries 8 hints in ~110 columns, ten views sit
on digits with no grouping, the sessions live behind a prefix (`ctrl+\`) that nothing on
screen advertises until you are already inside a session, and configuration is environment
variables only.

### Cited Findings
- The `?` overlay at 120×36 shows ~22 key rows plus 10 legend rows in one list spanning every
  view (`1 … 7  0  views`, `ctrl+\  the sessions · then 1-6 a view, 7-9 or ↑↓ a session…`,
  `x  then y  on view 2`), and several rows are truncated with `…` at that width — [docs/screenshots/120x36-help.txt](docs/screenshots/120x36-help.txt)
- The NODES footer at 120 columns carries eight hints: `↑↓ move · ⏎ open · c SQL on it · ←→ fold · tab insights · u pivot · / filter · s sort · ␣ healthy` — [00-context.md](00-context.md)
- The masthead has eight tabs on digits `1 nodes 2 queue 3 map 4 tape 5 airflow 6 jira 7 sessions 0 local`; `0` is LOCAL "drawn after the sessions so they keep 7–9", and the sessions are reached with `7` or `ctrl+\` then `7`–`9`/`↑↓` — [GUIDE.md §6, §8.1](../../../GUIDE.md); [CLAUDE.md "Six monitor views, sessions from 7"](../../../CLAUDE.md)
- Inside a session "every key belongs to the program … The one exception is `ctrl+\`, which opens the bar", and `F1`…`F9` are the fallback "for a terminal that keeps `ctrl+\` to itself" — [GUIDE.md §8.1](../../../GUIDE.md)
- The one destructive action is a two-key chord: "`x`, then `y` — cancel the job in Redash. `x` asks first and says what will happen. `y` does it. Any other key keeps the job" — [GUIDE.md §7.2](../../../GUIDE.md)
- Configuration is "environment-only (DESIGN.md §9)"; the desktop app reads `~/.config/cobserve/env` and a credential file — [CLAUDE.md](../../../CLAUDE.md)
- The band's third line (LOCAL) "needs 20 rows"; the day line (prayer times) is a second permanent line under the masthead — [CLAUDE.md](../../../CLAUDE.md); [GUIDE.md §6](../../../GUIDE.md)

### Inferences
- Four chrome rows (masthead, day line, FLEET, REDASH) plus a footer leave 31 of 36 rows at the target size and 19 of 24 at the minimum. Every pattern below that adds chrome must take a row from these, not add one.
- The current `?` is a *reference card*, not *help*: it answers "what exists" rather than "what can I do here, now". Textual, lazygit, k9s and yazi all scope help to the focused context (see Discoverability).

### Gaps
- No user research on cobserve itself exists in the repo; what is "hard" above is inferred from the artefacts, not observed.

---

## 1. First run and onboarding: setup from inside the app, config writing, demo modes

### Takeaway
The best CLIs do first-run setup as a short, skippable, prompt-by-prompt flow (gh, atuin),
store secrets in the OS credential store with a plain-text fallback that is *announced*
(gh), keep config at an XDG path in TOML/YAML (k9s, lazygit, posting), and let string values
in config reference environment variables so a password never lands in the file (Harlequin).
In Rust the `keyring` crate gives macOS Keychain / Secret Service / Windows Credential
Manager with one API. A "no fleet" screen that explains what to set (k9s, lazygit's
`notARepository`) plus a demo mode (`FAKE=1`, which cobserve already has) covers the
zero-config start.

### Cited Findings
- gh's interactive login asks, in order: "What is your preferred protocol for Git operations on this host?", "How would you like to authenticate GitHub CLI?" with the options "Login with a web browser" / "Paste an authentication token", prints "Tip: you can generate a Personal Access Token here https://%s/settings/tokens" and "The minimum required scopes are %s.", ends with "Authentication complete." and "Logged in as %s" — [cli/cli login_flow.go](https://github.com/cli/cli/blob/trunk/pkg/cmd/auth/shared/login_flow.go)
- gh stores the token "securely in the system credential store. If a credential store is not found or there is an issue using it gh will fallback to writing the token to a plain text file"; `--insecure-storage` opts into plain text; the fallback is announced as "Authentication credentials saved in plain text" — [gh auth login manual](https://cli.github.com/manual/gh_auth_login); [login_flow.go](https://github.com/cli/cli/blob/trunk/pkg/cmd/auth/shared/login_flow.go)
- gh also has non-interactive paths: `gh auth login --with-token < mytoken.txt`, and "gh will use the authentication token found in environment variables. This method is most suitable for 'headless' use" — [gh auth login manual](https://cli.github.com/manual/gh_auth_login)
- atuin: `atuin register -u <USERNAME> -e <EMAIL>`; "you can omit the password flag and Atuin will ask for it instead, keeping it out of your shell history"; after registration it generates an encryption key stored locally, and on a new machine `atuin login -u <USERNAME>` "will be prompted for your password and for your key" — [docs.atuin.sh Setting up Sync](https://docs.atuin.sh/guide/sync/)
- Harlequin profiles: "A profile's string values can name environment variables — `password = "${MYPASSWORD}"`, or `${MYHOST:-localhost}` to supply a default" and "values an adapter declares as secrets are masked wherever Harlequin prints them" — [harlequin README](https://github.com/tconbeer/harlequin)
- k9s keeps config in `$XDG_CONFIG_HOME/k9s/config.yaml`, hotkeys in `$XDG_CONFIG_HOME/k9s/hotkeys.yaml`, skins in `$XDG_DATA_HOME/k9s/skins/`; `k9s --readonly` "disables all modification commands" — [k9s README](https://github.com/derailed/k9s#readme); [k9s config](https://www.k9scli.io/topics/config)
- lazygit: `notARepository` — "What to do when opening Lazygit outside of a git repo: 'prompt', 'create', 'skip', or 'quit'"; `disableStartupPopups` — "If true, don't display introductory popups upon opening Lazygit." — [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- lazygit's first-run popup text begins "Thanks for using lazygit! Seriously you rock. Three things to share with you: 1) If you want to learn about lazygit's features, watch this vid: … 2) Be sure to read the latest release notes…" — [lazygit english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go)
- posting: without a collection "your requests are saved to the default collection, a directory Posting keeps for you"; its config is YAML (`theme: aurora`, `layout: horizontal`) — [posting guide](https://posting.sh/guide/)
- clig.dev: config precedence "Flags > Environment variables > Project-level configuration > User-level configuration > System-wide configuration"; "Only use prompts or interactive elements if `stdin` is an interactive terminal (a TTY)"; "Never _require_ a prompt"; "If `--no-input` is passed, don't prompt" — [clig.dev](https://clig.dev/)
- The `keyring` crate "provides secure storage support for Linux (secret-service and kernel keyutils), iOS (keychain), macOS (keychain), and Windows (credential manager)" and "delegates to whichever OS-level credential store is available, so secrets never touch application-controlled disk files in plaintext" — [docs.rs/keyring](https://docs.rs/keyring/)
- A ratatui app (teamctl-ui) does first-run onboarding as 10 `Step { heading, body }` screens, "Press any key to advance", `Esc` skips, reopenable with a `t` chord "from any non-modal state", tracked by a presence-only sentinel file `.team/state/ui-tutorial-completed`; e.g. "Agents + state glyphs: Each agent shows a single-cell glyph: ● working · ○ idle · ! approval pending · ✕ stopped · ? unknown." — [teamctl-ui onboarding.rs](https://docs.rs/teamctl-ui/0.11.0/src/teamctl_ui/onboarding.rs.html)
- Charm's `huh` is what gh uses for its accessible prompter ("Our new prompter is built using Charm's open source `charmbracelet/huh` prompting library") — [GitHub blog](https://github.blog/engineering/user-experience/building-a-more-accessible-github-cli/)

### Inferences
- **Yes, config can be entered and saved from inside the app safely**, by copying gh's exact shape: (1) a prompt sequence, one question per screen, each with a one-line "why"; (2) secrets to the OS store via `keyring` under service `cobserve`, account = the thing (`ch:<cluster>`, `redash`, `airflow`, `jira`); (3) when the store fails, write to `~/.config/cobserve/credentials` with mode 0600 and *say so* on screen ("saved in plain text at …, because the keychain refused"); (4) non-secrets to `~/.config/cobserve/config.toml` (XDG, `directories` or `etcetera` crate); (5) keep env vars as the override layer per clig, so today's `CH_SEED_URLS=…` keeps working and `--no-input`/`CI` disables prompting.
- **Order the wizard by payoff**, so a user who stops after one step still gets a working screen: ClickHouse seeds + cluster + user → first NODES screen; then Redash (view 2 lights up); Airflow; Jira; prayer city. Each step ends with a live check ("9 nodes answered · ch3 refused the password") instead of "saved".
- **A `:setup` / `config edit` command** and a `?`-reachable "Settings" screen should both exist; the empty state of each view should link to the step that fills it ("REDASH needs an API key — press `s` to add it").
- **Demo mode** already exists (`FAKE=1`). Make it discoverable: the no-config landing screen offers "`d` try it with a generated fleet" so a newcomer sees the UI before typing a single URL (mirrors posting's default collection and lazygit's `notARepository: prompt`).
- **Trade-offs**: keyring prompts the macOS Keychain dialog on first read of each item (one per secret; store a single JSON blob under one entry to get one prompt); the Secret Service on Linux needs D-Bus (headless servers fall back to the 0600 file); a wizard adds state (`Screen::Setup(step)`) that must not block the poll loop.

### Gaps
- Harlequin's `--config` wizard page returned 429 twice; I could not confirm its exact prompts, only that profiles exist and secrets are masked.
- k9s's "no cluster" screen wording was not captured (the README only says it "displays an informational state prompting proper configuration").
- ClickHouse client's own config prompts were not researched (out of budget).

---

## 2. Discoverability: which-key popups, `?`, command mode/palette, footer legends, empty states

### Takeaway
Every widely adopted TUI ships at least two of: a context-scoped `?` (lazygit, k9s, yazi),
a footer that shows only the keys that work *right now* (Textual, posting, zellij), a
which-key popup after a prefix (helix `auto-info`, yazi `[which]`), and a fuzzy command
palette (Textual `ctrl+p`, k9s `:`). The palette is the one that scales past ~15 commands;
the footer is the one that teaches without being asked. None of them is a 30-row card.

### Cited Findings
- Textual's command palette opens on **Ctrl+P**, matches fuzzily ("match the keys you type in the same order, but not necessarily at the start of the command", `th` → "Change **th**eme"), each `Hit` has display text with highlighted matches, optional help text and a callback, `discover()` lists commands when the input is empty, and built-ins cover theme, keyboard help, quit, screenshot — [Textual command palette guide](https://textual.textualize.io/guide/command_palette/)
- Textualize's stated goal for the palette: "once you learn how to use one Textual app, you can use them all" and "users only need to type two or three characters to get to any command" — [Textual 0.37 blog](https://textual.textualize.io/blog/2023/09/15/textual-0370-adds-a-command-palette/)
- Textual's Footer shows bindings of "the currently focused widget" cascading up to the app; `Binding(show=False)` hides one, `key_display` relabels a key, `compact` tightens it, and `show_command_palette` draws `^p palette` at the right — [Textual Footer](https://textual.textualize.io/widgets/footer/)
- posting: "The footer at the bottom of the screen always shows the most useful keys for whatever is focused"; "Press Ctrl+P to open the command palette, which lists everything Posting can do"; F1 opens keyboard-shortcut help — [posting guide](https://posting.sh/guide/)
- k9s: `?` "Show active keyboard mnemonics and help"; `ctrl-a` "Show all available resource alias"; `:` command mode accepts "singular, plural, short-name or alias" (`:pod⏎`, `:pod ns-x⏎`, `:pod /fred⏎`, `:pod app=fred,env=dev⏎`); `<esc>` "Bails out of view/command/filter mode"; a breadcrumb trail at the bottom; hotkeys in `hotkeys.yaml` — [k9s commands](https://k9scli.io/topics/commands/); [k9s README](https://github.com/derailed/k9s#readme)
- lazygit: `?` = `optionMenu` "Open keybindings menu", scoped per panel (Global / list navigation / Files / Commits / …); `gui.showBottomLine` — "show the bottom line that contains keybinding info and useful buttons"; `gui.showPanelJumps` — "show jump-to-window keybindings in window titles"; `jumpToBlock: ['1','2','3','4','5']` — [Keybindings_en.md](https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings/Keybindings_en.md); [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- helix: `auto-info` — "Whether to display info boxes", default `true`; `idle-timeout` 250 ms; prefixes `space` (pickers), `g` goto, `z`/`Z` view, `m` match, `Ctrl-w` window; `:` is command mode — [helix editor config](https://docs.helix-editor.com/editor.html); [helix keymap](https://docs.helix-editor.com/keymap.html)
- yazi: "Press q to quit, F1 or ~ to open the help menu"; its theme has a `[which]` section (which-key: `cols` 1–3, `cand` "Candidate key style", `rest`, `desc`, `separator` e.g. `" -> "`) and a `[help]` section (`chord`, `action`, `hovered`) — [yazi quick start](https://yazi-rs.github.io/docs/quick-start); [yazi theme](https://yazi-rs.github.io/docs/configuration/theme)
- zellij's compact bar can show its keybinding hints on demand: `compact-bar location="zellij:compact-bar" { tooltip "F1" }`; its "Unlock-First (non-colliding)" preset exists because default `Ctrl+…` binds collide with programs — [zellij FAQ](https://zellij.dev/documentation/faq.html)
- Empty-state strings in lazygit are plain sentences: "No changed files", "No branches for this repo" — [english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go)
- A heuristic checklist for TUIs renders Nielsen #6 as "Footer hints and `?` overlay exist; nothing works only if memorized" and #7 as "Keyboard shortcuts, batch/operator patterns, command mode for power users" — [tui-critique](https://skillsmp.com/creators/kyleking/dotfiles/private-dot-claude-skills-tui-critique)
- Of 1,100-odd survey answers Julia Evans grouped, "Poor discoverability" (55) and "Keyboard shortcut inconsistencies" (84) were named frustrations; "Remembering syntax" topped the list (115) — [jvns.ca](https://jvns.ca/blog/2025/02/05/some-terminal-frustrations/)
- Ratatui ecosystem: `ratatui-command-palette` 0.1.0 (ratatui-labs, "experimental", depends on ratatui ^0.30.2, five renderers: Modal, FlatOverlay, SplitPreview, Fullscreen, InlineDropdown, fuzzy matching, emits `PaletteEvent`); `ratada` also ships a palette — [docs.rs/ratatui-command-palette](https://docs.rs/ratatui-command-palette); [docs.rs/ratada](https://docs.rs/ratada)
- television uses `nucleo` ("the nucleo matcher used by the helix editor") for fuzzy matching on top of ratatui — [docs.rs/television](https://docs.rs/television)

### Inferences
- **Replace the single `?` card with three layers**, each small:
  1. *Footer* = at most 5 hints for the focused pane, ordered by frequency, Textual-style (`↑↓ move  ⏎ open  / filter  ? keys  : commands`); the rest live in `?`. One row.
  2. *`?` scoped to the focused pane*, lazygit-style sections ("NODES tree", "Insights", "Everywhere"), searchable by typing (yazi's `[help]` has a hovered item; filter-as-you-type is the natural extension). Target ≤ 12 rows at 80×24.
  3. *`:` palette* (or `ctrl+p`; see §3 on why `:` is safer in a PTY app) listing *every* action as a sentence with its key at the right — "Cancel this Redash job … x y", "Go to airflow … 5", "Open SQL on clickhouse3 … c" — fuzzy with `nucleo`, `discover()` showing the 8 most relevant when empty (what is wrong now first). The palette is also where rare settings live (`:theme light`, `:clock utc`, `:prayer off`, `:setup redash`), so the footer never has to mention `z`/`d`/`p`.
- **Which-key for the only prefix left**: if the session prefix stays, the bar that opens after `ctrl+\` *is* a which-key popup already; make it look like one (two columns, `cand -> desc`, yazi `[which]` layout) and show it 250 ms after the prefix (helix `idle-timeout`) so a fast user never sees it.
- **Empty states teach**: every pane with nothing to show prints one sentence + one key, e.g. QUEUE without Redis: "WAITING names need Redis · counts only — `:setup redash` adds `REDIS_URL`", AIRFLOW unconfigured: "No Airflow yet — `s` to connect one · `5` later".
- Crate choice: `ratatui-command-palette` is 0.1.0/experimental — fine to study, risky to depend on; a ~300-line own palette (`tui-input` for the line, `nucleo` for matching, a `Vec<Action { name, keys, scope, run }>`) is the safer bet and doubles as the single source for footer, `?` and palette (one action table, three renderers).

### Gaps
- Helix's docs do not describe the infobox's visual layout or delay beyond `auto-info`/`idle-timeout`; I could not confirm whether the infobox is delayed or immediate.
- No quantitative evidence (A/B, telemetry) that palettes beat `?` cards was found; the evidence is adoption and the Textualize/posting design statements.
- zellij's literal "Tip:" strings (status-bar `tip/data`) were not retrievable (404 on the path tried).

---

## 3. Navigation models and cognitive load: tabs vs palette, zoom, esc stacks, keys, mouse

### Takeaway
The tools with many screens use a *command* to get to a screen (k9s `:pod`) plus a
*history stack* (k9s breadcrumbs, `esc` back, `-`/`[`/`]`), and reserve digits for a
handful of panels (lazygit 1–5, yazi tabs 1–9). Zoom is a cycle on one key (lazygit
`+`/`_` normal → half → full). `esc` must always back out one level and never trap. For a
PTY-hosting app, the key problem is prefix collision; zellij's answer is a "locked" mode
and a non-colliding preset, and the kitty keyboard protocol (now in iTerm2, WezTerm,
Ghostty, Alacritty, foot, Windows Terminal, Rio, xterm.js) is what makes `ctrl+\`, `shift+⏎`
and friends reliable. Mouse is expected in 2026 (k9s, btop, lazygit, zellij all support it).

### Cited Findings
- k9s: "A breadcrumb trail appears at the bottom, enabling quick backtracking through your navigation history—comparable to browser history or `cd -`"; `-` and `[`/`]` move through command history; `esc` returns to the previous view; config has `crumbsless`, `headless`, `logoless`, `enableMouse`, `reactive` — [k9s README](https://github.com/derailed/k9s#readme); [k9s config](https://www.k9scli.io/topics/config)
- lazygit: `+` "nextScreenMode", `_` "prevScreenMode", `gui.screenMode` "One of: 'normal' (default) | 'half' | 'full'"; `<esc>` = `return`; `quitOnTopLevelReturn` "exit Lazygit when the user presses escape in a context where there is nothing to cancel/close"; `]`/`[` next/prev tab; `gui.mainPanelSplitMode: 'flexible'` = "split horizontally if wide enough, otherwise vertically"; `gui.expandFocusedSidePanel` = accordion — [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- yazi tabs: `t ⇒ t` new tab, digits 1–9 switch — [yazi quick start](https://yazi-rs.github.io/docs/quick-start)
- zellij: modes `normal, locked, resize, pane, move, tab, scroll, search, entersearch, renametab, renamepane, session, tmux`; "Unlock-First (non-colliding)" preset for key clashes; `simplified-ui true` for fonts without special glyphs — [zellij modes](https://zellij.dev/documentation/keybindings-modes.html); [zellij FAQ](https://zellij.dev/documentation/faq.html)
- helix: `jump-label-alphabet` "The characters that are used to generate two character jump labels", default `abcdefghijklmnopqrstuvwxyz`; `mouse` default `true`; `color-modes` colours the mode indicator; statusline modes `NOR`/`INS`/`SEL` — [helix editor config](https://docs.helix-editor.com/editor.html)
- posting jump mode: `Ctrl+O` — "Labels appear over every part of the screen: type the label of the place you want to go" — [posting guide](https://posting.sh/guide/)
- btop: `vim_keys` — "enable h,j,k,l,g,G keys for directional control in lists. Conflicting keys for h:'help' and k:'kill' is accessible while holding shift"; "Full mouse support" — [btop_config.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_config.cpp); [btop README](https://github.com/aristocratos/btop)
- Legacy key encoding: "No way to reliably use multiple modifier keys", "many existing escape codes … are ambiguous", "no reliable way to distinguish single Esc key presses from the start of a escape sequence"; protocol implemented by Alacritty, foot, Ghostty, iTerm2, Microsoft Terminal, Rio, Warp, WezTerm, **xterm.js**, and in libraries including **Crossterm**, Textual, Bubbletea, Tcell; programs include Helix, Neovim, Yazi, Fish, Nushell — [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
- crossterm `KeyboardEnhancementFlags`: `DISAMBIGUATE_ESCAPE_CODES` "Represent Escape and modified keys using CSI-u sequences, so they can be unambiguously read"; `REPORT_EVENT_TYPES` adds Repeat/Release; `REPORT_ALTERNATE_KEYS`; `REPORT_ALL_KEYS_AS_ESCAPE_CODES` — [docs.rs crossterm](https://docs.rs/crossterm/latest/crossterm/event/struct.KeyboardEnhancementFlags.html); `supports_keyboard_enhancement()` "Queries the terminal's support for progressive keyboard enhancement" and "On unix systems, this function will block and possibly time out while `crossterm::event::read` or `crossterm::event::poll` are being called" — [docs.rs crossterm](https://docs.rs/crossterm/latest/crossterm/terminal/fn.supports_keyboard_enhancement.html)
- dystroy (broot author): "many key combinations just don't work, because of your terminal, or of your system"; "there's no way for your application to know whether the user pressed space or shift-space"; he built `print_key` to test empirically — [dystroy.org](https://dystroy.org/blog/keybindings/)
- ratatui 0.30: split into `ratatui-core`, `ratatui-widgets`, `ratatui-crossterm`/`-termion`/`-termwiz`, `ratatui-macros`; new `ratatui::run()`, `init()`, `restore()`; `Rect::layout()`, `Flex::SpaceEvenly`; MSRV 1.86, edition 2024; no OSC 8 / OSC 52 / kitty mention in the highlights — [ratatui v0.30 highlights](https://ratatui.rs/highlights/v030/)
- OSC 8 in ratatui is done by hand: the official example writes `"\x1B]8;;{url}\x07{text}\x1B]8;;\x07"` into cell symbols in 2-character chunks to work around "a bug in the terminal code that incorrectly calculates the width of ANSI escape sequences" — [ratatui hyperlink example](https://github.com/ratatui/ratatui/blob/main/examples/apps/hyperlink/src/main.rs)
- Nielsen #3 for TUIs: "Esc always backs out one level; nothing traps input; long ops are cancellable"; #4 "Same key does the same thing in every view; vim conventions held throughout" — [tui-critique](https://skillsmp.com/creators/kyleking/dotfiles/private-dot-claude-skills-tui-critique)

### Inferences
- **Keep digits for ≤ 5 things, give the rest to `:`**. Ten digit-tabs where `0` means "this machine" and `7` means "fifty sessions" is a maze. A grouping that fits lazygit's 1–5 habit: `1 fleet` (NODES + MAP + TAPE as sub-views cycled with `tab`/`[ ]`), `2 queue`, `3 airflow`, `4 jira`, `5 sessions`, and LOCAL reached by `:local` / `0` / a click, or folded into a HOME screen (see below). Digits stop being advertised in the masthead at < 100 columns; the names stay.
- **A HOME / "what's wrong now" landing**: the INSIGHTS list already ranks worst-first across the fleet; promoting it to the first screen (one line per problem across *all* sources: a red node, a stale job, a failed DAG, an overdue ticket) with `⏎` = go there gives new users one place to start and on-call users the fastest path. k9s's breadcrumbs then give the way back: a `home › nodes › clickhouse3 › query 8f1c` trail in the footer's left, `esc` pops one.
- **One `esc` rule, written down**: `esc` closes the top-most thing (palette, filter, drawer, SQL pane, sub-view), never quits, never "waves a prayer away" (that is `d` today; move it to a toast dismiss).
- **Zoom**: lazygit's `+` cycle maps directly: `+` on the tree hides band + insights + drawer (normal → focus → full), repeat to come back. Cheaper than a "focus mode" key.
- **Jump labels** (helix `gw`, posting `ctrl+o`): after `f`, two-letter labels over every node/user row (`aa`, `ab`…) let a user land on a row without arrows — valuable on a 40-node MAP; cost is one overlay widget and a label alphabet.
- **Prefix collision**: enable `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS)` when `supports_keyboard_enhancement()` says yes (call it *before* the event loop starts, per the blocking caveat), and advertise what it buys in the footer the way GUIDE already does for `⇧⏎`. The desktop shell (xterm.js) supports the protocol, so the desktop app can promise `ctrl+\`, `shift+⏎`, `ctrl+⏎` unconditionally. Where unsupported, fall back to `F1–F10` and say so once in a toast ("this terminal cannot tell ⇧⏎ from ⏎ — use ctrl+j").
- **Mouse**: expected. Clicks on tabs/rows/footer hints (Textual lets footer keys be clicked; make every footer hint a button), wheel to scroll, drag the drawer boundary to resize (store a ratio like lazygit `sidePanelWidth`), OSC 8 links on Airflow runs and Jira keys (open in browser) using the chunked-cell trick until ratatui has first-class hyperlinks. `MOUSE=0` stays for people who want the terminal's selection.

### Gaps
- No study comparing numbered tabs vs palettes in TUIs; the recommendation rests on what k9s/lazygit/yazi chose and on Nielsen #6/#7.
- Whether ratatui 0.30.x has native OSC 8 cell support was not confirmed beyond the example's workaround comment.
- iTerm2's and Terminal.app's current (2026) kitty-protocol status: iTerm2 is on kitty's list; Apple Terminal is not listed.

---

## 4. Search everywhere: fuzzy finders, filter-as-you-type, jump-to, global search

### Takeaway
`/` for in-view filtering is universal (k9s, lazygit, yazi, helix) and k9s shows how far a
single filter line can go (regex, `!` inverse, `-f` fuzzy, `-l` label). A *global* fuzzy
finder across all objects is the television/helix-picker pattern: one input, `nucleo`
matching, a preview pane, channel switching with one key. Both are cheap in ratatui.

### Cited Findings
- k9s filter grammar: `/filter⏎` regex ("ie `fred|blee`"), `/:! filter⏎` inverse ("Keep everything that _doesn't_ match"), `/-l label-selector⏎`, `/-f filter⏎` fuzzy; and inside command mode `:pod /fred⏎` — [k9s commands](https://k9scli.io/topics/commands/)
- lazygit: `startSearch: '/'`, `filteringMenu: <ctrl+s>` — [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- yazi: `f` filter, `/` find next, `s` search by name (fd), `S` search by content (ripgrep) — [yazi quick start](https://yazi-rs.github.io/docs/quick-start)
- helix: `space f` file picker at workspace root, `space F` at cwd, `space b` buffers, `space s` symbols, `space d` diagnostics, `space /` workspace search — [helix keymap](https://docs.helix-editor.com/keymap.html)
- television: "channels" for files, text, git repos, env, aliases, stdin, "switch between them with a keystroke"; matching "fuzzy, substring, prefix and exact"; built on tokio + ratatui + nucleo with "prefix bucketing and SIMD acceleration" — [docs.rs/television](https://docs.rs/television)
- Textual's palette highlights the matched characters in each hit — [Textual command palette](https://textual.textualize.io/guide/command_palette/)
- cobserve today: `/` on NODES filters "by node, user, person, SQL or query id (`esc` clears)"; the session bar's `/` finds a session "by name, folder, kind, server or number" — [GUIDE.md §7.1, §8.1](../../../GUIDE.md)

### Inferences
- **Keep `/` as the local filter** on every list (it already is on NODES and sessions; add it to QUEUE, AIRFLOW, JIRA, TAPE with the same semantics), show the match count in the input line (`/ petrova  3 of 15`), highlight matched characters in rows (Textual), and `esc` clears.
- **Add one global finder**, `ctrl+f` or `:find`/`space f`-style, with channels switchable by `tab`: nodes · users/people · queries (by SQL text and id) · Redash jobs · DAGs/runs · tickets · sessions · actions. `⏎` navigates to the object in its view and pushes a breadcrumb; the right half previews (the node's drawer, the DAG's timeline, the ticket's summary). This is television's model and reuses the palette widget (same input, same matcher, different source).
- **Jump labels** cover the "I can see it, let me land on it" case that `/` does not (see §3).
- Implementation: `nucleo` (or `nucleo-matcher`) for scoring; keep every object's searchable text precomputed per poll so matching never touches the network; the finder reads the same snapshots the views draw.

### Gaps
- No usage data on whether users prefer `/` then type vs a dedicated finder; both are offered by the best tools.

---

## 5. Reducing clutter: progressive disclosure, drawers, density, responsive layouts

### Takeaway
Dense monitors stay readable by *hiding tiers on demand* (lazygit screen modes, btop
presets/`shown_boxes`, k9s `ctrl-w` wide columns and `headless`/`logoless`/`crumbsless`),
by flexible splits that change orientation with width (lazygit `flexible`), and by
degrading glyph resolution rather than content (btop braille → block → tty). cobserve's
"every percentage prints its denominator" rule survives if the denominator moves to a
column header or the drawer at narrow widths rather than every cell.

### Cited Findings
- lazygit: `gui.screenMode` normal/half/full with `+`/`_`; `mainPanelSplitMode: flexible` ("split horizontally if wide enough, otherwise vertically"); `expandFocusedSidePanel` accordion; `showBottomLine: false` hides the key line "except to display a loader for an in-progress action"; `showNumstatInFilesView`; `nerdFontsVersion` "'2' | '3' | empty string (default). If empty, do not show icons." — [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- btop: `presets` "Preset 0 is always all boxes shown with default settings. Max 9 presets. Format: box_name:P:G … G=graph symbol"; `shown_boxes` "cpu mem net proc" and gpu0–5; `graph_symbol` "braille" (highest resolution, "might not be included in all fonts"), "block" ("half the resolution of braille but uses more common characters"), "tty" ("only 3 different symbols … should work in a real TTY" with "half the horizontal resolution"); `force_tty` "force 16-color mode and TTY theme, set all graph symbols to tty and swap out other non tty friendly symbols"; `rounded_corners` "ignored if TTY mode is ON"; `update_ms` "recommended 2000 ms or above for better sample times" — [btop_config.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_config.cpp)
- k9s: `ctrl-w` "toggles wide columns", `ctrl-e` "hides the header"; config `headless`, `logoless`, `crumbsless` — [k9s README](https://github.com/derailed/k9s#readme); [k9s config](https://www.k9scli.io/topics/config)
- Textual Footer has a `compact` mode — [Textual Footer](https://textual.textualize.io/widgets/footer/)
- zellij: `--layout compact` and `simplified-ui` — [zellij FAQ](https://zellij.dev/documentation/faq.html)
- Nielsen #8 for TUIs: "Chrome (borders, padding, decoration) stays subordinate to content"; #10: "usable at 16-color/monochrome, survives tmux/zellij and SSH, no flicker from full redraws" and narrow terminals at 80×24 — [tui-critique](https://skillsmp.com/creators/kyleking/dotfiles/private-dot-claude-skills-tui-critique)
- cobserve today: band's third line needs 20 rows; MAP's "forty-node fleet fits on one screen"; AIRFLOW's timeline is "an hour a cell, half an hour from 160 columns" — [CLAUDE.md](../../../CLAUDE.md); [GUIDE.md §7.3, §7.5](../../../GUIDE.md)

### Inferences
- **Three named densities, one key** (`+` cycle, lazygit): *comfortable* (today's layout, ≥ 140 cols), *compact* (band collapses to one line `FLEET 36% mem · 29% cpu · 15 q · 4 ✕ │ REDASH 16 wait · 3m43s`, day line becomes a 12-column chip in the masthead `● Maghrib in 1h58m`, footer compact), *focus* (body only, footer only). Persist the choice in `config.toml` like lazygit's `screenMode`.
- **Breakpoints, declared**: 80×24 → one pane, no sparklines, denominators in headers (`MEM % of GiB`); 120×36 → today's target; ≥ 160×48 → drawer becomes a right column, timeline half-hour cells (already done for AIRFLOW). Use `Layout` with `Constraint::Min`/`Max`/`Ratio` and a single `fn density(area) -> Density` so every view asks the same question.
- **Glyph tiers like btop**: `braille` sparklines when the font has them, `▁▂▃▅▇` blocks otherwise, `=`/`#` bars in `force_tty`/`TERM=dumb`; one `Glyphs` table, chosen once at start (`COBSERVE_GLYPHS=plain` override). The Tarum theme already has mono/16-colour variants; add the glyph tier beside them.
- **Denominators without noise**: print `59.6/64 GiB` once per node row (as today), but on user/query rows show only the numerator and keep the `%` — the node's capacity is the denominator and is one row above; at 80 columns, the column header carries the unit. The rule "every percentage prints its denominator" is kept by making the denominator visible on screen, not in every cell.
- **Progressive disclosure that already exists** (tree fold, drawer, healthy-nodes fold) should become the *default* more aggressively: start NODES with only hot nodes expanded (it does), start QUEUE with STALE folded unless non-empty, start AIRFLOW with ACTIVITY folded below 36 rows.

### Gaps
- No published measurements of what btop/k9s do at exactly 80×24 beyond the config semantics; behaviour was read from config text, not observed.

---

## 6. Feedback and safety: confirmations, toasts, loading, stale data, errors, undo

### Takeaway
Confirmation style should scale with severity (clig: mild → none, moderate → prompt,
severe → type the name). k9s uses `ctrl-d` "TAB and ENTER to confirm" for delete and a
no-confirm `ctrl-k` for kill; lazygit uses `⏎`/`esc` dialogs whose body states the
consequence ("Your branch has diverged … Press {{.cancelKey}} to cancel, or {{.confirmKey}}
to force push"). Toasts have a settled shape: short, severity-coloured, auto-dismiss in ~5 s
(Textual default), stacked at one corner. Loading must show within 100 ms (clig). Errors are
rewritten for humans with a next step.

### Cited Findings
- clig.dev tiers: "Mild (small local changes): Optional confirmation"; "Moderate (larger changes, deletions, remote operations): Usually prompt; consider dry-run options"; "Severe: … asking them to type something non-trivial such as the name of the thing they're deleting" or `--confirm="name-of-thing"`; "If you change state, tell the user"; "Responsive is more important than fast. Print something to the user in <100ms"; "Catch errors and rewrite them for humans … 'Can't write to file.txt. You might need to make it writable by running chmod +w file.txt.'" — [clig.dev](https://clig.dev/)
- k9s: `ctrl-d` "To delete a resource (TAB and ENTER to confirm)"; `ctrl-k` "To kill a resource (no confirmation dialog)"; `k9s --readonly`; `noExitOnCtrlC` requires `:quit` — [k9s commands](https://k9scli.io/topics/commands/); [k9s README](https://github.com/derailed/k9s#readme)
- lazygit confirmation wording: "Are you sure you want to quit?"; "Your branch has diverged from the remote branch. Press {{.cancelKey}} to cancel, or {{.confirmKey}} to force push."; "Are you sure you want to discard changes to the selected file(s) from this commit?"; toast "File name copied to clipboard"; error "An error occurred! Please create an issue at …"; `confirmOnQuit`, `promptToReturnFromSubprocess` — [english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go); [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- Textual: `notify(message, title='', severity='information', timeout=None, markup=True)`, severities `information`/`warning`/`error`, `NOTIFICATION_TIMEOUT: float = 5` "Default number of seconds to show notifications before removing them", example `self.notify("Message", severity="error", timeout=10)`; a `ToastRack` positions the stack — [Textual App API](https://textual.textualize.io/api/app/); [Textual Toast](https://textual.textualize.io/widgets/toast/)
- yazi has `[confirm]` (title, body, list, yes/no buttons with labels) and `[notify]` (info/warning/error titles) components — [yazi theme](https://yazi-rs.github.io/docs/configuration/theme)
- GitHub CLI replaced the animated spinner with "a static text progress indicator (with a relevant message to the action being taken where possible, falling back to a general 'Working…' message)" in accessible mode — [GitHub blog](https://github.blog/engineering/user-experience/building-a-more-accessible-github-cli/)
- Nielsen #1/#9 for TUIs: "Loading spinners, progress bars, status bar reflects current state"; "Errors render in plain language in-app, not raw stderr/panic dumped to the pane" — [tui-critique](https://skillsmp.com/creators/kyleking/dotfiles/private-dot-claude-skills-tui-critique)
- Ratatui toast crates: `ratatui-comfy-toaster` ("An advanced toast-notification engine"), `ratatui-toaster` ("extremely lightweight"), `tui-overlay` ("drawers, modals, popovers, and toasts"); `tui-popup`, `tui-prompts` — [awesome-ratatui](https://github.com/ratatui/awesome-ratatui)
- cobserve today: `x` then `y` to cancel, with "`x` asks first and says what will happen"; the row shows `⊘` until the worker lets go; every accepted cancel is a tape line; the footer shows "any notice, then how the last poll went"; `↯` means "no numbers, so the value is unknown rather than zero" — [GUIDE.md §6, §7.2](../../../GUIDE.md)

### Inferences
- **Cancel is "moderate" by clig's tiers** (remote, reversible-ish: the job can be re-run; the ClickHouse query keeps running). The current `x` → explanation → `y` is the right tier; make the explanation a real dialog (yazi `[confirm]`: title, body, two buttons) whose body is the three facts the GUIDE already lists — who, which query, and "its ClickHouse query on ch3 keeps running" — with `y`/`⏎` confirm and `esc`/any key cancel. Do **not** add typed confirmation; that is for irreversible deletes. Keep `--readonly` (k9s) as a flag that greys `x` out for people who should never cancel.
- **Toasts, not footer notices**: bottom-right stack, max 3, 5 s default, 10 s for errors (Textual), severity glyph + colour (`✓ copied the link`, `▲ airflow refused the login · retry in 4m58s`, `✖ ch3: no answer within 1.5 s`), `esc` or click dismisses the top one, `:notifications` lists the last 20. Prayer reminders become toasts too (replaces `d`). The footer's right side keeps only the poll line (`9/9 nodes · 0.8 s · read-only`).
- **Loading per source**: each view's title line carries its own freshness and state: `AIRFLOW · read 4 s ago · reading…` with a one-glyph spinner that becomes a static `…` in accessible mode (gh). The band shows the worst source's state. Cobserve already marks `↯` for unknown; add a *stale* tier (data older than 3 polls) drawn dim with `· 47 s old` on the row, never silently frozen.
- **Error wording** = what happened + why + the key: "Jira refused the token (401) — `:setup jira` to paste a new one · `r` to retry". Never a Rust error string in the pane.
- **Undo** is largely moot in a read-only monitor; the two undo-able things are the SQL editor (`ctrl+z`, exists) and a cancel (cannot be undone — say so in the dialog: "This cannot be undone; the job can be run again from Redash").

### Gaps
- k9s flash-message duration/position is not documented in the sources I could reach.
- lazygit's toast position/timeout is not in Config.md; only the strings were confirmed.

---

## 7. Accessibility and comfort: colour-blind safety, themes, NO_COLOR, motion, screen readers, glyphs

### Takeaway
The concrete, shipped practice (GitHub CLI 2025–26, btop, zellij, helix) is: encode state
in a glyph *and* a colour, align colours to the 16 ANSI slots so the user's terminal theme
does the contrast work, offer a motion-free spinner and a numbered/linear prompter for
screen readers, and provide a "simplified"/"tty" glyph set. Screen readers remain poorly
served by any full-screen TUI; the honest answer is a linear fallback (`--plain`/log mode),
not an accessible TUI.

### Cited Findings
- GitHub CLI: "align our color palettes to 4-bit colors so our users can completely customize their experience using their terminal preferences"; the old palette "did not take the terminal's background color into account, leading to low contrast"; accessible prompter via `charmbracelet/huh`; static "Working…" indicator; `gh a11y` in v2.72.0+; "structural information is conveyed in a way that's programmatically determinable—even if no explicit markup is present" — [GitHub blog](https://github.blog/engineering/user-experience/building-a-more-accessible-github-cli/)
- Screen-reader critique of canvas-style TUIs: "the application doesn't just fail; it actively spams you"; "the cursor is teleporting all over the screen"; recommends letting users "hide the cursor" and preferring "a dumb, linear CLI stream" — [xogium.me](https://xogium.me/the-text-mode-lie-why-modern-tuis-are-a-nightmare-for-accessibility)
- btop: `graph_symbol` braille/block/tty tiers; `force_tty` → 16 colours + tty symbols; `truecolor` false → "convert 24-bit colors to 256 color (6x6x6 color cube)"; `theme_background` false for terminal transparency — [btop_config.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_config.cpp)
- zellij `simplified-ui true` "doesn't use these characters" for fonts lacking them — [zellij FAQ](https://zellij.dev/documentation/faq.html)
- lazygit `nerdFontsVersion` "'2' | '3' | empty string (default). If empty, do not show icons." — [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- helix `true-color` "override automatic detection of terminal truecolor support in the event of a false negative", default false; `color-modes` default false — [helix editor config](https://docs.helix-editor.com/editor.html)
- clig.dev: "Disable color if your program is not in a terminal or the user requested it" — check `NO_COLOR`, `TERM=dumb`, `--no-color` — [clig.dev](https://clig.dev/)
- A teamctl-ui onboarding step defines its glyph legend in one line: "● working · ○ idle · ! approval pending · ✕ stopped · ? unknown" — [teamctl-ui onboarding.rs](https://docs.rs/teamctl-ui/0.11.0/src/teamctl_ui/onboarding.rs.html)
- Colour issues (85) and copy/paste problems (75) are top-five terminal frustrations in Evans' survey — [jvns.ca](https://jvns.ca/blog/2025/02/05/some-terminal-frustrations/)
- cobserve today: Tarum theme with light and mono variants, 24-bit/256/16-colour; marks `✖ ▲ ✕ ↯ ↗ ↘ NEW ↻` beside colours — [00-context.md](00-context.md); [120x36-help.txt](docs/screenshots/120x36-help.txt)

### Inferences
- cobserve's "colour means severity, nothing else" plus glyphs already satisfies the colour-blind rule; the gap is *consistency*: `✖` is "critical" in the legend but `✕` is "runaway" — two near-identical glyphs for two meanings. Pick visibly different shapes per meaning (e.g. `✖` critical, `▲` warning, `⚡`/`!` runaway, `↯`/`?` unknown) and document them in a one-line legend in `?` and the onboarding (teamctl's style).
- **Default to the 16 ANSI slots** for severity (red/yellow/green/blue indices), keep Tarum's 24-bit palette as the *accent* layer only, and honour `NO_COLOR` → mono variant, `TERM=dumb`/`force_tty` → plain glyphs. Light theme should be picked by the terminal's reported background (OSC 11 query, with a `THEME=` override) rather than a flag only.
- **Reduced motion**: `MOTION=off` (or `a11y` toggle) turns the spinner into a static `…`, stops sparkline redraw animation (if any), and disables `tachyonfx`-style effects if adopted. gh's `gh a11y` is the precedent for a single switch.
- **Screen readers**: do not promise an accessible TUI. Offer `cobserve --plain` that prints the INSIGHTS list and the band as linear text every poll (log style), which is also what a tmux-less SSH user and a pager want. This is the only pattern the critique endorses.
- **Fonts**: no Nerd Font dependency (lazygit's default is "no icons"); use Unicode block/box/arrow glyphs only, with the `plain` tier for fallback. Keep the owner's `❖`/`◆` logo out of the fallback tier.

### Gaps
- No empirical study of colour-blind legibility of specific terminal glyphs was found; the shape+colour rule is from practitioner guidance (GitHub, arbisoft-type blogs) not measured.
- East Asian width handling in ratatui (for Indonesian none needed) was not researched.

---

## 8. Learning aids: guided tour, searchable `?`, cheat sheet, docs, "explain this number"

### Takeaway
Shipped onboarding in TUIs is modest: lazygit shows a one-time text popup, teamctl-ui runs a
10-step skippable tour tracked by a sentinel file and reopenable by a chord, yazi/helix make
help a searchable picker, Textual exposes a "keys" command in its palette. Nobody researched
here has an "explain this number" affordance; cobserve's rule that every number traces to a
DESIGN.md §5 formula makes it uniquely able to offer one.

### Cited Findings
- lazygit's intro popup ("Thanks for using lazygit! Seriously you rock. Three things to share with you…") is shown once and silenced by `disableStartupPopups` — [english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go); [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- teamctl-ui (ratatui): 10 steps budgeted at "under 90 seconds for a complete skim", "Press any key to advance", `Esc` skips, `t` chord reopens, sentinel `.team/state/ui-tutorial-completed` (presence only) — [teamctl-ui onboarding.rs](https://docs.rs/teamctl-ui/0.11.0/src/teamctl_ui/onboarding.rs.html)
- Textual's default palette includes "Displaying keyboard help" — [Textual command palette](https://textual.textualize.io/guide/command_palette/); posting documents shortcuts under F1 — [posting guide](https://posting.sh/guide/)
- yazi help: `F1`/`~`, with themed `chord`/`action`/`hovered` rows — [yazi quick start](https://yazi-rs.github.io/docs/quick-start); [yazi theme](https://yazi-rs.github.io/docs/configuration/theme)
- lazygit ships per-action tooltips (`CommitTooltip`, `StageTooltip`, `PushTooltip`, `PullTooltip`, `RefreshTooltip`, …) shown in its keybinding menu — [english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go)
- clig.dev: "Lead with examples"; "Display the most common flags and commands at the start of the help text"; "If the user did something wrong and you can guess what they meant, suggest it" — [clig.dev](https://clig.dev/)
- cobserve: "A number you cannot trace to a formula in DESIGN.md §5 is a bug, not a feature"; "Every percentage prints its denominator" — [CLAUDE.md](../../../CLAUDE.md)

### Inferences
- **A 6–8 step tour on first run**, teamctl-style, ≤ 60 s: (1) what you are looking at (masthead/band/body/footer), (2) the glyph legend in one line, (3) `⏎` opens / `esc` backs, (4) `/` filters here, `:` does anything, (5) `?` shows the keys for *this* pane, (6) the one write action and its dialog, (7) where the config lives and how to change it. Sentinel: `~/.local/state/cobserve/tour-done`; reopen with `:tour`. Steps are drawn over the *live* screen with the relevant region highlighted (dim everything else), not as a separate slideshow.
- **`?` as a searchable picker**: typing in the `?` overlay filters action rows (yazi/helix picker behaviour); `⏎` on a row runs it. This makes `?` and `:` the same widget with different initial state — one component to build, one to learn.
- **"Explain this number"**: `e` (or `⏎` on a number cell, or hover ≥ 700 ms with mouse) opens a small popover: the §5 formula, the inputs with their current values, the source table and the poll time — e.g. `93.1% = 59.6 GiB memory_usage ÷ 64 GiB OSMemoryTotal · system.asynchronous_metrics · 2 s ago · DESIGN.md §5.1`. This turns a house rule into the app's most distinctive learning aid and doubles as a debugging tool for FleetLens parity. Implementation: each `Metric` carries a `Provenance { formula_id, inputs: Vec<(name, value, unit, source)>, at }`; the popover renders it. Cost is threading provenance through the stats structs, which the §5 rule already requires conceptually.
- **Man page / docs**: keep GUIDE.md; add `cobserve --help` leading with three examples (clig) and a `:docs` palette entry that opens GUIDE at the right anchor via OSC 8 or `open`.

### Gaps
- No TUI with an "explain this value" popover was found in this research; the pattern is proposed, not borrowed.
- No evidence on tour completion/skip rates in terminal apps.

---

## 9. Studies and write-ups: what the literature says about TUI usability

### Takeaway
There is little formal research; the usable evidence is (a) Julia Evans' 2025 survey of
terminal frustrations, (b) clig.dev's guidelines, (c) Textualize's design statements, (d)
GitHub's accessibility engineering post, (e) practitioner adaptations of Nielsen's
heuristics to TUIs, and (f) the Kitty keyboard-protocol spec as the technical root cause
of many key problems. Charm's "modern CLI" principles page was not located.

### Cited Findings
- Evans' survey ranking (count of mentions): remembering syntax 115, switching systems 91, colour issues 85, keyboard shortcut inconsistencies 84, copy/paste 75, poor discoverability 55 ("hard to learn independently…assorted collection of stuff I've been told"), steep learning curve 44, shell history 42, bad documentation 37, scrollback 36, feels outdated 33 — [jvns.ca](https://jvns.ca/blog/2025/02/05/some-terminal-frustrations/)
- clig.dev's premise: "if a command is going to be used primarily by humans, it should be designed for humans first"; "Human-readable output is paramount" — [clig.dev](https://clig.dev/)
- Textualize on consistency across apps: "build apps that may look quite different but take no time to learn, because once you learn how to use one Textual app, you can use them all" — [Textual 0.37 blog](https://textual.textualize.io/blog/2023/09/15/textual-0370-adds-a-command-palette/)
- A TUI heuristic mapping keeps Nielsen 1–9 and replaces #10 with "Terminal Portability" (`NO_COLOR`, 16-colour, tmux/zellij/SSH, no full-redraw flicker, 80×24) — [tui-critique](https://skillsmp.com/creators/kyleking/dotfiles/private-dot-claude-skills-tui-critique)
- Common discoverability remedies named in community discussion: "a '?' popup (lazygit), context-aware popups during multi-key commands (helix), a command palette with search, a top menu bar with accelerator keys, or a bottom bar with context-available options" — [HN thread via zeli.app](https://zeli.app/story/49479837) (aggregator; secondary)
- Kitty protocol rationale and adopter list (terminals, libraries, programs) — [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
- Will McGugan's "7 lessons from building a modern TUI framework" exists as Talk Python episode 380 (Sept 2022) — [talkpython.fm](https://talkpython.fm/episodes/transcript/380/7-lessons-from-building-a-modern-tui-framework) (the blog URL returned 404; the seven items were not extracted)

### Inferences
- The survey's top items map directly onto cobserve choices: syntax/shortcut memory → palette + scoped `?` + footer; colour → 16-slot alignment + light/mono; copy/paste → OSC 52 copy (`y` already copies links) and shift-select guidance; discoverability → tour + empty states.
- Lacking formal studies, the strongest available "evidence" is convergence: four unrelated, widely used tools (k9s, lazygit, helix, yazi) independently settled on `?` scoped help + `/` filter + `esc` back + `:`/palette. Matching that convention is itself a usability win (Nielsen #4, Textualize's "learn one, use all").

### Gaps
- No peer-reviewed usability study of TUIs (k9s/lazygit/yazi adoption) was found.
- Charm's "A Modern CLI" principles post could not be located; only clig.dev and Charm's company blog surfaced.
- The seven McGugan lessons were not recovered in text form.

---

## Appendix A — Implementation map for ratatui 0.30 (Inference; crates cited above)

| Pattern | State model | Crates / APIs | Terminal needs |
|---|---|---|---|
| Action table (one source for footer, `?`, palette) | `Vec<Action { id, label, keys, scope: Scope, enabled: fn(&App)->bool, run }>` | own code; `nucleo` for matching | none |
| Footer legend | `footer(scope) -> top 5 enabled actions` | ratatui `Line`/`Span`; clickable via mouse `Rect::contains` | mouse capture (crossterm `EnableMouseCapture`) |
| `?` scoped help, searchable | `Overlay::Help { query, scope }` | `tui-input` for the query line; own list | none |
| `:` palette / global finder | `Overlay::Palette { query, channel, hits, preview }` | `nucleo`; study `ratatui-command-palette` 0.1 (experimental) | none |
| Which-key after prefix | `PendingPrefix { since: Instant }` → draw after 250 ms | own 2-column widget (yazi `[which]` layout) | kitty protocol for a reliable prefix key |
| Breadcrumb / esc stack | `Vec<Location>` pushed on every navigation | own | none |
| Toasts | `VecDeque<Toast { text, severity, until: Instant }>` | `ratatui-toaster` / `tui-overlay` or ~80 own lines | none |
| Confirm dialog | `Overlay::Confirm { title, body, yes_key, on_yes }` | `tui-popup` or own | none |
| Density / zoom | `enum Density { Comfortable, Compact, Focus }` cycled by `+` | `Layout`, `Constraint::Min/Max/Ratio`, `Flex` | none |
| Jump labels | `Overlay::Jump { labels: Vec<(String, Target)> }` | own; helix alphabet | none |
| Explain-this-number | `Provenance` on every metric | own popover | none |
| Setup wizard | `Screen::Setup { step, fields, check: Option<Pending> }` | `tui-input`/`tui-prompts`, `keyring`, `toml`+`serde`, `directories`; checks run on the existing async pollers | none |
| Hyperlinks | per-cell OSC 8 chunks | ratatui hyperlink-example technique | OSC 8 terminal (iTerm2, WezTerm, kitty, Ghostty, xterm.js) |
| Copy to clipboard over SSH | emit `\x1b]52;c;<base64>\x07` | own (gitu/zellij precedent) | OSC 52; zellij 0.45+ blocks *reads*, writes fine |
| Reliable `ctrl+\`, `⇧⏎`, `ctrl+⏎` | enable once at start if `supports_keyboard_enhancement()` | crossterm `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES \| REPORT_ALTERNATE_KEYS)` | kitty protocol terminals (list in §3) |
| Accessible / plain | `--plain` linear printer; `MOTION=off`; `NO_COLOR` | own | none |

## Appendix B — Suggested wording (Inference, modelled on the cited strings)

- Footer (NODES, comfortable): `↑↓ move   ⏎ open   / filter   : commands   ? keys`
- Footer (compact): `↑↓ ⏎ / : ?`
- Empty QUEUE: `Redash is not connected · press s to add its URL and API key · 2 to come back later`
- Cancel dialog: title `Cancel Redash job #8213?` body `grigol.gankava · "daily balances" · running 8m31s on worker 3` / `Its query on clickhouse3 keeps running; cobserve never sends KILL QUERY.` / `This cannot be undone. The job can be run again from Redash.` buttons `[y] cancel the job   [esc] keep it`
- Toasts: `✓ link copied`, `▲ airflow refused the login · retrying in 4m58s · r now`, `✖ clickhouse3 · no answer within 1.5 s · CH_TIMEOUT_MS raises it`
- Explain popover: `93.1 % = 59.6 GiB memory_usage ÷ 64 GiB OSMemoryTotal · system.asynchronous_metrics · 2 s ago · §5.1`
- Tour step 2: `Marks: ✖ critical · ▲ warning · ! runaway query · ↯ no answer · ↗↘ trend · NEW just joined — colour only repeats the mark`
- Setup, step 1: `Where is ClickHouse? One or more HTTP addresses, comma-separated — cobserve reads system.clusters from them to find the rest.` then `Checking… 9 nodes answered · ch-node-3 refused the password`
