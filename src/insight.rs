//! Insights: the screen, read out loud.
//!
//! Every number in view 1 is already on screen; what on call does with them at 3am is the
//! same few deductions every time — *which node is in trouble, is it a query or the server,
//! whose query, is it getting worse, why is the Redash queue full*. This module makes those
//! deductions once per poll and writes them down as ranked sentences, each pointing at the row
//! it is about so `⏎` can jump there.
//!
//! Pure: a fleet view, its history and the queue in, sentences out. Every claim is traceable
//! to a §5 number or to a slope over the history (`history.rs`); nothing is guessed.

use crate::fmt;
use crate::history::{eta_to, History};
use crate::model::{FleetView, JobState, NodeView, QueryStat, QueueStatus, UserSlice};
use crate::severity::{self, Severity};
use std::collections::{BTreeMap, HashSet};

/// What an insight is about — where `⏎` goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Fleet,
    Node(String),
    Query {
        node: String,
        user: String,
        person: Option<String>,
        query_id: String,
    },
    Queue,
}

/// How a piece of an insight is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Strong,
    Muted,
    Node,
    Person,
    Sev(Severity),
}

#[derive(Debug, Clone)]
pub struct Insight {
    pub level: Severity,
    /// Within a level, higher first.
    pub score: f64,
    pub subject: Subject,
    pub parts: Vec<(String, Tone)>,
}

impl Insight {
    pub fn text(&self) -> String {
        self.parts.iter().map(|(t, _)| t.as_str()).collect()
    }
}

/// A sentence being written.
#[derive(Default)]
struct Text(Vec<(String, Tone)>);

impl Text {
    fn add(mut self, text: impl Into<String>, tone: Tone) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.0.push((text, tone));
        }
        self
    }
    fn plain(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Plain)
    }
    fn strong(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Strong)
    }
    fn muted(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Muted)
    }
    fn node(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Node)
    }
    fn person(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Person)
    }
    fn sev(self, text: impl Into<String>, sev: Severity) -> Self {
        self.add(text, Tone::Sev(sev))
    }
}

/// Memory climbing slower than this per minute is not worth a forecast.
const FORECAST_MIN_SLOPE_PER_MIN: f64 = 0.3;
/// Forecasts further out than this are not news.
const FORECAST_HORIZON_S: f64 = 30.0 * 60.0;
/// The window the forecast regresses over.
const FORECAST_WINDOW_S: f64 = 90.0;
/// A node this slow to answer is worth a line.
const SLOW_POLL_MS: f64 = 750.0;
/// CPU severity is judged over this window (see `hot_node`).
const CPU_STEADY_S: f64 = 10.0;
/// A kill further out than this is not a forecast worth printing.
pub const KILL_HORIZON_S: f64 = 30.0 * 60.0;
/// How long a NEW node keeps its own insight.
const NEW_NODE_NEWS_S: f64 = 10.0 * 60.0;

/// Who a slice is, in the fewest words: the person when attribution found one (§6.4), the
/// ClickHouse user otherwise.
fn who(user: &str, person: Option<&str>) -> String {
    match person {
        Some(person) => crate::attrib::display_person(person),
        None => user.to_string(),
    }
}

fn who_slice(slice: &UserSlice<'_>) -> String {
    who(&slice.user, slice.person.as_deref())
}

/// Everything worth saying about the fleet right now, worst first.
pub fn analyze(
    view: &FleetView<'_>,
    history: &History,
    queue: &QueueStatus,
    new_nodes: &HashSet<String>,
) -> Vec<Insight> {
    let mut out = Vec::new();
    let now = history.now();

    for node in &view.nodes {
        unreachable(node, history, now, &mut out);
        if !node.node.reachable {
            continue;
        }
        hot_node(node, history, &mut out);
        memory_forecast(node, history, &mut out);
        lag(node, &mut out);
        slow_poll(node, &mut out);
        unknown_denominators(node, &mut out);
        new_node(node, new_nodes, history, now, &mut out);
        for slice in &node.users {
            for stat in &slice.queries {
                near_limit(node, slice, stat, history, &mut out);
            }
        }
    }
    runaways(view, &mut out);
    duplicates(view, &mut out);
    queue_backlog(queue, view, history, &mut out);
    heaviest(view, &mut out);

    if !out.iter().any(|i| i.level.is_problem()) {
        let reachable = view.nodes.iter().filter(|n| n.node.reachable).count();
        out.push(Insight {
            level: Severity::Ok,
            score: 0.0,
            subject: Subject::Fleet,
            parts: Text::default()
                .strong(format!("All {reachable} nodes healthy"))
                .plain(" · no runaway queries")
                .plain(if queue.reachable { " · Redash queue keeping up" } else { "" })
                .0,
        });
    }

    out.sort_by(|a, b| {
        b.level
            .cmp(&a.level)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.text().cmp(&b.text()))
    });
    out
}

