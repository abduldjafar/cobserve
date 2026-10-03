//! Flattening a snapshot into the rows the tree draws (§2), in both directions.
//!
//! Everything here is pure: it takes a snapshot, the previous snapshot for the CPU deltas,
//! and the view state, and returns rows plus the identity of the selected one. Selection is
//! keyed by row identity, never by index (§2.5), so a re-sort under the cursor keeps the
//! cursor on the same thing.

use crate::model::{
    fold_healthy, user_slices, FleetSnapshot, FleetUser, FleetView, NodeView, QueryStat, SortKey,
    UserNode, UserSlice,
};
use std::collections::HashSet;

/// Who a row is. `u` flips the tree over, and `:` is part of a node name, so the variants
/// stay separate rather than being packed into one string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowId {
    Node(String),
    User {
        node: String,
        user: String,
        person: Option<String>,
    },
    Query {
        node: String,
        user: String,
        person: Option<String>,
        query_id: String,
    },
    Closing(String),
    Folded,
    /// Pivot level 0: one user across the fleet.
    FleetUser(String),
    /// Pivot level 1: that user on one node.
    PivotNode {
        user: String,
        node: String,
    },
}

impl RowId {
    /// The row a vanished row falls back to (§2.5).
    pub fn parent(&self) -> Option<RowId> {
        match self {
            RowId::Query { node, user, person, .. } => Some(RowId::User {
                node: node.clone(),
                user: user.clone(),
                person: person.clone(),
            }),
            RowId::PivotNode { user, .. } => Some(RowId::FleetUser(user.clone())),
            RowId::User { node, .. } | RowId::Closing(node) => Some(RowId::Node(node.clone())),
            RowId::Node(_) | RowId::Folded | RowId::FleetUser(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Node,
    User,
    Query,
    Closing,
    Folded,
    FleetUser,
}

#[derive(Debug, Clone)]
pub struct Row<'a> {
    pub id: RowId,
    /// 0 = top level, 1 = under it, 2 = queries.
    pub depth: u8,
    pub kind: Kind,
    pub payload: Payload<'a>,
}

#[derive(Debug, Clone)]
pub enum Payload<'a> {
    Node(&'a NodeView<'a>),
    User {
        view: &'a NodeView<'a>,
        slice: &'a UserSlice<'a>,
    },
    Closing(&'a NodeView<'a>),
    Folded {
        names: Vec<String>,
        /// Worst pressure among the folded nodes, for the "(< 35%)" part of the line.
        worst: f64,
    },
    FleetUser(&'a FleetUser<'a>),
    PivotNode {
        user: &'a FleetUser<'a>,
        node: &'a UserNode<'a>,
    },
    Query {
        stat: &'a QueryStat<'a>,
        node: &'a str,
        user: &'a str,
    },
}

/// Everything the tree needs from the app besides the snapshot (§8).
#[derive(Debug, Clone)]
pub struct TreeState {
    /// Keyed by node name, so a re-sort does not lose what you opened (§2.5).
    pub expanded_nodes: HashSet<String>,
    /// Keyed by (parent row id, child row id) so it works in both directions.
    pub expanded_users: HashSet<(String, String)>,
    pub pivot: bool,
    pub sort: SortKey,
    /// Substring over node, user, person and SQL (§3). Empty means no filter.
    pub filter: String,
    /// `space` toggles the healthy fold (§2.5).
    pub fold_healthy: bool,
    /// Nodes flagged NEW this session (§2.6).
    pub new_nodes: HashSet<String>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self {
            expanded_nodes: HashSet::new(),
            expanded_users: HashSet::new(),
            pivot: false,
            sort: SortKey::Pressure,
            filter: String::new(),
            fold_healthy: true,
            new_nodes: HashSet::new(),
        }
    }
}

impl TreeState {
    fn user_key(parent: &str, child: &str) -> (String, String) {
        (parent.to_string(), child.to_string())
    }

