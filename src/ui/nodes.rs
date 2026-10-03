//! View 1: the tree (§2) as an aligned table, and the insights under it.
//!
//! Every row of every level — node, user, query, the closing row, and both pivot levels —
//! is laid on one grid, so the user rows' bars sit directly under their node's bar on the
//! same scale: you can see the user rows plus the closing row add up to the node (§5.3).

use super::widgets::{bar, rule, sparkline, tone_spans, Cells, PCT_SHAPE};
use crate::app::{scroll_into_view, App, Focus};
use crate::fmt;
use crate::history::Trend;
use crate::insight::Insight;
use crate::model::{FleetTotals, FleetUser, NodeView, QueryStat, SortKey};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::tree::{Kind, Payload, Row, TreeState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub const MIN_BAR: usize = 6;
pub const MAX_BAR: usize = 24;
pub const LABEL_MIN: usize = 22;
pub const LABEL_MAX: usize = 34;
const GAP: usize = 2;
/// `100.0%` and a trend arrow.
const PCT: usize = 7;
const QUERIES_W: usize = 7;
const LONGEST_W: usize = 9;
/// Below this terminal width the CPU bar goes, then LONGEST, then QUERIES (§7).
pub const NARROW: u16 = 100;

/// Where every column of the table goes, for one terminal width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub label: usize,
    pub mem_bar: usize,
    pub cpu_bar: usize,
    pub mem_abs: usize,
    pub cpu_abs: usize,
    pub queries: bool,
    pub longest: bool,
}

impl Grid {
    /// Fit the table into `width` content cells. `terminal_width` decides the §7 cut-offs,
    /// which are stated in terminal columns.
    pub fn compute(width: usize, terminal_width: u16, label_ideal: usize, mem_abs: usize, cpu_abs: usize) -> Grid {
        let mut g = Grid {
            label: label_ideal.clamp(LABEL_MIN, LABEL_MAX),
            mem_bar: 0,
            cpu_bar: 0,
            mem_abs,
            cpu_abs,
            queries: true,
            longest: true,
        };
        let fixed = |g: &Grid| -> isize {
            (1 + g.label
                + GAP
                + PCT
                + 1
                + g.mem_abs
                + GAP
                + PCT
                + 1
                + g.cpu_abs
                + if g.queries { GAP + QUERIES_W } else { 0 }
                + if g.longest { GAP + LONGEST_W } else { 0 }) as isize
        };
        let width = width as isize;
        let min_bar = MIN_BAR as isize;

        if terminal_width >= NARROW {
            // Both bars, giving up a little of the label column first if that is what it takes.
            let mut avail = width - fixed(&g);
            if avail < 2 * (min_bar + 1) {
                let shrink = ((2 * (min_bar + 1) - avail) as usize).min(g.label - LABEL_MIN);
                g.label -= shrink;
                avail += shrink as isize;
            }
            if avail >= 2 * (min_bar + 1) {
                let each = (((avail - 2) / 2) as usize).min(MAX_BAR);
                g.mem_bar = each;
                g.cpu_bar = each;
                return g;
            }
        }

        // §7, in order: the CPU bar goes (its number stays), then LONGEST, then QUERIES.
        let mut avail = width - fixed(&g);
        if avail > min_bar {
            g.mem_bar = ((avail - 1) as usize).min(MAX_BAR);
            return g;
        }
        g.longest = false;
        avail += (GAP + LONGEST_W) as isize;
        if avail > min_bar {
            g.mem_bar = ((avail - 1) as usize).min(MAX_BAR);
            return g;
        }
        g.queries = false;
        avail += (GAP + QUERIES_W) as isize;
        if avail > min_bar {
            g.mem_bar = ((avail - 1) as usize).min(MAX_BAR);
            return g;
        }
        // Then the label gives way, and last of all the memory bar.
        let need = (min_bar + 1 - avail) as usize;
        let shrink = need.min(g.label - LABEL_MIN.min(g.label));
        g.label -= shrink;
        avail += shrink as isize;
        if avail > min_bar {
            g.mem_bar = ((avail - 1) as usize).min(MAX_BAR);
        }
        g
    }

    fn mem_start(&self) -> usize {
        1 + self.label + GAP
    }

