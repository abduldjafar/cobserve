//! View 5: the sessions — Claude Code, OpenCode, a shell — listed beside the screen of the one
//! on it (a bar of tabs over it on a narrow terminal), drawn from its emulated terminal
//! (`claude.rs`); and, while a new one is opened, the folder picker in the screen's place.
//!
//! Above them the header and the two band lines stay as on every view, and the line under the
//! band names the worst thing in the fleet right now — so a node going red is seen without
//! leaving the conversation.

use super::widgets::{fit, rule, tone_spans, Cells};
use crate::app::{App, Hit};
use crate::claude::{Kind, Mode, PaneState, PickRow, Picker, Session, Sessions, MAX_SESSIONS};
use crate::fmt;
use crate::folders::Folder;
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
    if let Mode::Opening(choosing) = &app.claude.mode {
        picker(frame, app, theme, area, choosing);
        return;
    }
    app.viewport.hits.borrow_mut().push((area, Hit::Pane));

    let Some(session) = app.claude.current() else {
        message(frame, theme, area, vec![Line::from(vec![
            Span::styled("no session open · ", theme.muted()),
            Span::styled("⏎", theme.strong()),
            Span::styled(" starts Claude · ", theme.muted()),
            Span::styled("ctrl+\\ n", theme.strong()),
            Span::styled(" opens Claude, OpenCode or a terminal", theme.muted()),
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
    }
}

/// A kind's mark, in its colour: Claude's orange, OpenCode's white, a shell's green.
fn kind_style(kind: Kind, theme: &Theme) -> Style {
    match kind {
        Kind::Claude => theme.claude(),
        Kind::OpenCode => theme.strong(),
        Kind::Terminal => theme.sev(Severity::Ok).add_modifier(Modifier::BOLD),
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

/// The sessions as cards, like a terminal's tab list: the mark of what it runs (Claude, OpenCode,
/// a shell), the name — the user's, what the program says it is on, or the folder — and the
/// number that picks it; under it the folder and its branch. The card on screen is framed,
/// and so is a new one while its folder is chosen.
fn sidebar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let sessions = &app.claude;
    // A column short of the rule beside it, so a frame never runs into it.
    let width = (area.width as usize).saturating_sub(1);
    let inner = width.saturating_sub(4);
    let mut hits = app.viewport.hits.borrow_mut();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let picking = matches!(sessions.mode, Mode::Opening(_));
    let lit = sessions.mode == Mode::Bar;

    let mut title = Cells::new();
    title.push(" SESSIONS", theme.section());
    if lit {
        title.push("  which one?", theme.strong());
    }
    lines.push(title.line(width, if lit { theme.selected() } else { Style::default() }));

    // The framed card: the session on screen, or the new one being opened.
    let framed = if picking { Some(sessions.list.len()) } else { (!sessions.list.is_empty()).then_some(sessions.active) };
    let edge = |index: usize| match framed {
        Some(f) if f == index => card_edge(width, true, theme),
        Some(f) if f + 1 == index => card_edge(width, false, theme),
        _ => Line::from(""),
    };

    for (index, session) in sessions.list.iter().enumerate() {
        lines.push(edge(index));
        let y = area.y + lines.len() as u16;
        let on_screen = framed == Some(index);
        let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));

        // Claude's mark and the name; at the right, what it rang for and its number, a key
        // to press once ctrl+\ asks which.
        let mut right = Cells::new();
        match mark(session) {
            "●" => right.push("● ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
            "✕" => right.push("✕ ", theme.faint()),
            _ => &mut right,
        };
        let number = Sessions::number_of(index).to_string();
        if lit {
            right.push(format!(" {number} "), theme.keycap());
        } else {
            right.push(number, theme.faint());
        }
        let mut first = Cells::new();
        first.push(format!("{}  ", session.kind.glyph()), if ended { theme.faint() } else { kind_style(session.kind, theme) });
        let room = inner.saturating_sub(first.width() + right.width() + 1);
        match &sessions.mode {
            Mode::Naming(name) if index == sessions.active => {
                first.push(format!("{}▏", fmt::truncate(name, room)), theme.keycap());
            }
            _ => {
                let style = if on_screen { theme.strong() } else if ended { theme.faint() } else { theme.text() };
                first.push(fmt::truncate(&session.label(), room), style);
            }
        }
        first.pad_to(inner.saturating_sub(right.width()));
        first.spans(right.into_spans());
        lines.push(card_row(first, width, on_screen, theme));

        // Where it works — or, after one x, what a second does.
        let mut second = Cells::new();
        second.push("   ", Style::default());
        if index == sessions.active && sessions.closing {
            second.push("x again closes it", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        } else {
            place(&mut second, &session.dir, session.branch.as_deref(), inner, theme);
        }
        lines.push(card_row(second, width, on_screen, theme));
        hits.push((Rect::new(area.x, y, area.width, 2), Hit::Session(index)));
    }

    // A new session is one more card at the end of the list.
    let next = sessions.list.len();
    lines.push(edge(next));
    if picking || next < MAX_SESSIONS {
        let y = area.y + lines.len() as u16;
        let mut first = Cells::new();
        first.push("+  ", theme.accent().add_modifier(Modifier::BOLD));
        first.push("new session", if picking { theme.strong() } else { theme.muted() });
        lines.push(card_row(first, width, picking, theme));
        if picking {
            let mut second = Cells::new();
            second.push("   choose its folder →", theme.faint());
            lines.push(card_row(second, width, true, theme));
        }
        hits.push((Rect::new(area.x, y, area.width, if picking { 2 } else { 1 }), Hit::NewSession));
        lines.push(edge(next + 1));
    } else {
        let mut full = Cells::new();
        full.push(format!("   {MAX_SESSIONS} of {MAX_SESSIONS} open · x closes one"), theme.faint());
        lines.push(full.line(width, Style::default()));
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

/// The top or the bottom of the framed card.
fn card_edge(width: usize, top: bool, theme: &Theme) -> Line<'static> {
    let run = "─".repeat(width.saturating_sub(2));
    let text = if top { format!("╭{run}╮") } else { format!("╰{run}╯") };
    Line::from(Span::styled(text, theme.border()))
}

/// A line of a card: inside the frame and painted, for the card on screen; set in by as much
/// for the others, so nothing moves when the frame does.
fn card_row(inner: Cells, width: usize, framed: bool, theme: &Theme) -> Line<'static> {
    let body = width.saturating_sub(4);
    if !framed {
        let mut cells = Cells::new();
        cells.push("  ", Style::default());
        cells.spans(fit(inner.into_spans(), body, false));
        return cells.line(width, Style::default());
    }
    let fill = theme.selected();
    let mut spans = vec![Span::styled("│", theme.border()), Span::styled(" ", fill)];
    spans.extend(fit(inner.into_spans(), body, true).into_iter().map(|span| {
        let style = fill.patch(span.style);
        Span::styled(span.content, style)
    }));
    spans.push(Span::styled(" ", fill));
    spans.push(Span::styled("│", theme.border()));
    Line::from(spans)
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

/// On a narrow terminal: the sessions as tabs over the pane, like the header's.
fn session_bar(app: &App, theme: &Theme, area: Rect) -> Line<'static> {
    let sessions = &app.claude;
    let width = area.width as usize;
    let mut hits = app.viewport.hits.borrow_mut();
    let mut cells = Cells::new();
    for (index, session) in sessions.list.iter().enumerate() {
        let number = Sessions::number_of(index);
        let on_screen = index == sessions.active && !matches!(sessions.mode, Mode::Opening(_));
        let x = area.x + cells.width() as u16;
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
        hits.push((Rect::new(x, area.y, (area.x + cells.width() as u16).saturating_sub(x), 1), Hit::Session(index)));
        cells.push(" ", Style::default());
    }
    let x = area.x + cells.width() as u16;
    match &sessions.mode {
        Mode::Opening(_) => {
            cells.push(" + new session ", theme.tab_active());
            cells.push("  choose its folder below", theme.faint());
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
        kind => format!("New {} session", kind.title()),
    };
    head.push(what, theme.strong());
    head.push("  ·  where should it work?", theme.muted());
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
    kinds.push(" run  ", theme.muted());
    for kind in Kind::ALL {
        let chip = format!(" {} {} ", kind.glyph(), kind.title());
        let x = area.x + kinds.width() as u16;
        hits.push((Rect::new(x, y, fmt::width(&chip) as u16, 1), Hit::Kind(kind)));
        kinds.push(chip, if kind == picker.kind { theme.tab_active() } else { theme.keycap() });
        kinds.push("  ", Style::default());
    }
    kinds.push(" shift+tab switches", theme.faint());
    lines.push(kinds.line(width, Style::default()));
    lines.push(Line::from(""));

    // The search, in a box of its own; what it found so far on its right.
    let inner = width.saturating_sub(4);
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let status = if picker.looking {
        "looking…".to_string()
    } else if picker.searching() {
        count(picker.folders.len(), "found", "found")
    } else {
        count(picker.folders.len(), "folder", "folders")
    };
    let mut field = Cells::new();
    field.push("⌕  ", theme.accent());
    if picker.query.is_empty() {
        field.push("▏", theme.accent());
        field.push("search the folders below here, or type a path", theme.faint());
    } else {
        field.push(picker.query.clone(), theme.strong());
        field.push("▏", theme.accent());
    }
    field.pad_to(inner.saturating_sub(fmt::width(&status)));
    field.push(status, theme.faint());
    lines.push(card_edge(width, true, theme));
    lines.push(boxed(field, width, theme));
    lines.push(card_edge(width, false, theme));

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

    // The rows, the cursor kept in sight; a line left at the bottom for what to say about them.
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
        lines.push(pick_line(*row, picker, index == cursor, width, theme));
        hits.push((Rect::new(area.x, y, area.width, 1), Hit::Pick(*row)));
    }
    let note = if let Some(error) = &picker.error {
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

/// A line inside the picker's search box.
fn boxed(inner: Cells, width: usize, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("│ ", theme.border())];
    spans.extend(fit(inner.into_spans(), width.saturating_sub(4), true));
    spans.push(Span::styled(" │", theme.border()));
    Line::from(spans)
}

/// One row of the picker: the first opens the session here, the next goes up, the rest are
/// folders — a repository's with its branch.
fn pick_line(row: PickRow, picker: &Picker, on: bool, width: usize, theme: &Theme) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(if on { "▌" } else { " " }, theme.accent());
    let mut branch = None;
    match row {
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
