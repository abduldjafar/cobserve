//! `FAKE=1` data: believable snapshots with no network at all (DESIGN.md §8).
//!
//! This is step 1 of the build order — the screen has to look right before anything touches
//! a cluster. The numbers are not random: user bytes are chosen first, the node's own usage
//! is the sum of its users plus a server share, and `cpu_time_us` grows at exactly the cores
//! the row claims. That way the §5.3 invariant holds on screen too, not just in tests.

use crate::model::{FleetSnapshot, Job, JobState, NodeSnapshot, QueryRow, QueueRow, QueueStatus};
use std::time::{Duration, SystemTime};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

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
}

impl QueryTemplate {
    /// The comment Redash prepends to every query it runs (§6.4).
    fn sql_with_comment(&self) -> String {
        match (self.person, self.redash_id) {
            (Some(person), Some(id)) => format!(
                "/* Application: Redash */ /* Username: {person}@paysera.net, Redash query_id: {id}, Redash: 10.1.0 */ {}",
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
const CLUSTER_HISTORY: &str = "SELECT host_name, max(absolute_delay) AS lag, count() AS parts\nFROM clusterAllReplicas('ch_paysera', system.parts)\nGROUP BY host_name";
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
    },
    QueryTemplate {
        initial_age_s: 21.0,
        user: "r_redash",
        person: Some("m.kairys"),
        redash_id: Some(7711),
        sql: FX,
        lifetime_s: 28.0,
        mem_share: 0.115,
        cores: 1.1,
        mem_growth: 0.0001,
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
            (
                template.person.map(|p| format!("{p}@paysera.net")).map(|a| {
                    
                    a.split('@').next().unwrap_or_default().to_string()
                }),
                template.redash_id,
            )
        } else {
            (None, None)
        };

        let mut row = QueryRow::new(&live.id, template.user);
        row.person = person;
        row.redash_query_id = redash_id;
        row.elapsed_s = elapsed;
        row.memory_bytes = mem_bytes;
        row.read_rows = self.rng.range(1.0e8, 2.0e9) as u64;
        row.read_bytes = self.rng.range(4.0e10, 4.2e10) as u64;
        row.sql = sql;
        row.cpu_time_us = cpu_time_us;
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

            // The node's own usage is its users plus whatever the server needs, chosen so the
            // node lands on the target percentage: Σ user + closing row == the node (§5.3).
            // The floor keeps at least 1% for the server when the users alone would fill it.
            let mem_total = (template.mem_gib * GIB) as u64;
            let mem_target = template.user_pct / 100.0 * mem_total as f64;
            let server_mem = (mem_target - user_mem).max(mem_total as f64 * 0.01);
            let mem_used = (user_mem + server_mem) as u64;

            let cpu_target = template.cpu_pct / 100.0 * template.cores;
            let busy_cores =
                (cpu_target.max(user_cores + template.cores * 0.01)).min(template.cores);
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
            });
        }

        FleetSnapshot {
            taken_at: self.now(),
            nodes,
        }
    }

    /// A backed-up queue (§8): 12 waiting, oldest 1m40s, all 6 workers busy.
    pub fn queue(&mut self) -> QueueStatus {
        self.queue_tick += Duration::from_millis(3000);
        type WaitingSpec = (&'static str, &'static str, Option<u64>, &'static str);
        let waiting_specs: &[WaitingSpec] = &[
            ("r_redash", "r.simonyte", Some(8091), "July close pack · by product"),
            ("r_redash", "j.petrova", Some(8113), "AML dashboard · by country"),
            ("r_redash", "m.kairys", Some(7711), "FX exposure · intraday"),
            ("r_redash", "grigol.gankava", Some(8092), "Gateway transfers · hourly"),
            ("r_redash", "d.zaleckas", Some(8120), "Chargebacks · weekly"),
        ];
        type RunningSpec = (
            &'static str,
            &'static str,
            Option<u64>,
            &'static str,
            &'static str,
            &'static str,
        );
        let running_specs: &[RunningSpec] = &[
            ("r_redash", "grigol.gankava", Some(7438), "Gateway transfers", "clickhouse3", "c3e51cb5"),
            ("r_redash", "j.petrova", Some(8585), "AML dashboard", "clickhouse-bi", "8a1c4d02"),
            ("r_redash", "m.kairys", Some(7711), "FX exposure", "clickhouse2", "1f9d77ae"),
        ];

        let mut jobs: Vec<Job> = Vec::new();
        let mut oldest_wait = 0u64;
        let grown = self.queue_tick.as_secs();
        for (i, (user, person, redash_id, name)) in waiting_specs.iter().enumerate() {
            let age = (100 + grown).saturating_sub(i as u64 * 18);
            oldest_wait = oldest_wait.max(age);
            jobs.push(Job {
                id: format!("wait-{}", i + 1),
                state: JobState::Queued,
                queue: "queries".to_string(),
                user: Some((*user).to_string()),
                person: Some((*person).to_string()),
                redash_query_id: *redash_id,
                query_name: Some((*name).to_string()),
                data_source: Some(if i % 2 == 0 { "clickhouse-bi" } else { "clickhouse3" }.to_string()),
                age_s: age,
                ch_node: None,
                ch_query_id: None,
            });
        }
        for (i, (user, person, redash_id, name, node, ch_id)) in running_specs.iter().enumerate() {
            jobs.push(Job {
                id: format!("run-{}", i + 1),
                state: JobState::Started,
                queue: "queries".to_string(),
                user: Some((*user).to_string()),
                person: Some((*person).to_string()),
                redash_query_id: *redash_id,
                query_name: Some((*name).to_string()),
                data_source: Some((*node).to_string()),
                age_s: (275 + grown).saturating_sub(i as u64 * 60),
                ch_node: Some((*node).to_string()),
                ch_query_id: Some((*ch_id).to_string()),
            });
        }
        for (i, (name, person)) in [("Refresh merchant risk", "a.vaitkus"), ("Nightly settlement", "d.zaleckas")]
            .iter()
            .enumerate()
        {
            jobs.push(Job {
                id: format!("sched-{}", i + 1),
                state: JobState::Queued,
                queue: "scheduled_queries".to_string(),
                user: Some("r_redash".to_string()),
                person: Some((*person).to_string()),
                redash_query_id: None,
                query_name: Some((*name).to_string()),
                data_source: Some("clickhouse-bi".to_string()),
                age_s: 22 + grown.min(30) + (i as u64 * 9),
                ch_node: None,
                ch_query_id: None,
            });
        }

        let queues = vec![
            QueueRow {
                name: "queries".to_string(),
                waiting: 12,
                oldest_wait_s: Some(oldest_wait),
                workers_busy: 6,
                workers_total: 6,
                failed_5m: 2,
            },
            QueueRow {
                name: "scheduled_queries".to_string(),
                waiting: 3,
                oldest_wait_s: Some(22),
                workers_busy: 2,
                workers_total: 2,
                failed_5m: 0,
            },
            QueueRow {
                name: "periodic".to_string(),
                waiting: 0,
                oldest_wait_s: None,
                workers_busy: 0,
                workers_total: 1,
                failed_5m: 0,
            },
        ];

        QueueStatus {
            reachable: true,
            error: None,
            queues,
            jobs,
            names_available: true,
            host: Some("redash.paysera.net".to_string()),
            taken_at: self.epoch + self.queue_tick,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{fold_healthy, node_view, shares_add_up};

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
        assert!(queue.started().iter().any(|j| j.ch_node.is_some()));
        // §2.8: a waiting job has no ClickHouse counterpart, a started one does.
        assert!(queue.waiting("queries").iter().all(|j| j.ch_node.is_none()));
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
}

