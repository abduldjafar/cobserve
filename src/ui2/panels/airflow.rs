//! Panel 5, AIRFLOW (`02-panels.md` *5 · AIRFLOW*): the day in numbers, a 24-hour strip of every
//! DAG's runs with the clock's hours under it, then today's view 5 rows in order — running (the
//! stuck ones red), queued, failed, then the DAGs. Zoomed it is view 5 itself (`views.rs`).

use crate::airflow::{RowKey, RunState};
use crate::app::App;
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::paint::{tr, Painter};
use crate::ui2::PanelId;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// The panel's severity: the worst run that runs or waits now; a failure of the day is amber —
/// it happened, it is not happening.
pub fn title(app: &App) -> (Severity, String) {
    let a = &app.airflow;
    if !a.reachable {
        let sev = if a.is_placeholder() { Severity::None } else { Severity::Warn };
        return (sev, a.error.clone().unwrap_or_default());
    }
    let now = app.now();
    let s = a.sections(now);
    let live = s.running.iter().chain(&s.queued).map(|r| r.severity(now)).max().unwrap_or(Severity::None);
    let failed = if s.failed.is_empty() { Severity::None } else { Severity::Warn };
    (live.max(failed), "last 24 h".to_string())
}

/// The first hour of the strip, by the clock on screen: the current hour is its last slot.
fn first_hour(app: &App) -> i64 {
    let now = app.now();
    let offset = app.time.offset_s(now);
    (now + offset).div_euclid(3600) * 3600 - offset - 23 * 3600
}

