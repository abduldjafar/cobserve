//! The detail drawer (§2.4): everything about the selected row that does not fit in it.
//!
//! First line is a titled rule; the body gets whatever lines the layout gave it (0–3), most
//! important first, so a short terminal loses the least useful line.

use super::widgets::{bar, rule, sparkline, tone_spans, Cells, PCT_SHAPE};
use crate::app::{App, View};
use crate::fmt;
use crate::history::{eta_to, Trend};
use crate::model::{FleetUser, JobState, NodeView, QueryStat, UserNode, UserSlice};
use crate::severity::{self, Severity};
use crate::sqltext::{self, Token};
use crate::theme::Theme;
use crate::tree::{Payload, Row, TreeState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let (title, body) = match app.view {
        View::Nodes => nodes(app, theme, width),
        View::Queue => queue(app, theme, width),
        View::Map => map(app, theme, width),
        View::Tape => tape(app, theme, width),
    };
    frame.render_widget(
        Paragraph::new(rule(width, title, theme)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let room = area.height.saturating_sub(1);
    if room == 0 {
        return;
    }
    let lines: Vec<Line<'static>> = body.into_iter().take(room as usize).collect();
    let body_area = Rect::new(area.x, area.y + 1, area.width, room);
    if app.view == View::Tape {
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), body_area);
    } else {
        frame.render_widget(Paragraph::new(lines), body_area);
    }
}

type Drawer = (Vec<Span<'static>>, Vec<Line<'static>>);

fn title(text: impl Into<String>, theme: &Theme) -> Vec<Span<'static>> {
    vec![Span::styled(text.into(), theme.strong())]
}

fn muted_line(text: impl Into<String>, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(text.into(), theme.muted());
    cells.line_unpadded(width)
}

fn nodes(app: &App, theme: &Theme, width: usize) -> Drawer {
    if app.focus == crate::app::Focus::Insights {
        return insight_detail(app, theme, width);
    }
    let drawn = app.with_rows(|_, rows| {
        let index = crate::tree::selection_index(rows, app.selected());
        match index.and_then(|i| rows.get(i)) {
            Some(row) => row_detail(row, app, theme, width),
            None => (title("no selection", theme), vec![muted_line("↑↓ to pick a row", theme, width)]),
        }
    });
    drawn.unwrap_or_else(|| {
        (
            title("no snapshot yet", theme),
            vec![muted_line("waiting for the first poll of the fleet…", theme, width)],
        )
    })
}

/// The chosen insight: what its line is about, and the numbers behind it — one finding a
/// line, the line's own first.
fn insight_detail(app: &App, theme: &Theme, width: usize) -> Drawer {
    let insights = app.insights();
    let Some(insight) = insights.get(app.insight_selection()) else {
        return (title("insights", theme), Vec::new());
    };
    let goes = match &insight.subject {
        crate::insight::Subject::Fleet => "",
        crate::insight::Subject::Node(_) => " · ⏎ goes to the node",
        crate::insight::Subject::Query { .. } => " · ⏎ goes to the query",
        crate::insight::Subject::Queue => " · ⏎ opens the queue",
    };
    let head = vec![
        Span::styled(format!("{} ", insight.level.glyph()), theme.sev(insight.level).add_modifier(Modifier::BOLD)),
        Span::styled(insight.label.clone(), theme.accent().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" · {} of {}{goes}", app.insight_selection() + 1, insights.len()),
            theme.muted(),
        ),
    ];
    let lines = insight
        .details
        .iter()
        .map(|detail| {
            let mut cells = Cells::new();
            cells.spans(tone_spans(detail, theme));
            cells.line_unpadded(width)
        })
        .collect();
    (head, lines)
}

fn row_detail(row: &Row<'_>, app: &App, theme: &Theme, width: usize) -> Drawer {
    match &row.payload {
        Payload::Node(view) => node_detail(view, app, theme, width),
        Payload::User { view, slice } => user_detail(view, slice, app, theme, width),
        Payload::Closing(view) => closing_detail(view, theme, width),
        Payload::Folded { names, .. } => folded_detail(names, app, theme, width),
        Payload::FleetUser(user) => fleet_user_detail(user, app, theme, width),
        Payload::PivotNode { user, node } => pivot_node_detail(user, node, theme, width),
        Payload::Query { stat, node, user } => query_detail(stat, node, user, app, theme, width),
    }
}

