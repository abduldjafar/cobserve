//! The day line, under the masthead on every view: the day at the place prayer times are for,
//! from Subuh to Isya — each time a stop on the line, the present a point moving between them,
//! the way behind drawn solid and the way ahead dotted, the next stop saying how far off it is.
//!
//! ```text
//! ┄ Subuh 04:21 ━━━━━━━ Terbit 05:33 ━━━━━━━ Dzuhur 11:45 ━━━━●┄┄┄┄ Ashar 14:48 · in 2h52m ┄┄┄┄ Maghrib 17:50 ┄┄┄┄ Isya 18:59 ┄  Jakarta
//! ```
//!
//! The stops are spaced evenly, not by the clock: Maghrib to Isya is an hour, Terbit to Dzuhur
//! six, and both are a stretch of the day as it is lived. Within a stretch the point moves with
//! the clock.
//!
//! When a prayer is near, its reminder takes the line — `◷ Maghrib in 9:48 · 17:50 WIB` on the
//! colour of a warning, then `Time for Maghrib` on the colour of good news — until its time is a
//! few minutes past, or it is waved away (`d`, `ctrl+\ d` in a session, or a click on ✕).

use super::widgets::Cells;
use crate::app::{App, Hit, View};
use crate::fmt;
use crate::prayer::{Alert, Prayer, PlaceFrom, Schedule};
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 || area.width < 20 {
        return;
    }
    let width = area.width as usize;
    let line = match (app.prayer_alert(), app.prayers.schedule.as_ref()) {
        (Some(alert), _) => return banner(frame, app, theme, area, alert),
        (None, Some(schedule)) => rail(app, theme, schedule, width),
        (None, None) => {
            let mut cells = Cells::new();
            if app.prayers.place.is_none() {
                cells.push("┄┄ ", theme.faint());
                cells.push("prayer times: ", theme.muted());
                cells.push("PRAYER_CITY", theme.text2());
                cells.push(" (Bandung, Medan, Makassar…) or ", theme.muted());
                cells.push("PRAYER_AT", theme.text2());
                cells.push("=lat,lon says where", theme.muted());
            }
            cells
        }
    };
    frame.render_widget(Paragraph::new(line.line(width, Style::default())), area);
}

/// A stop on the line.
struct Stop {
    name: &'static str,
    at: i64,
}

/// How much of each stop is said, from the most to the least.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Say {
    /// `Subuh 04:21` everywhere, the place at the end.
    All,
    /// The same without the place.
    NoPlace,
    /// Passed stops by name only.
    PassedNamesOnly,
    /// Only the next stop with its time.
    NextOnly,
}

