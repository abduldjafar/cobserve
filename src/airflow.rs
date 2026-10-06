//! Airflow, view 5: what every DAG did over the last day — what runs now, what waits, what
//! failed, and when each one ran — as Airflow's REST API tells it (`sources/airflow.rs`).
//!
//! Pure, like `model.rs`: the runs as they were read, put into the view's sections and laid on
//! a timeline of the day. Nothing here does I/O; `ui/airflow.rs` draws it.

use crate::severity::Severity;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// How far back the activity reaches: a day.
pub const WINDOW_S: i64 = 24 * 3600;
/// A run that has gone on this long is long (amber); twice the window and it is stuck (red) —
/// a worker died under it, or a sensor waits for what will not come.
pub const LONG_RUN_S: i64 = 6 * 3600;
pub const STUCK_RUN_S: i64 = WINDOW_S;
/// A run queued for longer than this waits on something: `max_active_runs`, a full pool.
pub const LONG_QUEUE_S: i64 = 3600;

/// The activity's error when Airflow is not configured (it is optional, like Redash).
pub const NOT_CONFIGURED: &str = "not configured";
/// Before the first answer.
pub const NOT_READ: &str = "not read yet";

/// A DAG run's state, as Airflow names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunState {
    Queued,
    Running,
    Success,
    Failed,
    /// Anything Airflow adds later; shown, never guessed at.
    Other,
}

impl RunState {
    pub fn parse(state: &str) -> RunState {
        match state {
            "queued" => RunState::Queued,
            "running" => RunState::Running,
            "success" => RunState::Success,
            "failed" => RunState::Failed,
            _ => RunState::Other,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::Success => "success",
            RunState::Failed => "failed",
            RunState::Other => "other",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, RunState::Queued | RunState::Running)
    }

    /// Which state a timeline cell shows when several runs fall in it: a failure is what has to
    /// be seen, then what runs, what waits, and last what went well.
    fn weight(self) -> u8 {
        match self {
            RunState::Failed => 4,
            RunState::Running => 3,
            RunState::Queued => 2,
            RunState::Success => 1,
            RunState::Other => 0,
        }
    }
}

/// One DAG run.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub dag: String,
    pub id: String,
    /// `scheduled`, `manual`, `backfill`, `dataset_triggered`.
    pub kind: String,
    pub state: RunState,
    /// Unix seconds, each as Airflow has it.
    pub logical: Option<i64>,
    pub queued: Option<i64>,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub note: Option<String>,
}

impl Run {
    /// When it happened, for the timeline and the order: when it started, else when it was
    /// queued, else the moment it is for.
    pub fn at(&self) -> Option<i64> {
        self.start.or(self.queued).or(self.logical)
    }

    /// How long it ran, or has been running.
    pub fn took(&self, now: i64) -> Option<i64> {
        let start = self.start?;
        let end = if self.state.is_live() { now } else { self.end? };
        Some((end - start).max(0))
    }

    /// How long a queued run has waited.
    pub fn waited(&self, now: i64) -> Option<i64> {
        (self.state == RunState::Queued).then(|| self.queued.or(self.logical).map(|q| (now - q).max(0))).flatten()
    }

    /// Red for a failure and for a run going on past a day; amber for one past six hours, or a
    /// run queued for more than an hour.
    pub fn severity(&self, now: i64) -> Severity {
        match self.state {
            RunState::Failed => Severity::Crit,
            RunState::Running => match self.took(now) {
                Some(s) if s >= STUCK_RUN_S => Severity::Crit,
                Some(s) if s >= LONG_RUN_S => Severity::Warn,
                _ => Severity::None,
            },
            RunState::Queued if self.waited(now).is_some_and(|s| s >= LONG_QUEUE_S) => Severity::Warn,
            _ => Severity::None,
        }
    }

    /// Stuck: running for more than a day.
    pub fn is_stuck(&self, now: i64) -> bool {
        self.state == RunState::Running && self.took(now).is_some_and(|s| s >= STUCK_RUN_S)
    }

    fn key(&self) -> (String, String) {
        (self.dag.clone(), self.id.clone())
    }
}

