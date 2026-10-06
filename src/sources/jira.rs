//! Jira's REST API (`/rest/api/2`, Jira Server and Data Center), read with a personal access
//! token (`Authorization: Bearer`): the tickets assigned to the token's owner in the board's
//! columns, and when each moved into the one it is in.
//!
//! Every minute, and at once when `r` asks. GET only — nothing here can change a ticket. A
//! ticket's history is read once and again only when the ticket has changed: it is the heavy
//! part of the answer, and it does not change otherwise.

use super::{transport, unix};
use crate::app::Event;
use crate::config::JiraConfig;
use crate::jira::{self, Board, Ticket};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

const POLL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE: usize = 100;
/// Five hundred tickets is more than one person's board holds; past it the rest are left out.
const MAX_PAGES: usize = 5;
/// What a ticket's row and the drawer show, and nothing heavier: no description, no comments.
const FIELDS: &str = "summary,status,priority,issuetype,created,updated,resolutiondate,duedate,parent,labels,reporter,timespent";

#[derive(Debug, Deserialize)]
struct Search {
    #[serde(default)]
    total: usize,
    #[serde(default)]
    issues: Vec<ApiIssue>,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    key: String,
    #[serde(default)]
    fields: ApiFields,
    changelog: Option<ApiChangelog>,
}

