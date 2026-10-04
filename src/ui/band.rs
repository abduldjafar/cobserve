//! The two lines under the header, on every view: the fleet at a glance, and the Redash
//! queue strip (§1, §6.3).

use super::widgets::{bar, dots, sparkline, Cells, Scale, PCT_SHAPE};
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
}

/// `FLEET 8 nodes · 2 hot   MEM ▕████▍  ▏ 58.1% 372/640 GiB ▁▂▃▅   CPU …   QUERIES 14 · 3 ✕`
///
/// Built from the most to the least important piece, and each piece only if it still fits, so
/// a narrow terminal loses the sparklines before it loses a number.
fn fleet_line(app: &App, totals: Option<&FleetTotals>, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("FLEET", theme.section());
    let Some(totals) = totals else {
        cells.push("  waiting for the first snapshot…", theme.muted());
        return cells.line(width, theme.text());
    };

    cells.push(format!("  {}", fmt::plural(totals.nodes, "node", "nodes")), theme.text());
    let down = totals.nodes - totals.reachable;
    if down > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{down} down"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if totals.hot > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{} hot", totals.hot), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }

    // Each resource: label, bar, percentage, used/total, sparkline. The bar and the
    // sparkline shrink and go first when room runs out.
    let now = app.history.now();
    let bar_cells = if width >= 140 { 14 } else if width >= 116 { 10 } else { 6 };
    let spark_cells = if width >= 150 { 16 } else if width >= 116 { 10 } else { 0 };
    let resources = [
        (
            "MEM",
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
            "CPU",
            totals.cpu_pct(),
            if totals.cores >= 100.0 {
                format!("{:.0}/{:.0} cores", totals.busy_cores, totals.cores)
            } else {
                format!("{:.1}/{:.0} cores", totals.busy_cores, totals.cores)
            },
            &app.history.fleet_cpu_pct,
        ),
    ];
    for (label, pct, absolute, series) in resources {
        let sev = severity::node(pct);
        let mut piece = Cells::new();
        piece.push(format!("   {label} "), theme.muted());
        piece.spans(bar(pct, bar_cells, theme.bar_fill(sev), theme));
        piece.push(format!(" {}", fmt::pct(pct)), theme.sev(sev).add_modifier(Modifier::BOLD));
        piece.push(format!(" {absolute}"), theme.text2());
        if spark_cells > 0 && !series.is_empty() {
            piece.push(" ", theme.text());
            piece.spans(sparkline(Some(series), now, spark_cells, PCT_SHAPE, |v| {
                theme.bar_fill(severity::node(Some(v)))
            }));
        }
        if cells.width() + piece.width() <= width {
            cells.spans(piece.into_spans());
        }
    }

    let mut queries = Cells::new();
    queries.push("   QUERIES ", theme.muted());
    queries.push(totals.queries.to_string(), theme.strong());
    if totals.runaways > 0 {
        queries.push(" · ", theme.faint());
        queries.push(format!("{} ✕", totals.runaways), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if cells.width() + queries.width() <= width {
        cells.spans(queries.into_spans());
    }
    cells.line(width, theme.text())
}

/// `REDASH  12 waiting ▁▂▅▇ · oldest 1m43s ▲ · workers ●●●●●● 6/6 busy · 2 failed/5m`, or
/// `REDASH  unreachable (HTTP 401)` — never blank (§1, §6.3).
pub fn queue_strip(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("REDASH", theme.section());
    cells.push("  ", theme.text());
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
                let reason = other.unwrap_or("no answer");
                cells.push(format!("unreachable ({reason})"), theme.sev(Severity::Warn));
            }
        }
        return cells.line(width, theme.text());
    }

    let Some(q) = app.queue.queue("queries") else {
        cells.push("no queues reported", theme.sev(Severity::Warn));
        return cells.line(width, theme.text());
    };

    let saturated = q.saturated();
    let oldest = q.oldest_wait_s.unwrap_or(0);
    let wait_sev = severity::wait(oldest, false);
    let count_sev = severity::wait(oldest, saturated);

    cells.push(format!("{} waiting", q.waiting), theme.sev(count_sev).add_modifier(Modifier::BOLD));
    if width >= 110 && !app.history.queue_waiting.is_empty() {
        let series = app.history.queue("queries");
        let top = series.and_then(|s| s.max_in(crate::history::WINDOW_S)).unwrap_or(0.0).max(10.0);
        cells.push(" ", theme.text());
        cells.spans(sparkline(series, crate::history::secs(app.queue.taken_at), 8, Scale::Fixed(0.0, top), |_| {
            theme.bar_fill(count_sev)
        }));
    }
    cells.push(" · ", theme.faint());
    cells.push("oldest ", theme.muted());
    cells.push(fmt::opt_dur(q.oldest_wait_s), theme.sev(wait_sev).add_modifier(Modifier::BOLD));
    cells.push(wait_sev.mark(), theme.sev(wait_sev));
    cells.push(" · ", theme.faint());
    cells.push("workers ", theme.muted());
    let worker_sev = if saturated { Severity::Warn } else { Severity::None };
    let worker_dots = dots(q.workers_busy, q.workers_total, theme.bar_fill(worker_sev), theme);
    if !worker_dots.is_empty() {
        cells.spans(worker_dots);
        cells.push(" ", theme.text());
    }
    cells.push(
        format!("{}/{} {}", q.workers_busy, q.workers_total, if saturated { "busy" } else { "idle" }),
        theme.sev(worker_sev),
    );
    if q.failed_5m > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("{} failed/5m", q.failed_5m), theme.sev(Severity::Warn));
    }
    let others: u32 = app
        .queue
        .queues
        .iter()
        .filter(|row| row.name != "queries")
        .map(|row| row.waiting)
        .sum();
    if others > 0 {
        cells.push(" · ", theme.faint());
        cells.push(format!("+{others} in other queues"), theme.muted());
    }

    let mut line = Cells::new();
    let body = cells.line_unpadded(room);
    line.spans(body.spans);
    line.pad_to(width.saturating_sub(fmt::width(hint)));
    line.push(hint, theme.faint());
    line.line(width, theme.text())
}
