//! Rendering. `draw` reads `App` and nothing else (§8): no I/O, no network, and no panic on
//! resize — every width and height below is a number we clamp, not an assumption.
//!
//! Colour carries severity and nothing else (§7): users are told apart by rows, never by hue.
//! Every percentage prints its denominator.

use crate::app::{row_title, App, Footer, View};
use crate::attrib;
use crate::model::{
    column_known, FleetSnapshot, FleetUser, Job, JobState, NodeView, QueueRow, UserNode, UserSlice,
};
use crate::tree::{Kind, Payload, Row};
use ratatui::layout::Rect;
use std::time::SystemTime;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// The table header of §1: name column first, then the memory and CPU pairs.
const BAR_CELLS: usize = 20;
/// Below this the CPU bar goes, then LONGEST, then QUERIES (§7).
const NARROW: u16 = 100;
const SHORT: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    None,
    Warn,
    Crit,
}

impl Severity {
    fn style(self) -> Style {
        match self {
            Severity::None => Style::default(),
            Severity::Warn => Style::default().fg(Color::Yellow),
            Severity::Crit => Style::default().fg(Color::Red),
        }
    }

    fn mark(self) -> &'static str {
        match self {
            Severity::None => "",
            Severity::Warn => " ⚠",
            Severity::Crit => " ⚠",
        }
    }
}

/// §7: node memory and CPU.
fn sev_node(value: Option<f64>) -> Severity {
    match value {
        None => Severity::None,
        Some(v) if v >= 90.0 => Severity::Crit,
        Some(v) if v >= 75.0 => Severity::Warn,
        Some(_) => Severity::None,
    }
}

/// §7: a user's share of one node.
fn sev_user_mem(value: Option<f64>) -> Severity {
    match value {
        None => Severity::None,
        Some(v) if v >= 40.0 => Severity::Crit,
        Some(v) if v >= 20.0 => Severity::Warn,
        Some(_) => Severity::None,
    }
}

/// §7: query elapsed, where 30 s is also the runaway mark of §5.4.
fn sev_elapsed(seconds: f64) -> Severity {
    if seconds >= 30.0 {
        Severity::Crit
    } else if seconds >= 5.0 {
        Severity::Warn
    } else {
        Severity::None
    }
}

/// §7: replica lag.
fn sev_lag(seconds: u64) -> Severity {
    if seconds >= 60 {
        Severity::Crit
    } else if seconds >= 10 {
        Severity::Warn
    } else {
        Severity::None
    }
}

/// §7: queue waits — "or workers all busy" is the other way into amber.
fn sev_wait(seconds: u64, saturated: bool) -> Severity {
    if seconds >= 180 {
        Severity::Crit
    } else if seconds >= 60 || saturated {
        Severity::Warn
    } else {
        Severity::None
    }
}

// ---------------------------------------------------------------------------
// Numbers (§7)
// ---------------------------------------------------------------------------

pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn fmt_pct(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.1}%"),
        // A percentage without its denominator is not allowed, so an unknown denominator is
        // not a zero (§2.1).
        None => "—".to_string(),
    }
}

pub fn fmt_dur(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    if total < 60 {
        format!("{total}s")
    } else if total < 3600 {
        format!("{}m{:02}s", total / 60, total % 60)
    } else {
        format!("{}h{:02}m", total / 3600, (total % 3600) / 60)
    }
}

/// A duration that is genuinely unknown prints a dash instead of a zero (§2.1).
fn fmt_opt_dur(seconds: Option<u64>) -> String {
    seconds.map(|s| fmt_dur(s as f64)).unwrap_or_else(|| "—".to_string())
}

/// `58.2 / 64 GB`, or `58.2 GB / —` when the denominator is unknown (§2.1).
fn fmt_with_denominator(used: f64, total: Option<f64>, unit: &str) -> String {
    match total {
        Some(t) => format!("{used:.1} / {t:.0} {unit}"),
        None => format!("{used:.1} {unit} / —"),
    }
}

/// 20 cells, filled = round(pct / 5) (§7).
fn bar(value: Option<f64>) -> String {
    let filled = value
        .map(|v| ((v / 5.0).round() as usize).clamp(0, BAR_CELLS))
        .unwrap_or(0);
    format!("{}{}", "▇".repeat(filled), "░".repeat(BAR_CELLS - filled))
}

/// Collapse whitespace runs into single spaces and truncate to fit (§2.3): never wrap, never
/// let a row exceed one line.
fn collapse_sql(sql: &str, width: usize) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&flat, width)
}

fn truncate(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

/// Right-align a numeric column to `width`.
fn right(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return truncate(text, width);
    }
    format!("{}{}", " ".repeat(width - len), text)
}

/// Trim a row to the terminal width with a trailing `…` (§7): truncate, never wrap.
fn fit_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut used = 0usize;
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len());
    for span in spans {
        let len = span.content.chars().count();
        if used + len <= width {
            used += len;
            out.push(span);
            continue;
        }
        let room = width.saturating_sub(used);
        if room > 1 {
            let style = span.style;
            out.push(Span::styled(truncate(&span.content, room), style));
        }
        break;
    }
    out
}

fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return truncate(text, width);
    }
    format!("{text}{}", " ".repeat(width - len))
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Regions {
    strip: Rect,
    tree: Rect,
    drawer: Rect,
    footer: Rect,
}

/// One outer frame, a hairline under the queue strip and one under the tree, and the drawer's
/// own rule (§1). The header lives on the top border, so the first row inside is the strip.
///
/// Rows are handed out in order; nothing overlaps, because a tree that started on the hairline
/// would hide it.
fn regions(inner: Rect) -> Regions {
    let strip = Rect::new(inner.x, inner.y, inner.width, 1);
    let footer_height = 1u16;
    // Height < 30: the drawer shrinks to one line, then disappears (§7).
    let drawer_height = if inner.height < SHORT {
        0
    } else if inner.height < 30 {
        1
    } else {
        3
    };

    let mut used = 1 // strip
        + 1 // hairline under the strip
        + footer_height
        + drawer_height
        + u16::from(drawer_height > 0); // the drawer's own rule
    let tree_height = inner.height.saturating_sub(used).max(1);

    let tree = Rect::new(inner.x, strip.y + 2, inner.width, tree_height);
    let mut y = tree.y + tree_height + 1;
    let drawer = Rect::new(inner.x, y, inner.width, drawer_height);
    if drawer_height > 0 {
        y += drawer_height;
    }
    let footer = Rect::new(inner.x, y, inner.width, footer_height);
    used += 0; // keeps the accounting above explicit
    let _ = used;

    Regions {
        strip,
        tree,
        drawer,
        footer,
    }
}

