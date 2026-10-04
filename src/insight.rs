//! Insights: what needs looking at, in as few words as will do.
//!
//! One short line per thing in trouble — a node, a Redash queue, a Redash query running twice —
//! worst first, each pointing at the row it is about (`tab`, then `⏎`). A node with several
//! problems is still one line: the worst of them, and `+N more` for the rest. The numbers behind
//! a line, and everything else found about its subject, are in the drawer when the line is
//! chosen. What is merely interesting — the heaviest user, a slow poll, a node that joined — is
//! on screen elsewhere and is not repeated here. When nothing is wrong, one line says so.
//!
//! Pure: a fleet view, its history and the queue in, lines out. Every claim is a §5 number or a
//! slope over the history (`history.rs`); nothing is guessed.

use crate::fmt;
use crate::history::{eta_to, History};
use crate::model::{FleetView, JobState, NodeView, QueryStat, QueueStatus, UserSlice};
use crate::severity::{self, Severity};
use std::collections::BTreeMap;

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

/// A run of text in its tones.
pub type Phrase = Vec<(String, Tone)>;

#[derive(Debug, Clone)]
pub struct Insight {
    pub level: Severity,
    /// Within a level, higher first.
    pub score: f64,
    pub subject: Subject,
    /// What the line is about — a node, `Redash queries`, `Redash #8585` — drawn in a column
    /// of its own, so the lines read as a table.
    pub label: String,
    /// The line itself: the one thing to know.
    pub parts: Phrase,
    /// For the drawer when the line is chosen: the numbers behind it, and whatever else was
    /// found about the same subject.
    pub details: Vec<Phrase>,
}

impl Insight {
    /// The line as plain text, label first.
    #[cfg(test)]
    pub fn text(&self) -> String {
        format!("{}  {}", self.label, plain(&self.parts))
    }

    /// The drawer's lines as plain text.
    #[cfg(test)]
    pub fn detail_text(&self) -> Vec<String> {
        self.details.iter().map(|d| plain(d)).collect()
    }
}

#[cfg(test)]
fn plain(parts: &[(String, Tone)]) -> String {
    parts.iter().map(|(t, _)| t.as_str()).collect()
}

/// A phrase being written.
#[derive(Default, Clone)]
struct Text(Phrase);

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
/// CPU is judged over this window: a real node's CPU jumps twenty points from one poll to the
/// next, and a line that comes and goes every two seconds is noise.
const CPU_STEADY_S: f64 = 10.0;
/// A kill further out than this is not a forecast worth printing.
pub const KILL_HORIZON_S: f64 = 30.0 * 60.0;

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

fn query_subject(node: &NodeView<'_>, slice: &UserSlice<'_>, stat: &QueryStat<'_>) -> Subject {
    Subject::Query {
        node: node.node.name.clone(),
        user: slice.user.clone(),
        person: slice.person.clone(),
        query_id: stat.query.query_id.clone(),
    }
}

/// Everything worth saying about the fleet right now, worst first.
pub fn analyze(view: &FleetView<'_>, history: &History, queue: &QueueStatus) -> Vec<Insight> {
    let now = history.now();
    let names: Vec<&str> = view.nodes.iter().map(|n| n.node.name.as_str()).collect();
    let domain = shared_domain(&names);
    let mut out: Vec<Insight> = view
        .nodes
        .iter()
        .filter_map(|node| node_line(node, history, now))
        .map(|mut line| {
            if let Some(short) = domain.as_deref().and_then(|d| line.label.strip_suffix(d)) {
                line.label = short.to_string();
            }
            line
        })
        .collect();
    out.extend(queue_line(queue, view, history));
    out.extend(duplicate_lines(view));
    if out.is_empty() {
        out.push(all_clear(view, queue));
    }
    out.sort_by(|a, b| {
        b.level
            .cmp(&a.level)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.label.cmp(&b.label))
    });
    out
}

/// The domain every node's name ends in (`.example.net`): a label can do without it, the tree
/// above still has the whole name. None when any name is an address or has no domain.
fn shared_domain(names: &[&str]) -> Option<String> {
    let looks_like_address = |n: &str| n.contains(':') || n.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok();
    if names.iter().any(|n| looks_like_address(n) || !n.contains('.')) {
        return None;
    }
    let first = names.first()?;
    let mut suffix = &first[first.find('.')?..];
    loop {
        if names.iter().all(|n| n.len() > suffix.len() && n.ends_with(suffix)) {
            return Some(suffix.to_string());
        }
        let next = suffix[1..].find('.')?;
        suffix = &suffix[1 + next..];
    }
}

