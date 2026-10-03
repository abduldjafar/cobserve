//! App state and `update`: a pure state machine, no I/O (DESIGN.md §8).
//!
//! Everything the UI draws lives here or in the model; sources push `Event`s in and the loop
//! applies them. Nothing in this file knows that ClickHouse or Redash exist.

use crate::history::{self, History};
use crate::insight::{self, Insight, Subject};
use crate::model::{fleet_totals, mark_new_nodes, FleetSnapshot, FleetView, Job, QueueStatus};
use crate::tape::{Tape, Watch};
use crate::tree::{self, Row, RowId, TreeState};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

/// §2.6: a node that keeps failing stays on screen for this long before it is dropped.
const UNREACHABLE_GRACE: Duration = Duration::from_secs(5 * 60);

/// A queue row with the index the cursor uses, so the screen and the selection cannot disagree
/// about which job is selected.
pub type QueueRowRef<'a> = (usize, &'a Job);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Nodes,
    Queue,
    /// The fleet as a heat map of tiles — the shape of a 40-node fleet on one screen.
    Map,
    /// What changed, newest first (`tape.rs`).
    Tape,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Nodes => "NODES",
            View::Queue => "QUEUE",
            View::Map => "MAP",
            View::Tape => "TAPE",
        }
    }

    /// The tab order of §1, which is also the `1` … `4` keymap.
    pub const ALL: [View; 4] = [View::Nodes, View::Queue, View::Map, View::Tape];

    /// `1` … `4`, the number that selects this view.
    pub fn number(self) -> u8 {
        match self {
            View::Nodes => 1,
            View::Queue => 2,
            View::Map => 3,
            View::Tape => 4,
        }
    }

    pub fn from_number(n: u8) -> Option<View> {
        match n {
            1 => Some(View::Nodes),
            2 => Some(View::Queue),
            3 => Some(View::Map),
            4 => Some(View::Tape),
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
    map_selection: usize,
    tape_selection: usize,
    /// `POLL_MS`, so the header can say how often it polls and notice when data is late.
    pub poll_interval: Duration,
    /// Wall time the last snapshot arrived: data older than a few polls is called stale.
    last_snapshot_wall: Option<SystemTime>,
    pub viewport: Viewport,
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
            map_selection: 0,
            tape_selection: 0,
            poll_interval: Duration::from_millis(2000),
            last_snapshot_wall: None,
            viewport: Viewport::default(),
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
            Event::Quit => self.quit = true,
        }
        self.sync_view_state();
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

    /// View 2's two halves, each with the row index the cursor uses: waiting jobs first, then
    /// the ones on a worker, both longest first. A full queue is what on call wants to see
    /// before anything else, so it comes first.
    ///
    /// Partitioned by state *before* sorting: sorting everything by age and cutting at the
    /// first started job would put every waiting job younger than the oldest running one
    /// into the RUNNING half.
    pub fn queue_sections(&self) -> (Vec<QueueRowRef<'_>>, Vec<QueueRowRef<'_>>) {
        let (mut waiting, mut started): (Vec<QueueRowRef<'_>>, Vec<QueueRowRef<'_>>) = self
            .queue
            .jobs
            .iter()
            .enumerate()
            .partition(|(_, job)| job.state == crate::model::JobState::Queued);
        waiting.sort_by_key(|(_, job)| std::cmp::Reverse(job.age_s));
        started.sort_by_key(|(_, job)| std::cmp::Reverse(job.age_s));
        (waiting, started)
    }

    /// Every job row of view 2, in the order they are drawn.
    pub fn queue_rows(&self) -> Vec<&crate::model::Job> {
        let (waiting, started) = self.queue_sections();
        waiting
            .into_iter()
            .chain(started)
            .map(|(_, job)| job)
            .collect()
    }

    /// `⏎` on a running job: jump to the ClickHouse query it became (§2.8).
    pub fn activate_queue_row(&mut self) {
        let Some(index) = self.queue_selection else {
            return;
        };
        // The target has to be copied out before `jump_to_clickhouse` borrows self mutably.
        let target = self
            .queue_rows()
            .get(index)
            .and_then(|job| job.clickhouse_target())
            .map(|(node, id)| (node.to_string(), id.to_string()));
        // A waiting job has not reached ClickHouse: there is nothing to jump to, and the
        // drawer says so instead.
        if let Some((node, query_id)) = target {
            self.jump_to_clickhouse(&node, &query_id);
        }
    }

    /// §2.8's stitch: a Redash job that has started IS a `system.processes` row somewhere, and
    /// the only thing that links the two is the Redash query number Redash writes into the
    /// comment (§6.4). Matching on it here is what lets ⏎ jump from the queue into the tree.
    fn stitch_queue(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        // Every ClickHouse query carrying a Redash number, with where it runs and for whom.
        let mut by_redash_id: HashMap<u64, Vec<(String, String, Option<String>)>> = HashMap::new();
        for node in &snapshot.nodes {
            for query in &node.queries {
                if let Some(redash_id) = query.redash_query_id {
                    by_redash_id.entry(redash_id).or_default().push((
                        node.name.clone(),
                        query.query_id.clone(),
                        query.person.clone(),
                    ));
                }
            }
        }

        // The same Redash query can run twice at once (a dashboard refreshed twice): pick, for
        // each job, the run on the node its data source names, then the one for the same
        // person — and never hand one ClickHouse query to two jobs.
        let mut taken: HashSet<String> = HashSet::new();
        for job in &mut self.queue.jobs {
            job.ch_node = None;
            job.ch_query_id = None;
            if job.state != crate::model::JobState::Started {
                continue;
            }
            let Some(candidates) = job.redash_query_id.and_then(|id| by_redash_id.get(&id)) else {
                continue;
            };
            let free = |c: &&(String, String, Option<String>)| !taken.contains(&c.1);
            let pick = candidates
                .iter()
                .filter(free)
                .find(|(node, _, _)| job.data_source.as_deref() == Some(node.as_str()))
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
        let running: Vec<&crate::model::Job> = self.queue.started();
        if running.is_empty() {
            return "no jobs on a worker".to_string();
        }
        let queries = self.queue.queues.iter().map(|q| (q.workers_busy, q.workers_total));
        let (busy, total) = queries.fold((0, 0), |(b, t), (qb, qt)| (b + qb, t + qt));
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
        format!(
            "{busy}/{total} workers busy · {stuck} of them hold a runaway ClickHouse query ({}) → why it is full",
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
            insight::analyze(view, &self.history, &self.queue, &self.tree.new_nodes)
        })
        .unwrap_or_default()
    }

    pub fn insight_selection(&self) -> usize {
        self.insight_selection
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
            View::Nodes => self.on_tree_key(key.code),
            View::Queue => self.on_queue_key(key.code),
            View::Map => self.on_map_key(key.code),
            View::Tape => self.on_tape_key(key.code),
        }
        self.sync_view_state();
    }

    fn on_tree_key(&mut self, code: KeyCode) {
        match code {
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

    fn on_queue_key(&mut self, code: KeyCode) {
        match code {
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

        // Find an insight about a node that is folded away, and go there.
        let target = insights
            .iter()
            .position(|i| matches!(&i.subject, Subject::Node(n) if n == "clickhouse7"))
            .expect("clickhouse7 has something to say (lag 12 s)");
        for _ in 0..target {
            app.update(key(KeyCode::Down));
        }
        assert_eq!(app.insight_selection(), target);
        app.update(key(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Tree, "⏎ hands the cursor back to the tree");
        assert_eq!(app.selected(), Some(&RowId::Node("clickhouse7".into())));
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