#[derive(Debug, Default, Deserialize)]
struct ApiFields {
    summary: Option<String>,
    status: Option<ApiStatus>,
    priority: Option<Named>,
    issuetype: Option<Named>,
    created: Option<String>,
    updated: Option<String>,
    resolutiondate: Option<String>,
    duedate: Option<String>,
    parent: Option<ApiParent>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    reporter: Option<ApiUser>,
    timespent: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ApiStatus {
    name: String,
    #[serde(rename = "statusCategory")]
    category: Option<ApiCategory>,
}

#[derive(Debug, Deserialize)]
struct ApiCategory {
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiParent {
    key: String,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiChangelog {
    #[serde(default)]
    total: usize,
    #[serde(default)]
    histories: Vec<ApiHistory>,
}

#[derive(Debug, Deserialize)]
struct ApiHistory {
    created: String,
    #[serde(default)]
    items: Vec<ApiItem>,
}

#[derive(Debug, Deserialize)]
struct ApiItem {
    field: String,
}

#[derive(Debug, Deserialize)]
struct ServerInfo {
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Myself {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    /// The user name and key a worklog's author is known by.
    name: Option<String>,
    key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ApiWorklogs {
    #[serde(default)]
    total: usize,
    #[serde(default)]
    worklogs: Vec<ApiWorklog>,
}

#[derive(Debug, Deserialize)]
struct ApiWorklog {
    author: Option<ApiAuthor>,
    started: Option<String>,
    #[serde(rename = "timeSpentSeconds", default)]
    seconds: u64,
}

#[derive(Debug, Deserialize)]
struct ApiAuthor {
    name: Option<String>,
    key: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorklogSearch {
    #[serde(default)]
    total: usize,
    #[serde(default)]
    issues: Vec<WorklogIssue>,
}

#[derive(Debug, Deserialize)]
struct WorklogIssue {
    key: String,
    #[serde(default)]
    fields: WorklogFields,
}

#[derive(Debug, Default, Deserialize)]
struct WorklogFields {
    worklog: Option<ApiWorklogs>,
}

/// A ticket as its page shows it: what a row has, and the rest.
#[derive(Debug, Deserialize)]
struct ApiIssueDetail {
    key: String,
    #[serde(default)]
    fields: ApiDetailFields,
}

#[derive(Debug, Default, Deserialize)]
struct ApiDetailFields {
    #[serde(flatten)]
    base: ApiFields,
    description: Option<String>,
    assignee: Option<ApiUser>,
    comment: Option<ApiComments>,
    #[serde(default)]
    subtasks: Option<Vec<ApiSubtask>>,
    #[serde(default)]
    issuelinks: Option<Vec<ApiLink>>,
}

#[derive(Debug, Deserialize)]
struct ApiComments {
    #[serde(default)]
    comments: Vec<ApiComment>,
}

#[derive(Debug, Deserialize)]
struct ApiComment {
    author: Option<ApiAuthor>,
    created: Option<String>,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct ApiSubtask {
    key: String,
    #[serde(default)]
    fields: ApiLinkedFields,
}

#[derive(Debug, Default, Deserialize)]
struct ApiLinkedFields {
    summary: Option<String>,
    status: Option<ApiStatus>,
}

#[derive(Debug, Deserialize)]
struct ApiLink {
    #[serde(rename = "type")]
    kind: Option<ApiLinkType>,
    #[serde(rename = "inwardIssue")]
    inward: Option<ApiSubtask>,
    #[serde(rename = "outwardIssue")]
    outward: Option<ApiSubtask>,
}

#[derive(Debug, Deserialize)]
struct ApiLinkType {
    inward: Option<String>,
    outward: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Priority {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(rename = "errorMessages", default)]
    messages: Vec<String>,
    #[serde(default)]
    errors: HashMap<String, String>,
}

/// What one call can go wrong with — never with the token (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The status, and what Jira said about it when it said something.
    Http(u16, Option<String>),
    Connection(String),
    Body(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(401, _) => write!(f, "HTTP 401 · the token was refused — make one in Jira under Profile → Personal Access Tokens"),
            Error::Http(403, _) => write!(f, "HTTP 403 · the token may not read this"),
            // A JQL that names a status Jira does not have: its own words say which.
            Error::Http(400, Some(said)) => write!(f, "{said}"),
            Error::Http(code, Some(said)) => write!(f, "HTTP {code} · {said}"),
            Error::Http(code, None) => write!(f, "HTTP {code}"),
            Error::Connection(e) => write!(f, "{e}"),
            Error::Body(e) => write!(f, "{}{e}", super::redash::UNREADABLE),
        }
    }
}

/// What Jira's error answer says, in a line: `{"errorMessages": ["The value 'Review' does not
/// exist for the field 'status'."]}`.
fn said(body: &str) -> Option<String> {
    let error: ErrorBody = serde_json::from_str(body).ok()?;
    let mut parts = error.messages;
    let mut fields: Vec<(String, String)> = error.errors.into_iter().collect();
    fields.sort();
    parts.extend(fields.into_iter().map(|(field, message)| format!("{field}: {message}")));
    let line = parts.join(" · ");
    (!line.trim().is_empty()).then(|| line.chars().take(200).collect())
}

/// Why an answer is not what Jira sends, in words.
fn why_unreadable(body: &str, error: &serde_json::Error) -> String {
    if body.trim_start().starts_with('<') {
        return "a web page, not JSON — is the url Jira's own?".to_string();
    }
    error.to_string().chars().take(120).collect()
}

/// When a ticket moved into the status it is in: the last change of `status` in its history.
fn last_status_change(changelog: Option<&ApiChangelog>) -> Option<i64> {
    changelog?
        .histories
        .iter()
        .filter(|h| h.items.iter().any(|i| i.field == "status"))
        .filter_map(|h| unix(&h.created))
        .max()
}

/// A key as Jira makes them (`DATA-12647`) — nothing else is put into a search or a path.
fn is_key(key: &str) -> bool {
    match key.split_once('-') {
        Some((project, number)) => {
            !project.is_empty()
                && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn linked(how: String, issue: ApiSubtask) -> jira::Linked {
    jira::Linked {
        how,
        key: issue.key,
        summary: issue.fields.summary.unwrap_or_default(),
        status: issue.fields.status.map(|s| s.name).unwrap_or_default(),
    }
}

fn author_name(author: Option<ApiAuthor>) -> String {
    author.and_then(|a| a.display_name.or(a.name)).unwrap_or_else(|| "someone".to_string())
}

/// A ticket's answer as its page shows it.
fn issue_detail(api: ApiIssueDetail) -> jira::IssueDetail {
    let fields = api.fields;
    let base = ticket(ApiIssue { key: api.key.clone(), fields: fields.base, changelog: None }, None);
    let mut links = Vec::new();
    for link in fields.issuelinks.unwrap_or_default() {
        let kind = link.kind;
        if let Some(issue) = link.outward {
            links.push(linked(kind.as_ref().and_then(|k| k.outward.clone()).unwrap_or_else(|| "links to".into()), issue));
        } else if let Some(issue) = link.inward {
            links.push(linked(kind.as_ref().and_then(|k| k.inward.clone()).unwrap_or_else(|| "linked from".into()), issue));
        }
    }
    jira::IssueDetail {
        key: base.key,
        summary: base.summary,
        status: base.status,
        kind: base.kind,
        priority: base.priority,
        assignee: fields.assignee.and_then(|a| a.display_name.or(a.name)),
        reporter: base.reporter,
        created: base.created,
        updated: base.updated,
        due: base.due,
        labels: base.labels,
        parent: base.parent,
        logged_s: base.logged_s,
        description: fields.description.unwrap_or_default(),
        subtasks: fields.subtasks.unwrap_or_default().into_iter().map(|t| linked("sub-task".into(), t)).collect(),
        links,
        comments: fields
            .comment
            .map(|c| c.comments)
            .unwrap_or_default()
            .into_iter()
            .map(|c| jira::Comment { author: author_name(c.author), at: c.created.as_deref().and_then(unix), body: c.body })
            .collect(),
    }
}

/// One GET of Jira's API with the token, as every read here makes it.
async fn fetch<T: DeserializeOwned>(client: &reqwest::Client, base_url: &str, token: &str, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
    let response = client
        .get(format!("{base_url}{path}"))
        .query(query)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| Error::Connection(transport(e, REQUEST_TIMEOUT)))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| Error::Connection(transport(e, REQUEST_TIMEOUT)))?;
    if !status.is_success() {
        return Err(Error::Http(status.as_u16(), said(&body)));
    }
    serde_json::from_str(&body).map_err(|e| Error::Body(why_unreadable(&body, &e)))
}

/// What reads a ticket in full when its page asks: the source's client and token, apart from the
/// task that polls the board, so a page does not wait for a poll.
#[derive(Clone)]
pub struct JiraDetails {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl JiraDetails {
    pub async fn issue(&self, key: &str) -> Result<jira::IssueDetail, String> {
        if !is_key(key) {
            return Err(format!("{key} is not a ticket's key"));
        }
        let fields = "summary,status,priority,issuetype,created,updated,resolutiondate,duedate,parent,labels,reporter,timespent,description,assignee,comment,subtasks,issuelinks";
        let path = format!("/rest/api/2/issue/{key}");
        fetch::<ApiIssueDetail>(&self.client, &self.base_url, &self.token, &path, &[("fields", fields)])
            .await
            .map(issue_detail)
            .map_err(|e| e.to_string())
    }
}

fn ticket(issue: ApiIssue, ranks: Option<&HashMap<String, u32>>) -> Ticket {
    let fields = issue.fields;
    let priority = fields.priority.and_then(|p| p.name).map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let (status, category) = match fields.status {
        Some(status) => (status.name, status.category.and_then(|c| c.key).unwrap_or_default()),
        None => (String::new(), String::new()),
    };
    Ticket {
        key: issue.key,
        summary: fields.summary.unwrap_or_default().trim().to_string(),
        status,
        category,
        kind: fields.issuetype.and_then(|t| t.name),
        priority_rank: priority.as_deref().and_then(|p| ranks?.get(&p.to_lowercase()).copied()),
        priority,
        created: fields.created.as_deref().and_then(unix),
        updated: fields.updated.as_deref().and_then(unix),
        resolved: fields.resolutiondate.as_deref().and_then(unix),
        due: fields.duedate.filter(|d| jira::day_number(d).is_some()),
        status_since: None,
        parent: fields.parent.map(|p| p.key),
        labels: fields.labels.unwrap_or_default(),
        reporter: fields.reporter.and_then(|r| r.display_name.or(r.name)),
        logged_s: fields.timespent,
    }
}

pub struct JiraSource {
    base_url: String,
    token: String,
    client: reqwest::Client,
    statuses: Vec<String>,
    done_days: u32,
    /// Asked once: the version, whose token it is, the priorities in Jira's order.
    version: Option<String>,
    user: Option<String>,
    ranks: Option<HashMap<String, u32>>,
    /// By key: when the ticket was last updated as its history was read, and when it moved
    /// into its status.
    since: HashMap<String, (Option<i64>, Option<i64>)>,
    /// Who the token's owner is to a worklog: user name and key.
    me: Vec<String>,
    /// Your worklogs of the month, and when they were read.
    worklogs: Vec<jira::Worklog>,
    worklogs_read: Option<std::time::Instant>,
}

/// How often the month's worklogs are read again: they change when you log time.
const WORKLOGS_EVERY: Duration = Duration::from_secs(300);

impl JiraSource {
    /// `None` when Jira is not configured: the view then says how to, instead of pretending
    /// there are no tickets.
    pub fn new(config: &JiraConfig) -> Option<JiraSource> {
        let base_url = config.url.as_deref()?.trim_end_matches('/').to_string();
        let token = config.token.clone()?;
        let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().ok()?;
        Some(JiraSource {
            base_url,
            token,
            client,
            statuses: config.statuses.clone(),
            done_days: config.done_days,
            version: None,
            user: None,
            ranks: None,
            since: HashMap::new(),
            me: Vec::new(),
            worklogs: Vec::new(),
            worklogs_read: None,
        })
    }

    /// What reads a ticket in full for its page.
    pub fn details(&self) -> JiraDetails {
        JiraDetails { base_url: self.base_url.clone(), token: self.token.clone(), client: self.client.clone() }
    }

    /// Read until the loop ends: every minute, and at once when asked. Errors are shown, never
    /// fatal.
    pub async fn run(mut self, tx: tokio::sync::mpsc::UnboundedSender<Event>, mut refresh: tokio::sync::mpsc::UnboundedReceiver<()>) {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let asked = tokio::select! {
                _ = tick.tick() => false,
                Some(()) = refresh.recv() => {
                    while refresh.try_recv().is_ok() {}
                    tick.reset();
                    true
                }
            };
            if asked {
                self.worklogs_read = None;
            }
            // The board first; the month's time after it, when it is due — it is the slow part.
            let mut board = self.board().await;
            if tx.send(Event::Jira(Box::new(board.clone()))).is_err() {
                return;
            }
            if self.time_due() && self.time(&mut board).await && tx.send(Event::Jira(Box::new(board))).is_err() {
                return;
            }
        }
    }

    /// One read of the board.
    pub async fn board(&mut self) -> Board {
        // Asked once, and together: which Jira, whose token, the priorities in Jira's order.
        if self.user.is_none() {
            let (info, me, priorities) = tokio::join!(
                self.get::<ServerInfo>("/rest/api/2/serverInfo", &[]),
                self.get::<Myself>("/rest/api/2/myself", &[]),
                self.get::<Vec<Priority>>("/rest/api/2/priority", &[]),
            );
            self.version = info.ok().and_then(|i| i.version);
            match me {
                Ok(me) => {
                    self.user = me.display_name.or_else(|| Some(String::new()));
                    self.me = [me.name, me.key].into_iter().flatten().collect();
                }
                Err(e) => return self.failed(e),
            }
            if let Ok(list) = priorities {
                self.ranks = Some(list.into_iter().enumerate().map(|(i, p)| (p.name.to_lowercase(), i as u32)).collect());
            }
        }

        let jql = jira::jql(&self.statuses, self.done_days);
        let limit = PAGE.to_string();
        let mut issues: Vec<ApiIssue> = Vec::new();
        for page in 0..MAX_PAGES {
            let start = (page * PAGE).to_string();
            let query = [("jql", jql.as_str()), ("fields", FIELDS), ("maxResults", &limit), ("startAt", &start)];
            let search: Search = match self.get("/rest/api/2/search", &query).await {
                Ok(search) => search,
                Err(e) => return self.failed(e),
            };
            let got = search.issues.len();
            issues.extend(search.issues);
            if got < PAGE || issues.len() >= search.total {
                break;
            }
        }
        let mut tickets: Vec<Ticket> = issues.into_iter().map(|issue| ticket(issue, self.ranks.as_ref())).collect();
        self.read_since(&mut tickets).await;
        Board {
            reachable: true,
            error: None,
            base_url: Some(self.base_url.clone()),
            version: self.version.clone(),
            user: self.user.clone(),
            statuses: self.statuses.clone(),
            done_days: self.done_days,
            tickets,
            worklogs: self.worklogs.clone(),
            worklogs_read: self.worklogs_read.is_some(),
            taken_at: SystemTime::now(),
        }
    }

    /// Whether the month's time is to be read: every few minutes, at once after `r`.
    pub fn time_due(&self) -> bool {
        self.worklogs_read.is_none_or(|t| t.elapsed() >= WORKLOGS_EVERY)
    }

    /// The month's time read into `board`; whether it was. One that fails keeps the last.
    pub async fn time(&mut self, board: &mut Board) -> bool {
        if !board.reachable {
            return false;
        }
        let Ok(worklogs) = self.read_worklogs().await else {
            return false;
        };
        self.worklogs = worklogs;
        self.worklogs_read = Some(std::time::Instant::now());
        board.worklogs = self.worklogs.clone();
        board.worklogs_read = true;
        true
    }

    /// Your worklogs since the 1st of the month, on any ticket: the tickets you logged on (a
    /// search), then each one's worklogs when the search did not bring them all.
    async fn read_worklogs(&self) -> Result<Vec<jira::Worklog>, Error> {
        let jql = "worklogAuthor = currentUser() AND worklogDate >= startOfMonth(-1d)";
        let limit = PAGE.to_string();
        let mut issues = Vec::new();
        for page in 0..MAX_PAGES {
            let start = (page * PAGE).to_string();
            let query = [("jql", jql), ("fields", "worklog"), ("maxResults", &limit), ("startAt", &start)];
            let search: WorklogSearch = self.get("/rest/api/2/search", &query).await?;
            let got = search.issues.len();
            issues.extend(search.issues);
            if got < PAGE || issues.len() >= search.total {
                break;
            }
        }
        let mut out = Vec::new();
        for issue in issues {
            let mut logs = issue.fields.worklog.unwrap_or_default();
            if logs.worklogs.len() < logs.total && is_key(&issue.key) {
                let path = format!("/rest/api/2/issue/{}/worklog", issue.key);
                if let Ok(all) = self.get::<ApiWorklogs>(&path, &[]).await {
                    logs = all;
                }
            }
            for log in logs.worklogs {
                let mine = log.author.as_ref().is_some_and(|a| {
                    [a.name.as_deref(), a.key.as_deref()].into_iter().flatten().any(|n| self.me.iter().any(|m| m == n))
                });
                let day = log.started.as_deref().and_then(|s| s.get(..10)).and_then(jira::day_number);
                if let (true, Some(day)) = (mine, day) {
                    out.push(jira::Worklog { key: issue.key.clone(), day, seconds: log.seconds });
                }
            }
        }
        Ok(out)
    }

    fn failed(&self, error: Error) -> Board {
        let mut board = Board::unreachable(error.to_string());
        board.base_url = Some(self.base_url.clone());
        board.version = self.version.clone();
        board.user = self.user.clone();
        board.statuses = self.statuses.clone();
        board.done_days = self.done_days;
        board
    }

    /// When each ticket moved into its status, from its history: read for the tickets that are
    /// new or changed since it was last read, fifty to a search. A search that fails leaves them
    /// for the next read; meanwhile the screen counts from when the ticket was made.
    async fn read_since(&mut self, tickets: &mut [Ticket]) {
        self.since.retain(|key, _| tickets.iter().any(|t| &t.key == key));
        let stale: Vec<(String, Option<i64>)> = tickets
            .iter()
            .filter(|t| is_key(&t.key) && self.since.get(&t.key).is_none_or(|(updated, _)| *updated != t.updated))
            .map(|t| (t.key.clone(), t.updated))
            .collect();
        for chunk in stale.chunks(50) {
            let keys: Vec<&str> = chunk.iter().map(|(key, _)| key.as_str()).collect();
            let jql = format!("key in ({})", keys.join(", "));
            let limit = PAGE.to_string();
            let query = [("jql", jql.as_str()), ("fields", "status"), ("expand", "changelog"), ("maxResults", &limit)];
            let Ok(search) = self.get::<Search>("/rest/api/2/search", &query).await else {
                continue;
            };
            for issue in search.issues {
                // A search hands over a history up to a limit; a longer one is read whole.
                let cut = issue.changelog.as_ref().is_some_and(|c| c.histories.len() < c.total);
                let since = if cut {
                    let path = format!("/rest/api/2/issue/{}", issue.key);
                    match self.get::<ApiIssue>(&path, &[("fields", "status"), ("expand", "changelog")]).await {
                        Ok(whole) => last_status_change(whole.changelog.as_ref()),
                        Err(_) => last_status_change(issue.changelog.as_ref()),
                    }
                } else {
                    last_status_change(issue.changelog.as_ref())
                };
                let updated = chunk.iter().find(|(key, _)| *key == issue.key).and_then(|(_, updated)| *updated);
                self.since.insert(issue.key, (updated, since));
            }
        }
        for ticket in tickets {
            ticket.status_since = self.since.get(&ticket.key).and_then(|(_, since)| *since);
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, Error> {
        fetch(&self.client, &self.base_url, &self.token, path, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search answer shaped like Jira 9.12's, cut down to two tickets.
    const SEARCH: &str = r#"{
      "expand": "names,schema", "startAt": 0, "maxResults": 100, "total": 2,
      "issues": [
        { "id": "1", "key": "DATA-12647",
          "fields": {
            "summary": "DE - Client account balances as a daily PostHog warehouse table ",
            "status": { "name": "In Review", "statusCategory": { "key": "indeterminate", "name": "In Progress" } },
            "priority": { "name": "ASAP", "id": "10001" },
            "issuetype": { "name": "Task" },
            "created": "2026-09-12T09:01:02.000+0300",
            "updated": "2026-10-02T04:25:00.000+0300",
            "resolutiondate": null,
            "duedate": "2026-10-09",
            "labels": [],
            "reporter": { "name": "j.petrova", "displayName": "Jurgita Petrova" },
            "timespent": 25920
          } },
        { "id": "2", "key": "DATA-12650",
          "fields": {
            "summary": "DE - Add all gateway.bank_account columns",
            "status": { "name": "Done", "statusCategory": { "key": "done" } },
            "priority": null,
            "issuetype": { "name": "Code review" },
            "created": "2026-09-30T10:00:00.000+0300",
            "updated": "2026-10-01T05:44:00.000+0300",
            "resolutiondate": "2026-10-01T05:44:00.000+0300",
            "duedate": null,
            "parent": { "key": "DATA-12639" },
            "labels": null,
            "timespent": null
          } }
      ] }"#;

    #[test]
    fn a_search_answer_becomes_tickets() {
        let search: Search = serde_json::from_str(SEARCH).expect("parses");
        assert_eq!(search.total, 2);
        let ranks: HashMap<String, u32> = [("urgent".to_string(), 0), ("asap".to_string(), 1)].into_iter().collect();
        let tickets: Vec<Ticket> = search.issues.into_iter().map(|i| ticket(i, Some(&ranks))).collect();
        let review = &tickets[0];
        assert_eq!(review.key, "DATA-12647");
        assert_eq!(review.summary, "DE - Client account balances as a daily PostHog warehouse table");
        assert_eq!((review.status.as_str(), review.category.as_str()), ("In Review", "indeterminate"));
        assert_eq!((review.priority.as_deref(), review.priority_rank), (Some("ASAP"), Some(1)));
        assert_eq!(review.due.as_deref(), Some("2026-10-09"));
        assert_eq!(review.reporter.as_deref(), Some("Jurgita Petrova"));
        assert_eq!(review.logged_s, Some(25_920));
        assert_eq!(review.updated, unix("2026-10-02T01:25:00Z"));
        let done = &tickets[1];
        assert_eq!((done.priority.as_deref(), done.priority_rank), (None, None));
        assert_eq!(done.parent.as_deref(), Some("DATA-12639"));
        assert!(done.labels.is_empty());
        assert_eq!(done.resolved, unix("2026-10-01T02:44:00Z"));
    }

    #[test]
    fn the_status_since_is_the_last_status_change_in_the_history() {
        let changelog: ApiChangelog = serde_json::from_str(
            r#"{ "startAt": 0, "maxResults": 3, "total": 3, "histories": [
                { "created": "2026-09-24T10:59:00.000+0300", "items": [ { "field": "status", "fromString": "Backlog", "toString": "In progress" } ] },
                { "created": "2026-10-02T04:25:00.000+0300", "items": [ { "field": "status", "fromString": "In progress", "toString": "In Review" } ] },
                { "created": "2026-10-03T12:00:00.000+0300", "items": [ { "field": "timespent" }, { "field": "WorklogId" } ] }
            ] }"#,
        )
        .expect("parses");
        assert_eq!(last_status_change(Some(&changelog)), unix("2026-10-02T01:25:00Z"), "a worklog after it is not a move");
        assert_eq!(last_status_change(None), None);
    }

    #[test]
    fn jira_s_own_words_say_what_is_wrong() {
        let body = r#"{"errorMessages":["The value 'Review' does not exist for the field 'status'."],"errors":{}}"#;
        let error = Error::Http(400, said(body));
        assert_eq!(error.to_string(), "The value 'Review' does not exist for the field 'status'.");
        assert!(Error::Http(401, None).to_string().contains("Personal Access Tokens"));
        assert_eq!(Error::Http(502, said("<html>bad gateway</html>")).to_string(), "HTTP 502");
        assert_eq!(said(r#"{"errorMessages":[],"errors":{"jql":"bad"}}"#).as_deref(), Some("jql: bad"));
    }

    #[test]
    fn only_a_key_goes_into_a_search() {
        assert!(is_key("DATA-12647"));
        assert!(is_key("TIME-3509"));
        assert!(!is_key("DATA-12647) OR (1=1"));
        assert!(!is_key("DATA"));
        assert!(!is_key("-12"));
    }

    #[test]
    fn a_ticket_in_full_reads_its_description_links_and_comments() {
        let api: ApiIssueDetail = serde_json::from_str(
            r#"{ "key": "DATA-12647", "fields": {
                "summary": "Client balances", "status": { "name": "In Review" }, "priority": { "name": "ASAP" },
                "description": "h3. Asked for\n* a table", "assignee": { "name": "a.d", "displayName": "Abdul Djafar" },
                "subtasks": [ { "key": "DATA-12660", "fields": { "summary": "Code review", "status": { "name": "Done" } } } ],
                "issuelinks": [ { "type": { "inward": "is blocked by", "outward": "blocks" },
                                  "outwardIssue": { "key": "DATA-1", "fields": { "summary": "Dashboards", "status": { "name": "Backlog" } } } } ],
                "comment": { "comments": [ { "author": { "displayName": "Ana K" }, "created": "2026-10-02T09:00:00.000+0300", "body": "Looks good" } ] }
            } }"#,
        )
        .expect("parses");
        let detail = issue_detail(api);
        assert_eq!((detail.status.as_str(), detail.assignee.as_deref()), ("In Review", Some("Abdul Djafar")));
        assert_eq!(detail.description, "h3. Asked for\n* a table");
        assert_eq!((detail.subtasks[0].key.as_str(), detail.subtasks[0].status.as_str()), ("DATA-12660", "Done"));
        assert_eq!((detail.links[0].how.as_str(), detail.links[0].key.as_str()), ("blocks", "DATA-1"));
        assert_eq!((detail.comments[0].author.as_str(), detail.comments[0].at), ("Ana K", unix("2026-10-02T06:00:00Z")));
    }

    #[test]
    fn a_worklog_answer_has_its_author_day_and_time() {
        let search: WorklogSearch = serde_json::from_str(
            r#"{ "total": 1, "issues": [ { "key": "DATA-12788", "fields": { "worklog": { "total": 1, "worklogs": [
                { "author": { "name": "abdul.djafar@example.net", "key": "JIRAUSER1" }, "started": "2026-10-02T00:00:00.000+0000", "timeSpentSeconds": 10800 } ] } } } ] }"#,
        )
        .expect("parses");
        let log = &search.issues[0].fields.worklog.as_ref().unwrap().worklogs[0];
        assert_eq!(log.seconds, 10_800);
        assert_eq!(log.started.as_deref().and_then(|s| s.get(..10)).and_then(jira::day_number), jira::day_number("2026-10-02"));
    }
}