    fn mem_width(&self) -> usize {
        PCT + if self.mem_bar > 0 { 1 + self.mem_bar } else { 0 } + 1 + self.mem_abs
    }

    fn cpu_start(&self) -> usize {
        self.mem_start() + self.mem_width() + GAP
    }

    fn cpu_width(&self) -> usize {
        PCT + if self.cpu_bar > 0 { 1 + self.cpu_bar } else { 0 } + 1 + self.cpu_abs
    }

    fn queries_start(&self) -> usize {
        self.cpu_start() + self.cpu_width() + GAP
    }

    fn longest_start(&self) -> usize {
        self.queries_start() + if self.queries { QUERIES_W + GAP } else { 0 }
    }
}

/// One resource cell: percentage, trend arrow, bar, absolute value.
struct Measure {
    pct: Option<f64>,
    sev: Severity,
    trend: Trend,
    /// The fill colour override for the closing row.
    server: bool,
    abs: String,
    /// The part of `abs` after this byte offset is drawn muted (the denominator).
    abs_split: Option<usize>,
}

fn mem_abs_node(node: &NodeView<'_>) -> (String, Option<usize>) {
    let used = format!("{:.1}", fmt::gib(node.node.mem_used));
    let split = used.len();
    match node.node.mem_total {
        Some(total) => (format!("{used}/{:.0}", fmt::gib(total)), Some(split)),
        None => (format!("{used}/—"), Some(split)),
    }
}

fn cpu_abs_node(node: &NodeView<'_>) -> (String, Option<usize>) {
    let busy = match node.busy_cores {
        Some(b) => format!("{b:.1}"),
        None => "—".to_string(),
    };
    let split = busy.len();
    match node.node.cores {
        Some(cores) => (format!("{busy}/{cores:.0}"), Some(split)),
        None => (format!("{busy}/—"), Some(split)),
    }
}

/// GiB with the precision a value of that size needs: `16.3`, `1.20`, `0.04`, `<0.01`.
fn gib_short(bytes: u64) -> String {
    let g = fmt::gib(bytes);
    if bytes == 0 {
        "0".to_string()
    } else if g >= 10.0 {
        format!("{g:.1}")
    } else if g >= 0.01 {
        format!("{g:.2}")
    } else {
        "<0.01".to_string()
    }
}

fn cores_short(cores: f64) -> String {
    if cores >= 10.0 {
        format!("{cores:.1}")
    } else {
        format!("{cores:.2}")
    }
}