fn hairline(area: Rect) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(Color::DarkGray),
    ))
}

/// How long ago the numbers on screen were taken, in the words the drawer uses.
fn age_of(snapshot: &FleetSnapshot) -> String {
    match SystemTime::now().duration_since(snapshot.taken_at) {
        Ok(age) if age.as_secs() < 1 => "less than a second ago".to_string(),
        Ok(age) => format!("{} ago", fmt_dur(age.as_secs_f64())),
        Err(_) => "not yet".to_string(),
    }
}

/// A titled rule, like `── c3e51cb5 · grigol.gankava @ clickhouse3 ──`.
fn titled_rule(area: Rect, title: &str) -> Line<'static> {
    let width = area.width as usize;
    let title = truncate(title, width.saturating_sub(4));
    let mut spans = vec![Span::styled("─", Style::default().fg(Color::DarkGray))];
    if !title.is_empty() {
        spans.push(Span::raw(format!(" {title} ")));
    }
    let used = 1 + title.chars().count() + usize::from(!title.is_empty());
    spans.push(Span::styled(
        "─".repeat(width.saturating_sub(used + 1)),
        Style::default().fg(Color::DarkGray),
    ));
    Line::from(spans)
}

fn draw_regions(frame: &mut Frame, area: Rect, app: &App) {
    let title_left = Span::styled(
        " FLEETLENS ",
        Style::default().add_modifier(Modifier::BOLD),
    );
    let title_right = header_right(app);
    let block = Block::bordered()
        .title(title_left)
        .title_top(title_right.right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let regions = regions(inner);

    render_strip(frame, app, regions.strip);
    frame.render_widget(
        Paragraph::new(hairline(inner)),
        Rect::new(inner.x, regions.tree.y - 1, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(hairline(regions.tree)),
        Rect::new(
            regions.tree.x,
            regions.tree.y + regions.tree.height,
            regions.tree.width,
            1,
        ),
    );

    if app.view.is_placeholder() {
        render_placeholder(frame, app.view, regions.tree);
    } else {
        match app.view {
            View::Nodes => render_nodes(frame, app, regions.tree),
            View::Queue => render_queue(frame, app, regions.tree),
            View::Flow | View::Tape => render_placeholder(frame, app.view, regions.tree),
        }
    }

    if regions.drawer.height > 0 {
        let title = drawer_title(app);
        if regions.drawer.height >= 3 {
            frame.render_widget(
                Paragraph::new(titled_rule(regions.drawer, &title)),
                Rect::new(regions.drawer.x, regions.drawer.y, regions.drawer.width, 1),
            );
            let body = Rect::new(
                regions.drawer.x,
                regions.drawer.y + 1,
                regions.drawer.width,
                regions.drawer.height - 1,
            );
            render_drawer_body(frame, app, body);
        } else {
            // Height < 30: the drawer shrinks to one line instead of jumping (§7).
            frame.render_widget(
                Paragraph::new(titled_rule(regions.drawer, &title)),
                regions.drawer,
            );
        }
    }

    let footer = footer_line(app);
    frame.render_widget(Paragraph::new(footer), regions.footer);
}

fn header_right(app: &App) -> Line<'static> {
    let live = if !app.is_live() {
        Span::styled("○ PAUSED", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("● LIVE", Style::default().fg(Color::Green))
    };
    let clock = utc_clock(app.clock);
    let nodes = app.snapshot().map(|s| s.nodes.len()).unwrap_or(0);
    let mut spans = vec![
        live,
        Span::raw(format!(
            " {clock} · {nodes} nodes · sort {}",
            app.tree.sort.label()
        )),
        Span::raw(" · "),
    ];
    spans.extend(tabs(app.view).spans.iter().cloned());
    Line::from(spans)
}

fn tabs(view: View) -> Line<'static> {
    let mut spans = Vec::new();
    for candidate in View::ALL {
        let active = candidate == view;
        let style = if active {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("[{}]{}", candidate.number(), candidate.title()),
            style,
        ));
    }
    Line::from(spans)
}

/// UTC, HH:MM:SS, without pulling in a date library (§7).
pub fn utc_clock(at: std::time::SystemTime) -> String {
    let secs = at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

fn render_placeholder(frame: &mut Frame, view: View, area: Rect) {
    frame.render_widget(
        Paragraph::new(format!(
            "{} — not built yet (views 3–4 are named so the navigation is stable, DESIGN.md §0)",
            view.title()
        ))
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

// ---------------------------------------------------------------------------
// The queue strip (§1, §6.3)
// ---------------------------------------------------------------------------

fn render_strip(frame: &mut Frame, app: &App, area: Rect) {
    // The right-hand hint takes its own room: `⏎ open [2]` is 11 cells.
    let strip = queue_strip(app);
    let line = Line::from(fit_spans(strip.spans, (area.width as usize).saturating_sub(12)));
    frame.render_widget(Paragraph::new(line), area);
    let hint = Span::styled("⏎ open [2] ", Style::default().fg(Color::DarkGray));
    frame.render_widget(
        Paragraph::new(Line::from(hint).right_aligned()),
        area,
    );
}

/// `REDASH QUEUE  12 waiting · oldest 1m40s ⚠ · workers 6/6 busy ⚠ · 2 failed/5m`, or
/// `REDASH QUEUE  unreachable (HTTP 401)` — never blank (§1, §6.3).
pub fn queue_strip(app: &App) -> Line<'static> {
    let label = Span::styled(
        "REDASH QUEUE  ",
        Style::default().fg(Color::DarkGray),
    );
    if !app.queue.reachable {
        let reason = app
            .queue
            .error
            .clone()
            .unwrap_or_else(|| "no REDASH_URL".to_string());
        return Line::from(vec![
            label,
            Span::styled(format!("unreachable ({reason})"), Severity::Warn.style()),
        ]);
    }

    let queue = app.queue.queue("queries");
    let mut spans = vec![label];
    match queue {
        None => spans.push(Span::styled("no queues reported", Severity::Warn.style())),
        Some(q) => {
            spans.push(Span::raw(format!("{} waiting", q.waiting)));
            let saturated = q.saturated();
            let oldest = q.oldest_wait_s.unwrap_or(0);
            let sev = sev_wait(oldest, saturated);
            spans.push(Span::raw(" · oldest "));
            spans.push(Span::styled(
                match q.oldest_wait_s {
                    Some(s) => fmt_dur(s as f64),
                    None => "—".to_string(),
                },
                sev.style(),
            ));
            spans.push(Span::styled(sev.mark(), sev.style()));
            spans.push(Span::raw(" · workers "));
            let worker_sev = if saturated { Severity::Warn } else { Severity::None };
            spans.push(Span::styled(
                format!("{}/{} {}", q.workers_busy, q.workers_total, state_word(saturated)),
                worker_sev.style(),
            ));
            if q.failed_5m > 0 {
                spans.push(Span::styled(
                    format!(" · {} failed/5m", q.failed_5m),
                    Severity::Warn.style(),
                ));
            }
        }
    }
    Line::from(spans)
}

fn state_word(saturated: bool) -> &'static str {
    if saturated {
        "busy"
    } else {
        "idle"
    }
}

// ---------------------------------------------------------------------------
// View 1: the tree (§2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Columns {
    show_bar: bool,
    show_queries: bool,
    show_longest: bool,
}

