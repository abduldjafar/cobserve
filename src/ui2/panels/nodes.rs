//! Panel 3, NODES (`02-panels.md` *3 · NODES*): a tile per node, worst first, as many as two rows
//! hold; the last cell says how many more and how hot they get. Zoomed: bigger tiles with both
//! bars and their denominators, the sparkline and the person who holds the most.

use crate::app::App;
use crate::fmt;
use crate::history::WINDOW_S;
use crate::model::NodeView;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::glyph::{bar, graph};
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// A node's memory and CPU shares — the replayed minute's while replaying.
pub fn shares(app: &App, n: &NodeView<'_>) -> (Option<f64>, Option<f64>) {
    let h = app.history.node(n.name());
    (crate::ui2::replay::value(app, n.mem_pct, h.map(|h| &h.mem_pct)), crate::ui2::replay::value(app, n.cpu_pct, h.map(|h| &h.cpu_pct)))
}

/// A node's severity: the worse of its memory and CPU (§7); a node without numbers is red.
pub fn node_sev(app: &App, n: &NodeView<'_>) -> Severity {
    if !n.node.reachable {
        return Severity::Crit;
    }
    let (mem, cpu) = shares(app, n);
    severity::node(mem).max(severity::node(cpu))
}

/// The sparkline of a node's memory over the history, `cells` wide, worst of each slice.
pub fn spark(app: &App, name: &str, cells: usize) -> String {
    let Some(h) = app.history.node(name) else { return " ".repeat(cells) };
    graph::sparkline(&h.mem_pct.buckets(app.history.now(), WINDOW_S, cells), 100.0)
}

fn style_of(theme: &Theme, sev: Severity, plain: Style) -> Style {
    if sev.is_problem() { theme.sev(sev) } else { plain }
}

/// The tile title: the name and its NEW badge, in the top border, never over the corner.
fn tile_title(p: &mut Painter<'_>, theme: &Theme, app: &App, name: &str, r: Rect, chosen: bool) {
    let (x, y, w) = (r.x, r.y, r.width);
    let new = app.tree.new_nodes.contains(name);
    let text = if new { format!("{name} NEW") } else { name.to_string() };
    let style = if chosen { theme.tab_active() } else { theme.strong() };
    p.put(x + 1, y, &format!(" {} ", tr(&text, w.saturating_sub(4) as usize)), style);
}

/// The box of a tile, its inside painted when chosen.
fn tile_box(p: &mut Painter<'_>, theme: &Theme, r: Rect, sev: Severity, chosen: bool) {
    frame::rounded(p, r, frame::frame_style(theme, sev));
    if chosen && r.width > 2 && r.height > 2 {
        p.style(Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2), theme.selected());
    }
}

/// A dashboard tile (`19×5`, `20×5`, `16×4`), drawn `w` wide (the tile's width less its gap).
fn tile(p: &mut Painter<'_>, app: &App, theme: &Theme, n: &NodeView<'_>, r: Rect, chosen: bool) {
    let sev = node_sev(app, n);
    tile_box(p, theme, r, sev, chosen);
    tile_title(p, theme, app, n.name(), r, chosen);
    let (x, y, w, h) = (r.x, r.y, r.width, r.height);
    let iw = w.saturating_sub(4) as usize;
    if !n.node.reachable {
        p.put(x + 2, y + 1, &tr(&format!("↯ {}", n.node.down_word()), iw), theme.sev(Severity::Crit));
        if h >= 4 {
            p.put(x + 2, y + 2, &tr(n.node.down_detail().unwrap_or(""), iw), theme.muted());
        }
        return;
    }
    let mp = shares(app, n).0;
    let bw = iw.saturating_sub(5);
    let (fill, empty) = bar::share(mp.unwrap_or(0.0), bw);
    let after = p.put(x + 2, y + 1, &fill, style_of(theme, sev, theme.accent()));
    p.put(after, y + 1, &empty, style_of(theme, sev, theme.accent()));
    let pct = mp.map_or("—".to_string(), |m| format!("{}%", m.round() as i64));
    p.putr(x + w - 2, y + 1, &pct, style_of(theme, sev, theme.text()));
    p.put(x + 2, y + 2, &spark(app, n.name(), iw), style_of(theme, sev, theme.muted()));
    if h >= 5 {
        let queries: usize = n.users.iter().map(|u| u.queries.len()).sum();
        let count = if n.runaways > 0 { format!("{queries}q !{}", n.runaways) } else { format!("{queries}q") };
        p.put(x + 2, y + 3, &tr(&count, 6), if n.runaways > 0 { theme.sev(Severity::Crit) } else { theme.muted() });
        let longest = n.users.iter().flat_map(|u| u.queries.iter()).max_by(|a, b| a.query.elapsed_s.total_cmp(&b.query.elapsed_s));
        let (text, style) = match longest {
            Some(q) => (fmt::dur(q.query.elapsed_s), if q.runaway { theme.sev(Severity::Crit) } else { theme.muted() }),
            None if n.node.lag_s >= crate::model::FOLD_LAG_S => (format!("lag {}s", n.node.lag_s), theme.sev(Severity::Warn)),
            None => ("—".to_string(), theme.muted()),
        };
        p.putr(x + w - 2, y + 3, &text, style);
    }
}

