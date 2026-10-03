//! A short memory: the last few minutes of the numbers on screen, so the screen can say where
//! a number is going and not only where it is.
//!
//! Everything here is pure bookkeeping over values the model already derived (§5) — no new
//! measurement, no I/O. The time axis is the snapshot's own `taken_at` (or, for one query, its
//! own `elapsed`), never the wall clock of the machine drawing the screen, so a paused screen
//! or a slow redraw cannot bend a slope.

use crate::model::{FleetTotals, FleetView, QueueStatus};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::SystemTime;

/// What a sparkline covers: four minutes, i.e. 120 polls at the default `POLL_MS`.
pub const WINDOW_S: f64 = 240.0;
/// Points older than this are dropped. A little more than the window so the oldest cell of a
/// sparkline is never half-empty just because the point that filled it was trimmed.
const KEEP_S: f64 = WINDOW_S + 30.0;
/// A hard cap per series, for a `POLL_MS` far below the default.
const MAX_POINTS: usize = 600;

/// A slope needs at least this many points …
const MIN_POINTS: usize = 4;
/// … spread over at least this much time, or it is noise being called a trend.
const MIN_SPAN_S: f64 = 6.0;

/// Seconds since the epoch, the time axis of every series.
pub fn secs(at: SystemTime) -> f64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// One number over time, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Series {
    points: VecDeque<(f64, f64)>,
}

impl Series {
    pub fn push(&mut self, t: f64, value: f64) {
        if !value.is_finite() {
            return;
        }
        if let Some(last) = self.points.back_mut() {
            if (last.0 - t).abs() < 1e-9 {
                // The same poll twice (a redraw, a duplicate event): keep the newer value.
                last.1 = value;
                return;
            }
            if t < last.0 {
                // Time went backwards — a restarted source or a clock jump. A slope across
                // that would be fiction, so the series starts over.
                self.points.clear();
            }
        }
        self.points.push_back((t, value));
        while self.points.len() > MAX_POINTS {
            self.points.pop_front();
        }
        while self
            .points
            .front()
            .is_some_and(|(first, _)| t - first > KEEP_S)
        {
            self.points.pop_front();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    #[cfg(test)]
    pub fn last(&self) -> Option<f64> {
        self.points.back().map(|(_, v)| *v)
    }

    /// How much time the series covers.
    pub fn span_s(&self) -> f64 {
        match (self.points.front(), self.points.back()) {
            (Some(first), Some(last)) => last.0 - first.0,
            _ => 0.0,
        }
    }

    pub fn last_t(&self) -> Option<f64> {
        self.points.back().map(|(t, _)| *t)
    }

    #[cfg(test)]
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.points.iter().copied()
    }

    /// The average of the last `window_s` — a CPU reading that jumps between polls, steadied.
    pub fn mean_in(&self, window_s: f64) -> Option<f64> {
        let last = self.last_t()?;
        let (sum, n) = self
            .points
            .iter()
            .filter(|(t, _)| last - t <= window_s)
            .fold((0.0, 0usize), |(sum, n), (_, v)| (sum + v, n + 1));
        (n > 0).then(|| sum / n as f64)
    }

    pub fn max_in(&self, window_s: f64) -> Option<f64> {
        let last = self.last_t()?;
        self.points
            .iter()
            .filter(|(t, _)| last - t <= window_s)
            .map(|(_, v)| *v)
            .reduce(f64::max)
    }

    /// Least-squares slope, in value per second, over the points of the last `window_s`.
    ///
    /// A regression rather than first-minus-last, because a single noisy poll at either end
    /// would otherwise decide the arrow on screen.
    pub fn slope(&self, window_s: f64) -> Option<f64> {
        let last = self.last_t()?;
        let points: Vec<(f64, f64)> = self
            .points
            .iter()
            .copied()
            .filter(|(t, _)| last - t <= window_s)
            .collect();
        if points.len() < MIN_POINTS {
            return None;
        }
        let span = points.last()?.0 - points.first()?.0;
        if span < MIN_SPAN_S {
            return None;
        }
        let n = points.len() as f64;
        let mean_t = points.iter().map(|(t, _)| t).sum::<f64>() / n;
        let mean_v = points.iter().map(|(_, v)| v).sum::<f64>() / n;
        let mut num = 0.0;
        let mut den = 0.0;
        for (t, v) in &points {
            num += (t - mean_t) * (v - mean_v);
            den += (t - mean_t).powi(2);
        }
        (den > 0.0).then(|| num / den)
    }

    /// The series cut into `width` equal slices of time ending at `now`, each slice holding the
    /// largest value seen in it. Peaks are what on call looks for, so a slice keeps its worst
    /// moment rather than an average that would hide it. Slices with no point are `None`, so
    /// a gap (a node that did not answer) stays visible as a gap.
    pub fn buckets(&self, now: f64, window_s: f64, width: usize) -> Vec<Option<f64>> {
        let mut cells: Vec<Option<f64>> = vec![None; width];
        if width == 0 || window_s <= 0.0 {
            return cells;
        }
        let cell = window_s / width as f64;
        let start = now - window_s;
        for (t, v) in &self.points {
            if *t <= start || *t > now + 1e-9 {
                continue;
            }
            let index = (((t - start) / cell).ceil() as usize).clamp(1, width) - 1;
            cells[index] = Some(cells[index].map_or(*v, |old: f64| old.max(*v)));
        }
        cells
    }
}

/// A forecast needs this much history behind it; a trend arrow makes do with less.
pub const FORECAST_MIN_SPAN_S: f64 = 30.0;

/// Seconds until `current` reaches `target` at `slope` per second. `None` when it never will at
/// this rate, or when it is already there (nothing left to forecast).
pub fn eta_to(current: f64, slope_per_s: f64, target: f64) -> Option<f64> {
    if current >= target || slope_per_s <= 1e-9 {
        return None;
    }
    Some((target - current) / slope_per_s)
}

/// Where a number is heading, as one glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    RisingFast,
    Rising,
    Flat,
    Falling,
    FallingFast,
}

