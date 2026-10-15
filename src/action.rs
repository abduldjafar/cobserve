//! The new shell's actions (`docs/redesign/04-architecture.md` *Contract 1*): one table that feeds
//! the status bar, `?` and the palette, so the three cannot disagree — and the key router, so no
//! key does something unlisted and no listed key does nothing (`DESIGN.md` §3, made mechanical).
//!
//! It lists the keys of `docs/redesign/05-keys.md` *Everywhere*, and the palette-only actions
//! (no key). The ones whose work comes in a later phase say so in a toast when run; each phase
//! replaces its stubs. `run` takes the key that
//! was pressed, since one action can answer several (`1`–`8` focus a panel).

use crate::app::App;
use crate::ui2::PanelId;
use std::time::SystemTime;

/// Where an action can be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// On the grid and in any zoom.
    Everywhere,
    /// Only while this panel is zoomed. (The overlays — palette, explain, dialog, note, wizard,
    /// tour, the floating pane — take their keys before the table; `ui2/keys.rs`.)
    Zoomed(PanelId),
}

pub struct Action {
    /// `explain`, `zoom`, `cancel-job` — what the palette runs it by.
    pub id: &'static str,
    /// `explain`: the status bar's word for it.
    pub label: &'static str,
    /// `explain this number`: the palette's line for it.
    pub phrase: &'static str,
    /// `the value under the cursor · its §5 formula`.
    pub detail: &'static str,
    /// The keys it answers, as `KeyCode` names them; empty for a palette-only action.
    pub keys: &'static [&'static str],
    /// How the status bar and `?` write its keys: `1-8`, `←→↑↓`.
    pub shown: &'static str,
    pub scope: Scope,
    /// The status bar's order: the lowest ranks of the scope show.
    pub rank: u8,
    pub enabled: fn(&App) -> bool,
    pub run: fn(&mut App, &str),
}

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Action").field("id", &self.id).field("keys", &self.keys).field("scope", &self.scope).finish_non_exhaustive()
    }
}

fn always(_: &App) -> bool {
    true
}

/// What a later phase builds: said, not silently ignored.
fn later(app: &mut App, key: &str) {
    let action = ACTIONS.iter().find(|a| a.keys.contains(&key)).map_or(key, |a| a.label);
    app.shell.toast(format!("{action} — not built yet"), false, SystemTime::now());
}

fn focus_number(app: &mut App, key: &str) {
    if let Some(panel) = key.parse::<u8>().ok().and_then(PanelId::from_number) {
        app.shell.focus_on(panel);
    }
}

/// `esc`: what is open inside a zoomed view (a page, the month's time) first, then the shell's.
fn back(app: &mut App, key: &str) {
    if app.shell.palette.is_none() && app.shell.explain.is_none() && app.shell.filter.is_none() && crate::ui2::views::has_inner(app) {
        crate::ui2::views::pass(app, key);
        return;
    }
    app.shell.back();
}

fn read_now(app: &mut App, _: &str) {
    app.read_now();
    app.shell.toast("reading Airflow and Jira now · ClickHouse and Redash are read every poll", false, SystemTime::now());
}

fn pass(app: &mut App, key: &str) {
    crate::ui2::views::pass(app, key);
}

fn grid_or_local(app: &mut App, _: &str) {
    if !app.shell.back() {
        app.shell.local = true;
        app.set_view(crate::app::View::Local);
    }
}

/// `:theme`: the other Tarum variant than the one on screen.
fn theme_toggle(app: &mut App, _: &str) {
    let from_env = std::env::var("THEME").is_ok_and(|t| t.trim().eq_ignore_ascii_case("light"));
    let light = !app.shell.light.unwrap_or(from_env);
    app.shell.light = Some(light);
    let word = if light { "light · mori, the cloth before the dye" } else { "dark · soga and tarum" };
    app.shell.toast(format!("theme · {word}"), false, SystemTime::now());
}

fn prayer_toggle(app: &mut App, _: &str) {
    match app.shell.prayer_place.take() {
        Some(place) => {
            app.prayers.move_to(Some(place));
            app.shell.toast("prayer line · on", false, SystemTime::now());
        }
        None => {
            app.shell.prayer_place = app.prayers.place.clone();
            app.prayers.move_to(None);
            app.shell.toast("prayer line · off · : prayer brings it back", false, SystemTime::now());
        }
    }
}

