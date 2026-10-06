//! View 6: Jira — your tickets in the board's columns.
//!
//! The board's flow first, in counts (`In progress 4 › In Review 4 › Feedback 1 › Done 16 in 7
//! days`), then each column under its rule: the ticket's key, its summary with the team's tag
//! drawn faint so the words stand out, its priority, how long it has been in the column — or
//! how long ago it was finished — its due date against today and the time logged on it. The
//! drawer says the rest of the ticket under the cursor; ⏎ opens its page.

use super::widgets::{rule, Cells};
use crate::app::{scroll_into_view, App, Feed, Hit};
use crate::fmt;
use crate::jira::{self, Board, Column, Ticket};
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Columns are two spaces apart, so a full cell never runs into the next one.
const GAP: usize = 2;
/// The narrowest a summary gets before the cells after it give way.
const SUMMARY_MIN: usize = 24;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let board = &app.jira;
    let width = area.width as usize;
    let height = area.height as usize;
    if !board.reachable {
        let (text, style) = match board.error.as_deref() {
            Some(jira::NOT_CONFIGURED) => (
                "  Jira is not configured: set JIRA_URL and JIRA_TOKEN (a personal access token, from your Jira profile), or add jira: url, token to the --credential file".to_string(),
                theme.muted(),
            ),
            Some(jira::NOT_READ) | None => ("  reading Jira…".to_string(), theme.muted()),
            Some(why) => (format!("  Jira could not be read: {why}"), theme.sev(Severity::Warn)),
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))).wrap(Wrap { trim: false }), area);
        return;
    }

    let now = app.now();
    let today = jira::today(now, app.time.offset_s(now));
    let columns = board.columns();
    let others = board.others();
    let cells = Widths::of(board, today, width);
    let selected = app.jira_selection();

    let mut lines: Vec<Line<'static>> = vec![title_line(app, theme, width), flow_line(board, &columns, today, theme, width), Line::from("")];
    // The month's time, as a chart, when there is height for it and the columns under it.
    if board.worklogs_read && height >= 16 {
        let bars = if height >= 30 { 3 } else { 2 };
        lines.extend(month_chart(board, now, app.time.offset_s(now), bars, theme, width));
        lines.push(Line::from(""));
    }
    lines.push(header(&cells, theme, width));
    let mut rows: Vec<usize> = Vec::new();
    let mut selected_line = None;
    for column in &columns {
        lines.push(column_rule(board, column, theme, width));
        for ticket in &column.tickets {
            if selected == Some(ticket.key.as_str()) {
                selected_line = Some(lines.len());
            }
            rows.push(lines.len());
            lines.push(ticket_line(ticket, column.finished, selected == Some(ticket.key.as_str()), &cells, now, today, theme, width));
        }
    }
    if !others.is_empty() {
        lines.push(rule(
            width,
            vec![Span::styled("MOVED ON", theme.section()), Span::styled(format!(" · {} since the last read", others.len()), theme.muted())],
            theme,
        ));
        for ticket in &others {
            if selected == Some(ticket.key.as_str()) {
                selected_line = Some(lines.len());
            }
            rows.push(lines.len());
            lines.push(ticket_line(ticket, false, selected == Some(ticket.key.as_str()), &cells, now, today, theme, width));
        }
    }

    let len = lines.len();
    let offset = scroll_into_view(app.viewport.jira.get(), selected_line, height, len);
    app.viewport.jira.set(offset);
    let mut hits = app.viewport.hits.borrow_mut();
    for (index, &line) in rows.iter().enumerate() {
        if line >= offset && line < offset + height {
            hits.push((Rect::new(area.x, area.y + (line - offset) as u16, area.width, 1), Hit::Ticket(index)));
        }
    }
    drop(hits);
    let visible: Vec<Line<'static>> = lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

/// `jira.example.net · Jira 9.12.1 · Sam Example                          read 12s ago`
fn title_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let board = &app.jira;
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.push(board.host().unwrap_or_else(|| "Jira".to_string()), theme.strong());
    if let Some(version) = &board.version {
        cells.push(format!(" · Jira {version}"), theme.muted());
    }
    if let Some(user) = board.user.as_deref().filter(|u| !u.is_empty()) {
        cells.push(" · ", theme.muted());
        cells.push(user.to_string(), theme.person());
    }
    let right = freshness(board.age(app.clock), board.error.as_deref(), app.is_reading(Feed::Jira), theme, width / 2);
    cells.pad_to(width.saturating_sub(right.width()));
    cells.spans(right.into_spans());
    cells.line(width, Style::default())
}

