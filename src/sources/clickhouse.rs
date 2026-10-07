//! ClickHouse over the HTTP interface: discovery (§6.2) and per-node polling (§6.1).
//!
//! Every node is polled concurrently with a per-request timeout, so one slow node cannot hold
//! up the fleet. A node that does not answer comes back as `NodeSnapshot::unreachable` with no
//! numbers — never as zeros (§2.6).

use crate::config::{ClickHouseConfig, Credentials};
use crate::model::{
    attribution_from_sql, FleetSnapshot, NodeSnapshot, QueryRow, LOGIN_REFUSED, NO_ACCESS, NO_LOGIN,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// §6: 1.5 s per request. The poll interval is longer, so one slow node misses a poll instead
/// of delaying everybody else's.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(1500);

/// The time limit in force: §6's, or the one `CH_TIMEOUT_MS` set when the source was made.
static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();

fn request_timeout() -> Duration {
    TIMEOUT.get().copied().unwrap_or(REQUEST_TIMEOUT)
}
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
  extract(query, '(?i)query[ _]id:\\s*(\\d+)') AS redash_query_id,
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
  extract(query, '(?i)query[ _]id:\\s*(\\d+)') AS redash_query_id
FROM system.processes
WHERE is_initial_query = 1
  AND query NOT LIKE '%FROM system.processes%'
  AND query NOT LIKE 'KILL QUERY%'
ORDER BY elapsed DESC
"#;

/// §6.2, plus the two columns that say which row is the server answering: `is_local`, and
/// the server's own `hostName()` for when `is_local` cannot tell (a NAT, a container). Without
/// them a seed reached as `clickhouse1.example.net` and listed by its cluster as
/// `pay-ch-node-1.example.lan` is two nodes, one of them unreachable.
const CLUSTERS_SQL: &str = r#"
SELECT cluster, shard_num, replica_num, host_name, host_address, port, is_local,
       hostName() AS self_host
FROM system.clusters
WHERE cluster = {cluster:String}
ORDER BY shard_num, replica_num
"#;

/// One node to poll, and where to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTarget {
    /// What the screen calls it: the seed as it was typed, or the cluster's name for a host
    /// only discovery knows (and for a seed given as a bare IP address).
    pub name: String,
    pub url: String,
    /// The host as the cluster knows it (`system.clusters.host_name`), for the drawer.
    pub host: String,
    pub port: u16,
    pub shard: u32,
    pub replica: u32,
    /// Came from the credential file or `CH_SEED_URLS` rather than from discovery.
    pub seed: bool,
    /// Its own login (from the credential file or its URL), or the default login; `None` when
    /// neither exists for this host, which is then reported instead of polled.
    pub credentials: Option<Credentials>,
}

/// One row of `system.clusters`, as one seed saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterHost {
    pub host_name: String,
    pub host_address: String,
    /// The native port the cluster lists; the HTTP port is `CH_HTTP_PORT` or the seed's.
    pub port: u16,
    pub shard: u32,
    pub replica: u32,
    /// This row is the server that answered (`is_local`, or its own `hostName()`).
    pub is_self: bool,
}

/// What one seed answered to `CLUSTERS_SQL`.
#[derive(Debug, Clone)]
pub struct SeedAnswer {
    /// The seed target's URL (`scheme://authority`).
    pub url: String,
    pub hosts: Vec<ClusterHost>,
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
    #[serde(default, deserialize_with = "de_u64")]
    is_local: u64,
    #[serde(default)]
    self_host: String,
}

pub struct ClickHouseSource {
    client: reqwest::Client,
    /// The default login (`default_login` in the credential file, or `CH_USER` /
    /// `CH_PASSWORD`), for seeds without one of their own and for discovered hosts. `None`
    /// when every seed carries its own.
    default_login: Option<Credentials>,
    cluster: String,
    http_port: u16,
    /// Targets keyed by node name; the fleet is seeds ∪ discovered (§6.2).
    targets: Vec<NodeTarget>,
    /// Servers (by URL) that refused the query settings: a read-only user cannot set
    /// `max_execution_time`, so this is the normal path in production, not an edge case. Per
    /// server, because with a login per server one can be read-only and the next not.
    settings_refused: Mutex<HashSet<String>>,
    /// Servers that refused the extended `system.processes` columns (an older version); they
    /// get §6.1's own statement from then on.
    basic_processes: Mutex<HashSet<String>>,
    /// The name each server (by URL) gave in `X-ClickHouse-Server-Display-Name` — its host
    /// name unless configured otherwise. ClickHouse sends it with every answer, a refused
    /// login included, so it can say which cluster row a seed is when the seed would not
    /// answer the discovery query.
    server_names: Mutex<HashMap<String, String>>,
}

impl ClickHouseSource {
    pub fn new(config: &ClickHouseConfig) -> Result<Self, String> {
        let _ = TIMEOUT.set(config.timeout);
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| format!("http client: {e}"))?;

        // A seed is still a node even when it is not in any cluster (§6.2), so it joins the
        // fleet under its own host name — logging in with its own login when it has one, with
        // the default login otherwise.
        let mut targets: Vec<NodeTarget> = config
            .seeds
            .iter()
            .map(|seed| NodeTarget {
                name: seed.host.clone(),
                url: seed.url.clone(),
                host: seed.host.clone(),
                port: seed.port,
                shard: 0,
                replica: 0,
                seed: true,
                credentials: seed.credentials.clone().or_else(|| config.default_login.clone()),
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
            default_login: config.default_login.clone(),
            cluster: config.cluster.clone(),
            http_port: config.http_port,
            targets,
            settings_refused: Mutex::new(HashSet::new()),
            basic_processes: Mutex::new(HashSet::new()),
            server_names: Mutex::new(HashMap::new()),
        })
    }

    /// The fleet as it stands: seeds plus whatever discovery has found, keyed by name.
    #[cfg(test)]
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
        let mut answers: Vec<SeedAnswer> = Vec::new();

        let seed_targets: Vec<NodeTarget> = self.targets.iter().filter(|t| t.seed).cloned().collect();
        for seed in seed_targets {
            let url = seed.url.clone();
            // Named as the screen names it: the strip has no room for a scheme and a port.
            match self.post(&seed, &sql).await {
                Ok(body) => match parse_clusters(&body) {
                    Ok(hosts) => answers.push(SeedAnswer { url, hosts }),
                    Err(e) => errors.push(format!("{}: {e}", seed.name)),
                },
                Err(e) => errors.push(format!("{}: {e}", seed.name)),
            }
        }

        // A host only discovery knows is reached by its name when this machine can resolve
        // it, and by the address the cluster lists when it cannot — cluster-internal names
        // (`*.lan`) rarely resolve on a laptop, their addresses often do.
        let unknown: Vec<String> = answers
            .iter()
            .flat_map(|a| a.hosts.iter())
            .filter(|h| !self.targets.iter().any(|t| t.name == h.host_name))
            .map(|h| h.host_name.clone())
            .collect();
        let port = self.http_port;
        let checks = join_all(unknown.iter().map(|name| addresses(name.clone(), port))).await;
        let resolvable: std::collections::HashSet<String> = unknown
            .into_iter()
            .zip(checks)
            .filter_map(|(name, found)| (!found.is_empty()).then_some(name))
            .collect();

        // And what each seed's own name resolves to: a seed that is down cannot say which row
        // of the cluster it is, but its address can.
        let seeds: Vec<(String, String, u16)> = self
            .targets
            .iter()
            .filter(|t| t.seed)
            .filter_map(|t| {
                let rest = t.url.split("://").nth(1)?;
                let authority = rest.split('/').next()?;
                let (host, port) = match authority.rsplit_once(':') {
                    Some((h, p)) => (h.to_string(), p.parse().unwrap_or(port)),
                    None => (authority.to_string(), port),
                };
                Some((t.url.clone(), host, port))
            })
            .collect();
        let resolved = join_all(seeds.iter().map(|(_, host, port)| addresses(host.clone(), *port))).await;
        let clues = SeedClues {
            addresses: seeds
                .into_iter()
                .zip(resolved)
                .map(|((url, _, _), found)| (url, found))
                .collect(),
            names: self.server_names.lock().map(|names| names.clone()).unwrap_or_default(),
        };

        merge_discovered(
            &mut self.targets,
            &answers,
            self.http_port,
            &|name| resolvable.contains(name),
            &clues,
            self.default_login.as_ref(),
        );
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
        let capacity = self.post(target, CAPACITY_SQL).await?;
        let row: CapacityRow = parse_one_row(&capacity)?;
        let queries: Vec<QueryRow> = parse_processes(&self.processes(target).await);
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
    async fn processes(&self, target: &NodeTarget) -> String {
        let basic = self
            .basic_processes
            .lock()
            .map(|set| set.contains(&target.url))
            .unwrap_or(false);
        if !basic {
            match self.post(target, PROCESSES_SQL).await {
                Ok(body) => return body,
                // An unknown column is a property of the server, not of this poll: §6.1's
                // statement for the rest of the session. Any other error only costs this poll
                // its extras — the basic statement still gets the rows.
                Err(e) if e.starts_with("HTTP ") => {
                    if refuses_columns(&e)
                        && let Ok(mut set) = self.basic_processes.lock()
                    {
                        set.insert(target.url.clone());
                    }
                }
                // A timeout or a refused connection: the basic statement would fail the same way.
                Err(_) => return String::new(),
            }
        }
        self.post(target, PROCESSES_SQL_BASIC).await.unwrap_or_default()
    }

    /// POST the SQL as the body (§12: the HTTP interface truncates long GET query strings),
    /// logged in as this target's user.
    async fn post(&self, target: &NodeTarget, sql: &str) -> Result<String, String> {
        let login = target.credentials.as_ref().ok_or_else(|| NO_LOGIN.to_string())?;
        let refused = self
            .settings_refused
            .lock()
            .map(|set| set.contains(&target.url))
            .unwrap_or(false);
        let settings = if refused { SETTINGS_READONLY_FRIENDLY } else { SETTINGS };
        let (status, body) = self.send(&target.url, login, sql, settings).await?;
        if status.is_success() {
            return Ok(body);
        }

        // A read-only user cannot change `max_execution_time`, so the server refuses and names
        // the setting — with 500, not 400. Drop the settings once and remember it for this
        // server; the user's own profile already carries the limits (§9).
        if !refused && body.contains("Cannot modify") && body.contains("readonly mode") {
            if let Ok(mut set) = self.settings_refused.lock() {
                set.insert(target.url.clone());
            }
            let (status, body) = self.send(&target.url, login, sql, SETTINGS_READONLY_FRIENDLY).await?;
            if status.is_success() {
                return Ok(body);
            }
            return Err(http_error(status, &body, login));
        }
        Err(http_error(status, &body, login))
    }

    async fn send(
        &self,
        url: &str,
        login: &Credentials,
        sql: &str,
        settings: &str,
    ) -> Result<(reqwest::StatusCode, String), String> {
        let endpoint = format!("{url}/?{settings}&default_format=JSONEachRow");
        let response = self
            .client
            .post(&endpoint)
            // §9: credentials go in headers, and are never logged.
            .header("X-ClickHouse-User", &login.user)
            .header("X-ClickHouse-Key", &login.password)
            .body(sql.to_string())
            .send()
            .await
            .map_err(describe)?;
        let status = response.status();
        if let Some(name) = response
            .headers()
            .get("X-ClickHouse-Server-Display-Name")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            && let Ok(mut names) = self.server_names.lock()
        {
            names.insert(url.to_string(), name.to_string());
        }
        let body = response.text().await.map_err(describe)?;
        Ok((status, body))
    }
}

