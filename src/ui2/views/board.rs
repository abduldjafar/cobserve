//! JIRA zoomed (`docs/redesign/09-zooms.md` §3): *what is on my board, and what is late?* The
//! zoom is the board at every width — the columns as lanes side by side, each ticket a block of
//! its own: the key and its priority, the summary in two lines, how long it has been in the
//! column, its due date and the time logged. A ticket that is late or nearly so carries a stripe
//! in its severity down its left edge; the cursor's block a bar of the accent. The month's time
//! sits above the lanes; the ticket under the cursor in full in a card under them.
//!
//! ```text
//!  IN PROGRESS 4 ▲ 1 due soon   IN REVIEW 4 ✖ 1 overdue      FEEDBACK 1      DONE 8 in 7 days
//!  ━━━━━━━━━━━━━━━━━━━━━━━━━━━  ───────────────────────────  ──────────────  ────────────────
//!  ▌DATA-2207             High  ▎DATA-2647             ASAP   DATA-1289  High DATA-2638  ✔ 2h
//!  ▌Product metrics             ▎Client account balances      Daily per-cove… Enrich accounti…
//!  ▌integration — PostHog + C…  ▎as a daily warehouse tabl…   statement vs …  statement_misma…
//!  ▌11d in column               ▎4d · overdue 2d · 7h12m      12d · 4h logged 2h logged
//! ```
//!
//! `↑ ↓` walk a lane and on into the next, `← →` cross the lanes.

