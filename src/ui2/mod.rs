//! The new shell, mission control (`docs/redesign/`): one screen of numbered panels under a top
//! bar and over a status bar; a panel opens full-screen with `⏎`. `UI=2` selects it; the old shell
//! (`src/ui/`) goes in phase 6.
//!
//! `draw` is the only entry point: chrome, then the grid or the zoom, then the overlays and the
//! toasts (`04-architecture.md` *Render pipeline*).

// The shell is built in phases (`06-build-order.md`): pieces a later phase draws with are here
// before their callers. Phase 6 removes this.
#![allow(dead_code)]

pub mod chrome;
pub mod frame;
pub mod glyph;
pub mod keys;
pub mod layout;
pub mod mouse;
pub mod nav;
pub mod overlay;
pub mod paint;
pub mod panels;
pub mod sessions;
pub mod state;
pub mod views;

#[cfg(test)]
mod tests;

pub use layout::PanelId;
pub use state::ShellState;

use crate::app::App;
use crate::theme::{self, Theme};
use paint::Painter;
use ratatui::layout::Rect;
use ratatui::Frame;

/// Under this the grid cannot be drawn at all: one line says so.
pub const MIN_W: u16 = 40;
pub const MIN_H: u16 = 12;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_with(frame, app, theme::current());
}

pub fn draw_with(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    app.shell.last_size.set((area.width, area.height));
    let (grid, view_inside) = {
        let mut p = Painter::new(frame.buffer_mut());
        p.clear(area, theme.base());
        if area.width < MIN_W || area.height < MIN_H {
            let text = format!("cobserve · too small ({MIN_W}×{MIN_H} at least)");
            p.put(0, 0, &paint::tr(&text, area.width as usize), theme.muted());
            return;
        }
        let nodes = app.snapshot().map_or(0, |s| s.nodes.len());
        let grid = layout::compute(area.width, area.height, app.shell.preset, app.shell.density, nodes);
        chrome::top_bar(&mut p, app, theme, grid.top);
        let mut inside = None;
        match app.shell.zoom {
            Some(zoom) => {
                let top = grid.top.height;
                let r = Rect::new(0, top, area.width, area.height.saturating_sub(1 + top));
                inside = panels::draw_zoomed(&mut p, app, theme, zoom, r).map(|i| (zoom, i));
            }
            None => {
                for (id, r) in &grid.panels {
                    if r.height < 3 || r.width < 6 {
                        continue;
                    }
                    panels::draw(&mut p, app, theme, *id, *r, app.shell.focus == *id, (grid.tile_w, grid.tile_h));
                }
            }
        }
        (grid, inside)
    };
    // Today's views, zoomed: they draw with ratatui's widgets into the frame.
    if let Some((zoom, inside)) = view_inside {
        views::draw(frame, app, theme, zoom, inside);
    }
    // The floating session: its box, then the program's screen in it.
    if let Some(f) = &app.shell.float {
        let r = overlay::float::rect(f, area.width, area.height, grid.top.height);
        {
            let mut p = Painter::new(frame.buffer_mut());
            overlay::float::draw(&mut p, app, theme, f, r);
        }
        crate::ui::claude::pane(frame, app, theme, overlay::float::inner(r));
        // The which-key box goes over the screen.
        let mut p = Painter::new(frame.buffer_mut());
        overlay::float::which(&mut p, theme, f, r, app.clock);
    }
    let mut p = Painter::new(frame.buffer_mut());
    chrome::status_bar(&mut p, app, theme, grid.status);
    if let Some(e) = &app.shell.explain {
        overlay::explain::draw(&mut p, theme, e);
    }
    if let Some(job) = app.cancel_asked() {
        overlay::dialog::draw(&mut p, app, theme, job);
    }
    if let Some(palette) = &app.shell.palette {
        overlay::palette::draw(&mut p, app, theme, palette);
    }
    if let Some(note) = &app.shell.note {
        overlay::note::draw(&mut p, theme, note, grid.status);
    }
    let mut lines: Vec<(String, bool)> = app.shell.toasts.iter().map(|t| (t.text.clone(), t.error)).collect();
    if let Some(notice) = app.notice() {
        lines.push((notice.to_string(), false));
    }
    overlay::toast::draw(&mut p, theme, &lines);
}

/// The chosen node: the one picked last, while it is still in the fleet, else the first in the
/// sort order.
pub fn chosen_node(app: &App) -> Option<String> {
    app.with_view(|v| {
        if let Some(name) = app.shell.chosen.as_deref()
            && v.nodes.iter().any(|n| n.name() == name)
        {
            return Some(name.to_string());
        }
        panels::nodes::order(app, &v.nodes).first().map(|n| n.name().to_string())
    })
    .flatten()
}
