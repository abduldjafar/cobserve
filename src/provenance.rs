//! Where a number comes from (`docs/redesign/04-architecture.md` *Contract 2*): the formula, the
//! `DESIGN.md` section it is, its inputs with the system table each came from, and the poll — what
//! the new shell's *explain this number* (`e`) shows.
//!
//! `model.rs` keeps returning plain numbers on the hot path and is not touched: each `explain_*`
//! here rebuilds one §5 formula's provenance from the same inputs, on demand, and carries the
//! formula as a function too, so the tests can check that evaluating the inputs reproduces what
//! `model.rs` said — for every metric of the fake fleet.

use crate::model::{self, FleetTotals, NodeSnapshot, NodeView, QueryRow, QueryStat, UserSlice};
use std::time::SystemTime;

/// What an input or a result is counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Cores,
    Percent,
    Seconds,
    Microseconds,
    /// 1 or 0: a yes or a no.
    Flag,
}

impl Unit {
    /// `59.2 GiB`, `3.10 cores`, `92.5%`, `8m31s`.
    pub fn show(self, value: f64) -> String {
        match self {
            Unit::Bytes => crate::fmt::bytes(value.max(0.0) as u64),
            Unit::Cores => format!("{value:.2} cores"),
            Unit::Percent => format!("{value:.1}%"),
            Unit::Seconds => crate::fmt::dur(value),
            Unit::Microseconds => format!("{value:.0} µs"),
            Unit::Flag => (if value != 0.0 { "yes" } else { "no" }).to_string(),
        }
    }
}

/// One input of a formula: its name in the formula, its value and where it was read.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub name: &'static str,
    pub value: f64,
    pub unit: Unit,
    pub source: &'static str,
}

fn input(name: &'static str, value: f64, unit: Unit, source: &'static str) -> Input {
    Input { name, value, unit, source }
}

#[derive(Debug, Clone)]
pub struct Provenance {
    /// `node.mem_pct`: the quantity, as the popover's first line names it.
    pub quantity: &'static str,
    /// `clickhouse3 memory`, `grigol.gankava's CPU on clickhouse-bi`.
    pub metric: String,
    /// `mem_used ÷ mem_total × 100` — written with the inputs' names.
    pub formula: &'static str,
    /// `DESIGN.md §5.1`.
    pub reference: &'static str,
    pub inputs: Vec<Input>,
    /// What `model.rs` returned: `None` where it cannot know (an unknown denominator).
    pub value: Option<f64>,
    pub unit: Unit,
    /// The poll the inputs came from.
    pub at: SystemTime,
    /// `delta form (2nd poll on)`, `clamped ≥ 0`, `limit: the fleet's 9 GiB fallback`.
    pub note: Option<String>,
    /// The formula itself, over the inputs' values in order.
    eval: fn(&[f64]) -> Option<f64>,
}

impl Provenance {
    /// The formula evaluated over its inputs.
    pub fn evaluate(&self) -> Option<f64> {
        let values: Vec<f64> = self.inputs.iter().map(|i| i.value).collect();
        (self.eval)(&values)
    }

    /// The formula with the values in place of the names, and what it comes to:
    /// `59.2 GiB ÷ 64.0 GiB × 100 = 92.5%`.
    pub fn with_values(&self) -> String {
        let mut text = self.formula.to_string();
        let mut inputs: Vec<&Input> = self.inputs.iter().collect();
        // The longest names first, so `cpu_time_before` is not read as `cpu_time` + `_before`.
        inputs.sort_by_key(|i| std::cmp::Reverse(i.name.len()));
        for i in inputs {
            text = text.replace(i.name, &i.unit.show(i.value));
        }
        match self.value {
            Some(v) => format!("{text} = {}", self.unit.show(v)),
            None => format!("{text} = — (unknown)"),
        }
    }
}