/// A server's refusal, in words. A refused login names the user (never the password); a
/// missing grant names the grant; anything else keeps the gist of ClickHouse's first line.
/// ClickHouse answers a wrong password with 403 and `Code: 516 … (AUTHENTICATION_FAILED)`, a
/// missing one with 401, and a missing grant with `Code: 497 … (ACCESS_DENIED)`.
fn http_error(status: reqwest::StatusCode, body: &str, login: &Credentials) -> String {
    let refused_login = status == reqwest::StatusCode::UNAUTHORIZED
        || [
            "Authentication failed",
            "AUTHENTICATION_FAILED",
            "UNKNOWN_USER",
            "REQUIRED_PASSWORD",
            "WRONG_PASSWORD",
        ]
        .iter()
        .any(|marker| body.contains(marker));
    if refused_login {
        return format!("{LOGIN_REFUSED} — user {}: check its password on this server", login.user);
    }
    if body.contains("ACCESS_DENIED") || body.contains("Not enough privileges") {
        return match needed_grant(body) {
            Some(grant) => format!("{NO_ACCESS} — {} needs {grant}", login.user),
            None => format!("{NO_ACCESS} — {} lacks a grant the monitor needs", login.user),
        };
    }
    format!("HTTP {}: {}", status.as_u16(), gist(body, &login.user))
}

/// `… it's necessary to have the grant SELECT(metric, value) ON system.asynchronous_metrics.
/// (ACCESS_DENIED)` → `SELECT on system.asynchronous_metrics`: the column list is noise.
fn needed_grant(body: &str) -> Option<String> {
    let rest = body.split("necessary to have the grant ").nth(1)?;
    let grant = rest.lines().next()?.split(" (ACCESS_DENIED)").next()?.trim().trim_end_matches('.');
    let columns = regex::Regex::new(r"\([^)]*\)").expect("a valid pattern");
    let grant = columns.replace_all(grant, "").replace(" ON ", " on ");
    let grant = grant.split_whitespace().collect::<Vec<_>>().join(" ");
    (!grant.is_empty()).then(|| crate::fmt::truncate(&grant, 80))
}

/// ClickHouse's first line without its wrapping: `Code: 241. DB::Exception: monitor: Memory
/// limit (total) exceeded: … (MEMORY_LIMIT_EXCEEDED) (version 24.10…)` → `Memory limit (total)
/// exceeded: … (MEMORY_LIMIT_EXCEEDED)`, cut to fit a line.
fn gist(body: &str, user: &str) -> String {
    let mut text = body.lines().next().unwrap_or_default().trim();
    if let Some((_, after)) = text.strip_prefix("Code: ").and_then(|rest| rest.split_once(". ")) {
        text = after;
    }
    text = text.strip_prefix("DB::Exception: ").unwrap_or(text);
    if let Some(after) = text.strip_prefix(user).and_then(|rest| rest.strip_prefix(": ")) {
        text = after;
    }
    if let Some(at) = text.find(" (version ") {
        text = &text[..at];
    }
    crate::fmt::truncate(text.trim(), 120)
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

/// reqwest errors can echo the URL; the URL has no credential in it, but the password must
/// never appear in a message (§9).
fn clean_error(message: &str) -> String {
    message.chars().take(200).collect()
}

/// Why a request did not get an answer, in the words the drawer and the insights use.
/// reqwest's own text is `error sending request for url (…)` for every one of these; the cause
/// is further down its source chain.
fn describe(error: reqwest::Error) -> String {
    let timeout = error.is_timeout();
    let connect = error.is_connect();
    let mut chain = String::new();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        chain.push_str(&cause.to_string());
        chain.push(' ');
        source = cause.source();
    }
    reason(&chain, timeout, connect).unwrap_or_else(|| clean_error(&error.without_url().to_string()))
}