/// How old what is on screen is — and, when the last read failed, why it is not newer.
pub fn freshness(age: Option<std::time::Duration>, error: Option<&str>, reading: bool, theme: &Theme, room: usize) -> Cells {
    let mut cells = Cells::new();
    let age = age.map(|a| fmt::dur(a.as_secs_f64())).unwrap_or_else(|| "—".into());
    if reading {
        cells.push("reading… ", theme.accent());
    }
    match error {
        Some(why) => {
            cells.push(format!("◌ read {age} ago · "), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            let left = room.saturating_sub(cells.width() + 1);
            cells.push(format!("{} ", fmt::truncate(why, left)), theme.sev(Severity::Warn));
        }
        None => {
            cells.push(format!("read {age} ago "), theme.faint());
        }
    }
    cells
}

/// `In progress 4 › In Review 4 › Feedback 1 › Done 16 in 7 days     ✖ 1 overdue`
fn flow_line(board: &Board, columns: &[Column<'_>], today: i64, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("  ", Style::default());
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            cells.push("  ›  ", theme.faint());
        }
        let count = column.tickets.len();
        cells.push(column.status.to_string(), if count > 0 { theme.text2() } else { theme.muted() });
        cells.push(" ", Style::default());
        cells.push(count.to_string(), if count > 0 { theme.accent().add_modifier(Modifier::BOLD) } else { theme.faint() });
        if column.finished && board.done_days > 0 {
            cells.push(format!(" in {}", fmt::plural(board.done_days as usize, "day", "days")), theme.faint());
        }
    }
    // What a due date says about the open tickets, counted.
    let open = columns.iter().filter(|c| !c.finished).flat_map(|c| c.tickets.iter());
    let (mut overdue, mut soon) = (0, 0);
    for ticket in open {
        match ticket.due_day().map(|d| jira::due_severity(d, today, false)) {
            Some(Severity::Crit) => overdue += 1,
            Some(Severity::Warn) => soon += 1,
            _ => {}
        }
    }
    if overdue > 0 {
        cells.push("     ", Style::default());
        cells.push(format!("✖ {overdue} overdue"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if soon > 0 {
        cells.push(if overdue > 0 { "  " } else { "     " }, Style::default());
        cells.push(format!("▲ {soon} due soon"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    cells.line(width, Style::default())
}

/// The cells of a ticket's row, as wide as what is in them; what does not fit gives way — the
/// time logged first, then the due date. Both stay in the drawer.
#[derive(Debug, Clone, Copy)]
struct Widths {
    key: usize,
    summary: usize,
    priority: usize,
    age: usize,
    due: usize,
    logged: usize,
}

impl Widths {
    fn of(board: &Board, today: i64, width: usize) -> Widths {
        let rows = board.rows();
        let key = rows.iter().map(|t| fmt::width(&t.key)).max().unwrap_or(8).clamp(8, 14);
        let priority = rows.iter().filter(|t| !t.priority_unset()).filter_map(|t| t.priority.as_deref().map(fmt::width)).max().unwrap_or(1).clamp(8, 10);
        let finished = |t: &Ticket| board.statuses.last().is_some_and(|s| s.eq_ignore_ascii_case(&t.status)) && board.statuses.len() > 1;
        let due = rows
            .iter()
            .filter(|t| !finished(t))
            .filter_map(|t| t.due_day().map(|d| fmt::width(&jira::due_label(d, today))))
            .max()
            .unwrap_or(0);
        let logged = rows.iter().filter_map(|t| t.logged_s.map(|s| fmt::width(&jira::logged(s)))).max().unwrap_or(0);
        let mut widths = Widths { key, summary: 0, priority, age: 9, due, logged: logged.max(if logged > 0 { 6 } else { 0 }) };
        let used = |w: &Widths| 2 + w.key + GAP + GAP + w.priority + GAP + w.age + cell(w.due) + cell(w.logged);
        if width.saturating_sub(used(&widths)) < SUMMARY_MIN + 20 {
            widths.logged = 0;
        }
        if width.saturating_sub(used(&widths)) < SUMMARY_MIN + 10 {
            widths.due = 0;
        }
        widths.summary = width.saturating_sub(used(&widths)).max(SUMMARY_MIN);
        widths
    }
}

/// A cell and the gap before it, or nothing for a cell that is not shown.
fn cell(width: usize) -> usize {
    if width > 0 { width + GAP } else { 0 }
}

fn header(w: &Widths, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("  ", Style::default());
    cells.cell("KEY", w.key, theme.section());
    cells.gap(GAP);
    cells.cell("SUMMARY", w.summary, theme.section());
    cells.gap(GAP);
    cells.cell("PRIORITY", w.priority, theme.section());
    cells.gap(GAP);
    cells.cell_right("IN COLUMN", w.age, theme.section());
    if w.due > 0 {
        cells.gap(GAP);
        cells.cell("DUE", w.due, theme.section());
    }
    if w.logged > 0 {
        cells.gap(GAP);
        cells.cell_right("LOGGED", w.logged, theme.section());
    }
    cells.line(width, Style::default())
}

/// `─ IN REVIEW · 4 ────`, `─ DONE · 16 in the last 7 days ────`, `─ FEEDBACK · none ────`
fn column_rule(board: &Board, column: &Column<'_>, theme: &Theme, width: usize) -> Line<'static> {
    let count = column.tickets.len();
    let detail = match (count, column.finished && board.done_days > 0) {
        (0, true) => format!(" · none in the last {}", fmt::plural(board.done_days as usize, "day", "days")),
        (0, false) => " · none".to_string(),
        (n, true) => format!(" · {n} in the last {}", fmt::plural(board.done_days as usize, "day", "days")),
        (n, false) => format!(" · {n}"),
    };
    rule(
        width,
        vec![Span::styled(column.status.to_uppercase(), theme.section()), Span::styled(detail, theme.muted())],
        theme,
    )
}

#[allow(clippy::too_many_arguments)]
fn ticket_line(ticket: &Ticket, finished: bool, selected: bool, w: &Widths, now: i64, today: i64, theme: &Theme, width: usize) -> Line<'static> {
    let quiet = |style: Style| if finished { theme.muted() } else { style };
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    cells.cell(&ticket.key, w.key, quiet(theme.accent()));
    cells.gap(GAP);

    // The team's tag faint, then the words.
    let (tag, words) = jira::split_tag(&ticket.summary);
    let mut summary = Cells::new();
    if let Some(tag) = tag {
        summary.push(format!("{tag} "), theme.faint());
    }
    let room = w.summary.saturating_sub(summary.width());
    summary.push(fmt::truncate(words, room), if finished { theme.text2() } else if selected { theme.strong() } else { theme.text() });
    let used = summary.width();
    cells.spans(summary.into_spans());
    cells.gap(w.summary.saturating_sub(used) + GAP);

    let priority = if ticket.priority_unset() { "—".to_string() } else { ticket.priority.clone().unwrap_or_default() };
    let priority_style = match ticket.priority_severity() {
        _ if finished => theme.muted(),
        Severity::Crit => theme.sev(Severity::Crit).add_modifier(Modifier::BOLD),
        _ if ticket.priority_unset() => theme.faint(),
        _ if ticket.priority.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("high")) => theme.strong(),
        _ => theme.text2(),
    };
    cells.cell(&priority, w.priority, priority_style);
    cells.gap(GAP);

    match (finished, ticket.resolved) {
        (true, Some(at)) => {
            let ago = jira::age(now - at);
            cells.push(" ".repeat(w.age.saturating_sub(fmt::width(&ago) + 2)), Style::default());
            cells.push("✔ ", theme.sev(Severity::Ok));
            cells.push(ago, theme.muted());
        }
        _ => {
            let age = ticket.in_status(now).map(jira::age).unwrap_or_else(|| "—".into());
            cells.cell_right(&age, w.age, theme.text2());
        }
    }
    if w.due > 0 {
        cells.gap(GAP);
        match ticket.due_day() {
            Some(day) if !finished => {
                let sev = jira::due_severity(day, today, false);
                let style = if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text2() };
                cells.cell(&jira::due_label(day, today), w.due, style);
            }
            _ => {
                cells.cell("", w.due, Style::default());
            }
        }
    }
    if w.logged > 0 {
        cells.gap(GAP);
        let logged = ticket.logged_s.map(jira::logged).unwrap_or_default();
        cells.cell_right(&logged, w.logged, theme.muted());
    }
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

/// The month's time logged, as a chart: a line of totals, the hours over each day, a bar a day
/// (eighths of a cell, up to a full day of eight hours or the longest day if longer), and the
/// days under them — weekends faint, today lit.
///
/// ```text
/// LOGGED · October 2026   23h30m in 4 working days · 5h52m a day · today 2h
///     4½ 7  8  7½ ·  ·  2
///  8h ▅▅ ▇▇ ██ ▇▇
///     ██ ██ ██ ██       ▂▂
///      1  2  3  4  5  6  7  8 …
/// ```
fn month_chart(board: &Board, now: i64, offset_s: i64, bars: usize, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let month = jira::Month::of(now, offset_s);
    let days = month.per_day(&board.worklogs);
    let total: u64 = days.iter().sum();
    let today = days.get(month.today as usize - 1).copied().unwrap_or(0);
    let worked = month.working_days_so_far().max(1);

    let mut head = Cells::new();
    head.push("  ", Style::default());
    head.push("LOGGED", theme.section());
    head.push(format!(" · {}   ", month.name), theme.muted());
    head.push(jira::hours(total), theme.strong());
    head.push(format!(" in {}", fmt::plural(worked as usize, "working day", "working days")), theme.muted());
    head.push(" · ", theme.faint());
    head.push(jira::hours(total / u64::from(worked)), theme.text2());
    head.push(" a working day", theme.muted());
    head.push(" · today ", theme.faint());
    head.push(jira::hours(today), if today > 0 { theme.accent().add_modifier(Modifier::BOLD) } else { theme.muted() });
    let mut lines = vec![head.line(width, Style::default())];

    // A column a day, as wide as the width allows; the bar a cell narrower, for the gap.
    const AXIS: usize = 6;
    let column = (width.saturating_sub(AXIS + 2) / days.len().max(1)).clamp(2, 5);
    let bar = column.saturating_sub(1).max(1);
    let day_s: u64 = 8 * 3600;
    // Whole hours at the top, so its label fits the axis: `8h`, `11h`.
    let top = days.iter().copied().max().unwrap_or(0).max(day_s).div_ceil(3600) * 3600;
    let eighths = |seconds: u64| ((seconds as f64 / top as f64) * (bars * 8) as f64).round() as usize;
    let future = |day: u32| day > month.today;

    // The hours over each day: `7½`, `8`, `·` for a working day with nothing logged.
    let mut values = Cells::new();
    values.push(" ".repeat(AXIS), Style::default());
    for (i, seconds) in days.iter().enumerate() {
        let day = i as u32 + 1;
        let text = match *seconds {
            0 if future(day) || month.is_weekend(day) => String::new(),
            0 => "·".to_string(),
            s => half_hours(s),
        };
        let style = if day == month.today { theme.accent().add_modifier(Modifier::BOLD) } else { theme.text2() };
        values.cell_right(&text, bar, style);
        values.gap(column - bar);
    }
    lines.push(values.line(width, Style::default()));

    const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    for row in 0..bars {
        let mut cells = Cells::new();
        let label = if row == 0 { format!("{:>4} ", format!("{}h", top / 3600)) } else { " ".repeat(AXIS - 1) };
        cells.push(label, theme.faint());
        cells.push("│", theme.rule());
        for (i, seconds) in days.iter().enumerate() {
            let day = i as u32 + 1;
            let level = eighths(*seconds).saturating_sub((bars - 1 - row) * 8).min(8);
            let fill = if day == month.today { theme.accent() } else { Style::default().fg(theme.bar_fill(Severity::None)) };
            cells.push(BLOCKS[level].repeat(bar), fill);
            cells.gap(column - bar);
        }
        lines.push(cells.line(width, Style::default()));
    }

    let mut labels = Cells::new();
    labels.push(" ".repeat(AXIS), Style::default());
    for (i, seconds) in days.iter().enumerate() {
        let day = i as u32 + 1;
        let style = if day == month.today {
            theme.accent().add_modifier(Modifier::BOLD)
        } else if month.is_weekend(day) || future(day) {
            theme.faint()
        } else if *seconds == 0 {
            theme.muted()
        } else {
            theme.text2()
        };
        labels.cell_right(&day.to_string(), bar, style);
        labels.gap(column - bar);
    }
    lines.push(labels.line(width, Style::default()));
    lines
}

/// Hours to the half hour, in two cells: `7½`, `8`, `½`, `12`.
fn half_hours(seconds: u64) -> String {
    // Ten hours and more have no room for the half: whole hours.
    if seconds >= 10 * 3600 {
        return ((seconds + 1800) / 3600).to_string();
    }
    let halves = (seconds + 900) / 1800;
    match (halves / 2, halves % 2) {
        (0, 0) => "·".to_string(),
        (0, _) => "½".to_string(),
        (h, 0) => h.to_string(),
        (h, _) => format!("{h}½"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cells_after_the_summary_give_way_on_a_narrow_terminal() {
        let board = crate::fake::jira(1_791_103_927);
        let today = jira::today(1_791_103_927, 7 * 3600);
        let wide = Widths::of(&board, today, 156);
        assert!(wide.due > 0 && wide.logged > 0, "{wide:?}");
        let narrow = Widths::of(&board, today, 76);
        assert_eq!((narrow.due, narrow.logged), (0, 0), "{narrow:?}");
        assert!(narrow.summary >= SUMMARY_MIN);
    }

    #[test]
    fn hours_go_to_the_half_hour_in_two_cells() {
        assert_eq!(half_hours(7 * 3600 + 1800), "7½");
        assert_eq!(half_hours(8 * 3600), "8");
        assert_eq!(half_hours(1700), "½");
        assert_eq!(half_hours(600), "·");
        assert_eq!(half_hours(12 * 3600 + 1000), "12");
    }
}