/// The worst level among the insights: what the status pill in the header says.
pub fn overall(insights: &[Insight]) -> Severity {
    insights
        .iter()
        .map(|i| i.level)
        .max()
        .unwrap_or(Severity::None)
}

fn unreachable(node: &NodeView<'_>, history: &History, now: f64, out: &mut Vec<Insight>) {
    if node.node.reachable {
        return;
    }
    let since = history
        .node(&node.node.name)
        .and_then(|h| h.unreachable_since)
        .map(|t| now - t);
    let mut text = Text::default().node(&node.node.name).plain(" is ").sev("unreachable", Severity::Crit);
    if let Some(for_s) = since.filter(|s| *s >= 1.0) {
        text = text.plain(format!(" for {}", fmt::dur(for_s)));
    }
    if let Some(reason) = &node.node.unreachable_reason {
        text = text.muted(format!(" — {reason}"));
    }
    out.push(Insight {
        level: Severity::Crit,
        score: 1000.0,
        subject: Subject::Node(node.node.name.clone()),
        parts: text.0,
    });
}

/// One sentence per hot node: which resource, and who holds it — a user, or the server itself.
///
/// CPU is judged on its last 10 seconds rather than on one poll: a real node's CPU jumps by
/// twenty points from one poll to the next, and an insight that comes and goes every two
/// seconds is noise. The sentence still quotes the number on screen.
fn hot_node(node: &NodeView<'_>, history: &History, out: &mut Vec<Insight>) {
    let mem_sev = severity::node(node.mem_pct);
    let steady_cpu = history
        .node(&node.node.name)
        .and_then(|h| h.cpu_pct.mean_in(CPU_STEADY_S))
        .or(node.cpu_pct);
    let cpu_sev = severity::node(node.cpu_pct.and(steady_cpu));
    let level = mem_sev.max(cpu_sev);
    if !level.is_problem() {
        return;
    }

    let mut text = Text::default().node(&node.node.name).plain(" ");
    let mut first = true;
    if mem_sev.is_problem() {
        text = text
            .plain("memory ")
            .sev(fmt::pct(node.mem_pct), mem_sev);
        first = false;
    }
    if cpu_sev.is_problem() {
        text = text
            .plain(if first { "CPU " } else { ", CPU " })
            .sev(fmt::pct(node.cpu_pct), cpu_sev);
    }

    // Who holds each hot resource: the biggest user row, or the closing row when that is
    // bigger — "it is the server, not a query" is the most useful thing this line can say.
    // When one holder has both, it is said once.
    let mem_holder = mem_sev.is_problem().then(|| holder(node, |u| u.mem_pct, node.server_mem_pct));
    let cpu_holder = cpu_sev.is_problem().then(|| holder(node, |u| u.cpu_pct, node.server_cpu_pct));
    match (mem_holder, cpu_holder) {
        (Some(mem), Some(cpu)) if mem.0 == cpu.0 => {
            text = text.plain(" — ");
            text = mem.0.say(text);
            text = text.plain(format!(
                " {} {} of memory and {} of CPU",
                mem.0.verb(),
                fmt::pct0(mem.1),
                fmt::pct0(cpu.1)
            ));
        }
        (mem, cpu) => {
            let mut first = true;
            for (held, resource) in [(mem, "memory"), (cpu, "CPU")] {
                let Some((who, pct)) = held else {
                    continue;
                };
                text = text.plain(if first { " — " } else { ", " });
                first = false;
                text = who.say(text);
                text = text.plain(format!(" {} {} of {resource}", who.verb(), fmt::pct0(pct)));
            }
        }
    }

    let runaways: usize = node
        .users
        .iter()
        .flat_map(|u| u.queries.iter())
        .filter(|q| q.runaway)
        .count();
    if runaways > 0 {
        text = text.plain(" · ").sev(fmt::plural(runaways, "runaway", "runaways"), Severity::Crit);
    }

    // Headroom against the per-query limit: less free memory than one more query is allowed
    // to take means the next heavy dashboard can push the server over.
    if mem_sev.is_problem()
        && let Some(total) = node.node.mem_total
    {
        let free = total.saturating_sub(node.node.mem_used);
        let limit = node
            .users
            .iter()
            .flat_map(|u| u.queries.iter())
            .map(|q| q.limit)
            .max()
            .unwrap_or(node.mem_limit);
        if free < limit {
            text = text
                .plain(" · only ")
                .sev(fmt::bytes(free), mem_sev)
                .plain(format!(" free, less than one {} query", fmt::bytes(limit)));
        }
    }

    out.push(Insight {
        level,
        score: 500.0 + node.pressure,
        subject: Subject::Node(node.node.name.clone()),
        parts: text.0,
    });
}

