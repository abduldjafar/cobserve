//! View 2: Redash — what runs, what waits, and what only looks as if it ran (§2.8).
//!
//! First the queues, one line each — running, waiting, workers, stale — with the idle ones
//! folded into a single line. Then the jobs: RUNNING, every job a worker holds, with the
//! ClickHouse query it became and what that query costs right now (the stitch: a full queue
//! is usually a few workers stuck on runaway queries, and this is where that shows); WAITING,
//! who is waiting and for how long against the red line at 3 minutes; and STALE, what RQ's
//! started list still holds although no worker runs it. The cursor opens the job's SQL under
//! its row, like the tree does for a query.

use super::widgets::{code_block, dots, rule, sparkline, thin_bar, Cells, Scale};
use crate::app::{scroll_into_view, App, Hit, QueueRowRef};
use crate::fmt;
use crate::model::{Job, JobState, QueueRow, Stale};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The queue Redash runs people's queries on; its row is shown even when it is idle.
const MAIN_QUEUE: &str = "queries";
/// The narrowest the QUERY column gets before the line is cut at the end instead.
const QUERY_MIN: usize = 14;
/// Columns are two spaces apart, so a full cell never runs into the next one.
const GAP: usize = 2;

/// `21.2G`, `312M`, `<1M`: memory in the space of a few characters.
fn short_bytes(bytes: u64) -> String {
    let gib = crate::fmt::gib(bytes);
    if gib >= 1.0 {
        format!("{gib:.1}G")
    } else if bytes >= 1024 * 1024 {
        format!("{}M", bytes / (1024 * 1024))
    } else {
        "<1M".to_string()
    }
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let height = area.height as usize;

    if !app.queue.reachable {
        // The strip above already says why; repeating it would be noise, not information.
        app.viewport.sql_max.set(0);
        let message = if app.queue.error.as_deref() == Some(crate::model::QUEUE_NOT_CONFIGURED) {
            "  Redash is not configured: add redash: url, api_key (and redis_url, for the names of waiting jobs) to the --credential file, or set REDASH_URL, REDASH_ADMIN_API_KEY and REDIS_URL"
        } else {
            "  no queue data — the strip above says why"
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(message, theme.muted()))), area);
        return;
    }

    // Every row here is a job, so a block is always open: it gets less room than the tree's.
    let mut out = Out::new(app.queue_selection(), (height * 35 / 100).clamp(1, 9));
    out.lines.push(title_line(app, theme, width));
    out.lines.push(Line::from(""));
    queue_table(app, theme, width, &mut out.lines);

    let sections = app.queue_sections();
    let everyone: Vec<&Job> = sections
        .running
        .iter()
        .chain(&sections.waiting)
        .chain(&sections.stale)
        .map(|(_, job)| *job)
        .collect();
    let columns = Columns::of(&everyone, width);

    running_section(app, theme, width, &columns, &sections.running, &mut out);
    waiting_section(app, theme, width, &columns, &sections.waiting, &mut out);
    stale_section(app, theme, width, &columns, &sections.stale, &mut out);
    if out.selected_line.is_none() {
        app.viewport.sql_max.set(0);
    }

    // The window keeps the selected row and the SQL under it in view, the row first.
    let len = out.lines.len();
    let mut offset = app.viewport.queue.get();
    if let Some(line) = out.selected_line {
        offset = scroll_into_view(offset, Some(line + out.block_len), height, len);
        offset = scroll_into_view(offset, Some(line), height, len);
    } else {
        offset = scroll_into_view(offset, None, height, len);
    }
    app.viewport.queue.set(offset);
    for &(line, index) in &out.jobs {
        if line >= offset && line < offset + height {
            app.viewport.hits.borrow_mut().push((Rect::new(area.x, area.y + (line - offset) as u16, area.width, 1), Hit::Job(index)));
        }
    }
    let visible: Vec<Line<'static>> = out.lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

/// The lines of the view as they are built, and where the cursor's row landed.
struct Out {
    lines: Vec<Line<'static>>,
    /// Each job's line, and its index: what a click on it is.
    jobs: Vec<(usize, usize)>,
    /// The next job row's index, in the order [`App::queue_rows`] gives.
    index: usize,
    selected: Option<usize>,
    selected_line: Option<usize>,
    block_len: usize,
    /// How many lines the SQL under the selected row may take.
    room: usize,
}

