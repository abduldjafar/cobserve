//! `FAKE=1` data: believable snapshots with no network at all (DESIGN.md §8).
//!
//! This is step 1 of the build order — the screen has to look right before anything touches
//! a cluster. The numbers are not random: user bytes are chosen first, the node's own usage
//! is the sum of its users plus a server share, and `cpu_time_us` grows at exactly the cores
//! the row claims. That way the §5.3 invariant holds on screen too, not just in tests.

use crate::model::{FleetSnapshot, Job, JobState, NodeSnapshot, QueryRow, QueueRow, QueueStatus, Stale};
use std::time::{Duration, SystemTime};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// The e-mail domain of the fake fleet's people; FAKE=1 shows them by their local part.
pub const DOMAIN: &str = "example.net";

/// The node's memory and CPU percentage at `t` seconds into the session: its template's
/// value, moving. Smooth waves for most nodes; clickhouse3 climbs steadily and drops back when
/// its caches are "evicted", every three minutes — the shape a real hot node has, and what
/// the forecasts and the tape are for. At the first poll every node is within a point or two
/// of its template, which is what the tests of the §1 screen rely on.
fn load(template: &NodeTemplate, t: f64) -> (f64, f64) {
    let phase = template.name.bytes().map(f64::from).sum::<f64>();
    let wave = |amplitude: f64, period: f64| amplitude * ((t / period) * std::f64::consts::TAU + phase).sin();
    let healthy = template.user_pct < 35.0;
    let (mem_amp, cpu_amp) = if healthy { (1.5, 2.0) } else { (2.5, 5.0) };
    let mem = if template.name == "clickhouse3" {
        // +2.4 points a minute for three minutes, then back down.
        let cycle = t % 180.0;
        (template.user_pct + cycle * 0.04).min(98.5)
    } else {
        template.user_pct + wave(mem_amp, 97.0)
    };
    let cpu = template.cpu_pct + wave(cpu_amp, 41.0);
    (mem.clamp(1.0, 99.0), cpu.clamp(1.0, 99.0))
}

