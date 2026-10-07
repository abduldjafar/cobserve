//! View 5: the sessions — Claude Code, OpenCode, a shell — listed beside the screen of the one
//! on it (a bar of tabs over it on a narrow terminal), drawn from its emulated terminal
//! (`claude.rs`); and, while a new one is opened, the folder picker in the screen's place.
//!
//! Above them the header and the two band lines stay as on every view, and under the band a
//! card for every node says how it is — its memory and its CPU, the worst first — so a node
//! going red is seen without leaving the conversation.

use super::widgets::{fit, rule, thin_bar, Cells};
use crate::app::{App, Hit};
use crate::claude::{Kind, Mode, PaneState, PickRow, Picker, Session, Sessions, KEYED, MAX_SESSIONS};
use crate::fmt;
use crate::folders::Folder;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// From this width the sessions are a list beside the pane, like a terminal's tab list; below
/// it, a bar of tabs over the pane.
const SIDEBAR_FROM: u16 = 100;

/// How wide the list of sessions is, its margin included; none when the terminal is too narrow
/// for a list beside the pane.
pub fn sidebar_width(width: u16) -> Option<u16> {
    (width >= SIDEBAR_FROM).then(|| (width * 22 / 100).clamp(28, 38))
}

/// View 5 in the well under the shelf, `well` edge to edge: the list of sessions on the
/// chrome's surface from the left edge, the pane in the dark beside it.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, well: Rect, margin: u16) {
    if well.height == 0 || well.width == 0 {
        return;
    }
    let area = if let Some(side) = sidebar_width(well.width) {
        frame.render_widget(ratatui::widgets::Block::new().style(theme.surface()), Rect::new(well.x, well.y, side, well.height));
        sidebar(frame, app, theme, Rect::new(well.x + margin, well.y, side.saturating_sub(margin + 1), well.height));
        if !theme.paints_background() {
            let rule: Vec<Line<'static>> = (0..well.height).map(|_| Line::from(Span::styled("│", theme.rule()))).collect();
            frame.render_widget(Paragraph::new(rule), Rect::new(well.x + side, well.y, 1, well.height));
        }
        let x = well.x + side + 2;
        Rect::new(x, well.y + 1, (well.x + well.width).saturating_sub(x + margin), well.height.saturating_sub(1))
    } else {
        let bar = Rect::new(well.x, well.y, well.width, 1);
        frame.render_widget(ratatui::widgets::Block::new().style(theme.surface()), bar);
        let inset = Rect::new(well.x + margin, well.y, well.width.saturating_sub(2 * margin), 1);
        frame.render_widget(Paragraph::new(session_bar(app, theme, inset)), inset);
        Rect::new(well.x + margin, well.y + 1, well.width.saturating_sub(2 * margin), well.height - 1)
    };
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Every PTY is sized to what a pane gets here; the mouse needs to know where it is.
    app.claude.want_size.set((area.height, area.width));
    app.claude.pane_origin.set((area.y, area.x));
    if let Mode::Opening(choosing) = &app.claude.mode {
        picker(frame, app, theme, area, choosing);
        return;
    }
    // A query session is drawn by the monitor itself: there is no program's screen.
    if let Some(console) = app.claude.current().and_then(|s| s.console.as_deref()) {
        super::console::draw(frame, app, theme, area, console);
        return;
    }
    app.viewport.hits.borrow_mut().push((area, Hit::Pane));

    let Some(session) = app.claude.current() else {
        message(frame, theme, area, vec![Line::from(vec![
            Span::styled("no session open · ", theme.muted()),
            Span::styled("⏎", theme.strong()),
            Span::styled(" starts Claude · ", theme.muted()),
            Span::styled("ctrl+\\ n", theme.strong()),
            Span::styled(" opens Claude, OpenCode, a terminal or a query session", theme.muted()),
        ])]);
        return;
    };
    let program = app.claude.command_of(session.kind).first().cloned().unwrap_or_default();
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
            let mut lines = vec![Line::from(Span::styled(why.clone(), theme.sev(Severity::Warn))), Line::from("")];
            lines.extend(how_to_get(session.kind, &program).into_iter().map(|text| Line::from(Span::styled(text, theme.muted()))));
            message(frame, theme, area, lines);
        }
        PaneState::Running => {
            screen(frame, &session.pane, theme, area, true);
            scrolled_note(frame, &session.pane, theme, area);
        }
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

/// What a session of `kind` is, and how to get its program when it would not start.
fn how_to_get(kind: Kind, program: &str) -> [String; 2] {
    match kind {
        Kind::Claude => [
            "Claude Code runs here as it would in a terminal of its own, signed in with your Pro or Max plan — no API key.".into(),
            format!("Install it, run `claude` once and sign in with /login, then press ⏎ here. {} chooses another command.", kind.variable()),
        ],
        Kind::OpenCode => [
            "OpenCode runs here as it would in a terminal of its own, signed in the way `opencode auth login` set it up — ANTHROPIC_API_KEY is not passed on.".into(),
            format!(
                "Install it (curl -fsSL https://opencode.ai/install | bash), sign in once with `opencode auth login`, then press ⏎ here. {} chooses another command.",
                kind.variable()
            ),
        ],
        Kind::Terminal => [
            "A terminal runs your shell here, as a tab of its own would.".into(),
            format!("`{program}` would not start — {} chooses another shell; ⏎ tries again.", kind.variable()),
        ],
        Kind::Query => [
            "A query session runs SQL on a server of the fleet, read-only.".into(),
            format!("It needs the fleet's servers: {} names them.", kind.variable()),
        ],
    }
}

/// Over a shell scrolled back with the wheel: how far, and the way down.
fn scrolled_note(frame: &mut Frame, pane: &crate::claude::ClaudePane, theme: &Theme, area: Rect) {
    let back = pane.scrolled_back();
    if back == 0 {
        return;
    }
    let text = format!(" ↑ {} back · a key or the wheel comes down ", fmt::plural(back, "line", "lines"));
    let width = (fmt::width(&text) as u16).min(area.width);
    let at = Rect::new(area.x + area.width - width, area.y, width, 1);
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, theme.keycap()))), at);
}

/// A kind's mark, in its colour: Claude's orange, OpenCode's white, a shell's green.
fn kind_style(kind: Kind, theme: &Theme) -> Style {
    match kind {
        Kind::Claude => theme.claude(),
        Kind::OpenCode => theme.strong(),
        Kind::Terminal => theme.sev(Severity::Ok).add_modifier(Modifier::BOLD),
        Kind::Query => theme.accent().add_modifier(Modifier::BOLD),
    }
}

/// A session's state at a glance: `●` it rang while not on screen, `✕` its program ended, `↻`
/// kept from the last run, waiting to take its conversation up.
fn mark(session: &Session) -> &'static str {
    if session.waiting() {
        "↻"
    } else if session.pane.attention {
        "●"
    } else if matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_)) {
        "✕"
    } else {
        ""
    }
}

