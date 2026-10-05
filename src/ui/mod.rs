//! Rendering. `draw` reads `App` and nothing else (§8): no I/O, no network, and no panic on
//! resize — every width and height below is a number we clamp, not an assumption.
//!
//! The screen, top to bottom: the chrome on a surface of its own — a shelf across the top, the
//! footer, on view 5 the list of sessions — and the work in the well it leaves.
//!
//! ```text
//!   ◆ cobserve  ▲ degraded       1 nodes   2 queue   3 map   4 tape   5 sessions      ● live 2s   14:12:07 WIB
//!   Subuh 04:21 ━━━━━━ Terbit 05:33 ━━━━━━ Dzuhur 11:45 ━━━●┄┄┄┄ Ashar 14:48 · in 2h52m ┄┄┄┄ Maghrib …  Jakarta
//!
//!   FLEET   8 nodes · 2 hot   mem ━━━━━━━━╺━━━━━  58%  372/640 GiB ▁▂▃▅   cpu …   QUERIES 14 · 3 ✕
//!   REDASH  12 waiting ▁▂▅▇ · oldest 1m43s ▲ · 6 running · ●●●●●● 6/6 workers busy            [2] queue
//!         ✖ clickhouse3      ▲ clickhouse7 lag 12s  ● clickhouse-bi        view 5: a card per node
//!    mem  ━━━━━━━━━━━  93%   ━━━━━━━━╺━━━━━  60%    ━━━━━━━╺━━━━━━  67%
//!    cpu  ━━━━━━━━━━━  95%   ━━━━━╸━━━━━━━━  44%    ━━━━━━━━╸━━━━━  72%
//!
//!   the view: the tree, the queue, the map, the tape or the sessions
//!   ─ INSIGHTS 2 ✖ 4 ▲ ───────────────── view 1 only
//!   ─ the selected row ───────────────── the drawer, 3 lines
//!   ↑↓ move  ⏎ open  tab insights  …                          9 nodes polled · slowest 955 ms · read-only
//! ```
//!
//! The line under the header is the day at the place prayer times are for (`day.rs`), and a
//! prayer's reminder in its place when one is near. Colour comes from `theme.rs`; this module
//! asks for roles, never for colours.

mod band;
mod claude;
mod console;
mod day;
mod drawer;
mod map;
mod nodes;
mod queue;
mod tape;
pub mod widgets;

#[cfg(test)]
mod tests;

use crate::app::{App, Focus, Footer, Hit, View};
use crate::fmt;
use crate::insight;
use crate::severity::Severity;
use crate::theme::{self, Theme};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use widgets::{keycap, rule, Cells};

/// Terminal heights below which the drawer shrinks to its title (§7: below a 30-row terminal),
/// and below which it disappears.
const DRAWER_FULL: u16 = 30;
const DRAWER_TITLE_ONLY: u16 = 24;

/// The terminal's own title, for a tab in the background: how the fleet is and the next prayer —
/// its reminder, when it is near. Minutes, not seconds: a title that changes every second is
/// noise in a tab bar.
pub fn title(app: &App) -> String {
    let mut title = "cobserve".to_string();
    if app.snapshot().is_some() {
        match insight::overall(&app.insights()) {
            Severity::Crit => title.push_str(" ✖"),
            Severity::Warn => title.push_str(" ▲"),
            _ => {}
        }
    }
    let now = app.now();
    match (app.prayer_alert(), app.prayers.schedule.as_ref().and_then(|s| s.next_prayer(now))) {
        (Some(crate::prayer::Alert::Soon { moment, left }), _) => {
            title.push_str(&format!(" · ◷ {} in {} min", moment.name(), (left + 59) / 60));
        }
        (Some(crate::prayer::Alert::Now { moment, .. }), _) => title.push_str(&format!(" · {} now", moment.name())),
        (None, Some(next)) => title.push_str(&format!(" · {} {}", next.name(), app.time.local_hm(next.at))),
        (None, None) => {}
    }
    title
}

/// The terminal height from which view 5 shows a card for every node on the shelf; below it,
/// one line says them in a few words each.
const CARDS_FROM: u16 = 28;

