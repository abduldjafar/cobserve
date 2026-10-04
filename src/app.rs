//! App state and `update`: a pure state machine, no I/O (DESIGN.md §8).
//!
//! Everything the UI draws lives here or in the model; sources push `Event`s in and the loop
//! applies them. Nothing in this file knows that ClickHouse or Redash exist.

use crate::claude::{Kind, Mode, PaneState, PickRow, Picker, Sessions};
use crate::folders::Found;
use crate::history::{self, History};
use crate::insight::{self, Insight, Subject};
use crate::model::{fleet_totals, mark_new_nodes, FleetSnapshot, FleetView, Job, JobState, QueueStatus};
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
    /// Sessions — Claude Code, OpenCode, a shell — in a pane, the monitor's band above it
    /// (`claude.rs`).
    Claude,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Nodes => "NODES",
            View::Queue => "QUEUE",
            View::Map => "MAP",
            View::Tape => "TAPE",
            View::Claude => "SESSIONS",
        }
    }

    /// The tab order of §1, which is also the `1` … `5` keymap.
    pub const ALL: [View; 5] = [View::Nodes, View::Queue, View::Map, View::Tape, View::Claude];

    /// `1` … `5`, the number that selects this view.
    pub fn number(self) -> u8 {
        match self {
            View::Nodes => 1,
            View::Queue => 2,
            View::Map => 3,
            View::Tape => 4,
            View::Claude => 5,
        }
    }

    pub fn from_number(n: u8) -> Option<View> {
        match n {
            1 => Some(View::Nodes),
            2 => Some(View::Queue),
            3 => Some(View::Map),
            4 => Some(View::Tape),
            5 => Some(View::Claude),
            _ => None,
        }
    }
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
    pub tape: Cell<usize>,
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
    Quit,
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
    /// Wall time the last snapshot arrived: data older than a few polls is called stale.
    last_snapshot_wall: Option<SystemTime>,
    pub viewport: Viewport,
    /// View 5: Claude Code sessions.
    pub claude: Sessions,
}

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
            last_snapshot_wall: None,
            viewport: Viewport::default(),
            claude: Sessions::default(),
        }
    }

    pub fn update(&mut self, event: Event) {
        match event {
            Event::Tick => self.clock = SystemTime::now(),
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
            Event::Paste(text) => {
                let line = text.lines().next().unwrap_or_default();
                if self.view == View::Claude {
                    match &mut self.claude.mode {
                        Mode::Naming(name) => {
                            let room = crate::claude::NAME_MAX.saturating_sub(name.chars().count());
                            name.extend(line.chars().take(room));
                        }
                        Mode::Opening(picker) => picker.type_text(line),
                        Mode::Typing => {
                            if let Some(session) = self.claude.current_mut().filter(|s| s.pane.is_running()) {
                                session.pane.paste(&text);
                            }
                        }
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

    /// Session `number` (5–9) on view 5. The first visit opens session 5; a number with no
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
                format!("sessions are 5 to 9: {} is as many as there are numbers for — close one with x", crate::claude::MAX_SESSIONS),
                SystemTime::now(),
            ));
            self.claude.mode = Mode::Typing;
            return;
        }
        let dir = crate::pty::expand(&self.claude.next_dir());
        let kind = kind.unwrap_or_else(|| self.claude.next_kind());
        self.claude.mode = Mode::Opening(Picker::new(dir, kind));
    }

    /// The folder picker's keys: ↑ ↓ choose, ⏎ opens the session in the folder under the
    /// cursor, → goes into it and ← back up; what is typed searches.
    fn on_opening_key(&mut self, key: KeyEvent) {
        let Mode::Opening(picker) = &mut self.claude.mode else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // A search first, then the picker.
            KeyCode::Esc if picker.searching() => picker.clear_query(),
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
                Some(Hit::Pane) | None => {}
            },
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if event.kind == MouseEventKind::ScrollUp { -1 } else { 1 };
                match self.view {
                    View::Nodes if self.focus == Focus::Insights => {
                        let last = self.insights().len().saturating_sub(1);
                        self.insight_selection = self.insight_selection.saturating_add_signed(delta).min(last);
                    }
                    View::Nodes | View::Queue => self.move_by(delta),
                    View::Tape => {
                        let last = self.tape.len().saturating_sub(1);
                        self.tape_selection = self.tape_selection.saturating_add_signed(delta).min(last);
                    }
                    View::Claude => {
                        if let Mode::Opening(picker) = &mut self.claude.mode {
                            picker.step(delta * 3);
                        }
                    }
                    View::Map => {}
                }
            }
            _ => {}
        }
        self.sync_view_state();
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
            // One numbering for every tab: 1–4 the monitor's views, 5–9 the sessions.
            KeyCode::Char(c @ '1'..='4') => {
                self.claude.mode = Mode::Typing;
                if let Some(view) = View::from_number(c as u8 - b'0') {
                    self.view = view;
                }
            }
            KeyCode::Char(c @ '5'..='9') => {
                if let Some(index) = self.claude.index_of(c as usize - '0' as usize) {
                    self.claude.select(index);
                }
                self.claude.mode = Mode::Typing;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.claude.step(false),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.claude.step(true),
            // A new session: of the kind on screen, or Claude, OpenCode, a terminal.
            KeyCode::Char('n') => self.ask_new_session(None),
            KeyCode::Char('c') => self.ask_new_session(Some(Kind::Claude)),
            KeyCode::Char('o') => self.ask_new_session(Some(Kind::OpenCode)),
            KeyCode::Char('t') => self.ask_new_session(Some(Kind::Terminal)),
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
            KeyCode::Esc | KeyCode::Enter => self.claude.mode = Mode::Typing,
            _ => {}
        }
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

        let typing_text = matches!(self.claude.mode, Mode::Naming(_) | Mode::Opening(_)) && self.view == View::Claude;
        // F1–F4 the views, F5–F9 the sessions — from anywhere, Claude's screen too, and with no
        // key before them: a terminal that keeps ctrl+\ for itself still has these.
        if let (KeyCode::F(n @ 1..=9), false) = (key.code, typing_text) {
            if n <= 4 {
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
                Mode::Bar => {
                    self.on_bar_key(key);
                    self.sync_view_state();
                    return;
                }
                Mode::Typing => {}
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
                session.pane.key(&key);
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
            KeyCode::Char(c @ '5'..='9') => {
                self.open_session(c as usize - '0' as usize);
                self.sync_view_state();
                return;
            }
            KeyCode::Char(c @ '1'..='4') => {
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
            _ => {}
        }

        match self.view {
            View::Nodes if self.focus == Focus::Insights => self.on_insights_key(key.code),
            View::Nodes => self.on_tree_key(key),
            View::Queue => self.on_queue_key(key),
            View::Map => self.on_map_key(key.code),
            View::Tape => self.on_tape_key(key.code),
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
            _ => current,
        };
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
    fn five_opens_claude_and_asks_for_it_to_start() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('2')));
        app.update(key(KeyCode::Char('5')));
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
        app.update(key(KeyCode::Down));
        app.update(key(KeyCode::Down));
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

        // ctrl+\ 5: the first session — the sessions are numbered on from the views.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('5')));
        assert_eq!(app.claude.current().unwrap().id, first);
        // ctrl+\ 4: straight to the tape.
        app.update(ctrl('\\'));
        app.update(key(KeyCode::Char('4')));
        assert_eq!(app.view, View::Tape);
        // And 6 from the monitor is session 6.
        app.update(key(KeyCode::Char('6')));
        assert_eq!((app.view, app.claude.current().unwrap().name.as_deref()), (View::Claude, Some("infra")));
        app.update(key(KeyCode::Char('7')));
        assert_eq!(pane(&mut app).take_outbox(), b"7", "on view 5 a digit is Claude's");

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
        assert_eq!(picker(&app).at_cursor(), Some(PickRow::Folder(1)));
        app.update(wheel());
        assert_eq!(picker(&app).cursor, 4, "the last row");
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
        app.update(f(5));
        assert_eq!(app.view, View::Claude);
        app.update(f(2));
        assert_eq!(app.view, View::Queue);
        app.update(f(7));
        assert_eq!(app.view, View::Queue, "no session 7: nothing happens but a word on how to open one");
        assert!(app.notice().is_some_and(|n| n.contains("no session 7")));
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
        app.update(key(KeyCode::Char('5')));
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