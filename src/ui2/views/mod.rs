//! Five panels are today's views zoomed (`06-build-order.md` phase 3: "moved, not rewritten"):
//! REDASH QUEUE is view 2, AIRFLOW view 5, JIRA view 6, TAPE the tape. Their state — cursors,
//! open pages, the cancel being asked — lives in `App` as it always has, keyed by `app.view`; this
//! module keeps `app.view` on the panel that has the focus, hands those panels their keys, and
//! follows the view when one of its keys goes elsewhere (`⏎` on a running job opens its node).

pub mod airflow;
pub mod board;
pub mod claude;
pub mod console;
pub mod detail;
pub mod drawer;
pub mod jira;
pub mod local;
pub mod loom;
pub mod queue;
pub mod tape;
pub mod usage;
pub mod widgets;

#[cfg(test)]
mod tests;

use crate::app::{App, View};
use crate::tree::RowId;
use crate::ui2::PanelId;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The view under a panel; the panels that are not one of today's views stand on the tree's.
pub fn view_of(panel: PanelId) -> View {
    match panel {
        PanelId::Queue => View::Queue,
        PanelId::Airflow => View::Airflow,
        PanelId::Jira => View::Jira,
        PanelId::Tape => View::Tape,
        PanelId::Sessions => View::Claude,
        PanelId::Fleet | PanelId::Nodes | PanelId::Node => View::Nodes,
    }
}

/// Whether the panel's keys are its view's.
pub fn is_view(panel: PanelId) -> bool {
    matches!(panel, PanelId::Queue | PanelId::Airflow | PanelId::Jira | PanelId::Tape | PanelId::Sessions)
}

/// `app.view` on the panel the keys go to — the zoom, else the focus; the sessions' view while a
/// session floats or the sessions are zoomed, so a session on screen is seen as on screen.
pub fn sync(app: &mut App) {
    if app.shell.float.is_some() || app.shell.zoom == Some(PanelId::Sessions) {
        app.set_view(View::Claude);
        return;
    }
    let panel = app.shell.zoom.unwrap_or(app.shell.focus);
    let view = match view_of(panel) {
        // Panel 8 focused on the grid: its rows are the sessions themselves, no view needed.
        View::Claude => View::Nodes,
        other => other,
    };
    app.set_view(view);
}

/// A key by its `Action` name, as the view's handler wants it.
pub fn event(name: &str) -> Option<KeyEvent> {
    let code = match name {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Backspace" => KeyCode::Backspace,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Hand `name` to the view under the panel, then follow it if it went to another view.
pub fn pass(app: &mut App, name: &str) {
    let Some(key) = event(name) else { return };
    sync(app);
    let before = app.view;
    app.view_key(key);
    if app.view != before {
        follow(app);
    }
}

/// The view moved (a job's node, a tape line's subject): the panel that shows it, zoomed.
pub fn follow(app: &mut App) {
    let panel = match app.view {
        View::Nodes => {
            // The tree's cursor says which node, and which person on it.
            match app.selected().cloned() {
                Some(RowId::Query { node, user, person, .. } | RowId::User { node, user, person }) => {
                    app.shell.chosen = Some(node);
                    app.shell.person = Some(crate::attrib::user_label(&user, person.as_deref()));
                }
                Some(RowId::Node(node) | RowId::Closing(node)) => {
                    app.shell.chosen = Some(node);
                    app.shell.person = None;
                }
                _ => {}
            }
            PanelId::Node
        }
        View::Map => PanelId::Nodes,
        View::Queue => PanelId::Queue,
        View::Airflow => PanelId::Airflow,
        View::Jira => PanelId::Jira,
        View::Tape => PanelId::Tape,
        View::Claude => PanelId::Sessions,
        View::Local => return,
    };
    app.shell.focus = panel;
    app.shell.zoom = Some(panel);
}

/// Something open inside the view that `esc` closes before the zoom: a page, the month's time.
pub fn has_inner(app: &App) -> bool {
    match app.shell.zoom {
        Some(PanelId::Airflow) => !app.airflow_pages.is_empty(),
        Some(PanelId::Jira) => app.jira_page.is_some() || app.jira_time.is_some(),
        _ => false,
    }
}

/// Draw the view under a zoomed panel into `inside`: the view as today, with its drawer under it
/// as today.
pub fn draw(frame: &mut ratatui::Frame, app: &App, theme: &crate::theme::Theme, zoom: PanelId, inside: ratatui::layout::Rect) {
    use ratatui::layout::Rect;
    let drawer_h = if inside.height >= 20 { 4 } else { 0 };
    let body = Rect::new(inside.x, inside.y, inside.width, inside.height - drawer_h);
    let drawer = Rect::new(inside.x, inside.y + body.height, inside.width, drawer_h);
    match zoom {
        PanelId::Queue => queue::draw(frame, app, theme, body),
        PanelId::Tape => tape::draw(frame, app, theme, body),
        PanelId::Sessions => {
            claude::draw(frame, app, theme, inside, 0);
            return;
        }
        PanelId::Airflow => match app.page() {
            Some(page) => detail::draw(frame, app, theme, body, page),
            None => airflow::draw(frame, app, theme, body),
        },
        PanelId::Jira => match (app.page(), app.jira_time) {
            (Some(page), _) => detail::draw(frame, app, theme, body, page),
            (None, Some(time)) => jira::draw_time(frame, app, theme, body, time),
            (None, None) => jira::draw(frame, app, theme, body),
        },
        _ => return,
    }
    if drawer_h > 0 {
        drawer::draw(frame, app, theme, drawer);
    }
}

/// This machine zoomed (`:local`, `0`, F10): today's view 0 with its drawer under it.
pub fn draw_local(frame: &mut ratatui::Frame, app: &App, theme: &crate::theme::Theme, inside: ratatui::layout::Rect) {
    use ratatui::layout::Rect;
    let drawer_h = if inside.height >= 20 { 4 } else { 0 };
    local::draw(frame, app, theme, Rect::new(inside.x, inside.y, inside.width, inside.height - drawer_h));
    if drawer_h > 0 {
        drawer::draw(frame, app, theme, Rect::new(inside.x, inside.y + inside.height - drawer_h, inside.width, drawer_h));
    }
}

/// View 6's board lanes, each its tickets in the order drawn — what `← →` cross.
pub fn board_lanes(board: &crate::jira::Board) -> Vec<Vec<&crate::jira::Ticket>> {
    board::lanes(board).into_iter().map(|l| l.tickets).collect()
}
