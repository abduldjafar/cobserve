//! What the bot's messages look like: /status, the cards and what they turn into, the alerts and
//! the fixed lines — Telegram HTML, every piece of data through `esc`, every percentage beside
//! what it is of. Emoji are severity (🔴 🟠 🟢) and outcome (✅ ❌ ⏳ ⌛) only; inside `<pre>`,
//! where an emoji breaks the columns, the screen's own `✖ ▲` carry it.

use super::html::esc;
use crate::app::App;
use crate::fmt;
use crate::model::{JobState, NodeView};
use crate::severity::{self, Severity};
use crate::ui2::panels;

/// The mark of a level outside `<pre>`, with its space; nothing when fine.
pub fn dot(sev: Severity) -> &'static str {
    match sev {
        Severity::Crit => "🔴 ",
        Severity::Warn => "🟠 ",
        Severity::Ok => "🟢 ",
        _ => "",
    }
}

/// The mark of a level inside `<pre>` and in what Claude reads.
pub fn glyph(sev: Severity) -> &'static str {
    match sev {
        Severity::Crit => "✖",
        Severity::Warn => "▲",
        _ => " ",
    }
}

pub const HINT: &str = "<i>Nothing happens until you press — the button works for 5 minutes.</i>";

/// A node's name without the domain every node shares.
pub fn short(app: &App, name: &str) -> String {
    let names: Vec<String> = app.snapshot().map(|s| s.nodes.iter().map(|n| n.name.clone()).collect()).unwrap_or_default();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    match crate::insight::shared_domain(&refs) {
        Some(domain) => name.strip_suffix(domain.as_str()).unwrap_or(name).to_string(),
        None => name.to_string(),
    }
}

fn hm(app: &App, at: i64) -> String {
    app.time.format(at, "%H:%M")
}

fn gib(bytes: u64) -> String {
    let g = fmt::gib(bytes);
    if g >= 100.0 || (g - g.round()).abs() < 0.05 { format!("{g:.0}") } else { format!("{g:.1}") }
}

fn node_sev(n: &NodeView<'_>) -> Severity {
    if !n.node.reachable {
        return Severity::Crit;
    }
    severity::node(n.mem_pct).max(severity::node(n.cpu_pct))
}

// -- /status ---------------------------------------------------------------------------------

/// What /status shows: the overview, or one panel's detail. Its buttons switch between them,
/// editing the one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Fleet,
    Redash,
    Airflow,
    Jira,
}

impl Tab {
    pub fn data(self) -> &'static str {
        match self {
            Tab::Home => "s",
            Tab::Fleet => "t:f",
            Tab::Redash => "t:r",
            Tab::Airflow => "t:a",
            Tab::Jira => "t:j",
        }
    }

    pub fn of(data: &str) -> Option<Tab> {
        Some(match data {
            "s" => Tab::Home,
            "t:f" => Tab::Fleet,
            "t:r" => Tab::Redash,
            "t:a" => Tab::Airflow,
            "t:j" => Tab::Jira,
            _ => return None,
        })
    }
}

/// A short duration for a list: `8h`, `3m16s`, `262d`.
fn brief(seconds: i64) -> String {
    match seconds.max(0) {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3600 => format!("{}h", s / 3600),
        s => fmt::dur(s as f64),
    }
}