/// Connectors for each row: `├─`, `└─` and the `│` that continues a parent's branch.
fn connectors(rows: &[Row<'_>]) -> Vec<String> {
    let is_last = |i: usize| -> bool {
        let depth = rows[i].depth;
        for row in &rows[i + 1..] {
            if row.depth < depth {
                return true;
            }
            if row.depth == depth {
                return false;
            }
        }
        true
    };
    let mut out = Vec::with_capacity(rows.len());
    let mut parent_last = true;
    for (i, row) in rows.iter().enumerate() {
        match row.depth {
            0 => {
                out.push(String::new());
            }
            1 => {
                let last = is_last(i);
                parent_last = last;
                out.push(if last { "└─ " } else { "├─ " }.to_string());
            }
            _ => {
                let last = is_last(i);
                let trunk = if parent_last { "   " } else { "│  " };
                out.push(format!("{trunk}{}", if last { "└─ " } else { "├─ " }));
            }
        }
    }
    out
}

/// The natural width of a row's label column, before the grid decides what it gets.
fn natural_label(row: &Row<'_>, app: &App) -> usize {
    // Query summaries and the fold line are cut to whatever the others leave.
    if matches!(row.kind, Kind::Query | Kind::Folded) {
        return 0;
    }
    match &row.payload {
        Payload::Node(view) => 2 + fmt::width(view.name()) + badges_width(view, app) + 9,
        Payload::User { slice, .. } => 3 + fmt::width(&slice.label()),
        Payload::Closing(_) => 3 + fmt::width(CLOSING_LABEL),
        Payload::FleetUser(user) => 2 + fmt::width(&user.label()),
        Payload::PivotNode { node, .. } => 3 + fmt::width(&node.node.name),
        Payload::Query { .. } | Payload::Folded { .. } => 0,
    }
}

const CLOSING_LABEL: &str = "server · caches · merges";

fn badges_width(view: &NodeView<'_>, app: &App) -> usize {
    badges(view, app, &crate::theme::Theme::new(crate::theme::Depth::Mono, crate::theme::Variant::Dark))
        .iter()
        .map(|s| fmt::width(&s.content))
        .sum()
}

fn badges(view: &NodeView<'_>, app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if app.tree.new_nodes.contains(view.name()) {
        spans.push(Span::styled(" NEW", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD)));
    }
    if !view.node.reachable {
        spans.push(Span::styled(" ↯", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
    }
    let lag = severity::lag(view.node.lag_s);
    if lag.is_problem() {
        spans.push(Span::styled(
            format!(" lag {}", fmt::dur(view.node.lag_s as f64)),
            theme.sev(lag),
        ));
    }
    spans
}

fn trend_of(series: Option<&crate::history::Series>, step: f64) -> Trend {
    Trend::from_slope(series.and_then(|s| s.slope(60.0)), step)
}

fn trend_span(trend: Trend, theme: &Theme) -> Span<'static> {
    match trend {
        Trend::Flat => Span::raw(" "),
        Trend::Rising => Span::styled(trend.arrow(), theme.sev(Severity::Warn)),
        Trend::RisingFast => Span::styled(trend.arrow(), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        Trend::Falling | Trend::FallingFast => Span::styled(trend.arrow(), theme.sev(Severity::Ok)),
    }
}

/// Draw the column header and the tree into `area` (header on its first line).
pub fn draw_tree(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, terminal_width: u16) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let drawn = app.with_rows(|view, rows| {
        let totals = crate::model::fleet_totals(view);
        let label_ideal = rows.iter().map(|r| natural_label(r, app)).max().unwrap_or(LABEL_MIN);
        let (mem_abs, cpu_abs) = abs_widths(rows);
        let grid = Grid::compute(width, terminal_width, label_ideal, mem_abs, cpu_abs);

        let header = header_line(app, theme, &grid, width);
        let body_height = area.height.saturating_sub(1) as usize;

        if rows.is_empty() {
            let message = if app.tree.filter.trim().is_empty() {
                "no nodes in the fleet yet".to_string()
            } else {
                format!("nothing matches /{} · esc clears the filter", app.tree.filter.trim())
            };
            return (header, vec![Line::from(Span::styled(format!("  {message}"), theme.muted()))]);
        }

        let links = connectors(rows);
        let selected = crate::tree::selection_index(rows, app.selected());
        let offset = scroll_into_view(app.viewport.tree.get(), selected, body_height, rows.len());
        app.viewport.tree.set(offset);

        let mut lines = Vec::with_capacity(body_height);
        for (i, row) in rows.iter().enumerate().skip(offset).take(body_height) {
            let is_selected = selected == Some(i);
            lines.push(row_line(row, &links[i], app, theme, &grid, width, &totals, is_selected));
        }
        (header, lines)
    });

    let Some((header, lines)) = drawn else {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let secs = app
            .clock
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let glyph = spinner[(secs % spinner.len() as u64) as usize];
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {glyph} "), theme.accent()),
                Span::styled("waiting for the first snapshot…", theme.muted()),
            ])),
            area,
        );
        return;
    };
    frame.render_widget(Paragraph::new(header), Rect::new(area.x, area.y, area.width, 1));
    let body = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
    frame.render_widget(Paragraph::new(lines), body);
}

