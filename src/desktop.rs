//! What the desktop app's own header shows, said by cobserve when it runs inside it.
//!
//! `Cobserve.app` (`desktop/`) draws a header of its own — the views as tabs, the state of each
//! thing watched as a chip — over the terminal cobserve runs in. It learns them from cobserve: an
//! OSC 7700 sequence, written after a frame when what it says has changed, and only when
//! `TERM_PROGRAM` is `cobserve-desktop`; any other terminal never sees it. Pure: the `App` in,
//! JSON out. The numbers are the screen's own (the band, the views), never new ones.

use crate::app::{App, View};
use crate::severity::Severity;
use serde::Serialize;

/// The OSC number the app listens on: private, unused by any terminal.
pub const OSC: u32 = 7700;

/// Whether cobserve runs inside the desktop app.
pub fn inside_the_app() -> bool {
    std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "cobserve-desktop")
}

/// The header's state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct State {
    /// The view on screen: `nodes`, `queue`, `map`, `tape`, `airflow`, `jira`, `sessions`.
    pub view: &'static str,
    /// The fleet's worst, as the masthead marks it: `crit`, `warn`, `ok`, or `none` before a poll.
    pub status: &'static str,
    pub chips: Vec<Chip>,
    /// How many sessions there are, and whether one rang while out of sight.
    pub sessions: usize,
    pub calling: bool,
    pub paused: bool,
    /// The numbers are old (no poll for three intervals).
    pub stale: bool,
    /// The clock as the masthead says it: `15:52`, and its zone `WIB`.
    pub clock: String,
    pub zone: String,
    /// The next prayer: `Maghrib 17:50`; empty when prayer times are off.
    pub next: String,
    /// The filter typed on view 1, if any.
    pub filter: String,
}

/// One thing watched, in a few words, and the view that says more.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chip {
    pub id: &'static str,
    /// The view's number: what a click on the chip opens.
    pub view: u8,
    pub label: &'static str,
    pub text: String,
    /// `crit`, `warn`, `ok` or `none` — colour is severity, as on screen.
    pub sev: &'static str,
}

fn sev(severity: Severity) -> &'static str {
    match severity {
        Severity::Crit => "crit",
        Severity::Warn => "warn",
        Severity::Ok => "ok",
        Severity::None | Severity::Info => "none",
    }
}