impl Out {
    fn new(selected: Option<usize>, room: usize) -> Out {
        Out { lines: Vec::new(), jobs: Vec::new(), index: 0, selected, selected_line: None, block_len: 0, room }
    }

    fn is_selected(&self) -> bool {
        self.selected == Some(self.index)
    }

    /// A job's row, with its SQL under it when it is the selected one.
    fn push_job(&mut self, line: Line<'static>, job: &Job, app: &App, theme: &Theme, width: usize) {
        self.jobs.push((self.lines.len(), self.index));
        if self.is_selected() {
            self.selected_line = Some(self.lines.len());
            self.lines.push(line);
            let block = match app.job_sql(job) {
                Some((sql, _)) => {
                    let scroll = app.sql_scroll_for(&App::job_sql_key(job));
                    let (block, max_scroll) = code_block(&sql, "    ", scroll, self.room, width, theme);
                    app.viewport.sql_max.set(max_scroll);
                    block
                }
                None => {
                    app.viewport.sql_max.set(0);
                    Vec::new()
                }
            };
            self.block_len = block.len();
            self.lines.extend(block);
        } else {
            self.lines.push(line);
        }
        self.index += 1;
    }
}

/// `redash.example.net · Redash 10.1.0 · 7 workers, 6 busy            read 2s ago`
fn title_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let queue = &app.queue;
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.push(queue.host().unwrap_or("Redash").to_string(), theme.strong());
    if let Some(version) = &queue.version {
        cells.push(format!(" · Redash {version}"), theme.muted());
    }
    let workers = match (queue.workers_total, queue.workers_busy) {
        (0, _) => "no live worker".to_string(),
        (total, 0) => format!("{} idle", fmt::plural(total as usize, "worker", "workers")),
        (total, busy) => format!("{}, {busy} busy", fmt::plural(total as usize, "worker", "workers")),
    };
    cells.push(format!(" · {workers}"), if queue.workers_total == 0 { theme.sev(Severity::Warn) } else { theme.muted() });
    if let Some(age) = queue.age(app.clock) {
        let read = format!("read {} ago ", fmt::dur(age.as_secs_f64()));
        cells.pad_to(width.saturating_sub(fmt::width(&read)));
        cells.push(read, theme.faint());
    }
    cells.line(width, Style::default())
}

/// One line per queue that has something going on, the main one always, the idle rest
/// folded into one line.
fn queue_table(app: &App, theme: &Theme, width: usize, lines: &mut Vec<Line<'static>>) {
    let queue = &app.queue;
    let shown: Vec<&QueueRow> = queue
        .queues
        .iter()
        .filter(|q| !q.is_idle() || q.name == MAIN_QUEUE)
        .collect();
    let idle: Vec<&str> = queue
        .queues
        .iter()
        .filter(|q| q.is_idle() && q.name != MAIN_QUEUE)
        .map(|q| q.name.as_str())
        .collect();
    if shown.is_empty() && idle.is_empty() {
        lines.push(Line::from(Span::styled("  Redash reports no queues", theme.muted())));
        return;
    }
    let name_w = shown.iter().map(|q| fmt::width(&q.name)).max().unwrap_or(0).clamp(8, 24);
    // Ages exist only where Redis names the waiting jobs; elsewhere the column would be a
    // row of dashes.
    let oldest = queue.names_available;
    let trend = queue.queues.iter().any(|q| {
        app.history
            .queue(&q.name)
            .and_then(|s| s.max_in(crate::history::WINDOW_S))
            .is_some_and(|m| m > 0.0)
    });

    let mut header = Cells::new();
    header.push(" ", Style::default());
    header.cell("QUEUE", name_w, theme.section());
    header.gap(GAP);
    header.cell_right("RUNNING", 7, theme.section());
    header.gap(GAP);
    header.cell_right("WAITING", 7, theme.section());
    header.gap(GAP);
    if oldest {
        header.cell("OLDEST", 9, theme.section());
    }
    header.cell("WORKERS", 22, theme.section());
    header.cell_right("STALE", 5, theme.section());
    if trend {
        header.gap(3);
        header.push("WAITING · last 4 min", theme.section());
    }
    lines.push(header.line(width, theme.table_head()));

    for row in shown {
        let saturated = row.saturated();
        let wait = row.oldest_wait_s.unwrap_or(0);
        let mut cells = Cells::new();
        cells.push(" ", Style::default());
        cells.cell(&row.name, name_w, theme.strong());
        cells.gap(GAP);
        cells.cell_right(&row.running.to_string(), 7, if row.running > 0 { theme.strong() } else { theme.muted() });
        cells.gap(GAP);
        let count_sev = severity::wait(wait, saturated);
        cells.cell_right(
            &row.waiting.to_string(),
            7,
            if row.waiting > 0 { theme.sev(count_sev).add_modifier(Modifier::BOLD) } else { theme.muted() },
        );
        cells.gap(GAP);
        if oldest {
            let wait_sev = severity::wait(wait, false);
            let text = match row.oldest_wait_s {
                Some(s) if row.waiting > 0 => format!("{}{}", fmt::dur(s as f64), wait_sev.mark()),
                _ => String::new(),
            };
            cells.cell(&text, 9, theme.sev(wait_sev));
        }
        let workers = workers_cell(row, theme);
        let used = workers.width();
        cells.spans(workers.into_spans());
        cells.gap(22usize.saturating_sub(used));
        if row.stale > 0 {
            cells.cell_right(&row.stale.to_string(), 5, theme.muted());
        } else {
            cells.cell_right("·", 5, theme.faint());
        }
        if trend {
            cells.gap(3);
            let series = app.history.queue(&row.name);
            let top = series
                .and_then(|s| s.max_in(crate::history::WINDOW_S))
                .unwrap_or(0.0)
                .max(10.0);
            cells.spans(sparkline(series, crate::history::secs(queue.taken_at), 16, Scale::Fixed(0.0, top), |v| {
                theme.bar_fill(if v >= 10.0 { Severity::Warn } else { Severity::None })
            }));
        }
        lines.push(cells.line(width, Style::default()));
    }
    if !idle.is_empty() {
        let mut cells = Cells::new();
        cells.push(" ", Style::default());
        cells.push(idle.join(" · "), theme.text2());
        cells.push("  idle", theme.faint());
        lines.push(cells.line(width, Style::default()));
    }
}