pub static ACTIONS: &[Action] = &[
    Action { id: "focus", label: "panel", phrase: "focus a panel", detail: "focus a panel · while zoomed, switch the zoom", keys: &["1", "2", "3", "4", "5", "6", "7", "8"], shown: "1-8", scope: Scope::Everywhere, rank: 1, enabled: always, run: focus_number },
    Action { id: "zoom", label: "open", phrase: "open the focused panel", detail: "the focused panel full-screen · in a zoom, the panel's own open", keys: &["Enter"], shown: "⏎", scope: Scope::Everywhere, rank: 2, enabled: always, run: |app, _| crate::ui2::nav::open(app) },
    Action { id: "choose", label: "choose", phrase: "choose", detail: "move the focused panel's cursor", keys: &["Left", "Right", "Up", "Down", "h", "j", "k", "l", "PageUp", "PageDown", "Home", "End"], shown: "←→↑↓", scope: Scope::Everywhere, rank: 3, enabled: always, run: crate::ui2::nav::choose },
    Action { id: "explain", label: "explain", phrase: "explain this number", detail: "the number under the cursor · its §5 formula and inputs", keys: &["e"], shown: "e", scope: Scope::Everywhere, rank: 4, enabled: always, run: |app, _| crate::ui2::nav::explain_now(app) },
    Action { id: "layout", label: "layout", phrase: "layout · all → fleet → ops", detail: "preset all → fleet → ops", keys: &["p"], shown: "p", scope: Scope::Everywhere, rank: 5, enabled: always, run: |app, _| app.shell.cycle_preset() },
    Action { id: "palette", label: "command", phrase: "command palette", detail: "every action and object, fuzzy", keys: &[":"], shown: ":", scope: Scope::Everywhere, rank: 6, enabled: always, run: |app, _| crate::ui2::keys::open_palette(app, false) },
    Action { id: "keys", label: "keys", phrase: "keys for this screen", detail: "the keys of this scope", keys: &["?"], shown: "?", scope: Scope::Everywhere, rank: 7, enabled: always, run: |app, _| crate::ui2::keys::open_palette(app, true) },
    Action { id: "next-panel", label: "next panel", phrase: "next panel", detail: "the focus to the next panel", keys: &["Tab"], shown: "tab", scope: Scope::Everywhere, rank: 8, enabled: always, run: |app, _| app.shell.step(true) },
    Action { id: "prev-panel", label: "previous panel", phrase: "previous panel", detail: "the focus to the previous panel", keys: &["BackTab"], shown: "⇧tab", scope: Scope::Everywhere, rank: 9, enabled: always, run: |app, _| app.shell.step(false) },
    Action { id: "back", label: "back", phrase: "back · close the top-most thing", detail: "close the top-most thing: overlay, filter, zoom · never quits", keys: &["Esc", "Backspace"], shown: "esc", scope: Scope::Everywhere, rank: 10, enabled: always, run: back },
    Action { id: "grid", label: "grid", phrase: "the grid", detail: "the grid from a zoom · this Mac (`:local`) from the grid", keys: &["0"], shown: "0", scope: Scope::Everywhere, rank: 11, enabled: always, run: grid_or_local },
    Action { id: "zoom-toggle", label: "zoom ↔ grid", phrase: "zoom ↔ grid", detail: "the same as ⏎ and esc on a panel", keys: &["f"], shown: "f", scope: Scope::Everywhere, rank: 12, enabled: always, run: |app, _| app.shell.toggle_zoom() },
    Action { id: "estimate", label: "estimate", phrase: "explain estimate · this query", detail: "EXPLAIN ESTIMATE of the chosen query · read-only", keys: &["E"], shown: "E", scope: Scope::Everywhere, rank: 13, enabled: always, run: later },
    Action { id: "filter", label: "filter", phrase: "filter this list", detail: "the focused list, by substring · esc clears", keys: &["/"], shown: "/", scope: Scope::Everywhere, rank: 14, enabled: always, run: |app, _| crate::ui2::nav::filter(app) },
    Action { id: "float", label: "float a session", phrase: "float the session", detail: "the chosen session over any panel · again: the prefix", keys: &["\\", "ctrl+\\", "ctrl+4"], shown: "\\", scope: Scope::Everywhere, rank: 15, enabled: always, run: |app, _| crate::ui2::sessions::toggle(app) },
    Action { id: "sql", label: "SQL on it", phrase: "SQL on this node", detail: "a SQL session on the chosen node, floating", keys: &["c"], shown: "c", scope: Scope::Everywhere, rank: 16, enabled: always, run: |app, _| crate::ui2::sessions::sql_on_node(app) },
    Action { id: "density", label: "density", phrase: "density · comfortable → compact → focus", detail: "comfortable → compact → focus", keys: &["+"], shown: "+", scope: Scope::Everywhere, rank: 17, enabled: always, run: |app, _| {
        app.shell.cycle_density();
        let word = app.shell.density.name();
        app.shell.toast(format!("density · {word}"), false, SystemTime::now());
    } },
    Action { id: "sort", label: "sort", phrase: "sort · pressure → memory → CPU → name", detail: "the nodes: pressure → memory → CPU → name", keys: &["s"], shown: "s", scope: Scope::Everywhere, rank: 18, enabled: always, run: |app, _| crate::ui2::nav::sort(app) },
    Action { id: "replay-back", label: "a minute back", phrase: "replay · back one minute", detail: "replay one minute back, up to 59", keys: &["["], shown: "[", scope: Scope::Everywhere, rank: 19, enabled: always, run: |app, _| crate::ui2::replay::back(app) },
    Action { id: "replay-forward", label: "a minute forward", phrase: "replay · forward one minute", detail: "replay one minute forward", keys: &["]"], shown: "]", scope: Scope::Everywhere, rank: 20, enabled: always, run: |app, _| crate::ui2::replay::forward(app) },
    Action { id: "live", label: "live", phrase: "back to live", detail: "back from replay to now", keys: &["t"], shown: "t", scope: Scope::Everywhere, rank: 21, enabled: always, run: |app, _| crate::ui2::replay::live(app) },
    Action { id: "pause", label: "pause", phrase: "pause polling", detail: "stop taking new numbers · the clock goes on", keys: &[" "], shown: "space", scope: Scope::Everywhere, rank: 22, enabled: always, run: |app, _| app.paused = !app.paused },
    Action { id: "refresh", label: "read now", phrase: "read now", detail: "every source again now", keys: &["r"], shown: "r", scope: Scope::Everywhere, rank: 23, enabled: always, run: read_now },
    Action { id: "note", label: "note", phrase: "note on the tape", detail: "a note on the tape", keys: &["n"], shown: "n", scope: Scope::Everywhere, rank: 24, enabled: always, run: |app, _| app.shell.note = Some(String::new()) },
    Action { id: "copy-link", label: "copy link", phrase: "copy the row's link", detail: "the row's link · OSC 8 where it works", keys: &["y"], shown: "y", scope: Scope::Everywhere, rank: 25, enabled: always, run: crate::ui2::nav::copy_link },
    Action { id: "snooze", label: "snooze", phrase: "snooze notifications · 1 h", detail: "no notifications for an hour", keys: &["z"], shown: "z", scope: Scope::Everywhere, rank: 26, enabled: always, run: later },
    Action { id: "came-from", label: "where I was", phrase: "back to where I was", detail: "back to where you came from", keys: &["-"], shown: "-", scope: Scope::Everywhere, rank: 27, enabled: always, run: later },
    Action { id: "wall", label: "wall", phrase: "wall · the big numbers alone", detail: "the big numbers alone", keys: &["W"], shown: "W", scope: Scope::Everywhere, rank: 28, enabled: always, run: later },
    Action { id: "quit", label: "quit", phrase: "quit", detail: "leave cobserve", keys: &["q", "ctrl+c"], shown: "q", scope: Scope::Everywhere, rank: 29, enabled: always, run: |app, _| app.quit = true },
    // The zooms' own (`screens/*-zoom-*.txt`): what their status bars offer.
    Action { id: "fleet-node", label: "a node", phrase: "choose a node", detail: "the node rows under the graphs", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Fleet), rank: 1, enabled: always, run: crate::ui2::nav::choose },
    Action { id: "fleet-open", label: "open it", phrase: "open the chosen node", detail: "the node, zoomed", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Fleet), rank: 2, enabled: always, run: |app, _| crate::ui2::nav::open(app) },
    Action { id: "fleet-replay", label: "replay", phrase: "replay a minute", detail: "one minute back or forward on the graphs", keys: &["[", "]"], shown: "[ ]", scope: Scope::Zoomed(PanelId::Fleet), rank: 3, enabled: always, run: |app, key| if key == "[" { crate::ui2::replay::back(app) } else { crate::ui2::replay::forward(app) } },
    Action { id: "fleet-explain", label: "explain", phrase: "explain this number", detail: "the fleet's memory share · its formula", keys: &["e"], shown: "e", scope: Scope::Zoomed(PanelId::Fleet), rank: 4, enabled: always, run: |app, _| crate::ui2::nav::explain_now(app) },
    Action { id: "nodes-choose", label: "choose", phrase: "choose a tile", detail: "across the row, then between rows", keys: &["Left", "Right", "Up", "Down", "h", "j", "k", "l", "PageUp", "PageDown", "Home", "End"], shown: "←→↑↓", scope: Scope::Zoomed(PanelId::Nodes), rank: 1, enabled: always, run: crate::ui2::nav::choose },
    Action { id: "nodes-open", label: "open the node", phrase: "open the chosen node", detail: "the node, zoomed", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Nodes), rank: 2, enabled: always, run: |app, _| crate::ui2::nav::open(app) },
    Action { id: "nodes-sort", label: "sort", phrase: "sort · pressure → memory → CPU → name", detail: "the tiles' order", keys: &["s"], shown: "s", scope: Scope::Zoomed(PanelId::Nodes), rank: 3, enabled: always, run: |app, _| crate::ui2::nav::sort(app) },
    Action { id: "nodes-filter", label: "filter", phrase: "filter the tiles", detail: "by a node's name · esc clears", keys: &["/"], shown: "/", scope: Scope::Zoomed(PanelId::Nodes), rank: 4, enabled: always, run: |app, _| crate::ui2::nav::filter(app) },
    Action { id: "nodes-explain", label: "explain", phrase: "explain this number", detail: "the chosen node's memory share", keys: &["e"], shown: "e", scope: Scope::Zoomed(PanelId::Nodes), rank: 5, enabled: always, run: |app, _| crate::ui2::nav::explain_now(app) },
    Action { id: "node-person", label: "a person", phrase: "choose a person", detail: "the rows of who holds it", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Node), rank: 1, enabled: always, run: crate::ui2::nav::choose },
    Action { id: "node-explain", label: "explain", phrase: "explain this row", detail: "the row's share · its formula and inputs", keys: &["e"], shown: "e", scope: Scope::Zoomed(PanelId::Node), rank: 2, enabled: always, run: |app, _| crate::ui2::nav::explain_now(app) },
    Action { id: "node-sql", label: "SQL on it", phrase: "SQL on this node", detail: "a SQL session on the node, floating", keys: &["c"], shown: "c", scope: Scope::Zoomed(PanelId::Node), rank: 3, enabled: always, run: |app, _| crate::ui2::sessions::sql_on_node(app) },
    Action { id: "node-claude", label: "ask claude", phrase: "ask Claude about this row", detail: "the floating session, the row in its prompt", keys: &["\\"], shown: "\\", scope: Scope::Zoomed(PanelId::Node), rank: 4, enabled: always, run: |app, _| crate::ui2::nav::ask_about_row(app) },
    Action { id: "node-copy", label: "copy link", phrase: "copy the row's link", detail: "the query id, or the node", keys: &["y"], shown: "y", scope: Scope::Zoomed(PanelId::Node), rank: 5, enabled: always, run: crate::ui2::nav::copy_link },
    Action { id: "queue-move", label: "move", phrase: "move through the queue", detail: "running, waiting, stale", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Queue), rank: 1, enabled: always, run: pass },
    Action { id: "queue-open", label: "the node", phrase: "open the job's node", detail: "the ClickHouse query a running job became", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Queue), rank: 2, enabled: always, run: pass },
    Action { id: "cancel-job", label: "cancel job", phrase: "cancel this Redash job", detail: "the job under the cursor in Redash: a dialog, then y", keys: &["x"], shown: "x", scope: Scope::Zoomed(PanelId::Queue), rank: 3, enabled: always, run: pass },
    Action { id: "queue-sql", label: "scroll SQL", phrase: "scroll the job's SQL", detail: "the drawer's SQL, a line at a time", keys: &["J", "K"], shown: "J K", scope: Scope::Zoomed(PanelId::Queue), rank: 4, enabled: always, run: pass },
    Action { id: "airflow-move", label: "move", phrase: "move through the runs", detail: "running, queued, failed, the DAGs", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Airflow), rank: 1, enabled: always, run: pass },
    Action { id: "airflow-open", label: "task log", phrase: "open the run", detail: "its tasks, then a task's log", keys: &["Enter", "Right", "l"], shown: "⏎", scope: Scope::Zoomed(PanelId::Airflow), rank: 2, enabled: always, run: pass },
    Action { id: "airflow-browser", label: "in Airflow", phrase: "open in Airflow", detail: "the run or the DAG in the browser", keys: &["o"], shown: "o", scope: Scope::Zoomed(PanelId::Airflow), rank: 3, enabled: always, run: pass },
    Action { id: "airflow-copy", label: "copy link", phrase: "copy the run's link", detail: "its Airflow address on the clipboard", keys: &["y"], shown: "y", scope: Scope::Zoomed(PanelId::Airflow), rank: 4, enabled: always, run: pass },
    Action { id: "airflow-read", label: "read now", phrase: "read Airflow now", detail: "instead of at the next poll", keys: &["r"], shown: "r", scope: Scope::Zoomed(PanelId::Airflow), rank: 5, enabled: always, run: pass },
    Action { id: "jira-lane", label: "a lane", phrase: "cross to the next lane", detail: "the board's columns", keys: &["Left", "Right", "h", "l"], shown: "←→", scope: Scope::Zoomed(PanelId::Jira), rank: 1, enabled: always, run: pass },
    Action { id: "jira-ticket", label: "a ticket", phrase: "walk the lane", detail: "its tickets, by priority", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Jira), rank: 2, enabled: always, run: pass },
    Action { id: "jira-open", label: "ticket in full", phrase: "the ticket in full", detail: "its page, in here", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Jira), rank: 3, enabled: always, run: pass },
    Action { id: "jira-browser", label: "in Jira", phrase: "open in Jira", detail: "the ticket in the browser", keys: &["o"], shown: "o", scope: Scope::Zoomed(PanelId::Jira), rank: 4, enabled: always, run: pass },
    Action { id: "jira-copy", label: "copy link", phrase: "copy the ticket's link", detail: "its Jira address on the clipboard", keys: &["y"], shown: "y", scope: Scope::Zoomed(PanelId::Jira), rank: 5, enabled: always, run: pass },
    Action { id: "jira-time", label: "the month", phrase: "the month's time", detail: "logged time by day and ticket", keys: &["t"], shown: "t", scope: Scope::Zoomed(PanelId::Jira), rank: 6, enabled: always, run: pass },
    Action { id: "jira-read", label: "read now", phrase: "read Jira now", detail: "instead of at the next poll", keys: &["r"], shown: "r", scope: Scope::Zoomed(PanelId::Jira), rank: 7, enabled: always, run: pass },
    Action { id: "tape-scroll", label: "scroll", phrase: "scroll the tape", detail: "newest first", keys: &["Up", "Down", "j", "k", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Tape), rank: 1, enabled: always, run: pass },
    Action { id: "tape-open", label: "go to it", phrase: "go to what it is about", detail: "the node, the person, the queue", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Tape), rank: 2, enabled: always, run: pass },
    Action { id: "tape-note", label: "note", phrase: "note on the tape", detail: "a line of yours, kept with the events", keys: &["n"], shown: "n", scope: Scope::Zoomed(PanelId::Tape), rank: 3, enabled: always, run: |app, _| app.shell.note = Some(String::new()) },
    Action { id: "tape-export", label: "export report", phrase: "export the incident report", detail: "markdown with every denominator (feature B3)", keys: &["y"], shown: "y", scope: Scope::Zoomed(PanelId::Tape), rank: 4, enabled: always, run: later },
    Action { id: "sessions-choose", label: "a session", phrase: "choose a session", detail: "↑↓ one by one · ←→ the card beside, or the next project · PgUp PgDn a page", keys: &["Up", "Down", "j", "k", "Left", "Right", "h", "l", "PageUp", "PageDown", "Home", "End"], shown: "↑↓", scope: Scope::Zoomed(PanelId::Sessions), rank: 1, enabled: always, run: crate::ui2::nav::choose },
    Action { id: "sessions-open", label: "open it", phrase: "open the session", detail: "its program takes every key — in the preview beside the list, or edge to edge on the grid · ctrl+\\ comes back", keys: &["Enter"], shown: "⏎", scope: Scope::Zoomed(PanelId::Sessions), rank: 2, enabled: always, run: |app, _| crate::ui2::sessions::open_full(app) },
    Action { id: "sessions-spread", label: "full", phrase: "open the session edge to edge", detail: "the whole zoom, the list out of the way · ctrl+\\ comes back", keys: &["f"], shown: "f", scope: Scope::Zoomed(PanelId::Sessions), rank: 8, enabled: always, run: |app, _| crate::ui2::sessions::open_spread(app) },
    Action { id: "sessions-float", label: "float it", phrase: "float the session", detail: "over any panel", keys: &["\\"], shown: "\\", scope: Scope::Zoomed(PanelId::Sessions), rank: 3, enabled: always, run: |app, _| crate::ui2::sessions::open(app) },
    Action { id: "sessions-new", label: "new", phrase: "new session", detail: "the folder picker, then it starts there", keys: &["n"], shown: "n", scope: Scope::Zoomed(PanelId::Sessions), rank: 4, enabled: always, run: |app, _| app.ask_new_session(None) },
    Action { id: "sessions-find", label: "find one", phrase: "find a session", detail: "by name, folder, kind or number", keys: &["/"], shown: "/", scope: Scope::Zoomed(PanelId::Sessions), rank: 5, enabled: always, run: |app, _| app.claude.mode = crate::claude::Mode::Finding { query: String::new(), at: 0 } },
    Action { id: "sessions-rename", label: "rename", phrase: "rename the session", detail: "⏎ keeps the name · esc leaves it", keys: &["r"], shown: "r", scope: Scope::Zoomed(PanelId::Sessions), rank: 7, enabled: always, run: |app, _| {
        let name = app.claude.current().and_then(|s| s.name.clone()).unwrap_or_default();
        app.claude.mode = crate::claude::Mode::Naming(name);
    } },
    Action { id: "sessions-close", label: "close", phrase: "close the session", detail: "x twice: its program ends", keys: &["x"], shown: "x x", scope: Scope::Zoomed(PanelId::Sessions), rank: 6, enabled: always, run: |app, _| crate::ui2::sessions::close(app) },
    Action { id: "setup", label: "setup", phrase: "setup · every source", detail: "ClickHouse, Redash, Airflow, Jira, the day · secrets to the Keychain", keys: &[], shown: "", scope: Scope::Everywhere, rank: 41, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 0) },
    Action { id: "setup-clickhouse", label: "setup clickhouse", phrase: "setup · clickhouse", detail: "seeds, cluster, login → Keychain", keys: &[], shown: "", scope: Scope::Everywhere, rank: 42, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 0) },
    Action { id: "setup-redash", label: "setup redash", phrase: "setup · redash", detail: "admin API key and Redis, for the queue", keys: &[], shown: "", scope: Scope::Everywhere, rank: 43, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 1) },
    Action { id: "setup-airflow", label: "setup airflow", phrase: "setup · airflow", detail: "its web UI's login", keys: &[], shown: "", scope: Scope::Everywhere, rank: 44, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 2) },
    Action { id: "setup-jira", label: "setup jira", phrase: "setup · jira", detail: "a personal access token", keys: &[], shown: "", scope: Scope::Everywhere, rank: 45, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 3) },
    Action { id: "setup-day", label: "setup the day", phrase: "setup · the day", detail: "where prayer times are for, the reminder", keys: &[], shown: "", scope: Scope::Everywhere, rank: 46, enabled: always, run: |app, _| crate::ui2::wizard::open(app, 4) },
    Action { id: "tour", label: "tour", phrase: "tour · the screen in 6 steps", detail: "over the live screen", keys: &[], shown: "", scope: Scope::Everywhere, rank: 47, enabled: always, run: |app, _| app.shell.tour = Some(0) },
    Action { id: "local", label: "local", phrase: "local · this machine", detail: "its CPU, memory and processes, zoomed · 0 from the grid", keys: &[], shown: "", scope: Scope::Everywhere, rank: 48, enabled: always, run: |app, _| { app.shell.local = true; app.set_view(crate::app::View::Local); } },
    Action { id: "theme", label: "theme", phrase: "theme · light ↔ dark", detail: "Tarum: the cloth before and after the dye", keys: &[], shown: "", scope: Scope::Everywhere, rank: 49, enabled: always, run: theme_toggle },
    Action { id: "prayer", label: "prayer line", phrase: "prayer line · on / off", detail: "the day line in the top bar", keys: &[], shown: "", scope: Scope::Everywhere, rank: 50, enabled: always, run: prayer_toggle },
    Action { id: "clock", label: "clock · local ↔ UTC", phrase: "clock · local ↔ UTC", detail: "the servers' own time, or this machine's", keys: &[], shown: "", scope: Scope::Everywhere, rank: 40, enabled: always, run: |app, _| app.time.flip() },
];

