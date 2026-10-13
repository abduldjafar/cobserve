//! A query session on view 5 (`console.rs`): the server it runs on and the helper that writes
//! for it, the SQL on a raised field — the suggestions for the word at the cursor under it —
//! how the last run went, and its answer as a table.
//!
//! ```text
//! ▦ Query  on  ● clickhouse3 ▾    ✻ Claude ▾                  read-only · 30 s · 1000 rows · 9 tables
//!
//!   1  -- the ten users using the most memory right now
//!   2  SELECT user, count() AS queries FROM system.proc
//!      ⏎ after ; runs · ctrl+r runs · tab com ▌▦ system.processes   SystemProcesses
//!                                              ▦ system.part_log    SystemPartLog
//! ✔ 4 rows · 12 ms · read 1.2k rows, 3.4 KiB · on clickhouse3
//!
//!   user        queries
//!   ──────────  ───────
//!   r_redash          2
//! ```

use super::widgets::{fit, Cells};
use crate::app::{App, Hit};
use crate::complete::What;
use crate::console::{Answer, Assisting, Console, Focus, RunState, Suggest, ROW_LIMIT, TIME_LIMIT_S};
use crate::fmt;
use crate::severity::Severity;
use crate::sqltext::{highlight, Token};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// The widest a column of the answer gets; a value longer than that is cut with `…`.
const COLUMN_MAX: usize = 40;
/// The suggestions shown at once.
const SUGGESTIONS_SHOWN: usize = 8;
/// The line numbers' gutter.
const GUTTER: usize = 6;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, console: &Console) {
    if area.height < 6 || area.width < 24 {
        return;
    }
    let hit = |rect: Rect, what: Hit| app.viewport.hits.borrow_mut().push((rect, what));
    let width = area.width as usize;
    let bottom = area.y + area.height;
    let mut y = area.y;

    // What this is: the server it runs on — a click lists the others — the helper that writes
    // for it — a click changes it — and its limits.
    let servers = app.servers();
    let mut head = Cells::new();
    head.push("▦ ", theme.accent().add_modifier(Modifier::BOLD));
    head.push("Query", theme.strong());
    head.push("  on  ", theme.muted());
    let reachable = console.node.as_ref().and_then(|n| servers.iter().find(|(name, _)| name == n)).map(|(_, ok)| *ok);
    let chip_at = head.width();
    let chip = match (&console.node, reachable) {
        (Some(node), Some(false)) => format!(" ↯ {node} ▾ "),
        (Some(node), _) => format!(" ● {node} ▾ "),
        (None, _) => " choose a server ▾ ".to_string(),
    };
    let chip_style = match reachable {
        Some(false) => theme.raised().patch(theme.sev(Severity::Crit)),
        _ if console.node.is_none() => theme.raised().patch(theme.sev(Severity::Warn)),
        _ => theme.raised().patch(theme.strong()),
    };
    head.push(chip.clone(), chip_style);
    hit(Rect::new(area.x + chip_at as u16, y, fmt::width(&chip) as u16, 1), Hit::ConsoleServer);
    head.push("   ", Style::default());
    let helper_at = head.width();
    let helper = format!(" {} {} ⇄ ", console.assistant.glyph(), console.assistant.name());
    head.push(helper.clone(), theme.raised().patch(theme.claude()));
    hit(Rect::new(area.x + helper_at as u16, y, fmt::width(&helper) as u16, 1), Hit::ConsoleAssistant);
    let mut limits = format!("read-only · {TIME_LIMIT_S} s · {ROW_LIMIT} rows");
    match console.node.as_deref().and_then(|n| app.schemas.get(n)) {
        Some(state) if state.reading && state.schema.is_none() => limits.push_str(" · reading its tables…"),
        Some(state) if state.schema.is_some() => {
            let tables = state.schema.as_ref().map_or(0, |s| s.tables.iter().filter(|t| t.database != "system").count());
            limits.push_str(&format!(" · {}", fmt::plural(tables, "table", "tables")));
        }
        Some(state) if state.error.is_some() => limits.push_str(" · no list of its tables"),
        _ => {}
    }
    if head.width() + fmt::width(&limits) + 2 <= width {
        head.pad_to(width - fmt::width(&limits));
        head.push(limits, theme.faint());
    }
    frame.render_widget(Paragraph::new(head.line(width, Style::default())), Rect::new(area.x, y, area.width, 1));
    y += 2;

    // The SQL, on a raised field that grows with it up to a third of the pane.
    let rows = console.lines.len().clamp(3, (area.height as usize / 3).max(3)) as u16;
    let field = Rect::new(area.x, y, area.width, rows.min(bottom.saturating_sub(y)));
    frame.render_widget(Block::new().style(theme.raised()), field);
    hit(field, Hit::ConsoleText);
    // The text scrolls only as far as the cursor needs.
    let height = field.height as usize;
    let mut first = console.top.get().min(console.row);
    if console.row >= first + height {
        first = console.row + 1 - height;
    }
    let first = first.min(console.lines.len().saturating_sub(height));
    console.top.set(first);
    let text_width = width.saturating_sub(GUTTER + 1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cursor = None;
    let mut shift_on_row = 0;
    let selection = console.selection();
    for (n, text) in console.lines.iter().enumerate().skip(first).take(field.height as usize) {
        let mut cells = Cells::new();
        cells.cell_right(&(n + 1).to_string(), 4, theme.faint());
        cells.push("  ", Style::default());
        // The cursor's line slides along when it is longer than the field; the others are cut.
        let shift = if n == console.row { console.col.saturating_sub(text_width.saturating_sub(1)) } else { 0 };
        if n == console.row {
            shift_on_row = shift;
            cursor = Some((field.x + GUTTER as u16 + (console.col - shift) as u16, field.y + (n - first) as u16));
        }
        let shown: String = text.chars().skip(shift).collect();
        let mut spans = coloured(&shown, theme);
        // What is selected on this line — its end too, when the selection goes on past it.
        if let Some(((r0, c0), (r1, c1))) = selection
            && (r0..=r1).contains(&n)
        {
            let from = if n == r0 { c0 } else { 0 };
            let to = if n == r1 { c1 } else { text.chars().count() + 1 };
            spans = marked(spans, from.saturating_sub(shift), to.saturating_sub(shift), theme.marked());
        }
        // What the helper just wrote in place of a part, under its colour until it is changed.
        if let Assisting::Wrote { part: Some(((r0, c0), (r1, c1))) } = console.assisting
            && (r0..=r1).contains(&n)
        {
            let from = if n == r0 { c0 } else { 0 };
            let to = if n == r1 { c1 } else { text.chars().count() };
            spans = marked(spans, from.saturating_sub(shift), to.saturating_sub(shift), theme.written());
        }
        cells.spans(fit(spans, text_width, false));
        lines.push(on_raised(cells.line(width, Style::default()), theme));
    }
    app.viewport.console_text.set(Some(crate::app::TextLayout {
        text_x: field.x + GUTTER as u16,
        top: field.y,
        height: field.height,
        first,
        cursor_row: console.row,
        shift: shift_on_row,
    }));
    if console.lines.len() == 1 && console.lines[0].is_empty() {
        // What to do, where there is nothing yet.
        let hints = [
            format!("ctrl+k: tell {} what it should show — it writes the SQL", console.assistant.name()),
            "SELECT … FROM system.processes;    ⏎ after its ; runs it".to_string(),
        ];
        lines.clear();
        for (i, hint) in hints.iter().enumerate().take(field.height as usize) {
            let mut cells = Cells::new();
            cells.cell_right(if i == 0 { "1" } else { "" }, 4, theme.faint());
            cells.push("  ", Style::default());
            cells.push(fmt::truncate(hint, text_width), theme.faint().add_modifier(Modifier::ITALIC));
            lines.push(on_raised(cells.line(width, Style::default()), theme));
        }
    }
    frame.render_widget(Paragraph::new(lines), field);
    if console.focus == Focus::Editor
        && console.choosing.is_none()
        && console.instruction.is_none()
        && let Some((x, cy)) = cursor
        && x < area.x + area.width
    {
        frame.set_cursor_position((x, cy));
    }
    y += field.height;

    // Under the field: the line to tell the helper what to do, its turn, or how to go on.
    if y < bottom {
        if let Some(text) = &console.instruction {
            let (line, cursor_at) = instruction_line(theme, console, text, width);
            frame.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
            frame.set_cursor_position((area.x + (cursor_at as u16).min(area.width.saturating_sub(1)), y));
        } else {
            let (line, asks) = helper_line(app, theme, console, width);
            if let Some((at, cells)) = asks {
                hit(Rect::new(area.x + at as u16, y, cells as u16, 1), Hit::ConsoleAsk);
            }
            frame.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
        }
        y += 2;
    }

    // How the last run went.
    if y < bottom {
        let status_height = status(frame, app, theme, Rect::new(area.x, y, area.width, bottom - y), console);
        y += status_height + 1;
    }

    // The answer.
    if y < bottom
        && let Some(answer) = &console.answer
    {
        let table = Rect::new(area.x, y, area.width, bottom - y);
        hit(table, Hit::ConsoleAnswer);
        answer_table(frame, theme, table, answer, console);
    }

    // Over everything: the suggestions under the cursor, the servers under their chip.
    if let (Some(open), Some((_, cy))) = (&console.suggest, cursor)
        && console.focus == Focus::Editor
        && open.row == console.row
    {
        // The text of each lines up under the word, after the bar and the mark.
        let word_x = field.x + (GUTTER + open.from.saturating_sub(shift_on_row)) as u16;
        suggestions(frame, app, theme, area, open, word_x.saturating_sub(3), cy);
    }
    if let Some(at) = console.choosing {
        servers_list(frame, app, theme, Rect::new(area.x + chip_at as u16, area.y + 1, 0, 0), &servers, at, area);
    }
}

/// A line of the field, its spans on the raised surface.
fn on_raised(line: Line<'static>, theme: &Theme) -> Line<'static> {
    Line::from(line.spans.into_iter().map(|span| Span::styled(span.content, theme.raised().patch(span.style))).collect::<Vec<_>>())
}

/// `spans` with the characters `from..to` in `style` — and, when `to` is past their end, a cell
/// more, for the line's end that is selected too.
fn marked(spans: Vec<Span<'static>>, from: usize, to: usize, style: Style) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    let mut at = 0;
    for span in spans {
        let chars: Vec<char> = span.content.chars().collect();
        let (start, end) = (at, at + chars.len());
        at = end;
        if end <= from || start >= to {
            out.push(span);
            continue;
        }
        let (a, b) = (from.saturating_sub(start).min(chars.len()), (to - start).min(chars.len()));
        let piece = |x: usize, y: usize| chars[x..y].iter().collect::<String>();
        if a > 0 {
            out.push(Span::styled(piece(0, a), span.style));
        }
        out.push(Span::styled(piece(a, b), span.style.patch(style)));
        if b < chars.len() {
            out.push(Span::styled(piece(b, chars.len()), span.style));
        }
    }
    if to > at && from <= at {
        out.push(Span::styled(" ", style));
    }
    out
}

/// The line `ctrl+k` opens: what to ask the helper, typed — about the selected part, when there
/// is one. Returns the line and where its cursor is.
fn instruction_line(theme: &Theme, console: &Console, text: &str, width: usize) -> (Line<'static>, usize) {
    let fill = theme.popup();
    let mut cells = Cells::new();
    cells.push("      ", Style::default());
    cells.push(format!("{} ", console.assistant.glyph()), theme.claude());
    let about = if console.selection().is_some() { " about the selection" } else { "" };
    cells.push(format!("ask {}{about} ▸ ", console.assistant.name()), theme.strong());
    let keys = "⏎ asks · esc ";
    let room = width.saturating_sub(cells.width() + fmt::width(keys) + 2);
    // The end of a long question stays in sight, as the cursor is there.
    let shown: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(room.max(1))..].iter().collect()
    };
    cells.push(shown, theme.text());
    let cursor_at = cells.width();
    // Nothing typed yet: what to type — and what ⏎ alone does.
    if text.is_empty() {
        let failed = matches!(&console.state, RunState::Failed { node, .. } if !node.is_empty());
        let hint = if console.selection().is_some() {
            "make it faster, explain it, fix it…"
        } else if failed {
            "what it should do — or just ⏎: put right what the server said"
        } else if !console.sql().trim().is_empty() {
            "what it should do — or just ⏎: what its -- comments ask"
        } else {
            "what it should show — top 10 users by memory, slow queries today…"
        };
        cells.push(fmt::truncate(hint, room), theme.faint().add_modifier(Modifier::ITALIC));
    }
    cells.pad_to(width.saturating_sub(fmt::width(keys)));
    cells.push(keys, theme.faint());
    let line = cells.line(width, Style::default());
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .enumerate()
        .map(|(i, span)| if i == 0 { span } else { Span::styled(span.content, fill.patch(span.style)) })
        .collect();
    (Line::from(spans), cursor_at)
}

