//! Panel 2, REDASH QUEUE (`02-panels.md` *2 · REDASH QUEUE*): the jobs waiting in big digits,
//! running and stale beside them, the workers and the stitch to the runaways under them, and the
//! waiting jobs as a graph.

use crate::app::App;
use crate::fmt;
use crate::history::WINDOW_S;
use crate::model::JobState;
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::ui2::frame::{self, Title};
use crate::ui2::glyph::{big, graph};
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// The graph's fixed scale, in jobs waiting: a graph that rescales lies (`03-glyphs.md`), and the
/// columns turn amber at 30 and red at 60 waiting.
pub const GRAPH_MAX: f64 = 100.0;

/// Started jobs whose ClickHouse query is a runaway (§5.4), of the started jobs on ClickHouse.
pub fn runaway_holders(app: &App) -> (usize, usize) {
    let on_ch: Vec<(&str, &str)> = app.queue.jobs.iter().filter(|j| j.state == JobState::Started).filter_map(|j| j.clickhouse_target()).collect();
    let holding = on_ch.iter().filter(|(node, id)| app.query_is_runaway(node, id)).count();
    (holding, on_ch.len())
}

/// The oldest wait across the queues, when Redis can tell it.
fn oldest(app: &App) -> Option<u64> {
    app.queue.queues.iter().filter(|q| q.waiting > 0).filter_map(|q| q.oldest_wait_s).max()
}

pub fn title(app: &App) -> (Severity, String) {
    let q = &app.queue;
    if !q.reachable {
        let sev = if q.is_placeholder() { Severity::None } else { Severity::Warn };
        return (sev, q.error.clone().unwrap_or_default());
    }
    let mut sev = q
        .queues
        .iter()
        .filter(|row| row.waiting > 0)
        .map(|row| severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated()))
        .max()
        .unwrap_or(Severity::None);
    let (holding, started) = runaway_holders(app);
    if started > 0 && holding == started && q.workers_total > 0 && q.workers_busy >= q.workers_total {
        sev = Severity::Crit;
    }
    let right = match (q.total_waiting(), oldest(app)) {
        (0, _) => "nothing waits".to_string(),
        (_, Some(s)) => format!("oldest {}{}", fmt::dur(s as f64), severity::wait(s, false).mark()),
        (_, None) => "oldest —".to_string(),
    };
    (sev, right)
}

/// `●●●●●●○`: a dot a live worker, filled when busy; nothing past sixteen.
pub fn worker_dots(busy: u32, total: u32) -> String {
    if total == 0 || total > 16 {
        return String::new();
    }
    let busy = busy.min(total);
    format!("{}{}", "●".repeat(busy as usize), "○".repeat((total - busy) as usize))
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let (sev, right) = title(app);
    let i = frame::panel(p, theme, r, &Title { number: Some(2), name: PanelId::Queue.title(), sev, right: &right, focused });
    if i.height == 0 || i.width == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i, sev));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, sev: Severity) {
    let q = &app.queue;
    if !q.reachable {
        let text = match q.error.as_deref() {
            Some(crate::model::QUEUE_NOT_CONFIGURED) => "not configured · REDASH_URL and REDASH_ADMIN_API_KEY turn it on".to_string(),
            Some(crate::model::QUEUE_NOT_POLLED) => "connecting…".to_string(),
            Some(why) => format!("unreachable · {why}"),
            None => "unreachable".to_string(),
        };
        let style = if q.is_placeholder() { theme.muted() } else { theme.sev(Severity::Warn) };
        p.put(i.x, i.y, &tr(&text, i.width as usize), style);
        return;
    }
    let waiting = q.total_waiting();
    let (running, stale) = (q.total_running(), q.total_stale());
    let (holding, started) = runaway_holders(app);
    let dots = worker_dots(q.workers_busy, q.workers_total);
    let full = q.workers_total > 0 && q.workers_busy >= q.workers_total;
    let worker_style = if full { theme.sev(Severity::Warn) } else { theme.text() };
    let wait_style = if waiting > 0 && sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.strong() };
    let mut x;
    if i.height >= 4 && i.width >= 40 {
        x = big::draw(p, i.x, i.y, &waiting.to_string(), wait_style);
        p.put(x + 1, i.y, "waiting", theme.muted());
        p.put(x + 1, i.y + 1, &format!("{running} running"), theme.text());
        p.put(x + 1, i.y + 2, &format!("{stale} stale"), theme.muted());
        // The lines under the digits are as wide as under two digits, however many there are.
        x = x.max(i.x + 8);
        let room = (x + 11).saturating_sub(i.x) as usize;
        let workers = match (q.workers_total, q.workers_busy) {
            (0, _) => "no live worker".to_string(),
            (t, b) => format!("{dots} {b}/{t} workers busy"),
        };
        p.put(i.x, i.y + 3, &tr(workers.trim_start(), room), worker_style);
        if i.height >= 5 && started > 0 {
            let style = if holding > 0 { theme.sev(Severity::Crit) } else { theme.muted() };
            p.put(i.x, i.y + 4, &tr(&format!("{holding} of {started} hold a runaway"), room), style);
        }
        x += 12;
    } else {
        let oldest = oldest(app).map_or(String::new(), |s| format!(" · oldest {}", fmt::dur(s as f64)));
        p.put(i.x, i.y, &format!("{waiting} waiting{oldest}"), wait_style);
        p.put(i.x, i.y + 1, format!("{dots} {}/{} busy · {stale} stale", q.workers_busy, q.workers_total).trim_start(), worker_style);
        x = i.x + 28;
    }
    let gw = (i.x + i.width).saturating_sub(x + 1);
    let gh = i.height.saturating_sub(1).max(1);
    if gw <= 8 {
        return;
    }
    let tier = app.caps.graph;
    let per = graph::per_column(tier);
    let now = crate::history::secs(q.taken_at);
    let series = graph::stretch(app.history.queue_waiting.buckets(now, WINDOW_S, gw as usize * per));
    graph::area(p, tier, Rect::new(x, i.y, gw, gh), &series, GRAPH_MAX, &|v| {
        if v >= 60.0 {
            theme.sev(Severity::Crit)
        } else if v >= 30.0 {
            theme.sev(Severity::Warn)
        } else {
            theme.accent()
        }
    });
    if i.height >= 2 {
        p.put(x, i.y + gh, &super::fleet::window_label(), theme.muted());
        p.putr(x + gw, i.y + gh, "waiting jobs  now", theme.muted());
    }
}