fn columns(width: u16) -> Columns {
    // §7: below 100 columns the CPU bar goes first; the columns themselves follow.
    // §7, in order: below 100 the CPU bar goes, then LONGEST, then QUERIES.
    Columns {
        show_bar: width >= NARROW,
        show_longest: width >= NARROW - 12,
        show_queries: width >= NARROW - 24,
    }
}

fn render_nodes(frame: &mut Frame, app: &App, area: Rect) {
    let Some((lines, selected)) = app.with_rows(|_, rows| {
        let cols = columns(area.width);
        let name_width = node_name_width(rows);
        let label_width = user_label_width(rows);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut index_of: Vec<usize> = Vec::new();
        for row in rows {
            index_of.push(lines.len());
            lines.extend(row_lines(row, app, cols, name_width, label_width, area.width));
        }
        index_of.push(lines.len());
        let index = crate::tree::selection_index(rows, app.selected());
        (lines, index.map(|i| index_of[i]))
    }) else {
        frame.render_widget(
            Paragraph::new("waiting for the first snapshot…")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    };

    let scroll = app.scroll().min(lines.len().saturating_sub(1));
    let mut paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    if let Some(index) = selected
        && index >= scroll && index < scroll + area.height as usize {
            paragraph = paragraph.style(Style::default());
        }
    frame.render_widget(paragraph, area);
}

/// The name column is padded to the widest node name in the fleet (§2.1).
fn node_name_width(rows: &[Row<'_>]) -> usize {
    let widest = rows
        .iter()
        .filter_map(|row| match &row.payload {
            Payload::Node(view) => Some(view.name().len()),
            Payload::FleetUser(user) => Some(user.label().len()),
            _ => None,
        })
        .max()
        .unwrap_or(10);
    widest.max("clickhouse-bi".len())
}

/// The person column, sized to the widest label on screen so `r_redash → grigol.gankava` is
/// never cut in half. Capped, or one long partner address would eat the whole width.
fn user_label_width(rows: &[Row<'_>]) -> usize {
    const CAP: usize = 30;
    rows.iter()
        .filter_map(|row| match &row.payload {
            Payload::User { slice, .. } => Some(slice.label().len()),
            Payload::PivotNode { user, .. } => Some(user.label().len()),
            _ => None,
        })
        .max()
        .unwrap_or(18)
        .min(CAP)
}

fn row_lines(
    row: &Row<'_>,
    app: &App,
    cols: Columns,
    name_width: usize,
    label_width: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let selected = app.selected() == Some(&row.id);
    let base = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let indent = "  ".repeat(row.depth as usize);
    // An expanded node prints the column header of §1 on its own line right below itself.
    let mut table_header = None;

    match &row.payload {
        Payload::Node(view) => {
            let open = app.tree.node_expanded(view.name());
            spans.push(Span::styled(
                format!("{indent}{} ", if open { "▾" } else { "▸" }),
                base,
            ));
            spans.push(Span::styled(pad(view.name(), name_width + 1), base));
            if app.tree.new_nodes.contains(view.name()) {
                spans.push(Span::styled(
                    " NEW",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ));
            }
            spans.extend(node_metrics(view));
            if open && row.kind == Kind::Node {
                table_header = Some(Line::from(user_table_header(name_width, label_width, cols)));
            }
        }
        Payload::User { slice, .. } => {
            spans.push(Span::styled(format!("{indent}▸ "), base));
            spans.push(Span::styled(
                pad(&slice.label(), label_width.max("server · caches · merges".len()) + 1),
                base,
            ));
            spans.extend(user_metrics(slice, cols, base));
        }
        Payload::Closing(view) => {
            spans.push(Span::styled(format!("{indent}  "), base));
            // The closing row's label is fixed text: it gets its own width rather than being
            // cut down to whatever the longest person on screen happens to be.
            spans.push(Span::styled(
                pad("server · caches · merges", label_width.max(23) + 1),
                Style::default().fg(Color::DarkGray),
            ));
            spans.extend(server_metrics(view, cols));
        }
        Payload::Folded { names, worst } => {
            spans.push(Span::styled(format!("{indent}▸ "), base));
            spans.push(Span::styled(
                format!(
                    "{} healthy nodes folded — {} (< {:.0}%)",
                    names.len(),
                    row_title(row),
                    *worst
                ),
                Style::default().fg(Color::DarkGray),
            ));
        }
        Payload::FleetUser(user) => {
            let open = app.tree.node_expanded(&user.user);
            spans.push(Span::styled(
                format!("{indent}{} ", if open { "▾" } else { "▸" }),
                base,
            ));
            spans.push(Span::styled(pad(&user.label(), name_width + 1), base));
            spans.extend(fleet_user_metrics(user, cols));
        }
        Payload::PivotNode { user, node } => {
            spans.push(Span::styled(format!("{indent}▸ "), base));
            spans.push(Span::styled(
                pad(&node.node.name, name_width + 4),
                base,
            ));
            spans.extend(pivot_node_metrics(user, node, cols));
        }
        Payload::Query { stat, .. } => {
            spans.push(Span::styled(format!("{indent}  "), base));
            spans.push(Span::styled(
                pad(&stat.query.query_id, 9),
                Style::default().fg(Color::DarkGray),
            ));
            let sev = sev_elapsed(stat.query.elapsed_s);
            let mark = if stat.runaway { " ✕" } else { "" };
            spans.push(Span::styled(
                pad(&format!("{}{mark}", fmt_dur(stat.query.elapsed_s)), 9),
                sev.style(),
            ));
            spans.push(Span::raw(pad(&fmt_bytes(stat.query.memory_bytes), 10)));
            spans.push(Span::raw(format!(
                " {:.1}c ",
                stat.cores.max(0.0)
            )));
            spans.push(Span::styled(
                collapse_sql(&stat.query.sql, (width as usize).saturating_sub(indent.len() + 34)),
                Style::default().fg(Color::DarkGray),
            ));
            // The SQL is already sized to the row; `fit_spans` still trims what is left.
        }
    }

    let mut lines = vec![Line::from(fit_spans(spans, width as usize)).style(base)];
    if let Some(header) = table_header {
        lines.push(header);
    }
    lines
}

/// `mem 58.2 / 64 GB  91% ⚠   cpu 15.1 / 16 cores  94% ⚠   6 running · lag 0s · parts 1.2k`
fn node_metrics(view: &NodeView<'_>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let node = view.node;
    if !node.reachable {
        // §2.6: the row stays, with dashes where the numbers would be and the reason why.
        spans.push(Span::styled("mem — cpu —  ↯ unreachable", Severity::Crit.style()));
        if let Some(reason) = &node.unreachable_reason {
            spans.push(Span::styled(
                format!(" {reason}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        return spans;
    }
    let mem = sev_node(view.mem_pct);
    spans.push(Span::raw("mem "));
    spans.push(Span::raw(fmt_with_denominator(
        gib(node.mem_used),
        node.mem_total.map(gib),
        "GB",
    )));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("{}{}", right(&fmt_pct(view.mem_pct), 6), mem.mark()),
        mem.style(),
    ));

    spans.push(Span::raw("   cpu "));
    spans.push(Span::raw(fmt_with_denominator(
        view.busy_cores.unwrap_or(0.0),
        node.cores,
        "cores",
    )));
    let cpu = sev_node(view.cpu_pct);
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("{}{}", right(&fmt_pct(view.cpu_pct), 6), cpu.mark()),
        cpu.style(),
    ));

    spans.push(Span::raw(format!(
        "   {} running · lag ",
        node.running
    )));
    spans.push(Span::styled(
        fmt_dur(node.lag_s as f64),
        sev_lag(node.lag_s).style(),
    ));
    spans.push(Span::raw(format!(" · parts {}", fmt_parts(node.active_parts))));
    spans
}

fn user_table_header(name_width: usize, label_width: usize, cols: Columns) -> Vec<Span<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    vec![
        Span::styled(
            format!("{}  {}", " ".repeat(name_width + 1), pad("USER → PERSON", label_width + 1)),
            dim,
        ),
        Span::styled(format!("  {} ", right("MEM %", 6)), dim),
        Span::styled(format!(" {}", right("CPU %", 6)), dim),
    ]
    .into_iter()
    .chain(if cols.show_queries {
        Some(Span::styled(format!("  {}", right("QUERIES", 7)), dim))
    } else {
        None
    })
    .chain(if cols.show_longest {
        Some(Span::styled(format!("  {}", right("LONGEST", 8)), dim))
    } else {
        None
    })
    .chain(Some(Span::raw(" ")))
    .collect()
}

fn user_metrics(slice: &UserSlice<'_>, cols: Columns, base: Style) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            format!("{}{}", right(&fmt_pct(slice.mem_pct), 6), sev_user_mem(slice.mem_pct).mark()),
            sev_user_mem(slice.mem_pct).style(),
        ),
        Span::raw("  "),
        Span::styled(bar(slice.mem_pct), bar_style(sev_user_mem(slice.mem_pct))),
    ];
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("{}{}", right(&fmt_pct(slice.cpu_pct), 6), Severity::None.mark()),
        base,
    ));
    if cols.show_bar {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            bar(slice.cpu_pct),
            bar_style(sev_user_mem(slice.cpu_pct)),
        ));
    }
    if cols.show_queries {
        spans.push(Span::raw(format!(
            "  {}",
            right(&slice.queries.len().to_string(), 7)
        )));
    }
    if cols.show_longest {
        let sev = sev_elapsed(slice.longest_s);
        let mark = if slice.runaway { " ✕" } else { "" };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("  {}{mark}", right(&fmt_dur(slice.longest_s), 8)),
            if slice.runaway { Severity::Crit.style() } else { sev.style() },
        ));
    }
    spans
}