/// The header's state for the app as it is now.
pub fn state(app: &App) -> State {
    let now = app.now();
    let mut chips = Vec::new();

    // The fleet: nodes, the hot and the down — the band's FLEET line.
    if let Some(totals) = app.with_view(crate::model::fleet_totals) {
        let down = totals.nodes - totals.reachable;
        let mut text = crate::fmt::plural(totals.nodes, "node", "nodes");
        if down > 0 {
            text.push_str(&format!(" · {down} down"));
        }
        if totals.hot > 0 {
            text.push_str(&format!(" · {} hot", totals.hot));
        }
        let worst = crate::insight::overall(&app.insights());
        chips.push(Chip { id: "fleet", view: 1, label: "ClickHouse", text, sev: sev(worst) });
    }

    // Redash: what waits, the oldest's wait — the band's REDASH line.
    let queue = &app.queue;
    if queue.reachable {
        let waiting = queue.total_waiting();
        let oldest = queue.queues.iter().filter_map(|q| q.oldest_wait_s).max();
        let saturated = queue.queues.iter().any(|q| q.saturated() && q.waiting > 0);
        let text = match (waiting, oldest) {
            (0, _) => format!("{} running", queue.started().len()),
            (n, Some(s)) => format!("{n} waiting · {}", crate::fmt::dur(s as f64)),
            (n, None) => format!("{n} waiting"),
        };
        let severity = crate::severity::wait(oldest.unwrap_or(0), saturated);
        chips.push(Chip { id: "redash", view: 2, label: "Redash", text, sev: sev(if waiting == 0 { Severity::Ok } else { severity }) });
    } else if !queue.is_placeholder() {
        chips.push(Chip { id: "redash", view: 2, label: "Redash", text: "unreachable".into(), sev: "warn" });
    }

    // Airflow: the day's failures, what runs, a stuck run — view 5's line under its title.
    let airflow = &app.airflow;
    if airflow.reachable && airflow.day_read {
        let counts = airflow.counts(now);
        let stuck = airflow.runs.iter().filter(|r| r.is_stuck(now)).count();
        let mut parts = Vec::new();
        if counts.failed > 0 {
            parts.push(format!("{} failed", counts.failed));
        }
        parts.push(format!("{} running", counts.running));
        if stuck > 0 {
            parts.push(format!("{stuck} stuck"));
        }
        let severity = if counts.failed > 0 || stuck > 0 { Severity::Crit } else { Severity::Ok };
        chips.push(Chip { id: "airflow", view: 5, label: "Airflow", text: parts.join(" · "), sev: sev(severity) });
    } else if !airflow.reachable && !airflow.is_placeholder() {
        chips.push(Chip { id: "airflow", view: 5, label: "Airflow", text: "unreachable".into(), sev: "warn" });
    }

    // Jira: in progress, in review, and what is overdue or due soon — view 6's flow line.
    let board = &app.jira;
    if board.reachable {
        let today = crate::jira::today(now, app.time.offset_s(now));
        let columns = board.columns();
        let open: Vec<&crate::jira::Ticket> = columns.iter().filter(|c| !c.finished).flat_map(|c| c.tickets.iter().copied()).collect();
        let judged: Vec<Severity> = open.iter().filter_map(|t| t.due_day()).map(|d| crate::jira::due_severity(d, today, false)).collect();
        let overdue = judged.iter().filter(|s| **s == Severity::Crit).count();
        let soon = judged.iter().filter(|s| **s == Severity::Warn).count();
        let mut text = format!("{} open", open.len());
        if overdue > 0 {
            text.push_str(&format!(" · {overdue} overdue"));
        } else if soon > 0 {
            text.push_str(&format!(" · {soon} due soon"));
        }
        let severity = if overdue > 0 { Severity::Crit } else if soon > 0 { Severity::Warn } else { Severity::Ok };
        chips.push(Chip { id: "jira", view: 6, label: "Jira", text, sev: sev(severity) });
        if board.worklogs_read {
            let month = crate::jira::Month::of(now, app.time.offset_s(now));
            let days = month.per_day(&board.worklogs);
            let today = days.get(month.today as usize - 1).copied().unwrap_or(0);
            let total: u64 = days.iter().sum();
            let text = format!("today {} · month {}", crate::jira::hours(today), crate::jira::hours(total));
            chips.push(Chip { id: "logged", view: 6, label: "Logged", text, sev: "none" });
        }
    } else if !board.is_placeholder() {
        chips.push(Chip { id: "jira", view: 6, label: "Jira", text: "unreachable".into(), sev: "warn" });
    }

    // This machine: its CPU and the pressure on its memory — the band's LOCAL line. Before the
    // hours logged, which give way first when the bar is narrow.
    let local = &app.local;
    if local.shown() {
        let sample = local.latest.as_ref();
        let mut parts = vec![format!("cpu {}", local.cpu.map_or("—".into(), |c| crate::fmt::pct0(c.busy_pct)))];
        if let Some(pct) = sample.and_then(|s| s.memory).and_then(|m| m.used_pct()) {
            parts.push(format!("mem {}", crate::fmt::pct0(pct)));
        }
        if let Some(p) = sample.and_then(|s| s.pressure).filter(|p| p.severity() != Severity::Ok) {
            parts.push(format!("pressure {}", p.word()));
        }
        let chip = Chip { id: "local", view: 0, label: "Mac", text: parts.join(" · "), sev: sev(local.severity()) };
        let at = chips.iter().position(|c| c.id == "logged").unwrap_or(chips.len());
        chips.insert(at, chip);
    }

    // What Claude Code and OpenCode used today, and the plan's window — view 0's AI panel.
    if app.usage.read {
        let summary = app.usage.summary(now, app.time.offset_s(now));
        let used: u64 = summary.today.values().map(|t| t.tokens).sum();
        let mut text = format!("today {} tokens", crate::usage::tokens(used));
        if let Some(w) = summary.window {
            text.push_str(&format!(" · 5h {} left", crate::fmt::dur((w.end - now).max(0) as f64)));
        }
        let at = chips.iter().position(|c| c.id == "logged").unwrap_or(chips.len());
        chips.insert(at, Chip { id: "ai", view: 0, label: "AI", text, sev: "none" });
    }

    let status = match app.snapshot() {
        Some(_) => match crate::insight::overall(&app.insights()) {
            Severity::Crit => "crit",
            Severity::Warn => "warn",
            _ => "ok",
        },
        None => "none",
    };
    let next = app
        .prayers
        .schedule
        .as_ref()
        .and_then(|s| s.next_prayer(now))
        .map(|m| format!("{} {}", m.name(), app.time.local_hm(m.at)))
        .unwrap_or_default();
    let clock = app.time.hms(now);
    State {
        view: match app.view {
            View::Claude => "sessions",
            View::Nodes => "nodes",
            View::Queue => "queue",
            View::Map => "map",
            View::Tape => "tape",
            View::Airflow => "airflow",
            View::Jira => "jira",
            View::Local => "local",
        },
        status,
        chips,
        sessions: app.claude.list.len(),
        calling: app.claude.calling() && app.view != View::Claude,
        paused: app.paused,
        stale: app.is_stale(),
        clock: clock.get(..5).unwrap_or(&clock).to_string(),
        zone: app.time.label(now),
        next,
        filter: app.tree.filter.trim().to_string(),
    }
}