/// A line of SQL in colour: keywords, strings, numbers, and a `--` comment dimmed to its end.
fn coloured(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let (code, comment) = split_comment(text);
    let pieces = highlight(code);
    let mut spans: Vec<Span<'static>> = pieces
        .iter()
        .enumerate()
        .map(|(i, (piece, token))| {
            // A keyword before a dot is a database's name: `system.processes`.
            let named = pieces.get(i + 1).is_some_and(|(next, _)| next.starts_with('.'));
            let style = match token {
                Token::Keyword if named => theme.text(),
                Token::Keyword => theme.accent().add_modifier(Modifier::BOLD),
                Token::String => theme.sev(Severity::Ok),
                Token::Number => theme.sev(Severity::Warn),
                Token::Plain => theme.text(),
            };
            Span::styled(piece.clone(), style)
        })
        .collect();
    if !comment.is_empty() {
        spans.push(Span::styled(comment.to_string(), theme.muted().add_modifier(Modifier::ITALIC)));
    }
    spans
}

/// A line cut where its `--` comment starts, outside a string.
fn split_comment(line: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    let mut previous = '\0';
    for (at, c) in line.char_indices() {
        match quote {
            Some(q) if c == q && previous != '\\' => quote = None,
            Some(_) => {}
            None if matches!(c, '\'' | '"' | '`') => quote = Some(c),
            None if c == '-' && line[at + 1..].starts_with('-') => return (&line[..at], &line[at..]),
            None => {}
        }
        previous = c;
    }
    (line, "")
}

