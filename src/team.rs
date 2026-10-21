//! The team's board (view 6, `d`): the open tickets of a Jira Structure whose summary starts with
//! the team's word — `DE - …` — in lanes of their status, the ones nobody has taken first; and a
//! ticket moved to another status by a transition Jira allows, after `y`.
//!
//! Pure, like the model: tickets and transitions in, lanes and asks out. It is read only while it
//! is on screen — once when it opens, then every minute — never per frame.
//!
//! The one write cobserve makes to Jira is here: `POST /rest/api/2/issue/{key}/transitions`,
//! sent only after the dialog that names the ticket, where it is and where it goes was answered
//! `y`. A transition whose screen asks for fields (a resolution) is not offered: `o` opens Jira.

use crate::jira::{Ticket, Transition};
use std::time::{Duration, SystemTime};

/// The workflow's open statuses, in the order a ticket goes through them (Data platform's).
const WORKFLOW: [&str; 6] = ["Backlog", "Selected for Development", "In Progress", "On Hold", "In Review", "Feedback"];

/// How often the board is read while it is on screen.
pub const READ_EVERY: Duration = Duration::from_secs(60);

/// Whose tickets the lanes show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Nobody has them yet: what can be taken.
    #[default]
    Untaken,
    /// The team's word, whoever has them.
    Everyone,
}

impl Filter {
    pub fn word(self) -> &'static str {
        match self {
            Filter::Untaken => "not taken",
            Filter::Everyone => "everyone's",
        }
    }
}

/// What the board asks the loop to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Read,
    Transitions(String),
    Move { key: String, id: String, from: String, to: String },
}

/// A ticket being moved: the moves Jira allows, the one chosen, and whether `y` is awaited.
#[derive(Debug, Clone, PartialEq)]
pub struct Moving {
    pub key: String,
    pub summary: String,
    pub from: String,
    /// `None` while they are read.
    pub transitions: Option<Result<Vec<Transition>, String>>,
    pub chosen: usize,
    pub confirm: bool,
}

impl Moving {
    /// The moves offered: not to where it already is.
    pub fn offered(&self) -> Vec<&Transition> {
        match &self.transitions {
            Some(Ok(list)) => list.iter().filter(|t| !t.to.eq_ignore_ascii_case(&self.from)).collect(),
            _ => Vec::new(),
        }
    }

    pub fn current(&self) -> Option<&Transition> {
        self.offered().get(self.chosen).copied()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Team {
    /// `JIRA_STRUCTURE` (216) and `JIRA_TEAM_PREFIX` (DE); no structure, no board.
    pub structure: Option<u64>,
    pub prefix: String,
    pub tickets: Vec<Ticket>,
    pub read_at: Option<SystemTime>,
    pub error: Option<String>,
    pub reading: bool,
    pub filter: Filter,
    /// The ticket under the cursor, by key.
    pub selected: Option<String>,
    pub moving: Option<Moving>,
    asks: Vec<Ask>,
}

/// A lane: a status, and its tickets in the order drawn.
pub struct Lane<'a> {
    pub status: String,
    pub tickets: Vec<&'a Ticket>,
    /// All of the team's in this status, whatever the filter.
    pub all: usize,
}

impl Team {
    pub fn new(structure: Option<u64>, prefix: &str) -> Team {
        Team { structure, prefix: prefix.to_string(), ..Team::default() }
    }

    pub fn shown(&self, t: &Ticket) -> bool {
        match self.filter {
            Filter::Untaken => t.assignee.is_none(),
            Filter::Everyone => true,
        }
    }