/// The worst level among the insights: what the status pill in the header says.
pub fn overall(insights: &[Insight]) -> Severity {
    insights
        .iter()
        .map(|i| i.level)
        .max()
        .unwrap_or(Severity::None)
}

// -- one line per node ----------------------------------------------------------------------

/// Which finding leads a node's line when two are equally severe — in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Down,
    Memory,
    Limit,
    Cpu,
    Long,
    Lag,
}

impl Kind {
    /// Where a line of this kind sorts among lines of the same level.
    fn base(self) -> f64 {
        match self {
            Kind::Down => 900.0,
            Kind::Memory => 800.0,
            Kind::Limit => 700.0,
            Kind::Cpu => 600.0,
            Kind::Long => 400.0,
            Kind::Lag => 300.0,
        }
    }
}

/// Lines from other places sort between these.
const QUEUE_BASE: f64 = 650.0;
const DUPLICATE_BASE: f64 = 500.0;

/// Something found about a node, before the node's findings become its one line.
struct Finding {
    level: Severity,
    kind: Kind,
    /// Within its kind, more pressing first (0–99).
    weight: f64,
    subject: Subject,
    /// For the line, when this finding leads it.
    brief: Text,
    /// For the drawer.
    detail: Vec<Text>,
}

fn node_line(node: &NodeView<'_>, history: &History, now: f64) -> Option<Insight> {
    let mut found: Vec<Finding> = Vec::new();
    if node.node.reachable {
        found.extend(resources(node, history));
        let near = near_limit(node, history);
        // A query near its limit is said once, as that: not again among the long ones.
        let named: Vec<String> = near
            .iter()
            .filter_map(|f| match &f.subject {
                Subject::Query { query_id, .. } => Some(query_id.clone()),
                _ => None,
            })
            .collect();
        found.extend(near);
        found.extend(long_queries(node, &named));
        found.extend(lag(node));
    } else {
        found.push(down(node, history, now));
    }
    found.retain(|f| f.level.is_problem());
    found.sort_by(|a, b| {
        b.level
            .cmp(&a.level)
            .then(a.kind.cmp(&b.kind))
            .then(b.weight.total_cmp(&a.weight))
    });
    let lead = found.first()?;
    let mut parts = lead.brief.clone();
    if found.len() > 1 {
        parts = parts.muted(format!("  +{} more", found.len() - 1));
    }
    Some(Insight {
        level: lead.level,
        score: lead.kind.base() + lead.weight.clamp(0.0, 99.0),
        subject: lead.subject.clone(),
        label: node.node.name.clone(),
        parts: parts.0,
        details: found.iter().flat_map(|f| f.detail.iter().map(|t| t.0.clone())).collect(),
    })
}

/// A node without numbers: why, in a word and a few more.
fn down(node: &NodeView<'_>, history: &History, now: f64) -> Finding {
    let word = node.node.down_word();
    let detail = node.node.down_detail();
    let since = history
        .node(&node.node.name)
        .and_then(|h| h.unreachable_since)
        .map(|t| now - t)
        .filter(|s| *s >= 1.0);

    let mut brief = Text::default().sev(word, Severity::Crit);
    if let Some(detail) = detail {
        brief = brief.muted(format!(" · {}", short_reason(word, detail)));
    }
    let mut first = Text::default().sev(word, Severity::Crit);
    if let Some(for_s) = since {
        first = first.plain(format!(" for {}", fmt::dur(for_s)));
    }
    if let Some(detail) = detail {
        first = first.plain(format!(": {detail}"));
    }
    let mut lines = vec![first];
    if let Some(hint) = down_hint(word, detail.unwrap_or_default()) {
        lines.push(Text::default().muted(hint));
    }
    Finding {
        level: Severity::Crit,
        kind: Kind::Down,
        weight: since.unwrap_or(0.0) / 60.0,
        subject: Subject::Node(node.node.name.clone()),
        brief,
        detail: lines,
    }
}

/// The part of a reason that fits on the line; the drawer has the rest. Transport reasons
/// explain themselves after ` — `, the classed ones (`no access`, …) after `: `.
fn short_reason<'a>(word: &str, detail: &'a str) -> &'a str {
    let cut = if word == "unreachable" { detail.find(" — ") } else { detail.find(": ") };
    cut.map_or(detail, |at| &detail[..at])
}

fn down_hint(word: &str, detail: &str) -> Option<&'static str> {
    match word {
        crate::model::NO_ACCESS => Some("the monitor reads only system tables: SELECT on system.* is all it needs"),
        crate::model::LOGIN_REFUSED => Some("set this server's user and password in the credential file"),
        _ if detail.contains("DNS") => Some(
            "a name only the cluster knows? list the server in the credential file under a name or address that resolves here",
        ),
        _ => None,
    }
}