/// The nodes in the panel's order: sorted by `s`, filtered by `/`.
pub fn order<'v, 'a>(app: &App, nodes: &'v [NodeView<'a>]) -> Vec<&'v NodeView<'a>> {
    let mut out: Vec<&NodeView<'a>> = nodes.iter().collect();
    out.sort_by(|a, b| crate::model::compare_nodes(a, b, app.tree.sort));
    if let Some(f) = app.shell.filter.as_ref().filter(|f| !f.query.is_empty()) {
        let q = f.query.to_lowercase();
        out.retain(|n| n.name().to_lowercase().contains(&q));
    }
    out
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool, tile_w: u16, tile_h: u16) {
    let i = super::frame(p, app, theme, PanelId::Nodes, r, focused);
    if i.width == 0 || i.height == 0 {
        return;
    }
    let chosen = crate::ui2::chosen_node(app);
    app.with_view(|v| {
        let nodes = order(app, &v.nodes);
        let per = (i.width / tile_w).max(1);
        app.shell.tiles_per_row.set(per);
        let rows = (i.height / tile_h).max(1);
        let cap = (per * rows) as usize;
        p.within(i, |p| {
            for (k, n) in nodes.iter().enumerate().take(cap) {
                let x = i.x + (k as u16 % per) * tile_w;
                let y = i.y + (k as u16 / per) * tile_h;
                if k == cap - 1 && nodes.len() > cap {
                    let rest = &nodes[k..];
                    p.put(x + 1, y + 1, &format!("+{} more", rest.len()), theme.muted());
                    let hottest = rest.iter().filter_map(|n| n.mem_pct).fold(0.0, f64::max);
                    p.put(x + 1, y + 2, &format!("≤ {}%", hottest.round() as i64), theme.muted());
                    break;
                }
                // The chosen tile is marked wherever the focus is: NODE follows it.
                let picked = chosen.as_deref() == Some(n.name());
                tile(p, app, theme, n, Rect::new(x, y, tile_w - 1, tile_h), picked);
            }
            if nodes.is_empty() {
                let text = match app.shell.filter.as_ref() {
                    Some(f) if !f.query.is_empty() => format!("no node matches /{}", f.query),
                    _ => "waiting for the first snapshot…".to_string(),
                };
                p.put(i.x, i.y, &text, theme.muted());
            }
        });
    });
}

/// Zoomed: `28×8` tiles.
pub fn draw_zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    const TW: u16 = 28;
    const TH: u16 = 8;
    let chosen = crate::ui2::chosen_node(app);
    app.with_view(|v| {
        let nodes = order(app, &v.nodes);
        let per = (i.width / TW).max(1);
        app.shell.zoom_tiles_per_row.set(per);
        let first_row = scroll_row(&nodes, chosen.as_deref(), per, (i.height + 1) / TH);
        for (k, n) in nodes.iter().enumerate().skip((first_row * per) as usize) {
            let k = k as u16 - first_row * per;
            let x = i.x + (k % per) * TW;
            let y = i.y + (k / per) * TH;
            if y + TH > i.y + i.height + 1 {
                break;
            }
            big_tile(p, app, theme, n, Rect::new(x, y, TW - 1, TH), chosen.as_deref() == Some(n.name()));
        }
        if nodes.is_empty() {
            p.put(i.x, i.y, "no node matches", theme.muted());
        }
    });
}