/// The line under the field: the helper writing, what it wrote, why it could not — else the keys
/// that matter most here. Returns where on it the way to ask the helper is, for a click.
fn helper_line(app: &App, theme: &Theme, console: &Console, width: usize) -> (Line<'static>, Option<(usize, usize)>) {
    let mut cells = Cells::new();
    // Where the way to ask the helper is on the line, for a click: its column and width.
    let mut asks = None;
    cells.push("      ", Style::default());
    let who = console.assistant.name();
    let mark = format!("{} ", console.assistant.glyph());
    match &console.assisting {
        Assisting::Asking { since, .. } => {
            let spin = ["◐", "◓", "◑", "◒"][(app.now().rem_euclid(4)) as usize];
            cells.push(format!("{spin} "), theme.claude());
            cells.push(format!("{who} is writing it"), theme.text());
            cells.push(format!(" · {}", fmt::dur((app.now() - since).max(0) as f64)), theme.muted());
            cells.push(" · ctrl+c stops", theme.faint());
        }
        Assisting::Wrote { part: None } => {
            cells.push(mark, theme.claude());
            cells.push(format!("{who} wrote this"), theme.strong());
            cells.push(" — read it first: ⏎ after its ; runs it · ctrl+z puts back what was there", theme.muted());
        }
        Assisting::Wrote { part: Some(_) } => {
            cells.push(mark, theme.claude());
            cells.push(format!("{who} rewrote the part marked"), theme.strong());
            cells.push(" — read it first: ctrl+r runs all of it · ctrl+z puts back what was there", theme.muted());
        }
        Assisting::Failed(why) => {
            cells.push(mark, theme.sev(Severity::Warn));
            cells.push(format!("{who}: "), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            cells.push(why.clone(), theme.sev(Severity::Warn));
        }
        // What failed, with what the server said, is the helper's to put right.
        Assisting::Idle if matches!(&console.state, RunState::Failed { node, .. } if !node.is_empty()) => {
            let at = cells.width();
            cells.push(mark, theme.claude());
            cells.push("ctrl+k", theme.strong());
            cells.push(format!(": {who} puts it right, told what the server said"), theme.muted());
            asks = Some((at, cells.width() - at));
            cells.push(" · ctrl+r runs it again", theme.faint());
        }
        // Something selected: what can be done with it.
        Assisting::Idle if console.selection().is_some() => {
            let count = console.selected_text().map_or(0, |t| t.chars().count());
            cells.push(fmt::plural(count, "character", "characters"), theme.strong());
            cells.push(" selected", theme.muted());
            let ask = format!("asks {who} about it");
            let words: [(&str, &str); 4] = [("⌫", "deletes it"), ("ctrl+k", &ask), ("ctrl+r", "runs it"), ("ctrl+c", "copies")];
            for (key, what) in words {
                let mut piece = Cells::new();
                piece.push(" · ", theme.faint());
                let at = cells.width() + piece.width();
                piece.push(key, theme.muted().add_modifier(Modifier::BOLD));
                piece.push(format!(" {what}"), theme.faint());
                if cells.width() + piece.width() > width {
                    break;
                }
                if key == "ctrl+k" {
                    asks = Some((at, cells.width() + piece.width() - at));
                }
                cells.spans(piece.into_spans());
            }
        }
        Assisting::Idle => {
            let ask = format!("asks {who}");
            let words: [(&str, &str); 6] = [
                ("⏎", "after ; runs"),
                ("ctrl+k", &ask),
                ("ctrl+a", "selects all"),
                ("ctrl+u", "clears"),
                ("tab", "completes"),
                ("ctrl+o", "another server"),
            ];
            for (i, (key, what)) in words.iter().enumerate() {
                let mut piece = Cells::new();
                if i > 0 {
                    piece.push(" · ", theme.faint());
                }
                let at = cells.width() + piece.width();
                piece.push(*key, theme.muted().add_modifier(Modifier::BOLD));
                piece.push(format!(" {what}"), theme.faint());
                if cells.width() + piece.width() > width {
                    break;
                }
                if *key == "ctrl+k" {
                    asks = Some((at, cells.width() + piece.width() - at));
                }
                cells.spans(piece.into_spans());
            }
        }
    }
    (cells.line(width, Style::default()), asks)
}

/// The line about the last run: under way, stopped, refused, or what it answered. Returns the
/// rows it took.
fn status(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, console: &Console) -> u16 {
    let width = area.width as usize;
    let mut cells = Cells::new();
    match &console.state {
        RunState::Running { node, since, .. } => {
            let spin = ["◐", "◓", "◑", "◒"][(app.now().rem_euclid(4)) as usize];
            cells.push(format!("{spin} "), theme.accent().add_modifier(Modifier::BOLD));
            cells.push(format!("running on {node}"), theme.text());
            cells.push(format!(" · {}", fmt::dur((app.now() - since).max(0) as f64)), theme.muted());
            cells.push(" · ctrl+c stops it", theme.faint());
        }
        RunState::Stopped => {
            cells.push("■ ", theme.muted());
            cells.push("stopped — the server was told", theme.muted());
        }
        RunState::Failed { node, error } => {
            let mut text = vec![Span::styled("✖ ", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD))];
            if !node.is_empty() {
                text.push(Span::styled(format!("{node} · "), theme.muted()));
            }
            text.push(Span::styled(error.clone(), theme.sev(Severity::Crit)));
            let rows = ((fmt::width(error) + fmt::width(node) + 5) / width.max(1) + 1).min(4) as u16;
            let rows = rows.min(area.height);
            frame.render_widget(Paragraph::new(Line::from(text)).wrap(Wrap { trim: false }), Rect::new(area.x, area.y, area.width, rows));
            return rows;
        }
        RunState::Idle => match &console.answer {
            Some(answer) => {
                cells.push("✔ ", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD));
                if answer.text.is_some() && answer.columns.is_empty() {
                    cells.push("answered", theme.text());
                } else {
                    cells.push(fmt::plural(answer.rows.len(), "row", "rows"), theme.strong());
                }
                cells.push(format!(" · {}", millis(answer.elapsed_ms)), theme.text2());
                if let (Some(rows), Some(bytes)) = (answer.read_rows, answer.read_bytes) {
                    let what = if rows == 1 { "row" } else { "rows" };
                    cells.push(format!(" · read {} {what}, {}", fmt::count(rows), fmt::bytes(bytes)), theme.muted());
                }
                cells.push(format!(" · on {}", answer.node), theme.muted());
                if answer.cut {
                    cells.push(format!(" · the first {} only", answer.rows.len()), theme.sev(Severity::Warn));
                }
            }
            None => {
                cells.push("nothing that runs here can change a table: every query goes read-only, with a time and a row limit", theme.faint());
            }
        },
    }
    if console.focus == Focus::Answer
        && let Some(answer) = &console.answer
    {
        let at = format!("row {} of {} · y copies it · tab back", (console.scroll_row + 1).min(answer.rows.len()), answer.rows.len());
        if cells.width() + fmt::width(&at) + 3 <= width {
            cells.pad_to(width - fmt::width(&at));
            cells.push(at, theme.accent());
        }
    }
    frame.render_widget(Paragraph::new(cells.line(width, Style::default())), Rect::new(area.x, area.y, area.width, 1));
    1
}

