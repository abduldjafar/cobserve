//! `cobserve --plain` (`06-build-order.md` phase 5): no screen — what the panels' frames say, as
//! text, every poll. For a log, a pipe, a terminal that cannot draw the screen. Severity is the
//! glyph (`✖ ▲`), as it is without colour on screen.

use crate::app::App;
use crate::severity::Severity;
use crate::ui2::panels;
use crate::ui2::PanelId;

fn mark(sev: Severity) -> &'static str {
    match sev {
        Severity::Crit => "✖",
        Severity::Warn => "▲",
        _ => "·",
    }
}

/// One block: the clock, then a line a panel with something to say.
pub fn summary(app: &App) -> String {
    let now = app.now();
    let mut out = format!("{} {}\n", app.time.hms(now), app.time.label(now));
    if let Some(t) = app.with_view(crate::model::fleet_totals) {
        let (sev, right) = panels::fleet::title(app);
        out.push_str(&format!(
            "  {} FLEET    memory {} ({}) · cpu {} ({}) · {}\n",
            mark(sev),
            crate::fmt::pct(t.mem_pct()),
            panels::fleet::gib_of(t.mem_used, t.mem_total),
            crate::fmt::pct(t.cpu_pct()),
            panels::fleet::cores_of(&t),
            right
        ));
        let nodes = app
            .with_view(|v| {
                panels::nodes::order(app, &v.nodes)
                    .iter()
                    .map(|n| match n.mem_pct {
                        Some(m) if n.node.reachable => format!("{} {}", n.name(), crate::fmt::pct0(m)),
                        _ => format!("{} ↯ {}", n.name(), n.node.down_word()),
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            })
            .unwrap_or_default();
        let (sev, _) = panels::title(app, PanelId::Nodes);
        out.push_str(&format!("  {} NODES    {nodes}\n", mark(sev)));
    }
    let q = &app.queue;
    let (sev, right) = panels::queue::title(app);
    if q.reachable {
        out.push_str(&format!("  {} QUEUE    {} waiting · {} running · {} stale · {}/{} workers busy · {right}\n", mark(sev), q.total_waiting(), q.total_running(), q.total_stale(), q.workers_busy, q.workers_total));
    } else if !q.is_placeholder() {
        out.push_str(&format!("  {} QUEUE    {right}\n", mark(sev)));
    }
    if app.airflow.reachable && app.airflow.day_read {
        let c = app.airflow.counts(now);
        let (sev, _) = panels::airflow::title(app);
        out.push_str(&format!("  {} AIRFLOW  {} runs in 24 h · ✔ {} · ✖ {} · ▸ {} running · ◌ {} queued\n", mark(sev), c.runs, c.ok, c.failed, c.running, c.queued));
    }
    if app.jira.reachable {
        let (sev, _) = panels::jira::title(app);
        let (over, soon) = panels::jira::due(app);
        let flow: Vec<String> = app.jira.columns().iter().map(|c| format!("{} {}", c.tickets.len(), c.status)).collect();
        out.push_str(&format!("  {} JIRA     {} · {over} overdue · {soon} due soon\n", mark(sev), flow.join(" › ")));
    }
    if let Some(e) = app.tape.newest_first().next() {
        let text: String = e.parts.iter().map(|(t, _)| t.as_str()).collect();
        out.push_str(&format!("  {} TAPE     {} {text}\n", mark(e.level), app.time.local_hm(e.at as i64)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Event;

    #[test]
    fn the_summary_says_what_the_frames_say() {
        let mut fake = crate::fake::FakeSource::new();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        app.update(Event::Queue(Box::new(fake.queue())));
        let text = summary(&app);
        assert!(text.contains("FLEET    memory ") && text.contains(" GiB) · cpu "), "{text}");
        assert!(text.contains("NODES    clickhouse3 "), "{text}");
        assert!(text.contains("QUEUE    ") && text.contains(" waiting · "), "{text}");
        assert!(text.lines().count() >= 4, "{text}");
    }
}
