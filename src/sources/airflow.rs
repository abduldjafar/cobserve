//! Airflow's stable REST API (`/api/v1`, Airflow 2): the DAG runs of the last day, the task
//! instances that are live, the DAGs, and the health of the scheduler and the triggerer.
//!
//! The API is signed into the way the web UI is. An Airflow whose API takes only the session
//! of its login form (the default `auth_backends`) answers Basic auth with 403, so the source
//! asks for the form, sends it back with its CSRF token, and keeps the session cookie it is
//! given — and signs in again when the session runs out. A login that is refused is not tried
//! again for five minutes, or until `r`: a wrong password must not lock the account by being
//! sent every fifteen seconds. Everything after the login is GET.
//!
//! What runs and waits is read every 15 s; the runs of the day every minute, and at once when
//! a run has finished; the DAGs every five minutes.

use super::{segment, transport, unix};
use crate::airflow::{Activity, Beat, Dag, Health, Progress, Run, RunState, Task, TaskRun, WINDOW_S};
use crate::app::Event;
use crate::config::AirflowConfig;
use reqwest::header::{HeaderMap, ACCEPT, COOKIE, LOCATION, REFERER, SET_COOKIE};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

const LIVE_EVERY: Duration = Duration::from_secs(15);
const DAY_EVERY: Duration = Duration::from_secs(60);
const DAGS_EVERY: Duration = Duration::from_secs(300);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_RETRY: Duration = Duration::from_secs(300);
/// Airflow's own most per page (`maximum_page_limit`).
const PAGE: usize = 100;
/// Three thousand runs a day; a busier Airflow shows the latest three thousand.
const MAX_PAGES: usize = 30;
/// The states of a task instance that is live.
const LIVE_TASK_STATES: [&str; 6] = ["running", "queued", "up_for_retry", "up_for_reschedule", "deferred", "restarting"];
/// Running runs whose tasks are counted on every read; past this many, the rest go without.
const PROGRESS_MAX: usize = 20;
/// Requests in flight at once when there are many to make: the pages of a day, the tasks of a
/// dozen runs. Enough to make a first read take seconds, not twenty; few enough for a webserver.
const AT_ONCE: usize = 6;

// -- what the API answers -------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RunList {
    #[serde(default)]
    dag_runs: Vec<ApiRun>,
    #[serde(default)]
    total_entries: usize,
}

#[derive(Debug, Deserialize)]
struct ApiRun {
    dag_id: String,
    dag_run_id: Option<String>,
    run_type: Option<String>,
    state: Option<String>,
    logical_date: Option<String>,
    execution_date: Option<String>,
    queued_at: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskList {
    #[serde(default)]
    task_instances: Vec<ApiTask>,
    #[serde(default)]
    total_entries: usize,
}

#[derive(Debug, Deserialize)]
struct ApiTask {
    dag_id: String,
    dag_run_id: Option<String>,
    task_id: String,
    state: Option<String>,
    start_date: Option<String>,
    try_number: Option<u32>,
    max_tries: Option<u32>,
    operator: Option<String>,
    hostname: Option<String>,
    end_date: Option<String>,
    map_index: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DagList {
    #[serde(default)]
    dags: Vec<ApiDag>,
    #[serde(default)]
    total_entries: usize,
}

#[derive(Debug, Deserialize)]
struct ApiDag {
    dag_id: String,
    #[serde(default)]
    owners: Option<Vec<String>>,
    is_paused: Option<bool>,
    timetable_description: Option<String>,
    schedule_interval: Option<serde_json::Value>,
    next_dagrun_create_after: Option<String>,
    #[serde(default)]
    tags: Option<Vec<ApiTag>>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiTag {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct ApiHealth {
    metadatabase: Option<ApiBeat>,
    scheduler: Option<ApiBeat>,
    triggerer: Option<ApiBeat>,
    dag_processor: Option<ApiBeat>,
}

#[derive(Debug, Default, Deserialize)]
struct ApiBeat {
    status: Option<String>,
    #[serde(alias = "latest_scheduler_heartbeat", alias = "latest_triggerer_heartbeat", alias = "latest_dag_processor_heartbeat")]
    heartbeat: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Version {
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Count {
    #[serde(default)]
    total_entries: u32,
}

/// What one call can go wrong with — never with the password (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Http(u16),
    Connection(String),
    Body(String),
    /// The login form sent back did not sign in.
    LoginRefused,
    /// Nothing at `/login/` looks like Airflow's form.
    NoLoginForm,
    /// Signed in, and still refused: the login's role may not read DAGs.
    NoPermission,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(404) => write!(f, "HTTP 404 · no Airflow API there — is the url Airflow's own?"),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Connection(e) => write!(f, "{e}"),
            Error::Body(e) => write!(f, "{}{e}", super::redash::UNREADABLE),
            Error::LoginRefused => write!(f, "login refused — check the Airflow user and password (asked again in 5 min, or now with r)"),
            Error::NoLoginForm => write!(f, "no login form at /login/ — is the url Airflow's own?"),
            Error::NoPermission => write!(f, "HTTP 403 · signed in, but this login may not read the DAGs"),
        }
    }
}

fn why_unreadable(body: &str, error: &serde_json::Error) -> String {
    if body.trim_start().starts_with('<') {
        return "a web page, not JSON — is the url Airflow's own?".to_string();
    }
    error.to_string().chars().take(120).collect()
}

// -- the login --------------------------------------------------------------------------------

/// The cookies a server set, sent back with every request — just enough of a cookie jar for one
/// host: a name's latest value wins, an emptied one is dropped.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Jar(Vec<(String, String)>);

impl Jar {
    fn take(&mut self, headers: &HeaderMap) {
        for value in headers.get_all(SET_COOKIE) {
            let Some((name, value)) = value.to_str().ok().and_then(cookie_pair) else {
                continue;
            };
            self.0.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.0.push((name.to_string(), value.to_string()));
            }
        }
    }