fn abs_widths(rows: &[Row<'_>]) -> (usize, usize) {
    let mut mem = 3; // "GiB"
    let mut cpu = 5; // "cores"
    for row in rows {
        match &row.payload {
            Payload::Node(view) if view.node.reachable => {
                mem = mem.max(fmt::width(&mem_abs_node(view).0));
                cpu = cpu.max(fmt::width(&cpu_abs_node(view).0));
            }
            Payload::User { slice, .. } => {
                mem = mem.max(fmt::width(&gib_short(slice.mem_bytes)));
                cpu = cpu.max(fmt::width(&cores_short(slice.cores)));
            }
            Payload::FleetUser(user) => {
                mem = mem.max(fmt::width(&gib_short(user.mem_bytes)));
                cpu = cpu.max(fmt::width(&cores_short(user.cores)));
            }
            _ => {}
        }
    }
    (mem, cpu)
}

fn header_line(app: &App, theme: &Theme, grid: &Grid, width: usize) -> Line<'static> {
    let style = theme.section();
    let sorted = theme.accent().add_modifier(Modifier::BOLD);
    let sort = app.tree.sort;
    let mut cells = Cells::new();
    cells.push(" ", style);
    let label = if app.tree.pivot {
        "USER → PERSON · NODE"
    } else {
        match sort {
            SortKey::Name => "NODE ▲ · USER → PERSON",
            _ => "NODE · USER → PERSON",
        }
    };
    cells.cell(label, grid.label, if sort == SortKey::Name && !app.tree.pivot { sorted } else { style });

    let marks = |key: SortKey| -> bool {
        !app.tree.pivot && (sort == key || sort == SortKey::Pressure)
    };
    cells.pad_to(grid.mem_start());
    if app.tree.pivot {
        // The pivot is sorted by Σ memory (§2.7); its rows are shares of the fleet, then of
        // each node under them.
        cells.push("MEM ▼", sorted);
        cells.push(" % of fleet · of node", theme.muted());
    } else {
        let mem_label = if marks(SortKey::Mem) { "MEM ▼" } else { "MEM" };
        cells.push(mem_label, if marks(SortKey::Mem) { sorted } else { style });
    }
    cells.pad_to(grid.mem_start() + grid.mem_width() - grid.mem_abs);
    cells.cell_right("GiB", grid.mem_abs, theme.muted());

    cells.pad_to(grid.cpu_start());
    let cpu_label = if marks(SortKey::Cpu) { "CPU ▼" } else { "CPU" };
    cells.push(cpu_label, if marks(SortKey::Cpu) { sorted } else { style });
    cells.pad_to(grid.cpu_start() + grid.cpu_width() - grid.cpu_abs);
    cells.cell_right("cores", grid.cpu_abs, theme.muted());

    if grid.queries {
        cells.pad_to(grid.queries_start());
        cells.cell_right("QUERIES", QUERIES_W, style);
    }
    if grid.longest {
        cells.pad_to(grid.longest_start());
        cells.cell_right("LONGEST", LONGEST_W - 2, style);
    }
    cells.line(width, Style::default())
}