/// A task instance that is live: running, queued, waiting for a retry, deferred.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub dag: String,
    pub run: String,
    pub id: String,
    pub state: String,
    pub start: Option<i64>,
    pub try_number: u32,
    pub max_tries: u32,
    pub operator: Option<String>,
    pub host: Option<String>,
}

impl Task {
    pub fn retrying(&self) -> bool {
        self.state == "up_for_retry" || self.state == "up_for_reschedule"
    }
}

/// One task instance of a run, as its page lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaskRun {
    pub id: String,
    /// `-1` for a task that is not mapped; its index otherwise.
    pub map_index: i64,
    pub state: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub try_number: u32,
    pub max_tries: u32,
    pub operator: Option<String>,
    pub host: Option<String>,
}

impl TaskRun {
    /// How long it ran, or has been running.
    pub fn took(&self, now: i64) -> Option<i64> {
        let start = self.start?;
        Some((self.end.unwrap_or(now) - start).max(0))
    }

    /// Red for a failure, amber for a retry to come or a failure upstream.
    pub fn severity(&self) -> Severity {
        match self.state.as_str() {
            "failed" => Severity::Crit,
            "up_for_retry" | "upstream_failed" | "up_for_reschedule" => Severity::Warn,
            "success" => Severity::Ok,
            _ => Severity::None,
        }
    }

    /// Its name, with its index when it is mapped: `load [3]`.
    pub fn label(&self) -> String {
        if self.map_index >= 0 { format!("{} [{}]", self.id, self.map_index) } else { self.id.clone() }
    }
}

/// A DAG as Airflow lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dag {
    pub id: String,
    pub owners: Vec<String>,
    pub paused: bool,
    /// Airflow's words for the schedule: `At 22:00, only on Sunday`.
    pub schedule: Option<String>,
    /// The schedule as written: `0 22 * * 0`, `1 day`.
    pub cron: Option<String>,
    /// When the scheduler makes its next run.
    pub next: Option<i64>,
    pub tags: Vec<String>,
    pub description: Option<String>,
}

/// What a run's tasks have done, read for runs that are running or failed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    pub total: u32,
    /// Succeeded or skipped.
    pub done: u32,
    pub failed: Vec<String>,
    pub running: Vec<String>,
    pub retrying: Vec<String>,
    /// Of `done`, those skipped; and those that did not run because one before them failed.
    pub skipped: u32,
    pub upstream_failed: u32,
}

impl Progress {
    /// Why a failed run failed when none of its tasks did, in a few words: `1 skipped`.
    pub fn without_a_failure(&self) -> String {
        let mut parts = Vec::new();
        if self.upstream_failed > 0 {
            parts.push(format!("{} upstream failed", self.upstream_failed));
        }
        if self.skipped > 0 {
            parts.push(format!("{} skipped", self.skipped));
        }
        if !self.running.is_empty() {
            parts.push(format!("{} still running", self.running.len()));
        }
        let succeeded = self.done.saturating_sub(self.skipped);
        if succeeded > 0 {
            parts.push(format!("{succeeded} succeeded"));
        }
        if parts.is_empty() { "no task failed".to_string() } else { format!("no task failed · {}", parts.join(", ")) }
    }
}

/// A component's heartbeat from `/api/v1/health`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Beat {
    /// `healthy`, `unhealthy`; `None` where the component does not run.
    pub status: Option<String>,
    pub at: Option<i64>,
}

