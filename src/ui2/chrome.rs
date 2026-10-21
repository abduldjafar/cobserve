//! The chrome (`01-screen.md` *Chrome*): the top bar — the name, the day line or its chip, the
//! path while zoomed, the clock — and the status bar, whose hints come from the `Action` table and
//! whose right end is the poll line.

use super::paint::Painter;
use crate::action;
use crate::app::App;
use crate::fmt;
use crate::prayer::{Alert, Prayer};
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::layout::Density;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// What sits at the right of the top bar: the clock and whether the numbers are live.
fn clock_text(app: &App) -> (String, ClockState) {
    let now = app.now();
    if app.shell.replay > 0 {
        let then = now - i64::from(app.shell.replay) * 60;
        return (format!("◷ {} · replay −{}m  ", app.time.format(then, "%H:%M"), app.shell.replay), ClockState::Replay);
    }
    let ck = format!("{} {}", app.time.hms(now), app.time.label(now));
    if app.paused {
        (format!("○ paused   {ck} "), ClockState::Paused)
    } else if app.is_stale() {
        let age = app.data_age().map(|d| fmt::dur(d.as_secs_f64())).unwrap_or_default();
        (format!("◌ stale {age}   {ck} "), ClockState::Stale)
    } else {
        (format!("● live   {ck} "), ClockState::Live)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClockState {
    Live,
    Paused,
    Stale,
    Replay,
}

pub fn top_bar(p: &mut Painter<'_>, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 || area.width < 20 {
        return;
    }
    let (y, w) = (area.y, area.width);
    p.put(1, y, "☗", theme.accent());
    p.put(3, y, "cobserve", theme.strong());
    p.put(12, y, &format!("· {}", app.shell.profile), theme.muted());

    let (right, state) = clock_text(app);
    let right_w = fmt::width(&right) as u16;
    let start = p.putr(w - 1, y, &right, theme.text());
    match state {
        ClockState::Live => {
            p.put(start, y, "●", theme.sev(Severity::Ok));
        }
        ClockState::Paused => {
            p.put(start, y, &right, theme.muted());
        }
        ClockState::Stale | ClockState::Replay => {
            p.put(start, y, &right, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
    }
    let middle_end = w.saturating_sub(right_w + 3);

    if app.shell.local {
        let mut path = "dashboard › this machine".to_string();
        if let Some(drill) = &app.drill {
            // Narrow, the browser alone: `dashboard › Chrome`.
            path = if w >= 100 { format!("{path} › {}", drill.program) } else { format!("dashboard › {}", crate::browser::short(&drill.program)) };
        }
        p.within(Rect::new(0, y, middle_end, 1), |p| {
            p.put(22, y, &path, theme.muted());
        });
        p.putr(middle_end, y, "esc back ", theme.muted());
        return;
    }
    if let Some(zoom) = app.shell.zoom {
        let mut path = format!("dashboard › {}", zoom.slug());
        if zoom == crate::ui2::PanelId::Node
            && let Some(node) = super::chosen_node(app)
        {
            path.push_str(&format!(" › {node}"));
        }
        p.within(Rect::new(0, y, middle_end, 1), |p| {
            p.put(22, y, &path, theme.muted());
        });
        p.putr(middle_end, y, "esc back ", theme.muted());
        return;
    }

    let now = app.now();
    if let Some(alert) = app.prayer_alert() {
        let text = match alert {
            Alert::Soon { moment, left } => format!("◷ {} in {} ", moment.name(), fmt::countdown(left)),
            Alert::Now { moment, .. } => format!("☾ time for {} ", moment.name()),
        };
        let sev = if matches!(alert, Alert::Soon { .. }) { Severity::Warn } else { Severity::Ok };
        p.putr(middle_end, y, &text, theme.tint(sev).add_modifier(Modifier::BOLD));
        return;
    }
    let Some(schedule) = app.prayers.schedule.as_ref() else { return };
    let avail = w as i32 - right_w as i32 - 24;
    if app.shell.density == Density::Comfortable && avail >= 60 {
        let today = schedule.today();
        let friday = today.is_friday();
        let stops: Vec<(&str, i64)> = Prayer::ALL.into_iter().filter_map(|pr| Some((pr.name(friday), today.at(pr)?))).collect();
        let next = stops.iter().position(|s| s.1 > now);
        let fill = ((avail - 58) / 6).max(1) as u16;
        let names = avail >= 80;
        let mut x = 22u16;
        p.within(Rect::new(0, y, middle_end, 1), |p| {
            for (i, (name, at)) in stops.iter().enumerate() {
                let passed = *at <= now;
                let is_next = Some(i) == next;
                let label = if names { format!("{name} {}", app.time.local_hm(*at)) } else { app.time.local_hm(*at) };
                let style = if is_next {
                    theme.strong()
                } else if passed {
                    theme.text()
                } else {
                    theme.muted()
                };
                x = p.put(x, y, &label, style);
                if is_next {
                    x = p.put(x, y, &format!(" in {}", fmt::countdown(at - now)), theme.accent().add_modifier(Modifier::BOLD));
                }
                x += 1;
                if let Some((_, to)) = stops.get(i + 1) {
                    if now >= *to {
                        p.run(x, y, '━', fill, theme.accent());
                        x += fill + 1;
                    } else if now < *at {
                        p.run(x, y, '┄', fill, theme.muted());
                        x += fill + 1;
                    } else {
                        let gone = (((now - at) as f64 / (to - at).max(1) as f64) * f64::from(fill)).round() as u16;
                        let gone = gone.min(fill);
                        p.run(x, y, '━', gone, theme.accent());
                        p.put(x + gone, y, "●", theme.accent().add_modifier(Modifier::BOLD));
                        p.run(x + gone + 1, y, '┄', fill - gone, theme.muted());
                        x += fill + 2;
                    }
                }
            }
        });
    } else if let Some(next) = schedule.next_prayer(now) {
        p.putr(middle_end, y, &format!("☾ {} in {} ", next.name(), fmt::countdown(next.at - now)), theme.muted());
    }
}

/// This machine in a few words, for the status bar from 140 columns: `this Mac · cpu 31% · mem
/// 12/32 GiB` (LOCAL has no panel; `0` zooms it).
pub fn local_line(app: &App) -> Option<String> {
    let sample = app.local.latest.as_ref()?;
    if sample.error.is_some() {
        return None;
    }
    let cpu = app.local.cpu.map(|c| format!("cpu {}", fmt::pct0(c.busy_pct)));
    let mem = sample.memory.map(|m| format!("mem {:.0}/{:.0} GiB", fmt::gib(m.used()), fmt::gib(m.total)));
    let parts: Vec<String> = std::iter::once("this Mac".to_string()).chain(cpu).chain(mem).collect();
    (parts.len() > 1).then(|| parts.join(" · "))
}

/// The poll line: `<preset> · 9/9 nodes · 0.8 s · read-only`.
pub fn poll_line(app: &App, with_preset: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if with_preset {
        parts.push(app.shell.preset.name().to_string());
    }
    if let Some(snapshot) = app.snapshot() {
        let reachable = snapshot.nodes.iter().filter(|n| n.reachable).count();
        parts.push(format!("{reachable}/{} nodes", snapshot.nodes.len()));
        if let Some(ms) = snapshot.nodes.iter().filter_map(|n| n.poll_ms).max() {
            parts.push(format!("{:.1} s", f64::from(ms) / 1000.0));
        }
    }
    parts.push("read-only".into());
    parts.join(" · ")
}

/// The status bar: the hints of the scope in force, as many as fit, then the poll line.
pub fn status_bar(p: &mut Painter<'_>, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let (y, w) = (area.y, area.width);
    let status = match app.shell.zoom {
        Some(crate::ui2::PanelId::Node) => {
            let poll = crate::ui2::panels::node::with_chosen(app, |n| n.node.poll_ms).flatten();
            poll.map_or("read-only".to_string(), |ms| format!("poll {:.1} s · read-only", f64::from(ms) / 1000.0))
        }
        _ => poll_line(app, app.shell.zoom.is_none()),
    };
    let status = match local_line(app).filter(|_| w >= 140) {
        Some(local) => format!("{local}   {status}"),
        None => status,
    };
    // A filter in progress takes the right end: `/ query · esc clears`.
    if let Some(f) = &app.shell.filter {
        let cursor = if f.typing { "█" } else { "" };
        let text = format!("/ {}{cursor} · esc clears", f.query);
        let x = w.saturating_sub(40);
        p.clear(Rect::new(x, y, 40.min(w), 1), theme.surface());
        p.put(x + 1, y, &text, theme.accent().add_modifier(Modifier::BOLD).patch(theme.surface()));
        let limit = x;
        let mut hx = 1u16;
        for (key, desc) in hints(app) {
            let need = (fmt::width(&key) + fmt::width(&desc) + 4) as u16;
            if hx + need > limit {
                continue;
            }
            p.put(hx, y, &key, theme.accent().add_modifier(Modifier::BOLD));
            p.put(hx + fmt::width(&key) as u16 + 1, y, &desc, theme.muted());
            hx += need;
        }
        return;
    }
    let limit = w.saturating_sub(fmt::width(&status) as u16 + 2);
    let mut x = 1u16;
    for (key, desc) in hints(app) {
        let need = (fmt::width(&key) + fmt::width(&desc) + 4) as u16;
        if x + need > limit {
            continue;
        }
        p.put(x, y, &key, theme.accent().add_modifier(Modifier::BOLD));
        p.put(x + fmt::width(&key) as u16 + 1, y, &desc, theme.muted());
        x += need;
    }
    p.putr(w - 1, y, &status, theme.muted());
}

/// The hints the status bar offers, in rank order: `(key, what it does)`.
pub fn hints(app: &App) -> Vec<(String, String)> {
    if let Some(keys) = session_mode_hints(app) {
        return keys.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    }
    if let Some(keys) = session_full_hints(app) {
        return keys;
    }
    if app.shell.local {
        // This machine, and a browser opened from it: their own keys.
        let keys: &[(&str, &str)] = if app.drill.is_some() {
            &[("esc", "back"), ("↑↓", "move"), ("⏎", "open a site"), ("x", "stop process"), ("r", "read tabs")]
        } else {
            &[("esc", "back"), ("↑↓", "choose"), ("c", "chrome"), ("s", "sort"), ("g", "group")]
        };
        return keys.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    }
    let actions = action::actions_for(app);
    if let Some(zoom) = app.shell.zoom {
        // A zoom: esc first, then the zoom's own actions.
        let mut out = vec![("esc".to_string(), "back".to_string())];
        out.extend(actions.iter().filter(|a| matches!(a.scope, action::Scope::Zoomed(p) if p == zoom)).map(|a| (a.shown.to_string(), a.label.to_string())));
        return out;
    }
    actions
        .iter()
        .filter(|a| !a.keys.is_empty())
        .take(7)
        .map(|a| {
            let label = if a.id == "zoom" { format!("open {}", app.shell.focus.slug()) } else { a.label.to_string() };
            (a.shown.to_string(), label)
        })
        .collect()
}

/// While the zoomed sessions name a session, look for one or pick a new one's folder, every key
/// is theirs: the status bar says which (the keys of today's view 7, `App::on_session_key`).
fn session_mode_hints(app: &App) -> Option<&'static [(&'static str, &'static str)]> {
    use crate::claude::{Kind, Mode};
    if app.shell.zoom != Some(crate::ui2::PanelId::Sessions) {
        return None;
    }
    Some(match &app.claude.mode {
        Mode::Naming(_) => &[("⏎", "keep the name"), ("esc", "cancel")],
        Mode::Finding { .. } => &[("↑↓", "choose"), ("⏎", "go there"), ("esc", "back")],
        Mode::Opening(p) if p.kind == Kind::Query => &[("↑↓", "choose"), ("⏎", "open it on that server"), ("esc", "cancel")],
        Mode::Opening(p) if p.searching() => &[("↑↓", "choose"), ("⏎", "open the session there"), ("→", "go in"), ("esc", "clear the search")],
        Mode::Opening(_) => &[("↑↓", "choose"), ("⏎", "open the session there"), ("→", "go in"), ("←", "up"), ("esc", "cancel")],
        _ => return None,
    })
}

/// A session opened full: its program has every key but `ctrl+\\`.
fn session_full_hints(app: &App) -> Option<Vec<(String, String)>> {
    use crate::claude::Kind;
    if app.shell.zoom != Some(crate::ui2::PanelId::Sessions) || !app.shell.sessions_full {
        return None;
    }
    let s = app.claude.current()?;
    let who = match s.kind {
        _ if s.console.is_some() => "the query",
        Kind::Terminal => "the shell",
        other => other.title(),
    };
    Some(if s.console.is_some() || s.pane.is_running() {
        vec![("ctrl+\\".into(), format!("the list · every other key goes to {who}"))]
    } else {
        vec![("⏎".into(), format!("start {}", s.kind.title())), ("ctrl+\\".into(), "the list".into())]
    })
}
