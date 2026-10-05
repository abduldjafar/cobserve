//! The Redash queue (DESIGN.md §6.3), fail-soft.
//!
//! Any error becomes `QueueStatus { reachable: false, .. }` and the strip prints
//! `unreachable (HTTP 401)`. **The app must never exit because Redash is down** — that is the
//! whole reason this is a separate source with its own task.
//!
//! What it reads is what the Redash admins' own queue script reads, with the same admin API
//! key, and all of it read-only:
//!
//! - `/api/admin/queries/rq_status` — per queue, the jobs RQ lists as started and a count of
//!   the waiting ones; and every worker, with the job it holds right now;
//! - `/api/users/{id}`, `/api/queries/{id}`, `/api/data_sources` — who is behind a job, the
//!   query it runs (name and SQL) and what it runs on (a data source's name and type). These
//!   change when someone edits something, so they are cached.
//!
//! RQ's started list is not the same thing as running: a worker that dies leaves its job in
//! it, and without a time limit the entry stays for months. A job counts as running when a
//! live worker holds it; the rest are [`Stale`] and shown apart.
//!
//! `rq_status` reports queued jobs as a *count*, so the names of waiting jobs live in RQ's
//! Redis keys. That is read-only here too: two commands, no writes, no dependency.
//!
//! One call changes something, and only when someone asks for it on view 2 and says yes:
//! `DELETE /api/jobs/{id}`, which cancels that one job — what Redash's own Cancel button and
//! the admins' script do. Redash then lets go of it; a query it started in ClickHouse runs on
//! until a `KILL QUERY` there stops it, which nothing here sends.

use crate::attrib;
use crate::config::RedashConfig;
use crate::model::{Job, JobState, QueueRow, QueueStatus, Stale};
use crate::app::Event;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// §6.3: the queue moves every 3 s.
const QUEUE_POLL: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
/// A worker whose last heartbeat is older than this is gone. It is RQ's own number: an idle
/// worker beats once every `worker_ttl - 15` s (405 s by default), and RQ forgets a worker
/// `worker_ttl + 60` s (480 s) after its last beat. Anything shorter loses idle workers.
const HEARTBEAT_STALE: Duration = Duration::from_secs(480);
/// Started longer ago than this: stale whatever else is known — the cut-off the admins'
/// script uses for its zombies.
const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);
/// A job no worker holds is called stale only past this age. RQ puts a job in the started
/// list and on its worker in one step, but Redash reads the two one after the other.
const ORPHAN_GRACE: Duration = Duration::from_secs(60);
/// A saved query's name and SQL are read again after this long: they change when someone
/// edits the query.
const QUERY_TTL: Duration = Duration::from_secs(600);
/// A data source missing from Redash's list is looked for again at most this often.
const SOURCES_RETRY: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// The API shape, as captured from the running instance (tests/fixtures/rq_status.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RqStatus {
    #[serde(default, deserialize_with = "lenient_queues")]
    queues: HashMap<String, RqQueue>,
    #[serde(default, deserialize_with = "lenient_list")]
    workers: Vec<RqWorker>,
}

#[derive(Debug, Deserialize)]
struct RqQueue {
    #[serde(default, deserialize_with = "lenient_text")]
    name: String,
    /// Redash ≥ 10 with RQ reports a count here, not a list of jobs.
    #[serde(default, deserialize_with = "lenient_count")]
    queued: u32,
    #[serde(default, deserialize_with = "lenient_list")]
    started: Vec<RqJob>,
}

#[derive(Debug, Deserialize)]
struct RqJob {
    #[serde(default, deserialize_with = "lenient_text")]
    id: String,
    /// The task: `redash.tasks.queries.execution.execute_query`, `…refresh_schema`.
    #[serde(default, deserialize_with = "lenient_text")]
    name: String,
    #[serde(default, deserialize_with = "lenient_text")]
    origin: String,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    started_at: Option<String>,
    #[serde(default, deserialize_with = "lenient_meta")]
    meta: Option<RqMeta>,
}

/// What Redash puts in a job's `meta` as it enqueues it.
#[derive(Debug, Deserialize, Default)]
struct RqMeta {
    #[serde(default)]
    query_id: QueryRef,
    #[serde(default, deserialize_with = "lenient_id")]
    user_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient_id")]
    data_source_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient_bool")]
    scheduled: bool,
    /// Set when someone cancels the job; the worker running it then stops it — if there still
    /// is one.
    #[serde(default, deserialize_with = "lenient_bool")]
    cancelled: bool,
}

/// What a job's `query_id` says: a saved query's number, or `"adhoc"` — a query run from the
/// editor without being saved, which has no number at all.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum QueryRef {
    #[default]
    Unknown,
    Saved(u64),
    Adhoc,
}

impl QueryRef {
    fn from_value(value: &serde_json::Value) -> QueryRef {
        match value {
            serde_json::Value::Number(n) => n.as_u64().map_or(QueryRef::Unknown, QueryRef::Saved),
            serde_json::Value::String(text) => QueryRef::from_text(text),
            _ => QueryRef::Unknown,
        }
    }

    fn from_text(text: &str) -> QueryRef {
        let text = text.trim().trim_matches(|c| c == '\'' || c == '"');
        if text.eq_ignore_ascii_case("adhoc") {
            QueryRef::Adhoc
        } else {
            text.parse().map_or(QueryRef::Unknown, QueryRef::Saved)
        }
    }

    fn id(self) -> Option<u64> {
        match self {
            QueryRef::Saved(id) => Some(id),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for QueryRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Option::<serde_json::Value>::deserialize(d)?;
        Ok(value.as_ref().map_or(QueryRef::Unknown, QueryRef::from_value))
    }
}

#[derive(Debug, Deserialize)]
struct RqWorker {
    /// The worker id RQ registers; the healthcheck and the heartbeat are per worker.
    #[serde(default, deserialize_with = "lenient_text")]
    name: String,
    #[serde(default, deserialize_with = "lenient_text")]
    state: String,
    /// `<job id> (<function>)` while it runs one.
    #[serde(default, deserialize_with = "lenient_opt_text")]
    current_job: Option<String>,
    #[serde(default, deserialize_with = "lenient_text")]
    queues: String,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    last_heartbeat: Option<String>,
}

// The endpoint's shape moves between Redash versions and with what is running — an ad-hoc
// query's `query_id` is the text "adhoc" — and one field of an unexpected type used to fail
// the whole answer ("error decoding response body"). Every field is read for what it can
// give, and what it cannot give is left out instead.

/// A number, or a number written as text; anything else — `"adhoc"`, `null` — is none.
fn lenient_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }))
}

/// `true`, `"true"`, `"True"`, `1` — anything else is false.
fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match value {
        Some(serde_json::Value::Bool(b)) => b,
        Some(serde_json::Value::Number(n)) => n.as_u64() == Some(1),
        Some(serde_json::Value::String(text)) => matches!(text.trim().to_ascii_lowercase().as_str(), "true" | "1"),
        _ => false,
    })
}

/// A count: a number, a number as text, or the list it counts.
fn lenient_count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match value {
        Some(serde_json::Value::Number(n)) => n.as_u64().map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        Some(serde_json::Value::String(text)) => text.trim().parse().unwrap_or(0),
        Some(serde_json::Value::Array(items)) => u32::try_from(items.len()).unwrap_or(u32::MAX),
        _ => 0,
    })
}

/// Text from whatever it came as: a number written out, a list joined with commas, null empty.
fn lenient_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.map(text_of).unwrap_or_default())
}

fn lenient_opt_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.map(text_of).filter(|text| !text.is_empty()))
}

fn text_of(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Array(items) => items.into_iter().map(text_of).collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
}

/// The entries of a list that read as `T`; anything that is not a list is an empty one.
fn lenient_list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match value {
        Some(serde_json::Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

fn lenient_meta<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<RqMeta>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .filter(serde_json::Value::is_object)
        .and_then(|v| serde_json::from_value(v).ok()))
}

