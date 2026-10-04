//! The tape (view 4): what changed, in order.
//!
//! View 1 answers "what is happening now". The tape answers the question on call has after
//! looking away for ten minutes: *what happened while I was not looking?* A node that went
//! hot and cooled down, a query that ran away and then ended — probably killed at its memory
//! limit — a queue that backed up and drained. None of that is on a live screen any more.
//!
//! `Watch` remembers the last state of everything it reports on and turns differences into
//! events; it never measures anything itself (§5 already did).

use crate::fmt;
use crate::insight::{Subject, Tone};
use crate::model::{FleetView, NodeView, QueryStat, QueueStatus, UserSlice};
use crate::severity::{self, Severity};
use std::collections::{HashMap, HashSet, VecDeque};

/// Enough for a long shift at the default poll; older events fall off the end.
const CAPACITY: usize = 2000;

/// A node has to fall this many points below a threshold before the tape calls it recovered,
/// so a node hovering at 75% does not write "75.2% · back to 74.8% · 75.1%" all night. CPU
/// moves far more between two polls than memory does, so it gets more room.
const HYSTERESIS_MEM: f64 = 3.0;
const HYSTERESIS_CPU: f64 = 6.0;
/// And a new state has to be seen on this many polls in a row before it is reported: one poll
/// of 97% CPU on a node that is otherwise at 70% is a spike, not an event.
const CONFIRM_POLLS: u8 = 2;

/// §7's node severity, but sticky: once amber or red, a node stays there until it is clearly
/// below the line, not just a hair under it.
fn sticky(value: Option<f64>, before: Severity, hysteresis: f64) -> Severity {
    let now = severity::node(value);
    if now >= before {
        return now;
    }
    let threshold = match before {
        Severity::Crit => 90.0,
        Severity::Warn => 75.0,
        _ => return now,
    };
    match value {
        Some(v) if v > threshold - hysteresis => before,
        _ => now,
    }
}

/// The reported state moves to `candidate` only once it has been seen `CONFIRM_POLLS` times
/// in a row.
fn settle(reported: Severity, candidate: Severity, pending: &mut Option<(Severity, u8)>) -> Severity {
    if candidate == reported {
        *pending = None;
        return reported;
    }
    let seen = match pending {
        Some((sev, n)) if *sev == candidate => {
            *n += 1;
            *n
        }
        _ => {
            *pending = Some((candidate, 1));
            1
        }
    };
    if seen >= CONFIRM_POLLS {
        *pending = None;
        candidate
    } else {
        reported
    }
}

/// A query's end is news only when the query was: a runaway (§5.4), or one close to its
/// memory limit. A fleet ends hundreds of ordinary queries a minute; listing them would bury
/// the three lines that matter.
const ENDED_NEAR_LIMIT: f64 = crate::model::RUNAWAY_MEMORY_FRACTION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fleet,
    Node,
    Query,
    Queue,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Fleet => "FLEET",
            Kind::Node => "NODE",
            Kind::Query => "QUERY",
            Kind::Queue => "QUEUE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    /// Seconds since the epoch, from the snapshot that showed the change.
    pub at: f64,
    pub level: Severity,
    pub kind: Kind,
    pub parts: Vec<(String, Tone)>,
    pub subject: Option<Subject>,
}

