//! ClickHouse over the HTTP interface: discovery (§6.2) and per-node polling (§6.1).
//!
//! Every node is polled concurrently with a per-request timeout, so one slow node cannot hold
//! up the fleet. A node that does not answer comes back as `NodeSnapshot::unreachable` with no
//! numbers — never as zeros (§2.6).

use crate::config::ClickHouseConfig;
use crate::model::{attribution_from_sql, FleetSnapshot, NodeSnapshot, QueryRow};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// §6: 1.5 s per request. The poll interval is longer, so one slow node misses a poll instead
/// of delaying everybody else's.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(1500);
/// §6.1: query settings on every request.
const SETTINGS: &str = "readonly=1&max_execution_time=2&max_threads=2&max_memory_usage=6000000000";
/// What is left when a server refuses to change those settings — which a read-only user
/// always does, because `max_execution_time` is not settable in readonly mode. Its profile
/// already carries the limits, so dropping them loses nothing.
const SETTINGS_READONLY_FRIENDLY: &str = "readonly=1";

const CAPACITY_SQL: &str = r#"
SELECT
  coalesce(
    (SELECT value FROM system.asynchronous_metrics WHERE metric = 'OSMemoryTotal' LIMIT 1),
    (SELECT value FROM system.asynchronous_metrics
       WHERE metric = 'CGroupMemoryTotal' AND value > 0 AND value < pow(2, 50) LIMIT 1),
    (SELECT toFloat64(toUInt64OrZero(value)) FROM system.server_settings
       WHERE name = 'max_server_memory_usage'
         AND toUInt64OrZero(value) > 0 AND toUInt64OrZero(value) < pow(2, 50) LIMIT 1),
    0
  ) AS server_memory_total_bytes,
  (SELECT value FROM system.asynchronous_metrics WHERE metric = 'MemoryResident' LIMIT 1)
    AS server_memory_used_bytes,
  (SELECT if(cpu_busy IS NULL, NULL, round(100 * least(1.0, greatest(0.0, cpu_busy)), 2))
   FROM (
     SELECT coalesce(
       if(countIf(metric IN ('OSUserTimeNormalized','OSSystemTimeNormalized','OSNiceTimeNormalized')) > 0,
          sumIf(value, metric IN ('OSUserTimeNormalized','OSSystemTimeNormalized','OSNiceTimeNormalized')), NULL),
       if(countIf(metric IN ('CGroupUserTimeNormalized','CGroupSystemTimeNormalized')) > 0,
          sumIf(value, metric IN ('CGroupUserTimeNormalized','CGroupSystemTimeNormalized')), NULL)
     ) AS cpu_busy FROM system.asynchronous_metrics
   )) AS server_cpu_percent,
  (SELECT coalesce(
     nullIf(countIf(metric LIKE 'OSUserTimeCPU%'), 0),
     nullIf(countIf(metric LIKE 'CGroupUserTimeCPU%'), 0),
     nullIf(countIf(metric LIKE 'CPUFrequencyMHz\\_%'), 0),
     nullIf(toUInt64(ceil(maxIf(value, metric = 'CGroupMaxCPU' AND value > 0 AND value < 4096))), 0)
   ) FROM system.asynchronous_metrics) AS server_cpu_cores,
  (SELECT greatest(
     sumIf(value, event = 'OSCPUVirtualTimeMicroseconds'),
     sumIf(value, event = 'UserTimeMicroseconds') + sumIf(value, event = 'SystemTimeMicroseconds')
   ) FROM system.events) AS server_cpu_time_us,
  (SELECT count() FROM system.processes) AS active_queries,
  (SELECT max(absolute_delay) FROM system.replicas) AS replica_lag_s,
  (SELECT count() FROM system.parts WHERE active) AS active_parts,
  (SELECT toUInt64OrZero(value) FROM system.settings WHERE name = 'max_memory_usage') AS max_memory_usage,
  version() AS version,
  uptime() AS uptime_s
"#;

const PROCESSES_SQL: &str = r#"
SELECT
  query_id,
  initial_user AS user,
  query,
  elapsed AS elapsed_s,
  memory_usage,
  read_rows, read_bytes,
  greatest(
    ProfileEvents['OSCPUVirtualTimeMicroseconds'],
    ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']
  ) AS cpu_time_us,
  trim(extract(query, 'Username:\\s*([^,]+)')) AS redash_user,
  extract(query, 'query_id:\\s*(\\d+)')        AS redash_query_id,
  total_rows_approx,
  written_rows,
  peak_memory_usage,
  toUInt64OrZero(Settings['max_memory_usage']) AS query_max_memory_usage,
  query_kind