fn secs(at: std::time::SystemTime) -> i64 {
    at.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn sev_of(app: &App, tab: Tab) -> Severity {
    match tab {
        Tab::Home => Severity::None,
        Tab::Fleet => panels::title(app, crate::ui2::PanelId::Fleet).0.max(panels::title(app, crate::ui2::PanelId::Nodes).0),
        Tab::Redash => panels::queue::title(app).0,
        Tab::Airflow => panels::airflow::title(app).0,
        Tab::Jira => panels::jira::title(app).0,
    }
}

/// The tabs, each with its panel's mark, the one shown pointed at; Refresh last.
pub fn tabs(app: &App, shown: Tab) -> Vec<(String, String)> {
    let label = |tab: Tab, name: &str| {
        let here = if tab == shown { "· " } else { "" };
        (format!("{here}{}{name}", dot(sev_of(app, tab))), tab.data().to_string())
    };
    let mut row = vec![label(Tab::Fleet, "Fleet"), label(Tab::Redash, "Redash")];
    let mut second = Vec::new();
    if !app.airflow.is_placeholder() {
        second.push(label(Tab::Airflow, "Airflow"));
    }
    if !app.jira.is_placeholder() {
        second.push(label(Tab::Jira, "Jira"));
    }
    row.push((String::new(), String::new()));
    row.extend(second);
    row.push((String::new(), String::new()));
    let home = if shown == Tab::Home { "· Overview".to_string() } else { "‹ Overview".to_string() };
    row.push((home, Tab::Home.data().to_string()));
    row.push(("↻ Refresh".to_string(), format!("r:{}", shown.data())));
    row
}

/// /status: the overview or a tab, with when it was read at the foot.
pub fn status(app: &App, now: i64, tab: Tab) -> String {
    let profile = if app.shell.profile.is_empty() { String::new() } else { format!(" · {}", esc(&app.shell.profile)) };
    let body = match tab {
        Tab::Home => home(app),
        Tab::Fleet => fleet(app),
        Tab::Redash => queue(app),
        Tab::Airflow => airflow(app, now),
        Tab::Jira => jira(app, now),
    };
    let title = match tab {
        Tab::Home => {
            let worst = [Tab::Fleet, Tab::Redash, Tab::Airflow, Tab::Jira].into_iter().map(|t| sev_of(app, t)).max().unwrap_or(Severity::None);
            format!("{}<b>cobserve</b>{profile}", dot(if worst.is_problem() { worst } else { Severity::Ok }))
        }
        Tab::Fleet => format!("{}<b>Fleet</b>", dot(sev_of(app, tab))),
        Tab::Redash => format!("{}<b>Redash queue</b>", dot(sev_of(app, tab))),
        Tab::Airflow => format!("{}<b>Airflow</b>", dot(sev_of(app, tab))),
        Tab::Jira => format!("{}<b>Jira</b>", dot(sev_of(app, tab))),
    };
    let read = match tab {
        Tab::Home | Tab::Fleet => app.snapshot().map(|s| s.taken_at),
        Tab::Redash => app.queue.reachable.then_some(app.queue.taken_at),
        Tab::Airflow => app.airflow.reachable.then_some(app.airflow.taken_at),
        Tab::Jira => app.jira.reachable.then_some(app.jira.taken_at),
    };
    let foot = match read {
        Some(at) => format!("<i>{} {} · read {}</i>", hm(app, now), esc(&app.time.label(now)), fmt::ago(now - secs(at))),
        None => format!("<i>{} {} · not read yet</i>", hm(app, now), esc(&app.time.label(now))),
    };
    format!("{title}\n\n{}\n\n{foot}", body.trim_end())
}

/// The overview: the fleet in a line, then what needs a look, three at most.
fn home(app: &App) -> String {
    let mut out = Vec::new();
    if let Some(t) = app.with_view(crate::model::fleet_totals) {
        out.push(format!("memory {} · {}", panels::fleet::gib_of(t.mem_used, t.mem_total), fmt::pct(t.mem_pct())));
        out.push(format!("cpu {} · {}", panels::fleet::cores_of(&t), fmt::pct(t.cpu_pct())));
        out.push(String::new());
    }
    let insights = app.insights();
    let problems: Vec<&crate::insight::Insight> = insights.iter().filter(|i| i.level.is_problem()).collect();
    if problems.is_empty() {
        let line = insights.first().map(|i| format!("{} {}", i.label, phrase(&i.parts))).unwrap_or_else(|| "nothing wrong".to_string());
        out.push(format!("{}{}", dot(Severity::Ok), esc(&line)));
    }
    for i in problems.iter().take(3) {
        let line = phrase(&i.parts);
        let first = line.split(" · ").next().unwrap_or(&line).trim();
        out.push(format!("{}<b>{}</b>", dot(i.level), esc(&i.label)));
        out.push(esc(&fmt::truncate(first, 44)));
    }
    if problems.len() > 3 {
        out.push(format!("<i>+{} more — open a tab</i>", problems.len() - 3));
    }
    out.join("\n")
}

/// Every node in a table, the worst first, then the heaviest queries.
fn fleet(app: &App) -> String {
    let Some(text) = app.with_view(|v| {
        let t = crate::model::fleet_totals(v);
        let mut out = vec![
            format!("{} · {} runaway", fmt::plural(t.nodes, "node", "nodes"), t.runaways),
            format!("memory {} · {}", panels::fleet::gib_of(t.mem_used, t.mem_total), fmt::pct(t.mem_pct())),
            format!("cpu {} · {}", panels::fleet::cores_of(&t), fmt::pct(t.cpu_pct())),
        ];
        let mut nodes: Vec<&NodeView<'_>> = v.nodes.iter().collect();
        nodes.sort_by(|a, b| node_sev(b).cmp(&node_sev(a)).then(b.pressure.total_cmp(&a.pressure)));
        let mut pre = vec!["  node           mem  cpu of".to_string()];
        for n in nodes.iter().take(12) {
            let name = fmt::pad(&fmt::truncate(&short(app, n.name()), 13), 13);
            if !n.node.reachable {
                pre.push(format!("{} {name} {}", glyph(Severity::Crit), n.node.down_word()));
                continue;
            }
            let pct = |p: Option<f64>| p.map_or("  —".to_string(), |p| format!("{:>3}", fmt::pct0(p)));
            let of = format!("{}G/{:.0}c", n.node.mem_total.map(gib).unwrap_or_else(|| "—".into()), n.node.cores.unwrap_or(0.0));
            pre.push(format!("{} {name} {:>4} {:>4} {of}", glyph(node_sev(n)), pct(n.mem_pct), pct(n.cpu_pct)));
        }
        if nodes.len() > 12 {
            pre.push(format!("  … {} more", nodes.len() - 12));
        }
        out.push(String::new());
        out.push(format!("<pre>{}</pre>", esc(&pre.join("\n"))));
        let mut queries: Vec<(&str, &crate::model::QueryStat<'_>)> =
            v.nodes.iter().flat_map(|n| n.users.iter().flat_map(move |u| u.queries.iter().map(move |q| (n.name(), q)))).collect();
        queries.sort_by_key(|q| std::cmp::Reverse(q.1.query.memory_bytes));
        if !queries.is_empty() {
            out.push(String::new());
            out.push("<b>Heaviest queries</b>".to_string());
        }
        for (node, q) in queries.iter().take(3) {
            let who = q.query.person.clone().unwrap_or_else(|| q.query.user.clone());
            let mark = if q.runaway { dot(Severity::Warn) } else { "• " };
            out.push(format!("{mark}<b>{}</b> · {} · {}", fmt::bytes(q.query.memory_bytes), esc(&who), esc(&short(app, node))));
            out.push(format!("   <code>{}</code> · {}", esc(&q.query.query_id), fmt::dur(q.query.elapsed_s)));
        }
        out.join("\n")
    }) else {
        return "the fleet has not been read yet".to_string();
    };
    text
}

fn queue(app: &App) -> String {
    let q = &app.queue;
    if q.is_placeholder() {
        return "Redash is not configured".to_string();
    }
    if !q.reachable {
        return format!("unreachable — {}", esc(q.error.as_deref().unwrap_or("no answer")));
    }
    let mut out = vec![
        format!("{} waiting · {} running", q.total_waiting(), q.total_running()),
        format!("workers {}/{} busy{}", q.workers_busy, q.workers_total, if q.total_stale() > 0 { format!(" · {} stale", q.total_stale()) } else { String::new() }),
        String::new(),
    ];
    let idle: Vec<String> = q.queues.iter().filter(|r| r.is_idle()).map(|r| esc(&r.name)).collect();
    for row in q.queues.iter().filter(|r| !r.is_idle()) {
        let sev = row.oldest_wait_s.map_or(Severity::None, |s| severity::wait(s, row.saturated()));
        let mark = if sev.is_problem() { dot(sev) } else { "• " };
        out.push(format!("{mark}<b>{}</b>", esc(&row.name)));
        let mut line = format!("   {} waiting · {} running", row.waiting, row.running);
        if let Some(s) = row.oldest_wait_s.filter(|_| row.waiting > 0) {
            line.push_str(&format!(" · oldest {}", fmt::dur(s as f64)));
        }
        out.push(line);
    }
    if !idle.is_empty() {
        out.push(format!("<i>idle: {}</i>", idle.join(", ")));
    }
    let mut running: Vec<&crate::model::Job> = q.jobs.iter().filter(|j| j.state == JobState::Started).collect();
    running.sort_by_key(|j| std::cmp::Reverse(j.age_s));
    if !running.is_empty() {
        out.push(String::new());
        out.push("<b>Running longest</b>".to_string());
    }
    for j in running.iter().take(3) {
        out.push(format!("• <b>{}</b> · {}", fmt::dur(j.age_s as f64), esc(&j.label())));
        out.push(format!("   {}", esc(&fmt::truncate(&j.query_label(), 36))));
    }
    out.join("\n")
}

fn airflow(app: &App, now: i64) -> String {
    let a = &app.airflow;
    if a.is_placeholder() {
        return "Airflow is not configured".to_string();
    }
    if !a.reachable {
        return format!("not read — {}", esc(a.error.as_deref().unwrap_or("no answer")));
    }
    let c = a.counts(now);
    let mut out = vec![format!("{} runs in 24 h", c.runs), format!("{} ok · {} failed · {} running · {} queued", c.ok, c.failed, c.running, c.queued)];
    if let Some(errors) = a.import_errors.filter(|e| *e > 0) {
        out.push(format!("{}{}", dot(Severity::Crit), fmt::plural(errors as usize, "DAG import error", "DAG import errors")));
    }
    let sections = a.sections(now);
    if !sections.failed.is_empty() {
        out.push(String::new());
        out.push("<b>Failed</b>".to_string());
    }
    for r in sections.failed.iter().take(5) {
        let task = a.progress.get(&(r.dag.clone(), r.id.clone())).and_then(|p| p.failed.first().cloned());
        let ago = r.at().map(|t| format!("{} ago", brief(now - t))).unwrap_or_default();
        out.push(format!("{}<code>{}</code>", dot(Severity::Crit), esc(&fmt::truncate(&r.dag, 32))));
        out.push(format!("   {}{ago}", task.map(|t| format!("at {} · ", esc(&fmt::truncate(&t, 24)))).unwrap_or_default()));
    }
    let long: Vec<&&crate::airflow::Run> = sections.running.iter().filter(|r| r.severity(now).is_problem()).collect();
    if !long.is_empty() {
        out.push(String::new());
        out.push("<b>Running long</b>".to_string());
    }
    for r in long.iter().take(3) {
        let took = r.took(now).map(brief).unwrap_or_default();
        out.push(format!("{}<code>{}</code>", dot(r.severity(now)), esc(&fmt::truncate(&r.dag, 32))));
        out.push(format!("   {} {took}", if r.is_stuck(now) { "stuck" } else { "running" }));
    }
    out.join("\n")
}

fn jira(app: &App, now: i64) -> String {
    let b = &app.jira;
    if b.is_placeholder() {
        return "Jira is not configured".to_string();
    }
    if !b.reachable {
        return format!("not read — {}", esc(b.error.as_deref().unwrap_or("no answer")));
    }
    let columns = b.columns();
    let mut out: Vec<String> = columns.iter().map(|c| format!("• {} <b>{}</b>", esc(c.status), c.tickets.len())).collect();
    let today = crate::jira::today(now, app.time.offset_s(now));
    let mut due: Vec<(Severity, i64, &crate::jira::Ticket)> = columns
        .iter()
        .filter(|c| !c.finished)
        .flat_map(|c| c.tickets.iter())
        .filter_map(|t| {
            let day = t.due_day()?;
            let sev = crate::jira::due_severity(day, today, false);
            sev.is_problem().then_some((sev, day, *t))
        })
        .collect();
    due.sort_by_key(|(_, day, _)| *day);
    if !due.is_empty() {
        out.push(String::new());
        out.push("<b>Due</b>".to_string());
    }
    for (sev, day, t) in due.iter().take(5) {
        let when = match day - today {
            d if d < 0 => format!("overdue {}d", -d),
            0 => "due today".to_string(),
            1 => "due tomorrow".to_string(),
            d => format!("due in {d}d"),
        };
        out.push(format!("{}<code>{}</code> · {when}", dot(*sev), esc(&t.key)));
        out.push(format!("   {}", esc(&fmt::truncate(&t.summary, 38))));
    }
    out.join("\n")
}

fn phrase(parts: &[(String, crate::insight::Tone)]) -> String {
    parts.iter().map(|(t, _)| t.as_str()).collect()
}

// -- cards -----------------------------------------------------------------------------------

/// What a kill card knows of its query, for the lines it turns into.
#[derive(Debug, Clone, PartialEq)]
pub struct Killing {
    pub node: String,
    pub query_id: String,
    pub memory_bytes: u64,
    pub elapsed_s: f64,
    pub node_pct: Option<f64>,
    pub node_total: Option<u64>,
}

/// A card: its facts (kept on every edit), the command that makes a fresh one, and what a kill
/// or a cancel's follow-up needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub html: String,
    pub again: String,
    pub killing: Option<Killing>,
    /// For a cancel whose job runs on ClickHouse: its node, query id and memory.
    pub follow: Option<(String, String, u64)>,
}

pub fn kill_card(app: &App, node: &str, query_id: &str) -> Result<Facts, String> {
    let again = format!("/kill {node} {query_id}");
    let found = app.with_view(|view| {
        let Some(n) = view.nodes.iter().find(|n| n.name() == node || short(app, n.name()) == node) else {
            return Err(format!("❌ <b>No card</b>\nno node called <b>{}</b> on screen", esc(node)));
        };
        let node = n.name().to_string();
        let Some(row) = n.node.queries.iter().find(|q| q.query_id == query_id) else {
            return Err(format!("❌ <b>No card</b>\n<code>{}</code> is not running on <b>{}</b> any more — nothing to kill", esc(query_id), esc(&short(app, &node))));
        };
        let stat = n.users.iter().flat_map(|u| u.queries.iter()).find(|s| s.query.query_id == query_id);
        let name = short(app, &node);
        let who = match &row.person {
            Some(person) => format!("{} <i>via {}</i>", esc(person), esc(&row.user)),
            None => esc(&row.user),
        };
        let job = app.queue.jobs.iter().find(|j| j.ch_query_id.as_deref() == Some(query_id));
        let what = match (job, row.redash_query_id) {
            (Some(j), _) => Some(j.query_label()),
            (None, Some(id)) => Some(format!("Redash #{id}")),
            _ => None,
        };
        let mut lines = vec![
            "🛑 <b>Kill this query?</b>".to_string(),
            format!("<code>{}</code> on <b>{}</b>", esc(query_id), esc(&name)),
            String::new(),
            format!("<b>who</b>  {who}"),
        ];
        if let Some(what) = what {
            lines.push(format!("<b>what</b>  {}", esc(&what)));
        }
        let mut runs = format!("<b>runs</b>  {}", fmt::dur(row.elapsed_s));
        if let Some(stat) = stat {
            runs.push_str(&format!(" · {:.1} cores", stat.cores));
            if let Some(p) = stat.progress {
                runs.push_str(&format!(" · {:.0}% read", p * 100.0));
            }
        }
        lines.push(runs);
        let mut memory = format!("<b>memory</b>  {}", fmt::bytes(row.memory_bytes));
        if let Some(stat) = stat {
            memory.push_str(&format!(" of {} limit", fmt::bytes(stat.limit)));
            if stat.limit_holds() {
                memory.push_str(&format!(" · {}", fmt::pct0(stat.limit_fraction() * 100.0)));
            }
            if stat.runaway {
                memory.push_str(" 🟠 runaway");
            }
        }
        lines.push(memory);
        if let (Some(total), Some(pct)) = (n.node.mem_total.filter(|t| *t > 0), n.mem_pct) {
            let share = row.memory_bytes as f64 / total as f64 * 100.0;
            lines.push(format!(
                "<b>{}</b>  {} · {} · this query {}",
                esc(&name),
                panels::fleet::gib_of(n.node.mem_used, total),
                fmt::pct(Some(pct)),
                fmt::pct(Some(share))
            ));
        }
        let sql = sql_head(&row.sql);
        if !sql.is_empty() {
            lines.push(String::new());
            lines.push(format!("<pre><code class=\"language-sql\">{}</code></pre>", esc(&fmt::truncate(&sql, 160))));
        }
        Ok(Facts {
            html: lines.join("\n"),
            again: again.clone(),
            killing: Some(Killing {
                node: node.clone(),
                query_id: query_id.to_string(),
                memory_bytes: row.memory_bytes,
                elapsed_s: row.elapsed_s,
                node_pct: n.mem_pct,
                node_total: n.node.mem_total,
            }),
            follow: None,
        })
    });
    found.unwrap_or_else(|| Err("❌ <b>No card</b>\nthe fleet has not been read yet".to_string()))
}

pub fn cancel_card(app: &App, job: &str) -> Result<Facts, String> {
    let Some(j) = app.queue.jobs.iter().find(|j| j.id == job) else {
        return Err(format!("❌ <b>No card</b>\nRedash job <code>{}</code> is not in the queue any more — nothing to cancel", esc(job)));
    };
    let state = match j.state {
        JobState::Queued => format!("waiting {}", fmt::dur(j.age_s as f64)),
        JobState::Started => format!("running {}", fmt::dur(j.age_s as f64)),
        JobState::Stale(why) => format!("stale ({}) · {}", why.reason(), fmt::dur(j.age_s as f64)),
    };
    let mut lines = vec![
        "🛑 <b>Cancel this Redash job?</b>".to_string(),
        format!("<code>{}</code> · {}", esc(&j.id), esc(&j.query_label())),
        String::new(),
        format!("<b>who</b>  {}{}", esc(&j.label()), if j.scheduled { " <i>scheduled</i>" } else { "" }),
        format!("<b>state</b>  {state}"),
    ];
    let mut queue = format!("<b>queue</b>  <code>{}</code>", esc(&j.queue));
    if let Some(q) = app.queue.queue(&j.queue) {
        queue.push_str(&format!(" · {} waiting · workers {}/{} busy", q.waiting, q.workers_busy, q.workers_total));
    }
    lines.push(queue);
    let mut follow = None;
    if let Some((node, query_id)) = j.clickhouse_target() {
        let stat = app.with_view(|v| {
            v.nodes
                .iter()
                .filter(|n| n.name() == node)
                .flat_map(|n| n.users.iter().flat_map(|u| u.queries.iter()))
                .find(|s| s.query.query_id == query_id)
                .map(|s| (s.query.memory_bytes, s.limit, s.limit_holds().then(|| s.limit_fraction()), s.runaway))
        });
        let mut line = format!("<b>on {}</b>  <code>{}</code>", esc(&short(app, node)), esc(query_id));
        let mut memory = 0;
        if let Some(Some((mem, limit, fraction, runaway))) = stat {
            memory = mem;
            line.push_str(&format!(" · {} of {} limit", fmt::bytes(mem), fmt::bytes(limit)));
            if let Some(f) = fraction {
                line.push_str(&format!(" · {}", fmt::pct0(f * 100.0)));
            }
            if runaway {
                line.push_str(" 🟠 runaway");
            }
        }
        lines.push(line);
        lines.push(String::new());
        lines.push("<i>A Redash cancel does not reach ClickHouse: the query runs on there until KILL QUERY stops it.</i>".to_string());
        follow = Some((node.to_string(), query_id.to_string(), memory));
    }
    Ok(Facts { html: lines.join("\n"), again: format!("/cancel {}", j.id), killing: None, follow })
}

/// A query's SQL on one line, without the comments Redash and others put before it.
pub fn sql_head(sql: &str) -> String {
    let body: Vec<&str> = sql.lines().skip_while(|l| l.trim().is_empty() || l.trim_start().starts_with("--")).collect();
    let mut text = body.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    while text.starts_with("/*") {
        match text.find("*/") {
            Some(end) => text = text[end + 2..].trim_start().to_string(),
            None => break,
        }
    }
    text
}

/// The line under a killed query's card: what the kill freed, from what was there when it was
/// asked for.
pub fn killed(app: &App, how: &str, k: &Killing) -> String {
    let was = match (k.node_pct, k.node_total) {
        (Some(p), Some(total)) => format!(" (was {} of {} GiB)", fmt::pct(Some(p)), gib(total)),
        _ => String::new(),
    };
    format!("✅ <b>Killed</b> · {}\n{} to come back to {}{was}", esc(how), fmt::bytes(k.memory_bytes), esc(&short(app, &k.node)))
}

/// Once the query has left `system.processes`: how long it ran, and the node before and now.
pub fn gone(app: &App, k: &Killing, ran_s: f64) -> String {
    let name = short(app, &k.node);
    let now = app
        .with_view(|v| v.nodes.iter().find(|n| n.name() == k.node).and_then(|n| Some((n.mem_pct?, n.node.mem_used, n.node.mem_total?))))
        .flatten();
    let memory = match (k.node_pct, now) {
        (Some(before), Some((after, used, total))) => {
            format!(" · {} memory {} → {} ({})", esc(&name), fmt::pct(Some(before)), fmt::pct(Some(after)), panels::fleet::gib_of(used, total))
        }
        _ => String::new(),
    };
    let memory = memory.trim_start_matches(" · ");
    let memory = if memory.is_empty() { String::new() } else { format!("\n{memory}") };
    format!("✅ <b>Gone</b> · left {} after {}{memory}", esc(&name), fmt::dur(ran_s))
}

// -- alerts ----------------------------------------------------------------------------------

/// Where an alert line goes, in the order the groups are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Nodes,
    Queries,
    Redash,
    Fleet,
    Airflow,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Nodes => "Nodes",
            Group::Queries => "Queries",
            Group::Redash => "Redash",
            Group::Fleet => "Fleet",
            Group::Airflow => "Airflow",
        }
    }
}

