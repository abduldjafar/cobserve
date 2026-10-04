//! Rendering. `draw` reads `App` and nothing else (§8): no I/O, no network, and no panic on
//! resize — every width and height below is a number we clamp, not an assumption.
//!
//! The frame, top to bottom:
//!
//! ```text
//! ╭ ◆ FLEETLENS ▲ DEGRADED ─────────────── ● LIVE 2s · 15:32:28 UTC  1 NODES 2 QUEUE 3 MAP 4 TAPE ╮
//! │ FLEET 8 nodes · 2 hot   MEM ▕████▍ ▏ 58.1% 372/640 GiB ▁▂▃▅   CPU …   QUERIES 14 · 3 ✕       │
//! │ REDASH 12 waiting ▁▂▅▇ · oldest 1m43s ▲ · workers ●●●●●● 6/6 busy · 2 failed/5m  [2] queue │
//! │ ──────────────────────────────────────────────────────────────────────────────────────── │
//! │ the view: the tree, the queue, the map or the tape                                         │
//! │ ─ INSIGHTS 2 ✖ 4 ▲ ─────────────────── (view 1 only)                                       │
//! │ ─ the selected row ─────────────────── the drawer, 3 lines                                 │
//! │  ↑↓ move  ⏎ open  tab insights  …                                    the keys that work now │
//! ╰──────────────────────────────────────────────────────────────── updated 1s ago · read-only ╯
//! ```
//!
//! Colour comes from `theme.rs`; this module asks for roles, never for colours.

mod band;
mod claude;
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
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui::Frame;
use widgets::{keycap, pill, rule, Cells};

/// Height of the frame's inner area below which the drawer shrinks to its title (§7: below a
/// 30-row terminal), and below which it disappears.
const DRAWER_FULL: u16 = 28;
const DRAWER_TITLE_ONLY: u16 = 22;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_with(frame, app, theme::current());
}

/// Regions of the frame, in drawing order.
#[derive(Debug, Clone, Copy)]
struct Areas {
    band: Rect,
    band_rule: Rect,
    body: Rect,
    insights: Rect,
    drawer: Rect,
    footer: Rect,
}

