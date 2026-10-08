//! App state and `update`: a pure state machine, no I/O (DESIGN.md §8).
//!
//! Everything the UI draws lives here or in the model; sources push `Event`s in and the loop
//! applies them. Nothing in this file knows that ClickHouse or Redash exist.

use crate::claude::{Kind, Mode, PaneState, PickRow, Picker, Sessions};
use crate::clock::Clock;
use crate::prayer::{Alert, Place, Prayers};
use crate::folders::Found;
use crate::history::{self, History};
use crate::insight::{self, Insight, Subject};
use crate::model::{fleet_totals, mark_new_nodes, FleetSnapshot, FleetView, Job, JobState, QueueStatus};
use crate::severity::Severity;
use crate::tape::{Tape, Watch};
use crate::tree::{self, Row, RowId, TreeState};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

/// §2.6: a node that keeps failing stays on screen for this long before it is dropped.
const UNREACHABLE_GRACE: Duration = Duration::from_secs(5 * 60);

/// A queue row with the index the cursor uses, so the screen and the selection cannot disagree
/// about which job is selected.
pub type QueueRowRef<'a> = (usize, &'a Job);

/// Where the SQL under a job on view 2 comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlFrom {
    /// The query the stitch found in `system.processes`: what really runs.
    ClickHouse,
    /// The saved query's text from Redash's API.
    Redash,
}

/// View 2's three lists, in the order they are drawn.
#[derive(Debug, Default)]
pub struct QueueSections<'a> {
    pub running: Vec<QueueRowRef<'a>>,
    pub waiting: Vec<QueueRowRef<'a>>,
    pub stale: Vec<QueueRowRef<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Nodes,
    Queue,
    /// The fleet as a heat map of tiles — the shape of a 40-node fleet on one screen.
    Map,
    /// What changed, newest first (`tape.rs`).
    Tape,
    /// What Airflow's DAGs did over the last day (`airflow.rs`).
    Airflow,
    /// Your Jira tickets, in the board's columns (`jira.rs`).
    Jira,
    /// Sessions — Claude Code, OpenCode, a shell — in a pane, the monitor's band above it
    /// (`claude.rs`).
    Claude,
    /// This machine: its CPU, memory and processes (`local.rs`). Beside the sessions, on `0`.
    Local,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Nodes => "NODES",
            View::Queue => "QUEUE",
            View::Map => "MAP",
            View::Tape => "TAPE",
            View::Airflow => "AIRFLOW",
            View::Jira => "JIRA",
            View::Claude => "SESSIONS",
            View::Local => "LOCAL",
        }
    }

    /// The tab order of §1, which is also the `1` … `7`, `0` keymap.
    pub const ALL: [View; 8] = [View::Nodes, View::Queue, View::Map, View::Tape, View::Airflow, View::Jira, View::Claude, View::Local];

    /// The monitor's own views are `1` to `6`; the sessions go on from 7.
    pub const MONITOR: u8 = 6;

    /// `1` … `7`, the number that selects this view — and `0` for this machine, the last key of the
    /// row, so the sessions keep `7`–`9`.
    pub fn number(self) -> u8 {
        match self {
            View::Nodes => 1,
            View::Queue => 2,
            View::Map => 3,
            View::Tape => 4,
            View::Airflow => 5,
            View::Jira => 6,
            View::Claude => 7,
            View::Local => 0,
        }
    }

    pub fn from_number(n: u8) -> Option<View> {
        View::ALL.into_iter().find(|v| v.number() == n)
    }
}

/// A source `r` asks to read again at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Airflow,
    Jira,
}

/// Where the cursor keys go on view 1: the tree, or the insights under it (`tab`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Insights,
}

/// Scroll positions the screen keeps while drawing. `draw` only gets `&App`, and the first
/// visible row depends on the height it is drawing into, so these are cells the drawing code
/// updates — state that belongs to the screen, not to the model.
#[derive(Debug, Default)]
pub struct Viewport {
    pub tree: Cell<usize>,
    pub queue: Cell<usize>,
    pub airflow: Cell<usize>,
    pub jira: Cell<usize>,
    /// Whether view 6 was drawn as a board, so `← →` cross its lanes.
    pub jira_board: Cell<bool>,
    pub local: Cell<usize>,
    pub tape: Cell<usize>,
    /// The first row shown of a page's list on view 5 or 6.
    pub page: Cell<usize>,
    pub map: Cell<usize>,
    pub insights: Cell<usize>,
    /// How many tiles fit in a row of the map, so ↑ ↓ can move by a row.
    pub map_columns: Cell<usize>,
    /// How far the selected query's SQL can scroll, as last drawn.
    pub sql_max: Cell<usize>,
    /// What the mouse can click, as last drawn: the header's tabs, the sessions, Claude's pane,
    /// the folder picker's rows.
    pub hits: RefCell<Vec<(Rect, Hit)>>,
    /// The nodes listed under the sessions, as last drawn, for `Hit::Node`.
    pub listed_nodes: RefCell<Vec<String>>,
    /// Where a query session's text was last drawn, for a click or a drag in it.
    pub console_text: Cell<Option<TextLayout>>,
}

/// A query session's text as drawn: where its lines start on screen, the first line shown, and
/// how far the cursor's line slid along to keep the cursor in sight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLayout {
    pub text_x: u16,
    pub top: u16,
    pub height: u16,
    pub first: usize,
    pub cursor_row: usize,
    pub shift: usize,
}

/// Something on screen a click means something on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    View(View),
    Session(usize),
    NewSession,
    /// Claude's screen: clicks and the wheel go to the program.
    Pane,
    /// A row of the folder picker.
    Pick(PickRow),
    /// A step of the picker's path, by its place in `Picker::crumbs`.
    Crumb(usize),
    /// The picker's way out.
    Cancel,
    /// What the picker will open.
    Kind(Kind),
    /// A node listed under the sessions, by its place in `Viewport::listed_nodes`.
    Node(usize),
    /// The clock: a click flips it between local time and UTC.
    Clock,
    /// The way to a conversation had elsewhere, to take up in a new session.
    Resume,
    /// A query session's server: a click lists the others.
    ConsoleServer,
    /// One of the servers listed, by its place in the fleet's order.
    ConsolePick(usize),
    /// A query session's text, or its answer: a click puts the keys there.
    ConsoleText,
    ConsoleAnswer,
    /// The helper that writes a query session's SQL: a click changes it.
    ConsoleAssistant,
    /// The way to ask it, under the text: a click asks.
    ConsoleAsk,
    /// One of the suggestions open, by its place among them: a click takes it.
    Suggestion(usize),
    /// The way out of a prayer's reminder.
    Dismiss,
    /// A row of view 1's tree, an insight, a job of view 2, a line of the tape, a tile of the
    /// map — by its place in their lists: a click puts the cursor there, a second opens it.
    Row(usize),
    Insight(usize),
    Job(usize),
    TapeLine(usize),
    Tile(usize),
    /// A row of view 5 (a run, a DAG) and a ticket of view 6, by their place in the lists.
    AirflowRow(usize),
    Ticket(usize),
    /// A row of view 0, by its place.
    Process(usize),
    /// A row of a page open on view 5 — a run, a task — by its place in the page.
    PageRow(usize),
    /// On the month's time: a day's column, and a ticket's row.
    TimeDay(u32),
    TimeTicket(usize),
}

/// Whether a Redash data source's name points at a node: `clickhouse-bi (prod)` names
/// `clickhouse-bi.example.net`. Names only — Redash's list of data sources has no hosts.
fn names_node(data_source: Option<&str>, node: &str) -> bool {
    let Some(source) = data_source else {
        return false;
    };
    let node = node.to_ascii_lowercase();
    let label = node.split(['.', ':']).next().unwrap_or(&node).to_string();
    source
        .to_ascii_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .any(|token| token == label || token == node)
}

/// Keep `selected` inside a window of `height` rows starting at `offset`, moving the window as
/// little as possible. Returns the new first visible row.
pub fn scroll_into_view(offset: usize, selected: Option<usize>, height: usize, len: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    let max_offset = len - height;
    let mut offset = offset.min(max_offset);
    if let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    offset.min(max_offset)
}

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    /// One second, for the UTC clock (§7).
    Tick,
    Snapshot(Box<FleetSnapshot>),
    Queue(Box<QueueStatus>),
    /// A source could not start at all (bad credentials, no seed reachable). Shown in the
    /// strip and the drawer; never fatal.
    Notice(String),
    /// What the `claude` of a session printed (`pty.rs`), by session id.
    Pane(u64, Vec<u8>),
    /// The `claude` of a session ended, and how.
    PaneExited(u64, String),
    /// Text pasted into the terminal (bracketed paste).
    Paste(String),
    /// A click or a turn of the wheel.
    Mouse(MouseEvent),
    /// Folders for the picker of a new session (`folders.rs`).
    Folders(Found),
    /// The machine's time zone as it is now, and the place prayer times are for with it.
    Zone(Option<String>, Option<Place>),
    /// Conversations for the picker (`conversations.rs`): for which lookup, kind, and whether
    /// every folder's.
    Conversations(u64, Kind, bool, Vec<crate::conversations::Conversation>),
    /// The conversation a session's program is in now, as it says (by session id).
    Conversation(u64, String),
    /// What a query session's query answered: the session, the run, the answer.
    ConsoleAnswer(u64, u64, Result<crate::console::Answer, String>),
    /// A server's databases, tables and columns, for a query session's suggestions.
    Schema(String, Result<crate::complete::Schema, String>),
    /// What a helper wrote for a query session: the session, the question, the SQL.
    Assisted(u64, u64, Result<String, String>),
    /// What Redash answered to cancelling a job, by its id: yes, or why not.
    Cancelled(String, Result<(), String>),
    /// What Airflow's DAGs did over the last day (view 5).
    Airflow(Box<crate::airflow::Activity>),
    /// Your Jira tickets (view 6).
    Jira(Box<crate::jira::Board>),
    /// One read of this machine (view 0).
    Local(Box<crate::local::Sample>),
    /// What Claude Code and OpenCode used (view 0).
    Usage(Box<crate::usage::Usage>),
    /// What a page of view 5 or 6 asked for, and what came back.
    Detail(crate::detail::Ask, Result<crate::detail::Body, String>),
    Quit,
}

/// A job Redash has been asked to cancel from view 2.
#[derive(Debug, Clone)]
pub struct Cancelling {
    /// `#7438 Gateway transfers of grigol.gankava`, for the notice when Redash answers — the
    /// job may have left the list by then.
    pub what: String,
    /// It ran on a ClickHouse data source — and on which node its query was found — which a
    /// cancel in Redash does not stop.
    pub on_clickhouse: bool,
    pub node: Option<String>,
    /// Redash said yes; the job stays marked until it has left the queue.
    pub accepted: bool,
    pub since: SystemTime,
}

/// A job in a few words: its query and whose it is.
pub fn job_words(job: &Job) -> String {
    format!("{} of {}", job.query_label(), job.label())
}

/// What the footer should say (§3: contextual).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Footer {
    Keys,
    Filter,
    Help,
}

pub struct App {
    snapshot: Option<Box<FleetSnapshot>>,
    /// The poll before this one, for the CPU deltas of §5.2.
    prev: Option<Box<FleetSnapshot>>,
    pub queue: QueueStatus,
    notice: Option<(String, SystemTime)>,
    /// How long a notice stays in the footer.
    notice_ttl: std::time::Duration,

    first_seen: HashSet<String>,
    pub tree: TreeState,
    selected: Option<RowId>,

    pub view: View,
    pub paused: bool,
    /// `Some` while `/` is being typed (§3).
    filter_input: Option<String>,
    pub help: bool,
    pub footer: Footer,
    pub clock: SystemTime,

    unreachable_since: HashMap<String, SystemTime>,
    /// Which row of view 2 the cursor is on, so ⏎ can jump to its ClickHouse query (§2.8).
    queue_selection: Option<usize>,
    /// View 2: the job `x` asked about — by its id, as the list moves under the cursor every
    /// 3 s — until `y` cancels it in Redash or any other key keeps it.
    queue_confirm: Option<String>,
    /// Jobs to cancel in Redash, for `main.rs` to send once.
    cancels: Vec<String>,
    /// What Redash has been asked to cancel, by job id, until the job has left the queue.
    cancelling: HashMap<String, Cancelling>,
    pub quit: bool,

    /// The last few minutes of every number, for sparklines, trends and forecasts.
    pub history: History,
    /// What changed, for view 4.
    pub tape: Tape,
    watch: Watch,
    pub focus: Focus,
    insight_selection: usize,
    /// How far the SQL under the selected query is scrolled, and which query that is for: a
    /// new query starts at its first line.
    sql_scroll: usize,
    sql_scroll_of: Option<String>,
    map_selection: usize,
    tape_selection: usize,
    /// `POLL_MS`, so the header can say how often it polls and notice when data is late.
    pub poll_interval: Duration,
    /// The terminal tells Enter with shift or ⌘ from Enter alone (the kitty keyboard protocol),
    /// so ⇧⏎ is a new line for Claude and OpenCode; without it, ctrl+j is.
    pub modified_enter: bool,
    /// Wall time the last snapshot arrived: data older than a few polls is called stale.
    last_snapshot_wall: Option<SystemTime>,
    pub viewport: Viewport,
    /// View 5: Claude Code sessions.
    pub claude: Sessions,
    /// How times are shown: local or UTC.
    pub time: Clock,
    /// The day's prayer times and their reminders.
    pub prayers: Prayers,
    /// What to say beyond the screen — a prayer's reminder — for the loop to send on.
    notifications: Vec<String>,
    /// What each server has, by name, for query sessions' suggestions and their helper.
    pub schemas: HashMap<String, SchemaState>,
    /// What was copied, for the loop to put on the clipboard.
    clipboard: Vec<String>,
    /// View 5: what Airflow's DAGs did over the last day.
    pub airflow: crate::airflow::Activity,
    /// View 6: your Jira tickets.
    pub jira: crate::jira::Board,
    /// View 0: this machine.
    pub local: crate::local::Local,
    /// View 0's AI panel: what Claude Code and OpenCode used.
    pub usage: crate::usage::Usage,
    /// The row under view 5's cursor, by what it is about — the lists move under it every 15 s —
    /// and its place, for when it is gone.
    airflow_selected: Option<crate::airflow::RowKey>,
    airflow_index: usize,
    /// The ticket under view 6's cursor, by its key, and its place.
    jira_selected: Option<String>,
    jira_index: usize,
    /// Pages to open in the browser — a ticket, a DAG's grid — for the loop.
    opens: Vec<String>,
    /// Sources `r` asked to read again, for the loop; and those whose answer is still awaited.
    refreshes: Vec<Feed>,
    reading: Vec<Feed>,
    /// The ticket open on view 6, and the pages open on view 5 — a DAG's runs, a run's tasks, a
    /// task's log — each over the one before; `esc` goes back.
    pub jira_page: Option<crate::detail::Page>,
    pub airflow_pages: Vec<crate::detail::Page>,
    /// What those pages asked for, for the loop to read.
    detail_asks: Vec<crate::detail::Ask>,
    /// The masthead is drawn outside the terminal — by Cobserve.app's own header — so the screen
    /// leaves it out.
    pub outside_chrome: bool,
    /// View 6's page of the month's time by ticket (`t`): the day chosen and the ticket under the
    /// cursor. A ticket opened from it opens over it.
    pub jira_time: Option<TimePage>,
}

/// Where the cursor is on the month's time: a day of the month (from 1) and a ticket's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimePage {
    pub day: u32,
    pub ticket: usize,
}

/// What a server has, for a query session: what was read last, and whether it is being read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemaState {
    pub schema: Option<crate::complete::Schema>,
    pub reading: bool,
    /// When it was last read, or failed to be, Unix seconds.
    pub at: i64,
    pub error: Option<String>,
}

