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

