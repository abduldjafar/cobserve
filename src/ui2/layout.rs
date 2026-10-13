//! The grid of the new shell (`docs/redesign/01-screen.md`): a pure function of the screen's size,
//! the preset, the density and the node count, returning the rect of every panel — ported as is
//! from the simulation's `layout()`, rounding and all, so the screens in `docs/redesign/screens/`
//! are its acceptance tests.

use ratatui::layout::Rect;

/// The eight panels, numbered as the keys that focus them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PanelId {
    Fleet,
    Queue,
    Nodes,
    Node,
    Airflow,
    Jira,
    Tape,
    Sessions,
}

impl PanelId {
    pub const ALL: [PanelId; 8] = [
        PanelId::Fleet,
        PanelId::Queue,
        PanelId::Nodes,
        PanelId::Node,
        PanelId::Airflow,
        PanelId::Jira,
        PanelId::Tape,
        PanelId::Sessions,
    ];

    /// `1` … `8`.
    pub fn number(self) -> u8 {
        PanelId::ALL.iter().position(|p| *p == self).map_or(1, |i| i as u8 + 1)
    }

    pub fn from_number(n: u8) -> Option<PanelId> {
        PanelId::ALL.get(usize::from(n).checked_sub(1)?).copied()
    }

    /// The title its frame carries.
    pub fn title(self) -> &'static str {
        match self {
            PanelId::Fleet => "FLEET",
            PanelId::Queue => "REDASH QUEUE",
            PanelId::Nodes => "NODES",
            PanelId::Node => "NODE",
            PanelId::Airflow => "AIRFLOW",
            PanelId::Jira => "JIRA",
            PanelId::Tape => "TAPE",
            PanelId::Sessions => "SESSIONS",
        }
    }

    /// `tab`: the next panel, round the eight.
    pub fn next(self) -> PanelId {
        PanelId::ALL[usize::from(self.number()) % 8]
    }

    /// `shift+tab`.
    pub fn prev(self) -> PanelId {
        PanelId::ALL[(usize::from(self.number()) + 6) % 8]
    }
}

/// `p`: which panels share the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Every panel.
    #[default]
    All,
    /// NODES and NODE take the middle; AIRFLOW and JIRA leave the grid.
    Fleet,
    /// No tiles, no NODE; AIRFLOW and JIRA split the middle.
    Ops,
}

impl Preset {
    pub fn next(self) -> Preset {
        match self {
            Preset::All => Preset::Fleet,
            Preset::Fleet => Preset::Ops,
            Preset::Ops => Preset::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::All => "all",
            Preset::Fleet => "fleet",
            Preset::Ops => "ops",
        }
    }
}

/// `+`: how much chrome there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    /// The top bar with the day line.
    #[default]
    Comfortable,
    /// The top bar with a chip in place of the day line.
    Compact,
    /// No top bar; the clock moves into the status bar.
    Focus,
}

impl Density {
    pub fn next(self) -> Density {
        match self {
            Density::Comfortable => Density::Compact,
            Density::Compact => Density::Focus,
            Density::Focus => Density::Comfortable,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
            Density::Focus => "focus",
        }
    }
}

/// Where everything goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// The top bar: row 0, or nothing in focus density.
    pub top: Rect,
    /// The last row.
    pub status: Rect,
    /// The panels on the grid, top to bottom, left to right; a preset leaves some out.
    pub panels: Vec<(PanelId, Rect)>,
    /// A node tile's size, and how many fit a row of the NODES panel.
    pub tile_w: u16,
    pub tile_h: u16,
    pub per_row: u16,
}

impl Grid {
    pub fn rect(&self, id: PanelId) -> Option<Rect> {
        self.panels.iter().find(|(p, _)| *p == id).map(|(_, r)| *r)
    }
}

