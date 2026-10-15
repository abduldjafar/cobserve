//! The sessions in the new shell (`05-keys.md` *In the floating session or a zoomed session*):
//! the floating pane and its prefix, the F-key fallback, the sessions zoom opened full, and the
//! hand-off of a prompt to Claude. Running the programs is unchanged (`claude.rs`, `pty.rs`);
//! what reaches them goes through `App::on_key` as it does on today's view 7.

use crate::app::{App, View};
use crate::claude::{self, Kind, Mode};
use crate::ui2::state::Float;
use crate::ui2::PanelId;
use crossterm::event::{KeyCode, KeyEvent};
use std::time::Duration;

fn toast(app: &mut App, text: impl Into<String>) {
    let at = app.clock;
    app.shell.toast(text, false, at);
}

/// `\`: float the chosen session (start one when there is none), or hide it.
pub fn toggle(app: &mut App) {
    if app.shell.float.take().is_some() {
        return;
    }
    open(app);
}

/// Float the session on screen, starting a kept one; a first Claude when there is none.
pub fn open(app: &mut App) {
    app.shell.float = Some(Float::default());
    app.set_view(View::Claude);
    if app.claude.list.is_empty() {
        app.claude.open_new("", Kind::Claude);
    } else {
        let active = app.claude.active;
        app.claude.select(active);
    }
    if !app.caps.keyboard && !app.shell.fkeys_told {
        app.shell.fkeys_told = true;
        toast(app, "if ctrl+\\ does not reach cobserve here: F1–F8 a panel · F9 the next session · F10 this machine");
    }
}

/// `c`: a SQL session on the chosen node, floating.
pub fn sql_on_node(app: &mut App) {
    let node = crate::ui2::chosen_node(app);
    app.open_query_on(node);
    app.shell.float = Some(Float::default());
}

/// Hand `prompt` to a Claude session, floating: typed into its prompt, not sent — the person
/// reads it and presses ⏎. A session still starting gets it once it is up.
pub fn ask_claude(app: &mut App, prompt: String) {
    let index = app.claude.list.iter().position(|s| s.kind == Kind::Claude);
    match index {
        Some(i) => app.claude.select(i),
        None => {
            app.claude.open_new("", Kind::Claude);
        }
    }
    let id = app.claude.current().map(|s| s.id);
    open(app);
    if let Some(id) = id {
        app.shell.pending_prompt = Some((id, prompt, app.clock));
    }
}

/// Each tick: a prompt waiting for its session goes in once the program has been up a moment.
pub fn tick(app: &mut App) {
    let Some((id, text, asked)) = app.shell.pending_prompt.clone() else { return };
    let now = app.clock;
    let Some(session) = app.claude.by_id_mut(id) else {
        app.shell.pending_prompt = None;
        return;
    };
    let up_for = now.duration_since(asked).unwrap_or_default();
    if session.pane.is_running() && up_for >= Duration::from_secs(3) {
        session.pane.paste(&text);
        app.shell.pending_prompt = None;
    } else if up_for >= Duration::from_secs(60) {
        app.shell.pending_prompt = None;
    }
}

/// A key while a session floats: the prefix's commands after `ctrl+\`, the F-keys, and every
/// other key the program's.
pub fn float_key(app: &mut App, key: KeyEvent) {
    let prefix = app.shell.float.as_ref().and_then(|f| f.prefix).is_some();
    if claude::is_switch_key(&key) {
        if prefix {
            app.shell.float = None;
        } else if let Some(f) = app.shell.float.as_mut() {
            f.prefix = Some(app.clock);
            app.shell.wake = true;
        }
        return;
    }
    if prefix {
        if let Some(f) = app.shell.float.as_mut() {
            f.prefix = None;
        }
        return prefix_key(app, key);
    }
    to_program(app, key);
}

