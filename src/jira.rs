//! Jira, view 6: the tickets assigned to you, in your board's columns — what is in progress, in
//! review, waiting for feedback, and what was finished this week.
//!
//! Pure, like `model.rs`: what the source read (`sources/jira.rs`), put in columns and judged.
//! Nothing here does I/O; `ui/jira.rs` draws it.

use crate::severity::Severity;
use std::time::{Duration, SystemTime};

/// The columns asked for when nothing else is said: the Data platform board's, left to right,
/// without its Backlog, Selected for Development and On Hold.
pub const DEFAULT_STATUSES: [&str; 4] = ["In progress", "In Review", "Feedback", "Done"];

/// How far back the last column reaches, in days: the board hides what was finished more than
/// a week ago.
pub const DEFAULT_DONE_DAYS: u32 = 7;

/// The board's error when Jira is not configured (it is optional, like Redash).
pub const NOT_CONFIGURED: &str = "not configured";
/// Before the first answer.
pub const NOT_READ: &str = "not read yet";

/// One ticket, as the screen needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ticket {
    pub key: String,
    pub summary: String,
    pub status: String,
    /// Jira's category of the status: `new`, `indeterminate` or `done`.
    pub category: String,
    /// The issue type: `Task`, `Story`, `Bug`, `Code review`.
    pub kind: Option<String>,
    pub priority: Option<String>,
    /// Where the priority stands in Jira's own list, highest first; `None` when unknown.
    pub priority_rank: Option<u32>,
    /// Unix seconds.
    pub created: Option<i64>,
    pub updated: Option<i64>,
    pub resolved: Option<i64>,
    /// The due date as Jira keeps it: a day, `YYYY-MM-DD`.
    pub due: Option<String>,
    /// When it moved into the status it is in, from its history.
    pub status_since: Option<i64>,
    pub parent: Option<String>,
    pub labels: Vec<String>,
    pub reporter: Option<String>,
    /// Time logged on it, in seconds.
    pub logged_s: Option<u64>,
}

impl Ticket {
    /// The due date as days since the epoch.
    pub fn due_day(&self) -> Option<i64> {
        day_number(self.due.as_deref()?)
    }

    /// How long it has been in its status: since it moved there, else since it was made.
    pub fn in_status(&self, now: i64) -> Option<i64> {
        self.status_since.or(self.created).map(|at| (now - at).max(0))
    }

    /// Red when the priority says it cannot wait (`URGENT`, `ASAP`, `Highest`, `Blocker`).
    pub fn priority_severity(&self) -> Severity {
        match self.priority.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("urgent" | "asap" | "highest" | "blocker" | "critical") => Severity::Crit,
            _ => Severity::None,
        }
    }

    /// Whether Jira gave it no priority worth the name.
    pub fn priority_unset(&self) -> bool {
        matches!(
            self.priority.as_deref().map(str::to_ascii_lowercase).as_deref(),
            None | Some("unspecified" | "none" | "undefined" | "")
        )
    }
}

/// What one read of Jira found.
#[derive(Debug, Clone)]
pub struct Board {
    /// false → nothing has been read (yet); the view says why.
    pub reachable: bool,
    /// Why it could not be read — with `reachable` still true when what is shown is the last
    /// read that worked.
    pub error: Option<String>,
    /// `https://jira.example.net`, for the links.
    pub base_url: Option<String>,
    pub version: Option<String>,
    /// Whose tickets these are: the token's owner, by name.
    pub user: Option<String>,
    /// The columns, left to right. The last is where finished tickets go.
    pub statuses: Vec<String>,
    pub done_days: u32,
    pub tickets: Vec<Ticket>,
    /// Your worklogs of the month, on any ticket; read every few minutes.
    pub worklogs: Vec<Worklog>,
    pub worklogs_read: bool,
    pub taken_at: SystemTime,
}

/// One column of the board and its tickets, in the order they are drawn.
#[derive(Debug, Clone)]
pub struct Column<'a> {
    pub status: &'a str,
    /// The last column: finished tickets, those of the last `done_days` days only.
    pub finished: bool,
    pub tickets: Vec<&'a Ticket>,
}

