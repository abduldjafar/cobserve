//! View 5: Airflow — what every DAG did over the last day.
//!
//! The scheduler's health and the day in numbers first. Then what matters now: RUNNING, every
//! run in progress with how far its tasks are and what they are doing — one going for more than a
//! day is stuck, and is drawn so; QUEUED, what waits; FAILED, the day's failures and the task
//! each failed at. Last, ACTIVITY: a line per DAG that ran, its day on a timeline — a cell an hour
//! (finer on a wide terminal) on the hours of the clock on screen — so a night's batch, a gap or a
//! run of failures shows as a shape, beside its schedule, its runs, when it last ran, how long
//! that took and when it runs next.

use super::widgets::{rule, thin_bar, Cells};
use crate::airflow::{self, Activity, Axis, Cell, DagLine, Run, RunState};
use crate::app::{scroll_into_view, App, Feed, Hit};
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Columns are two spaces apart, so a full cell never runs into the next one.
const GAP: usize = 2;
/// `scheduled · 06:00`, `manual · Jan 16`, `scheduled · Apr 2024`.
const LABEL: usize = 20;
/// The longest a timeline gets: half-hour cells. Finer than that and a cell is a run.
const SLOTS_MAX: usize = 48;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let activity = &app.airflow;
    let width = area.width as usize;
    let height = area.height as usize;
    if !activity.reachable {
        let (text, style) = match activity.error.as_deref() {
            Some(airflow::NOT_CONFIGURED) => (
                "  Airflow is not configured: set AIRFLOW_URL, AIRFLOW_USER and AIRFLOW_PASSWORD (the login of its web UI), or add airflow: url, user, password to the --credential file".to_string(),
                theme.muted(),
            ),
            Some(airflow::NOT_READ) | None => ("  reading Airflow…".to_string(), theme.muted()),
            Some(why) => (format!("  Airflow could not be read: {why}"), theme.sev(Severity::Warn)),
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))).wrap(Wrap { trim: false }), area);
        return;
    }

    // Wide: the day woven, what needs a look told above it (`loom.rs`).
    if width >= super::loom::FROM_WIDTH {
        super::loom::draw(frame, app, theme, area);
        return;
    }
    let now = app.now();
    let offset_s = app.time.offset_s(now);
    let sections = activity.sections(now);
    let grid = Grid::of(&sections, width);
    let axis = Axis::new(now, grid.slots.max(1), offset_s);
    let selected = app.airflow_selection();
    let is = |key: airflow::RowKey| selected == Some(&key);

    let mut out = Out { lines: vec![title_line(app, now, theme, width), day_line(activity, now, theme, width)], rows: Vec::new(), selected_line: None };

    // RUNNING, always: an empty one says so.
    let stuck = sections.running.iter().filter(|r| r.is_stuck(now)).count();
    let mut detail = vec![Span::styled(format!(" · {}", sections.running.len()), theme.muted())];
    if stuck > 0 {
        detail.push(Span::styled(format!(" · {stuck} stuck"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)));
    }
    out.section("RUNNING", detail, theme, width);
    if sections.running.is_empty() {
        out.lines.push(Line::from(Span::styled("    nothing runs", theme.muted())));
    }
    for run in &sections.running {
        let selected = is(airflow::RowKey::Running(run.dag.clone(), run.id.clone()));
        out.row(running_line(activity, run, &grid, app, now, selected, theme, width), selected);
    }

    if !sections.queued.is_empty() {
        out.section("QUEUED", vec![Span::styled(format!(" · {}", sections.queued.len()), theme.muted())], theme, width);
        for run in &sections.queued {
            let selected = is(airflow::RowKey::Queued(run.dag.clone(), run.id.clone()));
            out.row(queued_line(run, &grid, app, now, selected, theme, width), selected);
        }
    }

    if !activity.day_read {
        out.section("ACTIVITY", vec![Span::styled(" · the last 24 h", theme.muted())], theme, width);
        out.lines.push(Line::from(Span::styled("    reading the last 24 h of runs…", theme.muted())));
    } else if !sections.failed.is_empty() {
        let detail = vec![Span::styled(format!(" · {} in the last 24 h", sections.failed.len()), theme.muted())];
        out.section("FAILED", detail, theme, width);
        for run in &sections.failed {
            let selected = is(airflow::RowKey::Failed(run.dag.clone(), run.id.clone()));
            out.row(failed_line(activity, run, &grid, app, now, selected, theme, width), selected);
        }
    }

    // The day, once it is in (above, until then, a line says it is being read).
    if activity.day_read {
        let detail = vec![Span::styled(format!(" · {} ran in the last 24 h", fmt::plural(sections.dags.len(), "DAG", "DAGs")), theme.muted())];
        out.section("ACTIVITY", detail, theme, width);
        if sections.dags.is_empty() {
            out.lines.push(Line::from(Span::styled("    no DAG ran in the last 24 h", theme.muted())));
        } else {
            out.lines.push(activity_header(&grid, &axis, offset_s, theme, width));
        }
        for line in &sections.dags {
            let selected = is(airflow::RowKey::Dag(line.id.to_string()));
            out.row(dag_line(line, &grid, &axis, now, selected, theme, width), selected);
        }
    }

    let len = out.lines.len();
    let offset = scroll_into_view(app.viewport.airflow.get(), out.selected_line, height, len);
    app.viewport.airflow.set(offset);
    let mut hits = app.viewport.hits.borrow_mut();
    for (index, &line) in out.rows.iter().enumerate() {
        if line >= offset && line < offset + height {
            hits.push((Rect::new(area.x, area.y + (line - offset) as u16, area.width, 1), Hit::AirflowRow(index)));
        }
    }
    drop(hits);
    let visible: Vec<Line<'static>> = out.lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