fn millis(ms: u64) -> String {
    if ms < 1000 { format!("{ms} ms") } else { format!("{:.2} s", ms as f64 / 1000.0) }
}

/// The answer as a table: as many columns as fit from the first one scrolled to, numbers on the
/// right, `NULL` dimmed; or, in a format of the query's own, the text as it came.
fn answer_table(frame: &mut Frame, theme: &Theme, area: Rect, answer: &Answer, console: &Console) {
    let width = area.width as usize;
    let height = area.height as usize;
    if answer.columns.is_empty() {
        let lines: Vec<Line<'static>> = answer
            .text
            .as_deref()
            .unwrap_or("the query answered nothing")
            .lines()
            .skip(console.scroll_row)
            .take(height)
            .map(|line| Line::from(Span::styled(fmt::truncate(line, width), theme.text())))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    // Each column as wide as its name and what is on screen of it, within reason.
    let body_rows = height.saturating_sub(2);
    let visible = answer.rows.iter().skip(console.scroll_row).take(body_rows.max(1));
    let mut widths: Vec<usize> = answer.columns.iter().map(|(name, _)| fmt::width(name).clamp(3, COLUMN_MAX)).collect();
    for row in visible {
        for (i, value) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(value.as_deref().map_or(4, fmt::width).min(COLUMN_MAX));
            }
        }
    }
    let first = console.scroll_col.min(answer.columns.len().saturating_sub(1));
    let mut shown = Vec::new();
    let mut used = 2;
    for (i, &cells) in widths.iter().enumerate().skip(first) {
        if used + cells > width && !shown.is_empty() {
            break;
        }
        used += cells + 2;
        shown.push(i);
    }
    let more_right = shown.last().is_some_and(|last| last + 1 < answer.columns.len());
    let numeric: Vec<bool> = answer.columns.iter().map(|(_, kind)| crate::console::is_number(kind)).collect();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut header = Cells::new();
    let mut rule = Cells::new();
    header.push(if first > 0 { "← " } else { "  " }, theme.accent());
    rule.push("  ", Style::default());
    for &i in &shown {
        let (name, _) = &answer.columns[i];
        let text = fmt::truncate(name, widths[i]);
        if numeric[i] {
            header.cell_right(&text, widths[i], theme.section());
        } else {
            header.cell(&text, widths[i], theme.section());
        }
        header.push("  ", Style::default());
        rule.push("─".repeat(widths[i]), theme.rule());
        rule.push("  ", Style::default());
    }
    if more_right {
        header.pad_to(width.saturating_sub(2));
        header.push(" →", theme.accent());
    }
    lines.push(header.line(width, Style::default()));
    lines.push(rule.line(width, Style::default()));
    if answer.rows.is_empty() {
        lines.push(Line::from(Span::styled("  no rows", theme.faint())));
    }
    for (n, row) in answer.rows.iter().enumerate().skip(console.scroll_row).take(body_rows) {
        let mut cells = Cells::new();
        let on = console.focus == Focus::Answer && n == console.scroll_row;
        cells.push(if on { "▌ " } else { "  " }, theme.accent());
        for &i in &shown {
            let (text, style) = match row.get(i).cloned().flatten() {
                Some(value) => (fmt::truncate(&value.replace(['\n', '\t'], " "), widths[i]), theme.text()),
                None => ("NULL".to_string(), theme.faint()),
            };
            if numeric[i] {
                cells.cell_right(&text, widths[i], style);
            } else {
                cells.cell(&text, widths[i], style);
            }
            cells.push("  ", Style::default());
        }
        lines.push(cells.line(width, if on { theme.selected() } else { Style::default() }));
    }
    if let Some(note) = &answer.text {
        lines.push(Line::from(Span::styled(format!("  {}", fmt::truncate(note, width.saturating_sub(2))), theme.sev(Severity::Warn))));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// What a suggestion is, at a glance.
fn glyph(what: What) -> &'static str {
    match what {
        What::Column => "▪",
        What::Table => "▦",
        What::Database => "◫",
        What::Function => "ƒ",
        What::Keyword => "·",
        What::Format => "≡",
    }
}

