//! Types (§4) and the math (§5) as pure functions. Nothing in here does I/O.
//!
//! Every number the UI shows comes from this file. §5 is the contract: a percentage on
//! screen that cannot be traced to a formula here is a bug (DESIGN.md §11).

use std::collections::HashMap;
use std::time::SystemTime;

/// The fleet's known per-query ceiling, used when `max_memory_usage` cannot be read (§5.4).
pub const FALLBACK_MAX_MEMORY_USAGE: u64 = 9 * 1024 * 1024 * 1024;

/// A query is runaway at this elapsed time (§5.4) …
pub const RUNAWAY_ELAPSED_S: f64 = 30.0;
/// … or at this share of the per-query memory limit.
pub const RUNAWAY_MEMORY_FRACTION: f64 = 0.8;

/// §2.5 folding thresholds. A node has to be below every one of them.
pub const FOLD_MEM_PCT: f64 = 35.0;
pub const FOLD_CPU_PCT: f64 = 35.0;
pub const FOLD_LAG_S: u64 = 10;

#[derive(Debug, Clone)]
pub struct FleetSnapshot {
    pub taken_at: SystemTime,
    /// Discovered order; the UI sorts.
    pub nodes: Vec<NodeSnapshot>,
}

#[derive(Debug, Clone)]
pub struct NodeSnapshot {
    /// `host_name` from `system.clusters`.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub shard: u32,
    pub replica: u32,
    pub version: String,
    /// false → the numbers below are stale or unknown (§2.6).
    pub reachable: bool,
    /// Bytes; `None` when the §6.1 fallback chain gave nothing usable.
    pub mem_total: Option<u64>,
    /// `MemoryResident`.
    pub mem_used: u64,
    pub cores: Option<f64>,
    /// Node-wide busy cores (§5.2).
    pub cpu_busy_cores: Option<f64>,
    pub running: u32,
    /// Max absolute_delay over `system.replicas`.
    pub lag_s: u64,
    pub active_parts: u64,
    pub queries: Vec<QueryRow>,

    // Three fields §4 does not list but the UI and the math need:
    /// `uptime()` — §2.4 shows it, and it is the wall side of the node CPU delta.
    pub uptime_s: Option<u64>,
    /// `system.events` server CPU time. On nodes that expose no `OS*`/`CGroup*` CPU
    /// family this is the only CPU signal there is (§5.2, §12).
    pub server_cpu_time_us: Option<u64>,
    /// `max_memory_usage` from `system.settings`, the runaway threshold (§5.4).
    pub max_memory_usage: Option<u64>,
    /// Why the last poll failed, for the `↯ unreachable` note (§2.6).
    pub unreachable_reason: Option<String>,
    /// How long this node took to answer the poll. A node that is slow to answer
    /// `system.processes` is usually a node in trouble, so the drawer and the insights say so.
    pub poll_ms: Option<u32>,
}

/// Why a node has no numbers when it was never asked: a host only discovery knows, with a
/// login per server and no default login to use for it. No server is sent another's password.
pub const NO_LOGIN: &str = "no login for this host — add it to the credential file, or set a default login";

impl NodeSnapshot {
    /// The word for a node without numbers: `unreachable`, or `not polled` when it was never
    /// asked because there is no login for it — a gap in the configuration, not an outage.
    pub fn down_word(&self) -> &'static str {
        if self.unreachable_reason.as_deref() == Some(NO_LOGIN) {
            "not polled"
        } else {
            "unreachable"
        }
    }

    /// A node the poll could not reach: it stays in the list (§2.6) with unknown numbers
    /// rather than zeros, so nothing renders a fake `0%`.
    pub fn unreachable(name: &str, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            host: String::new(),
            port: 0,
            shard: 0,
            replica: 0,
            version: String::new(),
            reachable: false,
            mem_total: None,
            mem_used: 0,
            cores: None,
            cpu_busy_cores: None,
            running: 0,
            lag_s: 0,
            active_parts: 0,
            queries: Vec::new(),
            uptime_s: None,
            server_cpu_time_us: None,
            max_memory_usage: None,
            unreachable_reason: Some(reason.into()),
            poll_ms: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryRow {
    pub query_id: String,
    /// `initial_user`.
    pub user: String,
    /// From `Username:` in the SQL (§6.4).
    pub person: Option<String>,
    pub redash_query_id: Option<u64>,
    pub elapsed_s: f64,
    pub memory_bytes: u64,
    // §4 lists a `cores` field here. It is not here: cores are a difference between two polls
    // (§5.2), so a value stored on the raw row would be wrong the moment it was written. They
    // live on `QueryStat::cores`, derived every poll.
    pub read_rows: u64,
    pub read_bytes: u64,
    /// Raw SQL; collapsed at render time.
    pub sql: String,
    /// Cumulative counter, kept so the next poll can difference it.
    pub cpu_time_us: u64,

    // What `system.processes` says beyond §6.1's columns. All of them exist on every server
    // this fleet runs (checked against 24.10 on the local rig), and all of them default to
    // "unknown" so a row parsed from an older answer still works.
    /// ClickHouse's own estimate of the rows the query will read; the progress bar is
    /// `read_rows / total_rows_approx`. 0 = no estimate (a `system.*` read, an INSERT VALUES).
    pub total_rows_approx: u64,
    pub written_rows: u64,
    pub peak_memory_bytes: u64,
    /// The query's own `max_memory_usage`, from its `Settings` map. This is the limit the
    /// server will actually kill it at — `system.settings` answers for the monitoring
    /// session instead, which carries the monitor's own (smaller) cap.
    pub memory_limit: Option<u64>,
    /// `Select`, `Insert`, … as ClickHouse classifies it.
    pub kind: Option<String>,
}

impl QueryRow {
    pub fn new(query_id: &str, user: &str) -> Self {
        Self {
            query_id: query_id.to_string(),
            user: user.to_string(),
            person: None,
            redash_query_id: None,
            elapsed_s: 0.0,
            memory_bytes: 0,
            read_rows: 0,
            read_bytes: 0,
            sql: String::new(),
            cpu_time_us: 0,
            total_rows_approx: 0,
            written_rows: 0,
            peak_memory_bytes: 0,
            memory_limit: None,
            kind: None,
        }
    }
}

/// One query with everything derived about it. §4's `UserSlice.queries` holds the raw
/// rows; the cores of a query are a per-poll delta, so they cannot live on the raw row.
#[derive(Debug, Clone)]
pub struct QueryStat<'a> {
    pub query: &'a QueryRow,
    pub cores: f64,
    pub runaway: bool,
    /// The memory limit this query is held to (its own, else the node's, else 9 GiB).
    pub limit: u64,
    /// `read_rows / total_rows_approx`, when the server has an estimate.
    pub progress: Option<f64>,
    /// Time left at the average pace so far, when there is progress to extrapolate from.
    pub eta_s: Option<f64>,
}

impl QueryStat<'_> {
    /// Memory as a fraction of the limit the server enforces on this query.
    pub fn limit_fraction(&self) -> f64 {
        if self.limit == 0 {
            return 0.0;
        }
        self.query.memory_bytes as f64 / self.limit as f64
    }
}

