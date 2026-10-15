//! What the four redrawn zooms share (`docs/redesign/09-zooms.md` §0): cards, a section rule, the
//! cursor's band, the spinner of a thing running now, the freshness that fades, and lines of
//! spans placed with the `Painter` so nothing crosses a card's side.

use super::widgets::{fit, Cells};
use crate::app::App;
use crate::caps::GraphTier;
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::Painter;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::time::Duration;

/// A thing running now: a frame a tick (`◐ ◓ ◑ ◒`), `| / - \` where the terminal draws ASCII.
pub fn spinner(app: &App) -> &'static str {
    const ROUND: [&str; 4] = ["◐", "◓", "◑", "◒"];
    const ASCII: [&str; 4] = ["|", "/", "-", "\\"];
    let at = (app.now().rem_euclid(4)) as usize;
    if app.caps.graph == GraphTier::Ascii { ASCII[at] } else { ROUND[at] }
}

/// Spans from column `x` of row `y`, cut with `…` to `max` cells; returns the column after them.
pub fn put_spans(p: &mut Painter<'_>, x: u16, y: u16, spans: Vec<Span<'static>>, max: usize) -> u16 {
    let mut col = x;
    for span in fit(spans, max, false) {
        col = p.put(col, y, &span.content, span.style);
    }
    col
}

pub fn put_cells(p: &mut Painter<'_>, x: u16, y: u16, cells: Cells, max: usize) -> u16 {
    put_spans(p, x, y, cells.into_spans(), max)
}

/// A built line, with its own style patched under each span.
pub fn put_line(p: &mut Painter<'_>, x: u16, y: u16, line: Line<'static>, max: usize) -> u16 {
    let base = line.style;
    let spans = line.spans.into_iter().map(|s| Span::styled(s.content, base.patch(s.style))).collect();
    put_spans(p, x, y, spans, max)
}

/// Spans ending just before column `end`; returns where they start.
pub fn put_spans_r(p: &mut Painter<'_>, end: u16, y: u16, spans: Vec<Span<'static>>) -> u16 {
    let width: usize = spans.iter().map(|s| fmt::width(&s.content)).sum();
    let x = end.saturating_sub(width as u16);
    put_spans(p, x, y, spans, width);
    x
}

/// The cursor's row: the selection band from the zoom frame's side to its other side.
pub fn band(p: &mut Painter<'_>, theme: &Theme, i: Rect, y: u16) {
    p.band(Rect::new(i.x.saturating_sub(1), y, i.width + 2, 1), theme.selected());
}

/// A card inside a zoom: a rounded box, ` title ` at column 1 of its top border, a note at the
/// right end of it and another in the bottom border. Returns the inside: one cell in from the
/// sides, past the borders.
pub struct Card {
    pub title: Vec<Span<'static>>,
    pub right: Vec<Span<'static>>,
    pub bottom: Vec<Span<'static>>,
    pub frame: Style,
}

impl Card {
    pub fn new(frame: Style) -> Card {
        Card { title: Vec::new(), right: Vec::new(), bottom: Vec::new(), frame }
    }

    pub fn title(mut self, spans: Vec<Span<'static>>) -> Card {
        self.title = spans;
        self
    }

    pub fn right(mut self, spans: Vec<Span<'static>>) -> Card {
        self.right = spans;
        self
    }

    pub fn bottom(mut self, spans: Vec<Span<'static>>) -> Card {
        self.bottom = spans;
        self
    }

    pub fn draw(self, p: &mut Painter<'_>, r: Rect) -> Rect {
        if r.width < 4 || r.height < 2 {
            return Rect::new(r.x, r.y, 0, 0);
        }
        frame::rounded(p, r, self.frame);
        let room = r.width.saturating_sub(4) as usize;
        let mut after = r.x + 1;
        if !self.title.is_empty() {
            let mut spans = vec![Span::styled(" ", self.frame)];
            spans.extend(self.title);
            spans.push(Span::raw(" "));
            after = put_spans(p, r.x + 1, r.y, spans, room);
        }
        if !self.right.is_empty() {
            let free = (r.right() - 2).saturating_sub(after + 1) as usize;
            let width: usize = self.right.iter().map(|s| fmt::width(&s.content)).sum::<usize>() + 2;
            if free >= width.min(6) {
                let mut spans = vec![Span::raw(" ")];
                spans.extend(self.right);
                spans.push(Span::raw(" "));
                let spans = fit(spans, free, false);
                put_spans_r(p, r.right() - 2, r.y, spans);
            }
        }
        if !self.bottom.is_empty() && r.height >= 3 {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(self.bottom);
            spans.push(Span::raw(" "));
            let spans = fit(spans, room, false);
            put_spans_r(p, r.right() - 2, r.bottom() - 1, spans);
        }
        Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2))
    }
}

/// `─ TITLE · detail ──────` across `w` cells.
pub fn rule(p: &mut Painter<'_>, theme: &Theme, x: u16, y: u16, w: u16, title: Vec<Span<'static>>) {
    let line = super::widgets::rule(w as usize, title, theme);
    put_line(p, x, y, line, w as usize);
}

/// `read 27s ago`, fading as it ages — text2, muted, faint — then `◌ stale 6m` amber: old
/// numbers are a warning. A read that failed says why, as `jira::freshness` does.
pub fn freshness(age: Option<Duration>, error: Option<&str>, reading: bool, theme: &Theme, room: usize) -> Cells {
    if error.is_some() {
        return super::jira::freshness(age, error, reading, theme, room);
    }
    let mut cells = Cells::new();
    if reading {
        cells.push("reading… ", theme.accent());
    }
    match age {
        Some(a) if a.as_secs() >= 300 => {
            cells.push(format!("◌ stale {}", fmt::dur(a.as_secs_f64())), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
        Some(a) => {
            let style = match a.as_secs() {
                0..10 => theme.text2(),
                10..60 => theme.muted(),
                _ => theme.faint(),
            };
            cells.push(format!("read {} ago", fmt::dur(a.as_secs_f64())), style);
        }
        None => {
            cells.push("read —", theme.faint());
        }
    }
    cells
}

/// The one shape of an empty, unconfigured or failed zoom: a mark and a sentence, the reason,
/// the key that fixes it — in the upper third of `r`.
pub fn message(p: &mut Painter<'_>, theme: &Theme, r: Rect, mark: (&str, Style), sentence: &str, lines: &[&str]) {
    if r.width < 8 || r.height == 0 {
        return;
    }
    let widest = lines.iter().map(|l| fmt::width(l)).chain([fmt::width(sentence) + 3]).max().unwrap_or(0) as u16;
    let x = r.x + r.width.saturating_sub(widest) / 2;
    let y = r.y + r.height / 4;
    let room = (r.right() - x) as usize;
    put_spans(p, x, y, vec![Span::styled(format!("{}  ", mark.0), mark.1), Span::styled(sentence.to_string(), theme.strong())], room);
    for (k, line) in lines.iter().enumerate() {
        let style = if k + 1 == lines.len() { theme.faint() } else { theme.muted() };
        put_spans(p, x + 3, y + 1 + k as u16, vec![Span::styled((*line).to_string(), style)], room.saturating_sub(3));
    }
}