#[allow(clippy::too_many_arguments)]
fn row_line(
    row: &Row<'_>,
    link: &str,
    app: &App,
    theme: &Theme,
    grid: &Grid,
    width: usize,
    totals: &FleetTotals,
    selected: bool,
) -> Line<'static> {
    let focused = selected && app.focus == Focus::Tree;
    let row_style = if focused { theme.selected() } else { Style::default() };
    let mut cells = Cells::new();

    // Column 0: the cursor, or a stripe in the node's severity colour.
    let marker = match &row.payload {
        _ if focused => Span::styled("▌", theme.accent()),
        _ if selected => Span::styled("▏", theme.muted()),
        Payload::Node(view) => {
            let sev = node_severity(view);
            if sev.is_problem() {
                Span::styled("▎", theme.sev(sev))
            } else {
                Span::raw(" ")
            }
        }
        _ => Span::raw(" "),
    };
    cells.span(marker);

    match &row.payload {
        Payload::Node(view) => {
            let open = app.tree.node_expanded(view.name());
            let mut label = Cells::new();
            label.push(if open { "▾ " } else { "▸ " }, theme.muted());
            label.push(view.name().to_string(), theme.strong());
            label.spans(badges(view, app, theme));
            // The memory history fills whatever the label column has left.
            let spark_room = grid.label.saturating_sub(label.width() + 1);
            let spark_cells = spark_room.min(10);
            cells.spans(label.into_spans());
            if spark_cells >= 4 && view.node.reachable {
                let series = app.history.node(view.name()).map(|h| &h.mem_pct);
                cells.pad_to(1 + grid.label - spark_cells);
                cells.spans(sparkline(series, app.history.now(), spark_cells, PCT_SHAPE, |v| {
                    theme.bar_fill(severity::node(Some(v)))
                }));
            }
            if !view.node.reachable {
                cells.pad_to(grid.mem_start());
                unreachable_cells(&mut cells, view, app, theme);
                return cells.line(width, row_style);
            }
            let history = app.history.node(view.name());
            let (mem_abs, mem_split) = mem_abs_node(view);
            let (cpu_abs, cpu_split) = cpu_abs_node(view);
            measure_cells(
                &mut cells,
                grid,
                theme,
                Measure {
                    pct: view.mem_pct,
                    sev: severity::node(view.mem_pct),
                    trend: trend_of(history.map(|h| &h.mem_pct), 0.5),
                    server: false,
                    abs: mem_abs,
                    abs_split: mem_split,
                },
                Measure {
                    pct: view.cpu_pct,
                    sev: severity::node(view.cpu_pct),
                    trend: trend_of(history.map(|h| &h.cpu_pct), 2.0),
                    server: false,
                    abs: cpu_abs,
                    abs_split: cpu_split,
                },
            );
            let queries: usize = view.users.iter().map(|u| u.queries.len()).sum();
            let runaways: usize = view
                .users
                .iter()
                .flat_map(|u| u.queries.iter())
                .filter(|q| q.runaway)
                .count();
            let longest = view
                .users
                .iter()
                .map(|u| (u.longest_s, u.runaway))
                .fold((0.0_f64, false), |acc, (s, r)| if s > acc.0 { (s, r) } else { acc });
            tail_cells(
                &mut cells,
                grid,
                theme,
                count_cell(queries, runaways, theme),
                (queries > 0).then_some(longest),
            );
        }
        Payload::User { view, slice } => {
            cells.push(link.to_string(), theme.faint());
            label_spans(&mut cells, &slice.user, slice.person.as_deref(), theme);
            let key = TreeState::user_row_key(slice);
            let series = app.history.user(view.name(), &key);
            measure_cells(
                &mut cells,
                grid,
                theme,
                Measure {
                    pct: slice.mem_pct,
                    sev: severity::user_share(slice.mem_pct),
                    trend: trend_of(series, 0.5),
                    server: false,
                    abs: gib_short(slice.mem_bytes),
                    abs_split: None,
                },
                Measure {
                    pct: slice.cpu_pct,
                    sev: severity::user_share(slice.cpu_pct),
                    trend: Trend::Flat,
                    server: false,
                    abs: cores_short(slice.cores),
                    abs_split: None,
                },
            );
            tail_cells(
                &mut cells,
                grid,
                theme,
                vec![Span::styled(slice.queries.len().to_string(), theme.text())],
                Some((slice.longest_s, slice.runaway)),
            );
        }
        Payload::Closing(view) => {
            cells.push(link.to_string(), theme.faint());
            cells.push(CLOSING_LABEL, theme.muted());
            let mem_bytes = match (view.server_mem_pct, view.node.mem_total) {
                (Some(pct), Some(total)) => gib_short((pct / 100.0 * total as f64) as u64),
                _ => "—".to_string(),
            };
            let cores = match (view.server_cpu_pct, view.node.cores) {
                (Some(pct), Some(cores)) => cores_short(pct / 100.0 * cores),
                _ => "—".to_string(),
            };
            measure_cells(
                &mut cells,
                grid,
                theme,
                Measure {
                    pct: view.server_mem_pct,
                    sev: Severity::None,
                    trend: Trend::Flat,
                    server: true,
                    abs: mem_bytes,
                    abs_split: None,
                },
                Measure {
                    pct: view.server_cpu_pct,
                    sev: Severity::None,
                    trend: Trend::Flat,
                    server: true,
                    abs: cores,
                    abs_split: None,
                },
            );
        }
        Payload::Folded { names, worst } => {
            cells.push("▸ ", theme.muted());
            cells.push(format!("{} healthy nodes folded", names.len()), theme.text2());
            cells.push(" · ", theme.faint());
            cells.push(names.join(" "), theme.muted());
            cells.push(format!(" · all ≤ {}", fmt::pct0(worst.ceil())), theme.faint());
            let hint = "␣ unfold";
            cells.pad_to(width.saturating_sub(fmt::width(hint) + 1));
            cells.push(hint, theme.faint());
        }
        Payload::FleetUser(user) => {
            let open = app.tree.node_expanded(&TreeState::fleet_user_key(user));
            cells.push(if open { "▾ " } else { "▸ " }, theme.muted());
            label_spans(&mut cells, &user.user, user.person.as_deref(), theme);
            let (mem_pct, cpu_pct) = fleet_shares(user, totals);
            measure_cells(
                &mut cells,
                grid,
                theme,
                Measure {
                    pct: mem_pct,
                    sev: severity::user_share(mem_pct),
                    trend: Trend::Flat,
                    server: false,
                    abs: gib_short(user.mem_bytes),
                    abs_split: None,
                },
                Measure {
                    pct: cpu_pct,
                    sev: severity::user_share(cpu_pct),
                    trend: Trend::Flat,
                    server: false,
                    abs: cores_short(user.cores),
                    abs_split: None,
                },
            );
            let runaways = user
                .nodes
                .iter()
                .flat_map(|n| n.queries.iter())
                .filter(|q| q.runaway)
                .count();
            tail_cells(
                &mut cells,
                grid,
                theme,
                count_cell(user.queries, runaways, theme),
                Some((user.longest_s, user.runaway)),
            );
        }
        Payload::PivotNode { node, .. } => {
            cells.push(link.to_string(), theme.faint());
            cells.push(node.node.name.clone(), theme.accent().add_modifier(Modifier::BOLD));
            let cores: f64 = node.queries.iter().map(|q| q.cores).sum();
            measure_cells(
                &mut cells,
                grid,
                theme,
                Measure {
                    pct: node.mem_pct,
                    sev: severity::user_share(node.mem_pct),
                    trend: Trend::Flat,
                    server: false,
                    abs: gib_short(node.mem_bytes),
                    abs_split: None,
                },
                Measure {
                    pct: node.cpu_pct,
                    sev: severity::user_share(node.cpu_pct),
                    trend: Trend::Flat,
                    server: false,
                    abs: cores_short(cores),
                    abs_split: None,
                },
            );
            tail_cells(
                &mut cells,
                grid,
                theme,
                vec![Span::styled(node.queries.len().to_string(), theme.text())],
                Some((node.longest_s, node.runaway)),
            );
        }
        Payload::Query { stat, node, .. } => {
            cells.push(link.to_string(), theme.faint());
            let summary = crate::sqltext::summary(&stat.query.sql);
            cells.push(summary.verb, theme.accent());
            if let Some(target) = &summary.target {
                cells.push(" · ", theme.faint());
                cells.push(target.clone(), theme.text2());
            }
            let node_view = app.with_view(|v| {
                v.nodes
                    .iter()
                    .find(|n| n.node.name == *node)
                    .map(|n| (n.node.mem_total, n.node.cores))
            });
            let (mem_total, cores) = node_view.flatten().unwrap_or((None, None));
            query_measures(&mut cells, grid, theme, stat, mem_total, cores);
            let progress = match stat.progress {
                Some(p) => vec![Span::styled(fmt::pct0(p * 100.0), theme.accent())],
                None => vec![Span::styled("—", theme.faint())],
            };
            tail_cells(&mut cells, grid, theme, progress, Some((stat.query.elapsed_s, stat.runaway)));
        }
    }

    // Cut the label column back to its width where a long label ran into the numbers.
    cells.line(width, row_style)
}

