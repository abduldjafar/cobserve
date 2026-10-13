//! Panel 1, FLEET (`02-panels.md` *1 · FLEET*): the fleet's memory share in big digits with its
//! denominator, the CPU share beside it, the counts under it, and the history as a graph — memory
//! as the filled area, CPU as dots on the same fixed scale.

use crate::app::App;
use crate::fmt;
use crate::history::WINDOW_S;
use crate::model::{fleet_totals, FleetTotals};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::ui2::frame::{self, Title};
use crate::ui2::glyph::{big, graph};
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// What the frame says: the worst node's severity and the summary.
pub fn title(app: &App) -> (Severity, String) {
    let Some((mut t, worst, down, hot)) = app.with_view(|v| {
        let t = fleet_totals(v);
        let sevs: Vec<Severity> = v.nodes.iter().filter(|n| n.node.reachable).map(|n| super::nodes::node_sev(app, n)).collect();
        let worst = sevs.iter().copied().max().unwrap_or(Severity::None);
        let hot = sevs.iter().filter(|s| s.is_problem()).count();
        (t, worst, v.nodes.iter().filter(|n| !n.node.reachable).count(), hot)
    }) else {
        return (Severity::None, String::new());
    };
    t.hot = hot;
    let mut parts = Vec::new();
    if t.runaways > 0 {
        parts.push(format!("{} runaway", t.runaways));
    }
    if down > 0 {
        parts.push(format!("{down} ↯"));
    }
    parts.push(format!("{} hot", t.hot));
    parts.push(fmt::plural(t.nodes, "node", "nodes"));
    (worst, parts.join(" · "))
}

/// `268/640 GiB`: whole GiB from a hundred, one decimal under it.
pub fn gib_of(used: u64, total: u64) -> String {
    let (u, t) = (fmt::gib(used), fmt::gib(total));
    if t >= 100.0 { format!("{u:.0}/{t:.0} GiB") } else { format!("{u:.1}/{t:.1} GiB") }
}

/// `52/160 c`.
pub fn cores_of(t: &FleetTotals) -> String {
    if t.cores >= 100.0 { format!("{:.0}/{:.0} c", t.busy_cores, t.cores) } else { format!("{:.1}/{:.0} c", t.busy_cores, t.cores) }
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let (sev, right) = title(app);
    let i = frame::panel(p, theme, r, &Title { number: Some(1), name: PanelId::Fleet.title(), sev, right: &right, focused });
    if i.height == 0 || i.width == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let Some((t, lag)) = app.with_view(|v| (fleet_totals(v), v.nodes.iter().filter(|n| n.node.reachable).map(|n| n.node.lag_s).max().unwrap_or(0))) else {
        p.put(i.x, i.y, "waiting for the first snapshot…", theme.muted());
        return;
    };
    // While replaying, the minute's shares (the denominators are now's: the fleet did not change).
    let mem_pct = crate::ui2::replay::value(app, t.mem_pct(), Some(&app.history.fleet_mem_pct));
    let cpu_pct = crate::ui2::replay::value(app, t.cpu_pct(), Some(&app.history.fleet_cpu_pct));
    let mut t = t;
    if app.shell.replay > 0 {
        t.mem_used = (mem_pct.unwrap_or(0.0) / 100.0 * t.mem_total as f64) as u64;
        t.busy_cores = cpu_pct.unwrap_or(0.0) / 100.0 * t.cores;
    }
    let mem_sev = severity::node(mem_pct);
    let cpu_sev = severity::node(cpu_pct);
    let num = |sev: Severity| if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.strong() };
    let mut x;
    if i.height >= 4 && i.width >= 60 {
        let text = mem_pct.map_or("—".to_string(), |m| format!("{}%", m.round() as i64));
        x = big::draw(p, i.x, i.y, &text, num(mem_sev));
        let room = (x + 7 - i.x) as usize;
        p.put(i.x, i.y + 3, &tr(&format!("{} memory", gib_of(t.mem_used, t.mem_total)), room), theme.muted());
        p.put(x + 1, i.y, "cpu", theme.muted());
        p.put(x + 1, i.y + 1, &fmt::pct(cpu_pct), num(cpu_sev));
        p.put(x + 1, i.y + 2, &cores_of(&t), theme.muted());
        if i.height >= 5 {
            p.put(i.x, i.y + 4, &tr(&format!("{} queries · {} ! · lag ≤ {lag} s", t.queries, t.runaways), room), theme.muted());
        }
        x += 8;
    } else {
        p.put(i.x, i.y, &format!("mem {} {}", fmt::pct(mem_pct), gib_of(t.mem_used, t.mem_total)), if mem_sev.is_problem() { theme.sev(mem_sev) } else { theme.text() });
        p.put(i.x, i.y + 1, &format!("cpu {} {}", fmt::pct(cpu_pct), cores_of(&t)), if cpu_sev.is_problem() { theme.sev(cpu_sev) } else { theme.text() });
        if i.height >= 3 {
            p.put(i.x, i.y + 2, &format!("{} q · {} !", t.queries, t.runaways), theme.muted());
        }
        x = i.x + 26;
    }
    let gw = (i.x + i.width).saturating_sub(x + 1);
    let gh = i.height.saturating_sub(1).max(1);
    if gw <= 8 {
        return;
    }
    let tier = app.caps.graph;
    let per = graph::per_column(tier);
    let now = app.history.now();
    let mem = graph::stretch(app.history.fleet_mem_pct.buckets(now, WINDOW_S, gw as usize * per));
    let cpu = graph::stretch(app.history.fleet_cpu_pct.buckets(now, WINDOW_S, gw as usize));
    let rect = Rect::new(x, i.y, gw, gh);
    graph::area(p, tier, rect, &mem, 100.0, &|v| {
        let s = severity::node(Some(v));
        if s.is_problem() { theme.sev(s) } else { theme.accent() }
    });
    graph::dots(p, rect, &cpu, 100.0, theme.strong());
    if let Some(c) = crate::ui2::replay::column(app, gw) {
        p.bg(Rect::new(x + c, i.y, 1, gh), theme.selection);
    }
    if i.height >= 2 {
        p.put(x, i.y + gh, &window_label(), theme.muted());
        p.putr(x + gw, i.y + gh, "█ mem  • cpu  now", theme.muted());
    }
}