fn rail(app: &App, theme: &Theme, schedule: &Schedule, width: usize) -> Cells {
    let now = app.now();
    let today = schedule.today();
    let friday = today.is_friday();
    let stops: Vec<Stop> = Prayer::ALL.into_iter().filter_map(|p| Some(Stop { name: p.name(friday), at: today.at(p)? })).collect();
    let mut cells = Cells::new();
    if stops.is_empty() {
        cells.push("no prayer times at this latitude today", theme.muted());
        return cells;
    }
    let next = stops.iter().position(|s| s.at > now);
    // The night either side of the day: from last night's Isya, and to tomorrow's Subuh.
    let night_before = schedule.days[0].at(Prayer::Isya).unwrap_or(stops[0].at - 9 * 3600);
    let night_after = schedule.days[2].at(Prayer::Subuh).unwrap_or(stops[stops.len() - 1].at + 9 * 3600);
    let place = app.prayers.place.as_ref().map(|p| p.name.clone()).unwrap_or_default();
    let tz_place = app.prayers.place.as_ref().is_some_and(|p| p.from == PlaceFrom::TimeZone);

    let label = |i: usize, say: Say| -> Cells {
        let stop = &stops[i];
        let mut c = Cells::new();
        let passed = stop.at <= now;
        let is_next = Some(i) == next;
        let time = app.time.local_hm(stop.at);
        match (passed, is_next) {
            (_, true) => {
                c.push(stop.name, theme.strong());
                c.push(format!(" {time}"), theme.strong());
                c.push(format!(" · in {}", fmt::countdown(stop.at - now)), theme.accent().add_modifier(Modifier::BOLD));
            }
            (true, _) => {
                c.push(stop.name, theme.muted());
                if say == Say::All || say == Say::NoPlace {
                    c.push(format!(" {time}"), theme.faint());
                }
            }
            (false, _) => {
                c.push(stop.name, theme.text2());
                if say != Say::NextOnly {
                    c.push(format!(" {time}"), theme.muted());
                }
            }
        }
        c
    };

    for say in [Say::All, Say::NoPlace, Say::PassedNamesOnly, Say::NextOnly] {
        let labels: Vec<Cells> = (0..stops.len()).map(|i| label(i, say)).collect();
        let place_cells = (say == Say::All && !place.is_empty()).then(|| {
            let mut c = Cells::new();
            c.push(format!("  {place}"), if tz_place { theme.faint() } else { theme.muted() });
            c
        });
        let night = 2usize;
        let fixed: usize = labels.iter().map(|l| l.width() + 2).sum::<usize>() + 2 * night + place_cells.as_ref().map_or(0, Cells::width);
        let gaps = stops.len().saturating_sub(1);
        let free = width.saturating_sub(fixed);
        if gaps > 0 && free / gaps < 3 && say != Say::NextOnly {
            continue;
        }
        let each = free.checked_div(gaps).unwrap_or(0);
        let mut extra = free.checked_rem(gaps).unwrap_or(0);
        let mut line = Cells::new();
        // Each stretch: from where it starts to where it ends, `cells` wide.
        let stretch = |line: &mut Cells, from: i64, to: i64, cells: usize| segment(line, from, to, now, cells, theme);
        stretch(&mut line, night_before, stops[0].at, night);
        for (i, label) in labels.into_iter().enumerate() {
            line.push(" ", Style::default());
            line.spans(label.into_spans());
            line.push(" ", Style::default());
            if i + 1 < stops.len() {
                let cells = each + usize::from(extra > 0);
                extra = extra.saturating_sub(1);
                stretch(&mut line, stops[i].at, stops[i + 1].at, cells);
            }
        }
        stretch(&mut line, stops[stops.len() - 1].at, night_after, night);
        if let Some(place) = place_cells {
            line.spans(place.into_spans());
        }
        return line;
    }
    cells
}

/// A stretch of the line between two times: solid where it is behind, dotted ahead, and the
/// present a point on it.
fn segment(line: &mut Cells, from: i64, to: i64, now: i64, cells: usize, theme: &Theme) {
    if cells == 0 {
        return;
    }
    if now >= to {
        line.push("━".repeat(cells), theme.border());
    } else if now < from {
        line.push("┄".repeat(cells), theme.faint());
    } else {
        let gone = ((now - from) as f64 / (to - from).max(1) as f64 * cells as f64) as usize;
        let gone = gone.min(cells - 1);
        line.push("━".repeat(gone), theme.accent());
        line.push("●", theme.accent().add_modifier(Modifier::BOLD));
        line.push("┄".repeat(cells - gone - 1), theme.faint());
    }
}

/// A prayer near or come, on the line's place: what it is, when, and the way to wave it away.
fn banner(frame: &mut Frame, app: &App, theme: &Theme, area: Rect, alert: Alert) {
    let width = area.width as usize;
    let moment = alert.moment();
    let now = app.now();
    let (sev, mark, what) = match alert {
        Alert::Soon { left, .. } => (Severity::Warn, "◷", format!("{} in {}", moment.name(), fmt::countdown(left))),
        Alert::Now { .. } => (Severity::Ok, "◆", format!("Time for {}", moment.name())),
    };
    let tint = theme.tint(sev);
    let place = app.prayers.place.as_ref().map(|p| p.name.clone()).unwrap_or_default();
    let mut cells = Cells::new();
    cells.push(format!(" {mark}  "), tint.add_modifier(Modifier::BOLD));
    cells.push(what, tint.add_modifier(Modifier::BOLD));
    cells.push(format!("  ·  {} {}", app.time.local_hm(moment.at), app.time.local_label(now)), tint);
    if !place.is_empty() {
        cells.push(format!("  ·  {place}"), tint);
    }
    let key = if app.view == View::Claude { "ctrl+\\ d" } else { "d" };
    let dismiss = format!(" {key}  ✕ dismiss ");
    let at = width.saturating_sub(fmt::width(&dismiss));
    cells.pad_to(at);
    cells.push(dismiss.clone(), tint.add_modifier(Modifier::BOLD));
    app.viewport.hits.borrow_mut().push((Rect::new(area.x + at as u16, area.y, fmt::width(&dismiss) as u16, 1), Hit::Dismiss));
    frame.render_widget(Paragraph::new(cells.line(width, tint)), area);
}
