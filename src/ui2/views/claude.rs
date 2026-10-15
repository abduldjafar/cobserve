//! SESSIONS zoomed (`docs/redesign/09-zooms.md` §4): *what are my sessions doing?* While they
//! all fit at a size that can be read, a grid of live thumbnails — every session a card whose
//! body is the bottom of its own screen, cropped from the same emulated terminal the full view
//! draws (`claude.rs`); past that, every session a line grouped by project beside the chosen
//! one's screen, large. A session opened full takes the inside under a bar of tabs; while a new
//! one is opened, the folder picker does.

use super::widgets::{fit, Cells};
use crate::app::{App, Hit};
use crate::claude::{Kind, Mode, PaneState, PickRow, Picker, Session, Sessions, MAX_SESSIONS};
use crate::fmt;
use crate::folders::Folder;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::paint::Painter;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// How the zoomed sessions are laid out at a size: a grid of live thumbnails while they all fit
/// at a size that can be read, the list beside a big preview past that (`09-zooms.md` §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Grid { cols: u16, rows: u16, card_w: u16, card_h: u16 },
    List { list_w: u16, preview_w: Option<u16> },
}

/// The strip and the hint line over the body, when there is height for the hint line.
fn body_height(height: u16) -> u16 {
    height.saturating_sub(if height >= 16 { 2 } else { 1 })
}

fn grid_cols(width: u16) -> u16 {
    if width < 76 { 1 } else { (width / 48).clamp(2, 4) }
}

/// How many cards the grid holds at a size: each at least 36 cells wide and 9 rows tall — a
/// title, an info row, six screen rows and a bottom; none when not even one fits.
pub fn cap(width: u16, height: u16) -> usize {
    let body = body_height(height);
    if width < 36 || body < 9 {
        return 0;
    }
    usize::from(grid_cols(width)) * usize::from((body / 9).clamp(1, 3))
}

/// The layout for `n` sessions at a size: a pure function, so the same sessions at the same
/// size always draw the same screen, and nothing flickers — `n` only changes by a key.
pub fn shape(width: u16, height: u16, n: usize) -> Shape {
    let body = body_height(height);
    if n <= cap(width, height) {
        let cols = grid_cols(width).min(n.max(1) as u16);
        let rows = (n.div_ceil(cols as usize) as u16).clamp(1, (body / 9).clamp(1, 3));
        let card_w = (width - 2 * (cols - 1)) / cols;
        return Shape::Grid { cols, rows, card_w, card_h: body / rows };
    }
    let list_w = width.saturating_sub(78).clamp(32, 70).min(width);
    let preview_w = width.saturating_sub(list_w + 2);
    Shape::List { list_w: if preview_w >= 48 { list_w } else { width }, preview_w: (preview_w >= 48).then_some(preview_w) }
}

/// A session's state, as a word and a mark, the severity it is drawn in, and whether it spins:
/// working (it printed within three seconds), needs you (it rang), idle, ended, kept, starting.
pub fn state_word(session: &Session, clock: std::time::SystemTime) -> (&'static str, &'static str, Severity, bool) {
    if session.waiting() {
        return ("kept", "↻", Severity::None, false);
    }
    if let Some(console) = &session.console {
        return match console.state {
            crate::console::RunState::Running { .. } => ("working", "◐", Severity::Ok, true),
            _ => ("ready", "○", Severity::None, false),
        };
    }
    if session.pane.attention {
        return ("needs you", "!", Severity::Warn, false);
    }
    match &session.pane.state {
        PaneState::Exited(_) | PaneState::Failed(_) => ("ended", "✕", Severity::None, false),
        PaneState::Idle | PaneState::Starting => ("starting", "◌", Severity::None, false),
        PaneState::Running => {
            let recent = session.pane.last_output.and_then(|at| clock.duration_since(at).ok()).is_some_and(|d| d.as_secs() < 3);
            if recent { ("working", "◐", Severity::Ok, true) } else { ("idle", "○", Severity::None, false) }
        }
    }
}

/// The mark drawn for a state: the spinner's frame for one that works.
fn state_mark(app: &App, session: &Session, theme: &Theme) -> (String, Style) {
    let (_, mark, sev, spinning) = state_word(session, app.clock);
    let style = match sev {
        Severity::Warn => theme.sev(Severity::Warn).add_modifier(Modifier::BOLD),
        Severity::Ok => theme.sev(Severity::Ok),
        _ if mark == "↻" => theme.accent(),
        _ => theme.muted(),
    };
    (if spinning { super::kit::spinner(app).to_string() } else { mark.to_string() }, style)
}

