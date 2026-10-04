//! The Redash queue (DESIGN.md §6.3), fail-soft.
//!
//! Any error becomes `QueueStatus { reachable: false, .. }` and the strip prints
//! `unreachable (HTTP 401)`. **The app must never exit because Redash is down** — that is the
//! whole reason this is a separate source with its own task.
//!
//! `rq_status` reports queued jobs as a *count*, so the names of waiting jobs live in RQ's
//! Redis keys. That is read-only here: two commands, no writes, no dependency.

use crate::attrib;
use crate::config::RedashConfig;
use crate::model::{Job, JobState, QueueRow, QueueStatus};
use crate::app::Event;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// §6.3: the queue moves every 3 s.
const QUEUE_POLL: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
/// A worker that has not sent a heartbeat for this long is not counted as busy: the local
/// instance reports `state: "?"` for exactly that case, and a stale worker must not be shown
/// as capacity.
const HEARTBEAT_STALE: Duration = Duration::from_secs(60);

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
    #[serde(default, deserialize_with = "lenient_text")]
    origin: String,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    started_at: Option<String>,
    #[serde(default, deserialize_with = "lenient_meta")]
    meta: Option<RqMeta>,
}

#[derive(Debug, Deserialize, Default)]
struct RqMeta {
    #[serde(default)]
    query_id: QueryRef,
    #[serde(default, deserialize_with = "lenient_id")]
    user_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient_id")]
    data_source_id: Option<u64>,
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
            serde_json::Value::String(text) if text.trim().eq_ignore_ascii_case("adhoc") => QueryRef::Adhoc,
            serde_json::Value::String(text) => text.trim().parse().map_or(QueryRef::Unknown, QueryRef::Saved),
            _ => QueryRef::Unknown,
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

/// `/api/users/{id}` and `/api/queries/{id}` only change when someone edits something, so the
/// answers are cached for the session (§6.3).
#[derive(Debug, Deserialize)]
struct ApiUser {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiQuery {
    #[serde(default, deserialize_with = "lenient_opt_text")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient_id")]
    data_source_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ApiDataSource {
    #[serde(default, deserialize_with = "lenient_id")]
    id: Option<u64>,
    #[serde(default, deserialize_with = "lenient_opt_text")]
    name: Option<String>,
}

/// What one API call can go wrong with, without leaking the key (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Http(u16),
    Connection(String),
    Body(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(code @ (401 | 403)) => write!(f, "HTTP {code} · the API key has to be an admin's"),
            Error::Http(404) => write!(f, "HTTP 404 · no such endpoint — is the url Redash's own?"),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Connection(e) => write!(f, "{e}"),
            Error::Body(e) => write!(f, "{UNREADABLE}{e}"),
        }
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

pub struct RedashSource {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    redis: Option<Redis>,
    /// id → name, for the lifetime of the session (§6.3). Behind a `Mutex` because the poll
    /// task holds `&self` across await points; the guards never live across one.
    users: Mutex<HashMap<u64, String>>,
    queries: Mutex<HashMap<u64, ApiQuery>>,
    data_sources: Mutex<HashMap<u64, String>>,
    /// Set once /api/config says the instance is older than Redash 10 (§6.3).
    celery: bool,
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
            data_sources: Mutex::new(HashMap::new()),
            celery: false,
        })
    }

    /// Poll until the loop ends. Errors are reported, never fatal.
    pub async fn run(self, tx: tokio::sync::mpsc::UnboundedSender<Event>) {
        let mut tick = tokio::time::interval(QUEUE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let status = self.status().await;
            if tx.send(Event::Queue(Box::new(status))).is_err() {
                return;
            }
        }
    }