impl Trend {
    /// `step` is what counts as moving, per minute, for this kind of number: 0.5 points of a
    /// percentage, one job in a queue.
    pub fn from_slope(per_s: Option<f64>, step_per_min: f64) -> Trend {
        let Some(per_s) = per_s else {
            return Trend::Flat;
        };
        let per_min = per_s * 60.0;
        if per_min >= step_per_min * 4.0 {
            Trend::RisingFast
        } else if per_min >= step_per_min {
            Trend::Rising
        } else if per_min <= -step_per_min * 4.0 {
            Trend::FallingFast
        } else if per_min <= -step_per_min {
            Trend::Falling
        } else {
            Trend::Flat
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Trend::RisingFast => "↑",
            Trend::Rising => "↗",
            Trend::Flat => "→",
            Trend::Falling => "↘",
            Trend::FallingFast => "↓",
        }
    }

    pub fn rising(self) -> bool {
        matches!(self, Trend::Rising | Trend::RisingFast)
    }
}

/// The eight heights of a block sparkline.
pub const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// One sparkline cell for `value` on a `[lo, hi]` scale.
pub fn spark_glyph(value: f64, lo: f64, hi: f64) -> char {
    if hi <= lo {
        return SPARK[0];
    }
    let level = ((value - lo) / (hi - lo) * 7.0).round().clamp(0.0, 7.0) as usize;
    SPARK[level]
}

// ---------------------------------------------------------------------------
// What is remembered
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct NodeHistory {
    pub mem_pct: Series,
    pub cpu_pct: Series,
    pub running: Series,
    pub lag_s: Series,
    pub poll_ms: Series,
    /// When this node first answered in this session (wall time of the snapshot).
    pub first_seen: f64,
    /// Since when it has not been answering, if it is not.
    pub unreachable_since: Option<f64>,
}

/// One running query, on its own time axis (`elapsed`), so the rates are exact even when a
/// poll arrives late.
#[derive(Debug, Clone, Default)]
pub struct QueryHistory {
    pub mem: Series,
    pub read_bytes: Series,
    pub read_rows: Series,
    /// The largest memory seen, for the tape's "finished" line.
    pub peak_mem: u64,
}

impl QueryHistory {
    /// Bytes per second of memory growth over the last 30 s of the query's life.
    pub fn mem_rate(&self) -> Option<f64> {
        self.mem.slope(30.0)
    }