/// The zoomed sessions in `inside`: the folder picker while one is opened; a session opened full
/// under a bar of tabs; else the strip, the hint line and the grid or the list.
pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, inside: Rect) {
    if inside.height == 0 || inside.width < 10 {
        return;
    }
    if matches!(app.claude.mode, Mode::Opening(_)) {
        pane(frame, app, theme, inside);
        return;
    }
    let n = app.claude.list.len();
    let shape = shape(inside.width, inside.height, n);
    app.shell.sessions_preview.set(matches!(shape, Shape::List { preview_w: Some(_), .. }));
    // Opened in the preview: the list stays, the program has the keys.
    if app.shell.sessions_full
        && !app.shell.sessions_spread
        && let Shape::List { list_w, preview_w: Some(w) } = shape
        && n > 0
    {
        let pane_r = {
            let mut p = Painter::new(frame.buffer_mut());
            let clip = Rect::new(inside.x.saturating_sub(1), inside.y, inside.width + 2, inside.height);
            p.within(clip, |p| {
                strip(p, app, theme, inside);
                let top = if inside.height >= 16 {
                    typing_hints(p, app, theme, Rect::new(inside.x, inside.y + 1, inside.width, 1));
                    inside.y + 2
                } else {
                    inside.y + 1
                };
                let body = Rect::new(inside.x, top, inside.width, inside.bottom().saturating_sub(top));
                let mut hits = Vec::new();
                let page = list(p, app, theme, Rect::new(body.x, body.y, list_w, body.height), &mut hits);
                app.shell.sessions_shape.set((0, page));
                app.viewport.hits.borrow_mut().extend(hits);
                let r = Rect::new(body.right() - w, body.y, w, body.height);
                let index = app.claude.active;
                let session = &app.claude.list[index];
                let card = super::kit::Card::new(theme.accent())
                    .title(card_title(app, session, index, true, theme))
                    .right(state_spans(app, session, theme))
                    .bottom(vec![Span::styled("typing here · ctrl+\\ the list", theme.muted())]);
                let inner = card.draw(p, r);
                Rect::new(inner.x, inner.y, inner.width + 1, inner.height)
            })
        };
        pane(frame, app, theme, pane_r);
        return;
    }
    if app.shell.sessions_full && !app.claude.list.is_empty() {
        let bar = Rect::new(inside.x, inside.y, inside.width, 1);
        frame.render_widget(Paragraph::new(session_bar(app, theme, bar)), bar);
        pane(frame, app, theme, Rect::new(inside.x, inside.y + 1, inside.width, inside.height - 1));
        return;
    }
    // Every PTY keeps the size it has opened full: a thumbnail is a crop, never a resize.
    app.claude.want_size.set((inside.height.saturating_sub(1).max(1), inside.width));
    let mut p = Painter::new(frame.buffer_mut());
    let clip = Rect::new(inside.x.saturating_sub(1), inside.y, inside.width + 2, inside.height);
    p.within(clip, |p| {
        strip(p, app, theme, inside);
        let top = if inside.height >= 16 {
            hints(p, app, theme, Rect::new(inside.x, inside.y + 1, inside.width, 1));
            inside.y + 2
        } else {
            inside.y + 1
        };
        let body = Rect::new(inside.x, top, inside.width, inside.bottom().saturating_sub(top));
        let mut hits = Vec::new();
        match shape {
            Shape::Grid { cols, rows, card_w, card_h } => {
                app.shell.sessions_shape.set((rows, cols * rows));
                grid(p, app, theme, body, cols, rows, card_w, card_h, &mut hits);
            }
            Shape::List { list_w, preview_w } => {
                let page = list(p, app, theme, Rect::new(body.x, body.y, list_w, body.height), &mut hits);
                app.shell.sessions_shape.set((0, page));
                if let Some(w) = preview_w {
                    let r = Rect::new(body.right() - w, body.y, w, body.height);
                    preview(p, app, theme, r, &mut hits);
                }
            }
        }
        app.viewport.hits.borrow_mut().extend(hits);
    });
}

