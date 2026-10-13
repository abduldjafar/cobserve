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
pub mod overlay;
pub mod paint;
pub mod panels;
pub mod state;

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
    let buf = frame.buffer_mut();
    let mut p = Painter::new(buf);
    p.clear(area, theme.base());
    if area.width < MIN_W || area.height < MIN_H {
        let text = format!("cobserve · too small ({MIN_W}×{MIN_H} at least)");
        p.put(0, 0, &paint::tr(&text, area.width as usize), theme.muted());
        return;
    }
    let nodes = app.snapshot().map_or(0, |s| s.nodes.len());
    let grid = layout::compute(area.width, area.height, app.shell.preset, app.shell.density, nodes);
    chrome::top_bar(&mut p, app, theme, grid.top);
    match app.shell.zoom {
        Some(zoom) => {
            let top = grid.top.height;
            let r = Rect::new(0, top, area.width, area.height.saturating_sub(1 + top));
            zoomed(&mut p, app, theme, zoom, r);
        }
        None => {
            for (id, r) in &grid.panels {
                if r.height < 3 || r.width < 6 {
                    continue;
                }
                panels::draw(&mut p, app, theme, *id, *r, app.shell.focus == *id);
            }
        }
    }
    chrome::status_bar(&mut p, app, theme, grid.status);
    if let Some(palette) = &app.shell.palette {
        overlay::palette::draw(&mut p, app, theme, palette);
    }
    let mut lines: Vec<(String, bool)> = app.shell.toasts.iter().map(|t| (t.text.clone(), t.error)).collect();
    if let Some(notice) = app.notice() {
        lines.push((notice.to_string(), false));
    }
    overlay::toast::draw(&mut p, theme, &lines);
}

/// A zoomed panel: one frame over the body, its title a chip.
fn zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, id: PanelId, r: Rect) {
    panels::frame(p, app, theme, id, r, true);
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
        let mut nodes: Vec<&crate::model::NodeView<'_>> = v.nodes.iter().collect();
        nodes.sort_by(|a, b| crate::model::compare_nodes(a, b, app.tree.sort));
        nodes.first().map(|n| n.name().to_string())
    })
    .flatten()
}
