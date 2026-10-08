//! The shelf as cards, when the terminal is wide and tall enough: ClickHouse, Redash and this
//! machine side by side, each in a rounded frame that takes its colour from its severity — the
//! same numbers as the band's lines (`band.rs`), with the room to set them out.
//!
//! ```text
//! ╭ ClickHouse ───────────── ▲ 2 hot ╮  ╭ Redash ──────────── ▲ 1m46s ╮  ╭ This Mac ───── normal ╮
//! │ mem ━━━━━━━━╺━━━━━━ 37.2% 262/704 GiB │ 18 waiting ▁▂▅▇ · oldest 1m46s │ cpu ━━╺━━━ 22.1% 2.2/10 │
//! │ cpu ━━━━━╸━━━━━━━━━ 29.5% 52/176 cores│ ●●●●●●○ 6/7 workers busy       │ mem ━━━━━ 83.1% 26.6/32 │
//! │ 8 nodes · queries 15 · 5 ✕            │ 6 running · 3 stale       [2]  │ swap 2.2 GiB · top …    │
//! ╰─────────────────────────────────────╯  ╰──────────────────────────────╯  ╰─────────────────────────╯
//! ```

use super::widgets::{dots, sparkline, thin_bar, Cells, Scale, PCT_SHAPE};
use crate::app::{App, Hit, View};
use crate::fmt;
use crate::history::Series;
use crate::model::fleet_totals;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

/// The narrowest terminal that gets cards; below it, the band's lines.
pub const FROM_WIDTH: u16 = 150;
/// A card's height: its frame and three lines.
pub const HEIGHT: u16 = 5;
const GAP: u16 = 2;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height < HEIGHT || area.width < 3 * 20 {
        return;
    }
    let local = app.local.latest.is_some();
    let count: u16 = if local { 3 } else { 2 };
    let each = (area.width - GAP * (count - 1)) / count;
    let mut x = area.x;
    let mut next = |index: u16| {
        // The last card takes what the division left over, so the row ends at the margin.
        let width = if index + 1 == count { area.x + area.width - x } else { each };
        let rect = Rect::new(x, area.y, width, HEIGHT);
        x += width + GAP;
        rect
    };
    // A click on a card opens its view, as its hint says.
    let (a, b) = (next(0), next(1));
    let c = local.then(|| next(2));
    {
        let mut hits = app.viewport.hits.borrow_mut();
        hits.push((a, Hit::View(View::Nodes)));
        hits.push((b, Hit::View(View::Queue)));
        if let Some(c) = c {
            hits.push((c, Hit::View(View::Local)));
        }
    }
    fleet(frame, app, theme, a);
    redash(frame, app, theme, b);
    if let Some(c) = c {
        machine(frame, app, theme, c);
    }
}

/// A frame: its title on the left, a word on how it is on the right, its colour the severity's.
fn card(frame: &mut Frame, theme: &Theme, area: Rect, title: &str, status: Vec<Span<'static>>, sev: Severity, lines: Vec<Line<'static>>) {
    let border = match sev {
        Severity::Crit | Severity::Warn => Style::default().fg(theme.sev_fg(sev)),
        _ => theme.rule(),
    };
    let mut right = vec![Span::raw(" ")];
    right.extend(status);
    right.push(Span::raw(" "));
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(vec![Span::raw(" "), Span::styled(title.to_string(), theme.section()), Span::raw(" ")]))
        .title(Line::from(right).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `mem ━━━━━━━━╺━━━━━ 37.2% 262/704 GiB ▁▂▃▅` — the bar takes what the numbers leave.
/// `history` is the series and the time it ends at.
fn gauge(label: &str, pct: Option<f64>, sev: Severity, of: &str, history: Option<(&Series, f64)>, theme: &Theme, width: usize) -> Line<'static> {
    let (series, now) = (history.map(|h| h.0), history.map_or(0.0, |h| h.1));
    let pct_text = fmt::pct(pct);
    let fixed = fmt::width(label) + 1 + 1 + 6 + 1 + fmt::width(of);
    let spark = if width >= fixed + 12 + 9 && series.is_some_and(|s| !s.is_empty()) { 8 } else { 0 };
    let bar = width.saturating_sub(fixed + if spark > 0 { spark + 1 } else { 0 }).min(24);
    let mut cells = Cells::new();
    cells.push(format!("{label} "), theme.faint());
    if bar >= 4 {
        cells.spans(thin_bar(pct, bar, theme.bar_fill(sev), theme));
        cells.push(" ", Style::default());
    }
    cells.push(format!("{pct_text:>6}"), theme.sev(sev).add_modifier(Modifier::BOLD));
    cells.push(format!(" {of}"), theme.text2());
    if spark > 0 {
        cells.push(" ", Style::default());
        cells.spans(sparkline(series, now, spark, PCT_SHAPE, |v| theme.bar_fill(severity::node(Some(v)))));
    }
    cells.line(width, Style::default())
}