/// The suggestions for the word at the cursor, under its line — over it when there is no room
/// below — from where the word starts: a click or `tab` takes one.
fn suggestions(frame: &mut Frame, app: &App, theme: &Theme, pane: Rect, open: &Suggest, x: u16, cursor_y: u16) {
    if open.items.is_empty() {
        return;
    }
    let shown = open.items.len().min(SUGGESTIONS_SHOWN);
    let first = open.at.saturating_sub(shown - 1).min(open.items.len() - shown);
    let text_width = open.items.iter().map(|s| fmt::width(&s.text)).max().unwrap_or(4).min(40);
    let detail_width = open.items.iter().map(|s| fmt::width(&s.detail)).max().unwrap_or(0).min(24);
    let width = (4 + text_width + if detail_width > 0 { detail_width + 3 } else { 1 }) as u16;
    let width = width.max(24).min(pane.width);
    let more = open.items.len() > shown;
    let height = shown as u16 + u16::from(more);
    let below = (pane.y + pane.height).saturating_sub(cursor_y + 1);
    let y = if below >= height { cursor_y + 1 } else { cursor_y.saturating_sub(height).max(pane.y) };
    let x = x.min((pane.x + pane.width).saturating_sub(width)).max(pane.x);
    let rect = Rect::new(x, y, width, height);
    frame.render_widget(Clear, rect);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, item) in open.items.iter().enumerate().skip(first).take(shown) {
        let on = i == open.at;
        let fill = if on { theme.popup_chosen() } else { theme.popup() };
        let mut cells = Cells::new();
        cells.push(if on { "▌" } else { " " }, theme.accent());
        let kind_style = match item.what {
            What::Function => theme.claude(),
            What::Keyword | What::Format => theme.accent(),
            What::Table | What::Database => theme.sev(Severity::Ok),
            What::Column => theme.muted(),
        };
        cells.push(format!("{} ", glyph(item.what)), kind_style);
        cells.push(fmt::truncate(&item.text, text_width), if on { theme.strong() } else { theme.text() });
        if detail_width > 0 {
            cells.pad_to(3 + text_width + 2);
            cells.push(fmt::truncate(&item.detail, detail_width), if on { theme.muted() } else { theme.faint() });
        }
        let line = cells.line(width as usize, Style::default());
        lines.push(Line::from(line.spans.into_iter().map(|s| Span::styled(s.content, fill.patch(s.style))).collect::<Vec<_>>()));
        app.viewport.hits.borrow_mut().push((Rect::new(rect.x, rect.y + lines.len() as u16 - 1, rect.width, 1), Hit::Suggestion(i)));
    }
    if more {
        let note = format!("   {} of {} · ↑↓ · tab takes it", open.at + 1, open.items.len());
        lines.push(Line::from(Span::styled(fmt::pad(&note, width as usize), theme.popup().patch(theme.faint()))));
    }
    frame.render_widget(Paragraph::new(lines), rect);
}