/// The sequence the app reads: `ESC ] 7700 ; {json} BEL`. JSON escapes every control character,
/// so nothing in it can end the sequence early.
pub fn sequence(state: &State) -> String {
    format!("\u{1b}]{OSC};{}\u{7}", serde_json::to_string(state).unwrap_or_else(|_| "{}".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_says_what_the_screen_says() {
        let mut fake = crate::fake::FakeSource::new();
        let mut app = App::new();
        app.update(crate::app::Event::Snapshot(Box::new(fake.snapshot())));
        app.update(crate::app::Event::Queue(Box::new(fake.queue())));
        let now = app.now();
        app.update(crate::app::Event::Airflow(Box::new(crate::fake::airflow(now))));
        app.update(crate::app::Event::Jira(Box::new(crate::fake::jira(now))));
        let state = state(&app);
        assert_eq!(state.view, "nodes");
        let ids: Vec<&str> = state.chips.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["fleet", "redash", "airflow", "jira", "logged"]);
        app.update(crate::app::Event::Local(Box::new(crate::fake::local(1, 100.0))));
        app.update(crate::app::Event::Local(Box::new(crate::fake::local(2, 102.0))));
        let state = super::state(&app);
        let ids: Vec<&str> = state.chips.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["fleet", "redash", "airflow", "jira", "local", "logged"]);
        let local = &state.chips[4];
        assert_eq!((local.id, local.view, local.sev), ("local", 0, "ok"));
        assert!(local.text.starts_with("cpu ") && local.text.contains(" · mem 83%"), "{local:?}");
        let airflow = state.chips.iter().find(|c| c.id == "airflow").unwrap();
        assert!(airflow.text.contains("3 failed") && airflow.text.contains("1 stuck") && airflow.sev == "crit", "{airflow:?}");
        assert_eq!(airflow.view, 5);
        let jira = state.chips.iter().find(|c| c.id == "jira").unwrap();
        assert!(jira.text.contains("1 overdue") && jira.sev == "crit", "{jira:?}");

        let seq = sequence(&state);
        assert!(seq.starts_with("\u{1b}]7700;{") && seq.ends_with('\u{7}'));
        assert!(!seq[1..seq.len() - 1].contains(['\u{7}', '\u{1b}']), "nothing ends it early");
    }
}