impl Board {
    pub fn unreachable(error: impl Into<String>) -> Board {
        Board {
            reachable: false,
            error: Some(error.into()),
            base_url: None,
            version: None,
            user: None,
            statuses: DEFAULT_STATUSES.iter().map(|s| s.to_string()).collect(),
            done_days: DEFAULT_DONE_DAYS,
            tickets: Vec::new(),
            worklogs: Vec::new(),
            worklogs_read: false,
            taken_at: SystemTime::now(),
        }
    }

    /// Not an outage: not configured, or not read yet.
    pub fn is_placeholder(&self) -> bool {
        !self.reachable && matches!(self.error.as_deref(), Some(NOT_CONFIGURED) | Some(NOT_READ))
    }

    pub fn host(&self) -> Option<String> {
        self.base_url.as_deref().map(crate::sources::host_of)
    }

    pub fn age(&self, now: SystemTime) -> Option<Duration> {
        now.duration_since(self.taken_at).ok()
    }

    /// The ticket's page in Jira.
    pub fn link(&self, key: &str) -> Option<String> {
        let base = self.base_url.as_deref()?.trim_end_matches('/');
        Some(format!("{base}/browse/{key}"))
    }

    /// The columns with their tickets: by priority as the board ranks them, the latest touched
    /// first among equals; the finished column by when each was finished.
    pub fn columns(&self) -> Vec<Column<'_>> {
        let last = self.statuses.len().saturating_sub(1);
        self.statuses
            .iter()
            .enumerate()
            .map(|(i, status)| {
                let finished = i == last && self.statuses.len() > 1;
                let mut tickets: Vec<&Ticket> = self.tickets.iter().filter(|t| t.status.eq_ignore_ascii_case(status)).collect();
                if finished {
                    tickets.sort_by(|a, b| b.resolved.or(b.updated).cmp(&a.resolved.or(a.updated)).then_with(|| a.key.cmp(&b.key)));
                } else {
                    tickets.sort_by(|a, b| {
                        a.priority_rank
                            .unwrap_or(u32::MAX)
                            .cmp(&b.priority_rank.unwrap_or(u32::MAX))
                            .then_with(|| b.updated.cmp(&a.updated))
                            .then_with(|| a.key.cmp(&b.key))
                    });
                }
                Column { status, finished, tickets }
            })
            .collect()
    }

    /// Tickets in a status no column names — moved on since the query ran. Shown, never dropped.
    pub fn others(&self) -> Vec<&Ticket> {
        let mut others: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| !self.statuses.iter().any(|s| t.status.eq_ignore_ascii_case(s)))
            .collect();
        others.sort_by_key(|t| std::cmp::Reverse(t.updated));
        others
    }

    /// Every ticket in the order drawn: what the cursor walks and a click lands on.
    pub fn rows(&self) -> Vec<&Ticket> {
        let mut rows: Vec<&Ticket> = self.columns().into_iter().flat_map(|c| c.tickets).collect();
        rows.extend(self.others());
        rows
    }
}

/// The search for the board: the tickets assigned to you in its columns, the last column only
/// for what was finished in the last `done_days` days — or is not finished at all, so a last
/// column that is not a done one still shows what is in it.
pub fn jql(statuses: &[String], done_days: u32) -> String {
    let quoted: Vec<String> = statuses.iter().map(|s| quote(s)).collect();
    let recent = |status: &str| {
        if done_days == 0 {
            format!("status = {status}")
        } else {
            format!("(status = {status} AND (resolved >= -{done_days}d OR resolution is EMPTY))")
        }
    };
    let clause = match quoted.split_last() {
        None => return "assignee = currentUser() ORDER BY updated DESC".to_string(),
        Some((last, [])) => recent(last),
        Some((last, rest)) => format!("(status in ({}) OR {})", rest.join(", "), recent(last)),
    };
    format!("assignee = currentUser() AND {clause} ORDER BY updated DESC")
}

/// A JQL string: in double quotes, `"` and `\` escaped.
pub fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `2026-10-12` → days since the epoch.
pub fn day_number(date: &str) -> Option<i64> {
    let day = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
    Some((day - epoch).num_days())
}

/// Today in the zone shown, as days since the epoch.
pub fn today(now: i64, offset_s: i64) -> i64 {
    (now + offset_s).div_euclid(86_400)
}

/// A due date against today: overdue is red, today or tomorrow amber. A finished ticket's due
/// date is history, and judged by nothing.
pub fn due_severity(due_day: i64, today: i64, finished: bool) -> Severity {
    if finished {
        return Severity::None;
    }
    match due_day - today {
        d if d < 0 => Severity::Crit,
        0 | 1 => Severity::Warn,
        _ => Severity::None,
    }
}

