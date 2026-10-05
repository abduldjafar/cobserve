//! View 3: the fleet as a map of tiles.
//!
//! The tree answers "who"; the map answers "where", for a fleet too big to read row by row.
//! Every node is one tile in §2.5's sort order — a card on the chrome's surface with its mark,
//! its two bars, its memory history and its queries — forty nodes on one screen, and the red
//! ones jump out before anything is read.

use super::widgets::{sparkline, thin_bar, Cells, PCT_SHAPE};
use crate::app::{scroll_into_view, App, Hit};
use crate::fmt;
use crate::model::NodeView;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

const TILE_MIN_WIDTH: u16 = 28;
/// Four lines and the gap under them.
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
            app.viewport.hits.borrow_mut().push((rect, Hit::Tile(i)));
            tile(frame, app, theme, rect, node, i == selected);
        }
    });
}

/// A tile: a card on the chrome's surface, the one under the cursor raised and marked at its
/// left, with the node's name and how it is, its two bars and its memory's last minutes.
///
/// ```text
/// ✖ clickhouse3       4q ✕2
/// mem ━━━━━━━━━━━━━━  93.1%
/// cpu ━━━━━━━━━━━━━━  93.4%
/// ▁▂▃▄▅▆▇▇▆▅
/// ```
fn tile(frame: &mut Frame, app: &App, theme: &Theme, rect: Rect, node: &NodeView<'_>, selected: bool) {
    // The gap to the next tile, right and below, is the tile's own.
    let card = Rect::new(rect.x, rect.y, rect.width.saturating_sub(2), rect.height.saturating_sub(1));
    if card.width < 12 || card.height < 3 {
        return;
    }
    let fill = if selected { theme.raised() } else { theme.surface() };
    frame.render_widget(Block::new().style(fill), card);
    let sev = super::nodes::severity_of(node);
    let width = card.width as usize - 2;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // The name, and how many queries it runs — runaways and NEW besides.
    let mut head = Cells::new();
    if sev.is_problem() {
        head.push(format!("{} ", sev.glyph()), theme.sev(sev).add_modifier(Modifier::BOLD));
    }
    let mut tail = Cells::new();
    let queries: usize = node.users.iter().map(|u| u.queries.len()).sum();
    let runaways: usize = node.users.iter().flat_map(|u| u.queries.iter()).filter(|q| q.runaway).count();
    if app.tree.new_nodes.contains(&node.node.name) {
        tail.push("NEW ", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD));
    }
    if node.node.reachable {
        tail.push(format!("{queries}q"), theme.muted());
        if runaways > 0 {
            tail.push(format!(" ✕{runaways}"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
    }
    let name_room = width.saturating_sub(head.width() + tail.width() + 1);
    head.push(fmt::truncate(&node.node.name, name_room), if selected { theme.accent().add_modifier(Modifier::BOLD) } else { theme.strong() });
    head.pad_to(width.saturating_sub(tail.width()));
    head.spans(tail.into_spans());
    lines.push(head.line(width, Style::default()));

    if !node.node.reachable {
        lines.push(Line::from(Span::styled(format!("↯ {}", node.node.down_word()), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD))));
        if let Some(detail) = node.node.down_detail() {
            lines.push(Line::from(Span::styled(fmt::truncate(detail, width), theme.muted())));
        }
    } else {
        // `mem ━━━━━━━━━━━━  91.0%` — the bar takes what the label and the number leave.
        let bar_cells = width.saturating_sub(4 + 7);
        for (label, pct) in [("mem", node.mem_pct), ("cpu", node.cpu_pct)] {
            let s = severity::node(pct);
            let mut cells = Cells::new();
            cells.push(format!("{label} "), theme.faint());
            cells.spans(thin_bar(pct, bar_cells, theme.bar_fill(s), theme));
            cells.cell_right(&fmt::pct(pct), 7, if s.is_problem() { theme.sev(s).add_modifier(Modifier::BOLD) } else { theme.text2() });
            lines.push(cells.line(width, Style::default()));
        }
        if card.height >= 4 {
            let mut cells = Cells::new();
            let lag = severity::lag(node.node.lag_s);
            let mut tail = Cells::new();
            if lag.is_problem() {
                tail.push(format!("lag {}", fmt::dur(node.node.lag_s as f64)), theme.sev(lag).add_modifier(Modifier::BOLD));
            }
            let spark_cells = width.saturating_sub(tail.width() + 2).min(18);
            if spark_cells >= 4 {
                let series = app.history.node(&node.node.name).map(|h| &h.mem_pct);
                cells.spans(sparkline(series, app.history.now(), spark_cells, PCT_SHAPE, |v| theme.bar_fill(severity::node(Some(v)))));
            }
            cells.pad_to(width.saturating_sub(tail.width()));
            cells.spans(tail.into_spans());
            lines.push(cells.line(width, Style::default()));
        }
    }
    // The card's own margin, and the mark of the one under the cursor.
    let lines: Vec<Line<'static>> = lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![if selected { Span::styled("▎", theme.accent()) } else { Span::raw(" ") }];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(fill), card);
}
