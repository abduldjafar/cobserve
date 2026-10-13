//! View 6 on a wide terminal: the tickets as a board — the columns side by side as lanes, each
//! ticket a block of its own rather than a row of a table.
//!
//! ```text
//!  in progress 4 ✖1            in review 4                 feedback 1        done 8 in 7 days
//!  ━━━━━━━━━━━━━━━━━━━━━━━━━   ───────────────────────     ──────────────    ────────────────
//! ▌DATA-2207           High    DATA-2647           ASAP    DATA-1289  High   DATA-2638   ✔ 2h
//! ▌Product metrics integration Client account balances     Daily per-cove…   Enrich accounting…
//! ▌— PostHog + ClickHouse →…   as a daily warehouse ta…    statement vs …    statement_misma…
//! ▌11d                         4d · overdue 2d · 7h12m     12d · 4h          2h
//! ```
//!
//! A lane's title says its count and, for open work, how much of it is overdue; the lane holding
//! the cursor is underlined in tarum. A block: the key and its priority, the summary in two
//! lines, and under them how long it has been in the column, its due date (coloured by how near)
//! and the time logged — a finished one when it was finished. `↑ ↓` walk a lane and on into the
//! next, `← →` cross the lanes. The month's time sits above the board, as on the narrow view.

use super::widgets::{fit, Cells};
use crate::app::{scroll_into_view, App, Hit};
use crate::fmt;
use crate::jira::{self, Ticket};
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The narrowest terminal that gets the board; below it, the list (`jira.rs`).
pub const FROM_WIDTH: usize = 140;
const GAP: usize = 3;
/// A block's lines, and the blank under it.
const BLOCK: usize = 4;

/// A lane: its title, whether it holds finished work, and its tickets in the order drawn.
pub struct Lane<'a> {
    pub title: String,
    pub finished: bool,
    pub tickets: Vec<&'a Ticket>,
}

/// The board's lanes: its columns, and tickets that moved on since the query ran in one more.
pub fn lanes(board: &jira::Board) -> Vec<Lane<'_>> {
    let mut lanes: Vec<Lane<'_>> = board
        .columns()
        .into_iter()
        .map(|c| Lane { title: c.status.to_lowercase(), finished: c.finished, tickets: c.tickets })
        .collect();
    let others = board.others();
    if !others.is_empty() {
        lanes.push(Lane { title: "moved on".into(), finished: false, tickets: others });
    }
    lanes
}

/// Words to `width` cells, in at most `max` lines, the last cut with an ellipsis if need be.
fn wrap(text: &str, width: usize, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word = if fmt::width(word) > width { fmt::truncate(word, width) } else { word.to_string() };
        if line.is_empty() {
            line = word;
        } else if fmt::width(&line) + 1 + fmt::width(&word) <= width {
            line = format!("{line} {word}");
        } else {
            lines.push(std::mem::replace(&mut line, word));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max && max > 0 {
        // The last line takes what follows it too, and is cut: the ellipsis says there is more.
        let last = format!("{} {}", lines[max - 1], lines[max]);
        lines.truncate(max);
        lines[max - 1] = fmt::truncate(&last, width);
    }
    lines
}

/// One piece of a board line: `cells` in `width` cells, on the selection's band when chosen.
fn segment(cells: Cells, width: usize, band: Option<Style>) -> Vec<Span<'static>> {
    let spans = fit(cells.into_spans(), width, true);
    match band {
        Some(band) => spans.into_iter().map(|s| Span::styled(s.content, band.patch(s.style))).collect(),
        None => spans,
    }
}