/// One alert: where it goes, its HTML, and its text for Claude.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub group: Group,
    pub html: String,
    pub plain: String,
}

/// A tape event as an alert, with what the screen knows about its subject now.
pub fn tape_line(app: &App, e: &crate::tape::Event, level: Severity) -> Line {
    let text: String = e.parts.iter().map(|(t, _)| t.as_str()).collect();
    let group = match e.kind {
        crate::tape::Kind::Node => Group::Nodes,
        crate::tape::Kind::Query => Group::Queries,
        crate::tape::Kind::Queue => Group::Redash,
        _ => Group::Fleet,
    };
    let live = match &e.subject {
        Some(crate::insight::Subject::Node(name)) => app
            .with_view(|v| {
                let n = v.nodes.iter().find(|n| n.name() == name)?;
                if !n.node.reachable {
                    let since = app.history.node(name)?.unreachable_since?;
                    return Some(format!(" · for {}", fmt::dur((app.now() as f64 - since).max(0.0))));
                }
                let mut s = String::new();
                if let (Some(p), Some(total)) = (n.mem_pct, n.node.mem_total) {
                    s.push_str(&format!(" · now {} · {}", panels::fleet::gib_of(n.node.mem_used, total), fmt::pct(Some(p))));
                }
                if let (Some(busy), Some(cores)) = (n.busy_cores, n.node.cores) {
                    s.push_str(&format!(" · {busy:.1}/{cores:.0} cores"));
                }
                Some(s)
            })
            .flatten()
            .unwrap_or_default(),
        Some(crate::insight::Subject::Query { node, query_id, .. }) => app
            .with_view(|v| v.nodes.iter().find(|n| n.name() == node)?.node.queries.iter().find(|q| q.query_id == *query_id).map(|q| format!(" · {} now", fmt::bytes(q.memory_bytes))))
            .flatten()
            .unwrap_or_default(),
        _ => String::new(),
    };
    let at = hm(app, e.at as i64);
    Line {
        group,
        html: format!("{}{at} {}{}", dot(level), esc(&text), esc(&live)),
        plain: format!("{} {at} {} {text}{live}", glyph(level), e.kind.label()),
    }
}