/// Who holds the biggest share of a resource on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Holder {
    Person(String),
    Server,
}

impl Holder {
    fn say(&self, text: Text) -> Text {
        match self {
            Holder::Person(who) => text.person(who.clone()),
            Holder::Server => text.strong("server/caches/merges"),
        }
    }

    fn verb(&self) -> &'static str {
        match self {
            Holder::Person(_) => "holds",
            Holder::Server => "hold",
        }
    }
}

fn holder(
    node: &NodeView<'_>,
    share: impl Fn(&UserSlice<'_>) -> Option<f64>,
    server: Option<f64>,
) -> (Holder, f64) {
    let top = node
        .users
        .iter()
        .filter_map(|u| share(u).map(|s| (u, s)))
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let server = server.unwrap_or(0.0);
    match top {
        Some((slice, pct)) if pct >= server => (Holder::Person(who_slice(slice)), pct),
        _ => (Holder::Server, server),
    }
}

/// Memory climbing steadily enough to run out inside half an hour.
fn memory_forecast(node: &NodeView<'_>, history: &History, out: &mut Vec<Insight>) {
    let (Some(pct), Some(series)) = (node.mem_pct, history.node(&node.node.name).map(|h| &h.mem_pct)) else {
        return;
    };
    if series.span_s() < crate::history::FORECAST_MIN_SPAN_S {
        return;
    }
    let Some(slope) = series.slope(FORECAST_WINDOW_S) else {
        return;
    };
    let per_min = slope * 60.0;
    if per_min < FORECAST_MIN_SLOPE_PER_MIN || pct < 50.0 {
        return;
    }
    let Some(eta) = eta_to(pct, slope, 100.0).filter(|e| *e <= FORECAST_HORIZON_S) else {
        return;
    };
    let level = if eta <= 5.0 * 60.0 { Severity::Crit } else { Severity::Warn };
    out.push(Insight {
        level,
        score: 450.0 + (FORECAST_HORIZON_S - eta) / 60.0,
        subject: Subject::Node(node.node.name.clone()),
        parts: Text::default()
            .node(&node.node.name)
            .plain(" memory climbing ")
            .sev(format!("+{per_min:.1}%/min"), level)
            .plain(" — full in ")
            .sev(fmt::eta(eta), level)
            .muted(" at this rate")
            .0,
    });
}

fn lag(node: &NodeView<'_>, out: &mut Vec<Insight>) {
    let sev = severity::lag(node.node.lag_s);
    if !sev.is_problem() {
        return;
    }
    out.push(Insight {
        level: sev,
        score: 200.0 + node.node.lag_s as f64,
        subject: Subject::Node(node.node.name.clone()),
        parts: Text::default()
            .node(&node.node.name)
            .plain(" replica lag ")
            .sev(fmt::dur(node.node.lag_s as f64), sev)
            .muted(" — reads there may be stale")
            .0,
    });
}

fn slow_poll(node: &NodeView<'_>, out: &mut Vec<Insight>) {
    let Some(ms) = node.node.poll_ms.map(f64::from).filter(|ms| *ms >= SLOW_POLL_MS) else {
        return;
    };
    out.push(Insight {
        level: Severity::Info,
        score: 60.0 + ms / 100.0,
        subject: Subject::Node(node.node.name.clone()),
        parts: Text::default()
            .node(&node.node.name)
            .plain(" is slow to answer: ")
            .strong(format!("{ms:.0} ms"))
            .muted(" per poll")
            .0,
    });
}

fn unknown_denominators(node: &NodeView<'_>, out: &mut Vec<Insight>) {
    let missing = match (node.node.mem_total, node.node.cores) {
        (None, None) => "neither a memory total nor a core count",
        (None, Some(_)) => "no memory total",
        (Some(_), None) => "no core count",
        (Some(_), Some(_)) => return,
    };
    out.push(Insight {
        level: Severity::Info,
        score: 40.0,
        subject: Subject::Node(node.node.name.clone()),
        parts: Text::default()
            .node(&node.node.name)
            .plain(format!(" reports {missing}"))
            .muted(" — its percentages show — instead of a guess")
            .0,
    });
}

fn new_node(
    node: &NodeView<'_>,
    new_nodes: &HashSet<String>,
    history: &History,
    now: f64,
    out: &mut Vec<Insight>,
) {
    if !new_nodes.contains(&node.node.name) {
        return;
    }
    let age = history
        .node(&node.node.name)
        .map(|h| now - h.first_seen)
        .unwrap_or(0.0);
    if age > NEW_NODE_NEWS_S {
        return;
    }
    out.push(Insight {
        level: Severity::Info,
        score: 70.0,
        subject: Subject::Node(node.node.name.clone()),
        parts: Text::default()
            .node(&node.node.name)
            .plain(" joined the fleet")
            .muted(if age >= 1.0 { format!(" {} ago", fmt::dur(age)) } else { String::new() })
            .0,
    });
}

/// A query close to the memory limit its own profile sets: ClickHouse is about to kill it with
/// MEMORY_LIMIT_EXCEEDED, and whoever started it will want to know before the dashboard
/// errors.
fn near_limit(
    node: &NodeView<'_>,
    slice: &UserSlice<'_>,
    stat: &QueryStat<'_>,
    history: &History,
    out: &mut Vec<Insight>,
) {
    let fraction = stat.limit_fraction();
    let sev = severity::limit_share(fraction);
    if !sev.is_problem() {
        return;
    }
    let query = stat.query;
    let mut text = Text::default()
        .person(who_slice(slice))
        .plain("'s query on ")
        .node(&node.node.name)
        .plain(" at ")
        .sev(fmt::pct0(fraction * 100.0), sev)
        .plain(format!(" of its {} memory limit", fmt::bytes(stat.limit)));
    let rate = history
        .query(&node.node.name, &query.query_id)
        .and_then(|h| h.mem_rate())
        .filter(|r| *r > 0.0);
    if let Some(rate) = rate {
        text = text.plain(" · ").strong(format!("+{}", fmt::rate(rate)));
        if let Some(eta) = eta_to(query.memory_bytes as f64, rate, stat.limit as f64)
            .filter(|eta| *eta <= KILL_HORIZON_S)
        {
            text = text.plain(" → killed in ").sev(fmt::eta(eta), Severity::Crit);
        }
    }
    out.push(Insight {
        level: sev,
        // Capped below a hot node's score: one query in trouble ranks under the node it is on.
        score: 400.0 + fraction.min(1.0) * 99.0,
        subject: query_subject(node, slice, stat),
        parts: text.0,
    });
}

fn query_subject(node: &NodeView<'_>, slice: &UserSlice<'_>, stat: &QueryStat<'_>) -> Subject {
    Subject::Query {
        node: node.node.name.clone(),
        user: slice.user.clone(),
        person: slice.person.clone(),
        query_id: stat.query.query_id.clone(),
    }
}

/// The runaways of the whole fleet in one line, led by the longest.
fn runaways(view: &FleetView<'_>, out: &mut Vec<Insight>) {
    let mut all: Vec<(&NodeView<'_>, &UserSlice<'_>, &QueryStat<'_>)> = Vec::new();
    for node in &view.nodes {
        for slice in &node.users {
            for stat in &slice.queries {
                if stat.runaway {
                    all.push((node, slice, stat));
                }
            }
        }
    }
    let Some(&(node, slice, longest)) = all
        .iter()
        .max_by(|a, b| a.2.query.elapsed_s.total_cmp(&b.2.query.elapsed_s))
    else {
        return;
    };
    let summary = crate::sqltext::summary(&longest.query.sql);
    let text = Text::default()
        .sev(fmt::plural(all.len(), "runaway query", "runaway queries"), Severity::Crit)
        .plain(" · longest ")
        .person(who_slice(slice))
        .plain(" ")
        .strong(fmt::dur(longest.query.elapsed_s))
        .plain(" on ")
        .node(&node.node.name)
        .muted(format!(" ({})", summary.label()));
    out.push(Insight {
        level: Severity::Warn,
        score: 380.0 + all.len() as f64,
        subject: query_subject(node, slice, longest),
        parts: text.0,
    });
}

/// The same Redash query running more than once at the same time. Distributed fan-out cannot
/// cause this — secondary queries are not initial (§5.1) — so two initial queries with one
/// Redash number are two executions: a dashboard refreshed twice, or two people on it.
fn duplicates(view: &FleetView<'_>, out: &mut Vec<Insight>) {
    let mut by_id: BTreeMap<u64, Vec<(&NodeView<'_>, &UserSlice<'_>, &QueryStat<'_>)>> = BTreeMap::new();
    for node in &view.nodes {
        for slice in &node.users {
            for stat in &slice.queries {
                if let Some(id) = stat.query.redash_query_id {
                    by_id.entry(id).or_default().push((node, slice, stat));
                }
            }
        }
    }
    for (id, runs) in by_id {
        if runs.len() < 2 {
            continue;
        }
        let mut nodes: Vec<&str> = runs.iter().map(|(n, _, _)| n.node.name.as_str()).collect();
        nodes.dedup();
        let mut people: Vec<String> = runs.iter().map(|(_, s, _)| who_slice(s)).collect();
        people.sort();
        people.dedup();
        let bytes: u64 = runs.iter().map(|(_, _, q)| q.query.memory_bytes).sum();
        let (node, slice, stat) = runs[0];
        let mut text = Text::default()
            .strong(format!("Redash #{id}"))
            .plain(" is running ")
            .sev(format!("{}×", runs.len()), Severity::Warn)
            .plain(" (")
            .node(nodes.join(", "))
            .plain(") by ")
            .person(people.join(", "));
        // What the copies cost together, when that is worth saying.
        if bytes >= 64 * 1024 * 1024 {
            text = text.plain(" — ").strong(fmt::bytes(bytes)).plain(" together");
        }
        out.push(Insight {
            level: Severity::Warn,
            score: 300.0 + runs.len() as f64,
            subject: query_subject(node, slice, stat),
            parts: text.0,
        });
    }
}

/// Why the Redash queue is full: are the workers stuck on runaway ClickHouse queries, or just
/// busy (§2.8)?
fn queue_backlog(queue: &QueueStatus, view: &FleetView<'_>, history: &History, out: &mut Vec<Insight>) {
    if !queue.reachable {
        match queue.error.as_deref() {
            _ if queue.is_placeholder() => {}
            None => {}
            Some(error) => out.push(Insight {
                level: Severity::Info,
                score: 150.0,
                subject: Subject::Queue,
                parts: Text::default()
                    .strong("Redash queue unreachable")
                    .muted(format!(" ({error}) — queue numbers are not live"))
                    .0,
            }),
        }
        return;
    }
    for row in &queue.queues {
        let saturated = row.saturated();
        let oldest = row.oldest_wait_s.unwrap_or(0);
        let sev = severity::wait(oldest, saturated);
        if !sev.is_problem() || row.waiting == 0 {
            continue;
        }
        let mut text = Text::default()
            .strong(format!("Redash {}", row.name))
            .plain(": ")
            .sev(format!("{} waiting", row.waiting), sev)
            .plain(", oldest ")
            .sev(fmt::opt_dur(row.oldest_wait_s), severity::wait(oldest, false))
            .plain(format!(" · workers {}/{} busy", row.workers_busy, row.workers_total));

        let stuck: Vec<&str> = queue
            .jobs
            .iter()
            .filter(|j| j.queue == row.name && j.state == JobState::Started)
            .filter_map(|j| j.clickhouse_target())
            .filter(|(node, query_id)| is_runaway(view, node, query_id))
            .map(|(node, _)| node)
            .collect();
        if stuck.is_empty() {
            text = text.muted(" — none of them stuck in ClickHouse");
        } else {
            let mut nodes = stuck.clone();
            nodes.sort();
            nodes.dedup();
            text = text
                .plain(" — ")
                .sev(format!("{} on runaway queries", stuck.len()), Severity::Crit)
                .plain(" (")
                .node(nodes.join(", "))
                .plain(")");
        }
        if let Some(series) = history.queue(&row.name)
            && let Some(slope) = series.slope(60.0)
        {
            let per_min = slope * 60.0;
            if per_min >= 1.0 {
                text = text.plain(" · ").sev(format!("growing +{per_min:.0}/min"), Severity::Warn);
            }
        }
        out.push(Insight {
            level: sev,
            score: 350.0 + oldest as f64 / 10.0,
            subject: Subject::Queue,
            parts: text.0,
        });
    }
}

fn is_runaway(view: &FleetView<'_>, node: &str, query_id: &str) -> bool {
    view.nodes
        .iter()
        .filter(|n| n.node.name == node)
        .flat_map(|n| n.users.iter())
        .flat_map(|u| u.queries.iter())
        .any(|q| q.query.query_id == query_id && q.runaway)
}

/// Who is using the most memory across the fleet (§2.7's question, answered without `u`).
fn heaviest(view: &FleetView<'_>, out: &mut Vec<Insight>) {
    let Some(top) = view.users.first().filter(|u| u.mem_bytes > 0) else {
        return;
    };
    let total: u64 = view
        .nodes
        .iter()
        .filter(|n| n.node.reachable)
        .filter_map(|n| n.node.mem_total)
        .sum();
    let mut text = Text::default()
        .plain("Heaviest user: ")
        .person(who(&top.user, top.person.as_deref()))
        .plain(" — ")
        .strong(fmt::bytes(top.mem_bytes))
        .plain(format!(
            " on {} · {}",
            fmt::plural(top.nodes.len(), "node", "nodes"),
            fmt::plural(top.queries, "query", "queries")
        ));
    if total > 0 {
        text = text.muted(format!(
            " ({:.1}% of fleet memory)",
            top.mem_bytes as f64 / total as f64 * 100.0
        ));
    }
    let first_node = top.nodes.first().map(|n| n.node.name.clone());
    out.push(Insight {
        level: Severity::Info,
        score: 100.0,
        subject: match first_node {
            Some(node) => Subject::Node(node),
            None => Subject::Fleet,
        },
        parts: text.0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use crate::model::{fleet_totals, fleet_view, FleetSnapshot, NodeSnapshot, QueryRow};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn quiet_node(name: &str) -> NodeSnapshot {
        NodeSnapshot {
            name: name.into(),
            host: name.into(),
            port: 9000,
            shard: 1,
            replica: 1,
            version: "24.10".into(),
            reachable: true,
            mem_total: Some(64 * GIB),
            mem_used: 10 * GIB,
            cores: Some(16.0),
            cpu_busy_cores: Some(2.0),
            running: 0,
            lag_s: 0,
            active_parts: 10,
            queries: vec![],
            uptime_s: Some(100),
            server_cpu_time_us: None,
            max_memory_usage: Some(9 * GIB),
            unreachable_reason: None,
            poll_ms: Some(20),
        }
    }

    fn redash_query(id: &str, person: &str, redash: u64, elapsed: f64, mem: u64) -> QueryRow {
        let mut q = QueryRow::new(id, "r_redash");
        q.person = Some(person.into());
        q.redash_query_id = Some(redash);
        q.elapsed_s = elapsed;
        q.memory_bytes = mem;
        q.sql = "SELECT count() FROM wallet.ledger".into();
        q
    }

    fn snap(nodes: Vec<NodeSnapshot>) -> FleetSnapshot {
        FleetSnapshot {
            taken_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000),
            nodes,
        }
    }

    fn analyze_snapshot(snapshot: &FleetSnapshot, queue: &QueueStatus) -> Vec<Insight> {
        let view = fleet_view(snapshot, None);
        let mut history = History::default();
        history.record_fleet(&view, &fleet_totals(&view), snapshot.taken_at);
        analyze(&view, &history, queue, &HashSet::new())
    }

    #[test]
    fn a_quiet_fleet_says_so() {
        let s = snap(vec![quiet_node("a"), quiet_node("b")]);
        let insights = analyze_snapshot(&s, &QueueStatus::unreachable("not polled yet"));
        assert_eq!(insights.len(), 1, "{:?}", insights.iter().map(Insight::text).collect::<Vec<_>>());
        assert_eq!(insights[0].level, Severity::Ok);
        assert!(insights[0].text().starts_with("All 2 nodes healthy"));
        assert_eq!(overall(&insights), Severity::Ok);
    }

    #[test]
    fn a_hot_node_names_who_holds_it() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 60 * GIB; // 93.75%
        hot.queries = vec![redash_query("q1", "grigol.gankava", 7438, 12.0, 30 * GIB)];
        let s = snap(vec![hot, quiet_node("calm")]);
        let insights = analyze_snapshot(&s, &QueueStatus::unreachable("not polled yet"));
        let first = &insights[0];
        assert_eq!(first.level, Severity::Crit);
        assert_eq!(first.subject, Subject::Node("hot".into()));
        let text = first.text();
        assert!(text.contains("hot memory 93.8%"), "{text}");
        assert!(text.contains("grigol.gankava holds 47% of memory"), "{text}");
        assert!(!text.contains("CPU"), "CPU is quiet on this node: {text}");
        assert!(text.contains("free, less than one"), "4 GiB free < a 9 GiB query: {text}");
    }

    #[test]
    fn one_holder_of_both_resources_is_named_once() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 60 * GIB;
        hot.cpu_busy_cores = Some(15.0);
        hot.queries = vec![redash_query("q1", "j.petrova", 1, 3.0, GIB)];
        let s = snap(vec![hot]);
        let text = analyze_snapshot(&s, &QueueStatus::unreachable("x"))[0].text();
        assert!(
            text.contains("server/caches/merges hold 92% of memory and 94% of CPU"),
            "{text}"
        );
        assert_eq!(text.matches("server/caches/merges").count(), 1, "{text}");
    }

    #[test]
    fn one_poll_of_hot_cpu_is_not_an_insight() {
        let mut history = History::default();
        let mut node = quiet_node("spiky");
        let mut last = None;
        // 20% for 10 s, then one poll at 95%.
        for i in 0..6u64 {
            node.cpu_busy_cores = Some(if i == 5 { 15.2 } else { 3.2 });
            let mut s = snap(vec![node.clone()]);
            s.taken_at += std::time::Duration::from_secs(i * 2);
            let view = fleet_view(&s, None);
            history.record_fleet(&view, &fleet_totals(&view), s.taken_at);
            last = Some(s);
        }
        let s = last.unwrap();
        let view = fleet_view(&s, None);
        let insights = analyze(&view, &history, &QueueStatus::unreachable("x"), &HashSet::new());
        assert!(!insights.iter().any(|i| i.text().contains("CPU")), "{:?}", insights.iter().map(Insight::text).collect::<Vec<_>>());
    }

    #[test]
    fn a_hot_node_held_by_the_server_says_it_is_not_a_query() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 50 * GIB; // 78%
        hot.queries = vec![redash_query("q1", "j.petrova", 1, 3.0, GIB)];
        let s = snap(vec![hot]);
        let text = analyze_snapshot(&s, &QueueStatus::unreachable("x"))[0].text();
        assert!(text.contains("server/caches/merges hold"), "{text}");
    }

    #[test]
    fn duplicates_are_two_initial_runs_of_one_redash_query() {
        let mut a = quiet_node("a");
        a.queries = vec![redash_query("q1", "j.petrova", 8585, 3.0, GIB)];
        let mut b = quiet_node("b");
        b.queries = vec![redash_query("q2", "j.petrova", 8585, 2.0, GIB)];
        let s = snap(vec![a, b]);
        let insights = analyze_snapshot(&s, &QueueStatus::unreachable("x"));
        let dup = insights
            .iter()
            .find(|i| i.text().contains("Redash #8585"))
            .expect("the duplicate is reported");
        assert!(dup.text().contains("2×"), "{}", dup.text());
        assert!(dup.text().contains("a, b"), "{}", dup.text());
        assert_eq!(dup.level, Severity::Warn);
    }

    #[test]
    fn a_query_near_its_own_limit_is_reported_with_the_limit_it_will_hit() {
        let mut node = quiet_node("n");
        let mut q = redash_query("q1", "m.kairys", 7711, 3.0, 8 * GIB);
        q.memory_limit = Some(8 * GIB + GIB / 4); // 97%
        node.queries = vec![q];
        let s = snap(vec![node]);
        let insights = analyze_snapshot(&s, &QueueStatus::unreachable("x"));
        let near = insights
            .iter()
            .find(|i| i.text().contains("memory limit"))
            .expect("near-limit query");
        assert_eq!(near.level, Severity::Crit);
        assert!(near.text().contains("m.kairys's query on n at 97%"), "{}", near.text());
        assert!(matches!(near.subject, Subject::Query { ref query_id, .. } if query_id == "q1"));
    }

    #[test]
    fn an_unreachable_node_comes_first() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 63 * GIB;
        let s = snap(vec![hot, NodeSnapshot::unreachable("dead", "connection refused")]);
        let insights = analyze_snapshot(&s, &QueueStatus::unreachable("x"));
        assert!(insights[0].text().starts_with("dead is unreachable"), "{}", insights[0].text());
        assert!(insights[0].text().contains("connection refused"));
    }

    #[test]
    fn a_steady_climb_is_forecast() {
        let mut history = History::default();
        let mut node = quiet_node("climb");
        let mut last = None;
        // +1% of 64 GiB every 10 s = +6%/min, from 70%.
        for i in 0..10u64 {
            node.mem_used = (64.0 * GIB as f64 * (0.70 + 0.01 * i as f64)) as u64;
            let mut s = snap(vec![node.clone()]);
            s.taken_at += std::time::Duration::from_secs(i * 10);
            let view = fleet_view(&s, None);
            history.record_fleet(&view, &fleet_totals(&view), s.taken_at);
            last = Some(s);
        }
        let s = last.unwrap();
        let view = fleet_view(&s, None);
        let insights = analyze(&view, &history, &QueueStatus::unreachable("x"), &HashSet::new());
        let forecast = insights
            .iter()
            .find(|i| i.text().contains("climbing"))
            .expect("a forecast");
        assert!(forecast.text().contains("+6.0%/min"), "{}", forecast.text());
        // 79% now, 21 points to go at 6/min = 3.5 min: red.
        assert!(forecast.text().contains("full in ~3m30s"), "{}", forecast.text());
        assert_eq!(forecast.level, Severity::Crit);
    }

    #[test]
    fn the_fake_fleet_explains_its_full_queue() {
        let mut fake = FakeSource::new();
        let snapshot = fake.snapshot();
        let mut queue = fake.queue();
        // The app's stitch: running jobs point at ClickHouse queries.
        for job in &mut queue.jobs {
            if job.state == JobState::Started
                && let Some(redash) = job.redash_query_id
                && let Some((node, q)) = snapshot.nodes.iter().find_map(|n| {
                    n.queries
                        .iter()
                        .find(|q| q.redash_query_id == Some(redash))
                        .map(|q| (n.name.clone(), q.query_id.clone()))
                })
            {
                job.ch_node = Some(node);
                job.ch_query_id = Some(q);
            }
        }
        let insights = analyze_snapshot(&snapshot, &queue);
        let backlog = insights
            .iter()
            .find(|i| i.subject == Subject::Queue)
            .expect("the backed-up queue is explained");
        let text = backlog.text();
        assert!(text.contains("waiting"), "{text}");
        assert!(text.contains("on runaway queries"), "{text}");
        // And the fleet's own trouble is all there.
        assert!(insights.iter().any(|i| i.text().starts_with("clickhouse3 ")));
        assert!(insights.iter().any(|i| i.text().contains("runaway quer")));
        assert_eq!(overall(&insights), Severity::Crit);
    }
}
