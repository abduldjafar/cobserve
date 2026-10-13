//! The new shell's key router (`04-architecture.md` *Events*): overlays first — here, the palette
//! — then the `Action` table, a panel's own actions before the global ones. A key no action lists
//! does nothing; a key an action lists always does what it says.

use crate::action;
use crate::app::App;
use crate::ui2::overlay::palette::{self, CHANNELS};
use crate::ui2::state::Palette;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The name a key has in the `Action` table: `e`, `Enter`, `ctrl+c`, `F1`.
pub fn name(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char(c) if ctrl => format!("ctrl+{}", c.to_ascii_lowercase()),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => return None,
    })
}

pub fn on_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    if app.shell.palette.is_some() {
        return palette_key(app, key);
    }
    if let Some(e) = app.shell.explain.take() {
        // Explain: ⏎ asks Claude, y copies, anything else closes.
        match key.code {
            KeyCode::Char('y') => {
                app.copy(e.text);
                app.shell.toast("✔ formula and inputs copied", false, std::time::SystemTime::now());
            }
            KeyCode::Enter => app.shell.toast("⏎ hands it to the floating Claude — phase 4", false, std::time::SystemTime::now()),
            _ => {}
        }
        return;
    }
    if app.shell.filter.as_ref().is_some_and(|f| f.typing) {
        return filter_key(app, key);
    }
    let Some(name) = name(&key) else { return };
    if let Some(a) = action::for_key(app, &name) {
        (a.run)(app, &name);
    }
}

/// Typing into `/`: printable keys add, `⌫` takes back, `⏎` keeps it, `esc` clears it.
fn filter_key(app: &mut App, key: KeyEvent) {
    let Some(f) = app.shell.filter.as_mut() else { return };
    match key.code {
        KeyCode::Esc => app.shell.filter = None,
        KeyCode::Enter => {
            f.typing = false;
            if f.query.is_empty() {
                app.shell.filter = None;
            }
        }
        KeyCode::Backspace => {
            f.query.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => f.query.push(c),
        _ => {}
    }
    // The chosen node stays chosen only while it still matches.
    if let Some(first) = crate::ui2::nav::node_names(app).first().cloned()
        && !crate::ui2::nav::node_names(app).iter().any(|n| Some(n) == app.shell.chosen.as_ref())
    {
        app.shell.chosen = Some(first);
    }
}

/// Open the palette on the actions channel, or the keys box.
pub fn open_palette(app: &mut App, help: bool) {
    app.shell.palette = Some(Palette { help, ..Palette::default() });
}

fn palette_key(app: &mut App, key: KeyEvent) {
    let Some(mut pal) = app.shell.palette.take() else { return };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => return,
        KeyCode::Char('c') if ctrl => return,
        KeyCode::Enter => {
            let chosen = palette::matches(app, &pal).into_iter().nth(pal.sel).map(|(item, _)| item.target);
            if let Some(target) = chosen {
                palette::run(app, target);
            }
            return;
        }
        KeyCode::Tab => {
            pal.channel = (pal.channel + 1) % CHANNELS.len();
            pal.sel = 0;
        }
        KeyCode::BackTab => {
            pal.channel = (pal.channel + CHANNELS.len() - 1) % CHANNELS.len();
            pal.sel = 0;
        }
        KeyCode::Down => pal.sel += 1,
        KeyCode::Up => pal.sel = pal.sel.saturating_sub(1),
        KeyCode::PageDown => pal.sel += 8,
        KeyCode::PageUp => pal.sel = pal.sel.saturating_sub(8),
        KeyCode::Backspace => {
            pal.query.pop();
            pal.sel = 0;
        }
        KeyCode::Char(c) if !ctrl => {
            pal.query.push(c);
            pal.sel = 0;
        }
        _ => {}
    }
    let len = palette::matches(app, &pal).len();
    pal.sel = pal.sel.min(len.saturating_sub(1));
    app.shell.palette = Some(pal);
}