/// The cause behind a transport error, from the text of its source chain — in the words every
/// source uses (`sources::reason`).
fn reason(chain: &str, timeout: bool, connect: bool) -> Option<String> {
    super::reason(chain, timeout.then_some(request_timeout()), connect)
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
                attribution_from_sql(&row.query, row.redash_user.as_deref(), row.redash_query_id);
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

/// What a host name resolves to here, within a second (empty when it does not). Blocking DNS
/// runs on the blocking pool so a slow resolver cannot stall the poller.
async fn addresses(name: String, port: u16) -> Vec<String> {
    use std::net::ToSocketAddrs;
    let lookup = tokio::task::spawn_blocking(move || {
        (name.as_str(), port)
            .to_socket_addrs()
            .map(|found| found.map(|a| a.ip().to_string()).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    match tokio::time::timeout(Duration::from_secs(1), lookup).await {
        Ok(Ok(found)) => found,
        _ => Vec::new(),
    }
}

fn is_ip_literal(host: &str) -> bool {
    host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok()
}

/// Two names for one host: equal ignoring case, or the same first label
/// (`pay-ch-node-1` and `pay-ch-node-1.example.lan`). Addresses only ever match exactly.
fn same_host(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    if is_ip_literal(a) || is_ip_literal(b) {
        return false;
    }
    let first = |s: &str| s.split('.').next().unwrap_or(s).to_ascii_lowercase();
    first(a) == first(b)
}

/// What is known about the seeds besides their answers, for telling which row of the cluster
/// a seed is when it could not say so itself.
#[derive(Debug, Default)]
pub struct SeedClues {
    /// Seed URL → the addresses the seed's host name resolves to here.
    pub addresses: HashMap<String, Vec<String>>,
    /// Seed URL → the name its server gave in `X-ClickHouse-Server-Display-Name`.
    pub names: HashMap<String, String>,
}

/// Fold what the seeds said about their cluster into the targets (§6.2).
///
/// Every seed is matched to the row of `system.clusters` that is the seed itself — by
/// `is_local` first, then by the name its server gave (in its answer or, when it refused the
/// query, in a header), then by an earlier round's binding, then (for an answer without
/// either) by the seed's host or address appearing in the row. A matched row only adds its
/// shard, replica and cluster name to the seed's target, so one server is never polled twice
/// under two names. A seed typed as a bare address takes the cluster's name, which reads
/// better; one typed as a name keeps it. Rows that match no seed are the rest of the cluster
/// and become targets of their own, reached by name when `resolvable` says it resolves here
/// and by the listed address otherwise, logging in with `default_login` — never with a seed's
/// own login, so one server's password is not sent to another.
pub fn merge_discovered(
    targets: &mut Vec<NodeTarget>,
    answers: &[SeedAnswer],
    http_port: u16,
    resolvable: &dyn Fn(&str) -> bool,
    clues: &SeedClues,
    default_login: Option<&Credentials>,
) {
    // Which seed each cluster host is, most certain first: the seed said so (`is_local`); the
    // seed's server gave the host's name, and no other row has it; an earlier round bound it
    // (the target already carries the cluster's name as its host); the seed's URL names the
    // host or its address; the seed's name resolves to its address.
    let mut bound: HashMap<String, String> = HashMap::new();
    for answer in answers {
        for host in answer.hosts.iter().filter(|h| h.is_self) {
            bound.entry(host.host_name.clone()).or_insert_with(|| answer.url.clone());
        }
    }
    let seeds: Vec<(String, String)> = targets
        .iter()
        .filter(|t| t.seed)
        .map(|t| (t.url.clone(), t.host.clone()))
        .collect();
    let mut listed: Vec<&ClusterHost> = Vec::new();
    for host in answers.iter().flat_map(|a| a.hosts.iter()) {
        if !listed.iter().any(|h| h.host_name == host.host_name) {
            listed.push(host);
        }
    }
    for (url, _) in &seeds {
        let Some(name) = clues.names.get(url) else { continue };
        if bound.values().any(|u| u == url) {
            continue;
        }
        let matching: Vec<&&ClusterHost> = listed
            .iter()
            .filter(|h| !bound.contains_key(&h.host_name) && same_host(name, &h.host_name))
            .collect();
        if let [host] = matching.as_slice() {
            bound.insert(host.host_name.clone(), url.clone());
        }
    }
    for host in &listed {
        if bound.contains_key(&host.host_name) {
            continue;
        }
        let taken = |url: &String| bound.values().any(|u| u == url);
        let by_memory = seeds.iter().find(|(url, known)| known == &host.host_name && !taken(url));
        let by_url = || {
            seeds.iter().find(|(url, _)| {
                !taken(url)
                    && (seed_contains(url, &host.host_name)
                        || (!host.host_address.is_empty() && seed_contains(url, &host.host_address)))
            })
        };
        let by_address = || {
            seeds.iter().find(|(url, _)| {
                !taken(url)
                    && !host.host_address.is_empty()
                    && clues
                        .addresses
                        .get(url)
                        .is_some_and(|found| found.iter().any(|a| a == &host.host_address))
            })
        };
        if let Some((url, _)) = by_memory.or_else(by_url).or_else(by_address) {
            bound.insert(host.host_name.clone(), url.clone());
        }
    }

    // A seed that neither answered nor was recognised may be any of the rows no seed claimed.
    // Those rows still become targets when they can be polled — if one is that seed under its
    // cluster name, its numbers show — but not when there is no login for them: such a row
    // would have no numbers, only advice to add a host that may already be a seed.
    let unaccounted = seeds
        .iter()
        .any(|(url, _)| !bound.values().any(|u| u == url) && !answers.iter().any(|a| &a.url == url));

    for host in &listed {
        match bound.get(&host.host_name) {
            Some(url) => {
                // A target an earlier round created for this host under its cluster name is
                // the same server as the seed: it goes.
                targets.retain(|t| t.seed || t.name != host.host_name || &t.url == url);
                if let Some(target) = targets.iter_mut().find(|t| &t.url == url) {
                    if is_ip_literal(&target.name) || target.name == host.host_name {
                        target.name = host.host_name.clone();
                    }
                    target.host = host.host_name.clone();
                    target.port = host.port;
                    target.shard = host.shard;
                    target.replica = host.replica;
                }
            }
            None => match targets.iter_mut().find(|t| t.name == host.host_name) {
                Some(target) => {
                    target.shard = host.shard;
                    target.replica = host.replica;
                }
                None if default_login.is_none() && unaccounted => {}
                None => {
                    let reach = if resolvable(&host.host_name) || host.host_address.is_empty() {
                        host.host_name.clone()
                    } else if host.host_address.contains(':') {
                        format!("[{}]", host.host_address)
                    } else {
                        host.host_address.clone()
                    };
                    targets.push(NodeTarget {
                        name: host.host_name.clone(),
                        url: format!("http://{reach}:{http_port}"),
                        host: host.host_name.clone(),
                        port: host.port,
                        shard: host.shard,
                        replica: host.replica,
                        seed: false,
                        // With a login per server and no shared one, there is none to use:
                        // the host says so (`NO_LOGIN`) instead of being polled.
                        credentials: default_login.cloned(),
                    });
                }
            },
        }
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    // Belt and braces: one URL is one server, and a seed's entry is the one to keep.
    let mut urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut order: Vec<usize> = (0..targets.len()).collect();
    order.sort_by_key(|i| !targets[*i].seed);
    let mut keep = vec![false; targets.len()];
    for i in order {
        if urls.insert(targets[i].url.clone()) {
            keep[i] = true;
        }
    }
    let mut i = 0;
    targets.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

/// The rows of one seed's `system.clusters`, de-duplicated by host name, with the row that is
/// the answering server marked (`is_local`, or its own `hostName()`).
pub fn parse_clusters(body: &str) -> Result<Vec<ClusterHost>, String> {
    let mut hosts: Vec<ClusterHost> = Vec::new();
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let row: ClusterRow = serde_json::from_str(line).map_err(|e| format!("cluster row: {e}"))?;
        let is_self = row.is_local == 1 || same_host(&row.host_name, &row.self_host);
        if let Some(existing) = hosts.iter_mut().find(|h| h.host_name == row.host_name) {
            existing.is_self |= is_self;
            continue;
        }
        hosts.push(ClusterHost {
            port: u16::try_from(row.port).unwrap_or(9000),
            shard: row.shard_num,
            replica: row.replica_num,
            is_self,
            host_address: row.host_address,
            host_name: row.host_name,
        });
    }
    // Only one row can be the server answering; if the server's short name matched several
    // rows, `is_local` decides, and without it none of them is trusted.
    if hosts.iter().filter(|h| h.is_self).count() > 1 {
        for host in &mut hosts {
            host.is_self = false;
        }
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(row) = serde_json::from_str::<ClusterRow>(line)
                && row.is_local == 1
                && let Some(host) = hosts.iter_mut().find(|h| h.host_name == row.host_name)
            {
                host.is_self = true;
            }
        }
    }
    Ok(hosts)
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

// -- query sessions ---------------------------------------------------------------------------

/// What a query session's queries are sent with: read-only always; a time and a row limit, the
/// answer whole before it is sent (so the summary says what the query read), and the query
/// stopped on the server when the session lets go of it — where the login may set them. A
/// read-only login's profile carries its own limits, and is sent `readonly=1` alone.
const CONSOLE_SETTINGS: &str = "readonly=1&max_execution_time=30&max_result_rows=1000&result_overflow_mode=break\
&wait_end_of_query=1&cancel_http_readonly_queries_on_client_close=1&log_comment=cobserve%20query%20session";
/// The session waits a little longer than the server may take.
const CONSOLE_TIMEOUT: Duration = Duration::from_secs(35);
/// The most of an answer that is read; the rest is cut, and the request let go.
const CONSOLE_BYTES: usize = 8 * 1024 * 1024;

/// Where a query session's queries go: a server's address and the login the monitor has for it.
/// Never in `App`: the login stays with the code that sends it.
#[derive(Clone)]
pub struct ConsoleTarget {
    pub url: String,
    pub login: Option<Credentials>,
}

impl std::fmt::Debug for ConsoleTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsoleTarget").field("url", &self.url).field("login", &self.login).finish()
    }
}

impl ClickHouseSource {
    /// Every server of the fleet by name, with its address and login, for query sessions.
    pub fn console_targets(&self) -> HashMap<String, ConsoleTarget> {
        self.targets
            .iter()
            .map(|t| (t.name.clone(), ConsoleTarget { url: t.url.clone(), login: t.credentials.clone() }))
            .collect()
    }
}

/// A query session's query on one server, and the first rows of what it answered.
pub async fn console_query(client: &reqwest::Client, target: &ConsoleTarget, node: &str, sql: &str) -> Result<crate::console::Answer, String> {
    let login = target.login.as_ref().ok_or_else(|| "no login for this server — give it one in the credential file".to_string())?;
    let started = std::time::Instant::now();
    let mut response = console_send(client, &target.url, login, sql, CONSOLE_SETTINGS).await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        if !(body.contains("Cannot modify") && body.contains("readonly mode")) {
            return Err(console_error(&body, login));
        }
        // A read-only login may not set the limits; its profile has its own.
        response = console_send(client, &target.url, login, sql, "readonly=1").await?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(console_error(&body, login));
        }
    }
    let summary = response
        .headers()
        .get("X-ClickHouse-Summary")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok());
    let number = |key: &str| summary.as_ref().and_then(|s| s.get(key)).and_then(|v| v.as_str().and_then(|t| t.parse().ok()).or(v.as_u64()));
    let mut body = Vec::new();
    let mut cut = false;
    while let Some(chunk) = response.chunk().await.map_err(describe_console)? {
        body.extend_from_slice(&chunk);
        if body.len() > CONSOLE_BYTES {
            cut = true;
            break;
        }
    }
    drop(response);
    let mut answer = parse_console(&String::from_utf8_lossy(&body), login)?;
    answer.node = node.to_string();
    answer.elapsed_ms = started.elapsed().as_millis() as u64;
    answer.read_rows = number("read_rows");
    answer.read_bytes = number("read_bytes");
    answer.cut |= cut;
    Ok(answer)
}