/// Memory climbing steadily enough to run out inside half an hour.
struct Forecast {
    per_min: f64,
    eta_s: f64,
    level: Severity,
}

fn memory_forecast(node: &NodeView<'_>, history: &History) -> Option<Forecast> {
    let pct = node.mem_pct?;
    let series = &history.node(&node.node.name)?.mem_pct;
    if series.span_s() < crate::history::FORECAST_MIN_SPAN_S || pct < 50.0 {
        return None;
    }
    let slope = series.slope(FORECAST_WINDOW_S)?;
    let per_min = slope * 60.0;
    if per_min < FORECAST_MIN_SLOPE_PER_MIN {
        return None;
    }
    let eta_s = eta_to(pct, slope, 100.0).filter(|e| *e <= FORECAST_HORIZON_S)?;
    let level = if eta_s <= 5.0 * 60.0 { Severity::Crit } else { Severity::Warn };
    Some(Forecast { per_min, eta_s, level })
}

/// Who holds a share of one resource on a node, biggest first: the user rows and the server's
/// own closing row (§5.3).
fn holders(
    node: &NodeView<'_>,
    share: impl Fn(&UserSlice<'_>) -> Option<f64>,
    server: Option<f64>,
) -> Vec<(Option<String>, f64)> {
    let mut all: Vec<(Option<String>, f64)> = node
        .users
        .iter()
        .filter_map(|u| share(u).map(|s| (Some(who_slice(u)), s)))
        .collect();
    if let Some(server) = server {
        all.push((None, server));
    }
    all.retain(|(_, pct)| *pct >= 0.5);
    all.sort_by(|a, b| b.1.total_cmp(&a.1));
    all
}

/// `grigol.gankava holds 47%`, or `the server itself holds 60%` — the most useful thing a hot
/// node's line can say is whether it is a person or the server.
fn holds(text: Text, holder: &(Option<String>, f64)) -> Text {
    let text = match &holder.0 {
        Some(person) => text.person(person.clone()),
        None => text.strong("the server itself"),
    };
    text.plain(format!(" holds {}", fmt::pct0(holder.1)))
}

fn held_by(resource: &str, holders: &[(Option<String>, f64)]) -> Text {
    let mut text = Text::default().muted(format!("{resource} held by "));
    for (i, (who, pct)) in holders.iter().take(3).enumerate() {
        if i > 0 {
            text = text.muted(" · ");
        }
        text = match who {
            Some(person) => text.person(person.clone()),
            None => text.strong("the server itself"),
        };
        text = text.plain(format!(" {}", fmt::pct0(*pct)));
    }
    text
}

/// Hot memory or CPU, and who holds it.
fn resources(node: &NodeView<'_>, history: &History) -> Option<Finding> {
    let steady_cpu = history
        .node(&node.node.name)
        .and_then(|h| h.cpu_pct.mean_in(CPU_STEADY_S))
        .or(node.cpu_pct);
    let cpu_sev = severity::node(node.cpu_pct.and(steady_cpu));
    let forecast = memory_forecast(node, history);
    let mem_sev = severity::node(node.mem_pct).max(forecast.as_ref().map_or(Severity::None, |f| f.level));
    let level = mem_sev.max(cpu_sev);
    if !level.is_problem() {
        return None;
    }
    let mem_holders = holders(node, |u| u.mem_pct, node.server_mem_pct);
    let cpu_holders = holders(node, |u| u.cpu_pct, node.server_cpu_pct);

    let mut brief = Text::default();
    let mut detail = Vec::new();
    if mem_sev.is_problem() {
        brief = brief.plain("memory ").sev(fmt::pct(node.mem_pct), mem_sev);
        let mut line = Text::default().plain("memory ").sev(fmt::pct(node.mem_pct), mem_sev);
        if let Some(total) = node.node.mem_total {
            line = line.muted(format!(" ({} of {})", fmt::bytes(node.node.mem_used), fmt::bytes(total)));
        }
        if let Some(f) = &forecast {
            brief = brief.plain(" ↗ full in ").sev(fmt::eta(f.eta_s), f.level);
            line = line
                .plain(format!(" · +{:.1}%/min, full in ", f.per_min))
                .sev(fmt::eta(f.eta_s), f.level)
                .muted(" at this rate");
        }
        detail.push(line);
        if !mem_holders.is_empty() {
            detail.push(held_by("memory", &mem_holders));
        }
        if let Some(line) = headroom(node) {
            detail.push(line);
        }
    }
    if cpu_sev.is_problem() {
        brief = brief
            .plain(if mem_sev.is_problem() { " · CPU " } else { "CPU " })
            .sev(fmt::pct(node.cpu_pct), cpu_sev);
        let mut line = Text::default().plain("CPU ").sev(fmt::pct(node.cpu_pct), cpu_sev);
        if let (Some(busy), Some(cores)) = (node.busy_cores, node.node.cores) {
            line = line.muted(format!(" ({busy:.1} of {cores:.0} cores, steady over 10 s)"));
        }
        detail.push(line);
        if !cpu_holders.is_empty() {
            detail.push(held_by("CPU", &cpu_holders));
        }
    }
    // One name on the line: who holds most of the leading resource.
    let lead = if mem_sev.is_problem() { &mem_holders } else { &cpu_holders };
    if let Some(top) = lead.first() {
        brief = holds(brief.plain(" · "), top);
    }
    Some(Finding {
        level,
        kind: if mem_sev.is_problem() { Kind::Memory } else { Kind::Cpu },
        weight: node.pressure.max(node.cpu_pct.unwrap_or(0.0)),
        subject: Subject::Node(node.node.name.clone()),
        brief,
        detail,
    })
}

/// Less free memory than one more query may take: the next heavy dashboard can push the
/// server over.
fn headroom(node: &NodeView<'_>) -> Option<Text> {
    let total = node.node.mem_total?;
    let free = total.saturating_sub(node.node.mem_used);
    let limit = node
        .users
        .iter()
        .flat_map(|u| u.queries.iter())
        .map(|q| q.limit)
        .max()
        .unwrap_or(node.mem_limit);
    (free < limit).then(|| {
        Text::default()
            .plain("only ")
            .sev(fmt::bytes(free), Severity::Warn)
            .plain(format!(" free — less than one {} query may take", fmt::bytes(limit)))
    })
}

/// A query close to the memory limit its own profile sets: ClickHouse is about to kill it with
/// MEMORY_LIMIT_EXCEEDED, and whoever started it will want to know before the dashboard errors.
fn near_limit(node: &NodeView<'_>, history: &History) -> Vec<Finding> {
    let mut out = Vec::new();
    for slice in &node.users {
        for stat in &slice.queries {
            if !stat.limit_holds() {
                continue;
            }
            let fraction = stat.limit_fraction();
            let level = severity::limit_share(fraction);
            if !level.is_problem() {
                continue;
            }
            let query = stat.query;
            let who = who_slice(slice);
            let rate = history
                .query(&node.node.name, &query.query_id)
                .and_then(|h| h.mem_rate())
                .filter(|r| *r > 0.0);
            let eta = rate
                .and_then(|r| eta_to(query.memory_bytes as f64, r, stat.limit as f64))
                .filter(|e| *e <= KILL_HORIZON_S);

            let mut brief = Text::default()
                .person(who.clone())
                .plain("'s query at ")
                .sev(fmt::pct0(fraction * 100.0), level)
                .plain(" of its memory limit");
            let mut line = Text::default()
                .person(who)
                .plain(" · ")
                .strong(fmt::bytes(query.memory_bytes))
                .plain(format!(" of its {} limit", fmt::bytes(stat.limit)));
            if let Some(rate) = rate {
                line = line.plain(format!(" · +{}", fmt::rate(rate)));
            }
            if let Some(eta) = eta {
                brief = brief.plain(" · killed in ").sev(fmt::eta(eta), Severity::Crit);
                line = line.plain(" → killed in ").sev(fmt::eta(eta), Severity::Crit);
            }
            line = line.muted(format!(" · {}", crate::sqltext::summary(&query.sql).label()));
            out.push(Finding {
                level,
                kind: Kind::Limit,
                weight: fraction.min(1.0) * 99.0,
                subject: query_subject(node, slice, stat),
                brief,
                detail: vec![line],
            });
        }
    }
    out
}

/// Queries running past the runaway mark (§5.4): one finding per node, led by the longest.
fn long_queries(node: &NodeView<'_>, already: &[String]) -> Option<Finding> {
    let mut long: Vec<(&UserSlice<'_>, &QueryStat<'_>)> = node
        .users
        .iter()
        .flat_map(|slice| slice.queries.iter().map(move |stat| (slice, stat)))
        .filter(|(_, stat)| {
            stat.query.elapsed_s >= crate::model::RUNAWAY_ELAPSED_S && !already.contains(&stat.query.query_id)
        })
        .collect();
    long.sort_by(|a, b| b.1.query.elapsed_s.total_cmp(&a.1.query.elapsed_s));
    let &(slice, top) = long.first()?;
    let lead = if long.len() == 1 {
        Text::default().plain("long query · ")
    } else {
        Text::default().plain(format!("{} long queries · longest ", long.len()))
    };
    let brief = lead
        .person(who_slice(slice))
        .plain(", ")
        .strong(fmt::dur(top.query.elapsed_s));
    let detail = long
        .iter()
        .take(3)
        .map(|(slice, stat)| {
            Text::default()
                .person(who_slice(slice))
                .plain(" · ")
                .strong(fmt::dur(stat.query.elapsed_s))
                .plain(format!(" · {}", fmt::bytes(stat.query.memory_bytes)))
                .muted(format!(" · {}", crate::sqltext::summary(&stat.query.sql).label()))
        })
        .collect();
    Some(Finding {
        level: Severity::Warn,
        kind: Kind::Long,
        weight: (top.query.elapsed_s / 60.0).min(99.0),
        subject: query_subject(node, slice, top),
        brief,
        detail,
    })
}

fn lag(node: &NodeView<'_>) -> Option<Finding> {
    let level = severity::lag(node.node.lag_s);
    let lag = fmt::dur(node.node.lag_s as f64);
    level.is_problem().then(|| Finding {
        level,
        kind: Kind::Lag,
        weight: (node.node.lag_s as f64 / 60.0).min(99.0),
        subject: Subject::Node(node.node.name.clone()),
        brief: Text::default().plain("replica lag ").sev(lag.clone(), level),
        detail: vec![Text::default()
            .plain("replica lag ")
            .sev(lag, level)
            .muted(" — reads there may be stale")],
    })
}

// -- the Redash side ----------------------------------------------------------------------

/// The Redash queues backing up, and whether their workers are stuck on long ClickHouse queries
/// or merely busy (§2.8): one line, led by the worst queue. An unreachable Redash is the band's
/// to say, not a line here.
fn queue_line(queue: &QueueStatus, view: &FleetView<'_>, history: &History) -> Option<Insight> {
    if !queue.reachable {
        return None;
    }
    struct Backlog<'q> {
        level: Severity,
        row: &'q crate::model::QueueRow,
        stuck_workers: usize,
    }
    let mut stuck_nodes: Vec<&str> = Vec::new();
    let mut backlogs: Vec<Backlog<'_>> = Vec::new();
    for row in &queue.queues {
        let level = severity::wait(row.oldest_wait_s.unwrap_or(0), row.saturated());
        if !level.is_problem() || row.waiting == 0 {
            continue;
        }
        let stuck: Vec<&str> = queue
            .jobs
            .iter()
            .filter(|j| j.queue == row.name && j.state == JobState::Started)
            .filter_map(|j| j.clickhouse_target())
            .filter(|(node, query_id)| is_runaway(view, node, query_id))
            .map(|(node, _)| node)
            .collect();
        stuck_nodes.extend(&stuck);
        backlogs.push(Backlog { level, row, stuck_workers: stuck.len() });
    }
    backlogs.sort_by(|a, b| {
        b.level
            .cmp(&a.level)
            .then_with(|| b.row.oldest_wait_s.cmp(&a.row.oldest_wait_s))
    });
    let lead = backlogs.first()?;
    let oldest = lead.row.oldest_wait_s.unwrap_or(0);

    let mut brief = Text::default()
        .plain(format!("{}: ", lead.row.name))
        .sev(format!("{} waiting", lead.row.waiting), lead.level)
        .plain(" · oldest ")
        .sev(fmt::opt_dur(lead.row.oldest_wait_s), severity::wait(oldest, false));
    if lead.stuck_workers > 0 {
        brief = brief.plain(" · ").sev(
            format!("{} of {} workers on long queries", lead.stuck_workers, lead.row.workers_total),
            Severity::Crit,
        );
    }
    if backlogs.len() > 1 {
        brief = brief.muted(format!("  +{} more", backlogs.len() - 1));
    }

    let mut details: Vec<Phrase> = backlogs
        .iter()
        .map(|b| {
            let mut line = Text::default().plain(format!(
                "{}: {} waiting, oldest {} · workers {}/{} busy",
                b.row.name,
                b.row.waiting,
                fmt::opt_dur(b.row.oldest_wait_s),
                b.row.workers_busy,
                b.row.workers_total
            ));
            if let Some(per_min) = history
                .queue(&b.row.name)
                .and_then(|series| series.slope(60.0))
                .map(|slope| slope * 60.0)
                .filter(|per_min| *per_min >= 1.0)
            {
                line = line.plain(" · ").sev(format!("growing +{per_min:.0}/min"), Severity::Warn);
            }
            line.0
        })
        .collect();
    stuck_nodes.sort_unstable();
    stuck_nodes.dedup();
    details.push(if stuck_nodes.is_empty() {
        Text::default().muted("no worker is stuck in ClickHouse — the queue is simply full").0
    } else {
        Text::default()
            .plain("workers waiting on long ClickHouse queries: ")
            .node(stuck_nodes.join(", "))
            .0
    });
    Some(Insight {
        level: lead.level,
        score: QUEUE_BASE + (oldest as f64 / 10.0).min(99.0),
        subject: Subject::Queue,
        label: "Redash".to_string(),
        parts: brief.0,
        details,
    })
}

