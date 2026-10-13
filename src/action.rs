//! The new shell's actions (`docs/redesign/04-architecture.md` *Contract 1*): one table that feeds
//! the status bar, `?` and the palette, so the three cannot disagree — and the key router, so no
//! key does something unlisted and no listed key does nothing (`DESIGN.md` §3, made mechanical).
//!
//! Phase 0 lists the keys of `docs/redesign/05-keys.md` *Everywhere*. The ones whose work comes in
//! a later phase say so in a toast when run; each phase replaces its stubs. `run` takes the key that
//! was pressed, since one action can answer several (`1`–`8` focus a panel).

#![allow(dead_code)]

use crate::app::App;
use crate::ui2::PanelId;
use std::time::SystemTime;

/// An overlay of the new shell, for the actions that live inside one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Palette,
    Keys,
    Explain,
    Float,
    Wizard,
    Dialog,
    Note,
    Tour,
}

/// Where an action can be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// On the grid and in any zoom.
    Everywhere,
    /// While this panel has the focus, on the grid or zoomed.
    Panel(PanelId),
    /// Only while this panel is zoomed.
    Zoomed(PanelId),
    /// Inside an overlay, which takes the keys first.
    Overlay(Overlay),
}

pub struct Action {
    /// `explain`, `zoom`, `cancel-job` — what the palette runs it by.
    pub id: &'static str,
    /// `explain this number`.
    pub label: &'static str,
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

fn zoomed(app: &App) -> bool {
    app.shell.zoom.is_some()
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

fn grid_or_local(app: &mut App, key: &str) {
    if !app.shell.back() {
        later(app, key);
    }
}

pub static ACTIONS: &[Action] = &[
    Action { id: "focus", label: "panel", detail: "focus a panel · while zoomed, switch the zoom", keys: &["1", "2", "3", "4", "5", "6", "7", "8"], shown: "1-8", scope: Scope::Everywhere, rank: 1, enabled: always, run: focus_number },
    Action { id: "zoom", label: "open", detail: "the focused panel full-screen · in a zoom, the panel's own open", keys: &["Enter"], shown: "⏎", scope: Scope::Everywhere, rank: 2, enabled: always, run: |app, _| app.shell.toggle_zoom() },
    Action { id: "choose", label: "choose", detail: "move the focused panel's cursor", keys: &["Left", "Right", "Up", "Down", "h", "j", "k", "l", "PageUp", "PageDown", "Home", "End"], shown: "←→↑↓", scope: Scope::Everywhere, rank: 3, enabled: always, run: later },
    Action { id: "explain", label: "explain", detail: "the number under the cursor · its §5 formula and inputs", keys: &["e"], shown: "e", scope: Scope::Everywhere, rank: 4, enabled: always, run: later },
    Action { id: "layout", label: "layout", detail: "preset all → fleet → ops", keys: &["p"], shown: "p", scope: Scope::Everywhere, rank: 5, enabled: always, run: |app, _| app.shell.cycle_preset() },
    Action { id: "palette", label: "command", detail: "every action and object, fuzzy", keys: &[":"], shown: ":", scope: Scope::Everywhere, rank: 6, enabled: always, run: later },
    Action { id: "keys", label: "keys", detail: "the keys of this scope", keys: &["?"], shown: "?", scope: Scope::Everywhere, rank: 7, enabled: always, run: later },
    Action { id: "next-panel", label: "next panel", detail: "the focus to the next panel", keys: &["Tab"], shown: "tab", scope: Scope::Everywhere, rank: 8, enabled: always, run: |app, _| app.shell.step(true) },
    Action { id: "prev-panel", label: "previous panel", detail: "the focus to the previous panel", keys: &["BackTab"], shown: "⇧tab", scope: Scope::Everywhere, rank: 9, enabled: always, run: |app, _| app.shell.step(false) },
    Action { id: "back", label: "back", detail: "close the top-most thing: overlay, filter, zoom · never quits", keys: &["Esc", "Backspace"], shown: "esc", scope: Scope::Everywhere, rank: 10, enabled: always, run: |app, _| {
        app.shell.back();
    } },
    Action { id: "grid", label: "grid", detail: "the grid from a zoom · this Mac (`:local`) from the grid", keys: &["0"], shown: "0", scope: Scope::Everywhere, rank: 11, enabled: always, run: grid_or_local },
    Action { id: "zoom-toggle", label: "zoom ↔ grid", detail: "the same as ⏎ and esc on a panel", keys: &["f"], shown: "f", scope: Scope::Everywhere, rank: 12, enabled: always, run: |app, _| app.shell.toggle_zoom() },
    Action { id: "estimate", label: "estimate", detail: "EXPLAIN ESTIMATE of the chosen query · read-only", keys: &["E"], shown: "E", scope: Scope::Everywhere, rank: 13, enabled: always, run: later },
    Action { id: "filter", label: "filter", detail: "the focused list, by substring · esc clears", keys: &["/"], shown: "/", scope: Scope::Everywhere, rank: 14, enabled: always, run: later },
    Action { id: "float", label: "float a session", detail: "the chosen session over any panel · again: the prefix", keys: &["\\", "ctrl+\\"], shown: "\\", scope: Scope::Everywhere, rank: 15, enabled: always, run: later },
    Action { id: "sql", label: "SQL on it", detail: "a SQL session on the chosen node, floating", keys: &["c"], shown: "c", scope: Scope::Everywhere, rank: 16, enabled: always, run: later },
    Action { id: "density", label: "density", detail: "comfortable → compact → focus", keys: &["+"], shown: "+", scope: Scope::Everywhere, rank: 17, enabled: always, run: |app, _| app.shell.cycle_density() },
    Action { id: "sort", label: "sort", detail: "the nodes: pressure → memory → CPU → name", keys: &["s"], shown: "s", scope: Scope::Everywhere, rank: 18, enabled: always, run: |app, _| app.tree.sort = app.tree.sort.next() },
    Action { id: "replay-back", label: "a minute back", detail: "replay one minute back, up to 59", keys: &["["], shown: "[", scope: Scope::Everywhere, rank: 19, enabled: always, run: later },
    Action { id: "replay-forward", label: "a minute forward", detail: "replay one minute forward", keys: &["]"], shown: "]", scope: Scope::Everywhere, rank: 20, enabled: always, run: later },
    Action { id: "live", label: "live", detail: "back from replay to now", keys: &["t"], shown: "t", scope: Scope::Everywhere, rank: 21, enabled: always, run: later },
    Action { id: "pause", label: "pause", detail: "stop taking new numbers · the clock goes on", keys: &[" "], shown: "space", scope: Scope::Everywhere, rank: 22, enabled: always, run: |app, _| app.paused = !app.paused },
    Action { id: "refresh", label: "read now", detail: "every source again now", keys: &["r"], shown: "r", scope: Scope::Everywhere, rank: 23, enabled: always, run: later },
    Action { id: "note", label: "note", detail: "a note on the tape", keys: &["n"], shown: "n", scope: Scope::Everywhere, rank: 24, enabled: always, run: later },
    Action { id: "copy-link", label: "copy link", detail: "the row's link · OSC 8 where it works", keys: &["y"], shown: "y", scope: Scope::Everywhere, rank: 25, enabled: always, run: later },
    Action { id: "snooze", label: "snooze", detail: "no notifications for an hour", keys: &["z"], shown: "z", scope: Scope::Everywhere, rank: 26, enabled: always, run: later },
    Action { id: "came-from", label: "where I was", detail: "back to where you came from", keys: &["-"], shown: "-", scope: Scope::Everywhere, rank: 27, enabled: always, run: later },
    Action { id: "wall", label: "wall", detail: "the big numbers alone", keys: &["W"], shown: "W", scope: Scope::Everywhere, rank: 28, enabled: always, run: later },
    Action { id: "quit", label: "quit", detail: "leave cobserve", keys: &["q", "ctrl+c"], shown: "q", scope: Scope::Everywhere, rank: 29, enabled: always, run: |app, _| app.quit = true },
    Action { id: "cancel-job", label: "cancel job", detail: "the job under the cursor in Redash: a dialog, then y", keys: &["x"], shown: "x", scope: Scope::Panel(PanelId::Queue), rank: 4, enabled: zoomed, run: later },
];

/// The actions that can be taken now: those of the scopes in force, enabled, by rank.
pub fn actions_for(app: &App) -> Vec<&'static Action> {
    let focus = app.shell.focus;
    let zoom = app.shell.zoom;
    let mut out: Vec<&'static Action> = ACTIONS
        .iter()
        .filter(|a| match a.scope {
            Scope::Everywhere => true,
            Scope::Panel(p) => p == focus,
            Scope::Zoomed(p) => zoom == Some(p),
            Scope::Overlay(_) => false,
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
        assert!(ACTIONS.iter().all(|a| !a.label.is_empty() && !a.detail.is_empty() && !a.shown.is_empty()));
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
        (for_key(&app, "e").unwrap().run)(&mut app, "e");
        assert_eq!(app.shell.toasts.last().map(|t| t.text.as_str()), Some("explain — not built yet"), "a later phase's key says so");
        (for_key(&app, "p").unwrap().run)(&mut app, "p");
        assert_eq!(app.shell.preset, crate::ui2::layout::Preset::Fleet);
        (for_key(&app, "q").unwrap().run)(&mut app, "q");
        assert!(app.quit);
    }
}