/// Queues by name: an object of queues as Redash 10 sends it, or a list of them.
fn lenient_queues<'de, D: serde::Deserializer<'de>>(d: D) -> Result<HashMap<String, RqQueue>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let entries: Vec<(Option<String>, serde_json::Value)> = match value {
        Some(serde_json::Value::Object(map)) => map.into_iter().map(|(k, v)| (Some(k), v)).collect(),
        Some(serde_json::Value::Array(items)) => items.into_iter().map(|v| (None, v)).collect(),
        _ => Vec::new(),
    };
    Ok(entries
        .into_iter()
        .filter_map(|(key, value)| {
            let queue: RqQueue = serde_json::from_value(value).ok()?;
            let name = key.unwrap_or_else(|| queue.name.clone());
            (!name.is_empty()).then_some((name, queue))
        })
        .collect())
}

/// Redash < 10 is Celery, and the endpoint is `/api/admin/queries/tasks` (§6.3). The shape is
/// different enough to be worth parsing rather than guessing, but it is only a fallback: the
/// instance this app is pointed at is Redash 10.
/// Celery's `/api/admin/queries/tasks` reports the active task ids only. That is the whole
/// reason this fallback exists: names and ages are not in the shape, so the rows say what they
/// can and no more (§6.3).
#[derive(Debug, Deserialize)]
struct CeleryTasks {
    #[serde(default)]
    active: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    #[serde(default, deserialize_with = "lenient_opt_text")]
    email: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiQuery {
    #[serde(default, deserialize_with = "lenient_opt_text")]
    name: Option<String>,
    /// The SQL as saved.
    #[serde(default, deserialize_with = "lenient_opt_text")]
    query: Option<String>,
    #[serde(default, deserialize_with = "lenient_id")]
    data_source_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ApiDataSource {
    #[serde(default, deserialize_with = "lenient_id")]
    id: Option<u64>,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    name: Option<String>,
    /// `clickhouse`, `mysql`, `pg`, `results` (Query Results, which runs inside Redash)…
    #[serde(default, rename = "type", deserialize_with = "lenient_opt_text")]
    kind: Option<String>,
}

/// What one API call can go wrong with, without leaking the key (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Http(u16),
    Connection(String),
    Body(String),
    /// A job id that is not one, kept out of the request's path.
    NotAJob,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(code @ (401 | 403)) => write!(f, "HTTP {code} · the API key has to be an admin's"),
            Error::Http(404) => write!(f, "HTTP 404 · no such endpoint — is the url Redash's own?"),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Connection(e) => write!(f, "{e}"),
            Error::Body(e) => write!(f, "{UNREADABLE}{e}"),
            Error::NotAJob => write!(f, "not a job id Redash gave"),
        }
    }
}

/// Why Redash did not cancel a job, in words for the notice.
fn cancel_error(error: &Error) -> String {
    match error {
        // Redash answers 500 for a job it no longer has: RQ cannot find it to cancel.
        Error::Http(code @ (404 | 500)) => format!("HTTP {code} · Redash no longer has it — it may have just finished"),
        other => other.to_string(),
    }
}

/// How an answer that could not be read starts, for the band to word it as such.
pub const UNREADABLE: &str = "unreadable answer: ";

/// Why an answer is not what the endpoint sends, in words — never with the key (§9).
fn why_unreadable(body: &str, error: &serde_json::Error) -> String {
    if body.trim_start().starts_with('<') {
        // A wrong or non-admin key is redirected to the login page, which is HTML.
        return "a web page, not JSON — check the url and that the API key is an admin's".to_string();
    }
    short(&error.to_string())
}

/// A Redash user as the screen shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Person {
    /// The way view 1 shows people (§6.4), so the two views name a person alike.
    label: String,
    /// `Name <address>`, for the drawer.
    full: String,
}

impl Person {
    fn from_api(user: &ApiUser, id: u64) -> Person {
        let email = user.email.as_deref().map(str::trim).filter(|e| !e.is_empty());
        let name = user.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let label = match (email, name) {
            (Some(email), _) => attrib::display_person(email),
            (None, Some(name)) => name.to_string(),
            (None, None) => format!("user {id}"),
        };
        let full = match (name, email) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (Some(only), None) | (None, Some(only)) => only.to_string(),
            (None, None) => format!("user {id}"),
        };
        Person { label, full }
    }
}

#[derive(Debug, Clone)]
struct DataSource {
    name: String,
    kind: Option<String>,
}

#[derive(Debug, Default)]
struct DataSources {
    by_id: HashMap<u64, DataSource>,
    /// When the list was last asked for, answered or refused.
    read_at: Option<SystemTime>,
}

/// What `/api/config` says about the instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Instance {
    version: Option<String>,
    /// Redash < 10: Celery, and another endpoint (§6.3).
    celery: bool,
}

impl Instance {
    fn from_config(config: &serde_json::Value) -> Instance {
        // Redash 10 puts it under `client_config`; older ones at the top.
        let version = config
            .get("version")
            .or_else(|| config.get("client_config").and_then(|c| c.get("version")))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        // No version at all: assume the modern shape and let the endpoint answer for itself.
        let major: u32 = version
            .as_deref()
            .and_then(|v| v.split('.').next())
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        Instance { version, celery: major < 10 }
    }
}

pub struct RedashSource {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    redis: Option<Redis>,
    // Caches for the lifetime of the session (§6.3), each behind a `Mutex` because the poll
    // task holds `&self` across await points; the guards never live across one.
    /// `None` for a user Redash would not show: asking again every 3 s will not change that.
    users: Mutex<HashMap<u64, Option<Person>>>,
    queries: Mutex<HashMap<u64, (SystemTime, Option<ApiQuery>)>>,
    sources: Mutex<DataSources>,
    instance: Mutex<Option<Instance>>,
    /// The jobs a worker held at the last poll. One that has just been let go is finishing,
    /// not stale: it leaves the started list at the next poll.
    held_before: Mutex<HashSet<String>>,
}

/// Read-only Redis, just enough of RESP for `LRANGE` and `HGETALL` (§6.3).
///
/// The design allows the `redis` crate; this is ~60 lines instead, and it cannot write by
/// accident, which is the property that matters when pointing it at Redash's live Redis.
pub struct Redis {
    address: String,
    password: Option<String>,
}

impl RedashSource {
    /// `None` when Redash is not configured at all: the strip then says so instead of
    /// pretending the queue is empty (§9).
    pub fn new(config: &RedashConfig) -> Option<RedashSource> {
        let url = config.url.clone()?;
        let api_key = config.admin_api_key.clone()?;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .ok()?;
        Some(RedashSource {
            base_url: url.trim_end_matches('/').to_string(),
            api_key,
            client,
            redis: config.redis_url.as_deref().and_then(Redis::from_url),
            users: Mutex::new(HashMap::new()),
            queries: Mutex::new(HashMap::new()),
            sources: Mutex::new(DataSources::default()),
            instance: Mutex::new(None),
            held_before: Mutex::new(HashSet::new()),
        })
    }