impl Beat {
    pub fn severity(&self) -> Severity {
        match self.status.as_deref() {
            Some("healthy") => Severity::Ok,
            Some(_) => Severity::Crit,
            None => Severity::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Health {
    pub metadatabase: Option<String>,
    pub scheduler: Beat,
    pub triggerer: Beat,
    pub dag_processor: Beat,
}

/// Everything one read of Airflow found.
#[derive(Debug, Clone)]
pub struct Activity {
    /// false → nothing has been read (yet); the view says why.
    pub reachable: bool,
    /// Why it could not be read — with `reachable` still true when what is shown is the last
    /// read that worked.
    pub error: Option<String>,
    /// `https://airflow.example.net`, for the links.
    pub base_url: Option<String>,
    pub version: Option<String>,
    pub health: Health,
    /// Every active DAG, paused or not.
    pub dags: Vec<Dag>,
    pub import_errors: Option<u32>,
    /// What runs and waits now, and every run of the last day.
    pub runs: Vec<Run>,
    /// The live task instances.
    pub tasks: Vec<Task>,
    /// By (dag, run): the tasks of runs that are running or failed.
    pub progress: HashMap<(String, String), Progress>,
    /// The runs of the day are in. The first read comes in two: what runs now at once, the day
    /// a few seconds after it.
    pub day_read: bool,
    pub taken_at: SystemTime,
}

/// A row of the view, by what it is about — the cursor stays on it while the lists move.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Running(String, String),
    Queued(String, String),
    Failed(String, String),
    Dag(String),
}

impl RowKey {
    pub fn dag(&self) -> &str {
        match self {
            RowKey::Running(dag, _) | RowKey::Queued(dag, _) | RowKey::Failed(dag, _) | RowKey::Dag(dag) => dag,
        }
    }

    pub fn run(&self) -> Option<&str> {
        match self {
            RowKey::Running(_, run) | RowKey::Queued(_, run) | RowKey::Failed(_, run) => Some(run),
            RowKey::Dag(_) => None,
        }
    }
}

/// One DAG's day.
#[derive(Debug, Clone)]
pub struct DagLine<'a> {
    pub id: &'a str,
    pub dag: Option<&'a Dag>,
    /// Its runs of the day, oldest first.
    pub runs: Vec<&'a Run>,
    pub ok: u32,
    pub failed: u32,
    pub live: u32,
    /// The latest run, and the latest that has finished.
    pub last: Option<&'a Run>,
    pub last_done: Option<&'a Run>,
}

impl DagLine<'_> {
    /// The worst of its day: a failure, a stuck run.
    pub fn severity(&self, now: i64) -> Severity {
        self.runs.iter().map(|r| r.severity(now)).max().unwrap_or(Severity::None)
    }
}

/// The view's lists, in the order they are drawn.
#[derive(Debug, Clone, Default)]
pub struct Sections<'a> {
    /// Stuck ones first, then the newest.
    pub running: Vec<&'a Run>,
    /// The longest waiting first.
    pub queued: Vec<&'a Run>,
    /// Runs of the day that failed, the latest first.
    pub failed: Vec<&'a Run>,
    /// Every DAG that ran in the day, the latest to run first.
    pub dags: Vec<DagLine<'a>>,
}

impl Sections<'_> {
    /// Every row's key, in the order drawn: what the cursor walks and a click lands on.
    pub fn keys(&self) -> Vec<RowKey> {
        let run = |r: &&Run| (r.dag.clone(), r.id.clone());
        let mut keys: Vec<RowKey> = Vec::new();
        keys.extend(self.running.iter().map(run).map(|(d, r)| RowKey::Running(d, r)));
        keys.extend(self.queued.iter().map(run).map(|(d, r)| RowKey::Queued(d, r)));
        keys.extend(self.failed.iter().map(run).map(|(d, r)| RowKey::Failed(d, r)));
        keys.extend(self.dags.iter().map(|l| RowKey::Dag(l.id.to_string())));
        keys
    }
}

/// The day in numbers, for the line under the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub runs: u32,
    pub ok: u32,
    pub failed: u32,
    pub running: u32,
    pub queued: u32,
    pub retrying: u32,
    pub dags: u32,
    pub paused: u32,
    pub ran: u32,
}

impl Activity {
    pub fn unreachable(error: impl Into<String>) -> Activity {
        Activity {
            reachable: false,
            error: Some(error.into()),
            base_url: None,
            version: None,
            health: Health::default(),
            dags: Vec::new(),
            import_errors: None,
            runs: Vec::new(),
            tasks: Vec::new(),
            progress: HashMap::new(),
            day_read: false,
            taken_at: SystemTime::now(),
        }
    }

    /// Not an outage: not configured, or not read yet.
    pub fn is_placeholder(&self) -> bool {
        !self.reachable && matches!(self.error.as_deref(), Some(NOT_CONFIGURED) | Some(NOT_READ))
    }

