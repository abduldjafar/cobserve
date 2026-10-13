//! The note (`n`, `screens/120x36-note.txt`): one input line on the status bar; `⏎` keeps it on
//! the tape, `esc` drops it.

use crate::theme::Theme;
use crate::ui2::paint::Painter;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub fn draw(p: &mut Painter<'_>, theme: &Theme, note: &str, status: Rect) {
    if status.height == 0 {
        return;
    }
    let (y, w) = (status.y, status.width);
    let surface = theme.surface().fg(theme.fg);
    p.clear(status, surface);
    p.put(1, y, "note › ", theme.accent().add_modifier(Modifier::BOLD).patch(theme.surface()));
    let end = p.putr(w.saturating_sub(1), y, "⏎ keeps it on the tape · esc ", theme.muted().patch(theme.surface()));
    p.within(Rect::new(8, y, end.saturating_sub(9), 1), |p| {
        let after = p.put(8, y, note, theme.text().patch(theme.surface()));
        p.put(after, y, "█", theme.text().patch(theme.surface()));
    });
}