/// A tiny LCG. Deterministic: the same tick always produces the same fleet, which makes the
/// UI reproducible while developing.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    /// 0.0 .. 1.0
    fn unit(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

struct QueryTemplate {
    user: &'static str,
    /// Age at the first poll, in seconds — the screen of §1 rather than an empty fleet.
    initial_age_s: f64,
    /// `Some(person local part)` — rendered into the Redash comment the way Redash does.
    person: Option<&'static str>,
    redash_id: Option<u64>,
    sql: &'static str,
    lifetime_s: f64,
    mem_share: f64,
    cores: f64,
    /// Grows during the query's life; `None` means flat.
    mem_growth: f64,
    /// The query's own `max_memory_usage` as its profile sets it (GiB); `None` leaves the
    /// node's limit to apply.
    limit_gib: Option<f64>,
}

impl QueryTemplate {
    /// The comment Redash prepends to every query it runs (§6.4).
    fn sql_with_comment(&self) -> String {
        match (self.person, self.redash_id) {
            (Some(person), Some(id)) => format!(
                "/* Application: Redash */ /* Username: {person}@{DOMAIN}, Redash query_id: {id}, Redash: 10.1.0 */ {}",
                self.sql
            ),
            _ => self.sql.to_string(),
        }
    }
}

const AML: &str = "WITH BankRecord AS (\n  SELECT BillOpId, Bank,\n    multiIf(AccNr LIKE 'LT%', 'LT', Bank = 'SEB', 'LV', 'OTHER') AS region,\n    toFloat64OrZero(amount) AS amount\n  FROM accounting_lt.bank_record\n  WHERE EventDate >= today() - 30\n)\nSELECT region, count() AS ops, sum(amount) AS total\nFROM BankRecord GROUP BY region ORDER BY total DESC";
const GATEWAY: &str = "SELECT\n  toDate(ts) AS d,\n  countIf(status = 'error') AS errors,\n  count() AS total\nFROM gateway.transfers\nWHERE ts >= now() - INTERVAL 1 DAY\nGROUP BY d ORDER BY d";
const FX: &str = "SELECT currency, sum(amount) AS volume, uniqExact(merchant_id) AS merchants\nFROM statistics.fx_exposure\nWHERE event_date = today()\nGROUP BY currency";
const JULY_CLOSE: &str = "SELECT merchant_id, sum(amount) AS july, sum(if(month = 7, amount, 0)) AS delta\nFROM wallet.ledger\nGROUP BY merchant_id\nHAVING delta > 1000\nORDER BY delta DESC";
const CLUSTER_HISTORY: &str = "SELECT host_name, max(absolute_delay) AS lag, count() AS parts\nFROM clusterAllReplicas('ch_cluster', system.parts)\nGROUP BY host_name";
const REFUNDS: &str = "SELECT r.id, r.amount, r.created_at\nFROM refunds r\nWHERE r.status = 'open'\nORDER BY r.created_at";
const AIRFLOW_ETL: &str = "INSERT INTO statistics.daily_rollup SELECT toDate(event_time) AS d, user, sum(amount) FROM accounting.raw GROUP BY d, user";

/// Per node: capacity, its own share of that capacity, and the queries running on it.
/// `server_share` is the fraction of the node that is server/caches/merges, not users.
struct NodeTemplate {
    name: &'static str,
    host: &'static str,
    shard: u32,
    replica: u32,
    mem_gib: f64,
    cores: f64,
    /// The node's own memory / CPU percentage, target values the generator hits by
    /// construction: users first, then whatever is left for the server.
    user_pct: f64,
    cpu_pct: f64,
    lag_s: u64,
    parts: u64,
    queries: &'static [QueryTemplate],
}

const CLICKHOUSE3: &[QueryTemplate] = &[
    QueryTemplate {
        initial_age_s: 275.0,
        user: "r_redash",
        person: Some("grigol.gankava"),
        redash_id: Some(7438),
        sql: AML,
        lifetime_s: 900.0,
        mem_share: 0.253,
        cores: 3.1,
        mem_growth: 0.0002,
        limit_gib: Some(24.0),
    },
    QueryTemplate {
        initial_age_s: 164.0,
        user: "r_redash",
        person: Some("j.petrova"),
        redash_id: Some(8585),
        sql: "SELECT\n  toStartOfHour(ts) AS h,\n  countIf(state = 'failed') AS failed\nFROM open_banking.ais\nWHERE ts >= now() - INTERVAL 6 HOUR\nGROUP BY h",
        lifetime_s: 600.0,
        mem_share: 0.128,
        cores: 0.7,
        mem_growth: 0.0001,
        limit_gib: Some(24.0),
    },
    QueryTemplate {
        initial_age_s: 22.0,
        user: "airflow",
        person: None,
        redash_id: None,
        sql: AIRFLOW_ETL,
        lifetime_s: 28.0,
        mem_share: 0.055,
        cores: 5.0,
        mem_growth: 0.0004,
        limit_gib: Some(16.0),
    },
    QueryTemplate {
        initial_age_s: 12.0,
        user: "haris",
        person: None,
        redash_id: None,
        sql: "EXPLAIN SELECT count() FROM payments.not_initiated WHERE created_at > now() - INTERVAL 3 DAY",
        lifetime_s: 26.0,
        mem_share: 0.019,
        cores: 0.3,
        mem_growth: 0.0,
        limit_gib: None,
    },
];

const CLICKHOUSE_BI: &[QueryTemplate] = &[
    QueryTemplate {
        initial_age_s: 164.0,
        user: "r_redash",
        person: Some("j.petrova"),
        redash_id: Some(8585),
        sql: "SELECT\n  toStartOfHour(ts) AS h,\n  countIf(state = 'failed') AS failed\nFROM open_banking.ais\nWHERE ts >= now() - INTERVAL 6 HOUR\nGROUP BY h",
        lifetime_s: 420.0,
        mem_share: 0.33,
        cores: 1.9,
        mem_growth: 0.0002,
        limit_gib: Some(24.0),
    },
    QueryTemplate {
        initial_age_s: 18.0,
        user: "bi_loader",
        person: None,
        redash_id: None,
        sql: "INSERT INTO statistics.fx_exposure SELECT * FROM staging.fx_exposure_2026_10_03",
        lifetime_s: 28.0,
        mem_share: 0.2,
        cores: 3.2,
        mem_growth: 0.0005,
        limit_gib: Some(20.0),
    },
    // The one that climbs: 6.4 GiB growing ~43 MiB/s against a 9 GiB limit, so the insights
    // can forecast its end — and at 60 s, at 99% of its limit, it "is killed" and starts over.
    QueryTemplate {
        initial_age_s: 21.0,
        user: "r_redash",
        person: Some("m.kairys"),
        redash_id: Some(7711),
        sql: FX,
        lifetime_s: 60.0,
        mem_share: 0.10,
        cores: 1.1,
        mem_growth: 0.042,
        limit_gib: Some(9.0),
    },
];

const CLICKHOUSE7: &[QueryTemplate] = &[
    QueryTemplate {
        initial_age_s: 300.0,
        user: "r_redash",
        person: Some("r.simonyte"),
        redash_id: Some(8091),
        sql: JULY_CLOSE,
        lifetime_s: 700.0,
        mem_share: 0.4,
        cores: 2.2,
        mem_growth: 0.0003,
        limit_gib: Some(64.0),
    },
    QueryTemplate {
        initial_age_s: 12.0,
        user: "etl",
        person: None,
        redash_id: None,
        sql: "INSERT INTO accounting_events.daily SELECT * FROM accounting.raw WHERE event_date = today()",
        lifetime_s: 28.0,
        mem_share: 0.18,
        cores: 1.4,
        mem_growth: 0.0003,
        limit_gib: Some(32.0),
    },
];

const CLICKHOUSE2: &[QueryTemplate] = &[
    QueryTemplate {
        initial_age_s: 21.0,
        user: "r_redash",
        person: Some("m.kairys"),
        redash_id: Some(7711),
        sql: FX,
        lifetime_s: 28.0,
        mem_share: 0.24,
        cores: 1.2,
        mem_growth: 0.0002,
        limit_gib: Some(24.0),
    },
    QueryTemplate {
        initial_age_s: 9.0,
        user: "airflow",
        person: None,
        redash_id: None,
        sql: "INSERT INTO redash_activity.query_stats SELECT query_id, elapsed FROM system.query_log WHERE type = 'QueryFinish' AND event_time > now() - 300",
        lifetime_s: 25.0,
        mem_share: 0.1,
        cores: 0.8,
        mem_growth: 0.0001,
        limit_gib: Some(16.0),
    },
];

/// The node that appears after 20 s (§8).
const CLICKHOUSE5: &[QueryTemplate] = &[QueryTemplate {
        initial_age_s: 95.0,
    user: "r_redash",
    person: Some("grigol.gankava"),
    redash_id: Some(8092),
    sql: GATEWAY,
    lifetime_s: 300.0,
    mem_share: 0.22,
    cores: 1.1,
    mem_growth: 0.0002,
    limit_gib: Some(24.0),
}];

/// The quiet query the healthy nodes run. Its lifetime stays under the 30 s runaway mark on
/// purpose: a node that is quiet on memory and CPU must not stay on screen because one of its
/// queries happened to cross the threshold.
const CLUSTER_HISTORY_QUERY: &[QueryTemplate] = &[QueryTemplate {
    initial_age_s: 6.0,
    user: "dba",
    person: None,
    redash_id: None,
    sql: CLUSTER_HISTORY,
    lifetime_s: 25.0,
    mem_share: 0.03,
    cores: 0.5,
    mem_growth: 0.0,
    limit_gib: None,
}];

const NODES: &[NodeTemplate] = &[
    NodeTemplate {
        name: "clickhouse3",
        host: "pay-ch-node-1",
        shard: 1,
        replica: 1,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 90.9,
        cpu_pct: 94.4,
        lag_s: 0,
        parts: 1200,
        queries: CLICKHOUSE3,
    },
    NodeTemplate {
        name: "clickhouse-bi",
        host: "pay-ch-bi",
        shard: 1,
        replica: 2,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 68.2,
        cpu_pct: 75.0,
        lag_s: 0,
        parts: 640,
        queries: CLICKHOUSE_BI,
    },
    NodeTemplate {
        name: "clickhouse7",
        host: "pay-ch-node-7",
        shard: 1,
        replica: 1,
        mem_gib: 128.0,
        cores: 32.0,
        user_pct: 62.0,
        cpu_pct: 40.0,
        lag_s: 12,
        parts: 980,
        queries: CLICKHOUSE7,
    },
    NodeTemplate {
        name: "clickhouse2",
        host: "pay-ch-node-2",
        shard: 1,
        replica: 2,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 41.0,
        cpu_pct: 25.0,
        lag_s: 0,
        parts: 410,
        queries: CLICKHOUSE2,
    },
    NodeTemplate {
        name: "clickhouse5",
        host: "pay-ch-node-5",
        shard: 1,
        replica: 1,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 38.0,
        cpu_pct: 8.0,
        lag_s: 0,
        parts: 220,
        queries: CLICKHOUSE5,
    },
    NodeTemplate {
        name: "ch4",
        host: "pay-ch-node-4",
        shard: 1,
        replica: 1,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 21.0,
        cpu_pct: 14.0,
        lag_s: 0,
        parts: 180,
        queries: CLUSTER_HISTORY_QUERY,
    },
    NodeTemplate {
        name: "ch6",
        host: "pay-ch-node-6",
        shard: 1,
        replica: 2,
        mem_gib: 64.0,
        cores: 16.0,
        user_pct: 18.0,
        cpu_pct: 11.0,
        lag_s: 3,
        parts: 150,
        queries: CLUSTER_HISTORY_QUERY,
    },
    NodeTemplate {
        name: "ch8",
        host: "pay-ch-node-8",
        shard: 1,
        replica: 1,
        mem_gib: 128.0,
        cores: 32.0,
        user_pct: 12.0,
        cpu_pct: 9.0,
        lag_s: 0,
        parts: 90,
        queries: CLUSTER_HISTORY_QUERY,
    },
    NodeTemplate {
        name: "ch9",
        host: "pay-ch-node-9",
        shard: 1,
        replica: 2,
        mem_gib: 128.0,
        cores: 32.0,
        user_pct: 9.0,
        cpu_pct: 6.0,
        lag_s: 0,
        parts: 70,
        queries: CLUSTER_HISTORY_QUERY,
    },
];

/// The node the fleet "discovers" after 20 s (§8: one NEW node appearing after 20 s).
const NEW_AFTER: Duration = Duration::from_secs(20);

/// A running query as the generator sees it: the template plus when it started.
#[derive(Clone)]
struct Live {
    template: &'static QueryTemplate,
    node: &'static str,
    /// When the query (re)started.
    born_at: Duration,
    /// How long it had already been running when the session started, so the first poll shows
    /// queries at believable ages instead of all at zero.
    age_bias: f64,
    id: String,
}

pub struct FakeSource {
    rng: Rng,
    epoch: SystemTime,
    /// The fleet's clock: one `POLL_MS` per poll.
    tick: Duration,
    /// The queue's clock: one 3 s per queue poll (§6.3). Kept apart from the fleet's, or every
    /// queue poll would also make the queries look older than they are.
    queue_tick: Duration,
    live: Vec<Live>,
    seq: u64,
    /// Jobs cancelled from view 2: gone from the queue from then on.
    cancelled: std::collections::HashSet<String>,
}

impl Default for FakeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSource {
    pub fn new() -> Self {
        let epoch = SystemTime::now();
        Self {
            rng: Rng(0x5eed_1234),
            epoch,
            tick: Duration::ZERO,
            queue_tick: Duration::ZERO,
            live: Vec::new(),
            seq: 0,
            cancelled: Default::default(),
        }
    }

    fn now(&self) -> SystemTime {
        self.epoch + self.tick
    }

    fn templates(&self) -> Vec<(&'static NodeTemplate, &'static [QueryTemplate])> {
        let mut out: Vec<(&'static NodeTemplate, &'static [QueryTemplate])> = NODES
            .iter()
            .map(|n| (n, n.queries))
            .collect();
        // Before the 20 s mark clickhouse5 is not in the fleet yet (§8).
        if self.tick < NEW_AFTER {
            out.retain(|(n, _)| n.name != "clickhouse5");
        }
        out
    }

    /// Start every template already partway through its life, so the first poll already has a
    /// fleet with queries on it instead of a screen full of 0 s rows.
    fn seed_live(&mut self, node: &'static NodeTemplate, templates: &'static [QueryTemplate]) {
        for template in templates.iter() {
            self.seq += 1;
            self.live.push(Live {
                template,
                node: node.name,
                born_at: Duration::ZERO,
                age_bias: template.initial_age_s,
                id: format!("{:08x}", (0xc3e5_0000u64 + self.seq * 0x1f3b) & 0xffff_ffff),
            });
        }
    }

    /// Retire queries past their lifetime and start a fresh one from the same template.
    fn churn(&mut self, fleet: &[(&'static NodeTemplate, &'static [QueryTemplate])]) {
        let tick = self.tick;
        let mut finished: Vec<usize> = Vec::new();
        for (i, live) in self.live.iter().enumerate() {
            let elapsed = tick.saturating_sub(live.born_at).as_secs_f64() + live.age_bias;
            // Turnover is the template's lifetime, runaway or not: a query that crossed 30 s and
            // then never left would leave a permanent red row on a quiet node, and a quiet node
            // that always looks busy is worse than no fake data at all. The long templates are
            // the long ones on purpose — those are the runaways.
            if elapsed > live.template.lifetime_s {
                finished.push(i);
            }
        }
        for i in finished.into_iter().rev() {
            let old = self.live.remove(i);
            self.seq += 1;
            self.live.push(Live {
                template: old.template,
                node: old.node,
                born_at: self.tick,
                age_bias: 0.0,
                id: format!("{:08x}", (0xc3e5_0000u64 + self.seq * 0x1f3b) & 0xffff_ffff),
            });
        }
        // Anything that belongs to a node that is not in the fleet right now stays out.
        let names: Vec<&str> = fleet.iter().map(|(n, _)| n.name).collect();
        self.live.retain(|l| names.contains(&l.node));
    }

    fn query_row(&mut self, live: &Live, node: &NodeTemplate) -> QueryRow {
        let elapsed = self.tick.saturating_sub(live.born_at).as_secs_f64() + live.age_bias;
        let template = live.template;
        let mem_base = template.mem_share * node.mem_gib * GIB;
        let growth = template.mem_growth * elapsed * GIB;
        let mem_bytes = (mem_base + growth) as u64;
        // cpu_time_us is cumulative and grows at exactly the cores the row will claim, so
        // the §5.2 average form and the delta form agree.
        let cpu_time_us = (template.cores * elapsed * 1e6) as u64;

        let sql = template.sql_with_comment();
        let (person, redash_id) = if template.user == crate::attrib::REDASH_USER {
            (template.person.map(str::to_string), template.redash_id)
        } else {
            (None, None)
        };

        // Reads are a steady scan through an input ClickHouse knows the size of, so progress
        // is the share of the template's lifetime gone by and the ETA is what is left of it.
        let rows_per_s = 4.0e6 * template.cores.max(0.2);
        let total_rows = rows_per_s * template.lifetime_s;
        let read_rows = (rows_per_s * elapsed).min(total_rows);
        let summary = crate::sqltext::summary(template.sql);

        let mut row = QueryRow::new(&live.id, template.user);
        row.person = person;
        row.redash_query_id = redash_id;
        row.elapsed_s = elapsed;
        row.memory_bytes = mem_bytes;
        row.peak_memory_bytes = mem_bytes;
        row.read_rows = read_rows as u64;
        row.read_bytes = (read_rows * 22.0) as u64;
        row.total_rows_approx = total_rows as u64;
        row.sql = sql;
        row.cpu_time_us = cpu_time_us;
        row.memory_limit = template.limit_gib.map(|g| (g * GIB) as u64);
        row.kind = Some(
            match summary.verb {
                "INSERT" => "Insert",
                "EXPLAIN" => "Explain",
                _ => "Select",
            }
            .to_string(),
        );
        if summary.verb == "INSERT" {
            row.written_rows = (read_rows * 0.9) as u64;
        }
        row
    }

    /// One poll's worth of fleet. Call as often as POLL_MS says.
    pub fn snapshot(&mut self) -> FleetSnapshot {
        self.tick += Duration::from_millis(2000);
        let fleet = self.templates();

        if self.live.is_empty() {
            for (node, templates) in &fleet {
                self.seed_live(node, templates);
            }
        }
        self.churn(&fleet);

        let mut nodes: Vec<NodeSnapshot> = Vec::new();
        for (template, _) in &fleet {
            let live: Vec<Live> = self
                .live
                .iter()
                .filter(|l| l.node == template.name)
                .cloned()
                .collect();

            let queries: Vec<QueryRow> = live.iter().map(|l| self.query_row(l, template)).collect();

            let user_mem: f64 = queries.iter().map(|q| q.memory_bytes as f64).sum();
            let user_cores: f64 = queries
                .iter()
                .map(|q| crate::model::query_cores(q, None, Some(template.cores)))
                .sum();

            // One node misses polls for half a minute, two minutes in, so the tape and the
            // insights have an outage to report and the screen shows a gap, not zeros.
            let t = self.tick.as_secs_f64();
            if template.name == "ch6" && (120.0..150.0).contains(&t) {
                nodes.push(NodeSnapshot::unreachable(template.name, "connection timed out after 1.5 s"));
                continue;
            }

            // The node's own usage is its users plus whatever the server needs, chosen so the
            // node lands on the target percentage: Σ user + closing row == the node (§5.3).
            // The floor keeps at least 1% for the server when the users alone would fill it.
            let (mem_pct, cpu_pct) = load(template, t);
            let mem_total = (template.mem_gib * GIB) as u64;
            let mem_target = mem_pct / 100.0 * mem_total as f64;
            let server_mem = (mem_target - user_mem).max(mem_total as f64 * 0.01);
            let mem_used = (user_mem + server_mem) as u64;

            let cpu_target = cpu_pct / 100.0 * template.cores;
            let busy_cores =
                (cpu_target.max(user_cores + template.cores * 0.01)).min(template.cores);
            // A believable answer time: tens of milliseconds, and one node that is slow.
            let base_ms = if template.name == "clickhouse7" { 820.0 } else { 25.0 + 10.0 * template.cores / 16.0 };
            let poll_ms = (base_ms + self.rng.range(0.0, base_ms * 0.4)) as u32;
            nodes.push(NodeSnapshot {
                name: template.name.to_string(),
                host: template.host.to_string(),
                port: 9000,
                shard: template.shard,
                replica: template.replica,
                version: "24.11.1.2557".to_string(),
                reachable: true,
                mem_total: Some(mem_total),
                mem_used,
                cores: Some(template.cores),
                cpu_busy_cores: Some(busy_cores),
                running: queries.len() as u32,
                lag_s: template.lag_s,
                active_parts: template.parts,
                queries,
                uptime_s: Some(4_000_000 + self.tick.as_secs()),
                server_cpu_time_us: Some(self.tick.as_micros() as u64 * 800_000),
                max_memory_usage: Some((9.0 * GIB) as u64),
                unreachable_reason: None,
                poll_ms: Some(poll_ms),
            });
        }

        FleetSnapshot {
            taken_at: self.now(),
            nodes,
        }
    }

    /// A backed-up Redash (§8): the four `queries` workers all busy — three on ClickHouse, one
    /// on MySQL — 12 waiting, the oldest past 1m40s; a scheduled refresh on ClickHouse and one
    /// on Query Results; and three entries RQ's started list never let go of.
    /// The queue 3 s on.
    pub fn queue(&mut self) -> QueueStatus {
        self.queue_tick += Duration::from_millis(3000);
        self.queue_now()
    }

    /// A job cancelled, as Redash would: a waiting one leaves its queue, a running one its
    /// worker, a leftover the started list. Its query in ClickHouse, if any, runs on.
    pub fn cancel(&mut self, id: &str) -> Result<(), String> {
        if !self.queue_now().jobs.iter().any(|job| job.id == id) {
            return Err("HTTP 500 · Redash no longer has it — it may have just finished".into());
        }
        self.cancelled.insert(id.to_string());
        Ok(())
    }

    /// The queue as it is now, without moving its clock.
    pub fn queue_now(&self) -> QueueStatus {
        let grown = self.queue_tick.as_secs();
        let clickhouse = |job: &mut Job, source: &str| {
            job.data_source = Some(source.to_string());
            job.data_source_type = Some("clickhouse".to_string());
        };
        let person = |job: &mut Job, local: &str| {
            job.person = Some(local.to_string());
            job.person_full = Some(format!("{local}@{DOMAIN}"));
        };
        let query = |job: &mut Job, id: u64, name: &str, sql: &str| {
            job.redash_query_id = Some(id);
            job.query_name = Some(name.to_string());
            job.sql = Some(sql.to_string());
        };

        let mut jobs: Vec<Job> = Vec::new();
        type Spec = (&'static str, u64, &'static str, &'static str, &'static str);
        let waiting: &[Spec] = &[
            ("r.simonyte", 8093, "July close pack · by country", JULY_CLOSE, "clickhouse-bi"),
            ("j.petrova", 8113, "AML dashboard · by country", AML, "clickhouse3"),
            ("m.kairys", 7712, "FX exposure · intraday", FX, "clickhouse-bi"),
            ("grigol.gankava", 8092, "Gateway transfers · hourly", GATEWAY, "clickhouse3"),
            ("d.zaleckas", 8120, "Chargebacks · weekly", GATEWAY, "clickhouse-bi"),
        ];
        let mut oldest_wait = 0u64;
        for (i, (who, id, name, sql, source)) in waiting.iter().enumerate() {
            let mut job = Job::new(format!("wait-{}", i + 1), JobState::Queued, "queries");
            person(&mut job, who);
            query(&mut job, *id, name, sql);
            clickhouse(&mut job, source);
            job.age_s = (100 + grown).saturating_sub(i as u64 * 18);
            oldest_wait = oldest_wait.max(job.age_s);
            jobs.push(job);
        }

        // On a worker, and in ClickHouse: the stitch finds them by their Redash number.
        let running: &[Spec] = &[
            ("grigol.gankava", 7438, "Gateway transfers", AML, "clickhouse3"),
            ("j.petrova", 8585, "AML dashboard", AML, "clickhouse-bi"),
            ("m.kairys", 7711, "FX exposure", FX, "clickhouse2"),
        ];
        for (i, (who, id, name, sql, source)) in running.iter().enumerate() {
            let mut job = Job::new(format!("run-{}", i + 1), JobState::Started, "queries");
            person(&mut job, who);
            query(&mut job, *id, name, sql);
            clickhouse(&mut job, source);
            job.age_s = (275 + grown).saturating_sub(i as u64 * 60);
            jobs.push(job);
        }
        // The fourth worker is on MySQL, which this monitor does not watch.
        let mut mysql = Job::new("run-4", JobState::Started, "queries");
        person(&mut mysql, "d.zaleckas");
        query(&mut mysql, 6120, "Gateway refunds · open", REFUNDS);
        mysql.data_source = Some("gateway-mysql".to_string());
        mysql.data_source_type = Some("mysql".to_string());
        mysql.age_s = 48 + grown;
        jobs.push(mysql);

        // Scheduled refreshes: one on ClickHouse, one on Query Results, which runs inside
        // Redash itself.
        let mut july = Job::new("sched-run-1", JobState::Started, "scheduled_queries");
        person(&mut july, "r.simonyte");
        query(&mut july, 8091, "July close pack", JULY_CLOSE);
        clickhouse(&mut july, "clickhouse7");
        july.scheduled = true;
        july.age_s = 140 + grown;
        jobs.push(july);
        let mut risk = Job::new("sched-run-2", JobState::Started, "scheduled_queries");
        person(&mut risk, "a.vaitkus");
        query(&mut risk, 8470, "Merchant risk · rollup", "SELECT merchant, sum(score) AS risk FROM query_8466 GROUP BY merchant");
        risk.data_source = Some("Query Results".to_string());
        risk.data_source_type = Some("results".to_string());
        risk.scheduled = true;
        risk.age_s = 31 + grown.min(20);
        jobs.push(risk);
        for (i, (name, who)) in [("Refresh merchant risk", "a.vaitkus"), ("Nightly settlement", "d.zaleckas")]
            .iter()
            .enumerate()
        {
            let mut job = Job::new(format!("sched-{}", i + 1), JobState::Queued, "scheduled_queries");
            person(&mut job, who);
            job.query_name = Some((*name).to_string());
            clickhouse(&mut job, "clickhouse-bi");
            job.scheduled = true;
            job.age_s = 22 + grown.min(30) + (i as u64 * 9);
            jobs.push(job);
        }

        // What RQ's started list still holds without anyone running it.
        let mut cancelled = Job::new("stale-1", JobState::Stale(Stale::Cancelled), "queries");
        person(&mut cancelled, "a.vaitkus");
        query(&mut cancelled, 6301, "Card margin · by day", "SELECT day, sum(margin) FROM cards.daily GROUP BY day");
        cancelled.data_source = Some("payments-mysql".to_string());
        cancelled.data_source_type = Some("mysql".to_string());
        cancelled.age_s = 3 * 86_400 + 4_000;
        jobs.push(cancelled);
        let mut old = Job::new("stale-2", JobState::Stale(Stale::OverADay), "queries");
        person(&mut old, "j.petrova");
        query(&mut old, 6302, "Ledger export · full", JULY_CLOSE);
        clickhouse(&mut old, "clickhouse-bi");
        old.age_s = 41 * 86_400 + 7_200;
        jobs.push(old);
        let mut orphan = Job::new("stale-3", JobState::Stale(Stale::NoWorker), "queries");
        person(&mut orphan, "d.zaleckas");
        query(&mut orphan, 162, "Replica health check", "SELECT * FROM query_160 WHERE lag > 60");
        orphan.data_source = Some("Query Results".to_string());
        orphan.data_source_type = Some("results".to_string());
        orphan.age_s = 742 + grown;
        jobs.push(orphan);

        // The queue breathes: a dashboard refresh lands, the workers chew through it.
        let t = self.queue_tick.as_secs_f64();
        let breathing = (12.0 + 4.0 * (t / 50.0 * std::f64::consts::TAU).sin()).round().max(0.0) as u32;
        let row = |name: &str, running: u32, waiting: u32, oldest: Option<u64>, stale: u32, busy: u32, total: u32| QueueRow {
            name: name.to_string(),
            running,
            waiting,
            oldest_wait_s: oldest,
            stale,
            workers_busy: busy,
            workers_total: total,
        };
        let mut queues = vec![
            row("default", 0, 0, None, 0, 0, 1),
            row("emails", 0, 0, None, 0, 0, 1),
            row("periodic", 0, 0, None, 0, 0, 1),
            row("queries", 4, breathing, Some(oldest_wait), 3, 4, 4),
            row("scheduled_queries", 2, 3, Some(22 + grown.min(30) + 9), 0, 2, 2),
            row("schemas", 0, 0, None, 0, 2, 2),
        ];
        // What was cancelled is gone, and its queue counts one less of it.
        let mut freed = 0;
        for job in jobs.iter().filter(|job| self.cancelled.contains(&job.id)) {
            let Some(row) = queues.iter_mut().find(|q| q.name == job.queue) else { continue };
            match job.state {
                JobState::Queued => row.waiting = row.waiting.saturating_sub(1),
                JobState::Started => {
                    row.running = row.running.saturating_sub(1);
                    row.workers_busy = row.workers_busy.saturating_sub(1);
                    freed += 1;
                }
                JobState::Stale(_) => row.stale = row.stale.saturating_sub(1),
            }
        }
        jobs.retain(|job| !self.cancelled.contains(&job.id));

        QueueStatus {
            reachable: true,
            error: None,
            queues,
            jobs,
            names_available: true,
            // Four on `queries`, two on scheduled_queries and schemas, one for the rest.
            workers_busy: 6u32.saturating_sub(freed),
            workers_total: 7,
            host: Some(format!("redash.{DOMAIN}")),
            version: Some("10.1.0".to_string()),
            taken_at: self.epoch + self.queue_tick,
        }
    }

}

// -- query sessions ---------------------------------------------------------

/// The made-up fleet's databases, tables and columns, as a server lists them: the system tables
/// and the ones its queries read.
pub fn schema() -> crate::complete::Schema {
    use crate::complete::{Schema, Table};
    let table = |database: &str, name: &str, engine: &str, columns: &[(&str, &str)]| Table {
        database: database.into(),
        name: name.into(),
        engine: engine.into(),
        columns: columns.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
        ..Default::default()
    };
    let tables = vec![
        table("accounting", "raw", "ReplicatedMergeTree", &[("event_time", "DateTime"), ("event_date", "Date"), ("user", "String"), ("amount", "Decimal(18, 2)")]),
        table("accounting_lt", "bank_record", "ReplicatedMergeTree", &[("BillOpId", "UInt64"), ("Bank", "LowCardinality(String)"), ("AccNr", "String"), ("amount", "String"), ("EventDate", "Date")]),
        table("default", "refunds", "ReplicatedMergeTree", &[("id", "UInt64"), ("amount", "Decimal(18, 2)"), ("status", "LowCardinality(String)"), ("created_at", "DateTime")]),
        table("gateway", "transfers", "ReplicatedMergeTree", &[("ts", "DateTime"), ("status", "LowCardinality(String)"), ("amount", "Decimal(18, 2)"), ("merchant_id", "UInt64"), ("currency", "LowCardinality(String)")]),
        table("open_banking", "ais", "ReplicatedMergeTree", &[("ts", "DateTime"), ("state", "LowCardinality(String)"), ("bank", "LowCardinality(String)")]),
        table("payments", "not_initiated", "ReplicatedMergeTree", &[("created_at", "DateTime"), ("payment_id", "UUID"), ("amount", "Decimal(18, 2)")]),
        table("statistics", "daily_rollup", "ReplicatedSummingMergeTree", &[("d", "Date"), ("user", "String"), ("amount", "Decimal(18, 2)")]),
        table("statistics", "fx_exposure", "ReplicatedMergeTree", &[("event_date", "Date"), ("currency", "LowCardinality(String)"), ("amount", "Decimal(18, 2)"), ("merchant_id", "UInt64")]),
        table("wallet", "ledger", "ReplicatedMergeTree", &[("merchant_id", "UInt64"), ("month", "UInt8"), ("amount", "Decimal(18, 2)"), ("created_at", "DateTime")]),
        table("system", "asynchronous_metrics", "SystemAsynchronousMetrics", &[("metric", "String"), ("value", "Float64"), ("description", "String")]),
        table("system", "clusters", "SystemClusters", &[("cluster", "String"), ("shard_num", "UInt32"), ("replica_num", "UInt32"), ("host_name", "String"), ("port", "UInt16"), ("is_local", "UInt8")]),
        table("system", "columns", "SystemColumns", &[("database", "String"), ("table", "String"), ("name", "String"), ("type", "String"), ("position", "UInt64"), ("data_compressed_bytes", "UInt64")]),
        table("system", "databases", "SystemDatabases", &[("name", "String"), ("engine", "String"), ("data_path", "String")]),
        table("system", "disks", "SystemDisks", &[("name", "String"), ("path", "String"), ("free_space", "UInt64"), ("total_space", "UInt64")]),
        table("system", "events", "SystemEvents", &[("event", "String"), ("value", "UInt64"), ("description", "String")]),
        table("system", "merges", "SystemMerges", &[("database", "String"), ("table", "String"), ("elapsed", "Float64"), ("progress", "Float64"), ("num_parts", "UInt64"), ("total_size_bytes_compressed", "UInt64"), ("memory_usage", "UInt64")]),
        table("system", "metrics", "SystemMetrics", &[("metric", "String"), ("value", "Int64"), ("description", "String")]),
        table("system", "mutations", "SystemMutations", &[("database", "String"), ("table", "String"), ("mutation_id", "String"), ("command", "String"), ("create_time", "DateTime"), ("is_done", "UInt8"), ("latest_fail_reason", "String")]),
        table("system", "parts", "SystemParts", &[("database", "String"), ("table", "String"), ("partition", "String"), ("name", "String"), ("active", "UInt8"), ("rows", "UInt64"), ("bytes_on_disk", "UInt64"), ("modification_time", "DateTime")]),
        table("system", "processes", "SystemProcesses", &[("is_initial_query", "UInt8"), ("user", "String"), ("query_id", "String"), ("elapsed", "Float64"), ("read_rows", "UInt64"), ("read_bytes", "UInt64"), ("total_rows_approx", "UInt64"), ("memory_usage", "Int64"), ("peak_memory_usage", "Int64"), ("query", "String"), ("query_kind", "String"), ("current_database", "String")]),
        table("system", "query_log", "SystemQueryLog", &[("type", "Enum8('QueryStart' = 1, 'QueryFinish' = 2, 'ExceptionBeforeStart' = 3, 'ExceptionWhileProcessing' = 4)"), ("event_date", "Date"), ("event_time", "DateTime"), ("query_duration_ms", "UInt64"), ("read_rows", "UInt64"), ("read_bytes", "UInt64"), ("result_rows", "UInt64"), ("memory_usage", "UInt64"), ("query", "String"), ("query_kind", "LowCardinality(String)"), ("exception_code", "Int32"), ("exception", "String"), ("user", "String"), ("query_id", "String"), ("log_comment", "String")]),
        table("system", "replicas", "SystemReplicas", &[("database", "String"), ("table", "String"), ("is_leader", "UInt8"), ("is_readonly", "UInt8"), ("absolute_delay", "UInt64"), ("queue_size", "UInt32"), ("active_replicas", "UInt8"), ("total_replicas", "UInt8")]),
        table("system", "tables", "SystemTables", &[("database", "String"), ("name", "String"), ("engine", "String"), ("total_rows", "Nullable(UInt64)"), ("total_bytes", "Nullable(UInt64)"), ("metadata_modification_time", "DateTime")]),
    ];
    let mut databases: Vec<String> = tables.iter().map(|t| t.database.clone()).collect();
    databases.sort();
    databases.dedup();
    Schema { databases, tables, functions: Schema::fallback().functions }
}

/// What the made-up fleet answers a query session — `system.processes` from the snapshot, its
/// tables, parts, databases and log, a few rows of its own tables — and how it refuses: a write
/// as a read-only login's, a table it does not have as unknown.
pub fn console_answer(snapshot: Option<&FleetSnapshot>, node: &str, sql: &str) -> Result<crate::console::Answer, String> {
    use crate::console::Answer;
    let text = crate::sqltext::strip_comments(sql);
    let lower = crate::sqltext::collapse(&text).to_lowercase();
    let first = lower.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()).unwrap_or_default().to_string();
    let writes = ["insert", "alter", "drop", "create", "truncate", "optimize", "kill", "delete", "update", "system", "rename", "grant", "revoke", "attach", "detach"];
    if writes.contains(&first.as_str()) {
        return Err("Code 164 · monitor: Cannot execute query in readonly mode. (READONLY)".into());
    }
    let schema = schema();
    let at = node_of(snapshot, node);
    let column = |name: &str, kind: &str| (name.to_string(), kind.to_string());
    let answer = |columns: Vec<(String, String)>, rows: Vec<Vec<Option<String>>>| Answer {
        node: node.to_string(),
        read_rows: Some(rows.len() as u64 * 37 + 1),
        read_bytes: Some(rows.len() as u64 * 4096 + 512),
        columns,
        rows,
        ..Answer::default()
    };
    let limit = lower.split("limit ").nth(1).and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse::<usize>().ok());
    let grouped_by_user = lower.contains("group by") && lower.contains("user");

    if lower.contains("system.processes") || lower.starts_with("show processlist") {
        let queries: Vec<&QueryRow> = at.map(|n| n.queries.iter().collect()).unwrap_or_default();
        if grouped_by_user {
            let mut users: Vec<(String, usize, u64)> = Vec::new();
            for query in &queries {
                match users.iter_mut().find(|(user, ..)| *user == query.user) {
                    Some(entry) => {
                        entry.1 += 1;
                        entry.2 += query.memory_bytes;
                    }
                    None => users.push((query.user.clone(), 1, query.memory_bytes)),
                }
            }
            users.sort_by_key(|entry| std::cmp::Reverse(entry.2));
            let rows = users.into_iter().take(limit.unwrap_or(1000)).map(|(user, n, memory)| vec![Some(user), Some(n.to_string()), Some(crate::fmt::bytes(memory))]).collect();
            return Ok(answer(vec![column("user", "String"), column("queries", "UInt64"), column("memory", "String")], rows));
        }
        let rows = queries
            .iter()
            .take(limit.unwrap_or(1000))
            .map(|q| {
                vec![
                    Some(q.query_id.clone()),
                    Some(q.user.clone()),
                    Some(format!("{:.1}", q.elapsed_s)),
                    Some(crate::fmt::bytes(q.memory_bytes)),
                    Some(crate::sqltext::collapse(&q.sql)),
                ]
            })
            .collect();
        let columns = vec![column("query_id", "String"), column("user", "String"), column("elapsed", "Float64"), column("memory", "String"), column("query", "String")];
        return Ok(answer(columns, rows));
    }
    if lower.contains("system.databases") || lower.starts_with("show databases") {
        return Ok(answer(vec![column("name", "String")], schema.databases.iter().map(|d| vec![Some(d.clone())]).collect()));
    }
    if lower.contains("system.tables") || lower.starts_with("show tables") {
        let rows = schema
            .tables
            .iter()
            .filter(|t| t.database != "system")
            .enumerate()
            .map(|(i, t)| vec![Some(t.database.clone()), Some(t.name.clone()), Some(t.engine.clone()), Some(((i as u64 + 3) * 7_919_311).to_string())])
            .collect();
        return Ok(answer(vec![column("database", "String"), column("name", "String"), column("engine", "String"), column("total_rows", "Nullable(UInt64)")], rows));
    }
    if lower.contains("system.parts") {
        let mut rows: Vec<Vec<Option<String>>> = schema
            .tables
            .iter()
            .filter(|t| t.database != "system")
            .enumerate()
            .map(|(i, t)| {
                let bytes = (i as u64 * 37 + 11) * 1_311_000_000 % 900_000_000_000;
                vec![Some(t.database.clone()), Some(t.name.clone()), Some((i * 13 + 9).to_string()), Some(((i as u64 + 2) * 48_211_017).to_string()), Some(crate::fmt::bytes(bytes))]
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row[3].as_ref().and_then(|r| r.parse::<u64>().ok()).unwrap_or(0)));
        rows.truncate(limit.unwrap_or(1000));
        let columns = vec![column("database", "String"), column("table", "String"), column("parts", "UInt64"), column("rows", "UInt64"), column("size", "String")];
        return Ok(answer(columns, rows));
    }
    if lower.contains("system.query_log") {
        let rows = [("r_redash", 41_210, "SELECT region, count() FROM accounting_lt.bank_record GROUP BY region"), ("airflow", 18_377, "INSERT INTO statistics.daily_rollup SELECT …"), ("grafana", 912, "SELECT toStartOfMinute(ts), count() FROM gateway.transfers …"), ("r_redash", 655, "SELECT currency, sum(amount) FROM statistics.fx_exposure …")]
            .iter()
            .take(limit.unwrap_or(1000))
            .map(|(user, ms, query)| vec![Some("2026-10-04 15:4".to_string() + &(ms % 10).to_string() + ":07"), Some(user.to_string()), Some(ms.to_string()), Some(query.to_string())])
            .collect();
        let columns = vec![column("event_time", "DateTime"), column("user", "String"), column("query_duration_ms", "UInt64"), column("query", "String")];
        return Ok(answer(columns, rows));
    }
    // A table of its own: a few rows of it; a table it does not have, unknown.
    let summary = crate::sqltext::summary(&text);
    if let Some(target) = summary.target.filter(|t| !t.ends_with("()")) {
        let (database, name) = match target.split_once('.') {
            Some((db, name)) => (Some(db), name),
            None => (None, target.as_str()),
        };
        let Some(table) = schema.table(database, name) else {
            return Err(format!("Code 60 · Unknown table expression identifier '{target}' in scope SELECT. (UNKNOWN_TABLE)"));
        };
        if first == "describe" || first == "desc" {
            return Ok(answer(vec![column("name", "String"), column("type", "String")], table.columns.iter().map(|(n, t)| vec![Some(n.clone()), Some(t.clone())]).collect()));
        }
        let rows = (0..limit.unwrap_or(8).min(8)).map(|i| table.columns.iter().map(|(_, kind)| Some(sample(kind, i))).collect()).collect();
        return Ok(answer(table.columns.clone(), rows));
    }
    if first == "select" {
        let expression = crate::sqltext::collapse(text.trim().trim_end_matches(';')).chars().skip(7).collect::<String>();
        let value = match expression.trim() {
            "version()" => at.map(|n| n.version.clone()).unwrap_or_default(),
            "hostName()" => node.to_string(),
            literal if literal.parse::<f64>().is_ok() => literal.to_string(),
            literal if literal.starts_with('\'') && literal.ends_with('\'') && literal.len() >= 2 => literal[1..literal.len() - 1].to_string(),
            _ => "1".to_string(),
        };
        return Ok(answer(vec![column(expression.trim(), "String")], vec![vec![Some(value)]]));
    }
    Ok(Answer { node: node.to_string(), text: Some("FAKE=1 answers system.processes, .tables, .parts, .databases, .query_log and its own tables".into()), ..Answer::default() })
}

