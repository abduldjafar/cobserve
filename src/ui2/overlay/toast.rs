//! Toasts (`02-panels.md` *Toasts*, `toasts()` in the simulation): the last three, newest at the
//! bottom-right just above the status bar, each on the panel's ground with a `▕` at its end.

use crate::fmt;
use crate::theme::Theme;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::state::TOASTS;
use ratatui::layout::Rect;

/// One line a toast: `(text, error)`, oldest first.
pub fn draw(p: &mut Painter<'_>, theme: &Theme, lines: &[(String, bool)]) {
    let screen = p.clip();
    let (w_all, h_all) = (screen.width, screen.height);
    if w_all < 10 || h_all < 4 {
        return;
    }
    for (i, (text, error)) in lines.iter().rev().take(TOASTS).enumerate() {
        let w = (w_all - 4).min(fmt::width(text) as u16 + 3);
        let x = w_all - w - 1;
        let Some(y) = (h_all - 2).checked_sub(i as u16) else { break };
        let surface = theme.surface().fg(theme.fg);
        p.clear(Rect::new(x, y, w, 1), surface);
        let style = if *error { theme.sev(crate::severity::Severity::Crit) } else { theme.text() };
        p.put(x + 1, y, &tr(text, w as usize - 3), style.patch(theme.surface()));
        p.put(x + w - 1, y, "▕", theme.muted().patch(theme.surface()));
    }
}
