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
    // Tour → wizard → palette → … (`04-architecture.md` *Events*).
    if let Some(step) = app.shell.tour {
        if key.code == KeyCode::Esc || step + 1 >= crate::ui2::overlay::tour::STEPS.len() {
            crate::ui2::tour_done(app);
        } else {
            app.shell.tour = Some(step + 1);
        }
        return;
    }
    if app.shell.wizard.is_some() {
        return crate::ui2::wizard::key(app, key);
    }
    // F1–F8 a panel zoomed, F9 the next session, F10 this machine, F11 its Chrome — from
    // anywhere, a session's screen included: what a terminal that keeps ctrl+\ for itself still
    // has, and what the desktop app's tabs send.
    if let KeyCode::F(n @ 1..=11) = key.code {
        return f_key(app, n);
    }
    if app.shell.palette.is_some() {
        return palette_key(app, key);
    }
    if app.cancel_asked().is_some() {
        // The dialog: `y` sends the cancel, anything else keeps the job.
        let yes = matches!(key.code, KeyCode::Char('y' | 'Y'));
        app.answer_cancel(yes);
        if !yes {
            app.shell.toast("kept · nothing was sent", false, std::time::SystemTime::now());
        }
        return;
    }
    if app.team.moving.is_some() && app.shell.zoom == Some(crate::ui2::PanelId::Jira) {
        // A ticket being moved: its picker and its dialog take every key (esc keeps it).
        app.set_view(crate::app::View::Jira);
        app.view_key(key);
        return;
    }
    if app.shell.note.is_some() {
        return note_key(app, key);
    }
    if app.shell.explain.is_some() && key.code == KeyCode::Char('e') {
        // `e` again: the subject's next number.
        crate::ui2::nav::explain_now(app);
        return;
    }
    if let Some(e) = app.shell.explain.take() {
        // Explain: ⏎ asks Claude, y copies, `e` the next number, anything else closes.
        match key.code {
            KeyCode::Char('y') => {
                app.copy(e.text);
                app.shell.toast("✔ formula and inputs copied", false, std::time::SystemTime::now());
            }
            KeyCode::Enter => {
                let prompt = format!("From cobserve (read-only on ClickHouse), {}:\n{}\nWhat should I look at?", e.title, e.text);
                crate::ui2::sessions::ask_claude(app, prompt);
            }
            _ => {}
        }
        return;
    }
    if app.shell.float.is_some() {
        crate::ui2::sessions::float_key(app, key);
        return crate::ui2::views::sync(app);
    }
    if app.shell.filter.as_ref().is_some_and(|f| f.typing) {
        return filter_key(app, key);
    }
    if app.shell.zoom == Some(crate::ui2::PanelId::Sessions) && crate::ui2::sessions::zoom_key(app, key) {
        return;
    }
    if app.shell.local {
        // A browser opened from it: its own keys first (esc closes it, back to the machine).
        if key.code == KeyCode::Char('0') && app.stop_asked().is_none() {
            app.drill = None;
            app.shell.local = false;
            return;
        }
        if app.drill_key(key.code) {
            return;
        }
        // This machine, zoomed: today's view 0 and its keys; esc or 0 goes back.
        match key.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('0') => app.shell.local = false,
            KeyCode::Char('q') => app.quit = true,
            _ => {
                app.set_view(crate::app::View::Local);
                app.view_key(key);
            }
        }
        return;
    }
    let Some(name) = name(&key) else { return };
    if let Some(a) = action::for_key(app, &name) {
        (a.run)(app, &name);
    }
    crate::ui2::views::sync(app);
}

pub fn f_key(app: &mut App, n: u8) {
    match n {
        1..=8 => {
            app.shell.float = None;
            app.shell.local = false;
            app.shell.palette = None;
            app.shell.explain = None;
            app.shell.sessions_full = false;
            let panel = crate::ui2::PanelId::ALL[n as usize - 1];
            app.shell.focus = panel;
            app.shell.zoom = Some(panel);
        }
        9 => app.claude.step(true),
        11 => {
            app.shell.float = None;
            app.shell.zoom = None;
            app.shell.local = true;
            app.set_view(crate::app::View::Local);
            app.open_browser();
            return;
        }
        _ => {
            app.shell.float = None;
            app.shell.zoom = None;
            app.shell.local = true;
        }
    }
    crate::ui2::views::sync(app);
    if app.shell.local {
        app.set_view(crate::app::View::Local);
    }
}

/// Writing a note: printable keys add, `⌫` takes back, `⏎` keeps it on the tape, `esc` drops it.
fn note_key(app: &mut App, key: KeyEvent) {
    let Some(mut note) = app.shell.note.take() else { return };
    match key.code {
        KeyCode::Esc => return,
        KeyCode::Enter => {
            if !note.trim().is_empty() {
                app.note(note.trim());
                app.shell.toast("✎ note kept on the tape", false, std::time::SystemTime::now());
            }
            return;
        }
        KeyCode::Backspace => {
            note.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => note.push(c),
        _ => {}
    }
    app.shell.note = Some(note);
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

/// A paste into what the new shell is typing into — the palette, a note, a filter; `false` when
/// none is open, and the paste goes where it goes today (a session).
pub fn paste(app: &mut App, text: &str) -> bool {
    let line = text.lines().next().unwrap_or_default();
    if crate::ui2::wizard::paste(app, text) {
        return true;
    }
    if let Some(p) = app.shell.palette.as_mut() {
        p.query.push_str(line);
        p.sel = 0;
    } else if let Some(n) = app.shell.note.as_mut() {
        n.push_str(line);
    } else if let Some(f) = app.shell.filter.as_mut().filter(|f| f.typing) {
        f.query.push_str(line);
    } else {
        return false;
    }
    true
}