/// The graph's window, printed rather than implied: the history keeps four minutes (the hour
/// comes with feature A1, `07-features.md`).
pub fn window_label() -> String {
    format!("-{}m", (WINDOW_S / 60.0).round() as u64)
}

/// A full-width graph with `100% · 50% · 0% ┤` ticks; returns the row after its labels.
pub fn ticked_graph(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, series: &crate::history::Series) -> u16 {
    let (x, y, gh) = (r.x, r.y, r.height);
    let gw = r.width.saturating_sub(12);
    if gw < 4 || gh == 0 {
        return y;
    }
    for (k, label) in ["100%", "50%", "0%"].iter().enumerate() {
        let row = y + ((k as f64 * f64::from(gh - 1)) / 2.0).round() as u16;
        p.put(x, row, &format!("{} ┤", crate::ui2::paint::lp(label, 5)), theme.muted());
    }
    let tier = app.caps.graph;
    let per = graph::per_column(tier);
    let samples = graph::stretch(series.buckets(app.history.now(), WINDOW_S, gw as usize * per));
    graph::area(p, tier, Rect::new(x + 8, y, gw, gh), &samples, 100.0, &|v| {
        let s = severity::node(Some(v));
        if s.is_problem() { theme.sev(s) } else { theme.accent() }
    });
    p.put(x + 8, y + gh, &window_label(), theme.muted());
    p.putr(x + 8 + gw, y + gh, "now", theme.muted());
    y + gh + 2
}

/// Zoomed (`02-panels.md`): memory then CPU over the history, each with its ticks, then a row a
/// node with the cursor; `⏎` opens the chosen one.
pub fn draw_zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let Some(t) = app.with_view(fleet_totals) else {
        p.put(i.x, i.y, "waiting for the first snapshot…", theme.muted());
        return;
    };
    let gh = (i.height.saturating_sub(8) / 2).max(3);
    let mut y = i.y;
    p.put(i.x, y, "memory", theme.strong());
    p.put(i.x + 8, y, &format!("{} {} · the fleet's share, Σ used ÷ Σ total", fmt::pct(t.mem_pct()), gib_of(t.mem_used, t.mem_total)), theme.muted());
    y = ticked_graph(p, app, theme, Rect::new(i.x, y + 1, i.width, gh), &app.history.fleet_mem_pct);
    if y + 1 < i.y + i.height {
        p.put(i.x, y, "cpu", theme.strong());
        p.put(i.x + 8, y, &format!("{} {}", fmt::pct(t.cpu_pct()), cores_of(&t).replace(" c", " cores")), theme.muted());
        y = ticked_graph(p, app, theme, Rect::new(i.x, y + 1, i.width, gh), &app.history.fleet_cpu_pct);
    }
    if y >= i.y + i.height {
        return;
    }
    p.put(i.x, y, "per node, now", theme.strong());
    y += 1;
    let chosen = crate::ui2::chosen_node(app);
    app.with_view(|v| {
        let nodes = super::nodes::order(app, &v.nodes);
        let at = nodes.iter().position(|n| Some(n.name()) == chosen.as_deref()).unwrap_or(0);
        let room = (i.y + i.height).saturating_sub(y) as usize;
        let first = at.saturating_sub(room.saturating_sub(1));
        let bw = i.width.saturating_sub(50).min(40) as usize;
        for (k, n) in nodes.iter().enumerate().skip(first).take(room) {
            if k == at {
                p.style(Rect::new(i.x.saturating_sub(1), y, i.width + 2, 1), theme.selected());
            }
            let sev = severity::node(n.mem_pct);
            let style = if sev.is_problem() { theme.sev(sev) } else { theme.text() };
            p.put(i.x, y, &crate::ui2::paint::rp(n.name(), 14), theme.strong());
            if n.node.reachable {
                let (fill, empty) = crate::ui2::glyph::bar::share(n.mem_pct.unwrap_or(0.0), bw);
                let bar_style = if sev.is_problem() { theme.sev(sev) } else { theme.accent() };
                let after = p.put(i.x + 15, y, &fill, bar_style);
                p.put(after, y, &empty, bar_style);
                let total = n.node.mem_total.map_or("—".to_string(), |t| format!("{:.0}", fmt::gib(t)));
                let text = format!("{}  {:.1}/{total} GiB   cpu {}", crate::ui2::paint::lp(&fmt::pct(n.mem_pct), 6), fmt::gib(n.node.mem_used), fmt::pct(n.cpu_pct));
                p.put(i.x + 16 + bw as u16, y, &text, style);
            } else {
                p.put(i.x + 15, y, &format!("↯ {}", n.node.down_word()), theme.sev(Severity::Crit));
            }
            y += 1;
        }
    });
}