/// The actions that can be taken now: those of the scopes in force, enabled, by rank.
pub fn actions_for(app: &App) -> Vec<&'static Action> {
    let zoom = app.shell.zoom;
    let mut out: Vec<&'static Action> = ACTIONS
        .iter()
        .filter(|a| match a.scope {
            Scope::Everywhere => true,
            Scope::Zoomed(p) => zoom == Some(p),
        })
        .filter(|a| (a.enabled)(app))
        .collect();
    out.sort_by_key(|a| a.rank);
    out
}

/// The action a key runs in the scopes in force: a panel's own before the global ones.
pub fn for_key(app: &App, key: &str) -> Option<&'static Action> {
    let found = actions_for(app);
    found.iter().copied().find(|a| a.scope != Scope::Everywhere && a.keys.contains(&key)).or_else(|| found.into_iter().find(|a| a.keys.contains(&key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_twice_in_a_scope_and_no_id_twice() {
        let scopes: Vec<Scope> = ACTIONS.iter().map(|a| a.scope).collect();
        for scope in &scopes {
            let mut seen: Vec<&str> = Vec::new();
            for action in ACTIONS.iter().filter(|a| a.scope == *scope) {
                for key in action.keys {
                    assert!(!seen.contains(key), "{key} twice in {scope:?}");
                    seen.push(key);
                }
            }
        }
        let mut ids: Vec<&str> = ACTIONS.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "an id twice");
        assert!(ACTIONS.iter().all(|a| !a.label.is_empty() && !a.detail.is_empty() && (a.keys.is_empty() || !a.shown.is_empty())));
    }

    #[test]
    fn the_status_bar_order_and_a_key_s_action() {
        let mut app = App::new();
        let first: Vec<&str> = actions_for(&app).iter().take(6).map(|a| a.shown).collect();
        assert_eq!(first, ["1-8", "⏎", "←→↑↓", "e", "p", ":"], "the 120×36 screen's status bar");
        assert!(for_key(&app, "x").is_none(), "cancel: only on the queue, zoomed");

        (for_key(&app, "3").unwrap().run)(&mut app, "3");
        assert_eq!(app.shell.focus, PanelId::Nodes);
        (for_key(&app, "2").unwrap().run)(&mut app, "2");
        (for_key(&app, "Enter").unwrap().run)(&mut app, "Enter");
        assert_eq!(app.shell.zoom, Some(PanelId::Queue));
        assert_eq!(for_key(&app, "x").map(|a| a.id), Some("cancel-job"), "zoomed on the queue: x cancels");
        (for_key(&app, "Esc").unwrap().run)(&mut app, "Esc");
        assert_eq!(app.shell.zoom, None);
        (for_key(&app, "W").unwrap().run)(&mut app, "W");
        assert_eq!(app.shell.toasts.last().map(|t| t.text.as_str()), Some("wall — not built yet"), "a later phase's key says so");
        (for_key(&app, "p").unwrap().run)(&mut app, "p");
        assert_eq!(app.shell.preset, crate::ui2::layout::Preset::Fleet);
        (for_key(&app, "q").unwrap().run)(&mut app, "q");
        assert!(app.quit);
    }
}