/// `✻ 287.0M today · ≈ $191.17 · session 11% · week 70%   ▣ 53.9M today · $0.86` and, at the
/// right, the counts across every session.
fn strip(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let sessions = &app.claude;
    let n = sessions.list.len();
    let working = sessions.list.iter().filter(|s| state_word(s, app.clock).1 == "◐").count();
    let calling = sessions.list.iter().filter(|s| s.pane.attention).count();
    let mut counts = Cells::new();
    counts.push(fmt::plural(n, "session", "sessions"), theme.text2());
    if working > 0 {
        counts.push(" · ", theme.faint());
        counts.push(format!("{working} working"), theme.sev(Severity::Ok));
    }
    if calling > 0 {
        counts.push(" · ", theme.faint());
        counts.push(format!("{calling} {}", if calling == 1 { "needs you" } else { "need you" }), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    let room = (i.width as usize).saturating_sub(counts.width() + 2);
    let mut usage = Cells::new();
    for (k, line) in super::usage::sidebar_lines(app, theme, 200).into_iter().enumerate() {
        let spans: Vec<Span<'static>> = line.spans.into_iter().filter(|s| !s.content.trim().is_empty() || s.content.len() < 3).collect();
        let mut piece = Cells::new();
        piece.spans(spans);
        let text_w = piece.width();
        if usage.width() + if k > 0 { 3 } else { 0 } + text_w > room {
            break;
        }
        if k > 0 {
            usage.push("   ", Style::default());
        }
        usage.spans(piece.into_spans());
    }
    super::kit::put_cells(p, i.x, i.y, usage, room);
    super::kit::put_spans_r(p, i.right(), i.y, counts.into_spans());
}

/// While typing in the preview: where the keys go, and the way back.
fn typing_hints(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let mut cells = Cells::new();
    let who = app.claude.current().map(|s| s.label()).unwrap_or_default();
    cells.push("⌨ ", theme.accent());
    cells.push(format!("keys go to {who}"), theme.text2());
    cells.push(" · ", theme.faint());
    cells.push("ctrl+\\ ", theme.strong());
    cells.push("the list", theme.muted());
    super::kit::put_cells(p, r.x, r.y, cells, r.width as usize);
}

/// The keys that act here — or, while finding, the search field.
fn hints(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let sessions = &app.claude;
    let mut cells = Cells::new();
    if let Mode::Finding { query, at } = &sessions.mode {
        let found = sessions.matching(query);
        cells.push("⌕ /", theme.accent());
        cells.push(query.clone(), theme.strong());
        cells.push("▏  ", theme.accent());
        if found.is_empty() {
            cells.push("none · no session is called that · esc back", theme.faint());
        } else {
            cells.push(format!("{} of {} · ↑↓ choose · ⏎ goes there · esc back", (at + 1).min(found.len()), found.len()), theme.faint());
        }
        super::kit::put_cells(p, r.x, r.y, cells, r.width as usize);
        return;
    }
    let narrow = r.width < 100;
    let full = sessions.list.len() >= MAX_SESSIONS;
    let mut new_w = 0u16;
    if full {
        cells.push(format!("{MAX_SESSIONS} of {MAX_SESSIONS} · x x closes one"), theme.faint());
    } else {
        cells.push("n ", theme.strong());
        cells.push(if narrow { "new" } else { "new session" }, theme.muted());
        new_w = cells.width() as u16;
    }
    cells.push(" · ", theme.faint());
    let resume_x = cells.width() as u16;
    cells.push("↻ ", theme.accent());
    cells.push(if narrow { "resume" } else { "past conversations" }, theme.muted());
    let resume_w = cells.width() as u16 - resume_x;
    let rest: [(&str, &str); 5] = if narrow {
        [("/", "find"), ("x x", "close"), ("\\", "float"), ("⏎", "open"), ("f", "full")]
    } else {
        [("/", "find one"), ("x x", "close"), ("\\", "float it"), ("⏎", "type in it"), ("f", "full")]
    };
    for (key, word) in rest.into_iter().filter(|(key, _)| !(full && *key == "x x")) {
        cells.push(" · ", theme.faint());
        cells.push(format!("{key} "), theme.strong());
        cells.push(word, theme.muted());
    }
    super::kit::put_cells(p, r.x, r.y, cells, r.width as usize);
    let mut hits = app.viewport.hits.borrow_mut();
    if new_w > 0 {
        hits.push((Rect::new(r.x, r.y, new_w, 1), Hit::NewSession));
    }
    hits.push((Rect::new(r.x + resume_x, r.y, resume_w, 1), Hit::Resume));
}

/// The sessions shown: every one, or those a search names.
fn shown(app: &App) -> Vec<usize> {
    match &app.claude.mode {
        Mode::Finding { query, .. } => app.claude.matching(query),
        _ => app.claude.order(),
    }
}

/// The chosen one: the active session, or the match a search is on.
fn chosen(app: &App) -> Option<usize> {
    match &app.claude.mode {
        Mode::Finding { query, at } => app.claude.matching(query).get(*at).copied(),
        _ => (!app.claude.list.is_empty()).then_some(app.claude.active),
    }
}

/// `~/work/billing ⎇ main · 2.1M today · claude`, or what a second `x` does.
fn info_row(app: &App, session: &Session, index: usize, used: &std::collections::HashMap<String, u64>, theme: &Theme, room: usize) -> Cells {
    let mut cells = Cells::new();
    if index == app.claude.active && app.claude.closing {
        cells.push("x again closes it", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        return cells;
    }
    if let Some(console) = &session.console {
        cells.push(format!("SQL on {} · read-only", console.node.as_deref().unwrap_or("no server")), theme.muted());
        return cells;
    }
    let mut tail = Cells::new();
    if let Some(n) = session.conversation.as_ref().and_then(|c| used.get(c)).filter(|n| **n > 0) {
        tail.push(" · ", theme.faint());
        tail.push(format!("{} today", crate::usage::tokens(*n)), theme.muted());
    }
    tail.push(" · ", theme.faint());
    tail.push(session.kind.title().to_lowercase(), theme.faint());
    place(&mut cells, &session.dir, session.branch.as_deref(), room.saturating_sub(tail.width()), theme);
    cells.spans(tail.into_spans());
    cells
}

/// A card's title: ` ✻ 7 · billing export `, or the name being typed.
fn card_title(app: &App, session: &Session, index: usize, lit: bool, theme: &Theme) -> Vec<Span<'static>> {
    let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));
    let number = Sessions::number_of(index);
    let name = match &app.claude.mode {
        Mode::Naming(name) if index == app.claude.active => return vec![
            Span::styled(format!("{} {number} · ", session.kind.glyph()), kind_style(session.kind, theme)),
            Span::styled(format!("{name}▏"), theme.keycap()),
        ],
        _ => session.label(),
    };
    if lit {
        return vec![Span::styled(format!("{} {number} · {name}", session.kind.glyph()), theme.tab_active())];
    }
    vec![
        Span::styled(session.kind.glyph().to_string(), if ended { theme.faint() } else { kind_style(session.kind, theme) }),
        Span::styled(format!(" {number} · "), theme.muted()),
        Span::styled(name, if ended || session.waiting() { theme.muted() } else { theme.strong() }),
    ]
}