fn node_of<'a>(snapshot: Option<&'a FleetSnapshot>, node: &str) -> Option<&'a NodeSnapshot> {
    snapshot?.nodes.iter().find(|n| n.name == node)
}

/// A made-up value of a column's type, the `i`th of a few.
fn sample(kind: &str, i: usize) -> String {
    let kind = kind.trim_start_matches("Nullable(").trim_start_matches("LowCardinality(");
    match kind {
        k if k.starts_with("Date") && !k.starts_with("DateTime") => format!("2026-10-{:02}", 4 - (i % 4)),
        k if k.starts_with("DateTime") => format!("2026-10-04 15:{:02}:{:02}", 51 - i % 50, (i * 17) % 60),
        k if k.starts_with("Decimal") || k.starts_with("Float") => format!("{}.{:02}", 1200 + i * 317, (i * 29) % 100),
        k if k.starts_with("UInt") || k.starts_with("Int") => (10_421 + i * 7).to_string(),
        "UUID" => format!("6f1c{i:04x}-2b7e-4c1a-9d3e-0a5b8c7d{:04x}", i * 31),
        _ => ["EUR", "open", "settled", "GBP", "failed", "USD", "pending", "LT"][i % 8].to_string(),
    }
}

/// What the made-up helper writes: a query for what the comments ask — the users by memory, the
/// biggest tables, the slowest queries, what runs — with the comments kept, after a moment.
pub fn assist(ask: &crate::console::Ask) -> Result<String, String> {
    // A part comes back as it was, said to be made up.
    if let Some(part) = &ask.selected {
        return Ok(format!("/* FAKE=1: as it was */ {}", part.trim()));
    }
    let comments: Vec<&str> = ask.sql.lines().filter(|l| l.trim_start().starts_with("--")).collect();
    let words = format!("{} {}", ask.sql, ask.instruction).to_lowercase();
    let has = |list: &[&str]| list.iter().any(|w| words.contains(w));
    let body = if has(&["memory", "memori", "ram"]) {
        "SELECT user, count() AS queries, formatReadableSize(sum(memory_usage)) AS memory\nFROM system.processes\nGROUP BY user\nORDER BY sum(memory_usage) DESC\nLIMIT 10;"
    } else if has(&["biggest", "largest", "size", "disk", "besar", "table", "tabel"]) {
        "SELECT database, table, count() AS parts, sum(rows) AS rows, formatReadableSize(sum(bytes_on_disk)) AS size\nFROM system.parts\nWHERE active\nGROUP BY database, table\nORDER BY sum(bytes_on_disk) DESC\nLIMIT 10;"
    } else if has(&["slow", "lambat", "lama", "log", "yesterday", "kemarin"]) {
        "SELECT event_time, user, query_duration_ms, query\nFROM system.query_log\nWHERE type = 'QueryFinish' AND event_date = today()\nORDER BY query_duration_ms DESC\nLIMIT 10;"
    } else if ask.error.is_some() && !ask.sql.trim().is_empty() {
        "SELECT query_id, user, elapsed, formatReadableSize(memory_usage) AS memory, query\nFROM system.processes\nORDER BY elapsed DESC;"
    } else {
        "SELECT query_id, user, round(elapsed, 1) AS elapsed, formatReadableSize(memory_usage) AS memory, query\nFROM system.processes\nORDER BY elapsed DESC\nLIMIT 20;"
    };
    Ok(if comments.is_empty() { body.to_string() } else { format!("{}\n{body}", comments.join("\n")) })
}

