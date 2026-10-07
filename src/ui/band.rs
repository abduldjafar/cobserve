//! The lines on the shelf, on every view: the fleet at a glance, the Redash queue strip (§1,
//! §6.3) and, with the height for it, this machine (`local.rs`, view 0).

use super::widgets::{dots, sparkline, thin_bar, Cells, Scale, PCT_SHAPE};
use crate::app::App;
use crate::fmt;
use crate::model::{fleet_totals, FleetTotals};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let totals = app.with_view(fleet_totals);
    let fleet = fleet_line(app, totals.as_ref(), theme, width);
    frame.render_widget(Paragraph::new(fleet), Rect::new(area.x, area.y, area.width, 1));
    if area.height >= 2 {
        let strip = queue_strip(app, theme, width);
        frame.render_widget(Paragraph::new(strip), Rect::new(area.x, area.y + 1, area.width, 1));
    }
    if area.height >= 3 {
        let local = super::local::band_line(app, theme, LABEL, width);
        frame.render_widget(Paragraph::new(local), Rect::new(area.x, area.y + 2, area.width, 1));
    }
}

/// The width of the labels at the start of both lines, so what follows them lines up.
const LABEL: usize = 8;

/// `FLEET   8 nodes · 2 hot   mem ━━━━━━━━╺━━━━━ 58.1% 372/640 GiB ▁▂▃▅   cpu …   queries 14 · 3 ✕`
///
/// Built from the most to the least important piece, and each piece only if it still fits, so
/// a narrow terminal loses the sparklines before it loses a number.
fn fleet_line(app: &App, totals: Option<&FleetTotals>, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.cell("FLEET", LABEL, theme.section());
    let Some(totals) = totals else {
        cells.push("waiting for the first snapshot…", theme.muted());
        return cells.line(width, theme.text());
    };

    cells.push(fmt::plural(totals.nodes, "node", "nodes"), theme.text());
    let down = totals.nodes - totals.reachable;
    if down > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{down} down"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if totals.hot > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{} hot", totals.hot), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }

    // Each resource: label, bar, percentage, used/total, sparkline — and the queries. What does
    // not fit gives way in that order, from the end: sparklines, then the used/total, then bar
    // length; a percentage is never dropped.
    let now = app.history.now();
    let resources = [
        (
            "mem",
            totals.mem_pct(),
            {
                let (used, total) = (fmt::gib(totals.mem_used), fmt::gib(totals.mem_total));
                if total >= 100.0 {
                    format!("{used:.0}/{total:.0} GiB")
                } else {
                    format!("{used:.1}/{total:.1} GiB")
                }
            },
            &app.history.fleet_mem_pct,
        ),
        (
            "cpu",
            totals.cpu_pct(),
            if totals.cores >= 100.0 {
                format!("{:.0}/{:.0} cores", totals.busy_cores, totals.cores)
            } else {
                format!("{:.1}/{:.0} cores", totals.busy_cores, totals.cores)
            },
            &app.history.fleet_cpu_pct,
        ),
    ];
    let mut queries = Cells::new();
    queries.push("   queries ", theme.faint());
    queries.push(totals.queries.to_string(), theme.strong());
    if totals.runaways > 0 {
        queries.push(" · ", theme.faint());
        queries.push(format!("{} ✕", totals.runaways), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let room = width.saturating_sub(cells.width() + queries.width());
    let shapes = [(16, true, 16), (12, true, 10), (12, true, 0), (10, true, 0), (8, true, 0), (10, false, 0), (6, false, 0), (0, false, 0)];
    for (bar_cells, absolute_shown, spark_cells) in shapes {
        let mut pieces = Cells::new();
        for (label, pct, absolute, series) in &resources {
            let sev = severity::node(*pct);
            pieces.push(format!("   {label} "), theme.faint());
            if bar_cells > 0 {
                pieces.spans(thin_bar(*pct, bar_cells, theme.bar_fill(sev), theme));
                pieces.push(" ", theme.text());
            }
            pieces.push(fmt::pct(*pct), theme.sev(sev).add_modifier(Modifier::BOLD));
            if absolute_shown {
                pieces.push(format!(" {absolute}"), theme.text2());
            }
            if spark_cells > 0 && !series.is_empty() {
                pieces.push(" ", theme.text());
                pieces.spans(sparkline(Some(series), now, spark_cells, PCT_SHAPE, |v| theme.bar_fill(severity::node(Some(v)))));
            }
        }
        if pieces.width() <= room || bar_cells == 0 {
            cells.spans(pieces.into_spans());
            break;
        }
    }
    if cells.width() + queries.width() <= width {
        cells.spans(queries.into_spans());
    }
    cells.line(width, theme.text())
}

/// `REDASH  15 waiting ▁▂▅▇ · oldest 1m43s ▲ · 6 running · ●●●●●●○ 6/7 workers busy · 3 stale`,
/// or `REDASH  unreachable (HTTP 401)` — never blank (§1, §6.3).
pub fn queue_strip(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.cell("REDASH", LABEL, theme.section());
    let hint = "[2] queue";
    let room = width.saturating_sub(fmt::width(hint) + 2);

    if !app.queue.reachable {
        match app.queue.error.as_deref() {
            Some(crate::model::QUEUE_NOT_CONFIGURED) => {
                cells.push("not configured", theme.muted());
                cells.push(
                    " · add redash: to the --credential file (or set REDASH_URL, REDASH_ADMIN_API_KEY)",
                    theme.faint(),
                );
            }
            Some(crate::model::QUEUE_NOT_POLLED) => {
                cells.push("connecting…", theme.muted());
            }
            other => {
                // A Redash that answered is not unreachable: it refused the key, failed, or
                // sent something that is not the queue.
                let reason = other.unwrap_or("no answer");
                let (word, detail) = if let Some(detail) = reason.strip_prefix(crate::sources::redash::UNREADABLE) {
                    ("unreadable", detail)
                } else if reason.starts_with("HTTP 401") || reason.starts_with("HTTP 403") {
                    ("refused", reason)
                } else if reason.starts_with("HTTP ") {
                    ("error", reason)
                } else {
                    ("unreachable", reason)
                };
                cells.push(format!("{word} ({detail})"), theme.sev(Severity::Warn));
            }
        }
        return cells.line(width, theme.text());
    }

    let queue = &app.queue;
    if queue.queues.is_empty() {
        cells.push("no queues reported", theme.sev(Severity::Warn));
        return cells.line(width, theme.text());
    }

    // Waiting first: it is what people feel. Its severity is the worst queue's — the oldest
    // wait, made worse when every worker of that queue is busy.
    let waiting = queue.total_waiting();
    let worst = queue
        .queues
        .iter()
        .filter(|row| row.waiting > 0)
        .max_by_key(|row| (severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated()), row.oldest_wait_s, row.waiting));
    let count_sev = worst.map_or(Severity::None, |row| severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated()));
    cells.push(
        format!("{waiting} waiting"),
        if waiting > 0 { theme.sev(count_sev).add_modifier(Modifier::BOLD) } else { theme.muted() },
    );
    // A flat line of zeros says nothing a "0" does not.
    let series = &app.history.queue_waiting;
    if width >= 110 && series.max_in(crate::history::WINDOW_S).is_some_and(|m| m > 0.0) {
        let top = series.max_in(crate::history::WINDOW_S).unwrap_or(0.0).max(10.0);
        cells.push(" ", theme.text());
        cells.spans(sparkline(Some(series), crate::history::secs(queue.taken_at), 8, Scale::Fixed(0.0, top), |_| {
            theme.bar_fill(count_sev)
        }));
    }
    // Only Redis knows how long; without it there is no "oldest" to show, not a dash.
    if let Some(oldest) = worst.and_then(|row| row.oldest_wait_s) {
        let wait_sev = severity::wait(oldest, false);
        cells.push(" · ", theme.faint());
        cells.push("oldest ", theme.muted());
        cells.push(fmt::dur(oldest as f64), theme.sev(wait_sev).add_modifier(Modifier::BOLD));
        cells.push(wait_sev.mark(), theme.sev(wait_sev));
    }

    cells.push(" · ", theme.faint());
    let running = queue.total_running();
    cells.push(format!("{running} running"), if running > 0 { theme.strong() } else { theme.muted() });

    cells.push(" · ", theme.faint());
    // Busy workers are a problem only with jobs waiting behind them.
    let full = worst.is_some_and(|row| row.saturated());
    let worker_sev = if full { Severity::Warn } else { Severity::None };
    let worker_dots = dots(queue.workers_busy, queue.workers_total, theme.bar_fill(worker_sev), theme);
    if !worker_dots.is_empty() {
        cells.spans(worker_dots);
        cells.push(" ", theme.text());
    }
    let workers = match (queue.workers_total, queue.workers_busy) {
        (0, _) => "no live worker".to_string(),
        (total, 0) => format!("{} idle", fmt::plural(total as usize, "worker", "workers")),
        (total, busy) => format!("{busy}/{total} workers busy"),
    };
    let worker_style = if queue.workers_total == 0 || full { theme.sev(Severity::Warn) } else { theme.text2() };
    cells.push(workers, worker_style);

    let stale = queue.total_stale();
    if stale > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{stale} stale"), theme.muted());
    }

    let mut line = Cells::new();
    let body = cells.line_unpadded(room);
    line.spans(body.spans);
    line.pad_to(width.saturating_sub(fmt::width(hint)));
    line.push(hint, theme.faint());
    line.line(width, theme.text())
}