/// §2.2: one row per initial user on one node — or per person when §6.4 resolves one, which
/// is what the screen in §1 shows: `r_redash → grigol.gankava` and `r_redash → j.petrova`
/// are two rows. Splitting by person never double counts (§5.1): the rows of one user still
/// add up to that user's memory, so the §5.3 invariant is unaffected.
///
/// Derived per poll, never stored (§4).
#[derive(Debug, Clone)]
pub struct UserSlice<'a> {
    pub user: String,
    pub person: Option<String>,
    /// Share of THIS node's memory (§5.1). `None` when the denominator is unknown.
    pub mem_pct: Option<f64>,
    /// Share of THIS node's cores (§5.2).
    pub cpu_pct: Option<f64>,
    pub mem_bytes: u64,
    pub cores: f64,
    /// Sorted by elapsed desc.
    pub queries: Vec<QueryStat<'a>>,
    pub longest_s: f64,
    pub runaway: bool,
}

impl<'a> UserSlice<'a> {
    /// `r_redash → grigol.gankava`, or just the user when §6.4 resolved nobody.
    pub fn label(&self) -> String {
        crate::attrib::user_label(&self.user, self.person.as_deref())
    }
}

/// A node plus everything §5 derives from it.
#[derive(Debug, Clone)]
pub struct NodeView<'a> {
    pub node: &'a NodeSnapshot,
    pub users: Vec<UserSlice<'a>>,
    pub mem_pct: Option<f64>,
    pub cpu_pct: Option<f64>,
    /// The closing row (§5.3).
    pub server_mem_pct: Option<f64>,
    pub server_cpu_pct: Option<f64>,
    pub busy_cores: Option<f64>,
    /// Node mem% including the users' rows — a node is unhealthy on this alone.
    pub pressure: f64,
    pub runaways: usize,
    pub mem_limit: u64,
}

impl<'a> NodeView<'a> {
    pub fn name(&self) -> &str {
        &self.node.name
    }
}

// ---------------------------------------------------------------------------
// §5.1 memory
// ---------------------------------------------------------------------------

/// `mem_used / mem_total * 100`. `None` — never `0` — when the denominator is unknown
/// (§2.1: a percentage without its denominator is not allowed anywhere in this UI).
pub fn node_mem_pct(node: &NodeSnapshot) -> Option<f64> {
    let total = node.mem_total.filter(|t| *t > 0)?;
    Some(node.mem_used as f64 / total as f64 * 100.0)
}

/// Σ memory of a user's initial queries on this node, as a share of the node (§5.1).
pub fn user_mem_pct(mem_bytes: u64, node: &NodeSnapshot) -> Option<f64> {
    let total = node.mem_total.filter(|t| *t > 0)?;
    Some(mem_bytes as f64 / total as f64 * 100.0)
}

// ---------------------------------------------------------------------------
// §5.2 CPU
// ---------------------------------------------------------------------------

/// Cores in use by one query.
///
/// Delta form between two polls is "right now"; the average since the query started is the
/// fallback for a first sighting. Clamped to `[0, node.cores]`.
pub fn query_cores(cur: &QueryRow, prev: Option<&QueryRow>, node_cores: Option<f64>) -> f64 {
    let raw = match prev {
        // ProfileEvents are cumulative for the life of the query, so the delta is the
        // difference between the two samples keyed by query_id (§12).
        Some(prev) if cur.elapsed_s > prev.elapsed_s => {
            let d_cpu = cur.cpu_time_us.saturating_sub(prev.cpu_time_us) as f64;
            let d_wall_us = (cur.elapsed_s - prev.elapsed_s) * 1e6;
            if d_wall_us > 0.0 {
                d_cpu / d_wall_us
            } else {
                average_cores(cur)
            }
        }
        _ => average_cores(cur),
    };
    clamp_cores(raw, node_cores)
}

fn average_cores(q: &QueryRow) -> f64 {
    if q.elapsed_s <= 0.0 {
        return 0.0;
    }
    q.cpu_time_us as f64 / (q.elapsed_s * 1e6)
}

fn clamp_cores(cores: f64, node_cores: Option<f64>) -> f64 {
    let capped = node_cores.map(|c| cores.min(c)).unwrap_or(cores);
    capped.max(0.0)
}

/// Node-wide busy cores: the `*Normalized` families when the node has them, the delta of
/// `system.events` server CPU time when it has neither (§5.2).
pub fn node_busy_cores(node: &NodeSnapshot, prev: Option<&NodeSnapshot>) -> Option<f64> {
    if let Some(busy) = node.cpu_busy_cores {
        return Some(busy);
    }
    let (cur_us, cur_up) = (node.server_cpu_time_us?, node.uptime_s?);
    let (prev_us, prev_up) = (prev?.server_cpu_time_us?, prev?.uptime_s?);
    let d_wall_us = (cur_up.checked_sub(prev_up)?) as f64 * 1e6;
    if d_wall_us <= 0.0 {
        return None;
    }
    Some(clamp_cores(cur_us.saturating_sub(prev_us) as f64 / d_wall_us, node.cores))
}

/// `busy_cores / cores * 100`.
pub fn node_cpu_pct(node: &NodeSnapshot, prev: Option<&NodeSnapshot>) -> Option<f64> {
    let cores = node.cores.filter(|c| *c > 0.0)?;
    let busy = node_busy_cores(node, prev)?;
    Some((busy / cores * 100.0).clamp(0.0, 100.0))
}

/// Σ cores of a user's queries on this node, as a share of the node (§5.2).
pub fn user_cpu_pct(cores: f64, node: &NodeSnapshot) -> Option<f64> {
    let total = node.cores.filter(|c| *c > 0.0)?;
    Some((cores / total * 100.0).clamp(0.0, 100.0))
}

// ---------------------------------------------------------------------------
// §5.4 runaway
// ---------------------------------------------------------------------------

/// The per-query memory limit on this node, or the fleet's known 9 GiB ceiling when
/// `system.settings` could not be read.
pub fn mem_limit(node: &NodeSnapshot) -> u64 {
    node.max_memory_usage
        .filter(|v| *v > 0)
        .unwrap_or(FALLBACK_MAX_MEMORY_USAGE)
}

/// The limit a query is held to: its own `max_memory_usage` when the server reported one,
/// otherwise the node's. The node's answer comes from the monitoring session's
/// `system.settings`, which is the monitor's cap rather than the user's, so it is only ever
/// the fallback.
pub fn query_mem_limit(query: &QueryRow, node: &NodeSnapshot) -> u64 {
    query
        .memory_limit
        .filter(|v| *v > 0)
        .unwrap_or_else(|| mem_limit(node))
}

/// Runaway = `elapsed_s >= 30` or `memory_bytes >= 0.8 × limit`.
pub fn is_runaway(query: &QueryRow, limit: u64) -> bool {
    query.elapsed_s >= RUNAWAY_ELAPSED_S
        || query.memory_bytes as f64 >= RUNAWAY_MEMORY_FRACTION * limit as f64
}

// ---------------------------------------------------------------------------
// Progress: how far through its input a query is, and how long it has left
// ---------------------------------------------------------------------------

/// `read_rows / total_rows_approx`, clamped to [0, 1]. `None` without an estimate.
pub fn query_progress(query: &QueryRow) -> Option<f64> {
    if query.total_rows_approx == 0 {
        return None;
    }
    Some((query.read_rows as f64 / query.total_rows_approx as f64).clamp(0.0, 1.0))
}

/// Time left at the average pace so far: `elapsed × (1 − p) / p`.
///
/// Only once there is something to extrapolate from (1% read and a second of wall time), and
/// never for a query that is already at 100% — ClickHouse's estimate is a floor, so a query
/// "at 100%" is finishing, not late.
pub fn query_eta_s(query: &QueryRow, progress: Option<f64>) -> Option<f64> {
    let p = progress?;
    if !(0.01..0.999).contains(&p) || query.elapsed_s < 1.0 {
        return None;
    }
    Some(query.elapsed_s * (1.0 - p) / p)
}