impl Event {
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.parts.iter().map(|(t, _)| t.as_str()).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Tape {
    events: VecDeque<Event>,
}

impl Tape {
    pub fn extend(&mut self, events: impl IntoIterator<Item = Event>) {
        for event in events {
            self.events.push_back(event);
            while self.events.len() > CAPACITY {
                self.events.pop_front();
            }
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Newest first: the tape reads like a log tailed backwards.
    pub fn newest_first(&self) -> impl Iterator<Item = &Event> {
        self.events.iter().rev()
    }

    pub fn get_newest(&self, index: usize) -> Option<&Event> {
        self.events.iter().rev().nth(index)
    }

    /// How many events of each problem level, for the view's title.
    pub fn counts(&self) -> (usize, usize) {
        let crit = self.events.iter().filter(|e| e.level == Severity::Crit).count();
        let warn = self.events.iter().filter(|e| e.level == Severity::Warn).count();
        (crit, warn)
    }
}

#[derive(Debug, Clone)]
struct NodeState {
    reachable: bool,
    /// The severities the tape has reported, which lag the raw ones by design.
    mem: Severity,
    cpu: Severity,
    lag: Severity,
    pending_mem: Option<(Severity, u8)>,
    pending_cpu: Option<(Severity, u8)>,
}

#[derive(Debug, Clone)]
struct QueryState {
    who: String,
    user: String,
    person: Option<String>,
    summary: String,
    elapsed_s: f64,
    peak_mem: u64,
    runaway: bool,
    /// Memory as a share of its own limit when last seen.
    limit_fraction: f64,
    limit: u64,
}

#[derive(Debug, Clone)]
struct QueueRowState {
    level: Severity,
    saturated: bool,
}

/// The last state of everything the tape reports on.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    started: bool,
    nodes: HashMap<String, NodeState>,
    queries: HashMap<(String, String), QueryState>,
    queue_reachable: Option<bool>,
    queues: HashMap<String, QueueRowState>,
}

struct Text(Vec<(String, Tone)>);

impl Text {
    fn new() -> Self {
        Text(Vec::new())
    }
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
    fn muted(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Muted)
    }
    fn strong(self, text: impl Into<String>) -> Self {
        self.add(text, Tone::Strong)
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

fn who(slice: &UserSlice<'_>) -> String {
    match &slice.person {
        Some(person) => crate::attrib::display_person(person),
        None => slice.user.clone(),
    }
}

fn node_subject(name: &str) -> Option<Subject> {
    Some(Subject::Node(name.to_string()))
}

impl Watch {
    /// Compare a new fleet view with what was seen last time.
    pub fn observe_fleet(
        &mut self,
        view: &FleetView<'_>,
        new_nodes: &HashSet<String>,
        at: f64,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        let first = !self.started;
        self.started = true;
        let event = |level, kind, text: Text, subject| Event {
            at,
            level,
            kind,
            parts: text.0,
            subject,
        };

        if first {
            let reachable = view.nodes.iter().filter(|n| n.node.reachable).count();
            out.push(event(
                Severity::Info,
                Kind::Fleet,
                Text::new()
                    .plain("watching ")
                    .strong(fmt::plural(view.nodes.len(), "node", "nodes"))
                    .muted(format!(" ({reachable} answering)")),
                None,
            ));
        }

        let mut seen_nodes: HashSet<String> = HashSet::new();
        for node in &view.nodes {
            let name = node.node.name.clone();
            seen_nodes.insert(name.clone());
            let now = match self.nodes.get(&name) {
                // First sight: no history to debounce against.
                None => NodeState {
                    reachable: node.node.reachable,
                    mem: severity::node(node.mem_pct),
                    cpu: severity::node(node.cpu_pct),
                    lag: severity::lag(node.node.lag_s),
                    pending_mem: None,
                    pending_cpu: None,
                },
                Some(before) => {
                    let mut pending_mem = before.pending_mem;
                    let mut pending_cpu = before.pending_cpu;
                    let (mem, cpu) = if node.node.reachable {
                        (
                            settle(before.mem, sticky(node.mem_pct, before.mem, HYSTERESIS_MEM), &mut pending_mem),
                            settle(before.cpu, sticky(node.cpu_pct, before.cpu, HYSTERESIS_CPU), &mut pending_cpu),
                        )
                    } else {
                        (before.mem, before.cpu)
                    };
                    NodeState {
                        reachable: node.node.reachable,
                        mem,
                        cpu,
                        lag: severity::lag(node.node.lag_s),
                        pending_mem,
                        pending_cpu,
                    }
                }
            };
            match self.nodes.get(&name) {
                None if !first => {
                    let fresh = new_nodes.contains(&name);
                    out.push(event(
                        Severity::Info,
                        Kind::Node,
                        Text::new()
                            .node(&name)
                            .plain(if fresh { " joined the fleet" } else { " is back in the fleet" }),
                        node_subject(&name),
                    ));
                }
                None => {}
                Some(before) => out.extend(node_changes(before, &now, node, at)),
            }
            // A problem that is already there when the session starts goes on the tape too:
            // otherwise the tape opens empty on the worst night of the year.
            if first {
                for (sev, what, value) in [
                    (now.mem, "memory", node.mem_pct),
                    (now.cpu, "CPU", node.cpu_pct),
                ] {
                    if sev.is_problem() {
                        out.push(event(
                            sev,
                            Kind::Node,
                            Text::new()
                                .node(&name)
                                .plain(format!(" {what} "))
                                .sev(fmt::pct(value), sev)
                                .muted(" at start"),
                            node_subject(&name),
                        ));
                    }
                }
                if !now.reachable {
                    out.push(event(
                        Severity::Crit,
                        Kind::Node,
                        Text::new().node(&name).plain(" ").sev(node.node.down_word(), Severity::Crit).muted(" at start"),
                        node_subject(&name),
                    ));
                }
            }
            self.nodes.insert(name, now);
        }
        let gone: Vec<String> = self
            .nodes
            .keys()
            .filter(|name| !seen_nodes.contains(*name))
            .cloned()
            .collect();
        for name in gone {
            self.nodes.remove(&name);
            out.push(event(
                Severity::Info,
                Kind::Node,
                Text::new().node(&name).plain(" left the fleet").muted(" (unreachable for 5 minutes, not in system.clusters)"),
                None,
            ));
        }

        // Queries: runaways as they happen, and an "ended" line for the ones worth one.
        let mut seen_queries: HashSet<(String, String)> = HashSet::new();
        for node in &view.nodes {
            if !node.node.reachable {
                // Its queries are unknown, not ended: keep their state until it answers.
                for key in self.queries.keys().filter(|(n, _)| n == &node.node.name) {
                    seen_queries.insert(key.clone());
                }
                continue;
            }
            for slice in &node.users {
                for stat in &slice.queries {
                    let key = (node.node.name.clone(), stat.query.query_id.clone());
                    seen_queries.insert(key.clone());
                    let state = query_state(slice, stat, self.queries.get(&key));
                    let before = self.queries.get(&key);
                    out.extend(query_changes(before, &state, node, stat, at));
                    self.queries.insert(key, state);
                }
            }
        }
        let ended: Vec<(String, String)> = self
            .queries
            .keys()
            .filter(|k| !seen_queries.contains(*k))
            .cloned()
            .collect();
        for key in ended {
            let Some(state) = self.queries.remove(&key) else {
                continue;
            };
            if !seen_nodes.contains(&key.0) {
                continue; // its node left; the node line already says so
            }
            if let Some(e) = ended_event(&key.0, &key.1, &state, at) {
                out.push(e);
            }
        }
        out
    }

    /// Compare a new queue status with the last one.
    pub fn observe_queue(&mut self, queue: &QueueStatus, at: f64) -> Vec<Event> {
        let mut out = Vec::new();
        // "Not polled yet" and "not configured" are states, not news.
        if queue.is_placeholder() {
            return out;
        }
        let event = |level, text: Text| Event {
            at,
            level,
            kind: Kind::Queue,
            parts: text.0,
            subject: Some(Subject::Queue),
        };
        match (self.queue_reachable, queue.reachable) {
            (Some(true) | None, false) => out.push(event(
                Severity::Warn,
                Text::new()
                    .strong("Redash queue")
                    .plain(" unreachable")
                    .muted(format!(" ({})", queue.error.as_deref().unwrap_or("no answer"))),
            )),
            (Some(false), true) => out.push(event(
                Severity::Ok,
                Text::new().strong("Redash queue").plain(" answering again"),
            )),
            _ => {}
        }
        self.queue_reachable = Some(queue.reachable);
        if !queue.reachable {
            return out;
        }

        for row in &queue.queues {
            let saturated = row.saturated();
            let level = severity::wait(row.oldest_wait_s.unwrap_or(0), false);
            let before = self.queues.get(&row.name).cloned();
            if let Some(before) = &before {
                if level > before.level && level.is_problem() {
                    out.push(event(
                        level,
                        Text::new()
                            .strong(format!("Redash {}", row.name))
                            .plain(" backed up: ")
                            .sev(format!("{} waiting", row.waiting), level)
                            .plain(", oldest ")
                            .sev(fmt::opt_dur(row.oldest_wait_s), level),
                    ));
                } else if level < before.level && !level.is_problem() {
                    out.push(event(
                        Severity::Ok,
                        Text::new()
                            .strong(format!("Redash {}", row.name))
                            .plain(" drained")
                            .muted(format!(" ({} waiting)", row.waiting)),
                    ));
                }
                if saturated && !before.saturated {
                    out.push(event(
                        Severity::Warn,
                        Text::new()
                            .strong(format!("Redash {}", row.name))
                            .plain(format!(": all {} workers busy", row.workers_total)),
                    ));
                } else if !saturated && before.saturated {
                    out.push(event(
                        Severity::Ok,
                        Text::new()
                            .strong(format!("Redash {}", row.name))
                            .plain(format!(": workers free again ({}/{} busy)", row.workers_busy, row.workers_total)),
                    ));
                }
            } else if level.is_problem() || saturated {
                // First sight of this queue already in trouble.
                out.push(event(
                    level.max(Severity::Warn),
                    Text::new()
                        .strong(format!("Redash {}", row.name))
                        .plain(": ")
                        .sev(format!("{} waiting", row.waiting), level.max(Severity::Warn))
                        .plain(format!(
                            ", oldest {}, workers {}/{} busy",
                            fmt::opt_dur(row.oldest_wait_s),
                            row.workers_busy,
                            row.workers_total
                        ))
                        .muted(" at start"),
                ));
            }
            self.queues.insert(row.name.clone(), QueueRowState { level, saturated });
        }
        out
    }
}

fn node_changes(before: &NodeState, now: &NodeState, node: &NodeView<'_>, at: f64) -> Vec<Event> {
    let name = &node.node.name;
    let mut out = Vec::new();
    let event = |level, text: Text| Event {
        at,
        level,
        kind: Kind::Node,
        parts: text.0,
        subject: node_subject(name),
    };
    match (before.reachable, now.reachable) {
        (true, false) => {
            out.push(event(
                Severity::Crit,
                Text::new()
                    .node(name)
                    .plain(" ")
                    .sev(node.node.down_word(), Severity::Crit)
                    .muted(match &node.node.unreachable_reason {
                        Some(reason) => format!(" — {reason}"),
                        None => String::new(),
                    }),
            ));
            return out;
        }
        (false, true) => out.push(event(Severity::Ok, Text::new().node(name).plain(" answering again"))),
        (false, false) => return out,
        (true, true) => {}
    }
    // Severity changes only between two answers: a node coming back is not "cooling down".
    if before.reachable {
        for (was, is, what, value) in [
            (before.mem, now.mem, "memory", node.mem_pct),
            (before.cpu, now.cpu, "CPU", node.cpu_pct),
        ] {
            if is > was && is.is_problem() {
                let threshold = if is == Severity::Crit { "≥ 90%" } else { "≥ 75%" };
                out.push(event(
                    is,
                    Text::new()
                        .node(name)
                        .plain(format!(" {what} "))
                        .sev(fmt::pct(value), is)
                        .muted(format!(" ({threshold})")),
                ));
            } else if is < was && !is.is_problem() {
                out.push(event(
                    Severity::Ok,
                    Text::new()
                        .node(name)
                        .plain(format!(" {what} back to "))
                        .strong(fmt::pct(value)),
                ));
            }
        }
        if now.lag > before.lag && now.lag.is_problem() {
            out.push(event(
                now.lag,
                Text::new()
                    .node(name)
                    .plain(" replica lag ")
                    .sev(fmt::dur(node.node.lag_s as f64), now.lag),
            ));
        } else if now.lag < before.lag && !now.lag.is_problem() {
            out.push(event(
                Severity::Ok,
                Text::new().node(name).plain(" replica caught up").muted(format!(" (lag {})", fmt::dur(node.node.lag_s as f64))),
            ));
        }
    }
    out
}

fn query_state(slice: &UserSlice<'_>, stat: &QueryStat<'_>, before: Option<&QueryState>) -> QueryState {
    let q = stat.query;
    QueryState {
        who: who(slice),
        user: slice.user.clone(),
        person: slice.person.clone(),
        summary: crate::sqltext::summary(&q.sql).label(),
        elapsed_s: q.elapsed_s,
        peak_mem: before
            .map(|b| b.peak_mem)
            .unwrap_or(0)
            .max(q.memory_bytes)
            .max(q.peak_memory_bytes),
        runaway: stat.runaway,
        limit_fraction: stat.limit_fraction(),
        limit: stat.limit,
    }
}

fn query_changes(
    before: Option<&QueryState>,
    now: &QueryState,
    node: &NodeView<'_>,
    stat: &QueryStat<'_>,
    at: f64,
) -> Vec<Event> {
    let mut out = Vec::new();
    let subject = Some(Subject::Query {
        node: node.node.name.clone(),
        user: now.user.clone(),
        person: now.person.clone(),
        query_id: stat.query.query_id.clone(),
    });
    let was_runaway = before.is_some_and(|b| b.runaway);
    if now.runaway && !was_runaway {
        let why = if stat.query.elapsed_s >= crate::model::RUNAWAY_ELAPSED_S {
            format!("{} running", fmt::dur(stat.query.elapsed_s))
        } else {
            format!("{} of its {} limit", fmt::pct0(now.limit_fraction * 100.0), fmt::bytes(now.limit))
        };
        out.push(Event {
            at,
            level: Severity::Warn,
            kind: Kind::Query,
            parts: Text::new()
                .sev("runaway ", Severity::Crit)
                .person(&now.who)
                .plain(" on ")
                .node(&node.node.name)
                .plain(" — ")
                .strong(why)
                .plain(if stat.query.memory_bytes >= 1024 * 1024 {
                    format!(" · {}", fmt::bytes(stat.query.memory_bytes))
                } else {
                    String::new()
                })
                .muted(format!(" · {}", now.summary))
                .0,
            subject: subject.clone(),
        });
    }
    let was = before.map_or(Severity::None, |b| severity::limit_share(b.limit_fraction));
    let is = severity::limit_share(now.limit_fraction);
    if is == Severity::Crit && was < Severity::Crit {
        out.push(Event {
            at,
            level: Severity::Crit,
            kind: Kind::Query,
            parts: Text::new()
                .person(&now.who)
                .plain("'s query on ")
                .node(&node.node.name)
                .plain(" at ")
                .sev(fmt::pct0(now.limit_fraction * 100.0), Severity::Crit)
                .plain(format!(" of its {} memory limit", fmt::bytes(now.limit)))
                .0,
            subject,
        });
    }
    out
}

fn ended_event(node: &str, query_id: &str, state: &QueryState, at: f64) -> Option<Event> {
    let near_limit = state.limit_fraction >= 0.95;
    if !state.runaway && state.limit_fraction < ENDED_NEAR_LIMIT {
        return None;
    }
    let mut text = Text::new()
        .plain("ended ")
        .person(&state.who)
        .plain(" on ")
        .node(node)
        .plain(" after ")
        .strong(format!("≥ {}", fmt::dur(state.elapsed_s)))
        .plain(format!(" · peak {}", fmt::bytes(state.peak_mem)));
    let level = if near_limit {
        // Gone while at its limit: the likeliest ending is ClickHouse killing it.
        text = text.sev(
            format!(" · probably killed (was at {} of its limit)", fmt::pct0(state.limit_fraction * 100.0)),
            Severity::Warn,
        );
        Severity::Warn
    } else {
        Severity::Info
    };
    text = text.muted(format!(" · {} · {}", state.summary, crate::fmt::truncate(query_id, 8)));
    Some(Event {
        at,
        level,
        kind: Kind::Query,
        parts: text.0,
        subject: Some(Subject::Node(node.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use crate::model::{fleet_view, NodeSnapshot};

    #[test]
    fn the_first_poll_opens_the_tape_with_what_is_already_wrong() {
        let mut fake = FakeSource::new();
        let snap = fake.snapshot();
        let view = fleet_view(&snap, None);
        let mut watch = Watch::default();
        let events = watch.observe_fleet(&view, &HashSet::new(), 1.0);
        assert!(events[0].text().starts_with("watching 8 nodes"), "{}", events[0].text());
        assert!(events.iter().any(|e| e.text().starts_with("clickhouse3 memory") && e.level == Severity::Crit));
        assert!(events.iter().any(|e| e.text().starts_with("runaway grigol.gankava on clickhouse3")));
    }

    #[test]
    fn a_node_going_away_and_coming_back_is_two_events() {
        let mut fake = FakeSource::new();
        let snap = fake.snapshot();
        let mut watch = Watch::default();
        watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 1.0);

        let mut broken = snap.clone();
        for node in &mut broken.nodes {
            if node.name == "ch4" {
                *node = NodeSnapshot::unreachable("ch4", "connection refused");
            }
        }
        let events = watch.observe_fleet(&fleet_view(&broken, None), &HashSet::new(), 3.0);
        let down = events.iter().find(|e| e.text().starts_with("ch4")).expect("ch4 event");
        assert_eq!(down.level, Severity::Crit);
        assert!(down.text().contains("unreachable — connection refused"));

        let events = watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 5.0);
        let up = events.iter().find(|e| e.text().starts_with("ch4")).expect("ch4 event");
        assert_eq!(up.level, Severity::Ok);
        assert!(up.text().contains("answering again"));
    }

    #[test]
    fn a_query_that_vanishes_at_its_limit_was_probably_killed() {
        let mut fake = FakeSource::new();
        let mut snap = fake.snapshot();
        let ch3 = snap.nodes.iter_mut().find(|n| n.name == "clickhouse3").unwrap();
        ch3.queries[0].memory_limit = Some(ch3.queries[0].memory_bytes + 1);
        let victim = ch3.queries[0].query_id.clone();
        let mut watch = Watch::default();
        watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 1.0);

        for node in &mut snap.nodes {
            node.queries.retain(|q| q.query_id != victim);
        }
        let events = watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 3.0);
        let ended = events
            .iter()
            .find(|e| e.text().starts_with("ended"))
            .expect("an ended line");
        assert!(ended.text().contains("probably killed"), "{}", ended.text());
        assert_eq!(ended.level, Severity::Warn);
    }

    #[test]
    fn a_short_query_ending_is_not_news() {
        let mut fake = FakeSource::new();
        let mut snap = fake.snapshot();
        let mut watch = Watch::default();
        watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 1.0);
        // The dba's routine query on ch4 and haris's EXPLAIN on clickhouse3 end; neither was a
        // runaway or near its limit, so neither is news.
        for node in &mut snap.nodes {
            if node.name == "ch4" {
                node.queries.clear();
            }
            if node.name == "clickhouse3" {
                node.queries.retain(|q| q.user != "haris");
            }
        }
        let events = watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 3.0);
        assert!(
            !events.iter().any(|e| e.text().contains("on ch4") || e.text().contains("haris")),
            "{:?}",
            events.iter().map(Event::text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_node_hovering_at_a_threshold_does_not_flap() {
        let h = HYSTERESIS_MEM;
        assert_eq!(sticky(Some(74.0), Severity::Warn, h), Severity::Warn, "a hair under 75 is still amber");
        assert_eq!(sticky(Some(71.9), Severity::Warn, h), Severity::None, "clearly under it is not");
        assert_eq!(sticky(Some(88.0), Severity::Crit, h), Severity::Crit);
        assert_eq!(sticky(Some(86.0), Severity::Crit, h), Severity::Warn);
        assert_eq!(sticky(Some(91.0), Severity::Warn, h), Severity::Crit, "going up is not sticky");
        assert_eq!(sticky(None, Severity::Warn, h), Severity::None);
    }

    #[test]
    fn a_one_poll_spike_is_not_an_event() {
        let mut pending = None;
        // 97% once, then back: nothing is reported.
        assert_eq!(settle(Severity::None, Severity::Crit, &mut pending), Severity::None);
        assert_eq!(settle(Severity::None, Severity::None, &mut pending), Severity::None);
        assert_eq!(pending, None);
        // Twice in a row is a state.
        assert_eq!(settle(Severity::None, Severity::Crit, &mut pending), Severity::None);
        assert_eq!(settle(Severity::None, Severity::Crit, &mut pending), Severity::Crit);
    }

    #[test]
    fn a_spiky_cpu_writes_one_line_not_ten() {
        let mut fake = FakeSource::new();
        let snap = fake.snapshot();
        let mut watch = Watch::default();
        watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 0.0);
        // ch4's CPU jumps between 20% and 95% on every poll.
        let mut events = Vec::new();
        for i in 0..10u32 {
            let mut s = snap.clone();
            for node in &mut s.nodes {
                if node.name == "ch4" {
                    node.cpu_busy_cores = Some(if i % 2 == 0 { 15.2 } else { 3.2 });
                }
            }
            events.extend(watch.observe_fleet(&fleet_view(&s, None), &HashSet::new(), f64::from(i)));
        }
        assert!(
            !events.iter().any(|e| e.text().starts_with("ch4")),
            "{:?}",
            events.iter().map(Event::text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_runaway_ending_is_news() {
        let mut fake = FakeSource::new();
        let mut snap = fake.snapshot();
        let mut watch = Watch::default();
        watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 1.0);
        // grigol's 4-minute AML query on clickhouse3 finishes.
        for node in &mut snap.nodes {
            node.queries.retain(|q| q.person.as_deref() != Some("grigol.gankava"));
        }
        let events = watch.observe_fleet(&fleet_view(&snap, None), &HashSet::new(), 3.0);
        let ended = events
            .iter()
            .find(|e| e.text().starts_with("ended grigol.gankava on clickhouse3"))
            .expect("the runaway's end is on the tape");
        assert_eq!(ended.level, Severity::Info, "not near its limit, so not a kill");
        assert!(ended.text().contains("SELECT · accounting_lt.bank_record"), "{}", ended.text());
    }

    #[test]
    fn the_queue_backing_up_and_draining_is_on_the_tape() {
        let mut fake = FakeSource::new();
        let mut queue = fake.queue();
        let mut watch = Watch::default();
        let start = watch.observe_queue(&queue, 1.0);
        assert!(start.iter().any(|e| e.text().contains("at start")), "already backed up at start");

        // Drain it.
        for row in &mut queue.queues {
            row.waiting = 0;
            row.oldest_wait_s = Some(1);
            row.workers_busy = 0;
        }
        let drained = watch.observe_queue(&queue, 4.0);
        assert!(drained.iter().any(|e| e.text().contains("drained") && e.level == Severity::Ok));
        assert!(drained.iter().any(|e| e.text().contains("workers free again")));

        let gone = watch.observe_queue(&QueueStatus::unreachable("HTTP 401"), 7.0);
        assert!(gone[0].text().contains("unreachable (HTTP 401)"));
        assert!(watch.observe_queue(&QueueStatus::unreachable("not polled yet"), 8.0).is_empty());
    }

    #[test]
    fn the_tape_keeps_the_newest_and_counts_problems() {
        let mut tape = Tape::default();
        let make = |i: usize, level| Event {
            at: i as f64,
            level,
            kind: Kind::Fleet,
            parts: vec![(format!("e{i}"), Tone::Plain)],
            subject: None,
        };
        tape.extend((0..CAPACITY + 5).map(|i| make(i, Severity::Info)));
        assert_eq!(tape.len(), CAPACITY);
        assert_eq!(tape.newest_first().next().unwrap().text(), format!("e{}", CAPACITY + 4));
        tape.extend([make(9999, Severity::Crit), make(10000, Severity::Warn)]);
        assert_eq!(tape.counts(), (1, 1));
        assert_eq!(tape.get_newest(0).unwrap().text(), "e10000");
    }
}