// -- Airflow and Jira, views 5 and 6 ----------------------------------------------------------

/// A made-up DAG on a schedule: every `every_s` seconds from `phase_s` past midnight UTC, each
/// run taking about `took_s`. `fails` counts back from its latest run that has finished: `Some(0)`
/// is the latest, which failed at `failed_task`.
struct FakeDag {
    id: &'static str,
    owner: &'static str,
    schedule: &'static str,
    cron: &'static str,
    every_s: i64,
    phase_s: i64,
    took_s: i64,
    tasks: u32,
    fails: Option<usize>,
    failed_task: &'static str,
    running_task: &'static str,
    tags: &'static [&'static str],
    description: &'static str,
}

const FAKE_DAGS: &[FakeDag] = &[
    FakeDag { id: "clickhouse_replication_check", owner: "data-platform", schedule: "Every 15 minutes", cron: "*/15 * * * *", every_s: 900, phase_s: 0, took_s: 95, tasks: 3, fails: None, failed_task: "", running_task: "compare_row_counts", tags: &["clickhouse", "replication"], description: "Row counts of every replicated table against its source" },
    FakeDag { id: "redash_scheduler_monitoring", owner: "airflow", schedule: "", cron: "1h", every_s: 3600, phase_s: 0, took_s: 40, tasks: 2, fails: None, failed_task: "", running_task: "read_rq_status", tags: &["redash"], description: "Alerts when Redash's scheduled queries stop running" },
    FakeDag { id: "postgres_blacklist_etl", owner: "tomas.r", schedule: "Every 30 minutes", cron: "*/30 * * * *", every_s: 1800, phase_s: 0, took_s: 370, tasks: 6, fails: None, failed_task: "", running_task: "restriction_restrictions_etl_process", tags: &["postgres", "etl"], description: "Restrictions and blacklists from PostgreSQL into ClickHouse" },
    FakeDag { id: "cbk_accounts_report", owner: "ana.k", schedule: "At 15 minutes past the hour, every 2 hours", cron: "15 */2 * * *", every_s: 7200, phase_s: 900, took_s: 240, tasks: 5, fails: Some(4), failed_task: "upload_report", running_task: "build_report", tags: &["accounting", "reports"], description: "The CBK accounts report, uploaded to the regulator's SFTP" },
    FakeDag { id: "kyc_onboarding_tables", owner: "ana.k", schedule: "At 05:30", cron: "30 5 * * *", every_s: 86_400, phase_s: 5 * 3600 + 1800, took_s: 720, tasks: 8, fails: Some(0), failed_task: "build_cohorts", running_task: "build_cohorts", tags: &["kyc", "posthog"], description: "Onboarding funnel cohorts and questionnaire decisions" },
    FakeDag { id: "accounting_statement_daily_agg", owner: "data-platform", schedule: "At 03:00", cron: "0 3 * * *", every_s: 86_400, phase_s: 3 * 3600, took_s: 2520, tasks: 12, fails: None, failed_task: "", running_task: "aggregate_currencies", tags: &["accounting", "clickhouse"], description: "STATEMENT_DAILY_AGG for every currency account" },
    FakeDag { id: "dbt_daily_models", owner: "tomas.r", schedule: "", cron: "1 day", every_s: 86_400, phase_s: 3600, took_s: 7800, tasks: 41, fails: None, failed_task: "", running_task: "dbt_run_marts", tags: &["dbt"], description: "Every dbt model, staging to marts" },
    FakeDag { id: "posthog_warehouse_refresh", owner: "data-platform", schedule: "At 20 minutes past the hour", cron: "20 * * * *", every_s: 3600, phase_s: 1200, took_s: 190, tasks: 4, fails: None, failed_task: "", running_task: "refresh_balances", tags: &["posthog", "clickhouse"], description: "The ClickHouse tables PostHog's warehouse reads" },
    FakeDag { id: "replication_app_consistency_check", owner: "j.petrova", schedule: "At 06:00", cron: "0 6 * * *", every_s: 86_400, phase_s: 6 * 3600, took_s: 540, tasks: 3, fails: Some(0), failed_task: "check_consistency", running_task: "check_consistency", tags: &["replication"], description: "Source and replica agree, table by table" },
    FakeDag { id: "single_table_reconcile", owner: "j.petrova", schedule: "At 10 minutes past the hour, every 4 hours", cron: "10 */4 * * *", every_s: 4 * 3600, phase_s: 600, took_s: 1320, tasks: 6, fails: None, failed_task: "", running_task: "reconcile", tags: &["accounting"], description: "Reconciles the largest tables one at a time" },
];

