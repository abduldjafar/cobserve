//! View 3: the fleet as a map of tiles.
//!
//! The tree answers "who"; the map answers "where", for a fleet too big to read row by row.
//! Every node is one tile in §2.5's sort order, framed in its severity colour, with its two
//! bars, its memory history and its queries — forty nodes on one screen, and the red ones
//! jump out before anything is read.

use super::widgets::{bar, sparkline, Cells, PCT_SHAPE};
use crate::app::{scroll_into_view, App};
use crate::fmt;
use crate::model::NodeView;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

const TILE_MIN_WIDTH: u16 = 26;
const TILE_HEIGHT: u16 = 5;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 || area.width < 10 {
        return;
    }
    let names = app.map_nodes();
    let columns = (area.width / TILE_MIN_WIDTH).max(1) as usize;
    app.viewport.map_columns.set(columns);

    // Title line: what the map is sorted by, and how to move on it.
    let mut title = Cells::new();
    title.push(" MAP", theme.section());
    title.push(
        format!(
            " · {} · sorted by {} · ←→↑↓ move · ⏎ open in view 1 · s sort",
            fmt::plural(names.len(), "node", "nodes"),
            app.tree.sort.label()
        ),
        theme.muted(),
    );
    frame.render_widget(
        Paragraph::new(title.line(area.width as usize, Style::default())),
        Rect::new(area.x, area.y, area.width, 1),
    );

    if names.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  waiting for the first snapshot…", theme.muted())),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
        return;
    }

    let grid_area = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
    let tile_width = grid_area.width / columns as u16;
    let visible_rows = (grid_area.height / TILE_HEIGHT).max(1) as usize;
    let total_rows = names.len().div_ceil(columns);
    let selected = app.map_selection().min(names.len() - 1);
    let offset = scroll_into_view(app.viewport.map.get(), Some(selected / columns), visible_rows, total_rows);
    app.viewport.map.set(offset);

    app.with_view(|view| {
        for (i, name) in names.iter().enumerate().skip(offset * columns).take(visible_rows * columns) {
            let Some(node) = view.nodes.iter().find(|n| &n.node.name == name) else {
                continue;
            };
            let slot = i - offset * columns;
            let (row, col) = (slot / columns, slot % columns);
            let rect = Rect::new(
                grid_area.x + col as u16 * tile_width,
                grid_area.y + row as u16 * TILE_HEIGHT,
                tile_width,
                TILE_HEIGHT.min(grid_area.height.saturating_sub(row as u16 * TILE_HEIGHT)),
            );
            if rect.height < 3 {
                continue;
            }
            tile(frame, app, theme, rect, node, i == selected);
        }
    });
}

fn tile(frame: &mut Frame, app: &App, theme: &Theme, rect: Rect, node: &NodeView<'_>, selected: bool) {
    let sev = super::nodes::severity_of(node);
    let border = if selected {
        theme.accent().add_modifier(Modifier::BOLD)
    } else if sev.is_problem() {
        theme.sev(sev)
    } else {
        theme.border()
    };
    let mut title = vec![Span::styled(
        format!(" {} ", fmt::truncate(&node.node.name, rect.width.saturating_sub(8) as usize)),
        if selected { theme.accent().add_modifier(Modifier::BOLD) } else { theme.strong() },
    )];
    if sev.is_problem() {
        title.push(Span::styled(format!("{} ", sev.glyph()), theme.sev(sev).add_modifier(Modifier::BOLD)));
    }
    if app.tree.new_nodes.contains(&node.node.name) {
        title.push(Span::styled("NEW ", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD)));
    }
    let block = Block::bordered()
        .border_type(if selected { BorderType::Thick } else { BorderType::Rounded })
        .border_style(border)
        .title_top(Line::from(title))
        .style(if selected { theme.selected() } else { Style::default() });
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width < 8 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !node.node.reachable {
        lines.push(Line::from(Span::styled(" ↯ unreachable", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD))));
        if let Some(reason) = &node.node.unreachable_reason {
            lines.push(Line::from(Span::styled(format!(" {}", fmt::truncate(reason, width - 1)), theme.muted())));
        }
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // " MEM ████████▊   91%" — the bar takes what the label and the number leave.
    let bar_cells = width.saturating_sub(4 + 1 + 6 + 1);
    for (label, pct) in [("MEM", node.mem_pct), ("CPU", node.cpu_pct)] {
        let s = severity::node(pct);
        let mut cells = Cells::new();
        cells.push(format!(" {label} "), theme.muted());
        cells.spans(bar(pct, bar_cells, theme.bar_fill(s), theme));
        cells.cell_right(&fmt::pct(pct), 7, theme.sev(s).add_modifier(Modifier::BOLD));
        lines.push(cells.line(width, Style::default()));
    }

    let queries: usize = node.users.iter().map(|u| u.queries.len()).sum();
    let runaways: usize = node.users.iter().flat_map(|u| u.queries.iter()).filter(|q| q.runaway).count();
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    let mut tail = Cells::new();
    tail.push(format!("{queries}q"), theme.text2());
    if runaways > 0 {
        tail.push(format!(" ✕{runaways}"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let lag = severity::lag(node.node.lag_s);
    if lag.is_problem() {
        tail.push(format!(" lag {}", fmt::dur(node.node.lag_s as f64)), theme.sev(lag));
    }
    let spark_cells = width.saturating_sub(tail.width() + 3).min(16);
    if spark_cells >= 4 {
        let series = app.history.node(&node.node.name).map(|h| &h.mem_pct);
        cells.spans(sparkline(series, app.history.now(), spark_cells, PCT_SHAPE, |v| {
            theme.bar_fill(severity::node(Some(v)))
        }));
        cells.push(" ", Style::default());
    }
    let tail_width = tail.width();
    cells.pad_to(width.saturating_sub(tail_width + 1));
    cells.spans(tail.into_spans());
    lines.push(cells.line(width, Style::default()));
    frame.render_widget(Paragraph::new(lines), inner);
}