/// From this height the shelf has room to breathe: a blank row above and below what is on it.
const SHELF_PADDED_FROM: u16 = 30;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_with(frame, app, theme::current());
}

/// The margin either side of everything drawn, by the terminal's width.
fn gutter(width: u16) -> u16 {
    if width >= 100 {
        2
    } else if width >= 60 {
        1
    } else {
        0
    }
}

/// Regions of the screen, in drawing order.
#[derive(Debug, Clone, Copy)]
struct Areas {
    masthead: Rect,
    day: Rect,
    /// The shelf, edge to edge, and in it the band and, on view 5, the fleet.
    shelf: Rect,
    band: Rect,
    fleet: Rect,
    /// The well, edge to edge — view 5 lays its list of sessions out from the edge — and inside
    /// the margins.
    well: Rect,
    body: Rect,
    insights: Rect,
    drawer: Rect,
    footer: Rect,
}

fn areas(screen: Rect, view: View, insights_len: usize, body_need: usize) -> Areas {
    let h = screen.height;
    let margin = gutter(screen.width);
    let full = |y: u16, height: u16| Rect::new(screen.x, y, screen.width, height);
    let inset = |r: Rect| Rect::new(r.x + margin, r.y, r.width.saturating_sub(2 * margin), r.height);
    let mut top = screen.y;
    let mut bottom = screen.y + h;

    let footer = if h >= 3 {
        bottom -= 1;
        full(bottom, 1)
    } else {
        full(bottom, 0)
    };
    let masthead = full(top, h.min(1));
    top += masthead.height;
    let day = full(top, if h >= 10 { 1 } else { 0 });
    top += day.height;

    let shelf_top = top;
    let padded = h >= SHELF_PADDED_FROM;
    top += u16::from(padded);
    let band_height = if h >= 14 { 2 } else if h >= 8 { 1 } else { 0 };
    let band = inset(full(top, band_height));
    top += band_height;
    let fleet_height = match view {
        View::Claude if band_height > 0 && h >= CARDS_FROM => 3,
        View::Claude if band_height > 0 && h >= 12 => 1,
        _ => 0,
    };
    top += u16::from(padded && fleet_height == 3);
    let fleet = inset(full(top, fleet_height));
    top += fleet_height;
    top += u16::from(padded && band_height > 0);
    let shelf = full(shelf_top, top - shelf_top);

    // §7: below a 30-row terminal the drawer is its title alone, then nothing.
    let drawer_body = if h >= 34 {
        3
    } else if h >= DRAWER_FULL {
        2
    } else {
        0
    };
    // View 5 gives its sessions every row the shelf leaves; their rows say all there is to say.
    let drawer_height = if view != View::Claude && h >= DRAWER_TITLE_ONLY { 1 + drawer_body } else { 0 };
    bottom = bottom.saturating_sub(drawer_height).max(top);
    let drawer = inset(full(bottom, drawer_height.min(screen.y + h - bottom)));

    // The insights get their usual share, plus whatever the tree does not need.
    let insights_height = if view == View::Nodes && insights_len > 0 && h >= 16 {
        let usual: u16 = if h >= 34 { 5 } else if h >= 30 { 3 } else if h >= 22 { 2 } else { 1 };
        let free = bottom.saturating_sub(top) as usize;
        let spare = free.saturating_sub(body_need + usual as usize + 1);
        let lines = (usual as usize + spare).min(insights_len) as u16;
        1 + lines
    } else {
        0
    };
    bottom = bottom.saturating_sub(insights_height).max(top);
    let insights = inset(full(bottom, insights_height.min(drawer.y.saturating_sub(bottom))));

    let well = full(top, bottom.saturating_sub(top));
    Areas {
        masthead,
        day,
        shelf,
        band,
        fleet,
        well,
        body: inset(well),
        insights,
        drawer,
        footer,
    }
}