// ---------------------------------------------------------------------------
// Aggregation: the per-node view (§5.1, §5.2, §5.3)
// ---------------------------------------------------------------------------

/// Group a node's queries by `initial_user` and derive every §5 number from them.
///
/// Only `is_initial_query = 1` rows ever reach here — the §6.1 SQL already filters — so a
/// distributed query cannot charge its initiator twice (§5.1, §12).
pub fn user_slices<'a>(
    node: &'a NodeSnapshot,
    prev: Option<&'a NodeSnapshot>,
) -> Vec<UserSlice<'a>> {
    let mut by_user: HashMap<(&str, Option<&str>), Vec<QueryStat<'a>>> = HashMap::new();

    for query in &node.queries {
        let prev_query = prev.and_then(|p| p.queries.iter().find(|q| q.query_id == query.query_id));
        let key = (query.user.as_str(), query.person.as_deref());
        let limit = query_mem_limit(query, node);
        let progress = query_progress(query);
        by_user.entry(key).or_default().push(QueryStat {
            query,
            cores: query_cores(query, prev_query, node.cores),
            runaway: is_runaway(query, limit),
            limit,
            progress,
            eta_s: query_eta_s(query, progress),
        });
    }

    let mut slices: Vec<UserSlice<'a>> = by_user
        .into_iter()
        .map(|((user, person), mut queries)| {
            queries.sort_by(|a, b| {
                b.query
                    .elapsed_s
                    .total_cmp(&a.query.elapsed_s)
                    .then_with(|| a.query.query_id.cmp(&b.query.query_id))
            });
            let mem_bytes: u64 = queries.iter().map(|q| q.query.memory_bytes).sum();
            let cores: f64 = queries.iter().map(|q| q.cores).sum();
            UserSlice {
                user: user.to_string(),
                person: person.map(str::to_string),
                mem_pct: user_mem_pct(mem_bytes, node),
                cpu_pct: user_cpu_pct(cores, node),
                mem_bytes,
                cores,
                longest_s: queries.first().map(|q| q.query.elapsed_s).unwrap_or(0.0),
                runaway: queries.iter().any(|q| q.runaway),
                queries,
            }
        })
        .collect();

    // §2.2: user rows are sorted by mem% desc. A node with an unknown denominator sorts by
    // bytes instead, which is the same ordering without pretending to know the share.
    slices.sort_by(|a, b| {
        b.mem_pct
            .partial_cmp(&a.mem_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.mem_bytes.cmp(&a.mem_bytes))
            .then_with(|| a.user.cmp(&b.user))
            .then_with(|| a.person.cmp(&b.person))
    });
    slices
}

/// One node, fully derived. `prev` is the previous poll of the same node, needed only for
/// the CPU deltas.
pub fn node_view<'a>(node: &'a NodeSnapshot, prev: Option<&'a NodeSnapshot>) -> NodeView<'a> {
    let users = user_slices(node, prev);
    let mem_pct = node_mem_pct(node);
    let cpu_pct = node_cpu_pct(node, prev);
    let busy_cores = node_busy_cores(node, prev);

    // §5.3: the closing row is what is left after the users. Clamped at 0, because query
    // memory can momentarily exceed MemoryResident accounting — and that clamp is exactly
    // why the invariant below is asserted as ±0.1 and not exactly 0.
    let server_mem_pct = mem_pct.map(|node_pct| (node_pct - users.iter().filter_map(|u| u.mem_pct).sum::<f64>()).max(0.0));
    let server_cpu_pct = cpu_pct.map(|node_pct| (node_pct - users.iter().filter_map(|u| u.cpu_pct).sum::<f64>()).max(0.0));

    let runaways = users.iter().filter(|u| u.runaway).count();
    let view = NodeView {
        node,
        users,
        mem_pct,
        cpu_pct,
        server_mem_pct,
        server_cpu_pct,
        busy_cores,
        pressure: mem_pct.unwrap_or(0.0).max(cpu_pct.unwrap_or(0.0)),
        runaways,
        mem_limit: mem_limit(node),
    };
    // §5.3 is an invariant, not a hope. In a debug build a change that breaks it stops the
    // app here instead of on someone's screen at 3am.
    debug_assert!(
        shares_add_up(&view),
        "{}: user rows plus the closing row cannot add up to less than the node",
        node.name
    );
    view
}

/// Σ user shares + the closing row = the node's own percentage, to within 0.1 (§5.3).
///
/// The one legal way this is not an equality is the clamp: when query memory momentarily
/// exceeds `MemoryResident` the closing row is pinned at 0 and the sum sits *above* the node's
/// percentage. So the test is `sum >= node`, and `the user rows plus the closing row equal the
/// node` covers the exact case.
pub fn shares_add_up(view: &NodeView<'_>) -> bool {
    for (node_pct, server_pct, sum) in [
        (
            view.mem_pct,
            view.server_mem_pct,
            view.users.iter().filter_map(|u| u.mem_pct).sum::<f64>(),
        ),
        (
            view.cpu_pct,
            view.server_cpu_pct,
            view.users.iter().filter_map(|u| u.cpu_pct).sum::<f64>(),
        ),
    ] {
        if let (Some(node_pct), Some(server_pct)) = (node_pct, server_pct)
            && sum + server_pct < node_pct - 0.1
        {
            return false;
        }
    }
    true
}

/// A node with an unknown denominator renders `—` for the whole column (§5.3): a partial
/// column is a lie.
pub fn column_known(view: &NodeView<'_>) -> bool {
    view.mem_pct.is_some() && view.cpu_pct.is_some()
}

// ---------------------------------------------------------------------------
// §2.5 sorting and folding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// `max(mem_pct, cpu_pct)` — the default.
    Pressure,
    Mem,
    Cpu,
    Name,
}

impl SortKey {
    /// `s` cycles pressure → mem → cpu → name → pressure.
    pub fn next(self) -> Self {
        match self {
            SortKey::Pressure => SortKey::Mem,
            SortKey::Mem => SortKey::Cpu,
            SortKey::Cpu => SortKey::Name,
            SortKey::Name => SortKey::Pressure,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Pressure => "pressure ▼",
            SortKey::Mem => "mem ▼",
            SortKey::Cpu => "cpu ▼",
            SortKey::Name => "name ▲",
        }
    }
}

/// pressure desc, then runaway count desc, then name asc (§2.5).
///
/// The comparator lives here, in the file that owns the numbers, so the tree and the tests
/// cannot end up sorting by different rules.
pub fn compare_nodes(a: &NodeView<'_>, b: &NodeView<'_>, key: SortKey) -> std::cmp::Ordering {
    let primary = match key {
        SortKey::Pressure => b.pressure.total_cmp(&a.pressure),
        SortKey::Mem => b
            .mem_pct
            .unwrap_or(-1.0)
            .total_cmp(&a.mem_pct.unwrap_or(-1.0)),
        SortKey::Cpu => b
            .cpu_pct
            .unwrap_or(-1.0)
            .total_cmp(&a.cpu_pct.unwrap_or(-1.0)),
        SortKey::Name => a.name().cmp(b.name()),
    };
    primary
        .then_with(|| b.runaways.cmp(&a.runaways))
        .then_with(|| a.name().cmp(b.name()))
}