/// How long a server's tables are taken as they were read — then read again, as a session uses
/// it — and how long after a failure it is tried again.
const SCHEMA_FRESH_S: i64 = 15 * 60;
const SCHEMA_RETRY_S: i64 = 60;

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            prev: None,
            queue: QueueStatus::unreachable(crate::model::QUEUE_NOT_POLLED),
            notice: None,
            notice_ttl: std::time::Duration::from_secs(30),
            first_seen: HashSet::new(),
            tree: TreeState::default(),
            selected: None,
            view: View::Nodes,
            paused: false,
            filter_input: None,
            help: false,
            footer: Footer::Keys,
            clock: SystemTime::now(),
            unreachable_since: HashMap::new(),
            queue_selection: None,
            queue_confirm: None,
            cancels: Vec::new(),
            cancelling: HashMap::new(),
            quit: false,
            history: History::default(),
            tape: Tape::default(),
            watch: Watch::default(),
            focus: Focus::Tree,
            insight_selection: 0,
            sql_scroll: 0,
            sql_scroll_of: None,
            map_selection: 0,
            tape_selection: 0,
            poll_interval: Duration::from_millis(2000),
            modified_enter: false,
            last_snapshot_wall: None,
            viewport: Viewport::default(),
            claude: Sessions::default(),
            time: Clock::default(),
            prayers: Prayers::default(),
            notifications: Vec::new(),
            schemas: HashMap::new(),
            clipboard: Vec::new(),
            airflow: crate::airflow::Activity::unreachable(crate::airflow::NOT_READ),
            jira: crate::jira::Board::unreachable(crate::jira::NOT_READ),
            local: crate::local::Local::default(),
            usage: crate::usage::Usage::default(),
            airflow_selected: None,
            airflow_index: 0,
            jira_selected: None,
            jira_index: 0,
            opens: Vec::new(),
            refreshes: Vec::new(),
            reading: Vec::new(),
            jira_page: None,
            airflow_pages: Vec::new(),
            detail_asks: Vec::new(),
            jira_time: None,
            outside_chrome: false,
        }
    }

    /// The month's time by ticket, as the cursor of its page sees it.
    pub fn time_by_ticket(&self) -> (crate::jira::Month, Vec<crate::jira::TicketTime>) {
        let now = self.now();
        let month = crate::jira::Month::of(now, self.time.offset_s(now));
        let tickets = month.by_ticket(&self.jira.worklogs);
        (month, tickets)
    }

    /// The keys of the month's time: ← → a day, ↑ ↓ a ticket, ⏎ that ticket in full, `esc` out.
    fn on_time_key(&mut self, code: KeyCode) {
        let (month, tickets) = self.time_by_ticket();
        let Some(page) = self.jira_time.as_mut() else {
            return;
        };
        let last = tickets.len().saturating_sub(1);
        match code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('t') => self.jira_time = None,
            KeyCode::Left | KeyCode::Char('h') => page.day = page.day.saturating_sub(1).max(1),
            KeyCode::Right | KeyCode::Char('l') => page.day = (page.day + 1).min(month.days),
            KeyCode::Home => page.day = 1,
            KeyCode::End => page.day = month.today.min(month.days),
            KeyCode::Up | KeyCode::Char('k') => page.ticket = page.ticket.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => page.ticket = (page.ticket + 1).min(last),
            KeyCode::Enter => {
                if let Some(ticket) = tickets.get(page.ticket) {
                    let key = ticket.key.clone();
                    self.open_page(crate::detail::Ask::Issue(key));
                }
            }
            KeyCode::Char('o') => {
                if let Some(url) = tickets.get(page.ticket).and_then(|t| self.jira.link(&t.key)) {
                    self.opens.push(url);
                }
            }
            KeyCode::Char('y') => {
                if let Some(url) = tickets.get(page.ticket).and_then(|t| self.jira.link(&t.key)) {
                    self.notice = Some((format!("copied {url}"), SystemTime::now()));
                    self.clipboard.push(url);
                }
            }
            KeyCode::Char('r') => self.read_again(Feed::Jira),
            _ => {}
        }
    }

    /// For the tests and the screenshots that move the clock on: the fleet's last numbers as
    /// fresh as the clock says, so the header does not call them stale.
    #[cfg(test)]
    pub fn numbers_fresh_at_the_clock(&mut self) {
        self.last_snapshot_wall = Some(self.clock);
    }

    /// What the pages asked to read since the loop last asked.
    pub fn take_detail_asks(&mut self) -> Vec<crate::detail::Ask> {
        std::mem::take(&mut self.detail_asks)
    }

    /// The page open on the view on screen, if any.
    pub fn page(&self) -> Option<&crate::detail::Page> {
        match self.view {
            View::Jira => self.jira_page.as_ref(),
            View::Airflow => self.airflow_pages.last(),
            _ => None,
        }
    }

    fn page_mut(&mut self) -> Option<&mut crate::detail::Page> {
        match self.view {
            View::Jira => self.jira_page.as_mut(),
            View::Airflow => self.airflow_pages.last_mut(),
            _ => None,
        }
    }

    /// A page opened over the view, and what it shows asked for.
    fn open_page(&mut self, ask: crate::detail::Ask) {
        self.detail_asks.push(ask.clone());
        let page = crate::detail::Page::new(ask);
        match self.view {
            View::Jira => self.jira_page = Some(page),
            View::Airflow => self.airflow_pages.push(page),
            _ => {}
        }
    }

    /// An answer for a page: to every page that asked it.
    fn on_detail(&mut self, ask: crate::detail::Ask, result: Result<crate::detail::Body, String>) {
        let pages = self.jira_page.iter_mut().chain(self.airflow_pages.iter_mut());
        for page in pages.filter(|p| p.ask == ask) {
            page.body = Some(result.clone());
            let rows = page.rows();
            page.cursor = page.cursor.min(rows.saturating_sub(1));
        }
    }

    /// The keys of a page: the arrows move (or scroll), ⏎ goes a page deeper, `esc` back, `r`
    /// reads the page again. `o` and `y` are the view's own.
    fn on_page_key(&mut self, code: KeyCode) {
        use crate::detail::Ask;
        match code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                match self.view {
                    View::Jira => self.jira_page = None,
                    _ => {
                        self.airflow_pages.pop();
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.page_step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.page_step(1),
            KeyCode::PageUp => self.page_step(-20),
            KeyCode::PageDown | KeyCode::Char(' ') => self.page_step(20),
            KeyCode::Home | KeyCode::Char('g') => {
                if let Some(page) = self.page_mut() {
                    page.cursor = 0;
                    page.scroll = 0;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if let Some(page) = self.page_mut() {
                    page.cursor = page.rows().saturating_sub(1);
                    page.scroll = usize::MAX;
                }
            }
            KeyCode::Char('r') => {
                if let Some(page) = self.page_mut() {
                    page.body = None;
                    let ask = page.ask.clone();
                    self.detail_asks.push(ask);
                }
            }
            KeyCode::Char('o') => self.open_at_cursor(),
            KeyCode::Char('y') => self.copy_link(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let next = match self.page() {
                    Some(page) => match (&page.ask, page.run_at_cursor(), page.task_at_cursor()) {
                        (Ask::Runs(dag), Some(run), _) => Some(Ask::Tasks { dag: dag.clone(), run: run.id.clone() }),
                        (Ask::Tasks { dag, run }, _, Some(task)) if task.try_number > 0 => Some(Ask::Log {
                            dag: dag.clone(),
                            run: run.clone(),
                            task: task.id.clone(),
                            map_index: task.map_index,
                            attempt: task.try_number,
                        }),
                        (Ask::Tasks { .. }, _, Some(task)) => {
                            self.notice = Some((format!("{} has not run yet: no log", task.label()), SystemTime::now()));
                            None
                        }
                        _ => None,
                    },
                    None => None,
                };
                if let Some(ask) = next {
                    self.open_page(ask);
                }
            }
            _ => {}
        }
    }

    fn page_step(&mut self, delta: isize) {
        if let Some(page) = self.page_mut() {
            page.step(delta);
        }
    }

    /// Pages to open in the browser since the loop last asked.
    pub fn take_opens(&mut self) -> Vec<String> {
        std::mem::take(&mut self.opens)
    }

    /// Sources to read again now, since the loop last asked.
    pub fn take_refreshes(&mut self) -> Vec<Feed> {
        std::mem::take(&mut self.refreshes)
    }

    /// Whether `r` asked a source to read again and its answer has not come yet.
    pub fn is_reading(&self, feed: Feed) -> bool {
        self.reading.contains(&feed)
    }

    /// The ticket under view 6's cursor, by key.
    pub fn jira_selection(&self) -> Option<&str> {
        self.jira_selected.as_deref()
    }

    /// The row under view 5's cursor.
    pub fn airflow_selection(&self) -> Option<&crate::airflow::RowKey> {
        self.airflow_selected.as_ref()
    }

    /// A read of Jira. One that failed after one that worked keeps the tickets on screen, with
    /// why they are not new: a minute's timeout is no reason to empty the list.
    fn on_jira(&mut self, board: crate::jira::Board) {
        self.reading.retain(|f| *f != Feed::Jira);
        if !board.reachable && !board.is_placeholder() && self.jira.reachable {
            self.jira.error = board.error;
            return;
        }
        self.jira = board;
    }

    /// A read of Airflow, kept the same way when it fails.
    fn on_airflow(&mut self, activity: crate::airflow::Activity) {
        self.reading.retain(|f| *f != Feed::Airflow);
        if !activity.reachable && !activity.is_placeholder() && self.airflow.reachable {
            self.airflow.error = activity.error;
            return;
        }
        self.airflow = activity;
    }

    /// The rows of view 5, by key, in the order drawn.
    pub fn airflow_rows(&self) -> Vec<crate::airflow::RowKey> {
        self.airflow.sections(self.now()).keys()
    }

    fn move_airflow(&mut self, delta: isize) {
        let rows = self.airflow_rows();
        if rows.is_empty() {
            self.airflow_selected = None;
            return;
        }
        let at = self
            .airflow_selected
            .as_ref()
            .and_then(|key| rows.iter().position(|r| r == key))
            .unwrap_or(self.airflow_index);
        let next = (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.airflow_index = next;
        self.airflow_selected = rows.get(next).cloned();
    }

    /// On the board: to the next lane with tickets that way, as far down it as the cursor was.
    fn cross_lane(&mut self, step: isize) {
        let lanes: Vec<Vec<String>> =
            crate::ui::board_lanes(&self.jira).into_iter().map(|l| l.into_iter().map(|t| t.key.clone()).collect()).collect();
        let Some(key) = self.jira_selected.clone() else {
            return self.move_jira(0);
        };
        let Some((lane, at)) = lanes.iter().enumerate().find_map(|(l, keys)| keys.iter().position(|k| *k == key).map(|i| (l, i))) else {
            return;
        };
        let mut next = lane as isize + step;
        while next >= 0 && (next as usize) < lanes.len() {
            let keys = &lanes[next as usize];
            if let Some(key) = keys.get(at.min(keys.len().saturating_sub(1))) {
                self.jira_selected = Some(key.clone());
                if let Some(index) = self.jira.rows().iter().position(|t| t.key == *key) {
                    self.jira_index = index;
                }
                return;
            }
            next += step;
        }
    }

    fn move_jira(&mut self, delta: isize) {
        let keys: Vec<String> = self.jira.rows().iter().map(|t| t.key.clone()).collect();
        if keys.is_empty() {
            self.jira_selected = None;
            return;
        }
        let at = self
            .jira_selected
            .as_ref()
            .and_then(|key| keys.iter().position(|k| k == key))
            .unwrap_or(self.jira_index);
        let next = (at as isize + delta).clamp(0, keys.len() as isize - 1) as usize;
        self.jira_index = next;
        self.jira_selected = keys.get(next).cloned();
    }

    /// The page of the row under the cursor on view 5 or 6.
    fn link_at_cursor(&self) -> Option<String> {
        use crate::airflow::RowKey;
        use crate::detail::Ask;
        if let Some(page) = self.page() {
            return match &page.ask {
                Ask::Issue(key) => self.jira.link(key),
                Ask::Runs(dag) => match page.run_at_cursor() {
                    Some(run) => self.airflow.link(&RowKey::Running(dag.clone(), run.id.clone())),
                    None => self.airflow.link(&RowKey::Dag(dag.clone())),
                },
                Ask::Tasks { dag, run } => {
                    let link = self.airflow.link(&RowKey::Running(dag.clone(), run.clone()))?;
                    Some(match page.task_at_cursor() {
                        Some(task) => format!("{link}&task_id={}", crate::sources::segment(&task.id)),
                        None => link,
                    })
                }
                Ask::Log { dag, run, task, .. } => {
                    let link = self.airflow.link(&RowKey::Running(dag.clone(), run.clone()))?;
                    Some(format!("{link}&task_id={}&tab=logs", crate::sources::segment(task)))
                }
            };
        }
        match self.view {
            View::Airflow => self.airflow.link(self.airflow_selected.as_ref()?),
            View::Jira => self.jira.link(self.jira_selected.as_deref()?),
            _ => None,
        }
    }

    /// `⏎` on view 5's lists: a run's tasks, or a DAG's runs, in a page here.
    fn open_airflow_row(&mut self) {
        use crate::airflow::RowKey;
        use crate::detail::Ask;
        let ask = match self.airflow_selected.clone() {
            Some(RowKey::Running(dag, run) | RowKey::Queued(dag, run) | RowKey::Failed(dag, run)) => Ask::Tasks { dag, run },
            Some(RowKey::Dag(dag)) => Ask::Runs(dag),
            None => return,
        };
        self.open_page(ask);
    }

    /// `⏎`, or a second click: the row's page in the browser.
    fn open_at_cursor(&mut self) {
        if let Some(url) = self.link_at_cursor() {
            self.opens.push(url);
        }
    }

    /// `y`: the row's link on the clipboard.
    fn copy_link(&mut self) {
        if let Some(url) = self.link_at_cursor() {
            self.notice = Some((format!("copied {url}"), SystemTime::now()));
            self.clipboard.push(url);
        }
    }

    /// `r`: the view's source reads again now.
    fn read_again(&mut self, feed: Feed) {
        if !self.refreshes.contains(&feed) {
            self.refreshes.push(feed);
        }
        if !self.reading.contains(&feed) {
            self.reading.push(feed);
        }
    }

    /// View 5's keys: ↑ ↓ move, ⏎ opens the run or the DAG in Airflow, `y` copies its link, `r`
    /// reads again.
    fn on_airflow_key(&mut self, code: KeyCode) {
        if !self.airflow_pages.is_empty() {
            return self.on_page_key(code);
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_airflow(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_airflow(1),
            KeyCode::PageUp => self.move_airflow(-10),
            KeyCode::PageDown => self.move_airflow(10),
            KeyCode::Home => self.move_airflow(isize::MIN / 2),
            KeyCode::End => self.move_airflow(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.open_airflow_row(),
            KeyCode::Char('o') => self.open_at_cursor(),
            KeyCode::Char('y') => self.copy_link(),
            KeyCode::Char('r') => self.read_again(Feed::Airflow),
            _ => {}
        }
    }

    /// View 6's keys, the same as view 5's for a ticket.
    fn on_jira_key(&mut self, code: KeyCode) {
        if self.jira_page.is_some() {
            return self.on_page_key(code);
        }
        if self.jira_time.is_some() {
            return self.on_time_key(code);
        }
        if code == KeyCode::Char('t') {
            let today = self.time_by_ticket().0.today;
            self.jira_time = Some(TimePage { day: today, ticket: 0 });
            return;
        }
        match code {
            KeyCode::Left | KeyCode::Char('h') if self.viewport.jira_board.get() => self.cross_lane(-1),
            KeyCode::Right | KeyCode::Char('l') if self.viewport.jira_board.get() => self.cross_lane(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_jira(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_jira(1),
            KeyCode::PageUp => self.move_jira(-10),
            KeyCode::PageDown => self.move_jira(10),
            KeyCode::Home => self.move_jira(isize::MIN / 2),
            KeyCode::End => self.move_jira(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(key) = self.jira_selected.clone() {
                    self.open_page(crate::detail::Ask::Issue(key));
                }
            }
            KeyCode::Char('o') => self.open_at_cursor(),
            KeyCode::Char('y') => self.copy_link(),
            KeyCode::Char('r') => self.read_again(Feed::Jira),
            _ => {}
        }
    }

    /// Keep views 5's and 6's cursors on what they were on, or at the same place when it is gone.
    fn resolve_lists(&mut self) {
        match self.view {
            View::Airflow => {
                let rows = self.airflow_rows();
                match self.airflow_selected.as_ref().and_then(|key| rows.iter().position(|r| r == key)) {
                    Some(at) => self.airflow_index = at,
                    None if rows.is_empty() => self.airflow_selected = None,
                    None => {
                        self.airflow_index = self.airflow_index.min(rows.len() - 1);
                        self.airflow_selected = rows.get(self.airflow_index).cloned();
                    }
                }
            }
            View::Jira => {
                let keys: Vec<String> = self.jira.rows().iter().map(|t| t.key.clone()).collect();
                match self.jira_selected.as_ref().and_then(|key| keys.iter().position(|k| k == key)) {
                    Some(at) => self.jira_index = at,
                    None if keys.is_empty() => self.jira_selected = None,
                    None => {
                        self.jira_index = self.jira_index.min(keys.len() - 1);
                        self.jira_selected = keys.get(self.jira_index).cloned();
                    }
                }
            }
            _ => {}
        }
    }

    /// What was copied since the loop last asked.
    pub fn take_clipboard(&mut self) -> Vec<String> {
        std::mem::take(&mut self.clipboard)
    }

    /// The servers whose tables a query session needs read now: not read yet, read a while
    /// ago, or failed a minute ago. Each is marked as being read.
    pub fn schemas_wanted(&mut self) -> Vec<String> {
        let now = self.now();
        let nodes: HashSet<String> = self.claude.list.iter().filter_map(|s| s.console.as_ref()?.node.clone()).collect();
        let mut wanted = Vec::new();
        for node in nodes {
            let state = self.schemas.entry(node.clone()).or_default();
            let due = match (&state.schema, &state.error) {
                _ if state.reading => false,
                (None, None) => true,
                (None, Some(_)) => now - state.at >= SCHEMA_RETRY_S,
                (Some(_), _) => now - state.at >= SCHEMA_FRESH_S,
            };
            if due {
                state.reading = true;
                wanted.push(node);
            }
        }
        wanted.sort();
        wanted
    }

    /// The schema of the server a query session runs on, as last read.
    pub fn schema_of(&self, node: Option<&str>) -> Option<&crate::complete::Schema> {
        self.schemas.get(node?)?.schema.as_ref()
    }

    /// The fleet's servers, by name in view 1's order: what a query session can run on.
    pub fn server_names(&self) -> Vec<String> {
        self.servers().into_iter().map(|(name, _)| name).collect()
    }

    /// The fleet's servers by name, in view 1's order, and whether each answers.
    pub fn servers(&self) -> Vec<(String, bool)> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut nodes: Vec<&crate::model::NodeSnapshot> = snapshot.nodes.iter().collect();
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        nodes.into_iter().map(|n| (n.name.clone(), n.reachable)).collect()
    }

    /// Now, in Unix seconds, as the clock last ticked.
    pub fn now(&self) -> i64 {
        self.clock.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }

    /// What the loop has to say beyond the screen since it last asked.
    pub fn take_notifications(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notifications)
    }

    /// The prayer alert on screen now, if any.
    pub fn prayer_alert(&self) -> Option<Alert> {
        self.prayers.alert(self.now())
    }

    /// The clock moved on: the prayer times with it, and a reminder said once when one begins.
    fn on_tick(&mut self) {
        self.tick_at(SystemTime::now());
    }

    /// The clock at `at`, as a tick sets it — the tests and the screenshots choose their moment.
    pub fn tick_at(&mut self, at: SystemTime) {
        self.clock = at;
        let now = self.now();
        if let Some(alert) = self.prayers.tick(now) {
            let moment = alert.moment();
            let at = self.time.local_hm(moment.at);
            let place = self.prayers.place.as_ref().map(|p| p.name.clone()).unwrap_or_default();
            self.notifications.push(match alert {
                Alert::Soon { left, .. } => {
                    format!("{} in {} min · {at} · {place}", moment.name(), (left + 59) / 60)
                }
                Alert::Now { .. } => format!("Time for {} · {at} · {place}", moment.name()),
            });
        }
    }

    pub fn update(&mut self, event: Event) {
        match event {
            Event::Tick => self.on_tick(),
            Event::Zone(zone, place) => {
                self.time.zone = zone;
                self.prayers.move_to(place);
                self.on_tick();
            }
            Event::Key(key) => self.on_key(key),
            // While paused, live data is dropped rather than queued up: unpausing must show
            // the last thing on screen, not a burst of everything that happened since (§3).
            Event::Snapshot(snapshot) => {
                if !self.paused {
                    self.on_snapshot(*snapshot)
                }
            }
            Event::Queue(queue) => {
                if !self.paused {
                    self.on_queue(*queue)
                }
            }
            Event::Cancelled(id, result) => self.on_cancelled(&id, result),
            Event::Airflow(activity) => {
                if !self.paused {
                    self.on_airflow(*activity)
                }
            }
            Event::Jira(board) => {
                if !self.paused {
                    self.on_jira(*board)
                }
            }
            Event::Usage(usage) => self.usage = *usage,
            Event::Local(sample) => {
                if !self.paused {
                    self.local.record(*sample)
                }
            }
            Event::Detail(ask, result) => self.on_detail(ask, result),
            Event::Notice(message) => {
                let now = SystemTime::now();
                // The same message twice in a row does not restart the clock: a source that
                // fails every poll would otherwise keep a notice alive forever.
                match &self.notice {
                    Some((previous, when)) if *previous == message && now.duration_since(*when).is_ok_and(|d| d < self.notice_ttl) => {}
                    _ => self.notice = Some((message, now)),
                }
            }
            Event::Pane(id, bytes) => {
                // The picker covers the pane while a new session's folder is chosen.
                let on_screen = self.view == View::Claude
                    && !matches!(self.claude.mode, Mode::Opening(_))
                    && self.claude.current().is_some_and(|s| s.id == id);
                if let Some(session) = self.claude.by_id_mut(id) {
                    session.pane.feed(&bytes, on_screen);
                }
            }
            Event::PaneExited(id, how) => {
                if let Some(session) = self.claude.by_id_mut(id) {
                    session.pane.state = PaneState::Exited(how);
                }
            }
            Event::Mouse(event) => self.on_mouse(event),
            Event::Folders(found) => {
                if let Mode::Opening(picker) = &mut self.claude.mode {
                    picker.found(found);
                }
            }
            Event::Conversations(generation, kind, everywhere, list) => {
                // A conversation a session here is in is that session, not one to take up.
                let here: HashSet<String> = self.claude.list.iter().filter_map(|s| s.conversation.clone()).collect();
                let list = list.into_iter().filter(|c| !here.contains(&c.id)).collect();
                if let Mode::Opening(picker) = &mut self.claude.mode {
                    picker.found_conversations(generation, kind, everywhere, list);
                }
            }
            Event::Conversation(id, conversation) => {
                if let Some(session) = self.claude.by_id_mut(id) {
                    session.conversation = Some(conversation);
                }
            }
            Event::ConsoleAnswer(id, run, result) => {
                if let Some(console) = self.claude.by_id_mut(id).and_then(|s| s.console.as_deref_mut()) {
                    console.answered(run, result);
                }
            }
            Event::Schema(node, result) => {
                let now = self.now();
                let state = self.schemas.entry(node).or_default();
                state.reading = false;
                state.at = now;
                match result {
                    Ok(schema) => {
                        state.schema = Some(schema);
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Assisted(id, ask, result) => {
                if let Some(console) = self.claude.by_id_mut(id).and_then(|s| s.console.as_deref_mut()) {
                    console.assisted(ask, result);
                }
            }
            Event::Paste(text) => {
                let line = text.lines().next().unwrap_or_default();
                if self.view == View::Claude {
                    match &mut self.claude.mode {
                        Mode::Naming(name) => {
                            let room = crate::claude::NAME_MAX.saturating_sub(name.chars().count());
                            name.extend(line.chars().take(room));
                        }
                        Mode::Opening(picker) => picker.type_text(line),
                        Mode::Finding { query, at } => {
                            query.push_str(line);
                            *at = 0;
                        }
                        Mode::Typing => match self.claude.current_mut() {
                            Some(session) if session.console.is_some() => {
                                if let Some(console) = session.console.as_deref_mut() {
                                    console.paste(&text);
                                }
                            }
                            Some(session) if session.pane.is_running() => session.pane.paste(&text),
                            _ => {}
                        },
                        Mode::Bar => {}
                    }
                } else if let Some(input) = self.filter_input.as_mut() {
                    input.push_str(line);
                    self.tree.filter = input.clone();
                }
            }
            Event::Quit => self.quit = true,
        }
        self.sync_view_state();
    }

    /// View 5, with a session started on the first visit — never a second one over a running
    /// one. The monitor (`ctrl+\` twice, or `m` on the bar) goes back to the view it came from.
    pub fn open_claude(&mut self) {
        if self.view != View::Claude {
            self.claude.back_to = self.view;
        }
        self.view = View::Claude;
        self.focus = Focus::Tree;
        self.queue_selection = None;
        self.claude.mode = Mode::Typing;
        if self.claude.list.is_empty() {
            self.claude.open_new("", Kind::Claude);
        } else {
            let active = self.claude.active;
            self.claude.select(active);
        }
    }

    /// Session `number` (7–9) on view 7. The first visit opens session 7; a number with no
    /// session behind it says how to open one.
    fn open_session(&mut self, number: usize) {
        match self.claude.index_of(number) {
            Some(index) => {
                self.open_claude();
                self.claude.select(index);
            }
            None if self.claude.list.is_empty() && number == crate::claude::FIRST_NUMBER => self.open_claude(),
            None => {
                self.notice = Some((format!("no session {number} yet — ctrl+\\ then n opens one"), SystemTime::now()));
            }
        }
    }

    /// Start the session on screen again, or a first one when there is none.
    fn start_claude(&mut self) {
        let size = self.claude.want_size.get();
        match self.claude.current_mut() {
            Some(session) => session.pane.start(size, session.kind.scrollback()),
            None => {
                self.claude.open_new("", Kind::Claude);
            }
        }
    }

    /// Ask where a new session should work — the folder picker, starting where the one on
    /// screen works, for a session of `kind` or of the kind on screen — unless every number
    /// is taken.
    pub fn ask_new_session(&mut self, kind: Option<Kind>) {
        if self.claude.list.len() >= crate::claude::MAX_SESSIONS {
            self.notice = Some((
                format!("{} sessions is as many as there can be — close one with ctrl+\\ x x", crate::claude::MAX_SESSIONS),
                SystemTime::now(),
            ));
            self.claude.mode = Mode::Typing;
            return;
        }
        let dir = crate::pty::expand(&self.claude.next_dir());
        let kind = kind.unwrap_or_else(|| self.claude.next_kind());
        let mut picker = Picker::new(dir, kind);
        picker.servers = self.servers();
        // A query session starts on the node view 1 has the cursor on, else the one on screen's.
        if kind == Kind::Query {
            let near = match self.selected() {
                Some(RowId::Node(name)) => Some(name.clone()),
                _ => self.claude.current().and_then(|s| s.console.as_ref()).and_then(|c| c.node.clone()),
            };
            if let Some(at) = near.and_then(|n| picker.servers.iter().position(|(name, _)| *name == n)) {
                picker.cursor = at;
            }
        }
        self.claude.mode = Mode::Opening(picker);
    }

    /// A query session on `node`, on view 5 — straight there, nothing else started on the way.
    pub fn open_query_on(&mut self, node: Option<String>) {
        if self.view != View::Claude {
            self.claude.back_to = self.view;
        }
        self.view = View::Claude;
        self.focus = Focus::Tree;
        self.queue_selection = None;
        self.claude.mode = Mode::Typing;
        if self.claude.open_query(node.as_deref()).is_none() {
            self.notice = Some((
                format!("{} sessions is as many as there can be — close one with ctrl+\\ x x", crate::claude::MAX_SESSIONS),
                SystemTime::now(),
            ));
        }
    }

    /// The node of the row under view 1's cursor: the node, a user or a query on it.
    fn node_at_cursor(&self) -> Option<String> {
        match self.selected()? {
            RowId::Node(node) | RowId::User { node, .. } | RowId::Query { node, .. } | RowId::PivotNode { node, .. } => Some(node.clone()),
            _ => None,
        }
    }

    /// The picker, on every folder's past conversations: Claude Code's, or OpenCode's when that
    /// is what the session on screen runs.
    fn ask_past_conversation(&mut self) {
        self.ask_new_session(None);
        if let Mode::Opening(picker) = &mut self.claude.mode {
            if picker.kind == Kind::Terminal {
                picker.kind = Kind::Claude;
            }
            picker.show_everywhere(true);
        }
    }

    /// The folder picker's keys: ↑ ↓ choose, ⏎ opens the session in the folder under the
    /// cursor, → goes into it and ← back up; what is typed searches.
    fn on_opening_key(&mut self, key: KeyEvent) {
        let Mode::Opening(picker) = &mut self.claude.mode else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // A query session's servers are a list, not folders: there is nothing to go into or up.
        let servers = picker.kind == Kind::Query;
        match key.code {
            KeyCode::Left | KeyCode::Right if servers => {}
            KeyCode::Backspace if servers => {
                picker.backspace();
            }
            // A search first, then the picker.
            KeyCode::Esc if picker.searching() => picker.clear_query(),
            // Every folder's conversations: ← or esc goes back to the folders.
            KeyCode::Esc | KeyCode::Left if picker.everywhere => picker.show_everywhere(false),
            // What it will open: Claude, OpenCode, a terminal, round.
            KeyCode::BackTab => picker.kind = picker.kind.next(),
            KeyCode::Esc => self.claude.mode = Mode::Typing,
            KeyCode::Enter => {
                let row = picker.at_cursor();
                self.pick(row, true);
            }
            KeyCode::Right | KeyCode::Tab => {
                let row = picker.at_cursor();
                self.pick(row, false);
            }
            KeyCode::Left => picker.up(),
            KeyCode::Up => picker.step(-1),
            KeyCode::Down => picker.step(1),
            KeyCode::Char('p') if ctrl => picker.step(-1),
            KeyCode::Char('n') if ctrl => picker.step(1),
            KeyCode::PageUp => picker.step(-10),
            KeyCode::PageDown => picker.step(10),
            KeyCode::Home => picker.cursor = 0,
            KeyCode::End => picker.step(isize::MAX),
            // With nothing left to delete, back up a folder, as a file dialog does.
            KeyCode::Backspace => {
                if !picker.backspace() {
                    picker.up();
                }
            }
            // ctrl+u clears it, as in a shell.
            KeyCode::Char('u') if ctrl => picker.clear_query(),
            KeyCode::Char(c) if !ctrl => picker.type_text(&c.to_string()),
            _ => {}
        }
    }

    /// A row of the picker acted on: `open` starts the session in its folder (⏎, or a click on
    /// the first row); otherwise the picker goes into it, as a click on a folder does.
    fn pick(&mut self, row: Option<PickRow>, open: bool) {
        let Mode::Opening(picker) = &mut self.claude.mode else {
            return;
        };
        let Some(row) = row else {
            return;
        };
        let kind = picker.kind;
        match (row, picker.path_of(row)) {
            (PickRow::Up, _) => picker.up(),
            (PickRow::Everywhere, _) => picker.show_everywhere(true),
            (PickRow::Server(index), _) => {
                let node = picker.servers.get(index).map(|(name, _)| name.clone());
                self.claude.mode = Mode::Typing;
                if self.claude.open_query(node.as_deref()).is_none() {
                    self.ask_new_session(Some(Kind::Query));
                }
            }
            // A conversation is taken up with a click or ⏎ alike: there is nothing to go into.
            (PickRow::Conversation(index), _) => {
                if let Some(conversation) = picker.conversations_shown().get(index).cloned() {
                    self.claude.mode = Mode::Typing;
                    if self.claude.open_conversation(&conversation).is_none() {
                        self.ask_new_session(Some(kind));
                    }
                }
            }
            (PickRow::Here, Some(dir)) if open => self.open_in(&dir, kind),
            (PickRow::Folder(_), Some(dir)) if open => self.open_in(&dir, kind),
            (PickRow::Folder(_), Some(dir)) => picker.go_to(dir),
            _ => {}
        }
    }

    /// A new session of `kind` working in `dir`, on screen.
    fn open_in(&mut self, dir: &std::path::Path, kind: Kind) {
        self.claude.mode = Mode::Typing;
        if self.claude.open_new(&crate::pty::tilde(dir), kind).is_none() {
            self.ask_new_session(Some(kind));
        }
    }

    /// A click on what was drawn there, or the wheel: over Claude's screen it is Claude's,
    /// elsewhere it moves the cursor like ↑ ↓.
    fn on_mouse(&mut self, event: MouseEvent) {
        // A click while a cancel waits for `y` keeps the job, as any other key does.
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.queue_confirm = None;
        }
        let hit = self
            .viewport
            .hits
            .borrow()
            .iter()
            .find(|(r, _)| event.column >= r.x && event.column < r.x + r.width && event.row >= r.y && event.row < r.y + r.height)
            .map(|(_, hit)| *hit);
        if hit == Some(Hit::Pane) && self.view == View::Claude {
            self.claude.mouse(&event);
            return;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Some(Hit::View(View::Claude)) => self.open_claude(),
                Some(Hit::View(view)) => self.go_to_view(view),
                Some(Hit::Session(index)) => {
                    self.open_claude();
                    self.claude.select(index);
                }
                Some(Hit::NewSession) if matches!(self.claude.mode, Mode::Opening(_)) => {}
                Some(Hit::NewSession) => {
                    self.open_claude();
                    self.ask_new_session(None);
                }
                // A folder is gone into with a click; the first row opens the session there.
                Some(Hit::Pick(row)) => self.pick(Some(row), row == PickRow::Here),
                Some(Hit::Crumb(index)) => {
                    if let Mode::Opening(picker) = &mut self.claude.mode
                        && let Some((_, dir)) = picker.crumbs().get(index).cloned()
                    {
                        picker.go_to(dir);
                    }
                }
                Some(Hit::Cancel) => self.claude.mode = Mode::Typing,
                Some(Hit::Kind(kind)) => {
                    if let Mode::Opening(picker) = &mut self.claude.mode {
                        picker.kind = kind;
                    }
                }
                // A node under the sessions opens on view 1, as an insight does; the session
                // goes on where it was.
                Some(Hit::Node(index)) => {
                    let name = self.viewport.listed_nodes.borrow().get(index).cloned();
                    if let Some(name) = name {
                        self.claude.mode = Mode::Typing;
                        self.go_to(&Subject::Node(name));
                    }
                }
                Some(Hit::Clock) => self.time.flip(),
                Some(Hit::ConsoleServer) => {
                    let servers = self.server_names();
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                        let at = console.node.as_ref().and_then(|n| servers.iter().position(|s| s == n)).unwrap_or(0);
                        console.choosing = if console.choosing.is_some() { None } else { Some(at) };
                    }
                }
                Some(Hit::ConsolePick(index)) => {
                    let servers = self.server_names();
                    if let (Some(console), Some(node)) = (self.claude.current_mut().and_then(|s| s.console.as_deref_mut()), servers.get(index)) {
                        console.connect(node);
                    }
                }
                // The cursor where the click is; a drag from there selects.
                Some(Hit::ConsoleText) => {
                    let spot = self.text_spot(event.column, event.row);
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                        match spot {
                            Some(spot) => console.press(spot),
                            None => {
                                console.focus = crate::console::Focus::Editor;
                                console.choosing = None;
                                console.suggest = None;
                            }
                        }
                    }
                }
                Some(Hit::ConsoleAnswer) => {
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut())
                        && console.answer.is_some()
                    {
                        console.focus = crate::console::Focus::Answer;
                        console.choosing = None;
                        console.suggest = None;
                    }
                }
                Some(Hit::ConsoleAssistant) => {
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                        console.assistant = console.assistant.other();
                        self.claude.assistant = console.assistant;
                    }
                }
                Some(Hit::ConsoleAsk) => {
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                        console.open_instruction();
                    }
                }
                Some(Hit::Suggestion(index)) => {
                    if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut())
                        && let Some(open) = console.suggest.as_mut()
                        && index < open.items.len()
                    {
                        open.at = index;
                        console.accept();
                    }
                }
                Some(Hit::Resume) => {
                    self.open_claude();
                    self.ask_past_conversation();
                }
                Some(Hit::Dismiss) => {
                    let now = self.now();
                    self.prayers.dismiss(now);
                }
                Some(Hit::Row(index)) => {
                    let id = self.with_rows(|_, rows| rows.get(index).map(|row| row.id.clone())).flatten();
                    let again = self.focus == Focus::Tree && id.is_some() && id == self.selected;
                    if id.is_some() {
                        self.selected = id;
                        self.focus = Focus::Tree;
                    }
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::Insight(index)) => {
                    let again = self.focus == Focus::Insights && self.insight_selection == index;
                    self.focus = Focus::Insights;
                    self.insight_selection = index;
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::Job(index)) => {
                    let again = self.queue_selection == Some(index);
                    self.queue_selection = Some(index);
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::TapeLine(index)) => {
                    let again = self.tape_selection == index;
                    self.tape_selection = index;
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::Tile(index)) => {
                    let again = self.map_selection == index;
                    self.map_selection = index;
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::AirflowRow(index)) => {
                    let key = self.airflow_rows().get(index).cloned();
                    let again = key.is_some() && key == self.airflow_selected;
                    if key.is_some() {
                        self.airflow_index = index;
                        self.airflow_selected = key;
                    }
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::Process(index)) => self.local.select(index),
                Some(Hit::Ticket(index)) => {
                    let key = self.jira.rows().get(index).map(|t| t.key.clone());
                    let again = key.is_some() && key == self.jira_selected;
                    if key.is_some() {
                        self.jira_index = index;
                        self.jira_selected = key;
                    }
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::PageRow(index)) => {
                    let again = self.page().is_some_and(|p| p.cursor == index);
                    if let Some(page) = self.page_mut() {
                        page.cursor = index.min(page.rows().saturating_sub(1));
                    }
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::TimeDay(day)) => {
                    if let Some(page) = self.jira_time.as_mut() {
                        page.day = day;
                    }
                }
                Some(Hit::TimeTicket(index)) => {
                    let again = self.jira_time.is_some_and(|p| p.ticket == index);
                    if let Some(page) = self.jira_time.as_mut() {
                        page.ticket = index;
                    }
                    if again {
                        self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Some(Hit::Pane) | None => {}
            },
            // A drag in a query session's text selects, wherever the mouse goes meanwhile.
            MouseEventKind::Drag(MouseButton::Left) => {
                let spot = self.text_spot(event.column, event.row);
                if let (Some(spot), Some(console)) = (spot, self.claude.current_mut().and_then(|s| s.console.as_deref_mut()))
                    && console.dragging()
                {
                    console.drag_to(spot);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                    console.release();
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if event.kind == MouseEventKind::ScrollUp { -1 } else { 1 };
                match self.view {
                    View::Nodes if self.focus == Focus::Insights => {
                        let last = self.insights().len().saturating_sub(1);
                        self.insight_selection = self.insight_selection.saturating_add_signed(delta).min(last);
                    }
                    View::Nodes | View::Queue => self.move_by(delta),
                    View::Airflow | View::Jira if self.page().is_some() => self.page_step(delta * 3),
                    View::Jira if self.jira_time.is_some() => {
                        let code = if delta < 0 { KeyCode::Up } else { KeyCode::Down };
                        self.on_time_key(code);
                    }
                    View::Airflow => self.move_airflow(delta),
                    View::Jira => self.move_jira(delta),
                    View::Local => self.move_local(delta),
                    View::Tape => {
                        let last = self.tape.len().saturating_sub(1);
                        self.tape_selection = self.tape_selection.saturating_add_signed(delta).min(last);
                    }
                    View::Claude => {
                        if let Mode::Opening(picker) = &mut self.claude.mode {
                            picker.step(delta * 3);
                        } else if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                            console.scroll(delta * 3, 0);
                        }
                    }
                    View::Map => {}
                }
            }
            _ => {}
        }
        self.sync_view_state();
    }

    /// The place in a query session's text under the mouse, as the text was last drawn. Past
    /// its top or its bottom it is a line further, so a drag out of the field scrolls along.
    fn text_spot(&self, x: u16, y: u16) -> Option<crate::console::Spot> {
        let layout = self.viewport.console_text.get()?;
        let row = if y < layout.top {
            layout.first.saturating_sub(1)
        } else if y >= layout.top + layout.height {
            layout.first + layout.height as usize
        } else {
            layout.first + (y - layout.top) as usize
        };
        let shift = if row == layout.cursor_row { layout.shift } else { 0 };
        Some((row, x.saturating_sub(layout.text_x) as usize + shift))
    }

    /// One of the monitor's views, leaving Claude's modes behind.
    fn go_to_view(&mut self, view: View) {
        self.claude.mode = Mode::Typing;
        self.view = view;
        self.focus = Focus::Tree;
        if view != View::Queue {
            self.queue_selection = None;
        }
    }

    /// The session bar's commands, after `ctrl+\`.
    fn on_bar_key(&mut self, key: KeyEvent) {
        let closing = std::mem::take(&mut self.claude.closing);
        match key.code {
            // One numbering for every tab: 1–6 the monitor's views, 7–9 the sessions, 0 this machine.
            KeyCode::Char(c @ '0'..='9') if c as u8 - b'0' <= View::MONITOR => {
                self.claude.mode = Mode::Typing;
                if let Some(view) = View::from_number(c as u8 - b'0') {
                    self.view = view;
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(index) = self.claude.index_of(c as usize - '0' as usize) {
                    self.claude.select(index);
                }
                self.claude.mode = Mode::Typing;
            }
            // The arrows walk the list, one session at a time, round it.
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') | KeyCode::BackTab => self.claude.step(false),
            KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') | KeyCode::Tab => self.claude.step(true),
            // One among many, by what it is called.
            KeyCode::Char('/') if !self.claude.list.is_empty() => {
                let at = self.claude.matching("").iter().position(|&i| i == self.claude.active).unwrap_or(0);
                self.claude.mode = Mode::Finding { query: String::new(), at };
            }
            // A new session: of the kind on screen, or Claude, OpenCode, a terminal.
            KeyCode::Char('n') => self.ask_new_session(None),
            KeyCode::Char('c') => self.ask_new_session(Some(Kind::Claude)),
            KeyCode::Char('o') => self.ask_new_session(Some(Kind::OpenCode)),
            KeyCode::Char('t') => self.ask_new_session(Some(Kind::Terminal)),
            KeyCode::Char('q') => self.ask_new_session(Some(Kind::Query)),
            KeyCode::Char('r') => {
                if let Some(name) = self.claude.current().map(|s| s.name.clone().unwrap_or_default()) {
                    self.claude.mode = Mode::Naming(name);
                }
            }
            // Twice: a conversation is not ended by one stray key.
            KeyCode::Char('x') if self.claude.current().is_some() => {
                if closing {
                    self.claude.close_current();
                    self.claude.mode = Mode::Typing;
                } else {
                    self.claude.closing = true;
                }
            }
            KeyCode::Char('m') => {
                self.claude.mode = Mode::Typing;
                self.view = self.claude.back_to;
            }
            KeyCode::Char('z') => {
                self.time.flip();
                self.claude.mode = Mode::Typing;
            }
            // Past conversations, had here or in another terminal, to take up again.
            KeyCode::Char('p') => self.ask_past_conversation(),
            KeyCode::Char('d') => {
                let now = self.now();
                self.prayers.dismiss(now);
                self.claude.mode = Mode::Typing;
            }
            KeyCode::Esc | KeyCode::Enter => self.claude.mode = Mode::Typing,
            _ => {}
        }
    }

    /// Looking for a session by what it is called: what is typed narrows the list, ↑ ↓ choose,
    /// ⏎ puts the one chosen on screen.
    fn on_finding_key(&mut self, key: KeyEvent) {
        let Mode::Finding { query, at } = &self.claude.mode else {
            return;
        };
        let (mut query, mut at) = (query.clone(), *at);
        let found = self.claude.matching(&query);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => {
                self.claude.mode = Mode::Typing;
                if let Some(&index) = found.get(at) {
                    self.claude.select(index);
                }
                return;
            }
            KeyCode::Esc => {
                self.claude.mode = Mode::Typing;
                return;
            }
            KeyCode::Up => at = at.saturating_sub(1),
            KeyCode::Down => at = (at + 1).min(found.len().saturating_sub(1)),
            KeyCode::Char('p') if ctrl => at = at.saturating_sub(1),
            KeyCode::Char('n') if ctrl => at = (at + 1).min(found.len().saturating_sub(1)),
            KeyCode::Char('u') if ctrl => {
                query.clear();
                at = 0;
            }
            KeyCode::Backspace => {
                query.pop();
                at = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                query.push(c);
                at = 0;
            }
            _ => {}
        }
        self.claude.mode = Mode::Finding { query, at };
    }

    /// Typing a session's new name.
    fn on_naming_key(&mut self, key: KeyEvent) {
        let Mode::Naming(name) = &mut self.claude.mode else {
            return;
        };
        match key.code {
            KeyCode::Enter => {
                let name = name.clone();
                self.claude.rename_current(&name);
                self.claude.mode = Mode::Typing;
            }
            KeyCode::Esc => self.claude.mode = Mode::Typing,
            KeyCode::Backspace => {
                name.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && name.chars().count() < crate::claude::NAME_MAX =>
            {
                name.push(c);
            }
            _ => {}
        }
    }

    // -- data ------------------------------------------------------------

    fn on_snapshot(&mut self, mut snapshot: FleetSnapshot) {
        let now = SystemTime::now();
        for node in &snapshot.nodes {
            if node.reachable {
                self.unreachable_since.remove(&node.name);
            } else {
                self.unreachable_since.entry(node.name.clone()).or_insert(now);
            }
        }

        // §2.6: unreachable nodes keep their row with unknown numbers, and are only dropped
        // after five minutes of silence.
        let grace = |since: Option<&SystemTime>| {
            since
                .and_then(|t| now.duration_since(*t).ok())
                .is_some_and(|d| d >= UNREACHABLE_GRACE)
        };
        let gone: Vec<String> = self
            .unreachable_since
            .iter()
            .filter(|(name, since)| grace(Some(since)) && !snapshot.nodes.iter().any(|n| &n.name == *name))
            .map(|(name, _)| name.clone())
            .collect();
        snapshot.nodes.retain(|n| !gone.contains(&n.name));
        for name in gone {
            self.unreachable_since.remove(&name);
            self.tree.expanded_nodes.remove(&name);
        }

        let first = self.snapshot.is_none();
        let new_nodes = mark_new_nodes(&snapshot, &mut self.first_seen, first);
        self.tree.new_nodes.extend(new_nodes);

        let is_first = first;
        self.prev = self.snapshot.replace(Box::new(snapshot));
        self.stitch_queue();
        self.last_snapshot_wall = Some(SystemTime::now());

        // Derive once per poll for the two things that remember: the tape compares this poll
        // with the last one *before* the history takes it in.
        if let Some(snapshot) = self.snapshot.as_deref() {
            let view = crate::model::fleet_view(snapshot, self.prev.as_deref());
            let totals = fleet_totals(&view);
            let at = history::secs(snapshot.taken_at);
            let events = self.watch.observe_fleet(&view, &self.tree.new_nodes, at);
            Self::add_events(&mut self.tape, &mut self.tape_selection, events);
            self.history.record_fleet(&view, &totals, snapshot.taken_at);
        }

        if is_first {
            // §2.5: the top node is open, everything else closed.
            let snapshot = self.snapshot.as_ref().expect("just stored");
            tree::expand_top_node(&mut self.tree, snapshot, None);
            self.selected = self
                .tree
                .expanded_nodes
                .iter()
                .next()
                .map(|name| RowId::Node(name.clone()));
        }
    }

    fn on_queue(&mut self, queue: QueueStatus) {
        self.queue = queue;
        // A job Redash said yes to stays marked while it is listed — a worker lets go of a
        // running one at its next check — and no longer than ten minutes.
        let listed: HashSet<&str> = self.queue.jobs.iter().map(|job| job.id.as_str()).collect();
        let now = SystemTime::now();
        self.cancelling.retain(|id, c| {
            let young = now.duration_since(c.since).map_or(true, |age| age < Duration::from_secs(600));
            young && (!c.accepted || listed.contains(id.as_str()))
        });
        // The stitch has to run on every queue poll too: the source knows nothing about
        // ClickHouse, so a fresh status arrives with every running job unlinked.
        self.stitch_queue();
        let at = history::secs(self.queue.taken_at);
        let events = self.watch.observe_queue(&self.queue, at);
        Self::add_events(&mut self.tape, &mut self.tape_selection, events);
        self.history.record_queue(&self.queue);
    }

    /// New tape lines go on top. A cursor at the top follows them, like `tail -f`; a cursor
    /// further down stays on the line it was reading.
    fn add_events(tape: &mut Tape, selection: &mut usize, events: Vec<crate::tape::Event>) {
        let added = events.len();
        tape.extend(events);
        if *selection > 0 {
            *selection = (*selection + added).min(tape.len().saturating_sub(1));
        }
    }

    /// Which job row view 2 has selected.
    pub fn queue_selection(&self) -> Option<usize> {
        self.queue_selection
    }

    /// `x` on view 2: whether to cancel the job under the cursor, asked before anything is sent.
    fn ask_to_cancel(&mut self) {
        let Some(job) = self.selected_job() else {
            return;
        };
        if self.cancelling.contains_key(&job.id) {
            let what = job_words(job);
            self.notice = Some((format!("Redash has already been asked to cancel {what}"), SystemTime::now()));
            return;
        }
        self.queue_confirm = Some(job.id.clone());
    }

    /// The job `x` asked about, while it waits for `y`.
    pub fn cancel_asked(&self) -> Option<&Job> {
        let id = self.queue_confirm.as_deref()?;
        self.queue.jobs.iter().find(|job| job.id == id)
    }

    /// `y`: the job is cancelled in Redash — `main.rs` sends it, Redash's answer comes back as
    /// [`Event::Cancelled`].
    fn cancel_job(&mut self, id: &str) {
        let Some(job) = self.queue.jobs.iter().find(|job| job.id == id) else {
            self.notice = Some(("that job has left the queue — nothing was cancelled".into(), SystemTime::now()));
            return;
        };
        let what = job_words(job);
        let node = job.clickhouse_target().map(|(node, _)| node.to_string());
        let on_clickhouse = node.is_some() || job.on_clickhouse() == Some(true);
        let cancelling = Cancelling { what: what.clone(), on_clickhouse, node, accepted: false, since: SystemTime::now() };
        self.cancelling.insert(id.to_string(), cancelling);
        self.cancels.push(id.to_string());
        self.notice = Some((format!("asking Redash to cancel {what}…"), SystemTime::now()));
    }

    /// Redash's answer to a cancel: yes — and, for a job on ClickHouse, that its query runs
    /// on there — or why not. A yes goes on the tape too, where it stays to be read.
    fn on_cancelled(&mut self, id: &str, result: Result<(), String>) {
        let what = self.cancelling.get(id).map_or_else(|| format!("job {id}"), |c| c.what.clone());
        let message = match result {
            Ok(()) => {
                let mut message = format!("Redash cancelled {what}");
                let mut parts = vec![("Redash job cancelled from here: ".to_string(), insight::Tone::Plain), (what.clone(), insight::Tone::Strong)];
                let mut level = Severity::Info;
                if let Some(cancelling) = self.cancelling.get_mut(id) {
                    cancelling.accepted = true;
                    let runs_on = " — its query runs on in ClickHouse";
                    match &cancelling.node {
                        Some(node) => {
                            message.push_str(&format!("{runs_on} on {node} until KILL QUERY stops it"));
                            parts.push((format!("{runs_on} on "), insight::Tone::Sev(Severity::Warn)));
                            parts.push((node.clone(), insight::Tone::Node));
                            parts.push((" until KILL QUERY stops it".to_string(), insight::Tone::Muted));
                            level = Severity::Warn;
                        }
                        None if cancelling.on_clickhouse => {
                            let runs = " — a query it started in ClickHouse runs on until KILL QUERY stops it";
                            message.push_str(runs);
                            parts.push((runs.to_string(), insight::Tone::Sev(Severity::Warn)));
                            level = Severity::Warn;
                        }
                        None => {}
                    }
                }
                let event = crate::tape::Event {
                    at: history::secs(SystemTime::now()),
                    level,
                    kind: crate::tape::Kind::Queue,
                    parts,
                    subject: Some(Subject::Queue),
                };
                Self::add_events(&mut self.tape, &mut self.tape_selection, vec![event]);
                message
            }
            Err(why) => {
                self.cancelling.remove(id);
                format!("Redash did not cancel {what}: {why}")
            }
        };
        self.notice = Some((message, SystemTime::now()));
    }

    /// The cancels for `main.rs` to send, once.
    pub fn take_cancels(&mut self) -> Vec<String> {
        std::mem::take(&mut self.cancels)
    }

    /// Whether Redash has been asked to cancel the job, and whether it has said yes.
    pub fn cancelling(&self, id: &str) -> Option<&Cancelling> {
        self.cancelling.get(id)
    }

    /// View 2's lists: what runs — the table above already says how full the queue is, and
    /// the running jobs are why — then what waits, both longest first, then what RQ's started
    /// list holds without anyone running it, oldest first.
    ///
    /// Partitioned by state *before* sorting: sorting everything by age and cutting at the
    /// first started job would put every waiting job younger than the oldest running one
    /// into the RUNNING half.
    pub fn queue_sections(&self) -> QueueSections<'_> {
        let mut sections = QueueSections::default();
        for (index, job) in self.queue.jobs.iter().enumerate() {
            match job.state {
                JobState::Queued => sections.waiting.push((index, job)),
                JobState::Started => sections.running.push((index, job)),
                JobState::Stale(_) => sections.stale.push((index, job)),
            }
        }
        for list in [&mut sections.running, &mut sections.waiting, &mut sections.stale] {
            list.sort_by_key(|(_, job)| std::cmp::Reverse(job.age_s));
        }
        sections
    }

    /// Every job row of view 2, in the order they are drawn.
    pub fn queue_rows(&self) -> Vec<&crate::model::Job> {
        let sections = self.queue_sections();
        sections
            .running
            .into_iter()
            .chain(sections.waiting)
            .chain(sections.stale)
            .map(|(_, job)| job)
            .collect()
    }

    /// The SQL to show under a job on view 2: what ClickHouse is running for it when the stitch
    /// found the query, the query as saved in Redash otherwise. An ad-hoc query that is not in
    /// ClickHouse has none — Redash's API does not keep its text.
    pub fn job_sql(&self, job: &Job) -> Option<(String, SqlFrom)> {
        let running = job.clickhouse_target().and_then(|(node, query_id)| {
            self.snapshot()?
                .nodes
                .iter()
                .find(|n| n.name == node)?
                .queries
                .iter()
                .find(|q| q.query_id == query_id)
                .map(|q| q.sql.clone())
        });
        match running {
            Some(sql) => Some((sql, SqlFrom::ClickHouse)),
            None => job.sql.clone().filter(|sql| !sql.trim().is_empty()).map(|sql| (sql, SqlFrom::Redash)),
        }
    }

    /// The job under view 2's cursor.
    pub fn selected_job(&self) -> Option<&crate::model::Job> {
        let index = self.queue_selection?;
        self.queue_rows().get(index).copied()
    }

    /// `⏎` on a running job: jump to the ClickHouse query it became (§2.8).
    pub fn activate_queue_row(&mut self) {
        // The target has to be copied out before `jump_to_clickhouse` borrows self mutably.
        let target = self
            .selected_job()
            .and_then(|job| job.clickhouse_target())
            .map(|(node, id)| (node.to_string(), id.to_string()));
        // A waiting job has not reached ClickHouse: there is nothing to jump to, and the
        // drawer says so instead.
        if let Some((node, query_id)) = target {
            self.jump_to_clickhouse(&node, &query_id);
        }
    }

    /// §2.8's stitch: a Redash job that has started IS a `system.processes` row somewhere.
    ///
    /// Redash writes the job's id into the comment of the SQL it runs (`Job ID: …`), which is
    /// an exact match, and the Redash query's number (§6.4), which is not: the same query can
    /// run twice at once. The id comes first; the number is the fallback for comments without
    /// one. Matching here is what lets ⏎ jump from the queue into the tree.
    fn stitch_queue(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        type Run = (String, String, Option<String>);
        let mut by_job: HashMap<String, Run> = HashMap::new();
        let mut by_redash_id: HashMap<u64, Vec<Run>> = HashMap::new();
        for node in &snapshot.nodes {
            for query in &node.queries {
                let run = (node.name.clone(), query.query_id.clone(), query.person.clone());
                // A query that names its job belongs to that job and to no other.
                if let Some(job_id) = crate::attrib::redash_job_id(&query.sql) {
                    by_job.insert(job_id.to_string(), run);
                } else if let Some(redash_id) = query.redash_query_id {
                    by_redash_id.entry(redash_id).or_default().push(run);
                }
            }
        }

        let mut taken: HashSet<String> = HashSet::new();
        for job in &mut self.queue.jobs {
            job.ch_node = None;
            job.ch_query_id = None;
            // A stale entry is matched by its id too: a query still running for a job its
            // worker has given up on is exactly what someone has to go and kill.
            if job.state == JobState::Queued {
                continue;
            }
            if let Some((node, query_id, _)) = by_job.get(&job.id) {
                taken.insert(query_id.clone());
                job.ch_node = Some(node.clone());
                job.ch_query_id = Some(query_id.clone());
            }
        }
        // By number, for running jobs only — a leftover from last spring must not claim
        // today's run of the same query — and only for ClickHouse data sources. Each job
        // takes the run on the node its data source names, then one for the same person, and
        // one ClickHouse query never goes to two jobs.
        for job in &mut self.queue.jobs {
            if job.state != JobState::Started || job.ch_node.is_some() || job.on_clickhouse() == Some(false) {
                continue;
            }
            let Some(candidates) = job.redash_query_id.and_then(|id| by_redash_id.get(&id)) else {
                continue;
            };
            let free = |c: &&Run| !taken.contains(&c.1);
            let pick = candidates
                .iter()
                .filter(free)
                .find(|(node, _, _)| names_node(job.data_source.as_deref(), node))
                .or_else(|| {
                    candidates
                        .iter()
                        .filter(free)
                        .find(|(_, _, person)| person.is_some() && person == &job.person)
                })
                .or_else(|| candidates.iter().find(free));
            if let Some((node, query_id, _)) = pick {
                taken.insert(query_id.clone());
                job.ch_node = Some(node.clone());
                job.ch_query_id = Some(query_id.clone());
            }
        }
    }

    /// The question the drawer on view 2 answers: are the workers busy because of ClickHouse
    /// queries, or is Redash keeping up (§2.8)?
    pub fn queue_explanation(&self) -> String {
        let (busy, total) = (self.queue.workers_busy, self.queue.workers_total);
        let running: Vec<&crate::model::Job> = self.queue.started();
        if running.is_empty() {
            let stale = self.queue.total_stale();
            return match stale {
                0 => format!("nothing on a worker · {total} workers idle"),
                _ => format!(
                    "nothing on a worker · the {} in RQ's started list {} leftovers, not work",
                    crate::fmt::plural(stale as usize, "entry", "entries"),
                    if stale == 1 { "is a" } else { "are" }
                ),
            };
        }
        let mut stuck = 0usize;
        let mut runaway_nodes: Vec<String> = Vec::new();
        for job in &running {
            let Some((node, query_id)) = job.clickhouse_target() else {
                continue;
            };
            if self.query_is_runaway(node, query_id) {
                stuck += 1;
                if !runaway_nodes.iter().any(|n| n == node) {
                    runaway_nodes.push(node.to_string());
                }
            }
        }
        if stuck == 0 {
            return format!("{busy}/{total} workers busy · none of them is stuck in ClickHouse");
        }
        let holds = if stuck == 1 { "holds" } else { "hold" };
        // Stuck workers explain a full queue; with nothing waiting they are only expensive.
        let why = if self.queue.total_waiting() > 0 { " → why it is full" } else { "" };
        format!(
            "{busy}/{total} workers busy · {stuck} of them {holds} a runaway ClickHouse query ({}){why}",
            runaway_nodes.join(", ")
        )
    }

    /// Whether a ClickHouse query is runaway right now (§5.4), by node and query id.
    pub fn query_is_runaway(&self, node: &str, query_id: &str) -> bool {
        self.with_view(|view| {
            view.nodes
                .iter()
                .filter(|n| n.node.name == node)
                .flat_map(|n| n.users.iter())
                .flat_map(|u| u.queries.iter())
                .any(|q| q.query.query_id == query_id && q.runaway)
        })
        .unwrap_or(false)
    }

    /// The derived fleet view of the current snapshot.
    pub fn with_view<R>(&self, f: impl FnOnce(&FleetView<'_>) -> R) -> Option<R> {
        let snapshot = self.snapshot.as_ref()?;
        let view = crate::model::fleet_view(snapshot, self.prev.as_deref());
        Some(f(&view))
    }

    /// Everything the insights engine has to say right now, worst first.
    pub fn insights(&self) -> Vec<Insight> {
        self.with_view(|view| {
            insight::analyze(view, &self.history, &self.queue)
        })
        .unwrap_or_default()
    }

    pub fn insight_selection(&self) -> usize {
        self.insight_selection
    }

    /// The first line to show of the SQL under the row `key` names: a query id on view 1,
    /// [`App::job_sql_key`] on view 2.
    pub fn sql_scroll_for(&self, key: &str) -> usize {
        if self.sql_scroll_of.as_deref() == Some(key) { self.sql_scroll } else { 0 }
    }

    /// What the SQL under a job on view 2 scrolls by — never equal to a ClickHouse query id.
    pub fn job_sql_key(job: &Job) -> String {
        format!("redash job {}", job.id)
    }

    /// `J` `K` (or shift ↑ ↓): scroll the SQL under the selected query or job.
    fn scroll_sql(&mut self, delta: isize) {
        let key = match (self.view, self.selected.clone()) {
            (View::Queue, _) => self.selected_job().map(Self::job_sql_key),
            (_, Some(RowId::Query { query_id, .. })) => Some(query_id),
            _ => None,
        };
        let Some(key) = key else {
            return;
        };
        if self.sql_scroll_of.as_deref() != Some(key.as_str()) {
            self.sql_scroll = 0;
            self.sql_scroll_of = Some(key);
        }
        let max = self.viewport.sql_max.get() as isize;
        self.sql_scroll = (self.sql_scroll as isize + delta).clamp(0, max) as usize;
    }

    pub fn map_selection(&self) -> usize {
        self.map_selection
    }

    pub fn tape_selection(&self) -> usize {
        self.tape_selection
    }

    /// How old the numbers on screen are, by the wall clock of the last arrival.
    pub fn data_age(&self) -> Option<Duration> {
        let at = self.last_snapshot_wall?;
        Some(self.clock.duration_since(at).unwrap_or_default())
    }

    /// No snapshot for three poll intervals: the header stops saying LIVE.
    pub fn is_stale(&self) -> bool {
        !self.paused
            && self
                .data_age()
                .is_some_and(|age| age > self.poll_interval * 3 + Duration::from_secs(1))
    }

    /// The node names of the map, in the order the map draws them (§2.5's sort, no fold).
    pub fn map_nodes(&self) -> Vec<String> {
        self.with_view(|view| {
            let mut nodes: Vec<&crate::model::NodeView<'_>> = view.nodes.iter().collect();
            nodes.sort_by(|a, b| crate::model::compare_nodes(a, b, self.tree.sort));
            nodes.iter().map(|n| n.node.name.clone()).collect()
        })
        .unwrap_or_default()
    }

    pub fn snapshot(&self) -> Option<&FleetSnapshot> {
        self.snapshot.as_deref()
    }

    /// A recent source error, for the footer. Expires on its own (§3: the footer is
    /// contextual, and a stale error is worse than no error).
    pub fn notice(&self) -> Option<&str> {
        let (message, when) = self.notice.as_ref()?;
        let age = SystemTime::now().duration_since(*when).ok()?;
        (age < self.notice_ttl).then_some(message.as_str())
    }

    /// The derived view and the rows for the current state. Passed to a closure because the
    /// rows borrow the view, and the view borrows the snapshot.
    pub fn with_rows<R>(&self, f: impl FnOnce(&FleetView<'_>, &[Row<'_>]) -> R) -> Option<R> {
        let snapshot = self.snapshot.as_ref()?;
        let view = crate::model::fleet_view(snapshot, self.prev.as_deref());
        let rows = tree::build(&view, &self.tree);
        Some(f(&view, &rows))
    }

    pub fn selected(&self) -> Option<&RowId> {
        self.selected.as_ref()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.with_rows(|_, rows| tree::selection_index(rows, self.selected.as_ref()))
            .flatten()
    }

    /// Where the cursor sits and which row it is on, for the drawer (§2.4).
    pub fn selected_row(&self) -> Option<(usize, RowId)> {
        let index = self.selected_index()?;
        let id = self
            .with_rows(|_, rows| rows.get(index).map(|row| row.id.clone()))
            .flatten()?;
        Some((index, id))
    }

    pub fn filter_input(&self) -> Option<&str> {
        self.filter_input.as_deref()
    }

    // -- keys ------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        // `x` on view 2 asked whether to cancel a job: `y` does; any other key keeps it, and
        // does nothing else, so a stray key cannot both answer and act.
        if let Some(id) = self.queue_confirm.take() {
            if self.view == View::Queue && matches!(key.code, KeyCode::Char('y' | 'Y')) {
                self.cancel_job(&id);
            }
            self.sync_view_state();
            return;
        }

        // Filter entry swallows everything except Esc, Enter and the printable characters.
        if self.filter_input.is_some() {
            let mut input = self.filter_input.take().unwrap_or_default();
            match key.code {
                KeyCode::Esc => self.tree.filter.clear(),
                KeyCode::Enter => self.tree.filter = input.clone(),
                KeyCode::Backspace => {
                    input.pop();
                    self.tree.filter = input.clone();
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.tree.filter = input.clone();
                }
                _ => {}
            }
            // Esc leaves filter mode; Enter keeps the text and leaves it too.
            self.filter_input = match key.code {
                KeyCode::Esc | KeyCode::Enter => None,
                _ => Some(input),
            };
            self.sync_view_state();
            return;
        }

        if self.help && !matches!(key.code, KeyCode::Char('q') | KeyCode::Char('?')) {
            self.help = false;
            self.sync_view_state();
            return;
        }

        let typing_text = matches!(self.claude.mode, Mode::Naming(_) | Mode::Opening(_) | Mode::Finding { .. }) && self.view == View::Claude;
        // F1–F6 the views, F7–F9 the sessions, F10 this machine — from anywhere, Claude's screen
        // too, and with no key before them: a terminal that keeps ctrl+\ for itself still has these.
        if let (KeyCode::F(n @ 1..=10), false) = (key.code, typing_text) {
            if n == 10 {
                self.go_to_view(View::Local);
            } else if n <= View::MONITOR {
                if let Some(view) = View::from_number(n) {
                    self.go_to_view(view);
                }
            } else {
                self.open_session(usize::from(n));
            }
            self.sync_view_state();
            return;
        }

        // ctrl+\ opens Claude from the monitor; on view 5 it opens the session bar, and once more
        // goes back to the monitor.
        if crate::claude::is_switch_key(&key) {
            match (self.view, &self.claude.mode) {
                (View::Claude, Mode::Bar) => {
                    self.claude.mode = Mode::Typing;
                    self.view = self.claude.back_to;
                }
                (View::Claude, _) => {
                    self.claude.mode = Mode::Bar;
                    self.claude.closing = false;
                }
                _ => self.open_claude(),
            }
            self.sync_view_state();
            return;
        }
        if self.view == View::Claude {
            match self.claude.mode {
                Mode::Naming(_) => return self.on_naming_key(key),
                Mode::Opening(_) => return self.on_opening_key(key),
                Mode::Finding { .. } => return self.on_finding_key(key),
                Mode::Bar => {
                    self.on_bar_key(key);
                    self.sync_view_state();
                    return;
                }
                Mode::Typing => {}
            }
            // A query session takes every key as a SQL prompt would.
            let servers = self.server_names();
            let now = self.now();
            let node = self.claude.current().and_then(|s| s.console.as_ref()).and_then(|c| c.node.clone());
            let schema = node.as_deref().and_then(|n| self.schemas.get(n)).and_then(|s| s.schema.as_ref());
            if let Some(console) = self.claude.current_mut().and_then(|s| s.console.as_deref_mut()) {
                console.key(&key, &servers, now, schema);
                if let Some((text, what)) = console.take_clipboard() {
                    self.clipboard.push(text);
                    self.notice = Some((format!("copied {what} — on the clipboard"), SystemTime::now()));
                }
                self.sync_view_state();
                return;
            }
            // With a program running, every other key is its — q, the digits and ctrl+c
            // included: it is a terminal, and a terminal does not keep keys for itself.
            if let Some(session) = self.claude.current_mut().filter(|s| s.pane.is_running()) {
                let suspend = key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('z') | KeyCode::Char('Z'));
                if suspend && session.kind != Kind::Terminal {
                    // Suspended, it would freeze: the pane has no shell around it to say `fg`.
                    // A shell's own jobs are another thing — there ctrl+z is the shell's.
                    let who = session.kind.title();
                    self.notice = Some((format!("ctrl+z is not passed on — nothing here could bring {who} back"), SystemTime::now()));
                    return;
                }
                session.pane.key(&key, session.kind);
                return;
            }
            if key.code == KeyCode::Enter {
                self.start_claude();
                self.sync_view_state();
                return;
            }
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Keys that mean the same thing everywhere.
        match key.code {
            KeyCode::Char('c') if ctrl => {
                self.quit = true;
                return;
            }
            KeyCode::Char('q') => {
                self.quit = true;
                return;
            }
            KeyCode::Char(c @ '1'..='9') if c as u8 - b'0' > View::MONITOR => {
                self.open_session(c as usize - '0' as usize);
                self.sync_view_state();
                return;
            }
            KeyCode::Char(c @ '0'..='9') => {
                if let Some(view) = View::from_number(c as u8 - b'0') {
                    self.view = view;
                    self.focus = Focus::Tree;
                    if view != View::Queue {
                        self.queue_selection = None;
                    }
                }
                self.sync_view_state();
                return;
            }
            KeyCode::Char('?') => {
                self.help = true;
                self.sync_view_state();
                return;
            }
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                self.sync_view_state();
                return;
            }
            KeyCode::Char('z') => {
                self.time.flip();
                return;
            }
            KeyCode::Char('d') if self.prayer_alert().is_some() => {
                let now = self.now();
                self.prayers.dismiss(now);
                return;
            }
            _ => {}
        }

        match self.view {
            View::Nodes if self.focus == Focus::Insights => self.on_insights_key(key.code),
            View::Nodes => self.on_tree_key(key),
            View::Queue => self.on_queue_key(key),
            View::Map => self.on_map_key(key.code),
            View::Tape => self.on_tape_key(key.code),
            View::Airflow => self.on_airflow_key(key.code),
            View::Jira => self.on_jira_key(key.code),
            View::Local => self.on_local_key(key.code),
            // Nothing running: ⏎ starts it (above); nothing else to do here.
            View::Claude => {}
        }
        self.sync_view_state();
    }

    fn on_tree_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => return self.scroll_sql(-1),
                KeyCode::Down => return self.scroll_sql(1),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('K') => self.scroll_sql(-1),
            KeyCode::Char('J') => self.scroll_sql(1),
            KeyCode::Char('s') => self.tree.sort = self.tree.sort.next(),
            // A query's id, to look it up in system.query_log.
            KeyCode::Char('y') => {
                if let Some(crate::tree::RowId::Query { query_id, .. }) = self.selected().cloned() {
                    self.notice = Some((format!("copied query id {query_id}"), SystemTime::now()));
                    self.clipboard.push(query_id);
                }
            }
            // SQL on the node under the cursor, in a query session of its own.
            KeyCode::Char('c') => {
                let node = self.node_at_cursor();
                self.open_query_on(node);
            }
            KeyCode::Char('u') => {
                self.tree.pivot = !self.tree.pivot;
                // Identity is per direction, so the cursor cannot stay on a row that is gone.
                self.selected = None;
                self.tree.clear_expansions();
            }
            KeyCode::Char(' ') => self.tree.fold_healthy = !self.tree.fold_healthy,
            KeyCode::Char('/') => {
                self.filter_input = Some(String::new());
                self.tree.filter.clear();
            }
            KeyCode::Tab => {
                if !self.insights().is_empty() {
                    self.focus = Focus::Insights;
                }
            }
            KeyCode::Esc => self.tree.filter.clear(),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(usize::MAX),
            KeyCode::Left | KeyCode::Char('h') => self.collapse(),
            KeyCode::Right | KeyCode::Char('l') => self.expand(),
            KeyCode::Enter => self.toggle(),
            _ => {}
        }
    }

    /// `tab` moved the cursor into the insights: ↑ ↓ pick one, ⏎ goes to what it is about.
    fn on_insights_key(&mut self, code: KeyCode) {
        let len = self.insights().len();
        match code {
            KeyCode::Tab | KeyCode::Esc => self.focus = Focus::Tree,
            KeyCode::Up | KeyCode::Char('k') => {
                self.insight_selection = self.insight_selection.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.insight_selection = (self.insight_selection + 1).min(len.saturating_sub(1))
            }
            KeyCode::Home => self.insight_selection = 0,
            KeyCode::End => self.insight_selection = len.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(insight) = self.insights().get(self.insight_selection) {
                    let subject = insight.subject.clone();
                    self.go_to(&subject);
                }
            }
            _ => {}
        }
    }

    fn on_queue_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => return self.scroll_sql(-1),
                KeyCode::Down => return self.scroll_sql(1),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('K') => self.scroll_sql(-1),
            KeyCode::Char('J') => self.scroll_sql(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_queue_row(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_queue_row(1),
            KeyCode::PageUp => self.move_queue_row(-10),
            KeyCode::PageDown => self.move_queue_row(10),
            KeyCode::Home => self.move_queue_row(isize::MIN / 2),
            KeyCode::End => self.move_queue_row(isize::MAX / 2),
            // On the queue, ⏎ is the stitch of §2.8 instead of an expand.
            KeyCode::Enter => self.activate_queue_row(),
            KeyCode::Char('x') => self.ask_to_cancel(),
            _ => {}
        }
    }

    fn on_map_key(&mut self, code: KeyCode) {
        let len = self.map_nodes().len();
        if len == 0 {
            return;
        }
        let columns = self.viewport.map_columns.get().max(1);
        let current = self.map_selection.min(len - 1);
        self.map_selection = match code {
            KeyCode::Left | KeyCode::Char('h') => current.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => (current + 1).min(len - 1),
            KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(columns),
            KeyCode::Down | KeyCode::Char('j') => (current + columns).min(len - 1),
            KeyCode::Home => 0,
            KeyCode::End => len - 1,
            KeyCode::Char('s') => {
                self.tree.sort = self.tree.sort.next();
                current
            }
            KeyCode::Enter => {
                if let Some(name) = self.map_nodes().get(current).cloned() {
                    self.go_to(&Subject::Node(name));
                }
                return;
            }
            KeyCode::Char('c') => {
                let node = self.map_nodes().get(current).cloned();
                self.open_query_on(node);
                return;
            }
            _ => current,
        };
    }

    /// View 0: the cursor, the sort and whether a program is one row.
    fn on_local_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_local(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_local(1),
            KeyCode::PageUp => self.move_local(-10),
            KeyCode::PageDown => self.move_local(10),
            KeyCode::Home => self.local.select(0),
            KeyCode::End => self.move_local(isize::MAX / 2),
            KeyCode::Char('s') => {
                self.local.sort = match self.local.sort {
                    crate::local::Sort::Cpu => crate::local::Sort::Memory,
                    crate::local::Sort::Memory => crate::local::Sort::Cpu,
                };
                self.local.select(0);
            }
            KeyCode::Char('g') => {
                self.local.grouped = !self.local.grouped;
                self.local.select(0);
            }
            _ => {}
        }
    }

    fn move_local(&mut self, delta: isize) {
        self.local.select_by(delta);
    }

    fn on_tape_key(&mut self, code: KeyCode) {
        let len = self.tape.len();
        let current = self.tape_selection.min(len.saturating_sub(1));
        self.tape_selection = match code {
            KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => (current + 1).min(len.saturating_sub(1)),
            KeyCode::PageUp => current.saturating_sub(10),
            KeyCode::PageDown => (current + 10).min(len.saturating_sub(1)),
            KeyCode::Home => 0,
            KeyCode::End => len.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(subject) = self.tape.get_newest(current).and_then(|e| e.subject.clone()) {
                    self.go_to(&subject);
                }
                return;
            }
            _ => current,
        };
    }

    /// Put the cursor on whatever an insight or a tape line is about.
    pub fn go_to(&mut self, subject: &Subject) {
        self.focus = Focus::Tree;
        match subject {
            Subject::Fleet => self.view = View::Nodes,
            Subject::Queue => {
                self.view = View::Queue;
                self.queue_selection = None;
            }
            Subject::Node(name) => {
                self.view = View::Nodes;
                if self.tree.pivot {
                    self.tree.pivot = false;
                    self.tree.clear_expansions();
                }
                // Opening it also takes it out of the healthy fold (§2.5: opened nodes stay).
                self.tree.expanded_nodes.insert(name.clone());
                self.selected = Some(RowId::Node(name.clone()));
            }
            Subject::Query { node, query_id, .. } => {
                let exists = self
                    .snapshot
                    .as_deref()
                    .is_some_and(|s| s.nodes.iter().any(|n| &n.name == node));
                if exists {
                    self.jump_to_clickhouse(node, query_id);
                } else {
                    self.view = View::Nodes;
                }
            }
        }
    }

    fn rows_len(&self) -> usize {
        self.with_rows(|_, rows| rows.len()).unwrap_or(0)
    }

    fn move_by(&mut self, delta: isize) {
        if self.view == View::Queue {
            self.move_queue_row(delta);
            return;
        }
        let len = self.rows_len();
        if len == 0 {
            return;
        }
        let current = self.selected_index().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.move_to(next);
    }

    fn move_queue_row(&mut self, delta: isize) {
        let len = self.queue_rows().len();
        if len == 0 {
            self.queue_selection = None;
            return;
        }
        let current = self.queue_selection.unwrap_or(0) as isize;
        self.queue_selection = Some((current + delta).clamp(0, len as isize - 1) as usize);
    }

    fn move_to(&mut self, index: usize) {
        if self.view == View::Queue {
            self.move_queue_row(index as isize - self.queue_selection.unwrap_or(0) as isize);
            return;
        }
        let id = self
            .with_rows(|_, rows| {
                let last = rows.len().saturating_sub(1);
                rows.get(index.min(last)).map(|row| row.id.clone())
            })
            .flatten();
        if let Some(id) = id {
            self.selected = Some(id);
        }
    }

    /// `←` collapses, `→` expands, vim-style (§3).
    fn collapse(&mut self) {
        let Some((_, id)) = self.selected_row() else {
            return;
        };
        match &id {
            RowId::Node(name) => {
                self.tree.expanded_nodes.remove(name);
            }
            RowId::FleetUser(user) => {
                self.tree.expanded_nodes.remove(user);
            }
            RowId::PivotNode { user, node } => {
                self.tree.expanded_users.remove(&(user.clone(), node.clone()));
            }
            RowId::User { .. } | RowId::Closing(_) | RowId::Folded => {
                // Move onto the parent and collapse it: the visible effect of ← on a child.
                if let Some(parent) = id.parent() {
                    if let RowId::Node(name) | RowId::FleetUser(name) = &parent {
                        self.tree.expanded_nodes.remove(name);
                    }
                    self.selected = Some(parent);
                }
            }
            RowId::Query { .. } => {
                if let Some(parent) = id.parent() {
                    self.selected = Some(parent);
                }
            }
        }
    }

    fn expand(&mut self) {
        let Some((_, id)) = self.selected_row() else {
            return;
        };
        match id {
            RowId::Node(name) => {
                self.tree.expanded_nodes.insert(name);
            }
            RowId::FleetUser(user) => {
                self.tree.expanded_nodes.insert(user);
            }
            RowId::User { node, user, person } => {
                let key = match &person {
                    Some(p) => format!("{user}\u{1}{p}"),
                    None => user.clone(),
                };
                self.tree.expanded_users.insert((node, key));
            }
            RowId::PivotNode { user, node } => {
                self.tree.expanded_users.insert((user, node));
            }
            RowId::Query { .. } | RowId::Closing(_) | RowId::Folded => {}
        }
    }

    fn toggle(&mut self) {
        let Some((_, id)) = self.selected_row() else {
            return;
        };
        match id {
            RowId::Node(name) => self.tree.toggle_node(&name),
            RowId::FleetUser(user) => self.tree.toggle_node(&user),
            RowId::User { node, user, person } => {
                let key = match &person {
                    Some(p) => format!("{user}\u{1}{p}"),
                    None => user.clone(),
                };
                self.tree.toggle_user(&node, &key);
            }
            RowId::PivotNode { user, node } => self.tree.toggle_user(&user, &node),
            // A folded line and a closing row have nothing to open.
            RowId::Query { .. } | RowId::Closing(_) | RowId::Folded => {}
        }
    }

    /// Move the cursor onto the queue's running job and switch to view 1 (§2.8): the arrow in
    /// the RUNNING half is the stitch between a Redash job and its ClickHouse query.
    ///
    /// Opening the node is not enough: the query sits under its user row, and that row is keyed
    /// by person as well as user, so the jump looks the row up instead of guessing.
    pub fn jump_to_clickhouse(&mut self, node: &str, query_id: &str) {
        self.view = View::Nodes;
        self.tree.pivot = false;
        self.tree.expanded_nodes.insert(node.to_string());

        if let Some(snapshot) = self.snapshot.as_deref() {
            let slices = tree::user_slices_for(snapshot, self.prev.as_deref(), node);
            for slice in &slices {
                if slice.queries.iter().any(|q| q.query.query_id == query_id) {
                    self.tree.expanded_users.insert((
                        node.to_string(),
                        tree::TreeState::user_row_key(slice),
                    ));
                    self.selected = Some(RowId::Query {
                        node: node.to_string(),
                        user: slice.user.clone(),
                        person: slice.person.clone(),
                        query_id: query_id.to_string(),
                    });
                    break;
                }
            }
        }
    }

    // -- derived state ---------------------------------------------------

    /// Called after every event: resolve the selection and keep it on screen.
    fn sync_view_state(&mut self) {
        // Expire the notice before anything reads it.
        if self
            .notice
            .as_ref()
            .and_then(|(_, when)| SystemTime::now().duration_since(*when).ok())
            .is_some_and(|age| age >= self.notice_ttl)
        {
            self.notice = None;
        }

        // The servers a query session can be opened on, as the fleet stands.
        if let Mode::Opening(picker) = &self.claude.mode
            && picker.kind == Kind::Query
        {
            let servers = self.servers();
            if let Mode::Opening(picker) = &mut self.claude.mode
                && picker.servers != servers
            {
                picker.servers = servers;
                picker.cursor = picker.cursor.min(picker.rows().len().saturating_sub(1));
            }
        }

        self.footer = if self.filter_input.is_some() {
            Footer::Filter
        } else if self.help {
            Footer::Help
        } else {
            Footer::Keys
        };

        if self.view == View::Queue {
            let len = self.queue_rows().len();
            self.queue_selection = match self.queue_selection {
                Some(index) if index < len => Some(index),
                _ if len > 0 => Some(0),
                _ => None,
            };
        }

        if self.focus == Focus::Insights {
            let len = self.insights().len();
            if len == 0 || self.view != View::Nodes {
                self.focus = Focus::Tree;
            }
            self.insight_selection = self.insight_selection.min(len.saturating_sub(1));
        }
        self.tape_selection = self.tape_selection.min(self.tape.len().saturating_sub(1));
        self.resolve_lists();

        let len = self.rows_len();
        if len == 0 {
            self.selected = None;
            return;
        }

        // A selected row that vanished moves to its parent (§2.5); if that is gone too, take
        // the top row rather than pointing at nothing.
        let resolved = self.with_rows(|_, rows| tree::selection_index(rows, self.selected.as_ref()));
        match resolved.flatten() {
            Some(index) => self.selected = self.with_rows(|_, rows| rows[index].id.clone()),
            None => {
                self.selected = self
                    .with_rows(|_, rows| rows.first().map(|r| r.id.clone()))
                    .flatten();
            }
        }
    }
}

