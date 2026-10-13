//! What the arrows, `⏎`, `e`, `s` and `/` do in the panel that has them (`01-screen.md` *Focus and
//! zoom*): the tile grid in NODES, the node rows of the zoomed FLEET, the people of NODE. Cursors
//! are kept by identity — a node's name, a row's label — so a new poll under them keeps them.

use crate::app::App;
use crate::provenance;
use crate::ui2::overlay::explain;
use crate::ui2::panels::{node, nodes};
use crate::ui2::PanelId;
use std::time::SystemTime;

/// The panel the keys go to: the zoom, else the focus.
pub fn target(app: &App) -> PanelId {
    app.shell.zoom.unwrap_or(app.shell.focus)
}

fn toast(app: &mut App, text: impl Into<String>) {
    app.shell.toast(text, false, SystemTime::now());
}

/// The nodes in NODES' order, by name.
pub fn node_names(app: &App) -> Vec<String> {
    app.with_view(|v| nodes::order(app, &v.nodes).iter().map(|n| n.name().to_string()).collect()).unwrap_or_default()
}

/// Step through a list of `len` from `at` by the key: one, a page, the ends.
fn step(at: usize, len: usize, key: &str, row: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    match key {
        "Up" | "k" => at.saturating_sub(row),
        "Down" | "j" => (at + row).min(last),
        "Left" | "h" => at.saturating_sub(1),
        "Right" | "l" => (at + 1).min(last),
        "PageUp" => at.saturating_sub(row.max(1) * 2),
        "PageDown" => (at + row.max(1) * 2).min(last),
        "Home" => 0,
        "End" => last,
        _ => at,
    }
}

pub fn choose(app: &mut App, key: &str) {
    match target(app) {
        PanelId::Nodes => {
            let names = node_names(app);
            let chosen = crate::ui2::chosen_node(app);
            let at = names.iter().position(|n| Some(n) == chosen.as_ref()).unwrap_or(0);
            let row = if app.shell.zoom.is_some() { app.shell.zoom_tiles_per_row.get() } else { app.shell.tiles_per_row.get() }.max(1) as usize;
            if let Some(name) = names.get(step(at, names.len(), key, row)) {
                if Some(name) != chosen.as_ref() {
                    app.shell.person = None;
                }
                app.shell.chosen = Some(name.clone());
            }
        }
        PanelId::Fleet => {
            if matches!(key, "Left" | "Right" | "h" | "l") {
                return;
            }
            let names = node_names(app);
            let chosen = crate::ui2::chosen_node(app);
            let at = names.iter().position(|n| Some(n) == chosen.as_ref()).unwrap_or(0);
            if let Some(name) = names.get(step(at, names.len(), key, 1)) {
                app.shell.chosen = Some(name.clone());
            }
        }
        PanelId::Node => {
            if matches!(key, "Left" | "Right" | "h" | "l") {
                return;
            }
            let keys: Vec<String> = node::with_chosen(app, |n| node::rows(n).into_iter().map(|r| r.key).collect()).unwrap_or_default();
            let at = app.shell.person.as_ref().and_then(|p| keys.iter().position(|k| k == p)).unwrap_or(0);
            if let Some(k) = keys.get(step(at, keys.len(), key, 1)) {
                app.shell.person = Some(k.clone());
            }
        }
        PanelId::Queue | PanelId::Airflow | PanelId::Jira | PanelId::Tape => crate::ui2::views::pass(app, key),
        PanelId::Sessions => match key {
            "Up" | "k" | "PageUp" => app.claude.step(false),
            "Down" | "j" | "PageDown" => app.claude.step(true),
            "Home" => {
                if let Some(&first) = app.claude.order().first() {
                    app.claude.select(first);
                }
            }
            "End" => {
                if let Some(&last) = app.claude.order().last() {
                    app.claude.select(last);
                }
            }
            _ => {}
        },
    }
}

/// `⏎`: on the grid, the focused panel full-screen; in a zoom, the panel's own open.
pub fn open(app: &mut App) {
    let Some(zoom) = app.shell.zoom else {
        app.shell.zoom = Some(app.shell.focus);
        return;
    };
    match zoom {
        PanelId::Nodes | PanelId::Fleet => {
            if app.shell.chosen.is_none() {
                app.shell.chosen = crate::ui2::chosen_node(app);
            }
            app.shell.focus = PanelId::Node;
            app.shell.zoom = Some(PanelId::Node);
        }
        // On a person, Claude with the row in its prompt; on the server, its arithmetic.
        PanelId::Node => {
            let on_person = node::with_chosen(app, |n| {
                let rows = node::rows(n);
                !rows[node::cursor(app, &rows)].is_server()
            });
            if on_person == Some(true) { ask_about_row(app) } else { explain_now(app) }
        }
        PanelId::Queue | PanelId::Airflow | PanelId::Jira | PanelId::Tape => crate::ui2::views::pass(app, "Enter"),
        PanelId::Sessions => crate::ui2::sessions::open_full(app),
    }
}

