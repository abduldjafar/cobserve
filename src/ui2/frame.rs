//! The rounded frame of a panel, a tile or an overlay (`03-glyphs.md` *Frames*, `panel()` and
//! `box()` in the simulation): the frame in the panel's severity, the title ` N NAME ` in its top
//! border (a chip when focused), the one-line summary right-aligned at the border's end.

use super::paint::Painter;
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The frame's colour: red or amber for a finding, dim otherwise (colour means severity).
pub fn frame_style(theme: &Theme, sev: Severity) -> Style {
    match sev {
        Severity::Crit | Severity::Warn => theme.sev(sev),
        _ => theme.border(),
    }
}

/// A rounded box's lines, in `style`; nothing for a box under 2×2.
pub fn rounded(p: &mut Painter<'_>, r: Rect, style: Style) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let inner = r.width - 2;
    p.put(r.x, r.y, &format!("╭{}╮", "─".repeat(inner as usize)), style);
    for y in r.y + 1..r.bottom() - 1 {
        p.put(r.x, y, "│", style);
        p.put(r.right() - 1, y, "│", style);
    }
    p.put(r.x, r.bottom() - 1, &format!("╰{}╯", "─".repeat(inner as usize)), style);
}

/// The inside of a panel's frame: two columns and one row in from each side.
pub fn inner(r: Rect) -> Rect {
    Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2))
}

pub struct Title<'a> {
    /// `1`–`8`; none for a tile or a frame without a number.
    pub number: Option<u8>,
    pub name: &'a str,
    pub sev: Severity,
    pub right: &'a str,
    pub focused: bool,
}

/// A panel: frame, title, summary; returns the inside.
pub fn panel(p: &mut Painter<'_>, theme: &Theme, r: Rect, t: &Title<'_>) -> Rect {
    rounded(p, r, frame_style(theme, t.sev));
    if r.width < 6 {
        return inner(r);
    }
    let room = r.width as usize - 3;
    let text = match t.number {
        Some(n) => format!(" {n} {} ", t.name),
        None => format!(" {} ", t.name),
    };
    let text = fmt::truncate(&text, room);
    let title_style = if t.focused {
        theme.tab_active()
    } else if t.sev.is_problem() {
        theme.sev(t.sev).add_modifier(Modifier::BOLD)
    } else {
        theme.strong()
    };
    let after = p.put(r.x + 1, r.y, &text, title_style);
    if !t.right.is_empty() {
        // The summary ends one cell before the corner and never runs into the title.
        let end = r.right() - 2;
        let free = end.saturating_sub(after + 1) as usize;
        if free >= 4 {
            let right = fmt::truncate(&format!(" {} ", t.right), free);
            let style = if t.sev.is_problem() { theme.sev(t.sev) } else { theme.muted() };
            p.putr(end, r.y, &right, style);
        }
    }
    inner(r)
}

/// An overlay's box: its own background, a shadow one cell right and below, a dim frame, the
/// title at column 2 and an optional note in the bottom border (`box()` in the simulation).
pub fn overlay(p: &mut Painter<'_>, theme: &Theme, r: Rect, title: &str, title_style: Style, bottom: &str) {
    let pan = theme.surface().fg(theme.fg);
    p.clear(r, pan);
    if theme.paints_background() {
        p.bg(Rect::new(r.x + 1, r.bottom(), r.width, 1), theme.border);
        p.bg(Rect::new(r.right(), r.y + 1, 1, r.height), theme.border);
    }
    rounded(p, r, theme.border().patch(theme.surface()));
    if !title.is_empty() && r.width > 6 {
        p.put(r.x + 2, r.y, &fmt::truncate(&format!(" {title} "), r.width as usize - 4), title_style.patch(theme.surface()));
    }
    if !bottom.is_empty() && r.width > 10 {
        p.put(r.x + 2, r.bottom() - 1, &fmt::truncate(&format!(" {bottom} "), r.width as usize - 4), theme.muted().patch(theme.surface()));
    }
}