const MEM_USED: &str = "system.asynchronous_metrics · MemoryResident";
const MEM_TOTAL: &str = "OSMemoryTotal → CGroupMemoryTotal → max_server_memory_usage (§6.1)";
const CORES: &str = "system.asynchronous_metrics · the count of OSUserTimeCPU* (§6.1)";
const QUERY_MEM: &str = "system.processes · memory_usage, Σ over the user's initial queries";
const QUERY_CPU: &str = "system.processes · ProfileEvents OSCPUVirtualTimeMicroseconds";

fn pct(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| part / whole * 100.0)
}

/// §5.1 / §5.3: the node's memory share.
pub fn explain_node_mem_pct(node: &NodeSnapshot, at: SystemTime) -> Provenance {
    Provenance {        quantity: "node.mem_pct",        metric: format!("{} memory", node.name),
        formula: "mem_used ÷ mem_total × 100",
        reference: "DESIGN.md §5.1",
        inputs: vec![
            input("mem_used", node.mem_used as f64, Unit::Bytes, MEM_USED),
            input("mem_total", node.mem_total.unwrap_or(0) as f64, Unit::Bytes, MEM_TOTAL),
        ],
        value: model::node_mem_pct(node),
        unit: Unit::Percent,
        at,
        note: node.mem_total.is_none().then(|| "the denominator is unknown: no share, never 0".to_string()),
        eval: |v| pct(v[0], v[1]),
    }
}

/// §5.2: the node's CPU — the `*Normalized` families when the node has them, the delta of
/// `system.events` server CPU time between two polls when it has neither.
pub fn explain_node_cpu_pct(node: &NodeSnapshot, prev: Option<&NodeSnapshot>, at: SystemTime) -> Provenance {
    let value = model::node_cpu_pct(node, prev);
    let cores = node.cores.unwrap_or(0.0);
    match node.cpu_busy_cores {
        Some(busy) => Provenance {            quantity: "node.cpu_pct",            metric: format!("{} CPU", node.name),
            formula: "busy_cores ÷ cores × 100, at most 100",
            reference: "DESIGN.md §5.2",
            inputs: vec![
                input("busy_cores", busy, Unit::Cores, "system.asynchronous_metrics · OS{User,System,Nice}TimeNormalized × cores"),
                input("cores", cores, Unit::Cores, CORES),
            ],
            value,
            unit: Unit::Percent,
            at,
            note: None,
            eval: |v| pct(v[0], v[1]).map(|p| p.clamp(0.0, 100.0)),
        },
        None => {
            let p = prev.unwrap_or(node);
            Provenance {                quantity: "node.cpu_pct",                metric: format!("{} CPU", node.name),
                formula: "(cpu_time_now − cpu_time_before) ÷ ((uptime_now − uptime_before) × 10⁶) ÷ cores × 100",
                reference: "DESIGN.md §5.2",
                inputs: vec![
                    input("cpu_time_now", node.server_cpu_time_us.unwrap_or(0) as f64, Unit::Microseconds, "system.events · OSCPUVirtualTimeMicroseconds"),
                    input("cpu_time_before", p.server_cpu_time_us.unwrap_or(0) as f64, Unit::Microseconds, "the same, a poll earlier"),
                    input("uptime_now", node.uptime_s.unwrap_or(0) as f64, Unit::Seconds, "uptime()"),
                    input("uptime_before", p.uptime_s.unwrap_or(0) as f64, Unit::Seconds, "the same, a poll earlier"),
                    input("cores", cores, Unit::Cores, CORES),
                ],
                value,
                unit: Unit::Percent,
                at,
                note: Some("delta of system.events: the node has no *Normalized family (§12)".to_string()),
                eval: |v| {
                    let wall = (v[2] - v[3]) * 1e6;
                    if wall <= 0.0 || v[4] <= 0.0 {
                        return None;
                    }
                    let busy = ((v[0] - v[1]).max(0.0) / wall).min(v[4]).max(0.0);
                    pct(busy, v[4]).map(|p| p.clamp(0.0, 100.0))
                },
            }
        }
    }
}

