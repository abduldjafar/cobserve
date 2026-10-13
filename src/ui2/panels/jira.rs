//! Panel 6, JIRA (`02-panels.md` *6 · JIRA*): the flow across the board's columns, what is overdue,
//! today's logged hours, then the tickets with their key, summary and due date or age. Zoomed it
//! is view 6 itself, the board (`views.rs`).

use crate::app::App;
use crate::fmt;
use crate::jira;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;

fn today(app: &App) -> i64 {
    let now = app.now();
    jira::today(now, app.time.offset_s(now))
}

/// The tickets not finished whose due day has passed, and those due within a day.
pub fn due(app: &App) -> (usize, usize) {
    let today = today(app);
    let cols = app.jira.columns();
    let open = cols.iter().filter(|c| !c.finished).flat_map(|c| c.tickets.iter());
    let mut over = 0;
    let mut soon = 0;
    for t in open {
        match t.due_day().map(|d| jira::due_severity(d, today, false)) {
            Some(Severity::Crit) => over += 1,
            Some(Severity::Warn) => soon += 1,
            _ => {}
        }
    }
    (over, soon)
}

/// How long, in its largest unit: `11d`, `8h`, `40m`.
pub fn coarse(seconds: i64) -> String {
    match seconds.max(0) {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3600 => format!("{}h", s / 3600),
        s => format!("{}m", s / 60),
    }
}

pub fn title(app: &App) -> (Severity, String) {
    let b = &app.jira;
    if !b.reachable {
        let sev = if b.is_placeholder() { Severity::None } else { Severity::Warn };
        return (sev, b.error.clone().unwrap_or_default());
    }
    let (over, soon) = due(app);
    let sev = if over > 0 {
        Severity::Crit
    } else if soon > 0 {
        Severity::Warn
    } else {
        Severity::None
    };
    (sev, "mine".to_string())
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let i = super::frame(p, app, theme, PanelId::Jira, r, focused);
    if i.width < 4 || i.height == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i, focused));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, focused: bool) {
    let b = &app.jira;
    let w = i.width as usize;
    if !b.reachable {
        let text = match b.error.as_deref() {
            Some(jira::NOT_CONFIGURED) => "not configured · JIRA_URL and JIRA_TOKEN".to_string(),
            Some(jira::NOT_READ) | None => "reading Jira…".to_string(),
            Some(why) => format!("could not be read · {why}"),
        };
        p.put(i.x, i.y, &tr(&text, w), if b.is_placeholder() { theme.muted() } else { theme.sev(Severity::Warn) });
        return;
    }
    let mut y = i.y;
    let cols = b.columns();
    let flow: Vec<String> = cols.iter().map(|c| c.tickets.len().to_string()).collect();
    let mut flow = flow.join(" › ");
    if cols.last().is_some_and(|c| c.finished) {
        flow.push_str(" done");
    }
    let (over, _) = due(app);
    let right = if over > 0 { format!("✖ {over} overdue") } else { String::new() };
    p.put(i.x, y, &tr(&flow, w.saturating_sub(fmt::width(&right) + 1)), theme.text());
    if !right.is_empty() {
        p.putr(i.x + i.width, y, &right, theme.sev(Severity::Crit));
    }
    y += 1;
    if i.height >= 5 && b.worklogs_read {
        let (month, _) = app.time_by_ticket();
        let days = month.per_day(&b.worklogs);
        let today = days.get(month.today.saturating_sub(1) as usize).copied().unwrap_or(0);
        let total: u64 = days.iter().sum();
        let worked = month.working_days_so_far().max(1);
        let text = format!("today {} · {} a working day", jira::hours(today), jira::hours(total / u64::from(worked)));
        p.put(i.x, y, &tr(&text, w), theme.muted());
        y += 1;
    }
    let today = today(app);
    let now = app.now();
    let selected = app.jira_selection().map(str::to_string);
    let rows = b.rows();
    let room = (i.y + i.height).saturating_sub(y) as usize;
    let at = selected.as_ref().and_then(|k| rows.iter().position(|t| &t.key == k)).unwrap_or(0);
    let first = if focused { at.saturating_sub(room.saturating_sub(1)) } else { 0 };
    let finished: Vec<&str> = cols.iter().filter(|c| c.finished).flat_map(|c| c.tickets.iter().map(|t| t.key.as_str())).collect();
    for t in rows.iter().skip(first).take(room) {
        if focused && selected.as_deref() == Some(t.key.as_str()) {
            p.band(Rect::new(i.x - 1, y, i.width + 2, 1), theme.selected());
        }
        let done = finished.contains(&t.key.as_str());
        let (right, sev) = match t.due_day() {
            Some(d) if !done => (jira::due_label(d, today), jira::due_severity(d, today, false)),
            _ => (t.in_status(now).map_or(String::new(), coarse), Severity::None),
        };
        p.put(i.x, y, &t.key, theme.accent());
        let kw = fmt::width(&t.key) as u16 + 1;
        if w >= 30 {
            let (_, summary) = jira::split_tag(&t.summary);
            p.put(i.x + kw, y, &tr(summary, w.saturating_sub(kw as usize + 2 + fmt::width(&right).max(3))), theme.text());
        }
        p.putr(i.x + i.width, y, &tr(&right, w.saturating_sub(kw as usize)), if sev.is_problem() { theme.sev(sev) } else { theme.muted() });
        y += 1;
    }
}