/// `overdue 3d`, `due today`, `due tomorrow`, `due Fri`, `due Oct 12`.
pub fn due_label(due_day: i64, today: i64) -> String {
    let days = due_day - today;
    match days {
        d if d < 0 => format!("overdue {}d", -d),
        0 => "due today".to_string(),
        1 => "due tomorrow".to_string(),
        _ => {
            let date = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).and_then(|e| e.checked_add_days(chrono::Days::new(due_day.max(0) as u64)));
            match date {
                Some(date) if days < 7 => format!("due {}", date.format("%a")),
                Some(date) => format!("due {}", date.format("%b %-d")),
                None => "due —".to_string(),
            }
        }
    }
}

/// `DE - Reload mv_statement_daily_agg…` → `("DE", "Reload mv_statement_daily_agg…")`: the
/// team's tag in front of a summary, which the screen draws faint so the words stand out.
pub fn split_tag(summary: &str) -> (Option<&str>, &str) {
    let text = summary.trim_start();
    let tag_len = text.chars().take_while(char::is_ascii_uppercase).count();
    if !(2..=4).contains(&tag_len) {
        return (None, text);
    }
    let (tag, rest) = text.split_at(tag_len);
    let after = rest.trim_start();
    let Some(dash) = after.chars().next().filter(|c| matches!(c, '-' | '–' | '—' | ':')) else {
        return (None, text);
    };
    let rest = after[dash.len_utf8()..].trim_start();
    if rest.is_empty() {
        return (None, text);
    }
    (Some(tag), rest)
}

/// Time logged, as the board says it: `45m`, `2h`, `7h12m`, `3d4h` (8-hour days, as Jira counts).
pub fn logged(seconds: u64) -> String {
    let minutes = seconds / 60;
    let (days, hours, mins) = (minutes / (8 * 60), (minutes % (8 * 60)) / 60, minutes % 60);
    match (days, hours, mins) {
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h{m:02}m"),
        (d, 0, _) => format!("{d}d"),
        (d, h, _) => format!("{d}d{h}h"),
    }
}

/// How long, in the fewest characters: `40m`, `5h`, `3d`, `11w`.
pub fn age(seconds: i64) -> String {
    let s = seconds.max(0);
    match s {
        0..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..1_209_600 => format!("{}d", s / 86_400),
        _ => format!("{}w", s / 604_800),
    }
}

// -- time logged this month ----------------------------------------------------------------

/// A worklog of yours: on which ticket, on which day as it was logged, how long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worklog {
    pub key: String,
    /// The ticket's summary, for a ticket that may not be on the board.
    pub summary: String,
    /// Days since the epoch: the date part of `started` as Jira wrote it, which is the day the
    /// time was logged for — not that moment moved into another zone.
    pub day: i64,
    pub seconds: u64,
}

/// The month around now, in the zone shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Month {
    /// Its first day, as days since the epoch.
    pub first: i64,
    pub days: u32,
    /// `October 2026`.
    pub name: String,
    /// Today's place in it, from 1.
    pub today: u32,
}

impl Month {
    pub fn of(now: i64, offset_s: i64) -> Month {
        use chrono::Datelike;
        let today = today(now, offset_s);
        let date = date_of(today);
        let first = chrono::NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
        let next = if date.month() == 12 {
            chrono::NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
        } else {
            chrono::NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
        }
        .unwrap_or(date);
        Month {
            first: today - i64::from(date.day0()),
            days: (next - first).num_days() as u32,
            name: date.format("%B %Y").to_string(),
            today: date.day(),
        }
    }

    /// Seconds logged on each day of the month, the 1st first.
    pub fn per_day(&self, worklogs: &[Worklog]) -> Vec<u64> {
        let mut days = vec![0u64; self.days as usize];
        for w in worklogs {
            let at = w.day - self.first;
            if (0..i64::from(self.days)).contains(&at) {
                days[at as usize] += w.seconds;
            }
        }
        days
    }