    /// The lanes in the workflow's order — the two a ticket is taken from always, the rest when
    /// the team has tickets in them — then any other status they are in. `extra` (your board's
    /// columns) names statuses the workflow here does not.
    pub fn lanes<'a>(&'a self, extra: &[String]) -> Vec<Lane<'a>> {
        let mut statuses: Vec<String> = WORKFLOW.iter().map(|s| s.to_string()).collect();
        for s in extra.iter().chain(self.tickets.iter().map(|t| &t.status)) {
            if !s.eq_ignore_ascii_case("done") && !statuses.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                statuses.push(s.clone());
            }
        }
        statuses
            .into_iter()
            .enumerate()
            .filter_map(|(k, status)| {
                let mine: Vec<&Ticket> = self.tickets.iter().filter(|t| t.status.eq_ignore_ascii_case(&status)).collect();
                let all = mine.len();
                (k < 2 || all > 0).then(|| Lane { status, tickets: mine.into_iter().filter(|t| self.shown(t)).collect(), all })
            })
            .collect()
    }

    /// The tickets in the order the cursor walks them: lane by lane.
    pub fn walk(&self, order: &[String]) -> Vec<String> {
        self.lanes(order).iter().flat_map(|l| l.tickets.iter().map(|t| t.key.clone())).collect()
    }

    pub fn current(&self) -> Option<&Ticket> {
        let key = self.selected.as_deref()?;
        self.tickets.iter().find(|t| t.key == key)
    }

    /// The cursor on a ticket still shown, or the first.
    pub fn settle(&mut self, order: &[String]) {
        let walk = self.walk(order);
        if self.selected.as_ref().is_none_or(|k| !walk.contains(k)) {
            self.selected = walk.first().cloned();
        }
    }

    pub fn step(&mut self, order: &[String], delta: isize) {
        let walk = self.walk(order);
        if walk.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|k| walk.iter().position(|w| w == k)).unwrap_or(0);
        let next = at.saturating_add_signed(delta).min(walk.len() - 1);
        self.selected = Some(walk[next].clone());
    }

    /// `← →`: the same place in the next lane that has tickets.
    pub fn cross(&mut self, order: &[String], delta: isize) {
        let lanes = self.lanes(order);
        let Some(key) = self.selected.clone() else { return };
        let Some(l) = lanes.iter().position(|l| l.tickets.iter().any(|t| t.key == key)) else { return };
        let row = lanes[l].tickets.iter().position(|t| t.key == key).unwrap_or(0);
        let mut next = l as isize + delta;
        while next >= 0 && (next as usize) < lanes.len() {
            let lane = &lanes[next as usize];
            if !lane.tickets.is_empty() {
                self.selected = Some(lane.tickets[row.min(lane.tickets.len() - 1)].key.clone());
                return;
            }
            next += delta;
        }
    }

    /// Due a read: never read, or a minute since — and not one under way.
    pub fn due(&self, now: SystemTime) -> bool {
        self.structure.is_some() && !self.reading && self.read_at.is_none_or(|at| now.duration_since(at).is_ok_and(|d| d >= READ_EVERY))
    }

    pub fn ask(&mut self, ask: Ask) {
        if ask == Ask::Read {
            if self.reading {
                return;
            }
            self.reading = true;
        }
        self.asks.push(ask);
    }

    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    pub fn on_read(&mut self, result: Result<Vec<Ticket>, String>, at: SystemTime, order: &[String]) {
        self.reading = false;
        self.read_at = Some(at);
        match result {
            Ok(tickets) => {
                self.tickets = tickets;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.settle(order);
    }

    /// `m`: the moves Jira allows for the ticket under the cursor are asked for.
    pub fn start_move(&mut self) {
        let Some(t) = self.current() else { return };
        let moving = Moving { key: t.key.clone(), summary: t.summary.clone(), from: t.status.clone(), transitions: None, chosen: 0, confirm: false };
        self.asks.push(Ask::Transitions(moving.key.clone()));
        self.moving = Some(moving);
    }

    pub fn on_transitions(&mut self, key: &str, result: Result<Vec<Transition>, String>) {
        if let Some(m) = self.moving.as_mut().filter(|m| m.key == key) {
            m.transitions = Some(result);
            m.chosen = 0;
        }
    }

    /// What came of a move: the ticket in its new lane at once, then read again to be sure.
    pub fn on_moved(&mut self, key: &str, to: &str, ok: bool) {
        if ok && let Some(t) = self.tickets.iter_mut().find(|t| t.key == key) {
            t.status = to.to_string();
            t.status_since = None;
        }
        self.read_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(key: &str, status: &str, assignee: Option<&str>) -> Ticket {
        Ticket { key: key.into(), summary: format!("DE - {key}"), status: status.into(), assignee: assignee.map(Into::into), ..Ticket::default() }
    }

    fn order() -> Vec<String> {
        ["Backlog", "Selected for Development", "In Progress", "On Hold", "In Review", "Feedback", "Done"].map(String::from).to_vec()
    }

    #[test]
    fn lanes_follow_the_board_and_the_filter_shows_what_nobody_has() {
        let mut team = Team::new(Some(216), "DE");
        team.on_read(Ok(vec![ticket("DATA-1", "Backlog", None), ticket("DATA-2", "In progress", Some("Sam")), ticket("DATA-3", "Backlog", Some("Jo")), ticket("DATA-4", "Odd", None)]), SystemTime::UNIX_EPOCH, &order());
        let lanes = team.lanes(&order());
        let names: Vec<&str> = lanes.iter().map(|l| l.status.as_str()).collect();
        assert_eq!(names, ["Backlog", "Selected for Development", "In Progress", "Odd"], "the workflow's order; an empty lane only for the two taken from; no Done");
        assert_eq!((lanes[0].tickets.len(), lanes[0].all), (1, 2));
        assert_eq!(lanes[2].all, 1, "statuses match whatever their case");
        assert_eq!(team.selected.as_deref(), Some("DATA-1"));
        team.filter = Filter::Everyone;
        assert_eq!(team.walk(&order()), ["DATA-1", "DATA-3", "DATA-2", "DATA-4"]);
        team.selected = Some("DATA-3".into());
        team.cross(&order(), 1);
        assert_eq!(team.selected.as_deref(), Some("DATA-2"), "the next lane with tickets");
    }

    #[test]
    fn a_move_offers_only_where_the_ticket_is_not() {
        let mut team = Team::new(Some(216), "DE");
        team.on_read(Ok(vec![ticket("DATA-1", "Backlog", None)]), SystemTime::UNIX_EPOCH, &order());
        team.start_move();
        assert_eq!(team.take_asks(), [Ask::Transitions("DATA-1".into())]);
        let t = |id: &str, to: &str| Transition { id: id.into(), name: to.into(), to: to.into(), needs_fields: false };
        team.on_transitions("DATA-1", Ok(vec![t("1", "Backlog"), t("2", "Selected for Development"), t("3", "In Progress")]));
        let m = team.moving.as_ref().unwrap();
        assert_eq!(m.offered().len(), 2);
        assert_eq!(m.current().unwrap().to, "Selected for Development");
        team.on_moved("DATA-1", "Selected for Development", true);
        assert_eq!(team.tickets[0].status, "Selected for Development");
        assert!(team.due(SystemTime::now()), "read again after a move");
    }
}