fn worse(a: Severity, b: Severity) -> Severity {
    let rank = |s: Severity| match s {
        Severity::Crit => 3,
        Severity::Warn => 2,
        Severity::Ok => 1,
        _ => 0,
    };
    if rank(b) > rank(a) { b } else { a }
}

fn fleet(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let Some(t) = app.with_view(fleet_totals) else {
        let line = Line::from(Span::styled("waiting for the first snapshot…", theme.muted()));
        card(frame, theme, area, "ClickHouse", Vec::new(), Severity::None, vec![line]);
        return;
    };
    let down = t.nodes - t.reachable;
    // How the fleet is, by what the band counts: a node down or a runaway is red, a hot node amber.
    let sev = if down > 0 || t.runaways > 0 {
        Severity::Crit
    } else if t.hot > 0 {
        Severity::Warn
    } else {
        Severity::None
    };
    // What makes it that colour, worst first.
    let mut status = Vec::new();
    let mut say = |text: String, sev: Severity| {
        if !status.is_empty() {
            status.push(Span::styled(" · ", theme.faint()));
        }
        status.push(Span::styled(text, theme.sev(sev).add_modifier(Modifier::BOLD)));
    };
    if down > 0 {
        say(format!("{} {down} down", Severity::Crit.glyph()), Severity::Crit);
    }
    if t.runaways > 0 {
        say(format!("{} {} runaway", Severity::Crit.glyph(), t.runaways), Severity::Crit);
    }
    if t.hot > 0 {
        say(format!("{} {} hot", Severity::Warn.glyph(), t.hot), Severity::Warn);
    }
    if status.is_empty() {
        status.push(Span::styled("all well", theme.muted()));
    }
    let now = app.history.now();
    let (used, total) = (fmt::gib(t.mem_used), fmt::gib(t.mem_total));
    let mem_of = if total >= 100.0 { format!("{used:.0}/{total:.0} GiB") } else { format!("{used:.1}/{total:.1} GiB") };
    let cpu_of = if t.cores >= 100.0 { format!("{:.0}/{:.0} cores", t.busy_cores, t.cores) } else { format!("{:.1}/{:.0} cores", t.busy_cores, t.cores) };
    let mut last = Cells::new();
    last.push(fmt::plural(t.nodes, "node", "nodes"), theme.text());
    last.push(" · queries ", theme.faint());
    last.push(t.queries.to_string(), theme.strong());
    if t.runaways > 0 {
        last.push(" · ", theme.faint());
        last.push(format!("{} ✕", t.runaways), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let hint = "[1] nodes";
    if last.width() + 2 + hint.len() <= width {
        last.pad_to(width - hint.len());
        last.push(hint, theme.faint());
    }
    let lines = vec![
        gauge("mem", t.mem_pct(), severity::node(t.mem_pct()), &mem_of, Some((&app.history.fleet_mem_pct, now)), theme, width),
        gauge("cpu", t.cpu_pct(), severity::node(t.cpu_pct()), &cpu_of, Some((&app.history.fleet_cpu_pct, now)), theme, width),
        last.line(width, Style::default()),
    ];
    card(frame, theme, area, "ClickHouse", status, sev, lines);
}

fn redash(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let queue = &app.queue;
    if !queue.reachable || queue.queues.is_empty() {
        // The band's own words for why: not configured, connecting, refused, unreachable.
        let line = super::band::queue_strip(app, theme, width + super::band::LABEL);
        let spans: Vec<Span<'static>> = line.spans.into_iter().collect();
        let mut cells = Cells::new();
        let mut skipped = 0;
        for span in spans {
            // Leave out the strip's REDASH label: the card's title says it.
            if skipped < super::band::LABEL {
                skipped += fmt::width(&span.content);
                continue;
            }
            cells.push(span.content.into_owned(), span.style);
        }
        let sev = if queue.is_placeholder() { Severity::None } else { Severity::Warn };
        card(frame, theme, area, "Redash", Vec::new(), sev, vec![cells.line_unpadded(width)]);
        return;
    }
    let waiting = queue.total_waiting();
    let worst = queue
        .queues
        .iter()
        .filter(|row| row.waiting > 0)
        .max_by_key(|row| (severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated()), row.oldest_wait_s, row.waiting));
    let sev = worst.map_or(Severity::None, |row| severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated()));
    let oldest = worst.and_then(|row| row.oldest_wait_s);

    let mut status = Vec::new();
    match oldest {
        Some(s) if waiting > 0 => {
            let wait_sev = severity::wait(s, false);
            status.push(Span::styled(format!("oldest {}{}", fmt::dur(s as f64), wait_sev.mark()), theme.sev(wait_sev).add_modifier(Modifier::BOLD)));
        }
        _ if waiting == 0 => status.push(Span::styled("nothing waits", theme.muted())),
        _ => {}
    }

    let mut first = Cells::new();
    first.push(format!("{waiting} waiting"), if waiting > 0 { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.muted() });
    let series = &app.history.queue_waiting;
    if series.max_in(crate::history::WINDOW_S).is_some_and(|m| m > 0.0) {
        let top = series.max_in(crate::history::WINDOW_S).unwrap_or(0.0).max(10.0);
        // At the right end, where the newest of it is.
        let spark = width.saturating_sub(first.width() + 2).min(24);
        first.pad_to(width - spark);
        first.spans(sparkline(Some(series), crate::history::secs(queue.taken_at), spark, Scale::Fixed(0.0, top), |_| theme.bar_fill(sev)));
    }

    let full = worst.is_some_and(|row| row.saturated());
    let worker_sev = if full { Severity::Warn } else { Severity::None };
    let mut second = Cells::new();
    let worker_dots = dots(queue.workers_busy, queue.workers_total, theme.bar_fill(worker_sev), theme);
    if !worker_dots.is_empty() {
        second.spans(worker_dots);
        second.push(" ", Style::default());
    }
    let workers = match (queue.workers_total, queue.workers_busy) {
        (0, _) => "no live worker".to_string(),
        (total, 0) => format!("{} idle", fmt::plural(total as usize, "worker", "workers")),
        (total, busy) => format!("{busy}/{total} workers busy"),
    };
    second.push(workers, if queue.workers_total == 0 || full { theme.sev(Severity::Warn) } else { theme.text2() });

    let mut third = Cells::new();
    let running = queue.total_running();
    third.push(format!("{running} running"), if running > 0 { theme.strong() } else { theme.muted() });
    let stale = queue.total_stale();
    if stale > 0 {
        third.push(" · ", theme.faint());
        third.push(format!("{stale} stale"), theme.muted());
    }
    let hint = "[2] queue";
    if third.width() + 2 + hint.len() <= width {
        third.pad_to(width - hint.len());
        third.push(hint, theme.faint());
    }
    let lines = vec![first.line(width, Style::default()), second.line(width, Style::default()), third.line(width, Style::default())];
    card(frame, theme, area, "Redash", status, sev, lines);
}