fn is_runaway(view: &FleetView<'_>, node: &str, query_id: &str) -> bool {
    view.nodes
        .iter()
        .filter(|n| n.node.name == node)
        .flat_map(|n| n.users.iter())
        .flat_map(|u| u.queries.iter())
        .any(|q| q.query.query_id == query_id && q.runaway)
}

/// The same Redash query running more than once at the same time. Distributed fan-out cannot
/// cause this — secondary queries are not initial (§5.1) — so two initial queries with one
/// Redash number are two executions: a dashboard refreshed twice, or two people on it.
fn duplicate_lines(view: &FleetView<'_>) -> Vec<Insight> {
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
    let mut out = Vec::new();
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
        let brief = Text::default()
            .plain("running ")
            .sev(format!("{}×", runs.len()), Severity::Warn)
            .plain(" · ")
            .person(people.join(", "));
        let mut first = Text::default()
            .plain(format!("{} runs at once on ", runs.len()))
            .node(nodes.join(", "));
        if bytes >= 64 * 1024 * 1024 {
            first = first.plain(format!(" · {} together", fmt::bytes(bytes)));
        }
        out.push(Insight {
            level: Severity::Warn,
            score: DUPLICATE_BASE + runs.len() as f64,
            subject: query_subject(node, slice, stat),
            label: format!("Redash #{id}"),
            parts: brief.0,
            details: vec![first.0, Text::default().muted("a dashboard refreshed twice, or two people on it").0],
        });
    }
    out
}