fn areas(content: Rect, view: View, insights_len: usize, body_need: usize) -> Areas {
    let h = content.height;
    let row = |y: u16, height: u16| Rect::new(content.x, y, content.width, height);
    let mut top = content.y;
    let mut bottom = content.y + h;

    let footer = if h >= 3 {
        bottom -= 1;
        row(bottom, 1)
    } else {
        row(bottom, 0)
    };

    let band_height = if h >= 12 { 2 } else if h >= 6 { 1 } else { 0 };
    let band = row(top, band_height);
    top += band_height;
    let band_rule = if band_height > 0 && h >= 12 {
        top += 1;
        row(top - 1, 1)
    } else {
        row(top, 0)
    };

    // §7: below a 30-row terminal the drawer is its title alone, then nothing.
    let drawer_body = if h >= 32 {
        3
    } else if h >= DRAWER_FULL {
        2
    } else {
        0
    };
    // View 5 gives Claude every row the band leaves; its rows say all there is to say.
    let drawer_height = if view != View::Claude && h >= DRAWER_TITLE_ONLY { 1 + drawer_body } else { 0 };
    bottom = bottom.saturating_sub(drawer_height);
    let drawer = row(bottom, drawer_height);

    // The insights get their usual share, plus whatever the tree does not need.
    let insights_height = if view == View::Nodes && insights_len > 0 && h >= 14 {
        let usual: u16 = if h >= 32 { 5 } else if h >= 28 { 3 } else if h >= 20 { 2 } else { 1 };
        let free = bottom.saturating_sub(top) as usize;
        let spare = free.saturating_sub(body_need + usual as usize + 1);
        let lines = (usual as usize + spare).min(insights_len) as u16;
        1 + lines
    } else {
        0
    };
    bottom = bottom.saturating_sub(insights_height);
    let insights = row(bottom, insights_height);

    let body = row(top, bottom.saturating_sub(top));
    Areas {
        band,
        band_rule,
        body,
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

    // A notice on the bottom border comes before the counts beside it: they make way for it,
    // and what is still too long is cut with … rather than run into them.
    let mut right = bottom_right(app, theme, true);
    let room = |right: &Line<'_>| (area.width as usize).saturating_sub(2 + right.width() + 2);
    if app.notice().is_some_and(|n| fmt::width(n) + 4 > room(&right)) {
        right = bottom_right(app, theme, false);
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border())
        .title_top(header_left(app, theme, status, area.width))
        .title_top(header_right(app, theme, area.width).right_aligned())
        .title_bottom(bottom_left(app, theme, room(&right)))
        .title_bottom(right.right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    tab_hits(frame, app, area);
    // One cell of breathing room on each side, when there is room to breathe.
    let content = if inner.width > 40 {
        Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height)
    } else {
        inner
    };
    if content.width == 0 || content.height == 0 {
        return;
    }

    let body_need = app.with_rows(|_, rows| rows.len() + 1).unwrap_or(2);
    let a = areas(content, app.view, insights.len(), body_need);

    band::draw(frame, app, theme, a.band);
    if a.band_rule.height > 0 {
        let line = if app.view == View::Claude {
            // On view 5 the rule carries the worst thing in the fleet.
            claude::watch_line(&insights, theme, a.band_rule.width as usize)
        } else {
            Line::from(Span::styled("─".repeat(a.band_rule.width as usize), theme.rule()))
        };
        frame.render_widget(Paragraph::new(line), a.band_rule);
    }
    match app.view {
        View::Nodes => nodes::draw_tree(frame, app, theme, a.body, area.width),
        View::Queue => queue::draw(frame, app, theme, a.body),
        View::Map => map::draw(frame, app, theme, a.body),
        View::Tape => tape::draw(frame, app, theme, a.body),
        View::Claude => claude::draw(frame, app, theme, a.body),
    }
    if a.insights.height > 0 {
        nodes::draw_insights(frame, app, theme, a.insights, &insights);
    }
    if a.drawer.height > 0 {
        drawer::draw(frame, app, theme, a.drawer);
    }
    if a.footer.height > 0 {
        frame.render_widget(Paragraph::new(footer_line(app, theme, a.footer.width as usize)), a.footer);
    }
    if app.help {
        draw_help(frame, area, theme);
    }
}

// ---------------------------------------------------------------------------
// Header and border
// ---------------------------------------------------------------------------

/// The header's tabs, found where the border drew them, so a click on one opens it.
fn tab_hits(frame: &mut Frame, app: &App, area: Rect) {
    let buffer = frame.buffer_mut();
    let row: Vec<String> = (area.x..area.x + area.width).map(|x| buffer[(x, area.y)].symbol().to_string()).collect();
    let mut hits = app.viewport.hits.borrow_mut();
    for view in View::ALL {
        let long = format!("{} {}", view.number(), view.title());
        let short = format!("{}{}", view.number(), &view.title()[..1]);
        for label in [long, short] {
            let wanted: Vec<String> = label.chars().map(|c| c.to_string()).collect();
            if let Some(at) = row.windows(wanted.len()).position(|w| w == wanted.as_slice()) {
                // The space either side is part of the tab.
                let x = (area.x + at as u16).saturating_sub(1);
                hits.push((Rect::new(x, area.y, wanted.len() as u16 + 2, 1), Hit::View(view)));
                break;
            }
        }
    }
}

fn header_left(app: &App, theme: &Theme, status: Severity, width: u16) -> Line<'static> {
    let mut spans = vec![
        Span::styled(" ◆ ", theme.accent()),
        Span::styled("FLEETLENS", theme.strong()),
        Span::raw(" "),
    ];
    if app.snapshot().is_some() && width >= 60 {
        let glyph = match status {
            Severity::Crit => "✖",
            Severity::Warn => "▲",
            _ => "✔",
        };
        let sev = if status.is_problem() { status } else { Severity::Ok };
        spans.push(pill(&format!("{glyph} {}", status.label()), sev, theme));
        spans.push(Span::raw(" "));
    }
    if !app.tree.filter.trim().is_empty() && width >= 90 {
        spans.push(Span::styled(format!(" /{} ", app.tree.filter.trim()), theme.keycap()));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn header_right(app: &App, theme: &Theme, width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    let poll = app.poll_interval.as_secs_f64();
    if app.paused {
        spans.push(Span::styled(" ○ PAUSED", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)));
    } else if app.is_stale() {
        let age = app.data_age().map(|d| fmt::dur(d.as_secs_f64())).unwrap_or_default();
        spans.push(Span::styled(format!(" ◌ STALE {age}"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" ● LIVE", theme.sev(Severity::Ok).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {}", fmt::dur(poll)), theme.muted()));
    }
    spans.push(Span::styled(" · ", theme.faint()));
    spans.push(Span::styled(fmt::utc_clock(app.clock), theme.text()));
    spans.push(Span::styled(" UTC ", theme.muted()));
    if width >= 80 {
        for view in View::ALL {
            spans.push(Span::raw(" "));
            // A Claude session rang while another view was open: it is done, or it asks.
            let calling = view == View::Claude && app.claude.calling() && app.view != View::Claude;
            let mark = if calling { "●" } else { "" };
            let label = if width >= 110 {
                format!(" {} {}{mark} ", view.number(), view.title())
            } else {
                format!(" {}{}{mark} ", view.number(), &view.title()[..1])
            };
            if view == app.view {
                spans.push(Span::styled(label, theme.tab_active()));
            } else if calling {
                spans.push(Span::styled(label, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)));
            } else {
                spans.push(Span::styled(label, theme.muted()));
            }
        }
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// The latest notice, in `room` cells.
fn bottom_left(app: &App, theme: &Theme, room: usize) -> Line<'static> {
    match app.notice() {
        Some(notice) if room > 8 => Line::from(vec![
            Span::styled(" ▲ ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} ", fmt::truncate(notice, room - 4)), theme.sev(Severity::Warn)),
        ]),
        _ => Line::from(""),
    }
}

/// How many nodes answered and how fast, when `counts`, and that nothing here writes.
fn bottom_right(app: &App, theme: &Theme, counts: bool) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(snapshot) = app.snapshot().filter(|_| counts) {
        let polled = snapshot.nodes.len();
        let slowest = snapshot.nodes.iter().filter_map(|n| n.poll_ms).max();
        let mut text = format!(" {} polled", fmt::plural(polled, "node", "nodes"));
        if let Some(ms) = slowest {
            text.push_str(&format!(" · slowest {ms} ms"));
        }
        spans.push(Span::styled(text, theme.faint()));
        spans.push(Span::styled(" · ", theme.faint()));
    } else {
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("read-only ", theme.faint()));
    Line::from(spans)
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
    let mut tail = None;
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
        View::Claude if matches!(app.claude.mode, crate::claude::Mode::Opening(_)) => &[
            ("⏎", "open it there"),
            ("esc", "cancel"),
            ("ctrl+u", "clear"),
        ],
        View::Claude if app.claude.mode == crate::claude::Mode::Bar => {
            // The bar Claude runs under, its ctrl+\ lit and the way back at its end: nothing
            // else moves.
            cells.push(" ctrl+\\ ", theme.tab_active());
            cells.push(" then", theme.muted());
            &[
                ("1-4", "views"),
                ("5-9", "sessions"),
                ("n", "new"),
                ("r", "rename"),
                ("x", "close"),
                ("ctrl+\\", "monitor"),
                ("esc", "back to Claude"),
            ]
        }
        View::Claude if app.claude.is_running() => {
            // Every other key is Claude's; the one that is not leads the bar.
            cells.push(" ctrl+\\ ", theme.keycap());
            cells.push(" then", theme.muted());
            tail = Some("every other key goes to Claude");
            &[
                ("1-4", "views"),
                ("5-9", "sessions"),
                ("n", "new"),
                ("r", "rename"),
                ("x", "close"),
                ("F1-F9", "any tab"),
            ]
        }
        View::Claude => &[
            ("⏎", "start Claude"),
            ("ctrl+\\", "sessions"),
            ("1", "nodes"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };
    for (key, label) in keys {
        let mut piece = Cells::new();
        if cells.width() > 0 {
            piece.push("  ", Style::default());
        }
        piece.spans(keycap(key, label, theme));
        // A key that does not fit is left out whole, never cut in half.
        if cells.width() + piece.width() > width {
            break;
        }
        cells.spans(piece.into_spans());
    }
    if let Some(tail) = tail.map(|t| format!("   {t}"))
        && cells.width() + fmt::width(&tail) <= width
    {
        cells.push(tail, theme.muted());
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
    ("s", "sort: pressure, memory, CPU, name"),
    ("/", "filter by node, user, person, SQL or query id · esc clears"),
    ("1 2 3 4 5", "views: nodes · queue · map · tape · claude — 5 to 9 are Claude's sessions"),
    ("ctrl+\\", "Claude · there, then 1-4 a view, 5-9 a session, n new, r rename, x close"),
    ("F1 … F9", "the same tabs from anywhere, Claude's screen too · or click them"),
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
    lines.push(Line::from(Span::styled(
        "not in this pass: k kill, a ask Klikas, mouse",
        theme.faint(),
    )));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(theme.accent())
                .title_top(Line::from(Span::styled(" keys · ? closes ", theme.strong())))
                .style(theme.base().bg(theme.panel)),
        ),
        popup,
    );
}

/// For the tests: the insights the frame would draw.
#[cfg(test)]
fn insights_of(app: &App) -> Vec<crate::insight::Insight> {
    app.insights()
}