/// `MEM 58.2/64 GiB 90.9% ▁▂▃▅▆▇█ ↗ +0.8%/min · full in ~11m`
#[allow(clippy::too_many_arguments)]
fn resource_line(
    cells: &mut Cells,
    label: &str,
    absolute: String,
    pct: Option<f64>,
    series: Option<&crate::history::Series>,
    now: f64,
    spark_cells: usize,
    theme: &Theme,
    forecast: bool,
) {
    let sev = severity::node(pct);
    cells.push(format!("{label} "), theme.section());
    cells.push(absolute, theme.text());
    cells.push(format!("  {}", fmt::pct(pct)), theme.sev(sev).add_modifier(Modifier::BOLD));
    if spark_cells > 0 {
        cells.push("  ", Style::default());
        cells.spans(sparkline(series, now, spark_cells, PCT_SHAPE, |v| {
            theme.bar_fill(severity::node(Some(v)))
        }));
    }
    let slope = series.and_then(|s| s.slope(60.0));
    let trend = Trend::from_slope(slope, 0.5);
    if let Some(slope) = slope {
        let per_min = slope * 60.0;
        let style = if trend.rising() { theme.sev(Severity::Warn) } else { theme.muted() };
        if trend == Trend::Flat {
            cells.push(" → steady", theme.muted());
        } else {
            cells.push(format!(" {} {per_min:+.1}%/min", trend.arrow()), style);
        }
        if forecast
            && trend.rising()
            && series.is_some_and(|s| s.span_s() >= crate::history::FORECAST_MIN_SPAN_S)
            && let Some(eta) = pct.and_then(|p| eta_to(p, slope, 100.0))
            && eta <= 3600.0
        {
            cells.push(" full ", theme.muted());
            cells.push(fmt::eta(eta), theme.sev(if eta <= 300.0 { Severity::Crit } else { Severity::Warn }));
        }
    }
}

