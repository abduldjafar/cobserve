//! A page open on view 5 or 6 (`detail.rs`): a ticket in full, a DAG's runs, a run's tasks, a
//! task's log — drawn in the view's place, a line saying where it is above it; `esc` goes back.
//!
//! Lists (runs, tasks) have a cursor and a click; text (a ticket, a log) scrolls. A log opens at
//! its end, where a failure says why, and its errors are drawn in the failure's colour.

use super::widgets::{rule, Cells};
use crate::airflow::{self, RunState};
use crate::app::{scroll_into_view, App, Hit};
use crate::detail::{first_line, Ask, Body, Page};
use crate::fmt;
use crate::jira::{self, Prose};
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const GAP: usize = 2;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, page: &Page) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let height = area.height as usize;
    let mut lines = vec![where_line(app, page, theme, width), Line::from("")];
    let head = lines.len();
    let room = height.saturating_sub(head);
    let now = app.now();

    match &page.body {
        None => lines.push(Line::from(Span::styled("  reading…", theme.accent()))),
        Some(Err(why)) => {
            for line in wrap(&format!("could not be read: {why}"), width.saturating_sub(4)) {
                lines.push(Line::from(Span::styled(format!("  {line}"), theme.sev(Severity::Warn))));
            }
            lines.push(Line::from(Span::styled("  r asks again · esc goes back", theme.muted())));
        }
        Some(Ok(Body::Issue(issue))) => {
            let text = issue_lines(issue, app, now, theme, width);
            let first = first_line(page.scroll, text.len(), room);
            lines.extend(text.into_iter().skip(first).take(room));
        }
        Some(Ok(Body::Log(log))) => {
            let mut text: Vec<Line<'static>> = Vec::new();
            for raw in log {
                let style = log_style(raw, theme);
                for part in wrap(raw, width.saturating_sub(2)) {
                    text.push(Line::from(Span::styled(format!(" {part}"), style)));
                }
            }
            if text.is_empty() {
                text.push(Line::from(Span::styled("  the log is empty", theme.muted())));
            }
            let first = first_line(page.scroll, text.len(), room);
            lines.extend(text.into_iter().skip(first).take(room));
        }
        Some(Ok(Body::Runs(runs))) => {
            let rows: Vec<Line<'static>> = runs.iter().enumerate().map(|(i, run)| run_line(run, i == page.cursor, app, now, theme, width)).collect();
            list(frame, app, area, head, rows, page.cursor, "  no run of this DAG", lines, theme);
            return;
        }
        Some(Ok(Body::Tasks(tasks))) => {
            let id_w = tasks.iter().map(|t| fmt::width(&t.label())).max().unwrap_or(10).clamp(10, 40);
            let rows: Vec<Line<'static>> = tasks.iter().enumerate().map(|(i, task)| task_line(task, i == page.cursor, id_w, app, now, theme, width)).collect();
            list(frame, app, area, head, rows, page.cursor, "  no task in this run", lines, theme);
            return;
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// A list under the page's line: the window follows the cursor, every row a click.
#[allow(clippy::too_many_arguments)]
fn list(frame: &mut Frame, app: &App, area: Rect, head: usize, rows: Vec<Line<'static>>, cursor: usize, empty: &str, mut lines: Vec<Line<'static>>, theme: &Theme) {
    let room = (area.height as usize).saturating_sub(head);
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(empty.to_string(), theme.muted())));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let len = rows.len();
    let offset = scroll_into_view(app.viewport.page.get(), Some(cursor), room, len);
    app.viewport.page.set(offset);
    let mut hits = app.viewport.hits.borrow_mut();
    for (i, row) in rows.into_iter().enumerate().skip(offset).take(room) {
        let y = area.y + lines.len() as u16;
        hits.push((Rect::new(area.x, y, area.width, 1), Hit::PageRow(i)));
        lines.push(row);
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

/// `◂ DATA-2647 · Client account balances…` · `◂ etl › scheduled · 06:00 › load · try 1`
fn where_line(app: &App, page: &Page, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(" ◂ ", theme.accent());
    let crumb = |cells: &mut Cells, text: String, last: bool| {
        cells.push(text, if last { theme.strong() } else { theme.text2() });
    };
    let sep = |cells: &mut Cells| {
        cells.push(" › ", theme.faint());
    };
    let now = app.now();
    let run_words = |dag: &str, run: &str| match app.airflow.run(dag, run) {
        Some(found) => airflow::run_label(found, &app.time, now),
        None => run.to_string(),
    };
    match &page.ask {
        Ask::Issue(key) => {
            cells.push(key.clone(), theme.accent().add_modifier(Modifier::BOLD));
            if let Some(Ok(Body::Issue(issue))) = &page.body {
                cells.push(" · ", theme.faint());
                cells.push(jira::split_tag(&issue.summary).1.to_string(), theme.strong());
            }
        }
        Ask::Runs(dag) => {
            crumb(&mut cells, dag.clone(), false);
            sep(&mut cells);
            crumb(&mut cells, "its runs".into(), true);
        }
        Ask::Tasks { dag, run } => {
            crumb(&mut cells, dag.clone(), false);
            sep(&mut cells);
            crumb(&mut cells, run_words(dag, run), true);
        }
        Ask::Log { dag, run, task, map_index, attempt } => {
            crumb(&mut cells, dag.clone(), false);
            sep(&mut cells);
            crumb(&mut cells, run_words(dag, run), false);
            sep(&mut cells);
            let task = if *map_index >= 0 { format!("{task} [{map_index}]") } else { task.clone() };
            crumb(&mut cells, format!("{task} · log of try {attempt}"), true);
        }
    }
    let hint = "esc back ";
    let mut line = Cells::new();
    line.spans(super::widgets::fit(cells.into_spans(), width.saturating_sub(fmt::width(hint) + 2), false));
    line.pad_to(width.saturating_sub(fmt::width(hint)));
    line.push(hint, theme.faint());
    line.line(width, Style::default())
}

/// Text broken into lines of at most `width` cells, at spaces where it can be; a line's indent is
/// kept on the lines it wraps onto, and a word longer than a line is cut, as a terminal would.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let indent_w = text.chars().take_while(|c| *c == ' ').count().min(width / 2);
    let indent = " ".repeat(indent_w);
    let mut out = Vec::new();
    let mut line = indent.clone();
    let mut used = indent_w;
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let w = fmt::width(&word);
            let gap = usize::from(used > indent_w);
            if used + gap + w <= width {
                if gap > 0 {
                    line.push(' ');
                }
                line.push_str(&word);
                used += gap + w;
                break;
            }
            if used > indent_w {
                out.push(std::mem::replace(&mut line, indent.clone()));
                used = indent_w;
                continue;
            }
            let (head, tail) = split_at_width(&word, width - used);
            line.push_str(&head);
            out.push(std::mem::replace(&mut line, indent.clone()));
            word = tail;
        }
    }
    out.push(line);
    out
}