    /// Poll until the loop ends, and cancel the jobs asked for on the way. Errors are
    /// reported, never fatal.
    pub async fn run(self, tx: tokio::sync::mpsc::UnboundedSender<Event>, mut cancels: tokio::sync::mpsc::UnboundedReceiver<String>) {
        let mut tick = tokio::time::interval(QUEUE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                Some(id) = cancels.recv() => {
                    let result = self.cancel(&id).await.map_err(|e| cancel_error(&e));
                    if tx.send(Event::Cancelled(id, result)).is_err() {
                        return;
                    }
                    // Read again at once, as the admins' script does after a cancel: the
                    // screen shows straight away whether the job let go of its worker.
                }
            }
            let status = self.status().await;
            if tx.send(Event::Queue(Box::new(status))).is_err() {
                return;
            }
        }
    }

    /// One job cancelled, as Redash's Cancel button does: `DELETE /api/jobs/{id}`. A waiting
    /// job leaves its queue; a running one is stopped on its worker, which lets go of it.
    pub async fn cancel(&self, id: &str) -> Result<(), Error> {
        // RQ's ids are UUIDs: anything else is not put into a path.
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(Error::NotAJob);
        }
        let url = format!("{}/api/jobs/{id}", self.base_url);
        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Key {}", self.api_key))
            .send()
            .await
            .map_err(|e| Error::Connection(short(&e.to_string())))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Http(status.as_u16()));
        }
        Ok(())
    }

    /// One poll: per queue what runs, what waits and what was left behind, with names.
    pub async fn status(&self) -> QueueStatus {
        let instance = self.instance().await;
        if instance.as_ref().is_some_and(|i| i.celery) {
            return self.status_celery().await;
        }
        let status: RqStatus = match self.get_json("/api/admin/queries/rq_status").await {
            Ok(status) => status,
            Err(e) => return QueueStatus::unreachable(e.to_string()),
        };
        let mut out = self.build_rq_status(status, SystemTime::now()).await;
        out.version = instance.and_then(|i| i.version);
        out
    }

    /// §6.3: branch on `/api/config`, asked once. An instance that refuses it is assumed to be
    /// the modern shape; one that cannot be reached is asked again next poll.
    async fn instance(&self) -> Option<Instance> {
        if let Some(known) = self.instance.lock().expect("instance").clone() {
            return Some(known);
        }
        let found = match self.get_json::<serde_json::Value>("/api/config").await {
            Ok(config) => Instance::from_config(&config),
            Err(Error::Connection(_)) => return None,
            Err(_) => Instance::default(),
        };
        *self.instance.lock().expect("instance") = Some(found.clone());
        Some(found)
    }

    async fn status_celery(&self) -> QueueStatus {
        match self.get_json::<CeleryTasks>("/api/admin/queries/tasks").await {
            Ok(tasks) => self.build_celery(tasks),
            Err(e) => QueueStatus::unreachable(e.to_string()),
        }
    }

    async fn build_rq_status(&self, status: RqStatus, now: SystemTime) -> QueueStatus {
        let live: Vec<&RqWorker> = status.workers.iter().filter(|w| is_live(w, now)).collect();
        let held: HashSet<String> = live.iter().filter_map(|w| held_job(w)).collect();
        let held_before = std::mem::replace(&mut *self.held_before.lock().expect("held jobs"), held.clone());

        let mut queues: Vec<QueueRow> = Vec::new();
        let mut jobs: Vec<Job> = Vec::new();
        let mut names_available = self.redis.is_some();

        for (key, queue) in &status.queues {
            let name = if queue.name.is_empty() { key.clone() } else { queue.name.clone() };
            // A worker belongs to the queues it was started for; `queues` is a comma list.
            let serving: Vec<&RqWorker> = live.iter().copied().filter(|w| serves(w, &name)).collect();
            let busy = serving.iter().filter(|w| is_busy(w)).count() as u32;
            // A job none of the workers holds means something only when each busy one says
            // which job it holds.
            let accounted = !serving.is_empty() && serving.iter().all(|w| !is_busy(w) || held_job(w).is_some());

            // Oldest wait: the API gives a count, so this only exists when Redis can name the
            // jobs. Never a guess.
            let waiting_ids = match &self.redis {
                Some(redis) => redis.queue_ids(&name).await.unwrap_or_else(|_| {
                    names_available = false;
                    Vec::new()
                }),
                None => Vec::new(),
            };
            let mut oldest: Option<u64> = None;
            for id in &waiting_ids {
                let Some(job) = self.redis_job(&name, id, now).await else {
                    continue;
                };
                oldest = Some(oldest.map_or(job.age_s, |o: u64| o.max(job.age_s)));
                jobs.push(job);
            }

            let (mut running, mut stale) = (0u32, 0u32);
            for started in &queue.started {
                let age_s = started
                    .started_at
                    .as_deref()
                    .and_then(parse_rfc3339_seconds)
                    .map_or(0, |t| now.duration_since(t).map_or(0, |d| d.as_secs()));
                let meta = started.meta.as_ref();
                let state = liveness(
                    held.contains(&started.id) || held_before.contains(&started.id),
                    meta.is_some_and(|m| m.cancelled),
                    age_s,
                    accounted,
                );
                if matches!(state, JobState::Stale(_)) {
                    stale += 1;
                } else {
                    running += 1;
                }
                let origin = if started.origin.is_empty() { &name } else { &started.origin };
                let mut job = self.job(&started.id, state, origin, meta, now).await;
                job.age_s = age_s;
                if job.redash_query_id.is_none() && !job.adhoc && job.query_name.is_none() {
                    // Not a query at all — a schema refresh, say: the task is its name.
                    job.query_name = task_label(&started.name);
                }
                jobs.push(job);
            }

            queues.push(QueueRow {
                name,
                running,
                // The count is authoritative; the job list is what fills in the names.
                waiting: queue.queued.max(waiting_ids.len() as u32),
                oldest_wait_s: oldest,
                stale,
                workers_busy: busy,
                workers_total: serving.len() as u32,
            });
        }

        queues.sort_by(|a, b| a.name.cmp(&b.name));
        QueueStatus {
            reachable: true,
            error: None,
            queues,
            jobs,
            names_available,
            workers_busy: live.iter().filter(|w| is_busy(w)).count() as u32,
            workers_total: live.len() as u32,
            host: Some(
                self.base_url
                    .split("://")
                    .nth(1)
                    .unwrap_or(&self.base_url)
                    .to_string(),
            ),
            version: None,
            taken_at: now,
        }
    }

    fn build_celery(&self, tasks: CeleryTasks) -> QueueStatus {
        let jobs: Vec<Job> = tasks
            .active
            .into_iter()
            .enumerate()
            // Celery does not report a start time in this shape; an age of 0 reads as
            // "unknown" rather than as "just started", and the row is still listed.
            .map(|(index, id)| Job::new(id, JobState::Started, format!("celery:{index}")))
            .collect();
        let running = jobs.len() as u32;
        QueueStatus {
            reachable: true,
            error: None,
            queues: vec![QueueRow {
                name: "default".to_string(),
                running,
                waiting: 0,
                oldest_wait_s: None,
                stale: 0,
                workers_busy: running,
                workers_total: running.max(1),
            }],
            jobs,
            names_available: false,
            workers_busy: running,
            workers_total: running.max(1),
            host: None,
            version: None,
            taken_at: SystemTime::now(),
        }
    }

    /// A waiting job, from RQ's hash of it.
    ///
    /// RQ pickles a job's `data` and `meta`, which nothing here can read; its `description` is
    /// the call written out — `execute_query('SELECT …', 3, {'Username': …, 'query_id': 7438},
    /// user_id=42, …)` — and that is plain text.
    async fn redis_job(&self, queue: &str, id: &str, now: SystemTime) -> Option<Job> {
        let hash = self.redis.as_ref()?.job_hash(id).await.ok()?;
        let enqueued_at = hash.get("enqueued_at").map(|t| t.trim_matches('"').to_string()).unwrap_or_default();
        let age_s = parse_rfc3339_seconds(&enqueued_at)
            .and_then(|t| now.duration_since(t).ok())
            .map_or(0, |d| d.as_secs());

        // A JSON serializer leaves `meta` readable; the call string is there either way.
        let mut meta: RqMeta = hash.get("meta").and_then(|m| serde_json::from_str(m).ok()).unwrap_or_default();
        if let Some(call) = hash.get("description").map(|d| meta_from_call(d)) {
            if meta.query_id == QueryRef::Unknown {
                meta.query_id = call.query_id;
            }
            meta.user_id = meta.user_id.or(call.user_id);
            meta.data_source_id = meta.data_source_id.or(call.data_source_id);
            meta.scheduled |= call.scheduled;
        }
        let mut job = self.job(id, JobState::Queued, queue, Some(&meta), now).await;
        job.age_s = age_s;
        Some(job)
    }

    /// A job with everything Redash can say about it: who, which query, on what (§6.3).
    async fn job(&self, id: &str, state: JobState, queue: &str, meta: Option<&RqMeta>, now: SystemTime) -> Job {
        let mut job = Job::new(id, state, queue);
        let Some(meta) = meta else {
            return job;
        };
        job.redash_query_id = meta.query_id.id();
        job.adhoc = meta.query_id == QueryRef::Adhoc;
        job.scheduled = meta.scheduled;
        if let Some(person) = match meta.user_id {
            Some(user) => self.person(user).await,
            None => None,
        } {
            job.person = Some(person.label);
            job.person_full = Some(person.full);
        }
        let query = match meta.query_id {
            QueryRef::Saved(id) => self.query(id, now).await,
            _ => None,
        };
        // What the job ran on is in its own meta; a saved query's data source is what it
        // would run on now, which is the next best thing.
        let source = match meta.data_source_id.or(query.as_ref().and_then(|q| q.data_source_id)) {
            Some(id) => self.data_source(id, now).await,
            None => None,
        };
        if let Some(source) = source {
            job.data_source = Some(source.name);
            job.data_source_type = source.kind;
        }
        if let Some(query) = query {
            job.query_name = query.name;
            job.sql = query.query;
        }
        job
    }

    async fn person(&self, id: u64) -> Option<Person> {
        if let Some(known) = self.users.lock().expect("user cache").get(&id) {
            return known.clone();
        }
        let found = match self.get_json::<ApiUser>(&format!("/api/users/{id}")).await {
            Ok(user) => Some(Person::from_api(&user, id)),
            // A connection that failed says nothing about the user: ask again next time.
            Err(Error::Connection(_)) => return None,
            // Deleted, or not visible to this key.
            Err(_) => None,
        };
        self.users.lock().expect("user cache").insert(id, found.clone());
        found
    }

    async fn query(&self, id: u64, now: SystemTime) -> Option<ApiQuery> {
        let cached = self.queries.lock().expect("query cache").get(&id).cloned();
        if let Some((read_at, known)) = &cached
            && now.duration_since(*read_at).is_ok_and(|age| age < QUERY_TTL)
        {
            return known.clone();
        }
        let found = match self.get_json::<ApiQuery>(&format!("/api/queries/{id}")).await {
            Ok(query) => Some(query),
            // Unreachable for now: what was read before is still the best answer.
            Err(Error::Connection(_)) => return cached.and_then(|(_, known)| known),
            Err(_) => None,
        };
        self.queries.lock().expect("query cache").insert(id, (now, found.clone()));
        found
    }

    /// One list call names every data source and says its type; it is asked again only for
    /// an id it did not have, and not more often than [`SOURCES_RETRY`].
    async fn data_source(&self, id: u64, now: SystemTime) -> Option<DataSource> {
        {
            let sources = self.sources.lock().expect("data source cache");
            if let Some(found) = sources.by_id.get(&id) {
                return Some(found.clone());
            }
            let asked_lately = sources
                .read_at
                .is_some_and(|at| now.duration_since(at).map_or(true, |age| age < SOURCES_RETRY));
            if asked_lately {
                return None;
            }
        }
        let list = match self.get_json::<serde_json::Value>("/api/data_sources").await {
            Ok(list) => list,
            Err(Error::Connection(_)) => return None,
            // Refused: remember that it was asked, and say no more than the job itself does.
            Err(_) => serde_json::Value::Null,
        };
        // A list, or `{"results": [...]}` from versions that page it.
        let entries = match list {
            serde_json::Value::Array(items) => items,
            serde_json::Value::Object(mut map) => match map.remove("results") {
                Some(serde_json::Value::Array(items)) => items,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let mut sources = self.sources.lock().expect("data source cache");
        sources.read_at = Some(now);
        for entry in entries {
            let Ok(source) = serde_json::from_value::<ApiDataSource>(entry) else {
                continue;
            };
            if let (Some(id), Some(name)) = (source.id, source.name) {
                sources.by_id.insert(id, DataSource { name, kind: source.kind });
            }
        }
        sources.by_id.get(&id).cloned()
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Key {}", self.api_key))
            .send()
            .await
            .map_err(|e| Error::Connection(short(&e.to_string())))?;

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Http(status.as_u16()));
        }
        let body = response
            .text()
            .await
            .map_err(|e| Error::Connection(short(&e.to_string())))?;
        serde_json::from_str::<T>(&body).map_err(|e| Error::Body(why_unreadable(&body, &e)))
    }
}