/// Made-up DAGs that are paused: listed, never run.
const FAKE_PAUSED: &[&str] = &["legacy_mysql_sync", "grafana_dashboard_export", "adhoc_backfill_2025"];

fn fake_time(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%S+00:00").to_string())
        .unwrap_or_default()
}

/// FAKE=1's Airflow at `now`: the day of ten DAGs on their schedules — three failures, one
/// stretch of dbt two hours long — plus a manual reload running, a backfill waiting, and a test
/// run stuck since January, the shape of what the real one shows.
pub fn airflow(now: i64) -> crate::airflow::Activity {
    use crate::airflow::{Activity, Beat, Dag, Health, Progress, Run, RunState, Task, WINDOW_S};
    let mut runs: Vec<Run> = Vec::new();
    let mut tasks: Vec<Task> = Vec::new();
    let mut progress = std::collections::HashMap::new();
    let mut dags: Vec<Dag> = Vec::new();

    for d in FAKE_DAGS {
        // The runs that began in the day, oldest first; each a little late, a little long or short.
        let first = ((now - WINDOW_S - d.phase_s).div_euclid(d.every_s) + 1) * d.every_s + d.phase_s;
        let starts: Vec<i64> = (0..).map(|k| first + k * d.every_s).take_while(|&t| t <= now).collect();
        let ends: Vec<i64> = starts.iter().map(|&slot| slot + 3 + d.took_s + d.took_s / 10 * ((slot / d.every_s) % 3 - 1)).collect();
        let finished: Vec<usize> = (0..starts.len()).filter(|&i| ends[i] <= now).collect();
        for (i, &slot) in starts.iter().enumerate() {
            let start = slot + 3;
            let end = ends[i];
            let logical = slot - d.every_s;
            let id = format!("scheduled__{}", fake_time(logical));
            let failed = d.fails.is_some_and(|back| finished.len() > back && finished[finished.len() - 1 - back] == i);
            let (state, end) = if end > now {
                (RunState::Running, None)
            } else if failed {
                (RunState::Failed, Some(end))
            } else {
                (RunState::Success, Some(end))
            };
            let key = (d.id.to_string(), id.clone());
            match state {
                RunState::Running => {
                    let done = (((now - start) as f64 / d.took_s as f64) * d.tasks as f64) as u32;
                    progress.insert(key, Progress { total: d.tasks, done: done.min(d.tasks - 1), running: vec![d.running_task.to_string()], ..Progress::default() });
                    tasks.push(Task {
                        dag: d.id.to_string(),
                        run: id.clone(),
                        id: d.running_task.to_string(),
                        state: "running".into(),
                        start: Some(start + 5),
                        try_number: 1,
                        max_tries: 2,
                        operator: Some("PythonOperator".into()),
                        host: Some("airflow-worker-1".into()),
                    });
                }
                RunState::Failed => {
                    progress.insert(key, Progress { total: d.tasks, failed: vec![d.failed_task.to_string()], ..Progress::default() });
                }
                _ => {}
            }
            runs.push(Run { dag: d.id.into(), id, kind: "scheduled".into(), state, logical: Some(logical), queued: Some(slot), start: Some(start), end, note: None });
        }
        let next = ((now - d.phase_s).div_euclid(d.every_s) + 1) * d.every_s + d.phase_s;
        dags.push(Dag {
            id: d.id.into(),
            owners: vec![d.owner.into()],
            paused: false,
            schedule: (!d.schedule.is_empty()).then(|| d.schedule.to_string()),
            cron: Some(d.cron.into()),
            next: Some(next),
            tags: d.tags.iter().map(|t| t.to_string()).collect(),
            description: Some(d.description.into()),
        });
    }

    // Manual runs, as people start them.
    let manual = |dag: &str, started: i64, state: RunState| Run {
        dag: dag.into(),
        id: format!("manual__{}", fake_time(started)),
        kind: "manual".into(),
        state,
        logical: Some(started),
        queued: Some(started),
        start: (state != RunState::Queued).then_some(started + 1),
        end: None,
        note: None,
    };
    let reload = manual("statement_daily_agg_reload", now - 2 * 3600 - 13 * 60, RunState::Running);
    progress.insert((reload.dag.clone(), reload.id.clone()), Progress { total: 7, done: 4, running: vec!["reload_partitions".into()], ..Progress::default() });
    tasks.push(Task {
        dag: reload.dag.clone(),
        run: reload.id.clone(),
        id: "reload_partitions".into(),
        state: "running".into(),
        start: Some(now - 41 * 60),
        try_number: 1,
        max_tries: 1,
        operator: Some("PythonOperator".into()),
        host: Some("airflow-worker-3".into()),
    });
    let stuck = manual("test_clickhouse_connection", now - 262 * 86_400 - 5 * 3600, RunState::Running);
    progress.insert((stuck.dag.clone(), stuck.id.clone()), Progress { total: 1, retrying: vec!["ping_clickhouse".into()], ..Progress::default() });
    tasks.push(Task {
        dag: stuck.dag.clone(),
        run: stuck.id.clone(),
        id: "ping_clickhouse".into(),
        state: "up_for_retry".into(),
        start: None,
        try_number: 1,
        max_tries: 1,
        operator: Some("PythonOperator".into()),
        host: None,
    });
    let waiting = manual("gateway_transfers_backfill", now - 4 * 60 - 12, RunState::Queued);
    for (id, owner, schedule, description) in [
        ("statement_daily_agg_reload", "data-platform", "Never, external triggers only", "Reloads STATEMENT_DAILY_AGG for the dates given"),
        ("test_clickhouse_connection", "airflow", "Never, external triggers only", "Pings every ClickHouse connection"),
        ("gateway_transfers_backfill", "tomas.r", "Never, external triggers only", "Backfills gateway.transfers for a range of days"),
    ] {
        dags.push(Dag { id: id.into(), owners: vec![owner.into()], schedule: Some(schedule.into()), description: Some(description.into()), ..Dag::default() });
    }
    runs.extend([reload, stuck, waiting]);
    for id in FAKE_PAUSED {
        dags.push(Dag { id: id.to_string(), owners: vec!["airflow".into()], paused: true, schedule: Some("At 04:00".into()), cron: Some("0 4 * * *".into()), ..Dag::default() });
    }

    Activity {
        reachable: true,
        error: None,
        base_url: Some("https://airflow.example.net".into()),
        version: Some("2.10.2".into()),
        health: Health {
            metadatabase: Some("healthy".into()),
            scheduler: Beat { status: Some("healthy".into()), at: Some(now - 2) },
            triggerer: Beat { status: Some("healthy".into()), at: Some(now - 4) },
            dag_processor: Beat::default(),
        },
        dags,
        import_errors: Some(0),
        runs,
        tasks,
        progress,
        day_read: true,
        taken_at: std::time::UNIX_EPOCH + Duration::from_secs(now.max(0) as u64),
    }
}

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

