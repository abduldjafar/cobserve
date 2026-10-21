//! A browser, opened from this machine (view 0): its processes part by part (`local::Breakdown`),
//! its open tabs by site, the tab process the owner asked to stop, and how greedy it is.
//!
//! Pure, like the model: a breakdown and a tab read in, rows out. Nothing here is worked out per
//! frame — the rows are rebuilt when a read or a key changes them, and the draw only places them.
//!
//! The share (`docs` §7 of the Chrome spec): `share = Σ rss ÷ hw.memsize × 100`, amber at
//! `CHROME_WARN_PCT` (40), red at `CHROME_CRIT_PCT` (60). RSS counts a shared page in each process
//! that maps it, so the sum is a little over what the browser holds — the screen says RSS.

use crate::local::{Breakdown, Helper, Local, Part};
use crate::severity::Severity;
use crate::sources::chrome::{self, Tab};
use std::collections::BTreeSet;

/// The default lines, in percent of the Mac's memory.
pub const WARN_PCT: f64 = 40.0;
pub const CRIT_PCT: f64 = 60.0;

/// How long a tab read is taken as it was, while the drill-down is open or the phone asked.
pub const TABS_FRESH_S: f64 = 30.0;

/// How greedy a browser is: its share of the Mac's memory against the two lines. Unknown is
/// `None`, never a made-up 0; no lines (`CHROME_WARN_PCT=off`) is `None` too.
pub fn share_severity(share: Option<f64>, lines: Option<(f64, f64)>) -> Severity {
    match (share, lines) {
        (Some(s), Some((_, crit))) if s >= crit => Severity::Crit,
        (Some(s), Some((warn, _))) if s >= warn => Severity::Warn,
        (Some(_), Some(_)) => Severity::Ok,
        _ => Severity::None,
    }
}

/// A browser's share of the Mac's memory, percent.
pub fn share(local: &Local, b: &Breakdown) -> Option<f64> {
    let total = local.latest.as_ref()?.memory?.total;
    (total > 0).then(|| b.rss as f64 / total as f64 * 100.0)
}

/// What a browser is called in a short line: `Chrome`, `Brave`, `Edge`.
pub fn short(program: &str) -> &str {
    match program {
        "Google Chrome" | "Google Chrome Beta" | "Google Chrome Canary" => "Chrome",
        "Brave Browser" => "Brave",
        "Microsoft Edge" => "Edge",
        other => other,
    }
}

/// A part in the screen's words.
pub fn part_word(part: Part) -> &'static str {
    match part {
        Part::Browser => "browser",
        Part::Tab => "tabs",
        Part::Extension => "extensions",
        Part::Gpu => "GPU",
        Part::Network => "network",
        Part::Storage => "storage",
        Part::Audio => "audio",
        Part::Other => "other",
    }
}

/// One process's part, singular: `tab`, `extension`.
pub fn helper_word(part: Part) -> &'static str {
    match part {
        Part::Tab => "tab",
        Part::Extension => "extension",
        other => part_word(other),
    }
}

/// The tabs of one site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub host: String,
    /// Indexes into the read's tabs, in Chrome's order.
    pub tabs: Vec<usize>,
}

/// One read of a browser's tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct TabsRead {
    pub program: String,
    /// When it was read, Unix seconds.
    pub at: f64,
    pub result: Result<Vec<Tab>, String>,
    /// The tabs by site, most tabs first, then by name: built once, when the read comes in.
    pub sites: Vec<Site>,
}

impl TabsRead {
    pub fn new(program: String, at: f64, result: Result<Vec<Tab>, String>) -> TabsRead {
        let sites = result.as_ref().map(|tabs| group(tabs)).unwrap_or_default();
        TabsRead { program, at, result, sites }
    }

    pub fn tabs(&self) -> &[Tab] {
        self.result.as_deref().unwrap_or_default()
    }

    /// The tab on screen: the front window's (Chrome's window 1).
    pub fn on_screen(&self) -> Option<&Tab> {
        self.tabs().iter().find(|t| t.window == 1 && t.active)
    }

    /// How many windows: Chrome numbers them from 1.
    pub fn windows(&self) -> u32 {
        self.tabs().iter().map(|t| t.window).max().unwrap_or(0)
    }
}

pub fn group(tabs: &[Tab]) -> Vec<Site> {
    let mut sites: Vec<Site> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        let host = chrome::site(&tab.url);
        match sites.iter_mut().find(|s| s.host == host) {
            Some(site) => site.tabs.push(i),
            None => sites.push(Site { host, tabs: vec![i] }),
        }
    }
    sites.sort_by(|a, b| b.tabs.len().cmp(&a.tabs.len()).then(a.host.cmp(&b.host)));
    sites
}

/// A row of the drill-down's one list.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Process(Helper),
    Site { host: String, count: usize },
    /// A tab of an opened site, by its index in the read.
    Tab(usize),
}

impl Row {
    fn key(&self) -> String {
        match self {
            Row::Process(h) => format!("p:{}", h.pid),
            Row::Site { host, .. } => format!("s:{host}"),
            Row::Tab(i) => format!("t:{i}"),
        }
    }
}