fn node_severity(view: &NodeView<'_>) -> Severity {
    if !view.node.reachable {
        return Severity::Crit;
    }
    severity::node(view.mem_pct)
        .max(severity::node(view.cpu_pct))
        .max(severity::lag(view.node.lag_s))
}

/// `r_redash → grigol.gankava` with the account muted and the person in their colour.
fn label_spans(cells: &mut Cells, user: &str, person: Option<&str>, theme: &Theme) {
    match person.map(crate::attrib::display_person) {
        Some(person) => {
            cells.push(format!("{user} → "), theme.muted());
            cells.push(person, theme.person());
        }
        None => {
            cells.push(user.to_string(), theme.text());
        }
    }
}

/// The label column is cut where the MEM column begins, so a long name never shifts the
/// numbers: everything is placed by `pad_to`, and anything that ran long is trimmed here.
fn clip_to(cells: Cells, col: usize) -> Cells {
    if cells.width() <= col {
        return cells;
    }
    let spans = super::widgets::fit(cells.into_spans(), col.saturating_sub(1), false);
    let mut out = Cells::new();
    out.spans(spans);
    out.push(" ", Style::default());
    out
}

fn measure_cells(cells: &mut Cells, grid: &Grid, theme: &Theme, mem: Measure, cpu: Measure) {
    let label_end = 1 + grid.label;
    let taken = std::mem::take(cells);
    *cells = clip_to(taken, label_end);
    cells.pad_to(grid.mem_start());
    one_measure(cells, theme, &mem, grid.mem_bar, grid.mem_abs);
    cells.pad_to(grid.cpu_start());
    one_measure(cells, theme, &cpu, grid.cpu_bar, grid.cpu_abs);
}