/// `●●●● 4/4 busy`, `○ 1 idle`, `no worker` — red when jobs wait for one.
fn workers_cell(row: &QueueRow, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    if row.workers_total == 0 {
        if row.waiting > 0 {
            cells.push("no worker", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        } else {
            cells.push("no worker", theme.muted());
        }
        return cells;
    }
    // Every worker busy is only a problem with jobs waiting behind them.
    let sev = if row.saturated() && row.waiting > 0 { Severity::Warn } else { Severity::None };
    let slots = dots(row.workers_busy, row.workers_total, theme.bar_fill(sev), theme);
    let drawn = !slots.is_empty();
    cells.spans(slots);
    let text = if row.workers_busy == 0 {
        format!("{} idle", row.workers_total)
    } else {
        format!("{}/{} busy", row.workers_busy, row.workers_total)
    };
    cells.push(format!("{}{text}", if drawn { " " } else { "" }), if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
    cells
}

/// The job lists' shared columns, as wide as what is in them.
#[derive(Clone, Copy)]
struct Columns {
    who: usize,
    /// 0 when every job is on one queue: the section titles name it then.
    queue: usize,
    source: usize,
}

/// Below this width the QUEUE column goes, and QUERY gets its room: the drawer still names
/// the queue of the selected job.
const QUEUE_COLUMN_FROM: usize = 150;

impl Columns {
    fn of(jobs: &[&Job], width: usize) -> Columns {
        let widest = |f: &dyn Fn(&Job) -> usize, lo: usize, hi: usize| jobs.iter().map(|j| f(j)).max().unwrap_or(0).clamp(lo, hi);
        let queue = one_queue(jobs).is_none() && width >= QUEUE_COLUMN_FROM;
        Columns {
            who: widest(&|j| fmt::width(&j.label()), 6, 22),
            queue: if queue { widest(&|j| fmt::width(&j.queue), 5, 18) } else { 0 },
            source: widest(&|j| j.data_source.as_deref().map_or(1, fmt::width), 6, 18),
        }
    }

    /// The columns for one list: no QUEUE column when all of it is on one queue.
    fn for_list(&self, jobs: &[&Job]) -> Columns {
        Columns { queue: if one_queue(jobs).is_some() { 0 } else { self.queue }, ..*self }
    }

    /// What the columns before QUERY take, after the time column.
    fn before_query(&self) -> usize {
        self.who + GAP + if self.queue > 0 { self.queue + GAP } else { 0 }
    }

    /// QUERY takes what the other columns leave.
    fn query(&self, width: usize, lead: usize, tail: usize) -> usize {
        let fixed = 2 + lead + self.before_query() + GAP + self.source + GAP + tail;
        width.saturating_sub(fixed).max(QUERY_MIN)
    }
}

/// The queue every job of a list is on, when there is just one.
fn one_queue<'a>(jobs: &[&'a Job]) -> Option<&'a str> {
    let first = jobs.first()?.queue.as_str();
    jobs.iter().all(|j| j.queue == first).then_some(first)
}