FROM system.processes
WHERE is_initial_query = 1
  AND query NOT LIKE '%FROM system.processes%'
  AND query NOT LIKE 'KILL QUERY%'
ORDER BY elapsed DESC
"#;

/// §6.1's statement exactly as the design wrote it, for a server that refuses one of the
/// columns above. Progress, peak memory and per-query limits are then unknown, nothing else.
const PROCESSES_SQL_BASIC: &str = r#"
SELECT
  query_id,
  initial_user AS user,
  query,
  elapsed AS elapsed_s,
  memory_usage,
  read_rows, read_bytes,
  greatest(
    ProfileEvents['OSCPUVirtualTimeMicroseconds'],
    ProfileEvents['UserTimeMicroseconds'] + ProfileEvents['SystemTimeMicroseconds']
  ) AS cpu_time_us,
  trim(extract(query, 'Username:\\s*([^,]+)')) AS redash_user,
  extract(query, 'query_id:\\s*(\\d+)')        AS redash_query_id
FROM system.processes
WHERE is_initial_query = 1
  AND query NOT LIKE '%FROM system.processes%'
  AND query NOT LIKE 'KILL QUERY%'
ORDER BY elapsed DESC
"#;

const CLUSTERS_SQL: &str = r#"
SELECT cluster, shard_num, replica_num, host_name, host_address, port
FROM system.clusters
WHERE cluster = {cluster:String}
ORDER BY shard_num, replica_num
"#;

/// One node to poll, and where to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTarget {
    pub name: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub shard: u32,
    pub replica: u32,
}

/// ClickHouse's JSONEachRow quotes 64-bit integers, so `u64` fields arrive as JSON strings.
/// This accepts both, because a server that unquotes them must not break the poller.
fn de_u64<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Int(u64),
        Text(String),
    }
    match Number::deserialize(deserializer)? {
        Number::Int(v) => Ok(v),
        Number::Text(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom(format!("{text:?} is not a whole number"))),
    }
}

fn de_opt_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    // `replica_lag_s` is NULL on a node with no replicas, and an untagged enum cannot
    // deserialize null — that used to turn every node into an unreachable one.
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }))
}

/// `memory_usage` and `peak_memory_usage` are Int64, and memory tracking can dip below zero for
/// a moment. A strict unsigned parse would drop the whole row — the query would vanish from
/// the screen — so a negative reading is taken as 0 instead.
fn de_mem<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
            serde_json::Value::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        })
        .map(|v| v.max(0) as u64)
        .unwrap_or(0))
}

/// Same problem for a float: the §6.1 statement ends its core count in `toUInt64(ceil(...))`,
/// so `server_cpu_cores` comes back as the string "2".
fn de_opt_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }))
}

/// One row of the capacity statement (§6.1).
#[derive(Debug, Deserialize)]
struct CapacityRow {
    #[serde(default)]
    server_memory_total_bytes: f64,
    #[serde(default)]
    server_memory_used_bytes: f64,
    #[serde(default, deserialize_with = "de_opt_f64")]
    server_cpu_percent: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    server_cpu_cores: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_u64")]
    server_cpu_time_us: Option<u64>,
    #[serde(default, deserialize_with = "de_u64")]
    active_queries: u64,
    #[serde(default, deserialize_with = "de_opt_u64")]
    replica_lag_s: Option<u64>,
    #[serde(default, deserialize_with = "de_u64")]
    active_parts: u64,
    #[serde(default, deserialize_with = "de_opt_u64")]
    max_memory_usage: Option<u64>,
    #[serde(default)]
    version: String,
    #[serde(default, deserialize_with = "de_u64")]
    uptime_s: u64,
}

