//! The eight panels (`02-panels.md`): each draws its dashboard form into its box of the grid.
//! The panels a later phase builds draw their frame, title and severity, and an empty body.

pub mod fleet;
pub mod queue;

use super::frame::{self, Title};
use super::paint::Painter;
use super::PanelId;
use crate::app::App;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;

/// A panel's dashboard form in `r`.
pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, id: PanelId, r: Rect, focused: bool) {
    match id {
        PanelId::Fleet => fleet::draw(p, app, theme, r, focused),
        PanelId::Queue => queue::draw(p, app, theme, r, focused),
        other => {
            frame(p, app, theme, other, r, focused);
        }
    }
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
        _ => (Severity::None, String::new()),
    }
}