pub fn airflow_line(app: &App, r: &crate::airflow::Run, now: i64) -> Line {
    let task = app.airflow.progress.get(&(r.dag.clone(), r.id.clone())).and_then(|p| p.failed.first().cloned());
    let at = r.end.or(r.at()).map(|t| hm(app, t)).unwrap_or_default();
    let took = r.took(now).map(|s| format!(" · ran {}", fmt::dur(s as f64))).unwrap_or_default();
    let at_task = task.as_ref().map(|t| format!(" at {}", esc(t))).unwrap_or_default();
    Line {
        group: Group::Airflow,
        html: format!("{}{at} <code>{}</code> failed{at_task}{took} · {} run", dot(Severity::Crit), esc(&r.dag), esc(&r.kind)),
        plain: format!("✖ {at} AIRFLOW {} failed{}{took} · run {}", r.dag, task.map(|t| format!(" at {t}")).unwrap_or_default(), r.id),
    }
}

/// A batch of alerts as one message: grouped, each group oldest first.
pub fn alerts(app: &App, lines: &[Line], now: i64) -> String {
    let mut out = vec![format!("🔔 <b>{} new</b> · {} {}", lines.len(), hm(app, now), esc(&app.time.label(now)))];
    let mut groups: Vec<Group> = lines.iter().map(|l| l.group).collect();
    groups.sort();
    groups.dedup();
    let mut shown = 0;
    for group in &groups {
        if groups.len() > 1 {
            out.push(format!("<b>{}</b>", group.title()));
        }
        for line in lines.iter().filter(|l| l.group == *group) {
            if shown < 12 {
                out.push(line.html.clone());
            }
            shown += 1;
        }
    }
    if shown > 12 {
        out.push(format!("<i>… and {} more on the tape</i>", shown - 12));
    }
    out.join("\n")
}