    pub fn host(&self) -> Option<String> {
        self.base_url.as_deref().map(crate::sources::host_of)
    }

    pub fn age(&self, now: SystemTime) -> Option<Duration> {
        now.duration_since(self.taken_at).ok()
    }

    pub fn dag(&self, id: &str) -> Option<&Dag> {
        self.dags.iter().find(|d| d.id == id)
    }

    pub fn run(&self, dag: &str, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.dag == dag && r.id == id)
    }

    pub fn progress_of(&self, run: &Run) -> Option<&Progress> {
        self.progress.get(&run.key())
    }

    /// The live tasks of one run.
    pub fn tasks_of(&self, run: &Run) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.dag == run.dag && t.run == run.id).collect()
    }

    /// The runs of the day: what started (or was queued) or ended in the last day, and whatever
    /// is still live however old it is — a run stuck since spring is still running.
    fn of_the_day(&self, now: i64) -> impl Iterator<Item = &Run> {
        let since = now - WINDOW_S;
        self.runs
            .iter()
            .filter(move |r| r.state.is_live() || r.at().is_some_and(|at| at >= since) || r.end.is_some_and(|end| end >= since))
    }

    pub fn sections(&self, now: i64) -> Sections<'_> {
        let mut running: Vec<&Run> = self.runs.iter().filter(|r| r.state == RunState::Running).collect();
        running.sort_by(|a, b| {
            b.is_stuck(now)
                .cmp(&a.is_stuck(now))
                .then_with(|| b.start.cmp(&a.start))
                .then_with(|| a.dag.cmp(&b.dag))
        });
        let mut queued: Vec<&Run> = self.runs.iter().filter(|r| r.state == RunState::Queued).collect();
        queued.sort_by(|a, b| b.waited(now).cmp(&a.waited(now)).then_with(|| a.dag.cmp(&b.dag)));
        // Before the day is in, what runs and waits is all there is to show — and to walk.
        if !self.day_read {
            return Sections { running, queued, ..Sections::default() };
        }
        let mut failed: Vec<&Run> = self
            .of_the_day(now)
            .filter(|r| r.state == RunState::Failed)
            .collect();
        failed.sort_by(|a, b| b.end.or(b.at()).cmp(&a.end.or(a.at())).then_with(|| a.dag.cmp(&b.dag)));

        let mut by_dag: HashMap<&str, Vec<&Run>> = HashMap::new();
        for run in self.of_the_day(now) {
            by_dag.entry(run.dag.as_str()).or_default().push(run);
        }
        let mut dags: Vec<DagLine<'_>> = by_dag
            .into_iter()
            .map(|(id, mut runs)| {
                runs.sort_by_key(|r| r.at());
                let count = |state: RunState| runs.iter().filter(|r| r.state == state).count() as u32;
                DagLine {
                    id,
                    dag: self.dag(id),
                    ok: count(RunState::Success),
                    failed: count(RunState::Failed),
                    live: runs.iter().filter(|r| r.state.is_live()).count() as u32,
                    last: runs.last().copied(),
                    last_done: runs.iter().rev().find(|r| !r.state.is_live()).copied(),
                    runs,
                }
            })
            .collect();
        dags.sort_by(|a, b| {
            let at = |l: &DagLine<'_>| l.last.and_then(Run::at);
            at(b).cmp(&at(a)).then_with(|| a.id.cmp(b.id))
        });
        Sections { running, queued, failed, dags }
    }

    pub fn counts(&self, now: i64) -> Counts {
        let day: Vec<&Run> = self.of_the_day(now).collect();
        let count = |state: RunState| day.iter().filter(|r| r.state == state).count() as u32;
        let mut dags_ran: Vec<&str> = day.iter().map(|r| r.dag.as_str()).collect();
        dags_ran.sort_unstable();
        dags_ran.dedup();
        Counts {
            runs: day.len() as u32,
            ok: count(RunState::Success),
            failed: count(RunState::Failed),
            running: count(RunState::Running),
            queued: count(RunState::Queued),
            retrying: self.tasks.iter().filter(|t| t.retrying()).count() as u32,
            dags: self.dags.len() as u32,
            paused: self.dags.iter().filter(|d| d.paused).count() as u32,
            ran: dags_ran.len() as u32,
        }
    }

    /// The DAG's page in Airflow: its grid, on the run when there is one.
    pub fn link(&self, key: &RowKey) -> Option<String> {
        let base = self.base_url.as_deref()?.trim_end_matches('/');
        let dag = crate::sources::segment(key.dag());
        Some(match key.run() {
            Some(run) => format!("{base}/dags/{dag}/grid?dag_run_id={}", crate::sources::segment(run)),
            None => format!("{base}/dags/{dag}/grid"),
        })
    }
}