    pub fn node_expanded(&self, node: &str) -> bool {
        self.expanded_nodes.contains(node)
    }

    pub fn toggle_node(&mut self, node: &str) {
        if !self.expanded_nodes.remove(node) {
            self.expanded_nodes.insert(node.to_string());
        }
    }

    pub fn user_expanded(&self, parent: &str, child: &str) -> bool {
        self.expanded_users
            .contains(&Self::user_key(parent, child))
    }

    /// A user row's identity inside its node: two `r_redash` rows are different rows.
    pub fn user_row_key(slice: &UserSlice<'_>) -> String {
        match &slice.person {
            Some(person) => format!("{}\u{1}{person}", slice.user),
            None => slice.user.clone(),
        }
    }

    pub fn toggle_user(&mut self, parent: &str, child: &str) {
        let key = Self::user_key(parent, child);
        if !self.expanded_users.remove(&key) {
            self.expanded_users.insert(key);
        }
    }

    pub fn clear_expansions(&mut self) {
        self.expanded_nodes.clear();
        self.expanded_users.clear();
    }

    fn needle(&self) -> Option<String> {
        let trimmed = self.filter.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
    }

    /// Whether a node is a candidate for the fold: quiet, and not something the user opened.
    fn foldable(&self, view: &NodeView<'_>) -> bool {
        self.fold_healthy
            && self.needle().is_none()
            && !self.node_expanded(view.name())
            && fold_healthy(view)
    }
}

/// Build every visible row from an already-derived fleet view.
///
/// The view is passed in rather than derived here because the rows borrow it: the caller
/// (the UI, once per event) owns it for as long as it draws.
pub fn build<'a>(view: &'a FleetView<'a>, state: &TreeState) -> Vec<Row<'a>> {
    if state.pivot {
        build_pivot(view, state)
    } else {
        build_nodes(view, state)
    }
}

fn build_nodes<'a>(view: &'a FleetView<'a>, state: &TreeState) -> Vec<Row<'a>> {
    let needle = state.needle();
    let mut views: Vec<&'a NodeView<'a>> = view.nodes.iter().collect();
    sort_node_refs(&mut views, state.sort);

    let mut rows = Vec::new();
    let mut folded: Vec<String> = Vec::new();
    let mut worst = 0.0_f64;

    for node_view in views {
        if let Some(needle) = &needle
            && !node_matches(node_view, needle) {
                continue;
            }
        if state.foldable(node_view) {
            folded.push(node_view.name().to_string());
            worst = worst.max(node_view.pressure);
            continue;
        }

        rows.push(Row {
            id: RowId::Node(node_view.name().to_string()),
            depth: 0,
            kind: Kind::Node,
            payload: Payload::Node(node_view),
        });

        if !state.node_expanded(node_view.name()) {
            continue;
        }

        for slice in &node_view.users {
            if let Some(needle) = &needle
                && !slice_matches(node_view, slice, needle) {
                    continue;
                }
            rows.push(Row {
                id: RowId::User {
                    node: node_view.name().to_string(),
                    user: slice.user.clone(),
                    person: slice.person.clone(),
                },
                depth: 1,
                kind: Kind::User,
                payload: Payload::User {
                    view: node_view,
                    slice,
                },
            });

            if !state.user_expanded(node_view.name(), &TreeState::user_row_key(slice)) {
                continue;
            }
            for stat in &slice.queries {
                if let Some(needle) = &needle
                    && !query_matches(stat, needle) {
                        continue;
                    }
                rows.push(Row {
                    id: RowId::Query {
                        node: node_view.name().to_string(),
                        user: slice.user.clone(),
                        person: slice.person.clone(),
                        query_id: stat.query.query_id.clone(),
                    },
                    depth: 2,
                    kind: Kind::Query,
                    payload: Payload::Query {
                        stat,
                        node: &node_view.node.name,
                        user: &slice.user,
                    },
                });
            }
        }

        // The closing row is last in every expanded node (§2.2): user rows plus this row are
        // the node's own percentage.
        rows.push(Row {
            id: RowId::Closing(node_view.name().to_string()),
            depth: 1,
            kind: Kind::Closing,
            payload: Payload::Closing(node_view),
        });
    }

    if !folded.is_empty() {
        // After the nodes that stayed visible, like the screen in §1.
        rows.push(Row {
            id: RowId::Folded,
            depth: 0,
            kind: Kind::Folded,
            payload: Payload::Folded { names: folded, worst },
        });
    }
    rows
}

