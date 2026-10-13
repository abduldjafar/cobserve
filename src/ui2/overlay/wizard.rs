//! The setup wizard's box (`screens/120x36-wizard.txt`).

use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{rp, tr, Painter};
use crate::ui2::wizard::{Wizard, STEPS};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub fn draw(p: &mut Painter<'_>, theme: &Theme, wz: &Wizard) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    let step = wz.current();
    let w = sw.saturating_sub(4).min(96);
    let h = (12 + step.fields.len() as u16).min(sh);
    if w < 40 || h < 8 {
        return;
    }
    let x = (sw - w) / 2;
    let y = (sh.saturating_sub(h) / 2).max(2).min(sh - h);
    let title = format!("setup · {} of {} · {}", wz.step + 1, STEPS.len(), step.title);
    frame::overlay(p, theme, Rect::new(x, y, w, h), &title, theme.strong(), "");
    let s = theme.surface();
    let room = w as usize - 6;
    let mut yy = y + 2;
    p.put(x + 3, yy, step.question, theme.strong().patch(s));
    yy += 1;
    for line in step.why {
        p.put(x + 3, yy, &tr(line, room), theme.muted().patch(s));
        yy += 1;
    }
    yy += 1;
    for (i, field) in step.fields.iter().enumerate() {
        let chosen = i == wz.field;
        p.put(x + 3, yy, &rp(field.label, 10), if chosen { theme.accent().add_modifier(Modifier::BOLD).patch(s) } else { theme.muted().patch(s) });
        p.put(x + 13, yy, "› ", theme.accent().patch(s));
        let value = &wz.values[wz.step][i];
        let shown = if field.secret { "•".repeat(value.chars().count()) } else { value.clone() };
        let shown = if field.secret && value.is_empty() && wz.kept[wz.step][i] { "(kept — empty keeps it)".to_string() } else { shown };
        let cursor = if chosen { "█" } else { "" };
        let text = format!("{shown}{cursor}");
        let value_w = crate::fmt::width(&text).max(22) + 2;
        let end = p.put(x + 15, yy, &tr(&text, room.saturating_sub(13)), theme.text().patch(s));
        let _ = end;
        if !field.note.is_empty() {
            let nx = x + 15 + value_w as u16 + 2;
            if nx + 10 < x + w - 2 {
                p.put(nx, yy, &tr(field.note, (x + w - 3 - nx) as usize), theme.muted().patch(s));
            }
        }
        yy += 1;
    }
    yy += 1;
    if let Some((message, sev)) = &wz.message {
        let style = match sev {
            Severity::Ok => theme.sev(Severity::Ok),
            Severity::Warn | Severity::Crit => theme.sev(*sev),
            _ => theme.muted(),
        };
        p.put(x + 3, yy, &tr(message, room), style.patch(s));
    }
    let next = STEPS.get(wz.step + 1).map_or("finish".to_string(), |n| format!("next: {}", n.title));
    let hint = format!("⏎ {next}    tab next field    esc skip this step    ctrl+d a generated fleet");
    p.put(x + 3, y + h - 2, &tr(&hint, room), theme.muted().patch(s));
}
