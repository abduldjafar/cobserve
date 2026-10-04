//! View 5: Claude Code's sessions — a list of them beside the screen of the one on it (a bar of
//! tabs over it on a narrow terminal), drawn from its emulated terminal (`claude.rs`).
//!
//! Above them the header and the two band lines stay as on every view, and the line under the
//! band names the worst thing in the fleet right now — so a node going red is seen without
//! leaving the conversation.

use super::widgets::{rule, tone_spans, Cells};
use crate::app::{App, Hit};
use crate::claude::{Mode, PaneState, Session, Sessions};
use crate::fmt;
use crate::insight::Insight;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// From this width the sessions are a list beside the pane, like a terminal's tab list; below
/// it, a bar of tabs over the pane.
const SIDEBAR_FROM: u16 = 100;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let area = if area.width >= SIDEBAR_FROM {
        let side = (area.width * 22 / 100).clamp(26, 36);
        sidebar(frame, app, theme, Rect::new(area.x, area.y, side, area.height));
        let rule: Vec<Line<'static>> = (0..area.height).map(|_| Line::from(Span::styled("│", theme.rule()))).collect();
        frame.render_widget(Paragraph::new(rule), Rect::new(area.x + side, area.y, 1, area.height));
        Rect::new(area.x + side + 2, area.y, area.width.saturating_sub(side + 2), area.height)
    } else {
        let bar = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget(Paragraph::new(session_bar(app, theme, bar)), bar);
        Rect::new(area.x, area.y + 1, area.width, area.height - 1)
    };
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Every PTY is sized to what a pane gets here; the mouse needs to know where it is.
    app.claude.want_size.set((area.height, area.width));
    app.claude.pane_origin.set((area.y, area.x));
    app.viewport.hits.borrow_mut().push((area, Hit::Pane));

    let program = app.claude.command.first().cloned().unwrap_or_default();
    if let Mode::Opening(_) = app.claude.mode {
        message(frame, theme, area, vec![
            Line::from(Span::styled("a new session — where should it work?", theme.strong())),
            Line::from(Span::styled(
                "type a directory on the left (~ is your home) · ⏎ opens it there · esc cancels",
                theme.muted(),
            )),
        ]);
        return;
    }
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
                    Span::styled(format!(" in {} …", session.dir), theme.muted()),
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
            cells.push(" ⏎ start it again · ctrl+\\ then a number for another tab ", theme.muted());
            let bottom = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            frame.render_widget(Paragraph::new(cells.line(area.width as usize, theme.selected())), bottom);
        }
    }
}

/// A session's state at a glance: `●` it rang while not on screen, `✕` its program ended.
fn mark(session: &Session) -> &'static str {
    if session.pane.attention {
        "●"
    } else if matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_)) {
        "✕"
    } else {
        ""
    }
}

