//! Panel 4, NODE — the chosen node (`02-panels.md` *4 · NODE*): who holds it, one row a person and
//! then the server, as bars on one scale (the node), so the rows' filled cells add up to the
//! node's (§5.3 made visible). Zoomed: the rows with GiB and cores, the chosen row's query and its
//! SQL, and the node's memory and CPU over the history.

use crate::app::App;
use crate::fmt;
use crate::history::WINDOW_S;
use crate::model::NodeView;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::ui2::glyph::{bar, graph};
use crate::ui2::paint::{lp, tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

/// The closing row's name.
pub const SERVER: &str = "server · caches · merges";

/// One row of WHO HOLDS IT.
pub struct Row {
    /// The cursor's identity: the row's label (`attrib::user_label`); empty for the server.
    pub key: String,
    /// What the row shows: the person, else the user.
    pub name: String,
    pub mem_pct: Option<f64>,
    pub mem_bytes: u64,
    pub cores: f64,
    /// The longest query: elapsed, and whether it is a runaway.
    pub longest: Option<(f64, bool)>,
}

/// The rows of a node: its people by memory (as `model.rs` orders them), then the server.
pub fn rows(n: &NodeView<'_>) -> Vec<Row> {
    let mut out: Vec<Row> = n
        .users
        .iter()
        .map(|u| Row {
            key: u.label(),
            name: u.person.clone().unwrap_or_else(|| u.user.clone()),
            mem_pct: u.mem_pct,
            mem_bytes: u.mem_bytes,
            cores: u.cores,
            longest: u.queries.first().map(|q| (q.query.elapsed_s, q.runaway)),
        })
        .collect();
    let users_mem: u64 = n.users.iter().map(|u| u.mem_bytes).sum();
    let users_cores: f64 = n.users.iter().map(|u| u.cores).sum();
    out.push(Row {
        key: String::new(),
        name: SERVER.into(),
        mem_pct: n.server_mem_pct,
        mem_bytes: n.node.mem_used.saturating_sub(users_mem),
        cores: (n.busy_cores.unwrap_or(0.0) - users_cores).max(0.0),
        longest: None,
    });
    out
}

/// The cursor's row: the one whose key is the cursor, else the first.
pub fn cursor(app: &App, rows: &[Row]) -> usize {
    app.shell.person.as_ref().and_then(|p| rows.iter().position(|r| &r.key == p)).unwrap_or(0)
}

impl Row {
    pub fn is_server(&self) -> bool {
        self.key.is_empty()
    }
}

/// The bars of the rows, `width` cells on one scale, summing to the node's share exactly.
fn cells(rows: &[Row], width: usize) -> Vec<usize> {
    let parts: Vec<f64> = rows.iter().map(|r| r.mem_pct.unwrap_or(0.0) / 100.0).collect();
    bar::largest_remainder(&parts, width)
}

fn row_style(theme: &Theme, r: &Row) -> (Style, Style) {
    if r.is_server() {
        return (Style::default().fg(theme.server_bar), theme.muted());
    }
    let sev = severity::user_share(r.mem_pct);
    let fill = if sev.is_problem() { theme.sev(sev) } else { theme.accent() };
    let text = if sev.is_problem() { theme.sev(sev) } else { theme.text() };
    (fill, text)
}

/// Runs `f` with the chosen node's view, if there is one.
pub fn with_chosen<R>(app: &App, f: impl FnOnce(&NodeView<'_>) -> R) -> Option<R> {
    let chosen = crate::ui2::chosen_node(app)?;
    app.with_view(|v| v.nodes.iter().find(|n| n.name() == chosen).map(f)).flatten()
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let i = super::frame(p, app, theme, PanelId::Node, r, focused);
    if i.width < 4 || i.height == 0 {
        return;
    }
    let drawn = with_chosen(app, |n| {
        p.within(i, |p| body(p, app, theme, n, i, focused));
    });
    if drawn.is_none() {
        p.put(i.x, i.y, "waiting for the first snapshot…", theme.muted());
    }
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, n: &NodeView<'_>, i: Rect, focused: bool) {
    if !n.node.reachable {
        p.put(i.x, i.y, &tr(&format!("↯ {}", n.node.down_word()), i.width as usize), theme.sev(Severity::Crit));
        p.put(i.x, i.y + 1, &tr(n.node.down_detail().unwrap_or(""), i.width as usize), theme.muted());
        return;
    }
    let rows = rows(n);
    let at = cursor(app, &rows);
    let nw = (i.width as usize).saturating_sub(34).clamp(8, 16);
    let bw = (i.width as usize).saturating_sub(nw + 20).max(6);
    let graph_rows = if i.height >= 8 { 3 } else { 0 };
    let room = i.height.saturating_sub(graph_rows) as usize;
    let fills = cells(&rows, bw);
    let first = at.saturating_sub(room.saturating_sub(1));
    for (k, (row, fill)) in rows.iter().zip(&fills).enumerate().skip(first).take(room) {
        let y = i.y + (k - first) as u16;
        if focused && k == at {
            p.band(Rect::new(i.x - 1, y, i.width + 2, 1), theme.selected());
        }
        let (fill_style, text_style) = row_style(theme, row);
        let name_style = if row.is_server() { theme.muted() } else { theme.person() };
        p.put(i.x, y, &tr(&row.name, nw), name_style);
        let bx = i.x + nw as u16 + 1;
        p.put(bx, y, &"█".repeat(*fill), fill_style);
        p.put(bx + *fill as u16, y, &"░".repeat(bw - fill), fill_style);
        p.put(bx + bw as u16 + 1, y, &lp(&fmt::pct(row.mem_pct), 6), text_style);
        if let Some((elapsed, runaway)) = row.longest
            && i.width >= 40
        {
            let text = format!("{}{}", fmt::dur(elapsed), if runaway { " !" } else { "" });
            p.putr(i.x + i.width, y, &text, if runaway { theme.sev(Severity::Crit) } else { theme.muted() });
        }
    }
    if graph_rows > 0 {
        let y = i.y + i.height - 3;
        let tier = app.caps.graph;
        let per = graph::per_column(tier);
        let samples = app.history.node(n.name()).map(|h| graph::stretch(h.mem_pct.buckets(app.history.now(), WINDOW_S, i.width as usize * per))).unwrap_or_default();
        graph::area(p, tier, Rect::new(i.x, y, i.width, 2), &samples, 100.0, &|v| {
            let s = severity::node(Some(v));
            if s.is_problem() { theme.sev(s) } else { theme.accent() }
        });
        p.put(i.x, y + 2, &super::fleet::window_label(), theme.muted());
        let people = rows.len() - 1;
        let legend = format!("memory · {} + the server = {}   ⏎ open · e explain", fmt::plural(people, "person", "people"), fmt::pct(n.mem_pct));
        p.putr(i.x + i.width, y + 2, &tr(&legend, i.width as usize - 6), theme.muted());
    }
}

/// The zoom's title: `clickhouse3 · pay-ch-node-1:9000`; its right end the node's facts.
pub fn zoom_title(app: &App) -> Option<(String, String)> {
    with_chosen(app, |n| {
        let node = n.node;
        let name = if node.host.is_empty() { node.name.clone() } else { format!("{} · {}:{}", node.name, node.host, node.port) };
        let mut facts = vec![format!("shard {} · replica {}", node.shard, node.replica)];
        if !node.version.is_empty() {
            facts.push(format!("v{}", node.version));
        }
        if let Some(up) = node.uptime_s {
            facts.push(format!("up {}", fmt::dur(up as f64)));
        }
        if let Some(ms) = node.poll_ms {
            facts.push(format!("poll {:.1} s", f64::from(ms) / 1000.0));
        }
        facts.push("esc back".into());
        (name, facts.join(" · "))
    })
}

pub fn draw_zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let drawn = with_chosen(app, |n| zoomed(p, app, theme, n, i));
    if drawn.is_none() {
        p.put(i.x, i.y, "waiting for the first snapshot…", theme.muted());
    }
}

fn zoomed(p: &mut Painter<'_>, app: &App, theme: &Theme, n: &NodeView<'_>, i: Rect) {
    if !n.node.reachable {
        p.put(i.x, i.y, &format!("↯ {}", n.node.down_word()), theme.sev(Severity::Crit));
        p.put(i.x, i.y + 1, &tr(n.node.down_detail().unwrap_or(""), i.width as usize), theme.muted());
        return;
    }
    let lw = (i.width as f64 * 0.55).floor().min(66.0) as u16;
    let rows = rows(n);
    let at = cursor(app, &rows);
    let mut y = i.y;
    // The rows run to the divider, two cells past the left half.
    p.within(Rect::new(i.x.saturating_sub(1), i.y, lw + 3, i.height), |p| {
        p.put(i.x, y, "WHO HOLDS IT", theme.strong());
        p.put(i.x + 14, y, "memory, share of the node · the rows add up to the node (§5.3)", theme.muted());
        y += 1;
        let bw = (lw as usize).saturating_sub(36).max(4);
        let fills = cells(&rows, bw);
        for (k, (row, fill)) in rows.iter().zip(&fills).enumerate() {
            if y >= i.y + i.height {
                break;
            }
            if k == at {
                p.style(Rect::new(i.x - 1, y, lw + 1, 1), theme.selected());
            }
            let (fill_style, text_style) = row_style(theme, row);
            p.put(i.x, y, &tr(&row.name, 16), if row.is_server() { theme.muted() } else { theme.person() });
            p.put(i.x + 17, y, &"█".repeat(*fill), fill_style);
            p.put(i.x + 17 + *fill as u16, y, &"░".repeat(bw - fill), fill_style);
            let text = format!("{} {}G {}c", lp(&fmt::pct(row.mem_pct), 6), lp(&format!("{:.1}", fmt::gib(row.mem_bytes)), 5), lp(&format!("{:.2}", row.cores), 5));
            p.put(i.x + 18 + bw as u16, y, &text, text_style);
            y += 1;
        }
        y += 1;
        // Under the rows, what the chosen row is doing.
        match rows.get(at).filter(|r| !r.is_server()).map(|r| r.key.clone()) {
            Some(key) => {
                if let Some(u) = n.users.iter().find(|u| u.label() == key)
                    && let Some(q) = u.queries.first()
                {
                    let row = q.query;
                    let redash = row.redash_query_id.map_or("ad hoc".to_string(), |id| format!("Redash #{id}"));
                    let short: String = row.query_id.chars().take(8).collect();
                    let head = format!("{short} · {redash} · {}{}", fmt::dur(row.elapsed_s), if q.runaway { " !" } else { "" });
                    p.put(i.x, y, &tr(&head, lw as usize), if q.runaway { theme.sev(Severity::Crit).add_modifier(Modifier::BOLD) } else { theme.strong() });
                    y += 1;
                    let progress = q.progress.map_or("—".to_string(), |f| format!("{}% done", (f * 100.0).round() as i64));
                    let read = format!("read {} / {} rows · {progress} · E estimate", fmt::bytes(row.read_bytes), fmt::rows(row.read_rows));
                    p.put(i.x, y, &tr(&read, lw as usize), theme.muted());
                    y += 1;
                    let sql = crate::sqltext::strip_comments(&row.sql);
                    for line in crate::sqltext::layout(&sql, (lw as usize).saturating_sub(2)) {
                        if y >= i.y + i.height {
                            break;
                        }
                        let text: String = line.iter().map(|(t, _)| t.as_str()).collect();
                        p.put(i.x, y, "▎", theme.accent());
                        p.put(i.x + 2, y, &tr(&text, (lw as usize).saturating_sub(2)), theme.muted());
                        y += 1;
                    }
                }
            }
            None => {
                p.put(i.x, y, &tr(&format!("the server itself: caches, merges, dictionaries · {} active parts", fmt::count(n.node.active_parts)), lw as usize), theme.muted());
                y += 1;
                p.put(i.x, y, &tr("server.mem_pct = node mem% − Σ user.mem_pct · e explains it", lw as usize), theme.muted());
            }
        }
    });

    // Right: memory and CPU over the history, then the node's own facts.
    let rx = i.x + lw + 3;
    let rw = (i.x + i.width).saturating_sub(rx);
    if rw < 10 {
        return;
    }
    for k in 0..i.height {
        p.put(rx - 1, i.y + k, "│", theme.border());
    }
    let gh = (i.height.saturating_sub(8) / 2).max(3);
    let tier = app.caps.graph;
    let per = graph::per_column(tier);
    let mut y = i.y;
    let h = app.history.node(n.name());
    let window = (WINDOW_S / 60.0).round() as u64;
    let total = n.node.mem_total.map_or("—".to_string(), |t| format!("{:.0} GiB", fmt::gib(t)));
    let cores = n.node.cores.map_or("—".to_string(), |c| format!("{c:.0} cores"));
    for (label, right, series, pct) in [
        ("MEMORY", format!("{} of {total}", fmt::pct(n.mem_pct)), h.map(|h| &h.mem_pct), n.mem_pct),
        ("CPU", format!("{} of {cores}", fmt::pct(n.cpu_pct)), h.map(|h| &h.cpu_pct), n.cpu_pct),
    ] {
        if y + gh + 1 >= i.y + i.height {
            break;
        }
        p.put(rx, y, &format!("{label} · {window} min"), theme.strong());
        let sev = severity::node(pct);
        p.putr(rx + rw, y, &right, if sev.is_problem() { theme.sev(sev) } else { theme.text() });
        y += 1;
        let samples = series.map(|s| graph::stretch(s.buckets(app.history.now(), WINDOW_S, rw as usize * per))).unwrap_or_default();
        graph::area(p, tier, Rect::new(rx, y, rw, gh), &samples, 100.0, &|v| {
            let s = severity::node(Some(v));
            if s.is_problem() { theme.sev(s) } else { theme.accent() }
        });
        y += gh;
        p.put(rx, y, &super::fleet::window_label(), theme.muted());
        p.putr(rx + rw, y, "now", theme.muted());
        y += 2;
    }
    if y < i.y + i.height {
        let free = n.node.mem_total.map(|t| t.saturating_sub(n.node.mem_used));
        let free_text = free.map_or("free —".to_string(), |f| format!("free {:.1} GiB", fmt::gib(f)));
        let line = format!("lag {} s · {} parts · {free_text}", n.node.lag_s, fmt::count(n.node.active_parts));
        p.put(rx, y, &tr(&line, rw as usize), theme.muted());
    }
}