    fn header(&self) -> String {
        self.0.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>().join("; ")
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `session=abc; Expires=…; HttpOnly; Path=/` → `("session", "abc")`.
fn cookie_pair(set_cookie: &str) -> Option<(&str, &str)> {
    let first = set_cookie.split(';').next()?;
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    (!name.is_empty()).then_some((name, value.trim().trim_matches('"')))
}

/// The CSRF token of the login form: `<input id="csrf_token" name="csrf_token" type="hidden"
/// value="…">`, its attributes in whatever order.
fn csrf_token(html: &str) -> Option<String> {
    html.split('<')
        .filter(|tag| tag.starts_with("input") && tag.contains("name=\"csrf_token\""))
        .find_map(|tag| {
            let value = tag.split("value=\"").nth(1)?;
            let token = value.split('"').next()?;
            (!token.is_empty()).then(|| token.to_string())
        })
}

// -- from the API's shapes to the model's -------------------------------------------------

fn run(api: ApiRun) -> Option<Run> {
    Some(Run {
        id: api.dag_run_id?,
        dag: api.dag_id,
        kind: api.run_type.unwrap_or_else(|| "scheduled".to_string()),
        state: RunState::parse(api.state.as_deref().unwrap_or("")),
        logical: api.logical_date.or(api.execution_date).as_deref().and_then(unix),
        queued: api.queued_at.as_deref().and_then(unix),
        start: api.start_date.as_deref().and_then(unix),
        end: api.end_date.as_deref().and_then(unix),
        note: api.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
    })
}

fn task(api: ApiTask) -> Option<Task> {
    Some(Task {
        run: api.dag_run_id?,
        dag: api.dag_id,
        id: api.task_id,
        state: api.state.unwrap_or_default(),
        start: api.start_date.as_deref().and_then(unix),
        try_number: api.try_number.unwrap_or(0),
        max_tries: api.max_tries.unwrap_or(0),
        operator: api.operator,
        host: api.hostname.filter(|h| !h.is_empty()),
    })
}

fn task_run(api: ApiTask) -> TaskRun {
    TaskRun {
        id: api.task_id,
        map_index: api.map_index.unwrap_or(-1),
        state: api.state.unwrap_or_else(|| "none".to_string()),
        start: api.start_date.as_deref().and_then(unix),
        end: api.end_date.as_deref().and_then(unix),
        try_number: api.try_number.unwrap_or(0),
        max_tries: api.max_tries.unwrap_or(0),
        operator: api.operator,
        host: api.hostname.filter(|h| !h.is_empty()),
    }
}

/// `{"__type": "CronExpression", "value": "0 22 * * 0"}` → `0 22 * * 0`;
/// `{"__type": "TimeDelta", "days": 0, "seconds": 3600}` → `1h`.
fn cron(schedule: Option<&serde_json::Value>) -> Option<String> {
    let schedule = schedule?;
    match schedule.get("__type").and_then(|t| t.as_str()) {
        Some("CronExpression") => schedule.get("value").and_then(|v| v.as_str()).map(str::to_string),
        Some("TimeDelta") => {
            let days = schedule.get("days").and_then(|d| d.as_i64()).unwrap_or(0);
            let seconds = schedule.get("seconds").and_then(|s| s.as_i64()).unwrap_or(0);
            Some(match (days, seconds) {
                (d, 0) if d > 0 => format!("{d} day{}", if d == 1 { "" } else { "s" }),
                (0, s) if s % 3600 == 0 => format!("{}h", s / 3600),
                (0, s) => format!("{}m", s / 60),
                (d, s) => format!("{d}d {}h", s / 3600),
            })
        }
        _ => schedule.as_str().map(str::to_string),
    }
}

fn dag(api: ApiDag) -> Dag {
    Dag {
        id: api.dag_id,
        owners: api.owners.unwrap_or_default(),
        paused: api.is_paused.unwrap_or(false),
        schedule: api.timetable_description,
        cron: cron(api.schedule_interval.as_ref()),
        next: api.next_dagrun_create_after.as_deref().and_then(unix),
        tags: api.tags.unwrap_or_default().into_iter().map(|t| t.name).collect(),
        description: api.description.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
    }
}

fn beat(api: Option<ApiBeat>) -> Beat {
    let api = api.unwrap_or_default();
    Beat { status: api.status, at: api.heartbeat.as_deref().and_then(unix) }
}

fn health(api: ApiHealth) -> Health {
    Health {
        metadatabase: api.metadatabase.and_then(|m| m.status),
        scheduler: beat(api.scheduler),
        triggerer: beat(api.triggerer),
        dag_processor: beat(api.dag_processor),
    }
}

/// What a run's task instances add up to.
fn progress(tasks: &[ApiTask], total: usize) -> Progress {
    let mut out = Progress { total: total.max(tasks.len()) as u32, ..Progress::default() };
    for task in tasks {
        match task.state.as_deref() {
            Some("success") => out.done += 1,
            Some("skipped") => {
                out.done += 1;
                out.skipped += 1;
            }
            Some("upstream_failed") => out.upstream_failed += 1,
            Some("failed") => out.failed.push(task.task_id.clone()),
            Some("running" | "queued" | "deferred" | "restarting") => out.running.push(task.task_id.clone()),
            Some("up_for_retry" | "up_for_reschedule") => out.retrying.push(task.task_id.clone()),
            _ => {}
        }
    }
    out
}

/// Unix seconds as the API's filters take them: `2026-10-03T08:52:07Z`.
fn rfc3339(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn now_s() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A request's path under `/api/v1` and its query, owned, so it can go to a task of its own.
type Ask = (String, Vec<(String, String)>);

fn ask(path: impl Into<String>, query: &[(&str, &str)]) -> Ask {
    (path.into(), query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

/// One GET of the API with the session's cookies.
async fn fetch<T: DeserializeOwned>(client: &reqwest::Client, url: &str, query: &[(String, String)], cookies: &str) -> Result<T, Error> {
    let mut request = client.get(url).query(query).header(ACCEPT, "application/json");
    if !cookies.is_empty() {
        request = request.header(COOKIE, cookies);
    }
    let response = request.send().await.map_err(|e| Error::Connection(transport(e, REQUEST_TIMEOUT)))?;
    let status = response.status();
    // Sent to the login page: the session is gone.
    if status.is_redirection() {
        return Err(Error::Http(401));
    }
    if !status.is_success() {
        return Err(Error::Http(status.as_u16()));
    }
    let body = response.text().await.map_err(|e| Error::Connection(transport(e, REQUEST_TIMEOUT)))?;
    serde_json::from_str(&body).map_err(|e| Error::Body(why_unreadable(&body, &e)))
}

// -- a page's reads ---------------------------------------------------------------------------

/// What reads a page's detail when it is asked for — a DAG's runs, a run's tasks, a task's log —
/// with the client and the session of the source, apart from the task that polls, so a page does
/// not wait for a poll. The session is the source's: it signs in, this follows.
#[derive(Clone)]
pub struct AirflowDetails {
    base_url: String,
    client: reqwest::Client,
    session: std::sync::Arc<std::sync::Mutex<String>>,
}

/// The most runs a DAG's page lists.
const RUNS_LISTED: &str = "50";

impl AirflowDetails {
    fn cookies(&self) -> Result<String, String> {
        let cookies = self.session.lock().map(|c| c.clone()).unwrap_or_default();
        if cookies.is_empty() {
            return Err("not signed in to Airflow yet — its first read is still on its way".to_string());
        }
        Ok(cookies)
    }

    fn said(error: Error) -> String {
        match error {
            Error::Http(401 | 403) => "the Airflow session ran out — r reads Airflow again and signs in".to_string(),
            other => other.to_string(),
        }
    }

    /// A DAG's latest runs, newest first.
    pub async fn runs(&self, dag: &str) -> Result<Vec<Run>, String> {
        let cookies = self.cookies()?;
        let url = format!("{}/api/v1/dags/{}/dagRuns", self.base_url, segment(dag));
        let query = ask("", &[("order_by", "-execution_date"), ("limit", RUNS_LISTED)]).1;
        let list: RunList = fetch(&self.client, &url, &query, &cookies).await.map_err(Self::said)?;
        Ok(list.dag_runs.into_iter().filter_map(run).collect())
    }

    /// A run's task instances, in the order they started; those that have not, after.
    pub async fn tasks(&self, dag: &str, run: &str) -> Result<Vec<TaskRun>, String> {
        let cookies = self.cookies()?;
        let url = format!("{}/api/v1/dags/{}/dagRuns/{}/taskInstances", self.base_url, segment(dag), segment(run));
        let mut tasks = Vec::new();
        for page in 0..5 {
            let offset = (page * PAGE).to_string();
            let query = ask("", &[("limit", "100"), ("offset", &offset)]).1;
            let list: TaskList = fetch(&self.client, &url, &query, &cookies).await.map_err(Self::said)?;
            let got = list.task_instances.len();
            tasks.extend(list.task_instances.into_iter().map(task_run));
            if got < PAGE || tasks.len() >= list.total_entries {
                break;
            }
        }
        tasks.sort_by(|a, b| {
            a.start
                .is_none()
                .cmp(&b.start.is_none())
                .then(a.start.cmp(&b.start))
                .then_with(|| a.id.cmp(&b.id))
                .then(a.map_index.cmp(&b.map_index))
        });
        Ok(tasks)
    }

    /// One try of a task's log, as text.
    pub async fn log(&self, dag: &str, run: &str, task: &str, map_index: i64, attempt: u32) -> Result<String, String> {
        let cookies = self.cookies()?;
        let url = format!(
            "{}/api/v1/dags/{}/dagRuns/{}/taskInstances/{}/logs/{attempt}",
            self.base_url,
            segment(dag),
            segment(run),
            segment(task)
        );
        let mut query = vec![("full_content".to_string(), "true".to_string())];
        if map_index >= 0 {
            query.push(("map_index".to_string(), map_index.to_string()));
        }
        let response = self
            .client
            .get(&url)
            .query(&query)
            .header(ACCEPT, "text/plain")
            .header(COOKIE, cookies)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| transport(e, Duration::from_secs(60)))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(Self::said(Error::Http(401)));
        }
        if !status.is_success() {
            return Err(Self::said(Error::Http(status.as_u16())));
        }
        response.text().await.map_err(|e| transport(e, Duration::from_secs(60)))
    }
}

// -- the source -------------------------------------------------------------------------------

pub struct AirflowSource {
    base_url: String,
    user: String,
    password: String,
    /// Follows no redirect: the login's answer is a redirect, and its cookie is on it.
    client: reqwest::Client,
    cookies: Jar,
    /// The session's cookies as they are now, for the pages' reads.
    session: std::sync::Arc<std::sync::Mutex<String>>,
    /// When a login was last refused, so it is not sent again for a while.
    refused: Option<Instant>,
    version: Option<String>,
    dags: Vec<Dag>,
    dags_read: Option<Instant>,
    import_errors: Option<u32>,
    day: Vec<Run>,
    day_read: Option<Instant>,
    /// The runs that were live at the last read: one gone means a run has finished.
    live_before: HashSet<(String, String)>,
    /// The failed tasks of failed runs, read once: a finished run does not change.
    failed_tasks: HashMap<(String, String), Progress>,
}

impl AirflowSource {
    /// `None` when Airflow is not configured: the view then says how to.
    pub fn new(config: &AirflowConfig) -> Option<AirflowSource> {
        let base_url = config.url.as_deref()?.trim_end_matches('/').to_string();
        let user = config.user.clone()?;
        let password = config.password.clone()?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(AirflowSource {
            base_url,
            user,
            password,
            client,
            cookies: Jar::default(),
            session: std::sync::Arc::default(),
            refused: None,
            version: None,
            dags: Vec::new(),
            dags_read: None,
            import_errors: None,
            day: Vec::new(),
            day_read: None,
            live_before: HashSet::new(),
            failed_tasks: HashMap::new(),
        })
    }

    /// What reads a page's detail, on the session this source keeps.
    pub fn details(&self) -> AirflowDetails {
        AirflowDetails { base_url: self.base_url.clone(), client: self.client.clone(), session: std::sync::Arc::clone(&self.session) }
    }

    /// Read until the loop ends: every 15 s, and at once when asked — which also asks a refused
    /// login again. Errors are shown, never fatal. The first read comes in two: what runs now,
    /// in a few seconds, then the day, which takes a dozen more.
    pub async fn run(mut self, tx: tokio::sync::mpsc::UnboundedSender<Event>, mut refresh: tokio::sync::mpsc::UnboundedReceiver<()>) {
        let now = self.now_only().await;
        if now.reachable && tx.send(Event::Airflow(Box::new(now))).is_err() {
            return;
        }
        let mut tick = tokio::time::interval(LIVE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let asked = tokio::select! {
                _ = tick.tick() => false,
                Some(()) = refresh.recv() => {
                    while refresh.try_recv().is_ok() {}
                    tick.reset();
                    true
                }
            };
            let activity = self.activity(asked).await;
            if tx.send(Event::Airflow(Box::new(activity))).is_err() {
                return;
            }
        }
    }

    /// One read. `asked`: everything is read again now, a refused login included.
    pub async fn activity(&mut self, asked: bool) -> Activity {
        self.read(asked, true).await
    }

    /// What runs and waits now, and the scheduler's health — the first read's first half.
    pub async fn now_only(&mut self) -> Activity {
        self.read(false, false).await
    }

    /// One read; with `day` false, what runs now and nothing of the day — unless the day was
    /// read before, which is then kept.
    async fn read(&mut self, asked: bool, day: bool) -> Activity {
        if asked {
            self.refused = None;
            self.day_read = None;
            self.dags_read = None;
        }
        let now = now_s();
        // Public: which Airflow, and how its scheduler is.
        if self.version.is_none() {
            self.version = self.get_once::<Version>("/version", &[]).await.ok().and_then(|v| v.version);
        }
        let health = match self.get_once::<ApiHealth>("/health", &[]).await {
            Ok(api) => health(api),
            Err(e) => return self.failed(e),
        };

        // What runs and waits now, and the live tasks.
        let live_query = [("state", "running"), ("state", "queued"), ("order_by", "-start_date"), ("limit", "100")];
        let live: Vec<Run> = match self.get::<RunList>("/dags/~/dagRuns", &live_query).await {
            Ok(list) => list.dag_runs.into_iter().filter_map(run).collect(),
            Err(e) => return self.failed(e),
        };
        let mut task_query: Vec<(&str, &str)> = LIVE_TASK_STATES.iter().map(|s| ("state", *s)).collect();
        task_query.push(("limit", "100"));
        let tasks: Vec<Task> = match self.get::<TaskList>("/dags/~/dagRuns/~/taskInstances", &task_query).await {
            Ok(list) => list.task_instances.into_iter().filter_map(task).collect(),
            Err(e) => return self.failed(e),
        };

        // The day: every minute, and at once when a run that was live has finished.
        let live_now: HashSet<(String, String)> = live.iter().map(|r| (r.dag.clone(), r.id.clone())).collect();
        let finished = self.live_before.iter().any(|key| !live_now.contains(key));
        if day {
            self.live_before = live_now.clone();
        }
        if day && (finished || self.day_read.is_none_or(|t| t.elapsed() >= DAY_EVERY)) {
            match self.read_day(now).await {
                Ok(day) => {
                    self.day = day;
                    self.day_read = Some(Instant::now());
                }
                // The first read of the day is what the view needs; a later one that fails
                // leaves the last one standing.
                Err(e) if self.day_read.is_none() => return self.failed(e),
                Err(_) => {}
            }
        }
        if day && self.dags_read.is_none_or(|t| t.elapsed() >= DAGS_EVERY) {
            match self.read_dags().await {
                Ok(dags) => {
                    self.dags = dags;
                    self.dags_read = Some(Instant::now());
                    self.import_errors = self.get::<Count>("/importErrors", &[("limit", "1")]).await.ok().map(|c| c.total_entries);
                }
                Err(e) if self.dags_read.is_none() => return self.failed(e),
                Err(_) => {}
            }
        }

        // What is live wins over what the day's read had of the same run.
        let mut runs = live;
        runs.extend(self.day.iter().filter(|r| !live_now.contains(&(r.dag.clone(), r.id.clone()))).cloned());

        // How far the running runs are, every read; what the day's failed runs failed at, once.
        let mut progress: HashMap<(String, String), Progress> = HashMap::new();
        let running: Vec<(String, String)> = runs
            .iter()
            .filter(|r| r.state == RunState::Running)
            .take(PROGRESS_MAX)
            .map(|r| (r.dag.clone(), r.id.clone()))
            .collect();
        let since = now - WINDOW_S;
        let failed: Vec<(String, String)> = runs
            .iter()
            .filter(|r| r.state == RunState::Failed && r.end.or(r.at()).is_some_and(|at| at >= since))
            .map(|r| (r.dag.clone(), r.id.clone()))
            .collect();
        self.failed_tasks.retain(|key, _| failed.contains(key));
        let unread: Vec<(String, String)> = failed.iter().filter(|key| !self.failed_tasks.contains_key(*key)).cloned().collect();
        let wanted: Vec<(String, String)> = if day { running.iter().chain(&unread).cloned().collect() } else { Vec::new() };
        for (key, read) in wanted.iter().zip(self.runs_tasks(&wanted).await) {
            let Some(p) = read else {
                continue;
            };
            if unread.contains(key) {
                self.failed_tasks.insert(key.clone(), p);
            } else {
                progress.insert(key.clone(), p);
            }
        }
        for key in failed {
            if let Some(p) = self.failed_tasks.get(&key) {
                progress.insert(key, p.clone());
            }
        }

        Activity {
            reachable: true,
            error: None,
            base_url: Some(self.base_url.clone()),
            version: self.version.clone(),
            health,
            dags: self.dags.clone(),
            import_errors: self.import_errors,
            runs,
            tasks,
            progress,
            day_read: self.day_read.is_some(),
            taken_at: SystemTime::now(),
        }
    }

    fn failed(&self, error: Error) -> Activity {
        let mut activity = Activity::unreachable(error.to_string());
        activity.base_url = Some(self.base_url.clone());
        activity.version = self.version.clone();
        activity
    }

    /// Every run that ended in the last day, the latest first: the first page says how many
    /// there are, the rest are read at once.
    async fn read_day(&mut self, now: i64) -> Result<Vec<Run>, Error> {
        let since = rfc3339(now - WINDOW_S);
        let page = |offset: usize| {
            let (limit, offset) = (PAGE.to_string(), offset.to_string());
            ask("/dags/~/dagRuns", &[("end_date_gte", since.as_str()), ("order_by", "-end_date"), ("limit", &limit), ("offset", &offset)])
        };
        let (path, query) = page(0);
        let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let first: RunList = self.get(&path, &query).await?;
        let total = first.total_entries.min(PAGE * MAX_PAGES);
        let mut runs: Vec<Run> = first.dag_runs.into_iter().filter_map(run).collect();
        let rest: Vec<Ask> = (1..total.div_ceil(PAGE)).map(|p| page(p * PAGE)).collect();
        for list in self.get_many::<RunList>(rest).await {
            runs.extend(list?.dag_runs.into_iter().filter_map(run));
        }
        // A run that ended while the pages were read can be on two of them.
        let mut seen = HashSet::new();
        runs.retain(|r| seen.insert((r.dag.clone(), r.id.clone())));
        Ok(runs)
    }

    /// Every active DAG, paused ones too: the first page, then the rest at once.
    async fn read_dags(&mut self) -> Result<Vec<Dag>, Error> {
        let page = |offset: usize| {
            let (limit, offset) = (PAGE.to_string(), offset.to_string());
            ask("/dags", &[("limit", limit.as_str()), ("offset", &offset), ("only_active", "true")])
        };
        let (path, query) = page(0);
        let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let first: DagList = self.get(&path, &query).await?;
        let total = first.total_entries.min(PAGE * MAX_PAGES);
        let mut dags: Vec<Dag> = first.dags.into_iter().map(dag).collect();
        let rest: Vec<Ask> = (1..total.div_ceil(PAGE)).map(|p| page(p * PAGE)).collect();
        for list in self.get_many::<DagList>(rest).await {
            dags.extend(list?.dags.into_iter().map(dag));
        }
        Ok(dags)
    }

    /// The tasks of several runs, added up, all asked at once: a run with more than a page of
    /// tasks counts its first page against the total Airflow gives.
    async fn runs_tasks(&mut self, runs: &[(String, String)]) -> Vec<Option<Progress>> {
        let limit = PAGE.to_string();
        let asks: Vec<Ask> = runs
            .iter()
            .map(|(dag, run)| ask(format!("/dags/{}/dagRuns/{}/taskInstances", segment(dag), segment(run)), &[("limit", limit.as_str())]))
            .collect();
        self.get_many::<TaskList>(asks)
            .await
            .into_iter()
            .map(|list| list.ok().map(|l| progress(&l.task_instances, l.total_entries)))
            .collect()
    }

    /// Signed in, then asked; signed in again once when the session has run out.
    async fn get<T: DeserializeOwned>(&mut self, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
        if self.cookies.is_empty() {
            self.sign_in().await?;
        }
        match self.get_once(path, query).await {
            Err(Error::Http(401 | 403)) => {
                self.sign_in().await?;
                self.get_once(path, query).await.map_err(|e| match e {
                    Error::Http(401 | 403) => Error::NoPermission,
                    other => other,
                })
            }
            other => other,
        }
    }

    async fn get_once<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
        let (_, query) = ask("", query);
        fetch(&self.client, &format!("{}/api/v1{path}", self.base_url), &query, &self.cookies.header()).await
    }

    /// Many GETs, `AT_ONCE` in flight at a time, the answers in the order asked. One refused
    /// because the session ran out meanwhile is asked again after a new login.
    async fn get_many<T: DeserializeOwned + Send + 'static>(&mut self, asks: Vec<Ask>) -> Vec<Result<T, Error>> {
        if self.cookies.is_empty()
            && let Err(e) = self.sign_in().await
        {
            return asks.iter().map(|_| Err(e.clone())).collect();
        }
        let mut answers: Vec<Option<Result<T, Error>>> = (0..asks.len()).map(|_| None).collect();
        for (chunk, group) in asks.chunks(AT_ONCE).enumerate() {
            let mut tasks = tokio::task::JoinSet::new();
            for (i, (path, query)) in group.iter().cloned().enumerate() {
                let (client, url, cookies) = (self.client.clone(), format!("{}/api/v1{path}", self.base_url), self.cookies.header());
                tasks.spawn(async move { (chunk * AT_ONCE + i, fetch::<T>(&client, &url, &query, &cookies).await) });
            }
            while let Some(done) = tasks.join_next().await {
                if let Ok((at, answer)) = done {
                    answers[at] = Some(answer);
                }
            }
        }
        let mut out = Vec::with_capacity(asks.len());
        for ((path, query), answer) in asks.iter().zip(answers) {
            match answer {
                Some(Err(Error::Http(401 | 403))) | None => {
                    let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    out.push(self.get(path, &query).await);
                }
                Some(answer) => out.push(answer),
            }
        }
        out
    }

    /// A login, unless one was refused a moment ago.
    async fn sign_in(&mut self) -> Result<(), Error> {
        if self.refused.is_some_and(|at| at.elapsed() < LOGIN_RETRY) {
            return Err(Error::LoginRefused);
        }
        let result = self.login().await;
        if result == Err(Error::LoginRefused) {
            self.refused = Some(Instant::now());
        }
        result
    }

    /// The web UI's login: the form for its CSRF token and the session it lives in, then the
    /// form sent back. Signed in, Airflow sends the browser on, away from the login page.
    async fn login(&mut self) -> Result<(), Error> {
        let url = format!("{}/login/", self.base_url);
        let connection = |e| Error::Connection(transport(e, REQUEST_TIMEOUT));
        self.cookies = Jar::default();
        let page = self.client.get(&url).send().await.map_err(connection)?;
        self.cookies.take(page.headers());
        let status = page.status();
        let html = page.text().await.map_err(connection)?;
        if !status.is_success() {
            return Err(Error::Http(status.as_u16()));
        }
        let csrf = csrf_token(&html).ok_or(Error::NoLoginForm)?;
        let form = [("csrf_token", csrf.as_str()), ("username", self.user.as_str()), ("password", self.password.as_str())];
        let answer = self
            .client
            .post(&url)
            .header(COOKIE, self.cookies.header())
            .header(REFERER, &url)
            .form(&form)
            .send()
            .await
            .map_err(connection)?;
        self.cookies.take(answer.headers());
        let location = answer.headers().get(LOCATION).and_then(|l| l.to_str().ok()).unwrap_or_default();
        let signed_in = answer.status().is_redirection() && !location.contains("/login");
        if !signed_in {
            self.cookies = Jar::default();
        }
        if let Ok(mut session) = self.session.lock() {
            *session = self.cookies.header();
        }
        if signed_in {
            self.refused = None;
            Ok(())
        } else {
            Err(Error::LoginRefused)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn the_form_s_csrf_token_is_found_whatever_the_order_of_its_attributes() {
        let html = r#"<form class="form" action="" method="post" name="login">
            <input id="csrf_token" name="csrf_token" type="hidden" value="IjQ5ZTBk.ZwXyz">
            <input class="form-control" id="username" name="username" type="text">"#;
        assert_eq!(csrf_token(html).as_deref(), Some("IjQ5ZTBk.ZwXyz"));
        let reordered = r#"<input value="tok" type="hidden" name="csrf_token" id="csrf_token"/>"#;
        assert_eq!(csrf_token(reordered).as_deref(), Some("tok"));
        assert_eq!(csrf_token("<html>no form</html>"), None);
    }

    #[test]
    fn the_jar_keeps_the_latest_value_of_each_cookie() {
        let mut headers = HeaderMap::new();
        headers.append(SET_COOKIE, HeaderValue::from_static("session=first; Path=/; HttpOnly; SameSite=Lax"));
        let mut jar = Jar::default();
        jar.take(&headers);
        assert_eq!(jar.header(), "session=first");
        let mut later = HeaderMap::new();
        later.append(SET_COOKIE, HeaderValue::from_static("session=signed.in; Expires=Wed, 04 Nov 2026 08:52:07 GMT; Secure"));
        later.append(SET_COOKIE, HeaderValue::from_static("remember=; Max-Age=0"));
        jar.take(&later);
        assert_eq!(jar.header(), "session=signed.in", "replaced, and an emptied cookie is not kept");
    }

    #[test]
    fn runs_tasks_and_dags_are_read_from_airflow_s_shapes() {
        let list: RunList = serde_json::from_str(
            r#"{ "dag_runs": [ {
                "conf": {}, "dag_id": "test_clickhouse_connection_4",
                "dag_run_id": "manual__2026-01-16T14:24:48.157778+00:00",
                "data_interval_end": "2026-01-16T14:24:48.157778+00:00",
                "end_date": null, "execution_date": "2026-01-16T14:24:48.157778+00:00",
                "external_trigger": true, "logical_date": "2026-01-16T14:24:48.157778+00:00",
                "note": null, "run_type": "manual", "start_date": "2026-01-16T14:24:48.230815+00:00",
                "state": "running" } ], "total_entries": 4 }"#,
        )
        .expect("parses");
        assert_eq!(list.total_entries, 4);
        let stuck = run(list.dag_runs.into_iter().next().unwrap()).unwrap();
        assert_eq!((stuck.state, stuck.kind.as_str()), (RunState::Running, "manual"));
        assert_eq!(stuck.start, unix("2026-01-16T14:24:48Z"));
        assert!(stuck.is_stuck(now_s()));

        let tasks: TaskList = serde_json::from_str(
            r#"{ "task_instances": [
                { "dag_id": "etl", "dag_run_id": "r", "task_id": "load", "state": "success" },
                { "dag_id": "etl", "dag_run_id": "r", "task_id": "skip", "state": "skipped" },
                { "dag_id": "etl", "dag_run_id": "r", "task_id": "check", "state": "running", "try_number": 1, "max_tries": 2,
                  "start_date": "2026-10-05T15:20:03.327364+00:00", "hostname": "airflow-worker-0", "operator": "PythonOperator" },
                { "dag_id": "etl", "dag_run_id": "r", "task_id": "flaky", "state": "up_for_retry" },
                { "dag_id": "etl", "dag_run_id": "r", "task_id": "push", "state": null }
            ], "total_entries": 5 }"#,
        )
        .expect("parses");
        let p = progress(&tasks.task_instances, tasks.total_entries);
        assert_eq!((p.total, p.done), (5, 2));
        assert_eq!((p.running, p.retrying), (vec!["check".to_string()], vec!["flaky".to_string()]));
        let live = task(tasks.task_instances.into_iter().nth(2).unwrap()).unwrap();
        assert_eq!((live.try_number, live.max_tries, live.host.as_deref()), (1, 2, Some("airflow-worker-0")));