/// `─ TITLE · detail ─────`, then the column header.
fn section_head(
    title: &str,
    detail: String,
    header: Vec<(&str, usize)>,
    theme: &Theme,
    width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    lines.push(Line::from(""));
    lines.push(rule(
        width,
        vec![Span::styled(title.to_string(), theme.section()), Span::styled(detail, theme.muted())],
        theme,
    ));
    let mut cells = Cells::new();
    cells.push("  ", Style::default());
    for (text, cells_wide) in header {
        if cells_wide == 0 {
            cells.push(text.to_string(), theme.section());
        } else {
            cells.cell(text, cells_wide, theme.section());
        }
    }
    lines.push(cells.line(width, theme.table_head()));
}

/// The columns every job row shares: WHO, QUEUE when there are several, QUERY, SOURCE.
fn job_cells(cells: &mut Cells, job: &Job, columns: &Columns, query_w: usize, dim: bool, theme: &Theme) {
    let quiet = |style: Style| if dim { theme.muted() } else { style };
    cells.cell(&job.label(), columns.who, if job.person.is_some() { quiet(theme.person()) } else { theme.faint() });
    cells.gap(GAP);
    if columns.queue > 0 {
        cells.cell(&job.queue, columns.queue, theme.muted());
        cells.gap(GAP);
    }
    let known = job.redash_query_id.is_some() || job.query_name.is_some();
    cells.cell(&job.query_label(), query_w, if known { quiet(theme.text()) } else { theme.muted() });
    cells.gap(GAP);
    cells.cell(job.data_source.as_deref().unwrap_or("—"), columns.source, quiet(theme.accent()));
    cells.gap(GAP);
}

fn header_columns<'a>(lead: (&'a str, usize), columns: &Columns, query_w: usize, tail: &'a str) -> Vec<(&'a str, usize)> {
    let mut header = vec![lead, ("WHO", columns.who + GAP)];
    if columns.queue > 0 {
        header.push(("QUEUE", columns.queue + GAP));
    }
    header.push(("QUERY", query_w + GAP));
    header.push(("SOURCE", columns.source + GAP));
    if !tail.is_empty() {
        header.push((tail, 0));
    }
    header
}

/// The selection bar and the space after it.
fn row_start(selected: bool, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    cells
}

fn row_style(selected: bool, theme: &Theme) -> Style {
    if selected { theme.selected() } else { Style::default() }
}

// -- WAITING ------------------------------------------------------------------------------

/// The time column of WAITING: the wait, its mark, and how far along to the red line at
/// 3 minutes it is.
const WAIT_LEAD: usize = 7 + 2 + 1 + 6 + GAP;