/// A moment in the fewest words that leave no doubt: `06:00` within the day, `Jan 16` within
/// the year, `Apr 2024` before.
pub fn when(at: i64, clock: &crate::clock::Clock, now: i64) -> String {
    match (now - at).abs() {
        d if d < 20 * 3600 => clock.format(at, "%H:%M"),
        d if d < 300 * 86_400 => clock.format(at, "%b %-d"),
        _ => clock.format(at, "%b %Y"),
    }
}

/// A run in a few words, by when it ran — not its logical date, which for a daily DAG is the
/// day before: `scheduled · 06:00`, `manual · Jan 16`, `scheduled · Apr 2024`.
pub fn run_label(run: &Run, clock: &crate::clock::Clock, now: i64) -> String {
    let when = run.at().map(|at| when(at, clock, now)).unwrap_or_else(|| "—".to_string());
    let kind = match run.kind.as_str() {
        "dataset_triggered" => "dataset",
        other => other,
    };
    format!("{kind} · {when}")
}

/// Airflow's words for a schedule, in the few a column has: `At 03:00` → `03:00`, `Every 30
/// minutes` → `every 30m`, `At 22:00, only on Sunday` → `Sun 22:00`.
pub fn short_schedule(dag: Option<&Dag>) -> String {
    let Some(dag) = dag else {
        return "—".to_string();
    };
    let Some(text) = dag.schedule.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return dag.cron.clone().unwrap_or_else(|| "—".to_string());
    };
    if text.starts_with("Never") {
        return "triggered".to_string();
    }
    if text == "Every hour" {
        return "hourly".to_string();
    }
    if text == "Every minute" {
        return "every 1m".to_string();
    }
    if let Some(rest) = text.strip_prefix("Every ") {
        if let Some(n) = rest.strip_suffix(" minutes") {
            return format!("every {n}m");
        }
        if let Some(n) = rest.strip_suffix(" hours") {
            return format!("every {n}h");
        }
    }
    if let Some(rest) = text.strip_prefix("At ")
        && let Some(minutes) = rest.strip_suffix(" minutes past the hour")
    {
        // `At 20 minutes past the hour`, `At 15 and 45 minutes past the hour`
        let marks: Vec<String> = minutes.split(" and ").map(|m| format!(":{:0>2}", m.trim())).collect();
        return if marks.len() == 1 { format!("{} hourly", marks[0]) } else { marks.join(" & ") };
    }
    // `At 15 minutes past the hour, every 2 hours`
    if let Some(rest) = text.strip_prefix("At ")
        && let Some((minutes, every)) = rest.split_once(" minutes past the hour, every ")
        && let Some(hours) = every.strip_suffix(" hours")
    {
        return format!(":{minutes:0>2} every {hours}h");
    }
    if let Some(rest) = text.strip_prefix("At ") {
        // `At 03:00` · `At 22:00, only on Sunday` · `At 02:00, on day 1 of the month`
        let (time, tail) = rest.split_once(", ").unwrap_or((rest, ""));
        if let Some(day) = tail.strip_prefix("only on ") {
            let day: String = day.chars().take(3).collect();
            return format!("{day} {time}");
        }
        if let Some(day) = tail.strip_prefix("on day ").and_then(|d| d.strip_suffix(" of the month")) {
            return format!("{time} day {day}");
        }
        if tail.is_empty() {
            return time.to_string();
        }
    }
    text.to_string()
}

// -- the timeline ---------------------------------------------------------------------------