fn one_measure(cells: &mut Cells, theme: &Theme, m: &Measure, bar_cells: usize, abs_cells: usize) {
    let pct_style = if m.server {
        theme.muted()
    } else {
        theme.sev(m.sev).add_modifier(if m.sev.is_problem() { Modifier::BOLD } else { Modifier::empty() })
    };
    cells.cell_right(&fmt::pct(m.pct), PCT - 1, pct_style);
    cells.span(trend_span(m.trend, theme));
    if bar_cells > 0 {
        cells.push(" ", Style::default());
        let fill = if m.server { theme.server_bar } else { theme.bar_fill(m.sev) };
        cells.spans(bar(m.pct, bar_cells, fill, theme));
    }
    cells.push(" ", Style::default());
    let pad = abs_cells.saturating_sub(fmt::width(&m.abs));
    cells.push(" ".repeat(pad), Style::default());
    match m.abs_split {
        Some(split) if split <= m.abs.len() => {
            cells.push(m.abs[..split].to_string(), theme.text());
            cells.push(m.abs[split..].to_string(), theme.muted());
        }
        _ => {
            cells.push(m.abs.clone(), if m.server { theme.muted() } else { theme.text() });
        }
    }
}

fn query_measures(
    cells: &mut Cells,
    grid: &Grid,
    theme: &Theme,
    stat: &QueryStat<'_>,
    mem_total: Option<u64>,
    cores: Option<f64>,
) {
    let mem_pct = mem_total
        .filter(|t| *t > 0)
        .map(|t| stat.query.memory_bytes as f64 / t as f64 * 100.0);
    let cpu_pct = cores
        .filter(|c| *c > 0.0)
        .map(|c| (stat.cores / c * 100.0).clamp(0.0, 100.0));
    // A query's own severity is how close it is to its memory limit, not its share of the node.
    let limit_sev = severity::limit_share(stat.limit_fraction());
    measure_cells(
        cells,
        grid,
        theme,
        Measure {
            pct: mem_pct,
            sev: limit_sev.max(severity::user_share(mem_pct)),
            trend: Trend::Flat,
            server: false,
            abs: gib_short(stat.query.memory_bytes),
            abs_split: None,
        },
        Measure {
            pct: cpu_pct,
            sev: Severity::None,
            trend: Trend::Flat,
            server: false,
            abs: cores_short(stat.cores),
            abs_split: None,
        },
    );
}

