//! View 5: Claude Code's sessions — a bar of tabs, and the screen of the one on it, drawn from
//! its emulated terminal (`claude.rs`).
//!
//! Above them the header and the two band lines stay as on every view, and the line under the
//! band names the worst thing in the fleet right now — so a node going red is seen without
//! leaving the conversation.

use super::widgets::{rule, tone_spans, Cells};
use crate::app::App;
use crate::claude::{Mode, PaneState};
use crate::fmt;
use crate::insight::Insight;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let bar = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(Paragraph::new(session_bar(app, theme, area.width as usize)), bar);
    let area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    if area.height == 0 {
        return;
    }
    // Every PTY is sized to what a pane gets here.
    app.claude.want_size.set((area.height, area.width));
    let program = app.claude.command.first().cloned().unwrap_or_default();
    let Some(session) = app.claude.current() else {
        message(frame, theme, area, vec![Line::from(vec![
            Span::styled("no Claude session open · ", theme.muted()),
            Span::styled("⏎", theme.strong()),
            Span::styled(" starts one", theme.muted()),
        ])]);
        return;
    };
    match &session.pane.state {
        PaneState::Idle | PaneState::Starting => {
            message(frame, theme, area, vec![
                Line::from(vec![
                    Span::styled("starting ", theme.muted()),
                    Span::styled(program, theme.strong()),
                    Span::styled(" …", theme.muted()),
                ]),
            ]);
        }
        PaneState::Failed(why) => {
            message(frame, theme, area, vec![
                Line::from(Span::styled(why.clone(), theme.sev(Severity::Warn))),
                Line::from(""),
                Line::from(Span::styled(
                    "Claude Code runs here as it would in a terminal of its own, signed in with your Pro or Max plan — no API key.",
                    theme.muted(),
                )),
                Line::from(Span::styled(
                    "Install it, run `claude` once and sign in with /login, then press ⏎ here. CLAUDE_CMD chooses another command.",
                    theme.muted(),
                )),
            ]);
        }
        PaneState::Running => screen(frame, &session.pane, theme, area, true),
        PaneState::Exited(how) => {
            screen(frame, &session.pane, theme, area, false);
            let mut cells = Cells::new();
            cells.push(format!(" {program} — {how} "), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            cells.push(" ⏎ start it again · ctrl+\\ for the bar and the monitor ", theme.muted());
            let bottom = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            frame.render_widget(Paragraph::new(cells.line(area.width as usize, theme.selected())), bottom);
        }
    }
}

/// The sessions as tabs, like the header's: the one on screen lit, one that rang marked `●`,
/// one that ended marked `✕`. While a name is typed its tab is the input; after `ctrl+\` the
/// bar is lit, waiting for a command.
fn session_bar(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let sessions = &app.claude;
    let mut cells = Cells::new();
    for (index, session) in sessions.list.iter().enumerate() {
        let number = index + 1;
        let on_screen = index == sessions.active;
        if let (true, Mode::Naming(name)) = (on_screen, &sessions.mode) {
            cells.push(format!(" {number} "), theme.tab_active());
            cells.push(format!("{name}▏"), theme.keycap());
            cells.push(" ", theme.tab_active());
            cells.push(" ", Style::default());
            continue;
        }
        let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));
        let mark = if session.pane.attention { " ●" } else if ended { " ✕" } else { "" };
        let label = format!(" {number} {}{mark} ", fmt::truncate(&session.label(number), 22));
        let style = if on_screen {
            theme.tab_active()
        } else if session.pane.attention {
            theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)
        } else if ended {
            theme.faint()
        } else {
            theme.muted()
        };
        cells.push(label, style);
        cells.push(" ", Style::default());
    }
    match &sessions.mode {
        Mode::Bar if sessions.closing => {
            let name = sessions.current().map(|s| s.label(sessions.active + 1)).unwrap_or_default();
            cells.push(
                format!(" x again closes “{name}” — `claude --resume` finds the conversation later "),
                theme.sev(Severity::Warn).add_modifier(Modifier::BOLD),
            );
        }
        Mode::Bar => {
            cells.push(" which one? ", theme.strong());
        }
        Mode::Typing if sessions.list.len() < 2 => {
            cells.push(" ctrl+\\ then n opens another session", theme.faint());
        }
        Mode::Typing | Mode::Naming(_) => {}
    }
    let lit = matches!(sessions.mode, Mode::Bar);
    cells.line(width, if lit { theme.selected() } else { Style::default() })
}

/// A few lines in the middle of the pane.
fn message(frame: &mut Frame, theme: &Theme, area: Rect, lines: Vec<Line<'static>>) {
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 3;
    let height = (lines.len() as u16).min(area.height);
    let block = Rect::new(area.x + 2.min(area.width), top, area.width.saturating_sub(4), height);
    frame.render_widget(Paragraph::new(lines).style(theme.text()), block);
}

/// The emulated screen, cell by cell, runs of one style joined into one span.
fn screen(frame: &mut Frame, pane: &crate::claude::ClaudePane, theme: &Theme, area: Rect, live: bool) {
    let screen = pane.screen();
    let (rows, cols) = screen.size();
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows.min(area.height) as usize);
    for row in 0..rows.min(area.height) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut run = String::new();
        let mut run_style = Style::default();
        for col in 0..cols.min(area.width) {
            let Some(cell) = screen.cell(row, col) else {
                break;
            };
            // The second half of a wide character: the first half already drew it.
            if cell.is_wide_continuation() {
                continue;
            }
            let style = style_of(cell, live);
            if style != run_style && !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), run_style));
            }
            run_style = style;
            if cell.has_contents() {
                run.push_str(cell.contents());
            } else {
                run.push(' ');
            }
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, run_style));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(theme.text()), area);
    if live && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < area.height && col < area.width {
            frame.set_cursor_position((area.x + col, area.y + row));
        }
    }
}

fn color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

/// A cell's style as the program asked for it; a program that has ended is drawn dimmed.
fn style_of(cell: &vt100::Cell, live: bool) -> Style {
    let mut style = Style::default();
    if let Some(fg) = color(cell.fgcolor()) {
        style = style.fg(fg);
    }
    if let Some(bg) = color(cell.bgcolor()) {
        style = style.bg(bg);
    }
    for (on, modifier) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim() || !live, Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        if on {
            style = style.add_modifier(modifier);
        }
    }
    style
}

/// The line under the band on view 5: the worst thing in the fleet right now, or that there
/// is nothing — the monitor, in one line.
pub fn watch_line(insights: &[Insight], theme: &Theme, width: usize) -> Line<'static> {
    let problems: Vec<&Insight> = insights.iter().filter(|i| i.level.is_problem()).collect();
    let Some(worst) = problems.first() else {
        let quiet = insights.first().map_or_else(Vec::new, |i| tone_spans(&i.parts, theme));
        let mut title = vec![Span::styled("✔ ", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD))];
        title.extend(quiet.into_iter().map(|s| Span::styled(s.content, theme.muted())));
        return rule(width, title, theme);
    };
    let mut title = vec![
        Span::styled(format!("{} ", worst.level.glyph()), theme.sev(worst.level).add_modifier(Modifier::BOLD)),
        Span::styled(worst.label.clone(), theme.strong()),
        Span::raw("  "),
    ];
    title.extend(tone_spans(&worst.parts, theme));
    if problems.len() > 1 {
        title.push(Span::styled(format!("   ·  {} more on view 1", problems.len() - 1), theme.muted()));
    }
    rule(width, title, theme)
}