fn node_detail(view: &NodeView<'_>, app: &App, theme: &Theme, width: usize) -> Drawer {
    let node = view.node;
    let mut head = vec![Span::styled(node.name.clone(), theme.accent().add_modifier(Modifier::BOLD))];
    if node.reachable {
        head.push(Span::styled(format!(" · {}:{}", node.host, node.port), theme.muted()));
    } else {
        head.push(Span::styled(" · ↯ unreachable", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
    }
    if app.tree.new_nodes.contains(&node.name) {
        head.push(Span::styled(" · NEW this session", theme.sev(Severity::Ok)));
    }

    let mut lines = Vec::new();
    if !node.reachable {
        let mut cells = Cells::new();
        cells.push(node.down_word(), theme.sev(Severity::Crit));
        if let Some(detail) = node.down_detail() {
            cells.push(format!(": {detail}"), theme.text());
        }
        cells.push(" · its numbers are unknown, not zero · dropped after 5 minutes if it leaves system.clusters", theme.muted());
        lines.push(cells.line_unpadded(width));
        return (head, lines);
    }

    let history = app.history.node(&node.name);
    let now = app.history.now();
    let spark = if width >= 170 { 24 } else if width >= 140 { 16 } else if width >= 100 { 8 } else { 0 };

    // 1: the two resources, with their history and where they are heading.
    let mut cells = Cells::new();
    resource_line(
        &mut cells,
        "MEM",
        match node.mem_total {
            Some(total) => format!("{:.1}/{:.0} GiB", fmt::gib(node.mem_used), fmt::gib(total)),
            None => format!("{:.1} GiB/—", fmt::gib(node.mem_used)),
        },
        view.mem_pct,
        history.map(|h| &h.mem_pct),
        now,
        spark,
        theme,
        true,
    );
    cells.push("    ", Style::default());
    resource_line(
        &mut cells,
        "CPU",
        match (view.busy_cores, node.cores) {
            (Some(busy), Some(cores)) => format!("{busy:.1}/{cores:.0} cores"),
            (None, Some(cores)) => format!("—/{cores:.0} cores"),
            (Some(busy), None) => format!("{busy:.1} cores/—"),
            (None, None) => "—/—".to_string(),
        },
        view.cpu_pct,
        history.map(|h| &h.cpu_pct),
        now,
        spark,
        theme,
        false,
    );
    lines.push(cells.line_unpadded(width));

    // 2: who holds it — users against the server — and how much room is left.
    let mut cells = Cells::new();
    let users_mem: f64 = view.users.iter().filter_map(|u| u.mem_pct).sum();
    let queries: usize = view.users.iter().map(|u| u.queries.len()).sum();
    let runaways: usize = view.users.iter().flat_map(|u| u.queries.iter()).filter(|q| q.runaway).count();
    if view.mem_pct.is_some() {
        cells.push("memory: queries ", theme.muted());
        cells.push(fmt::pct(Some(users_mem)), theme.strong());
        cells.push(" + server ", theme.muted());
        cells.push(fmt::pct(view.server_mem_pct), theme.strong());
        cells.push(" · ", theme.faint());
    }
    cells.push(format!("{queries} running"), theme.text());
    if runaways > 0 {
        cells.push(format!(" ({runaways} ✕)"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    cells.push(" · lag ", theme.muted());
    cells.push(fmt::dur(node.lag_s as f64), theme.sev(severity::lag(node.lag_s)));
    cells.push(format!(" · {} parts", fmt::count(node.active_parts)), theme.muted());
    if let Some(total) = node.mem_total {
        let free = total.saturating_sub(node.mem_used);
        // Against the biggest limit a query here actually has, which is what a new one of
        // those could take; the node's own setting is the fallback.
        let limit = view
            .users
            .iter()
            .flat_map(|u| u.queries.iter())
            .map(|q| q.limit)
            .max()
            .unwrap_or(view.mem_limit);
        let fits = free as f64 / limit.max(1) as f64;
        cells.push(" · free ", theme.muted());
        cells.push(fmt::bytes(free), theme.sev(if fits < 1.0 { Severity::Warn } else { Severity::None }));
        cells.push(format!(" = {fits:.1}× a {} query", fmt::bytes(limit)), theme.muted());
    }
    if !crate::model::column_known(view) {
        cells.push(" · ", theme.faint());
        cells.push(
            match (node.mem_total, node.cores) {
                (None, Some(_)) => "no usable memory total, so no memory %",
                (Some(_), None) => "no usable core count, so no CPU %",
                _ => "neither a memory total nor a core count",
            },
            theme.sev(Severity::Warn),
        );
    }
    lines.push(cells.line_unpadded(width));

    // 3: where it is.
    let mut cells = Cells::new();
    cells.push(
        format!(
            "shard {} · replica {} · v{} · up {}",
            node.shard,
            node.replica,
            node.version,
            fmt::opt_dur(node.uptime_s)
        ),
        theme.muted(),
    );
    if let Some(ms) = node.poll_ms {
        cells.push(" · poll ", theme.muted());
        cells.push(format!("{ms} ms"), if ms >= 750 { theme.sev(Severity::Warn) } else { theme.muted() });
    }
    if let Some(snapshot) = app.snapshot() {
        cells.push(format!(" · numbers from {} UTC", fmt::utc_clock(snapshot.taken_at)), theme.faint());
    }
    lines.push(cells.line_unpadded(width));
    (head, lines)
}

fn user_detail(view: &NodeView<'_>, slice: &UserSlice<'_>, app: &App, theme: &Theme, width: usize) -> Drawer {
    let mut head = Vec::new();
    match &slice.person {
        Some(person) => {
            head.push(Span::styled(crate::attrib::display_person(person), theme.person().add_modifier(Modifier::BOLD)));
            head.push(Span::styled(format!(" ({})", slice.user), theme.muted()));
        }
        None => head.push(Span::styled(slice.user.clone(), theme.strong())),
    }
    head.push(Span::styled(" @ ", theme.muted()));
    head.push(Span::styled(view.name().to_string(), theme.accent().add_modifier(Modifier::BOLD)));

    let mut lines = Vec::new();
    let mut cells = Cells::new();
    cells.push(fmt::plural(slice.queries.len(), "query", "queries"), theme.text());
    cells.push(" · Σ ", theme.muted());
    cells.push(fmt::bytes(slice.mem_bytes), theme.strong());
    cells.push(format!(" = {} of {}", fmt::pct(slice.mem_pct), view.name()), theme.muted());
    cells.push(" · Σ ", theme.muted());
    cells.push(format!("{:.2} cores", slice.cores), theme.strong());
    cells.push(format!(" = {}", fmt::pct(slice.cpu_pct)), theme.muted());
    cells.push(" · longest ", theme.muted());
    cells.push(fmt::dur(slice.longest_s), theme.sev(if slice.runaway { Severity::Crit } else { severity::elapsed(slice.longest_s) }));
    if slice.runaway {
        cells.push(" ✕", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let key = TreeState::user_row_key(slice);
    if let Some(series) = app.history.user(view.name(), &key)
        && let Some(slope) = series.slope(60.0)
    {
        let trend = Trend::from_slope(Some(slope), 0.5);
        cells.push(
            format!(" · share {} {:+.1}%/min", trend.arrow(), slope * 60.0),
            if trend.rising() { theme.sev(Severity::Warn) } else { theme.muted() },
        );
    }
    lines.push(cells.line_unpadded(width));

    // Where else the same person is running something — the pivot's answer, inline.
    let mut cells = Cells::new();
    let limit = slice.queries.iter().map(|q| q.limit).max().unwrap_or(view.mem_limit);
    cells.push(
        format!(
            "runaway at ≥ 30 s or ≥ {} (80% of its limit)",
            fmt::bytes((crate::model::RUNAWAY_MEMORY_FRACTION * limit as f64) as u64)
        ),
        theme.muted(),
    );
    let elsewhere: Vec<String> = app
        .with_view(|fleet| {
            fleet
                .users
                .iter()
                .filter(|u| u.user == slice.user && u.person == slice.person)
                .flat_map(|u| u.nodes.iter())
                .filter(|n| n.node.name != view.name())
                .map(|n| format!("{} {}", n.node.name, fmt::pct(n.mem_pct)))
                .collect()
        })
        .unwrap_or_default();
    if !elsewhere.is_empty() {
        cells.push(" · also on ", theme.muted());
        cells.push(elsewhere.join(", "), theme.accent());
    }
    lines.push(cells.line_unpadded(width));

    let mut cells = Cells::new();
    let mut redash: Vec<u64> = slice.queries.iter().filter_map(|q| q.query.redash_query_id).collect();
    redash.sort_unstable();
    redash.dedup();
    if !redash.is_empty() {
        cells.push("Redash ", theme.muted());
        cells.push(
            redash.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" "),
            theme.strong(),
        );
        cells.push(" · ", theme.faint());
    }
    cells.push(
        slice
            .queries
            .iter()
            .map(|q| format!("{} {}", fmt::truncate(&q.query.query_id, 8), fmt::dur(q.query.elapsed_s)))
            .collect::<Vec<_>>()
            .join(" · "),
        theme.muted(),
    );
    cells.push("  ⏎ opens its queries", theme.faint());
    lines.push(cells.line_unpadded(width));
    (head, lines)
}

fn closing_detail(view: &NodeView<'_>, theme: &Theme, width: usize) -> Drawer {
    let head = vec![
        Span::styled("server · caches · merges", theme.strong()),
        Span::styled(" @ ", theme.muted()),
        Span::styled(view.name().to_string(), theme.accent().add_modifier(Modifier::BOLD)),
    ];
    let mut cells = Cells::new();
    cells.push("everything on ", theme.muted());
    cells.push(view.name().to_string(), theme.text());
    cells.push(" that is not a user query: ", theme.muted());
    cells.push(fmt::pct(view.server_mem_pct), theme.strong());
    if let (Some(pct), Some(total)) = (view.server_mem_pct, view.node.mem_total) {
        cells.push(format!(" of memory ({})", fmt::bytes((pct / 100.0 * total as f64) as u64)), theme.muted());
    }
    cells.push(", ", theme.muted());
    cells.push(fmt::pct(view.server_cpu_pct), theme.strong());
    cells.push(" of CPU", theme.muted());
    let second = muted_line(
        "mark and uncompressed caches, merges and mutations, background pools, the server itself — not attributable to a person",
        theme,
        width,
    );
    let third = muted_line(
        "the user rows plus this row add up to the node's own percentage (DESIGN.md §5.3)",
        theme,
        width,
    );
    (head, vec![cells.line_unpadded(width), second, third])
}

fn folded_detail(names: &[String], app: &App, theme: &Theme, width: usize) -> Drawer {
    let head = title(format!("{} healthy nodes folded", names.len()), theme);
    let mut cells = Cells::new();
    let pressures = app
        .with_view(|view| {
            names
                .iter()
                .filter_map(|name| {
                    view.nodes
                        .iter()
                        .find(|n| &n.node.name == name)
                        .map(|n| (name.clone(), n.mem_pct, n.cpu_pct))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (i, (name, mem, cpu)) in pressures.iter().enumerate() {
        if i > 0 {
            cells.push(" · ", theme.faint());
        }
        cells.push(name.clone(), theme.accent());
        cells.push(format!(" {} / {}", fmt::pct(*mem), fmt::pct(*cpu)), theme.muted());
    }
    let rule_line = muted_line(
        "below 35% memory and CPU, no runaway query, replica lag under 10 s · ␣ unfolds them",
        theme,
        width,
    );
    (head, vec![cells.line_unpadded(width), rule_line])
}

fn fleet_user_detail(user: &FleetUser<'_>, app: &App, theme: &Theme, width: usize) -> Drawer {
    let head = vec![
        Span::styled(user.label(), theme.person().add_modifier(Modifier::BOLD)),
        Span::styled(" · across the fleet", theme.muted()),
    ];
    let mut cells = Cells::new();
    cells.push("Σ ", theme.muted());
    cells.push(fmt::bytes(user.mem_bytes), theme.strong());
    cells.push(" · Σ ", theme.muted());
    cells.push(format!("{:.2} cores", user.cores), theme.strong());
    cells.push(
        format!(
            " · {} on {} · longest {}",
            fmt::plural(user.queries, "query", "queries"),
            fmt::plural(user.nodes.len(), "node", "nodes"),
            fmt::dur(user.longest_s)
        ),
        theme.muted(),
    );
    if user.runaway {
        cells.push(" ✕", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let total = app.with_view(|v| crate::model::fleet_totals(v).mem_total).unwrap_or(0);
    if total > 0 {
        cells.push(
            format!(" · {:.1}% of fleet memory", user.mem_bytes as f64 / total as f64 * 100.0),
            theme.muted(),
        );
    }
    let mut per_node = Cells::new();
    for (i, node) in user.nodes.iter().enumerate() {
        if i > 0 {
            per_node.push(" · ", theme.faint());
        }
        per_node.push(node.node.name.clone(), theme.accent());
        per_node.push(format!(" {} mem / {} cpu", fmt::pct(node.mem_pct), fmt::pct(node.cpu_pct)), theme.muted());
    }
    (head, vec![cells.line_unpadded(width), per_node.line_unpadded(width)])
}

fn pivot_node_detail(user: &FleetUser<'_>, node: &UserNode<'_>, theme: &Theme, width: usize) -> Drawer {
    let label = match node.person.as_deref() {
        Some(person) => crate::attrib::user_label(&user.user, Some(person)),
        None => user.label(),
    };
    let head = vec![
        Span::styled(label, theme.person().add_modifier(Modifier::BOLD)),
        Span::styled(" @ ", theme.muted()),
        Span::styled(node.node.name.clone(), theme.accent().add_modifier(Modifier::BOLD)),
    ];
    let mut cells = Cells::new();
    cells.push(fmt::pct(node.mem_pct), theme.strong());
    cells.push(format!(" of the node's memory ({})", fmt::bytes(node.mem_bytes)), theme.muted());
    cells.push(" · ", theme.faint());
    cells.push(fmt::pct(node.cpu_pct), theme.strong());
    cells.push(format!(" of its CPU · {}", fmt::plural(node.queries.len(), "query", "queries")), theme.muted());
    let list = muted_line(
        node.queries
            .iter()
            .map(|q| format!("{} {}", fmt::truncate(&q.query.query_id, 8), fmt::dur(q.query.elapsed_s)))
            .collect::<Vec<_>>()
            .join(" · "),
        theme,
        width,
    );
    (head, vec![cells.line_unpadded(width), list])
}

fn query_detail(stat: &QueryStat<'_>, node: &str, user: &str, app: &App, theme: &Theme, width: usize) -> Drawer {
    let query = stat.query;
    let mut head = vec![Span::styled(query.query_id.clone(), theme.strong())];
    head.push(Span::styled(" · ", theme.faint()));
    match query.person.as_deref() {
        Some(person) => head.push(Span::styled(crate::attrib::display_person(person), theme.person())),
        None => head.push(Span::styled(user.to_string(), theme.text())),
    }
    head.push(Span::styled(" @ ", theme.muted()));
    head.push(Span::styled(node.to_string(), theme.accent().add_modifier(Modifier::BOLD)));
    if let Some(redash) = query.redash_query_id {
        head.push(Span::styled(format!(" · Redash #{redash}"), theme.muted()));
    }

    let history = app.history.query(node, &query.query_id);
    let mut lines = Vec::new();

    // 1: how long, how much memory against the limit it will be killed at, how fast it grows.
    let mut cells = Cells::new();
    let elapsed_sev = if stat.runaway { Severity::Crit } else { severity::elapsed(query.elapsed_s) };
    cells.push("⏱ ", theme.muted());
    cells.push(fmt::dur(query.elapsed_s), theme.sev(elapsed_sev).add_modifier(Modifier::BOLD));
    if stat.runaway {
        cells.push(" ✕ runaway", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let fraction = stat.limit_fraction();
    let holds = stat.limit_holds();
    cells.push("   MEM ", theme.section());
    cells.push(fmt::bytes(query.memory_bytes), theme.strong());
    if holds {
        let limit_sev = severity::limit_share(fraction);
        cells.push(format!(" = {} of its {} limit ", fmt::pct0(fraction * 100.0), fmt::bytes(stat.limit)), theme.muted());
        cells.spans(bar(Some(fraction * 100.0), 10, theme.bar_fill(limit_sev), theme));
    } else {
        // Twelve times past the limit its settings show is not a query about to be killed: the
        // server holds it to something else.
        cells.push(format!(" · past the {} limit its settings show", fmt::bytes(stat.limit)), theme.muted());
    }
    if let Some(rate) = history.and_then(|h| h.mem_rate()).filter(|r| r.abs() >= 1024.0 * 1024.0) {
        let growing = rate > 0.0;
        cells.push(
            format!(" {}{}", if growing { "+" } else { "−" }, fmt::rate(rate.abs())),
            if growing { theme.sev(Severity::Warn) } else { theme.muted() },
        );
        if growing
            && holds
            && let Some(eta) = eta_to(query.memory_bytes as f64, rate, stat.limit as f64)
                .filter(|eta| *eta <= crate::insight::KILL_HORIZON_S)
        {
            cells.push(" → killed in ", theme.muted());
            cells.push(fmt::eta(eta), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
    }
    cells.push("   CPU ", theme.section());
    cells.push(format!("{:.2} cores", stat.cores), theme.strong());
    lines.push(cells.line_unpadded(width));

    // 2: how far through its input it is, and how fast it reads.
    let mut cells = Cells::new();
    cells.push("READ ", theme.section());
    cells.push(fmt::bytes(query.read_bytes), theme.text());
    cells.push(format!(" / {} rows", fmt::rows(query.read_rows)), theme.muted());
    if let Some(rate) = history.and_then(|h| h.read_rate()).filter(|r| *r > 0.0) {
        cells.push(format!(" @ {}", fmt::rate(rate)), theme.text2());
    }
    if query.written_rows > 0 {
        cells.push(format!(" · wrote {} rows", fmt::rows(query.written_rows)), theme.muted());
    }
    match stat.progress {
        Some(p) => {
            cells.push("   PROGRESS ", theme.section());
            cells.spans(bar(Some(p * 100.0), 12, theme.accent, theme));
            cells.push(format!(" {}", fmt::pct0(p * 100.0)), theme.accent().add_modifier(Modifier::BOLD));
            cells.push(
                format!(" of ~{} rows", fmt::rows(query.total_rows_approx)),
                theme.muted(),
            );
            if let Some(eta) = stat.eta_s {
                cells.push(" · ETA ", theme.muted());
                cells.push(fmt::eta(eta), theme.strong());
            }
        }
        None => {
            cells.push("   no row estimate, so no progress", theme.faint());
        }
    }
    if let Some(snapshot) = app.snapshot() {
        let started = crate::history::secs(snapshot.taken_at) - query.elapsed_s;
        cells.push(format!(" · started {} UTC", fmt::utc_clock_secs(started)), theme.faint());
    }
    if let Some(kind) = &query.kind {
        cells.push(format!(" · {kind}"), theme.faint());
    }
    lines.push(cells.line_unpadded(width));

    // 3: the SQL, without Redash's comment, coloured.
    let mut cells = Cells::new();
    for (text, token) in sqltext::highlight(&sqltext::preview(&query.sql)) {
        let style = match token {
            Token::Keyword => theme.accent(),
            Token::String => theme.sev(Severity::Ok),
            Token::Number => theme.sev(Severity::Warn),
            Token::Plain => theme.text2(),
        };
        cells.push(text, style);
    }
    lines.push(cells.line_unpadded(width));
    (head, lines)
}

// ---------------------------------------------------------------------------
// Other views
// ---------------------------------------------------------------------------

fn queue(app: &App, theme: &Theme, width: usize) -> Drawer {
    let rows = app.queue_rows();
    let explanation = app.queue_explanation();
    let selected = app.queue_selection().and_then(|i| rows.get(i).copied());
    let Some(job) = selected else {
        return (
            title("queue", theme),
            vec![muted_line(explanation, theme, width)],
        );
    };

    let head = vec![
        Span::styled(format!("queue · {}", job.queue), theme.strong()),
        Span::styled(" · ", theme.faint()),
        Span::styled(job.label(), theme.person()),
    ];
    let mut lines = vec![muted_line(explanation, theme, width)];

    let mut cells = Cells::new();
    match (job.state, job.clickhouse_target()) {
        // A waiting job has not reached ClickHouse yet: there is nothing to kill, and the
        // wait is the queue (§2.8).
        (JobState::Queued, _) => {
            let ahead = app
                .queue
                .waiting(&job.queue)
                .iter()
                .filter(|other| other.age_s > job.age_s)
                .count();
            cells.push(format!("waiting {}", fmt::dur(job.age_s as f64)), theme.text());
            cells.push(
                format!(" · {ahead} job{} ahead of it", if ahead == 1 { "" } else { "s" }),
                theme.sev(Severity::Warn),
            );
            cells.push(" · nothing to kill: it has not reached ClickHouse", theme.muted());
        }
        (JobState::Started, Some((node, query_id))) => {
            cells.push(format!("running {}", fmt::dur(job.age_s as f64)), theme.text());
            cells.push(format!(" · job {} → ", job.id), theme.muted());
            cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
            cells.push(format!(" · {query_id}"), theme.muted());
            if app.query_is_runaway(node, query_id) {
                cells.push(" ✕ runaway", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
            }
            cells.push(" · ⏎ jumps to the query on view 1", theme.faint());
        }
        (JobState::Started, None) => {
            cells.push(
                "running · no ClickHouse query matches its Redash number yet",
                theme.muted(),
            );
        }
    }
    lines.push(cells.line_unpadded(width));

    let mut cells = Cells::new();
    if let Some(ds) = &job.data_source {
        cells.push(format!("data source {ds}"), theme.muted());
    }
    if let Some(id) = job.redash_query_id {
        cells.push(format!(" · Redash #{id}"), theme.muted());
    }
    if let Some(name) = &job.query_name {
        cells.push(format!(" {name}"), theme.text2());
    }
    if let Some(age) = app.queue.age(app.clock) {
        cells.push(format!(" · queue read {} ago", fmt::dur(age.as_secs_f64())), theme.faint());
    }
    lines.push(cells.line_unpadded(width));
    (head, lines)
}

fn map(app: &App, theme: &Theme, width: usize) -> Drawer {
    let names = app.map_nodes();
    let Some(name) = names.get(app.map_selection().min(names.len().saturating_sub(1))) else {
        return (title("map", theme), vec![muted_line("no nodes yet", theme, width)]);
    };
    app.with_view(|view| {
        view.nodes
            .iter()
            .find(|n| &n.node.name == name)
            .map(|node| {
                let (head, mut lines) = node_detail(node, app, theme, width);
                lines.truncate(2);
                lines.push(muted_line("⏎ opens it in view 1", theme, width));
                (head, lines)
            })
    })
    .flatten()
    .unwrap_or_else(|| (title(name.clone(), theme), Vec::new()))
}

fn tape(app: &App, theme: &Theme, width: usize) -> Drawer {
    let Some(event) = app.tape.get_newest(app.tape_selection()) else {
        return (
            title("tape", theme),
            vec![muted_line("nothing has happened yet — events appear here as the fleet changes", theme, width)],
        );
    };
    let head = vec![
        Span::styled(format!("{} UTC", fmt::utc_clock_secs(event.at)), theme.strong()),
        Span::styled(format!(" · {}", event.kind.label()), theme.muted()),
    ];
    let mut cells = Cells::new();
    cells.push(format!("{} ", event.level.glyph()), theme.sev(event.level).add_modifier(Modifier::BOLD));
    cells.spans(tone_spans(&event.parts, theme));
    let mut lines = vec![Line::from(cells.into_spans())];
    if event.subject.is_some() {
        lines.push(muted_line("⏎ goes to what it is about", theme, width));
    }
    (head, lines)
}