/// The commands after `ctrl+\`.
fn prefix_key(app: &mut App, key: KeyEvent) {
    let closing = app.shell.float.as_mut().map(|f| std::mem::take(&mut f.closing)).unwrap_or(false);
    match key.code {
        KeyCode::Char(c @ '1'..='6') => {
            app.shell.float = None;
            app.shell.focus_on(PanelId::ALL[(c as u8 - b'1') as usize]);
        }
        KeyCode::Char(c @ '7'..='9') => match app.claude.index_of(c as usize - '0' as usize) {
            Some(i) => app.claude.select(i),
            None => toast(app, format!("no session {c} · ctrl+\\ n opens one")),
        },
        KeyCode::Down => app.claude.step(true),
        KeyCode::Up => app.claude.step(false),
        KeyCode::Char('f') => {
            if let Some(f) = app.shell.float.as_mut() {
                f.full = !f.full;
            }
        }
        KeyCode::Char('/') => {
            app.shell.float = None;
            zoom(app);
            app.claude.mode = Mode::Finding { query: String::new(), at: 0 };
        }
        KeyCode::Char('n') => {
            app.shell.float = None;
            zoom(app);
            app.ask_new_session(None);
        }
        KeyCode::Char(c @ ('c' | 'o' | 't' | 'q')) => {
            let kind = match c {
                'c' => Kind::Claude,
                'o' => Kind::OpenCode,
                't' => Kind::Terminal,
                _ => Kind::Query,
            };
            app.shell.float = None;
            zoom(app);
            app.ask_new_session(Some(kind));
        }
        KeyCode::Char('r') => {
            // Its own name to edit, empty when it has none (the label is the folder's, not a name).
            let name = app.claude.current().and_then(|s| s.name.clone()).unwrap_or_default();
            app.shell.float = None;
            zoom(app);
            app.claude.mode = Mode::Naming(name);
        }
        KeyCode::Char('x') => {
            if closing {
                app.claude.close_current();
                if app.claude.list.is_empty() {
                    app.shell.float = None;
                }
            } else if let Some(f) = app.shell.float.as_mut() {
                f.closing = true;
                f.prefix = Some(app.clock);
                toast(app, "x again closes the session");
            }
        }
        _ => {}
    }
}

/// The sessions zoomed, with its view under it.
fn zoom(app: &mut App) {
    app.shell.focus = PanelId::Sessions;
    app.shell.zoom = Some(PanelId::Sessions);
    app.shell.sessions_full = false;
    app.set_view(View::Claude);
}

/// A key for the program on screen, through today's view 7 handling — never one that would act
/// on the monitor instead (a digit, `q`) when no program runs.
pub fn to_program(app: &mut App, key: KeyEvent) {
    app.set_view(View::Claude);
    let running = app.claude.current().is_some_and(|s| s.pane.is_running() || s.console.is_some());
    let picking = !matches!(app.claude.mode, Mode::Typing | Mode::Bar);
    if running || picking || key.code == KeyCode::Enter {
        app.on_key_passthrough(key);
    }
}

/// A key on the zoomed sessions. Browsing: the list's keys (`Zoomed` actions). Opened full, or a
/// picker, a name or a search open: today's handling; `ctrl+\` comes back to the list.
pub fn zoom_key(app: &mut App, key: KeyEvent) -> bool {
    let picking = !matches!(app.claude.mode, Mode::Typing | Mode::Bar);
    if picking {
        let opening = matches!(app.claude.mode, Mode::Opening(_));
        let before = app.claude.list.len();
        app.on_key_passthrough(key);
        // A session just opened from the picker is the one to work in: full, at once.
        if opening && app.claude.list.len() > before && matches!(app.claude.mode, Mode::Typing) {
            open_full(app);
        }
        return true;
    }
    if app.shell.sessions_full {
        if claude::is_switch_key(&key) {
            app.shell.sessions_full = false;
        } else {
            to_program(app, key);
        }
        return true;
    }
    false
}

/// `⏎` on the zoomed sessions: the chosen one opened, its program taking every key — in the
/// preview beside the list when there is one, so the others stay in sight; edge to edge on the
/// grid.
pub fn open_full(app: &mut App) {
    // The zoom's inside at the terminal's size: the frame and the bars take two rows and four
    // columns each way.
    let (w, h) = app.shell.last_size.get();
    let preview = if w > 0 && h > 0 {
        let shape = crate::ui2::views::claude::shape(w.saturating_sub(4), h.saturating_sub(4), app.claude.list.len());
        matches!(shape, crate::ui2::views::claude::Shape::List { preview_w: Some(_), .. })
    } else {
        app.shell.sessions_preview.get()
    };
    open_as(app, !preview);
}

/// `f`: the chosen one opened edge to edge, wherever it is.
pub fn open_spread(app: &mut App) {
    open_as(app, true);
}

fn open_as(app: &mut App, spread: bool) {
    if app.claude.list.is_empty() {
        app.ask_new_session(Some(Kind::Claude));
        return;
    }
    let active = app.claude.active;
    app.claude.select(active);
    app.shell.sessions_full = true;
    app.shell.sessions_spread = spread;
}

/// `x x` on the zoomed list.
pub fn close(app: &mut App) {
    if app.claude.closing {
        app.claude.close_current();
    } else {
        app.claude.closing = true;
        toast(app, "x again closes the session");
    }
}

/// When the prefix began, for the loop's wake-up.
pub fn wake(app: &mut App) -> bool {
    std::mem::take(&mut app.shell.wake)
}
