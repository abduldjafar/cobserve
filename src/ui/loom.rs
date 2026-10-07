//! View 5 on a wide terminal: the day woven. Each DAG a thread across the last 24 hours — the
//! threads are the view — and above them, told in a few words each, the runs that need a look or
//! are under way: knots in the cloth.
//!
//! ```text
//!  now  2 running · 1 stuck · 1 waiting
//! ▌✖ test_clickhouse_connection      stuck 262d     ▸ statement_daily_agg_reload   ━━━━━╸━━ 4/7
//! ▌  manual · Jan 18 · ↻ ping_clickhouse to retry     manual · 12:31 · 2h13m · ▸ reload_partitions
//!
//!  failed today  3
//!  ✖ replication_app_consistency_check            ✖ kyc_onboarding_tables
//!    at check_consistency · 13:08, 1h ago · 8m06s   at build_cohorts · 12:40, 2h ago · 10m48s
//!
//!  the day  13 DAGs
//!                              15    18    21    00    03    06    09    12      now
//!  clickhouse_replication_check ▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪▪   every 15m · in 12m
//!  cbk_accounts_report          ··▪···▪···▪···▪···✖···▪···▪···▪···▪···▪···    ✖1 · :15 every 2h
//! ```
//!
//! The same rows as the narrow view, in the same order (`Sections::keys`): what the cursor walks
//! and a click lands on. A knot's colour is its severity; a thread's name is red when its day
//! has a failure. Everything else is in the drawer and a page away (`⏎`).

use super::airflow::{day_line, short, timeline_spans, title_line};
use super::widgets::{fit, thin_bar, Cells};
use crate::airflow::{self, Activity, Axis, RunState, Run};
use crate::app::{scroll_into_view, App, Hit};
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The narrowest terminal that gets the loom; below it, the list (`airflow.rs`).
pub const FROM_WIDTH: usize = 140;
const GAP: usize = 4;
/// The finest a thread gets: a cell a quarter of an hour.
const SLOTS_MAX: usize = 96;

/// What is drawn, and where each row landed: `(line, column, width, height)`, by row.
struct Out {
    lines: Vec<Line<'static>>,
    spots: Vec<(usize, usize, usize, usize)>,
    selected_line: Option<(usize, usize)>,
}

impl Out {
    fn label(&mut self, title: &str, detail: Vec<Span<'static>>, theme: &Theme) {
        // One blank line above, whatever came before.
        if self.lines.last().is_some_and(|l| l.width() > 0) {
            self.lines.push(Line::from(""));
        }
        let mut spans = vec![Span::raw(" "), Span::styled(title.to_string(), theme.section()), Span::raw("  ")];
        spans.extend(detail);
        self.lines.push(Line::from(spans));
    }
}

/// One piece of a line: `cells` in exactly `width`, on the selection's band when chosen.
fn segment(cells: Cells, width: usize, band: Option<Style>) -> Vec<Span<'static>> {
    let spans = fit(cells.into_spans(), width, true);
    match band {
        Some(band) => spans.into_iter().map(|s| Span::styled(s.content, band.patch(s.style))).collect(),
        None => spans,
    }
}