/// The fleet's servers, under the chip: a click or `⏎` runs the session's queries on one.
fn servers_list(frame: &mut Frame, app: &App, theme: &Theme, at: Rect, servers: &[(String, bool)], cursor: usize, pane: Rect) {
    if servers.is_empty() {
        return;
    }
    let width = servers.iter().map(|(name, _)| fmt::width(name)).max().unwrap_or(10).max(18) + 18;
    let width = (width as u16).min(pane.width.saturating_sub(at.x - pane.x));
    let height = (servers.len() as u16 + 2).min(pane.height.saturating_sub(1));
    let list = Rect::new(at.x, at.y, width, height);
    frame.render_widget(Clear, list);
    frame.render_widget(Block::new().style(theme.popup()), list);
    let mut lines = vec![Line::from(Span::styled(" run on…", theme.popup().patch(theme.muted())))];
    let first = cursor.saturating_sub(height.saturating_sub(3) as usize);
    for (i, (name, ok)) in servers.iter().enumerate().skip(first).take(height.saturating_sub(2) as usize) {
        let on = i == cursor;
        let mut cells = Cells::new();
        cells.push(if on { "▌" } else { " " }, theme.accent());
        cells.push(if *ok { "● " } else { "↯ " }, if *ok { theme.sev(Severity::Ok) } else { theme.sev(Severity::Crit) });
        cells.push(name.clone(), if on { theme.strong() } else { theme.text() });
        if !ok {
            cells.push("  not answering", theme.faint());
        }
        let fill = if on { theme.popup_chosen() } else { theme.popup() };
        let line = cells.line(width as usize, Style::default());
        lines.push(Line::from(line.spans.into_iter().map(|s| Span::styled(s.content, fill.patch(s.style))).collect::<Vec<_>>()));
        app.viewport.hits.borrow_mut().push((Rect::new(list.x, list.y + lines.len() as u16 - 1, list.width, 1), Hit::ConsolePick(i)));
    }
    frame.render_widget(Paragraph::new(lines), list);
}

#[cfg(test)]
mod tests {
    use super::split_comment;

    #[test]
    fn a_comment_is_cut_off_where_it_starts_outside_a_string() {
        assert_eq!(split_comment("SELECT 1 -- one"), ("SELECT 1 ", "-- one"));
        assert_eq!(split_comment("SELECT '--' AS dashes"), ("SELECT '--' AS dashes", ""));
        assert_eq!(split_comment("-- all of it"), ("", "-- all of it"));
        assert_eq!(split_comment("SELECT 5 - -3"), ("SELECT 5 - -3", ""));
    }
}