fn state_spans(app: &App, session: &Session, theme: &Theme) -> Vec<Span<'static>> {
    let (word, _, _, _) = state_word(session, app.clock);
    let (mark, style) = state_mark(app, session, theme);
    vec![Span::styled(format!("{mark} {word}"), style)]
}

/// A card of the grid, or the preview: the frame in the cursor's accent (or amber when it rang),
/// the title and the state, the info row, the bottom of its screen.
#[allow(clippy::too_many_arguments)]
fn session_card(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, index: usize, lit: bool, live: bool, used: &std::collections::HashMap<String, u64>) {
    let session = &app.claude.list[index];
    let frame_style = if lit {
        theme.accent()
    } else if session.pane.attention {
        theme.sev(Severity::Warn)
    } else {
        theme.border()
    };
    let mut card = super::kit::Card::new(frame_style).title(card_title(app, session, index, lit, theme)).right(state_spans(app, session, theme));
    if lit {
        let open = if app.shell.sessions_preview.get() { "⏎ type in it here · f full" } else { "⏎ opens it full" };
        card = card.bottom(vec![Span::styled(format!("{open} · \\ floats it · r renames it"), theme.muted())]);
    }
    let inner = card.draw(p, r);
    if inner.height == 0 {
        return;
    }
    let mut y = inner.y;
    if inner.height >= 5 || r.height >= 8 {
        super::kit::put_cells(p, inner.x, y, info_row(app, session, index, used, theme, inner.width as usize), inner.width as usize);
        y += 1;
    }
    let screen_r = Rect::new(inner.x, y, inner.width + 1, inner.bottom().saturating_sub(y));
    thumbnail(p, app, theme, session, screen_r, lit || live);
}

