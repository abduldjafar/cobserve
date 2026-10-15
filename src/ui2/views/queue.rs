//! REDASH QUEUE zoomed (`docs/redesign/09-zooms.md` §1): *why is the queue full, and who holds
//! it?* Three cards answer it at a glance — the jobs waiting in big digits, a worker board per
//! queue, the waiting line of the last four minutes on its fixed scale — then the jobs as one
//! list: RUNNING with a spinner, a live age and the stitch into ClickHouse drawn as a progress
//! bar with its memory and cores; WAITING with each wait drawn as a bar to the red line at three
//! minutes; STALE last. The job under the cursor opens in a card at the bottom with its facts and
//! its SQL (`J K` scroll), so the list never moves when the cursor does.

use super::kit::{self, Card};
use super::widgets::{code_block, dots, thin_bar, Cells};
use crate::app::{scroll_into_view, App, Hit, QueueRowRef, SqlFrom};
use crate::fmt;
use crate::history::WINDOW_S;
use crate::model::{Job, JobState, QueueRow, QueueStatus, Stale};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use crate::ui2::glyph::{big, graph};
use crate::ui2::panels::queue::{runaway_holders, GRAPH_MAX};
use crate::ui2::paint::{lp, Painter};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::Frame;
use std::time::SystemTime;

/// The queue Redash runs people's queries on; its row is shown even when it is idle.
const MAIN_QUEUE: &str = "queries";
/// The red line of a wait (§7): what the WAITING bars run to.
const RED_LINE_S: f64 = 180.0;
/// The gutter, the mark, the age and the flag in front of every job.
const LEAD: usize = 14;
/// The narrowest QUERY gets before the line is cut at its end instead.
const QUERY_MIN: usize = 14;
/// From this width the cards stand over the list; below it, two lines.
const CARDS_FROM: u16 = 100;

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

/// A job's age as of now: what Redash said at the last read, plus the time since — up to a
/// minute, past which the freshness says the numbers are old instead.
pub fn live_age(job: &Job, queue: &QueueStatus, clock: SystemTime) -> u64 {
    live(job.age_s, queue, clock)
}

/// What a running job's ClickHouse query costs right now (§5.4).
pub struct Stitch {
    pub memory_bytes: u64,
    pub limit: u64,
    pub cores: f64,
    pub progress: Option<f64>,
    pub runaway: bool,
}

pub fn stitch(app: &App, job: &Job) -> Option<Stitch> {
    let (node, query_id) = job.clickhouse_target()?;
    app.with_view(|view| {
        view.nodes
            .iter()
            .filter(|n| n.node.name == node)
            .flat_map(|n| n.users.iter())
            .flat_map(|u| u.queries.iter())
            .find(|q| q.query.query_id == query_id)
            .map(|q| Stitch { memory_bytes: q.query.memory_bytes, limit: q.limit, cores: q.cores, progress: q.progress, runaway: q.runaway })
    })
    .flatten()
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, i: Rect) {
    if i.height == 0 || i.width < 10 {
        return;
    }
    let mut p = Painter::new(frame.buffer_mut());
    let clip = Rect::new(i.x.saturating_sub(1), i.y, i.width + 2, i.height);
    p.within(clip, |p| body(p, app, theme, i));
}

fn body(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let q = &app.queue;
    if !q.reachable {
        app.viewport.sql_max.set(0);
        unreachable(p, app, theme, i);
        return;
    }
    let mut y = i.y;
    if i.width >= CARDS_FROM && i.height >= 26 {
        hero(p, app, theme, Rect::new(i.x, y, i.width, 6));
        y += 6;
    } else {
        y = compact_hero(p, app, theme, i);
    }
    let card_h = match i.height {
        40.. => 12,
        30..40 => 9,
        20..30 => 6,
        _ => 0,
    };
    let list_h = (i.bottom().saturating_sub(card_h)).saturating_sub(y);
    list(p, app, theme, Rect::new(i.x, y, i.width, list_h));
    if card_h > 0 {
        detail(p, app, theme, Rect::new(i.x, i.bottom() - card_h, i.width, card_h));
    } else {
        app.viewport.sql_max.set(0);
    }
}

fn unreachable(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) {
    let warn = (("✖"), theme.sev(Severity::Warn).add_modifier(Modifier::BOLD));
    let calm = ("◌", theme.muted());
    match app.queue.error.as_deref() {
        Some(crate::model::QUEUE_NOT_CONFIGURED) => kit::message(
            p,
            theme,
            i,
            calm,
            "Redash is not configured",
            &["the queue needs REDASH_URL and REDASH_ADMIN_API_KEY (and REDIS_URL, for the names of waiting jobs)", ":setup redash · or redash: url, api_key in the --credential file"],
        ),
        Some(crate::model::QUEUE_NOT_POLLED) => kit::message(p, theme, i, calm, "connecting to Redash…", &["the first read is on its way"]),
        other => {
            // A Redash that answered is not unreachable: it refused the key, failed, or sent
            // something that is not the queue.
            let reason = other.unwrap_or("no answer");
            let (word, detail) = if let Some(detail) = reason.strip_prefix(crate::sources::redash::UNREADABLE) {
                ("unreadable", detail)
            } else if reason.starts_with("HTTP 401") || reason.starts_with("HTTP 403") {
                ("refused", reason)
            } else if reason.starts_with("HTTP ") {
                ("error", reason)
            } else {
                ("unreachable", reason)
            };
            kit::message(p, theme, i, warn, &format!("Redash {word} ({detail})"), &["the next read is in a few seconds · :setup redash checks the key"]);
        }
    }
}

