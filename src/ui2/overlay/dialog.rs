//! The cancel dialog (`x` on a queue row, `screens/120x36-cancel-dialog.txt`): what will be
//! cancelled, what it means for its ClickHouse query, that it cannot be undone; `y` sends it,
//! anything else keeps the job. The only write cobserve makes, asked before it is made.

use crate::app::App;
use crate::fmt;
use crate::model::{Job, JobState};
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::{tr, Painter};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// The stop dialog (`x` on a browser's tab or extension process): what is stopped, what its tabs
/// show after, that it cannot be undone; `y` stops it, anything else keeps it.
pub fn draw_stop(p: &mut Painter<'_>, app: &App, theme: &Theme, helper: &crate::local::Helper) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    if sw < 30 || sh < 10 {
        return;
    }
    let w = (sw - 4).min(86);
    let h = 9;
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    let program = app.drill.as_ref().map_or("Google Chrome", |d| d.program.as_str());
    let short = crate::browser::short(program);
    let word = crate::browser::helper_word(helper.part);
    let title = format!("Stop {short} {word} process {} · {}?", helper.pid, fmt::bytes(helper.rss));
    frame::overlay(p, theme, Rect::new(x, y, w, h), &title, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD), "");
    let b = app.local.browser(program);
    let of = b.filter(|b| b.rss > 0).map_or(String::new(), |b| format!(" · {} of {short}'s {}", fmt::pct(Some(helper.rss as f64 / b.rss as f64 * 100.0)), fmt::bytes(b.rss)));
    let age = helper.started.map_or("age unknown".to_string(), |s| format!("{} old", fmt::dur(app.now() as f64 - s)));
    let what = format!("{word} process · {} RSS{of} · {:.2} cores · {age}", fmt::bytes(helper.rss), helper.cores);
    let means = if helper.part == crate::local::Part::Extension {
        format!("The extension stops until {short} starts it again — it does when it is next used.")
    } else {
        format!("Its tabs show Aw, Snap! — reload one to get it back. Which, only {short} knows.")
    };
    let surface = theme.surface();
    let lines = [(what, theme.text()), (means, theme.sev(Severity::Warn)), (format!("Its memory is freed; {short} keeps running. This cannot be undone."), theme.text())];
    for (k, (line, style)) in lines.iter().enumerate() {
        p.put(x + 3, y + 2 + k as u16, &tr(line, w as usize - 6), style.patch(surface));
    }
    p.put(x + 3, y + h - 2, "[y] stop the process     [esc] keep it", theme.accent().add_modifier(Modifier::BOLD).patch(surface));
}

pub fn draw(p: &mut Painter<'_>, _app: &App, theme: &Theme, job: &Job) {
    let screen = p.clip();
    let (sw, sh) = (screen.width, screen.height);
    if sw < 30 || sh < 10 {
        return;
    }
    let w = (sw - 4).min(86);
    let h = 9;
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    let title = format!("Cancel Redash job {} · {}?", job.id, job.label());
    frame::overlay(p, theme, Rect::new(x, y, w, h), &title, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD), "");
    let state = match job.state {
        JobState::Started => format!("running {}", fmt::dur(job.age_s as f64)),
        JobState::Queued => format!("waiting {}", fmt::dur(job.age_s as f64)),
        JobState::Stale(_) => "in the started list, no worker holds it".to_string(),
    };
    let source = job.data_source.clone().unwrap_or_else(|| "—".into());
    let what = format!("{} · {source} · {state}", job.query_label());
    let means = match (job.state, job.clickhouse_target()) {
        (JobState::Started, Some((node, _))) => format!("Its query on {node} keeps running: Redash's cancel does not reach it, and cobserve never sends KILL QUERY."),
        (JobState::Started, None) => "It is stopped on its worker; the next job starts.".to_string(),
        (JobState::Queued, _) => "It leaves the queue and never runs; the person can run it again from Redash.".to_string(),
        (JobState::Stale(_), _) => "It leaves the started list; no worker holds it anyway.".to_string(),
    };
    let surface = theme.surface();
    let lines = [(what, theme.text()), (means, theme.sev(Severity::Warn)), ("This cannot be undone. The job can be run again from Redash.".to_string(), theme.text())];
    for (k, (line, style)) in lines.iter().enumerate() {
        p.put(x + 3, y + 2 + k as u16, &tr(line, w as usize - 6), style.patch(surface));
    }
    p.put(x + 3, y + h - 2, "[y] cancel the job     [esc] keep it", theme.accent().add_modifier(Modifier::BOLD).patch(surface));
}