fn server_metrics(view: &NodeView<'_>, cols: Columns) -> Vec<Span<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(right(&fmt_pct(view.server_mem_pct), 6), dim),
        Span::raw("  "),
        Span::styled(bar(view.server_mem_pct), dim),
    ];
    spans.push(Span::raw("  "));
    spans.push(Span::styled(right(&fmt_pct(view.server_cpu_pct), 6), dim));
    if cols.show_bar {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(bar(view.server_cpu_pct), dim));
    }
    spans.push(Span::styled(format!("  {}  {}", "—".repeat(4), "—".repeat(4)), dim));
    spans
}

fn fleet_user_metrics(user: &FleetUser<'_>, cols: Columns) -> Vec<Span<'static>> {
    let sev = Severity::None;
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{} ", right(&fmt_bytes(user.mem_bytes), 9)), Style::default()),
        Span::raw(format!(
            " {} on {} node{}",
            user.queries,
            user.nodes.len(),
            if user.nodes.len() == 1 { "" } else { "s" }
        )),
    ];
    if cols.show_longest {
        let mark = if user.runaway { " ✕" } else { "" };
        spans.push(Span::styled(
            format!(" · longest {}{mark}", fmt_dur(user.longest_s)),
            if user.runaway {
                Severity::Crit.style()
            } else {
                sev.style()
            },
        ));
    }
    spans
}

fn pivot_node_metrics(user: &FleetUser<'_>, node: &UserNode<'_>, cols: Columns) -> Vec<Span<'static>> {
    let sev = sev_user_mem(node.mem_pct);
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(right(&fmt_pct(node.mem_pct), 6), sev.style()),
    ];
    if cols.show_bar {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(bar(node.mem_pct), bar_style(sev)));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(right(&fmt_pct(node.cpu_pct), 6), Style::default()));
    if cols.show_bar {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(bar(node.cpu_pct), Style::default()));
    }
    if cols.show_queries {
        spans.push(Span::raw(format!(
            "  {}",
            right(&node.queries.len().to_string(), 7)
        )));
    }
    if cols.show_longest {
        let mark = if node.runaway { " ✕" } else { "" };
        spans.push(Span::styled(
            format!("  {}{mark}", right(&fmt_dur(node.longest_s), 8)),
            if node.runaway { Severity::Crit.style() } else { Style::default() },
        ));
    }
    let _ = user;
    spans
}