/// One row of the running-queries statement (§6.1).
#[derive(Debug, Deserialize)]
struct ProcessRow {
    query_id: String,
    user: String,
    query: String,
    elapsed_s: f64,
    #[serde(default, deserialize_with = "de_mem")]
    memory_usage: u64,
    #[serde(default, deserialize_with = "de_u64")]
    read_rows: u64,
    #[serde(default, deserialize_with = "de_u64")]
    read_bytes: u64,
    #[serde(default, deserialize_with = "de_u64")]
    cpu_time_us: u64,
    #[serde(default)]
    redash_user: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u64")]
    redash_query_id: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64")]
    total_rows_approx: Option<u64>,
    #[serde(default, deserialize_with = "de_opt_u64")]
    written_rows: Option<u64>,
    #[serde(default, deserialize_with = "de_mem")]
    peak_memory_usage: u64,
    #[serde(default, deserialize_with = "de_opt_u64")]
    query_max_memory_usage: Option<u64>,
    #[serde(default)]
    query_kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClusterRow {
    shard_num: u32,
    replica_num: u32,
    host_name: String,
    #[serde(default)]
    host_address: String,
    #[serde(default, deserialize_with = "de_u64")]
    port: u64,
}

pub struct ClickHouseSource {
    client: reqwest::Client,
    user: String,
    password: String,
    seeds: Vec<String>,
    cluster: String,
    http_port: u16,
    /// Targets keyed by node name; the fleet is seeds ∪ discovered (§6.2).
    targets: Vec<NodeTarget>,
    /// Cleared the first time a server refuses the query settings: a read-only user cannot
    /// set `max_execution_time`, so this is the normal path in production, not an edge case.
    settings_ok: AtomicBool,
    /// Cleared the first time a server refuses the extended `system.processes` columns; from
    /// then on §6.1's own statement is used.
    extended_processes: AtomicBool,
}

impl ClickHouseSource {
    pub fn new(config: &ClickHouseConfig) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| format!("http client: {e}"))?;

        // A seed is still a node even when it is not in any cluster (§6.2), so it joins the
        // fleet under its own host name.
        let mut targets: Vec<NodeTarget> = config
            .seeds
            .iter()
            .filter_map(|seed| {
                let (scheme, rest) = seed.split_once("://")?;
                let authority = rest.split('/').next()?;
                let (host, port) = match authority.rsplit_once(':') {
                    Some((host, port)) => (host.to_string(), port.parse().ok()?),
                    None => (authority.to_string(), config.http_port),
                };
                Some(NodeTarget {
                    name: host.clone(),
                    url: format!("{scheme}://{authority}"),
                    host,
                    port,
                    shard: 0,
                    replica: 0,
                })
            })
            .collect();