/// The bottom of a session's screen, cropped to `r` — never resized: the rows that end at its
/// cursor (or its last line with something on it), the columns from the left. Another program's
/// colours, dimmed unless `bright`; a query session's SQL and answer; a pane not started yet says
/// so.
fn thumbnail(p: &mut Painter<'_>, app: &App, theme: &Theme, session: &Session, r: Rect, bright: bool) {
    if r.height == 0 || r.width == 0 {
        return;
    }
    if let Some(console) = &session.console {
        console_thumbnail(p, theme, console, r, bright);
        return;
    }
    let program = app.claude.command_of(session.kind).first().cloned().unwrap_or_default();
    let note = |p: &mut Painter<'_>, lines: Vec<(String, Style)>| {
        for (k, (text, style)) in lines.into_iter().enumerate() {
            let y = r.y + (r.height / 3).min(r.height.saturating_sub(1)) + k as u16;
            if y < r.bottom() {
                p.put(r.x + 1, y, &fmt::truncate(&text, r.width.saturating_sub(2) as usize), style);
            }
        }
    };
    match &session.pane.state {
        PaneState::Idle if session.waiting() => note(p, vec![(format!("↻ kept from the last run · ⏎ takes {} up again", session.kind.title()), theme.muted())]),
        PaneState::Idle | PaneState::Starting => note(p, vec![(format!("starting {program} in {} …", session.dir), theme.muted())]),
        PaneState::Failed(why) => {
            let mut lines = vec![(why.clone(), theme.sev(Severity::Warn))];
            lines.extend(how_to_get(session.kind, &program).into_iter().map(|t| (t, theme.muted())));
            note(p, lines);
        }
        PaneState::Running | PaneState::Exited(_) => {
            let live = matches!(session.pane.state, PaneState::Running);
            let screen = session.pane.screen();
            let (rows, cols) = screen.size();
            let mut last = screen.cursor_position().0.min(rows.saturating_sub(1));
            for row in (0..rows).rev() {
                if (0..cols).any(|c| screen.cell(row, c).is_some_and(|cell| cell.has_contents() && cell.contents() != " ")) {
                    last = last.max(row);
                    break;
                }
            }
            let first = (last + 1).saturating_sub(r.height);
            for (k, row) in (first..=last).enumerate() {
                let y = r.y + k as u16;
                let mut x = r.x;
                for col in 0..cols.min(r.width) {
                    let Some(cell) = screen.cell(row, col) else { break };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut style = style_of(cell, live);
                    if !bright {
                        style = style.add_modifier(Modifier::DIM);
                    }
                    let text = if cell.has_contents() { cell.contents().to_string() } else { " ".to_string() };
                    x = p.put(x, y, &text, theme.text().patch(style));
                }
            }
            if let PaneState::Exited(how) = &session.pane.state
                && bright
            {
                let y = r.bottom() - 1;
                p.band(Rect::new(r.x, y, r.width, 1), theme.selected());
                super::kit::put_spans(
                    p,
                    r.x,
                    y,
                    vec![Span::styled(format!(" {program} — {how} "), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)), Span::styled(" ⏎ start it again", theme.muted())],
                    r.width as usize,
                );
            }
        }
    }
}

