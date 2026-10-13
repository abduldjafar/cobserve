//! The eight panels (`02-panels.md`): each draws its dashboard form into its box of the grid.
//! The panels a later phase builds draw their frame, title and severity, and an empty body.

pub mod airflow;
pub mod fleet;
pub mod jira;
pub mod node;
pub mod nodes;
pub mod queue;
pub mod tape;

use super::frame::{self, Title};
use super::paint::Painter;
use super::PanelId;
use crate::app::App;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;

/// A panel's dashboard form in `r`; `tile` is the grid's tile size for NODES.
pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, id: PanelId, r: Rect, focused: bool, tile: (u16, u16)) {
    match id {
        PanelId::Fleet => fleet::draw(p, app, theme, r, focused),
        PanelId::Queue => queue::draw(p, app, theme, r, focused),
        PanelId::Nodes => nodes::draw(p, app, theme, r, focused, tile.0, tile.1),
        PanelId::Node => node::draw(p, app, theme, r, focused),
        PanelId::Airflow => airflow::draw(p, app, theme, r, focused),
        PanelId::Jira => jira::draw(p, app, theme, r, focused),
        PanelId::Tape => tape::draw(p, app, theme, r, focused),
        other => {
            frame(p, app, theme, other, r, focused);
        }
    }
}

/// `host · version`, the way a zoom's title names its source.
fn source(host: Option<String>, version: Option<&str>) -> String {
    let mut parts: Vec<String> = host.into_iter().collect();
    parts.extend(version.map(str::to_string));
    parts.join(" · ")
}

/// A zoom's title and the right end of its top border (`screens/*-zoom-*.txt`).
pub fn zoom_title(app: &App, id: PanelId) -> (String, String) {
    let with = |name: &str, src: String| if src.is_empty() { name.to_string() } else { format!("{name} · {src}") };
    match id {
        PanelId::Fleet => (format!("FLEET · {} min", (crate::history::WINDOW_S / 60.0).round() as u64), "esc back · [ ] replay · t live".to_string()),
        PanelId::Nodes => (format!("NODES · {}", app.snapshot().map_or(0, |s| s.nodes.len())), nodes::zoom_summary(app)),
        PanelId::Node => node::zoom_title(app).unwrap_or_else(|| ("NODE".into(), "esc back".into())),
        PanelId::Queue => {
            let q = &app.queue;
            let right = if q.workers_total > 0 { format!("{} workers, {} busy · esc back", q.workers_total, q.workers_busy) } else { "esc back".into() };
            (with("REDASH QUEUE", source(q.host.clone(), q.version.as_deref())), right)
        }
        PanelId::Airflow => {
            let a = &app.airflow;
            let now = app.now();
            let mut beats = Vec::new();
            for (name, beat) in [("scheduler", &a.health.scheduler), ("triggerer", &a.health.triggerer)] {
                match (beat.status.as_deref(), beat.at) {
                    (Some("healthy"), Some(at)) => beats.push(format!("{name} ● {}", crate::fmt::dur((now - at).max(0) as f64))),
                    (Some("healthy"), None) => beats.push(format!("{name} ●")),
                    (Some(other), _) => beats.push(format!("{name} ✖ {other}")),
                    (None, _) => {}
                }
            }
            beats.push("esc back".into());
            (with("AIRFLOW", source(a.host(), a.version.as_deref())), beats.join(" · "))
        }
        PanelId::Jira => {
            let b = &app.jira;
            let mut src = source(b.host(), b.version.as_deref());
            if let Some(user) = &b.user {
                src = if src.is_empty() { user.clone() } else { format!("{src} · {user}") };
            }
            let (sev, _) = jira::title(app);
            let right = if sev.is_problem() { format!("{} · esc back", jira_due_words(app)) } else { "esc back".into() };
            (with("JIRA", src), right)
        }
        PanelId::Tape => (format!("TAPE · {} · newest first", crate::fmt::plural(app.tape.len(), "line", "lines")), "n note · y export · esc back".to_string()),
        PanelId::Sessions => (format!("SESSIONS · {}", app.claude.list.len()), "esc back".to_string()),
    }
}