    /// Bytes per second read over the last 30 s.
    pub fn read_rate(&self) -> Option<f64> {
        self.read_bytes.slope(30.0).map(|r| r.max(0.0))
    }
}

#[derive(Debug, Clone, Default)]
pub struct History {
    nodes: HashMap<String, NodeHistory>,
    queries: HashMap<(String, String), QueryHistory>,
    /// Memory share of one user row, keyed by (node, user row key), for the user trend arrow.
    users: HashMap<(String, String), Series>,
    pub fleet_mem_pct: Series,
    pub fleet_cpu_pct: Series,
    pub fleet_queries: Series,
    pub fleet_runaways: Series,
    queues: HashMap<String, Series>,
    pub queue_waiting: Series,
    pub queue_oldest: Series,
    /// The `taken_at` of the last fleet snapshot recorded.
    now: f64,
}

impl History {
    pub fn now(&self) -> f64 {
        self.now
    }

    pub fn node(&self, name: &str) -> Option<&NodeHistory> {
        self.nodes.get(name)
    }

    pub fn query(&self, node: &str, query_id: &str) -> Option<&QueryHistory> {
        self.queries.get(&(node.to_string(), query_id.to_string()))
    }

    pub fn user(&self, node: &str, user_key: &str) -> Option<&Series> {
        self.users.get(&(node.to_string(), user_key.to_string()))
    }

    pub fn queue(&self, name: &str) -> Option<&Series> {
        self.queues.get(name)
    }

    /// Record one derived fleet snapshot.
    pub fn record_fleet(&mut self, view: &FleetView<'_>, totals: &FleetTotals, at: SystemTime) {
        let t = secs(at);
        self.now = t;

        if let Some(pct) = totals.mem_pct() {
            self.fleet_mem_pct.push(t, pct);
        }
        if let Some(pct) = totals.cpu_pct() {
            self.fleet_cpu_pct.push(t, pct);
        }
        self.fleet_queries.push(t, totals.queries as f64);
        self.fleet_runaways.push(t, totals.runaways as f64);

        let mut seen_queries: HashSet<(String, String)> = HashSet::new();
        let mut seen_users: HashSet<(String, String)> = HashSet::new();
        for node in &view.nodes {
            let name = node.node.name.clone();
            let entry = self.nodes.entry(name.clone()).or_insert_with(|| NodeHistory {
                first_seen: t,
                ..NodeHistory::default()
            });
            if !node.node.reachable {
                // A gap, not a zero: the sparkline shows nothing for the time it was gone.
                entry.unreachable_since.get_or_insert(t);
                continue;
            }
            entry.unreachable_since = None;
            if let Some(pct) = node.mem_pct {
                entry.mem_pct.push(t, pct);
            }
            if let Some(pct) = node.cpu_pct {
                entry.cpu_pct.push(t, pct);
            }
            entry.running.push(t, f64::from(node.node.running));
            entry.lag_s.push(t, node.node.lag_s as f64);
            if let Some(ms) = node.node.poll_ms {
                entry.poll_ms.push(t, f64::from(ms));
            }

            for slice in &node.users {
                let key = (name.clone(), crate::tree::TreeState::user_row_key(slice));
                if let Some(pct) = slice.mem_pct {
                    self.users.entry(key.clone()).or_default().push(t, pct);
                }
                seen_users.insert(key);
                for stat in &slice.queries {
                    let q = stat.query;
                    let key = (name.clone(), q.query_id.clone());
                    let history = self.queries.entry(key.clone()).or_default();
                    history.mem.push(q.elapsed_s, q.memory_bytes as f64);
                    history.read_bytes.push(q.elapsed_s, q.read_bytes as f64);
                    history.read_rows.push(q.elapsed_s, q.read_rows as f64);
                    history.peak_mem = history
                        .peak_mem
                        .max(q.memory_bytes)
                        .max(q.peak_memory_bytes);
                    seen_queries.insert(key);
                }
            }
        }

        // Finished queries and vanished user rows are forgotten: their numbers have nothing
        // left to say, and a query id is never reused.
        self.queries.retain(|key, _| seen_queries.contains(key));
        let fleet: HashSet<&str> = view.nodes.iter().map(|n| n.node.name.as_str()).collect();
        self.users.retain(|key, series| {
            seen_users.contains(key)
                || (t - series.last_t().unwrap_or(t) < KEEP_S && fleet.contains(key.0.as_str()))
        });
        // A node that left the fleet (§2.6 drops it after five minutes) takes its history along.
        self.nodes.retain(|name, _| fleet.contains(name.as_str()));
    }