/// Whether a job in RQ's started list is running, or an entry left behind ([`Stale`]).
fn liveness(held: bool, cancelled: bool, age_s: u64, accounted: bool) -> JobState {
    if held {
        // Whatever its age: a worker is busy with it.
        return JobState::Started;
    }
    if cancelled {
        return JobState::Stale(Stale::Cancelled);
    }
    if age_s > STALE_AFTER.as_secs() {
        return JobState::Stale(Stale::OverADay);
    }
    if accounted && age_s > ORPHAN_GRACE.as_secs() {
        return JobState::Stale(Stale::NoWorker);
    }
    JobState::Started
}

/// A worker RQ still knows: registered by name, and heard from within [`HEARTBEAT_STALE`].
/// A heartbeat from the future is a clock that runs ahead, not a dead worker.
fn is_live(worker: &RqWorker, now: SystemTime) -> bool {
    !worker.name.is_empty()
        && worker
            .last_heartbeat
            .as_deref()
            .and_then(parse_rfc3339_seconds)
            .is_some_and(|t| now.duration_since(t).map_or(true, |age| age < HEARTBEAT_STALE))
}

/// `queues: ""` means the worker listens to every queue.
fn serves(worker: &RqWorker, queue: &str) -> bool {
    let queues = worker.queues.trim();
    queues.is_empty() || queues.split(',').any(|q| q.trim() == queue)
}

fn is_busy(worker: &RqWorker) -> bool {
    worker.state == "busy" || worker.current_job.is_some()
}