// -- the cards ----------------------------------------------------------------------------

/// Seconds Redash reported, as of now: what it said, plus the time since the read (a minute
/// at most, as `live_age`).
fn live(seconds: u64, q: &QueueStatus, clock: SystemTime) -> u64 {
    seconds + clock.duration_since(q.taken_at).map_or(0, |d| d.as_secs()).min(60)
}

/// The oldest wait across the queues, when Redis can tell it, as of now.
fn oldest(q: &QueueStatus, clock: SystemTime) -> Option<u64> {
    q.queues.iter().filter(|r| r.waiting > 0).filter_map(|r| r.oldest_wait_s).max().map(|s| live(s, q, clock))
}

fn waiting_severity(q: &QueueStatus, clock: SystemTime) -> Severity {
    q.queues
        .iter()
        .filter(|r| r.waiting > 0)
        .map(|r| severity::wait(live(r.oldest_wait_s.unwrap_or(0), q, clock), r.saturated()))
        .max()
        .unwrap_or(Severity::None)
}

/// The queues worth a row: every one with something going on, and the main one always.
fn shown_queues(q: &QueueStatus) -> (Vec<&QueueRow>, Vec<&str>) {
    let shown = q.queues.iter().filter(|r| !r.is_idle() || r.name == MAIN_QUEUE).collect();
    let idle = q.queues.iter().filter(|r| r.is_idle() && r.name != MAIN_QUEUE).map(|r| r.name.as_str()).collect();
    (shown, idle)
}

/// `●●●● 4/4 busy`, `○ 1 idle`, `no worker` — amber when jobs wait behind a full queue.
fn workers(busy: u32, total: u32, waiting: u32, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    if total == 0 {
        let style = if waiting > 0 { theme.sev(Severity::Crit).add_modifier(Modifier::BOLD) } else { theme.muted() };
        cells.push("no worker", style);
        return cells;
    }
    let sev = if busy >= total && waiting > 0 { Severity::Warn } else { Severity::None };
    let slots = dots(busy, total, theme.bar_fill(sev), theme);
    let drawn = !slots.is_empty();
    cells.spans(slots);
    let text = if busy == 0 { format!("{total} idle") } else { format!("{busy}/{total} busy") };
    cells.push(format!("{}{text}", if drawn { " " } else { "" }), if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
    cells
}

fn hero(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let q = &app.queue;
    // The board needs about sixty cells; a wider screen gives the rest to the graphs — the
    // oldest wait gets its own from 152 cells.
    let wait_w = 28u16;
    let queues_w = r.width.saturating_sub(wait_w + 30).min(64);
    let rest = r.width.saturating_sub(wait_w + queues_w);
    let (graph_w, oldest_w) = if rest >= 60 { (rest - rest / 2, rest / 2) } else { (rest, 0) };

    // The jobs waiting, in big digits, the oldest against the red line, who Redis could name.
    let sev = waiting_severity(q, app.clock);
    let waiting = q.total_waiting();
    let inner = Card::new(theme.border()).title(vec![Span::styled("waiting", theme.muted())]).draw(p, Rect::new(r.x, r.y, wait_w, r.height));
    let digits = if waiting > 0 && sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.strong() };
    let after = big::draw(p, inner.x, inner.y, &waiting.to_string(), digits) + 1;
    let room = inner.right().saturating_sub(after) as usize;
    p.put(after, inner.y, &fmt::truncate("jobs waiting", room), theme.muted());
    let (old_text, old_style) = match (waiting, oldest(q, app.clock)) {
        (0, _) => ("nothing waits".to_string(), theme.sev(Severity::Ok)),
        (_, Some(s)) => {
            let sev = severity::wait(s, false);
            (format!("oldest {}{}", fmt::dur(s as f64), sev.mark()), if sev.is_problem() { theme.sev(sev) } else { theme.text() })
        }
        (_, None) => ("oldest —".to_string(), theme.muted()),
    };
    p.put(after, inner.y + 1, &fmt::truncate(&old_text, room), old_style);
    if q.names_available && waiting > 0 {
        let named = q.jobs.iter().filter(|j| j.state == JobState::Queued).count();
        p.put(after, inner.y + 2, &fmt::truncate(&format!("{named} named of {waiting}"), room), theme.faint());
    }
    let (holding, started) = runaway_holders(app);
    if started > 0 {
        let (text, style) = if holding > 0 {
            (format!("{holding} of {started} hold a runaway !"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD))
        } else {
            (format!("0 of {started} hold a runaway"), theme.muted())
        };
        p.put(inner.x, inner.y + 3, &fmt::truncate(&text, inner.width as usize), style);
    }

    // A worker board per queue.
    if queues_w >= 30 {
        let fresh = kit::freshness(q.age(app.clock), None, false, theme, 20);
        let inner = Card::new(theme.border())
            .title(vec![Span::styled("queues", theme.muted())])
            .right(fresh.into_spans())
            .draw(p, Rect::new(r.x + wait_w, r.y, queues_w, r.height));
        queue_board(p, theme, q, app.clock, inner);
    }

    // The waiting line over the window, on the panel's fixed scale and thresholds.
    let minutes = (WINDOW_S / 60.0).round() as u64;
    let gx = r.x + wait_w + queues_w;
    let g = Graph { title: format!("waiting · {minutes} min"), ticks: ["100", "50", "0"], max: GRAPH_MAX, warn: 30.0, crit: 60.0, legend: ("30", "60") };
    g.draw(p, app, theme, &app.history.queue_waiting, Rect::new(gx, r.y, graph_w, r.height));
    if oldest_w > 0 {
        let g = Graph { title: format!("oldest wait · {minutes} min"), ticks: ["5m", "2.5m", "0"], max: 300.0, warn: 60.0, crit: RED_LINE_S, legend: ("1m", "3m") };
        g.draw(p, app, theme, &app.history.queue_oldest, Rect::new(gx + graph_w, r.y, oldest_w, r.height));
    }
}