/// FAKE=1's board: (key, summary, status, priority, rank, kind, in status for, logged, due in
/// days).
#[allow(clippy::type_complexity)]
const FAKE_TICKETS: &[(&str, &str, &str, &str, u32, &str, i64, u64, Option<i64>)] = &[
    ("DATA-2850", "DE - STATEMENT_DAILY_AGG: upstream moves drifted 131 historical EUR dates", "In progress", "Unspecified", 6, "Task", 12 * HOUR, 0, Some(1)),
    ("DATA-2849", "DE - airflow-dags: extract duplicated Google auth and parsing helpers into utils (MR !809)", "In progress", "Unspecified", 6, "Task", 15 * HOUR, 1800, None),
    ("DATA-2804", "DE - Reload mv_statement_daily_agg for 198 dates missing rows of 18 currency accounts", "In progress", "Medium", 3, "Task", 8 * HOUR, 5400, None),
    ("DATA-2207", "DE - Product metrics integration — PostHog + ClickHouse → portal (PoC)", "In progress", "High", 2, "Story", 11 * DAY, 0, None),
    ("DATA-2647", "DE - Client account balances as a daily warehouse table (private/business, EUR buckets)", "In Review", "ASAP", 1, "Task", 4 * DAY, 25_920, Some(-2)),
    ("DATA-2773", "DE - KYC onboarding tables for funnel cohorts and questionnaire decisions", "In Review", "High", 2, "Task", 2 * DAY + 5 * HOUR, 14_400, None),
    ("DATA-2788", "DE - cbk_accounts_report: close the duplicate-upload race and fix verdict handling (MR !805)", "In Review", "High", 2, "Task", 2 * DAY + 4 * HOUR, 10_800, None),
    ("DATA-1817", "DE - Replicate gateway.bank_transfer_data (transfer origin rows) to DS 31", "In Review", "High", 2, "Task", 2 * DAY + 3 * HOUR, 7200, None),
    ("DATA-1289", "DE - Daily per-covenantee history of statement vs accounting balance", "Feedback", "High", 2, "Task", 12 * DAY, 14_400, None),
    ("DATA-2638", "DE - Enrich accounting_monitoring.statement_mismatches with operation details", "Done", "URGENT", 0, "Task", 2 * HOUR, 7200, None),
    ("DATA-2737", "DE - Google Chat failure alerts for the 10 highest-impact enabled DAGs", "Done", "High", 2, "Task", 2 * DAY, 7200, None),
    ("DATA-2650", "DE - Add all gateway.bank_account columns to the ClickHouse reporting table", "Done", "URGENT", 0, "Code review", 3 * DAY + 2 * HOUR, 1800, None),
    ("DATA-2653", "DE - Replicate CreditOnline tables to ClickHouse BI", "Done", "High", 2, "Code review", 3 * DAY + 3 * HOUR, 1800, None),
    ("DATA-2594", "DE - Schedule the refresh of the ClickHouse tables the warehouse reads", "Done", "High", 2, "Task", 3 * DAY + 5 * HOUR, 21_600, None),
    ("DATA-2640", "DE - CBK DAG: immediate alerts with row errors, daily summary", "Done", "ASAP", 1, "Code review", 4 * DAY, 10_800, None),
    ("DATA-2504", "DE - Replicate kyc_control_panel case, case_event and staff tables", "Done", "ASAP", 1, "Task", 4 * DAY + 2 * HOUR, 21_600, None),
    ("DATA-2490", "DE - Accounting operations MV for the unmatched-instant dashboards", "Done", "High", 2, "Task", 5 * DAY + 6 * HOUR, 10_800, None),
];

