//! `:tour` (`screens/120x36-tour.txt`): six boxes over the live screen, each with the row it is
//! about lit; any key advances, `esc` skips. Shown once on the new shell's first start.

use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::layout::Grid;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub const STEPS: [[&str; 2]; 6] = [
    ["One screen, eight panels, numbered 1–8. The clock and the day sit in the top bar;", "● green means the numbers are live, ◷ amber means you are replaying a minute."],
    ["FLEET and REDASH: one big number each, and the history as a graph. A panel's frame", "takes the colour of its worst finding; red and amber mean severity and nothing else."],
    ["NODES: a tile per node, worst first. ←→↑↓ pick one; the NODE panel under it follows.", "⏎ opens the tiles one screen; ⏎ again opens the node with everything about it."],
    ["NODE: who holds the node, as bars that add up to the node (§5.3). Press e on a row for", "its formula, its inputs and the system table they came from."],
    ["`:` opens the palette with everything else, `?` the keys of the screen you are on,", "`\\` floats a session over any panel; panel 8 lists every session. `p` changes the layout."],
    ["That is the tour. The numbers are the fleet's own, through the same §5 arithmetic.", "Press : and type tour any time to see it again."],
];

/// The row each box lights: the top bar, then the first row inside FLEET, NODES and NODE.
fn row(step: usize, grid: &Grid) -> Option<u16> {
    let inside = |id: PanelId| grid.rect(id).map(|r| r.y + 1);
    match step {
        0 => (grid.top.height > 0).then_some(0),
        1 => inside(PanelId::Fleet),
        2 => inside(PanelId::Nodes),
        3 => inside(PanelId::Node),
        _ => None,
    }
}

pub fn draw(p: &mut Painter<'_>, theme: &Theme, step: usize, grid: &Grid) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    let Some(text) = STEPS.get(step) else { return };
    let w = sw.saturating_sub(6).min(92);
    if w < 40 || sh < 12 {
        return;
    }
    let x = (sw - w) / 2;
    let lit = row(step, grid).filter(|r| *r + 10 < sh);
    let y = match lit {
        Some(r) => r + 2,
        None => sh - 9,
    };
    if let Some(r) = lit {
        p.style(Rect::new(0, r, sw, 1), theme.selected());
    }
    frame::overlay(p, theme, Rect::new(x, y, w, 5), &format!("tour · {} of {}", step + 1, STEPS.len()), theme.accent().add_modifier(Modifier::BOLD), "any key advances · esc skips");
    for (k, line) in text.iter().enumerate() {
        p.put(x + 3, y + 1 + k as u16, &tr(line, w as usize - 6), theme.text().patch(theme.surface()));
    }
}

/// Where the tour's "seen" mark is kept.
pub fn marker() -> Option<std::path::PathBuf> {
    crate::setup::dir().map(|d| d.join("tour-done"))
}