/// A ticket's block: its four lines, each `width` cells.
fn block(ticket: &Ticket, finished: bool, selected: bool, now: i64, today: i64, theme: &Theme, width: usize) -> Vec<Vec<Span<'static>>> {
    let band = selected.then(|| theme.selected());
    let inner = width.saturating_sub(2);
    let bar = |cells: &mut Cells| {
        cells.push(if selected { "▌" } else { " " }, theme.accent());
        cells.push(" ", Style::default());
    };

    // The key, and on the right its priority — or, finished, when.
    let mut first = Cells::new();
    bar(&mut first);
    first.push(ticket.key.clone(), if finished { theme.muted() } else { theme.accent().add_modifier(Modifier::BOLD) });
    let mut right = Cells::new();
    match (finished, ticket.resolved) {
        (true, Some(at)) => {
            right.push("✔ ", theme.sev(Severity::Ok));
            right.push(jira::age(now - at), theme.muted());
        }
        _ if ticket.priority_unset() => {}
        _ => {
            let word = ticket.priority.clone().unwrap_or_default();
            let style = match ticket.priority_severity() {
                _ if finished => theme.muted(),
                Severity::Crit => theme.sev(Severity::Crit).add_modifier(Modifier::BOLD),
                _ if word.eq_ignore_ascii_case("high") => theme.strong(),
                _ => theme.text2(),
            };
            right.push(word, style);
        }
    }
    if first.width() + 1 + right.width() <= width {
        first.pad_to(width - right.width());
        first.spans(right.into_spans());
    }

    // The summary in two lines, the team's tag left off: every ticket of the board has it.
    let words = jira::split_tag(&ticket.summary).1;
    let text_style = if finished { theme.text2() } else if selected { theme.strong() } else { theme.text() };
    let mut summary: Vec<Cells> = wrap(words, inner, 2)
        .into_iter()
        .map(|text| {
            let mut cells = Cells::new();
            bar(&mut cells);
            cells.push(text, text_style);
            cells
        })
        .collect();
    while summary.len() < 2 {
        let mut cells = Cells::new();
        bar(&mut cells);
        summary.push(cells);
    }

    // How long in the column, the due date, the time logged.
    let mut last = Cells::new();
    bar(&mut last);
    let mut pieces: Vec<(String, Style)> = Vec::new();
    if !finished {
        if let Some(age) = ticket.in_status(now) {
            pieces.push((jira::age(age), theme.text2()));
        }
        if let Some(day) = ticket.due_day() {
            let sev = jira::due_severity(day, today, false);
            let style = if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.muted() };
            pieces.push((jira::due_label(day, today), style));
        }
    }
    if let Some(logged) = ticket.logged_s.filter(|s| *s > 0) {
        pieces.push((format!("{} logged", jira::logged(logged)), theme.muted()));
    }
    for (i, (text, style)) in pieces.into_iter().enumerate() {
        if i > 0 {
            last.push(" · ", theme.faint());
        }
        last.push(text, style);
    }

    let mut lines = vec![segment(first, width, band)];
    lines.extend(summary.into_iter().map(|c| segment(c, width, band)));
    lines.push(segment(last, width, band));
    lines
}