use super::kit::{self, Card};
use super::widgets::Cells;
use crate::app::{scroll_into_view, App, Feed, Hit};
use crate::fmt;
use crate::jira::{self, Ticket};
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use crate::ui2::paint::Painter;
use ratatui::text::Span;
use ratatui::Frame;


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
pub(super) fn wrap(text: &str, width: usize, max: usize) -> Vec<String> {
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

/// A ticket's block, `width` cells wide: the key and its priority (or, finished, when), the
/// summary in `summary_lines`, then how long in the column, the due date and the time logged.
#[allow(clippy::too_many_arguments)]
pub(super) fn block(ticket: &Ticket, finished: bool, selected: bool, summary_lines: usize, now: i64, today: i64, theme: &Theme, width: usize) -> Vec<Cells> {
    let due = if finished { Severity::None } else { ticket.due_day().map_or(Severity::None, |d| jira::due_severity(d, today, false)) };
    let edge = |cells: &mut Cells| {
        match (selected, due) {
            (true, _) => cells.push("▌", theme.accent()),
            (false, Severity::Crit | Severity::Warn) => cells.push("▎", theme.sev(due)),
            _ => cells.push(" ", Style::default()),
        };
    };
    let inner = width.saturating_sub(1);

    // The key, and on the right its priority — or, finished, when.
    let mut first = Cells::new();
    edge(&mut first);
    first.push(ticket.key.clone(), if finished { theme.muted() } else { theme.accent().add_modifier(Modifier::BOLD) });
    let mut right = Cells::new();
    match (finished, ticket.resolved) {
        (true, Some(at)) => {
            right.push("✔ ", theme.sev(Severity::Ok));
            right.push(jira::age(now - at), theme.muted());
        }
        _ if ticket.priority_unset() => {
            right.push("—", theme.faint());
        }
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

    // The summary, the team's tag left off: every ticket of the board has it.
    let words = jira::split_tag(&ticket.summary).1;
    let text_style = if finished { theme.text2() } else if selected { theme.strong() } else { theme.text() };
    let mut lines = vec![first];
    let mut summary = wrap(words, inner, summary_lines);
    summary.resize(summary_lines, String::new());
    for text in summary {
        let mut cells = Cells::new();
        edge(&mut cells);
        cells.push(text, text_style);
        lines.push(cells);
    }

    // How long in the column, the due date, the time logged: the words go first when cut.
    let mut pieces: Vec<(String, String, Style)> = Vec::new();
    if !finished {
        if let Some(age) = ticket.in_status(now) {
            pieces.push((jira::age(age), " in column".into(), theme.text2()));
        }
        if let Some(day) = ticket.due_day() {
            let style = if due.is_problem() { theme.sev(due).add_modifier(Modifier::BOLD) } else { theme.muted() };
            pieces.push((jira::due_label(day, today), String::new(), style));
        }
    }
    if let Some(logged) = ticket.logged_s.filter(|s| *s > 0) {
        pieces.push((jira::logged(logged), " logged".into(), theme.muted()));
    }
    let alone = pieces.len() == 1;
    let full = |words: bool| -> usize {
        pieces.iter().map(|(t, w, _)| fmt::width(t) + if words && (alone || !w.contains("column")) { fmt::width(w) } else { 0 }).sum::<usize>() + 3 * pieces.len().saturating_sub(1)
    };
    let words = full(true) <= inner;
    let mut last = Cells::new();
    edge(&mut last);
    for (i, (text, word, style)) in pieces.into_iter().enumerate() {
        if i > 0 {
            last.push(" · ", theme.faint());
        }
        last.push(text, style);
        if words && (alone || !word.contains("column")) {
            last.push(word, theme.faint());
        }
    }
    lines.push(last);
    lines
}

/// A lane's title: `IN REVIEW 4 ✖ 1 overdue`, `DONE 8 in 7 days`.
fn lane_header(lane: &Lane<'_>, board: &jira::Board, today: i64, theme: &Theme, width: usize) -> Cells {
    let mut title = Cells::new();
    title.push(lane.title.to_uppercase(), if lane.tickets.is_empty() { theme.muted() } else { theme.section() });
    title.push(format!(" {}", lane.tickets.len()), if lane.tickets.is_empty() { theme.faint() } else { theme.strong() });
    if lane.finished && board.done_days > 0 {
        title.push(format!(" in {}", fmt::plural(board.done_days as usize, "day", "days")), theme.faint());
    }
    if !lane.finished {
        let judged: Vec<Severity> = lane.tickets.iter().filter_map(|t| t.due_day()).map(|d| jira::due_severity(d, today, false)).collect();
        let overdue = judged.iter().filter(|s| **s == Severity::Crit).count();
        let soon = judged.iter().filter(|s| **s == Severity::Warn).count();
        // The words when they fit the lane, the marks and counts alone when not.
        let words = title.width() + if overdue > 0 { 12 } else { 0 } + if soon > 0 { 13 } else { 0 } <= width;
        if overdue > 0 {
            title.push(if words { format!(" ✖ {overdue} overdue") } else { format!(" ✖{overdue}") }, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
        if soon > 0 {
            title.push(if words { format!(" ▲ {soon} due soon") } else { format!(" ▲{soon}") }, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
    }
    title
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, i: Rect) {
    if i.height == 0 || i.width < 10 {
        return;
    }
    let mut p = Painter::new(frame.buffer_mut());
    let clip = Rect::new(i.x.saturating_sub(1), i.y, i.width + 2, i.height);
    p.within(clip, |p| body(p, app, theme, i));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let board = &app.jira;
    if !board.reachable {
        let calm = ("◌", theme.muted());
        match board.error.as_deref() {
            Some(jira::NOT_CONFIGURED) => kit::message(
                p,
                theme,
                i,
                calm,
                "Jira is not configured",
                &["JIRA_URL and JIRA_TOKEN (a personal access token, from your Jira profile) turn it on", ":setup jira · or jira: url, token in the --credential file"],
            ),
            Some(jira::NOT_READ) | None => kit::message(p, theme, i, calm, "reading Jira…", &["your tickets and this month's worklogs"]),
            Some(why) => kit::message(p, theme, i, ("✖", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)), "Jira could not be read", &[why, "r reads again"]),
        }
        return;
    }
    // The lanes are always lanes: `← →` cross them at every width.
    app.viewport.jira_board.set(true);
    let now = app.now();
    let offset_s = app.time.offset_s(now);
    let today = jira::today(now, offset_s);
    let w = i.width as usize;
    let fresh = kit::freshness(board.age(app.clock), board.error.as_deref(), app.is_reading(Feed::Jira), theme, w / 2);

    // The month's time: the chart when there is height for it, a line otherwise.
    let mut y = i.y;
    if board.worklogs_read && i.height >= 26 {
        let bars = if i.height >= 40 { 3 } else { 2 };
        for line in super::jira::month_chart(board, now, offset_s, bars, theme, w) {
            kit::put_line(p, i.x.saturating_sub(2), y, line, w + 2);
            y += 1;
        }
        kit::put_spans_r(p, i.right(), i.y, fresh.into_spans());
        y += 1;
    } else {
        let mut cells = Cells::new();
        if board.worklogs_read {
            let month = jira::Month::of(now, offset_s);
            let days = month.per_day(&board.worklogs);
            let total: u64 = days.iter().sum();
            let today_s = days.get(month.today as usize - 1).copied().unwrap_or(0);
            cells.push("LOGGED", theme.section());
            cells.push(format!(" · {} · ", month.name.split(' ').next().unwrap_or_default()), theme.muted());
            cells.push(jira::hours(total), theme.strong());
            cells.push(format!(" in {}", fmt::plural(month.working_days_so_far().max(1) as usize, "working day", "working days")), theme.muted());
            cells.push(" · today ", theme.faint());
            cells.push(jira::hours(today_s), if today_s > 0 { theme.accent().add_modifier(Modifier::BOLD) } else { theme.muted() });
            cells.push("   t by ticket", theme.faint());
        } else {
            cells.push("reading this month's worklogs…", theme.accent());
        }
        let room = w.saturating_sub(fresh.width() + 2);
        kit::put_cells(p, i.x, y, cells, room);
        kit::put_spans_r(p, i.right(), y, fresh.into_spans());
        y += 1;
    }

    let card_h: u16 = if i.height >= 30 { 5 } else { 0 };
    let lanes_r = Rect::new(i.x, y, i.width, i.bottom().saturating_sub(card_h).saturating_sub(y));
    lanes_view(p, app, theme, lanes_r, now, today);
    if card_h > 0 {
        detail(p, app, theme, Rect::new(i.x, i.bottom() - card_h, i.width, card_h));
    }
}

fn lanes_view(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, now: i64, today: i64) {
    if r.height < 3 {
        return;
    }
    let board = &app.jira;
    let lanes = lanes(board);
    let count = lanes.len().max(1);
    let width = r.width as usize;
    let gap = if width >= 100 { 2 } else { 1 };
    let lane_w = width.saturating_sub(gap * (count - 1)) / count;
    let summary_lines = if width >= 100 && r.height >= 14 { 2 } else { 1 };
    let block_h = summary_lines + 2;
    let selected = app.jira_selection();
    let holding = lanes.iter().position(|l| l.tickets.iter().any(|t| Some(t.key.as_str()) == selected));
    let lane_x = |l: usize| r.x + (l * (lane_w + gap)) as u16;

    // The lanes' titles, and a thread under each — the accent under the lane holding the cursor.
    for (l, lane) in lanes.iter().enumerate() {
        kit::put_cells(p, lane_x(l), r.y, lane_header(lane, board, today, theme, lane_w), lane_w);
        let (glyph, style) = if holding == Some(l) { ("━", theme.accent()) } else { ("─", theme.rule()) };
        p.put(lane_x(l), r.y + 1, &glyph.repeat(lane_w.saturating_sub(1)), style);
    }

    // Each lane a column of blocks, all lanes on one vertical offset.
    let mut index = 0usize;
    let mut columns: Vec<Vec<Cells>> = Vec::new();
    let mut spots: Vec<(usize, usize, usize, bool)> = Vec::new(); // lane, first row, ticket index, chosen
    let mut selected_row = None;
    for (l, lane) in lanes.iter().enumerate() {
        let mut rows: Vec<Cells> = Vec::new();
        for ticket in &lane.tickets {
            let chosen = Some(ticket.key.as_str()) == selected;
            if chosen {
                selected_row = Some(rows.len());
            }
            spots.push((l, rows.len(), index, chosen));
            index += 1;
            rows.extend(block(ticket, lane.finished, chosen, summary_lines, now, today, theme, lane_w));
            rows.push(Cells::new());
        }
        if lane.tickets.is_empty() {
            let mut none = Cells::new();
            none.push(" nothing here", theme.faint());
            rows.push(none);
        }
        columns.push(rows);
    }
    let tall = columns.iter().map(Vec::len).max().unwrap_or(0);
    let room = r.height.saturating_sub(2) as usize;
    // The block under the cursor whole on screen: its first row and its last.
    let offset = scroll_into_view(app.viewport.jira.get(), selected_row.map(|row| row + block_h - 1), room, tall);
    let offset = match selected_row {
        Some(row) if row < offset => row,
        _ => offset,
    };
    app.viewport.jira.set(offset);
    let top = r.y + 2;
    for &(l, first, _, chosen) in &spots {
        if !chosen {
            continue;
        }
        for k in first..first + block_h {
            if k >= offset && k < offset + room {
                p.band(Rect::new(lane_x(l), top + (k - offset) as u16, lane_w as u16, 1), theme.selected());
            }
        }
    }
    for (l, column) in columns.into_iter().enumerate() {
        for (k, cells) in column.into_iter().enumerate().skip(offset).take(room) {
            kit::put_cells(p, lane_x(l), top + (k - offset) as u16, cells, lane_w);
        }
    }
    let mut hits = app.viewport.hits.borrow_mut();
    for (lane, first, ticket, _) in spots {
        let (a, b) = (first.max(offset), (first + block_h).min(offset + room));
        if a < b {
            hits.push((Rect::new(lane_x(lane), top + (a - offset) as u16, lane_w as u16, (b - a) as u16), Hit::Ticket(ticket)));
        }
    }
}

/// The ticket under the cursor in full: its summary with the tag, who and when, its link.
fn detail(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let (head, lines) = super::drawer::jira(app, theme, r.width.saturating_sub(4) as usize);
    let right = vec![Span::styled("⏎ in full · o in Jira · y copy link · r read now", theme.muted())];
    let inner = Card::new(theme.border()).title(head).right(right).draw(p, r);
    for (k, mut line) in lines.into_iter().enumerate().take(inner.height as usize) {
        // The link alone: the keys are in the card's title.
        if k == 2 && line.spans.len() > 1 {
            line.spans.truncate(1);
        }
        kit::put_line(p, inner.x, inner.y + k as u16, line, inner.width as usize);
    }
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