/// A word cut where `cells` run out: what fits, and the rest.
fn split_at_width(word: &str, cells: usize) -> (String, String) {
    let mut used = 0;
    let mut cut = word.len();
    for (at, c) in word.char_indices() {
        let w = fmt::width(&c.to_string());
        if used + w > cells.max(1) {
            cut = at;
            break;
        }
        used += w;
    }
    (word[..cut].to_string(), word[cut..].to_string())
}

/// A log line in its level's colour: errors and tracebacks red, warnings amber, the rest quiet.
fn log_style(line: &str, theme: &Theme) -> Style {
    let error = [" ERROR ", " CRITICAL ", "Traceback (most recent call last)", "Exception:", "Error:", "  File \""];
    if error.iter().any(|e| line.contains(e)) || line.starts_with("airflow.exceptions") {
        return theme.sev(Severity::Crit);
    }
    if line.contains(" WARNING ") {
        return theme.sev(Severity::Warn);
    }
    if line.starts_with("***") || line.contains("::group::") || line.contains("::endgroup::") {
        return theme.faint();
    }
    theme.text2()
}

/// A ticket in full: what it is, its description, its sub-tasks and links, its comments.
fn issue_lines(issue: &jira::IssueDetail, app: &App, now: i64, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let text_w = width.saturating_sub(4);
    for part in wrap(&issue.summary, text_w) {
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(part, theme.strong())]));
    }
    let mut facts: Vec<(String, Style)> = Vec::new();
    facts.push((issue.status.clone(), theme.accent().add_modifier(Modifier::BOLD)));
    facts.extend(issue.kind.clone().map(|k| (k, theme.text2())));
    if let Some(p) = issue.priority.clone().filter(|p| !p.eq_ignore_ascii_case("unspecified")) {
        let sev = jira::Ticket { priority: Some(p.clone()), ..jira::Ticket::default() }.priority_severity();
        facts.push((p, if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text2() }));
    }
    facts.extend(issue.assignee.clone().map(|a| (format!("on {a}"), theme.person())));
    facts.extend(issue.reporter.clone().map(|r| (format!("from {r}"), theme.person())));
    facts.extend(issue.created.map(|c| (format!("made {}", app.time.format(c, "%b %-d")), theme.text2())));
    facts.extend(issue.updated.map(|u| (format!("updated {}", fmt::ago(now - u)), theme.text2())));
    if let Some(due) = issue.due.clone() {
        let today = jira::today(now, app.time.offset_s(now));
        let day = jira::day_number(&due).unwrap_or(today);
        let sev = jira::due_severity(day, today, false);
        facts.push((format!("due {due} ({})", jira::due_label(day, today)), if sev.is_problem() { theme.sev(sev) } else { theme.text2() }));
    }
    facts.extend(issue.logged_s.map(|s| (format!("{} logged", jira::logged(s)), theme.text2())));
    facts.extend(issue.parent.clone().map(|p| (format!("under {p}"), theme.accent())));
    if !issue.labels.is_empty() {
        facts.push((issue.labels.join(", "), theme.muted()));
    }
    let mut row = Cells::new();
    row.push("  ", Style::default());
    for (i, (text, style)) in facts.into_iter().enumerate() {
        let piece = if i > 0 { 3 } else { 0 } + fmt::width(&text);
        if row.width() + piece > width {
            lines.push(row.line_unpadded(width));
            row = Cells::new();
            row.push("  ", Style::default());
        } else if i > 0 {
            row.push(" · ", theme.faint());
        }
        row.push(text, style);
    }
    lines.push(row.line_unpadded(width));

    let section = |lines: &mut Vec<Line<'static>>, title: &str, detail: String| {
        lines.push(Line::from(""));
        lines.push(rule(width, vec![Span::styled(title.to_string(), theme.section()), Span::styled(detail, theme.muted())], theme));
    };
    section(&mut lines, "DESCRIPTION", String::new());
    let prose = jira::prose(&issue.description);
    if prose.is_empty() {
        lines.push(Line::from(Span::styled("  no description", theme.muted())));
    }
    lines.extend(prose_lines(&prose, text_w, theme));

    if !issue.subtasks.is_empty() || !issue.links.is_empty() {
        section(&mut lines, "LINKED", format!(" · {}", issue.subtasks.len() + issue.links.len()));
        let how_w = issue.subtasks.iter().chain(&issue.links).map(|l| fmt::width(&l.how)).max().unwrap_or(8).min(22);
        for link in issue.subtasks.iter().chain(&issue.links) {
            let mut cells = Cells::new();
            cells.push("  ", Style::default());
            cells.cell(&link.how, how_w, theme.muted());
            cells.gap(GAP);
            cells.push(link.key.clone(), theme.accent());
            cells.push("  ", Style::default());
            cells.push(link.summary.clone(), theme.text());
            cells.push(format!("  {}", link.status), theme.faint());
            lines.push(cells.line(width, Style::default()));
        }
    }

    section(&mut lines, "COMMENTS", format!(" · {}", issue.comments.len()));
    if issue.comments.is_empty() {
        lines.push(Line::from(Span::styled("  no comment yet", theme.muted())));
    }
    for comment in &issue.comments {
        let mut who = Cells::new();
        who.push("  ", Style::default());
        who.push(comment.author.clone(), theme.person().add_modifier(Modifier::BOLD));
        if let Some(at) = comment.at {
            who.push(format!(" · {} · {}", app.time.format(at, "%b %-d %H:%M"), fmt::ago(now - at)), theme.muted());
        }
        lines.push(who.line_unpadded(width));
        let body: Vec<(Prose, String)> = jira::prose(&comment.body).into_iter().map(|(k, t)| (k, format!("  {t}"))).collect();
        lines.extend(prose_lines(&body, text_w, theme));
        lines.push(Line::from(""));
    }
    lines
}