    /// The month's time by ticket: each ticket's hours day by day and in all, the most first.
    pub fn by_ticket(&self, worklogs: &[Worklog]) -> Vec<TicketTime> {
        let mut tickets: Vec<TicketTime> = Vec::new();
        for w in worklogs {
            let at = w.day - self.first;
            if !(0..i64::from(self.days)).contains(&at) {
                continue;
            }
            let i = match tickets.iter().position(|t| t.key == w.key) {
                Some(i) => i,
                None => {
                    tickets.push(TicketTime { key: w.key.clone(), summary: w.summary.clone(), days: vec![0; self.days as usize], total: 0 });
                    tickets.len() - 1
                }
            };
            tickets[i].days[at as usize] += w.seconds;
            tickets[i].total += w.seconds;
        }
        tickets.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.key.cmp(&b.key)));
        tickets
    }

    /// The date of a day of the month: `Thu 1 Oct`.
    pub fn day_name(&self, day: u32) -> String {
        date_of(self.first + i64::from(day) - 1).format("%a %-d %b").to_string()
    }

    /// Saturday or Sunday.
    pub fn is_weekend(&self, day: u32) -> bool {
        use chrono::Datelike;
        date_of(self.first + i64::from(day) - 1).weekday().number_from_monday() >= 6
    }

    /// The working days (Monday to Friday) from the 1st to today.
    pub fn working_days_so_far(&self) -> u32 {
        (1..=self.today).filter(|d| !self.is_weekend(*d)).count() as u32
    }
}

/// One ticket's time in the month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketTime {
    pub key: String,
    pub summary: String,
    /// Seconds on each day, the 1st first.
    pub days: Vec<u64>,
    pub total: u64,
}

fn date_of(day: i64) -> chrono::NaiveDate {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("the epoch");
    epoch.checked_add_signed(chrono::Duration::days(day)).unwrap_or(epoch)
}

/// Hours as a person says them: `7h`, `7h30m`, `0h`.
pub fn hours(seconds: u64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (h, 0) => format!("{h}h"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h{m:02}m"),
    }
}

// -- a ticket in full ---------------------------------------------------------------------

/// What ⏎ on a ticket shows: the ticket as Jira has it, read when asked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IssueDetail {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub kind: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub created: Option<i64>,
    pub updated: Option<i64>,
    pub due: Option<String>,
    pub labels: Vec<String>,
    pub parent: Option<String>,
    pub logged_s: Option<u64>,
    /// In Jira's wiki markup, as written.
    pub description: String,
    pub subtasks: Vec<Linked>,
    pub links: Vec<Linked>,
    pub comments: Vec<Comment>,
}

/// A ticket another one points at: `blocks DATA-12 · summary · status`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Linked {
    pub how: String,
    pub key: String,
    pub summary: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub author: String,
    pub at: Option<i64>,
    pub body: String,
}

/// What a line of wiki markup is, for the screen to draw it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prose {
    Heading,
    Bullet,
    Code,
    Text,
    Blank,
}

/// Jira's wiki markup made readable in a terminal: `h3.` a heading, `*` and `#` items bullets,
/// `{code}` and `{noformat}` blocks code, `{{x}}` and `*x*` their text, `[text|url]` its text and
/// where it goes. Each line with what it is; nothing is dropped but the markup.
pub fn prose(markup: &str) -> Vec<(Prose, String)> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in markup.replace("\r\n", "\n").lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        // A panel's title is a heading; the panel's own markup goes.
        if let Some(title) = panel_title(trimmed) {
            out.push((Prose::Heading, title));
        }
        let unboxed = strip_macros(trimmed);
        let trimmed = unboxed.trim();
        let fence = trimmed.starts_with("{code") || trimmed.starts_with("{noformat");
        if fence {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push((Prose::Code, line.to_string()));
            continue;
        }
        if trimmed.is_empty() {
            if !line.trim().is_empty() {
                // A line that was only markup leaves nothing behind.
                continue;
            }
            if out.last().is_some_and(|(kind, _)| *kind != Prose::Blank) {
                out.push((Prose::Blank, String::new()));
            }
            continue;
        }
        let heading = ["h1. ", "h2. ", "h3. ", "h4. ", "h5. ", "h6. "].iter().find_map(|h| trimmed.strip_prefix(h));
        if let Some(text) = heading {
            out.push((Prose::Heading, inline(text)));
            continue;
        }
        let marks = trimmed.chars().take_while(|c| *c == '*' || *c == '#' || *c == '-').count();
        if marks > 0 && trimmed[marks..].starts_with(' ') {
            let indent = "  ".repeat(marks - 1);
            out.push((Prose::Bullet, format!("{indent}• {}", inline(trimmed[marks..].trim_start()))));
            continue;
        }
        out.push((Prose::Text, inline(trimmed)));
    }
    while out.last().is_some_and(|(kind, _)| *kind == Prose::Blank) {
        out.pop();
    }
    out
}