/// A query session in a card: its SQL, how the last run went, the answer's first rows.
fn console_thumbnail(p: &mut Painter<'_>, theme: &Theme, console: &crate::console::Console, r: Rect, bright: bool) {
    let dim = |s: Style| if bright { s } else { s.add_modifier(Modifier::DIM) };
    let w = r.width.saturating_sub(1) as usize;
    let mut y = r.y;
    let bottom = r.bottom();
    for line in console.lines.iter().filter(|l| !l.trim().is_empty()) {
        if y >= bottom {
            return;
        }
        p.put(r.x, y, &fmt::truncate(line, w), dim(theme.text()));
        y += 1;
    }
    if y + 1 >= bottom {
        return;
    }
    y += 1;
    let mut status = Cells::new();
    console_line(&mut status, console, theme);
    super::kit::put_cells(p, r.x, y, status, w);
    y += 1;
    let Some(answer) = console.answer.as_ref().filter(|a| !a.columns.is_empty()) else { return };
    let widths: Vec<usize> = answer
        .columns
        .iter()
        .enumerate()
        .map(|(c, (name, _))| answer.rows.iter().take(20).filter_map(|row| row.get(c).and_then(|v| v.as_deref()).map(fmt::width)).chain([fmt::width(name)]).max().unwrap_or(4).min(18))
        .collect();
    let row_text = |cells: Vec<String>| -> String { cells.iter().zip(&widths).map(|(t, w)| fmt::pad(&fmt::truncate(t, *w), *w)).collect::<Vec<_>>().join("  ") };
    if y < bottom {
        p.put(r.x, y, &fmt::truncate(&row_text(answer.columns.iter().map(|(n, _)| n.clone()).collect()), w), dim(theme.section()));
        y += 1;
    }
    for row in &answer.rows {
        if y >= bottom {
            break;
        }
        let text = row_text(row.iter().map(|v| v.clone().unwrap_or_else(|| "NULL".into())).collect());
        p.put(r.x, y, &fmt::truncate(&text, w), dim(theme.text2()));
        y += 1;
    }
}

/// The grid: the sessions column by column in the list's order, so `↓` is the card below and
/// the next one at once; a free cell offers a new session, the next one a past conversation.
#[allow(clippy::too_many_arguments)]
fn grid(p: &mut Painter<'_>, app: &App, theme: &Theme, body: Rect, cols: u16, rows: u16, card_w: u16, card_h: u16, hits: &mut Vec<(Rect, Hit)>) {
    let order = shown(app);
    let lit = chosen(app);
    let used = super::usage::by_session(app);
    let cell = |k: usize| -> Rect {
        let (col, row) = ((k / rows as usize) as u16, (k % rows as usize) as u16);
        Rect::new(body.x + col * (card_w + 2), body.y + row * card_h, card_w, card_h)
    };
    for (k, &index) in order.iter().enumerate() {
        if k >= usize::from(cols * rows) {
            break;
        }
        let r = cell(k);
        session_card(p, app, theme, r, index, lit == Some(index), false, &used);
        hits.push((r, Hit::Session(index)));
    }
    if matches!(app.claude.mode, Mode::Finding { .. }) {
        return;
    }
    // The free cells: a new session, then a past conversation.
    let free = usize::from(cols * rows).saturating_sub(order.len());
    let ghosts: [(&str, &str, Hit); 2] = [
        ("+ new session", "n · Claude, OpenCode, a terminal or a query session", Hit::NewSession),
        ("↻ past conversations", "a conversation had elsewhere, taken up here", Hit::Resume),
    ];
    if order.is_empty() {
        let w = body.width.saturating_sub(2) / 2;
        for (k, (title, text, hit)) in ghosts.into_iter().enumerate() {
            let r = Rect::new(body.x + k as u16 * (w + 2), body.y, w, 5.min(body.height));
            ghost(p, theme, r, title, text);
            hits.push((r, hit));
        }
        return;
    }
    for (k, (title, text, hit)) in ghosts.into_iter().enumerate().take(free) {
        if app.claude.list.len() >= MAX_SESSIONS && hit == Hit::NewSession {
            continue;
        }
        let r = cell(order.len() + k);
        ghost(p, theme, r, title, text);
        hits.push((r, hit));
    }
}

fn ghost(p: &mut Painter<'_>, theme: &Theme, r: Rect, title: &str, text: &str) {
    let inner = super::kit::Card::new(theme.faint()).title(vec![Span::styled(title.to_string(), theme.muted())]).draw(p, r);
    if inner.height > 0 {
        p.put(inner.x, inner.y + inner.height / 2, &fmt::truncate(text, inner.width as usize), theme.faint());
    }
}