/// How much of each card the list has room for: two lines and a gap, two lines, or one — the lit
/// card keeping its second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Density {
    Full,
    Close,
    Line,
}

/// The sessions as cards, like a terminal's tab list: the mark of what it runs (Claude, OpenCode,
/// a shell, SQL), the name — the user's, what the program says it is on, or the folder — and the
/// number; under it the folder and its branch, or a query session's last run. The card on
/// screen is lit, and so is a new one while its folder is chosen, or the one a search is on.
/// The more there are the closer they sit — fifty make a line each — and the list follows the
/// lit card, saying how many are above and below it.
fn sidebar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let sessions = &app.claude;
    let width = area.width as usize;
    // The bar at the left of a card, and the card's own margin either side.
    let inner = width.saturating_sub(4);
    let picking = matches!(sessions.mode, Mode::Opening(_));
    let lit = sessions.mode == Mode::Bar;
    let finding = match &sessions.mode {
        Mode::Finding { query, at } => Some((query.clone(), *at)),
        _ => None,
    };
    // Those listed: every one, or those the search names.
    let listed: Vec<usize> = match &finding {
        Some((query, _)) => sessions.matching(query),
        None => (0..sessions.list.len()).collect(),
    };
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    let mut clickable: Vec<(Rect, Hit)> = Vec::new();

    let mut title = Cells::new();
    title.push("SESSIONS", theme.section());
    match &finding {
        Some((query, at)) => {
            title.push("  /", theme.accent());
            title.push(query.clone(), theme.strong());
            title.push("▏", theme.accent());
            let count = if listed.is_empty() { "none".to_string() } else { format!("{} of {}", (at + 1).min(listed.len()), listed.len()) };
            title.pad_to(width.saturating_sub(fmt::width(&count)));
            title.push(count, theme.faint());
        }
        None if lit => {
            title.push("  which one?", theme.strong());
        }
        None if !sessions.list.is_empty() => {
            title.push(format!("  {}", sessions.list.len()), theme.faint());
        }
        None => {}
    }
    lines.push(title.line(width, Style::default()));
    // Many sessions on several projects: grouped by project, a line each (`grouped_rows`).
    let grouped = finding.is_none() && sessions.list.len() >= GROUPED_FROM && sessions.groups().len() >= 2;
    if grouped {
        let mut find = Cells::new();
        find.push("⌕ ", theme.accent());
        find.push("find one", theme.muted());
        find.push("  ctrl+\\ /", theme.faint());
        lines.push(find.line(width, Style::default()));
    }
    lines.push(Line::from(""));

    // The card that is lit: the one a search is on, the new one being opened, the one on screen.
    let lit_card = match &finding {
        Some((_, at)) => listed.get(*at).copied(),
        None if picking => None,
        None => (!sessions.list.is_empty()).then_some(sessions.active),
    };
    // What is under the cards: the way to a new session and to past conversations.
    let foot = match () {
        _ if finding.is_some() => 1,
        _ if picking || sessions.list.len() < MAX_SESSIONS => 2,
        _ => 1,
    };
    let room = (area.height as usize).saturating_sub(lines.len() + foot + 1);
    let count = listed.len();
    let density = if 3 * count <= room {
        Density::Full
    } else if 2 * count <= room {
        Density::Close
    } else {
        Density::Line
    };
    // A line each, and still more than fit: a window that follows the lit card, a line at each
    // end for what is past it.
    let lit_at = lit_card.and_then(|card| listed.iter().position(|&i| i == card));
    let extra = usize::from(lit_at.is_some());
    let (first, shown) = if density == Density::Line && count + extra > room {
        let window = room.saturating_sub(extra + 2).max(1);
        let first = crate::app::scroll_into_view(sessions.list_scroll.get(), lit_at, window, count);
        (first, window)
    } else {
        (0, count)
    };
    sessions.list_scroll.set(first);

    if grouped {
        let top = area.y + lines.len() as u16;
        let (rows, hits) = grouped_rows(app, theme, area, top, room, lit_card, lit);
        lines.extend(rows);
        clickable.extend(hits);
        lines.push(Line::from(""));
    }
    if !grouped && first > 0 {
        let y = area.y + lines.len() as u16;
        lines.push(Line::from(Span::styled(format!("   ↑ {} more", first), theme.faint())));
        clickable.push((Rect::new(area.x, y, area.width, 1), Hit::Session(listed[first - 1])));
    }
    for &index in listed.iter().skip(first).take(if grouped { 0 } else { shown }) {
        let session = &sessions.list[index];
        let y = area.y + lines.len() as u16;
        let on_screen = lit_card == Some(index);
        let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));

        // Its mark and name; at the right, what it rang for and its number — a key to press
        // once ctrl+\ asks which, for the five a digit picks.
        let mut right = Cells::new();
        match mark(session) {
            "●" => right.push("● ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
            "✕" => right.push("✕ ", theme.faint()),
            "↻" => right.push("↻ ", theme.accent()),
            _ => &mut right,
        };
        let number = Sessions::number_of(index);
        if lit && index < KEYED {
            right.push(format!(" {number} "), theme.keycap());
        } else {
            right.push(number.to_string(), theme.faint());
        }
        let mut first_line = Cells::new();
        first_line.push(format!("{}  ", session.kind.glyph()), if ended { theme.faint() } else { kind_style(session.kind, theme) });
        let name_room = inner.saturating_sub(first_line.width() + right.width() + 1);
        match &sessions.mode {
            Mode::Naming(name) if index == sessions.active => {
                first_line.push(format!("{}▏", fmt::truncate(name, name_room)), theme.keycap());
            }
            _ => {
                let style = if on_screen { theme.strong() } else if ended || session.waiting() { theme.muted() } else { theme.text() };
                first_line.push(fmt::truncate(&session.label(), name_room), style);
            }
        }
        first_line.pad_to(inner.saturating_sub(right.width()));
        first_line.spans(right.into_spans());
        lines.push(card_row(first_line, width, on_screen, theme));

        // Where it works — or, after one x, what a second does.
        let two_lines = density != Density::Line || on_screen;
        if two_lines {
            let mut second = Cells::new();
            second.push("   ", Style::default());
            if index == sessions.active && sessions.closing {
                second.push("x again closes it", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
            } else if let Some(console) = &session.console {
                console_line(&mut second, console, theme);
            } else {
                place(&mut second, &session.dir, session.branch.as_deref(), inner, theme);
            }
            lines.push(card_row(second, width, on_screen, theme));
        }
        if density == Density::Full {
            lines.push(Line::from(""));
        }
        clickable.push((Rect::new(area.x, y, area.width, 1 + u16::from(two_lines)), Hit::Session(index)));
    }
    if !grouped && first + shown < count {
        let y = area.y + lines.len() as u16;
        lines.push(Line::from(Span::styled(format!("   ↓ {} more", count - first - shown), theme.faint())));
        clickable.push((Rect::new(area.x, y, area.width, 1), Hit::Session(listed[first + shown])));
    }
    if !grouped && density != Density::Full && count > 0 {
        lines.push(Line::from(""));
    }

    if finding.is_some() {
        let mut note = Cells::new();
        note.push(
            if listed.is_empty() { "   no session is called that" } else { "   ⏎ goes there · esc back" },
            theme.faint(),
        );
        lines.push(note.line(width, Style::default()));
    } else if picking || sessions.list.len() < MAX_SESSIONS {
        // A new session, and one that takes a conversation up again, at the end of the list.
        let y = area.y + lines.len() as u16;
        let mut first_line = Cells::new();
        first_line.push("+  ", theme.accent().add_modifier(Modifier::BOLD));
        first_line.push("new session", if picking { theme.strong() } else { theme.text2() });
        lines.push(card_row(first_line, width, picking, theme));
        if picking {
            let mut second = Cells::new();
            let query = matches!(&sessions.mode, Mode::Opening(p) if p.kind == Kind::Query);
            second.push(if query { "   choose its server →" } else { "   choose its folder →" }, theme.faint());
            lines.push(card_row(second, width, true, theme));
        }
        clickable.push((Rect::new(area.x, y, area.width, if picking { 2 } else { 1 }), Hit::NewSession));
        if !picking {
            let y = area.y + lines.len() as u16;
            let mut resume = Cells::new();
            resume.push("↻  ", theme.accent());
            resume.push("past conversations", theme.muted());
            lines.push(card_row(resume, width, false, theme));
            clickable.push((Rect::new(area.x, y, area.width, 1), Hit::Resume));
        }
    } else {
        let mut full = Cells::new();
        full.push(format!("   {MAX_SESSIONS} of {MAX_SESSIONS} · x closes one"), theme.faint());
        lines.push(full.line(width, Style::default()));
    }

    let bottom = area.y + area.height;
    app.viewport.hits.borrow_mut().extend(clickable.into_iter().filter(|(rect, _)| rect.y < bottom));
    frame.render_widget(Paragraph::new(lines), area);
}