fn build_pivot<'a>(view: &'a FleetView<'a>, state: &TreeState) -> Vec<Row<'a>> {
    let needle = state.needle();
    let mut rows = Vec::new();

    for fleet_user in &view.users {
        if let Some(needle) = &needle
            && !fleet_user_matches(fleet_user, needle) {
                continue;
            }
        rows.push(Row {
            id: RowId::FleetUser(fleet_user.user.clone()),
            depth: 0,
            kind: Kind::FleetUser,
            payload: Payload::FleetUser(fleet_user),
        });

        if !state.node_expanded(&fleet_user.user) {
            continue;
        }

        for user_node in &fleet_user.nodes {
            if let Some(needle) = &needle
                && !user_node_matches(user_node, needle) {
                    continue;
                }
            rows.push(Row {
                id: RowId::PivotNode {
                    user: fleet_user.user.clone(),
                    node: user_node.node.name.clone(),
                },
                depth: 1,
                kind: Kind::User,
                payload: Payload::PivotNode {
                    user: fleet_user,
                    node: user_node,
                },
            });

            if !state.user_expanded(&fleet_user.user, &user_node.node.name) {
                continue;
            }
            for stat in &user_node.queries {
                if let Some(needle) = &needle
                    && !query_matches(stat, needle) {
                        continue;
                    }
                rows.push(Row {
                    id: RowId::Query {
                        node: user_node.node.name.clone(),
                        user: fleet_user.user.clone(),
                        person: stat.query.person.clone(),
                        query_id: stat.query.query_id.clone(),
                    },
                    depth: 2,
                    kind: Kind::Query,
                    payload: Payload::Query {
                        stat,
                        node: &user_node.node.name,
                        user: &fleet_user.user,
                    },
                });
            }
        }
    }
    rows
}

fn sort_node_refs<'a>(views: &mut [&'a NodeView<'a>], key: SortKey) {
    views.sort_by(|a, b| crate::model::compare_nodes(a, b, key));
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

fn node_matches(view: &NodeView<'_>, needle: &str) -> bool {
    let hay = format!(
        "{} {} {}",
        view.name(),
        view.node.host,
        view.node.version
    );
    if hay.to_lowercase().contains(needle) {
        return true;
    }
    view.users
        .iter()
        .any(|u| slice_matches(view, u, needle))
}

fn slice_matches(view: &NodeView<'_>, slice: &UserSlice<'_>, needle: &str) -> bool {
    if slice.user.to_lowercase().contains(needle) {
        return true;
    }
    if let Some(person) = &slice.person
        && person.to_lowercase().contains(needle) {
            return true;
        }
    slice.queries.iter().any(|q| query_matches(q, needle))
        || view.node.name.to_lowercase().contains(needle)
}

fn query_matches(stat: &QueryStat<'_>, needle: &str) -> bool {
    stat.query.user.to_lowercase().contains(needle)
        || stat
            .query
            .person
            .as_ref()
            .is_some_and(|p| p.to_lowercase().contains(needle))
        || stat.query.sql.to_lowercase().contains(needle)
        || stat.query.query_id.to_lowercase().contains(needle)
}

fn fleet_user_matches(user: &FleetUser<'_>, needle: &str) -> bool {
    if user.user.to_lowercase().contains(needle) {
        return true;
    }
    if user
        .person
        .as_ref()
        .is_some_and(|p| p.to_lowercase().contains(needle))
    {
        return true;
    }
    user.nodes.iter().any(|n| {
        n.node.name.to_lowercase().contains(needle)
            || n.queries.iter().any(|q| query_matches(q, needle))
    })
}