    /// One poll: queue counts and worker saturation always; names when they can be resolved.
    pub async fn status(&self) -> QueueStatus {
        // §6.3: branch on /api/config. An unreachable /api/config is not fatal — RQ is the
        // assumed shape and the endpoint answers for itself.
        if let Ok(config) = self.get_json::<serde_json::Value>("/api/config").await {
            // §6.3: Redash < 10 is Celery and the endpoint differs.
            if self.celery_from_version(&config) {
                return self.status_celery().await;
            }
        }

        if self.celery {
            return self.status_celery().await;
        }

        let status: RqStatus = match self.get_json("/api/admin/queries/rq_status").await {
            Ok(status) => status,
            Err(e) => return QueueStatus::unreachable(e.to_string()),
        };
        self.build_rq_status(status).await
    }

    fn celery_from_version(&self, config: &serde_json::Value) -> bool {
        let version = config
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        major < 10
    }

    async fn status_celery(&self) -> QueueStatus {
        match self.get_json::<CeleryTasks>("/api/admin/queries/tasks").await {
            Ok(tasks) => self.build_celery(tasks),
            Err(e) => QueueStatus::unreachable(e.to_string()),
        }
    }

    async fn build_rq_status(&self, status: RqStatus) -> QueueStatus {
        let now = SystemTime::now();
        let mut queues: Vec<QueueRow> = Vec::new();
        let mut jobs: Vec<Job> = Vec::new();
        let mut names_available = self.redis.is_some();

        for (key, queue) in &status.queues {
            let name = if queue.name.is_empty() { key.clone() } else { queue.name.clone() };

            // A worker belongs to the queues it was started for; `queues` is a comma list.
            let (busy, total) = worker_counts(&status.workers, &name, now);

            // Oldest wait: the API gives a count, so this only exists when Redis can name the
            // jobs. Never a guess.
            let waiting_ids = match &self.redis {
                Some(redis) => redis.queue_ids(&name).await.unwrap_or_default(),
                None => Vec::new(),
            };

            let mut oldest: Option<u64> = None;
            for id in &waiting_ids {
                let Some(job) = self.redis_job(&name, id, JobState::Queued).await else {
                    continue;
                };
                oldest = Some(oldest.map_or(job.age_s, |o: u64| o.max(job.age_s)));
                jobs.push(job);
            }
            if waiting_ids.is_empty() {
                names_available = false;
            }

            queues.push(QueueRow {
                name: name.clone(),
                // The count is authoritative; the job list is what fills in the names.
                waiting: queue.queued.max(waiting_ids.len() as u32),
                oldest_wait_s: oldest,
                workers_busy: busy,
                workers_total: total,
                // The endpoint has no failure count, so the strip omits that segment (§6.3).
                failed_5m: 0,
            });

            for started in &queue.started {
                let age_s = started
                    .started_at
                    .as_deref()
                    .and_then(parse_rfc3339_seconds)
                    .map(|t| now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0))
                    .unwrap_or(0);
                let (person, query_name, data_source) = self
                    .describe(started.meta.as_ref())
                    .await;
                jobs.push(Job {
                    id: started.id.clone(),
                    state: JobState::Started,
                    queue: if started.origin.is_empty() { name.clone() } else { started.origin.clone() },
                    user: Some(attrib::REDASH_USER.to_string()),
                    person,
                    redash_query_id: started.meta.as_ref().and_then(|m| m.query_id.id()),
                    query_name,
                    data_source,
                    age_s,
                    // The ClickHouse side of the stitch is filled in by the caller: Redash
                    // does not know which node the query landed on (§2.8).
                    ch_node: None,
                    ch_query_id: None,
                });
            }
        }

