//! AIRFLOW zoomed (`docs/redesign/09-zooms.md` §2): *what needs a look right now?* A card for
//! every run that is running, queued or failed today, in the order the cursor walks them — the
//! state and a live duration, who and when, what its tasks are doing. Under them the day as a
//! loom: a thread per DAG that ran in the last 24 h, the hours of the clock over the cells and a
//! `┃` now line every running thread grows into. Last, the run under the cursor as a Gantt on its
//! own time axis, task bars ending in `✔ ✖ ◐`, the running one reaching the now line.

use super::kit::{self, Card};
use super::widgets::{thin_bar, Cells};
use crate::airflow::{self, Activity, Axis, Cell, DagLine, RowKey, Run, RunState, TaskRun};
use crate::app::{scroll_into_view, App, Feed, Hit};
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::frame;
use crate::ui2::paint::Painter;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::Frame;

/// A run's card: wide enough for `stuck · running 262d04h` and a task's name.
const CARD_W: u16 = 38;
const CARD_H: u16 = 5;
/// From this width, and this many rows inside, the runs are cards; below, a line each.
const CARDS_FROM_W: u16 = 100;
const CARDS_FROM_H: u16 = 24;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, i: Rect) {
    if i.height == 0 || i.width < 10 {
        return;
    }
    let mut p = Painter::new(frame.buffer_mut());
    let clip = Rect::new(i.x.saturating_sub(1), i.y, i.width + 2, i.height);
    p.within(clip, |p| body(p, app, theme, i));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let activity = &app.airflow;
    if !activity.reachable {
        let calm = ("◌", theme.muted());
        match activity.error.as_deref() {
            Some(airflow::NOT_CONFIGURED) => kit::message(
                p,
                theme,
                i,
                calm,
                "Airflow is not configured",
                &["set AIRFLOW_URL, AIRFLOW_USER and AIRFLOW_PASSWORD (the login of its web UI)", ":setup airflow · or airflow: url, user, password in the --credential file"],
            ),
            Some(airflow::NOT_READ) | None => kit::message(p, theme, i, calm, "reading Airflow…", &["the runs of the last 24 h and every DAG"]),
            Some(why) => kit::message(p, theme, i, ("✖", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)), "Airflow could not be read", &[why, "r reads again"]),
        }
        return;
    }
    let now = app.now();
    let sections = activity.sections(now);
    let keys = sections.keys();
    let selected = app.airflow_selection();
    let at = selected.and_then(|k| keys.iter().position(|x| x == k));
    let knots: Vec<&Run> = sections.running.iter().chain(&sections.queued).chain(&sections.failed).copied().collect();

    day_line(p, app, theme, i, now);
    let gantt_h: u16 = match i.height {
        30.. => 8,
        24..30 => 6,
        _ => 0,
    };
    let mut y = i.y + 1;
    now_label(p, theme, &sections, now, i, y);
    y += 1;

    // The knots: cards when there is room, a line each otherwise.
    let cards = i.width >= CARDS_FROM_W && i.height >= CARDS_FROM_H;
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    if !knots.is_empty() {
        let used = if cards {
            knot_cards(p, app, theme, &knots, at, Rect::new(i.x, y, i.width, i.height), now, &mut hits)
        } else {
            let room = (i.height / 3).max(3);
            knot_lines(p, app, theme, &knots, at, Rect::new(i.x, y, i.width, room), now, &mut hits)
        };
        y += used;
    }

    // The day.
    let bottom = i.bottom().saturating_sub(gantt_h);
    if bottom > y {
        threads(p, app, theme, &sections.dags, knots.len(), at, Rect::new(i.x, y, i.width, bottom - y), now, &mut hits);
    }
    if gantt_h > 0 {
        let r = Rect::new(i.x, i.bottom() - gantt_h, i.width, gantt_h);
        match selected {
            Some(RowKey::Running(dag, run) | RowKey::Queued(dag, run) | RowKey::Failed(dag, run)) => {
                if let Some(run) = activity.run(dag, run) {
                    gantt(p, app, theme, r, run, now);
                }
            }
            Some(RowKey::Dag(dag)) => {
                if let Some(line) = sections.dags.iter().find(|l| l.id == dag) {
                    dag_card(p, app, theme, r, line, now);
                }
            }
            None => {}
        }
    }
    app.viewport.hits.borrow_mut().extend(hits);
}

