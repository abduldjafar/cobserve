//! Panel 7, TAPE (`02-panels.md` *7 · TAPE*): two density rows — `✖` and `▲` events a minute over
//! the last hour — beside the newest events. Zoomed it is the tape view itself (`views.rs`).

use crate::app::App;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::glyph::graph::SPK;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;

pub fn title(app: &App) -> (Severity, String) {
    let newest = app.tape.newest_first().next().map_or(Severity::None, |e| e.level);
    let sev = match newest {
        Severity::Crit | Severity::Warn => newest,
        _ => Severity::None,
    };
    (sev, "newest first · n note · y export".to_string())
}

/// Events of `level` a minute over the last hour, `cells` wide, oldest first: `▁` for none, then
/// two levels an event.
pub fn strip(app: &App, level: Severity, cells: usize) -> String {
    let now = app.now() as f64;
    let mut per_minute = [0usize; 60];
    for e in app.tape.newest_first() {
        let ago = (now - e.at) / 60.0;
        if !(0.0..60.0).contains(&ago) {
            continue;
        }
        if e.level == level {
            per_minute[ago as usize] += 1;
        }
    }
    (0..cells)
        .map(|c| {
            let minute = ((cells - 1 - c) * 60 / cells.max(1)).min(59);
            match per_minute[minute] {
                0 => '▁',
                n => SPK[(n * 2).min(7)],
            }
        })
        .collect()
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let i = super::frame(p, app, theme, PanelId::Tape, r, focused);
    if i.width < 4 || i.height == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i, focused));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, focused: bool) {
    let sw = ((i.width as usize * 3) / 10).clamp(10, 30);
    p.put(i.x, i.y, "✖", theme.sev(Severity::Crit));
    p.put(i.x + 2, i.y, &strip(app, Severity::Crit, sw), theme.sev(Severity::Crit));
    if i.height >= 2 {
        p.put(i.x, i.y + 1, "▲", theme.sev(Severity::Warn));
        p.put(i.x + 2, i.y + 1, &strip(app, Severity::Warn, sw), theme.sev(Severity::Warn));
    }
    let x = i.x + sw as u16 + 4;
    let w = (i.x + i.width).saturating_sub(x) as usize;
    if app.tape.is_empty() {
        p.put(x, i.y, &tr("nothing has happened yet", w), theme.muted());
        return;
    }
    let at = app.tape_selection();
    let room = i.height as usize;
    let first = if focused { at.saturating_sub(room.saturating_sub(1)) } else { 0 };
    for (k, e) in app.tape.newest_first().enumerate().skip(first).take(room) {
        let y = i.y + (k - first) as u16;
        if focused && k == at {
            p.band(Rect::new(x - 1, y, (w + 1) as u16, 1), theme.selected());
        }
        let (glyph, style) = match e.level {
            Severity::Crit => ("✖", theme.sev(Severity::Crit)),
            Severity::Warn => ("▲", theme.sev(Severity::Warn)),
            _ => ("●", theme.sev(Severity::Ok)),
        };
        p.put(x, y, glyph, style);
        let time = app.time.local_hm(e.at as i64);
        p.put(x + 2, y, &time, theme.muted());
        let text: String = e.parts.iter().map(|(t, _)| t.as_str()).collect();
        p.put(x + 3 + time.len() as u16, y, &tr(&text, w.saturating_sub(time.len() + 3)), theme.text());
    }
}