/// The board, under `head` (the title and the month's time), in `area`.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, head: Vec<Line<'static>>) {
    let board = &app.jira;
    let width = area.width as usize;
    let height = area.height as usize;
    let now = app.now();
    let today = jira::today(now, app.time.offset_s(now));
    let lanes = lanes(board);
    let count = lanes.len().max(1);
    let lane_w = width.saturating_sub(GAP * (count - 1)) / count;
    let selected = app.jira_selection();
    let holding = lanes.iter().position(|l| l.tickets.iter().any(|t| Some(t.key.as_str()) == selected));

    // The lanes' titles, and a thread under each — tarum under the lane holding the cursor.
    let mut titles: Vec<Span<'static>> = Vec::new();
    let mut threads: Vec<Span<'static>> = Vec::new();
    for (i, lane) in lanes.iter().enumerate() {
        if i > 0 {
            titles.push(Span::raw(" ".repeat(GAP)));
            threads.push(Span::raw(" ".repeat(GAP)));
        }
        let mut title = Cells::new();
        title.push(" ", Style::default());
        title.push(lane.title.clone(), if lane.tickets.is_empty() { theme.muted() } else { theme.section() });
        title.push(format!(" {}", lane.tickets.len()), theme.faint());
        if lane.finished && board.done_days > 0 {
            title.push(format!(" in {}", fmt::plural(board.done_days as usize, "day", "days")), theme.faint());
        }
        if !lane.finished {
            let judged: Vec<Severity> = lane.tickets.iter().filter_map(|t| t.due_day()).map(|d| jira::due_severity(d, today, false)).collect();
            let overdue = judged.iter().filter(|s| **s == Severity::Crit).count();
            let soon = judged.iter().filter(|s| **s == Severity::Warn).count();
            if overdue > 0 {
                title.push(format!("  ✖ {overdue} overdue"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
            } else if soon > 0 {
                title.push(format!("  ▲ {soon} due soon"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            }
        }
        titles.extend(segment(title, lane_w, None));
        let (glyph, style) = if holding == Some(i) { ("━", theme.accent()) } else { ("─", theme.rule()) };
        threads.push(Span::styled(format!(" {}", glyph.repeat(lane_w.saturating_sub(2))), style));
        threads.push(Span::raw(" "));
    }

    // Each lane as a column of blocks; the board's rows, lane beside lane.
    let mut index = 0usize;
    let mut columns: Vec<Vec<Vec<Span<'static>>>> = Vec::new();
    let mut spots: Vec<(usize, usize, usize)> = Vec::new(); // (lane, first row, ticket index)
    let mut selected_row = None;
    for (l, lane) in lanes.iter().enumerate() {
        let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
        for ticket in &lane.tickets {
            let chosen = Some(ticket.key.as_str()) == selected;
            if chosen {
                selected_row = Some(rows.len());
            }
            spots.push((l, rows.len(), index));
            index += 1;
            rows.extend(block(ticket, lane.finished, chosen, now, today, theme, lane_w));
            rows.push(vec![Span::raw(" ".repeat(lane_w))]);
        }
        if lane.tickets.is_empty() {
            let mut none = Cells::new();
            none.push("  nothing here", theme.faint());
            rows.push(segment(none, lane_w, None));
        }
        columns.push(rows);
    }
    let tall = columns.iter().map(Vec::len).max().unwrap_or(0);

    let mut lines = head;
    lines.push(Line::from(titles));
    lines.push(Line::from(threads));
    let fixed = lines.len().min(height);
    let room = height - fixed;
    // The block under the cursor whole on screen: its first row and its last.
    let offset = scroll_into_view(app.viewport.jira.get(), selected_row.map(|r| r + BLOCK - 1), room, tall);
    let offset = match selected_row {
        Some(r) if r < offset => r,
        _ => offset,
    };
    app.viewport.jira.set(offset);
    for row in offset..(offset + room).min(tall) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (l, column) in columns.iter().enumerate() {
            if l > 0 {
                spans.push(Span::raw(" ".repeat(GAP)));
            }
            match column.get(row) {
                Some(segment) => spans.extend(segment.iter().cloned()),
                None => spans.push(Span::raw(" ".repeat(lane_w))),
            }
        }
        lines.push(Line::from(spans));
    }

    let mut hits = app.viewport.hits.borrow_mut();
    for (lane, first, ticket) in spots {
        let (top, bottom) = (first.max(offset), (first + BLOCK).min(offset + room));
        if top < bottom {
            let x = area.x + (lane * (lane_w + GAP)) as u16;
            let y = area.y + (fixed + top - offset) as u16;
            hits.push((Rect::new(x, y, lane_w as u16, (bottom - top) as u16), Hit::Ticket(ticket)));
        }
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn words_wrap_in_two_lines_and_the_rest_is_cut() {
        assert_eq!(wrap("Product metrics integration — PostHog + ClickHouse → portal (PoC)", 24, 2), ["Product metrics", "integration — PostHog +…"]);
        assert_eq!(wrap("short", 24, 2), ["short"]);
        assert_eq!(wrap("a_very_long_identifier_without_spaces and more", 10, 2), ["a_very_lo…", "and more"]);
    }
}