pub fn draw_with(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    // What can be clicked is what this frame draws.
    app.viewport.hits.borrow_mut().clear();
    frame.render_widget(Block::new().style(theme.base()), area);
    if area.width < 4 || area.height < 3 {
        return;
    }

    let insights = app.insights();
    let status = if app.snapshot().is_some() {
        insight::overall(&insights)
    } else {
        Severity::None
    };
    let body_need = app.with_rows(|_, rows| rows.len() + 1).unwrap_or(2);
    let a = areas(area, app.view, insights.len(), body_need);
    let margin = gutter(area.width);

    // The chrome's surface: the shelf from the top edge down, and the footer.
    let chrome = Rect::new(area.x, area.y, area.width, a.shelf.y + a.shelf.height - area.y);
    frame.render_widget(Block::new().style(theme.surface()), chrome);
    frame.render_widget(Block::new().style(theme.surface()), a.footer);

    masthead(frame, app, theme, a.masthead, status, margin);
    if a.day.height > 0 {
        day::draw(frame, app, theme, Rect::new(a.day.x + margin, a.day.y, a.day.width.saturating_sub(2 * margin), 1));
    }
    band::draw(frame, app, theme, a.band);
    if a.fleet.height >= 3 {
        claude::fleet_strip(frame, app, theme, a.fleet);
    } else if a.fleet.height > 0 {
        frame.render_widget(Paragraph::new(claude::fleet_line(app, theme, a.fleet)), a.fleet);
    }
    // Without a painted surface, a hairline says where the shelf ends.
    if !theme.paints_background() && a.shelf.height > 0 && a.well.height > 0 {
        let y = a.shelf.y + a.shelf.height - 1;
        if y > a.band.y + a.band.height.saturating_sub(1) && y >= a.fleet.y + a.fleet.height {
            let line = Line::from(Span::styled("─".repeat(a.band.width as usize), theme.rule()));
            frame.render_widget(Paragraph::new(line), Rect::new(a.band.x, y, a.band.width, 1));
        }
    }

    match app.view {
        View::Nodes => nodes::draw_tree(frame, app, theme, a.body, area.width),
        View::Queue => queue::draw(frame, app, theme, a.body),
        View::Map => map::draw(frame, app, theme, a.body),
        View::Tape => tape::draw(frame, app, theme, a.body),
        View::Claude => claude::draw(frame, app, theme, a.well, margin),
    }
    if a.insights.height > 0 {
        nodes::draw_insights(frame, app, theme, a.insights, &insights);
    }
    if a.drawer.height > 0 {
        drawer::draw(frame, app, theme, a.drawer);
    }
    if a.footer.height > 0 {
        let inner = Rect::new(a.footer.x + margin, a.footer.y, a.footer.width.saturating_sub(2 * margin), 1);
        frame.render_widget(Paragraph::new(footer_line(app, theme, inner.width as usize)), inner);
    }
    if app.help {
        draw_help(frame, area, theme);
    }
}

// ---------------------------------------------------------------------------
// The masthead
// ---------------------------------------------------------------------------

