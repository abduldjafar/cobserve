//! The palette `:` and the keys box `?` (`05-keys.md` *The palette*, `drawPalette()` in the
//! simulation): one box over whatever is under it, a query line, the matches filtered by fuzzy
//! subsequence with the matched letters marked, the channels along the bottom border.
//!
//! The actions channel lists the panels (each with its digit), then the actions of the scope in
//! force by rank, then the palette-only ones — all from the `Action` table, so the palette, the
//! status bar and `?` cannot disagree. The other channels list the objects on screen; choosing one
//! opens its panel.

use crate::action::{self, Action};
use crate::app::App;
use crate::fmt;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{rp, tr, Painter};
use crate::ui2::state::Palette;
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub const CHANNELS: [&str; 9] = ["actions", "nodes", "people", "queries", "jobs", "dags", "tickets", "sessions", "tape"];

/// At most this many matches are listed.
const ROWS: usize = 8;

/// What a row does when run.
#[derive(Debug, Clone)]
pub enum Target {
    /// `0`: the grid.
    Grid,
    /// A panel, zoomed.
    Panel(PanelId),
    /// An action, run as if its first key were pressed.
    Action(&'static Action),
    /// A node: chosen, and its panel zoomed.
    Node(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub key: String,
    pub target: Target,
}

/// The simulation's `fz`: `query`'s letters in order inside `text`, case-insensitive. The score
/// counts a gap before a letter as 2 and a letter not at a word's start as 1, so lower is better;
/// returns the matched positions (chars) too.
pub fn fuzzy(query: &str, text: &str) -> Option<(u32, Vec<usize>)> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let s: Vec<char> = text.to_lowercase().chars().collect();
    let (mut j, mut score, mut last) = (0usize, 0u32, None::<usize>);
    let mut idx = Vec::new();
    for (i, ch) in s.iter().enumerate() {
        if j == q.len() {
            break;
        }
        if *ch == q[j] {
            idx.push(i);
            score += if last.is_some_and(|l| l + 1 == i) { 0 } else { 2 };
            score += if i == 0 || s[i - 1] == ' ' { 0 } else { 1 };
            last = Some(i);
            j += 1;
        }
    }
    (j == q.len()).then_some((score, idx))
}

/// The actions channel: the panels, then the actions in force, then the palette-only ones.
fn action_items(app: &App, help: bool) -> Vec<Item> {
    let mut items = vec![Item { label: "dashboard".into(), detail: "the dashboard view".into(), key: "0".into(), target: Target::Grid }];
    for id in PanelId::ALL {
        items.push(Item { label: id.slug().into(), detail: format!("the {} view", id.slug()), key: id.number().to_string(), target: Target::Panel(id) });
    }
    for a in action::actions_for(app) {
        if matches!(a.id, "focus" | "choose" | "palette" | "keys") || (help && a.keys.is_empty()) {
            continue;
        }
        items.push(Item { label: if a.phrase.is_empty() { a.label.into() } else { a.phrase.into() }, detail: a.detail.into(), key: a.shown.into(), target: Target::Action(a) });
    }
    items
}

/// The objects of a channel.
fn channel_items(app: &App, channel: &str) -> Vec<Item> {
    let mut items = Vec::new();
    match channel {
        "nodes" => {
            app.with_view(|v| {
                for n in &v.nodes {
                    let detail = if n.node.reachable {
                        format!("{} memory · cpu {} · {} queries", fmt::pct(n.mem_pct), fmt::pct(n.cpu_pct), n.users.iter().map(|u| u.queries.len()).sum::<usize>())
                    } else {
                        format!("↯ {}", n.node.down_word())
                    };
                    items.push(Item { label: n.name().into(), detail, key: String::new(), target: Target::Node(n.name().into()) });
                }
            });
        }
        "people" => {
            app.with_view(|v| {
                for u in &v.users {
                    let on = u.nodes.iter().max_by_key(|n| n.mem_bytes).map(|n| n.node.name.clone());
                    let runaway = if u.nodes.iter().any(|n| n.runaway) { " · ! runaway" } else { "" };
                    let longest = u.nodes.iter().map(|n| n.longest_s).fold(0.0, f64::max);
                    let detail = format!("{} on {} · {} queries · longest {}{runaway}", fmt::bytes(u.mem_bytes), fmt::plural(u.nodes.len(), "node", "nodes"), u.queries, fmt::dur(longest));
                    let target = on.map_or(Target::Panel(PanelId::Nodes), Target::Node);
                    items.push(Item { label: u.label(), detail, key: String::new(), target });
                }
            });
        }
        "queries" => {
            app.with_view(|v| {
                for n in &v.nodes {
                    for u in &n.users {
                        for q in &u.queries {
                            let detail = format!("{} · {} · {}", u.label(), n.name(), fmt::dur(q.query.elapsed_s));
                            items.push(Item { label: q.query.query_id.clone(), detail, key: String::new(), target: Target::Node(n.name().into()) });
                        }
                    }
                }
            });
        }
        "jobs" => {
            for j in &app.queue.jobs {
                items.push(Item { label: j.label(), detail: crate::app::job_words(j), key: String::new(), target: Target::Panel(PanelId::Queue) });
            }
        }
        "dags" => {
            for d in &app.airflow.dags {
                let detail = d.schedule.clone().or_else(|| d.cron.clone()).unwrap_or_default();
                items.push(Item { label: d.id.clone(), detail, key: String::new(), target: Target::Panel(PanelId::Airflow) });
            }
        }
        "tickets" => {
            for t in &app.jira.tickets {
                items.push(Item { label: format!("{} {}", t.key, t.summary), detail: t.status.clone(), key: String::new(), target: Target::Panel(PanelId::Jira) });
            }
        }
        "sessions" => {
            for (i, s) in app.claude.list.iter().enumerate() {
                let number = crate::claude::Sessions::number_of(i);
                items.push(Item { label: s.label(), detail: s.dir.clone(), key: number.to_string(), target: Target::Panel(PanelId::Sessions) });
            }
        }
        "tape" => {
            for e in app.tape.newest_first().take(200) {
                let text: String = e.parts.iter().map(|(t, _)| t.as_str()).collect();
                items.push(Item { label: text, detail: app.time.local_hm(e.at as i64), key: String::new(), target: Target::Panel(PanelId::Tape) });
            }
        }
        _ => {}
    }
    items
}

/// The matches of the open palette, best first, with their matched positions.
pub fn matches(app: &App, palette: &Palette) -> Vec<(Item, Vec<usize>)> {
    let base = if palette.channel == 0 { action_items(app, palette.help) } else { channel_items(app, CHANNELS[palette.channel % CHANNELS.len()]) };
    let mut scored: Vec<(u32, Item, Vec<usize>)> = base.into_iter().filter_map(|it| fuzzy(&palette.query, &it.label).map(|(s, idx)| (s, it, idx))).collect();
    scored.sort_by_key(|(s, _, _)| *s);
    scored.into_iter().map(|(_, it, idx)| (it, idx)).collect()
}

/// Run what a row says.
pub fn run(app: &mut App, target: Target) {
    match target {
        Target::Grid => app.shell.zoom = None,
        Target::Panel(id) => {
            app.shell.focus = id;
            app.shell.zoom = Some(id);
        }
        Target::Node(name) => {
            app.shell.chosen = Some(name);
            app.shell.focus = PanelId::Node;
            app.shell.zoom = Some(PanelId::Node);
        }
        Target::Action(a) => {
            let key = a.keys.first().copied().unwrap_or("");
            (a.run)(app, key);
        }
    }
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, palette: &Palette) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    if sw < 30 || sh < 8 {
        return;
    }
    let all = matches(app, palette);
    let rows = all.len().clamp(1, ROWS);
    let w = (sw - 4).min(78);
    let x = (sw - w) / 2;
    let h = (rows as u16 + 5).min(sh);
    let y = ((sh.saturating_sub(h)) / 3).max(2).min(sh.saturating_sub(h));
    let scope = match app.shell.zoom {
        Some(z) => z.slug(),
        None => "dash",
    };
    let title = if palette.help { format!("? keys · {scope}") } else { ": ".to_string() };
    let r = Rect::new(x, y, w, h);
    frame::overlay(p, theme, r, &title, theme.strong(), "");
    let surface = theme.surface();
    p.put(x + 2, y + 1, "› ", theme.accent().patch(surface));
    let after = p.put(x + 4, y + 1, &palette.query, theme.text().patch(surface));
    p.put(after, y + 1, "█", theme.text().patch(surface));
    if palette.query.is_empty() {
        let hint = if palette.help { "type to filter the keys of this screen" } else { "type two or three letters · tab for nodes, people, queries, jobs…" };
        p.put(x + 5, y + 1, &tr(hint, w as usize - 7), theme.muted().patch(surface));
    }
    let sep = format!("├{}┤", "─".repeat(w as usize - 2));
    p.put(x, y + 2, &sep, theme.border().patch(surface));
    let sel = palette.sel.min(all.len().saturating_sub(1));
    // The window of rows around the choice.
    let first = sel.saturating_sub(ROWS - 1);
    for (k, (item, idx)) in all.iter().skip(first).take(rows).enumerate() {
        let yy = y + 3 + k as u16;
        let chosen = first + k == sel;
        let row_style = if chosen { theme.selected().patch(surface) } else { surface };
        if chosen {
            p.style(Rect::new(x + 1, yy, w - 2, 1), theme.selected());
            p.put(x + 1, yy, "▌", theme.accent().patch(theme.selected()));
        }
        let label = rp(&item.label, 30);
        let mut xx = x + 3;
        for (c, ch) in label.chars().enumerate() {
            let style = if idx.contains(&c) { theme.accent().add_modifier(Modifier::BOLD) } else { theme.text() };
            xx = p.put(xx, yy, &ch.to_string(), style.patch(row_style));
        }
        p.put(x + 34, yy, &tr(&item.detail, (w as usize).saturating_sub(40)), theme.muted().patch(row_style));
        if !item.key.is_empty() {
            p.putr(x + w - 2, yy, &item.key, theme.accent().add_modifier(Modifier::BOLD).patch(row_style));
        }
    }
    if all.is_empty() {
        p.put(x + 3, y + 3, "nothing matches · esc", theme.muted().patch(surface));
    }
    p.put(x, y + h - 2, &sep, theme.border().patch(surface));
    let mut cx = x + 2;
    p.put(cx, y + h - 1, " ↑↓ choose   ⏎ run   tab channel › ", theme.muted().patch(surface));
    cx += 34;
    for (i, c) in CHANNELS.iter().enumerate() {
        if cx + fmt::width(c) as u16 > x + w - 2 {
            break;
        }
        let style = if i == palette.channel { theme.text().add_modifier(Modifier::UNDERLINED) } else { theme.muted() };
        p.put(cx, y + h - 1, c, style.patch(surface));
        cx += fmt::width(c) as u16 + 3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_is_the_simulation_s_fz() {
        assert_eq!(fuzzy("exp", "explain this number"), Some((4, vec![0, 1, 2])));
        assert!(fuzzy("exp", "export incident report").is_some());
        assert_eq!(fuzzy("zz", "explain"), None);
        let (a, _) = fuzzy("ai", "setup · airflow").unwrap();
        let (b, _) = fuzzy("ai", "chain").unwrap();
        assert!(a < b, "a word start beats a letter inside one");
    }
}