/// FAKE=1's Jira at `now`: seventeen tickets of a data engineer's week across the board's
/// columns — one overdue in review, one due tomorrow, a week of finished ones.
pub fn jira(now: i64) -> crate::jira::Board {
    use crate::jira::{Board, Ticket, DEFAULT_DONE_DAYS, DEFAULT_STATUSES};
    const H: i64 = 3600;
    const D: i64 = 86_400;
    let day = |offset_days: i64| {
        let date = chrono::DateTime::from_timestamp(now + offset_days * D, 0).map(|t| t.date_naive());
        date.map(|d| d.format("%Y-%m-%d").to_string())
    };
    let rows = FAKE_TICKETS;
    let tickets = rows
        .iter()
        .map(|&(key, summary, status, priority, rank, kind, since, logged, due)| {
            let moved = now - since;
            let done = status == "Done" || status == "Feedback";
            Ticket {
                key: key.into(),
                summary: summary.into(),
                status: status.into(),
                category: if done { "done" } else { "indeterminate" }.into(),
                kind: Some(kind.into()),
                priority: Some(priority.into()),
                priority_rank: Some(rank),
                created: Some(moved - 3 * D),
                updated: Some(moved + since.min(3 * H) / 2),
                resolved: done.then_some(moved),
                due: due.and_then(day),
                status_since: Some(moved),
                parent: (kind == "Code review").then(|| "DATA-2611".to_string()),
                labels: if key == "DATA-2207" { vec!["posthog".into(), "clickhouse".into(), "poc".into()] } else { Vec::new() },
                reporter: Some(if key.ends_with('7') { "Jurgita Petrova" } else { "Tomas Rimkus" }.into()),
                logged_s: (logged > 0).then_some(logged),
            }
        })
        .collect();
    Board {
        reachable: true,
        error: None,
        base_url: Some("https://jira.example.net".into()),
        version: Some("9.12.1".into()),
        user: Some("Sam Example".into()),
        statuses: DEFAULT_STATUSES.iter().map(|s| s.to_string()).collect(),
        done_days: DEFAULT_DONE_DAYS,
        tickets,
        worklogs: fake_worklogs(now),
        worklogs_read: true,
        taken_at: std::time::UNIX_EPOCH + Duration::from_secs(now.max(0) as u64),
    }
}

/// The summary FAKE=1's board gives a ticket.
fn fake_summary(key: &str) -> String {
    FAKE_TICKETS.iter().find(|t| t.0 == key).map(|t| t.1.to_string()).unwrap_or_default()
}

/// FAKE=1's time logged this month: a working day of six to nine hours over two or three
/// tickets, nothing at weekends, today so far a couple of hours.
fn fake_worklogs(now: i64) -> Vec<crate::jira::Worklog> {
    let month = crate::jira::Month::of(now, 7 * 3600);
    let keys = ["DATA-2647", "DATA-2773", "DATA-2804", "DATA-2849", "DATA-2207", "DATA-2638"];
    let mut out = Vec::new();
    for day in 1..=month.today {
        if month.is_weekend(day) {
            continue;
        }
        let at = month.first + i64::from(day) - 1;
        let hours: &[u64] = if day == month.today { &[2] } else { [&[4u64, 3][..], &[5, 2, 1], &[6, 2], &[3, 3, 2], &[7]][day as usize % 5] };
        for (i, h) in hours.iter().enumerate() {
            let key = keys[(day as usize + i) % keys.len()];
            out.push(crate::jira::Worklog { key: key.to_string(), summary: fake_summary(key), day: at, seconds: h * 3600 + if i == 0 { 1800 * (day as u64 % 2) } else { 0 } });
        }
    }
    out
}

/// FAKE=1's answer to a page of view 5 or 6, from the same made-up Airflow and Jira the views show.
pub fn detail(ask: &crate::detail::Ask, now: i64) -> Result<crate::detail::Body, String> {
    use crate::detail::{Ask, Body};
    match ask {
        Ask::Issue(key) => {
            let board = jira(now);
            let ticket = board.tickets.iter().find(|t| &t.key == key).ok_or_else(|| format!("{key} is not on the made-up board"))?;
            Ok(Body::Issue(Box::new(crate::jira::IssueDetail {
                key: ticket.key.clone(),
                summary: ticket.summary.clone(),
                status: ticket.status.clone(),
                kind: ticket.kind.clone(),
                priority: ticket.priority.clone(),
                assignee: Some("Sam Example".into()),
                reporter: ticket.reporter.clone(),
                created: ticket.created,
                updated: ticket.updated,
                due: ticket.due.clone(),
                labels: ticket.labels.clone(),
                parent: ticket.parent.clone(),
                logged_s: ticket.logged_s,
                description: format!(
                    "Request from the reporting team: {}.\n\nh3. Asked for\n\n* one row per day and currency, *aggregated only*\n* the EUR equivalent beside every total\n** at the ECB rate of that day\n\nh3. Source\n\nThe balances are in {{{{final.internal_account}}}}; its history is rebuilt from the ledger:\n\n{{code:sql}}\nSELECT toDate(created_at) AS day, currency, sum(amount) AS total\nFROM ledger.movements\nGROUP BY day, currency\n{{code}}\n\nSee [the dashboard|https://redash.example.net/dashboards/42] for the numbers today.",
                    crate::jira::split_tag(&ticket.summary).1.to_lowercase()
                ),
                subtasks: vec![crate::jira::Linked { how: "sub-task".into(), key: "DATA-2660".into(), summary: "Code review".into(), status: "Done".into() }],
                links: vec![crate::jira::Linked { how: "blocks".into(), key: "DATA-2701".into(), summary: "Balances on the CFO dashboard".into(), status: "Backlog".into() }],
                comments: vec![
                    crate::jira::Comment { author: "Jurgita Petrova".into(), at: Some(now - 2 * 86_400), body: "Could the buckets be configurable? *Finance* asks for >=50 too.".into() },
                    crate::jira::Comment { author: "Sam Example".into(), at: Some(now - 86_400 - 3600), body: "Added >=50; the table is rebuilt nightly at 03:00.".into() },
                ],
            })))
        }
        Ask::Runs(dag) => {
            let activity = airflow(now);
            let mut runs: Vec<crate::airflow::Run> = activity.runs.into_iter().filter(|r| &r.dag == dag).collect();
            runs.sort_by_key(|r| std::cmp::Reverse(r.at()));
            Ok(Body::Runs(runs))
        }
        Ask::Tasks { dag, run } => {
            let activity = airflow(now);
            let found = activity.runs.iter().find(|r| &r.dag == dag && &r.id == run).ok_or_else(|| format!("no run {run} of {dag}"))?;
            Ok(Body::Tasks(fake_tasks(found, now)))
        }
        Ask::Log { dag, run, task, attempt, .. } => {
            let failed = FAKE_DAGS.iter().any(|d| d.id == dag && d.failed_task == task) && airflow(now).runs.iter().any(|r| &r.id == run && r.state == crate::airflow::RunState::Failed);
            let mut text = format!(
                "airflow-worker-1.airflow-worker.svc.cluster.local\n*** Found local files:\n***   * /opt/airflow/logs/dag_id={dag}/run_id={run}/task_id={task}/attempt={attempt}.log\n"
            );
            for i in 0..40 {
                text.push_str(&format!("[2026-10-04T05:30:{:02}.{:03}+0000] {{taskinstance.py:2612}} INFO - step {i} of {task}: {} rows\n", i % 60, i * 7, 1000 + i * 37));
            }
            if failed {
                text.push_str("Traceback (most recent call last):\n  File \"/opt/airflow/dags/repo/dag.py\", line 78, in run\n    check(result)\n  File \"/opt/airflow/dags/repo/dag.py\", line 65, in check\n    raise AirflowException(f'{bad} table(s) need a look')\nairflow.exceptions.AirflowException: 3 table(s) need a look: ledger_movements (too_many), fx_rates (missing), payouts (too_many)\n[2026-10-04T05:42:08.408+0000] {local_task_job_runner.py:266} INFO - Task exited with return code 1\n");
            } else {
                text.push_str("[2026-10-04T05:42:08.408+0000] {local_task_job_runner.py:266} INFO - Task exited with return code 0\n");
            }
            Ok(Body::Log(crate::detail::log_lines(&text)))
        }
    }
}

/// The tasks of a made-up run, their states as its outcome says.
fn fake_tasks(run: &crate::airflow::Run, now: i64) -> Vec<crate::airflow::TaskRun> {
    use crate::airflow::{RunState, TaskRun};
    let template = FAKE_DAGS.iter().find(|d| d.id == run.dag);
    let count = template.map_or(3, |d| d.tasks.min(12)) as usize;
    let special = template.map(|d| (d.running_task, d.failed_task));
    let mut names: Vec<String> = ["extract", "validate", "transform", "load", "refresh_dicts", "check_counts", "publish", "notify", "archive", "cleanup", "stats", "done"]
        .iter()
        .take(count)
        .map(|s| s.to_string())
        .collect();
    let at = count / 2;
    match (run.state, special) {
        (RunState::Failed, Some((_, failed))) if !failed.is_empty() => names[at] = failed.to_string(),
        (_, Some((running, _))) if !running.is_empty() => names[at] = running.to_string(),
        _ => {}
    }
    let start = run.start.unwrap_or(now);
    let step = 40;
    names
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            let began = start + i as i64 * step;
            let (state, ran) = match run.state {
                RunState::Success => ("success", true),
                RunState::Failed if i < at => ("success", true),
                RunState::Failed if i == at => ("failed", true),
                RunState::Failed => ("upstream_failed", false),
                RunState::Running if i < at => ("success", true),
                RunState::Running if i == at => ("running", true),
                RunState::Running => ("scheduled", false),
                _ => ("queued", false),
            };
            TaskRun {
                id,
                map_index: -1,
                state: state.into(),
                start: ran.then_some(began),
                end: (ran && state != "running").then_some(began + step - 5),
                try_number: u32::from(ran),
                max_tries: 1,
                operator: Some(if i % 3 == 0 { "SSHOperator" } else { "PythonOperator" }.into()),
                host: ran.then(|| "airflow-worker-1".to_string()),
            }
        })
        .collect()
}