/// The drill-down: a browser's processes and its tabs, under one cursor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drill {
    pub program: String,
    pub rows: Vec<Row>,
    pub selected: usize,
    selected_key: Option<String>,
    /// The sites whose tabs are shown.
    pub open: BTreeSet<String>,
    /// The process `x` asked about, until `y` stops it or another key keeps it.
    pub asked: Option<Helper>,
    /// The first row of the list on screen, kept by the draw's last frame.
    pub offset: usize,
}

impl Drill {
    pub fn new(program: &str, local: &Local, tabs: Option<&TabsRead>, on: Option<u32>) -> Drill {
        let mut drill = Drill { program: program.to_string(), selected_key: on.map(|pid| format!("p:{pid}")), ..Drill::default() };
        drill.rebuild(local, tabs);
        drill
    }

    /// The rows again from the latest read and tab read, the cursor kept on what it was on.
    pub fn rebuild(&mut self, local: &Local, tabs: Option<&TabsRead>) {
        let mut rows: Vec<Row> = local.browser(&self.program).map(|b| b.heaviest.iter().cloned().map(Row::Process).collect()).unwrap_or_default();
        if let Some(read) = tabs.filter(|t| t.program == self.program) {
            for site in &read.sites {
                rows.push(Row::Site { host: site.host.clone(), count: site.tabs.len() });
                if self.open.contains(&site.host) {
                    rows.extend(site.tabs.iter().map(|&i| Row::Tab(i)));
                }
            }
        }
        self.rows = rows;
        let found = self.selected_key.as_ref().and_then(|k| self.rows.iter().position(|r| r.key() == *k));
        self.selected = found.unwrap_or(self.selected).min(self.rows.len().saturating_sub(1));
        self.selected_key = self.rows.get(self.selected).map(Row::key);
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        self.selected_key = self.rows.get(self.selected).map(Row::key);
    }