/// From this many sessions, on more than one project, the list groups them.
const GROUPED_FROM: usize = 6;

/// The sessions grouped by project: a project's name over its sessions — its branch with it when
/// they share one, and how many rang — then a line a session: what it runs, its name, its mark and
/// its number; the one on screen also says where it works. More than fit: a window that follows
/// the lit one, a line at each end for how many are past it.
#[allow(clippy::too_many_arguments)]
fn grouped_rows(app: &App, theme: &Theme, area: Rect, top: u16, room: usize, lit_card: Option<usize>, asking: bool) -> (Vec<Line<'static>>, Vec<(Rect, Hit)>) {
    let sessions = &app.claude;
    let width = area.width as usize;
    let inner = width.saturating_sub(4);
    // Every line, and the session it is of, before the window is cut.
    let mut all: Vec<(Line<'static>, Option<usize>)> = Vec::new();
    for (g, (project, members)) in sessions.groups().into_iter().enumerate() {
        if g > 0 {
            all.push((Line::from(""), None));
        }
        let mut head = Cells::new();
        head.push("  ", Style::default());
        head.push(project, theme.section());
        head.push(format!(" {}", members.len()), theme.faint());
        // Who rang first — the branch is the first to go when the list is narrow.
        let calling = members.iter().filter(|&&i| sessions.list[i].pane.attention).count();
        if calling > 0 {
            head.push(format!("  ● {calling}"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
        let branches: Vec<Option<&str>> = members.iter().map(|&i| sessions.list[i].branch.as_deref()).collect();
        let shared = branches.first().copied().flatten().filter(|b| branches.iter().all(|x| *x == Some(*b)));
        if let Some(branch) = shared {
            head.push(format!("  ⎇ {branch}"), theme.faint());
        }
        all.push((head.line(width, Style::default()), None));
        for index in members {
            let session = &sessions.list[index];
            let on_screen = lit_card == Some(index);
            let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));
            let mut right = Cells::new();
            match mark(session) {
                "●" => right.push("● ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
                "✕" => right.push("✕ ", theme.faint()),
                "↻" => right.push("↻ ", theme.accent()),
                _ => &mut right,
            };
            let number = Sessions::number_of(index);
            if asking && index < KEYED {
                right.push(format!(" {number} "), theme.keycap());
            } else {
                right.push(number.to_string(), theme.faint());
            }
            let mut line = Cells::new();
            line.push(format!("{}  ", session.kind.glyph()), if ended { theme.faint() } else { kind_style(session.kind, theme) });
            let name_room = inner.saturating_sub(line.width() + right.width() + 1);
            match &sessions.mode {
                Mode::Naming(name) if index == sessions.active => {
                    line.push(format!("{}▏", fmt::truncate(name, name_room)), theme.keycap());
                }
                _ => {
                    let style = if on_screen { theme.strong() } else if ended || session.waiting() { theme.muted() } else { theme.text() };
                    line.push(fmt::truncate(&session.label(), name_room), style);
                }
            }
            line.pad_to(inner.saturating_sub(right.width()));
            line.spans(right.into_spans());
            all.push((card_row(line, width, on_screen, theme), Some(index)));
            if on_screen {
                let mut second = Cells::new();
                second.push("  ", Style::default());
                if index == sessions.active && sessions.closing {
                    second.push("x again closes it", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
                } else if let Some(console) = &session.console {
                    console_line(&mut second, console, theme);
                } else {
                    place(&mut second, &session.dir, session.branch.as_deref(), inner, theme);
                }
                all.push((card_row(second, width, true, theme), Some(index)));
            }
        }
    }

    // The window: everything when it fits, else what follows the lit session.
    let lit_at = lit_card.and_then(|card| all.iter().rposition(|(_, i)| *i == Some(card)));
    let (first, shown) = if all.len() > room {
        let window = room.saturating_sub(2).max(1);
        (crate::app::scroll_into_view(sessions.list_scroll.get(), lit_at, window, all.len()), window)
    } else {
        (0, all.len())
    };
    sessions.list_scroll.set(first);
    let count = |range: &[(Line<'static>, Option<usize>)]| range.iter().filter_map(|(_, i)| *i).collect::<std::collections::BTreeSet<usize>>().len();
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    let mut y = top;
    if first > 0 {
        lines.push(Line::from(Span::styled(format!("   ↑ {} more", count(&all[..first])), theme.faint())));
        if let Some(index) = all[..first].iter().rev().find_map(|(_, i)| *i) {
            hits.push((Rect::new(area.x, y, area.width, 1), Hit::Session(index)));
        }
        y += 1;
    }
    let end = (first + shown).min(all.len());
    for (line, index) in all[first..end].iter().cloned() {
        if let Some(index) = index {
            hits.push((Rect::new(area.x, y, area.width, 1), Hit::Session(index)));
        }
        lines.push(line);
        y += 1;
    }
    if end < all.len() {
        lines.push(Line::from(Span::styled(format!("   ↓ {} more", count(&all[end..])), theme.faint())));
        if let Some(index) = all[end..].iter().find_map(|(_, i)| *i) {
            hits.push((Rect::new(area.x, y, area.width, 1), Hit::Session(index)));
        }
    }
    (lines, hits)
}

/// A line of a card: on a raised surface with a bar of the accent at its left, for the card on
/// screen; set in by as much for the others, so nothing moves when the light does.
fn card_row(inner: Cells, width: usize, lit: bool, theme: &Theme) -> Line<'static> {
    let body = width.saturating_sub(4);
    if !lit {
        let mut cells = Cells::new();
        cells.push("  ", Style::default());
        cells.spans(fit(inner.into_spans(), body, false));
        return cells.line(width, Style::default());
    }
    let fill = theme.raised();
    let mut spans = vec![Span::styled("▎", theme.accent().patch(fill)), Span::styled(" ", fill)];
    spans.extend(fit(inner.into_spans(), body, true).into_iter().map(|span| {
        let style = fill.patch(span.style);
        Span::styled(span.content, style)
    }));
    spans.push(Span::styled("  ", fill));
    Line::from(spans)
}

/// A query session's second line: its run or its helper under way, how the last went, else what
/// its SQL is.
fn console_line(cells: &mut Cells, console: &crate::console::Console, theme: &Theme) {
    use crate::console::RunState;
    match &console.state {
        RunState::Running { .. } => {
            cells.push("◐ running…", theme.accent());
        }
        _ if console.asking() => {
            cells.push(format!("{} writing…", console.assistant.glyph()), theme.claude());
        }
        RunState::Failed { error, .. } => {
            cells.push(format!("✖ {error}"), theme.sev(Severity::Crit));
        }
        _ => match &console.answer {
            Some(answer) if !answer.columns.is_empty() => {
                cells.push(format!("✔ {}", fmt::plural(answer.rows.len(), "row", "rows")), theme.muted());
            }
            _ => {
                let sql = console.sql();
                let what = if sql.trim().is_empty() { "read-only SQL".to_string() } else { crate::sqltext::summary(&sql).label() };
                cells.push(what, theme.faint());
            }
        },
    }
}

/// Where a session works, in what is left of `room`: its folder — cut from the left, the end
/// of a path being the part that says where — and its branch. When both do not fit, the
/// folder keeps two thirds of the room, or its own name if that is more.
fn place(cells: &mut Cells, dir: &str, branch: Option<&str>, room: usize, theme: &Theme) {
    let room = room.saturating_sub(cells.width());
    let branch = branch.map(|b| format!("  ⎇ {b}"));
    let name = dir.trim_end_matches('/').rsplit('/').next().unwrap_or(dir);
    let (dir_width, branch_width) = (fmt::width(dir), branch.as_deref().map_or(0, fmt::width));
    let dir_room = if dir_width + branch_width <= room {
        dir_width
    } else {
        dir_width.min((room * 2 / 3).max(fmt::width(name) + 2)).min(room)
    };
    cells.push(shorten_left(dir, dir_room), theme.muted());
    if let Some(branch) = branch {
        cells.push(branch, theme.faint());
    }
}

/// A path in `max` cells: whole, or its longest tail that starts at a `/`, after a `…`.
fn shorten_left(path: &str, max: usize) -> String {
    if fmt::width(path) <= max {
        return path.to_string();
    }
    for (at, _) in path.match_indices('/') {
        let tail = &path[at..];
        if fmt::width(tail) < max {
            return format!("…{tail}");
        }
    }
    fmt::truncate(path.rsplit('/').next().unwrap_or(path), max)
}

/// On a narrow terminal: the sessions as tabs over the pane, like the header's — as many as fit
/// round the lit one, with how many more there are either side.
fn session_bar(app: &App, theme: &Theme, area: Rect) -> Line<'static> {
    let sessions = &app.claude;
    let width = area.width as usize;
    let finding = match &sessions.mode {
        Mode::Finding { query, at } => Some((query.clone(), *at)),
        _ => None,
    };
    let listed: Vec<usize> = match &finding {
        Some((query, _)) => sessions.matching(query),
        None => (0..sessions.list.len()).collect(),
    };
    let lit_card = match &finding {
        Some((_, at)) => listed.get(*at).copied(),
        None if matches!(sessions.mode, Mode::Opening(_)) => None,
        None => Some(sessions.active),
    };

    // Each tab, drawn on its own.
    let tabs: Vec<(usize, Cells)> = listed
        .iter()
        .map(|&index| {
            let session = &sessions.list[index];
            let number = Sessions::number_of(index);
            let on_screen = lit_card == Some(index);
            let mut cells = Cells::new();
            if let (true, Mode::Naming(name)) = (on_screen, &sessions.mode) {
                cells.push(format!(" {number} "), theme.tab_active());
                cells.push(format!("{name}▏"), theme.keycap());
                cells.push(" ", theme.tab_active());
            } else {
                let mark = mark(session);
                let glyph = session.kind.glyph();
                let label = format!(" {number} {glyph} {}{}{mark} ", fmt::truncate(&session.label(), 18), if mark.is_empty() { "" } else { " " });
                let style = if on_screen {
                    theme.tab_active()
                } else if session.pane.attention {
                    theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)
                } else {
                    theme.muted()
                };
                cells.push(label, style);
            }
            (index, cells)
        })
        .collect();

    // What the bar ends with: what it is doing, or the way to a new session.
    let mut tail = Cells::new();
    let mut tail_hit = false;
    match (&sessions.mode, &finding) {
        (_, Some((query, _))) => {
            tail.push(format!(" /{query}▏ "), theme.keycap());
            if listed.is_empty() {
                tail.push("  no session is called that", theme.faint());
            }
        }
        (Mode::Opening(picker), _) => {
            tail.push(" + new session ", theme.tab_active());
            tail.push(if picker.kind == Kind::Query { "  choose its server below" } else { "  choose its folder below" }, theme.faint());
        }
        (Mode::Bar, _) if sessions.closing => {
            tail.push(" x again closes it ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
        (Mode::Bar, _) => {
            tail.push(" which one? ", theme.strong());
        }
        _ => {
            tail.push(" + ", theme.keycap());
            tail_hit = true;
        }
    }

    // The tabs that fit round the lit one, leaving room for the tail and what is either side.
    let gap = |n: usize| if n == 0 { 0 } else { 6 };
    let room = width.saturating_sub(tail.width() + 1);
    let centre = lit_card.and_then(|card| tabs.iter().position(|(i, _)| *i == card)).unwrap_or(tabs.len().saturating_sub(1));
    let (mut start, mut end) = (centre.min(tabs.len()), (centre + 1).min(tabs.len()));
    let used = |start: usize, end: usize| -> usize {
        tabs[start..end].iter().map(|(_, c)| c.width() + 1).sum::<usize>() + gap(start) + gap(tabs.len() - end)
    };
    loop {
        let mut grew = false;
        if end < tabs.len() && used(start, end + 1) <= room {
            end += 1;
            grew = true;
        }
        if start > 0 && used(start - 1, end) <= room {
            start -= 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }

    let mut hits = app.viewport.hits.borrow_mut();
    let mut cells = Cells::new();
    if start > 0 {
        let x = area.x + cells.width() as u16;
        cells.push(format!("‹ {start}"), theme.faint());
        cells.pad_to(6);
        hits.push((Rect::new(x, area.y, 5, 1), Hit::Session(tabs[start - 1].0)));
    }
    for (index, tab) in tabs.into_iter().skip(start).take(end - start) {
        let x = area.x + cells.width() as u16;
        let tab_width = tab.width() as u16;
        cells.spans(tab.into_spans());
        hits.push((Rect::new(x, area.y, tab_width, 1), Hit::Session(index)));
        cells.push(" ", Style::default());
    }
    if end < listed.len() {
        let x = area.x + cells.width() as u16;
        cells.push(format!("{} ›", listed.len() - end), theme.faint());
        cells.push(" ", Style::default());
        hits.push((Rect::new(x, area.y, 5, 1), Hit::Session(listed[end])));
    }
    let x = area.x + cells.width() as u16;
    if tail_hit {
        hits.push((Rect::new(x, area.y, 3, 1), Hit::NewSession));
    }
    cells.spans(tail.into_spans());
    let lit = sessions.mode == Mode::Bar;
    cells.line(width, if lit { theme.selected() } else { Style::default() })
}

/// A new session's folder, chosen as in a file explorer: a search box, the way back up, and the
/// folders here. A click on a folder goes into it; ⏎, or a click on the first row, opens the
/// session there.
fn picker(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, picker: &Picker) {
    let width = area.width as usize;
    let mut hits = app.viewport.hits.borrow_mut();
    let mut lines: Vec<Line<'static>> = Vec::new();

    // What this is, and the way out.
    let mut head = Cells::new();
    head.push(format!("{} ", picker.kind.glyph()), kind_style(picker.kind, theme));
    let what = match picker.kind {
        Kind::Terminal => "New terminal".to_string(),
        Kind::Query => "New query session".to_string(),
        kind => format!("New {} session", kind.title()),
    };
    head.push(what, theme.strong());
    head.push(
        match picker.kind {
            Kind::Query => "  ·  which server should it run on?",
            _ if picker.everywhere => "  ·  take a conversation up, from any folder",
            _ => "  ·  where should it work?",
        },
        theme.muted(),
    );
    let cancel = " ✕ cancel ";
    let at = width.saturating_sub(fmt::width(cancel));
    head.pad_to(at);
    head.push(cancel, theme.keycap());
    lines.push(head.line(width, Style::default()));
    hits.push((Rect::new(area.x + at as u16, area.y, fmt::width(cancel) as u16, 1), Hit::Cancel));
    lines.push(Line::from(""));

    // What it will run, a click to change.
    let y = area.y + lines.len() as u16;
    let mut kinds = Cells::new();
    kinds.push("run   ", theme.muted());
    for kind in Kind::ALL {
        let chip = format!(" {} {} ", kind.glyph(), kind.title());
        let x = area.x + kinds.width() as u16;
        hits.push((Rect::new(x, y, fmt::width(&chip) as u16, 1), Hit::Kind(kind)));
        kinds.push(chip, if kind == picker.kind { theme.tab_active() } else { theme.text2() });
        kinds.push("  ", Style::default());
    }
    kinds.push("  shift+tab switches", theme.faint());
    lines.push(kinds.line(width, Style::default()));
    lines.push(Line::from(""));

    if picker.everywhere {
        let mut back = Cells::new();
        back.push("←  ", theme.accent());
        back.push("back to the folders", theme.muted());
        if picker.looking_for_conversations() {
            back.push("   looking…", theme.faint());
        }
        lines.push(back.line(width, Style::default()));
        lines.push(Line::from(""));
        return rows_and_note(frame, app, theme, area, picker, lines, hits);
    }

    // The search, on a raised field of its own; what it found so far on its right.
    let inner = width.saturating_sub(4);
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    if picker.kind == Kind::Query {
        let shown = picker.servers_shown().len();
        let status = count(shown, "server", "servers");
        let mut field = Cells::new();
        field.push("  ⌕  ", theme.accent());
        if picker.query.is_empty() {
            field.push("▏", theme.accent());
            field.push("search the servers", theme.faint());
        } else {
            field.push(picker.query.clone(), theme.strong());
            field.push("▏", theme.accent());
        }
        field.pad_to(inner.saturating_sub(fmt::width(&status)));
        field.push(format!("{status}  "), theme.faint());
        lines.push(if theme.paints_background() { raised_line(field, width, theme) } else { boxed(field, width, theme) });
        lines.push(Line::from(""));
        let mut note = Cells::new();
        note.push("   it runs read-only, with a time and a row limit — nothing it does can change a table", theme.faint());
        lines.push(note.line(width, Style::default()));
        lines.push(Line::from(""));
        return rows_and_note(frame, app, theme, area, picker, lines, hits);
    }
    let status = if picker.looking {
        "looking…".to_string()
    } else if picker.searching() {
        count(picker.folders.len(), "found", "found")
    } else {
        count(picker.folders.len(), "folder", "folders")
    };
    let mut field = Cells::new();
    field.push("  ⌕  ", theme.accent());
    if picker.query.is_empty() {
        field.push("▏", theme.accent());
        field.push("search the folders below here, or type a path", theme.faint());
    } else {
        field.push(picker.query.clone(), theme.strong());
        field.push("▏", theme.accent());
    }
    field.pad_to(inner.saturating_sub(fmt::width(&status)));
    field.push(format!("{status}  "), theme.faint());
    lines.push(if theme.paints_background() { raised_line(field, width, theme) } else { boxed(field, width, theme) });
    lines.push(Line::from(""));

    // Where it is: every step of the path a click back up.
    let y = area.y + lines.len() as u16;
    let mut path = Cells::new();
    path.push(" ", Style::default());
    let crumbs = picker.crumbs();
    let tag = picker.branch.as_ref().map(|b| format!("⎇ {b} "));
    let room = width.saturating_sub(tag.as_deref().map_or(0, fmt::width) + 2);
    for index in crumbs_shown(&crumbs, room) {
        match index {
            Some(index) => {
                if path.width() > 1 {
                    path.push(" › ", theme.faint());
                }
                let (name, _) = &crumbs[index];
                let x = area.x + path.width() as u16;
                let last = index + 1 == crumbs.len();
                path.push(name.clone(), if last { theme.strong() } else { theme.accent() });
                hits.push((Rect::new(x, y, fmt::width(name) as u16, 1), Hit::Crumb(index)));
            }
            None => {
                path.push(" › …", theme.faint());
            }
        }
    }
    if let Some(tag) = tag {
        path.pad_to(width.saturating_sub(fmt::width(&tag)));
        path.push(tag, theme.muted());
    }
    lines.push(path.line(width, Style::default()));
    lines.push(Line::from(""));
    rows_and_note(frame, app, theme, area, picker, lines, hits);
}

/// A line on the raised surface, edge to edge of `width`.
fn raised_line(inner: Cells, width: usize, theme: &Theme) -> Line<'static> {
    let fill = theme.raised();
    Line::from(fit(inner.into_spans(), width, true).into_iter().map(|span| Span::styled(span.content, fill.patch(span.style))).collect::<Vec<_>>())
}

/// The picker's rows under what `lines` already holds, the cursor kept in sight, and a line at
/// the bottom for what to say about them.
fn rows_and_note(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    area: Rect,
    picker: &Picker,
    mut lines: Vec<Line<'static>>,
    mut hits: std::cell::RefMut<'_, Vec<(Rect, Hit)>>,
) {
    let width = area.width as usize;
    let rows = picker.rows();
    let room = (area.height as usize).saturating_sub(lines.len() + 1);
    let cursor = picker.cursor.min(rows.len().saturating_sub(1));
    let mut scroll = picker.scroll.get();
    if cursor < scroll {
        scroll = cursor;
    }
    if room > 0 && cursor >= scroll + room {
        scroll = cursor + 1 - room;
    }
    scroll = scroll.min(rows.len().saturating_sub(room));
    picker.scroll.set(scroll);
    for (index, row) in rows.iter().enumerate().skip(scroll).take(room) {
        let y = area.y + lines.len() as u16;
        lines.push(pick_line(*row, picker, index == cursor, width, app.now(), theme));
        hits.push((Rect::new(area.x, y, area.width, 1), Hit::Pick(*row)));
    }
    let note = if picker.kind == Kind::Query {
        if picker.servers.is_empty() {
            Some(("no server yet — the fleet is still being found".to_string(), theme.faint()))
        } else if rows.is_empty() {
            Some(("no server is called that".to_string(), theme.faint()))
        } else if rows.len() > scroll + room {
            Some((format!("↓ {} more", rows.len() - scroll - room), theme.faint()))
        } else {
            None
        }
    } else if picker.everywhere && rows.is_empty() && !picker.looking_for_conversations() {
        Some((format!("no {} conversation to take up", picker.kind.title()), theme.faint()))
    } else if picker.everywhere {
        None
    } else if let Some(error) = &picker.error {
        Some((error.clone(), theme.sev(Severity::Warn)))
    } else if picker.looking {
        None
    } else if picker.searching() && picker.folders.is_empty() {
        Some(("no folder below here has that in its name".to_string(), theme.faint()))
    } else if picker.folders.is_empty() {
        Some(("no folders in here — open the session here, or go up".to_string(), theme.faint()))
    } else if rows.len() > scroll + room {
        Some((format!("↓ {} more", rows.len() - scroll - room), theme.faint()))
    } else if picker.cut_short {
        Some(("that is as far as a search goes — go into a folder to look deeper".to_string(), theme.faint()))
    } else {
        None
    };
    if let Some((text, style)) = note {
        lines.push(Line::from(Span::styled(format!("   {text}"), style)));
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Which steps of the path fit in `room`: all of them, or the first, a `…` (`None`), and as
/// many of the last as there is room for.
fn crumbs_shown(crumbs: &[(String, std::path::PathBuf)], room: usize) -> Vec<Option<usize>> {
    let width = |i: usize| fmt::width(&crumbs[i].0) + 3;
    if (0..crumbs.len()).map(width).sum::<usize>() <= room {
        return (0..crumbs.len()).map(Some).collect();
    }
    let mut used = width(0) + 4;
    let mut tail = Vec::new();
    for index in (1..crumbs.len()).rev() {
        if used + width(index) > room && !tail.is_empty() {
            break;
        }
        used += width(index);
        tail.push(Some(index));
    }
    tail.reverse();
    let mut shown = vec![Some(0)];
    if tail.first().is_some_and(|first| *first != Some(1)) {
        shown.push(None);
    }
    shown.extend(tail);
    shown
}

/// Where no background is painted, the search field between brackets instead.
fn boxed(inner: Cells, width: usize, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("[ ", theme.border())];
    spans.extend(fit(inner.into_spans(), width.saturating_sub(4), true));
    spans.push(Span::styled(" ]", theme.border()));
    Line::from(spans)
}

/// One row of the picker: the first opens the session here, the next goes up, the rest are
/// folders — a repository's with its branch.
fn pick_line(row: PickRow, picker: &Picker, on: bool, width: usize, now: i64, theme: &Theme) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(if on { "▌" } else { " " }, theme.accent());
    let mut branch = None;
    match row {
        PickRow::Conversation(index) => {
            let Some(conversation) = picker.conversations_shown().get(index) else {
                return Line::from("");
            };
            let mut tag = String::new();
            if picker.everywhere {
                // The folder gives way to the title: its last part is what says where.
                tag.push_str(&shorten_left(&crate::pty::tilde(&conversation.dir), (width / 3).max(12)));
                tag.push_str(" · ");
            }
            tag.push_str(&fmt::ago(now - conversation.updated));
            if conversation.open {
                tag.push_str(" · open in another terminal");
            }
            let tag = format!("{tag} ");
            let room = width.saturating_sub(5 + fmt::width(&tag) + 2);
            cells.push(" ↻  ", theme.accent());
            cells.push(fmt::truncate(&conversation.title, room), theme.text());
            let at = width.saturating_sub(fmt::width(&tag));
            cells.pad_to(at);
            cells.push(tag, if conversation.open { theme.sev(Severity::Warn) } else { theme.muted() });
            return cells.line(width, if on { theme.selected() } else { Style::default() });
        }
        PickRow::Everywhere => {
            cells.push(" ↻  ", theme.muted());
            cells.push("conversations in every folder…", theme.text2());
        }
        PickRow::Server(index) => {
            let Some((name, answering)) = picker.servers.get(index) else {
                return Line::from("");
            };
            if *answering {
                cells.push(" ●  ", theme.sev(Severity::Ok));
                cells.push(name.clone(), theme.text());
            } else {
                cells.push(" ↯  ", theme.sev(Severity::Crit));
                cells.push(name.clone(), theme.muted());
                cells.push("  not answering", theme.faint());
            }
        }
        PickRow::Here => {
            cells.push(" ", Style::default());
            cells.push(" ⏎  Open the session here ", theme.keycap());
        }
        PickRow::Up => {
            cells.push(" ↰  ", theme.muted());
            cells.push("..", theme.text());
            if let Some(parent) = picker.dir.parent() {
                cells.push(format!("  up to {}", crate::pty::tilde(parent)), theme.faint());
            }
        }
        PickRow::Folder(index) => {
            let Some(folder) = picker.folders.get(index) else {
                return Line::from("");
            };
            cells.push(" ▸  ", if folder.branch.is_some() { theme.accent() } else { theme.faint() });
            lit(&mut cells, folder, theme);
            branch = folder.branch.clone();
        }
    }
    if let Some(branch) = branch {
        let tag = format!("⎇ {branch} ");
        let at = width.saturating_sub(fmt::width(&tag));
        if cells.width() + 2 <= at {
            cells.pad_to(at);
            cells.push(tag, theme.muted());
        }
    }
    cells.line(width, if on { theme.selected() } else { Style::default() })
}

/// A folder's text, what the search matched lit up and the path above its name dimmed.
fn lit(cells: &mut Cells, folder: &Folder, theme: &Theme) {
    let shown = folder.shown.as_str();
    let name_at = shown.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1);
    let (a, b) = folder
        .hit
        .filter(|&(a, b)| name_at <= a && a <= b && b <= shown.len() && shown.is_char_boundary(a) && shown.is_char_boundary(b))
        .unwrap_or((shown.len(), shown.len()));
    cells.push(&shown[..name_at], theme.muted());
    cells.push(&shown[name_at..a], theme.text());
    cells.push(&shown[a..b], theme.accent().add_modifier(Modifier::BOLD));
    cells.push(&shown[b..], theme.text());
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
    // Scrolled back, the cursor is somewhere below what is shown.
    if live && !screen.hide_cursor() && pane.scrolled_back() == 0 {
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


/// A node as view 5 shows it, above the sessions.
struct Glance {
    name: String,
    /// The name less the domain every node shares.
    label: String,
    /// The worst of its memory, its CPU and its lag — `Crit` when it does not answer.
    level: Severity,
    mem: Option<f64>,
    cpu: Option<f64>,
    /// Its replication lag, when that is a problem, and how bad.
    lag: Option<(String, Severity)>,
    /// Why it does not answer (`unreachable`, `no access`), and what it said.
    down: Option<(&'static str, Option<String>)>,
}

/// Every node, the worst first; within a level, view 1's order.
fn glances(app: &App) -> Vec<Glance> {
    let mut glances = app
        .with_view(|view| {
            let mut nodes: Vec<&crate::model::NodeView<'_>> = view.nodes.iter().collect();
            nodes.sort_by(|a, b| crate::model::compare_nodes(a, b, app.tree.sort));
            let names: Vec<&str> = nodes.iter().map(|n| n.node.name.as_str()).collect();
            // The domain every node shares says nothing in a space this small.
            let domain = crate::insight::shared_domain(&names);
            nodes
                .iter()
                .map(|node| {
                    let name = node.node.name.clone();
                    let lag = severity::lag(node.node.lag_s);
                    Glance {
                        label: domain.as_deref().and_then(|d| name.strip_suffix(d)).unwrap_or(&name).to_string(),
                        level: super::nodes::severity_of(node),
                        mem: node.mem_pct,
                        cpu: node.cpu_pct,
                        lag: lag.is_problem().then(|| (fmt::dur(node.node.lag_s as f64), lag)),
                        down: (!node.node.reachable).then(|| (node.node.down_word(), node.node.down_detail().map(str::to_string))),
                        name,
                    }
                })
                .collect::<Vec<Glance>>()
        })
        .unwrap_or_default();
    glances.sort_by_key(|glance| std::cmp::Reverse(glance.level));
    glances
}

/// A node's mark and the style of its name: `✖` or `▲` in its colour when it is in trouble,
/// a green `●` when it is not.
fn node_mark(level: Severity, theme: &Theme) -> (&'static str, Style, Style) {
    if level.is_problem() {
        (level.glyph(), theme.sev(level).add_modifier(Modifier::BOLD), theme.strong())
    } else {
        ("●", theme.sev(Severity::Ok), theme.text2())
    }
}

/// A share as a number, quiet while it is fine and in its colour, bold, when it is not.
fn share(value: Option<f64>, theme: &Theme) -> (String, Style) {
    let level = severity::node(value);
    let text = value.map_or_else(|| "—".to_string(), fmt::pct0);
    let style = if level.is_problem() { theme.sev(level).add_modifier(Modifier::BOLD) } else { theme.text2() };
    (text, style)
}

/// The cards' columns: the legend at the left, the room between two cards, and how narrow and
/// how wide a card may be.
const LEGEND: usize = 6;
const GAP: usize = 2;
const CARD_MIN: usize = 17;
const CARD_MAX: usize = 26;

/// Under the band on view 5, a card for every node — the worst first, then view 1's order —
/// with how it is, and its memory and its CPU as thin bars, each with its share:
///
/// ```text
///       ✖ clickhouse3          ▲ clickhouse7  lag 12s  ● clickhouse-bi        +6 more
/// mem   ━━━━━━━━━━━━━━━━  93%  ━━━━━━━━━━╺━━━━━━  60%  ━━━━━━━━━━━╺━━━━  67%  ≤ 42%
/// cpu   ━━━━━━━━━━━━━━━━  93%  ━━━━━━━╸━━━━━━━━━  44%  ━━━━━━━━━━━━╺━━━  72%  ≤ 29%
/// ```
///
/// The cards are all as wide as the widest of them needs — a name and its lag whole — and
/// wider when there is room, so their bars can be compared at a glance. As many as fit, then
/// one saying how many more and how high the rest go. A click on a card opens its node on
/// view 1.
pub fn fleet_strip(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height < 3 || area.width == 0 {
        return;
    }
    let width = area.width as usize;
    let glances = glances(app);
    let mut rows: [Cells; 3] = Default::default();
    for (row, legend) in rows.iter_mut().zip(["", "mem", "cpu"]) {
        row.push(format!(" {legend}"), theme.faint());
    }
    if glances.is_empty() {
        rows[1].pad_to(LEGEND);
        rows[1].push("waiting for the first snapshot…", theme.muted());
    }

    // How many cards, and how wide: from all of them down, until they fit.
    let room = width.saturating_sub(LEGEND);
    let (mut shown, mut card) = (glances.len(), CARD_MIN);
    while shown > 0 {
        let widest = glances[..shown].iter().map(card_need).max().unwrap_or(CARD_MIN);
        let more = if shown < glances.len() { GAP + more_need(&glances[shown..]) } else { 0 };
        if shown * (widest + GAP) - GAP + more <= room {
            card = ((room - more + GAP) / shown - GAP).clamp(widest, CARD_MAX.max(widest));
            break;
        }
        shown -= 1;
    }

    let mut hits = app.viewport.hits.borrow_mut();
    let mut listed = Vec::with_capacity(shown);
    for (index, glance) in glances.iter().take(shown).enumerate() {
        let at = LEGEND + index * (card + GAP);
        for row in rows.iter_mut() {
            row.pad_to(at);
        }
        card_name(&mut rows[0], glance, card, theme);
        match &glance.down {
            Some((word, detail)) => {
                let crit = theme.sev(Severity::Crit).add_modifier(Modifier::BOLD);
                rows[1].push(fmt::truncate(&format!("↯ {word}"), card), crit);
                rows[2].push(fmt::truncate(detail.as_deref().unwrap_or_default(), card), theme.muted());
            }
            None => {
                for (row, value) in [(1, glance.mem), (2, glance.cpu)] {
                    let (text, style) = share(value, theme);
                    rows[row].spans(thin_bar(value, card - 5, theme.bar_fill(severity::node(value)), theme));
                    rows[row].cell_right(&text, 5, style);
                }
            }
        }
        hits.push((Rect::new(area.x + at as u16, area.y, card as u16, 3), Hit::Node(listed.len())));
        listed.push(glance.name.clone());
    }
    let hidden = &glances[shown..];
    let at = if shown > 0 { LEGEND + shown * (card + GAP) } else { LEGEND };
    if !hidden.is_empty() && at + more_need(hidden) <= width {
        for row in rows.iter_mut() {
            row.pad_to(at);
        }
        let worst = hidden.iter().map(|glance| glance.level).max().unwrap_or(Severity::None);
        if worst.is_problem() {
            rows[0].push(format!("{} ", worst.glyph()), theme.sev(worst).add_modifier(Modifier::BOLD));
        }
        rows[0].push(format!("+{} more", hidden.len()), theme.text2());
        // How high the rest go, so what is not shown is not a mystery.
        let most = |of: fn(&Glance) -> Option<f64>| hidden.iter().filter_map(of).reduce(f64::max);
        for (row, value) in [(1, most(|glance| glance.mem)), (2, most(|glance| glance.cpu))] {
            if let Some(value) = value {
                rows[row].push(format!("≤ {}", fmt::pct0(value)), theme.muted());
            }
        }
        let rect = Rect::new(area.x + at as u16, area.y, more_need(hidden) as u16, 3);
        hits.push((rect, Hit::View(crate::app::View::Nodes)));
    }
    drop(hits);
    *app.viewport.listed_nodes.borrow_mut() = listed;

    for (y, row) in (area.y..).zip(rows) {
        frame.render_widget(Paragraph::new(row.line(width, Style::default())), Rect::new(area.x, y, area.width, 1));
    }
}

/// How wide a node's card has to be for its mark, its name and its lag to be whole.
fn card_need(glance: &Glance) -> usize {
    let tag = glance.lag.as_ref().map_or(0, |(lag, _)| fmt::width(&format!(" lag {lag}")));
    (2 + fmt::width(&glance.label) + tag).clamp(CARD_MIN, CARD_MAX)
}

/// How wide the card for the nodes not shown has to be: `▲ +3 more`, `≤ 100%`.
fn more_need(hidden: &[Glance]) -> usize {
    let mark = if hidden.iter().any(|glance| glance.level.is_problem()) { 2 } else { 0 };
    (mark + fmt::width(&format!("+{} more", hidden.len()))).max(fmt::width("≤ 100%"))
}

/// A card's first line: the node's mark and name and, when it is behind, its lag at the right
/// — as many words of the lag as fit beside the whole name, none rather than a cut one.
fn card_name(cells: &mut Cells, glance: &Glance, card: usize, theme: &Theme) {
    let (mark, mark_style, name_style) = node_mark(glance.level, theme);
    let room = card.saturating_sub(2);
    let name = fmt::width(&glance.label);
    let tag = glance.lag.as_ref().and_then(|(lag, sev)| {
        [format!("lag {lag}"), lag.clone()]
            .into_iter()
            .find(|tag| name + 1 + fmt::width(tag) <= room)
            .map(|tag| (tag, *sev))
    });
    let start = cells.width();
    cells.push(mark, mark_style).push(" ", Style::default()).push(fmt::truncate(&glance.label, room), name_style);
    if let Some((tag, sev)) = tag {
        cells.pad_to(start + card - fmt::width(&tag));
        cells.push(tag, theme.sev(sev).add_modifier(Modifier::BOLD));
    }
}

/// On a terminal too short for the cards the line under the band carries the fleet instead:
/// every node in a few words, the worst first — `✖ clickhouse3 mem 93% cpu 97%` — as many as
/// the width holds, then how many more. A click on one opens it on view 1.
pub fn fleet_line(app: &App, theme: &Theme, area: Rect) -> Line<'static> {
    let width = area.width as usize;
    let glances = glances(app);
    if glances.is_empty() {
        app.viewport.listed_nodes.borrow_mut().clear();
        return rule(width, vec![Span::styled("waiting for the first snapshot…", theme.muted())], theme);
    }
    let mut hits = app.viewport.hits.borrow_mut();
    let mut listed = Vec::with_capacity(glances.len());
    let separator = "   ";
    let more = |left: usize| format!("{separator}+{left} more");
    let mut cells = Cells::new();
    cells.push("─ ", theme.rule());
    for (index, glance) in glances.iter().enumerate() {
        let mut piece = Cells::new();
        let (mark, mark_style, name_style) = node_mark(glance.level, theme);
        piece.push(format!("{mark} "), mark_style).push(glance.label.clone(), name_style);
        match &glance.down {
            Some((word, _)) => {
                piece.push(format!(" {word}"), theme.sev(Severity::Crit));
            }
            None => {
                for (what, value) in [("mem", glance.mem), ("cpu", glance.cpu)] {
                    let (text, style) = share(value, theme);
                    piece.push(format!(" {what} "), theme.faint()).push(text, style);
                }
                if let Some((lag, sev)) = &glance.lag {
                    piece.push(" lag ", theme.faint()).push(lag.clone(), theme.sev(*sev).add_modifier(Modifier::BOLD));
                }
            }
        }
        // This one, and room after it to say how many are left if it is not the last.
        let left = glances.len() - index - 1;
        let gap = if index > 0 { fmt::width(separator) } else { 0 };
        let after = if left > 0 { fmt::width(&more(left)) } else { 0 };
        if cells.width() + gap + piece.width() + after + 2 > width {
            let x = area.x + cells.width() as u16;
            let rest = more(glances.len() - index);
            hits.push((Rect::new(x, area.y, fmt::width(&rest) as u16, 1), Hit::View(crate::app::View::Nodes)));
            cells.push(rest, theme.muted());
            break;
        }
        cells.push(if index > 0 { separator } else { "" }, Style::default());
        let x = area.x + cells.width() as u16;
        hits.push((Rect::new(x, area.y, piece.width() as u16, 1), Hit::Node(listed.len())));
        listed.push(glance.name.clone());
        cells.spans(piece.into_spans());
    }
    drop(hits);
    *app.viewport.listed_nodes.borrow_mut() = listed;
    cells.push(" ", Style::default());
    let rest = width.saturating_sub(cells.width());
    cells.push("─".repeat(rest), theme.rule());
    cells.line_unpadded(width)
}
