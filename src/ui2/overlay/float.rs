//! The floating session (`\`, `screens/120x36-float-session*.txt`): the chosen session's screen in
//! a box over the panels — 80 % by 72 % of the body, or the whole body full — with the other
//! sessions named in its title. `ctrl+\` starts the prefix: the bottom border names what follows,
//! and after 250 ms the which-key box lists it.

use crate::app::App;
use crate::claude::Sessions;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{rp, tr, Painter};
use crate::ui2::panels::sessions;
use crate::ui2::state::{Float, WHICH_KEY_AFTER};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::SystemTime;

/// The box the pane floats in, for a screen of `w × h` with `top` rows of top bar.
pub fn rect(f: &Float, w: u16, h: u16, top: u16) -> Rect {
    if f.full {
        return Rect::new(1, top, w.saturating_sub(2), h.saturating_sub(top + 2));
    }
    let fw = ((f64::from(w) * 0.8).floor() as u16).max(60).min(w.saturating_sub(2));
    let fh = ((f64::from(h.saturating_sub(top + 2)) * 0.72).floor() as u16).max(10).min(h.saturating_sub(top + 2));
    Rect::new((w - fw) / 2, top + 1, fw, fh)
}

/// The pane's screen inside the box.
pub fn inner(r: Rect) -> Rect {
    Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2))
}

const WHICH: [(&str, &str); 10] = [
    ("1-6", "a panel, the pane hidden"),
    ("7-9 ↑↓", "a session"),
    ("/", "find one by name"),
    ("n", "new session · same kind"),
    ("c o t q", "new claude · opencode · terminal · query"),
    ("f", "full · back to floating"),
    ("r", "rename"),
    ("x x", "close"),
    ("esc", "back to the session"),
    ("ctrl+\\", "hide the pane"),
];

/// The box, its title and hints; the screen itself is drawn after, by `ui2::draw`.
pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, f: &Float, r: Rect) {
    let list = &app.claude;
    let title = match list.current() {
        Some(s) => format!("{} {} · {} · {}", Sessions::number_of(list.active), s.kind.title().to_lowercase(), sessions::place(s), sessions::state(s, app.clock).0),
        None => "no session yet · ⏎ starts Claude here".to_string(),
    };
    frame::overlay(p, theme, r, &title, theme.strong(), "");
    let others: Vec<String> = list
        .order()
        .into_iter()
        .filter(|&i| i != list.active)
        .map(|i| format!("{} {} {}", Sessions::number_of(i), sessions::glyph(list.list[i].kind), list.list[i].kind.title().to_lowercase()))
        .chain(std::iter::once("+ new".to_string()))
        .collect();
    let room = (r.width as usize).saturating_sub(crate::fmt::width(&title) + 8);
    if room > 8 {
        p.putr(r.right() - 2, r.y, &format!(" {} ", tr(&others.join(" · "), room)), theme.muted().patch(theme.surface()));
    }
    let hint = if f.prefix.is_some() {
        "ctrl+\\ · then: 1-6 a panel · 7-9 ↑↓ a session · f full · / find · n new · x x close · esc back"
    } else {
        "ctrl+\\ hide · ctrl+\\ 1-6 a panel · ctrl+\\ 7-9 ↑↓ a session · ctrl+\\ f full · ctrl+\\ / find"
    };
    let style = if f.prefix.is_some() { theme.accent().add_modifier(Modifier::BOLD) } else { theme.muted() };
    p.put(r.x + 2, r.bottom() - 1, &format!(" {} ", tr(hint, (r.width as usize).saturating_sub(6))), style.patch(theme.surface()));
}

/// The which-key box over the pane's screen, once the prefix has waited long enough.
pub fn which(p: &mut Painter<'_>, theme: &Theme, f: &Float, r: Rect, now: SystemTime) {
    if let Some(at) = f.prefix
        && now.duration_since(at).is_ok_and(|d| d >= WHICH_KEY_AFTER)
    {
        let ww = 46.min(r.width.saturating_sub(4));
        let hh = WHICH.len() as u16 + 2;
        if r.height > hh + 2 {
            let b = Rect::new(r.right().saturating_sub(ww + 2), r.bottom().saturating_sub(hh + 1), ww, hh);
            frame::overlay(p, theme, b, "ctrl+\\ then", theme.accent().add_modifier(Modifier::BOLD), "");
            for (k, (key, what)) in WHICH.iter().enumerate() {
                let y = b.y + 1 + k as u16;
                p.put(b.x + 2, y, &rp(key, 9), theme.accent().add_modifier(Modifier::BOLD).patch(theme.surface()));
                p.put(b.x + 12, y, &tr(what, (ww as usize).saturating_sub(14)), theme.text().patch(theme.surface()));
            }
        }
    }
}