// -- fixed lines -----------------------------------------------------------------------------

pub const HELP: &str = "<b>cobserve on your phone.</b> Ask anything about what it watches — the ClickHouse fleet, the queries running, the Redash queue, Airflow, Jira — in any language. I answer from the screen, never from memory, and I remember the conversation.
<blockquote>node mana yang paling penuh?
siapa yang bikin queue antri?
kenapa DAG kyc_onboarding_tables gagal?</blockquote>
Ask me to stop something and you get a card: nothing happens until you press its button.

<code>/status</code> — every panel in one message, without Claude
<code>/kill &lt;node&gt; &lt;query_id&gt;</code> · <code>/cancel &lt;job id&gt;</code> — a card
<code>/alerts red|all|off</code> — what I say unasked (red by default)
<code>/forget</code> — start a new conversation";

pub fn stranger(chat: i64) -> String {
    format!("cobserve answers only the chats it is told to. This chat's id is <code>{chat}</code>: set <code>TELEGRAM_CHAT_ID={chat}</code> and start cobserve again.")
}

pub fn late(text: &str, ago_s: i64) -> String {
    format!(
        "⌛ <b>Came while cobserve was not running</b> · {}\n<blockquote>{}</blockquote>\n<i>Ask again if it still matters.</i>",
        fmt::ago(ago_s),
        esc(&fmt::truncate(text, 200))
    )
}