fn bar_style(sev: Severity) -> Style {
    match sev {
        // Otherwise the default foreground at reduced intensity (§7).
        Severity::None => Style::default().fg(Color::DarkGray),
        other => other.style(),
    }
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn gib(bytes: u64) -> f64 {
    bytes as f64 / GIB
}

fn fmt_parts(parts: u64) -> String {
    if parts >= 1000 {
        format!("{:.1}k", parts as f64 / 1000.0)
    } else {
        parts.to_string()
    }
}

// ---------------------------------------------------------------------------
// View 2: the queue behind the counts (§2.8)
// ---------------------------------------------------------------------------

fn render_queue(frame: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !app.queue.reachable {
        // The strip above already says this; repeating it would be noise, not information.
        lines.push(Line::from(Span::styled(
            "no queue data — the strip above says why",
            dim,
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    if let Some(host) = app.queue.host() {
        lines.push(Line::from(Span::styled(format!(" {host}"), dim)));
    }
    for queue in &app.queue.queues {
        lines.push(queue_summary(queue));
    }

    // The app owns the ordering and the row numbers, so the cursor and the screen cannot
    // disagree about which job is selected.
    let (waiting, started) = app.queue_sections();


    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(pad("WAITING", 24), dim)));
    lines.push(waiting_header());
    if waiting.is_empty() {
        lines.push(Line::from(Span::styled(
            if app.queue.names_available {
                "  nothing waiting"
            } else {
                "  names unavailable (no REDIS_URL) — counts only"
            },
            dim,
        )));
    }
    for (row, job) in &waiting {
        lines.push(waiting_row(*row + 1, job, app));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        pad("RUNNING · on a worker", 24),
        dim,
    )));
    lines.push(running_header());
    if started.is_empty() {
        lines.push(Line::from(Span::styled("  no jobs on a worker", dim)));
    }
    for (row, job) in &started {
        lines.push(running_row(*row + 1, job, app, app.queue_selection() == Some(*row)));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn queue_summary(queue: &QueueRow) -> Line<'static> {
    let saturated = queue.saturated();
    let oldest = queue.oldest_wait_s.unwrap_or(0);
    let sev = sev_wait(oldest, saturated);
    Line::from(vec![
        Span::styled(pad(&queue.name, 22), Style::default()),
        Span::raw(format!(
            "{} waiting   oldest ",
            right(&queue.waiting.to_string(), 3)
        )),
        Span::styled(
            match queue.oldest_wait_s {
                Some(s) => right(&fmt_dur(s as f64), 7),
                None => right("—", 7),
            },
            sev.style(),
        ),
        Span::raw("   workers "),
        Span::styled(
            right(
                &format!("{}/{} {}", queue.workers_busy, queue.workers_total, state_word(saturated)),
                12,
            ),
            if saturated { Severity::Warn.style() } else { Style::default() },
        ),
        Span::raw(format!("   failed/5m {}", queue.failed_5m)),
    ])
}

fn waiting_header() -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(format!("  {}  ", right("#", 3)), dim),
        Span::styled(format!("{}  ", right("WAIT", 8)), dim),
        Span::styled(pad("USER → PERSON", 26), dim),
        Span::styled(pad("DATA SOURCE", 16), dim),
        Span::styled("QUERY", dim),
    ])
}

fn waiting_row(index: usize, job: &Job, app: &App) -> Line<'static> {
    let saturated = app.queue.queue(&job.queue).is_some_and(QueueRow::saturated);
    let sev = sev_wait(job.age_s, saturated);
    Line::from(vec![
        Span::raw(format!("  {}  ", right(&index.to_string(), 3))),
        Span::styled(
            format!("{}{}  ", right(&fmt_dur(job.age_s as f64), 8), sev.mark()),
            sev.style(),
        ),
        Span::raw(pad(&job.label(), 26)),
        Span::raw(pad(job.data_source.as_deref().unwrap_or("—"), 16)),
        Span::raw(match (job.redash_query_id, &job.query_name) {
            (Some(id), Some(name)) => format!("#{id} {name}"),
            (Some(id), None) => format!("#{id}"),
            (None, Some(name)) => name.clone(),
            (None, None) => "—".to_string(),
        }),
    ])
}

fn running_header() -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(format!("  {}  ", right("#", 3)), dim),
        Span::styled(format!("{}  ", right("RUNNING", 8)), dim),
        Span::styled(pad("USER → PERSON", 26), dim),
        Span::styled(pad("DATA SOURCE", 16), dim),
        Span::styled(pad("QUERY", 22), dim),
        Span::styled("→ CLICKHOUSE", dim),
    ])
}

fn running_row(index: usize, job: &Job, _app: &App, selected: bool) -> Line<'static> {
    let sev = sev_elapsed(job.age_s as f64);
    let target = match job.clickhouse_target() {
        Some((node, id)) => format!("→ {node} · {}…", truncate(id, 6)),
        // A started job with no ClickHouse row is a worker doing something else.
        None => "→ not in ClickHouse yet".to_string(),
    };
    Line::from(vec![
        Span::raw(format!("  {}  ", right(&index.to_string(), 3))),
        Span::styled(
            format!("{}{}  ", right(&fmt_dur(job.age_s as f64), 8), if sev == Severity::Crit { " ✕" } else { "" }),
            sev.style(),
        ),
        Span::raw(pad(&job.label(), 26)),
        Span::raw(pad(job.data_source.as_deref().unwrap_or("—"), 16)),
        Span::raw(pad(
            &match (job.redash_query_id, &job.query_name) {
                (Some(id), Some(name)) => format!("#{id} {name}"),
                (Some(id), None) => format!("#{id}"),
                (None, Some(name)) => name.clone(),
                (None, None) => "—".to_string(),
            },
            22,
        )),
        Span::styled(target, Style::default().fg(Color::DarkGray)),
    ])
    .style(if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    })
}

// ---------------------------------------------------------------------------
// The detail drawer (§2.4)
// ---------------------------------------------------------------------------

fn drawer_title(app: &App) -> String {
    if app.view == View::Queue {
        let rows = app.queue_rows();
        return match app.queue_selection().and_then(|i| rows.get(i)) {
            Some(job) => format!("queue · {} · {}", job.queue, job.label()),
            None => format!("queue · {}", app.queue_explanation()),
        };
    }
    if app.view != View::Nodes {
        return app.view.title().to_lowercase();
    }
    app.with_rows(|_, rows| {
        let index = crate::tree::selection_index(rows, app.selected());
        match index.and_then(|i| rows.get(i)) {
            Some(row) => drawer_title_for(row, app),
            None => "no selection".to_string(),
        }
    })
    .unwrap_or_else(|| "no snapshot yet".to_string())
}

fn drawer_title_for(row: &Row<'_>, app: &App) -> String {
    match &row.payload {
        Payload::Node(view) => {
            if view.node.reachable {
                format!("{} · {}", view.name(), view.node.host)
            } else {
                format!("{} · unreachable", view.name())
            }
        }
        Payload::User { view, slice } => format!("{} · {}", slice.label(), view.name()),
        Payload::Closing(view) => format!("server · caches · merges · {}", view.name()),
        Payload::FleetUser(user) => format!("{} · fleet", user.label()),
        Payload::PivotNode { node, .. } => format!("{} · fleet", node.node.name),
        Payload::Folded { names, .. } => format!("{} healthy nodes folded", names.len()),
        Payload::Query { stat, node, .. } => {
            let _ = app;
            format!("{} · {}", stat.query.query_id, node)
        }
    }
}