/// `✖ 1 overdue · ▲ 1 due soon`.
fn jira_due_words(app: &App) -> String {
    let (over, soon) = jira::due(app);
    let mut parts = Vec::new();
    if over > 0 {
        parts.push(format!("✖ {over} overdue"));
    }
    if soon > 0 {
        parts.push(format!("▲ {soon} due soon"));
    }
    parts.join(" · ")
}

/// A panel zoomed: one frame over the body with the zoom's title, then its zoomed form. Returns
/// the inside when the zoomed form is one of today's views, for `ui2::draw` to draw it there.
pub fn draw_zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, id: PanelId, r: Rect) -> Option<Rect> {
    let (sev, _) = title(app, id);
    let (name, right) = zoom_title(app, id);
    let i = frame::panel(p, theme, r, &Title { number: Some(id.number()), name: &name, sev, right: &right, focused: true });
    if i.width == 0 || i.height == 0 {
        return None;
    }
    if crate::ui2::views::is_view(id) {
        return Some(i);
    }
    p.within(Rect::new(i.x.saturating_sub(1), i.y, i.width + 2, i.height), |p| match id {
        PanelId::Fleet => fleet::draw_zoomed(p, app, theme, i),
        PanelId::Nodes => nodes::draw_zoomed(p, app, theme, i),
        PanelId::Node => node::draw_zoomed(p, app, theme, i),
        _ => {}
    });
    None
}

/// A panel's frame with its title and summary; returns the inside.
pub fn frame(p: &mut Painter<'_>, app: &App, theme: &Theme, id: PanelId, r: Rect, focused: bool) -> Rect {
    let (sev, right) = title(app, id);
    let name = name(app, id);
    // NODE's summary is part of its title.
    let right = if id == PanelId::Node { String::new() } else { right };
    frame::panel(p, theme, r, &Title { number: Some(id.number()), name: &name, sev, right: &right, focused })
}

/// What a panel's title says: its name, or for NODE the chosen node.
pub fn name(app: &App, id: PanelId) -> String {
    match id {
        // The chosen node and its two numbers, as one title: `clickhouse3 · 93.1% of 64 GiB · cpu 93.1%`.
        PanelId::Node => match (super::chosen_node(app), title(app, id).1) {
            (Some(node), summary) if !summary.is_empty() => format!("{node} · {summary}"),
            (Some(node), _) => node,
            (None, _) => "NODE".into(),
        },
        other => other.title().to_string(),
    }
}

/// A panel's severity and the summary at the right of its frame.
pub fn title(app: &App, id: PanelId) -> (Severity, String) {
    match id {
        PanelId::Fleet => fleet::title(app),
        PanelId::Queue => queue::title(app),
        PanelId::Nodes => {
            let worst = app
                .with_view(|v| v.nodes.iter().filter(|n| n.node.reachable).map(|n| severity::node(n.mem_pct).max(severity::node(n.cpu_pct))).max())
                .flatten()
                .unwrap_or(Severity::None);
            let sort = match app.tree.sort {
                crate::model::SortKey::Pressure => "pressure",
                crate::model::SortKey::Mem => "memory",
                crate::model::SortKey::Cpu => "CPU",
                crate::model::SortKey::Name => "name",
            };
            (worst, format!("sorted by {sort} · s"))
        }
        PanelId::Node => {
            let chosen = super::chosen_node(app);
            app.with_view(|v| {
                let node = v.nodes.iter().find(|n| Some(n.name()) == chosen.as_deref())?;
                let sev = severity::node(node.mem_pct).max(severity::node(node.cpu_pct));
                let total = node.node.mem_total.map_or("—".to_string(), |t| format!("{:.0} GiB", crate::fmt::gib(t)));
                Some((sev, format!("{} of {total} · cpu {}", crate::fmt::pct(node.mem_pct), crate::fmt::pct(node.cpu_pct))))
            })
            .flatten()
            .unwrap_or((Severity::None, String::new()))
        }
        PanelId::Airflow => airflow::title(app),
        PanelId::Jira => jira::title(app),
        PanelId::Tape => tape::title(app),
        _ => (Severity::None, String::new()),
    }
}