/// The lines as they are built, each row's line in the order of `Sections::keys`, and where
/// the cursor's row landed.
struct Out {
    lines: Vec<Line<'static>>,
    rows: Vec<usize>,
    selected_line: Option<usize>,
}

impl Out {
    fn section(&mut self, title: &str, detail: Vec<Span<'static>>, theme: &Theme, width: usize) {
        self.lines.push(Line::from(""));
        let mut spans = vec![Span::styled(title.to_string(), theme.section())];
        spans.extend(detail);
        self.lines.push(rule(width, spans, theme));
    }

    fn row(&mut self, line: Line<'static>, selected: bool) {
        if selected {
            self.selected_line = Some(self.lines.len());
        }
        self.rows.push(self.lines.len());
        self.lines.push(line);
    }
}

/// `airflow.example.net · Airflow 2.10.2 · scheduler ● 2s · triggerer ● 4s        read 4s ago`
pub(super) fn title_line(app: &App, now: i64, theme: &Theme, width: usize) -> Line<'static> {
    let activity = &app.airflow;
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.push(activity.host().unwrap_or_else(|| "Airflow".to_string()), theme.strong());
    if let Some(version) = &activity.version {
        cells.push(format!(" · Airflow {version}"), theme.muted());
    }
    let health = &activity.health;
    for (name, beat) in [("scheduler", &health.scheduler), ("triggerer", &health.triggerer), ("dag processor", &health.dag_processor)] {
        let sev = beat.severity();
        if beat.status.is_none() {
            continue;
        }
        cells.push(format!(" · {name} "), theme.muted());
        if sev == Severity::Ok {
            cells.push("●", theme.sev(Severity::Ok));
            if let Some(at) = beat.at {
                cells.push(format!(" {}", fmt::dur((now - at).max(0) as f64)), theme.faint());
            }
        } else {
            cells.push(format!("✖ {}", beat.status.as_deref().unwrap_or("?")), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        }
    }
    if let Some(db) = health.metadatabase.as_deref().filter(|s| *s != "healthy") {
        cells.push(" · metadatabase ", theme.muted());
        cells.push(format!("✖ {db}"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    if let Some(errors) = activity.import_errors.filter(|n| *n > 0) {
        cells.push(" · ", theme.muted());
        cells.push(format!("▲ {}", fmt::plural(errors as usize, "import error", "import errors")), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    let right = super::jira::freshness(activity.age(app.clock), activity.error.as_deref(), app.is_reading(Feed::Airflow), theme, width / 2);
    cells.pad_to(width.saturating_sub(right.width()));
    cells.spans(right.into_spans());
    cells.line(width, Style::default())
}

/// `last 24 h  631 runs  ✔ 628  ✖ 3     ▸ 3 running  ◌ 1 queued  ↻ 1 retrying     13 of 16 DAGs ran · 3 paused`
pub(super) fn day_line(activity: &Activity, now: i64, theme: &Theme, width: usize) -> Line<'static> {
    let c = activity.counts(now);
    let mut cells = Cells::new();
    cells.push("  last 24 h   ", theme.faint());
    if !activity.day_read {
        cells.push("reading…", theme.accent());
        return cells.line(width, Style::default());
    }
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
    cells.push("     ", Style::default());
    cells.push("▸ ", theme.accent());
    cells.push(c.running.to_string(), if c.running > 0 { theme.strong() } else { theme.faint() });
    cells.push(" running  ", theme.muted());
    cells.push("◌ ", theme.muted());
    cells.push(c.queued.to_string(), if c.queued > 0 { theme.strong() } else { theme.faint() });
    cells.push(" queued", theme.muted());
    if c.retrying > 0 {
        cells.push("  ", Style::default());
        cells.push(format!("↻ {} retrying", c.retrying), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    }
    let active = c.dags.saturating_sub(c.paused);
    let tail = format!("{} of {active} DAGs ran · {} paused", c.ran, c.paused);
    if cells.width() + 5 + fmt::width(&tail) <= width {
        cells.push("     ", Style::default());
        cells.push(tail, theme.muted());
    }
    cells.line(width, Style::default())
}

/// The cells every row shares, as wide as what is in them. The timeline gets what is left, at
/// least a cell an hour; to make that room the next run goes, then how long the last took, then
/// the DAG's name is cut shorter, then the schedule.
#[derive(Debug, Clone, Copy)]
struct Grid {
    dag: usize,
    owner: usize,
    schedule: usize,
    runs: usize,
    slots: usize,
    last: usize,
    took: usize,
    next: usize,
}

/// From this width the owner of each DAG has a column.
const OWNER_FROM: usize = 150;

impl Grid {
    fn of(sections: &airflow::Sections<'_>, width: usize) -> Grid {
        let names = sections
            .running
            .iter()
            .chain(&sections.queued)
            .chain(&sections.failed)
            .map(|r| fmt::width(&r.dag))
            .chain(sections.dags.iter().map(|l| fmt::width(l.id)));
        let owners = sections.dags.iter().filter_map(|l| l.dag.map(|d| fmt::width(&d.owners.join(", "))));
        let mut grid = Grid {
            dag: names.max().unwrap_or(16).clamp(16, 34),
            owner: if width >= OWNER_FROM { owners.max().unwrap_or(0).clamp(5, 16) } else { 0 },
            schedule: 12,
            runs: 6,
            slots: 0,
            last: 9,
            took: 7,
            next: 7,
        };
        for step in 0..4 {
            if grid.room(width) >= 24 {
                break;
            }
            match step {
                0 => grid.next = 0,
                1 => grid.took = 0,
                2 => grid.dag = grid.dag.min(24),
                _ => grid.schedule = 0,
            }
        }
        grid.slots = airflow::slots_for(grid.room(width).min(SLOTS_MAX));
        grid
    }

    /// What is left for the timeline.
    fn room(&self, width: usize) -> usize {
        let fixed = 2 + self.dag + cell(self.owner) + cell(self.schedule) + cell(self.runs) + GAP + cell(self.last) + cell(self.took) + cell(self.next);
        width.saturating_sub(fixed)
    }
}

/// A cell and the gap before it, or nothing for a cell that is not shown.
fn cell(width: usize) -> usize {
    if width > 0 { width + GAP } else { 0 }
}

/// The selection bar, then the row's mark.
fn row_start(selected: bool, mark: &str, style: Style, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    cells.cell(mark, 2, style);
    cells
}

fn row_style(selected: bool, theme: &Theme) -> Style {
    if selected { theme.selected() } else { Style::default() }
}

/// The first columns of a run's row: its DAG and which run it is.
fn run_head(cells: &mut Cells, run: &Run, grid: &Grid, app: &App, now: i64, theme: &Theme) {
    cells.cell(&run.dag, grid.dag, theme.strong());
    cells.gap(GAP);
    cells.cell(&airflow::run_label(run, &app.time, now), LABEL, theme.muted());
    cells.gap(GAP);
}

/// `▸ statement_daily_agg_reload   manual · 13:39   2h13m   4/7 ━━━━━╸━━   ▸ reload_partitions`
#[allow(clippy::too_many_arguments)]
fn running_line(activity: &Activity, run: &Run, grid: &Grid, app: &App, now: i64, selected: bool, theme: &Theme, width: usize) -> Line<'static> {
    let sev = run.severity(now);
    let (mark, mark_style) = match sev {
        Severity::Crit => ("✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
        Severity::Warn => ("▲", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        _ => ("▸", theme.accent().add_modifier(Modifier::BOLD)),
    };
    let mut cells = row_start(selected, mark, mark_style, theme);
    run_head(&mut cells, run, grid, app, now, theme);
    let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
    let took_style = if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text() };
    cells.cell_right(&took, 8, took_style);
    cells.gap(GAP);

    let progress = activity.progress_of(run);
    match progress.filter(|p| p.total > 0) {
        Some(p) => {
            cells.cell_right(&format!("{}/{}", p.done, p.total), 7, theme.text2());
            cells.push(" ", Style::default());
            let share = f64::from(p.done) / f64::from(p.total) * 100.0;
            cells.spans(thin_bar(Some(share), 8, theme.bar_fill(Severity::None), theme));
        }
        None => {
            cells.gap(16);
        }
    }
    cells.gap(GAP);

    // What its tasks are doing — running ones by name, one waiting for a retry — and for a stuck
    // run since when, that first. Each piece whole or not at all; the drawer has every one.
    let tasks = activity.tasks_of(run);
    let running: Vec<String> = tasks.iter().filter(|t| !t.retrying()).map(|t| t.id.clone()).collect();
    let mut pieces: Vec<Cells> = Vec::new();
    if run.is_stuck(now) {
        let since = run.start.map(|s| airflow::when(s, &app.time, now)).unwrap_or_default();
        let mut piece = Cells::new();
        piece.push(format!("stuck since {since}"), theme.sev(Severity::Crit));
        pieces.push(piece);
    }
    if let Some(first) = running.first() {
        let mut piece = Cells::new();
        piece.push("▸ ", theme.accent());
        piece.push(first.clone(), theme.text2());
        if running.len() > 1 {
            piece.push(format!(" +{}", running.len() - 1), theme.muted());
        }
        pieces.push(piece);
    }
    if let Some(task) = tasks.iter().find(|t| t.retrying()) {
        let mut piece = Cells::new();
        piece.push(format!("↻ {} to retry", task.id), theme.sev(Severity::Warn));
        pieces.push(piece);
    }
    let room = width.saturating_sub(cells.width());
    let mut tail = Cells::new();
    for piece in pieces {
        let gap = if tail.width() > 0 { 3 } else { 0 };
        if tail.width() + gap + piece.width() > room {
            continue;
        }
        if gap > 0 {
            tail.push(" · ", theme.faint());
        }
        tail.spans(piece.into_spans());
    }
    cells.spans(tail.into_spans());
    cells.line(width, row_style(selected, theme))
}

/// `◌ gateway_transfers_backfill   manual · 15:48   4m12s   waiting for a slot`
fn queued_line(run: &Run, grid: &Grid, app: &App, now: i64, selected: bool, theme: &Theme, width: usize) -> Line<'static> {
    let sev = run.severity(now);
    let mut cells = row_start(selected, "◌", theme.muted(), theme);
    run_head(&mut cells, run, grid, app, now, theme);
    let waited = run.waited(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
    cells.cell_right(&waited, 8, if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text() });
    cells.gap(GAP);
    cells.push("waiting to start", theme.faint());
    cells.line(width, row_style(selected, theme))
}

/// `✖ kyc_onboarding_tables   scheduled · 05:30   12m00s   ended 05:42 · 3h ago   at build_cohorts`
#[allow(clippy::too_many_arguments)]
fn failed_line(activity: &Activity, run: &Run, grid: &Grid, app: &App, now: i64, selected: bool, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = row_start(selected, "✖", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD), theme);
    run_head(&mut cells, run, grid, app, now, theme);
    let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
    cells.cell_right(&took, 8, theme.text());
    cells.gap(GAP);
    if let Some(end) = run.end {
        cells.push(format!("ended {}", app.time.format(end, "%H:%M")), theme.text2());
        cells.push(format!(" · {}", fmt::ago(now - end)), theme.muted());
        cells.gap(GAP);
    }
    let room = width.saturating_sub(cells.width());
    let mut tail = Cells::new();
    match activity.progress_of(run) {
        Some(p) if !p.failed.is_empty() => {
            tail.push("at ", theme.muted());
            tail.push(p.failed[0].clone(), theme.sev(Severity::Crit));
            if p.failed.len() > 1 {
                tail.push(format!(" +{}", p.failed.len() - 1), theme.muted());
            }
        }
        // Failed by the scheduler or by hand, or out of time: no task of it did.
        Some(p) => {
            tail.push(p.without_a_failure(), theme.muted());
        }
        None => {
            tail.push("its tasks are being read…", theme.faint());
        }
    }
    cells.spans(super::widgets::fit(tail.into_spans(), room, false));
    cells.line(width, row_style(selected, theme))
}

/// `DAG  SCHEDULE  RUNS  18    00    06    12   LAST  TOOK  NEXT` — the hours over the cells
/// they begin.
fn activity_header(grid: &Grid, axis: &Axis, offset_s: i64, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("  ", Style::default());
    cells.cell("DAG", grid.dag, theme.section());
    if grid.owner > 0 {
        cells.gap(GAP);
        cells.cell("OWNER", grid.owner, theme.section());
    }
    if grid.schedule > 0 {
        cells.gap(GAP);
        cells.cell("SCHEDULE", grid.schedule, theme.section());
    }
    cells.gap(GAP);
    cells.cell_right("RUNS", grid.runs, theme.section());
    cells.gap(GAP);
    if grid.slots > 0 {
        let mut hours = vec![' '; grid.slots];
        for (at, label) in axis.labels(offset_s) {
            if at + label.chars().count() <= grid.slots {
                for (i, c) in label.chars().enumerate() {
                    hours[at + i] = c;
                }
            }
        }
        cells.push(hours.into_iter().collect::<String>(), theme.faint());
    }
    if grid.last > 0 {
        cells.gap(GAP);
        cells.cell_right("LAST RUN", grid.last, theme.section());
    }
    if grid.took > 0 {
        cells.gap(GAP);
        cells.cell_right("TOOK", grid.took, theme.section());
    }
    if grid.next > 0 {
        cells.gap(GAP);
        cells.cell_right("NEXT", grid.next, theme.section());
    }
    cells.line(width, theme.table_head())
}

/// One DAG's day: `clickhouse_replication_check  every 15m  96  ▪▪▪▪▪▪▪▪▪▪▪  3m ago  1m35s  in 12m`
fn dag_line(line: &DagLine<'_>, grid: &Grid, axis: &Axis, now: i64, selected: bool, theme: &Theme, width: usize) -> Line<'static> {
    let sev = line.severity(now);
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    let name_style = if sev == Severity::Crit { theme.sev(Severity::Crit) } else { theme.text() };
    cells.cell(line.id, grid.dag, name_style);
    if grid.owner > 0 {
        cells.gap(GAP);
        let owners = line.dag.map(|d| d.owners.join(", ")).unwrap_or_default();
        cells.cell(&owners, grid.owner, theme.person());
    }
    if grid.schedule > 0 {
        cells.gap(GAP);
        cells.cell(&airflow::short_schedule(line.dag), grid.schedule, theme.muted());
    }
    cells.gap(GAP);
    // `96`, or `11 ✖1` when some failed.
    let mut runs = Cells::new();
    runs.push(line.runs.len().to_string(), theme.text2());
    if line.failed > 0 {
        runs.push(format!(" ✖{}", line.failed), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    cells.gap(grid.runs.saturating_sub(runs.width()));
    cells.spans(runs.into_spans());
    cells.gap(GAP);
    if grid.slots > 0 {
        cells.spans(timeline_spans(&airflow::timeline(&line.runs, axis, now), theme));
    }
    if grid.last > 0 {
        cells.gap(GAP);
        match line.last {
            Some(run) if run.state == RunState::Running => cells.cell_right("running", grid.last, theme.accent()),
            Some(run) if run.state == RunState::Queued => cells.cell_right("queued", grid.last, theme.muted()),
            Some(run) => cells.cell_right(&run.at().map(|at| fmt::ago(now - at)).unwrap_or_default(), grid.last, theme.text2()),
            None => cells.cell_right("—", grid.last, theme.faint()),
        };
    }
    if grid.took > 0 {
        cells.gap(GAP);
        let took = line.last_done.and_then(|r| r.took(now)).map(|s| fmt::dur(s as f64)).unwrap_or_else(|| "—".into());
        cells.cell_right(&took, grid.took, theme.muted());
    }
    if grid.next > 0 {
        cells.gap(GAP);
        let next = match line.dag {
            Some(dag) if dag.paused => "paused".to_string(),
            Some(dag) => dag.next.filter(|n| *n > now).map(|n| format!("in {}", short(n - now))).unwrap_or_else(|| "—".into()),
            None => "—".to_string(),
        };
        cells.cell_right(&next, grid.next, theme.muted());
    }
    cells.line(width, row_style(selected, theme))
}

/// How long until, in the fewest characters: `12m`, `5h`, `6d`.
pub(super) fn short(seconds: i64) -> String {
    crate::jira::age(seconds)
}

/// The day's cells: a success a calm square, a failure a red cross, a run going on an arrow, a
/// wait a ring; a run that went on past its cell a line through the cells it filled.
pub(super) fn timeline_spans(cells: &[Cell], theme: &Theme) -> Vec<Span<'static>> {
    let calm = Style::default().fg(theme.bar_fill(Severity::None));
    let crit = theme.sev(Severity::Crit).add_modifier(Modifier::BOLD);
    let run = theme.accent().add_modifier(Modifier::BOLD);
    let mut out = Cells::new();
    for cell in cells {
        let (glyph, style) = match cell {
            Cell::Empty => ("·", theme.faint()),
            Cell::Start(RunState::Success) => ("▪", calm),
            Cell::Through(RunState::Success) => ("━", calm),
            Cell::Start(RunState::Failed) => ("✖", crit),
            Cell::Through(RunState::Failed) => ("━", theme.sev(Severity::Crit)),
            Cell::Start(RunState::Running) => ("▸", run),
            Cell::Through(RunState::Running) => ("━", theme.accent()),
            Cell::Start(RunState::Queued) => ("◌", theme.muted()),
            Cell::Through(RunState::Queued) => ("┄", theme.muted()),
            Cell::Start(RunState::Other) | Cell::Through(RunState::Other) => ("▫", theme.muted()),
        };
        out.push(glyph, style);
    }
    out.into_spans()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeline_gets_an_hour_a_cell_at_the_target_size_and_more_when_wider() {
        let activity = crate::fake::airflow(1_791_103_927);
        let sections = activity.sections(1_791_103_927);
        let target = Grid::of(&sections, 116);
        assert_eq!(target.slots, 24, "{target:?}");
        assert_eq!(target.owner, 0, "no room for owners at 120: {target:?}");
        let wide = Grid::of(&sections, 196);
        assert_eq!(wide.slots, 48, "{wide:?}");
        assert!(wide.owner > 0, "{wide:?}");
        let narrow = Grid::of(&sections, 76);
        assert!(narrow.slots >= 12, "a timeline still fits at 80: {narrow:?}");
        assert_eq!((narrow.next, narrow.took), (0, 0), "{narrow:?}");
    }
}