fn render_drawer_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.view == View::Queue {
        render_queue_drawer(frame, app, area);
        return;
    }
    let lines = app.with_rows(|_, rows| {
        let index = crate::tree::selection_index(rows, app.selected());
        index
            .and_then(|i| rows.get(i))
            .map(|row| drawer_body(row, app))
            .unwrap_or_else(|| vec![Line::from(Span::styled(
                "nothing selected",
                Style::default().fg(Color::DarkGray),
            ))])
    });
    let Some(lines) = lines else {
        frame.render_widget(
            Paragraph::new("waiting for the first snapshot…")
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

/// View 2's drawer: the question the DA actually has, answered in words.
fn render_queue_drawer(frame: &mut Frame, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines = vec![Line::from(Span::styled(app.queue_explanation(), dim))];
    if let Some(age) = app.queue.age(SystemTime::now()) {
        lines.push(Line::from(Span::styled(
            format!("queue read {}", fmt_dur(age.as_secs_f64()).trim_start_matches("0s").trim()),
            dim,
        )));
    }

    let rows = app.queue_rows();
    match app.queue_selection().and_then(|i| rows.get(i)) {
        None => {}
        Some(job) => match (job.state, job.clickhouse_target()) {
            // A waiting job has not reached ClickHouse yet: there is nothing to kill, and the
            // wait is the queue (§2.8).
            (JobState::Queued, _) => {
                let ahead = app
                    .queue
                    .waiting(&job.queue)
                    .iter()
                    .filter(|other| other.age_s > job.age_s)
                    .count();
                lines.push(Line::from(vec![
                    Span::raw(format!("waiting {}", fmt_dur(job.age_s as f64))),
                    Span::styled(
                        format!(" · {ahead} job{} ahead of it", if ahead == 1 { "" } else { "s" }),
                        Severity::Warn.style(),
                    ),
                    Span::styled(" · nothing to kill: it has not reached ClickHouse", dim),
                ]));
            }
            (JobState::Started, Some((node, query_id))) => lines.push(Line::from(vec![
                Span::raw(format!("running {}", fmt_dur(job.age_s as f64))),
                Span::raw(format!(" · job {} → {node} · {query_id}", job.id)),
                Span::styled(" · ⏎ jumps to the query on view 1", dim),
            ])),
            (JobState::Started, None) => lines.push(Line::from(Span::styled(
                "running · no ClickHouse query matches its Redash number yet",
                dim,
            ))),
        },
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

/// The two or three lines §2.4 asks for, depending on what is selected.
fn drawer_body(row: &Row<'_>, app: &App) -> Vec<Line<'static>> {
    match &row.payload {
        Payload::Node(view) => {
            let node = view.node;
            let mut spans = vec![
                Span::styled(format!("{}:{port}", node.host, port = node.port), Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    " · shard {}/replica {} · {} · up {}",
                    node.shard,
                    node.replica,
                    node.version,
                    fmt_opt_dur(node.uptime_s)
                )),
                Span::raw(format!(
                    " · {} running · lag {} · {} parts",
                    node.running,
                    fmt_dur(node.lag_s as f64),
                    fmt_parts(node.active_parts)
                )),
            ];
            if !node.reachable {
                spans.push(Span::styled(" · ↯ unreachable", Severity::Crit.style()));
                if let Some(reason) = &node.unreachable_reason {
                    spans.push(Span::styled(format!(" ({reason})"), Style::default().fg(Color::DarkGray)));
                }
            }
            spans.push(Span::styled(
                format!(
                    " · mem {} · cpu {}",
                    fmt_pct(view.mem_pct),
                    fmt_pct(view.cpu_pct)
                ),
                Style::default().fg(Color::DarkGray),
            ));
            // §5.3: a partial column is a lie, so say which denominator is missing instead of
            // showing `—` and hoping.
            if !column_known(view) {
                spans.push(Span::styled(
                    format!(
                        " · this node reports {}",
                        match (view.node.mem_total, view.node.cores) {
                            (None, Some(_)) => "no usable memory total, so no memory %",
                            (Some(_), None) => "no usable core count, so no CPU %",
                            _ => "neither a memory total nor a core count",
                        }
                    ),
                    Severity::Warn.style(),
                ));
            }
            let mut lines = vec![Line::from(spans)];
            if let Some(snapshot) = app.snapshot() {
                lines.push(Line::from(Span::styled(
                    format!("numbers from the poll {}", age_of(snapshot)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines
        }
        Payload::User { view, slice } => vec![
            Line::from(vec![
                Span::raw(format!("{} ({})", slice.person.as_deref().unwrap_or("—"), slice.user)),
                Span::raw(format!(
                    " · {} quer{} · Σ mem {} · Σ {:.2} cores",
                    slice.queries.len(),
                    if slice.queries.len() == 1 { "y" } else { "ies" },
                    fmt_bytes(slice.mem_bytes),
                    slice.cores
                )),
                Span::raw(format!(
                    " · {}% of {} · longest {} · runaway at {} of memory",
                    fmt_pct(slice.mem_pct),
                    view.name(),
                    fmt_dur(slice.longest_s),
                    fmt_bytes((0.8 * view.mem_limit as f64) as u64)
                )),
            ]),
            Line::from(Span::styled(
                slice
                    .queries
                    .first()
                    .map(|q| q.query.query_id.clone())
                    .unwrap_or_else(|| "—".to_string()),
                Style::default().fg(Color::DarkGray),
            )),
        ],
        Payload::Closing(view) => vec![Line::from(Span::styled(
            format!(
                "everything on {} that is not a user row: {} of memory, {} of CPU",
                view.name(),
                fmt_pct(view.server_mem_pct),
                fmt_pct(view.server_cpu_pct)
            ),
            Style::default().fg(Color::DarkGray),
        ))],
        Payload::Folded { names, .. } => vec![Line::from(Span::styled(
            format!("space unfold: {}", names.join(" ")),
            Style::default().fg(Color::DarkGray),
        ))],
        Payload::FleetUser(user) => vec![
            Line::from(format!(
                "{} · Σ mem {} · Σ {:.2} cores · {} queries on {} nodes",
                user.label(),
                fmt_bytes(user.mem_bytes),
                user.cores,
                user.queries,
                user.nodes.len()
            )),
            Line::from(Span::styled(
                user.nodes
                    .iter()
                    .map(|n| format!("{} {} mem / {} cpu", n.node.name, fmt_pct(n.mem_pct), fmt_pct(n.cpu_pct)))
                    .collect::<Vec<_>>()
                    .join(" · "),
                Style::default().fg(Color::DarkGray),
            )),
        ],
        Payload::PivotNode { user, node } => vec![
            Line::from(format!(
                "{} on {} · {} mem of the node · {} cpu · {} queries",
                match node.person.as_deref() {
                    Some(person) => attrib::user_label(&user.user, Some(person)),
                    None => user.label(),
                },
                node.node.name,
                fmt_pct(node.mem_pct),
                fmt_pct(node.cpu_pct),
                node.queries.len()
            )),
            Line::from(Span::styled(
                node.queries
                    .iter()
                    .map(|q| format!("{} {}", q.query.query_id, fmt_dur(q.query.elapsed_s)))
                    .collect::<Vec<_>>()
                    .join(" · "),
                Style::default().fg(Color::DarkGray),
            )),
        ],
        Payload::Query { stat, node, user } => {
            let query = stat.query;
            let mut spans = vec![
                Span::raw(query.query_id.clone()),
                Span::raw(format!(" · {}", crate::attrib::user_label(user, query.person.as_deref()))),
            ];
            if let Some(redash_id) = query.redash_query_id {
                spans.push(Span::raw(format!(" · Redash #{redash_id}")));
            }
            spans.push(Span::raw(format!(" · {node}")));
            spans.push(Span::raw(format!(" · {}", fmt_dur(query.elapsed_s))));
            spans.push(Span::raw(format!(" · {}", fmt_bytes(query.memory_bytes))));
            spans.push(Span::raw(format!(" · {:.1} cores", stat.cores)));
            spans.push(Span::styled(
                format!(
                    " · {} / {} rows",
                    fmt_bytes(query.read_bytes),
                    fmt_rows(query.read_rows)
                ),
                Style::default().fg(Color::DarkGray),
            ));
            if stat.runaway {
                spans.push(Span::styled("  ✕ runaway", Severity::Crit.style()));
            }
            let _ = app;
            vec![
                Line::from(spans),
                Line::from(Span::styled(
                    collapse_sql(&query.sql, 400),
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        }
    }
}

fn fmt_rows(rows: u64) -> String {
    if rows >= 1_000_000_000 {
        format!("{:.1}B", rows as f64 / 1e9)
    } else if rows >= 1_000_000 {
        format!("{:.1}M", rows as f64 / 1e6)
    } else if rows >= 1_000 {
        format!("{:.1}k", rows as f64 / 1e3)
    } else {
        rows.to_string()
    }
}

// ---------------------------------------------------------------------------
// Footer and help (§3)
// ---------------------------------------------------------------------------

fn footer_line(app: &App) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    match app.footer {
        Footer::Filter => Line::from(vec![
            Span::styled("filter  ", dim),
            Span::raw(format!(
                "/{}",
                app.filter_input().unwrap_or_default()
            )),
            Span::styled("   ⏎ keep   esc clear", dim),
        ]),
        Footer::Help => Line::from(Span::styled(
            "? close   any other key goes back to the tree",
            dim,
        )),
        Footer::Keys => match app.notice() {
            Some(notice) => Line::from(vec![
                Span::styled("⚠ ", Severity::Warn.style()),
                Span::styled(notice.to_string(), Severity::Warn.style()),
            ]),
            None => Line::from(Span::styled(
                "↑↓ move   ⏎ expand   u pivot by user   / filter   s sort   p pause   1-4 view   q quit",
                dim,
            )),
        },
    }
}

/// `?` — the keymap of §3, plus the two things that are deliberately absent this pass.
const HELP: &[(&str, &str)] = &[
    ("↑ ↓ / j k", "move the cursor"),
    ("⏎", "expand / collapse the selected node or user"),
    ("← →", "collapse / expand, vim-style"),
    ("space", "toggle the healthy fold"),
    ("u", "pivot node ↔ user"),
    ("s", "cycle sort: pressure, mem, cpu, name"),
    ("/", "filter node, user, person, SQL · esc clears"),
    ("p", "pause polling"),
    ("1 … 4", "views: nodes, queue, flow, tape"),
    ("?", "this help"),
    ("q / ctrl-c", "quit"),
];

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = 60.min(area.width.saturating_sub(4)).max(20);
    let height = (HELP.len() as u16 + 3).min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let mut lines: Vec<Line<'static>> = HELP
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("{key:>10}"), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("  {what}")),
            ])
        })
        .collect();
    lines.push(Line::from(Span::styled(
        "not in this pass: k kill, a ask Klikas, mouse",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys ")),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    draw_regions(frame, area, app);
    if app.help {
        draw_help(frame, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app_with_fake() -> App {
        let mut fake = FakeSource::new();
        let mut app = App::new();
        app.update(crate::app::Event::Snapshot(Box::new(fake.snapshot())));
        app.update(crate::app::Event::Queue(Box::new(fake.queue())));
        app
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .chunks(width as usize)
            .map(|row| row.iter().copied().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_screen_renders_at_the_target_size() {
        let app = app_with_fake();
        let screen = render(&app, 120, 36);
        let lines: Vec<&str> = screen.lines().collect();

        assert!(lines[0].contains("FLEETLENS"), "{}", lines[0]);
        assert!(lines[0].contains("LIVE") || lines[0].contains("PAUSED"));
        assert!(lines[0].contains("[1]NODES"), "the tabs are in the header");
        assert!(lines[1].contains("REDASH QUEUE"), "{}", lines[1]);
        assert!(lines[1].contains("waiting"), "{}", lines[1]);

        let all = screen.replace('\n', " ");
        assert!(all.contains("clickhouse3"), "a node row");
        assert!(all.contains("mem ") && all.contains("/ 64 GB"), "denominators");
        assert!(all.contains("r_redash → grigol.gankava"), "a user row");
        assert!(all.contains("server · caches · merges"), "the closing row");
        assert!(all.contains("healthy nodes folded"), "the fold line");
        assert!(all.contains("USER → PERSON"), "the column header");
        assert!(all.contains("up"), "footer verbs");
    }

    #[test]
    fn it_renders_at_the_minimum_and_below_it_without_panicking() {
        let app = app_with_fake();
        for (width, height) in [(100u16, 30u16), (80, 24), (60, 18), (40, 10), (20, 5), (10, 3)] {
            let screen = render(&app, width, height);
            assert_eq!(screen.lines().count(), height as usize, "{width}x{height}");
        }
    }

    #[test]
    fn narrow_terminals_drop_columns_in_the_documented_order() {
        let app = app_with_fake();

        let wide = render(&app, 120, 36);
        assert!(wide.contains("QUERIES") && wide.contains("LONGEST"));

        // Below 100 the CPU bar goes first; the number stays.
        let no_bar = render(&app, 96, 36);
        assert!(
            no_bar.matches('▇').count() < wide.matches('▇').count(),
            "one bar per row instead of two"
        );
        assert!(no_bar.contains("CPU %"), "the CPU header is still there");
        assert!(no_bar.contains("LONGEST"), "...but LONGEST survives the first cut");

        // Then LONGEST.
        let no_longest = render(&app, 86, 36);
        assert!(!no_longest.contains("LONGEST"), "LONGEST is the second to go");
        assert!(no_longest.contains("QUERIES"), "QUERIES is last");

        // Then QUERIES.
        let bare = render(&app, 74, 36);
        assert!(!bare.contains("QUERIES"), "QUERIES goes last");
        assert!(bare.contains("r_redash"), "the rows themselves never disappear");
    }

    #[test]
    fn short_terminals_shrink_the_drawer() {
        let app = app_with_fake();
        let tall = render(&app, 120, 36);
        assert!(tall.contains("shard"), "the drawer shows node detail");

        let short = render(&app, 120, 28);
        assert!(!short.contains("shard"), "below 30 rows the drawer shrinks");
    }

    #[test]
    fn the_queue_view_lists_people_and_the_stitch_to_clickhouse() {
        let mut app = app_with_fake();
        app.update(crate::app::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('2'),
            crossterm::event::KeyModifiers::NONE,
        )));
        let screen = render(&app, 120, 36);
        assert!(screen.contains("WAITING"), "{screen}");
        assert!(screen.contains("RUNNING"), "{screen}");
        assert!(screen.contains("r_redash → r.simonyte"), "a waiting person");
        assert!(screen.contains("→ clickhouse3"), "the arrow into ClickHouse");
        assert!(screen.contains("6/6 busy"), "worker saturation");
    }

    #[test]
    fn an_unreachable_queue_still_says_something() {
        let mut app = App::new();
        app.update(crate::app::Event::Queue(Box::new(
            crate::model::QueueStatus::unreachable("HTTP 401"),
        )));
        let screen = render(&app, 120, 36);
        assert!(screen.contains("unreachable (HTTP 401)"), "{screen}");
    }

    #[test]
    fn a_placeholder_view_says_it_is_not_built() {
        let mut app = app_with_fake();
        app.view = View::Flow;
        let screen = render(&app, 120, 36);
        assert!(screen.contains("FLOW"), "{screen}");
        assert!(screen.contains("not built yet"), "{screen}");
    }

    #[test]
    fn the_help_overlay_lists_the_keymap() {
        let mut app = app_with_fake();
        app.help = true;
        let screen = render(&app, 120, 36);
        assert!(screen.contains("keys"), "{screen}");
        assert!(screen.contains("pivot"), "{screen}");
        assert!(screen.contains("filter"), "{screen}");
        assert!(screen.contains("not in this pass"), "unbuilt keys stay out (§3)");
    }

    #[test]
    fn a_filter_is_shown_in_the_footer_while_typing() {
        let mut app = app_with_fake();
        app.update(crate::app::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        )));
        let screen = render(&app, 120, 36);
        assert!(screen.contains("filter"), "{screen}");
        assert!(screen.contains("esc clear"), "{screen}");
    }

    #[test]
    fn numbers_follow_the_rules_of_section_seven() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(1024), "1.0 KiB");
        assert_eq!(fmt_bytes(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(fmt_pct(Some(25.34)), "25.3%");
        assert_eq!(fmt_pct(None), "—", "never a fake 0%");
        assert_eq!(fmt_dur(12.0), "12s");
        assert_eq!(fmt_dur(275.0), "4m35s");
        assert_eq!(fmt_dur(3720.0), "1h02m");
        assert_eq!(bar(Some(25.3)), format!("{}{}", "▇".repeat(5), "░".repeat(15)));
        assert_eq!(bar(None), "░".repeat(20));
        assert_eq!(bar(Some(100.0)), "▇".repeat(20));
    }

    #[test]
    fn sql_is_collapsed_and_truncated_to_one_line() {
        let sql = "WITH x AS (\n  SELECT 1,\n  2\n)\nSELECT * FROM x";
        assert_eq!(collapse_sql(sql, 100), "WITH x AS ( SELECT 1, 2 ) SELECT * FROM x");
        assert_eq!(collapse_sql(sql, 12), "WITH x AS (…");
    }

    #[test]
    fn severity_matches_the_table() {
        assert_eq!(sev_node(Some(74.9)), Severity::None);
        assert_eq!(sev_node(Some(75.0)), Severity::Warn);
        assert_eq!(sev_node(Some(90.0)), Severity::Crit);
        assert_eq!(sev_user_mem(Some(19.9)), Severity::None);
        assert_eq!(sev_user_mem(Some(20.0)), Severity::Warn);
        assert_eq!(sev_user_mem(Some(40.0)), Severity::Crit);
        assert_eq!(sev_elapsed(4.9), Severity::None);
        assert_eq!(sev_elapsed(5.0), Severity::Warn);
        assert_eq!(sev_elapsed(30.0), Severity::Crit);
        assert_eq!(sev_lag(9), Severity::None);
        assert_eq!(sev_lag(10), Severity::Warn);
        assert_eq!(sev_lag(60), Severity::Crit);
        assert_eq!(sev_wait(59, false), Severity::None);
        assert_eq!(sev_wait(10, true), Severity::Warn, "all workers busy is amber");
        assert_eq!(sev_wait(180, false), Severity::Crit);
    }

    #[test]
    fn dump_screen() {
        let app = app_with_fake();
        println!("{}", render(&app, 120, 36));
    }

    #[test]
    fn the_clock_is_utc() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(12 * 3600 + 34 * 60 + 56);
        assert_eq!(utc_clock(at), "12:34:56");
    }

    #[test]
    fn selecting_a_query_shows_its_detail() {
        let mut app = app_with_fake();
        // Walk down to the first query row: ⏎ on a user opens it, then the cursor reaches it.
        for _ in 0..8 {
            app.update(crate::app::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            )));
            if app
                .selected_row()
                .map(|(_, id)| matches!(id, crate::tree::RowId::Query { .. }))
                .unwrap_or(false)
            {
                break;
            }
            app.update(crate::app::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            )));
        }
        assert!(
            app.selected_row()
                .map(|(_, id)| matches!(id, crate::tree::RowId::Query { .. }))
                .unwrap_or(false),
            "landed on a query row"
        );

        let screen = render(&app, 120, 36);
        assert!(screen.contains("cores"), "the drawer describes the query");
        assert!(screen.contains("rows"), "and its read volume");
    }
}