async fn console_send(client: &reqwest::Client, url: &str, login: &Credentials, sql: &str, settings: &str) -> Result<reqwest::Response, String> {
    client
        .post(format!("{url}/?{settings}&default_format=JSONCompactEachRowWithNamesAndTypes"))
        .timeout(CONSOLE_TIMEOUT)
        // §9: credentials go in headers, and are never logged.
        .header("X-ClickHouse-User", &login.user)
        .header("X-ClickHouse-Key", &login.password)
        .body(sql.to_string())
        .send()
        .await
        .map_err(describe_console)
}

/// A transport error, with the console's own time limit in the words for a timeout.
fn describe_console(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return format!("no answer within {} s", CONSOLE_TIMEOUT.as_secs());
    }
    describe(error)
}

/// ClickHouse's error as a line: `Code 60 · Unknown table … (UNKNOWN_TABLE)`, its version cut and
/// the login's password never in it — taken out of the answer's own format when the server
/// wrote it there (`[]`, `[]`, then `["Code: 164. …"]`).
fn console_error(body: &str, login: &Credentials) -> String {
    let body = body.lines().find_map(exception_in).unwrap_or_else(|| body.to_string());
    let mut text = body.trim().replace('\n', " ");
    if login.password.chars().count() >= 4 {
        text = text.replace(&login.password, "…");
    }
    if let Some(at) = text.find(" (version ") {
        text.truncate(at);
    }
    let text = match text.strip_prefix("Code: ").and_then(|rest| rest.split_once(". DB::Exception: ")) {
        Some((code, message)) => format!("Code {code} · {message}"),
        None => text,
    };
    text.chars().take(600).collect()
}

/// What a server's tables are read with, for suggestions: read-only, and short.
const SCHEMA_SETTINGS: &str = "readonly=1&max_execution_time=20&max_result_rows=250000&result_overflow_mode=break\
&log_comment=cobserve%20suggestions";
/// The most of a server's list that is read.
const SCHEMA_BYTES: usize = 32 * 1024 * 1024;

const SCHEMA_DATABASES: &str = "SELECT name FROM system.databases ORDER BY name FORMAT TabSeparated";
const SCHEMA_TABLES: &str = "SELECT database, name, engine FROM system.tables \
WHERE database NOT IN ('INFORMATION_SCHEMA', 'information_schema') AND NOT is_temporary \
ORDER BY database, name LIMIT 20000 FORMAT TabSeparated";
/// The same with how many rows each holds and what was written about it, for the helper to tell
/// the tables that matter; a server without those columns gets the list above.
const SCHEMA_TABLES_FULL: &str = "SELECT database, name, engine, ifNull(total_rows, 0), replaceRegexpAll(comment, '[\\t\\n]', ' ') FROM system.tables \
WHERE database NOT IN ('INFORMATION_SCHEMA', 'information_schema') AND NOT is_temporary \
ORDER BY database, name LIMIT 20000 FORMAT TabSeparated";
const SCHEMA_COLUMNS: &str = "SELECT database, table, name, type FROM system.columns \
WHERE database NOT IN ('INFORMATION_SCHEMA', 'information_schema') \
ORDER BY database, table, position LIMIT 200000 FORMAT TabSeparated";
const SCHEMA_FUNCTIONS: &str = "SELECT name, is_aggregate FROM system.functions WHERE NOT startsWith(name, '_') ORDER BY name FORMAT TabSeparated";

/// A server's databases, tables, columns and functions, for a query session's suggestions and
/// its helper. The tables are what is needed; a login that may not list the columns or the
/// functions still gets the tables.
pub async fn console_schema(client: &reqwest::Client, target: &ConsoleTarget) -> Result<crate::complete::Schema, String> {
    use crate::complete::{Schema, Table};
    let databases = schema_rows(client, target, SCHEMA_DATABASES).await?;
    let tables = match schema_rows(client, target, SCHEMA_TABLES_FULL).await {
        Ok(rows) => rows,
        Err(_) => schema_rows(client, target, SCHEMA_TABLES).await?,
    };
    let columns = schema_rows(client, target, SCHEMA_COLUMNS).await.unwrap_or_default();
    let functions = schema_rows(client, target, SCHEMA_FUNCTIONS).await.unwrap_or_default();
    let mut schema = Schema {
        databases: databases.into_iter().filter_map(|row| row.into_iter().next()).collect(),
        tables: tables
            .into_iter()
            .filter(|row| row.len() >= 3)
            .map(|row| Table {
                database: row[0].clone(),
                name: row[1].clone(),
                engine: row[2].clone(),
                columns: Vec::new(),
                rows: row.get(3).and_then(|n| n.parse().ok()).filter(|n| *n > 0),
                comment: row.get(4).map(|c| c.trim().to_string()).unwrap_or_default(),
            })
            .collect(),
        functions: functions.into_iter().filter(|row| row.len() >= 2).map(|row| (row[0].clone(), row[1] == "1")).collect(),
    };
    let at: HashMap<(String, String), usize> = schema.tables.iter().enumerate().map(|(i, t)| ((t.database.clone(), t.name.clone()), i)).collect();
    for row in columns.into_iter().filter(|row| row.len() >= 4) {
        if let Some(&i) = at.get(&(row[0].clone(), row[1].clone())) {
            schema.tables[i].columns.push((row[2].clone(), row[3].clone()));
        }
    }
    if schema.functions.is_empty() {
        schema.functions = Schema::fallback().functions;
    }
    Ok(schema)
}

/// One of those lists, its rows' fields as text.
async fn schema_rows(client: &reqwest::Client, target: &ConsoleTarget, sql: &str) -> Result<Vec<Vec<String>>, String> {
    let login = target.login.as_ref().ok_or_else(|| "no login for this server".to_string())?;
    let mut response = console_send(client, &target.url, login, sql, SCHEMA_SETTINGS).await?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        if !(body.contains("Cannot modify") && body.contains("readonly mode")) {
            return Err(console_error(&body, login));
        }
        response = console_send(client, &target.url, login, sql, "readonly=1").await?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(console_error(&body, login));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe_console)? {
        body.extend_from_slice(&chunk);
        if body.len() > SCHEMA_BYTES {
            break;
        }
    }
    Ok(parse_tsv(&String::from_utf8_lossy(&body)))
}

/// `TabSeparated` rows, their fields unescaped. A last line cut short is left out.
pub fn parse_tsv(body: &str) -> Vec<Vec<String>> {
    let complete = if body.ends_with('\n') { body } else { body.rsplit_once('\n').map_or("", |(whole, _)| whole) };
    complete.lines().filter(|line| !line.is_empty()).map(|line| line.split('\t').map(unescape_tsv).collect()).collect()
}