/// QUERIES and LONGEST. `longest` is (seconds, runaway).
fn tail_cells(cells: &mut Cells, grid: &Grid, theme: &Theme, count: Vec<Span<'static>>, longest: Option<(f64, bool)>) {
    if grid.queries {
        cells.pad_to(grid.queries_start());
        let w: usize = count.iter().map(|s| fmt::width(&s.content)).sum();
        cells.gap(QUERIES_W.saturating_sub(w));
        cells.spans(count);
    }
    if grid.longest {
        cells.pad_to(grid.longest_start());
        match longest {
            Some((seconds, runaway)) => {
                let sev = if runaway { Severity::Crit } else { severity::elapsed(seconds) };
                cells.cell_right(&fmt::dur(seconds), LONGEST_W - 2, theme.sev(sev));
                cells.push(if runaway { " ✕" } else { "  " }, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
            }
            None => {
                cells.cell_right("—", LONGEST_W - 2, theme.faint());
            }
        }
    }
}

fn count_cell(queries: usize, runaways: usize, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(queries.to_string(), theme.text())];
    if runaways > 0 {
        spans.push(Span::styled(format!(" ✕{runaways}"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
    }
    spans
}

fn unreachable_cells(cells: &mut Cells, view: &NodeView<'_>, app: &App, theme: &Theme) {
    let since = app
        .history
        .node(view.name())
        .and_then(|h| h.unreachable_since)
        .map(|t| app.history.now() - t)
        .filter(|s| *s >= 1.0);
    cells.push("↯ unreachable", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    if let Some(for_s) = since {
        cells.push(format!(" for {}", fmt::dur(for_s)), theme.sev(Severity::Crit));
    }
    if let Some(reason) = &view.node.unreachable_reason {
        cells.push(format!(" · {reason}"), theme.muted());
    }
    cells.push(" · numbers unknown, not zero", theme.faint());
}

/// A user's share of the whole fleet, for the pivot's top level: Σ their memory over Σ the
/// fleet's (nodes with a known total only), and the same for cores.
fn fleet_shares(user: &FleetUser<'_>, totals: &FleetTotals) -> (Option<f64>, Option<f64>) {
    let known: u64 = user
        .nodes
        .iter()
        .filter(|n| n.node.mem_total.is_some_and(|t| t > 0))
        .map(|n| n.mem_bytes)
        .sum();
    let mem = (totals.mem_total > 0).then(|| known as f64 / totals.mem_total as f64 * 100.0);
    let cpu = (totals.cores > 0.0).then(|| (user.cores / totals.cores * 100.0).clamp(0.0, 100.0));
    (mem, cpu)
}

// ---------------------------------------------------------------------------
// The insights panel
// ---------------------------------------------------------------------------

/// The rule and as many insights as `area` has lines for (the rule is its first line).
pub fn draw_insights(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, insights: &[Insight]) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let focused = app.focus == Focus::Insights;
    let lines_room = area.height.saturating_sub(1) as usize;

    let mut title: Vec<Span<'static>> = vec![Span::styled(
        "INSIGHTS",
        if focused { theme.accent().add_modifier(Modifier::BOLD) } else { theme.section() },
    )];
    for level in [Severity::Crit, Severity::Warn, Severity::Info] {
        let n = insights.iter().filter(|i| i.level == level).count();
        if n > 0 {
            title.push(Span::raw(" "));
            title.push(Span::styled(format!("{n} {}", level.glyph()), theme.sev(level)));
        }
    }
    let hint = if focused {
        " · ↑↓ choose · ⏎ go there · tab back"
    } else if insights.len() > lines_room {
        " · tab to browse all"
    } else {
        " · tab to go to one"
    };
    title.push(Span::styled(hint, theme.faint()));
    frame.render_widget(
        Paragraph::new(rule(width, title, theme)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if lines_room == 0 {
        return;
    }

    let selected = focused.then_some(app.insight_selection());
    let offset = if focused {
        scroll_into_view(app.viewport.insights.get(), selected, lines_room, insights.len())
    } else {
        0
    };
    app.viewport.insights.set(offset);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let visible: Vec<(usize, &Insight)> = insights.iter().enumerate().skip(offset).take(lines_room).collect();
    let hidden = insights.len().saturating_sub(offset + visible.len());
    for (n, (index, insight)) in visible.iter().enumerate() {
        let last_line = n + 1 == lines_room;
        if last_line && hidden > 0 && !focused {
            let mut cells = Cells::new();
            cells.push(format!("  … {hidden} more", hidden = hidden + 1), theme.muted());
            cells.push(" · tab to browse", theme.faint());
            lines.push(cells.line(width, Style::default()));
            break;
        }
        let is_selected = selected == Some(*index);
        let mut cells = Cells::new();
        cells.push(if is_selected { "▌" } else { " " }, theme.accent());
        cells.push(format!("{} ", insight.level.glyph()), theme.sev(insight.level).add_modifier(Modifier::BOLD));
        cells.spans(tone_spans(&insight.parts, theme));
        lines.push(cells.line(width, if is_selected { theme.selected() } else { Style::default() }));
    }
    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(area.x, area.y + 1, area.width, area.height - 1),
    );
}

/// What a node row's whole state is, for the map tiles and the drawer.
pub fn severity_of(view: &NodeView<'_>) -> Severity {
    node_severity(view)
}

#[cfg(test)]
pub fn grid_for(width: u16) -> Grid {
    Grid::compute(width.saturating_sub(4) as usize, width, 28, 8, 7)
}