/// Fold a node away when it is below every threshold in §2.5. An unknown denominator does
/// not count as "below": a node nobody can measure stays on screen.
pub fn fold_healthy(view: &NodeView<'_>) -> bool {
    let mem = view.mem_pct.is_some_and(|m| m < FOLD_MEM_PCT);
    let cpu = view.cpu_pct.is_some_and(|c| c < FOLD_CPU_PCT);
    mem && cpu && view.runaways == 0 && view.node.lag_s < FOLD_LAG_S
}

/// §2.6: flag nodes that were not in the first snapshot of this session, and remember the
/// ones that were.
pub fn mark_new_nodes(
    snapshot: &FleetSnapshot,
    first_seen: &mut std::collections::HashSet<String>,
    session_started: bool,
) -> std::collections::HashSet<String> {
    if session_started && first_seen.is_empty() {
        // First snapshot: everything present now is not new.
        for node in &snapshot.nodes {
            first_seen.insert(node.name.clone());
        }
        return Default::default();
    }
    let new: std::collections::HashSet<String> = snapshot
        .nodes
        .iter()
        .map(|n| n.name.clone())
        .filter(|name| !first_seen.contains(name))
        .collect();
    for name in &new {
        first_seen.insert(name.clone());
    }
    new
}

// ---------------------------------------------------------------------------
// §2.7 pivot: the same snapshots, grouped by user across the fleet
// ---------------------------------------------------------------------------

/// One user's share of one node — the level-1 row in pivot mode.
#[derive(Debug, Clone)]
pub struct UserNode<'a> {
    pub node: &'a NodeSnapshot,
    /// The person these queries resolved to, so pivot rows can show them too.
    pub person: Option<String>,
    pub mem_pct: Option<f64>,
    pub cpu_pct: Option<f64>,
    pub mem_bytes: u64,
    pub queries: Vec<QueryStat<'a>>,
    pub runaway: bool,
    pub longest_s: f64,
}

/// One user across the whole fleet — the level-0 row in pivot mode.
#[derive(Debug, Clone)]
pub struct FleetUser<'a> {
    pub user: String,
    pub person: Option<String>,
    pub nodes: Vec<UserNode<'a>>,
    pub mem_bytes: u64,
    pub cores: f64,
    pub queries: usize,
    pub runaway: bool,
    pub longest_s: f64,
}

impl<'a> FleetUser<'a> {
    /// `r_redash → grigol.gankava`, via §6.4's display rule.
    pub fn label(&self) -> String {
        crate::attrib::user_label(&self.user, self.person.as_deref())
    }
}

/// The whole fleet, derived once per snapshot: nodes for view 1, users for the pivot.
#[derive(Debug, Clone)]
pub struct FleetView<'a> {
    pub nodes: Vec<NodeView<'a>>,
    pub users: Vec<FleetUser<'a>>,
}

/// The whole fleet in one line: what the band at the top of view 1 shows.
///
/// Sums only over the nodes whose denominator is known, numerator and denominator alike — a
/// node that cannot say how much memory it has is left out of both, never counted as 0 of
/// something (§2.1).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FleetTotals {
    pub nodes: usize,
    pub reachable: usize,
    pub mem_used: u64,
    pub mem_total: u64,
    pub busy_cores: f64,
    pub cores: f64,
    pub queries: usize,
    /// Runaway *queries* (not users), across the fleet.
    pub runaways: usize,
    /// Nodes at amber or worse on memory or CPU (§7).
    pub hot: usize,
}

impl FleetTotals {
    pub fn mem_pct(&self) -> Option<f64> {
        (self.mem_total > 0).then(|| self.mem_used as f64 / self.mem_total as f64 * 100.0)
    }

    pub fn cpu_pct(&self) -> Option<f64> {
        (self.cores > 0.0).then(|| (self.busy_cores / self.cores * 100.0).clamp(0.0, 100.0))
    }
}

pub fn fleet_totals(view: &FleetView<'_>) -> FleetTotals {
    let mut totals = FleetTotals {
        nodes: view.nodes.len(),
        ..FleetTotals::default()
    };
    for node in &view.nodes {
        if !node.node.reachable {
            continue;
        }
        totals.reachable += 1;
        if let Some(total) = node.node.mem_total.filter(|t| *t > 0) {
            totals.mem_total += total;
            totals.mem_used += node.node.mem_used;
        }
        if let (Some(cores), Some(busy)) = (node.node.cores.filter(|c| *c > 0.0), node.busy_cores) {
            totals.cores += cores;
            totals.busy_cores += busy;
        }
        totals.queries += node.users.iter().map(|u| u.queries.len()).sum::<usize>();
        totals.runaways += node
            .users
            .iter()
            .flat_map(|u| u.queries.iter())
            .filter(|q| q.runaway)
            .count();
        let worst = crate::severity::node(node.mem_pct).max(crate::severity::node(node.cpu_pct));
        if worst.is_problem() {
            totals.hot += 1;
        }
    }
    totals
}

pub fn fleet_view<'a>(
    snapshot: &'a FleetSnapshot,
    prev: Option<&'a FleetSnapshot>,
) -> FleetView<'a> {
    let nodes: Vec<NodeView<'a>> = snapshot
        .nodes
        .iter()
        .map(|node| {
            let prev_node = prev.and_then(|p| p.nodes.iter().find(|n| n.name == node.name));
            node_view(node, prev_node)
        })
        .collect();
    let users = pivot(&nodes);
    FleetView { nodes, users }
}

/// Group the same per-node slices the other way round: by person across the fleet (§2.7).
/// Sorted by Σ memory bytes desc, which is the question the pivot exists to answer.
///
/// Keyed by (user, person) like the node rows, because "who is burning the fleet" is a
/// question about people: `r_redash` alone would hide that three of them are the answer.
pub fn pivot<'a>(nodes: &[NodeView<'a>]) -> Vec<FleetUser<'a>> {
    let mut by_user: HashMap<(&str, Option<&str>), Vec<UserNode<'a>>> = HashMap::new();

    for view in nodes {
        for slice in &view.users {
            by_user
                .entry((slice.user.as_str(), slice.person.as_deref()))
                .or_default()
                .push(UserNode {
                    node: view.node,
                    person: slice.person.clone(),
                    mem_pct: slice.mem_pct,
                    cpu_pct: slice.cpu_pct,
                    mem_bytes: slice.mem_bytes,
                    runaway: slice.runaway,
                    longest_s: slice.longest_s,
                    queries: slice.queries.clone(),
                });
        }
    }

    let mut users: Vec<FleetUser<'a>> = by_user
        .into_iter()
        .map(|((user, person), user_nodes)| {
            let mem_bytes: u64 = user_nodes.iter().map(|n| n.mem_bytes).sum();
            let cores: f64 = user_nodes
                .iter()
                .flat_map(|n| n.queries.iter())
                .map(|q| q.cores)
                .sum();
            let queries: usize = user_nodes.iter().map(|n| n.queries.len()).sum();
            FleetUser {
                user: user.to_string(),
                person: person.map(str::to_string),
                nodes: user_nodes,
                mem_bytes,
                cores,
                queries,
                runaway: nodes
                    .iter()
                    .flat_map(|v| v.users.iter())
                    .any(|s| s.user == user && s.person.as_deref() == person && s.runaway),
                longest_s: nodes
                    .iter()
                    .flat_map(|v| v.users.iter())
                    .filter(|s| s.user == user && s.person.as_deref() == person)
                    .map(|s| s.longest_s)
                    .fold(0.0, f64::max),
            }
        })
        .collect();

    users.sort_by(|a, b| {
        b.mem_bytes
            .cmp(&a.mem_bytes)
            .then_with(|| b.runaway.cmp(&a.runaway))
            .then_with(|| a.user.cmp(&b.user))
            .then_with(|| a.person.cmp(&b.person))
    });
    users
}