/// §5.1: a user's (or a person's) memory share of one node.
pub fn explain_user_mem_pct(slice: &UserSlice<'_>, node: &NodeSnapshot, at: SystemTime) -> Provenance {
    Provenance {        quantity: "user.mem_pct",        metric: format!("{}'s memory on {}", slice.label(), node.name),
        formula: "user_memory ÷ mem_total × 100",
        reference: "DESIGN.md §5.1",
        inputs: vec![
            input("user_memory", slice.mem_bytes as f64, Unit::Bytes, QUERY_MEM),
            input("mem_total", node.mem_total.unwrap_or(0) as f64, Unit::Bytes, MEM_TOTAL),
        ],
        value: slice.mem_pct,
        unit: Unit::Percent,
        at,
        note: Some(format!("{} initial queries — a distributed query is counted once", slice.queries.len())),
        eval: |v| pct(v[0], v[1]),
    }
}

/// §5.2: a user's CPU share of one node.
pub fn explain_user_cpu_pct(slice: &UserSlice<'_>, node: &NodeSnapshot, at: SystemTime) -> Provenance {
    Provenance {        quantity: "user.cpu_pct",        metric: format!("{}'s CPU on {}", slice.label(), node.name),
        formula: "user_cores ÷ cores × 100, at most 100",
        reference: "DESIGN.md §5.2",
        inputs: vec![
            input("user_cores", slice.cores, Unit::Cores, "Σ the cores of its queries (§5.2, each a delta between two polls)"),
            input("cores", node.cores.unwrap_or(0.0), Unit::Cores, CORES),
        ],
        value: slice.cpu_pct,
        unit: Unit::Percent,
        at,
        note: None,
        eval: |v| pct(v[0], v[1]).map(|p| p.clamp(0.0, 100.0)),
    }
}

/// §5.2: the cores one query uses — the delta between two polls, else the average since it
/// started; at most the node's cores.
pub fn explain_query_cores(stat: &QueryStat<'_>, prev: Option<&QueryRow>, node: &NodeSnapshot, at: SystemTime) -> Provenance {
    let q = stat.query;
    let delta = prev.filter(|p| q.elapsed_s > p.elapsed_s);
    let (before_cpu, before_elapsed) = delta.map_or((0.0, 0.0), |p| (p.cpu_time_us as f64, p.elapsed_s));
    Provenance {        quantity: "query.cores",        metric: format!("query {} cores", q.query_id),
        formula: "(cpu_time_now − cpu_time_before) ÷ ((elapsed_now − elapsed_before) × 10⁶), at most node_cores",
        reference: "DESIGN.md §5.2",
        inputs: vec![
            input("cpu_time_now", q.cpu_time_us as f64, Unit::Microseconds, QUERY_CPU),
            input("cpu_time_before", before_cpu, Unit::Microseconds, "the same, a poll earlier (0 on a first sighting)"),
            input("elapsed_now", q.elapsed_s, Unit::Seconds, "system.processes · elapsed"),
            input("elapsed_before", before_elapsed, Unit::Seconds, "the same, a poll earlier (0 on a first sighting)"),
            input("node_cores", node.cores.unwrap_or(f64::INFINITY), Unit::Cores, CORES),
        ],
        value: Some(stat.cores),
        unit: Unit::Cores,
        at,
        note: Some(if delta.is_some() { "delta form, between the last two polls" } else { "average since it started: a first sighting" }.to_string()),
        eval: |v| {
            let wall = (v[2] - v[3]) * 1e6;
            let raw = if wall > 0.0 { (v[0] - v[1]).max(0.0) / wall } else { 0.0 };
            Some(raw.min(v[4]).max(0.0))
        },
    }
}