/// `◆ cobserve  ▲ degraded      1 nodes  2 queue  3 map  4 tape  5 sessions      ● live 2s  14:12:07 WIB`
///
/// The name and how the fleet is, the tabs, and how fresh the numbers are with the clock — a
/// click on a tab opens it, on the clock flips it between the local zone and UTC. What does not
/// fit gives way in that order: the poll interval, the tabs' names, the status's word.
fn masthead(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, status: Severity, margin: u16) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let room = width.saturating_sub(2 * margin as usize);
    let now = app.now();

    let left = |word: bool| {
        let mut cells = Cells::new();
        cells.push("◆ ", theme.accent());
        cells.push("cobserve", theme.strong());
        if app.snapshot().is_some() {
            let (glyph, sev) = match status {
                Severity::Crit => ("✖", Severity::Crit),
                Severity::Warn => ("▲", Severity::Warn),
                _ => ("✔", Severity::Ok),
            };
            cells.push("  ", Style::default());
            let text = if word { format!(" {glyph} {} ", status.label().to_lowercase()) } else { format!(" {glyph} ") };
            cells.push(text, theme.tint(sev).add_modifier(Modifier::BOLD));
        }
        if !app.tree.filter.trim().is_empty() && word {
            cells.push("  ", Style::default());
            cells.push(format!(" /{} ", app.tree.filter.trim()), theme.keycap());
        }
        cells
    };
    let right = |poll: bool| {
        let mut cells = Cells::new();
        if app.paused {
            cells.push("○ paused", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        } else if app.is_stale() {
            let age = app.data_age().map(|d| fmt::dur(d.as_secs_f64())).unwrap_or_default();
            cells.push(format!("◌ stale {age}"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        } else {
            cells.push("● ", theme.sev(Severity::Ok));
            cells.push("live", theme.text2());
            if poll {
                cells.push(format!(" {}", fmt::dur(app.poll_interval.as_secs_f64())), theme.muted());
            }
        }
        cells.push("   ", Style::default());
        let clock = app.time.hms(now);
        let (hm, s) = clock.split_at(clock.len().saturating_sub(3));
        cells.push(hm.to_string(), theme.strong());
        cells.push(s.to_string(), theme.muted());
        cells.push(format!(" {}", app.time.label(now)), theme.muted());
        cells
    };
    let tabs = |long: bool| {
        let mut cells = Cells::new();
        let mut spots = Vec::new();
        for (i, view) in View::ALL.into_iter().enumerate() {
            if i > 0 {
                cells.push(if long { "   " } else { "  " }, Style::default());
            }
            // A session rang while another view was open: it is done, or it asks.
            let calling = view == View::Claude && app.claude.calling() && app.view != View::Claude;
            let title = view.title().to_lowercase();
            let name = if long { title } else { title[..1].to_string() };
            let start = cells.width();
            if view == app.view {
                cells.push(format!("{} ", view.number()), theme.accent());
                cells.push(name, theme.accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED));
            } else {
                cells.push(format!("{} ", view.number()), theme.faint());
                cells.push(name, if calling { theme.sev(Severity::Warn).add_modifier(Modifier::BOLD) } else { theme.text2() });
            }
            if calling {
                cells.push("●", theme.sev(Severity::Warn));
            }
            spots.push((start, cells.width() - start, view));
        }
        (cells, spots)
    };

    // The widest that fits.
    let options = [(true, true, true), (false, true, true), (false, false, true), (false, false, false)];
    let (mut l, mut r, (mut t, mut spots)) = (left(true), right(true), tabs(true));
    for (poll, long, word) in options {
        (l, r, (t, spots)) = (left(word), right(poll), tabs(long));
        if l.width() + t.width() + r.width() + 6 <= room {
            break;
        }
    }
    let show_tabs = l.width() + t.width() + r.width() + 4 <= room;
    let x0 = area.x + margin;
    let mut line = Cells::new();
    line.spans(l.into_spans());
    let right_at = room.saturating_sub(r.width());
    let mut hits = app.viewport.hits.borrow_mut();
    if show_tabs {
        // The tabs in the middle of the screen when there is room either side, else after the name.
        let centred = (room.saturating_sub(t.width())) / 2;
        let tabs_at = if centred >= line.width() + 3 && centred + t.width() + 3 <= right_at { centred } else { line.width() + 3 };
        line.pad_to(tabs_at);
        for (start, len, view) in spots {
            hits.push((Rect::new(x0 + (tabs_at + start) as u16, area.y, len as u16, 1), Hit::View(view)));
        }
        line.spans(t.into_spans());
    }
    line.pad_to(right_at);
    hits.push((Rect::new(x0 + right_at as u16, area.y, r.width() as u16, 1), Hit::Clock));
    line.spans(r.into_spans());
    drop(hits);
    let inner = Rect::new(x0, area.y, room as u16, 1);
    frame.render_widget(Paragraph::new(line.line(room, Style::default())), inner);
}

/// The footer's right end: how many nodes answered and how fast, and that nothing here writes.
fn footer_status(app: &App, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    if let Some(snapshot) = app.snapshot() {
        let polled = snapshot.nodes.len();
        let slowest = snapshot.nodes.iter().filter_map(|n| n.poll_ms).max();
        let mut text = format!("{} polled", fmt::plural(polled, "node", "nodes"));
        if let Some(ms) = slowest {
            text.push_str(&format!(" · slowest {ms} ms"));
        }
        text.push_str(" · ");
        cells.push(text, theme.faint());
    }
    cells.push("read-only", theme.faint());
    cells
}

// ---------------------------------------------------------------------------
// Footer and help (§3)
// ---------------------------------------------------------------------------

fn footer_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    match app.footer {
        Footer::Filter => {
            cells.push(" / ", theme.keycap());
            cells.push(format!(" {}", app.filter_input().unwrap_or_default()), theme.strong());
            cells.push("▏", theme.accent());
            let matches = app
                .with_rows(|_, rows| rows.iter().filter(|r| r.depth > 0).count())
                .unwrap_or(0);
            cells.push(format!("   {matches} matching rows"), theme.muted());
            cells.push("   ", Style::default());
            cells.spans(keycap("⏎", "keep", theme));
            cells.push("  ", Style::default());
            cells.spans(keycap("esc", "clear", theme));
            return cells.line(width, Style::default());
        }
        Footer::Help => {
            cells.spans(keycap("?", "close", theme));
            cells.push("   any other key goes back", theme.muted());
            return cells.line(width, Style::default());
        }
        Footer::Keys => {}
    }

    // What follows the keys, when there is room for it.
    let mut tail: Option<String> = None;
    // What the session on screen runs, for what its keys say.
    let kind = app.claude.current().map_or(crate::claude::Kind::Claude, |s| s.kind);
    let keys: &[(&str, &str)] = match app.view {
        View::Nodes if app.focus == Focus::Insights => &[
            ("↑↓", "choose"),
            ("⏎", "go there"),
            ("tab", "back to the tree"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Nodes => &[
            ("↑↓", "move"),
            ("⏎", "open"),
            ("c", "SQL on it"),
            ("←→", "fold"),
            ("tab", "insights"),
            ("u", "pivot"),
            ("/", "filter"),
            ("s", "sort"),
            ("␣", "healthy"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Queue => &[
            ("↑↓", "move"),
            ("⏎", "jump to ClickHouse"),
            ("J K", "scroll SQL"),
            ("1", "nodes"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Map => &[
            ("←→↑↓", "move"),
            ("⏎", "open in view 1"),
            ("c", "SQL on it"),
            ("s", "sort"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Tape => &[
            ("↑↓", "scroll"),
            ("⏎", "go to it"),
            ("1", "nodes"),
            ("p", "pause"),
            ("?", "help"),
            ("q", "quit"),
        ],
        View::Claude if matches!(app.claude.mode, crate::claude::Mode::Naming(_)) => &[
            ("⏎", "keep the name"),
            ("esc", "cancel"),
        ],
        View::Claude if matches!(app.claude.mode, crate::claude::Mode::Finding { .. }) => {
            tail = Some("type a name, a folder, a kind, a server or a number".into());
            &[("↑↓", "choose"), ("⏎", "go there"), ("esc", "back")]
        }
        View::Claude if matches!(&app.claude.mode, crate::claude::Mode::Opening(p) if p.kind == crate::claude::Kind::Query) => {
            tail = Some("type to search · shift+tab: what it runs".into());
            &[("↑↓", "choose"), ("⏎", "open it on that server"), ("esc", "cancel")]
        }
        View::Claude if matches!(&app.claude.mode, crate::claude::Mode::Opening(p) if p.searching()) => {
            tail = Some("a click on a folder goes into it".into());
            &[("↑↓", "choose"), ("⏎", "open the session there"), ("→", "go in"), ("esc", "clear the search")]
        }
        View::Claude if matches!(app.claude.mode, crate::claude::Mode::Opening(_)) => {
            tail = Some("type to search · a click on a folder goes into it".into());
            &[("↑↓", "choose"), ("⏎", "open the session there"), ("→", "go in"), ("←", "up"), ("esc", "cancel")]
        }
        View::Claude if app.claude.mode == crate::claude::Mode::Bar => {
            // The bar Claude runs under, its ctrl+\ lit and the way back at its end: nothing
            // else moves.
            cells.push(" ctrl+\\ ", theme.tab_active());
            cells.push(" then", theme.muted());
            &[
                ("1-4", "views"),
                ("5-9 ↑↓", "sessions"),
                ("/", "find one"),
                ("n", "new"),
                ("q", "SQL"),
                ("r", "rename"),
                ("x", "close"),
                ("esc", "back"),
                ("ctrl+\\", "monitor"),
                ("p", "past"),
            ]
        }
        View::Claude if app.claude.current().is_some_and(|s| s.console.is_some()) => {
            use crate::console::{Assistant, Focus as Keys};
            match app.claude.current().and_then(|s| s.console.as_deref()) {
                Some(c) if c.choosing.is_some() => &[("↑↓", "choose"), ("⏎", "run on it"), ("esc", "keep this one")],
                Some(c) if c.focus == Keys::Answer => {
                    &[("↑↓←→", "move"), ("y", "copy the row"), ("Y", "copy it all"), ("PgUp PgDn", "a page"), ("tab", "back to the SQL")]
                }
                Some(c) if c.suggest.is_some() => {
                    tail = Some("a click takes one too".into());
                    &[("tab", "take it"), ("↑↓", "choose"), ("esc", "close them")]
                }
                Some(c) if c.assistant == Assistant::OpenCode => &[
                    ("ctrl+r", "run"),
                    ("tab", "complete"),
                    ("ctrl+k", "ask OpenCode"),
                    ("ctrl+t", "Claude instead"),
                    ("ctrl+o", "server"),
                    ("⇧tab", "the answer"),
                    ("ctrl+\\", "sessions"),
                ],
                _ => &[
                    ("ctrl+r", "run"),
                    ("tab", "complete"),
                    ("ctrl+k", "ask Claude"),
                    ("ctrl+t", "OpenCode instead"),
                    ("ctrl+o", "server"),
                    ("⇧tab", "the answer"),
                    ("ctrl+\\", "sessions"),
                ],
            }
        }
        View::Claude if app.claude.is_running() => {
            // Every other key is Claude's; the one that is not leads the bar.
            cells.push(" ctrl+\\ ", theme.keycap());
            cells.push(" then", theme.muted());
            tail = Some(format!("every other key goes to {}", kind.listener()));
            &[
                ("1-4", "views"),
                ("5-9 ↑↓", "sessions"),
                ("/", "find one"),
                ("n", "new"),
                ("r", "rename"),
                ("x", "close"),
                ("F1-F9", "any tab"),
            ]
        }
        View::Claude => match kind {
            crate::claude::Kind::Claude => &[("⏎", "start Claude"), ("ctrl+\\", "sessions"), ("1", "nodes"), ("?", "help"), ("q", "quit")],
            crate::claude::Kind::OpenCode => &[("⏎", "start OpenCode"), ("ctrl+\\", "sessions"), ("1", "nodes"), ("?", "help"), ("q", "quit")],
            crate::claude::Kind::Terminal => &[("⏎", "start the shell"), ("ctrl+\\", "sessions"), ("1", "nodes"), ("?", "help"), ("q", "quit")],
            crate::claude::Kind::Query => &[("ctrl+\\", "sessions"), ("1", "nodes"), ("?", "help"), ("q", "quit")],
        },
    };
    // A notice comes before the keys' last ones; nothing else does.
    let notice = app.notice().filter(|n| !n.trim().is_empty()).map(|notice| {
        let mut cells = Cells::new();
        cells.push("▲ ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        cells.push(fmt::truncate(notice, (width * 3 / 5).saturating_sub(2)), theme.sev(Severity::Warn));
        cells
    });
    let keys_room = width.saturating_sub(notice.as_ref().map_or(0, |n| n.width() + 3));
    for (key, label) in keys {
        let mut piece = Cells::new();
        if cells.width() > 0 {
            piece.push("  ", Style::default());
        }
        piece.spans(keycap(key, label, theme));
        // A key that does not fit is left out whole, never cut in half.
        if cells.width() + piece.width() > keys_room {
            break;
        }
        cells.spans(piece.into_spans());
    }
    if let Some(notice) = notice {
        cells.pad_to(width.saturating_sub(notice.width()));
        cells.spans(notice.into_spans());
        return cells.line(width, Style::default());
    }
    // Then what the keys go to, then how the poll went — each where it fits.
    if let Some(tail) = tail.map(|t| format!("   {t}"))
        && cells.width() + fmt::width(&tail) <= width
    {
        cells.push(tail, theme.muted());
    }
    let status = footer_status(app, theme);
    if cells.width() + status.width() + 3 <= width {
        cells.pad_to(width.saturating_sub(status.width()));
        cells.spans(status.into_spans());
    }
    cells.line(width, Style::default())
}

/// `?` — the keymap of §3 and what every glyph on screen means.
const HELP_KEYS: &[(&str, &str)] = &[
    ("↑ ↓  j k", "move the cursor · PgUp PgDn Home End jump"),
    ("⏎", "open / close · on an insight, a queue job, a tile or a tape line: go there"),
    ("← →  h l", "collapse / expand, vim-style"),
    ("J K  ⇧↑↓", "scroll the SQL under the selected query or Redash job"),
    ("tab", "move between the tree and the insights"),
    ("space", "fold / unfold the healthy nodes"),
    ("u", "pivot node ↔ user: who is burning the fleet"),
    ("c", "SQL on the node under the cursor, in a query session: read-only, it suggests"),
    ("s", "sort: pressure, memory, CPU, name"),
    ("/", "filter by node, user, person, SQL or query id · esc clears"),
    ("1 2 3 4 5", "views: nodes · queue · map · tape · sessions — up to 50, the first five on 5-9"),
    ("ctrl+\\", "the sessions · then 1-4 a view, 5-9 or ↑↓ a session, / one by name, n new (c o t q:"),
    ("", "Claude, OpenCode, a terminal, SQL), p a past conversation, r rename, x close"),
    ("F1 … F9", "the same tabs from anywhere, a session's screen too · or click them"),
    ("z", "the clock: the local zone ↔ UTC · or click it · ctrl+\\ z in a session"),
    ("d", "wave a prayer's reminder away · ctrl+\\ d in a session · or click ✕"),
    ("p", "pause: the numbers stop, the clock does not"),
    ("q  ctrl-c", "quit"),
];

const HELP_LEGEND: &[(&str, &str)] = &[
    ("✖ ▲", "critical (red) · warning (amber), by DESIGN.md §7"),
    ("✕", "runaway query: ≥ 30 s, or ≥ 80% of its own memory limit"),
    ("↯", "node not answering — its numbers are unknown, not zero"),
    ("↗ ↑  ↘ ↓", "rising · rising fast · falling, over the last minute"),
    ("▁▂▃▅▇", "the last 4 minutes, each cell its worst moment"),
    ("NEW", "joined the fleet during this session"),
    ("━━●┄┄", "the day: prayer times at the place, ● now · PRAYER_CITY or PRAYER_AT sets where"),
    ("↻", "a session kept from the last run: it takes its conversation up when opened"),
];

fn draw_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let width = 92.min(area.width.saturating_sub(4)).max(20);
    let height = (HELP_KEYS.len() + HELP_LEGEND.len() + 7) as u16;
    let height = height.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let inner_width = width.saturating_sub(4) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let row = |key: &str, what: &str, key_style: Style| -> Line<'static> {
        let mut cells = Cells::new();
        cells.cell_right(key, 11, key_style);
        cells.push("  ", Style::default());
        cells.push(what.to_string(), theme.text());
        cells.line_unpadded(inner_width)
    };
    lines.push(rule(inner_width, vec![Span::styled("KEYS", theme.section())], theme));
    for (key, what) in HELP_KEYS {
        lines.push(row(key, what, theme.accent().add_modifier(Modifier::BOLD)));
    }
    lines.push(rule(inner_width, vec![Span::styled("ON SCREEN", theme.section())], theme));
    for (glyph, what) in HELP_LEGEND {
        lines.push(row(glyph, what, theme.strong()));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "read-only by design: nothing here can kill or change a query",
        theme.muted(),
    )));
    frame.render_widget(Clear, popup);
    let card = if theme.paints_background() {
        Block::new().style(theme.base().patch(theme.raised()))
    } else {
        Block::bordered().border_style(theme.accent())
    };
    let title = Line::from(vec![Span::styled(" ◆ ", theme.accent()), Span::styled("keys", theme.strong()), Span::styled(" · ? closes ", theme.muted())]);
    frame.render_widget(
        Paragraph::new(lines).block(card.title_top(title).padding(ratatui::widgets::Padding::new(1, 1, 1, 0))),
        popup,
    );
}

/// For the tests: the insights the frame would draw.
#[cfg(test)]
fn insights_of(app: &App) -> Vec<crate::insight::Insight> {
    app.insights()
}