/// A knot: a run told in two lines — its mark, its DAG and what is wrong or how far it is; then
/// which run it is and what its tasks are doing.
fn knot(activity: &Activity, run: &Run, app: &App, now: i64, selected: bool, theme: &Theme, width: usize) -> [Cells; 2] {
    let sev = run.severity(now);
    let bold = |s: Severity| theme.sev(s).add_modifier(Modifier::BOLD);
    let (mark, mark_style) = match (run.state, sev) {
        (RunState::Failed, _) | (_, Severity::Crit) => ("✖", bold(Severity::Crit)),
        (_, Severity::Warn) => ("▲", bold(Severity::Warn)),
        (RunState::Queued, _) => ("◌", theme.muted()),
        _ => ("▸", theme.accent().add_modifier(Modifier::BOLD)),
    };
    let bar = |cells: &mut Cells| {
        cells.push(if selected { "▌" } else { " " }, theme.accent());
    };
    let mut first = Cells::new();
    bar(&mut first);
    first.push(format!("{mark} "), mark_style);
    first.push(run.dag.clone(), if run.state == RunState::Failed { theme.sev(Severity::Crit) } else { theme.strong() });

    // On the right of the first line: how it stands.
    let mut right = Cells::new();
    let progress = activity.progress_of(run);
    match run.state {
        _ if run.is_stuck(now) => {
            let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_default();
            right.push(format!("stuck {took}"), bold(Severity::Crit));
        }
        RunState::Running => {
            if let Some(p) = progress.filter(|p| p.total > 0) {
                let share = f64::from(p.done) / f64::from(p.total) * 100.0;
                right.spans(thin_bar(Some(share), 8, theme.bar_fill(Severity::None), theme));
                right.push(format!(" {}/{}", p.done, p.total), theme.text2());
            }
        }
        RunState::Queued => {
            let waited = run.waited(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
            right.push(format!("waiting {waited}"), if sev.is_problem() { bold(sev) } else { theme.muted() });
        }
        _ => {}
    }
    if first.width() + 2 + right.width() <= width {
        first.pad_to(width - right.width() - 1);
        first.spans(right.into_spans());
    }

    // The second line: which run, and what its tasks are doing or where it failed.
    let mut second = Cells::new();
    bar(&mut second);
    second.push("   ", Style::default());
    let mut pieces: Vec<(String, Style)> = Vec::new();
    if run.state == RunState::Failed {
        match progress {
            Some(p) if !p.failed.is_empty() => {
                let more = if p.failed.len() > 1 { format!(" +{}", p.failed.len() - 1) } else { String::new() };
                pieces.push((format!("at {}{more}", p.failed[0]), theme.sev(Severity::Crit)));
            }
            Some(p) => pieces.push((p.without_a_failure(), theme.muted())),
            None => pieces.push(("its tasks are being read…".into(), theme.faint())),
        }
        if let Some(end) = run.end {
            pieces.push((format!("{}, {}", app.time.format(end, "%H:%M"), fmt::ago(now - end)), theme.text2()));
        }
        if let Some(took) = run.took(now) {
            pieces.push((fmt::dur(took as f64), theme.muted()));
        }
    } else {
        pieces.push((airflow::run_label(run, &app.time, now), theme.muted()));
        if run.state == RunState::Running
            && !run.is_stuck(now)
            && let Some(took) = run.took(now)
        {
            pieces.push((fmt::dur(took as f64), theme.text2()));
        }
        let tasks = activity.tasks_of(run);
        if let Some(task) = tasks.iter().find(|t| t.retrying()) {
            pieces.push((format!("↻ {} to retry", task.id), theme.sev(Severity::Warn)));
        }
        let running: Vec<&str> = tasks.iter().filter(|t| !t.retrying()).map(|t| t.id.as_str()).collect();
        if let Some(first) = running.first() {
            let more = if running.len() > 1 { format!(" +{}", running.len() - 1) } else { String::new() };
            pieces.push((format!("▸ {first}{more}"), theme.text2()));
        }
    }
    for (i, (text, style)) in pieces.into_iter().enumerate() {
        if i > 0 {
            second.push(" · ", theme.faint());
        }
        second.push(text, style);
    }
    [first, second]
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let activity = &app.airflow;
    let width = area.width as usize;
    let height = area.height as usize;
    let now = app.now();
    let offset_s = app.time.offset_s(now);
    let sections = activity.sections(now);
    let selected = app.airflow_selection();
    let keys = sections.keys();
    let mut out = Out { lines: vec![title_line(app, now, theme, width), day_line(activity, now, theme, width)], spots: Vec::new(), selected_line: None };
    let mut index = 0usize;

    // The knots, side by side: two to a row, three on a very wide terminal.
    let per_row = if width >= 210 { 3 } else { 2 };
    let knot_w = width.saturating_sub(GAP * (per_row - 1)) / per_row;
    let knots = |out: &mut Out, runs: &[&Run], index: &mut usize| {
        for chunk in runs.chunks(per_row) {
            let top = out.lines.len();
            let mut rows: [Vec<Span<'static>>; 2] = [Vec::new(), Vec::new()];
            for (i, run) in chunk.iter().enumerate() {
                let chosen = keys.get(*index).is_some_and(|k| Some(k) == selected);
                if chosen {
                    out.selected_line = Some((top, top + 1));
                }
                out.spots.push((top, i * (knot_w + GAP), knot_w, 2));
                *index += 1;
                let band = chosen.then(|| theme.selected());
                for (line, cells) in knot(activity, run, app, now, chosen, theme, knot_w).into_iter().enumerate() {
                    if i > 0 {
                        rows[line].push(Span::raw(" ".repeat(GAP)));
                    }
                    rows[line].extend(segment(cells, knot_w, band));
                }
            }
            let [a, b] = rows;
            out.lines.push(Line::from(a));
            out.lines.push(Line::from(b));
            out.lines.push(Line::from(""));
        }
    };

    // Now: what runs and what waits.
    let stuck = sections.running.iter().filter(|r| r.is_stuck(now)).count();
    let mut detail = vec![Span::styled(format!("{} running", sections.running.len()), theme.muted())];
    if stuck > 0 {
        detail.push(Span::styled(" · ", theme.faint()));
        detail.push(Span::styled(format!("{stuck} stuck"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
    }
    if !sections.queued.is_empty() {
        detail.push(Span::styled(format!(" · {} waiting", sections.queued.len()), theme.muted()));
    }
    out.label("now", detail, theme);
    if sections.running.is_empty() && sections.queued.is_empty() {
        out.lines.push(Line::from(Span::styled("   nothing runs, nothing waits", theme.muted())));
    }
    let live: Vec<&Run> = sections.running.iter().chain(&sections.queued).copied().collect();
    knots(&mut out, &live, &mut index);

    if !activity.day_read {
        out.label("the day", vec![Span::styled("reading the last 24 h of runs…", theme.muted())], theme);
    } else {
        if !sections.failed.is_empty() {
            out.label("failed today", vec![Span::styled(sections.failed.len().to_string(), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD))], theme);
            knots(&mut out, &sections.failed, &mut index);
        }

        // The day: a thread a DAG, its name before it and its rhythm after.
        let name_w = sections.dags.iter().map(|l| fmt::width(l.id)).max().unwrap_or(16).clamp(16, 34);
        let tail_w = 24;
        let room = width.saturating_sub(2 + name_w + 2 + 2 + tail_w);
        let slots = airflow::slots_for(room.min(SLOTS_MAX));
        let axis = Axis::new(now, slots.max(1), offset_s);
        out.label("the day", vec![Span::styled(fmt::plural(sections.dags.len(), "DAG", "DAGs"), theme.muted())], theme);
        // The hours over the cells they begin, and now at the end.
        let mut hours = vec![' '; slots];
        for (at, label) in axis.labels(offset_s) {
            if at + label.chars().count() <= slots {
                for (i, c) in label.chars().enumerate() {
                    hours[at + i] = c;
                }
            }
        }
        let mut head = Cells::new();
        head.gap(2 + name_w + 2);
        head.push(hours.into_iter().collect::<String>(), theme.faint());
        head.push("  now", theme.accent());
        out.lines.push(head.line(width, Style::default()));
        for line in &sections.dags {
            let chosen = keys.get(index).is_some_and(|k| Some(k) == selected);
            let at = out.lines.len();
            if chosen {
                out.selected_line = Some((at, at));
            }
            out.spots.push((at, 0, width, 1));
            index += 1;
            let sev = line.severity(now);
            let mut cells = Cells::new();
            cells.push(if chosen { "▌ " } else { "  " }, theme.accent());
            let name_style = if sev == Severity::Crit { theme.sev(Severity::Crit) } else if chosen { theme.strong() } else { theme.text() };
            cells.cell(line.id, name_w, name_style);
            cells.gap(2);
            if slots > 0 {
                cells.spans(timeline_spans(&airflow::timeline(&line.runs, &axis, now), theme));
            }
            cells.gap(4);
            let mut tail: Vec<(String, Style)> = Vec::new();
            if line.failed > 0 {
                tail.push((format!("✖{}", line.failed), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
            }
            let schedule = airflow::short_schedule(line.dag);
            if !schedule.is_empty() {
                tail.push((schedule, theme.muted()));
            }
            match line.dag {
                Some(dag) if dag.paused => tail.push(("paused".into(), theme.faint())),
                Some(dag) => {
                    if let Some(next) = dag.next.filter(|n| *n > now) {
                        tail.push((format!("in {}", short(next - now)), theme.faint()));
                    }
                }
                None => {}
            }
            for (i, (text, style)) in tail.into_iter().enumerate() {
                if i > 0 {
                    cells.push(" · ", theme.faint());
                }
                cells.push(text, style);
            }
            out.lines.push(cells.line(width, if chosen { theme.selected() } else { Style::default() }));
        }
        if sections.dags.is_empty() {
            out.lines.push(Line::from(Span::styled("   no DAG ran in the last 24 h", theme.muted())));
        }
    }

    // The title and the day's numbers stay; the rest scrolls, the chosen row whole on screen.
    let fixed = 2.min(height);
    let room = height - fixed;
    let body = out.lines.split_off(fixed.min(out.lines.len()));
    let chosen = out.selected_line.map(|(a, b)| (a - fixed, b - fixed));
    let offset = scroll_into_view(app.viewport.airflow.get(), chosen.map(|c| c.1), room, body.len());
    let offset = match chosen {
        Some((a, _)) if a < offset => a,
        _ => offset,
    };
    app.viewport.airflow.set(offset);
    let mut lines = out.lines;
    lines.extend(body.into_iter().skip(offset).take(room));
    let mut hits = app.viewport.hits.borrow_mut();
    for (row, &(line, x, w, h)) in out.spots.iter().enumerate() {
        let (top, bottom) = ((line - fixed).max(offset), (line - fixed + h).min(offset + room));
        if top < bottom {
            let y = area.y + (fixed + top - offset) as u16;
            hits.push((Rect::new(area.x + x as u16, y, w as u16, (bottom - top) as u16), Hit::AirflowRow(row)));
        }
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), area);
}