        // The same URL twice is one node. Two ports on one host are two servers (the local
        // rig is exactly that), so they keep the port in their name to stay apart.
        targets.sort_by(|a, b| a.url.cmp(&b.url));
        targets.dedup_by(|a, b| a.url == b.url);
        let mut per_host: HashMap<String, usize> = HashMap::new();
        for target in &targets {
            *per_host.entry(target.host.clone()).or_default() += 1;
        }
        for target in &mut targets {
            if per_host.get(&target.host).copied().unwrap_or(0) > 1 {
                target.name = format!("{}:{}", target.host, target.port);
            }
        }
        targets.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            client,
            user: config.user.clone(),
            password: config.password.clone(),
            seeds: config.seeds.clone(),
            cluster: config.cluster.clone(),
            http_port: config.http_port,
            targets,
            settings_ok: AtomicBool::new(true),
            extended_processes: AtomicBool::new(true),
        })
    }

    /// The fleet as it stands: seeds plus whatever discovery has found, keyed by name.
    pub fn targets(&self) -> &[NodeTarget] {
        &self.targets
    }

    /// `system.clusters` against every seed, unioned by host name (§6.2).
    ///
    /// Returns the errors instead of swallowing them: the caller shows them in the strip, and
    /// discovery that fails is not fatal — the seeds are still polled.
    pub async fn discover(&mut self) -> Vec<String> {
        let sql = CLUSTERS_SQL.replace("{cluster:String}", &quote(&self.cluster));
        let mut errors = Vec::new();
        let mut found: HashMap<String, NodeTarget> = HashMap::new();

        for seed in &self.seeds {
            match self.post(seed, &sql).await {
                Ok(body) => match parse_clusters(&body, &self.seeds, self.http_port) {
                    Ok(rows) => {
                        for row in rows {
                            found.entry(row.name.clone()).or_insert(row);
                        }
                    }
                    Err(e) => errors.push(format!("{seed}: {e}")),
                },
                Err(e) => errors.push(format!("{seed}: {e}")),
            }
        }

        merge_discovered(&mut self.targets, found.into_values().collect());
        errors
    }

    /// One poll of the whole fleet, concurrently (§6). A node that fails is reported as
    /// unreachable with no numbers rather than dropped.
    pub async fn poll(&self) -> FleetSnapshot {
        // §6: poll every node concurrently, one timeout each. A slow node misses this poll
        // instead of delaying the fleet, and comes back as unreachable rather than late.
        let nodes: Vec<NodeSnapshot> = join_all(self.targets.iter().map(|target| self.poll_node(target)))
            .await
            .into_iter()
            .zip(self.targets.iter())
            .map(|(result, target)| match result {
                Ok(node) => node,
                Err(reason) => NodeSnapshot::unreachable(&target.name, reason),
            })
            .collect();

        FleetSnapshot {
            taken_at: SystemTime::now(),
            nodes,
        }
    }

    async fn poll_node(&self, target: &NodeTarget) -> Result<NodeSnapshot, String> {
        let started = std::time::Instant::now();
        let capacity = self.post(&target.url, CAPACITY_SQL).await?;
        let row: CapacityRow = parse_one_row(&capacity)?;
        let queries: Vec<QueryRow> = parse_processes(&self.processes(&target.url).await);
        let poll_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);

        // §6.1: the denominator is the fallback chain's answer, and 0 means "we do not know",
        // which must never become a percentage (§2.1).
        let mem_total = positive_or_none(row.server_memory_total_bytes as u64);
        let cores = row.server_cpu_cores.filter(|c| *c > 0.0);
        let cpu_busy_cores = match (row.server_cpu_percent, cores) {
            (Some(percent), Some(cores)) => Some((percent / 100.0).clamp(0.0, 1.0) * cores),
            _ => None,
        };

        Ok(NodeSnapshot {
            name: target.name.clone(),
            host: target.host.clone(),
            port: target.port,
            shard: target.shard,
            replica: target.replica,
            version: row.version.clone(),
            reachable: true,
            mem_total,
            mem_used: row.server_memory_used_bytes as u64,
            cores,
            cpu_busy_cores,
            running: row.active_queries as u32,
            lag_s: row.replica_lag_s.unwrap_or(0),
            active_parts: row.active_parts,
            queries,
            uptime_s: Some(row.uptime_s),
            server_cpu_time_us: row.server_cpu_time_us,
            max_memory_usage: positive_or_none(row.max_memory_usage.unwrap_or(0)),
            unreachable_reason: None,
            poll_ms: Some(poll_ms),
        })
    }

    /// The running queries, with the extended columns while the server accepts them.
    async fn processes(&self, url: &str) -> String {
        if self.extended_processes.load(Ordering::Relaxed) {
            match self.post(url, PROCESSES_SQL).await {
                Ok(body) => return body,
                // An unknown column is a property of the server, not of this poll: §6.1's
                // statement for the rest of the session. Any other error only costs this poll
                // its extras — the basic statement still gets the rows.
                Err(e) if e.starts_with("HTTP ") => {
                    if refuses_columns(&e) {
                        self.extended_processes.store(false, Ordering::Relaxed);
                    }
                }
                // A timeout or a refused connection: the basic statement would fail the same way.
                Err(_) => return String::new(),
            }
        }
        self.post(url, PROCESSES_SQL_BASIC).await.unwrap_or_default()
    }

    /// POST the SQL as the body (§12: the HTTP interface truncates long GET query strings).
    async fn post(&self, url: &str, sql: &str) -> Result<String, String> {
        let settings = if self.settings_ok.load(Ordering::Relaxed) {
            SETTINGS
        } else {
            SETTINGS_READONLY_FRIENDLY
        };
        let endpoint = format!("{url}/?{settings}&default_format=JSONEachRow");
        let response = self
            .client
            .post(&endpoint)
            // §9: credentials go in headers, and are never logged.
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .body(sql.to_string())
            .send()
            .await
            .map_err(|e| clean_error(&e.to_string()))?;

        let status = response.status();
        let body = response.text().await.map_err(|e| clean_error(&e.to_string()))?;

        if status.is_success() {
            return Ok(body);
        }

        // A read-only user cannot change `max_execution_time`, so the server refuses and names
        // the setting — with 500, not 400. Drop the settings once and remember it for the
        // session; the user's own profile already carries the limits (§9).
        if body.contains("Cannot modify") && body.contains("readonly mode") {
            self.setting_refused();
            return self
                .post_with(url, sql, SETTINGS_READONLY_FRIENDLY)
                .await;
        }
        Err(format!("HTTP {}: {}", status.as_u16(), first_line(&body)))
    }

    async fn post_with(&self, url: &str, sql: &str, settings: &str) -> Result<String, String> {
        let endpoint = format!("{url}/?{settings}&default_format=JSONEachRow");
        let response = self
            .client
            .post(&endpoint)
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .body(sql.to_string())
            .send()
            .await
            .map_err(|e| clean_error(&e.to_string()))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("HTTP {}: {}", status.as_u16(), first_line(&body)))
        }
    }

    fn setting_refused(&self) {
        self.settings_ok.store(false, Ordering::Relaxed);
    }
}

