//! The team's board (view 6, `d`): the open tickets of a Jira Structure whose summary starts with
//! the team's word, in lanes of their status — what nobody has taken by default (`f` for
//! everyone's) — the ticket under the cursor in a card under them, and `m` to move it: the moves
//! Jira allows, then the dialog that says what will be sent (`team.rs`).
//!
//! ```text
//!  TEAM · structure 216 · DE · not taken 9 of 14                          f everyone's · d yours
//!  BACKLOG 3 of 4        SELECTED FOR DEV… 2  IN PROGRESS 0 of 1  ON HOLD 1  IN REVIEW 0 of 1
//!  ━━━━━━━━━━━━━━━━━━━━  ──────────────────  ──────────────────  ─────────  ──────────────────
//!  ▌DATA-13301    High    DATA-13241   High   nothing here         DATA-13…
//! ```

use super::kit::{self, Card};
use super::widgets::Cells;
use crate::app::App;
use crate::jira;
use crate::severity::Severity;
use crate::team::{Filter, Team};
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{tr, Painter};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, i: Rect) {
    if i.height == 0 || i.width < 10 {
        return;
    }
    let mut p = Painter::new(frame.buffer_mut());
    let clip = Rect::new(i.x.saturating_sub(1), i.y, i.width + 2, i.height);
    p.within(clip, |p| body(p, app, theme, i));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let team = &app.team;
    let w = i.width as usize;
    if team.tickets.is_empty() && team.read_at.is_none() {
        kit::message(p, theme, i, ("◌", theme.muted()), "reading the team's board…", &[&format!("the {} tickets of Jira Structure {}", team.prefix, team.structure.unwrap_or(0)), "d for your own board"]);
        return;
    }
    if let (Some(why), true) = (&team.error, team.tickets.is_empty()) {
        kit::message(p, theme, i, ("✖", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)), "the team's board could not be read", &[why, "r reads again · d for your own board"]);
        return;
    }
    let order = &app.jira.statuses;
    let lanes = team.lanes(order);
    let untaken = team.tickets.iter().filter(|t| t.assignee.is_none()).count();
    let mut head = Cells::new();
    head.push("TEAM", theme.section());
    head.push(format!(" · structure {} · {}", team.structure.unwrap_or(0), team.prefix), theme.muted());
    head.push(" · ", theme.faint());
    head.push(format!("{untaken} not taken"), if untaken > 0 { theme.strong() } else { theme.muted() });
    head.push(format!(" of {} open", team.tickets.len()), theme.muted());
    head.push(format!(" · showing {}", team.filter.word()), theme.faint());
    let age = team.read_at.and_then(|at| app.clock.duration_since(at).ok());
    let fresh = kit::freshness(age, team.error.as_deref(), team.reading, theme, w / 3);
    kit::put_cells(p, i.x, i.y, head, w.saturating_sub(fresh.width() + 2));
    kit::put_spans_r(p, i.right(), i.y, fresh.into_spans());

    let card_h: u16 = if i.height >= 24 { 5 } else { 0 };
    let r = Rect::new(i.x, i.y + 1, i.width, i.height.saturating_sub(1 + card_h));
    lanes_view(p, app, theme, r, team, &lanes);
    if card_h > 0 {
        detail(p, app, theme, Rect::new(i.x, i.bottom() - card_h, i.width, card_h));
    }
    if let Some(m) = &team.moving {
        if m.confirm {
            confirm(p, theme, m);
        } else {
            picker(p, theme, i, m);
        }
    }
}