pub fn thinking(question: &str, for_s: i64) -> String {
    format!("⏳ One at a time — still on <i>{}</i> · {for_s} s", esc(&fmt::truncate(question, 60)))
}

pub fn claude_failed(why: &str) -> String {
    format!("❌ <b>Claude could not answer</b> — {}\n<i>Try again, or /status for the numbers without Claude.</i>", esc(why))
}

pub const SAID_NOTHING: &str = "Claude answered with nothing — ask again, maybe more precisely.";
pub const EXPLAIN_GONE: &str = "⌛ That alert is older than the 30 I keep — ask about it in your own words.";
pub const FORGOTTEN: &str = "<b>Forgotten</b> — the next question starts a new conversation.";
pub const KILL_USAGE: &str = "<code>/kill &lt;node&gt; &lt;query_id&gt;</code> — or just ask: “kill the biggest query on clickhouse3”.";
pub const CANCEL_USAGE: &str = "<code>/cancel &lt;job id&gt;</code> — or just ask: “cancel j.petrova's job that waits longest”.";

pub fn alerts_set(alerts: super::Alerts) -> &'static str {
    match alerts {
        super::Alerts::Red => "🔔 <b>Alerts: red only</b> — I speak unasked when something goes red.",
        super::Alerts::All => "🔔 <b>Alerts: amber and red</b> — I speak unasked when something goes amber or red.",
        super::Alerts::Off => "<b>Alerts: off</b> — I speak only when asked.",
    }
}

