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
use std::collections::BTreeMap;
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
        lines.extend(program(tool, &tally, &summary, app, theme, width));
        lines.push(Line::from(""));
    }

    lines.extend(week(&summary, app, now, theme, width));
    lines.push(Line::from(""));
    lines.extend(months(&summary, theme, width));
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
fn program(tool: Tool, tally: &Tally, summary: &Summary, app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let now = app.now();
    let plan = app.limits.as_ref().filter(|_| tool == Tool::Claude);
    let mut head = Cells::new();
    head.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
    head.push(tool.name(), theme.text());
    if let Some(plan) = plan.filter(|p| !p.plan.is_empty()) {
        head.push(format!(" · {}", plan.plan), theme.muted());
    }
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

    // The plan's own limits, as `/usage` shows them; without them, the window worked out here.
    if let Some(plan) = plan.filter(|p| !p.limits.is_empty()) {
        out.extend(plan_lines(plan, app, theme, width));
    } else if tool == Tool::Claude
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

/// A line a limit of the plan: what it is, a bar of how much is used — coloured by §7's 75 and 90
/// — the share of that limit, and when it resets; under them, when they are not live, from when.
///
/// ```text
///   session          ━━╸━━━━━━━━━━━━━━━━━  11% of it   resets 20:39
///   this week        ━━━━━━━━━━━━━━╸━━━━━━  70% of it   resets Sun 19:59
///   Fable this week  ━━━━━━╸━━━━━━━━━━━━━━  29% of it   resets Sun 19:59
/// ```
fn plan_lines(plan: &crate::limits::PlanLimits, app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let now = app.now();
    let label_w = plan.limits.iter().map(|l| crate::fmt::width(&l.label)).max().unwrap_or(7).max(7);
    let mut out = Vec::new();
    for limit in &plan.limits {
        let sev = crate::severity::node(Some(limit.percent));
        let mut line = Cells::new();
        line.push("  ", Style::default());
        line.cell(&limit.label, label_w + 2, theme.faint());
        let resets = limit.resets_at.map(|at| {
            let fmt = if at - now < 20 * 3600 { "%H:%M" } else { "%a %H:%M" };
            format!("resets {}", app.time.format(at, fmt))
        });
        let tail = 4 + 8 + resets.as_ref().map_or(0, |r| r.chars().count() + 3);
        let bar = width.saturating_sub(line.width() + tail).clamp(6, 24);
        line.spans(thin_bar(Some(limit.percent), bar, theme.bar_fill(sev), theme));
        line.push(format!("  {:>3.0}%", limit.percent), theme.sev(sev).add_modifier(Modifier::BOLD));
        line.push(" of it", theme.faint());
        if let Some(resets) = resets {
            line.push(format!("   {resets}"), theme.muted());
        }
        out.push(line.line(width, Style::default()));
    }
    if !plan.live {
        let mut line = Cells::new();
        let age = crate::fmt::dur((now - plan.as_of).max(0) as f64);
        line.push(format!("  as Claude Code last saw them, {age} ago"), theme.faint());
        if let Some(why) = &plan.note {
            line.push(format!(" · {why}"), theme.faint());
        }
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

/// Today by model — its tokens, a bar of its share, its money — then by project, each program's
/// tokens and money in a column of its own and the project's total, the day's all under them.
///
/// ```text
/// by model                                       tokens          cost
///   ✻ opus-5-5                  363.2M  ━━━━━━━━━━         ≈ $240.10
///   ▣ deepseek-v4.1-flash        60.1M  ━╸                     $0.42
///
/// by project          ✻ Claude Code       ▣ OpenCode        total
///   airflow-dags      240.1M ≈ $181.90    35.9M   $0.40    276.0M
///   cobserve           39.9M  ≈ $26.20        —             39.9M
///   ───────────────────────────────────────────────────────────────
///   all               ...
/// ```
fn breakdown(summary: &Summary, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    // By model.
    let top = summary.models.first().map_or(1, |m| m.2.tokens).max(1);
    let cost_w = 11;
    let bar_w = 10.min(width / 6);
    let name_w = width.saturating_sub(4 + 8 + 2 + bar_w + cost_w);
    // The column names over the columns: tokens over its numbers, cost at the right.
    let mut head = Cells::new();
    head.push("by model", theme.section());
    head.pad_to(4 + name_w + 8 - 6);
    head.push("tokens", theme.faint());
    head.pad_to(width.saturating_sub(4));
    head.push("cost", theme.faint());
    out.push(head.line(width, Style::default()));
    for (model, tool, tally) in summary.models.iter().take(5) {
        let mut cells = Cells::new();
        cells.push("  ", Style::default());
        cells.push(format!("{} ", tool.glyph()), tool_style(*tool, theme));
        cells.cell(&crate::fmt::truncate(model, name_w), name_w, theme.text());
        cells.cell_right(&tokens(tally.tokens), 8, theme.strong());
        cells.push("  ", Style::default());
        cells.spans(thin_bar(Some(tally.tokens as f64 / top as f64 * 100.0), bar_w, theme.bar_fill(Severity::None), theme));
        cells.cell_right(&money(*tool, tally).0, cost_w, theme.text2());
        out.push(cells.line(width, Style::default()));
    }
    if summary.models.is_empty() {
        out.push(Line::from(ratatui::text::Span::styled("  nothing today", theme.faint())));
    }
    out.push(Line::from(""));

    // By project, a column a program.
    let shown = 6;
    let mut rows: Vec<(String, BTreeMap<Tool, Tally>, Style)> =
        summary.projects.iter().take(shown).map(|(p, by)| (p.clone(), by.clone(), theme.text())).collect();
    let more = (summary.projects.len() > shown).then(|| {
        let rest: u64 = summary.projects.iter().skip(shown).flat_map(|(_, by)| by.values()).map(|t| t.tokens).sum();
        format!("+{} more · {}", summary.projects.len() - shown, tokens(rest))
    });
    if !summary.projects.is_empty() {
        rows.push(("all".to_string(), summary.today.clone(), theme.strong()));
    }
    out.extend(table("by project", &rows, more, true, theme, width));
    out
}

/// A table a column a program — its tokens and its money, `—` where it did not work — and the
/// row's total: by project, by month. `rule_before_last` sets the last row apart as a total.
fn table(title: &str, rows: &[(String, BTreeMap<Tool, Tally>, Style)], more: Option<String>, rule_before_last: bool, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let tools = [Tool::Claude, Tool::OpenCode];
    let tok_w = 7;
    let cost_of = |tool: Tool| if tool == Tool::Claude { 10 } else { 8 };
    let col = |tool: Tool| tok_w + 1 + cost_of(tool);
    let total_w = 7;
    let name_w = width.saturating_sub(2 + col(Tool::Claude) + 3 + col(Tool::OpenCode) + 3 + total_w).max(8);
    let mut out = Vec::new();
    let mut head = Cells::new();
    head.push(title.to_string(), theme.section());
    head.pad_to(2 + name_w);
    for (i, tool) in tools.into_iter().enumerate() {
        if i > 0 {
            head.gap(3);
        }
        let mut label = Cells::new();
        label.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
        label.push(tool.name(), theme.faint());
        let w = label.width();
        head.gap(col(tool).saturating_sub(w));
        head.spans(label.into_spans());
    }
    head.gap(3);
    head.cell_right("total", total_w, theme.faint());
    out.push(head.line(width, Style::default()));
    if rows.is_empty() {
        out.push(Line::from(ratatui::text::Span::styled("  nothing today", theme.faint())));
        return out;
    }
    for (i, (name, by, style)) in rows.iter().enumerate() {
        let last = i + 1 == rows.len();
        if last && rule_before_last && rows.len() > 1 {
            if let Some(more) = &more {
                let mut line = Cells::new();
                line.push(format!("  {more}"), theme.faint());
                out.push(line.line(width, Style::default()));
            }
            let mut rule = Cells::new();
            rule.push("  ", Style::default());
            rule.push("─".repeat(width.saturating_sub(2)), theme.rule());
            out.push(rule.line(width, Style::default()));
        }
        let mut cells = Cells::new();
        cells.push("  ", Style::default());
        cells.cell(&crate::fmt::truncate(name, name_w), name_w, *style);
        for (j, tool) in tools.into_iter().enumerate() {
            if j > 0 {
                cells.gap(3);
            }
            match by.get(&tool).filter(|t| t.replies > 0) {
                Some(tally) => {
                    cells.cell_right(&tokens(tally.tokens), tok_w, theme.text2());
                    cells.push(" ", Style::default());
                    cells.cell_right(&money(tool, tally).0, cost_of(tool), theme.muted());
                }
                None => {
                    cells.cell_right("—", tok_w, theme.faint());
                    cells.gap(1 + cost_of(tool));
                }
            }
        }
        cells.gap(3);
        let total: u64 = by.values().map(|t| t.tokens).sum();
        if total == 0 {
            cells.cell_right("—", total_w, theme.faint());
        } else {
            cells.cell_right(&tokens(total), total_w, theme.strong());
        }
        out.push(cells.line(width, Style::default()));
    }
    out
}

/// This month so far and the whole of the last, in the same columns.
fn months(summary: &Summary, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let (this, last) = &summary.month_names;
    let rows = vec![
        (format!("{this} to the {}", ordinal(summary.day_of_month)), summary.this_month.clone(), theme.text()),
        (last.clone(), summary.last_month.clone(), theme.text2()),
    ];
    table("by month", &rows, None, false, theme, width)
}

/// `1st`, `2nd`, `3rd`, `8th`, `21st`.
fn ordinal(day: u32) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
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
        if tool == Tool::Claude {
            match app.limits.as_ref().map(|p| p.short()).filter(|s| !s.is_empty()) {
                Some(short) => cells.push(format!(" · {short}"), theme.faint()),
                None => match summary.window {
                    Some(w) => cells.push(format!(" · 5h {} left", crate::fmt::dur((w.end - now).max(0) as f64)), theme.faint()),
                    None => &mut cells,
                },
            };
        }
    }
    cells.line(width, Style::default())
}

/// View 7's list, over its sessions: today for each program in a line, Claude Code's window with it.
///
/// ```text
/// ✻ 287M today · ≈ $191 · 5h 1h26m
/// ▣ 54M today · $0.86
/// ```
pub fn sidebar_lines(app: &App, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    if !app.usage.read {
        return Vec::new();
    }
    let now = app.now();
    let summary = app.usage.summary(now, app.time.offset_s(now));
    let mut out = Vec::new();
    for tool in [Tool::Claude, Tool::OpenCode] {
        let Some(tally) = summary.today.get(&tool).copied().filter(|t| t.replies > 0) else { continue };
        let mut cells = Cells::new();
        cells.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
        cells.push(tokens(tally.tokens), theme.text2());
        cells.push(" today · ", theme.faint());
        cells.push(money(tool, &tally).0, theme.muted());
        if tool == Tool::Claude {
            limit_glance(&mut cells, app, &summary, now, theme);
        }
        out.push(cells.line(width, Style::default()));
    }
    out
}

/// Today's tokens of each conversation, for the list's rows.
pub fn by_session(app: &App) -> std::collections::HashMap<String, u64> {
    if !app.usage.read {
        return std::collections::HashMap::new();
    }
    let now = app.now();
    app.usage.by_session(now, app.time.offset_s(now)).into_iter().map(|(id, t)| (id, t.tokens)).collect()
}

/// The AI at a glance, for the shelf on every view: `✻ 287M · 5h 1h26m   ▣ 54M` — today's tokens
/// of each program that used any, and Claude Code's window left. `None` before the first read or
/// on a day nothing was used.
pub fn glance(app: &App, theme: &Theme) -> Option<Cells> {
    if !app.usage.read {
        return None;
    }
    let now = app.now();
    let summary = app.usage.summary(now, app.time.offset_s(now));
    let mut cells = Cells::new();
    for tool in [Tool::Claude, Tool::OpenCode] {
        let Some(tally) = summary.today.get(&tool).copied().filter(|t| t.replies > 0) else { continue };
        if cells.width() > 0 {
            cells.push("   ", Style::default());
        }
        cells.push(format!("{} ", tool.glyph()), tool_style(tool, theme));
        cells.push(tokens(tally.tokens), theme.text2());
        if tool == Tool::Claude {
            limit_glance(&mut cells, app, &summary, now, theme);
        }
    }
    (cells.width() > 0).then_some(cells)
}

/// After Claude Code's tokens in a short line: the plan's session and week, coloured by how near
/// their limits — else the window worked out here.
fn limit_glance(cells: &mut Cells, app: &App, summary: &Summary, now: i64, theme: &Theme) {
    match app.limits.as_ref().filter(|p| !p.limits.is_empty()) {
        Some(plan) => {
            for (i, label) in ["session", "this week"].into_iter().enumerate() {
                if let Some(limit) = plan.limits.iter().find(|l| l.label == label) {
                    let sev = crate::severity::node(Some(limit.percent));
                    cells.push(if i == 0 { " · session " } else { " · week " }, theme.faint());
                    cells.push(format!("{:.0}%", limit.percent), if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.muted() });
                }
            }
        }
        None => {
            if let Some(w) = summary.window {
                cells.push(" · 5h ", theme.faint());
                cells.push(crate::fmt::dur((w.end - now).max(0) as f64), theme.muted());
            }
        }
    }
}