fn lanes_view(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, team: &Team, lanes: &[crate::team::Lane<'_>]) {
    if r.height < 3 {
        return;
    }
    let now = app.now();
    let today = jira::today(now, app.time.offset_s(now));
    let count = lanes.len().max(1);
    let width = r.width as usize;
    let gap = if width >= 100 { 2 } else { 1 };
    let lane_w = width.saturating_sub(gap * (count - 1)) / count;
    let summary_lines = if width >= 100 && r.height >= 14 { 2 } else { 1 };
    let block_h = summary_lines + 3;
    let selected = team.selected.as_deref();
    let holding = lanes.iter().position(|l| l.tickets.iter().any(|t| Some(t.key.as_str()) == selected));
    let lane_x = |l: usize| r.x + (l * (lane_w + gap)) as u16;
    for (l, lane) in lanes.iter().enumerate() {
        let mut title = Cells::new();
        title.push(lane.status.to_uppercase(), if lane.tickets.is_empty() { theme.muted() } else { theme.section() });
        title.push(format!(" {}", lane.tickets.len()), if lane.tickets.is_empty() { theme.faint() } else { theme.strong() });
        if team.filter == Filter::Untaken && lane.all != lane.tickets.len() {
            title.push(format!(" of {}", lane.all), theme.faint());
        }
        kit::put_cells(p, lane_x(l), r.y, title, lane_w);
        let (glyph, style) = if holding == Some(l) { ("━", theme.accent()) } else { ("─", theme.rule()) };
        p.put(lane_x(l), r.y + 1, &glyph.repeat(lane_w.saturating_sub(1)), style);
    }
    // Each lane its column of blocks; the cursor's block on screen.
    let mut columns: Vec<Vec<Cells>> = Vec::new();
    let mut chosen: Option<(usize, usize)> = None;
    for (l, lane) in lanes.iter().enumerate() {
        let mut rows: Vec<Cells> = Vec::new();
        for ticket in &lane.tickets {
            let is = Some(ticket.key.as_str()) == selected;
            if is {
                chosen = Some((l, rows.len()));
            }
            rows.extend(super::board::block(ticket, false, is, summary_lines, now, today, theme, lane_w));
            let mut who = Cells::new();
            who.push(if is { "▌" } else { " " }, theme.accent());
            match &ticket.assignee {
                Some(name) => who.push(name.clone(), theme.text2()),
                None => who.push("not taken", theme.faint()),
            };
            if let Some(kind) = &ticket.kind {
                who.push(format!(" · {}", kind.to_lowercase()), theme.faint());
            }
            rows.push(who);
            rows.push(Cells::new());
        }
        if lane.tickets.is_empty() {
            let mut none = Cells::new();
            none.push(" nothing here", theme.faint());
            rows.push(none);
        }
        columns.push(rows);
    }
    let room = r.height.saturating_sub(2) as usize;
    let tall = columns.iter().map(Vec::len).max().unwrap_or(0);
    let offset = match chosen {
        Some((_, row)) if row + block_h > room => (row + block_h - room).min(tall.saturating_sub(room)),
        _ => 0,
    };
    let top = r.y + 2;
    if let Some((l, first)) = chosen {
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
}

fn detail(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let right = vec![Span::styled("m move · ⏎ in full · o in Jira · f untaken/all · d yours", theme.muted())];
    let Some(t) = app.team.current() else {
        Card::new(theme.border()).title(vec![Span::styled("nothing to choose", theme.muted())]).right(right).draw(p, r);
        return;
    };
    let title = vec![Span::styled(t.key.clone(), theme.accent().add_modifier(Modifier::BOLD)), Span::styled(format!(" · {}", t.status), theme.text2())];
    let inner = Card::new(theme.border()).title(title).right(right).draw(p, r);
    let w = inner.width as usize;
    kit::put_spans(p, inner.x, inner.y, vec![Span::styled(t.summary.clone(), theme.strong())], w);
    let mut facts = Cells::new();
    match &t.assignee {
        Some(name) => facts.push(format!("{name} has it"), theme.text2()),
        None => facts.push("nobody has it yet", theme.text2()),
    };
    if let Some(kind) = &t.kind {
        facts.push(format!(" · {kind}"), theme.muted());
    }
    if let Some(pr) = &t.priority {
        facts.push(format!(" · {pr}"), theme.muted());
    }
    if let Some(age) = t.in_status(app.now()) {
        facts.push(format!(" · {} in {}", jira::age(age), t.status), theme.muted());
    }
    if let Some(parent) = &t.parent {
        facts.push(format!(" · under {parent}"), theme.faint());
    }
    kit::put_cells(p, inner.x, inner.y + 1, facts, w);
    if let Some(base) = &app.jira.base_url {
        kit::put_spans(p, inner.x, inner.y + 2, vec![Span::styled(format!("{base}/browse/{}", t.key), theme.faint())], w);
    }
}

/// The moves Jira allows, one a line; `⏎` asks to confirm the chosen one.
fn picker(p: &mut Painter<'_>, theme: &Theme, i: Rect, m: &crate::team::Moving) {
    let offered = m.offered();
    let lines: Vec<(String, ratatui::style::Style)> = match &m.transitions {
        None => vec![("◌ asking Jira what it allows…".into(), theme.accent())],
        Some(Err(e)) => vec![(format!("✖ {e}"), theme.sev(Severity::Warn)), ("any key closes this".into(), theme.faint())],
        Some(Ok(_)) if offered.is_empty() => vec![("Jira allows no move from here".into(), theme.muted())],
        Some(Ok(_)) => offered
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let mark = if k == m.chosen { "▸ " } else { "  " };
                let note = if t.needs_fields { "  · only in Jira (o)" } else { "" };
                let style = if k == m.chosen { theme.accent().add_modifier(Modifier::BOLD) } else if t.needs_fields { theme.faint() } else { theme.text() };
                (format!("{mark}{}{note}", t.to), style)
            })
            .collect(),
    };
    let w = (i.width.saturating_sub(4)).min(60);
    let h = (lines.len() as u16 + 4).min(i.height);
    let x = i.x + (i.width.saturating_sub(w)) / 2;
    let y = i.y + (i.height.saturating_sub(h)) / 2;
    let title = format!("Move {} from {}", m.key, m.from);
    frame::overlay(p, theme, Rect::new(x, y, w, h), &title, theme.strong(), "↑↓ choose · ⏎ move · esc keep");
    let surface = theme.surface();
    for (k, (line, style)) in lines.iter().enumerate().take(h.saturating_sub(4) as usize) {
        p.put(x + 3, y + 2 + k as u16, &tr(line, w as usize - 6), style.patch(surface));
    }
}

/// What will be sent, before it is: `y` moves it, anything else keeps it.
fn confirm(p: &mut Painter<'_>, theme: &Theme, m: &crate::team::Moving) {
    let Some(t) = m.current() else { return };
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    if sw < 30 || sh < 10 {
        return;
    }
    let w = (sw - 4).min(86);
    let h = 9;
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    let title = format!("Move {} · {} → {}?", m.key, m.from, t.to);
    frame::overlay(p, theme, Rect::new(x, y, w, h), &title, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD), "");
    let surface = theme.surface();
    let lines = [
        (jira::split_tag(&m.summary).1.to_string(), theme.text()),
        (format!("Jira's “{}” transition is sent; everyone sees it in {}.", t.name, t.to), theme.sev(Severity::Warn)),
        ("It can be moved back the same way. Nothing else about it changes.".to_string(), theme.text()),
    ];
    for (k, (line, style)) in lines.iter().enumerate() {
        p.put(x + 3, y + 2 + k as u16, &tr(line, w as usize - 6), style.patch(surface));
    }
    p.put(x + 3, y + h - 2, "[y] move it     [esc] keep it", theme.accent().add_modifier(Modifier::BOLD).patch(surface));
}