fn user_node_matches(node: &UserNode<'_>, needle: &str) -> bool {
    node.node.name.to_lowercase().contains(needle)
        || node.queries.iter().any(|q| query_matches(q, needle))
}

// ---------------------------------------------------------------------------
// Selection (§2.5)
// ---------------------------------------------------------------------------

/// Where the cursor is, as an index into `rows`. When the selected row is gone the cursor
/// moves to its parent, and keeps climbing until it finds something that exists.
pub fn selection_index(rows: &[Row<'_>], selected: Option<&RowId>) -> Option<usize> {
    let mut wanted = selected.cloned();
    while let Some(id) = wanted {
        if let Some(index) = rows.iter().position(|row| row.id == id) {
            return Some(index);
        }
        wanted = id.parent();
    }
    None
}

/// Expand the top-pressure node once, on the first snapshot only (§2.5).
pub fn expand_top_node(state: &mut TreeState, snapshot: &FleetSnapshot, prev: Option<&FleetSnapshot>) {
    let view = crate::model::fleet_view(snapshot, prev);
    let mut views: Vec<&NodeView<'_>> = view.nodes.iter().collect();
    sort_node_refs(&mut views, state.sort);
    if let Some(top) = views.first() {
        state.expanded_nodes.insert(top.name().to_string());
    }
}

/// Users on a node, derived the same way the rows are, so a jump from view 2 can expand
/// exactly the right user row (§2.8).
pub fn user_slices_for<'a>(
    snapshot: &'a FleetSnapshot,
    prev: Option<&'a FleetSnapshot>,
    node: &str,
) -> Vec<UserSlice<'a>> {
    let node_view = snapshot
        .nodes
        .iter()
        .find(|n| n.name == node)
        .expect("node from the tree must exist in the snapshot");
    let prev_view = prev.and_then(|p| p.nodes.iter().find(|n| n.name == node));
    user_slices(node_view, prev_view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    
    fn ids(rows: &[Row<'_>]) -> Vec<RowId> {
        rows.iter().map(|r| r.id.clone()).collect()
    }

    /// One poll of fake data, which is all the tree needs.
    fn fake_snapshot() -> FleetSnapshot {
        FakeSource::new().snapshot()
    }

    /// The default state after the first snapshot: top node open (§2.5).
    fn state_for(snapshot: &FleetSnapshot) -> TreeState {
        let mut state = TreeState::default();
        expand_top_node(&mut state, snapshot, None);
        state
    }

    // Rows borrow the derived view, so every test keeps `view` alive next to its rows: the
    // rows cannot outlive it.

    #[test]
    fn the_tree_has_three_levels_and_a_closing_row() {
        let snap = fake_snapshot();
        let state = state_for(&snap);
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);

        assert_eq!(rows[0].kind, Kind::Node);
        assert_eq!(rows[0].payload.as_node_name(), Some("clickhouse3"));

        let user_rows: Vec<&Row<'_>> = rows.iter().filter(|r| r.depth == 1).collect();
        assert!(user_rows.len() >= 4);
        assert!(user_rows
            .iter()
            .any(|r| matches!(&r.payload, Payload::User { slice, .. } if slice.user == "r_redash")));

        // The closing row comes after that node's user rows, not at the end of the screen.
        let closing = rows
            .iter()
            .position(|r| r.kind == Kind::Closing)
            .expect("an expanded node has a closing row");
        assert_eq!(rows[closing].id, RowId::Closing("clickhouse3".into()));
    }

    #[test]
    fn expanding_a_user_adds_its_queries() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);
        let view = crate::model::fleet_view(&snap, None);
        let before = build(&view, &state).len();
        state.toggle_user("clickhouse3", "r_redash\u{1}grigol.gankava");
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);

        assert!(rows.len() > before);
        assert!(rows.iter().any(|r| matches!(r.kind, Kind::Query)));
        assert!(rows.iter().all(|r| r.depth <= 2));
    }

    #[test]
    fn the_fold_keeps_healthy_nodes_on_one_line() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);

        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        let folded = rows.iter().find(|r| r.kind == Kind::Folded).expect("fold row");
        match &folded.payload {
            Payload::Folded { names, .. } => {
                assert_eq!(names, &["ch4", "ch6", "ch8", "ch9"].map(String::from).to_vec())
            }
            _ => panic!("expected the fold row"),
        }

        state.fold_healthy = false;
        let view = crate::model::fleet_view(&snap, None);
        let unfolded = build(&view, &state);
        assert!(unfolded.iter().all(|r| r.kind != Kind::Folded));
        assert!(unfolded.len() > rows.len());
    }

    #[test]
    fn selection_survives_a_refresh_that_reorders_nodes() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);

        // Select a node that is not the first row.
        let target = rows
            .iter()
            .find(|r| r.kind == Kind::Node && r.payload.as_node_name() == Some("clickhouse-bi"))
            .expect("clickhouse-bi is in the fleet")
            .id
            .clone();

        // A re-sort puts it first.
        state.sort = SortKey::Name;
        let view = crate::model::fleet_view(&snap, None);
        let resorted = build(&view, &state);
        let index = selection_index(&resorted, Some(&target)).expect("still selected");
        assert_eq!(resorted[index].id, target);
    }

    #[test]
    fn a_vanished_query_moves_the_selection_to_its_user() {
        let mut snap = fake_snapshot();
        // Give grigol a second query so the user row outlives the one that finishes.
        let second = crate::model::QueryRow {
            query_id: "cafe0001".into(),
            user: "r_redash".into(),
            person: Some("grigol.gankava".into()),
            redash_query_id: Some(7439),
            elapsed_s: 9.0,
            memory_bytes: 512 * 1024 * 1024,
            read_rows: 10,
            read_bytes: 1024,
            sql: "/* Username: grigol.gankava@paysera.net, */ SELECT 2".into(),
            cpu_time_us: 1_800_000,
        };
        snap.nodes
            .iter_mut()
            .find(|n| n.name == "clickhouse3")
            .unwrap()
            .queries
            .push(second);

        let mut state = state_for(&snap);
        state.toggle_user("clickhouse3", "r_redash\u{1}grigol.gankava");

        let selected = {
            let view = crate::model::fleet_view(&snap, None);
            let rows = build(&view, &state);
            rows.iter()
                .find(|r| r.kind == Kind::Query)
                .expect("r_redash has queries")
                .id
                .clone()
        };

        // That query finishes; the user row stays, because its other query is still running.
        let RowId::Query { query_id, .. } = &selected else {
            panic!("expected a query row")
        };
        for node in &mut snap.nodes {
            node.queries.retain(|q| &q.query_id != query_id);
        }
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        let index = selection_index(&rows, Some(&selected)).expect("fell back somewhere");
        assert_eq!(
            rows[index].id,
            RowId::User {
                node: "clickhouse3".into(),
                user: "r_redash".into(),
                person: Some("grigol.gankava".into()),
            }
        );
    }

    #[test]
    fn a_query_whose_user_row_also_vanishes_falls_back_to_the_node() {
        let mut snap = fake_snapshot();
        let mut state = state_for(&snap);
        state.toggle_user("clickhouse3", "r_redash\u{1}j.petrova");

        let selected = {
            let view = crate::model::fleet_view(&snap, None);
            let rows = build(&view, &state);
            rows.iter()
                .find(|r| matches!(&r.payload, Payload::Query { stat, .. } if stat.query.person.as_deref() == Some("j.petrova")))
                .expect("j.petrova has a query")
                .id
                .clone()
        };

        // Nobody is left on that node under that person, so there is no user row either.
        let RowId::Query { node, .. } = &selected else {
            panic!("expected a query row")
        };
        let node = node.clone();
        snap.nodes
            .iter_mut()
            .find(|n| n.name == node)
            .unwrap()
            .queries
            .retain(|q| q.person.as_deref() != Some("j.petrova"));

        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        let index = selection_index(&rows, Some(&selected)).expect("fell back to the node");
        assert_eq!(rows[index].id, RowId::Node(node));
    }

    #[test]
    fn a_vanished_node_moves_the_selection_to_nowhere_rather_than_a_wrong_row() {
        let mut snap = fake_snapshot();
        let gone = RowId::Node("ch4".into());
        snap.nodes.retain(|n| n.name != "ch4");
        let state = TreeState::default();
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        assert_eq!(selection_index(&rows, Some(&gone)), None);
    }

    #[test]
    fn the_pivot_round_trips_to_the_same_rows() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);

        let view = crate::model::fleet_view(&snap, None);
        let node_rows = build(&view, &state);

        state.pivot = true;
        let view = crate::model::fleet_view(&snap, None);
        let pivot_rows = build(&view, &state);
        assert_eq!(pivot_rows[0].kind, Kind::FleetUser);
        assert!(pivot_rows
            .iter()
            .any(|r| matches!(r.id, RowId::FleetUser(ref u) if u == "r_redash")));

        state.pivot = false;
        let view = crate::model::fleet_view(&snap, None);
        let again = build(&view, &state);
        assert_eq!(ids(&node_rows), ids(&again), "the flip is lossless");
    }

    #[test]
    fn the_pivot_puts_the_heaviest_user_first_and_shows_each_node() {
        let snap = fake_snapshot();
        let mut state = TreeState {
            pivot: true,
            ..TreeState::default()
        };
        state.toggle_node("r_redash");
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);

        assert_eq!(rows[0].id, RowId::FleetUser("r_redash".into()));
        let nodes: Vec<&RowId> = rows
            .iter()
            .filter(|r| r.depth == 1)
            .map(|r| &r.id)
            .collect();
        assert!(nodes.iter().any(|id| matches!(
            id,
            RowId::PivotNode { node, .. } if node == "clickhouse3"
        )));
        assert!(nodes.iter().any(|id| matches!(
            id,
            RowId::PivotNode { node, .. } if node == "clickhouse-bi"
        )));
    }

    #[test]
    fn the_filter_keeps_the_containers_of_what_matched() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);
        state.filter = "j.petrova".into();
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);

        assert!(!rows.is_empty());
        assert!(rows.iter().any(|r| matches!(&r.payload,
            Payload::User { slice, .. } if slice.person.as_deref() == Some("j.petrova"))));
        // The node row of a match is kept even though the node name does not match.
        assert!(rows.iter().any(|r| r.kind == Kind::Node));
    }

    #[test]
    fn the_filter_looks_inside_sql() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);
        state.filter = "bank_record".into();
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        assert!(rows.iter().any(|r| r.kind == Kind::Node));
        state.toggle_user("clickhouse3", "r_redash\u{1}grigol.gankava");
        let view = crate::model::fleet_view(&snap, None);
        let expanded = build(&view, &state);
        assert!(expanded
            .iter()
            .any(|r| matches!(&r.payload, Payload::Query { stat, .. }
                if stat.query.sql.to_lowercase().contains("bank_record"))));
    }

    #[test]
    fn a_filter_suspends_the_fold() {
        let snap = fake_snapshot();
        let mut state = state_for(&snap);
        state.filter = "ch4".into();
        let view = crate::model::fleet_view(&snap, None);
        let rows = build(&view, &state);
        assert!(
            rows.iter().all(|r| r.kind != Kind::Folded),
            "folding while filtering would hide what the filter found"
        );
    }

    impl<'a> Payload<'a> {
        fn as_node_name(&self) -> Option<&'a str> {
            match self {
                Payload::Node(view) => Some(view.name()),
                Payload::Closing(view) => Some(view.name()),
                _ => None,
            }
        }
    }
}
