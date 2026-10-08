//! View 0's AI panel: what Claude Code and OpenCode used on this machine (`usage.rs`).
//!
//! ```text
//!  AI · today                                     Wed 8 Oct
//!
//!  ✻ Claude Code          287.0M tokens        ≈ $191.17
//!    in 1.2M · out 3.4M · cache 282.4M · 1321 replies
//!    5h window  ━━━━━━━━━━━━━━╸━━━━━━  1h26m left · 207.0M
//!
//!  ▣ OpenCode              53.9M tokens            $0.86
//!    in 120k · out 340k · cache 53.4M · 131 replies
//!
//!  last 7 days                                      3.1B
//!     ▅▅   ▆▆   ▂▂   ██   ▄▄   ██   ▄▄
//!     Thu  Fri  Sat  Sun  Mon  Tue  Wed
//!
//!  by model                    by project
//!    opus-5-5      238.7M        airflow-dags   181.5M
//! ```
//!
//! A number a line, its parts quiet under it; a bar only where a share is the point. Counts, so no
//! colour but each program's own mark: severity has nothing to say here.

use super::widgets::{thin_bar, Cells};
use crate::app::App;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::usage::{dollars, tokens, Summary, Tally, Tool};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

fn tool_style(tool: Tool, theme: &Theme) -> Style {
    match tool {
        Tool::Claude => theme.claude().add_modifier(Modifier::BOLD),
        Tool::OpenCode => theme.strong(),
    }
}

/// `≈ $191.17` for Claude Code (no price on a plan), `$0.86` for OpenCode (its own record).
fn money(tool: Tool, tally: &Tally) -> (String, Style, Style) {
    let floor = if tally.unpriced { "≥ " } else { "" };
    match tool {
        Tool::Claude => (format!("≈ {floor}{}", dollars(tally.dollars)), Style::default(), Style::default()),
        Tool::OpenCode => (format!("{floor}{}", dollars(tally.dollars)), Style::default(), Style::default()),
    }
}