/// `last 24 h  217 runs  ✔ 211  ✖ 3   ▸ 2 running  ◌ 1 queued  ↻ 1 retrying   13 of 13 DAGs ran ·
/// 3 paused`, and how fresh it is at the right.
fn day_line(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, now: i64) {
    let activity = &app.airflow;
    let w = i.width as usize;
    let fresh = kit::freshness(activity.age(app.clock), activity.error.as_deref(), app.is_reading(Feed::Airflow), theme, w / 2);
    let c = activity.counts(now);
    let mut cells = Cells::new();
    cells.push("last 24 h  ", theme.faint());
    if !activity.day_read {
        cells.push("reading…", theme.accent());
    } else {
        cells.push(c.runs.to_string(), theme.strong());
        cells.push(" runs  ", theme.muted());
        cells.push("✔ ", theme.sev(Severity::Ok));
        cells.push(c.ok.to_string(), theme.text2());
        cells.push("  ", Style::default());
        if c.failed > 0 {
            cells.push(format!("✖ {}", c.failed), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        } else {
            cells.push("✖ 0", theme.faint());
        }
        let wide = w >= 100;
        cells.push("   ", Style::default());
        cells.push("▸ ", theme.accent());
        cells.push(c.running.to_string(), if c.running > 0 { theme.strong() } else { theme.faint() });
        if wide {
            cells.push(" running", theme.muted());
        }
        cells.push("  ◌ ", theme.muted());
        cells.push(c.queued.to_string(), if c.queued > 0 { theme.strong() } else { theme.faint() });
        if wide {
            cells.push(" queued", theme.muted());
        }
        if c.retrying > 0 {
            cells.push("  ", Style::default());
            cells.push(if wide { format!("↻ {} retrying", c.retrying) } else { format!("↻ {}", c.retrying) }, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
        }
        let active = c.dags.saturating_sub(c.paused);
        let tail = if wide { format!("{} of {active} DAGs ran · {} paused", c.ran, c.paused) } else { format!("{} of {active} DAGs", c.ran) };
        if cells.width() + 3 + fmt::width(&tail) + fresh.width() + 2 <= w {
            cells.push("   ", Style::default());
            cells.push(tail, theme.muted());
        }
    }
    // What is unwell beyond the title's two beats: the DAG processor, the metadatabase.
    if activity.health.dag_processor.severity() == Severity::Crit {
        cells.push(format!("  ✖ dag processor {}", activity.health.dag_processor.status.as_deref().unwrap_or("?")), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if let Some(db) = activity.health.metadatabase.as_deref().filter(|s| *s != "healthy") {
        cells.push(format!("  ✖ metadatabase {db}"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if let Some(errors) = activity.import_errors.filter(|n| *n > 0) {
        cells.push(format!("  ▲ {}", fmt::plural(errors as usize, "import error", "import errors")), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    let room = w.saturating_sub(fresh.width() + 1);
    kit::put_cells(p, i.x, i.y, cells, room);
    kit::put_spans_r(p, i.right(), i.y, fresh.into_spans());
}

/// `NOW · 2 running · 1 stuck · 1 queued · 3 failed today`.
fn now_label(p: &mut Painter<'_>, theme: &Theme, sections: &airflow::Sections<'_>, now: i64, i: Rect, y: u16) {
    let mut cells = Cells::new();
    cells.push("NOW", theme.section());
    let stuck = sections.running.iter().filter(|r| r.is_stuck(now)).count();
    if sections.running.is_empty() && sections.queued.is_empty() && sections.failed.is_empty() {
        cells.push(" · nothing runs, nothing waits · ", theme.muted());
        cells.push("the day went well ✔", theme.sev(Severity::Ok));
    } else {
        cells.push(format!(" · {} running", sections.running.len()), theme.muted());
        if stuck > 0 {
            cells.push(format!(" · {stuck} stuck"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
        if !sections.queued.is_empty() {
            cells.push(format!(" · {} queued", sections.queued.len()), theme.muted());
        }
        if !sections.failed.is_empty() {
            cells.push(format!(" · {} failed today", sections.failed.len()), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
    }
    kit::put_cells(p, i.x, y, cells, i.width as usize);
}

fn owners(activity: &Activity, dag: &str) -> Option<String> {
    activity.dag(dag).map(|d| d.owners.join(", ")).filter(|o| !o.is_empty())
}

fn mark(run: &Run, now: i64, theme: &Theme) -> (&'static str, Style) {
    match (run.state, run.severity(now)) {
        (RunState::Failed, _) | (_, Severity::Crit) => ("✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        (_, Severity::Warn) => ("▲", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        (RunState::Queued, _) => ("◌", theme.muted()),
        _ => ("▸", theme.accent().add_modifier(Modifier::BOLD)),
    }
}

/// A knot's three lines: its state and a live duration, who and when, what its tasks are doing.
fn knot_lines_of(app: &App, run: &Run, now: i64, theme: &Theme, room: usize) -> [Cells; 3] {
    let activity = &app.airflow;
    let spin = kit::spinner(app);
    let mut first = Cells::new();
    let mut second = Cells::new();
    let mut third = Cells::new();
    let when = |at: Option<i64>| at.map(|a| airflow::when(a, &app.time, now)).unwrap_or_default();
    let owner = owners(activity, &run.dag);
    match run.state {
        RunState::Running => {
            let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
            if run.is_stuck(now) {
                first.push("stuck · ", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
                first.push(format!("running {took}"), theme.sev(Severity::Crit));
            } else {
                let sev = run.severity(now);
                first.push(format!("running {took}"), if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text() });
                if let Some(pr) = activity.progress_of(run).filter(|p| p.total > 0) {
                    first.push(format!(" · {} of {} ", pr.done, pr.total), theme.text2());
                    first.spans(thin_bar(Some(f64::from(pr.done) / f64::from(pr.total) * 100.0), 8, theme.bar_fill(Severity::None), theme));
                }
            }
            second.push(airflow::run_label(run, &app.time, now), theme.muted());
            if run.is_stuck(now) {
                second.push(format!(" {}", app.time.format(run.start.unwrap_or(now), "%H:%M")), theme.muted());
            }
            let tasks = activity.tasks_of(run);
            if let Some(task) = tasks.iter().find(|t| t.retrying()) {
                third.push(format!("↻ {} waits for a retry", task.id), theme.sev(Severity::Warn));
            } else if let Some(task) = tasks.iter().find(|t| !t.retrying()) {
                third.push(format!("{spin} "), theme.accent());
                third.push(task.id.clone(), theme.text2());
                if let Some(start) = task.start {
                    third.push(format!(" · {}", fmt::dur((now - start).max(0) as f64)), theme.muted());
                }
                let more = tasks.iter().filter(|t| !t.retrying()).count().saturating_sub(1);
                if more > 0 {
                    third.push(format!(" +{more}"), theme.muted());
                }
            } else {
                third.push("no task is live right now", theme.faint());
            }
        }
        RunState::Queued => {
            let waited = run.waited(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
            let sev = run.severity(now);
            first.push("queued · ", theme.muted());
            first.push(format!("waiting {waited}"), if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text() });
            second.push(airflow::run_label(run, &app.time, now), theme.muted());
            third.push("no task has started", theme.faint());
        }
        _ => {
            match activity.progress_of(run) {
                Some(pr) if !pr.failed.is_empty() => {
                    first.push("failed at ", theme.sev(Severity::Crit));
                    first.push(pr.failed[0].clone(), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
                    if pr.failed.len() > 1 {
                        first.push(format!(" +{}", pr.failed.len() - 1), theme.muted());
                    }
                }
                Some(pr) => {
                    first.push(pr.without_a_failure(), theme.sev(Severity::Crit));
                }
                None => {
                    first.push("failed · its tasks are being read…", theme.sev(Severity::Crit));
                }
            }
            let kind = airflow::run_label(run, &app.time, now);
            let kind = kind.split(" · ").next().unwrap_or_default().to_string();
            second.push(format!("{kind} · {}", when(run.start.or(run.at()))), theme.muted());
            if let Some(end) = run.end {
                second.push(format!(" → {}", app.time.format(end, "%H:%M")), theme.muted());
            }
            if let Some(took) = run.took(now) {
                second.push(format!(" · {}", fmt::dur(took as f64)), theme.text2());
            }
            if let Some(end) = run.end {
                third.push(fmt::ago(now - end), theme.text2());
                let since = activity.runs.iter().filter(|r| r.dag == run.dag && r.state == RunState::Success && r.start.is_some_and(|s| s > end)).count();
                if since > 0 {
                    third.push(" · ", theme.faint());
                    third.push(format!("✔ {} since", fmt::plural(since, "run", "runs")), theme.sev(Severity::Ok));
                }
            }
        }
    }
    if let Some(owner) = owner {
        let line = if run.state == RunState::Failed { &mut third } else { &mut second };
        if line.width() + 3 + fmt::width(&owner) <= room {
            if line.width() > 0 {
                line.push(" · ", theme.faint());
            }
            line.push(owner, theme.person());
        }
    }
    [first, second, third]
}

/// The knots as cards, `per_row` a row, at most two rows — the row of the cursor's card kept
/// on screen; returns the rows used.
#[allow(clippy::too_many_arguments)]
fn knot_cards(p: &mut Painter<'_>, app: &App, theme: &Theme, knots: &[&Run], at: Option<usize>, r: Rect, now: i64, hits: &mut Vec<(Rect, Hit)>) -> u16 {
    let per_row = ((r.width + 1) / (CARD_W + 1)).max(1) as usize;
    let card_w = (r.width - (per_row as u16 - 1)) / per_row as u16;
    let rows = knots.len().div_ceil(per_row);
    let shown = rows.min(2);
    let cursor_row = at.filter(|a| *a < knots.len()).map(|a| a / per_row).unwrap_or(0);
    let first_row = cursor_row.saturating_sub(shown - 1).min(rows - shown);
    for (k, run) in knots.iter().enumerate() {
        let row = k / per_row;
        if row < first_row || row >= first_row + shown {
            continue;
        }
        let x = r.x + (k % per_row) as u16 * (card_w + 1);
        let y = r.y + ((row - first_row) as u16) * CARD_H;
        let rect = Rect::new(x, y, card_w, CARD_H);
        let chosen = at == Some(k);
        let (glyph, style) = mark(run, now, theme);
        let frame_style = if chosen { theme.accent() } else { frame::frame_style(theme, run.severity(now)) };
        let title = if chosen {
            vec![Span::styled(format!("{glyph} {}", run.dag), theme.tab_active())]
        } else {
            vec![Span::styled(glyph.to_string(), style), Span::styled(format!(" {}", run.dag), theme.strong())]
        };
        let inner = Card::new(frame_style).title(title).draw(p, rect);
        for (n, line) in knot_lines_of(app, run, now, theme, inner.width as usize).into_iter().enumerate() {
            kit::put_cells(p, inner.x, inner.y + n as u16, line, inner.width as usize);
        }
        hits.push((rect, Hit::AirflowRow(k)));
    }
    shown as u16 * CARD_H
}

/// The knots a line each, under 100 columns or 24 rows, in a window that follows the cursor;
/// returns the rows used.
#[allow(clippy::too_many_arguments)]
fn knot_lines(p: &mut Painter<'_>, app: &App, theme: &Theme, knots: &[&Run], at: Option<usize>, r: Rect, now: i64, hits: &mut Vec<(Rect, Hit)>) -> u16 {
    let room = r.height as usize;
    let cursor = at.filter(|a| *a < knots.len());
    let first = scroll_into_view(0, cursor, room, knots.len());
    let name_w = if r.width >= 100 { 32 } else { 28 };
    for (n, (k, run)) in knots.iter().enumerate().skip(first).take(room).enumerate() {
        let y = r.y + n as u16;
        let chosen = cursor == Some(k);
        if chosen {
            kit::band(p, theme, r, y);
        }
        let (glyph, style) = mark(run, now, theme);
        let mut cells = Cells::new();
        cells.push(if chosen { "▌" } else { " " }, theme.accent());
        cells.push(glyph.to_string(), style);
        cells.push(" ", Style::default());
        cells.cell(&run.dag, name_w, theme.strong());
        cells.gap(2);
        let [a, b, c] = knot_lines_of(app, run, now, theme, r.width as usize);
        cells.spans(a.into_spans());
        if c.width() > 0 && run.state != RunState::Queued {
            cells.push(" · ", theme.faint());
            cells.spans(c.into_spans());
        } else {
            cells.push(" · ", theme.faint());
            cells.spans(b.into_spans());
        }
        kit::put_cells(p, r.x, y, cells, r.width as usize);
        hits.push((Rect::new(r.x, y, r.width, 1), Hit::AirflowRow(k)));
    }
    room.min(knots.len()) as u16
}

/// The day's cells: a success a calm square, a failure a red cross, a run going on an arrow, a
/// wait a ring; a run that went on past its cell a line through the cells it filled.
fn cell_glyph(cell: &Cell, theme: &Theme) -> (&'static str, Style) {
    let calm = Style::default().fg(theme.bar_fill(Severity::None));
    match cell {
        Cell::Empty => ("·", theme.faint()),
        Cell::Start(RunState::Success) => ("▪", calm),
        Cell::Through(RunState::Success) => ("━", calm),
        Cell::Start(RunState::Failed) => ("✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        Cell::Through(RunState::Failed) => ("━", theme.sev(Severity::Crit)),
        Cell::Start(RunState::Running) => ("▸", theme.accent().add_modifier(Modifier::BOLD)),
        Cell::Through(RunState::Running) => ("━", theme.accent()),
        Cell::Start(RunState::Queued) => ("◌", theme.muted()),
        Cell::Through(RunState::Queued) => ("┄", theme.muted()),
        Cell::Start(RunState::Other) | Cell::Through(RunState::Other) => ("▫", theme.muted()),
    }
}

/// How long until, in the fewest characters: `12m`, `5h`, `6d`.
fn short(seconds: i64) -> String {
    crate::jira::age(seconds)
}

/// A thread's tail: `✖1 · every 15m · in 7m`, `triggered · running 2h12m`, `paused`.
fn thread_tail(line: &DagLine<'_>, now: i64, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    if line.failed > 0 {
        cells.push(format!("✖{}", line.failed), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        cells.push(" · ", theme.faint());
    }
    cells.push(airflow::short_schedule(line.dag), theme.muted());
    match line.last {
        Some(run) if run.state == RunState::Running => {
            let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_default();
            cells.push(" · ", theme.faint());
            cells.push(format!("running {took}"), theme.accent());
            return cells;
        }
        Some(run) if run.state == RunState::Queued => {
            cells.push(" · ", theme.faint());
            cells.push("queued", theme.muted());
            return cells;
        }
        _ => {}
    }
    match line.dag {
        Some(dag) if dag.paused => {
            cells.push(" · ", theme.faint());
            cells.push("paused", theme.muted());
        }
        Some(dag) => {
            if let Some(next) = dag.next.filter(|n| *n > now) {
                cells.push(" · ", theme.faint());
                cells.push(format!("in {}", short(next - now)), theme.text2());
            }
        }
        None => {}
    }
    cells
}

/// The day as a loom: the label, the hours over the cells with the now line, a thread a DAG.
#[allow(clippy::too_many_arguments)]
fn threads(p: &mut Painter<'_>, app: &App, theme: &Theme, dags: &[DagLine<'_>], first_index: usize, at: Option<usize>, r: Rect, now: i64, hits: &mut Vec<(Rect, Hit)>) {
    let activity = &app.airflow;
    if r.height < 2 {
        return;
    }
    if !activity.day_read {
        kit::put_spans(p, r.x, r.y, vec![Span::styled("THE DAY", theme.section()), Span::styled(" · reading the last 24 h of runs…", theme.accent())], r.width as usize);
        return;
    }
    let w = r.width as usize;
    let (name_w, tail_w) = if w >= 100 { (32, 30) } else { (28, 18) };
    let slots = airflow::slots_for(w.saturating_sub(2 + name_w + 2 + 2 + tail_w));
    let offset_s = app.time.offset_s(now);
    let axis = Axis::new(now, slots.max(1), offset_s);
    let cell = match slots {
        96 => "a cell a quarter hour",
        48 => "a cell a half hour",
        24 => "a cell an hour",
        12 => "a cell two hours",
        _ => "",
    };
    let room = r.height.saturating_sub(2) as usize;
    let cursor = at.and_then(|a| a.checked_sub(first_index)).filter(|a| *a < dags.len());
    let offset = scroll_into_view(app.viewport.airflow.get(), cursor, room, dags.len());
    app.viewport.airflow.set(offset);
    let mut label = vec![Span::styled("THE DAY", theme.section()), Span::styled(format!(" · {} ran", fmt::plural(dags.len(), "DAG", "DAGs")), theme.muted())];
    if !cell.is_empty() {
        label.push(Span::styled(format!(" · {cell}"), theme.faint()));
    }
    if dags.len() > room {
        let below = dags.len().saturating_sub(offset + room);
        let mut more = format!(" · {room} shown");
        if offset > 0 {
            more.push_str(&format!(" · ↑ {offset} more"));
        }
        if below > 0 {
            more.push_str(&format!(" · ↓ {below} more"));
        }
        label.push(Span::styled(more, theme.faint()));
    }
    kit::put_spans(p, r.x, r.y, label, w);
    if dags.is_empty() {
        kit::put_spans(p, r.x + 2, r.y + 1, vec![Span::styled("no DAG ran in the last 24 h", theme.muted())], w);
        return;
    }
    // The hours over the cells they begin, the now line after the last, the clock by it.
    let cells_x = r.x + 2 + name_w as u16 + 2;
    let now_x = cells_x + slots as u16;
    for (at, label) in axis.labels(offset_s) {
        if at + label.chars().count() <= slots {
            p.put(cells_x + at as u16, r.y + 1, &label, theme.faint());
        }
    }
    p.put(now_x, r.y + 1, "┃", theme.accent());
    p.put(now_x + 2, r.y + 1, &format!("now {}", app.time.format(now, if w >= 100 { "%H:%M:%S" } else { "%H:%M" })), theme.accent());
    for (n, (k, line)) in dags.iter().enumerate().skip(offset).take(room).enumerate() {
        let y = r.y + 2 + n as u16;
        let chosen = cursor == Some(k);
        if chosen {
            kit::band(p, theme, r, y);
        }
        p.put(r.x, y, if chosen { "▌" } else { " " }, theme.accent());
        let name_style = if line.severity(now) == Severity::Crit { theme.sev(Severity::Crit) } else if chosen { theme.strong() } else { theme.text() };
        p.put(r.x + 2, y, &fmt::truncate(line.id, name_w), name_style);
        for (c, cell) in airflow::timeline(&line.runs, &axis, now).iter().enumerate() {
            let (glyph, style) = cell_glyph(cell, theme);
            p.put(cells_x + c as u16, y, glyph, style);
        }
        p.put(now_x, y, "┃", theme.accent());
        kit::put_cells(p, now_x + 2, y, thread_tail(line, now, theme), r.right().saturating_sub(now_x + 2) as usize);
        hits.push((Rect::new(r.x, y, r.width, 1), Hit::AirflowRow(first_index + k)));
    }
}

/// Clock labels for an axis of `cells` from `start` to `end`: at round times, at least eight
/// cells apart, as (cell, label).
pub fn time_axis(start: i64, end: i64, cells: usize, offset_s: i64, clock: &crate::clock::Clock) -> Vec<(usize, String)> {
    let span = (end - start).max(1);
    if cells == 0 {
        return Vec::new();
    }
    let per_cell = span as f64 / cells as f64;
    const STEPS: [i64; 16] = [60, 120, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400, 172_800, 604_800, 2_592_000, 7_776_000];
    let step = STEPS.into_iter().find(|s| *s as f64 / per_cell >= 10.0).unwrap_or(31_536_000);
    let format = match step {
        2_592_000.. => "%b",
        86_400.. => "%b %-d",
        _ => "%H:%M",
    };
    // Months are not thirty days: past a week the labels go where the month (or the day) turns.
    let mut t = (start + offset_s).div_euclid(step.min(86_400)) * step.min(86_400) - offset_s;
    if t < start {
        t += step.min(86_400);
    }
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut last_end = 0usize;
    let mut last_label = String::new();
    while t < end {
        let at = ((t - start) as f64 / per_cell) as usize;
        let label = clock.format(t, format);
        let round = step < 86_400 || label != last_label;
        if round && (out.is_empty() || at >= last_end + 2) && at + label.chars().count() <= cells {
            last_end = at + label.chars().count();
            out.push((at, label.clone()));
        }
        last_label = label;
        t += if step < 86_400 { step } else { 86_400 };
    }
    out
}

/// A task's place on the axis, in cells: where it began and where it ended (or now).
fn span_cells(start: i64, end: i64, axis_start: i64, per_cell: f64, cells: usize) -> (usize, usize) {
    let a = (((start - axis_start) as f64 / per_cell).floor().max(0.0) as usize).min(cells.saturating_sub(1));
    let b = (((end - axis_start) as f64 / per_cell).ceil().max(0.0) as usize).clamp(a + 1, cells);
    (a, b)
}

/// The run under the cursor as a Gantt on its own time axis.
fn gantt(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, run: &Run, now: i64) {
    let activity = &app.airflow;
    let progress = activity.progress_of(run);
    let mut title = vec![Span::styled(run.dag.clone(), theme.strong()), Span::styled(format!(" · {}", airflow::run_label(run, &app.time, now)), theme.muted())];
    let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_default();
    let state = match run.state {
        RunState::Running => format!(" · running {took}"),
        RunState::Queued => format!(" · queued {}", run.waited(now).map(|s| fmt::dur(s as f64)).unwrap_or_default()),
        RunState::Failed => format!(" · failed after {took}"),
        _ => format!(" · {} in {took}", run.state.word()),
    };
    title.push(Span::styled(state, if run.severity(now).is_problem() { theme.sev(run.severity(now)) } else { theme.text2() }));
    if let Some(pr) = progress.filter(|p| p.total > 0) {
        title.push(Span::styled(format!(" · {} of {} tasks", pr.done, pr.total), theme.muted()));
    }
    let right = vec![Span::styled("⏎ task log · o in Airflow", theme.muted())];
    let key = (run.dag.clone(), run.id.clone());
    let tasks = app.airflow_tasks.get(&key).map(|(t, _)| t.as_slice());
    let mut card = Card::new(theme.border()).title(title).right(right);
    let inner_h = r.height.saturating_sub(2) as usize;
    let task_rows = inner_h.saturating_sub(1);
    // The rows around what matters: the running or failed task, the done ones above it.
    let window = tasks.map(|tasks| {
        let focus = tasks.iter().position(|t| matches!(t.state.as_str(), "running" | "failed" | "up_for_retry")).unwrap_or(tasks.len().saturating_sub(1));
        let first = focus.saturating_sub(task_rows.saturating_sub(1) / 2 + task_rows.saturating_sub(1) % 2).min(tasks.len().saturating_sub(task_rows));
        (first, first + task_rows.min(tasks.len()))
    });
    if let (Some(tasks), Some((a, b))) = (tasks, window)
        && (a > 0 || b < tasks.len())
    {
        let mut note = vec![Span::styled(fmt::plural(tasks.len(), "task", "tasks"), theme.muted())];
        if a > 0 {
            let t = &tasks[a - 1];
            note.push(Span::styled(format!(" · ↑ {} {}", t.label(), task_glyph(t, "◐", theme).0), theme.faint()));
        }
        if b < tasks.len() {
            let t = &tasks[b];
            note.push(Span::styled(format!(" · ↓ {} {}", t.label(), task_glyph(t, "◐", theme).0), theme.faint()));
        }
        card = card.bottom(note);
    }
    let inner = card.draw(p, r);
    if inner.height == 0 {
        return;
    }
    let w = inner.width as usize;
    let Some(tasks) = tasks else {
        let mut cells = Cells::new();
        cells.push(format!("{} ", kit::spinner(app)), theme.accent());
        if let Some(pr) = progress {
            cells.push(format!("{} of {} done", pr.done, pr.total), theme.text2());
            if let Some(task) = pr.running.first() {
                cells.push(format!(" · ▸ {task}"), theme.text2());
            }
            cells.push(" · ", theme.faint());
        }
        cells.push("reading its tasks…", theme.accent());
        kit::put_cells(p, inner.x, inner.y, cells, w);
        return;
    };
    if tasks.is_empty() {
        kit::put_spans(p, inner.x, inner.y, vec![Span::styled("no task of this run has been scheduled", theme.muted())], w);
        return;
    }
    let name_w = tasks.iter().map(|t| fmt::width(&t.label())).max().unwrap_or(8).clamp(8, 24);
    let status_w = 30usize.min(w / 3);
    let cells = w.saturating_sub(name_w + 2 + 2 + status_w);
    if cells < 8 {
        return;
    }
    let live = run.state.is_live();
    let start = run.start.or(run.queued).or_else(|| tasks.iter().filter_map(|t| t.start).min()).unwrap_or(now);
    let end = if live { now } else { run.end.or_else(|| tasks.iter().filter_map(|t| t.end).max()).unwrap_or(now) };
    let end = end.max(start + 60);
    let per_cell = (end - start) as f64 / cells as f64;
    let bars_x = inner.x + name_w as u16 + 2;
    let line_x = bars_x + cells as u16;
    let offset_s = app.time.offset_s(now);
    // The axis: round clock times, and the now line (or the run's end) with its time.
    p.put(inner.x, inner.y, "task", theme.section());
    for (at, label) in time_axis(start, end, cells, offset_s, &app.time) {
        p.put(bars_x + at as u16, inner.y, &label, theme.faint());
    }
    let edge_style = if live { theme.accent() } else { theme.rule() };
    p.put(line_x, inner.y, "┃", edge_style);
    let edge = if live { format!("now {}", app.time.format(now, "%H:%M:%S")) } else { format!("ended {}", app.time.format(end, "%H:%M")) };
    p.put(line_x + 2, inner.y, &edge, edge_style);
    let (a, b) = window.unwrap_or((0, 0));
    let spin = kit::spinner(app);
    for (n, task) in tasks[a..b].iter().enumerate() {
        let y = inner.y + 1 + n as u16;
        if y >= inner.bottom() {
            break;
        }
        let sev = task.severity();
        p.put(inner.x, y, &fmt::truncate(&task.label(), name_w), if sev.is_problem() && sev != Severity::Ok { theme.sev(sev) } else { theme.text2() });
        let (glyph, glyph_style) = task_glyph(task, spin, theme);
        if let Some(t0) = task.start {
            let t1 = task.end.unwrap_or(now).min(end);
            let (c0, c1) = span_cells(t0, t1, start, per_cell, cells);
            let bar_style = match task.state.as_str() {
                "failed" => theme.sev(Severity::Crit),
                "running" => theme.accent(),
                _ => Style::default().fg(theme.bar_fill(Severity::None)),
            };
            p.put(bars_x + c0 as u16, y, &"━".repeat(c1 - c0), bar_style);
            let at = if task.state == "running" { c0 } else { c1.saturating_sub(1) };
            p.put(bars_x + at as u16, y, glyph, glyph_style);
        }
        p.put(line_x, y, "┃", edge_style);
        let status = task_status(app, task, now, theme);
        kit::put_cells(p, line_x + 2, y, status, inner.right().saturating_sub(line_x + 2) as usize);
    }
}

fn task_glyph(task: &TaskRun, spin: &'static str, theme: &Theme) -> (&'static str, Style) {
    match task.state.as_str() {
        "success" => ("✔", theme.sev(Severity::Ok)),
        "failed" => ("✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        "running" => (spin, theme.accent().add_modifier(Modifier::BOLD)),
        "up_for_retry" | "up_for_reschedule" => ("↻", theme.sev(Severity::Warn)),
        "upstream_failed" => ("▲", theme.sev(Severity::Warn)),
        "skipped" => ("»", theme.muted()),
        _ => ("◌", theme.muted()),
    }
}

/// `done 14:40 · 42m`, `running 41m · try 1 of 1`, `◌ queued`.
fn task_status(app: &App, task: &TaskRun, now: i64, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    let took = task.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_default();
    match task.state.as_str() {
        "success" => {
            cells.push(format!("done {} · {took}", task.end.map(|e| app.time.format(e, "%H:%M")).unwrap_or_default()), theme.muted());
        }
        "failed" => {
            cells.push(format!("failed {} · {took}", task.end.map(|e| app.time.format(e, "%H:%M")).unwrap_or_default()), theme.sev(Severity::Crit));
        }
        "running" => {
            cells.push(format!("running {took}"), theme.text());
            cells.push(format!(" · try {} of {}", task.try_number.max(1), task.max_tries.max(1)), theme.muted());
        }
        "up_for_retry" => {
            cells.push(format!("↻ waits for try {} of {}", task.try_number + 1, task.max_tries.max(1)), theme.sev(Severity::Warn));
        }
        "upstream_failed" => {
            cells.push("▲ upstream failed", theme.sev(Severity::Warn));
        }
        other => {
            let word = if other.is_empty() { "not scheduled" } else { other };
            cells.push(format!("◌ {}", word.replace('_', " ")), theme.muted());
        }
    }
    cells
}

/// A DAG's thread under the cursor: its schedule and owners, the day in numbers, its
/// description.
fn dag_card(p: &mut Painter<'_>, _app: &App, theme: &Theme, r: Rect, line: &DagLine<'_>, now: i64) {
    let mut title = vec![Span::styled(line.id.to_string(), theme.strong())];
    title.push(Span::styled(format!(" · {}", airflow::short_schedule(line.dag)), theme.muted()));
    if let Some(dag) = line.dag
        && !dag.owners.is_empty()
    {
        title.push(Span::styled(" · ", theme.faint()));
        title.push(Span::styled(dag.owners.join(", "), theme.person()));
    }
    let inner = Card::new(theme.border()).title(title).right(vec![Span::styled("⏎ its runs · o in Airflow", theme.muted())]).draw(p, r);
    let w = inner.width as usize;
    let mut cells = Cells::new();
    cells.push(fmt::plural(line.runs.len(), "run", "runs"), theme.strong());
    cells.push(" in the last 24 h · ", theme.muted());
    cells.push(format!("✔ {}", line.ok), theme.sev(Severity::Ok));
    if line.failed > 0 {
        cells.push("  ", Style::default());
        cells.push(format!("✖ {}", line.failed), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if line.live > 0 {
        cells.push(format!("  ▸ {} live", line.live), theme.accent());
    }
    if let Some(run) = line.last_done {
        cells.push(format!(" · last ended {}", run.end.map(|e| fmt::ago(now - e)).unwrap_or_default()), theme.text2());
        if let Some(took) = run.took(now) {
            cells.push(format!(" after {}", fmt::dur(took as f64)), theme.text2());
        }
    }
    if let Some(dag) = line.dag {
        match dag.next.filter(|n| *n > now) {
            _ if dag.paused => cells.push(" · paused", theme.muted()),
            Some(next) => cells.push(format!(" · next in {}", short(next - now)), theme.text2()),
            None => &mut cells,
        };
    }
    kit::put_cells(p, inner.x, inner.y, cells, w);
    if let Some(text) = line.dag.and_then(|d| d.description.as_deref()).filter(|d| !d.trim().is_empty()) {
        let lines = super::board::wrap(text, w, (inner.height as usize).saturating_sub(2).max(1));
        for (k, l) in lines.into_iter().enumerate() {
            p.put(inner.x, inner.y + 1 + k as u16, &l, theme.text2());
        }
    }
    if let Some(dag) = line.dag
        && !dag.tags.is_empty()
        && inner.height >= 2
    {
        p.put(inner.x, inner.bottom() - 1, &fmt::truncate(&dag.tags.join(" · "), w), theme.faint());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gantt_s_axis_is_labelled_at_round_times_far_enough_apart() {
        let clock = crate::clock::Clock { pinned: chrono::FixedOffset::east_opt(7 * 3600), ..Default::default() };
        // 13:39 to 15:52 Jakarta on 56 cells: half hours, never closer than ten cells.
        let start = 1_791_096_000 + 39 * 60 - 7 * 3600 + 13 * 3600 - 1_791_096_000 % 86_400;
        let labels = time_axis(start, start + 133 * 60, 56, 7 * 3600, &clock);
        assert!(labels.len() >= 3, "{labels:?}");
        assert!(labels.windows(2).all(|w| w[1].0 - w[0].0 >= 10), "{labels:?}");
        assert!(labels.iter().all(|(_, l)| l.ends_with(":00") || l.ends_with(":30")), "{labels:?}");
    }

    #[test]
    fn a_task_takes_at_least_a_cell_and_never_runs_past_the_axis() {
        assert_eq!(span_cells(0, 10, 0, 60.0, 50), (0, 1));
        assert_eq!(span_cells(600, 1200, 0, 60.0, 50), (10, 20));
        assert_eq!(span_cells(2900, 9000, 0, 60.0, 50), (48, 50));
    }
}