/// The first row of tiles shown, so the chosen one is on screen.
fn scroll_row(nodes: &[&NodeView<'_>], chosen: Option<&str>, per: u16, rows: u16) -> u16 {
    let at = nodes.iter().position(|n| Some(n.name()) == chosen).unwrap_or(0) as u16 / per.max(1);
    let rows = rows.max(1);
    at.saturating_sub(rows - 1)
}

fn big_tile(p: &mut Painter<'_>, app: &App, theme: &Theme, n: &NodeView<'_>, r: Rect, chosen: bool) {
    let sev = node_sev(app, n);
    tile_box(p, theme, r, sev, chosen);
    tile_title(p, theme, app, n.name(), r, chosen);
    let (x, y, w) = (r.x, r.y, r.width);
    if n.node.lag_s >= crate::model::FOLD_LAG_S {
        p.putr(x + w - 1, y, &format!(" lag {}s ", n.node.lag_s), theme.sev(Severity::Warn));
    }
    let iw = w.saturating_sub(4) as usize;
    if !n.node.reachable {
        p.put(x + 2, y + 1, &tr(&format!("↯ {}", n.node.down_word()), iw), theme.sev(Severity::Crit));
        p.put(x + 2, y + 2, &tr(n.node.down_detail().unwrap_or(""), iw), theme.muted());
        return;
    }
    let mem_sev = severity::node(n.mem_pct);
    let cpu_sev = severity::node(n.cpu_pct);
    let bar_line = |p: &mut Painter<'_>, yy: u16, pct: Option<f64>, sev: Severity| {
        let (fill, empty) = bar::share(pct.unwrap_or(0.0), iw);
        let after = p.put(x + 2, yy, &fill, style_of(theme, sev, theme.accent()));
        p.put(after, yy, &empty, style_of(theme, sev, theme.accent()));
    };
    bar_line(p, y + 1, n.mem_pct, mem_sev);
    let total = n.node.mem_total.map_or("—".to_string(), |t| format!("{:.0}", fmt::gib(t)));
    p.put(x + 2, y + 2, &tr(&format!("mem {} {:.1}/{total} GiB", fmt::pct(n.mem_pct), fmt::gib(n.node.mem_used)), iw), style_of(theme, mem_sev, theme.text()));
    bar_line(p, y + 3, n.cpu_pct, cpu_sev);
    let cores = n.node.cores.map_or("—".to_string(), |c| format!("{c:.0}"));
    p.put(x + 2, y + 4, &tr(&format!("cpu {} {:.1}/{cores} c", fmt::pct(n.cpu_pct), n.busy_cores.unwrap_or(0.0)), iw), style_of(theme, cpu_sev, theme.text()));
    p.put(x + 2, y + 5, &spark(app, n.name(), iw), style_of(theme, sev, theme.muted()));
    let top = n.users.iter().max_by_key(|u| u.mem_bytes);
    match top {
        Some(u) => {
            let who = u.person.clone().unwrap_or_else(|| u.user.clone());
            p.put(x + 2, y + 6, &tr(&format!("{who} {}", fmt::pct(u.mem_pct)), iw), theme.person());
        }
        None => {
            p.put(x + 2, y + 6, "the server alone", theme.muted());
        }
    }
}

/// The right end of the zoom's top border.
pub fn zoom_summary(app: &App) -> String {
    let sort = match app.tree.sort {
        crate::model::SortKey::Pressure => "pressure",
        crate::model::SortKey::Mem => "memory",
        crate::model::SortKey::Cpu => "CPU",
        crate::model::SortKey::Name => "name",
    };
    format!("sorted by {sort} · s · / filter · ⏎ opens one")
}