/// The sessions as a list, like a terminal's tabs: number, name — or what Claude says it is
/// on — and under it the directory it works in and its branch. A `+` opens another; the keys
/// are the footer's, as on every view.
fn sidebar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let sessions = &app.claude;
    let width = area.width as usize;
    let mut hits = app.viewport.hits.borrow_mut();
    let mut lines: Vec<Line<'static>> = Vec::new();

    let lit = sessions.mode == Mode::Bar;
    let mut title = Cells::new();
    title.push(" SESSIONS", theme.section());
    if lit {
        title.push("  which one?", theme.strong());
    }
    title.pad_to(width.saturating_sub(3));
    title.push(" + ", theme.keycap());
    lines.push(title.line(width, if lit { theme.selected() } else { Style::default() }));
    hits.push((Rect::new(area.x + area.width.saturating_sub(3), area.y, 3.min(area.width), 1), Hit::NewSession));
    lines.push(Line::from(""));

    for (index, session) in sessions.list.iter().enumerate() {
        let y = area.y + lines.len() as u16;
        let on_screen = index == sessions.active;
        let row = if on_screen { theme.selected() } else { Style::default() };
        let mut first = Cells::new();
        first.push(if on_screen { "▌" } else { " " }, theme.accent());
        first.push(format!("{} ", Sessions::number_of(index)), theme.faint());
        let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));
        first.push("✳ ", if ended { theme.faint() } else { theme.person() });
        let mark = mark(session);
        let room = width.saturating_sub(first.width() + 3);
        match (&sessions.mode, on_screen) {
            (Mode::Naming(name), true) => {
                first.push(format!("{}▏", fmt::truncate(name, room)), theme.keycap());
            }
            _ => {
                let style = if on_screen { theme.strong() } else if ended { theme.faint() } else { theme.text() };
                first.push(fmt::truncate(&session.label(), room), style);
            }
        }
        if !mark.is_empty() {
            first.pad_to(width.saturating_sub(2));
            let style = if mark == "●" { theme.sev(Severity::Warn).add_modifier(Modifier::BOLD) } else { theme.faint() };
            first.push(mark, style);
        }
        lines.push(first.line(width, row));

        // The directory first: it is short, and says which project; a long branch is cut.
        let mut second = Cells::new();
        second.push("    ", Style::default());
        second.push(session.dir_label(), theme.faint());
        if let Some(branch) = &session.branch {
            second.push(" ⎇ ", theme.faint());
            second.push(branch.clone(), theme.muted());
        }
        lines.push(second.line(width, row));
        hits.push((Rect::new(area.x, y, area.width, 2), Hit::Session(index)));

        if on_screen && sessions.closing {
            let mut warn = Cells::new();
            warn.push("    x again closes it", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            lines.push(warn.line(width, Style::default()));
        }
        lines.push(Line::from(""));
    }

    match &sessions.mode {
        Mode::Opening(dir) => {
            let mut head = Cells::new();
            head.push(" + new session, in:", theme.strong());
            lines.push(head.line(width, Style::default()));
            let mut input = Cells::new();
            input.push("   ", Style::default());
            // The end of a long path is the part that says where.
            let shown = dir.chars().rev().take(width.saturating_sub(5)).collect::<Vec<_>>().into_iter().rev().collect::<String>();
            input.push(format!("{shown}▏"), theme.keycap());
            lines.push(input.line(width, Style::default()));
            let mut keys = Cells::new();
            keys.push("   ⏎ open · esc cancel · ctrl+u clear", theme.faint());
            lines.push(keys.line(width, Style::default()));
        }
        _ if sessions.list.len() < crate::claude::MAX_SESSIONS => {
            hits.push((Rect::new(area.x, area.y + lines.len() as u16, area.width, 1), Hit::NewSession));
            let mut new = Cells::new();
            new.push(" + ", theme.keycap());
            new.push(" new session", theme.muted());
            lines.push(new.line(width, Style::default()));
        }
        _ => {}
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

/// On a narrow terminal: the sessions as tabs over the pane, like the header's.
fn session_bar(app: &App, theme: &Theme, area: Rect) -> Line<'static> {
    let sessions = &app.claude;
    let width = area.width as usize;
    let mut hits = app.viewport.hits.borrow_mut();
    let mut cells = Cells::new();
    for (index, session) in sessions.list.iter().enumerate() {
        let number = Sessions::number_of(index);
        let on_screen = index == sessions.active;
        let x = area.x + cells.width() as u16;
        if let (true, Mode::Naming(name)) = (on_screen, &sessions.mode) {
            cells.push(format!(" {number} "), theme.tab_active());
            cells.push(format!("{name}▏"), theme.keycap());
            cells.push(" ", theme.tab_active());
        } else {
            let mark = mark(session);
            let label = format!(" {number} {}{}{mark} ", fmt::truncate(&session.label(), 18), if mark.is_empty() { "" } else { " " });
            let style = if on_screen {
                theme.tab_active()
            } else if session.pane.attention {
                theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)
            } else {
                theme.muted()
            };
            cells.push(label, style);
        }
        hits.push((Rect::new(x, area.y, (area.x + cells.width() as u16).saturating_sub(x), 1), Hit::Session(index)));
        cells.push(" ", Style::default());
    }
    let x = area.x + cells.width() as u16;
    match &sessions.mode {
        Mode::Opening(dir) => {
            cells.push(" + in: ", theme.strong());
            cells.push(format!("{dir}▏"), theme.keycap());
            cells.push("  ⏎ open · esc cancel", theme.faint());
        }
        Mode::Bar if sessions.closing => {
            cells.push(" x again closes it ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
        Mode::Bar => {
            cells.push(" which one? ", theme.strong());
        }
        _ => {
            cells.push(" + ", theme.keycap());
            hits.push((Rect::new(x, area.y, 3, 1), Hit::NewSession));
        }
    }
    let lit = sessions.mode == Mode::Bar;
    cells.line(width, if lit { theme.selected() } else { Style::default() })
}

/// A few lines in the upper middle of the pane, wrapped to it.
fn message(frame: &mut Frame, theme: &Theme, area: Rect, lines: Vec<Line<'static>>) {
    let top = area.y + area.height / 4;
    let block = Rect::new(area.x + 2.min(area.width), top, area.width.saturating_sub(4), area.height - (top - area.y));
    frame.render_widget(Paragraph::new(lines).style(theme.text()).wrap(Wrap { trim: true }), block);
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