fn unescape_tsv(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The server's exception in a line of an answer: as text, or written in the answer's format.
fn exception_in(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("Code: ") {
        return Some(line.to_string());
    }
    if line.starts_with("[\"Code: ") {
        return serde_json::from_str::<Vec<String>>(line).ok()?.into_iter().next();
    }
    if line.starts_with("{\"exception\"") {
        return serde_json::from_str::<serde_json::Value>(line).ok()?.get("exception")?.as_str().map(str::to_string);
    }
    None
}

/// `JSONCompactEachRowWithNamesAndTypes` — names, types, then a row per line — into an answer,
/// at most `ROW_LIMIT` rows of it; anything else (a query with a `FORMAT` of its own) as text.
pub fn parse_console(body: &str, login: &Credentials) -> Result<crate::console::Answer, String> {
    use crate::console::{Answer, ROW_LIMIT};
    let mut lines = body.lines();
    let strings = |line: Option<&str>| -> Option<Vec<String>> {
        let values: Vec<serde_json::Value> = serde_json::from_str(line?).ok()?;
        values.into_iter().map(|v| v.as_str().map(str::to_string)).collect()
    };
    let (Some(names), Some(types)) = (strings(lines.next()), strings(lines.next())) else {
        if body.lines().any(|line| exception_in(line).is_some()) {
            return Err(console_error(body, login));
        }
        let text: String = body.lines().take(ROW_LIMIT).collect::<Vec<_>>().join("\n");
        return Ok(Answer { text: Some(text), ..Answer::default() });
    };
    let mut answer = Answer { columns: names.into_iter().zip(types).collect(), ..Answer::default() };
    for line in lines {
        if exception_in(line).is_some() {
            // The server stopped it after the first rows: say why, with what came.
            let error = console_error(line, login);
            if answer.rows.is_empty() {
                return Err(error);
            }
            answer.text = Some(error);
            answer.cut = true;
            break;
        }
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(line) else {
            continue;
        };
        if answer.rows.len() >= ROW_LIMIT {
            answer.cut = true;
            break;
        }
        answer.rows.push(
            values
                .into_iter()
                .map(|v| match v {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(text) => Some(text),
                    other => Some(other.to_string()),
                })
                .collect(),
        );
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_query_session_s_answer_is_read_row_by_row_and_errors_say_what_clickhouse_said() {
        let login = Credentials { user: "monitor".into(), password: "s3cret-pw".into() };
        let body = "[\"user\",\"queries\",\"memory\"]\n[\"String\",\"UInt64\",\"Nullable(Float64)\"]\n[\"r_redash\",\"2\",1.5]\n[\"airflow\",\"1\",null]\n";
        let answer = parse_console(body, &login).unwrap();
        assert_eq!(answer.columns, [("user".to_string(), "String".to_string()), ("queries".into(), "UInt64".into()), ("memory".into(), "Nullable(Float64)".into())]);
        assert_eq!(answer.rows, [vec![Some("r_redash".to_string()), Some("2".into()), Some("1.5".into())], vec![Some("airflow".into()), Some("1".into()), None]]);
        assert!(!answer.cut && answer.text.is_none());
        // A FORMAT of its own: the text as it came.
        let pretty = parse_console("┏━━━┓\n┃ 1 ┃\n", &login).unwrap();
        assert_eq!(pretty.text.as_deref(), Some("┏━━━┓\n┃ 1 ┃"));
        // Refused, in ClickHouse's words — without its version, and without the password.
        let refused = "Code: 164. DB::Exception: monitor: Cannot execute query in readonly mode. For queries over HTTP, method GET implies readonly s3cret-pw. (READONLY) (version 24.10.1.2812 (official build))";
        let error = parse_console(refused, &login).unwrap_err();
        assert!(error.starts_with("Code 164 · monitor: Cannot execute query in readonly mode."), "{error}");
        assert!(error.ends_with("(READONLY)") && !error.contains("s3cret") && !error.contains("version"), "{error}");
        // Stopped after the first rows: they stay, with why.
        let timed_out = "[\"n\"]\n[\"UInt64\"]\n[\"1\"]\nCode: 159. DB::Exception: Timeout exceeded: elapsed 30.0 seconds. (TIMEOUT_EXCEEDED)\n";
        let partial = parse_console(timed_out, &login).unwrap();
        assert!(partial.cut && partial.rows.len() == 1 && partial.text.as_deref().unwrap().contains("TIMEOUT_EXCEEDED"));
        // Written in the answer's own format, as a read-only login's refusal is.
        let in_format = "[]\n[]\n[\"Code: 164. DB::Exception: monitor: Cannot execute query in readonly mode. (READONLY) (version 24.10.4.191 (official build))\"]\n";
        assert_eq!(parse_console(in_format, &login).unwrap_err(), "Code 164 · monitor: Cannot execute query in readonly mode. (READONLY)");
        let mid_stream = "[\"n\"]\n[\"UInt64\"]\n[\"1\"]\n[\"Code: 159. DB::Exception: Timeout exceeded. (TIMEOUT_EXCEEDED)\"]\n";
        assert!(parse_console(mid_stream, &login).unwrap().text.unwrap().starts_with("Code 159 · Timeout exceeded."));
        // More rows than are kept: cut, and said.
        let many: String = std::iter::once("[\"n\"]\n[\"UInt64\"]\n".to_string()).chain((0..1500).map(|n| format!("[\"{n}\"]\n"))).collect();
        let cut = parse_console(&many, &login).unwrap();
        assert_eq!((cut.rows.len(), cut.cut), (crate::console::ROW_LIMIT, true));
        assert!(!format!("{:?}", ConsoleTarget { url: "http://ch:8123".into(), login: Some(login) }).contains("s3cret"), "Debug never shows it");
    }

    #[test]
    fn a_server_s_lists_are_read_as_tab_separated_rows() {
        let rows = parse_tsv("system\tprocesses\tSystemProcesses\nwallet\tweird\\tname\tMergeTree\ncut");
        assert_eq!(rows, [vec!["system", "processes", "SystemProcesses"], vec!["wallet", "weird\tname", "MergeTree"]]);
        assert_eq!(parse_tsv("a\\\\b\n"), [vec!["a\\b"]]);
        assert!(parse_tsv("").is_empty());
    }

    use super::*;

    const CAPACITY_FIXTURE: &str = r#"{"server_memory_total_bytes":68719476736.0,"server_memory_used_bytes":59756847104.0,"server_cpu_percent":93.75,"server_cpu_cores":16,"server_cpu_time_us":500000000000,"active_queries":6,"replica_lag_s":0,"active_parts":1204,"max_memory_usage":9000000000,"version":"24.11.1.2557","uptime_s":4306429}
{"server_memory_total_bytes":0.0,"server_memory_used_bytes":418078720.0,"server_cpu_percent":null,"server_cpu_cores":null,"server_cpu_time_us":12,"active_queries":0,"replica_lag_s":null,"active_parts":0,"max_memory_usage":0,"version":"24.10.4.191","uptime_s":7}
"#;

    const PROCESSES_FIXTURE: &str = r#"{"query_id":"c3e51cb5","user":"r_redash","query":"/* Application: Redash */ /* Username: grigol.gankava@example.net, Redash query_id: 7438, Redash: */ SELECT count() FROM accounting_lt.bank_record","elapsed_s":275.4,"memory_usage":17380000000,"read_rows":1900000000,"read_bytes":41200000000,"cpu_time_us":853600000,"redash_user":"grigol.gankava@example.net","redash_query_id":"7438"}
{"query_id":"beef0001","user":"airflow","query":"INSERT INTO statistics.daily_rollup SELECT 1","elapsed_s":12.0,"memory_usage":1000000,"read_rows":10,"read_bytes":2048,"cpu_time_us":3000000,"redash_user":"","redash_query_id":null}
"#;

    const CLUSTERS_FIXTURE: &str = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1","host_address":"172.16.17.132","port":9000}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2","host_address":"172.16.17.133","port":9000}
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
        assert_eq!(redash.person.as_deref(), Some("grigol.gankava"), "the local part at the home domain");
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
            r#"{"query_id":"x","user":"r_redash","query":"/* Username: m.kairys@example.net, */ SELECT 1","elapsed_s":1,"memory_usage":1,"read_rows":1,"read_bytes":1,"cpu_time_us":1,"redash_user":"","redash_query_id":null}
"#,
        );
        assert_eq!(queries[0].person.as_deref(), Some("m.kairys"));
    }

    fn seed(url: &str) -> NodeTarget {
        let host = url.split("://").nth(1).unwrap().split(':').next().unwrap().to_string();
        NodeTarget {
            name: host.clone(),
            url: url.to_string(),
            host,
            port: 8123,
            shard: 0,
            replica: 0,
            seed: true,
            credentials: None,
        }
    }

    /// `cargo test live_poll_times -- --ignored --nocapture`: three polls of your fleet with a
    /// 10 s limit, and how long each node took — to choose `CH_TIMEOUT_MS`. Names and times only.
    /// The file is `COBSERVE_CREDENTIAL`, else `~/.config/cobserve/credentials.yaml`.
    #[tokio::test]
    #[ignore]
    async fn live_poll_times() {
        let path = std::env::var("COBSERVE_CREDENTIAL")
            .unwrap_or_else(|_| format!("{}/.config/cobserve/credentials.yaml", std::env::var("HOME").unwrap_or_default()));
        let args = crate::config::Args::parse(["--credential".to_string(), path]).expect("args");
        let mut config = crate::config::Config::load(&args).expect("config").clickhouse;
        config.timeout = Duration::from_secs(10);
        let mut source = ClickHouseSource::new(&config).expect("source");
        for error in source.discover().await {
            println!("discovery: {error}");
        }
        for round in 1..=3 {
            let started = std::time::Instant::now();
            let snapshot = source.poll().await;
            println!("poll {round}: {} ms", started.elapsed().as_millis());
            for node in &snapshot.nodes {
                match (&node.poll_ms, &node.unreachable_reason) {
                    (Some(ms), _) => println!("  {:<24} {ms:>6} ms", node.name),
                    (None, why) => println!("  {:<24} ↯ {}", node.name, why.as_deref().unwrap_or("?")),
                }
            }
        }
    }

    fn config(seeds: &[&str], login: Option<(&str, &str)>) -> ClickHouseConfig {
        ClickHouseConfig {
            seeds: seeds
                .iter()
                .map(|seed| crate::config::parse_seed(seed, 8123).unwrap())
                .collect(),
            cluster: "c".into(),
            default_login: login.map(|(user, password)| Credentials {
                user: user.into(),
                password: password.into(),
            }),
            http_port: 8123,
            timeout: REQUEST_TIMEOUT,
        }
    }

    fn login(user: &str, password: &str) -> Credentials {
        Credentials {
            user: user.into(),
            password: password.into(),
        }
    }

    fn names(targets: &[NodeTarget]) -> Vec<&str> {
        targets.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn clusters_become_targets_with_the_configured_http_port() {
        let hosts = parse_clusters(CLUSTERS_FIXTURE).unwrap();
        assert_eq!(hosts.len(), 2);
        let mut targets = Vec::new();
        let answers = [SeedAnswer { url: "http://seed:8123".into(), hosts }];
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &SeedClues::default(), None);
        assert_eq!(names(&targets), vec!["pay-ch-node-1", "pay-ch-node-2"]);
        assert_eq!(targets[0].url, "http://pay-ch-node-1:8123");
        assert_eq!(targets[0].port, 9000, "the native port is what system.clusters reports");
        assert_eq!((targets[0].shard, targets[0].replica), (1, 1));
        assert!(!targets[0].seed);
    }

    #[test]
    fn a_seed_wins_over_the_assumed_port() {
        let mut targets = vec![seed("http://127.0.0.1:8124")];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"127.0.0.1","host_address":"127.0.0.1","port":9000}