/// What one cell of a DAG's day shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Empty,
    /// A run began in it.
    Start(RunState),
    /// A run that began earlier was still going.
    Through(RunState),
}

/// The day cut into cells, each starting on the hour (or half or quarter hour) of the clock on
/// screen, the last one holding now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axis {
    pub start: i64,
    pub slot_s: i64,
    pub slots: usize,
}

/// How many cells a day takes, the finest that fits in `room` cells.
pub fn slots_for(room: usize) -> usize {
    [96, 72, 48, 24, 12].into_iter().find(|&n| n <= room).unwrap_or(0)
}

impl Axis {
    /// `slots` cells over the last day, the last one holding `now`; `offset_s` is the clock's
    /// distance east of UTC, so the cells begin where its hours do.
    pub fn new(now: i64, slots: usize, offset_s: i64) -> Axis {
        let slots = slots.max(1);
        let slot_s = (WINDOW_S / slots as i64).max(60);
        let end = ((now + offset_s).div_euclid(slot_s) + 1) * slot_s - offset_s;
        Axis { start: end - slot_s * slots as i64, slot_s, slots }
    }

    /// The cell `at` falls in; `None` before the first.
    pub fn index(&self, at: i64) -> Option<usize> {
        if at < self.start {
            return None;
        }
        Some((((at - self.start) / self.slot_s) as usize).min(self.slots - 1))
    }

    /// The hours to write over the cells: `(cell, "06")` where a cell begins on one, every few
    /// hours so the labels keep six cells apart.
    pub fn labels(&self, offset_s: i64) -> Vec<(usize, String)> {
        let per_hour = (3600 / self.slot_s).max(1) as usize;
        let every = [1, 2, 3, 4, 6, 12, 24].into_iter().find(|h| h * per_hour >= 6).unwrap_or(24) as i64;
        (0..self.slots)
            .filter_map(|i| {
                let local = self.start + i as i64 * self.slot_s + offset_s;
                let hour = local.div_euclid(3600).rem_euclid(24);
                (local.rem_euclid(3600) == 0 && hour % every == 0).then(|| (i, format!("{hour:02}")))
            })
            .collect()
    }
}