/// The panel.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let width = area.width as usize;
    let now = app.now();
    let summary = app.usage.summary(now, app.time.offset_s(now));
    let mut lines: Vec<Line<'static>> = Vec::new();

    let mut title = Cells::new();
    title.push("AI", theme.section());
    title.push(" · today", theme.muted());
    let date = app.time.format(now, "%a %-d %b");
    title.pad_to(width.saturating_sub(date.len() + 1));
    title.push(date, theme.faint());
    lines.push(title.line(width, Style::default()));
    lines.push(Line::from(""));

    for tool in [Tool::Claude, Tool::OpenCode] {
        let tally = summary.today.get(&tool).copied().unwrap_or_default();
        lines.extend(program(tool, &tally, &summary, now, theme, width));
        lines.push(Line::from(""));
    }

    lines.extend(week(&summary, app, now, theme, width));
    lines.push(Line::from(""));
    lines.extend(breakdown(&summary, theme, width));

    // What ≈ means, and what could not be read.
    let room = area.height as usize;
    let mut foot: Vec<Line<'static>> = Vec::new();
    for note in &app.usage.notes {
        let mut cells = Cells::new();
        cells.push(format!("▲ {note}"), theme.sev(Severity::Warn));
        foot.push(cells.line(width, Style::default()));
    }
    let mut legend = Cells::new();
    legend.push("≈ Claude Code at API prices — a plan has none", theme.faint());
    foot.push(legend.line(width, Style::default()));
    if lines.len() + 1 + foot.len() <= room {
        while lines.len() + foot.len() < room {
            lines.push(Line::from(""));
        }
        lines.extend(foot);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// A program's day: its mark, name, tokens and money; its parts under; Claude Code's window.
fn program(tool: Tool, tally: &Tally, summary: &Summary, now: i64, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut head = Cells::new();
    head.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
    head.push(tool.name(), theme.text());
    let (money, _, _) = money(tool, tally);
    let used = format!("{} tokens", tokens(tally.tokens));
    let middle = width.saturating_sub(money.len()).saturating_sub(used.len() + 4);
    head.pad_to(middle.max(head.width() + 2));
    head.push(tokens(tally.tokens), theme.strong());
    head.push(" tokens", theme.muted());
    head.pad_to(width.saturating_sub(money.chars().count()));
    head.push(money, if tally.dollars > 0.0 { theme.text() } else { theme.faint() });
    let mut out = vec![head.line(width, Style::default())];

    let mut parts = Cells::new();
    parts.push("  ", Style::default());
    if tally.replies == 0 {
        parts.push("nothing today", theme.faint());
    } else {
        for (i, (label, value)) in [("in", tally.input), ("out", tally.output), ("cache", tally.cache)].into_iter().enumerate() {
            if i > 0 {
                parts.push(" · ", theme.faint());
            }
            parts.push(format!("{label} "), theme.faint());
            parts.push(tokens(value), theme.text2());
        }
        parts.push(" · ", theme.faint());
        parts.push(format!("{} replies", tally.replies), theme.muted());
    }
    out.push(parts.line(width, Style::default()));

    if tool == Tool::Claude
        && let Some(window) = summary.window
    {
        let mut line = Cells::new();
        line.push("  5h window  ", theme.faint());
        let left = (window.end - now).max(0);
        let share = ((now - window.start) as f64 / (window.end - window.start) as f64 * 100.0).clamp(0.0, 100.0);
        let tail = format!("  {} left · {}", crate::fmt::dur(left as f64), tokens(window.tally.tokens));
        let bar = width.saturating_sub(line.width() + tail.chars().count()).clamp(6, 28);
        line.spans(thin_bar(Some(share), bar, theme.bar_fill(Severity::None), theme));
        line.push(format!("  {}", crate::fmt::dur(left as f64)), theme.text2());
        line.push(" left · ", theme.faint());
        line.push(tokens(window.tally.tokens), theme.text2());
        out.push(line.line(width, Style::default()));
    }
    out
}

/// The last seven days as columns, eighths of a cell, three rows tall, the days under them.
fn week(summary: &Summary, app: &App, now: i64, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut head = Cells::new();
    head.push("last 7 days", theme.section());
    let total = tokens(summary.week.tokens);
    head.pad_to(width.saturating_sub(total.len()));
    head.push(total, theme.text2());
    let mut out = vec![head.line(width, Style::default())];

    let totals: Vec<u64> = summary.days.iter().map(|d| d.values().sum()).collect();
    let top = totals.iter().copied().max().unwrap_or(0).max(1);
    const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let column = (width.saturating_sub(2) / 7).clamp(3, 7);
    let fill = Style::default().fg(theme.bar_fill(Severity::None));
    let rows = 3;
    for row in (0..rows).rev() {
        let mut line = Cells::new();
        line.push("  ", Style::default());
        for (i, &n) in totals.iter().enumerate() {
            let height = (n as f64 / top as f64 * (rows * 8) as f64).round() as usize;
            let here = height.saturating_sub(row * 8).min(8);
            let glyph = EIGHTHS[if n > 0 && height == 0 && row == 0 { 1 } else { here }];
            let style = if i == 6 { theme.accent() } else { fill };
            line.push(glyph.repeat(2), style);
            line.gap(column - 2);
        }
        out.push(line.line(width, Style::default()));
    }
    let mut days = Cells::new();
    days.push("  ", Style::default());
    for i in 0..7 {
        let at = now - (6 - i as i64) * 86_400;
        let name = app.time.format(at, "%a");
        days.cell(&name.chars().take(column.saturating_sub(1)).collect::<String>(), column, if i == 6 { theme.accent() } else { theme.faint() });
    }
    out.push(days.line(width, Style::default()));
    out
}

/// Today by model and by project, side by side when there is room, each the five most.
fn breakdown(summary: &Summary, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let half = width / 2;
    let side = width >= 56;
    let models: Vec<(String, Style, u64)> = summary.models.iter().take(5).map(|(m, t, n)| (m.clone(), tool_style(*t, theme), *n)).collect();
    let projects: Vec<(String, Style, u64)> = summary.projects.iter().take(5).map(|(p, n)| (p.clone(), theme.text(), *n)).collect();
    let block = |title: &str, rows: &[(String, Style, u64)], room: usize| -> Vec<Cells> {
        let mut out = Vec::new();
        let mut head = Cells::new();
        head.push(title.to_string(), theme.section());
        out.push(head);
        let top = rows.first().map_or(1, |r| r.2).max(1);
        for (name, style, n) in rows {
            let mut cells = Cells::new();
            cells.push("  ", Style::default());
            let name_room = room.saturating_sub(2 + 8 + 1 + 6);
            cells.cell(&crate::fmt::truncate(name, name_room), name_room, if *style == theme.text() { theme.text() } else { *style });
            cells.cell_right(&tokens(*n), 7, theme.text2());
            cells.push(" ", Style::default());
            let share = *n as f64 / top as f64 * 100.0;
            cells.spans(thin_bar(Some(share), 6.min(room.saturating_sub(cells.width())), theme.bar_fill(Severity::None), theme));
            out.push(cells);
        }
        if rows.is_empty() {
            let mut none = Cells::new();
            none.push("  nothing today", theme.faint());
            out.push(none);
        }
        out
    };
    if side {
        let left = block("by model", &models, half.saturating_sub(2));
        let right = block("by project", &projects, width - half);
        (0..left.len().max(right.len()))
            .map(|i| {
                let mut line = Cells::new();
                if let Some(cells) = left.get(i) {
                    line.spans(cells.clone().into_spans());
                }
                line.pad_to(half);
                if let Some(cells) = right.get(i) {
                    line.spans(cells.clone().into_spans());
                }
                line.line(width, Style::default())
            })
            .collect()
    } else {
        let mut out: Vec<Line<'static>> = block("by model", &models, width).into_iter().map(|c| c.line(width, Style::default())).collect();
        out.push(Line::from(""));
        out.extend(block("by project", &projects, width).into_iter().map(|c| c.line(width, Style::default())));
        out
    }
}

/// The narrow view's line of it, under the machine's pressure:
/// `AI        ✻ 287.0M today ≈ $191.17 · 5h 1h26m left   ▣ 53.9M $0.86`
pub fn line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let now = app.now();
    let summary = app.usage.summary(now, app.time.offset_s(now));
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.cell("AI", 10, theme.section());
    for (i, tool) in [Tool::Claude, Tool::OpenCode].into_iter().enumerate() {
        let tally = summary.today.get(&tool).copied().unwrap_or_default();
        if i > 0 {
            cells.push("   ", Style::default());
        }
        cells.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
        cells.push(tokens(tally.tokens), theme.strong());
        cells.push(if i == 0 { " today " } else { " " }, theme.faint());
        cells.push(money(tool, &tally).0, theme.text2());
        if tool == Tool::Claude
            && let Some(w) = summary.window
        {
            cells.push(format!(" · 5h {} left", crate::fmt::dur((w.end - now).max(0) as f64)), theme.faint());
        }
    }
    cells.line(width, Style::default())
}