/// One glyph an hour for the last 24 clock hours, oldest first: `✖` a failure in it, `▸` a run
/// going on, `▪` runs that went well, `·` nothing.
fn hours(app: &App) -> Vec<char> {
    let start = first_hour(app);
    let mut out = vec!['·'; 24];
    let mut weight = [0u8; 24];
    for r in &app.airflow.runs {
        let Some(at) = r.at() else { continue };
        if at < start {
            continue;
        }
        let slot = (((at - start) / 3600) as usize).min(23);
        let (w, c) = match r.state {
            RunState::Failed => (3, '✖'),
            RunState::Running | RunState::Queued => (2, '▸'),
            _ => (1, '▪'),
        };
        if w > weight[slot] {
            weight[slot] = w;
            out[slot] = c;
        }
    }
    out
}

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, focused: bool) {
    let i = super::frame(p, app, theme, PanelId::Airflow, r, focused);
    if i.width < 4 || i.height == 0 {
        return;
    }
    p.within(i, |p| body(p, app, theme, i, focused));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, focused: bool) {
    let a = &app.airflow;
    let w = i.width as usize;
    if !a.reachable {
        let text = match a.error.as_deref() {
            Some(crate::airflow::NOT_CONFIGURED) => "not configured · AIRFLOW_URL, AIRFLOW_USER, AIRFLOW_PASSWORD".to_string(),
            Some(crate::airflow::NOT_READ) | None => "reading Airflow…".to_string(),
            Some(why) => format!("could not be read · {why}"),
        };
        p.put(i.x, i.y, &tr(&text, w), if a.is_placeholder() { theme.muted() } else { theme.sev(Severity::Warn) });
        return;
    }
    let now = app.now();
    let c = a.counts(now);
    let mut y = i.y;
    if !a.day_read {
        p.put(i.x, y, "the day: reading…", theme.muted());
    } else if w >= 34 {
        let mut x = p.put(i.x, y, &format!("{} runs  ", c.runs), theme.text());
        x = p.put(x, y, &format!("✔ {}", c.ok), theme.sev(Severity::Ok));
        x = p.put(x, y, "  ", theme.text());
        x = p.put(x, y, &format!("✖ {}", c.failed), if c.failed > 0 { theme.sev(Severity::Crit).add_modifier(Modifier::BOLD) } else { theme.muted() });
        // The retries go first when the line is too long for the panel.
        let rest = format!("  ▸ {}  ◌ {}  ↻ {}", c.running, c.queued, c.retrying);
        let short = format!("  ▸ {}  ◌ {}", c.running, c.queued);
        let fits = |t: &str| x + fmt::width(t) as u16 <= i.x + i.width;
        p.put(x, y, if fits(&rest) { &rest } else { &short }, theme.text());
    } else {
        p.put(i.x, y, &tr(&format!("{} runs · ✔ {} · ✖ {}", c.runs, c.ok, c.failed), w), theme.text());
    }
    y += 1;
    // The strip: as many cells an hour as fit, at most two.
    let per = (w.saturating_sub(6) / 24).clamp(1, 2);
    let hours = hours(app);
    if y < i.y + i.height {
        let mut x = i.x;
        for h in &hours {
            let style = match h {
                '✖' => theme.sev(Severity::Crit),
                '▸' => theme.sev(Severity::Ok),
                _ => theme.muted(),
            };
            let cell: String = std::iter::repeat_n(*h, per).collect();
            x = p.put(x, y, &cell, style);
        }
        if c.running > 0 && x + 3 <= i.x + i.width {
            p.put(x + 1, y, "▸━", theme.sev(Severity::Ok));
        }
        y += 1;
    }
    if y < i.y + i.height {
        // The clock's hours under the strip, every six, and `now` past its end.
        let offset = app.time.offset_s(now);
        let start = first_hour(app);
        let mut line = vec![' '; 24 * per];
        for slot in 0..24 {
            let hour = (start + slot as i64 * 3600 + offset).div_euclid(3600).rem_euclid(24);
            if hour % 6 == 0 {
                for (k, ch) in format!("{hour:02}").chars().enumerate() {
                    if let Some(c) = line.get_mut(slot * per + k) {
                        *c = ch;
                    }
                }
            }
        }
        let text: String = line.into_iter().collect();
        let text = format!("{text} now");
        p.put(i.x, y, &tr(&text, w), theme.muted());
        y += 1;
    }
    let s = a.sections(now);
    let selected = app.airflow_selection().cloned();
    let name_w = 26.min(w.saturating_sub(14));
    let mut rows: Vec<(RowKey, char, Severity, String, String)> = Vec::new();
    for r in &s.running {
        let sev = r.severity(now);
        let glyph = if r.is_stuck(now) { '✖' } else { '▸' };
        rows.push((RowKey::Running(r.dag.clone(), r.id.clone()), glyph, if r.is_stuck(now) { Severity::Crit } else { Severity::Ok.max(sev) }, r.dag.clone(), r.took(now).map_or("—".into(), |t| fmt::dur(t as f64))));
    }
    for r in &s.queued {
        rows.push((RowKey::Queued(r.dag.clone(), r.id.clone()), '◌', Severity::Warn, r.dag.clone(), r.waited(now).map_or("—".into(), |t| fmt::dur(t as f64))));
    }
    for r in &s.failed {
        rows.push((RowKey::Failed(r.dag.clone(), r.id.clone()), '✖', Severity::Crit, r.dag.clone(), r.took(now).map_or("—".into(), |t| fmt::dur(t as f64))));
    }
    for d in &s.dags {
        let last = d.last.and_then(|r| r.at()).map_or("—".into(), |at| fmt::ago(now - at));
        rows.push((RowKey::Dag(d.id.to_string()), '▪', Severity::None, d.id.to_string(), last));
    }
    let room = (i.y + i.height).saturating_sub(y) as usize;
    let at = selected.as_ref().and_then(|k| rows.iter().position(|r| &r.0 == k)).unwrap_or(0);
    let first = if focused { at.saturating_sub(room.saturating_sub(1)) } else { 0 };
    for (k, (key, glyph, sev, name, right)) in rows.iter().enumerate().skip(first).take(room) {
        if focused && selected.as_ref() == Some(key) {
            p.band(Rect::new(i.x - 1, y, i.width + 2, 1), theme.selected());
        }
        let style = match sev {
            Severity::Crit | Severity::Warn => theme.sev(*sev),
            Severity::Ok => theme.sev(Severity::Ok),
            _ => theme.muted(),
        };
        p.put(i.x, y, &glyph.to_string(), style);
        p.put(i.x + 2, y, &tr(name, name_w), theme.text());
        p.putr(i.x + i.width, y, right, if *sev == Severity::Crit { theme.sev(Severity::Crit) } else { theme.muted() });
        let _ = k;
        y += 1;
    }
}