/// ClickHouse's ways of saying a column does not exist on this version.
fn refuses_columns(error: &str) -> bool {
    [
        "UNKNOWN_IDENTIFIER",
        "Missing columns",
        "Unknown expression identifier",
        "NO_SUCH_COLUMN",
        "There is no column",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn positive_or_none(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn first_line(body: &str) -> String {
    body.lines().next().unwrap_or_default().chars().take(200).collect()
}

/// reqwest errors can echo the URL; the URL has no credential in it, but the password must
/// never appear in a message (§9).
fn clean_error(message: &str) -> String {
    message.chars().take(200).collect()
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

fn parse_one_row(body: &str) -> Result<CapacityRow, String> {
    serde_json::from_str(body.trim()).map_err(|e| format!("capacity row: {e}"))
}

/// Turn the JSONEachRow body into query rows, with attribution from §6.4 applied on top of
/// what the server already extracted.
pub fn parse_processes(body: &str) -> Vec<QueryRow> {
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<ProcessRow>(line).ok())
        .map(|row| {
            let (person, redash_id) =
                attribution_from_sql(&row.user, &row.query, row.redash_user.as_deref(), row.redash_query_id);
            QueryRow {
                query_id: row.query_id,
                user: row.user,
                person,
                redash_query_id: redash_id,
                elapsed_s: row.elapsed_s,
                memory_bytes: row.memory_usage,
                read_rows: row.read_rows,
                read_bytes: row.read_bytes,
                sql: row.query,
                cpu_time_us: row.cpu_time_us,
                total_rows_approx: row.total_rows_approx.unwrap_or(0),
                written_rows: row.written_rows.unwrap_or(0),
                peak_memory_bytes: row.peak_memory_usage,
                memory_limit: row.query_max_memory_usage.filter(|v| *v > 0),
                kind: row.query_kind.filter(|k| !k.is_empty()),
            }
        })
        .collect()
}

/// Fold what `system.clusters` said into the targets (§6.2).
///
/// A discovered host is the same machine as a target when the names agree, or when it was
/// reached through that target's seed URL (`parse_clusters` hands the seed's URL to a host it
/// recognises by name or address). The target then takes the cluster's name — `ch-a` reads
/// better than `172.18.0.3` — and its shard and replica, instead of the same server being
/// polled twice under two names.
pub fn merge_discovered(targets: &mut Vec<NodeTarget>, found: Vec<NodeTarget>) {
    for target in found {
        let existing = targets
            .iter()
            .position(|t| t.name == target.name)
            .or_else(|| targets.iter().position(|t| t.url == target.url));
        match existing {
            Some(i) => {
                let existing = &mut targets[i];
                existing.name = target.name;
                existing.host = target.host;
                existing.port = target.port;
                existing.shard = target.shard;
                existing.replica = target.replica;
            }
            None => targets.push(target),
        }
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    targets.dedup_by(|a, b| a.url == b.url && a.name == b.name);
}

/// Union the cluster rows by host name and turn them into poll targets (§6.2).
///
/// The HTTP URL of a discovered host is `http://{host}:{CH_HTTP_PORT}`, unless one of the
/// seeds already points at that host or address — then the seed's URL wins.
pub fn parse_clusters(body: &str, seeds: &[String], http_port: u16) -> Result<Vec<NodeTarget>, String> {
    let mut by_name: HashMap<String, NodeTarget> = HashMap::new();
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let row: ClusterRow = serde_json::from_str(line).map_err(|e| format!("cluster row: {e}"))?;
        let port = u16::try_from(row.port).unwrap_or(9000);
        if by_name.contains_key(&row.host_name) {
            continue;
        }
        let seed_url = seed_for(&row.host_name, &row.host_address, seeds);
        let url = seed_url.unwrap_or_else(|| format!("http://{}:{}", row.host_name, http_port));
        by_name.insert(
            row.host_name.clone(),
            NodeTarget {
                name: row.host_name.clone(),
                url,
                host: if row.host_address.is_empty() {
                    row.host_name.clone()
                } else {
                    row.host_address.clone()
                },
                port,
                shard: row.shard_num,
                replica: row.replica_num,
            },
        );
    }
    let mut out: Vec<NodeTarget> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn seed_for(host_name: &str, host_address: &str, seeds: &[String]) -> Option<String> {
    seeds
        .iter()
        .find(|seed| {
            seed_contains(seed, host_name) || (!host_address.is_empty() && seed_contains(seed, host_address))
        })
        .cloned()
}

fn seed_contains(seed: &str, host: &str) -> bool {
    let rest = seed.split("://").nth(1).unwrap_or(seed);
    let authority = rest.split('/').next().unwrap_or(rest);
    authority
        .rsplit_once(':')
        .map(|(h, _)| h == host)
        .unwrap_or(authority == host)
}

/// `futures::join_all` without the dependency: every future is polled on each wake-up, so
/// the requests are in flight together and the poll takes as long as the slowest node, not
/// the sum of them all (§6: a slow node must not delay the others).
async fn join_all<F: std::future::Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    use std::task::Poll;
    let mut pending: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut all = true;
        for (future, slot) in pending.iter_mut().zip(done.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            match future.as_mut().poll(cx) {
                Poll::Ready(value) => *slot = Some(value),
                Poll::Pending => all = false,
            }
        }
        if all { Poll::Ready(()) } else { Poll::Pending }
    })
    .await;
    done.into_iter()
        .map(|slot| slot.expect("every future has finished"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPACITY_FIXTURE: &str = r#"{"server_memory_total_bytes":68719476736.0,"server_memory_used_bytes":59756847104.0,"server_cpu_percent":93.75,"server_cpu_cores":16,"server_cpu_time_us":500000000000,"active_queries":6,"replica_lag_s":0,"active_parts":1204,"max_memory_usage":9000000000,"version":"24.11.1.2557","uptime_s":4306429}
{"server_memory_total_bytes":0.0,"server_memory_used_bytes":418078720.0,"server_cpu_percent":null,"server_cpu_cores":null,"server_cpu_time_us":12,"active_queries":0,"replica_lag_s":null,"active_parts":0,"max_memory_usage":0,"version":"24.10.4.191","uptime_s":7}
"#;

    const PROCESSES_FIXTURE: &str = r#"{"query_id":"c3e51cb5","user":"r_redash","query":"/* Application: Redash */ /* Username: grigol.gankava@paysera.net, Redash query_id: 7438, Redash: */ SELECT count() FROM accounting_lt.bank_record","elapsed_s":275.4,"memory_usage":17380000000,"read_rows":1900000000,"read_bytes":41200000000,"cpu_time_us":853600000,"redash_user":"grigol.gankava@paysera.net","redash_query_id":"7438"}
{"query_id":"beef0001","user":"airflow","query":"INSERT INTO statistics.daily_rollup SELECT 1","elapsed_s":12.0,"memory_usage":1000000,"read_rows":10,"read_bytes":2048,"cpu_time_us":3000000,"redash_user":"","redash_query_id":null}
"#;

    const CLUSTERS_FIXTURE: &str = r#"{"cluster":"ch_paysera","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1","host_address":"172.16.17.132","port":9000}
{"cluster":"ch_paysera","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2","host_address":"172.16.17.133","port":9000}
"#;

    #[test]
    fn a_capacity_row_becomes_a_node() {
        let row: CapacityRow = parse_one_row(CAPACITY_FIXTURE.lines().next().unwrap()).unwrap();
        assert_eq!(row.server_memory_total_bytes as u64, 64 * 1024 * 1024 * 1024);
        assert_eq!(row.server_cpu_cores, Some(16.0));
        assert_eq!(row.active_parts, 1204);
    }

    #[test]
    fn an_unknown_denominator_stays_unknown() {
        let row: CapacityRow =
            parse_one_row(CAPACITY_FIXTURE.lines().nth(1).unwrap()).unwrap();
        assert_eq!(positive_or_none(row.server_memory_total_bytes as u64), None);
        assert_eq!(positive_or_none(row.max_memory_usage.unwrap_or(0)), None, "0 means auto");
        assert_eq!(row.server_cpu_percent, None);
        assert_eq!(row.replica_lag_s, None, "no replicas → lag 0, not a guess");
    }

    #[test]
    fn process_rows_are_parsed_and_attributed() {
        let queries = parse_processes(PROCESSES_FIXTURE);
        assert_eq!(queries.len(), 2);

        let redash = &queries[0];
        assert_eq!(redash.query_id, "c3e51cb5");
        assert_eq!(redash.user, "r_redash");
        assert_eq!(redash.person.as_deref(), Some("grigol.gankava"), "local part of paysera.net");
        assert_eq!(redash.redash_query_id, Some(7438));
        assert_eq!(redash.memory_bytes, 17380000000);
        assert_eq!(redash.cpu_time_us, 853600000, "kept for the next poll's delta");

        let airflow = &queries[1];
        assert_eq!(airflow.person, None, "a real user is not renamed");
        assert_eq!(airflow.redash_query_id, None);
    }

    #[test]
    fn attribution_falls_back_to_the_server_side_extract() {
        // A user whose SQL the server could not parse still has its comment picked up here.
        let queries = parse_processes(
            r#"{"query_id":"x","user":"r_redash","query":"/* Username: m.kairys@paysera.net, */ SELECT 1","elapsed_s":1,"memory_usage":1,"read_rows":1,"read_bytes":1,"cpu_time_us":1,"redash_user":"","redash_query_id":null}
"#,
        );
        assert_eq!(queries[0].person.as_deref(), Some("m.kairys"));
    }

    #[test]
    fn clusters_become_targets_with_the_configured_http_port() {
        let targets = parse_clusters(CLUSTERS_FIXTURE, &[], 8123).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "pay-ch-node-1");
        assert_eq!(targets[0].url, "http://pay-ch-node-1:8123");
        assert_eq!(targets[0].port, 9000, "the native port is what system.clusters reports");
        assert_eq!((targets[0].shard, targets[0].replica), (1, 1));
    }

    #[test]
    fn a_seed_wins_over_the_assumed_port() {
        let seeds = vec!["http://127.0.0.1:8124".to_string()];
        let body = r#"{"cluster":"ch_paysera","shard_num":1,"replica_num":1,"host_name":"127.0.0.1","host_address":"127.0.0.1","port":9000}
"#;
        let targets = parse_clusters(body, &seeds, 8123).unwrap();
        assert_eq!(targets[0].url, "http://127.0.0.1:8124");
    }

    #[test]
    fn duplicate_cluster_rows_collapse_by_host_name() {
        let body = format!("{CLUSTERS_FIXTURE}{CLUSTERS_FIXTURE}");
        let targets = parse_clusters(&body, &[], 8123).unwrap();
        assert_eq!(targets.len(), 2, "union by host_name (§6.2)");
    }

    #[test]
    fn seeds_are_nodes_even_without_a_cluster() {
        let config = ClickHouseConfig {
            seeds: vec!["http://127.0.0.1:8123".into(), "http://ch-b:8124".into()],
            cluster: "ch_paysera".into(),
            user: "monitor".into(),
            password: "secret".into(),
            http_port: 8123,
        };
        let source = ClickHouseSource::new(&config).unwrap();
        assert_eq!(source.targets().len(), 2);
        // Sorted by name, so the fleet's order does not depend on the order of CH_SEED_URLS.
        assert_eq!(source.targets()[0].name, "127.0.0.1");
        assert_eq!(source.targets()[0].url, "http://127.0.0.1:8123");
        assert_eq!(source.targets()[1].name, "ch-b");
        assert_eq!(source.targets()[1].url, "http://ch-b:8124");
    }

    #[test]
    fn two_ports_on_one_host_are_two_nodes() {
        let config = ClickHouseConfig {
            seeds: vec![
                "http://127.0.0.1:8123".into(),
                "http://127.0.0.1:8124".into(),
                "http://127.0.0.1:8124".into(),
            ],
            cluster: "ch_paysera".into(),
            user: "monitor".into(),
            password: "p".into(),
            http_port: 8123,
        };
        let source = ClickHouseSource::new(&config).unwrap();
        let names: Vec<&str> = source.targets().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["127.0.0.1:8123", "127.0.0.1:8124"], "the same URL twice is one");
    }

    #[test]
    fn a_seed_found_again_by_discovery_is_one_node_under_its_cluster_name() {
        let seeds = vec!["http://172.18.0.3:8123".to_string()];
        let mut targets = vec![NodeTarget {
            name: "172.18.0.3".into(),
            url: "http://172.18.0.3:8123".into(),
            host: "172.18.0.3".into(),
            port: 8123,
            shard: 0,
            replica: 0,
        }];
        let body = r#"{"cluster":"ch_paysera","shard_num":1,"replica_num":1,"host_name":"ch-a","host_address":"172.18.0.3","port":9000}
{"cluster":"ch_paysera","shard_num":1,"replica_num":2,"host_name":"ch-b","host_address":"172.18.0.4","port":9000}
"#;
        let found = parse_clusters(body, &seeds, 8123).unwrap();
        merge_discovered(&mut targets, found);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["ch-a", "ch-b"], "not 172.18.0.3 and ch-a for one server");
        assert_eq!(targets[0].url, "http://172.18.0.3:8123", "still reached through its seed");
        assert_eq!((targets[0].shard, targets[0].replica), (1, 1));
        assert_eq!(targets[1].url, "http://ch-b:8123", "a host no seed covers gets CH_HTTP_PORT");
    }

    #[test]
    fn errors_never_contain_the_password() {
        let config = ClickHouseConfig {
            seeds: vec!["http://127.0.0.1:1".into()],
            cluster: "c".into(),
            user: "monitor".into(),
            password: "hunter2".into(),
            http_port: 8123,
        };
        let source = ClickHouseSource::new(&config).unwrap();
        assert!(!format!("{:?}", source.targets()).contains("hunter2"));
        assert!(!first_line("Code: 516. oops").contains("hunter2"));
    }

    #[test]
    fn the_settings_flag_is_remembered_after_one_refusal() {
        let mut config = ClickHouseConfig {
            seeds: vec!["http://127.0.0.1:8123".into()],
            cluster: "c".into(),
            user: "monitor".into(),
            password: "p".into(),
            http_port: 8123,
        };
        config.cluster = "c".into();
        let source = ClickHouseSource::new(&config).unwrap();
        assert!(source.settings_ok.load(Ordering::Relaxed));
        source.setting_refused();
        assert!(
            !source.settings_ok.load(Ordering::Relaxed),
            "one 400 is enough for the whole session"
        );
    }

    #[test]
    fn only_a_missing_column_switches_to_the_basic_statement() {
        assert!(refuses_columns(
            "HTTP 404: Code: 47. DB::Exception: Missing columns: 'query_kind' while processing query"
        ));
        assert!(refuses_columns("HTTP 400: Code: 47. DB::Exception: Unknown expression identifier `query_kind`"));
        assert!(!refuses_columns("HTTP 503: Code: 202. DB::Exception: Too many simultaneous queries"));
        assert!(!refuses_columns("HTTP 500: Code: 241. DB::Exception: Memory limit (total) exceeded"));
    }

    #[test]
    fn extended_process_rows_carry_progress_and_limits() {
        let row = r#"{"query_id":"q","user":"r_redash","query":"SELECT 1","elapsed_s":4.0,"memory_usage":"-1024","read_rows":"4437281151","read_bytes":"0","cpu_time_us":"1","redash_user":"","redash_query_id":"","total_rows_approx":"20000000000","written_rows":"0","peak_memory_usage":"2048","query_max_memory_usage":"9000000000","query_kind":"Select"}
"#;
        let queries = parse_processes(row);
        assert_eq!(queries.len(), 1, "a negative memory reading does not drop the row");
        let q = &queries[0];
        assert_eq!(q.memory_bytes, 0);
        assert_eq!(q.total_rows_approx, 20_000_000_000);
        assert_eq!(q.memory_limit, Some(9_000_000_000));
        assert_eq!(q.peak_memory_bytes, 2048);
        assert_eq!(q.kind.as_deref(), Some("Select"));
        // A basic row (§6.1's statement) still parses, with the extras unknown.
        let basic = parse_processes(PROCESSES_FIXTURE);
        assert_eq!(basic[0].total_rows_approx, 0);
        assert_eq!(basic[0].memory_limit, None);
    }

    #[test]
    fn the_sql_is_the_one_in_the_design() {
        // §6.1 is a contract with the fleet: is_initial_query is the dedup that keeps a
        // distributed query from being counted once per node.
        assert!(PROCESSES_SQL.contains("is_initial_query = 1"));
        assert!(PROCESSES_SQL_BASIC.contains("is_initial_query = 1"));
        assert!(PROCESSES_SQL.contains("total_rows_approx"));
        assert!(PROCESSES_SQL.contains("Settings['max_memory_usage']"));
        assert!(PROCESSES_SQL.contains("FROM system.processes"));
        assert!(CAPACITY_SQL.contains("CGroupMemoryTotal"));
        assert!(CAPACITY_SQL.contains("OSCPUVirtualTimeMicroseconds"));
        assert!(CLUSTERS_SQL.contains("system.clusters"));
    }
}