/// `e`: the provenance of the number under the cursor.
pub fn explain_now(app: &mut App) {
    let at = app.snapshot().map_or(SystemTime::now(), |s| s.taken_at);
    let built = match target(app) {
        PanelId::Fleet => app.with_view(|v| {
            let t = crate::model::fleet_totals(v);
            explain::build(&provenance::explain_fleet_mem_pct(&t, at), &explain::polled(app, None))
        }),
        PanelId::Nodes => node::with_chosen(app, |n| explain::build(&provenance::explain_node_mem_pct(n.node, at), &explain::polled(app, Some(n.node)))),
        PanelId::Node => node::with_chosen(app, |n| {
            let rows = node::rows(n);
            let row = &rows[node::cursor(app, &rows)];
            let prov = if row.is_server() {
                Some(provenance::explain_server_mem_pct(n, at))
            } else {
                n.users.iter().find(|u| u.label() == row.key).map(|u| provenance::explain_user_mem_pct(u, n.node, at))
            };
            prov.map(|p| explain::build(&p, &explain::polled(app, Some(n.node))))
        })
        .flatten(),
        _ => None,
    };
    match built {
        Some(e) => app.shell.explain = Some(e),
        None => toast(app, "nothing under the cursor to explain"),
    }
}

/// `s`: the nodes' order, pressure → memory → CPU → name.
pub fn sort(app: &mut App) {
    app.tree.sort = app.tree.sort.next();
    let name = match app.tree.sort {
        crate::model::SortKey::Pressure => "pressure",
        crate::model::SortKey::Mem => "memory",
        crate::model::SortKey::Cpu => "CPU",
        crate::model::SortKey::Name => "name",
    };
    toast(app, format!("sort · {name}"));
}

/// `/`: a filter on the focused list (the tiles, for now).
pub fn filter(app: &mut App) {
    match target(app) {
        PanelId::Nodes | PanelId::Fleet => {
            app.shell.filter = Some(crate::ui2::state::Filter { query: String::new(), typing: true });
        }
        other => toast(app, format!("/ filters the tiles; {} lists keep their own order", other.slug())),
    }
}

/// `y`: the link or the id of what is under the cursor, on the clipboard.
pub fn copy_link(app: &mut App, key: &str) {
    let text = match target(app) {
        PanelId::Airflow | PanelId::Jira => return crate::ui2::views::pass(app, key),
        PanelId::Queue => app.selected_job().map(|j| j.id.clone()),
        PanelId::Node => node::with_chosen(app, |n| {
            let rows = node::rows(n);
            let row = &rows[node::cursor(app, &rows)];
            n.users.iter().find(|u| u.label() == row.key).and_then(|u| u.queries.first()).map_or_else(|| n.name().to_string(), |q| q.query.query_id.clone())
        }),
        PanelId::Fleet | PanelId::Nodes => crate::ui2::chosen_node(app),
        PanelId::Tape | PanelId::Sessions => None,
    };
    match text {
        Some(t) => {
            toast(app, format!("copied {t}"));
            app.copy(t);
        }
        None => toast(app, "nothing under the cursor to copy"),
    }
}

/// `\` on NODE's row (and `⏎` on a person): Claude, floating, with the row in its prompt.
pub fn ask_about_row(app: &mut App) {
    let prompt = node::with_chosen(app, |n| {
        let rows = node::rows(n);
        let row = &rows[node::cursor(app, &rows)];
        let mut text = format!("From cobserve (read-only on ClickHouse): on {}, {} holds {} of the node's memory", n.name(), row.name, crate::fmt::pct(row.mem_pct));
        if let Some(u) = n.users.iter().find(|u| u.label() == row.key)
            && let Some(q) = u.queries.first()
        {
            text.push_str(&format!(
                "; their longest query {} has run {} with {} and read {} / {} rows{}. Its SQL:\n{}",
                q.query.query_id,
                crate::fmt::dur(q.query.elapsed_s),
                crate::fmt::bytes(q.query.memory_bytes),
                crate::fmt::bytes(q.query.read_bytes),
                crate::fmt::rows(q.query.read_rows),
                if q.runaway { " — a runaway (§5.4)" } else { "" },
                crate::sqltext::strip_comments(&q.query.sql).trim()
            ));
        }
        text.push_str("\nWhat should I look at?");
        text
    });
    match prompt {
        Some(p) => crate::ui2::sessions::ask_claude(app, p),
        None => toast(app, "no node chosen"),
    }
}