/// Once, when the bot has read everything a first time.
pub fn up(app: &App, alerts: super::Alerts) -> String {
    let mut watching = Vec::new();
    if let Some((n, m)) = app.with_view(|v| (v.nodes.len(), v.nodes.iter().filter(|n| n.node.reachable).count())) {
        watching.push(format!("{} ({m} answering)", fmt::plural(n, "node", "nodes")));
    }
    if !app.queue.is_placeholder() {
        watching.push("the Redash queue".to_string());
    }
    if !app.airflow.is_placeholder() {
        watching.push("Airflow".to_string());
    }
    if !app.jira.is_placeholder() {
        watching.push("Jira".to_string());
    }
    let alerts = match alerts {
        super::Alerts::Red => "alerts: red only",
        super::Alerts::All => "alerts: amber and red",
        super::Alerts::Off => "alerts: off",
    };
    format!("🟢 <b>cobserve is up</b> · watching {}\n<i>{alerts} · /status · /help</i>", watching.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sql_on_a_card_starts_where_the_query_does() {
        assert_eq!(sql_head("/* Application: Redash */ /* Username: a@b */\n  SELECT  1\n FROM t"), "SELECT 1 FROM t");
        assert_eq!(sql_head("-- a note\nSELECT 2"), "SELECT 2");
        assert_eq!(sql_head("/* never closed SELECT 3"), "/* never closed SELECT 3");
    }

    #[test]
    fn status_is_short_and_every_tab_fits_a_phone() {
        let app = crate::ui2::tests::app_after(10);
        let now = app.now();
        for tab in [Tab::Home, Tab::Fleet, Tab::Redash, Tab::Airflow, Tab::Jira] {
            let s = status(&app, now, tab);
            let plain = crate::telegram::html::plain_of(&s);
            for line in plain.lines() {
                assert!(fmt::width(line) <= 46, "{tab:?}: {line:?}");
            }
            assert!(plain.lines().count() <= 30, "{tab:?}: {s}");
        }
        let home = status(&app, now, Tab::Home);
        assert!(home.starts_with("🔴 <b>cobserve</b>") && home.lines().count() <= 16, "{home}");
        assert!(status(&app, now, Tab::Fleet).contains("<pre>  node"), "the fleet's table");
        let labels: Vec<String> = tabs(&app, Tab::Fleet).into_iter().map(|b| b.0).collect();
        assert!(labels.contains(&"· 🔴 Fleet".to_string()) && labels.contains(&"↻ Refresh".to_string()), "{labels:?}");
    }

    /// The messages as they are sent, to look at: `cargo test telegram::look::tests::show -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn show() {
        let app = crate::ui2::tests::app_after(10);
        let now = app.now();
        for tab in [Tab::Home, Tab::Fleet, Tab::Redash, Tab::Airflow, Tab::Jira] {
            println!("{}\n{:?}\n\n----\n", status(&app, now, tab), tabs(&app, tab).iter().map(|b| b.0.clone()).collect::<Vec<_>>());
        }
        let (node, query_id) = app.with_view(|v| {
            let n = v.nodes.iter().max_by_key(|n| n.node.queries.iter().map(|q| q.memory_bytes).max().unwrap_or(0)).unwrap();
            let q = n.node.queries.iter().max_by_key(|q| q.memory_bytes).unwrap();
            (n.node.name.clone(), q.query_id.clone())
        }).unwrap();
        let facts = kill_card(&app, &node, &query_id).unwrap();
        println!("{}\n{HINT}\n\n{}\n\n----\n", facts.html, killed(&app, "asked", facts.killing.as_ref().unwrap()));
        let job = app.queue.jobs.iter().find(|j| j.clickhouse_target().is_some()).unwrap_or(&app.queue.jobs[0]);
        println!("{}\n\n----\n", cancel_card(&app, &job.id).unwrap().html);
        let lines: Vec<Line> = app.tape.newest_first().take(4).map(|e| tape_line(&app, e, e.level.max(Severity::Warn))).collect();
        println!("{}\n\n----\n{}", alerts(&app, &lines, now), up(&app, super::super::Alerts::Red));
    }
}
