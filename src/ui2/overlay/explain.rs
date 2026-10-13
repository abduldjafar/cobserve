//! The explain popover (`e`, `02-panels.md` *Overlays*; `screens/120x36-explain-*.txt`): the
//! formula, the same formula with the values in it, the inputs with the system table each came
//! from, the poll, and the `DESIGN.md` reference — all from the metric's `Provenance`.

use crate::app::App;
use crate::fmt;
use crate::provenance::Provenance;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{lp, rp, tr, Painter};
use crate::ui2::state::Explained;
use ratatui::layout::Rect;

/// The popover's lines for `prov`; `polled` says which poll and from where.
pub fn build(prov: &Provenance, polled: &str) -> Explained {
    let value = prov.value.map_or("—".to_string(), |v| prov.unit.show(v));
    let mut lines: Vec<(String, bool)> = Vec::new();
    let head = format!("{} = ", prov.quantity);
    lines.push((format!("{head}{}", prov.formula), false));
    lines.push((format!("{}= {}", " ".repeat(fmt::width(&head) - 2), prov.with_values()), false));
    lines.push((String::new(), true));
    for i in &prov.inputs {
        lines.push((format!("{} {}   {}", rp(i.name, 13), lp(&i.unit.show(i.value), 10), i.source), true));
    }
    if let Some(note) = &prov.note {
        lines.push((note.clone(), true));
    }
    if !polled.is_empty() {
        lines.push((polled.to_string(), true));
    }
    lines.push((String::new(), true));
    lines.push((prov.reference.to_string(), true));
    let text = std::iter::once(prov.metric.clone()).chain(lines.iter().map(|(l, _)| l.clone())).collect::<Vec<_>>().join("\n");
    Explained { title: format!("explain · {value}"), lines, text }
}

/// `polled 2 s ago · host:8123 · answered in 0.6 s`.
pub fn polled(app: &App, node: Option<&crate::model::NodeSnapshot>) -> String {
    let ago = app.data_age().map_or(String::new(), |d| format!("polled {} ago", fmt::dur(d.as_secs_f64())));
    let mut parts = vec![ago];
    if let Some(n) = node {
        if !n.host.is_empty() {
            parts.push(format!("{}:{}", n.host, n.port));
        }
        if let Some(ms) = n.poll_ms {
            parts.push(format!("answered in {:.1} s", f64::from(ms) / 1000.0));
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

pub fn draw(p: &mut Painter<'_>, theme: &Theme, e: &Explained) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    if sw < 30 || sh < 8 {
        return;
    }
    let w = sw.saturating_sub(6).min(84);
    let x = (sw / 4).max(1).min(sw - w - 1);
    let h = (e.lines.len() as u16 + 2).min(sh - 2);
    let y = (sh / 3).min(sh.saturating_sub(h + 2)).max(3.min(sh - h));
    let r = Rect::new(x, y, w, h);
    frame::overlay(p, theme, r, &e.title, theme.strong(), "⏎ ask Claude about it · y copy · esc");
    let surface = theme.surface();
    for (k, (line, dim)) in e.lines.iter().enumerate().take(h as usize - 2) {
        let style = if *dim { theme.muted() } else { theme.text() };
        p.put(x + 2, y + 1 + k as u16, &tr(line, w as usize - 4), style.patch(surface));
    }
}