/// A hero card's graph: a fixed scale with its ticks, coloured by its two thresholds, which
/// the legend under it prints.
struct Graph {
    title: String,
    ticks: [&'static str; 3],
    max: f64,
    warn: f64,
    crit: f64,
    legend: (&'static str, &'static str),
}

impl Graph {
    fn draw(&self, p: &mut Painter<'_>, app: &App, theme: &Theme, series: &crate::history::Series, r: Rect) {
        let inner = Card::new(theme.border()).title(vec![Span::styled(self.title.clone(), theme.muted())]).draw(p, r);
        let gh = inner.height.saturating_sub(1).max(1);
        let tick_w = self.ticks.iter().map(|t| fmt::width(t)).max().unwrap_or(1) as u16;
        for (k, label) in self.ticks.iter().enumerate() {
            let row = inner.y + ((k as f64 * f64::from(gh.saturating_sub(1))) / 2.0).round() as u16;
            p.put(inner.x, row, &format!("{} ┤", lp(label, tick_w as usize)), theme.muted());
        }
        let gx = inner.x + tick_w + 2;
        let gw = inner.right().saturating_sub(gx);
        let tier = app.caps.graph;
        let per = graph::per_column(tier);
        let now = crate::history::secs(app.queue.taken_at);
        let samples = graph::stretch(series.buckets(now, WINDOW_S, gw as usize * per));
        let (warn, crit) = (self.warn, self.crit);
        graph::area(p, tier, Rect::new(gx, inner.y, gw, gh), &samples, self.max, &|v| {
            if v >= crit {
                theme.sev(Severity::Crit)
            } else if v >= warn {
                theme.sev(Severity::Warn)
            } else {
                theme.accent()
            }
        });
        let foot = inner.y + gh;
        p.put(gx, foot, &crate::ui2::panels::fleet::window_label(), theme.muted());
        kit::put_spans_r(
            p,
            inner.right(),
            foot,
            vec![
                Span::styled("▲", theme.sev(Severity::Warn)),
                Span::styled(format!(" {} · ", self.legend.0), theme.faint()),
                Span::styled("✖", theme.sev(Severity::Crit)),
                Span::styled(format!(" {}  now", self.legend.1), theme.faint()),
            ],
        );
    }
}

/// `QUEUE  RUN  WAIT  OLDEST  WORKERS  STALE` and a row a queue that has something going on,
/// the idle ones folded into a line.
fn queue_board(p: &mut Painter<'_>, theme: &Theme, q: &QueueStatus, clock: SystemTime, r: Rect) {
    let (shown, idle) = shown_queues(q);
    if shown.is_empty() && idle.is_empty() {
        p.put(r.x, r.y, "Redash reports no queues", theme.muted());
        return;
    }
    let name_w = shown.iter().map(|row| fmt::width(&row.name)).max().unwrap_or(5).clamp(5, 18);
    let w = r.width as usize;
    // Columns drop whole from the right of what matters: STALE, then OLDEST.
    let oldest_on = q.names_available && w >= name_w + 1 + 3 + 2 + 4 + 2 + 8 + 14;
    let stale_on = w >= name_w + 1 + 3 + 2 + 4 + 2 + if oldest_on { 8 } else { 0 } + 14 + 6;
    let mut head = Cells::new();
    head.cell("QUEUE", name_w, theme.section());
    head.gap(1);
    head.cell_right("RUN", 3, theme.section());
    head.gap(2);
    head.cell_right("WAIT", 4, theme.section());
    head.gap(2);
    if oldest_on {
        head.cell("OLDEST", 8, theme.section());
    }
    head.cell("WORKERS", 14, theme.section());
    if stale_on {
        head.cell_right("STALE", 6, theme.section());
    }
    kit::put_cells(p, r.x, r.y, head, w);
    let rows_room = r.height.saturating_sub(1 + u16::from(!idle.is_empty())) as usize;
    for (k, row) in shown.iter().take(rows_room.max(1)).enumerate() {
        let y = r.y + 1 + k as u16;
        if y >= r.bottom() {
            break;
        }
        let wait = live(row.oldest_wait_s.unwrap_or(0), q, clock);
        let mut cells = Cells::new();
        cells.cell(&row.name, name_w, theme.strong());
        cells.gap(1);
        cells.cell_right(&row.running.to_string(), 3, if row.running > 0 { theme.strong() } else { theme.muted() });
        cells.gap(2);
        let count_sev = severity::wait(wait, row.saturated());
        cells.cell_right(&row.waiting.to_string(), 4, if row.waiting > 0 { theme.sev(count_sev).add_modifier(Modifier::BOLD) } else { theme.muted() });
        cells.gap(2);
        if oldest_on {
            let sev = severity::wait(wait, false);
            let text = match row.oldest_wait_s {
                Some(_) if row.waiting > 0 => format!("{}{}", fmt::dur(wait as f64), sev.mark()),
                _ => String::new(),
            };
            cells.cell(&text, 8, theme.sev(sev));
        }
        let board = workers(row.workers_busy, row.workers_total, row.waiting, theme);
        let used = board.width();
        cells.spans(board.into_spans());
        cells.gap(14usize.saturating_sub(used));
        if stale_on {
            if row.stale > 0 {
                cells.cell_right(&row.stale.to_string(), 6, theme.muted());
            } else {
                cells.cell_right("·", 6, theme.faint());
            }
        }
        kit::put_cells(p, r.x, y, cells, w);
    }
    if !idle.is_empty() {
        let y = (r.y + 1 + shown.len().min(rows_room.max(1)) as u16).min(r.bottom() - 1);
        let mut cells = Cells::new();
        cells.push(idle.join(" · "), theme.text2());
        cells.push("  idle", theme.faint());
        kit::put_cells(p, r.x, y, cells, w);
    }
}

/// Under 100 columns or 26 rows: the cards as two lines (one under 8 rows); returns the row
/// after them.
fn compact_hero(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect) -> u16 {
    let q = &app.queue;
    let sev = waiting_severity(q, app.clock);
    let waiting = q.total_waiting();
    let mut cells = Cells::new();
    cells.push(format!("{waiting} waiting"), if waiting > 0 && sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.strong() });
    if let Some(s) = oldest(q, app.clock).filter(|_| waiting > 0) {
        let sev = severity::wait(s, false);
        cells.push(" · ", theme.faint());
        cells.push(format!("oldest {}{}", fmt::dur(s as f64), sev.mark()), if sev.is_problem() { theme.sev(sev) } else { theme.text() });
    }
    cells.push("  ", Style::default());
    cells.spans(workers(q.workers_busy, q.workers_total, waiting, theme).into_spans());
    if q.total_stale() > 0 {
        cells.push(format!(" · {} stale", q.total_stale()), theme.muted());
    }
    let (holding, started) = runaway_holders(app);
    if holding > 0 {
        cells.push(format!(" · {holding} of {started} !"), theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
    }
    let fresh = kit::freshness(q.age(app.clock), None, false, theme, 20);
    if cells.width() + fresh.width() + 2 <= i.width as usize {
        kit::put_spans_r(p, i.right(), i.y, fresh.into_spans());
    }
    kit::put_cells(p, i.x, i.y, cells, i.width as usize);
    if i.height < 8 {
        return i.y + 1;
    }
    let (shown, _) = shown_queues(q);
    let mut chips = Cells::new();
    for row in shown.iter().filter(|r| !r.is_idle()) {
        if chips.width() > 0 {
            chips.push("   ", Style::default());
        }
        chips.push(format!("{} ", row.name), theme.text2());
        chips.spans(dots(row.workers_busy, row.workers_total, theme.bar_fill(Severity::None), theme));
        chips.push(format!(" {}/{} · {} ◌", row.workers_busy, row.workers_total, row.waiting), theme.muted());
        if let Some(s) = row.oldest_wait_s.filter(|_| row.waiting > 0).map(|s| live(s, q, app.clock)) {
            let sev = severity::wait(s, false);
            chips.push(format!(" · {}{}", fmt::dur(s as f64), sev.mark()), theme.sev(sev));
        }
    }
    if chips.width() == 0 {
        chips.push("every queue is idle", theme.faint());
    }
    kit::put_cells(p, i.x, i.y + 1, chips, i.width as usize);
    i.y + 2
}

// -- the list -----------------------------------------------------------------------------

enum Item {
    Blank,
    Rule(Vec<Span<'static>>),
    Line(Cells),
    Job(usize, Cells),
}

/// The list's columns at a width: WHO and the tail by what the screen holds, QUERY the rest.
struct Columns {
    who: usize,
    query: usize,
    tail: usize,
    wide: bool,
}

impl Columns {
    fn of(jobs: &[&Job], width: usize) -> Columns {
        let wide = width >= CARDS_FROM as usize;
        let who = if width < 60 { 0 } else { jobs.iter().map(|j| fmt::width(&j.label())).max().unwrap_or(6).clamp(6, 16) };
        let tail = if wide { 44 } else { 22 };
        let used = LEAD + if who > 0 { who + 2 } else { 0 } + 2 + tail;
        Columns { who, query: width.saturating_sub(used).max(QUERY_MIN), tail, wide }
    }
}

fn row_start(selected: bool, mark: &str, mark_style: Style, age: u64, age_style: Style, flag: (&str, Style), theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    cells.push(if selected { "▌" } else { " " }, theme.accent());
    cells.push(" ", Style::default());
    cells.push(mark.to_string(), mark_style);
    cells.push(" ", Style::default());
    cells.cell_right(&fmt::dur(age as f64), 6, age_style);
    cells.push(" ", Style::default());
    cells.cell(flag.0, 1, flag.1);
    cells.push("  ", Style::default());
    cells
}

fn who_and_query(cells: &mut Cells, job: &Job, c: &Columns, dim: bool, theme: &Theme) {
    let quiet = |s: Style| if dim { theme.muted() } else { s };
    if c.who > 0 {
        cells.cell(&job.label(), c.who, if job.person.is_some() { quiet(theme.person()) } else { theme.faint() });
        cells.gap(2);
    }
    let known = job.redash_query_id.is_some() || job.query_name.is_some();
    cells.cell(&job.query_label(), c.query, if known { quiet(theme.text()) } else { theme.muted() });
    cells.gap(2);
}

fn head(lead: &str, c: &Columns, tail: Vec<(&str, usize, bool)>, theme: &Theme, width: usize) -> Item {
    let mut cells = Cells::new();
    cells.push(format!("    {}    ", lp(lead, 6)), theme.section());
    if c.who > 0 {
        cells.cell("WHO", c.who + 2, theme.section());
    }
    cells.cell("QUERY", c.query + 2, theme.section());
    for (text, w, right) in tail {
        if right {
            cells.cell_right(text, w, theme.section());
        } else {
            cells.cell(text, w, theme.section());
        }
    }
    let line = cells.line(width, theme.table_head());
    let mut out = Cells::new();
    out.spans(line.spans.into_iter().map(|s| Span::styled(s.content, theme.table_head().patch(s.style))));
    Item::Line(out)
}

/// `⊘ cancelling… · ` on a job Redash has been asked to cancel.
fn cancel_mark(app: &App, job: &Job, theme: &Theme) -> Cells {
    let mut cells = Cells::new();
    match app.cancelling(&job.id) {
        Some(c) if c.accepted => cells.push("⊘ cancelled · ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        Some(_) => cells.push("⊘ cancelling… · ", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
        None => &mut cells,
    };
    cells
}

/// Where a running job is in ClickHouse and what it costs there right now — or why it is not
/// there at all.
fn running_tail(app: &App, job: &Job, c: &Columns, theme: &Theme) -> Cells {
    let mut cells = cancel_mark(app, job, theme);
    let Some((node, _)) = job.clickhouse_target() else {
        let why = match (job.on_clickhouse(), job.data_source_type.as_deref()) {
            (Some(false), Some("results")) => "runs inside Redash".to_string(),
            (Some(false), Some(kind)) => format!("{kind} · not ClickHouse"),
            _ => "not found in ClickHouse".to_string(),
        };
        let text = match job.data_source.as_deref().filter(|_| c.wide) {
            Some(source) => format!("{source} · {why}"),
            None => why,
        };
        cells.push(text, theme.faint());
        return cells;
    };
    let cost = stitch(app, job);
    let mut place = Cells::new();
    place.push("→ ", theme.faint());
    place.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
    let node_w = if c.wide { 17 } else { 16 };
    let used = place.width().min(node_w);
    cells.spans(super::widgets::fit(place.into_spans(), node_w, false));
    cells.gap(node_w - used);
    let progress = cost.as_ref().and_then(|s| s.progress);
    let pct = progress.map_or("—".to_string(), |p| fmt::pct0(p * 100.0));
    if c.wide {
        match progress {
            Some(share) => cells.spans(thin_bar(Some(share * 100.0), 8, theme.bar_fill(Severity::None), theme)),
            None => cells.push("        ", Style::default()),
        };
        cells.push(" ", Style::default());
        cells.cell_right(&pct, 4, if progress.is_some() { theme.accent() } else { theme.faint() });
        if let Some(s) = &cost {
            cells.gap(2);
            let sev = if s.limit > 0 { severity::limit_share(s.memory_bytes as f64 / s.limit as f64) } else { Severity::None };
            cells.cell_right(&short_bytes(s.memory_bytes), 5, if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
            cells.gap(2);
            cells.cell_right(&format!("{:.1}c", s.cores), 4, theme.text2());
        }
    } else {
        cells.cell_right(&pct, 4, if progress.is_some() { theme.accent() } else { theme.faint() });
    }
    cells
}

fn waiting_tail(app: &App, job: &Job, age: u64, sev: Severity, c: &Columns, theme: &Theme) -> Cells {
    let mut cells = cancel_mark(app, job, theme);
    let share = (age as f64 / RED_LINE_S * 100.0).min(100.0);
    let bar_w = if c.wide { 12 } else { 8 };
    cells.spans(thin_bar(Some(share), bar_w, theme.bar_fill(sev), theme));
    let label = if c.wide { format!("{:>4} of 3 min", fmt::pct0(share)) } else { format!(" {} of 3m", fmt::pct0(share)) };
    cells.push(format!("  {label}"), if sev.is_problem() { theme.sev(sev) } else { theme.muted() });
    if c.wide {
        cells.gap(2);
        cells.push(job.data_source.clone().unwrap_or_else(|| "—".into()), theme.faint());
    }
    cells
}

/// Why an entry is stale — unless ClickHouse still runs its query, which matters more.
fn stale_tail(app: &App, job: &Job, theme: &Theme) -> Cells {
    let mut cells = cancel_mark(app, job, theme);
    if let Some((node, _)) = job.clickhouse_target() {
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

/// The queue every job of a list is on, when there is just one.
fn one_queue<'a>(rows: &[QueueRowRef<'a>]) -> Option<&'a str> {
    let first = rows.first()?.1.queue.as_str();
    rows.iter().all(|(_, j)| j.queue == first).then_some(first)
}

fn items(app: &App, theme: &Theme, width: usize) -> Vec<Item> {
    let q = &app.queue;
    let sections = app.queue_sections();
    let everyone: Vec<&Job> = sections.running.iter().chain(&sections.waiting).chain(&sections.stale).map(|(_, j)| *j).collect();
    let c = Columns::of(&everyone, width);
    let selected = app.queue_selection();
    let spin = kit::spinner(app);
    let mut out = Vec::new();
    let mut index = 0usize;

    // RUNNING, always: an empty one says so.
    let mut detail = format!(" · {} on a worker", sections.running.len());
    if let Some(name) = one_queue(&sections.running) {
        detail.push_str(&format!(" · {name}"));
    }
    out.push(Item::Rule(vec![Span::styled("RUNNING", theme.section()), Span::styled(detail, theme.muted())]));
    if c.wide && !sections.running.is_empty() {
        out.push(head("FOR", &c, vec![("IN CLICKHOUSE", 17, false), ("progress", 9, false), ("done", 4, true), ("mem", 7, true), ("cores", 6, true)], theme, width));
    }
    if sections.running.is_empty() {
        let mut cells = Cells::new();
        cells.push("    nothing on a worker", theme.muted());
        out.push(Item::Line(cells));
    }
    for (_, job) in &sections.running {
        let runaway = job.clickhouse_target().is_some_and(|(node, id)| app.query_is_runaway(node, id));
        let age = live_age(job, q, app.clock);
        // A running job inherits the runaway state of its ClickHouse query (§2.8).
        let sev = if runaway { Severity::Crit } else { severity::elapsed(age as f64) };
        let flag = match app.cancelling(&job.id) {
            Some(_) => ("⊘", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
            None if runaway => ("!", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD)),
            None => ("", Style::default()),
        };
        let mut cells = row_start(selected == Some(index), spin, theme.accent(), age, theme.sev(sev).add_modifier(Modifier::BOLD), flag, theme);
        who_and_query(&mut cells, job, &c, false, theme);
        cells.spans(running_tail(app, job, &c, theme).into_spans());
        out.push(Item::Job(index, cells));
        index += 1;
    }

    // WAITING, when anything waits.
    let total = q.total_waiting();
    if total > 0 || !sections.waiting.is_empty() {
        out.push(Item::Blank);
        let mut detail = format!(" · {total}");
        if let Some(name) = one_queue(&sections.waiting) {
            detail.push_str(&format!(" in {name}"));
        } else if !sections.waiting.is_empty() {
            detail.push_str(" in the queues");
        }
        if q.names_available && (sections.waiting.len() as u32) < total {
            detail.push_str(&format!(" · {} named", sections.waiting.len()));
        }
        out.push(Item::Rule(vec![Span::styled("WAITING", theme.section()), Span::styled(detail, theme.muted())]));
        if c.wide && !sections.waiting.is_empty() {
            out.push(head("WAIT", &c, vec![("TO THE RED LINE · 3 min", 28, false), ("SOURCE", 14, false)], theme, width));
        }
        if sections.waiting.is_empty() {
            let mut cells = Cells::new();
            let text = if q.names_available {
                "    Redis has not named them yet"
            } else {
                "    who is waiting needs Redis: REDIS_URL, or redis_url under redash: in the credential file — Redash's API only counts them"
            };
            cells.push(text, theme.muted());
            out.push(Item::Line(cells));
        }
        for (_, job) in &sections.waiting {
            let saturated = q.queue(&job.queue).is_some_and(QueueRow::saturated);
            let age = live_age(job, q, app.clock);
            let sev = severity::wait(age, saturated);
            let flag = match app.cancelling(&job.id) {
                Some(_) => ("⊘", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
                None => (sev.mark().trim(), theme.sev(sev)),
            };
            let mut cells = row_start(selected == Some(index), "◌", theme.muted(), age, theme.sev(sev).add_modifier(Modifier::BOLD), flag, theme);
            who_and_query(&mut cells, job, &c, false, theme);
            cells.spans(waiting_tail(app, job, age, sev, &c, theme).into_spans());
            out.push(Item::Job(index, cells));
            index += 1;
        }
    }

    // STALE, when RQ holds any.
    if !sections.stale.is_empty() {
        out.push(Item::Blank);
        let mut detail = format!(" · {} in RQ's started list that no worker runs", sections.stale.len());
        if let Some(name) = one_queue(&sections.stale) {
            detail.push_str(&format!(" · {name}"));
        }
        out.push(Item::Rule(vec![Span::styled("STALE", theme.section()), Span::styled(detail, theme.muted())]));
        for (_, job) in &sections.stale {
            let flag = match app.cancelling(&job.id) {
                Some(_) => ("⊘", theme.sev(Severity::Warn).add_modifier(Modifier::BOLD)),
                None => ("", Style::default()),
            };
            let mut cells = row_start(selected == Some(index), "·", theme.faint(), job.age_s, theme.muted(), flag, theme);
            who_and_query(&mut cells, job, &c, true, theme);
            cells.spans(stale_tail(app, job, theme).into_spans());
            out.push(Item::Job(index, cells));
            index += 1;
        }
        let mut cells = Cells::new();
        cells.push("    their worker died, or a cancel never reached one · they hold no worker · a Redash admin can clear them", theme.faint());
        out.push(Item::Line(cells));
    }
    let _ = c.tail;
    out
}

fn list(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    if r.height == 0 {
        return;
    }
    let width = r.width as usize;
    let items = items(app, theme, width);
    let selected = app.queue_selection();
    let line_of = items.iter().position(|it| matches!(it, Item::Job(i, _) if Some(*i) == selected));
    let height = r.height as usize;
    let offset = scroll_into_view(app.viewport.queue.get(), line_of, height, items.len());
    app.viewport.queue.set(offset);
    let mut hits = app.viewport.hits.borrow_mut();
    for (k, item) in items.into_iter().skip(offset).take(height).enumerate() {
        let y = r.y + k as u16;
        match item {
            Item::Blank => {}
            Item::Rule(title) => kit::rule(p, theme, r.x, y, r.width, title),
            Item::Line(cells) => {
                kit::put_cells(p, r.x, y, cells, width);
            }
            Item::Job(index, cells) => {
                if Some(index) == selected {
                    kit::band(p, theme, r, y);
                }
                kit::put_cells(p, r.x, y, cells, width);
                hits.push((Rect::new(r.x, y, r.width, 1), Hit::Job(index)));
            }
        }
    }
}

// -- the job under the cursor ---------------------------------------------------------------

fn detail(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect) {
    let explanation = app.queue_explanation();
    let Some(job) = app.selected_job() else {
        app.viewport.sql_max.set(0);
        let inner = Card::new(theme.border()).title(vec![Span::styled("queue", theme.strong())]).draw(p, r);
        kit::put_spans(p, inner.x, inner.y, vec![Span::styled(explanation, theme.muted())], inner.width as usize);
        return;
    };
    let sql = app.job_sql(job);
    let source = match &sql {
        Some((_, SqlFrom::ClickHouse)) => "SQL as ClickHouse runs it",
        Some((_, SqlFrom::Redash)) => "SQL as saved in Redash",
        None if job.adhoc => "Redash keeps no text for ad-hoc queries",
        None => "no SQL to show",
    };
    let mut right = vec![Span::styled(source, theme.muted())];
    let title = vec![
        Span::styled(job.query_label(), theme.strong()),
        Span::styled(" · ", theme.faint()),
        Span::styled(job.label(), theme.person()),
        Span::styled(format!(" · {} · job {}", job.queue, job.id), theme.muted()),
    ];
    let room = r.height.saturating_sub(4) as usize;
    let block = sql.as_ref().map(|(text, _)| {
        let scroll = app.sql_scroll_for(&App::job_sql_key(job));
        code_block(text, "", scroll, room, r.width.saturating_sub(4) as usize, theme)
    });
    match &block {
        Some((_, max)) if *max > 0 => {
            right.push(Span::styled(" · J K scroll", theme.faint()));
            app.viewport.sql_max.set(*max);
        }
        Some((_, max)) => app.viewport.sql_max.set(*max),
        None => app.viewport.sql_max.set(0),
    }
    let inner = Card::new(theme.border()).title(title).right(right).draw(p, r);
    let w = inner.width as usize;
    kit::put_cells(p, inner.x, inner.y, state_line(app, job, theme), w);
    if inner.height > 1 {
        kit::put_spans(p, inner.x, inner.y + 1, vec![Span::styled(explanation, theme.muted())], w);
    }
    if let Some((lines, _)) = block {
        for (k, line) in lines.into_iter().enumerate() {
            let y = inner.y + 2 + k as u16;
            if y >= inner.bottom() {
                break;
            }
            kit::put_line(p, inner.x, y, line, w);
        }
    }
}

/// What the job is doing, as of now, and what can be done about it from here.
fn state_line(app: &App, job: &Job, theme: &Theme) -> Cells {
    let q = &app.queue;
    let age = fmt::dur(live_age(job, q, app.clock) as f64);
    let mut cells = Cells::new();
    match (job.state, job.clickhouse_target()) {
        // A waiting job has not reached ClickHouse yet: there is nothing to kill, and the wait
        // is the queue (§2.8).
        (JobState::Queued, _) => {
            let ahead = q.waiting(&job.queue).iter().filter(|other| other.age_s > job.age_s).count();
            cells.push(format!("waiting {age}"), theme.text());
            cells.push(format!(" · {}", fmt::plural(ahead, "job ahead of it", "jobs ahead of it")), theme.sev(Severity::Warn));
            cells.push(" · not in ClickHouse yet: x takes it out of the queue", theme.muted());
        }
        (JobState::Started, Some((node, query_id))) => {
            cells.push(format!("running {age} → "), theme.text());
            cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
            cells.push(format!(" · {query_id}"), theme.muted());
            if let Some(s) = stitch(app, job) {
                if s.runaway {
                    cells.push(" ! runaway", theme.sev(Severity::Crit).add_modifier(Modifier::BOLD));
                }
                let sev = if s.limit > 0 { severity::limit_share(s.memory_bytes as f64 / s.limit as f64) } else { Severity::None };
                cells.push(" · ", theme.faint());
                cells.push(format!("{} of {} limit", short_bytes(s.memory_bytes), short_bytes(s.limit)), if sev.is_problem() { theme.sev(sev) } else { theme.text2() });
                cells.push(format!(" · {:.1} cores", s.cores), theme.text2());
                if let Some(pr) = s.progress {
                    cells.push(format!(" · {} done", fmt::pct0(pr * 100.0)), theme.accent());
                }
            }
            cells.push(" · ⏎ the node · x cancels the job", theme.faint());
        }
        (JobState::Started, None) => {
            cells.push(format!("running {age}"), theme.text());
            let why = match (job.on_clickhouse(), job.data_source_type.as_deref()) {
                (Some(false), Some("results")) => " · Query Results runs inside Redash, on the worker: nothing in ClickHouse to jump to".to_string(),
                (Some(false), kind) => format!(" · on {}, not ClickHouse: this monitor does not see it run", kind.unwrap_or("another database")),
                _ => " · no ClickHouse query carries its job id or its Redash number".to_string(),
            };
            cells.push(why, theme.muted());
        }
        (JobState::Stale(why), target) => {
            cells.push(format!("in RQ's started list for {}", fmt::dur(job.age_s as f64)), theme.text());
            let reason = match why {
                Stale::Cancelled => " · cancelled, and no worker was left to stop it",
                Stale::NoWorker => " · no live worker holds it: the one that ran it died or restarted",
                Stale::OverADay => " · started over a day ago and never let go of",
            };
            cells.push(reason, theme.muted());
            if let Some((node, _)) = target {
                cells.push(" · ClickHouse still runs it on ", theme.sev(Severity::Warn));
                cells.push(node.to_string(), theme.accent().add_modifier(Modifier::BOLD));
                cells.push(" · ⏎ to see it", theme.faint());
            }
        }
    }
    match app.cancelling(&job.id) {
        Some(c) if c.accepted => cells.push(" · ⊘ cancelled in Redash — its worker lets go at its next check", theme.sev(Severity::Warn)),
        Some(_) => cells.push(" · ⊘ asking Redash to cancel it…", theme.sev(Severity::Warn)),
        None => &mut cells,
    };
    if let Some(source) = job.data_source.as_deref().filter(|_| job.state != JobState::Started || job.clickhouse_target().is_none()) {
        cells.push(format!(" · on {source}"), theme.faint());
    }
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
    fn a_job_s_age_ticks_for_a_minute_after_the_read_and_no_longer() {
        let mut queue = QueueStatus::unreachable("test");
        queue.taken_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        let mut job = Job::new("a", JobState::Started, "queries");
        job.age_s = 100;
        assert_eq!(live_age(&job, &queue, queue.taken_at + std::time::Duration::from_secs(5)), 105);
        assert_eq!(live_age(&job, &queue, queue.taken_at + std::time::Duration::from_secs(600)), 160);
        assert_eq!(live_age(&job, &queue, queue.taken_at - std::time::Duration::from_secs(5)), 100, "a clock behind the read adds nothing");
    }

    #[test]
    fn query_takes_what_the_other_columns_leave() {
        let mut a = Job::new("a", JobState::Started, "queries");
        a.person = Some("grigol.gankava".into());
        let wide = Columns::of(&[&a], 116);
        assert_eq!((wide.who, wide.tail), (14, 44));
        assert_eq!(wide.query, 116 - LEAD - 16 - 2 - 44);
        let narrow = Columns::of(&[&a], 76);
        assert_eq!(narrow.tail, 22);
        assert_eq!(Columns::of(&[&a], 50).who, 0, "a very narrow zoom drops WHO");
        assert_eq!(Columns::of(&[&a], 40).query, QUERY_MIN);
    }
}