/// `{panel:title=At a glance|borderStyle=solid}` → `At a glance`.
fn panel_title(line: &str) -> Option<String> {
    static PANEL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PANEL.get_or_init(|| regex::Regex::new(r"\{panel:[^}]*?title=([^|}]+)[^}]*\}").expect("a valid pattern"));
    re.captures(line).map(|c| c[1].trim().to_string()).filter(|t| !t.is_empty())
}

/// The block macros that only box or colour text — `{panel}`, `{quote}`, `{color:red}`, `{info}` —
/// and the empty braces Jira puts between a word and its markup (`{}`, `{_}`), taken out.
fn strip_macros(line: &str) -> String {
    static MACRO: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = MACRO.get_or_init(|| {
        regex::Regex::new(r"\{(?:panel|quote|color|expand|info|note|warning|tip|section|column)(?::[^}]*)?\}|\{[_*+\-^~?]?\}").expect("a valid pattern")
    });
    re.replace_all(line, "").into_owned()
}

/// The markup inside a line: `{{x}}` → `x`, `*x*` and `_x_` → `x`, `[text|url]` → `text (url)`,
/// `[url]` → `url`, and the emoticons a terminal can draw as a mark.
fn inline(text: &str) -> String {
    let mut out = text.replace("{{", "").replace("}}", "");
    for (face, mark) in [("(/)", "✔"), ("(x)", "✖"), ("(!)", "!"), ("(i)", "ⓘ"), ("(?)", "?"), ("(y)", "+"), ("(n)", "−"), ("(on)", "●"), ("(off)", "○")] {
        out = out.replace(face, mark);
    }
    // Links.
    let mut done = String::new();
    while let Some(open) = out.find('[') {
        let Some(close) = out[open..].find(']').map(|c| open + c) else {
            break;
        };
        done.push_str(&out[..open]);
        let inside = &out[open + 1..close];
        match inside.split_once('|') {
            Some((label, url)) => done.push_str(&format!("{label} ({url})")),
            None => done.push_str(inside.trim_start_matches('~')),
        }
        out = out[close + 1..].to_string();
    }
    done.push_str(&out);
    // Bold: `*word*` at word edges.
    let chars: Vec<char> = done.chars().collect();
    let mut plain = String::with_capacity(done.len());
    for (i, c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1);
        let mark = *c == '*' || *c == '_';
        let opens = mark && before.is_none_or(|b| !b.is_alphanumeric()) && after.is_some_and(|a| !a.is_whitespace() && *a != *c);
        let closes = mark && before.is_some_and(|b| !b.is_whitespace() && b != *c) && after.is_none_or(|a| !a.is_alphanumeric());
        if !(opens || closes) {
            plain.push(*c);
        }
    }
    plain
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(key: &str, status: &str) -> Ticket {
        Ticket { key: key.into(), summary: format!("DE - {key}"), status: status.into(), ..Ticket::default() }
    }

    fn board(tickets: Vec<Ticket>) -> Board {
        let mut b = Board::unreachable("x");
        b.reachable = true;
        b.error = None;
        b.tickets = tickets;
        b
    }

    #[test]
    fn the_search_is_yours_in_the_columns_and_the_last_one_only_for_the_week() {
        let statuses: Vec<String> = DEFAULT_STATUSES.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            jql(&statuses, 7),
            "assignee = currentUser() AND (status in (\"In progress\", \"In Review\", \"Feedback\") OR (status = \"Done\" AND (resolved >= -7d OR resolution is EMPTY))) ORDER BY updated DESC"
        );
        assert_eq!(jql(&statuses[..1], 0), "assignee = currentUser() AND status = \"In progress\" ORDER BY updated DESC");
        assert_eq!(jql(&[], 7), "assignee = currentUser() ORDER BY updated DESC");
        assert_eq!(quote(r#"Say "hi" \o/"#), r#""Say \"hi\" \\o/""#);
    }

    #[test]
    fn tickets_fall_into_their_columns_whatever_the_case() {
        let mut high = ticket("DATA-2", "In progress");
        high.priority_rank = Some(2);
        high.updated = Some(100);
        let mut asap = ticket("DATA-1", "IN PROGRESS");
        asap.priority_rank = Some(1);
        asap.updated = Some(50);
        let mut done_old = ticket("DATA-3", "Done");
        done_old.resolved = Some(10);
        let mut done_new = ticket("DATA-4", "Done");
        done_new.resolved = Some(20);
        let moved = ticket("DATA-5", "Selected for Development");
        let b = board(vec![high, asap, done_old, done_new, moved, ticket("DATA-6", "Feedback")]);
        let columns = b.columns();
        let keys = |c: &Column<'_>| c.tickets.iter().map(|t| t.key.to_string()).collect::<Vec<_>>();
        assert_eq!(columns.len(), 4);
        assert_eq!(keys(&columns[0]), ["DATA-1", "DATA-2"], "ASAP before High, as the board ranks them");
        assert!(columns[1].tickets.is_empty());
        assert_eq!(keys(&columns[2]), ["DATA-6"]);
        assert!(columns[3].finished && !columns[2].finished);
        assert_eq!(keys(&columns[3]), ["DATA-4", "DATA-3"], "the latest finished first");
        let rows: Vec<&str> = b.rows().iter().map(|t| t.key.as_str()).collect();
        assert_eq!(rows, ["DATA-1", "DATA-2", "DATA-6", "DATA-4", "DATA-3", "DATA-5"], "a ticket that moved on is still listed, last");
    }

    #[test]
    fn a_due_date_is_judged_against_today_unless_the_ticket_is_finished() {
        let today = day_number("2026-10-04").unwrap();
        assert_eq!(due_severity(today - 1, today, false), Severity::Crit);
        assert_eq!(due_severity(today, today, false), Severity::Warn);
        assert_eq!(due_severity(today + 1, today, false), Severity::Warn);
        assert_eq!(due_severity(today + 2, today, false), Severity::None);
        assert_eq!(due_severity(today - 9, today, true), Severity::None);
        assert_eq!(due_label(today - 3, today), "overdue 3d");
        assert_eq!(due_label(today, today), "due today");
        assert_eq!(due_label(today + 1, today), "due tomorrow");
        assert_eq!(due_label(today + 5, today), "due Fri", "2026-10-09 is a Friday");
        assert_eq!(due_label(today + 8, today), "due Oct 12");
        // 15:52 in Jakarta on the 4th is still the 4th there, whatever UTC says.
        assert_eq!(super::today(1_791_103_927, 7 * 3600), today);
    }

    #[test]
    fn the_team_tag_is_split_off_the_summary() {
        assert_eq!(split_tag("DE - Reload mv_statement_daily_agg"), (Some("DE"), "Reload mv_statement_daily_agg"));
        assert_eq!(split_tag(" DE -CBK DAG: immediate alerts"), (Some("DE"), "CBK DAG: immediate alerts"));
        assert_eq!(split_tag("DE — Implement checks"), (Some("DE"), "Implement checks"));
        assert_eq!(split_tag("DE  - replicate CreditOnline"), (Some("DE"), "replicate CreditOnline"));
        assert_eq!(split_tag("[Grafana migration] Data Team"), (None, "[Grafana migration] Data Team"));
        assert_eq!(split_tag("STATEMENT_DAILY_AGG: drift"), (None, "STATEMENT_DAILY_AGG: drift"));
        assert_eq!(split_tag("Test the middleware"), (None, "Test the middleware"));
    }

    #[test]
    fn time_is_said_in_a_few_characters() {
        assert_eq!(logged(45 * 60), "45m");
        assert_eq!(logged(2 * 3600), "2h");
        assert_eq!(logged(7 * 3600 + 12 * 60), "7h12m");
        assert_eq!(logged(25_920), "7h12m");
        assert_eq!(logged(3 * 8 * 3600 + 4 * 3600), "3d4h");
        assert_eq!(age(40 * 60), "40m");
        assert_eq!(age(5 * 3600), "5h");
        assert_eq!(age(3 * 86_400), "3d");
        assert_eq!(age(80 * 86_400), "11w");
    }

    #[test]
    fn urgent_and_asap_are_red_and_unspecified_is_no_priority() {
        let with = |p: &str| Ticket { priority: Some(p.into()), ..Ticket::default() };
        assert_eq!(with("ASAP").priority_severity(), Severity::Crit);
        assert_eq!(with("URGENT").priority_severity(), Severity::Crit);
        assert_eq!(with("High").priority_severity(), Severity::None);
        assert!(with("Unspecified").priority_unset());
        assert!(Ticket::default().priority_unset());
        assert!(!with("Low").priority_unset());
    }

    #[test]
    fn a_ticket_links_to_its_page() {
        let mut b = board(Vec::new());
        b.base_url = Some("https://jira.example.net/".into());
        assert_eq!(b.link("DATA-12647").as_deref(), Some("https://jira.example.net/browse/DATA-12647"));
    }

    #[test]
    fn the_month_is_counted_day_by_day_in_the_zone_shown() {
        // 2026-10-04 15:52 in Jakarta: the 4th of October, a Sunday.
        let month = Month::of(1_791_103_927, 7 * 3600);
        assert_eq!((month.days, month.today, month.name.as_str()), (31, 4, "October 2026"));
        assert_eq!(month.first, day_number("2026-10-01").unwrap());
        assert!(month.is_weekend(4) && month.is_weekend(3) && !month.is_weekend(2));
        assert_eq!(month.working_days_so_far(), 2, "Thursday the 1st and Friday the 2nd");
        let log = |date: &str, h: u64| Worklog { key: "DATA-1".into(), summary: String::new(), day: day_number(date).unwrap(), seconds: h * 3600 };
        let days = month.per_day(&[log("2026-10-01", 4), log("2026-10-01", 2), log("2026-10-02", 3), log("2026-09-30", 8)]);
        assert_eq!((days[0], days[1], days[2]), (6 * 3600, 3 * 3600, 0), "September's is not October's");
        assert_eq!(hours(27_000), "7h30m");
        let on = |key: &str, date: &str, h: u64| Worklog { key: key.into(), summary: format!("{key} work"), day: day_number(date).unwrap(), seconds: h * 3600 };
        let tickets = month.by_ticket(&[on("DATA-2", "2026-10-01", 2), on("DATA-1", "2026-10-01", 3), on("DATA-2", "2026-10-02", 4), on("DATA-3", "2026-09-30", 9)]);
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["DATA-2", "DATA-1"], "the most first; September's is not October's");
        assert_eq!((tickets[0].days[0], tickets[0].days[1], tickets[0].total), (7200, 14_400, 21_600));
        assert_eq!(month.day_name(1), "Thu 1 Oct");
        assert_eq!(hours(0), "0h");
        assert_eq!(hours(1800), "30m");
    }

    #[test]
    fn wiki_markup_reads_as_text() {
        let lines = prose("Asked by *Ana*, like {{clickhouse_transfers_daily}}.\n\nh3. Asked for\n\n* split by [client type|https://x.example/a]\n** savings apart\n{code:sql}\nSELECT 1\n{code}\n\n");
        let kinds: Vec<Prose> = lines.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, [Prose::Text, Prose::Blank, Prose::Heading, Prose::Blank, Prose::Bullet, Prose::Bullet, Prose::Code]);
        assert_eq!(lines[0].1, "Asked by Ana, like clickhouse_transfers_daily.");
        assert_eq!(lines[2].1, "Asked for");
        assert_eq!(lines[4].1, "• split by client type (https://x.example/a)");
        assert_eq!(lines[5].1, "  • savings apart");
        assert_eq!(lines[6].1, "SELECT 1");
        assert_eq!(prose("2 * 3 = 6")[0].1, "2 * 3 = 6", "a lone star is not bold");
        let real = prose("{panel:title=At a glance|borderStyle=solid|bgColor=#ffffff}\n* Target: by 25 Sep ({_}confirm{_})\n* (i) _Reporter to be changed later._{panel}\nOn the {}Atomic Portal{} via gateway_bank_account.");
        assert_eq!(real[0], (Prose::Heading, "At a glance".to_string()));
        assert_eq!(real[1].1, "• Target: by 25 Sep (confirm)");
        assert_eq!(real[2].1, "• ⓘ Reporter to be changed later.");
        assert_eq!(real[3].1, "On the Atomic Portal via gateway_bank_account.", "snake_case is not italic");
    }
}