/// §6.4 attribution, preferring what the server already extracted and falling back to the
/// Rust regexes (`extract` can come back empty when the comment is unusual).
pub fn attribution_from_sql(
    user: &str,
    sql: &str,
    server_person: Option<&str>,
    server_redash_query_id: Option<u64>,
) -> (Option<String>, Option<u64>) {
    if user != crate::attrib::REDASH_USER {
        return (None, None);
    }
    let person = server_person
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| crate::attrib::person_address(sql))
        .map(|address| crate::attrib::display_person(&address));
    let redash_id = server_redash_query_id.or_else(|| crate::attrib::redash_query_id(sql));
    (person, redash_id)
}

// ---------------------------------------------------------------------------
// §4 / §6.3 the Redash queue
// ---------------------------------------------------------------------------

/// The queue's error before the first Redash poll has come back.
pub const QUEUE_NOT_POLLED: &str = "not polled yet";
/// The queue's error when Redash is not configured at all (§9: it is optional).
pub const QUEUE_NOT_CONFIGURED: &str = "not configured";

#[derive(Debug, Clone)]
pub struct QueueStatus {
    /// false → the strip prints "unreachable"; the app keeps running (§6.3).
    pub reachable: bool,
    /// Why it is unreachable, for the strip: `HTTP 401`, `connection refused`.
    pub error: Option<String>,
    pub queues: Vec<QueueRow>,
    /// Waiting and started jobs, with people (§6.3).
    pub jobs: Vec<Job>,
    /// false → the WAITING list shows counts only, never invented names.
    pub names_available: bool,
    /// The Redash instance name for the header, when we know it.
    pub host: Option<String>,
    pub taken_at: SystemTime,
}

impl QueueStatus {
    pub fn unreachable(error: impl Into<String>) -> Self {
        Self {
            reachable: false,
            error: Some(error.into()),
            queues: Vec::new(),
            jobs: Vec::new(),
            names_available: false,
            host: None,
            taken_at: SystemTime::now(),
        }
    }

    /// Not an outage: Redash is not configured, or has not answered its first poll yet.
    pub fn is_placeholder(&self) -> bool {
        !self.reachable
            && matches!(self.error.as_deref(), Some(QUEUE_NOT_POLLED) | Some(QUEUE_NOT_CONFIGURED))
    }

    /// The queue Redash uses for ordinary ad-hoc and dashboard queries.
    pub fn queue(&self, name: &str) -> Option<&QueueRow> {
        self.queues.iter().find(|q| q.name == name)
    }

    /// The Redash instance, for the view 2 header (§2.8).
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// When this status was taken, so the drawer can say how old it is.
    pub fn age(&self, now: SystemTime) -> Option<std::time::Duration> {
        now.duration_since(self.taken_at).ok()
    }

    /// Jobs on a worker: the RUNNING half of view 2 (§2.8).
    /// Jobs on a worker: the RUNNING half of view 2 (§2.8).
    pub fn started(&self) -> Vec<&Job> {
        self.jobs
            .iter()
            .filter(|j| j.state == JobState::Started)
            .collect()
    }

    pub fn waiting(&self, queue: &str) -> Vec<&Job> {
        self.jobs
            .iter()
            .filter(|j| j.queue == queue && j.state == JobState::Queued)
            .collect()
    }

}

#[derive(Debug, Clone)]
pub struct QueueRow {
    pub name: String,
    pub waiting: u32,
    pub oldest_wait_s: Option<u64>,
    pub workers_busy: u32,
    pub workers_total: u32,
    pub failed_5m: u32,
}

impl QueueRow {
    /// All workers on this queue are busy — a reason to look at RUNNING (§2.8).
    pub fn saturated(&self) -> bool {
        self.workers_total > 0 && self.workers_busy >= self.workers_total
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Has not reached ClickHouse yet: nothing to kill, the wait is the queue (§2.8).
    Queued,
    /// On a worker, so it IS a `system.processes` row somewhere.
    Started,
}

#[derive(Debug, Clone)]
pub struct Job {
    /// The RQ job id, or a synthetic one in fake mode. Shown in the drawer: it is what you
    /// look up in Redash when the row on screen is not enough.
    pub id: String,
    pub state: JobState,
    pub queue: String,
    /// The account the job runs as — `r_redash` for everything coming from Redash.
    pub user: Option<String>,
    /// Resolved from the Redash user's email, then displayed per §6.4.
    pub person: Option<String>,
    pub redash_query_id: Option<u64>,
    pub query_name: Option<String>,
    pub data_source: Option<String>,
    /// Waiting time while queued, running time once started.
    pub age_s: u64,
    /// The stitch: the ClickHouse node and `query_id` a started job became.
    pub ch_node: Option<String>,
    pub ch_query_id: Option<String>,
}

impl Job {
    /// `r_redash → grigol.gankava`, or just the user when the person is unknown.
    pub fn label(&self) -> String {
        match &self.user {
            Some(user) => crate::attrib::user_label(user, self.person.as_deref()),
            None => self.person.clone().unwrap_or_else(|| "—".to_string()),
        }
    }