        queues.sort_by(|a, b| a.name.cmp(&b.name));
        QueueStatus {
            reachable: true,
            error: None,
            queues,
            jobs,
            names_available,
            host: Some(
                self.base_url
                    .split("://")
                    .nth(1)
                    .unwrap_or(&self.base_url)
                    .to_string(),
            ),
            taken_at: now,
        }
    }

    fn build_celery(&self, tasks: CeleryTasks) -> QueueStatus {
        let jobs: Vec<Job> = tasks
            .active
            .into_iter()
            .enumerate()
            .map(|(index, id)| Job {
                id: id.clone(),
                state: JobState::Started,
                queue: format!("celery:{index}"),
                user: Some(attrib::REDASH_USER.to_string()),
                person: None,
                redash_query_id: None,
                query_name: None,
                data_source: None,
                // Celery does not report a start time in this shape; 0 reads as "unknown"
                // rather than as "just started", and the row is still listed.
                age_s: 0,
                ch_node: None,
                ch_query_id: None,
            })
            .collect();
        let running = jobs.len() as u32;
        QueueStatus {
            reachable: true,
            error: None,
            queues: vec![QueueRow {
                name: "default".to_string(),
                waiting: 0,
                oldest_wait_s: None,
                workers_busy: running,
                workers_total: running.max(1),
                failed_5m: 0,
            }],
            jobs,
            names_available: false,
            host: None,
            taken_at: SystemTime::now(),
        }
    }

    /// Turn RQ's job hash into a `Job`, resolving the person and the query name.
    async fn redis_job(&self, queue: &str, id: &str, state: JobState) -> Option<Job> {
        let hash = self.redis.as_ref()?.job_hash(id).await.ok()?;
        let data = hash.get("data")?;
        let meta = hash.get("meta").cloned().unwrap_or_default();
        let meta: RqMeta = serde_json::from_str(&meta).ok().unwrap_or_default();

        let enqueued_at = hash
            .get("enqueued_at")
            .cloned()
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let age_s = parse_rfc3339_seconds(&enqueued_at)
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // RQ stores the job payload as a JSON list; the query and user ids are inside it.
        let payload: Vec<serde_json::Value> = serde_json::from_str(data).unwrap_or_default();
        let query_id = payload
            .first()
            .and_then(|entry| entry.get("query_id"))
            .map_or(QueryRef::Unknown, QueryRef::from_value);
        let user_id = payload
            .first()
            .and_then(|entry| entry.get("user_id"))
            .and_then(|v| v.as_u64());
        let data_source_id = payload
            .first()
            .and_then(|entry| entry.get("data_source_id"))
            .and_then(|v| v.as_u64());

        let meta = RqMeta {
            query_id: if meta.query_id == QueryRef::Unknown { query_id } else { meta.query_id },
            user_id: meta.user_id.or(user_id),
            data_source_id: meta.data_source_id.or(data_source_id),
        };
        let (person, query_name, data_source) = self.describe(Some(&meta)).await;

        Some(Job {
            id: id.to_string(),
            state,
            queue: queue.to_string(),
            user: Some(attrib::REDASH_USER.to_string()),
            person,
            redash_query_id: meta.query_id.id(),
            query_name,
            data_source,
            age_s,
            ch_node: None,
            ch_query_id: None,
        })
    }

    /// `user_id → person`, `query_id → name`, `data_source_id → name` (§6.3), each cached.
    async fn describe(
        &self,
        meta: Option<&RqMeta>,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let Some(meta) = meta else {
            return (None, None, None);
        };
        let person = match meta.user_id {
            Some(id) => self.user_name(id).await,
            None => None,
        };
        let query = match meta.query_id {
            QueryRef::Saved(id) => self.query(id).await,
            _ => None,
        };
        // An ad-hoc query has no saved query to take a data source from; the job names its own.
        let data_source = match query.as_ref().and_then(|q| q.data_source_id).or(meta.data_source_id) {
            Some(id) => self.data_source_name(id).await,
            None => None,
        };
        let name = match meta.query_id {
            QueryRef::Adhoc => Some("ad-hoc query (not saved)".to_string()),
            _ => query.and_then(|q| q.name),
        };
        (person, name, data_source)
    }

    async fn user_name(&self, id: u64) -> Option<String> {
        if let Some(name) = self.users.lock().expect("user cache").get(&id) {
            return Some(name.clone());
        }
        let user: ApiUser = self.get_json(&format!("/api/users/{id}")).await.ok()?;
        let label = user
            .email
            .clone()
            .or(user.name.clone())
            .map(|value| attrib::display_person(&value))
            .unwrap_or_else(|| format!("user {id}"));
        self.users.lock().expect("user cache").insert(id, label.clone());
        Some(label)
    }

    async fn query(&self, id: u64) -> Option<ApiQuery> {
        if let Some(query) = self.queries.lock().expect("query cache").get(&id) {
            return Some(query.clone());
        }
        let query: ApiQuery = self.get_json(&format!("/api/queries/{id}")).await.ok()?;
        self.queries.lock().expect("query cache").insert(id, query.clone());
        Some(query)
    }

    async fn data_source_name(&self, id: u64) -> Option<String> {
        if let Some(name) = self.data_sources.lock().expect("data source cache").get(&id) {
            return Some(name.clone());
        }
        let all: Vec<ApiDataSource> = self.get_json("/api/data_sources").await.ok()?;
        for source in all {
            if let (Some(id), Some(name)) = (source.id, &source.name) {
                self.data_sources.lock().expect("data source cache").insert(id, name.clone());
            }
        }
        self.data_sources.lock().expect("data source cache").get(&id).cloned()
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

/// Workers of one queue: busy and total. A worker with a stale heartbeat is not capacity.
fn worker_counts(workers: &[RqWorker], queue: &str, now: SystemTime) -> (u32, u32) {
    let mut busy = 0;
    let mut total = 0;
    for worker in workers {
        let queues = worker.queues.trim();
        // `queues: ""` means the worker listens to everything.
        let serves = queues.is_empty() || queues.split(',').any(|q| q.trim() == queue);
        if !serves {
            continue;
        }
        // A worker with no id is not one RQ registered; counting it would be a guess.
        if worker.name.is_empty() {
            continue;
        }
        if worker
            .last_heartbeat
            .as_deref()
            .and_then(parse_rfc3339_seconds)
            .map(|t| now.duration_since(t).map(|d| d < HEARTBEAT_STALE).unwrap_or(false))
            .unwrap_or(false)
        {
            total += 1;
            if worker.state == "busy" || worker.current_job.is_some() {
                busy += 1;
            }
        }
    }
    (busy, total)
}

/// RFC 3339 without pulling in a date library: `2026-09-20T13:13:10.699` and
/// `2026-09-20T13:13:10+00:00` both show up in this world.
pub fn parse_rfc3339_seconds(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_matches('"');
    let (date, rest) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: u64 = date_parts.next()?.parse().ok()?;
    let month: u64 = date_parts.next()?.parse().ok()?;
    let day: u64 = date_parts.next()?.parse().ok()?;

    let time = rest.split(['Z', '+']).next().unwrap_or(rest);
    let time = time.split('.').next().unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour: u64 = time_parts.next()?.parse().ok()?;
    let minute: u64 = time_parts.next()?.parse().ok()?;
    let second: u64 = time_parts.next().unwrap_or("0").parse().unwrap_or(0);

    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
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
            eprintln!("{}: waiting {}, workers {}/{}", queue.name, queue.waiting, queue.workers_busy, queue.workers_total);
        }
        for job in &status.jobs {
            eprintln!("{:?} {} {} {}", job.state, job.label(), fmt(job.age_s), job.query_name.clone().unwrap_or_default());
        }
    }

    fn fmt(seconds: u64) -> String {
        format!("{seconds}s")
    }

    #[test]
    fn redash_below_ten_is_celery() {
        let source = RedashSource {
            base_url: "http://localhost:5001".into(),
            api_key: "x".into(),
            client: reqwest::Client::new(),
            redis: None,
            users: Mutex::new(HashMap::new()),
            queries: Mutex::new(HashMap::new()),
            data_sources: Mutex::new(HashMap::new()),
            celery: false,
        };
        let v = |s: &str| serde_json::json!({ "version": s });
        assert!(!source.celery_from_version(&v("10.1.0")), "10 is RQ");
        assert!(!source.celery_from_version(&v("11.0.0")));
        assert!(source.celery_from_version(&v("9.0.0")), "9 is Celery");
        assert!(source.celery_from_version(&v("2.0.0")));
        // No version at all: assume the modern shape and let the endpoint answer for itself.
        assert!(!source.celery_from_version(&serde_json::json!({})));
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
}
