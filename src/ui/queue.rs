//! View 2: the people behind the queue counts (§2.8).
//!
//! One line per queue with its workers drawn as slots and its depth over the last minutes,
//! then WAITING — who is waiting, for how long, against the red line at 3 minutes — and
//! RUNNING, every job on a worker with the ClickHouse query it became and that query's state
//! right now. That last column is the stitch: a full queue is usually a few workers stuck on
//! runaway queries, and this is where that becomes visible.

use super::widgets::{bar, dots, rule, sparkline, Cells, Scale};
use crate::app::{scroll_into_view, App};
use crate::fmt;
use crate::model::{Job, QueueRow};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const NAME: usize = 20;
const WHO: usize = 26;
const SOURCE: usize = 14;
/// Everything on a RUNNING row before the QUERY column.
const RUNNING_FIXED: usize = 1 + 3 + 2 + 7 + 3 + WHO + SOURCE;
/// What the stitch needs after it: `→ clickhouse-bi 21.2G 1.9c 95%`.
const STITCH: usize = 32;

/// The QUERY column of RUNNING takes what the stitch leaves.
fn query_cells(width: usize) -> usize {
    width.saturating_sub(RUNNING_FIXED + STITCH).clamp(16, 48)
}

/// `21.2G`, `312M`, `<1M`: memory in the space of a few characters.
fn short_bytes(bytes: u64) -> String {
    let gib = crate::fmt::gib(bytes);
    if gib >= 1.0 {
        format!("{gib:.1}G")
    } else if bytes >= 1024 * 1024 {
        format!("{}M", bytes / (1024 * 1024))
    } else {
        "<1M".to_string()
    }
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !app.queue.reachable {
        // The strip above already says why; repeating it would be noise, not information.
        lines.push(Line::from(Span::styled(
            if app.queue.error.as_deref() == Some(crate::model::QUEUE_NOT_CONFIGURED) {
                "  Redash is not configured: add redash: url, api_key (and redis_url, for the names of waiting jobs) to the --credential file, or set REDASH_URL, REDASH_ADMIN_API_KEY and REDIS_URL"
            } else {
                "  no queue data — the strip above says why"
            },
            theme.muted(),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    // -- the queues ---------------------------------------------------------
    let mut header = Cells::new();
    header.push(" ", Style::default());
    header.cell(app.queue.host().unwrap_or("QUEUE"), NAME, theme.section());
    header.cell_right("WAITING", 8, theme.section());
    header.gap(3);
    header.cell("OLDEST", 10, theme.section());
    header.cell("WORKERS", 26, theme.section());
    header.cell_right("FAILED/5m", 9, theme.section());
    header.gap(3);
    header.push("DEPTH · last 4 min", theme.section());
    lines.push(header.line(width, Style::default()));
    for row in &app.queue.queues {
        lines.push(queue_row(row, app, theme, width));
    }

    let (waiting, started) = app.queue_sections();
    let selected = app.queue_selection();
    let mut selected_line = None;

    // -- WAITING --------------------------------------------------------------
    lines.push(Line::from(""));
    let total_waiting: u32 = app.queue.queues.iter().map(|q| q.waiting).sum();
    let mut title = vec![Span::styled("WAITING", theme.section())];
    title.push(Span::styled(
        format!(" · {total_waiting} in the queues · {} named", waiting.len()),
        theme.muted(),
    ));
    lines.push(rule(width, title, theme));
    let mut h = Cells::new();
    h.push(" ", Style::default());
    h.cell_right("#", 3, theme.section());
    h.gap(2);
    h.cell("WAIT", 18, theme.section());
    h.cell("USER → PERSON", WHO, theme.section());
    h.cell("QUEUE", 12, theme.section());
    h.cell("DATA SOURCE", SOURCE, theme.section());
    h.push("QUERY", theme.section());
    lines.push(h.line(width, Style::default()));
    if waiting.is_empty() {
        lines.push(Line::from(Span::styled(
            if app.queue.names_available {
                "     nothing waiting"
            } else {
                "     names unavailable (no REDIS_URL) — counts only"
            },
            theme.muted(),
        )));
    }
    let mut index = 0usize;
    for (_, job) in &waiting {
        let is_selected = selected == Some(index);
        if is_selected {
            selected_line = Some(lines.len());
        }
        lines.push(waiting_row(index + 1, job, app, theme, width, is_selected));
        index += 1;
    }

    // -- RUNNING --------------------------------------------------------------
    lines.push(Line::from(""));
    let title = vec![
        Span::styled("RUNNING", theme.section()),
        Span::styled(format!(" · on a worker · {}", started.len()), theme.muted()),
    ];
    lines.push(rule(width, title, theme));
    let mut h = Cells::new();
    h.push(" ", Style::default());
    h.cell_right("#", 3, theme.section());
    h.gap(2);
    h.cell("RUNNING", 10, theme.section());
    h.cell("USER → PERSON", WHO, theme.section());
    h.cell("DATA SOURCE", SOURCE, theme.section());
    h.cell("QUERY", query_cells(width), theme.section());
    h.push("→ CLICKHOUSE · mem · cores · done", theme.section());
    lines.push(h.line(width, Style::default()));
    if started.is_empty() {
        lines.push(Line::from(Span::styled("     no jobs on a worker", theme.muted())));
    }
    for (n, (_, job)) in started.iter().enumerate() {
        let is_selected = selected == Some(index);
        if is_selected {
            selected_line = Some(lines.len());
        }
        lines.push(running_row(n + 1, job, app, theme, width, is_selected));
        index += 1;
    }

    let height = area.height as usize;
    let offset = scroll_into_view(app.viewport.queue.get(), selected_line, height, lines.len());
    app.viewport.queue.set(offset);
    let visible: Vec<Line<'static>> = lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn queue_row(row: &QueueRow, app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let saturated = row.saturated();
    let oldest = row.oldest_wait_s.unwrap_or(0);
    let count_sev = severity::wait(oldest, saturated);
    let wait_sev = severity::wait(oldest, false);
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.cell(&row.name, NAME, theme.strong());
    cells.cell_right(
        &row.waiting.to_string(),
        8,
        if row.waiting > 0 { theme.sev(count_sev).add_modifier(Modifier::BOLD) } else { theme.muted() },
    );
    cells.gap(3);
    cells.cell(
        &format!("{}{}", fmt::opt_dur(row.oldest_wait_s), wait_sev.mark()),
        10,
        theme.sev(wait_sev),
    );
    let worker_sev = if saturated { Severity::Warn } else { Severity::None };
    let mut workers = Cells::new();
    let slots = dots(row.workers_busy, row.workers_total, theme.bar_fill(worker_sev), theme);
    let spaced = !slots.is_empty();
    workers.spans(slots);
    workers.push(
        format!(
            "{}{}/{} {}",
            if spaced { " " } else { "" },
            row.workers_busy,
            row.workers_total,
            if saturated { "busy" } else if row.workers_busy == 0 { "idle" } else { "working" }
        ),
        theme.sev(worker_sev),
    );
    let w = workers.width();
    cells.spans(workers.into_spans());
    cells.gap(26usize.saturating_sub(w));
    cells.cell_right(
        &row.failed_5m.to_string(),
        9,
        if row.failed_5m > 0 { theme.sev(Severity::Warn) } else { theme.muted() },
    );
    cells.gap(3);
    let series = app.history.queue(&row.name);
    let top = series
        .and_then(|s| s.max_in(crate::history::WINDOW_S))
        .unwrap_or(0.0)
        .max(10.0);
    cells.spans(sparkline(series, crate::history::secs(app.queue.taken_at), 16, Scale::Fixed(0.0, top), |v| {
        theme.bar_fill(if v >= 10.0 { Severity::Warn } else { Severity::None })
    }));
    cells.line(width, Style::default())
}

fn query_label(job: &Job) -> String {
    match (job.redash_query_id, &job.query_name) {
        (Some(id), Some(name)) => format!("#{id} {name}"),
        (Some(id), None) => format!("#{id}"),
        (None, Some(name)) => name.clone(),
        (None, None) => "—".to_string(),
    }
}

fn who(job: &Job, cells: &mut Cells, theme: &Theme) {
    let start = cells.width();
    match (&job.user, &job.person) {
        (Some(user), Some(person)) => {
            cells.push(format!("{user} → "), theme.muted());
            cells.push(crate::attrib::display_person(person), theme.person());
        }
        (Some(user), None) => {
            cells.push(user.clone(), theme.text());
        }
        (None, Some(person)) => {
            cells.push(crate::attrib::display_person(person), theme.person());
        }
        (None, None) => {
            cells.push("—", theme.faint());
        }
    }
    let used = cells.width() - start;
    if used < WHO {
        cells.gap(WHO - used);
    }
}

fn waiting_row(index: usize, job: &Job, app: &App, theme: &Theme, width: usize, selected: bool) -> Line<'static> {
    let saturated = app.queue.queue(&job.queue).is_some_and(QueueRow::saturated);
    let sev = severity::wait(job.age_s, saturated);
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.cell_right(&index.to_string(), 3, theme.muted());
    cells.gap(2);
    cells.cell_right(&fmt::dur(job.age_s as f64), 6, theme.sev(sev).add_modifier(Modifier::BOLD));
    cells.push(format!("{} ", if sev.mark().is_empty() { "  " } else { sev.mark() }), theme.sev(sev));
    // How far along to the 3-minute red line it is.
    cells.spans(bar(Some(job.age_s as f64 / 180.0 * 100.0), 8, theme.bar_fill(sev), theme));
    cells.gap(1);
    who(job, &mut cells, theme);
    cells.cell(&job.queue, 12, theme.muted());
    cells.cell(job.data_source.as_deref().unwrap_or("—"), SOURCE, theme.accent());
    cells.push(query_label(job), theme.text());
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

fn running_row(index: usize, job: &Job, app: &App, theme: &Theme, width: usize, selected: bool) -> Line<'static> {
    let target = job.clickhouse_target();
    let runaway = target.is_some_and(|(node, id)| app.query_is_runaway(node, id));
    // A running job inherits the runaway state of its ClickHouse query (§2.8).
    let sev = if runaway { Severity::Crit } else { severity::elapsed(job.age_s as f64) };
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.cell_right(&index.to_string(), 3, theme.muted());
    cells.gap(2);
    cells.cell_right(&fmt::dur(job.age_s as f64), 7, theme.sev(sev).add_modifier(Modifier::BOLD));
    cells.push(if runaway { " ✕ " } else { "   " }, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    who(job, &mut cells, theme);
    cells.cell(job.data_source.as_deref().unwrap_or("—"), SOURCE, theme.accent());
    cells.cell(&query_label(job), query_cells(width), theme.text());
    match target {
        Some((node, query_id)) => {
            // The query id is in the drawer; the row has room for what matters about it.
            cells.push("→ ", theme.faint());
            cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
            // The ClickHouse side of the stitch, live: what the query costs right now.
            let detail = app.with_view(|view| {
                view.nodes
                    .iter()
                    .filter(|n| n.node.name == node)
                    .flat_map(|n| n.users.iter())
                    .flat_map(|u| u.queries.iter())
                    .find(|q| q.query.query_id == query_id)
                    .map(|q| (q.query.memory_bytes, q.cores, q.progress))
            });
            if let Some(Some((bytes, cores, progress))) = detail {
                cells.push(format!(" {} {cores:.1}c", short_bytes(bytes)), theme.text2());
                if let Some(p) = progress {
                    cells.push(format!(" {}", fmt::pct0(p * 100.0)), theme.accent());
                }
            }
        }
        // A started job with no ClickHouse row is a worker doing something else.
        None => {
            cells.push("→ not in ClickHouse yet", theme.faint());
        }
    }
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_fits_in_a_few_characters() {
        assert_eq!(short_bytes(21 * 1024 * 1024 * 1024 + 200 * 1024 * 1024), "21.2G");
        assert_eq!(short_bytes(312 * 1024 * 1024), "312M");
        assert_eq!(short_bytes(10), "<1M");
    }

    #[test]
    fn the_query_column_grows_with_the_terminal() {
        assert_eq!(query_cells(80), 16, "never narrower than 16");
        assert!(query_cells(140) > query_cells(116));
        assert_eq!(query_cells(400), 48, "nor wider than 48");
    }
}
