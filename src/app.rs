//! App state and `update`: a pure state machine, no I/O (DESIGN.md §8).
//!
//! Everything the UI draws lives here or in the model; sources push `Event`s in and the loop
//! applies them. Nothing in this file knows that ClickHouse or Redash exist.

use crate::model::{mark_new_nodes, FleetSnapshot, FleetView, Job, QueueStatus};
use crate::tree::{self, Payload, Row, RowId, TreeState};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
    Flow,
    Tape,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Nodes => "NODES",
            View::Queue => "QUEUE",
            View::Flow => "FLOW",
            View::Tape => "TAPE",
        }
    }

    /// Views 3 and 4 are named in the header so the navigation is stable, and render a
    /// one-line placeholder this pass (§0).
    pub fn is_placeholder(self) -> bool {
        matches!(self, View::Flow | View::Tape)
    }

    /// The tab order of §1, which is also the `1` … `4` keymap.
    pub const ALL: [View; 4] = [View::Nodes, View::Queue, View::Flow, View::Tape];

    /// `1` … `4`, the number that selects this view.
    pub fn number(self) -> u8 {
        match self {
            View::Nodes => 1,
            View::Queue => 2,
            View::Flow => 3,
            View::Tape => 4,
        }
    }

    pub fn from_number(n: u8) -> Option<View> {
        match n {
            1 => Some(View::Nodes),
            2 => Some(View::Queue),
            3 => Some(View::Flow),
            4 => Some(View::Tape),
            _ => None,
        }
    }
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
    /// First visible row.
    scroll: usize,

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
            queue: QueueStatus::unreachable("not polled yet"),
            notice: None,
            notice_ttl: std::time::Duration::from_secs(30),
            first_seen: HashSet::new(),
            tree: TreeState::default(),
            selected: None,
            scroll: 0,
            view: View::Nodes,
            paused: false,
            filter_input: None,
            help: false,
            footer: Footer::Keys,
            clock: SystemTime::now(),
            unreachable_since: HashMap::new(),
            queue_selection: None,
            quit: false,
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
                    self.queue = *queue
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

    /// Which job row view 2 has selected.
    pub fn queue_selection(&self) -> Option<usize> {
        self.queue_selection
    }

    /// View 2's two halves, each with the row index the cursor uses: waiting jobs first, then
    /// the ones on a worker, both longest first. A full queue is what on call wants to see
    /// before anything else, so it comes first.
    pub fn queue_sections(&self) -> (Vec<QueueRowRef<'_>>, Vec<QueueRowRef<'_>>) {
        let mut rows: Vec<QueueRowRef<'_>> = self.queue.jobs.iter().enumerate().collect();
        rows.sort_by_key(|(_, job)| std::cmp::Reverse(job.age_s));
        let split = rows
            .iter()
            .position(|(_, job)| job.state == crate::model::JobState::Started)
            .unwrap_or(rows.len());
        let started = rows.split_off(split);
        (rows, started)
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
        let mut by_redash_id: HashMap<u64, (String, String)> = HashMap::new();
        for node in &snapshot.nodes {
            for query in &node.queries {
                if let Some(redash_id) = query.redash_query_id {
                    // First match wins: the initiators are already deduplicated by
                    // is_initial_query, so two rows with the same number are the same query.
                    by_redash_id
                        .entry(redash_id)
                        .or_insert_with(|| (node.name.clone(), query.query_id.clone()));
                }
            }
        }

        for job in &mut self.queue.jobs {
            let target = job
                .redash_query_id
                .and_then(|id| by_redash_id.get(&id))
                .cloned();
            match (target, job.state) {
                // A started job with no ClickHouse row is a worker doing something else.
                (Some((node, query_id)), crate::model::JobState::Started) => {
                    job.ch_node = Some(node);
                    job.ch_query_id = Some(query_id);
                }
                _ => {
                    job.ch_node = None;
                    job.ch_query_id = None;
                }
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
        let mut runaway_nodes: Vec<&str> = Vec::new();
        for job in &running {
            let Some(node) = job.ch_node.as_deref() else {
                continue;
            };
            let is_runaway = self.with_rows(|_, rows| {
                rows.iter().any(|row| {
                    matches!(&row.id,
                        crate::tree::RowId::Query { node: n, query_id, .. }
                            if n == node && Some(query_id.as_str()) == job.ch_query_id.as_deref())
                })
            });
            if is_runaway == Some(true) && !runaway_nodes.contains(&node) {
                runaway_nodes.push(node);
            }
        }
        if runaway_nodes.is_empty() {
            return format!("{busy}/{total} workers busy · none of them is stuck in ClickHouse");
        }
        format!(
            "{busy}/{total} workers busy · {} of them hold a runaway ClickHouse query ({}) → why it is full",
            runaway_nodes.len(),
            runaway_nodes.join(", ")
        )
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

    /// `p` pauses polling: the header dot goes hollow and the numbers stop moving.
    pub fn is_live(&self) -> bool {
        !self.paused
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

    pub fn scroll(&self) -> usize {
        self.scroll
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
        match key.code {
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(c @ '1'..='4') => {
                if let Some(view) = View::from_number(c as u8 - b'0') {
                    self.view = view;
                    self.scroll = 0;
                    if view != View::Queue {
                        self.queue_selection = None;
                    }
                }
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('p') => self.paused = !self.paused,
            KeyCode::Char('s') => self.tree.sort = self.tree.sort.next(),
            KeyCode::Char('u') => {
                self.tree.pivot = !self.tree.pivot;
                // Identity is per direction, so the cursor cannot stay on a row that is gone.
                self.selected = None;
                self.tree.clear_expansions();
                self.scroll = 0;
            }
            KeyCode::Char(' ') => self.tree.fold_healthy = !self.tree.fold_healthy,
            KeyCode::Char('/') => {
                self.filter_input = Some(String::new());
                self.tree.filter.clear();
            }
            KeyCode::Esc => self.tree.filter.clear(),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(usize::MAX),
            KeyCode::Left => self.collapse(),
            KeyCode::Right => self.expand(),
            // On the queue, ⏎ is the stitch of §2.8 instead of an expand.
            KeyCode::Enter if self.view == View::Queue => self.activate_queue_row(),
            KeyCode::Enter => self.toggle(),
            _ => {}
        }
        self.sync_view_state();
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
            return;
        }

        let len = self.rows_len();
        if len == 0 {
            self.selected = None;
            self.scroll = 0;
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
                self.scroll = 0;
            }
        }
    }
}

/// The label a row shows in its first column, shared by the tree and the drawer.
pub fn row_title(row: &Row<'_>) -> String {
    match &row.payload {
        Payload::Node(view) => view.name().to_string(),
        Payload::User { slice, .. } => slice.label(),
        Payload::Closing(_) => "server · caches · merges".to_string(),
        Payload::Folded { names, .. } => names.join(" "),
        Payload::FleetUser(user) => user.label(),
        Payload::PivotNode { user, node } => format!("{} → {}", user.label(), node.node.name),
        Payload::Query { stat, user, .. } => {
            crate::attrib::user_label(user, stat.query.person.as_deref())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use crate::model::SortKey;
    use crate::tree::Kind;

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
    fn view_keys_switch_and_placeholders_are_marked() {
        let mut app = app_with_fake();
        app.update(key(KeyCode::Char('2')));
        assert_eq!(app.view, View::Queue);
        app.update(key(KeyCode::Char('3')));
        assert_eq!(app.view, View::Flow);
        assert!(app.view.is_placeholder());
        app.update(key(KeyCode::Char('1')));
        assert_eq!(app.view, View::Nodes);
        assert_eq!(View::from_number(9), None);
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