/// §5.3: the closing row — what the users do not account for, at least 0.
pub fn explain_server_mem_pct(view: &NodeView<'_>, at: SystemTime) -> Provenance {
    let users: f64 = view.users.iter().filter_map(|u| u.mem_pct).sum();
    Provenance {        quantity: "server.mem_pct",        metric: format!("{} server · caches · merges, memory", view.name()),
        formula: "node_share − Σ user_shares, at least 0",
        reference: "DESIGN.md §5.3",
        inputs: vec![
            input("node_share", view.mem_pct.unwrap_or(f64::NAN), Unit::Percent, "the node's memory share (§5.1)"),
            input("user_shares", users, Unit::Percent, "the user rows above it (§5.1)"),
        ],
        value: view.server_mem_pct,
        unit: Unit::Percent,
        at,
        note: Some("so the rows add up to the node".to_string()),
        eval: |v| (!v[0].is_nan()).then(|| (v[0] - v[1]).max(0.0)),
    }
}

/// §5.3: the closing row's CPU.
pub fn explain_server_cpu_pct(view: &NodeView<'_>, at: SystemTime) -> Provenance {
    let users: f64 = view.users.iter().filter_map(|u| u.cpu_pct).sum();
    Provenance {        quantity: "server.cpu_pct",        metric: format!("{} server · caches · merges, CPU", view.name()),
        formula: "node_share − Σ user_shares, at least 0",
        reference: "DESIGN.md §5.3",
        inputs: vec![
            input("node_share", view.cpu_pct.unwrap_or(f64::NAN), Unit::Percent, "the node's CPU share (§5.2)"),
            input("user_shares", users, Unit::Percent, "the user rows above it (§5.2)"),
        ],
        value: view.server_cpu_pct,
        unit: Unit::Percent,
        at,
        note: Some("so the rows add up to the node".to_string()),
        eval: |v| (!v[0].is_nan()).then(|| (v[0] - v[1]).max(0.0)),
    }
}

/// §5.4: whether a query is a runaway — 30 s or more, or 80% or more of the limit it is held to.
pub fn explain_runaway(stat: &QueryStat<'_>, node: &NodeSnapshot, at: SystemTime) -> Provenance {
    let q = stat.query;
    let note = match (q.memory_limit.filter(|v| *v > 0), node.max_memory_usage.filter(|v| *v > 0)) {
        (Some(_), _) => "limit: the query's own max_memory_usage",
        (None, Some(_)) => "limit: the node's max_memory_usage",
        (None, None) => "limit: the fleet's 9 GiB fallback",
    };
    Provenance {        quantity: "query.runaway",        metric: format!("query {} runaway", q.query_id),
        formula: "elapsed ≥ 30 s or memory ≥ 0.8 × limit",
        reference: "DESIGN.md §5.4",
        inputs: vec![
            input("elapsed", q.elapsed_s, Unit::Seconds, "system.processes · elapsed"),
            input("memory", q.memory_bytes as f64, Unit::Bytes, "system.processes · memory_usage"),
            input("limit", stat.limit as f64, Unit::Bytes, "Settings['max_memory_usage'] → system.settings → 9 GiB"),
        ],
        value: Some(if stat.runaway { 1.0 } else { 0.0 }),
        unit: Unit::Flag,
        at,
        note: Some(note.to_string()),
        eval: |v| Some(if v[0] >= model::RUNAWAY_ELAPSED_S || v[1] >= model::RUNAWAY_MEMORY_FRACTION * v[2] { 1.0 } else { 0.0 }),
    }
}

/// The fleet's memory share: the reachable nodes with a known denominator, summed (§5.1).
pub fn explain_fleet_mem_pct(totals: &FleetTotals, at: SystemTime) -> Provenance {
    Provenance {        quantity: "fleet.mem_pct",        metric: "fleet memory".to_string(),
        formula: "Σ mem_used ÷ Σ mem_total × 100",
        reference: "DESIGN.md §5.1, summed over the reachable nodes",
        inputs: vec![
            input("Σ mem_used", totals.mem_used as f64, Unit::Bytes, MEM_USED),
            input("Σ mem_total", totals.mem_total as f64, Unit::Bytes, MEM_TOTAL),
        ],
        value: totals.mem_pct(),
        unit: Unit::Percent,
        at,
        note: Some(format!("{} of {} nodes reachable", totals.reachable, totals.nodes)),
        eval: |v| pct(v[0], v[1]),
    }
}