    pub fn select_by(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    pub fn current(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// `⏎` on a site: its tabs shown, or folded again.
    pub fn toggle(&mut self, local: &Local, tabs: Option<&TabsRead>) {
        if let Some(Row::Site { host, .. }) = self.current() {
            let host = host.clone();
            if !self.open.remove(&host) {
                self.open.insert(host);
            }
            self.rebuild(local, tabs);
        }
    }

    /// How many process rows lead the list.
    pub fn processes(&self) -> usize {
        self.rows.iter().take_while(|r| matches!(r, Row::Process(_))).count()
    }
}

/// The level a browser's share is at, held across reads so it does not flicker on the line: a
/// level is raised after two reads at or over its line, and lowered after two reads 3 points
/// under it — the tape's own rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Greed {
    pub level: Severity,
    /// When the level now held was raised, Unix seconds.
    pub since: f64,
    pending: Option<(Severity, u8)>,
}

impl Default for Greed {
    fn default() -> Greed {
        Greed { level: Severity::None, since: 0.0, pending: None }
    }
}

/// Reads at a new level before it is taken.
pub const CONFIRM_READS: u8 = 2;
/// How far under a line a share must fall to clear it.
pub const MARGIN: f64 = 3.0;

impl Greed {
    /// The share's level now, given the level held: what it would be lowered to only once it is
    /// `MARGIN` under the line.
    fn target(&self, share: f64, lines: (f64, f64)) -> Severity {
        let (warn, crit) = lines;
        let held = self.level;
        if share >= crit || (held == Severity::Crit && share >= crit - MARGIN) {
            Severity::Crit
        } else if share >= warn || (held >= Severity::Warn && share >= warn - MARGIN) {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }

    /// One read: the new level, when it changed.
    pub fn observe(&mut self, share: f64, lines: (f64, f64), at: f64) -> Option<Severity> {
        let target = self.target(share, lines);
        if target == self.level {
            self.pending = None;
            return None;
        }
        let seen = match self.pending {
            Some((level, n)) if level == target => n + 1,
            _ => 1,
        };
        if seen >= CONFIRM_READS {
            self.level = target;
            self.since = at;
            self.pending = None;
            Some(target)
        } else {
            self.pending = Some((target, seen));
            None
        }
    }
}

/// The tape's line for a browser whose level changed (`before` the level held and since when).
pub fn tape_line(b: &Breakdown, share: f64, total: Option<u64>, lines: (f64, f64), before: (Severity, f64), level: Severity, at: f64) -> Option<crate::tape::Event> {
    let (warn, crit) = lines;
    let of = total.map_or(String::new(), |t| format!(" of {:.0} GiB", crate::fmt::gib(t)));
    let head = format!("{} {} RSS = {:.1}%{of}", b.program, crate::fmt::bytes(b.rss), share);
    let tabs = b.parts.iter().find(|p| p.part == Part::Tab);
    let tail = tabs.map_or(String::new(), |t| format!(" · {} tabs hold {}", t.count, crate::fmt::bytes(t.rss)));
    let text = match (before.0, level) {
        (_, Severity::Crit) => format!("{head} (≥ {crit:.0}%) · {} processes{tail}", b.count),
        (Severity::Crit, Severity::Warn) => format!("{head} · under {crit:.0}% again, still ≥ {warn:.0}%"),
        (_, Severity::Warn) => format!("{head} (≥ {warn:.0}%) · {} processes{tail}", b.count),
        (Severity::Warn | Severity::Crit, _) => format!("{head} · under {warn:.0}% again after {}", crate::fmt::dur(at - before.1)),
        // From nothing known to fine: nothing to say.
        _ => return None,
    };
    let shown = if level >= Severity::Warn { level } else { Severity::Ok };
    Some(event(shown, text, at))
}

/// A browser that quit while it was held greedy.
pub fn quit_line(program: &str, at: f64) -> crate::tape::Event {
    event(Severity::Ok, format!("{program} quit · what it held is given back"), at)
}

/// A stop the owner asked for, and what came of it.
pub fn stop_line(stop: &crate::app::Stop, result: &Result<String, String>, at: f64) -> crate::tape::Event {
    let what = format!("{} {} process {}", short(&stop.program), helper_word(stop.helper.part), stop.helper.pid);
    let how = match stop.asked {
        crate::app::Asked::Screen => "asked at the screen",
        crate::app::Asked::Phone { .. } => "asked on the phone",
    };
    match result {
        Ok(_) => event(Severity::Info, format!("stopped {what} · {} · {how}", crate::fmt::bytes(stop.helper.rss)), at),
        Err(e) => event(Severity::Warn, format!("not stopped: {what} — {e}"), at),
    }
}

fn event(level: Severity, text: String, at: f64) -> crate::tape::Event {
    crate::tape::Event { at, level, kind: crate::tape::Kind::Local, parts: vec![(text, crate::insight::Tone::Plain)], subject: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(window: u32, url: &str) -> Tab {
        Tab { window, title: url.to_string(), url: url.to_string(), active: false }
    }

    #[test]
    fn tabs_are_grouped_by_site_most_first() {
        let tabs = [tab(1, "https://github.com/a"), tab(1, "https://docs.google.com/1"), tab(2, "https://github.com/b"), tab(2, "chrome://newtab/")];
        let read = TabsRead::new("Google Chrome".into(), 0.0, Ok(tabs.to_vec()));
        let hosts: Vec<(&str, usize)> = read.sites.iter().map(|s| (s.host.as_str(), s.tabs.len())).collect();
        assert_eq!(hosts, [("github.com", 2), ("chrome://newtab", 1), ("docs.google.com", 1)]);
        assert_eq!(read.windows(), 2);
        assert!(TabsRead::new("Google Chrome".into(), 0.0, Err("no".into())).sites.is_empty());
    }

    #[test]
    fn a_greedy_chrome_is_one_tape_line_after_two_reads() {
        let mut app = crate::app::App::new();
        // The fake Chrome holds 11.5% of 32 GiB: over lines of 5 and 10.
        app.chrome_lines = Some((5.0, 10.0));
        let local = |app: &mut crate::app::App, step: u64| app.update(crate::app::Event::Local(Box::new(crate::fake::local(step, 1000.0 + 2.0 * step as f64))));
        local(&mut app, 1);
        assert_eq!(app.tape.len(), 0, "one read is not enough");
        local(&mut app, 2);
        let line: String = app.tape.newest_first().next().expect("a line").parts.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(line, "Google Chrome 3.7 GiB RSS = 11.5% of 32 GiB (≥ 10%) · 8 processes · 3 tabs hold 2.2 GiB");
        assert_eq!(app.greed_of("Google Chrome"), Severity::Crit);
        local(&mut app, 3);
        assert_eq!(app.tape.len(), 1, "not one a read");
    }

    #[test]
    fn the_share_has_two_lines_and_can_be_off() {
        let lines = Some((WARN_PCT, CRIT_PCT));
        assert_eq!(share_severity(Some(39.9), lines), Severity::Ok);
        assert_eq!(share_severity(Some(40.0), lines), Severity::Warn);
        assert_eq!(share_severity(Some(60.0), lines), Severity::Crit);
        assert_eq!(share_severity(None, lines), Severity::None);
        assert_eq!(share_severity(Some(90.0), None), Severity::None);
    }

    #[test]
    fn a_greedy_browser_is_one_change_not_one_a_read() {
        let lines = (WARN_PCT, CRIT_PCT);
        let mut g = Greed::default();
        assert_eq!(g.observe(30.0, lines, 0.0), None);
        assert_eq!(g.observe(30.0, lines, 2.0), Some(Severity::Ok));
        assert_eq!(g.observe(40.1, lines, 4.0), None, "one read over is not enough");
        assert_eq!(g.observe(40.1, lines, 6.0), Some(Severity::Warn));
        assert_eq!(g.since, 6.0);
        // Hovering just under the line holds it…
        assert_eq!(g.observe(38.0, lines, 8.0), None);
        assert_eq!(g.observe(38.0, lines, 10.0), None);
        // …3 points under clears it, after two reads.
        assert_eq!(g.observe(36.0, lines, 12.0), None);
        assert_eq!(g.observe(36.0, lines, 14.0), Some(Severity::Ok));
    }
}