        let dags: DagList = serde_json::from_str(
            r#"{ "dags": [ { "dag_id": "ACCOUNTING_MARTS_MV", "owners": ["abdul.djafar"], "is_paused": false,
                "timetable_description": "At 22:00, only on Sunday",
                "schedule_interval": { "__type": "CronExpression", "value": "0 22 * * 0" },
                "next_dagrun_create_after": "2026-10-11T22:00:00+00:00",
                "tags": [ { "name": "clickhouse" } ], "description": "Creates the shared MV" },
              { "dag_id": "redash_scheduler_monitoring", "owners": null, "is_paused": true, "tags": null,
                "schedule_interval": { "__type": "TimeDelta", "days": 0, "seconds": 3600, "microseconds": 0 } } ],
              "total_entries": 2 }"#,
        )
        .expect("parses");
        let dags: Vec<Dag> = dags.dags.into_iter().map(dag).collect();
        assert_eq!(dags[0].cron.as_deref(), Some("0 22 * * 0"));
        assert_eq!(dags[0].tags, ["clickhouse"]);
        assert_eq!(dags[0].next, unix("2026-10-11T22:00:00Z"));
        assert_eq!((dags[1].paused, dags[1].cron.as_deref()), (true, Some("1h")));
        assert!(dags[1].owners.is_empty());
    }

    #[test]
    fn health_names_each_heartbeat_by_its_component() {
        let api: ApiHealth = serde_json::from_str(
            r#"{ "dag_processor": { "latest_dag_processor_heartbeat": null, "status": null },
                 "metadatabase": { "status": "healthy" },
                 "scheduler": { "latest_scheduler_heartbeat": "2026-10-05T15:15:37.660939+00:00", "status": "healthy" },
                 "triggerer": { "latest_triggerer_heartbeat": "2026-10-05T15:15:37.932159+00:00", "status": "unhealthy" } }"#,
        )
        .expect("parses");
        let h = health(api);
        assert_eq!(h.metadatabase.as_deref(), Some("healthy"));
        assert_eq!(h.scheduler.at, unix("2026-10-05T15:15:37Z"));
        assert_eq!(h.scheduler.severity(), crate::severity::Severity::Ok);
        assert_eq!(h.triggerer.severity(), crate::severity::Severity::Crit);
        assert_eq!(h.dag_processor.severity(), crate::severity::Severity::None, "not run here: nothing to say");
    }

    #[test]
    fn the_api_s_filters_take_utc_with_a_z() {
        assert_eq!(rfc3339(1_791_103_927), "2026-10-04T08:52:07Z");
    }
}