"#;
        let answers = [SeedAnswer { url: "http://127.0.0.1:8124".into(), hosts: parse_clusters(body).unwrap() }];
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &SeedClues::default(), None);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].url, "http://127.0.0.1:8124");
    }

    #[test]
    fn duplicate_cluster_rows_collapse_by_host_name() {
        let body = format!("{CLUSTERS_FIXTURE}{CLUSTERS_FIXTURE}");
        assert_eq!(parse_clusters(&body).unwrap().len(), 2, "union by host_name (§6.2)");
    }

    /// What on call saw: two seeds by their public names, a cluster that calls the same two
    /// servers by internal names this laptop cannot resolve. Two nodes, not four.
    #[test]
    fn seeds_with_other_names_than_their_cluster_are_still_one_node_each() {
        let mut targets = vec![
            seed("http://clickhouse1.example.net:8123"),
            seed("http://clickhouse2.example.net:8123"),
        ];
        let rows = |local: u8| {
            format!(
                r#"{{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":{}}}
{{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2.example.lan","host_address":"10.0.0.12","port":9000,"is_local":{}}}
"#,
                u8::from(local == 1),
                u8::from(local == 2)
            )
        };
        let answers = [
            SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(&rows(1)).unwrap() },
            SeedAnswer { url: "http://clickhouse2.example.net:8123".into(), hosts: parse_clusters(&rows(2)).unwrap() },
        ];
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);

        assert_eq!(names(&targets), vec!["clickhouse1.example.net", "clickhouse2.example.net"]);
        assert_eq!(targets[0].host, "pay-ch-node-1.example.lan", "the cluster's name, for the drawer");
        assert_eq!((targets[0].shard, targets[0].replica), (1, 1));
        assert_eq!((targets[1].shard, targets[1].replica), (1, 2));
        assert!(targets.iter().all(|t| t.seed));
    }

    #[test]
    fn without_is_local_the_servers_own_name_says_which_row_it_is() {
        let mut targets = vec![seed("http://clickhouse1.example.net:8123")];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":0,"self_host":"pay-ch-node-1"}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2.example.lan","host_address":"10.0.0.12","port":9000,"is_local":0,"self_host":"pay-ch-node-1"}