/// A DAG's day, one cell per slot: where each run began, and where it was still going. In a
/// cell several runs share, a failure shows over a run, a run over a wait, a wait over a success.
pub fn timeline(runs: &[&Run], axis: &Axis, now: i64) -> Vec<Cell> {
    let mut cells = vec![Cell::Empty; axis.slots];
    let weight = |cell: Cell| match cell {
        Cell::Empty => (0, 0),
        Cell::Through(s) => (1, s.weight()),
        Cell::Start(s) => (2, s.weight()),
    };
    for run in runs {
        let Some(at) = run.at() else {
            continue;
        };
        let end = if run.state.is_live() { now } else { run.end.unwrap_or(at) };
        let first = axis.index(at);
        let last = axis.index(end.max(at));
        let (from, to) = match (first, last) {
            (Some(f), Some(l)) => (f, l),
            // Began before the day, still going in it.
            (None, Some(l)) => (0, l),
            _ => continue,
        };
        for (i, cell) in cells.iter_mut().enumerate().take(to + 1).skip(from) {
            let here = if Some(i) == first { Cell::Start(run.state) } else { Cell::Through(run.state) };
            if weight(here) > weight(*cell) {
                *cell = here;
            }
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_103_927; // 2026-10-04 08:52:07 UTC, 15:52 in Jakarta

    fn run(dag: &str, id: &str, state: RunState, start: Option<i64>, end: Option<i64>) -> Run {
        Run {
            dag: dag.into(),
            id: id.into(),
            kind: "scheduled".into(),
            state,
            logical: start,
            queued: start,
            start,
            end,
            note: None,
        }
    }

    fn activity(runs: Vec<Run>) -> Activity {
        let mut a = Activity::unreachable("x");
        a.reachable = true;
        a.error = None;
        a.day_read = true;
        a.runs = runs;
        a
    }

    #[test]
    fn a_run_is_judged_by_how_it_ended_and_how_long_it_runs() {
        assert_eq!(run("a", "1", RunState::Failed, Some(NOW - 60), Some(NOW)).severity(NOW), Severity::Crit);
        assert_eq!(run("a", "1", RunState::Success, Some(NOW - 60), Some(NOW)).severity(NOW), Severity::None);
        assert_eq!(run("a", "1", RunState::Running, Some(NOW - 600), None).severity(NOW), Severity::None);
        assert_eq!(run("a", "1", RunState::Running, Some(NOW - 7 * 3600), None).severity(NOW), Severity::Warn);
        let stuck = run("a", "1", RunState::Running, Some(NOW - 262 * 86_400), None);
        assert_eq!(stuck.severity(NOW), Severity::Crit);
        assert!(stuck.is_stuck(NOW));
        let mut waiting = run("a", "1", RunState::Queued, None, None);
        waiting.queued = Some(NOW - 2 * 3600);
        assert_eq!(waiting.severity(NOW), Severity::Warn);
        assert_eq!(waiting.waited(NOW), Some(7200));
    }

    #[test]
    fn the_sections_put_what_matters_first() {
        let a = activity(vec![
            run("fresh", "r1", RunState::Running, Some(NOW - 60), None),
            run("stuck", "r0", RunState::Running, Some(NOW - 262 * 86_400), None),
            run("etl", "f1", RunState::Failed, Some(NOW - 9 * 3600), Some(NOW - 8 * 3600)),
            run("etl", "s1", RunState::Success, Some(NOW - 3 * 3600), Some(NOW - 2 * 3600)),
            run("old", "x", RunState::Failed, Some(NOW - 30 * 3600), Some(NOW - 29 * 3600)),
            run("hourly", "h1", RunState::Success, Some(NOW - 600), Some(NOW - 500)),
        ]);
        let s = a.sections(NOW);
        let ids = |runs: &[&Run]| runs.iter().map(|r| r.dag.to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&s.running), ["stuck", "fresh"], "a stuck run first");
        assert_eq!(ids(&s.failed), ["etl"], "a failure of yesterday's yesterday is not today's");
        let dags: Vec<&str> = s.dags.iter().map(|l| l.id).collect();
        assert_eq!(dags, ["fresh", "hourly", "etl", "stuck"], "the latest to run first; a live run counts whenever it began");
        let etl = s.dags.iter().find(|l| l.id == "etl").unwrap();
        assert_eq!((etl.ok, etl.failed, etl.live), (1, 1, 0));
        assert_eq!(etl.last_done.map(|r| r.id.as_str()), Some("s1"));
        assert_eq!(etl.severity(NOW), Severity::Crit);

        let keys = s.keys();
        assert_eq!(keys[0], RowKey::Running("stuck".into(), "r0".into()));
        assert_eq!(keys[2], RowKey::Failed("etl".into(), "f1".into()));
        assert_eq!(keys.last(), Some(&RowKey::Dag("stuck".into())));

        let counts = a.counts(NOW);
        assert_eq!((counts.runs, counts.ok, counts.failed, counts.running, counts.ran), (5, 2, 1, 2, 4));

        // The first read, before the day is in: only what runs and waits, to show and to walk.
        let mut first = a.clone();
        first.day_read = false;
        let s = first.sections(NOW);
        assert_eq!((s.running.len(), s.failed.len(), s.dags.len()), (2, 0, 0));
        assert_eq!(s.keys().len(), 2);
    }

    #[test]
    fn the_cells_begin_on_the_hours_of_the_clock() {
        // Jakarta is seven hours east: the last cell is 15:00–16:00 there.
        let axis = Axis::new(NOW, 24, 7 * 3600);
        assert_eq!(axis.slot_s, 3600);
        assert_eq!((axis.start + 7 * 3600).rem_euclid(3600), 0, "on the hour");
        assert_eq!(axis.index(NOW), Some(23));
        assert_eq!(axis.index(NOW - 3600), Some(22));
        assert_eq!(axis.index(NOW - 25 * 3600), None);
        let labels = axis.labels(7 * 3600);
        assert_eq!(labels.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["18", "00", "06", "12"]);
        // 16:00 yesterday is cell 0, so 18:00 is cell 2.
        assert_eq!(labels[0].0, 2);
        // Half-hour cells say every third hour.
        let fine = Axis::new(NOW, 48, 7 * 3600);
        assert_eq!(fine.slot_s, 1800);
        assert_eq!(fine.labels(7 * 3600).len(), 8);
        assert_eq!(slots_for(30), 24);
        assert_eq!(slots_for(50), 48);
        assert_eq!(slots_for(5), 0);
    }

    #[test]
    fn a_failure_shows_over_a_success_and_a_long_run_stretches() {
        let axis = Axis::new(NOW, 24, 0);
        let ok = run("d", "ok", RunState::Success, Some(NOW - 600), Some(NOW - 500));
        let bad = run("d", "bad", RunState::Failed, Some(NOW - 300), Some(NOW - 200));
        let long = run("d", "long", RunState::Success, Some(NOW - 5 * 3600), Some(NOW - 2 * 3600));
        let live = run("d", "live", RunState::Running, Some(NOW - 30 * 3600), None);
        let cells = timeline(&[&ok, &bad, &long], &axis, NOW);
        assert_eq!(cells[23], Cell::Start(RunState::Failed), "the failure, not the success beside it");
        let start = axis.index(NOW - 5 * 3600).unwrap();
        assert_eq!(cells[start], Cell::Start(RunState::Success));
        assert_eq!(cells[start + 1], Cell::Through(RunState::Success));
        assert_eq!(cells[start + 3], Cell::Through(RunState::Success));
        assert_eq!(cells[start + 4], Cell::Empty);
        let cells = timeline(&[&live], &axis, NOW);
        assert!(cells.iter().all(|c| *c == Cell::Through(RunState::Running)), "begun before the day, going all through it: {cells:?}");
    }

    #[test]
    fn schedules_are_said_in_a_few_characters() {
        let dag = |schedule: &str| Dag { schedule: Some(schedule.into()), ..Dag::default() };
        let short = |s: &str| short_schedule(Some(&dag(s)));
        assert_eq!(short("At 03:00"), "03:00");
        assert_eq!(short("Every 30 minutes"), "every 30m");
        assert_eq!(short("Every hour"), "hourly");
        assert_eq!(short("At 20 minutes past the hour"), ":20 hourly");
        assert_eq!(short("At 15 minutes past the hour, every 2 hours"), ":15 every 2h");
        assert_eq!(short("At 15 and 45 minutes past the hour"), ":15 & :45");
        assert_eq!(short("At 22:00, only on Sunday"), "Sun 22:00");
        assert_eq!(short("At 02:00, on day 1 of the month"), "02:00 day 1");
        assert_eq!(short("Never, external triggers only"), "triggered");
        assert_eq!(short("Something new"), "Something new");
        assert_eq!(short_schedule(None), "—");
    }

    #[test]
    fn a_moment_says_its_year_when_it_is_old() {
        let clock = crate::clock::Clock { shown: crate::clock::Shown::Utc, ..crate::clock::Clock::default() };
        assert_eq!(when(NOW - 3600, &clock, NOW), "07:52");
        assert_eq!(when(NOW - 262 * 86_400, &clock, NOW), "Jan 15");
        assert_eq!(when(NOW - 893 * 86_400, &clock, NOW), "Apr 2024");
        let p = Progress { total: 1, done: 1, skipped: 1, ..Progress::default() };
        assert_eq!(p.without_a_failure(), "no task failed · 1 skipped");
    }

    #[test]
    fn links_go_to_the_dag_s_grid_and_its_run() {
        let mut a = activity(Vec::new());
        a.base_url = Some("https://airflow.example.net/".into());
        assert_eq!(a.link(&RowKey::Dag("etl".into())).as_deref(), Some("https://airflow.example.net/dags/etl/grid"));
        assert_eq!(
            a.link(&RowKey::Failed("etl".into(), "scheduled__2026-10-04T06:00:00+00:00".into())).as_deref(),
            Some("https://airflow.example.net/dags/etl/grid?dag_run_id=scheduled__2026-10-04T06%3A00%3A00%2B00%3A00")
        );
    }
}