    /// Record one queue status. Queue polls run on their own clock (§6.3).
    pub fn record_queue(&mut self, queue: &QueueStatus) {
        if !queue.reachable {
            return;
        }
        let t = secs(queue.taken_at);
        let mut total = 0.0;
        let mut oldest: f64 = 0.0;
        for row in &queue.queues {
            self.queues
                .entry(row.name.clone())
                .or_default()
                .push(t, f64::from(row.waiting));
            total += f64::from(row.waiting);
            oldest = oldest.max(row.oldest_wait_s.unwrap_or(0) as f64);
        }
        self.queue_waiting.push(t, total);
        self.queue_oldest.push(t, oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeSource;
    use crate::model::{fleet_totals, fleet_view};

    fn series(points: &[(f64, f64)]) -> Series {
        let mut s = Series::default();
        for (t, v) in points {
            s.push(*t, *v);
        }
        s
    }

    #[test]
    fn a_slope_is_a_regression_over_the_window() {
        // +1 per second, exactly.
        let s = series(&[(0.0, 10.0), (2.0, 12.0), (4.0, 14.0), (6.0, 16.0), (8.0, 18.0)]);
        assert!((s.slope(60.0).unwrap() - 1.0).abs() < 1e-9);
        // One noisy point does not flip it.
        let noisy = series(&[(0.0, 10.0), (2.0, 12.0), (4.0, 30.0), (6.0, 16.0), (8.0, 18.0)]);
        assert!(noisy.slope(60.0).unwrap() > 0.0);
    }

    #[test]
    fn too_little_history_is_no_trend() {
        assert_eq!(series(&[(0.0, 1.0), (2.0, 2.0), (4.0, 3.0)]).slope(60.0), None, "3 points");
        assert_eq!(
            series(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)]).slope(60.0),
            None,
            "4 points but only 3 s apart"
        );
        assert_eq!(Trend::from_slope(None, 1.0), Trend::Flat);
    }

    #[test]
    fn time_going_backwards_starts_the_series_over() {
        let mut s = series(&[(10.0, 1.0), (12.0, 2.0)]);
        s.push(5.0, 9.0);
        assert_eq!(s.len(), 1);
        assert_eq!(s.last(), Some(9.0));
        // The same instant twice keeps one point with the newer value.
        s.push(5.0, 7.0);
        assert_eq!(s.len(), 1);
        assert_eq!(s.last(), Some(7.0));
    }

    #[test]
    fn old_points_are_trimmed() {
        let mut s = Series::default();
        for i in 0..400 {
            s.push(f64::from(i) * 2.0, 1.0);
        }
        let first = s.points().next().unwrap().0;
        assert!(s.last_t().unwrap() - first <= KEEP_S);
    }

    #[test]
    fn the_mean_steadies_a_jumpy_reading() {
        let s = series(&[(0.0, 20.0), (2.0, 95.0), (4.0, 20.0), (6.0, 95.0)]);
        assert_eq!(s.mean_in(4.0), Some((95.0 + 20.0 + 95.0) / 3.0));
        assert_eq!(Series::default().mean_in(10.0), None);
    }

    #[test]
    fn eta_is_only_for_numbers_on_their_way_up() {
        assert_eq!(eta_to(90.0, 0.5, 100.0), Some(20.0));
        assert_eq!(eta_to(90.0, 0.0, 100.0), None, "flat never arrives");
        assert_eq!(eta_to(90.0, -1.0, 100.0), None, "falling never arrives");
        assert_eq!(eta_to(100.0, 1.0, 100.0), None, "already there");
    }

    #[test]
    fn trend_arrows_follow_the_step() {
        let per_s = |per_min: f64| Some(per_min / 60.0);
        assert_eq!(Trend::from_slope(per_s(0.2), 0.5), Trend::Flat);
        assert_eq!(Trend::from_slope(per_s(0.6), 0.5), Trend::Rising);
        assert_eq!(Trend::from_slope(per_s(3.0), 0.5), Trend::RisingFast);
        assert_eq!(Trend::from_slope(per_s(-0.6), 0.5), Trend::Falling);
        assert_eq!(Trend::from_slope(per_s(-3.0), 0.5), Trend::FallingFast);
        assert_eq!(Trend::RisingFast.arrow(), "↑");
        assert!(Trend::Rising.rising());
    }