/// A made-up laptop for view 0, `step` polls in: the CPU time of each process goes on at its own
/// pace, so from the second read its cores are the delta form, as on a real one.
pub fn local(step: u64, at: f64) -> crate::local::Sample {
    use crate::local::{Memory, Pressure, Process, Sample};
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    let chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    let renderer = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)";
    // pid, name, cores it runs at, MiB resident, CPU time it had at the first read
    let table: [(u32, &str, f64, u64, f64); 14] = [
        (28932, "/Users/sam/.opencode/bin/opencode", 0.47, 825, 144.0),
        (28839, "/Users/sam/.opencode/bin/opencode", 0.22, 737, 88.0),
        (164, "/System/Library/PrivateFrameworks/SkyLight.framework/Resources/WindowServer", 0.46, 691, 216_253.0),
        (28807, "/System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent", 0.19, 432, 50.0),
        (40112, "/Applications/Cobserve.app/Contents/MacOS/cobserve", 0.07, 48, 25.0),
        (40110, "/Applications/Cobserve.app/Contents/MacOS/cobserve-desktop", 0.02, 96, 12.0),
        (343, "/usr/sbin/coreaudiod", 0.04, 38, 34_296.0),
        (14897, chrome, 0.024, 857, 23_237.0),
        (15564, renderer, 0.012, 834, 1500.0),
        (62018, renderer, 0.004, 783, 960.0),
        (5792, renderer, 0.002, 679, 410.0),
        (29329, "/Users/sam/.local/bin/claude", 0.07, 387, 9.7),
        (11989, "/Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)", 0.007, 728, 1504.0),
        (37781, "/System/Library/Frameworks/VideoToolbox.framework/Versions/A/XPCServices/VTDecoderXPCService.xpc/Contents/MacOS/VTDecoderXPCService", 0.0, 489, 2.0),
    ];
    // Each poll is 2 s; a process runs at its cores, give or take a quarter, poll by poll.
    let wobble = |pid: u32, k: u64| 1.0 + 0.25 * ((k as f64 * 0.7 + f64::from(pid % 7)).sin());
    let poll = 2.0;
    let used = |pid: u32, cores: f64| (1..=step).map(|k| cores * wobble(pid, k) * poll).sum::<f64>();
    let processes = table
        .iter()
        .map(|&(pid, command, cores, mib, start)| Process {
            pid,
            ppid: if command.contains("Helper") { 14897 } else { 1 },
            user: if pid < 400 { "_system".into() } else { "sam".into() },
            pcpu: cores * 100.0,
            rss: mib * MIB,
            cpu_time_s: start + used(pid, cores),
            command: command.into(),
        })
        .collect();
    // 10 cores at 100 ticks a second: what the processes use, and the kernel's 0.9 cores on top —
    // the rest.
    let busy_s: f64 = table.iter().map(|t| used(t.0, t.2)).sum::<f64>() + 0.9 * poll * step as f64;
    let busy = (busy_s * 100.0) as u64;
    let idle = (10.0 * poll * step as f64 * 100.0) as u64 - busy;
    Sample {
        at,
        host: "sam-macbook.local".into(),
        cores: 10,
        ticks: Some([busy * 3 / 4, busy / 4, idle, 0]),
        memory: Some(Memory { total: 32 * GIB, app: 12_290 * MIB, wired: 2_890 * MIB, compressed: 12_050 * MIB, cached: 3_760 * MIB }),
        swap: Some((2_230 * MIB, 3 * GIB)),
        pressure: Some(Pressure { level: 1, free_pct: 50 }),
        load: Some([3.21, 3.05, 2.98]),
        uptime_s: Some(31 * 86_400 + 2 * 3600),
        processes,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{fold_healthy, node_view, shares_add_up};

    #[test]
    fn a_cancelled_job_leaves_the_queue_and_frees_its_worker() {
        let mut fake = FakeSource::new();
        let before = fake.queue();
        let queries = |status: &QueueStatus| status.queue("queries").map(|q| (q.running, q.workers_busy)).unwrap();
        assert_eq!((queries(&before), before.workers_busy), ((4, 4), 6));
        fake.cancel("run-1").unwrap();
        fake.cancel("wait-2").unwrap();
        let after = fake.queue_now();
        assert!(!after.jobs.iter().any(|j| j.id == "run-1" || j.id == "wait-2"));
        assert_eq!((queries(&after), after.workers_busy), ((3, 3), 5));
        assert_eq!(after.queue("queries").unwrap().waiting, before.queue("queries").unwrap().waiting - 1);
        assert!(fake.cancel("run-1").is_err(), "gone: Redash no longer has it");
    }

    #[test]
    fn the_fleet_looks_like_the_screen_in_the_design() {
        let mut fake = FakeSource::new();
        let snap = fake.snapshot();

        assert_eq!(snap.nodes.len(), 8, "the NEW node shows up after 20 s");

        let ch3 = snap.nodes.iter().find(|n| n.name == "clickhouse3").unwrap();
        let view = node_view(ch3, None);
        assert!((view.mem_pct.unwrap() - 90.9).abs() < 3.0);
        assert!((view.cpu_pct.unwrap() - 94.4).abs() < 5.0);
        assert!(shares_add_up(&view), "user rows + closing row == the node");
        // §1 marks both r_redash rows on clickhouse3 with ✕, and nothing else on that node.
        assert_eq!(view.runaways, 2, "the two long r_redash queries");
        assert!(view.users.iter().all(|u| !u.runaway || u.user == "r_redash"));
        assert_eq!(view.users[0].user, "r_redash");
        assert_eq!(view.users[0].person.as_deref(), Some("grigol.gankava"));
        assert!(view.users[0].runaway);
    }

    #[test]
    fn every_node_satisfies_the_closing_row_invariant() {
        let mut fake = FakeSource::new();
        for _ in 0..5 {
            for node in &fake.snapshot().nodes {
                let view = node_view(node, None);
                assert!(shares_add_up(&view), "{} broke the invariant", node.name);
            }
        }
    }

    #[test]
    fn the_healthy_nodes_are_exactly_the_foldable_ones() {
        let mut fake = FakeSource::new();
        let snap = fake.snapshot();
        let foldable: Vec<String> = snap
            .nodes
            .iter()
            .filter(|n| fold_healthy(&node_view(n, None)))
            .map(|n| n.name.clone())
            .collect();
        assert_eq!(foldable, vec!["ch4", "ch6", "ch8", "ch9"]);

        // clickhouse7 has lag 12 s, so it stays on screen even though it is quiet.
        assert!(!foldable.contains(&"clickhouse7".to_string()));
    }

    #[test]
    fn a_tenth_node_appears_and_is_marked_new_by_the_app() {
        let mut fake = FakeSource::new();
        let mut first_seen = std::collections::HashSet::new();
        let new_flags = crate::model::mark_new_nodes(&fake.snapshot(), &mut first_seen, true);
        assert!(new_flags.is_empty());

        // The flag fires on the poll where the node first shows up, and only then: §2.6 keeps
        // the badge for the session, which is App's job, not this poll's.
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..12 {
            for name in crate::model::mark_new_nodes(&fake.snapshot(), &mut first_seen, false) {
                seen.push(name);
            }
        }
        assert_eq!(seen, vec!["clickhouse5".to_string()]);

        // And it stays known after that.
        assert!(crate::model::mark_new_nodes(&fake.snapshot(), &mut first_seen, false).is_empty());
    }

    #[test]
    fn queries_age_and_turn_over() {
        let mut fake = FakeSource::new();
        let first = fake.snapshot();
        let later = fake.snapshot();

        let elapsed_of = |snap: &FleetSnapshot, id: &str| -> f64 {
            snap.nodes
                .iter()
                .flat_map(|n| n.queries.iter())
                .find(|q| q.query_id == id)
                .map(|q| q.elapsed_s)
                .unwrap_or(-1.0)
        };
        let grew = later
            .nodes
            .iter()
            .flat_map(|n| n.queries.iter())
            .any(|q| q.elapsed_s > elapsed_of(&first, &q.query_id) && elapsed_of(&first, &q.query_id) >= 0.0);
        assert!(grew, "surviving queries must age between polls");
    }

    #[test]
    fn the_queue_is_backed_up_but_consistent() {
        let mut fake = FakeSource::new();
        let queue = fake.queue();
        assert!(queue.reachable);
        let queries = queue.queue("queries").unwrap();
        assert!(queries.saturated());
        // The wait grows with the queue's own clock: 100 s on the first poll.
        assert_eq!(queries.oldest_wait_s, Some(103));
        assert!(queries.oldest_wait_s.unwrap() >= 60, "amber on screen");
        assert!(!queue.waiting("queries").is_empty());
        // §2.8: a waiting job has no ClickHouse counterpart; the app's stitch finds the
        // started ones' from the snapshot.
        assert!(queue.jobs.iter().all(|j| j.ch_node.is_none()));
        // Every count matches the jobs behind it, and no queue runs more than its workers.
        for row in &queue.queues {
            let of = |state: fn(&Job) -> bool| queue.jobs.iter().filter(|j| j.queue == row.name && state(j)).count() as u32;
            assert_eq!(row.running, of(|j| j.state == JobState::Started), "{}", row.name);
            assert_eq!(row.stale, of(|j| matches!(j.state, JobState::Stale(_))), "{}", row.name);
            assert!(row.waiting >= of(|j| j.state == JobState::Queued), "{}", row.name);
            assert!(row.running <= row.workers_busy && row.workers_busy <= row.workers_total, "{}", row.name);
        }
        assert_eq!(queue.total_running(), queue.started().len() as u32);
        assert!(queue.workers_busy <= queue.workers_total);
    }

    #[test]
    fn generated_snapshots_are_deterministic() {
        let mut a = FakeSource::new();
        let mut b = FakeSource::new();
        for _ in 0..3 {
            let (sa, sb) = (a.snapshot(), b.snapshot());
            assert_eq!(sa.nodes.len(), sb.nodes.len());
            for (na, nb) in sa.nodes.iter().zip(sb.nodes.iter()) {
                assert_eq!(na.name, nb.name);
                assert_eq!(na.mem_used, nb.mem_used);
                assert_eq!(
                    na.queries.iter().map(|q| q.query_id.clone()).collect::<Vec<_>>(),
                    nb.queries.iter().map(|q| q.query_id.clone()).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn a_query_session_on_the_made_up_fleet_gets_believable_answers() {
        let mut fake = FakeSource::new();
        let snapshot = fake.snapshot();
        let per_user = console_answer(Some(&snapshot), "clickhouse3", "SELECT user, count() FROM system.processes GROUP BY user;").unwrap();
        assert_eq!(per_user.columns[0].0, "user");
        assert!(per_user.rows.iter().any(|r| r[0].as_deref() == Some("r_redash")), "{:?}", per_user.rows);
        let parts = console_answer(Some(&snapshot), "clickhouse3", "SELECT * FROM system.parts LIMIT 3").unwrap();
        assert_eq!(parts.rows.len(), 3);
        let ledger = console_answer(Some(&snapshot), "clickhouse3", "SELECT * FROM wallet.ledger").unwrap();
        assert_eq!((ledger.columns.len(), ledger.rows.len()), (4, 8));
        assert!(console_answer(Some(&snapshot), "clickhouse3", "DROP TABLE wallet.ledger").unwrap_err().contains("READONLY"));
        assert!(console_answer(Some(&snapshot), "clickhouse3", "SELECT * FROM wallet.nope").unwrap_err().contains("UNKNOWN_TABLE"));
        assert_eq!(console_answer(Some(&snapshot), "clickhouse3", "SELECT 1").unwrap().rows, [vec![Some("1".to_string())]]);
        // Its tables are the ones its queries read.
        let schema = schema();
        assert!(schema.table(Some("gateway"), "transfers").is_some() && schema.table(Some("system"), "processes").is_some());
        let ask = crate::console::Ask { id: 1, assistant: crate::console::Assistant::Claude, node: None, sql: "-- top users by memory".into(), error: None, instruction: String::new(), selected: None };
        let sql = assist(&ask).unwrap();
        assert!(sql.starts_with("-- top users by memory\nSELECT user") && sql.ends_with(';'), "{sql}");
    }
}