"#;
        let hosts = parse_clusters(body).unwrap();
        assert!(hosts[0].is_self && !hosts[1].is_self);
        let answers = [SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts }];
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);
        assert_eq!(names(&targets), vec!["clickhouse1.example.net", "pay-ch-node-2.example.lan"]);
        // The other node's name does not resolve here, so it is reached by its address.
        assert_eq!(targets[1].url, "http://10.0.0.12:8123");
    }

    #[test]
    fn a_phantom_from_an_earlier_round_is_folded_back_into_its_seed() {
        let mut targets = vec![
            seed("http://clickhouse1.example.net:8123"),
            NodeTarget {
                name: "pay-ch-node-1.example.lan".into(),
                url: "http://pay-ch-node-1.example.lan:8123".into(),
                host: "pay-ch-node-1.example.lan".into(),
                port: 9000,
                shard: 1,
                replica: 1,
                seed: false,
                credentials: None,
            },
        ];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":1}
"#;
        let answers = [SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(body).unwrap() }];
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);
        assert_eq!(names(&targets), vec!["clickhouse1.example.net"]);
    }

    #[test]
    fn a_seed_given_as_an_address_takes_its_cluster_name() {
        let mut targets = vec![seed("http://172.18.0.3:8123")];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"ch-a","host_address":"172.18.0.3","port":9000,"is_local":1}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"ch-b","host_address":"172.18.0.4","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://172.18.0.3:8123".into(), hosts: parse_clusters(body).unwrap() }];
        merge_discovered(&mut targets, &answers, 8123, &|name| name == "ch-b", &SeedClues::default(), None);
        assert_eq!(names(&targets), vec!["ch-a", "ch-b"], "not 172.18.0.3 and ch-a for one server");
        assert_eq!(targets[0].url, "http://172.18.0.3:8123", "still reached through its seed");
        assert_eq!((targets[0].shard, targets[0].replica), (1, 1));
        assert_eq!(targets[1].url, "http://ch-b:8123", "a host that resolves is reached by name");
    }

    /// clickhouse2 is down when the app starts, so only clickhouse1 answers discovery — and it
    /// lists pay-ch-node-2 as "not me". Its address is what clickhouse2's name resolves to.
    #[test]
    fn a_seed_that_is_down_is_recognised_by_its_address() {
        let mut targets = vec![
            seed("http://clickhouse1.example.net:8123"),
            seed("http://clickhouse2.example.net:8123"),
        ];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":1}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2.example.lan","host_address":"10.0.0.12","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(body).unwrap() }];
        let addresses: HashMap<String, Vec<String>> = [
            ("http://clickhouse1.example.net:8123".to_string(), vec!["10.0.0.11".to_string()]),
            ("http://clickhouse2.example.net:8123".to_string(), vec!["10.0.0.12".to_string()]),
        ]
        .into();
        let clues = SeedClues { addresses, ..Default::default() };
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &clues, None);
        assert_eq!(names(&targets), vec!["clickhouse1.example.net", "clickhouse2.example.net"]);
        assert_eq!(targets[1].host, "pay-ch-node-2.example.lan");
    }

    #[test]
    fn a_seed_bound_once_stays_bound_while_it_is_down() {
        let mut targets = vec![
            seed("http://clickhouse1.example.net:8123"),
            seed("http://clickhouse2.example.net:8123"),
        ];
        let both = |local: u8| {
            format!(
                r#"{{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":{}}}
{{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2.example.lan","host_address":"10.0.0.12","port":9000,"is_local":{}}}
"#,
                u8::from(local == 1),
                u8::from(local == 2)
            )
        };
        // Round one: both answer.
        let answers = [
            SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(&both(1)).unwrap() },
            SeedAnswer { url: "http://clickhouse2.example.net:8123".into(), hosts: parse_clusters(&both(2)).unwrap() },
        ];
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);
        // Round two: clickhouse2 is down and DNS says nothing useful.
        let answers = [SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(&both(1)).unwrap() }];
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);
        assert_eq!(names(&targets), vec!["clickhouse1.example.net", "clickhouse2.example.net"]);
    }

    #[test]
    fn two_rows_claiming_to_be_the_server_trust_is_local_alone() {
        let body = r#"{"cluster":"c","shard_num":1,"replica_num":1,"host_name":"ch1.dc1","host_address":"10.0.0.1","port":9000,"is_local":0,"self_host":"ch1"}
{"cluster":"c","shard_num":2,"replica_num":1,"host_name":"ch1.dc2","host_address":"10.0.1.1","port":9000,"is_local":1,"self_host":"ch1"}
"#;
        let hosts = parse_clusters(body).unwrap();
        assert_eq!(hosts.iter().filter(|h| h.is_self).count(), 1);
        assert!(hosts[1].is_self);
    }

    #[test]
    fn host_names_match_on_their_first_label_and_addresses_exactly() {
        assert!(same_host("pay-ch-node-1.example.lan", "pay-ch-node-1"));
        assert!(same_host("CH-A", "ch-a"));
        assert!(!same_host("pay-ch-node-1.example.lan", "pay-ch-node-2"));
        assert!(!same_host("10.0.0.1", "10.0.0.12"));
        assert!(!same_host("", "ch-a"));
        assert!(is_ip_literal("172.18.0.3") && is_ip_literal("[::1]") && !is_ip_literal("ch-a"));
    }

    #[test]
    fn seeds_are_nodes_even_without_a_cluster() {
        let config = config(&["http://127.0.0.1:8123", "http://ch-b:8124"], Some(("monitor", "secret")));
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
        let config = config(
            &["http://127.0.0.1:8123", "http://127.0.0.1:8124", "http://127.0.0.1:8124"],
            Some(("monitor", "p")),
        );
        let source = ClickHouseSource::new(&config).unwrap();
        let names: Vec<&str> = source.targets().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["127.0.0.1:8123", "127.0.0.1:8124"], "the same URL twice is one");
    }

    #[test]
    fn transport_errors_are_said_plainly() {
        let dns = "client error (Connect) dns error failed to lookup address information: Name or service not known";
        assert_eq!(reason(dns, false, true).as_deref(), Some("name does not resolve from here (DNS)"));
        assert!(reason("tcp connect error Connection refused (os error 111)", false, true)
            .unwrap()
            .starts_with("connection refused"));
        assert_eq!(reason("", true, false).as_deref(), Some("no answer within 1.5 s"));
        assert_eq!(reason("something else", false, true).as_deref(), Some("cannot connect"));
        assert_eq!(reason("something else", false, false), None, "then reqwest's own words");
    }

    #[test]
    fn errors_never_contain_the_password() {
        let config = config(
            &["http://127.0.0.1:1", "http://mon2:Zq9-seed@127.0.0.1:2"],
            Some(("monitor", "Zq9-shared")),
        );
        let source = ClickHouseSource::new(&config).unwrap();
        let shown = format!("{:?}", source.targets());
        assert!(!shown.contains("Zq9"), "{shown}");
        assert!(shown.contains("mon2") && shown.contains("monitor"), "the users are fine to show: {shown}");
        assert!(source.targets().iter().all(|t| !t.url.contains('@')), "a URL never carries the login");
        assert!(!gist("Code: 516. oops", "mon2").contains("Zq9"));
    }

    #[test]
    fn a_refused_login_names_the_user_and_never_the_password() {
        let mon2 = login("mon2", "Zq9-seed");
        let body = "Code: 516. DB::Exception: mon2: Authentication failed: password is incorrect, or there is no user with such name. (AUTHENTICATION_FAILED) (version 24.10.4.191 (official build))";
        let said = http_error(reqwest::StatusCode::FORBIDDEN, body, &mon2);
        assert_eq!(said, "login refused — user mon2: check its password on this server");
        assert_eq!(http_error(reqwest::StatusCode::UNAUTHORIZED, "", &mon2), said, "no password at all");
        let node = NodeSnapshot::unreachable("n", said);
        assert_eq!((node.down_word(), node.down_detail()), ("login refused", Some("user mon2: check its password on this server")));
    }

    /// What one fleet answered for a user without the grants: a server that answers is not
    /// "unreachable", and the line names the grant instead of ClickHouse's paragraph.
    #[test]
    fn a_missing_grant_reads_no_access_with_the_grant_it_needs() {
        let sync = login("r_reports_daily", "p");
        let body = "Code: 497. DB::Exception: r_reports_daily: Not enough privileges. To execute this query, it's necessary to have the grant SELECT(metric, value) ON system.asynchronous_metrics. (ACCESS_DENIED) (version 24.11.1.2557 (official build))";
        let said = http_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, body, &sync);
        assert_eq!(said, "no access — r_reports_daily needs SELECT on system.asynchronous_metrics");
        let node = NodeSnapshot::unreachable("metrics", said);
        assert_eq!(node.down_word(), "no access");
        assert_eq!(node.down_detail(), Some("r_reports_daily needs SELECT on system.asynchronous_metrics"));

        let vague = http_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "Code: 497. DB::Exception: Not enough privileges", &sync);
        assert_eq!(vague, "no access — r_reports_daily lacks a grant the monitor needs");
    }

    #[test]
    fn other_errors_keep_the_gist_of_the_first_line() {
        let monitor = login("monitor", "p");
        let other = http_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "Code: 241. DB::Exception: monitor: Memory limit (total) exceeded: would use 1.20 TiB. (MEMORY_LIMIT_EXCEEDED) (version 24.10.4.191 (official build))\nmore",
            &monitor,
        );
        assert_eq!(other, "HTTP 500: Memory limit (total) exceeded: would use 1.20 TiB. (MEMORY_LIMIT_EXCEEDED)");
        assert!(refuses_columns(&http_error(
            reqwest::StatusCode::NOT_FOUND,
            "Code: 47. DB::Exception: Missing columns: 'query_kind' while processing query",
            &monitor
        )), "the basic-statement switch still recognises a missing column");
        assert_eq!(NodeSnapshot::unreachable("n", other).down_word(), "unreachable");
    }

    #[test]
    fn each_seed_logs_in_as_its_own_user() {
        let source = ClickHouseSource::new(&config(
            &["http://alice:pa@ch-a:8123", "http://ch-b:8123", "http://bob:pb@ch-a:8123"],
            Some(("monitor", "shared")),
        ))
        .unwrap();
        let users: Vec<(&str, &str)> = source
            .targets()
            .iter()
            .map(|t| (t.name.as_str(), t.credentials.as_ref().unwrap().user.as_str()))
            .collect();
        // ch-b has no login of its own; ch-a typed twice is one node, with the first login.
        assert_eq!(users, [("ch-a", "alice"), ("ch-b", "monitor")]);
    }

    #[test]
    fn a_host_only_discovery_knows_never_borrows_a_seeds_login() {
        let alice = || {
            vec![NodeTarget {
                credentials: Some(login("alice", "pa")),
                ..seed("http://ch-a:8123")
            }]
        };
        let body = r#"{"cluster":"c","shard_num":1,"replica_num":1,"host_name":"ch-a","host_address":"10.0.0.1","port":9000,"is_local":1}
{"cluster":"c","shard_num":1,"replica_num":2,"host_name":"ch-c","host_address":"10.0.0.3","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://ch-a:8123".into(), hosts: parse_clusters(body).unwrap() }];

        let mut targets = alice();
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &SeedClues::default(), None);
        assert_eq!(names(&targets), ["ch-a", "ch-c"]);
        assert_eq!(targets[0].credentials, Some(login("alice", "pa")), "a seed keeps its own");
        assert_eq!(targets[1].credentials, None, "ch-c is not sent alice's password");

        let shared = login("monitor", "p");
        let mut targets = alice();
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &SeedClues::default(), Some(&shared));
        assert_eq!(targets[1].credentials.as_ref(), Some(&shared), "the shared login is for exactly this");
        assert_eq!(targets[0].credentials, Some(login("alice", "pa")));
    }

    /// clickhouse2's password is wrong, so it refuses the discovery query — but ClickHouse
    /// names itself in a header even then.
    #[test]
    fn a_seed_that_refuses_the_login_is_recognised_by_the_name_it_gives() {
        let mut targets = vec![
            seed("http://clickhouse1.example.net:8123"),
            seed("http://clickhouse2.example.net:8123"),
        ];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"pay-ch-node-1.example.lan","host_address":"10.0.0.11","port":9000,"is_local":1}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"pay-ch-node-2.example.lan","host_address":"10.0.0.12","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://clickhouse1.example.net:8123".into(), hosts: parse_clusters(body).unwrap() }];
        let clues = SeedClues {
            names: [("http://clickhouse2.example.net:8123".to_string(), "pay-ch-node-2".to_string())].into(),
            ..Default::default()
        };
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &clues, Some(&login("monitor", "p")));
        assert_eq!(names(&targets), ["clickhouse1.example.net", "clickhouse2.example.net"]);
        assert_eq!(targets[1].host, "pay-ch-node-2.example.lan");
        assert_eq!((targets[1].shard, targets[1].replica), (1, 2));
    }

    #[test]
    fn a_name_two_rows_could_have_binds_neither() {
        let mut targets = vec![seed("http://lb.example:8123")];
        let body = r#"{"cluster":"c","shard_num":1,"replica_num":1,"host_name":"ch1.dc1","host_address":"10.0.0.1","port":9000,"is_local":0}
{"cluster":"c","shard_num":2,"replica_num":1,"host_name":"ch1.dc2","host_address":"10.0.1.1","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://other:8123".into(), hosts: parse_clusters(body).unwrap() }];
        let clues = SeedClues {
            names: [("http://lb.example:8123".to_string(), "ch1".to_string())].into(),
            ..Default::default()
        };
        merge_discovered(&mut targets, &answers, 8123, &|_| true, &clues, Some(&login("monitor", "p")));
        assert_eq!(names(&targets), ["ch1.dc1", "ch1.dc2", "lb.example"], "neither row is the seed");
        assert_eq!(targets[2].host, "lb.example");
    }

    /// Every seed brings its own login and clickhouse2 neither answered nor was recognised: the
    /// row no seed claimed may be clickhouse2 itself, and there is no login to find out with.
    #[test]
    fn no_row_without_a_login_is_added_while_a_seed_is_unaccounted_for() {
        let own = |url: &str, user: &str| NodeTarget {
            credentials: Some(login(user, "p")),
            ..seed(url)
        };
        let fleet = || vec![own("http://127.0.0.1:8123", "monitor"), own("http://127.0.0.1:8124", "r_redash")];
        let body = r#"{"cluster":"ch_cluster","shard_num":1,"replica_num":1,"host_name":"ch-a","host_address":"172.18.0.3","port":9000,"is_local":1}
{"cluster":"ch_cluster","shard_num":1,"replica_num":2,"host_name":"ch-b","host_address":"172.18.0.4","port":9000,"is_local":0}
"#;
        let answers = [SeedAnswer { url: "http://127.0.0.1:8123".into(), hosts: parse_clusters(body).unwrap() }];

        let mut targets = fleet();
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), None);
        assert_eq!(targets.len(), 2, "no ch-b row telling on call to add a host they already have");

        // With a shared login the row is polled, so it can show what it is.
        let mut targets = fleet();
        let shared = login("monitor", "p");
        merge_discovered(&mut targets, &answers, 8123, &|_| false, &SeedClues::default(), Some(&shared));
        assert_eq!(targets.len(), 3);

        // And once both seeds have answered, ch-b is clickhouse2 and nothing is added.
        let both = [
            SeedAnswer { url: "http://127.0.0.1:8123".into(), hosts: parse_clusters(body).unwrap() },
            SeedAnswer {
                url: "http://127.0.0.1:8124".into(),
                hosts: parse_clusters(&body.replace(r#""is_local":1"#, r#""is_local":2"#).replace(r#""is_local":0"#, r#""is_local":1"#).replace(r#""is_local":2"#, r#""is_local":0"#)).unwrap(),
            },
        ];
        let mut targets = fleet();
        merge_discovered(&mut targets, &both, 8123, &|_| false, &SeedClues::default(), None);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[1].host, "ch-b");
    }

    type Seen = std::sync::Arc<Mutex<Vec<(String, String, String)>>>;

    /// Enough of a ClickHouse for `poll` and `discover`, on a free local port: each request is
    /// answered by `answer(user, key, query string, sql)`, every answer names the server
    /// `name` the way ClickHouse does, and the login and query string of every request are
    /// kept for the test to look at.
    fn fake_clickhouse(name: &'static str, answer: fn(&str, &str, &str, &str) -> (u16, String)) -> (String, Seen) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen: Seen = Default::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).ok();
                let query = request.split_whitespace().nth(1).unwrap_or_default().to_string();
                let (mut user, mut key, mut length) = (String::new(), String::new(), 0);
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        let value = value.trim().to_string();
                        match name.trim().to_ascii_lowercase().as_str() {
                            "x-clickhouse-user" => user = value,
                            "x-clickhouse-key" => key = value,
                            "content-length" => length = value.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).ok();
                let (status, text) = answer(&user, &key, &query, &String::from_utf8_lossy(&body));
                log.lock().unwrap().push((user, key, query));
                let response = format!(
                    "HTTP/1.1 {status} X\r\nX-ClickHouse-Server-Display-Name: {name}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                stream.write_all(response.as_bytes()).ok();
            }
        });
        (url, seen)
    }

    const FAKE_CAPACITY: &str = r#"{"server_memory_total_bytes":1000,"server_memory_used_bytes":10,"server_cpu_percent":1,"server_cpu_cores":2,"server_cpu_time_us":1,"active_queries":0,"replica_lag_s":null,"active_parts":1,"max_memory_usage":0,"version":"24.10.4.191","uptime_s":7}"#;

    fn refuse(user: &str) -> (u16, String) {
        (
            403,
            format!("Code: 516. DB::Exception: {user}: Authentication failed: password is incorrect, or there is no user with such name. (AUTHENTICATION_FAILED)"),
        )
    }

    fn capacity_or_nothing(sql: &str) -> (u16, String) {
        let body = if sql.contains("server_memory_total_bytes") { FAKE_CAPACITY } else { "" };
        (200, body.to_string())
    }

    /// Three servers, three logins: alice's own (a read-only user, so her server refuses the
    /// session settings), the shared one for a seed without a login, and carol's with a wrong
    /// password. Nobody's password goes to anybody else's server, and one server's refusals
    /// change nothing for the others.
    #[tokio::test]
    async fn every_server_gets_its_own_login() {
        let (a, seen_a) = fake_clickhouse("node-a", |user, key, query, sql| {
            if (user, key) != ("alice", "pw-a") {
                return refuse(user);
            }
            if query.contains("max_execution_time") {
                let refusal = "Code: 164. DB::Exception: Cannot modify 'max_execution_time' setting in readonly mode. (READONLY)";
                return (500, refusal.to_string());
            }
            capacity_or_nothing(sql)
        });
        let (b, seen_b) = fake_clickhouse("node-b", |user, key, _, sql| {
            if (user, key) != ("monitor", "pw-shared") {
                return refuse(user);
            }
            capacity_or_nothing(sql)
        });
        let (c, seen_c) = fake_clickhouse("node-c", |user, key, _, sql| {
            if (user, key) != ("carol", "pw-c") {
                return refuse(user);
            }
            capacity_or_nothing(sql)
        });
        let with_login = |url: &str, login: &str| url.replace("http://", &format!("http://{login}@"));
        let seeds = [with_login(&a, "alice:pw-a"), b.clone(), with_login(&c, "carol:Zq9-wrong")];
        let seeds: Vec<&str> = seeds.iter().map(String::as_str).collect();
        let source = ClickHouseSource::new(&config(&seeds, Some(("monitor", "pw-shared")))).unwrap();

        source.poll().await;
        let fleet = source.poll().await;
        let node = |url: &str| {
            let name = url.trim_start_matches("http://");
            fleet.nodes.iter().find(|n| n.name == name).unwrap()
        };
        assert!(node(&a).reachable && node(&b).reachable);
        assert!(!node(&c).reachable);
        let reason = node(&c).unreachable_reason.clone().unwrap();
        assert_eq!(reason, "login refused — user carol: check its password on this server");

        let users = |seen: &Seen| -> Vec<String> {
            let seen = seen.lock().unwrap();
            let mut users: Vec<String> = seen.iter().map(|(user, key, _)| format!("{user}:{key}")).collect();
            users.dedup();
            users
        };
        assert_eq!(users(&seen_a), ["alice:pw-a"]);
        assert_eq!(users(&seen_b), ["monitor:pw-shared"]);
        assert_eq!(users(&seen_c), ["carol:Zq9-wrong"]);

        // alice's server refused the settings once, and only her server stopped getting them.
        let with_settings = |seen: &Seen| -> Vec<bool> {
            seen.lock().unwrap().iter().map(|(_, _, query)| query.contains("max_execution_time")).collect()
        };
        assert_eq!(with_settings(&seen_a), [true, false, false, false, false]);
        assert!(with_settings(&seen_b).iter().all(|sent| *sent), "{:?}", with_settings(&seen_b));
    }

    /// Every seed brought its own login and there is no shared one: a host only discovery
    /// knows is not polled at all, and says how to give it a login.
    #[tokio::test]
    async fn a_host_without_a_login_is_not_contacted() {
        let (a, _) = fake_clickhouse("node-a", |_, _, _, sql| capacity_or_nothing(sql));
        let (d, seen_d) = fake_clickhouse("node-d", |_, _, _, sql| capacity_or_nothing(sql));
        let seed_a = a.replace("http://", "http://alice:pw-a@");
        let mut source = ClickHouseSource::new(&config(&[&seed_a], None)).unwrap();
        source.targets.push(NodeTarget {
            name: "ch-d".into(),
            url: d.clone(),
            host: "ch-d".into(),
            port: 9000,
            shard: 1,
            replica: 2,
            seed: false,
            credentials: None,
        });

        let fleet = source.poll().await;
        let ch_d = fleet.nodes.iter().find(|n| n.name == "ch-d").unwrap();
        assert!(!ch_d.reachable);
        assert_eq!(ch_d.unreachable_reason.as_deref(), Some(NO_LOGIN));
        assert!(seen_d.lock().unwrap().is_empty(), "no request, so no password, went to ch-d");
        assert!(fleet.nodes.iter().any(|n| n.reachable), "the seed is polled as usual");
    }

    /// The whole round on the wire: alice's server answers discovery and lists two nodes;
    /// carol's refuses her (wrong) password but names itself, so it is node-b — and node-b is
    /// not added a second time, with no login, under its cluster name.
    #[tokio::test]
    async fn discovery_recognises_a_seed_whose_login_is_refused() {
        let (a, _) = fake_clickhouse("node-a", |user, key, _, sql| {
            if (user, key) != ("alice", "pw-a") {
                return refuse(user);
            }
            if sql.contains("system.clusters") {
                let rows = r#"{"cluster":"c","shard_num":1,"replica_num":1,"host_name":"node-a","host_address":"10.255.0.1","port":9000,"is_local":1,"self_host":"node-a"}
{"cluster":"c","shard_num":1,"replica_num":2,"host_name":"node-b","host_address":"10.255.0.2","port":9000,"is_local":0,"self_host":"node-a"}
"#;
                return (200, rows.to_string());
            }
            capacity_or_nothing(sql)
        });
        let (c, _) = fake_clickhouse("node-b", |user, key, _, sql| {
            if (user, key) != ("carol", "pw-c") {
                return refuse(user);
            }
            capacity_or_nothing(sql)
        });
        let seeds = [a.replace("http://", "http://alice:pw-a@"), c.replace("http://", "http://carol:Zq9-wrong@")];
        let seeds: Vec<&str> = seeds.iter().map(String::as_str).collect();
        let mut source = ClickHouseSource::new(&config(&seeds, None)).unwrap();

        let errors = source.discover().await;
        let carol = c.trim_start_matches("http://");
        assert_eq!(errors, [format!("{carol}: login refused — user carol: check its password on this server")]);
        let mut hosts: Vec<&str> = source.targets().iter().map(|t| t.host.as_str()).collect();
        hosts.sort();
        assert_eq!(hosts, ["node-a", "node-b"]);
        assert!(source.targets().iter().all(|t| t.seed), "{:?}", source.targets());
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