    /// A started job with no ClickHouse counterpart cannot be jumped to.
    pub fn clickhouse_target(&self) -> Option<(&str, &str)> {
        match (self.ch_node.as_deref(), self.ch_query_id.as_deref()) {
            (Some(node), Some(id)) => Some((node, id)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// The §1 screen's clickhouse3: 64 GB, 16 cores, 58.2 GB used, 15.1 busy cores, and the
    /// four user rows drawn there. The fixture is built from the percentages on that screen
    /// so the tests assert the same numbers the UI is supposed to show.
    fn mem_of(pct: f64) -> u64 {
        (64.0 * GIB as f64 * pct / 100.0) as u64
    }

    /// CPU microseconds a query needs to look like it owns `pct` of 16 cores.
    fn cpu_of(pct: f64, elapsed_s: f64) -> u64 {
        (pct / 100.0 * 16.0 * elapsed_s * 1e6) as u64
    }

    fn query(id: &str, user: &str, elapsed: f64, mem: u64, cpu_us: u64) -> QueryRow {
        QueryRow {
            elapsed_s: elapsed,
            memory_bytes: mem,
            cpu_time_us: cpu_us,
            ..QueryRow::new(id, user)
        }
    }

    /// A query as Redash leaves it: `r_redash` as the ClickHouse user, the person in the
    /// comment (§6.4). The fixture would not reproduce §1's screen without this.
    fn redash_query(
        id: &str,
        person: &str,
        redash_id: u64,
        elapsed: f64,
        mem: u64,
        cpu_us: u64,
    ) -> QueryRow {
        let mut q = query(id, crate::attrib::REDASH_USER, elapsed, mem, cpu_us);
        q.person = Some(person.to_string());
        q.redash_query_id = Some(redash_id);
        q.sql = format!(
            "/* Application: Redash */ /* Username: {person}@example.net, Redash query_id: {redash_id}, Redash: 10.1.0 */ SELECT 1"
        );
        q
    }

    /// clickhouse3 as drawn in §1.
    fn node_with_three_users() -> NodeSnapshot {
        NodeSnapshot {
            name: "clickhouse3".into(),
            host: "ch3".into(),
            port: 9000,
            shard: 1,
            replica: 1,
            version: "24.11".into(),
            reachable: true,
            mem_total: Some(64 * GIB),
            mem_used: mem_of(90.9),
            cores: Some(16.0),
            cpu_busy_cores: Some(15.1),
            running: 6,
            lag_s: 0,
            active_parts: 1200,
            queries: vec![
                redash_query("q1", "grigol.gankava", 7438, 275.0, mem_of(25.3), cpu_of(19.4, 275.0)),
                redash_query("q2", "j.petrova", 8585, 164.0, mem_of(12.8), cpu_of(4.4, 164.0)),
                query("q3", "airflow", 37.0, mem_of(5.5), cpu_of(31.2, 37.0)),
                query("q4", "haris", 12.0, mem_of(1.9), cpu_of(1.8, 12.0)),
            ],
            uptime_s: Some(1_000_000),
            server_cpu_time_us: Some(500_000_000_000),
            max_memory_usage: Some(9 * GIB),
            unreachable_reason: None,
            poll_ms: Some(40),
        }
    }

    fn snapshot(nodes: Vec<NodeSnapshot>) -> FleetSnapshot {
        FleetSnapshot {
            taken_at: SystemTime::UNIX_EPOCH,
            nodes,
        }
    }

    // -- §5.3 the invariant ------------------------------------------------

    #[test]
    fn user_rows_plus_closing_row_equal_the_node() {
        let node = node_with_three_users();
        let view = node_view(&node, None);

        let mem_sum: f64 = view.users.iter().filter_map(|u| u.mem_pct).sum();
        let cpu_sum: f64 = view.users.iter().filter_map(|u| u.cpu_pct).sum();

        // Exact, not just "not less": nothing clamps here, so §5.3 is an equality.
        assert!((mem_sum + view.server_mem_pct.unwrap() - view.mem_pct.unwrap()).abs() <= 0.1);
        assert!((cpu_sum + view.server_cpu_pct.unwrap() - view.cpu_pct.unwrap()).abs() <= 0.1);
        assert!(shares_add_up(&view));
    }

    #[test]
    fn the_screen_numbers_come_back_out() {
        let node = node_with_three_users();
        let view = node_view(&node, None);
        let of = |user: &str, person: Option<&str>| {
            let u = view
                .users
                .iter()
                .find(|u| u.user == user && u.person.as_deref() == person)
                .expect("row exists");
            (u.mem_pct.unwrap(), u.cpu_pct.unwrap())
        };

        // §1 draws r_redash twice, once per person: 25.3% and 12.8% of memory, 19.4% and
        // 4.4% of CPU.
        let (g_mem, g_cpu) = of("r_redash", Some("grigol.gankava"));
        assert!((g_mem - 25.3).abs() < 0.1, "grigol mem {g_mem}");
        assert!((g_cpu - 19.4).abs() < 0.1, "grigol cpu {g_cpu}");
        let (j_mem, j_cpu) = of("r_redash", Some("j.petrova"));
        assert!((j_mem - 12.8).abs() < 0.1, "j.petrova mem {j_mem}");
        assert!((j_cpu - 4.4).abs() < 0.1, "j.petrova cpu {j_cpu}");
        assert!((of("airflow", None).0 - 5.5).abs() < 0.1);
        assert!((of("airflow", None).1 - 31.2).abs() < 0.1);
        assert!((of("haris", None).0 - 1.9).abs() < 0.1);
        assert!((of("haris", None).1 - 1.8).abs() < 0.1);

        // Node 58.2 / 64 GB — 90.9% — and 15.1 / 16 cores; the closing rows take the rest,
        // which is where §1's 45.5% and 37.2% come from.
        assert!((view.mem_pct.unwrap() - 90.9).abs() < 0.05);
        assert!((view.cpu_pct.unwrap() - 94.4).abs() < 0.05);
        assert!((view.server_mem_pct.unwrap() - 45.4).abs() < 0.15);
        assert!((view.server_cpu_pct.unwrap() - 37.6).abs() < 0.15);
    }

    #[test]
    fn one_row_per_user_and_person_sorted_by_mem() {
        let node = node_with_three_users();
        let view = node_view(&node, None);
        let labels: Vec<String> = view.users.iter().map(|u| u.label()).collect();
        assert_eq!(
            labels,
            vec!["r_redash → grigol.gankava", "r_redash → j.petrova", "airflow", "haris"]
        );
        assert_eq!(view.users[0].queries.len(), 1);
        assert_eq!(view.users[0].longest_s, 275.0);
    }

    #[test]
    fn splitting_by_person_does_not_change_the_totals() {
        // §5.1 is about not counting a distributed query twice; grouping by person is a
        // refinement of "by initial user", so the sums have to be identical.
        let node = node_with_three_users();
        let view = node_view(&node, None);
        let redash_bytes: u64 = view
            .users
            .iter()
            .filter(|u| u.user == "r_redash")
            .map(|u| u.mem_bytes)
            .sum();
        let query_bytes: u64 = node
            .queries
            .iter()
            .filter(|q| q.user == "r_redash")
            .map(|q| q.memory_bytes)
            .sum();
        assert_eq!(redash_bytes, query_bytes);
        assert!(shares_add_up(&view));
    }

    #[test]
    fn an_unreachable_node_says_why() {
        let node = NodeSnapshot::unreachable("ch4", "HTTP 502: Bad Gateway");
        assert!(!node.reachable);
        assert!(node.mem_total.is_none());
        assert_eq!(node.unreachable_reason.as_deref(), Some("HTTP 502: Bad Gateway"));
    }

    #[test]
    fn an_unknown_memory_denominator_is_none_never_zero() {
        let mut node = node_with_three_users();
        node.mem_total = None;
        let view = node_view(&node, None);

        assert!(view.mem_pct.is_none());
        assert!(view.server_mem_pct.is_none());
        assert!(view.users.iter().all(|u| u.mem_pct.is_none()));
        assert!(!column_known(&view), "a partial column is a lie (§5.3)");
        // CPU is still fine, but the node is not folded: it cannot be measured.
        assert!(!fold_healthy(&view));
    }

    #[test]
    fn a_zero_denominator_is_treated_as_unknown() {
        let mut node = node_with_three_users();
        node.mem_total = Some(0);
        assert!(node_view(&node, None).mem_pct.is_none());
    }

    // -- §5.2 CPU ----------------------------------------------------------

    #[test]
    fn cores_use_the_delta_when_two_samples_exist() {
        let node = node_with_three_users();
        let prev_node = node.clone();
        let mut cur = node;
        // q1 advanced 10 s of wall time and burned 5 s of CPU → 0.5 cores.
        cur.queries[0].elapsed_s = 285.0;
        cur.queries[0].cpu_time_us = prev_node.queries[0].cpu_time_us + 5_000_000;

        let view = node_view(&cur, Some(&prev_node));
        let q1 = view.users[0].queries[0].cores;
        assert!((q1 - 0.5).abs() < 0.001, "delta form gives 0.5, got {q1}");
    }

    #[test]
    fn cores_fall_back_to_the_average_on_first_sighting() {
        let node = node_with_three_users();
        let view = node_view(&node, None);
        // q1 looks like 19.4% of 16 cores = 3.104 cores, and its average over 275 s says the
        // same thing because the fixture burned CPU evenly.
        let q1 = view.users[0].queries[0].cores;
        assert!((q1 - 3.104).abs() < 0.01, "got {q1}");
    }

    #[test]
    fn cores_are_clamped_to_the_node() {
        let mut q = query("q", "r_redash", 1.0, 1, 5_000_000);
        // 5 s of CPU in 1 s of wall time would be 5 cores; the node only has 16, so try a
        // tighter node to prove the clamp bites.
        assert_eq!(query_cores(&q, None, Some(16.0)), 5.0);
        assert_eq!(query_cores(&q, None, Some(2.0)), 2.0);

        // A counter that went backwards (ProfileEvents reset) must not produce a negative
        // share; the saturating subtraction makes it 0.
        let mut prev = q.clone();
        prev.elapsed_s = 0.5;
        prev.cpu_time_us = 9_000_000_000;
        q.cpu_time_us = 1_000;
        assert_eq!(query_cores(&q, Some(&prev), Some(16.0)), 0.0);
    }

    #[test]
    fn node_cpu_falls_back_to_the_event_delta_when_no_metric_family_exists() {
        let mut node = node_with_three_users();
        node.cpu_busy_cores = None;
        node.cores = Some(2.0);

        let mut prev = node.clone();
        node.server_cpu_time_us = Some(10_000_000);
        node.uptime_s = Some(1_010);
        prev.server_cpu_time_us = Some(8_000_000);
        prev.uptime_s = Some(1_000);

        // 2 s of CPU in 10 s of wall time = 0.2 busy cores, and the node has 2 → 10%.
        let pct = node_cpu_pct(&node, Some(&prev)).unwrap();
        assert!((pct - 10.0).abs() < 0.001, "got {pct}");

        // Without a previous sample there is nothing to difference.
        assert!(node_cpu_pct(&node, None).is_none());
    }

    #[test]
    fn node_cpu_prefers_the_async_metric_family_when_present() {
        let node = node_with_three_users(); // cpu_busy_cores 15.1 of 16
        let pct = node_cpu_pct(&node, None).unwrap();
        assert!((pct - 94.375).abs() < 0.01, "got {pct}");
    }

    #[test]
    fn the_closing_cpu_row_covers_the_rest_of_the_node() {
        let node = node_with_three_users();
        let view = node_view(&node, None);
        let cpu_sum: f64 = view.users.iter().filter_map(|u| u.cpu_pct).sum();
        assert!(view.server_cpu_pct.unwrap() >= 0.0);
        assert!((cpu_sum + view.server_cpu_pct.unwrap() - view.cpu_pct.unwrap()).abs() <= 0.1);
    }

    // -- §5.4 runaway ------------------------------------------------------

    #[test]
    fn runaway_by_elapsed_time() {
        let limit = 9 * GIB;
        assert!(is_runaway(&query("q", "u", 30.0, 1, 0), limit));
        assert!(is_runaway(&query("q", "u", 120.0, 1, 0), limit));
        assert!(!is_runaway(&query("q", "u", 29.9, 1, 0), limit));
    }

    #[test]
    fn runaway_by_memory_share_of_the_limit() {
        let limit = 10 * GIB;
        assert!(is_runaway(&query("q", "u", 1.0, 8 * GIB, 0), limit));
        assert!(!is_runaway(&query("q", "u", 1.0, 8 * GIB - 1, 0), limit));
    }

    #[test]
    fn the_fallback_limit_is_nine_gibibytes() {
        let mut node = node_with_three_users();
        node.max_memory_usage = None;
        assert_eq!(mem_limit(&node), FALLBACK_MAX_MEMORY_USAGE);
        assert_eq!(FALLBACK_MAX_MEMORY_USAGE, 9 * GIB);

        let mut zero = node.clone();
        zero.max_memory_usage = Some(0);
        assert_eq!(mem_limit(&zero), FALLBACK_MAX_MEMORY_USAGE, "0 means auto");
    }

    // -- §2.5 fold and sort ------------------------------------------------

    fn healthy(mut node: NodeSnapshot, mem: u64, cpu: Option<f64>) -> NodeSnapshot {
        node.mem_used = mem;
        node.cpu_busy_cores = cpu;
        node.lag_s = 0;
        node.queries.clear();
        node
    }

    #[test]
    fn only_nodes_below_every_threshold_fold() {
        let base = node_with_three_users();
        let quiet = healthy(base.clone(), 20 * GIB, Some(4.0));
        let hot_mem = healthy(base.clone(), 40 * GIB, Some(4.0));
        let hot_cpu = healthy(base.clone(), 20 * GIB, Some(9.0));
        let mut lagging = healthy(base.clone(), 20 * GIB, Some(4.0));
        lagging.lag_s = 12;

        let views: Vec<NodeView<'_>> = vec![
            node_view(&quiet, None),
            node_view(&hot_mem, None),
            node_view(&hot_cpu, None),
            node_view(&lagging, None),
        ];

        let folded: Vec<bool> = views.iter().map(fold_healthy).collect();
        assert_eq!(folded, vec![true, false, false, false]);

        // A runaway query keeps a quiet node on screen.
        let mut with_runaway = healthy(base, 20 * GIB, Some(4.0));
        with_runaway.queries = vec![query("q", "r_redash", 90.0, GIB, 0)];
        assert!(!fold_healthy(&node_view(&with_runaway, None)));
    }

    #[test]
    fn sort_is_pressure_then_runaways_then_name() {
        let base = node_with_three_users();
        let mut quiet = healthy(base.clone(), 10 * GIB, Some(2.0));
        quiet.name = "quiet".into();
        let mut busy = healthy(base.clone(), 60 * GIB, Some(15.0));
        busy.name = "busy".into();
        let mut middle = healthy(base.clone(), 30 * GIB, Some(9.0));
        middle.name = "middle".into();

        // Same pressure, one with a runaway: the runaway one must come first.
        let mut tie_a = healthy(base.clone(), 40 * GIB, Some(8.0));
        tie_a.name = "tie-a".into();
        let mut tie_b = healthy(base.clone(), 40 * GIB, Some(8.0));
        tie_b.name = "tie-b".into();
        tie_b.queries = vec![query("q", "airflow", 31.0, GIB, 0)];

        let nodes = [quiet, busy, middle, tie_a, tie_b];
        let mut views: Vec<NodeView<'_>> = nodes.iter().map(|n| node_view(n, None)).collect();

        views.sort_by(|a, b| compare_nodes(a, b, SortKey::Pressure));
        let names: Vec<&str> = views.iter().map(NodeView::name).collect();
        assert_eq!(names, vec!["busy", "tie-b", "tie-a", "middle", "quiet"]);

        views.sort_by(|a, b| compare_nodes(a, b, SortKey::Name));
        let names: Vec<&str> = views.iter().map(NodeView::name).collect();
        assert_eq!(names, vec!["busy", "middle", "quiet", "tie-a", "tie-b"]);

        assert_eq!(SortKey::Pressure.next(), SortKey::Mem);
        assert_eq!(SortKey::Name.next(), SortKey::Pressure);
    }

    // -- §2.6 NEW ----------------------------------------------------------

    #[test]
    fn a_node_absent_from_the_first_snapshot_is_new_once() {
        let mut first_seen = std::collections::HashSet::new();
        let first = snapshot(vec![
            NodeSnapshot::unreachable("ch1", "first snapshot"),
            NodeSnapshot::unreachable("ch2", "first snapshot"),
        ]);
        assert!(mark_new_nodes(&first, &mut first_seen, true).is_empty());

        let second = snapshot(vec![
            NodeSnapshot::unreachable("ch1", "first snapshot"),
            NodeSnapshot::unreachable("ch2", "first snapshot"),
            NodeSnapshot::unreachable("ch9", "appeared later"),
        ]);
        let new = mark_new_nodes(&second, &mut first_seen, false);
        assert!(new.contains("ch9"));
        assert_eq!(new.len(), 1);

        // Same node next poll: no longer new, and it stays known.
        let third = snapshot(vec![
            NodeSnapshot::unreachable("ch1", "first snapshot"),
            NodeSnapshot::unreachable("ch2", "first snapshot"),
            NodeSnapshot::unreachable("ch9", "appeared later"),
        ]);
        assert!(mark_new_nodes(&third, &mut first_seen, false).is_empty());
    }

    // -- §2.7 pivot --------------------------------------------------------

    #[test]
    fn the_pivot_groups_the_same_data_by_user() {
        let a = node_with_three_users();
        let mut b = healthy(node_with_three_users(), 30 * GIB, Some(4.0));
        b.name = "clickhouse-bi".into();
        b.queries = vec![query("q9", "haris", 300.0, 20 * GIB, 0)];

        let snap = snapshot(vec![a, b]);
        let view = fleet_view(&snap, None);

        let labels: Vec<String> = view.users.iter().map(FleetUser::label).collect();
        // Σ bytes desc: haris runs on both nodes and outweighs either r_redash person.
        assert_eq!(
            labels,
            vec!["haris", "r_redash → grigol.gankava", "r_redash → j.petrova", "airflow"]
        );

        let haris = &view.users[0];
        assert_eq!(haris.user, "haris");
        assert_eq!(haris.nodes.len(), 2, "haris runs on both nodes");
        assert_eq!(haris.mem_bytes, 20 * GIB + mem_of(1.9));
        // Σ bytes desc is the pivot's sort key (§2.7).
        assert!(haris.mem_bytes > view.users[1].mem_bytes);
        assert_eq!(view.users[1].user, "r_redash");
        assert_eq!(view.users[1].person.as_deref(), Some("grigol.gankava"));
        assert_eq!(view.users[2].person.as_deref(), Some("j.petrova"));
    }

    #[test]
    fn the_closing_row_clamps_at_zero_when_users_exceed_the_node() {
        // §5.3: query memory can momentarily exceed MemoryResident accounting. When it does,
        // the closing row is 0 and the sum sits above the node's own percentage instead of
        // equal to it — which is why the invariant is asserted with a tolerance.
        let mut node = node_with_three_users();
        node.mem_used = mem_of(10.0);
        node.cpu_busy_cores = Some(1.0);
        let view = node_view(&node, None);

        assert_eq!(view.server_mem_pct, Some(0.0));
        assert_eq!(view.server_cpu_pct, Some(0.0));
        let mem_sum: f64 = view.users.iter().filter_map(|u| u.mem_pct).sum();
        assert!(mem_sum > view.mem_pct.unwrap());
    }

    #[test]
    fn a_node_view_is_independent_of_the_snapshot_it_came_from() {
        let snap = snapshot(vec![node_with_three_users()]);
        let view = fleet_view(&snap, None);
        assert_eq!(view.nodes.len(), 1);
        assert_eq!(view.users.len(), 4, "two r_redash people plus airflow and haris");
        assert!(shares_add_up(&view.nodes[0]));
    }

    // -- per-query limits and progress ------------------------------------

    #[test]
    fn a_query_is_held_to_its_own_limit_before_the_nodes() {
        let node = node_with_three_users();
        let mut q = query("q", "r_redash", 1.0, 7 * GIB, 0);
        // The node says 9 GiB (the monitor's session), the query's own profile says 8 GiB:
        // 7 GiB is 87.5% of the real limit, so it is runaway by memory.
        q.memory_limit = Some(8 * GIB);
        assert_eq!(query_mem_limit(&q, &node), 8 * GIB);
        assert!(is_runaway(&q, query_mem_limit(&q, &node)));
        // Without its own limit it falls back to the node's and is fine.
        q.memory_limit = None;
        assert_eq!(query_mem_limit(&q, &node), 9 * GIB);
        assert!(!is_runaway(&q, query_mem_limit(&q, &node)));
        // 0 is "unlimited/auto", which is not a limit to measure against.
        q.memory_limit = Some(0);
        assert_eq!(query_mem_limit(&q, &node), 9 * GIB);
    }

    #[test]
    fn progress_and_eta_extrapolate_the_pace_so_far() {
        let mut q = query("q", "u", 60.0, 1, 0);
        assert_eq!(query_progress(&q), None, "no estimate, no progress");
        q.total_rows_approx = 1_000;
        q.read_rows = 250;
        let p = query_progress(&q).unwrap();
        assert!((p - 0.25).abs() < 1e-9);
        // A quarter in 60 s → three quarters left at the same pace → 180 s.
        assert!((query_eta_s(&q, Some(p)).unwrap() - 180.0).abs() < 1e-6);

        // Reading past the estimate is a finishing query, not one at 120%.
        q.read_rows = 1_500;
        assert_eq!(query_progress(&q), Some(1.0));
        assert_eq!(query_eta_s(&q, Some(1.0)), None);

        // Too early to extrapolate.
        q.read_rows = 5;
        assert_eq!(query_eta_s(&q, query_progress(&q)), None);
    }

    #[test]
    fn query_stats_carry_limit_progress_and_eta() {
        let mut node = node_with_three_users();
        node.queries[0].total_rows_approx = 2_000;
        node.queries[0].read_rows = 1_000;
        let view = node_view(&node, None);
        let q1 = &view.users[0].queries[0];
        assert_eq!(q1.limit, 9 * GIB);
        assert_eq!(q1.progress, Some(0.5));
        assert!((q1.eta_s.unwrap() - 275.0).abs() < 1e-6, "half way after 275 s");
        assert!((q1.limit_fraction() - mem_of(25.3) as f64 / (9 * GIB) as f64).abs() < 1e-9);
    }

    // -- fleet totals --------------------------------------------------------

    #[test]
    fn fleet_totals_skip_what_cannot_be_measured() {
        let a = node_with_three_users();
        let mut b = healthy(node_with_three_users(), 16 * GIB, Some(4.0));
        b.name = "b".into();
        let mut unknown = node_with_three_users();
        unknown.name = "unknown".into();
        unknown.mem_total = None;
        unknown.cores = None;
        let dead = NodeSnapshot::unreachable("dead", "connection refused");

        let snap = snapshot(vec![a, b, unknown, dead]);
        let view = fleet_view(&snap, None);
        let totals = fleet_totals(&view);

        assert_eq!(totals.nodes, 4);
        assert_eq!(totals.reachable, 3);
        // a and b only: the unknown node is in neither the numerator nor the denominator.
        assert_eq!(totals.mem_total, 128 * GIB);
        assert_eq!(totals.mem_used, mem_of(90.9) + 16 * GIB);
        assert!((totals.cores - 32.0).abs() < 1e-9);
        assert!((totals.busy_cores - 19.1).abs() < 1e-9);
        assert!((totals.cpu_pct().unwrap() - 19.1 / 32.0 * 100.0).abs() < 1e-9);
        // 4 queries on a, 4 on unknown, none on b.
        assert_eq!(totals.queries, 8);
        // q1 and q2 run past 30 s, so do q3 (37 s) — on a and on unknown.
        assert_eq!(totals.runaways, 6);
        assert_eq!(totals.hot, 1, "a is hot; b is quiet and unknown cannot be measured");

        let empty = fleet_totals(&FleetView { nodes: vec![], users: vec![] });
        assert_eq!(empty.mem_pct(), None, "no denominator, no percentage");
        assert_eq!(empty.cpu_pct(), None);
    }
}