fn waiting_section(app: &App, theme: &Theme, width: usize, columns: &Columns, rows: &[QueueRowRef<'_>], out: &mut Out) {
    let total = app.queue.total_waiting();
    if total == 0 && rows.is_empty() {
        // The table already says 0; a section saying it again is noise.
        return;
    }
    let jobs: Vec<&Job> = rows.iter().map(|(_, j)| *j).collect();
    let columns = &columns.for_list(&jobs);
    let query_w = columns.query(width, WAIT_LEAD, 0);
    let mut detail = format!(" · {total}");
    if let Some(queue) = one_queue(&jobs) {
        detail.push_str(&format!(" in {queue}"));
    } else if !jobs.is_empty() {
        detail.push_str(" in the queues");
    }
    if app.queue.names_available && (jobs.len() as u32) < total {
        detail.push_str(&format!(" · {} named", jobs.len()));
    }
    section_head("WAITING", detail, header_columns(("WAIT", WAIT_LEAD), columns, query_w, ""), theme, width, &mut out.lines);
    if jobs.is_empty() {
        out.lines.push(Line::from(Span::styled(
            if app.queue.names_available {
                "    Redis has not named them yet"
            } else {
                "    who is waiting needs Redis: add redis_url under redash: in the credential file (or set REDIS_URL) — Redash's API only counts them"
            },
            theme.muted(),
        )));
        return;
    }
    for (_, job) in rows {
        let selected = out.is_selected();
        let saturated = app.queue.queue(&job.queue).is_some_and(QueueRow::saturated);
        let sev = severity::wait(job.age_s, saturated);
        let mut cells = row_start(selected, theme);
        cells.cell_right(&fmt::dur(job.age_s as f64), 7, theme.sev(sev).add_modifier(Modifier::BOLD));
        match app.cancelling(&job.id) {
            Some(_) => cells.cell(" ⊘", 3, theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
            None => cells.cell(sev.mark(), 3, theme.sev(sev)),
        };
        cells.spans(thin_bar(Some(job.age_s as f64 / 180.0 * 100.0), 6, theme.bar_fill(sev), theme));
        cells.gap(GAP);
        job_cells(&mut cells, job, columns, query_w, false, theme);
        out.push_job(cells.line(width, row_style(selected, theme)), job, app, theme, width);
    }
}

// -- RUNNING ------------------------------------------------------------------------------

/// The time column of RUNNING and STALE: the age and a mark.
const AGE_LEAD: usize = 7 + 2 + GAP;

fn running_section(app: &App, theme: &Theme, width: usize, columns: &Columns, rows: &[QueueRowRef<'_>], out: &mut Out) {
    let jobs: Vec<&Job> = rows.iter().map(|(_, j)| *j).collect();
    let columns = &columns.for_list(&jobs);
    let tails: Vec<Cells> = jobs
        .iter()
        .map(|job| {
            let mut tail = cancel_mark(job, app, theme);
            tail.spans(clickhouse_cell(job, app, theme).into_spans());
            tail
        })
        .collect();
    let tail_head = "IN CLICKHOUSE · mem · cores · done";
    let tail_w = tails.iter().map(Cells::width).max().unwrap_or(0).max(fmt::width(tail_head)).min(40);
    let query_w = columns.query(width, AGE_LEAD, tail_w);
    let mut detail = format!(" · {} on a worker", jobs.len());
    if let Some(queue) = one_queue(&jobs) {
        detail.push_str(&format!(" · {queue}"));
    }
    section_head(
        "RUNNING",
        detail,
        header_columns(("FOR", AGE_LEAD), columns, query_w, tail_head),
        theme,
        width,
        &mut out.lines,
    );
    if jobs.is_empty() {
        out.lines.push(Line::from(Span::styled("    nothing on a worker", theme.muted())));
        return;
    }
    for (job, tail) in jobs.into_iter().zip(tails) {
        let selected = out.is_selected();
        let runaway = job.clickhouse_target().is_some_and(|(node, id)| app.query_is_runaway(node, id));
        // A running job inherits the runaway state of its ClickHouse query (§2.8).
        let sev = if runaway { Severity::Crit } else { severity::elapsed(job.age_s as f64) };
        let mut cells = row_start(selected, theme);
        cells.cell_right(&fmt::dur(job.age_s as f64), 7, theme.sev(sev).add_modifier(Modifier::BOLD));
        cells.cell(if runaway { " ✕" } else { "" }, 2, theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
        cells.gap(GAP);
        job_cells(&mut cells, job, columns, query_w, false, theme);
        cells.spans(tail.into_spans());
        out.push_job(cells.line(width, row_style(selected, theme)), job, app, theme, width);
    }
}

/// `⊘` on a job Redash has been asked to cancel: sent, or taken and not let go of yet.
fn cancel_mark(job: &Job, app: &App, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    match app.cancelling(&job.id) {
        Some(c) if c.accepted => cells.push("⊘ cancelled · ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        Some(_) => cells.push("⊘ cancelling… · ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        None => &mut cells,
    };
    cells
}

/// Where a running job is in ClickHouse, and what it costs there right now — or why it is
/// not there at all.
fn clickhouse_cell(job: &Job, app: &App, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    match job.clickhouse_target() {
        Some((node, query_id)) => {
            cells.push("→ ", theme.faint());
            cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
            let cost = app
                .with_view(|view| {
                    view.nodes
                        .iter()
                        .filter(|n| n.node.name == node)
                        .flat_map(|n| n.users.iter())
                        .flat_map(|u| u.queries.iter())
                        .find(|q| q.query.query_id == query_id)
                        .map(|q| (q.query.memory_bytes, q.cores, q.progress))
                })
                .flatten();
            if let Some((bytes, cores, progress)) = cost {
                cells.push(format!(" {} {cores:.1}c", short_bytes(bytes)), theme.text2());
                if let Some(p) = progress {
                    cells.push(format!(" {}", fmt::pct0(p * 100.0)), theme.accent());
                }
            }
        }
        None => {
            let why = match (job.on_clickhouse(), job.data_source_type.as_deref()) {
                (Some(false), Some("results")) => "runs inside Redash".to_string(),
                (Some(false), Some(kind)) => format!("{kind} · not ClickHouse"),
                _ => "not found in ClickHouse".to_string(),
            };
            cells.push(why, theme.faint());
        }
    }
    cells
}

// -- STALE --------------------------------------------------------------------------------

fn stale_section(app: &App, theme: &Theme, width: usize, columns: &Columns, rows: &[QueueRowRef<'_>], out: &mut Out) {
    if rows.is_empty() {
        return;
    }
    let jobs: Vec<&Job> = rows.iter().map(|(_, j)| *j).collect();
    let columns = &columns.for_list(&jobs);
    let tails: Vec<Cells> = jobs
        .iter()
        .map(|job| match app.cancelling(&job.id) {
            Some(_) => cancel_mark(job, app, theme),
            None => stale_cell(job, theme),
        })
        .collect();
    let tail_w = tails.iter().map(Cells::width).max().unwrap_or(0).clamp(9, 32);
    let query_w = columns.query(width, AGE_LEAD, tail_w);
    let mut detail = format!(" · {} in RQ's started list that no worker runs", jobs.len());
    if let Some(queue) = one_queue(&jobs) {
        detail.push_str(&format!(" · {queue}"));
    }
    section_head("STALE", detail, header_columns(("AGE", AGE_LEAD), columns, query_w, "WHY"), theme, width, &mut out.lines);
    for (job, tail) in jobs.into_iter().zip(tails) {
        let selected = out.is_selected();
        let mut cells = row_start(selected, theme);
        cells.cell_right(&fmt::dur(job.age_s as f64), 7, theme.muted());
        cells.gap(2 + GAP);
        job_cells(&mut cells, job, columns, query_w, true, theme);
        cells.spans(tail.into_spans());
        out.push_job(cells.line(width, row_style(selected, theme)), job, app, theme, width);
    }
    out.lines.push(Line::from(Span::styled(
        "    their worker died, or a cancel never reached one · they hold no worker · a Redash admin can clear them",
        theme.faint(),
    )));
}

/// Why an entry is stale — unless ClickHouse still runs its query, which matters more.
fn stale_cell(job: &Job, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    if let Some((node, _)) = job.clickhouse_target() {
        // Redash gave up on it; ClickHouse did not.
        cells.push("still running on ", theme.sev(Severity::Warn));
        cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
        return cells;
    }
    let why = match job.state {
        JobState::Stale(why) => why,
        _ => Stale::NoWorker,
    };
    cells.push(why.reason(), theme.muted());
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_fits_in_a_few_characters() {
        assert_eq!(short_bytes(21 * 1024 * 1024 * 1024 + 200 * 1024 * 1024), "21.2G");
        assert_eq!(short_bytes(312 * 1024 * 1024), "312M");
        assert_eq!(short_bytes(10), "<1M");
    }

    #[test]
    fn columns_are_as_wide_as_their_content() {
        let mut a = Job::new("a", JobState::Started, "queries");
        a.person = Some("grigol.gankava".into());
        a.data_source = Some("clickhouse-bi".into());
        let mut b = Job::new("b", JobState::Started, "queries");
        b.person = Some("a.b".into());
        let columns = Columns::of(&[&a, &b], 200);
        assert_eq!((columns.who, columns.queue, columns.source), (14, 0, 13), "one queue: no QUEUE column");
        let mut c = Job::new("c", JobState::Started, "scheduled_queries");
        c.person = Some("someone.with.a.very.long.name@partner.example".into());
        let columns = Columns::of(&[&a, &b, &c], 200);
        assert_eq!((columns.who, columns.queue), (22, 17), "capped, and a QUEUE column for two queues");
        assert_eq!(Columns::of(&[&a, &b, &c], 120).queue, 0, "a narrow terminal gives its room to QUERY");
        // QUERY takes the rest, and never less than its minimum.
        assert!(columns.query(200, AGE_LEAD, 30) > columns.query(140, AGE_LEAD, 30));
        assert_eq!(columns.query(60, AGE_LEAD, 30), QUERY_MIN);
    }
}