/// Wiki markup's lines, wrapped and drawn by what each is.
fn prose_lines(prose: &[(Prose, String)], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (kind, text) in prose {
        match kind {
            Prose::Blank => out.push(Line::from("")),
            Prose::Code => out.push(Line::from(vec![Span::raw("  "), Span::styled(format!(" {} ", fmt::truncate(text, width.saturating_sub(2))), theme.code())])),
            Prose::Heading => out.push(Line::from(vec![Span::raw("  "), Span::styled(text.clone(), theme.strong().add_modifier(Modifier::UNDERLINED))])),
            Prose::Bullet => {
                let lead: String = text.chars().take_while(|c| *c == ' ' || *c == '•').collect();
                let hang = " ".repeat(fmt::width(&lead));
                for (i, part) in wrap(text.trim_start_matches([' ', '•']), width.saturating_sub(fmt::width(&lead))).into_iter().enumerate() {
                    let head = if i == 0 { Span::styled(lead.clone(), theme.accent()) } else { Span::raw(hang.clone()) };
                    out.push(Line::from(vec![Span::raw("  "), head, Span::styled(part, theme.text())]));
                }
            }
            Prose::Text => {
                for part in wrap(text, width) {
                    out.push(Line::from(vec![Span::raw("  "), Span::styled(part, theme.text())]));
                }
            }
        }
    }
    out
}