    #[test]
    fn buckets_keep_the_peak_and_show_gaps() {
        // 4 cells of 10 s ending at t=40.
        let s = series(&[(5.0, 1.0), (8.0, 9.0), (25.0, 3.0), (39.0, 4.0)]);
        assert_eq!(
            s.buckets(40.0, 40.0, 4),
            vec![Some(9.0), None, Some(3.0), Some(4.0)],
            "the 10–20 s slice had no poll"
        );
        assert_eq!(s.buckets(40.0, 40.0, 0), Vec::<Option<f64>>::new());
    }

    #[test]
    fn spark_glyphs_span_the_scale() {
        assert_eq!(spark_glyph(0.0, 0.0, 100.0), '▁');
        assert_eq!(spark_glyph(100.0, 0.0, 100.0), '█');
        assert_eq!(spark_glyph(250.0, 0.0, 100.0), '█', "clamped");
        assert_eq!(spark_glyph(5.0, 3.0, 3.0), '▁', "an empty scale is the floor");
    }

    #[test]
    fn recording_the_fake_fleet_remembers_nodes_queries_and_rates() {
        let mut fake = FakeSource::new();
        let mut history = History::default();
        let mut prev = None;
        for _ in 0..10 {
            let snap = fake.snapshot();
            {
                let view = fleet_view(&snap, prev.as_ref());
                let totals = fleet_totals(&view);
                history.record_fleet(&view, &totals, snap.taken_at);
            }
            prev = Some(snap);
        }
        let ch3 = history.node("clickhouse3").expect("clickhouse3 has history");
        assert_eq!(ch3.mem_pct.len(), 10);
        assert!(ch3.unreachable_since.is_none());
        assert_eq!(history.fleet_mem_pct.len(), 10);

        // The long r_redash query grows memory and reads steadily in the fake fleet.
        let snap = prev.unwrap();
        let ch3_node = snap.nodes.iter().find(|n| n.name == "clickhouse3").unwrap();
        let longest = ch3_node
            .queries
            .iter()
            .max_by(|a, b| a.elapsed_s.total_cmp(&b.elapsed_s))
            .unwrap();
        let q = history
            .query("clickhouse3", &longest.query_id)
            .expect("a running query has history");
        assert!(q.mem_rate().unwrap() > 0.0, "its memory grows");
        assert!(q.read_rate().unwrap() > 0.0, "it reads");
        assert!(q.peak_mem >= longest.memory_bytes);
    }

    #[test]
    fn an_unreachable_node_is_a_gap_not_a_zero() {
        let mut fake = FakeSource::new();
        let mut history = History::default();
        let mut snap = fake.snapshot();
        {
            let view = fleet_view(&snap, None);
            history.record_fleet(&view, &fleet_totals(&view), snap.taken_at);
        }
        for node in &mut snap.nodes {
            if node.name == "ch4" {
                *node = crate::model::NodeSnapshot::unreachable("ch4", "connection refused");
            }
        }
        snap.taken_at += std::time::Duration::from_secs(2);
        let view = fleet_view(&snap, None);
        history.record_fleet(&view, &fleet_totals(&view), snap.taken_at);
        let ch4 = history.node("ch4").unwrap();
        assert_eq!(ch4.mem_pct.len(), 1, "no point recorded while it was gone");
        assert_eq!(ch4.unreachable_since, Some(secs(snap.taken_at)));
    }

    #[test]
    fn the_queue_is_recorded_on_its_own_clock() {
        let mut fake = FakeSource::new();
        let mut history = History::default();
        for _ in 0..3 {
            history.record_queue(&fake.queue());
        }
        assert_eq!(history.queue_waiting.len(), 3);
        assert_eq!(history.queue("queries").map(Series::len), Some(3));
        history.record_queue(&QueueStatus::unreachable("HTTP 401"));
        assert_eq!(history.queue_waiting.len(), 3, "an unreachable queue records nothing");
    }
}