/// The last line a session's program printed, for the list's tails.
fn last_line(session: &Session) -> String {
    if let Some(console) = &session.console {
        let mut cells = Cells::new();
        console_line(&mut cells, console, crate::theme::current());
        return cells.into_spans().into_iter().map(|s| s.content.into_owned()).collect();
    }
    if !matches!(session.pane.state, PaneState::Running | PaneState::Exited(_)) {
        return String::new();
    }
    let screen = session.pane.screen();
    let (rows, cols) = screen.size();
    let cursor = screen.cursor_position().0;
    // The last line that says something: not a rule, a box's side or an empty prompt.
    let says = |t: &str| !t.chars().all(|c| c.is_whitespace() || "─━│┃╭╮╰╯┌┐└┘❯›>".contains(c));
    (0..=cursor.min(rows.saturating_sub(1))).rev().map(|row| screen.contents_between(row, 0, row, cols).trim().to_string()).find(|t| says(t)).unwrap_or_default()
}

/// A line of the list: what is listed, and which session a click on it chooses.
enum Row {
    Header(String, usize, usize, Option<String>, usize),
    Session(usize),
}

/// The list: a header a project and a line a session, in the order `↑↓` walk — or, while
/// finding, the matches a line each; a window that follows the chosen line when they do not all
/// fit, a line at each end for how many sessions are past it. Returns the session lines a page
/// holds.
fn list(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, hits: &mut Vec<(Rect, Hit)>) -> u16 {
    let sessions = &app.claude;
    let finding = matches!(sessions.mode, Mode::Finding { .. });
    let lit = chosen(app);
    let used = super::usage::by_session(app);
    let mut rows: Vec<Row> = Vec::new();
    if finding {
        rows.extend(shown(app).into_iter().map(Row::Session));
    } else {
        for (project, members) in sessions.groups() {
            let calling = members.iter().filter(|&&i| sessions.list[i].pane.attention).count();
            let branch = members.first().and_then(|&i| sessions.list[i].branch.clone()).filter(|b| members.iter().all(|&i| sessions.list[i].branch.as_deref() == Some(b.as_str())));
            rows.push(Row::Header(project, members.len(), calling, branch, members[0]));
            rows.extend(members.into_iter().map(Row::Session));
        }
    }
    let height = r.height as usize;
    let at = rows.iter().position(|row| matches!(row, Row::Session(i) if Some(*i) == lit));
    // A window that follows the chosen line when they do not all fit; a line at an end only
    // when something is past it.
    let (first, window) = if rows.len() <= height {
        (0, rows.len())
    } else {
        let window = height.saturating_sub(2).max(1);
        let first = crate::app::scroll_into_view(sessions.list_scroll.get(), at, window, rows.len());
        if first == 0 { (0, height.saturating_sub(1).max(1)) } else { (first, window) }
    };
    sessions.list_scroll.set(first);
    let w = r.width as usize;
    let tails = w >= 60;
    let count_sessions = |rs: &[Row]| rs.iter().filter(|row| matches!(row, Row::Session(_))).count();
    let mut y = r.y;
    if first > 0 {
        let above = count_sessions(&rows[..first]);
        let mut cells = Cells::new();
        if above > 0 {
            cells.push(format!("  ↑ {above} more"), theme.faint());
        }
        // The cut group's header, so the lines under it keep their project.
        if let Some(Row::Session(_)) = rows.get(first)
            && let Some(Row::Header(project, n, calling, branch, _)) = rows[..first].iter().rev().find(|row| matches!(row, Row::Header(..)))
        {
            let header = group_header(project, *n, *calling, branch.as_deref(), theme);
            cells.pad_to(w.saturating_sub(header.width() + 1));
            cells.spans(header.into_spans());
        }
        if above > 0
            && let Some(i) = rows[..first].iter().rev().find_map(|row| if let Row::Session(i) = row { Some(*i) } else { None })
        {
            hits.push((Rect::new(r.x, y, r.width, 1), Hit::Session(i)));
        }
        super::kit::put_cells(p, r.x, y, cells, w);
        y += 1;
    }
    for row in rows.iter().skip(first).take(window) {
        match row {
            Row::Header(project, n, calling, branch, first_member) => {
                let mut cells = Cells::new();
                cells.push("  ", Style::default());
                cells.spans(group_header(project, *n, *calling, branch.as_deref(), theme).into_spans());
                super::kit::put_cells(p, r.x, y, cells, w);
                hits.push((Rect::new(r.x, y, r.width, 1), Hit::Session(*first_member)));
            }
            Row::Session(index) => {
                let session = &sessions.list[*index];
                let on = lit == Some(*index);
                if on {
                    p.band(Rect::new(r.x.saturating_sub(1), y, r.width + 1, 1), theme.selected());
                }
                let ended = matches!(session.pane.state, PaneState::Exited(_) | PaneState::Failed(_));
                let (mark, mark_style) = state_mark(app, session, theme);
                let mut cells = Cells::new();
                cells.push(if on { "▌" } else { " " }, theme.accent());
                cells.push(mark, mark_style);
                cells.push(" ", Style::default());
                cells.push(session.kind.glyph().to_string(), if ended { theme.faint() } else { kind_style(session.kind, theme) });
                cells.push(format!(" {:>2} ", Sessions::number_of(*index)), theme.muted());
                let mut right = Cells::new();
                if finding {
                    right.push(session.project(), theme.faint());
                } else if let Some(n) = session.conversation.as_ref().and_then(|c| used.get(c)).filter(|n| **n > 0) {
                    right.push(crate::usage::tokens(*n), theme.faint());
                }
                let name_style = if on { theme.strong() } else if ended || session.waiting() { theme.muted() } else { theme.text() };
                let name = match &sessions.mode {
                    Mode::Naming(name) if *index == sessions.active => {
                        cells.push(format!("{name}▏"), theme.keycap());
                        None
                    }
                    _ => Some(session.label()),
                };
                if let Some(name) = name {
                    let name_w = if tails { 24 } else { w.saturating_sub(cells.width() + right.width() + 2) };
                    cells.push(fmt::pad(&fmt::truncate(&name, name_w), name_w.min(fmt::width(&name).max(if tails { name_w } else { 0 }))), name_style);
                }
                if tails {
                    let room = w.saturating_sub(cells.width() + right.width() + 3);
                    let tail = if *index == sessions.active && sessions.closing { "x again closes it".to_string() } else { last_line(session) };
                    let style = if *index == sessions.active && sessions.closing { theme.sev(Severity::Warn).add_modifier(Modifier::BOLD) } else { theme.faint() };
                    cells.push(" ", Style::default());
                    cells.push(fmt::truncate(&tail, room), style);
                }
                cells.pad_to(w.saturating_sub(right.width() + 1));
                cells.spans(right.into_spans());
                super::kit::put_cells(p, r.x, y, cells, w);
                hits.push((Rect::new(r.x, y, r.width, 1), Hit::Session(*index)));
            }
        }
        y += 1;
    }
    if rows.len() > height {
        let below = count_sessions(&rows[(first + window).min(rows.len())..]);
        if below > 0 {
            super::kit::put_spans(p, r.x, y, vec![Span::styled(format!("  ↓ {below} more"), theme.faint())], w);
            if let Some(i) = rows[first + window..].iter().find_map(|row| if let Row::Session(i) = row { Some(*i) } else { None }) {
                hits.push((Rect::new(r.x, y, r.width, 1), Hit::Session(i)));
            }
        }
    }
    count_sessions(&rows[first..(first + window).min(rows.len())]).max(1) as u16
}

/// `billing 3  ! 1  ⎇ main`.
fn group_header(project: &str, n: usize, calling: usize, branch: Option<&str>, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    cells.push(project.to_string(), theme.section());
    cells.push(format!(" {n}"), theme.faint());
    if calling > 0 {
        cells.push(format!("  ! {calling}"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    if let Some(branch) = branch {
        cells.push(format!("  ⎇ {branch}"), theme.faint());
    }
    cells
}

/// The chosen session beside the list, large: its card in the cursor's accent, its screen.
fn preview(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let Some(index) = chosen(app) else { return };
    let used = super::usage::by_session(app);
    session_card(p, app, theme, r, index, true, true, &used);
    hits.push((r, Hit::Session(index)));
}

/// The session on screen in `area`: the folder picker while one is opened, a query session's
/// console, or the program's screen. Every PTY is sized to it.
pub(crate) fn pane(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
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
                let mark = match state_word(session, app.clock).1 {
                    "◐" | "○" | "◌" => "",
                    other => other,
                };
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
pub(crate) fn screen(frame: &mut Frame, pane: &crate::claude::ClaudePane, theme: &Theme, area: Rect, live: bool) {
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