/// The id of the job a worker holds: Redash writes it as `<id> (<function>)`.
fn held_job(worker: &RqWorker) -> Option<String> {
    worker
        .current_job
        .as_deref()?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// `redash.tasks.queries.maintenance.refresh_schema` → `refresh_schema`; a query's own task
/// says nothing its query does not.
fn task_label(task: &str) -> Option<String> {
    let last = task.rsplit('.').next()?.trim();
    (!last.is_empty() && last != "execute_query").then(|| last.to_string())
}

/// The ids in RQ's call string of a Redash job:
/// `…execute_query('SELECT …', 3, {'Username': 'a@b', 'query_id': 7438}, user_id=42,
/// scheduled_query_id=None, …)`.
fn meta_from_call(call: &str) -> RqMeta {
    fn re(pattern: &str, cell: &'static OnceLock<Regex>) -> &'static Regex {
        cell.get_or_init(|| Regex::new(pattern).expect("static regex"))
    }
    static QUERY: OnceLock<Regex> = OnceLock::new();
    static USER: OnceLock<Regex> = OnceLock::new();
    static SOURCE: OnceLock<Regex> = OnceLock::new();
    static SCHEDULED: OnceLock<Regex> = OnceLock::new();
    let capture = |regex: &Regex| regex.captures(call).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
    RqMeta {
        query_id: capture(re(r#"['"]query_id['"]:\s*('adhoc'|"adhoc"|\d+)"#, &QUERY))
            .map_or(QueryRef::Unknown, |text| QueryRef::from_text(&text)),
        user_id: capture(re(r"\buser_id=(\d+)", &USER)).and_then(|id| id.parse().ok()),
        // The second argument, after the SQL written out as a Python string.
        data_source_id: capture(re(r#"\(\s*(?:'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*")\s*,\s*(\d+)"#, &SOURCE))
            .and_then(|id| id.parse().ok()),
        scheduled: capture(re(r"\bscheduled_query_id=(\d+)", &SCHEDULED)).is_some(),
        cancelled: false,
    }
}

/// RFC 3339 without pulling in a date library: `2026-09-20T13:13:10.699`,
/// `2026-09-20T13:13:10Z` and `2026-09-20 16:13:10+03:00` all show up in this world. No zone
/// means UTC, which is what RQ writes.
pub fn parse_rfc3339_seconds(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_matches('"');
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut date_parts = date.split('-');
    let year: u64 = date_parts.next()?.parse().ok()?;
    let month: u64 = date_parts.next()?.parse().ok()?;
    let day: u64 = date_parts.next()?.parse().ok()?;

    let (clock, zone) = match rest.find(['Z', 'z', '+', '-']) {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let clock = clock.split('.').next().unwrap_or(clock);
    let mut time_parts = clock.split(':');
    let hour: u64 = time_parts.next()?.trim().parse().ok()?;
    let minute: u64 = time_parts.next()?.trim().parse().ok()?;
    let second: u64 = time_parts.next().unwrap_or("0").trim().parse().unwrap_or(0);

    let days = days_from_civil(year, month, day);
    let local = days * 86_400 + hour * 3600 + minute * 60 + second;
    let utc = i64::try_from(local).ok()? - zone_offset(zone);
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(utc).ok()?))
}

/// `+03:00`, `-0500`, `Z` → seconds east of UTC.
fn zone_offset(zone: &str) -> i64 {
    let sign = match zone.chars().next() {
        Some('+') => 1,
        Some('-') => -1,
        _ => return 0,
    };
    let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
    let hours: i64 = digits.get(0..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minutes: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

/// Howard Hinnant's civil-date algorithm: days since the epoch, no dependencies.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    // u64 arithmetic: no negative years reach here, so the era is a plain division.
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Trim a message to something printable, without any query string: §9 says credentials are
/// never printed, and a URL that ends up in an error message is the easy way to break that.
fn short(message: &str) -> String {
    
    message
        .split("?")
        .next()
        .unwrap_or(message)
        .chars()
        .take(120)
        .collect()
}

impl Redis {
    /// `redis://[:password@]host:port/db`
    pub fn from_url(url: &str) -> Option<Redis> {
        let rest = url.strip_prefix("redis://").or_else(|| url.strip_prefix("rediss://"))?;
        let (authority, _db) = match rest.split_once('/') {
            Some((a, db)) => (a, db),
            None => (rest, ""),
        };
        let (credentials, host) = match authority.rsplit_once('@') {
            Some((creds, host)) => (Some(creds), host),
            None => (None, authority),
        };
        // `redis://:secret@host` puts the password after the colon, `redis://user:secret@host`
        // before it. Take whichever half is not empty.
        let password = credentials
            .map(|c| c.trim_start_matches(':'))
            .filter(|c| !c.is_empty())
            .map(|c| match c.split_once(':') {
                Some((user, pass)) if !user.is_empty() => pass.to_string(),
                _ => c.to_string(),
            });
        let address = if host.contains(':') {
            host.to_string()
        } else {
            format!("{host}:6379")
        };
        Some(Redis { address, password })
    }

    /// `LRANGE rq:queue:<name> 0 -1` — the job ids, oldest first.
    async fn queue_ids(&self, queue: &str) -> Result<Vec<String>, Error> {
        let reply = self
            .command(&["LRANGE", &format!("rq:queue:{queue}"), "0", "-1"])
            .await?;
        Ok(match reply {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|item| match item {
                    Value::Bulk(bytes) => String::from_utf8(bytes).ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    /// `HGETALL rq:job:<id>` — RQ's job hash, which is where `data` and `meta` live.
    async fn job_hash(&self, id: &str) -> Result<HashMap<String, String>, Error> {
        let reply = self.command(&["HGETALL", &format!("rq:job:{id}")]).await?;
        let mut out = HashMap::new();
        if let Value::Array(items) = reply {
            let mut iter = items.into_iter();
            while let (Some(Value::Bulk(key)), Some(Value::Bulk(value))) = (iter.next(), iter.next()) {
                if let (Ok(key), Ok(value)) = (
                    String::from_utf8(key),
                    String::from_utf8(value),
                ) {
                    out.insert(key, value);
                }
            }
        }
        Ok(out)
    }

    /// One RESP command. Only these two calls exist in this file, so nothing here can write.
    async fn command(&self, parts: &[&str]) -> Result<Value, Error> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = tokio::net::TcpStream::connect(&self.address)
            .await
            .map_err(|e| Error::Connection(short(&e.to_string())))?;
        let _ = stream.set_nodelay(true);

        let mut request = String::from("*2\r\n");
        if let Some(password) = &self.password {
            request = format!("*2\r\n${}\r\nAUTH\r\n${}\r\n{password}\r\n", "AUTH".len(), password.len());
        }
        let request = format!("{request}*{}\r\n", parts.len());
        let mut buffer = request;
        for part in parts {
            buffer.push_str(&format!("${}\r\n{part}\r\n", part.len()));
        }

        stream
            .write_all(buffer.as_bytes())
            .await
            .map_err(|e| Error::Connection(short(&e.to_string())))?;

        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|e| Error::Connection(short(&e.to_string())))?;
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if let Some(value) = Value::parse(&raw) {
                return Ok(value);
            }
        }
        Err(Error::Connection("redis closed the connection".to_string()))
    }
}

/// Just enough RESP to read two replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Simple(String),
    Error(String),
    Bulk(Vec<u8>),
    Array(Vec<Value>),
    Null,
}

impl Value {
    /// Returns the value once the buffer holds a complete reply.
    pub fn parse(raw: &[u8]) -> Option<Value> {
        let mut cursor = 0usize;
        parse_at(raw, &mut cursor)
    }
}

fn parse_at(raw: &[u8], cursor: &mut usize) -> Option<Value> {
    let line_end = find_crlf(raw, *cursor)?;
    let line = std::str::from_utf8(&raw[*cursor..line_end]).ok()?;
    let (kind, rest) = line.split_at(1);
    match kind {
        "+" => {
            *cursor = line_end + 2;
            Some(Value::Simple(rest.to_string()))
        }
        "-" => {
            *cursor = line_end + 2;
            Some(Value::Error(rest.to_string()))
        }
        ":" => {
            *cursor = line_end + 2;
            None // An integer reply is not something these two commands return.
        }
        "$" => {
            let len: i64 = rest.trim().parse().ok()?;
            *cursor = line_end + 2;
            if len < 0 {
                return Some(Value::Null);
            }
            let len = len as usize;
            if raw.len() < *cursor + len + 2 {
                return None;
            }
            let bytes = raw[*cursor..*cursor + len].to_vec();
            *cursor += len + 2;
            Some(Value::Bulk(bytes))
        }
        "*" => {
            let len: i64 = rest.trim().parse().ok()?;
            *cursor = line_end + 2;
            if len < 0 {
                return Some(Value::Null);
            }
            let mut items = Vec::with_capacity(len as usize);
            for _ in 0..len {
                items.push(parse_at(raw, cursor)?);
            }
            Some(Value::Array(items))
        }
        _ => None,
    }
}

fn find_crlf(raw: &[u8], from: usize) -> Option<usize> {
    raw[from..]
        .windows(2)
        .position(|w| w == b"\r\n")
        .map(|i| from + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        include_str!("../../tests/fixtures/rq_status.json").to_string()
    }

    fn parse(body: &str) -> RqStatus {
        serde_json::from_str(body).expect("the fixture is the real endpoint's shape")
    }

    /// Busy and total of the live workers serving `queue`, the way a queue row counts them.
    fn worker_counts(workers: &[RqWorker], queue: &str, now: SystemTime) -> (u32, u32) {
        let serving: Vec<&RqWorker> = workers.iter().filter(|w| is_live(w, now) && serves(w, queue)).collect();
        (serving.iter().filter(|w| is_busy(w)).count() as u32, serving.len() as u32)
    }

    #[test]
    fn the_captured_response_parses() {
        let status = parse(&fixture());
        assert!(status.queues.contains_key("default"));
        assert!(status.queues.contains_key("emails"));
        assert!(!status.workers.is_empty());
        assert!(
            status.workers.iter().all(|w| !w.name.is_empty()),
            "workers are identified by name"
        );
    }

    #[test]
    fn the_captured_response_has_counts_not_names() {
        // §6.3: this is the case the design predicts — queued is a number, so the WAITING list
        // can only be filled from Redis.
        let status = parse(&fixture());
        let queries = status.queues.get("queries");
        assert!(queries.is_none(), "an idle Redash has no queries queue at all");
        for (name, queue) in &status.queues {
            assert_eq!(name, &queue.name);
            assert!(queue.started.is_empty());
        }
    }

    #[test]
    fn a_started_job_carries_the_query_and_the_user() {
        let body = r#"{
          "queues": {"queries": {"name": "queries", "queued": 3, "started": [
            {"id": "job-1", "origin": "queries", "enqueued_at": "2026-10-03T09:00:00",
             "started_at": "2026-10-03T09:01:00",
             "meta": {"query_id": 7438, "user_id": 42, "data_source_id": 3}}
          ]}},
          "workers": [
            {"name": "w1", "state": "busy", "current_job": "job-1", "queues": "queries",
             "last_heartbeat": "2026-10-03T12:40:00.000"}
          ]
        }"#;
        let status = parse(body);
        let queue = status.queues.get("queries").unwrap();
        assert_eq!(queue.queued, 3);
        assert_eq!(queue.started.len(), 1);
        assert_eq!(queue.started[0].meta.as_ref().unwrap().query_id.id(), Some(7438));
        assert_eq!(queue.started[0].meta.as_ref().unwrap().user_id, Some(42));

        // The heartbeat in the fixture is 2026-10-03T12:40:00Z; 30 s later the worker counts.
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_031_230);
        assert_eq!(worker_counts(&status.workers, "queries", now), (1, 1));
    }

    #[test]
    fn a_stale_heartbeat_is_not_capacity() {
        let body = r#"{
          "queues": {"queries": {"name": "queries", "queued": 0, "started": []}},
          "workers": [{"name": "w1", "state": "busy", "current_job": "job-1",
                       "queues": "queries", "last_heartbeat": "2020-01-01T00:00:00"}]
        }"#;
        let status = parse(body);
        let now = SystemTime::now();
        assert_eq!(worker_counts(&status.workers, "queries", now), (0, 0));
    }

    #[test]
    fn a_worker_on_another_queue_is_not_counted() {
        let body = r#"{
          "queues": {"queries": {"name": "queries", "queued": 0, "started": []},
                     "periodic": {"name": "periodic", "queued": 0, "started": []}},
          "workers": [{"name": "w1", "state": "idle", "current_job": null,
                       "queues": "periodic", "last_heartbeat": "2026-10-03T12:40:00"}]
        }"#;
        let status = parse(body);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_031_230);
        assert_eq!(worker_counts(&status.workers, "queries", now), (0, 0));
        assert_eq!(worker_counts(&status.workers, "periodic", now), (0, 1));
    }

    #[test]
    fn an_empty_queues_list_still_counts_as_serving() {
        let workers = vec![RqWorker {
            name: "w1".into(),
            state: "busy".into(),
            current_job: Some("j".into()),
            queues: String::new(),
            last_heartbeat: None,
        }];
        let now = SystemTime::UNIX_EPOCH;
        assert_eq!(worker_counts(&workers, "queries", now), (0, 0), "no heartbeat, no capacity");
    }

    #[test]
    fn timestamps_parse_without_a_date_library() {
        let t = parse_rfc3339_seconds("2026-09-20T13:13:10.699").unwrap();
        assert_eq!(t, parse_rfc3339_seconds("2026-09-20T13:13:10Z").unwrap());
        assert_eq!(
            parse_rfc3339_seconds("2026-09-20T13:13:10.699").unwrap(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_789_909_990)
        );
        assert!(parse_rfc3339_seconds("not a time").is_none());
        assert!(parse_rfc3339_seconds("2026-09-20").is_none());
        // A zone moves the instant; a space instead of the T is Python's own str().
        assert_eq!(parse_rfc3339_seconds("2026-09-20 16:13:10+03:00"), parse_rfc3339_seconds("2026-09-20T13:13:10"));
        assert_eq!(parse_rfc3339_seconds("2026-09-20T08:13:10.5-0500"), parse_rfc3339_seconds("2026-09-20T13:13:10Z"));
    }

    #[test]
    fn an_epoch_day_count_is_right() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2026, 10, 3), 20729);
    }

    #[test]
    fn a_redis_url_is_split_into_address_and_password() {
        let redis = Redis::from_url("redis://:secret@redis:6379/0").unwrap();
        assert_eq!(redis.address, "redis:6379");
        assert_eq!(redis.password.as_deref(), Some("secret"));

        let plain = Redis::from_url("redis://127.0.0.1:6379").unwrap();
        assert_eq!(plain.address, "127.0.0.1:6379");
        assert_eq!(plain.password, None);

        assert!(Redis::from_url("postgres://x").is_none());
    }

    #[test]
    fn resp_replies_are_parsed() {
        let raw = b"*2\r\n$5\r\nhello\r\n$5\r\nworld\r\n";
        assert_eq!(
            Value::parse(raw),
            Some(Value::Array(vec![
                Value::Bulk(b"hello".to_vec()),
                Value::Bulk(b"world".to_vec()),
            ]))
        );
        assert_eq!(Value::parse(b"+OK\r\n"), Some(Value::Simple("OK".to_string())));
        assert_eq!(Value::parse(b"$-1\r\n"), Some(Value::Null));
        // A partial reply must ask for more bytes rather than guess.
        assert_eq!(Value::parse(b"*2\r\n$5\r\nhel"), None);
        assert_eq!(
            Value::parse(b"*2\r\n$1\r\na\r\n$3\r\nfoo\r\n"),
            Some(Value::Array(vec![
                Value::Bulk(b"a".to_vec()),
                Value::Bulk(b"foo".to_vec())
            ]))
        );
    }

    #[test]
    fn errors_print_without_the_key() {
        assert_eq!(Error::Http(401).to_string(), "HTTP 401 · the API key has to be an admin's");

        // §9: a message that carries a URL must not carry a credential either. The key goes in
        // a header, never in a query string, and the scrubber drops the query part anyway.
        let text = short("error sending request for url (http://redash/api/admin?key=abc)");
        assert!(!text.contains("abc"), "no credentials in a message: {text}");
    }

    /// A smoke test against a real instance. Ignored by default because it needs REDASH_URL
    /// and REDASH_ADMIN_API_KEY; run it with
    /// `cargo test -- --ignored live_instance` when something looks wrong with parsing.
    #[test]
    #[ignore = "needs a live Redash"]
    fn live_instance_answers() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(live_instance_answers_inner());
    }

    async fn live_instance_answers_inner() {
        let config = crate::config::RedashConfig {
            url: Some(std::env::var("REDASH_URL").expect("REDASH_URL")),
            admin_api_key: Some(std::env::var("REDASH_ADMIN_API_KEY").expect("REDASH_ADMIN_API_KEY")),
            redis_url: std::env::var("REDIS_URL").ok(),
        };
        let source = RedashSource::new(&config).expect("configured");
        let status = source.status().await;
        assert!(status.reachable, "status was {status:?}");
        assert!(!status.queues.is_empty(), "no queues: {status:?}");
        for queue in &status.queues {
            eprintln!(
                "{}: running {}, waiting {}, stale {}, workers {}/{}",
                queue.name, queue.running, queue.waiting, queue.stale, queue.workers_busy, queue.workers_total
            );
        }
        for job in &status.jobs {
            eprintln!(
                "{:?} {} {} {} {:?}",
                job.state,
                job.label(),
                fmt(job.age_s),
                job.query_label(),
                job.data_source_type
            );
        }
    }

    fn fmt(seconds: u64) -> String {
        format!("{seconds}s")
    }

    #[test]
    fn redash_below_ten_is_celery() {
        let celery = |config: serde_json::Value| Instance::from_config(&config).celery;
        assert!(!celery(serde_json::json!({ "version": "10.1.0" })), "10 is RQ");
        assert!(!celery(serde_json::json!({ "version": "11.0.0" })));
        assert!(celery(serde_json::json!({ "version": "9.0.0" })), "9 is Celery");
        assert!(celery(serde_json::json!({ "version": "2.0.0" })));
        // No version at all: assume the modern shape and let the endpoint answer for itself.
        assert!(!celery(serde_json::json!({})));
        // Redash 10 keeps it under client_config.
        let nested = Instance::from_config(&serde_json::json!({ "client_config": { "version": "10.1.0" } }));
        assert_eq!(nested, Instance { version: Some("10.1.0".into()), celery: false });
    }

    /// What a busy Redash sends while someone runs a query from the editor without saving it:
    /// `query_id` is the text "adhoc". One such job used to fail the whole answer with
    /// "error decoding response body".
    const BUSY: &str = r#"{
      "queues": {
        "queries": {
          "name": "queries",
          "queued": "2",
          "started": [
            {"id": "j-adhoc", "name": "redash.tasks.queries.execution.execute_query", "origin": "queries",
             "enqueued_at": "2026-10-04T03:00:00.000", "started_at": "2026-10-04T03:00:01.000",
             "meta": {"data_source_id": 3, "org_id": 1, "scheduled": false, "query_id": "adhoc", "user_id": 7}},
            {"id": "j-saved", "origin": "queries", "started_at": "2026-10-04T03:00:02.000",
             "meta": {"data_source_id": "3", "query_id": 8585, "user_id": null}},
            {"id": "j-odd", "origin": "queries", "started_at": 1759546800, "meta": "not a map"},
            "not a job at all"
          ]
        },
        "scheduled_queries": {"name": "scheduled_queries", "queued": [{"id": "a"}, {"id": "b"}, {"id": "c"}], "started": 0}
      },
      "workers": [
        {"name": "w1", "queues": ["queries", "scheduled_queries"], "state": "busy", "last_heartbeat": "2026-10-04T03:00:05.000", "pid": 12, "current_job": "j-adhoc (execute_query)"},
        {"name": "w2", "queues": "queries", "state": "idle", "last_heartbeat": null}
      ]
    }"#;

    #[test]
    fn an_ad_hoc_query_does_not_break_the_answer() {
        let status: RqStatus = serde_json::from_str(BUSY).expect("every field is read for what it can give");
        let queries = &status.queues["queries"];
        assert_eq!(queries.queued, 2, "a count written as text");
        assert_eq!(queries.started.len(), 3, "the entry that is not a job is skipped, the others kept");
        let adhoc = queries.started[0].meta.as_ref().unwrap();
        assert_eq!(adhoc.query_id, QueryRef::Adhoc);
        assert_eq!((adhoc.user_id, adhoc.data_source_id), (Some(7), Some(3)));
        let saved = queries.started[1].meta.as_ref().unwrap();
        assert_eq!((saved.query_id.id(), saved.data_source_id, saved.user_id), (Some(8585), Some(3), None));
        assert!(queries.started[2].meta.is_none() && queries.started[2].started_at.as_deref() == Some("1759546800"));
        assert_eq!(status.queues["scheduled_queries"].queued, 3, "a list counts its entries");
        assert!(status.queues["scheduled_queries"].started.is_empty());
        assert_eq!(status.workers[0].queues, "queries,scheduled_queries", "a list of queues is joined");
        assert_eq!(status.workers[1].last_heartbeat, None);
    }

    #[test]
    fn an_answer_that_is_not_the_queue_says_what_it_is() {
        let html = "<!DOCTYPE html><html><head><title>Login to Redash</title></head></html>";
        let error = serde_json::from_str::<RqStatus>(html).unwrap_err();
        let said = Error::Body(why_unreadable(html, &error)).to_string();
        assert_eq!(said, "unreadable answer: a web page, not JSON — check the url and that the API key is an admin's");
        assert_eq!(Error::Http(403).to_string(), "HTTP 403 · the API key has to be an admin's");
        assert!(Error::Http(404).to_string().contains("is the url Redash's own"));
        assert_eq!(Error::Http(502).to_string(), "HTTP 502");
    }

    #[test]
    fn only_a_job_a_worker_holds_is_running() {
        let day = STALE_AFTER.as_secs();
        // Held: running, whatever its age or flags — a worker is busy with it.
        assert_eq!(liveness(true, false, 30, true), JobState::Started);
        assert_eq!(liveness(true, true, day * 100, true), JobState::Started);
        // Not held, and every worker of the queue accounted for.
        assert_eq!(liveness(false, true, 5, true), JobState::Stale(Stale::Cancelled));
        assert_eq!(liveness(false, false, day + 1, true), JobState::Stale(Stale::OverADay));
        assert_eq!(liveness(false, false, 772, true), JobState::Stale(Stale::NoWorker));
        assert_eq!(liveness(false, false, 20, true), JobState::Started, "too young to judge");
        // Workers that cannot say what they hold: only the day and the cancel count.
        assert_eq!(liveness(false, false, 772, false), JobState::Started);
        assert_eq!(liveness(false, false, day + 1, false), JobState::Stale(Stale::OverADay));
    }

    #[test]
    fn an_idle_worker_beats_rarely_and_is_still_there() {
        let worker = |heartbeat: String| RqWorker {
            name: "w1".into(),
            state: "idle".into(),
            current_job: None,
            queues: "queries".into(),
            last_heartbeat: Some(heartbeat),
        };
        let now = SystemTime::now();
        assert!(is_live(&worker(ago(400)), now), "RQ's idle worker beats every 405 s");
        assert!(!is_live(&worker(ago(600)), now), "RQ itself forgets it after 480 s");
        let ahead = format!("{}Z", &ago(0)[..19]).replace(&ago(0)[..4], &(ago(0)[..4].parse::<u32>().unwrap() + 1).to_string());
        assert!(is_live(&worker(ahead), now), "a clock that runs ahead is not a dead worker");
        assert_eq!(held_job(&RqWorker { current_job: Some("j-1 (execute_query)".into()), ..worker(ago(1)) }).as_deref(), Some("j-1"));
    }

    #[test]
    fn a_waiting_job_is_read_from_its_call_string() {
        let call = r#"redash.tasks.queries.execution.execute_query('SELECT 1 /* it\'s, 9 */', 3, {'Username': 'a@example.net', 'query_id': 7438}, user_id=42, scheduled_query_id=None, is_api_key=False)"#;
        let meta = meta_from_call(call);
        assert_eq!((meta.query_id, meta.user_id, meta.data_source_id, meta.scheduled), (QueryRef::Saved(7438), Some(42), Some(3), false));
        let scheduled = meta_from_call(r#"execute_query("SELECT 'x'", 12, {'query_id': 'adhoc'}, user_id=7, scheduled_query_id=8585)"#);
        assert_eq!((scheduled.query_id, scheduled.data_source_id, scheduled.scheduled), (QueryRef::Adhoc, Some(12), true));
        assert_eq!(meta_from_call("something else").query_id, QueryRef::Unknown);
        assert_eq!(task_label("redash.tasks.queries.maintenance.refresh_schema").as_deref(), Some("refresh_schema"));
        assert_eq!(task_label("redash.tasks.queries.execution.execute_query"), None);
        assert_eq!(task_label(""), None);
    }

    /// `now - seconds`, written the way RQ writes it.
    fn ago(seconds: u64) -> String {
        let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() - seconds;
        let (days, rest) = ((t / 86_400) as i64, t % 86_400);
        // Howard Hinnant's civil_from_days, the inverse of `days_from_civil`.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000", rest / 3600, rest % 3600 / 60, rest % 60)
    }

    type Requests = std::sync::Arc<Mutex<Vec<(String, String)>>>;

    /// Enough of a Redash on a free local port: a GET is answered from `routes` by its path,
    /// another method by `METHOD path`, anything else is a 404, and the path (`METHOD path`
    /// for all but a GET) and `Authorization` of every request are kept.
    fn fake_redash(routes: Vec<(String, String)>) -> (String, Requests) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen: Requests = Default::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).ok();
                let mut words = request.split_whitespace();
                let method = words.next().unwrap_or_default().to_string();
                let path = words.next().unwrap_or_default().to_string();
                let path = if method == "GET" { path } else { format!("{method} {path}") };
                let mut auth = String::new();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.trim().eq_ignore_ascii_case("authorization")
                    {
                        auth = value.trim().to_string();
                    }
                }
                log.lock().unwrap().push((path.clone(), auth));
                let (status, body) = routes
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or((404, "{}".to_string()), |(_, body)| (200, body.clone()));
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).ok();
            }
        });
        (url, seen)
    }

    fn config(url: &str) -> RedashConfig {
        RedashConfig { url: Some(url.to_string()), admin_api_key: Some("test-key".into()), redis_url: None }
    }

    /// The instance on the screenshot that started this: a quiet Redash whose started list
    /// is full of leftovers — two from months ago (one cancelled), two from twelve minutes ago
    /// that no worker holds (the one that claims one stopped beating an hour ago) — and one
    /// scheduled refresh that really runs.
    fn quiet_redash_with_leftovers() -> Vec<(String, String)> {
        let rq_status = format!(
            r#"{{
              "queues": {{
                "queries": {{"name": "queries", "queued": 0, "started": [
                  {{"id": "z1", "origin": "queries", "started_at": "{z1}", "meta": {{"query_id": 5120, "user_id": 11, "data_source_id": 1, "scheduled": false}}}},
                  {{"id": "z2", "origin": "queries", "started_at": "{z2}", "meta": {{"query_id": 6301, "user_id": 12, "data_source_id": 2, "cancelled": true}}}},
                  {{"id": "o1", "origin": "queries", "started_at": "{o1}", "meta": {{"query_id": 161, "user_id": 13, "data_source_id": 3}}}},
                  {{"id": "o2", "origin": "queries", "started_at": "{o2}", "meta": {{"query_id": "adhoc", "user_id": 13, "data_source_id": 3}}}}
                ]}},
                "scheduled_queries": {{"name": "scheduled_queries", "queued": 0, "started": [
                  {{"id": "s1", "origin": "scheduled_queries", "started_at": "{s1}", "meta": {{"query_id": 8585, "user_id": 12, "data_source_id": 1, "scheduled": true}}}}
                ]}},
                "emails": {{"name": "emails", "queued": 0, "started": []}}
              }},
              "workers": [
                {{"name": "adhoc-1", "queues": "queries", "state": "idle", "current_job": null, "last_heartbeat": "{idle}"}},
                {{"name": "sched-1", "queues": "scheduled_queries, schemas", "state": "busy", "current_job": "s1 (execute_query)", "last_heartbeat": "{now}"}},
                {{"name": "sched-2", "queues": "scheduled_queries, schemas", "state": "idle", "current_job": null, "last_heartbeat": "{idle}"}},
                {{"name": "gone", "queues": "queries", "state": "busy", "current_job": "o1 (execute_query)", "last_heartbeat": "{gone}"}}
              ]
            }}"#,
            z1 = ago(177 * 86_400),
            z2 = ago(173 * 86_400),
            o1 = ago(772),
            o2 = ago(742),
            s1 = ago(95),
            idle = ago(300),
            now = ago(2),
            gone = ago(3600),
        );
        let route = |path: &str, body: &str| (path.to_string(), body.to_string());
        vec![
            route("/api/config", r#"{"org_slug": "default", "client_config": {"version": "10.1.0"}}"#),
            ("/api/admin/queries/rq_status".to_string(), rq_status),
            route("/api/users/11", r#"{"id": 11, "name": "Grigol Gankava", "email": "grigol.gankava@example.net"}"#),
            route("/api/users/12", r#"{"id": 12, "name": "Jana Petrova", "email": "j.petrova@example.net"}"#),
            route("/api/users/13", r#"{"id": 13, "name": "Darius", "email": null}"#),
            route("/api/queries/5120", r#"{"id": 5120, "name": "Transfers · weekly summary", "query": "SELECT 1", "data_source_id": 1}"#),
            route("/api/queries/6301", r#"{"id": 6301, "name": "Card margin · by day", "query": "SELECT 2", "data_source_id": 2}"#),
            route("/api/queries/161", r#"{"id": 161, "name": "Replica health check", "query": "SELECT * FROM query_12", "data_source_id": 3}"#),
            route("/api/queries/8585", r#"{"id": 8585, "name": "AML dashboard", "query": "SELECT count() FROM aml", "data_source_id": 1}"#),
            route(
                "/api/data_sources",
                r#"[{"id": 1, "name": "clickhouse-bi", "type": "clickhouse"},
                    {"id": 2, "name": "payments-mysql", "type": "mysql"},
                    {"id": 3, "name": "Query Results", "type": "results"}]"#,
            ),
        ]
    }

    #[tokio::test]
    async fn a_job_is_cancelled_with_a_delete_of_it_and_nothing_else_is_sent() {
        let mut routes = quiet_redash_with_leftovers();
        routes.push(("DELETE /api/jobs/s1".to_string(), "null".to_string()));
        let (url, requests) = fake_redash(routes);
        let source = RedashSource::new(&config(&url)).unwrap();
        assert_eq!(source.cancel("s1").await, Ok(()));
        // One that Redash no longer has, and one that is not an id at all.
        assert_eq!(source.cancel("gone-1").await, Err(Error::Http(404)));
        assert!(cancel_error(&Error::Http(500)).contains("may have just finished"));
        assert_eq!(source.cancel("../admin").await, Err(Error::NotAJob));
        let seen = requests.lock().unwrap().clone();
        assert_eq!(seen, [
            ("DELETE /api/jobs/s1".to_string(), "Key test-key".to_string()),
            ("DELETE /api/jobs/gone-1".to_string(), "Key test-key".to_string()),
        ], "the key in the header only, and nothing sent for a path that is not an id");

        // Through the loop: asked once, answered, and the queue read again straight away.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(source.run(tx, cancel_rx));
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap();
        assert!(matches!(first, Some(Event::Queue(_))), "the first poll");
        cancel_tx.send("s1".to_string()).unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap();
        assert!(matches!(&answer, Some(Event::Cancelled(id, Ok(()))) if id == "s1"), "{answer:?}");
        let again = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap();
        assert!(matches!(again, Some(Event::Queue(_))), "read again before the next tick");
        task.abort();
    }

    #[tokio::test]
    async fn leftovers_in_the_started_list_are_not_running() {
        let (url, requests) = fake_redash(quiet_redash_with_leftovers());
        let source = RedashSource::new(&config(&url)).unwrap();
        let status = source.status().await;
        assert!(status.reachable, "{status:?}");
        assert_eq!(status.version.as_deref(), Some("10.1.0"));

        let queries = status.queue("queries").unwrap();
        assert_eq!((queries.running, queries.stale, queries.waiting), (0, 4, 0), "{queries:?}");
        assert_eq!((queries.workers_busy, queries.workers_total), (0, 1), "the dead worker is not counted");
        let scheduled = status.queue("scheduled_queries").unwrap();
        assert_eq!((scheduled.running, scheduled.stale), (1, 0));
        assert_eq!((scheduled.workers_busy, scheduled.workers_total), (1, 2));
        assert!(status.queue("emails").unwrap().is_idle());
        assert_eq!((status.workers_busy, status.workers_total), (1, 3), "every live worker once");

        let job = |id: &str| status.jobs.iter().find(|j| j.id == id).unwrap();
        assert_eq!(job("z1").state, JobState::Stale(Stale::OverADay));
        assert_eq!(job("z2").state, JobState::Stale(Stale::Cancelled));
        assert_eq!(job("o1").state, JobState::Stale(Stale::NoWorker));
        assert_eq!(job("o2").state, JobState::Stale(Stale::NoWorker));
        assert_eq!(job("s1").state, JobState::Started);

        // Who, what and on what — the script's three lookups.
        let s1 = job("s1");
        assert_eq!(s1.person.as_deref(), Some("j.petrova"), "shown the way view 1 shows people");
        assert_eq!(s1.person_full.as_deref(), Some("Jana Petrova <j.petrova@example.net>"));
        assert_eq!((s1.data_source.as_deref(), s1.data_source_type.as_deref()), (Some("clickhouse-bi"), Some("clickhouse")));
        assert_eq!((s1.query_label().as_str(), s1.sql.as_deref()), ("#8585 AML dashboard", Some("SELECT count() FROM aml")));
        assert!(s1.scheduled && s1.on_clickhouse() == Some(true));
        let o2 = job("o2");
        assert!(o2.adhoc && o2.sql.is_none(), "an ad-hoc query has no text in the API");
        assert_eq!(o2.query_label(), "ad-hoc query (not saved)");
        assert_eq!((o2.person.as_deref(), o2.data_source_type.as_deref()), (Some("Darius"), Some("results")));
        assert_eq!(job("z2").on_clickhouse(), Some(false));

        // Every request carries the key, in the header and nowhere else; the list of data
        // sources is asked for once, and the next poll asks nothing it already knows.
        let first = requests.lock().unwrap().clone();
        assert!(first.iter().all(|(path, auth)| auth == "Key test-key" && !path.contains("test-key")));
        assert_eq!(first.iter().filter(|(p, _)| p == "/api/data_sources").count(), 1);
        assert!(!format!("{status:?}").contains("test-key"));
        source.status().await;
        let second: Vec<String> = requests.lock().unwrap()[first.len()..].iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(second, ["/api/admin/queries/rq_status"], "users, queries, data sources and the version are cached");
    }

    #[tokio::test]
    async fn a_job_a_worker_just_let_go_is_finishing_not_stale() {
        // No meta, so nothing is looked up and nothing listens on this port.
        let source = RedashSource::new(&config("http://127.0.0.1:9")).unwrap();
        let poll = |current: &str| {
            let current = if current.is_empty() { "null".to_string() } else { format!("\"{current}\"") };
            serde_json::from_str::<RqStatus>(&format!(
                r#"{{"queues": {{"queries": {{"name": "queries", "queued": 0, "started": [{{"id": "j1", "started_at": "{}"}}]}}}},
                    "workers": [{{"name": "w1", "queues": "queries", "state": "idle", "current_job": {current}, "last_heartbeat": "{}"}}]}}"#,
                ago(600),
                ago(1)
            ))
            .unwrap()
        };
        let state = |status: &QueueStatus| status.jobs[0].state;
        let now = SystemTime::now();
        assert_eq!(state(&source.build_rq_status(poll("j1 (execute_query)"), now).await), JobState::Started);
        // Redash read the started list a moment before the worker let go of it.
        assert_eq!(state(&source.build_rq_status(poll(""), now).await), JobState::Started);
        assert_eq!(state(&source.build_rq_status(poll(""), now).await), JobState::Stale(Stale::NoWorker));
    }
}