/// `Math.round` of the simulation: halves go up.
fn round(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

/// The grid for a `w`×`h` screen (`01-screen.md` *The grid*).
pub fn compute(w: u16, h: u16, preset: Preset, density: Density, nodes: usize) -> Grid {
    let (wi, hi) = (i32::from(w), i32::from(h));
    let top = i32::from(density != Density::Focus);
    let tile_w: i32 = if wi >= 150 { 20 } else if wi >= 100 { 19 } else { 16 };
    let tile_h: i32 = if hi >= 30 { 5 } else { 4 };
    let per_row = ((wi - 4) / tile_w).max(1);
    let tile_rows = (nodes as i32 + per_row - 1).div_euclid(per_row).clamp(0, 2);

    let r1 = if hi >= 40 { 9 } else if hi >= 30 { 7 } else { 5 };
    let mut tiles = if preset == Preset::Ops { 0 } else { tile_rows * tile_h + 2 };
    let mut bottom = if hi >= 40 { 7 } else if hi >= 30 { 6 } else { 3 };
    if preset == Preset::Fleet {
        bottom = 3;
    }
    let mut mid = hi - 1 - top - r1 - tiles - bottom;
    if mid < 6 && tile_rows > 1 && preset != Preset::Ops {
        tiles = tile_h + 2;
        mid = hi - 1 - top - r1 - tiles - bottom;
    }
    if mid < 6 {
        bottom = 3;
        mid = hi - 1 - top - r1 - tiles - bottom;
    }

    // Below the supported size a row can come out negative: it is then nothing, and the rows
    // under it lose what it took. Nothing is ever placed past the status bar.
    let mut panels: Vec<(PanelId, Rect)> = Vec::new();
    let floor = hi - 1;
    let mut y = top;
    let mut row = |height: i32, cells: &[(PanelId, i32, i32)], panels: &mut Vec<(PanelId, Rect)>| {
        let height = height.max(0).min(floor - y).max(0);
        if height > 0 {
            for &(id, x, width) in cells {
                if width > 0 {
                    panels.push((id, Rect::new(x as u16, y as u16, width as u16, height as u16)));
                }
            }
        }
        y += height;
    };
    let fw = round(f64::from(wi) * 0.55);
    row(r1, &[(PanelId::Fleet, 0, fw), (PanelId::Queue, fw, wi - fw)], &mut panels);
    if tiles > 0 {
        row(tiles, &[(PanelId::Nodes, 0, wi)], &mut panels);
    }
    match preset {
        Preset::Fleet => row(mid, &[(PanelId::Node, 0, wi)], &mut panels),
        Preset::Ops => {
            let a = round(f64::from(wi) * 0.5);
            row(mid, &[(PanelId::Airflow, 0, a), (PanelId::Jira, a, wi - a)], &mut panels);
        }
        Preset::All => {
            let (a, c) = (round(f64::from(wi) * 0.4), round(f64::from(wi) * 0.32));
            row(mid, &[(PanelId::Node, 0, a), (PanelId::Airflow, a, c), (PanelId::Jira, a + c, wi - a - c)], &mut panels);
        }
    }
    if wi >= 100 && bottom >= 5 {
        let tw = round(f64::from(wi) * 0.6);
        row(bottom, &[(PanelId::Tape, 0, tw), (PanelId::Sessions, tw, wi - tw)], &mut panels);
    } else {
        row(bottom, &[(PanelId::Sessions, 0, wi)], &mut panels);
    }

    Grid {
        top: Rect::new(0, 0, w, (top as u16).min(h)),
        status: Rect::new(0, h.saturating_sub(1), w, h.min(1)),
        panels,
        tile_w: tile_w as u16,
        tile_h: tile_h as u16,
        per_row: per_row as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows' heights, top to bottom: the top bar, each panel row, the status bar.
    fn rows(grid: &Grid) -> Vec<u16> {
        let mut out = vec![grid.top.height];
        let mut seen: Vec<u16> = Vec::new();
        for (_, r) in &grid.panels {
            if !seen.contains(&r.y) {
                seen.push(r.y);
                out.push(r.height);
            }
        }
        out.push(grid.status.height);
        out
    }

    #[test]
    fn the_worked_sizes_come_out_as_the_screens_draw_them() {
        // 01-screen.md and the screens: 120×36 → 1 + 7 + 12 + 9 + 6 + 1; 80×24 → 1 + 5 + 6 + 8 +
        // 3 + 1; 160×48 → 1 + 9 + 12 + 18 + 7 + 1 (two rows of tiles, as screens/160x48-dash.txt
        // draws them) — the nine nodes of the fake fleet.
        assert_eq!(rows(&compute(120, 36, Preset::All, Density::Comfortable, 9)), [1, 7, 12, 9, 6, 1]);
        assert_eq!(rows(&compute(80, 24, Preset::All, Density::Comfortable, 9)), [1, 5, 6, 8, 3, 1]);
        assert_eq!(rows(&compute(160, 48, Preset::All, Density::Comfortable, 9)), [1, 9, 12, 18, 7, 1]);

        let g = compute(120, 36, Preset::All, Density::Comfortable, 9);
        assert_eq!((g.tile_w, g.tile_h, g.per_row), (19, 5, 6));
        let at = |id| g.rect(id).unwrap();
        assert_eq!((at(PanelId::Fleet).width, at(PanelId::Queue).width), (66, 54));
        assert_eq!((at(PanelId::Node).width, at(PanelId::Airflow).width, at(PanelId::Jira).width), (48, 38, 34));
        assert_eq!((at(PanelId::Tape).width, at(PanelId::Sessions).width), (72, 48));
        // 80 columns: no room for the tape beside the sessions.
        let narrow = compute(80, 24, Preset::All, Density::Comfortable, 9);
        assert_eq!(narrow.rect(PanelId::Tape), None);
        assert_eq!(narrow.rect(PanelId::Sessions).unwrap().width, 80);
    }

    #[test]
    fn presets_and_densities_move_the_rows() {
        let fleet = compute(120, 36, Preset::Fleet, Density::Comfortable, 9);
        assert!(fleet.rect(PanelId::Airflow).is_none() && fleet.rect(PanelId::Jira).is_none());
        assert_eq!(fleet.rect(PanelId::Node).unwrap().width, 120);
        assert_eq!(rows(&fleet), [1, 7, 12, 12, 3, 1], "the bottom row shrinks to 3");
        let ops = compute(120, 36, Preset::Ops, Density::Comfortable, 9);
        assert!(ops.rect(PanelId::Nodes).is_none() && ops.rect(PanelId::Node).is_none());
        assert_eq!((ops.rect(PanelId::Airflow).unwrap().width, ops.rect(PanelId::Jira).unwrap().width), (60, 60));
        let focus = compute(120, 36, Preset::All, Density::Focus, 9);
        assert_eq!(focus.top.height, 0);
        assert_eq!(focus.rect(PanelId::Fleet).unwrap().y, 0, "no top bar: the grid starts at row 0");
        assert_eq!(rows(&focus)[3], 10, "the middle takes the row the top bar gave up");
        assert_eq!(PanelId::Sessions.next(), PanelId::Fleet);
        assert_eq!(PanelId::Fleet.prev(), PanelId::Sessions);
        assert_eq!((PanelId::from_number(4), PanelId::from_number(0), PanelId::from_number(9)), (Some(PanelId::Node), None, None));
    }

    /// The panels tile the screen between the top bar and the status bar: no overlap, no gap.
    fn tiles_exactly(grid: &Grid, w: u16, h: u16) -> Result<(), String> {
        let mut cover = vec![0u8; usize::from(w) * usize::from(h)];
        for (id, r) in &grid.panels {
            if r.x + r.width > w || r.y + r.height > h {
                return Err(format!("{id:?} {r:?} is off a {w}×{h} screen"));
            }
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    cover[usize::from(y) * usize::from(w) + usize::from(x)] += 1;
                }
            }
        }
        let first = grid.top.height;
        for y in 0..h {
            for x in 0..w {
                let n = cover[usize::from(y) * usize::from(w) + usize::from(x)];
                let chrome = y < first || y == h - 1;
                if (chrome && n != 0) || (!chrome && n != 1) {
                    return Err(format!("cell {x},{y} covered {n} times on {w}×{h}"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn five_hundred_sizes_tile_the_screen_and_none_panics() {
        // A small deterministic generator: the same 500 sizes every run.
        let mut seed: u64 = 0x2026_1013;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let presets = [Preset::All, Preset::Fleet, Preset::Ops];
        let densities = [Density::Comfortable, Density::Compact, Density::Focus];
        for _ in 0..500 {
            let (w, h) = (80 + next(171) as u16, 24 + next(57) as u16);
            let (p, d) = (presets[next(3) as usize], densities[next(3) as usize]);
            let nodes = next(40) as usize;
            let grid = compute(w, h, p, d, nodes);
            if let Err(why) = tiles_exactly(&grid, w, h) {
                panic!("{why} ({p:?}, {d:?}, {nodes} nodes)");
            }
        }
        // Below the supported size: nothing off the screen, nothing panics.
        for w in 0..80u16 {
            for h in 0..24u16 {
                let grid = compute(w, h, Preset::All, Density::Comfortable, 9);
                assert!(grid.panels.iter().all(|(_, r)| r.y + r.height < h.max(1) && r.x + r.width <= w), "{w}×{h}");
            }
        }
    }
}
