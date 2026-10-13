//! The mouse in the new shell (`05-keys.md` *Mouse*): a click on a panel focuses it, a second
//! click opens it; the wheel moves the focused cursor; a click on `❖ cobserve` goes to the grid and
//! on the clock flips it to UTC. Inside a floating session or a zoomed view the click is theirs, as
//! it is today.

use crate::app::App;
use crate::ui2::{layout, nav, views, PanelId};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

fn inside(r: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

pub fn on_mouse(app: &mut App, event: MouseEvent) {
    let (w, h) = app.shell.last_size.get();
    let (col, row) = (event.column, event.row);
    let top = u16::from(app.shell.density != layout::Density::Focus);
    if let Some(f) = app.shell.float.clone() {
        let r = crate::ui2::overlay::float::rect(&f, w, h, top);
        if inside(r, col, row) {
            app.on_mouse_passthrough(event);
        }
        return;
    }
    if let Some(zoom) = app.shell.zoom
        && views::is_view(zoom)
        && row > top
        && row + 1 < h
    {
        let before = app.view;
        app.on_mouse_passthrough(event);
        if app.view != before {
            views::follow(app);
        }
        views::sync(app);
        return;
    }
    match event.kind {
        MouseEventKind::ScrollUp => nav::choose(app, "Up"),
        MouseEventKind::ScrollDown => nav::choose(app, "Down"),
        MouseEventKind::Down(MouseButton::Left) => {
            if top == 1 && row == 0 {
                if col <= 12 {
                    app.shell.zoom = None;
                } else if col + 24 >= w {
                    app.time.flip();
                }
                return;
            }
            if app.shell.zoom.is_some() {
                return;
            }
            let nodes = app.snapshot().map_or(0, |s| s.nodes.len());
            let grid = layout::compute(w, h, app.shell.preset, app.shell.density, nodes);
            let Some((id, _)) = grid.panels.iter().find(|(_, r)| inside(*r, col, row)).copied() else { return };
            if app.shell.focus == id {
                nav::open(app);
            } else {
                app.shell.focus_on(id);
                if id == PanelId::Nodes {
                    pick_tile(app, &grid, col, row);
                }
            }
        }
        _ => {}
    }
    views::sync(app);
}

/// A click on a tile chooses its node.
fn pick_tile(app: &mut App, grid: &layout::Grid, col: u16, row: u16) {
    let Some(r) = grid.rect(PanelId::Nodes) else { return };
    let (x0, y0) = (r.x + 2, r.y + 1);
    if col < x0 || row < y0 || grid.tile_w == 0 || grid.tile_h == 0 {
        return;
    }
    let per = ((r.width.saturating_sub(4)) / grid.tile_w).max(1);
    let k = ((row - y0) / grid.tile_h) * per + (col - x0) / grid.tile_w;
    if let Some(name) = nav::node_names(app).get(k as usize) {
        app.shell.chosen = Some(name.clone());
        app.shell.person = None;
    }
}
