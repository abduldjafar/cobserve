//! Panel 8, SESSIONS (`02-panels.md` *8 · SESSIONS*): one row a session — the mark of what it runs,
//! its number, its name, its folder and branch, and its state. Zoomed it is today's sessions view
//! (`views.rs`): the list beside the chosen session's screen, opened full with `⏎`.

use crate::app::App;
use crate::claude::{Kind, Session, Sessions};
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The mark of what a session runs (`03-glyphs.md` *Marks*).
pub fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "✻",
        Kind::OpenCode => "▣",
        Kind::Query => "▦",
        Kind::Terminal => "❯",
    }
}

/// A session's state in words, and its severity: it rang (done, or asking) amber; running
/// green; kept or not started dim; ended dim with `✕`.
/// A session's state in a word, as the zoom says it (`views::claude::state_word`): `◐ working`
/// while it prints, `! needs you` when it rang, `○ idle`, `✕ ended`, `↻ kept`.
pub fn state(s: &Session, clock: std::time::SystemTime) -> (String, Severity) {
    let (word, mark, sev, _) = crate::ui2::views::claude::state_word(s, clock);
    (format!("{mark} {word}"), sev)
}

pub fn kind_style(theme: &Theme, kind: Kind) -> Style {
    match kind {
        Kind::Claude => theme.claude(),
        _ => theme.accent(),
    }
}

pub fn title(app: &App) -> (Severity, String) {
    let n = app.claude.list.len();
    let sev = if app.claude.calling() { Severity::Warn } else { Severity::None };
    (sev, format!("{n} · \\ floats one"))
}

/// `~/work/billing ⎇ main`.
pub fn place(s: &Session) -> String {
    match (&s.console, &s.branch) {
        (Some(c), _) => c.node.clone().unwrap_or_else(|| "no server".into()),
        (None, Some(b)) => format!("{} ⎇ {b}", s.dir),
        (None, None) => s.dir.clone(),
    }
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let i = super::frame(p, app, theme, PanelId::Sessions, r, focused);
    if i.width < 6 || i.height == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i, focused));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, focused: bool) {
    let sessions = &app.claude;
    let order = sessions.order();
    let w = i.width as usize;
    if order.is_empty() {
        p.put(i.x, i.y, &tr("no session yet · \\ floats a new Claude · ⏎ the list", w), theme.muted());
        return;
    }
    let room = i.height as usize;
    let at = order.iter().position(|&k| k == sessions.active).unwrap_or(0);
    let first = at.saturating_sub(room / 2).min(order.len().saturating_sub(room));
    for (row, &index) in order.iter().enumerate().skip(first).take(room) {
        let y = i.y + (row - first) as u16;
        let s = &sessions.list[index];
        let chosen = index == sessions.active;
        if focused && chosen {
            p.band(Rect::new(i.x - 1, y, i.width + 2, 1), theme.selected());
        }
        p.put(i.x, y, glyph(s.kind), kind_style(theme, s.kind));
        p.put(i.x + 2, y, &Sessions::number_of(index).to_string(), theme.muted());
        let name_w = 18.min(w.saturating_sub(30));
        p.put(i.x + 5, y, &tr(&s.label(), name_w), if chosen { theme.strong() } else { theme.text() });
        let (word, sev) = state(s, app.clock);
        if w >= 46 {
            p.put(i.x + 24, y, &tr(&place(s), w.saturating_sub(38)), theme.muted());
        }
        let style = match sev {
            Severity::Ok => theme.sev(Severity::Ok),
            Severity::Warn => theme.sev(Severity::Warn).add_modifier(Modifier::BOLD),
            _ => theme.muted(),
        };
        p.putr(i.x + i.width, y, &word, style);
    }
    if room > order.len() {
        p.put(i.x, i.y + order.len() as u16, &tr("+ new session · ↻ past conversations", w), theme.muted());
    }
}