/// The fleet's CPU share (§5.2, summed).
pub fn explain_fleet_cpu_pct(totals: &FleetTotals, at: SystemTime) -> Provenance {
    Provenance {        quantity: "fleet.cpu_pct",        metric: "fleet CPU".to_string(),
        formula: "Σ busy_cores ÷ Σ cores × 100, at most 100",
        reference: "DESIGN.md §5.2, summed over the reachable nodes",
        inputs: vec![
            input("Σ busy_cores", totals.busy_cores, Unit::Cores, "each node's busy cores (§5.2)"),
            input("Σ cores", totals.cores, Unit::Cores, CORES),
        ],
        value: totals.cpu_pct(),
        unit: Unit::Percent,
        at,
        note: None,
        eval: |v| pct(v[0], v[1]).map(|p| p.clamp(0.0, 100.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(p: &Provenance) {
        match (p.evaluate(), p.value) {
            (Some(a), Some(b)) => assert!((a - b).abs() <= 0.1, "{}: the formula gives {a}, the model {b} — {}", p.metric, p.with_values()),
            (None, None) => {}
            (a, b) => panic!("{}: the formula gives {a:?}, the model {b:?}", p.metric),
        }
    }

    #[test]
    fn every_metric_of_the_fake_fleet_is_reproduced_by_its_provenance() {
        let mut fake = crate::fake::FakeSource::new();
        let prev = fake.snapshot();
        let snapshot = fake.snapshot();
        let at = snapshot.taken_at;
        let view = model::fleet_view(&snapshot, Some(&prev));
        let mut checked = 0;
        for node in &view.nodes {
            let before = prev.nodes.iter().find(|n| n.name == node.node.name);
            for p in [
                explain_node_mem_pct(node.node, at),
                explain_node_cpu_pct(node.node, before, at),
                explain_server_mem_pct(node, at),
                explain_server_cpu_pct(node, at),
            ] {
                close(&p);
                checked += 1;
            }
            for slice in &node.users {
                close(&explain_user_mem_pct(slice, node.node, at));
                close(&explain_user_cpu_pct(slice, node.node, at));
                for stat in &slice.queries {
                    let earlier = before.and_then(|b| b.queries.iter().find(|q| q.query_id == stat.query.query_id));
                    close(&explain_query_cores(stat, earlier, node.node, at));
                    close(&explain_runaway(stat, node.node, at));
                    checked += 2;
                }
                checked += 2;
            }
        }
        let totals = model::fleet_totals(&view);
        close(&explain_fleet_mem_pct(&totals, at));
        close(&explain_fleet_cpu_pct(&totals, at));
        assert!(checked > 50, "the fake fleet has enough to check: {checked}");
    }

    #[test]
    fn the_formula_reads_with_its_values() {
        let mut node = NodeSnapshot::unreachable("ch-a", "x");
        node.mem_used = 59 * 1024 * 1024 * 1024;
        node.mem_total = Some(64 * 1024 * 1024 * 1024);
        let p = explain_node_mem_pct(&node, SystemTime::UNIX_EPOCH);
        assert_eq!(p.with_values(), "59.0 GiB ÷ 64.0 GiB × 100 = 92.2%");
        assert_eq!(p.reference, "DESIGN.md §5.1");
        node.mem_total = None;
        let unknown = explain_node_mem_pct(&node, SystemTime::UNIX_EPOCH);
        assert_eq!((unknown.value, unknown.evaluate()), (None, None), "no denominator: no share, never 0");
        assert!(unknown.note.unwrap().contains("never 0"));
    }
}