/// A state as a mark in its colour: `✔` went well, `✖` failed, `▸` runs, `◌` waits.
fn state_mark(state: &str, theme: &Theme) -> (&'static str, Style) {
    match state {
        "success" => ("✔", theme.sev(Severity::Ok)),
        "failed" => ("✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        "running" => ("▸", theme.accent().add_modifier(Modifier::BOLD)),
        "up_for_retry" | "up_for_reschedule" => ("↻", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        "upstream_failed" => ("▲", theme.sev(Severity::Warn)),
        "skipped" => ("»", theme.muted()),
        "queued" | "scheduled" | "deferred" => ("◌", theme.muted()),
        _ => ("·", theme.faint()),
    }
}

fn row_start(selected: bool, mark: (&str, Style), theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    cells.cell(mark.0, 2, mark.1);
    cells
}

/// A run of a DAG's page: `✔ scheduled · 06:00   success   started 06:00:03   took 12m   “note”`
fn run_line(run: &airflow::Run, selected: bool, app: &App, now: i64, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = row_start(selected, state_mark(run.state.word(), theme), theme);
    cells.cell(&airflow::run_label(run, &app.time, now), 22, theme.text());
    cells.gap(GAP);
    let sev = run.severity(now);
    cells.cell(run.state.word(), 8, if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
    cells.gap(GAP);
    let started = run.start.map(|s| app.time.format(s, "%b %-d %H:%M:%S")).unwrap_or_else(|| "—".into());
    cells.cell(&started, 15, theme.muted());
    cells.gap(GAP);
    let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
    cells.cell_right(&took, 8, if run.state == RunState::Running { theme.accent() } else { theme.text2() });
    cells.gap(GAP);
    cells.push(run.id.clone(), theme.faint());
    if let Some(note) = &run.note {
        cells.push(format!("  “{note}”"), theme.text());
    }
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

/// A task of a run's page: `✖ check_consistency   failed   06:00:12   8m06s   try 1/2   SSHOperator   worker-3`
#[allow(clippy::too_many_arguments)]
fn task_line(task: &airflow::TaskRun, selected: bool, id_w: usize, app: &App, now: i64, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = row_start(selected, state_mark(&task.state, theme), theme);
    let sev = task.severity();
    cells.cell(&task.label(), id_w, if sev == Severity::Crit { theme.sev(Severity::Crit) } else { theme.text() });
    cells.gap(GAP);
    cells.cell(&task.state.replace('_', " "), 15, if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
    cells.gap(GAP);
    let started = task.start.map(|s| app.time.format(s, "%H:%M:%S")).unwrap_or_else(|| "—".into());
    cells.cell(&started, 8, theme.muted());
    cells.gap(GAP);
    let took = task.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
    cells.cell_right(&took, 8, if task.state == "running" { theme.accent() } else { theme.text2() });
    cells.gap(GAP);
    let tries = if task.try_number > 0 { format!("try {}/{}", task.try_number, task.max_tries + 1) } else { String::new() };
    cells.cell(&tries, 9, theme.muted());
    cells.gap(GAP);
    cells.cell(task.operator.as_deref().unwrap_or(""), 16, theme.faint());
    cells.gap(GAP);
    let host = task.host.as_deref().map(|h| h.split('.').next().unwrap_or(h).to_string()).unwrap_or_default();
    cells.push(host, theme.faint());
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_wraps_at_spaces_and_cuts_a_word_too_long() {
        assert_eq!(wrap("the quick brown fox jumps", 10), ["the quick", "brown fox", "jumps"]);
        assert_eq!(wrap("abcdefghijklmnopqrstuvwxyz", 10), ["abcdefghij", "klmnopqrst", "uvwxyz"]);
        assert_eq!(wrap("    indented line that wraps here", 16), ["    indented", "    line that", "    wraps here"]);
        assert_eq!(wrap("", 10), [""]);
    }
}