fn machine(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let local = &app.local;
    let title = "This Mac";
    let Some(sample) = local.latest.as_ref() else { return };
    if let (Some(why), false) = (&sample.error, local.shown()) {
        let line = Line::from(Span::styled(format!("could not be read: {why}"), theme.sev(Severity::Warn)));
        card(frame, theme, area, title, Vec::new(), Severity::Warn, vec![line]);
        return;
    }
    let cpu_pct = local.cpu.map(|c| c.busy_pct);
    let cpu_sev = severity::node(cpu_pct);
    // Memory takes the pressure's colour, never the used share's (DESIGN.md §13, LOCAL).
    let pressure = sample.pressure;
    let mem_sev = match pressure.map(|p| p.severity()) {
        Some(Severity::Ok) | None => Severity::None,
        Some(s) => s,
    };
    let mut status = Vec::new();
    if let Some(p) = pressure {
        let sev = p.severity();
        status.push(Span::styled("pressure ", theme.faint()));
        status.push(Span::styled(p.word(), if sev == Severity::Ok { theme.text2() } else { theme.sev(sev).add_modifier(Modifier::BOLD) }));
    }
    let now = crate::history::secs(app.clock);
    let busy = local.cpu.map_or("—".to_string(), |c| format!("{:.1}", c.busy_cores));
    let cpu_of = format!("{busy}/{} cores", sample.cores);
    let mut lines = vec![gauge("cpu", cpu_pct, cpu_sev, &cpu_of, Some((&local.cpu_pct, now)), theme, width)];
    match sample.memory {
        Some(m) => {
            let of = format!("{:.1}/{:.1} GiB", fmt::gib(m.used()), fmt::gib(m.total));
            lines.push(gauge("mem", m.used_pct(), mem_sev, &of, Some((&local.mem_pct, now)), theme, width));
        }
        None => lines.push(Line::from(Span::styled("mem not read", theme.muted()))),
    }
    // Its last line: what Claude Code and OpenCode used today, when they used anything — the
    // busiest process and the swap otherwise (view 0 has both either way).
    let ai = super::usage::glance(app, theme);
    let quiet = ai.is_none();
    let mut last = ai.unwrap_or_default();
    if quiet && let Some((top, cores)) = local.top() {
        last.push("top ", theme.faint());
        last.push(top.name().to_string(), theme.text());
        last.push(format!(" {cores:.2}"), theme.text2());
    }
    if quiet && let Some((used, _)) = sample.swap.filter(|s| s.0 > 0) {
        last.push(" · swap ", theme.faint());
        last.push(format!("{:.1} GiB", fmt::gib(used)), theme.text2());
    }
    let hint = "[0] local";
    if last.width() + 2 + hint.len() <= width {
        last.pad_to(width - hint.len());
        last.push(hint, theme.faint());
    }
    lines.push(last.line(width, Style::default()));
    card(frame, theme, area, title, status, worse(cpu_sev, mem_sev), lines);
}