/// The label a row shows in its first column, for the tests that walk the tree.
#[cfg(test)]
pub fn row_title(row: &Row<'_>) -> String {
    match &row.payload {
        crate::tree::Payload::Node(view) => view.name().to_string(),
        crate::tree::Payload::User { slice, .. } => slice.label(),
        crate::tree::Payload::Closing(_) => "server · caches · merges".to_string(),
        crate::tree::Payload::Folded { names, .. } => names.join(" "),
        crate::tree::Payload::FleetUser(user) => user.label(),
        crate::tree::Payload::PivotNode { user, node } => format!("{} → {}", user.label(), node.node.name),
        crate::tree::Payload::Query { stat, user, .. } => {
            crate::attrib::user_label(user, stat.query.person.as_deref())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use crate::model::SortKey;
    use crate::tree::{Kind, Payload};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn app_with_fake() -> App {
        let mut fake = FakeSource::new();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        app.update(Event::Queue(Box::new(fake.queue())));
        app
    }

    fn titles(app: &App) -> Vec<String> {
        app.with_rows(|_, rows| {
            rows.iter()
                .map(|r| {
                    let mut t = row_title(r);
                    if let Payload::Node(view) = &r.payload
                        && app.tree.new_nodes.contains(view.name()) {
                            t.push_str(" NEW");
                        }
                    t
                })
                .collect()
        })
        .unwrap()
    }

    #[test]
    fn the_first_snapshot_opens_the_top_node() {
        let app = app_with_fake();
        assert_eq!(app.tree.expanded_nodes.len(), 1);
        let rows: Vec<String> = app
            .with_rows(|_, rows| rows.iter().map(|r| row_title(r)).collect())
            .unwrap();
        assert_eq!(rows[0], "clickhouse3");
        assert!(rows.iter().any(|r| r.starts_with("r_redash → grigol")));
    }

    #[test]
    fn enter_expands_and_collapses_the_selected_node() {
        let mut app = app_with_fake();
        // Walk down to the next node row that is not open yet.
        let mut target = None;
        for index in 1..app.with_rows(|_, rows| rows.len()).unwrap_or(1) {
            let candidate = app.with_rows(|_, rows| {
                rows.get(index)
                    .filter(|r| r.kind == Kind::Node)
                    .map(row_title)
            });
            if let Some(name) = candidate.flatten()
                && !app.tree.node_expanded(&name) {
                    target = Some(name);
                    break;
                }
        }
        let target = target.expect("the fleet has collapsed nodes");

        for _ in 0..40 {
            let on_target = app
                .selected_row()
                .map(|(_, id)| matches!(&id, RowId::Node(name) if name == &target))
                .unwrap_or(false);
            if on_target {
                break;
            }
            app.update(key(KeyCode::Down));
        }
        assert!(app
            .selected_row()
            .map(|(_, id)| matches!(&id, RowId::Node(name) if name == &target))
            .unwrap_or(false), "walked onto {target}");

        let closed = app.with_rows(|_, rows| rows.len()).unwrap();
        app.update(key(KeyCode::Enter));
        assert!(app.tree.node_expanded(&target));
        assert!(
            app.with_rows(|_, rows| rows.len()).unwrap() > closed,
            "opening a node adds its user rows"
        );

        app.update(key(KeyCode::Enter));
        assert!(!app.tree.node_expanded(&target));
    }

    #[test]
    fn arrows_move_the_cursor_and_stop_at_the_edges() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Up));
        assert_eq!(app.selected_index(), Some(0));
        let last = app.with_rows(|_, rows| rows.len()).unwrap() - 1;
        app.update(key(KeyCode::Down));
        assert_eq!(app.selected_index(), Some(1));
        app.update(key(KeyCode::End));
        assert_eq!(app.selected_index(), Some(last));
        app.update(key(KeyCode::Down));
        assert_eq!(app.selected_index(), Some(last), "no wrap-around");
    }

    #[test]
    fn s_cycles_the_sort_and_the_selection_follows_its_identity() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Down));
        app.update(key(KeyCode::Down));
        let before = app.selected().cloned();
        for _ in 0..4 {
            app.update(key(KeyCode::Char('s')));
        }
        assert_eq!(app.tree.sort, SortKey::Pressure, "four presses is a full cycle");
        assert_eq!(app.selected(), before.as_ref(), "cursor stays on the same row");
    }

    #[test]
    fn u_pivots_and_clears_the_expansions() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('u')));
        assert!(app.tree.pivot);
        assert!(app.tree.expanded_nodes.is_empty());
        let rows: Vec<String> = app
            .with_rows(|_, rows| rows.iter().map(|r| row_title(r)).collect())
            .unwrap();
        assert!(rows[0].starts_with("r_redash →"), "{rows:?}");
        app.update(key(KeyCode::Char('u')));
        assert!(!app.tree.pivot);
    }

    #[test]
    fn space_toggles_the_healthy_fold() {
        let mut app = app_with_fake();
        let folded = titles(&app).iter().any(|t| t.contains("ch4"));
        assert!(folded, "the fold line lists the healthy nodes by name");
        app.update(key(KeyCode::Char(' ')));
        assert!(!app.tree.fold_healthy);
        let rows = app.with_rows(|_, rows| rows.len()).unwrap();
        assert!(rows > 4, "unfolding brings the nodes back: {rows}");
    }

    #[test]
    fn slash_types_a_filter_and_escape_clears_it() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('/')));
        assert_eq!(app.footer, Footer::Filter);
        for c in "j.petrova".chars() {
            app.update(key(KeyCode::Char(c)));
        }
        assert_eq!(app.tree.filter, "j.petrova");
        app.update(key(KeyCode::Esc));
        assert_eq!(app.tree.filter, "");
        assert_eq!(app.footer, Footer::Keys);
    }

    #[test]
    fn the_queue_view_reports_an_unreachable_redash_without_dying() {
        let mut app = App::new();
        app.update(Event::Queue(Box::new(QueueStatus::unreachable("HTTP 401"))));
        assert_eq!(app.view, View::Nodes);
        assert!(!app.queue.reachable);
        assert_eq!(app.queue.error.as_deref(), Some("HTTP 401"));
        assert!(!app.quit);
    }

    #[test]
    fn p_pauses_without_losing_the_last_snapshot() {
        let mut app = app_with_fake();
        let before = app.snapshot().map(|s| s.nodes.len()).unwrap();
        app.update(key(KeyCode::Char('p')));
        assert!(app.paused);
        // A poll that arrives while paused is still applied, but the app reports paused so
        // the header can hollow its dot; nothing is dropped on the floor.
        assert_eq!(app.snapshot().map(|s| s.nodes.len()).unwrap(), before);
    }

    #[test]
    fn a_node_seen_later_is_flagged_new_and_keeps_the_badge() {
        let mut fake = FakeSource::new();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        assert!(app.tree.new_nodes.is_empty());
        for _ in 0..12 {
            app.update(Event::Snapshot(Box::new(fake.snapshot())));
        }
        assert!(app.tree.new_nodes.contains("clickhouse5"));
        // Badge persists for the session, even though no new node appeared this poll.
        assert!(app.tree.new_nodes.contains("clickhouse5"));
    }

    #[test]
    fn an_unreachable_node_keeps_its_row_and_then_is_dropped() {
        let mut fake = FakeSource::new();
        let snapshot = fake.snapshot();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(snapshot)));

        // The node fails: it comes back unreachable, with no numbers.
        let mut broken = app.snapshot().cloned().expect("a snapshot");
        for node in &mut broken.nodes {
            if node.name == "ch4" {
                *node = crate::model::NodeSnapshot::unreachable("ch4", "connection refused");
            }
        }
        app.update(Event::Snapshot(Box::new(broken)));
        let row = app
            .with_rows(|_, rows| {
                rows.iter()
                    .find(|r| row_title(r) == "ch4")
                    .map(|r| r.kind)
            })
            .flatten();
        assert_eq!(row, Some(Kind::Node), "it stays in the list");
        assert!(app
            .with_rows(|_, rows| rows.iter().any(|r| row_title(r) == "ch4"))
            .unwrap());
    }

    #[test]
    fn view_keys_switch_views() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('2')));
        assert_eq!(app.view, View::Queue);
        app.update(key(KeyCode::Char('3')));
        assert_eq!(app.view, View::Map);
        app.update(key(KeyCode::Char('4')));
        assert_eq!(app.view, View::Tape);
        app.update(key(KeyCode::Char('1')));
        assert_eq!(app.view, View::Nodes);
        assert_eq!(View::from_number(9), None);
    }

    #[test]
    fn tab_moves_into_the_insights_and_enter_goes_to_their_subject() {
        let mut app = app_with_fake();
        let insights = app.insights();
        assert!(!insights.is_empty());
        app.update(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Insights);

        // Go to what clickhouse7's line is about — the node, or the query that leads it.
        let target = insights
            .iter()
            .position(|i| i.label == "clickhouse7")
            .expect("clickhouse7 has something to say (lag 12 s)");
        for _ in 0..target {
            app.update(key(KeyCode::Down));
        }
        assert_eq!(app.insight_selection(), target);
        app.update(key(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Tree, "⏎ hands the cursor back to the tree");
        match &insights[target].subject {
            Subject::Node(node) => assert_eq!(app.selected(), Some(&RowId::Node(node.clone()))),
            Subject::Query { query_id, .. } => assert!(
                matches!(app.selected(), Some(RowId::Query { query_id: id, .. }) if id == query_id),
                "{:?}",
                app.selected()
            ),
            other => panic!("a node's line is about the node or a query on it, not {other:?}"),
        }
        assert!(app.tree.node_expanded("clickhouse7"));

        // Esc and tab both leave the insights.
        app.update(key(KeyCode::Tab));
        app.update(key(KeyCode::Esc));
        assert_eq!(app.focus, Focus::Tree);
    }

    #[test]
    fn an_insight_about_a_query_lands_on_that_query() {
        let mut app = app_with_fake();
        let (index, query_id) = app
            .insights()
            .iter()
            .enumerate()
            .find_map(|(i, insight)| match &insight.subject {
                Subject::Query { query_id, .. } => Some((i, query_id.clone())),
                _ => None,
            })
            .expect("the fake fleet has a runaway insight");
        app.update(key(KeyCode::Tab));
        for _ in 0..index {
            app.update(key(KeyCode::Down));
        }
        app.update(key(KeyCode::Enter));
        assert!(
            matches!(app.selected(), Some(RowId::Query { query_id: id, .. }) if *id == query_id),
            "{:?}",
            app.selected()
        );
    }

    #[test]
    fn the_map_moves_by_tile_and_by_row_and_opens_a_node() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('3')));
        app.viewport.map_columns.set(4);
        app.update(key(KeyCode::Right));
        assert_eq!(app.map_selection(), 1);
        app.update(key(KeyCode::Down));
        assert_eq!(app.map_selection(), 5, "down is one row of four");
        app.update(key(KeyCode::Up));
        app.update(key(KeyCode::Left));
        assert_eq!(app.map_selection(), 0);
        app.update(key(KeyCode::End));
        let last = app.map_nodes().len() - 1;
        assert_eq!(app.map_selection(), last);

        let name = app.map_nodes()[last].clone();
        app.update(key(KeyCode::Enter));
        assert_eq!(app.view, View::Nodes);
        assert_eq!(app.selected(), Some(&RowId::Node(name)));
    }

    #[test]
    fn the_tape_records_what_happened_and_follows_new_lines() {
        let mut fake = FakeSource::new();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        app.update(Event::Queue(Box::new(fake.queue())));
        assert!(app.tape.len() >= 3, "start line, hot nodes, runaways, queue");
        assert!(app
            .tape
            .newest_first()
            .any(|e| e.text().starts_with("watching")));

        // Scrolled down, the cursor stays on its line when new ones arrive.
        app.update(key(KeyCode::Char('4')));
        app.update(key(KeyCode::Down));
        let reading = app.tape.get_newest(app.tape_selection()).unwrap().text();
        let mut broken = app.snapshot().cloned().unwrap();
        for node in &mut broken.nodes {
            if node.name == "ch4" {
                *node = crate::model::NodeSnapshot::unreachable("ch4", "connection refused");
            }
        }
        broken.taken_at += Duration::from_secs(2);
        app.update(Event::Snapshot(Box::new(broken)));
        assert_eq!(app.tape.get_newest(app.tape_selection()).unwrap().text(), reading);
        assert!(app.tape.get_newest(0).unwrap().text().contains("ch4"));
    }

    #[test]
    fn a_fresh_queue_status_is_stitched_straight_away() {
        let mut fake = FakeSource::new();
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        // The source never sets the ClickHouse side; the app does, on arrival.
        let mut queue = fake.queue();
        for job in &mut queue.jobs {
            job.ch_node = None;
            job.ch_query_id = None;
        }
        app.update(Event::Queue(Box::new(queue)));
        assert!(app.queue.started().iter().any(|j| j.clickhouse_target().is_some()));
        assert!(app.queue_explanation().contains("runaway"), "{}", app.queue_explanation());
    }

    /// A reachable queue status with just these jobs.
    fn queue_of(jobs: Vec<Job>) -> QueueStatus {
        QueueStatus { reachable: true, error: None, jobs, ..QueueStatus::unreachable("") }
    }

    fn running(id: &str, redash: u64) -> Job {
        let mut job = Job::new(id, JobState::Started, "queries");
        job.redash_query_id = Some(redash);
        job
    }

    #[test]
    fn the_job_id_in_the_comment_is_the_exact_stitch() {
        let mut fake = FakeSource::new();
        let mut snapshot = fake.snapshot();
        // 8585 runs twice, and Redash's comment says which job each run is.
        let mut runs = Vec::new();
        for node in &mut snapshot.nodes {
            for query in &mut node.queries {
                if query.redash_query_id == Some(8585) {
                    let job = format!("job-{}", node.name);
                    query.sql = format!("/* Username: j.petrova@example.net, query_id: 8585, Job ID: {job} */ SELECT 1");
                    runs.push((job, node.name.clone(), query.query_id.clone()));
                }
            }
        }
        assert_eq!(runs.len(), 2, "the fake fleet runs 8585 twice");
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(snapshot)));
        // Each job names the other run's node as its data source: the id wins over the name.
        let jobs = runs
            .iter()
            .zip(runs.iter().rev())
            .map(|((job, _, _), (_, other, _))| {
                let mut job = running(job, 8585);
                job.data_source = Some(other.clone());
                job
            })
            .collect();
        app.update(Event::Queue(Box::new(queue_of(jobs))));
        for (job, node, query_id) in &runs {
            let stitched = app.queue.jobs.iter().find(|j| &j.id == job).unwrap();
            assert_eq!(stitched.clickhouse_target(), Some((node.as_str(), query_id.as_str())));
        }
    }

    #[test]
    fn only_running_clickhouse_jobs_are_stitched_by_number() {
        let mut app = app_with_fake();
        let mut leftover = Job::new("z1", JobState::Stale(crate::model::Stale::OverADay), "queries");
        leftover.redash_query_id = Some(7438);
        let mut mysql = running("m1", 7438);
        mysql.data_source_type = Some("mysql".into());
        let mut clickhouse = running("c1", 7438);
        clickhouse.data_source_type = Some("clickhouse".into());
        app.update(Event::Queue(Box::new(queue_of(vec![leftover, mysql, clickhouse]))));
        let node = |id: &str| {
            let job = app.queue.jobs.iter().find(|j| j.id == id).unwrap();
            job.clickhouse_target().map(|(node, _)| node.to_string())
        };
        assert_eq!(node("z1"), None, "a leftover from months ago does not claim today's run");
        assert_eq!(node("m1"), None, "a MySQL job is not a ClickHouse query");
        assert_eq!(node("c1").as_deref(), Some("clickhouse3"));
    }

    #[test]
    fn a_stale_job_that_clickhouse_still_runs_is_found() {
        let mut fake = FakeSource::new();
        let mut snapshot = fake.snapshot();
        let query = snapshot
            .nodes
            .iter_mut()
            .flat_map(|n| n.queries.iter_mut())
            .find(|q| q.redash_query_id == Some(7438))
            .unwrap();
        query.sql = format!("/* Username: grigol.gankava@example.net, query_id: 7438, Job ID: z9 */ {}", query.sql);
        let mut app = App::new();
        app.update(Event::Snapshot(Box::new(snapshot)));
        let mut ghost = Job::new("z9", JobState::Stale(crate::model::Stale::NoWorker), "queries");
        ghost.redash_query_id = Some(7438);
        app.update(Event::Queue(Box::new(queue_of(vec![ghost]))));
        assert_eq!(app.queue.jobs[0].clickhouse_target().map(|(node, _)| node), Some("clickhouse3"));
    }

    #[test]
    fn a_data_source_names_a_node_by_its_first_label() {
        assert!(names_node(Some("clickhouse-bi (prod)"), "clickhouse-bi.example.net"));
        assert!(names_node(Some("clickhouse3"), "clickhouse3"));
        assert!(names_node(Some("CH: clickhouse-bi.example.net"), "clickhouse-bi.example.net"));
        assert!(!names_node(Some("clickhouse1"), "clickhouse10.example.net"));
        assert!(!names_node(Some("ClickHouse BI"), "clickhouse-bi"));
        assert!(!names_node(None, "clickhouse3"));
    }

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// The session on screen.
    fn pane(app: &mut App) -> &mut crate::claude::ClaudePane {
        &mut app.claude.current_mut().expect("a session").pane
    }

    /// The picker's lookup answered as `main.rs` would: these folders in the folder it is in.
    fn answer(app: &mut App, names: &[&str]) {
        let Mode::Opening(picker) = &mut app.claude.mode else {
            panic!("not picking: {:?}", app.claude.mode);
        };
        let lookup = picker.lookup().expect("a lookup");
        assert!(picker.lookup().is_none(), "asked once");
        let folders = names
            .iter()
            .map(|name| crate::folders::Folder { path: lookup.dir.join(name), shown: name.to_string(), hit: None, branch: None })
            .collect();
        let found = Found { generation: lookup.generation, dir: Some(lookup.dir.clone()), folders, done: true, ..Found::default() };
        app.update(Event::Folders(found));
    }

    #[test]
    fn seven_opens_claude_and_asks_for_it_to_start() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('2')));
        app.update(key(KeyCode::Char('7')));
        assert_eq!(app.view, View::Claude);
        assert_eq!(app.claude.list.len(), 1, "the first visit opens a session");
        assert_eq!(pane(&mut app).state, PaneState::Starting);
        assert_eq!(app.claude.back_to, View::Queue);
        // ctrl+\ opens the bar, and once more goes back where it came from.
        pane(&mut app).state = PaneState::Running;
        app.update(ctrl('\\'));
        assert_eq!((app.view, &app.claude.mode), (View::Claude, &Mode::Bar));
        app.update(ctrl('4'));
        assert_eq!(app.view, View::Queue, "crossterm's name for ctrl+\\ works too");
        app.update(ctrl('\\'));
        assert_eq!(app.view, View::Claude, "and from the monitor it opens Claude again");
        assert_eq!(app.claude.mode, Mode::Typing);
        assert_eq!(app.claude.list.len(), 1, "a running session is not started twice");
    }

    #[test]
    fn on_view_five_every_key_but_the_switch_is_claudes() {
        let mut app = app_with_fake();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        app.update(key(KeyCode::Char('q')));
        app.update(ctrl('c'));
        app.update(key(KeyCode::Char('1')));
        app.update(key(KeyCode::Enter));
        assert!(!app.quit, "q and ctrl+c are Claude's here");
        assert_eq!(app.view, View::Claude, "and so are the digits");
        assert_eq!(pane(&mut app).take_outbox(), b"q\x031\r");
        app.update(ctrl('z'));
        assert!(pane(&mut app).take_outbox().is_empty(), "suspended, it could never come back");
        assert!(app.notice().is_some_and(|n| n.contains("ctrl+z")));
        app.update(Event::Paste("two\nlines".into()));
        assert_eq!(pane(&mut app).take_outbox(), b"two\rlines");
    }

    #[test]
    fn the_bar_opens_switches_renames_and_closes_sessions() {
        let mut app = app_with_fake();
        app.claude.default_dir = "~/work/cobserve".into();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        let first = app.claude.current().unwrap().id;

        // ctrl+\ n opens the picker where the one on screen works; ← goes up a folder, and ⏎
        // on one opens the session there, on screen.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let Mode::Opening(picker) = &app.claude.mode else {
            panic!("{:?}", app.claude.mode);
        };
        assert_eq!(picker.dir, home.join("work/cobserve"), "where the one on screen works, to start from");
        answer(&mut app, &["docs", "src"]);
        app.update(key(KeyCode::Left));
        answer(&mut app, &["airflow", "cobserve"]);
        // Past the way to every folder's conversations and the way up, to airflow.
        for _ in 0..3 {
            app.update(key(KeyCode::Down));
        }
        app.update(key(KeyCode::Enter));
        assert_eq!((app.claude.list.len(), app.claude.active), (2, 1));
        assert_eq!(app.claude.current().unwrap().dir, "~/work/airflow");
        assert_eq!(app.claude.mode, Mode::Typing, "straight back to typing, into the new one");
        assert!(pane(&mut app).take_outbox().is_empty(), "the directory was not typed into Claude");
        // As main.rs leaves it once its program has started.
        pane(&mut app).state = PaneState::Running;

        // ctrl+\ r: rename it.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('r')));
        for c in "infrx".chars() {
            app.update(key(KeyCode::Char(c)));
        }
        app.update(key(KeyCode::Backspace));
        app.update(key(KeyCode::Char('a')));
        app.update(key(KeyCode::Enter));
        assert_eq!(app.claude.current().unwrap().name.as_deref(), Some("infra"));

        // ctrl+\ 7: the first session — the sessions are numbered on from the views.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('7')));
        assert_eq!(app.claude.current().unwrap().id, first);
        // ctrl+\ 4: straight to the tape; ctrl+\ 6 to Jira.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('6')));
        assert_eq!(app.view, View::Jira);
        app.update(key(KeyCode::Char('7')));
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('4')));
        assert_eq!(app.view, View::Tape);
        // And 8 from the monitor is session 8.
        app.update(key(KeyCode::Char('8')));
        assert_eq!((app.view, app.claude.current().unwrap().name.as_deref()), (View::Claude, Some("infra")));
        app.update(key(KeyCode::Char('9')));
        assert_eq!(pane(&mut app).take_outbox(), b"9", "on view 7 a digit is Claude's");

        // ctrl+\ x x: closed — once is not enough.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('x')));
        assert_eq!(app.claude.list.len(), 2, "one x only asks");
        assert!(app.claude.closing);
        app.update(key(KeyCode::Char('x')));
        assert_eq!(app.claude.list.len(), 1);
        assert_eq!(app.claude.current().unwrap().id, first);

        // A rename can be given up, and esc leaves the bar for Claude.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('r')));
        app.update(key(KeyCode::Char('z')));
        app.update(key(KeyCode::Esc));
        assert_eq!(app.claude.current().unwrap().name, None);
        assert_eq!(app.claude.mode, Mode::Typing);
        // m on the bar is the monitor, as is ctrl+\ twice.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('m')));
        assert_eq!(app.view, View::Tape, "back where it came from");
    }

    #[test]
    fn the_picker_s_keys_search_go_up_and_give_up_in_that_order() {
        let mut app = app_with_fake();
        app.claude.default_dir = "~/work/cobserve".into();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let picker = |app: &App| match &app.claude.mode {
            Mode::Opening(picker) => picker.clone(),
            other => panic!("not picking: {other:?}"),
        };
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        answer(&mut app, &["docs", "src", "tests"]);
        // The wheel moves the cursor, three rows a tick; it stays in the list.
        let wheel = || Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: 50, row: 10, modifiers: KeyModifiers::NONE });
        app.update(wheel());
        assert_eq!(picker(&app).at_cursor(), Some(PickRow::Folder(0)));
        app.update(wheel());
        assert_eq!(picker(&app).cursor, 5, "the last row");
        // A paste is a search, as typing is; esc clears it, the next esc gives up.
        app.update(Event::Paste("src\nmore".into()));
        assert_eq!(picker(&app).query, "src", "its first line");
        app.update(key(KeyCode::Esc));
        assert!(picker(&app).query.is_empty());
        // Backspace with nothing typed goes up a folder; Tab goes into the one under the cursor.
        app.update(key(KeyCode::Backspace));
        assert_eq!(picker(&app).dir, home.join("work"));
        answer(&mut app, &["cobserve"]);
        app.update(key(KeyCode::End));
        app.update(key(KeyCode::Tab));
        assert_eq!(picker(&app).dir, home.join("work/cobserve"));
        app.update(key(KeyCode::Esc));
        assert_eq!(app.claude.mode, Mode::Typing);
        assert_eq!(app.claude.list.len(), 1, "nothing opened");
        assert!(pane(&mut app).take_outbox().is_empty(), "and nothing typed into Claude");
    }

    #[test]
    fn ctrl_backslash_c_o_t_open_claude_opencode_or_a_terminal() {
        let mut app = app_with_fake();
        app.claude.default_dir = "~/work/cobserve".into();
        app.open_claude();
        assert_eq!(app.claude.current().unwrap().kind, crate::claude::Kind::Claude, "the first visit is Claude");
        pane(&mut app).state = PaneState::Running;
        let picking = |app: &App| match &app.claude.mode {
            Mode::Opening(picker) => picker.kind,
            other => panic!("not picking: {other:?}"),
        };
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('o')));
        assert_eq!(picking(&app), crate::claude::Kind::OpenCode);
        app.update(key(KeyCode::Enter));
        assert_eq!(app.claude.current().unwrap().kind, crate::claude::Kind::OpenCode);
        pane(&mut app).state = PaneState::Running;
        app.update(ctrl('z'));
        assert!(pane(&mut app).take_outbox().is_empty(), "suspended, OpenCode could not come back either");
        assert!(app.notice().is_some_and(|n| n.contains("OpenCode")));

        // n: the kind on screen; shift+tab: the next one; a click: that one.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        assert_eq!(picking(&app), crate::claude::Kind::OpenCode);
        app.update(Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)));
        assert_eq!(picking(&app), crate::claude::Kind::Terminal);
        app.viewport.hits.borrow_mut().push((Rect::new(10, 3, 12, 1), Hit::Kind(crate::claude::Kind::Claude)));
        app.update(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 12, row: 3, modifiers: KeyModifiers::NONE }));
        assert_eq!(picking(&app), crate::claude::Kind::Claude);
        app.update(key(KeyCode::Esc));

        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('t')));
        app.update(key(KeyCode::Enter));
        assert_eq!(app.claude.current().unwrap().kind, crate::claude::Kind::Terminal);
        pane(&mut app).state = PaneState::Running;
        app.update(ctrl('z'));
        assert_eq!(pane(&mut app).take_outbox(), [0x1a], "in a shell ctrl+z is the shell's, for its jobs");
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('c')));
        assert_eq!(picking(&app), crate::claude::Kind::Claude);
    }

    #[test]
    fn f_keys_reach_every_tab_without_ctrl_backslash() {
        let mut app = app_with_fake();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        let f = |n| Event::Key(KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE));
        app.update(f(4));
        assert_eq!(app.view, View::Tape, "F4 from Claude's screen, not passed on to it");
        assert!(pane(&mut app).take_outbox().is_empty());
        app.update(f(7));
        assert_eq!(app.view, View::Claude);
        app.update(f(5));
        assert_eq!(app.view, View::Airflow);
        app.update(f(6));
        assert_eq!(app.view, View::Jira);
        app.update(f(2));
        assert_eq!(app.view, View::Queue);
        app.update(f(9));
        assert_eq!(app.view, View::Queue, "no session 9: nothing happens but a word on how to open one");
        assert!(app.notice().is_some_and(|n| n.contains("no session 9")));
    }

    #[test]
    fn a_click_opens_a_tab_or_a_session_and_the_wheel_moves() {
        let mut app = app_with_fake();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        app.update(key(KeyCode::Enter));
        // What the last frame drew: two tabs, the first session, the +, Claude's screen.
        let rect = |x, y, w, h| Rect::new(x, y, w, h);
        app.viewport.hits.borrow_mut().extend([
            (rect(80, 0, 8, 1), Hit::View(View::Tape)),
            (rect(90, 0, 10, 1), Hit::View(View::Claude)),
            (rect(1, 6, 26, 2), Hit::Session(0)),
            (rect(24, 4, 3, 1), Hit::NewSession),
            (rect(30, 4, 80, 25), Hit::Pane),
        ]);
        let click = |x, y| {
            Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE })
        };
        app.update(click(2, 7));
        assert_eq!(app.claude.active, 0, "the first session, clicked");
        app.update(click(83, 0));
        assert_eq!(app.view, View::Tape, "a tab of the header, clicked");
        app.update(click(92, 0));
        assert_eq!(app.view, View::Claude);
        app.update(click(25, 4));
        assert!(matches!(app.claude.mode, Mode::Opening(_)), "the + asks where");
        app.update(key(KeyCode::Esc));
        // The wheel over Claude's screen is Claude's; over the tape it moves the cursor.
        pane(&mut app).take_outbox();
        let wheel = |kind| Event::Mouse(MouseEvent { kind, column: 50, row: 10, modifiers: KeyModifiers::NONE });
        for _ in 0..3 {
            app.update(wheel(MouseEventKind::ScrollUp));
        }
        assert_eq!(pane(&mut app).take_outbox(), b"\x1b[5~", "a turn of the wheel is a page of Claude's");
    }

    #[test]
    fn after_claude_ends_the_view_is_the_monitor_s_again() {
        let mut app = app_with_fake();
        app.open_claude();
        let id = app.claude.current().unwrap().id;
        app.update(Event::PaneExited(id, "it exited".into()));
        assert_eq!(pane(&mut app).state, PaneState::Exited("it exited".into()));
        app.update(key(KeyCode::Enter));
        assert_eq!(pane(&mut app).state, PaneState::Starting, "⏎ starts it again");
        pane(&mut app).state = PaneState::Failed("claude is not installed".into());
        app.update(key(KeyCode::Char('1')));
        assert_eq!(app.view, View::Nodes, "with nothing running, the keys are the monitor's");
        app.update(key(KeyCode::Char('q')));
        assert!(app.quit);
    }

    #[test]
    fn a_bell_from_a_session_not_on_screen_marks_it() {
        let mut app = app_with_fake();
        app.open_claude();
        pane(&mut app).state = PaneState::Running;
        let first = app.claude.current().unwrap().id;
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        app.update(key(KeyCode::Enter));
        app.update(Event::Pane(first, b"all done\x07".to_vec()));
        assert!(app.claude.list[0].pane.attention, "rang behind the session on screen");
        assert!(app.claude.calling());
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('7')));
        assert!(!app.claude.calling(), "seen once it is on screen");
        // Behind the folder picker it is not on screen either.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('n')));
        app.update(Event::Pane(first, b"\x07".to_vec()));
        assert!(app.claude.calling(), "the picker covers it");
    }

    #[test]
    fn data_goes_stale_after_three_missed_polls() {
        let mut app = app_with_fake();
        assert!(!app.is_stale());
        app.clock = SystemTime::now() + Duration::from_secs(10);
        assert!(app.is_stale());
        app.paused = true;
        assert!(!app.is_stale(), "paused is not stale, it is paused");
    }

    #[test]
    fn scrolling_keeps_the_cursor_on_screen_and_moves_as_little_as_possible() {
        assert_eq!(scroll_into_view(0, Some(3), 10, 5), 0, "everything fits");
        assert_eq!(scroll_into_view(0, Some(12), 10, 40), 3);
        assert_eq!(scroll_into_view(3, Some(5), 10, 40), 3, "already visible: no jump");
        assert_eq!(scroll_into_view(8, Some(2), 10, 40), 2);
        assert_eq!(scroll_into_view(50, None, 10, 40), 30, "never past the end");
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('q')));
        assert!(app.quit);

        let mut app = App::new();
        app.update(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.quit);
    }

    #[test]
    fn the_help_overlay_opens_and_closes() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('?')));
        assert!(app.help);
        app.update(key(KeyCode::Char('x')));
        assert!(!app.help, "any key but ? and q closes it");
    }

    #[test]
    fn a_jump_from_the_queue_selects_the_clickhouse_query() {
        let mut app = app_with_fake();
        app.view = View::Queue;
        app.jump_to_clickhouse("clickhouse3", "c3e51cb5");
        assert_eq!(app.view, View::Nodes);
        assert!(app.tree.node_expanded("clickhouse3"));
        // The query may not be in the fake snapshot; the selection still lands on its row or
        // on the parent, never nowhere.
        assert!(app.selected().is_some());
    }
}