/// Nothing to look at: one line that says so, with the size of what was looked at.
fn all_clear(view: &FleetView<'_>, queue: &QueueStatus) -> Insight {
    let nodes = view.nodes.len();
    let queries: usize = view.nodes.iter().map(|n| n.node.queries.len()).sum();
    let mut brief = Text::default()
        .strong(if nodes == 1 { "the node is healthy".to_string() } else { format!("all {nodes} nodes healthy") })
        .plain(format!(" · {} running", fmt::plural(queries, "query", "queries")));
    if queue.reachable {
        brief = brief.plain(" · Redash keeping up");
    }
    Insight {
        level: Severity::Ok,
        score: 0.0,
        subject: Subject::Fleet,
        label: "fleet".to_string(),
        parts: brief.0,
        details: vec![Text::default()
            .muted("no node above 75%, no query near its limit or past 30 s, no lag, no queue waiting")
            .0],
    }
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
        analyze(&view, &history, queue)
    }

    fn texts(insights: &[Insight]) -> Vec<String> {
        insights.iter().map(Insight::text).collect()
    }

    fn no_queue() -> QueueStatus {
        QueueStatus::unreachable("not polled yet")
    }

    #[test]
    fn a_quiet_fleet_is_one_calm_line() {
        let mut slow = quiet_node("b");
        slow.poll_ms = Some(2_000);
        slow.mem_total = None;
        let s = snap(vec![quiet_node("a"), slow]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["fleet  all 2 nodes healthy · 0 queries running"], "a slow poll or an unknown total is not news");
        assert_eq!(insights[0].level, Severity::Ok);
        assert_eq!(overall(&insights), Severity::Ok);
    }

    #[test]
    fn a_hot_node_is_one_line_that_names_who_holds_it() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 60 * GIB; // 93.75%
        hot.queries = vec![redash_query("q1", "grigol.gankava", 7438, 12.0, 30 * GIB)];
        let s = snap(vec![hot, quiet_node("calm")]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["hot  memory 93.8% · grigol.gankava holds 47%"]);
        assert_eq!(insights[0].level, Severity::Crit);
        assert_eq!(insights[0].subject, Subject::Node("hot".into()));
        let details = insights[0].detail_text();
        assert!(details[0].starts_with("memory 93.8% (60.0 GiB of 64.0 GiB)"), "{details:?}");
        assert!(details[1].starts_with("memory held by grigol.gankava 47%"), "{details:?}");
        assert!(details[2].contains("free — less than one 9.0 GiB query"), "4 GiB free: {details:?}");
    }

    #[test]
    fn a_node_the_server_itself_holds_says_so() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 50 * GIB; // 78%
        hot.queries = vec![redash_query("q1", "j.petrova", 1, 3.0, GIB)];
        let s = snap(vec![hot]);
        let text = analyze_snapshot(&s, &no_queue())[0].text();
        assert_eq!(text, "hot  memory 78.1% · the server itself holds 77%");
    }

    #[test]
    fn hot_memory_and_cpu_share_the_line() {
        let mut hot = quiet_node("hot");
        hot.mem_used = 60 * GIB;
        hot.cpu_busy_cores = Some(15.0);
        hot.queries = vec![redash_query("q1", "j.petrova", 1, 3.0, GIB)];
        let s = snap(vec![hot]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].text(), "hot  memory 93.8% · CPU 93.8% · the server itself holds 92%");
        assert!(insights[0].detail_text().iter().any(|d| d.starts_with("CPU held by")));
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
        let insights = analyze(&view, &history, &no_queue());
        assert_eq!(texts(&insights), ["fleet  the node is healthy · 0 queries running"]);
    }

    #[test]
    fn a_node_with_several_problems_is_still_one_line() {
        let mut node = quiet_node("busy");
        node.mem_used = 60 * GIB;
        node.lag_s = 120;
        let mut long = redash_query("q1", "j.petrova", 8585, 95.0, 2 * GIB);
        long.sql = "SELECT * FROM gateway.transfers".into();
        node.queries = vec![long];
        let s = snap(vec![node]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(insights.len(), 1, "{:?}", texts(&insights));
        let text = insights[0].text();
        assert!(text.starts_with("busy  memory 93.8%"), "{text}");
        assert!(text.ends_with("+2 more"), "the long query and the lag: {text}");
        let details = insights[0].detail_text().join("\n");
        assert!(details.contains("j.petrova · 1m35s · 2.0 GiB · SELECT · gateway.transfers"), "{details}");
        assert!(details.contains("replica lag 2m00s — reads there may be stale"), "{details}");
    }

    #[test]
    fn a_long_query_names_who_and_how_long() {
        let mut node = quiet_node("n");
        node.queries = vec![
            redash_query("q1", "j.petrova", 1, 1670.0, 2 * GIB),
            redash_query("q2", "m.kairys", 2, 40.0, GIB),
        ];
        let s = snap(vec![node]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["n  2 long queries · longest j.petrova, 27m50s"]);
        assert_eq!(insights[0].level, Severity::Warn);
        assert!(matches!(&insights[0].subject, Subject::Query { query_id, .. } if query_id == "q1"), "⏎ goes to the longest");
    }

    #[test]
    fn a_query_near_its_own_limit_is_said_once() {
        let mut node = quiet_node("n");
        let mut q = redash_query("q1", "m.kairys", 7711, 45.0, 8 * GIB);
        q.memory_limit = Some(8 * GIB + GIB / 4); // 97%
        node.queries = vec![q];
        let s = snap(vec![node]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["n  m.kairys's query at 97% of its memory limit"], "not again as a long query");
        assert_eq!(insights[0].level, Severity::Crit);
        assert!(matches!(&insights[0].subject, Subject::Query { query_id, .. } if query_id == "q1"));
        assert!(insights[0].detail_text()[0].starts_with("m.kairys · 8.0 GiB of its 8.2 GiB limit"));
    }

    /// What one fleet showed: a 71.5 GiB CREATE whose settings say 5.6 GiB. A limit the query is
    /// twelve times past is not the one holding it, so there is no "1279% of its limit".
    #[test]
    fn a_limit_far_behind_the_query_is_not_quoted() {
        let mut node = quiet_node("n");
        node.mem_total = Some(453 * GIB);
        let mut q = redash_query("q1", "ch_user", 1, 1670.0, 71 * GIB + GIB / 2);
        q.memory_limit = Some(6_000_000_000);
        q.sql = "CREATE MATERIALIZED VIEW materialized_views.mv_invoice_monthly_totals AS SELECT 1".into();
        node.queries = vec![q];
        let s = snap(vec![node]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["n  long query · ch_user, 27m50s"]);
        assert!(!insights[0].detail_text().join(" ").contains('%'), "{:?}", insights[0].detail_text());
    }

    #[test]
    fn a_node_that_answered_without_access_says_no_access_in_short() {
        let s = snap(vec![
            quiet_node("a"),
            NodeSnapshot::unreachable("metrics", "no access — r_reports_daily needs SELECT on system.asynchronous_metrics"),
            NodeSnapshot::unreachable("dead", "connection refused — nothing listening on that port"),
        ]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(
            texts(&insights),
            [
                "dead  unreachable · connection refused",
                "metrics  no access · r_reports_daily needs SELECT on system.asynchronous_metrics",
            ]
        );
        let details = insights[1].detail_text();
        assert_eq!(details[0], "no access: r_reports_daily needs SELECT on system.asynchronous_metrics");
        assert!(details[1].contains("SELECT on system.*"), "{details:?}");
        assert_eq!(insights[0].detail_text()[0], "unreachable: connection refused — nothing listening on that port");
    }

    #[test]
    fn a_host_without_a_login_is_not_polled() {
        let s = snap(vec![NodeSnapshot::unreachable("ch-x", crate::model::NO_LOGIN)]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["ch-x  not polled · no login for this host"]);
        assert!(insights[0].detail_text()[0].contains("add it to the credential file"));
    }

    #[test]
    fn a_steady_climb_is_forecast_on_the_node_line() {
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
        let insights = analyze(&view, &history, &no_queue());
        // 79% now, 21 points to go at 6/min = 3.5 min: red.
        assert_eq!(texts(&insights), ["climb  memory 79.0% ↗ full in ~3m30s · the server itself holds 79%"]);
        assert_eq!(insights[0].level, Severity::Crit);
        assert!(insights[0].detail_text()[0].contains("+6.0%/min, full in ~3m30s at this rate"));
    }

    #[test]
    fn a_redash_query_running_twice_is_its_own_line() {
        let mut a = quiet_node("a");
        a.queries = vec![redash_query("q1", "j.petrova", 8585, 3.0, GIB)];
        let mut b = quiet_node("b");
        b.queries = vec![redash_query("q2", "j.petrova", 8585, 2.0, GIB)];
        let s = snap(vec![a, b]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(texts(&insights), ["Redash #8585  running 2× · j.petrova"]);
        assert_eq!(insights[0].detail_text()[0], "2 runs at once on a, b · 2.0 GiB together");
    }

    fn stitched_fake() -> (FleetSnapshot, QueueStatus) {
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
        (snapshot, queue)
    }

    #[test]
    fn the_fake_fleet_explains_its_full_queue() {
        let (snapshot, queue) = stitched_fake();
        let insights = analyze_snapshot(&snapshot, &queue);
        let backlog = insights
            .iter()
            .find(|i| i.subject == Subject::Queue)
            .expect("the backed-up queue is explained");
        assert_eq!(backlog.label, "Redash");
        assert!(backlog.text().starts_with("Redash  queries: "), "the worst queue leads: {}", backlog.text());
        assert!(backlog.text().contains("workers on long queries"), "{}", backlog.text());
        assert!(backlog.text().ends_with("+1 more"), "scheduled_queries is waiting too: {}", backlog.text());
        assert_eq!(insights.iter().filter(|i| i.subject == Subject::Queue).count(), 1, "one line for Redash");
        assert!(insights.iter().any(|i| i.label == "clickhouse3"), "{:?}", texts(&insights));
        assert_eq!(overall(&insights), Severity::Crit);
    }

    #[test]
    fn labels_leave_out_the_domain_every_node_shares() {
        assert_eq!(
            shared_domain(&["clickhouse1.example.net", "clickhouse-metrics.example.net"]).as_deref(),
            Some(".example.net")
        );
        assert_eq!(shared_domain(&["a.dc1.example.net", "b.dc2.example.net"]).as_deref(), Some(".example.net"));
        assert_eq!(shared_domain(&["ch-a", "ch-b"]), None);
        assert_eq!(shared_domain(&["127.0.0.1:8123", "127.0.0.1:8124"]), None);
        assert_eq!(shared_domain(&["10.0.0.1", "10.0.0.2"]), None);
        let s = snap(vec![
            NodeSnapshot::unreachable("clickhouse-metrics.example.net", "connection refused"),
            quiet_node("clickhouse1.example.net"),
        ]);
        let insights = analyze_snapshot(&s, &no_queue());
        assert_eq!(insights[0].label, "clickhouse-metrics");
        assert_eq!(insights[0].subject, Subject::Node("clickhouse-metrics.example.net".into()), "⏎ still finds it");
    }

    /// The point of the rewrite: nothing is said twice, and every line fits a laptop.
    #[test]
    fn every_subject_is_one_short_line() {
        let (snapshot, queue) = stitched_fake();
        let insights = analyze_snapshot(&snapshot, &queue);
        let mut labels: Vec<&str> = insights.iter().map(|i| i.label.as_str()).collect();
        let all = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all, "{:?}", texts(&insights));
        for line in texts(&insights) {
            assert!(fmt::width(&line) <= 90, "too long for a line ({}): {line}", fmt::width(&line));
        }
    }

}
