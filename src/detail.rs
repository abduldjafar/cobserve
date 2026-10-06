//! What ⏎ opens on views 5 and 6: a page of detail, read when asked — a ticket in full, a DAG's
//! runs, a run's tasks, a task's log — instead of a page in the browser.
//!
//! Pure: what was asked, what came back, and where the cursor and the scroll are. The sources
//! answer the asks (`sources/jira.rs`, `sources/airflow.rs`); `ui/detail.rs` draws the pages.

use crate::airflow::{Run, TaskRun};
use crate::jira::IssueDetail;

/// What a page shows, and so what is asked for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ask {
    /// A Jira ticket in full.
    Issue(String),
    /// A DAG's latest runs.
    Runs(String),
    /// A run's task instances.
    Tasks { dag: String, run: String },
    /// One try of a task's log.
    Log { dag: String, run: String, task: String, map_index: i64, attempt: u32 },
}

/// What came back.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Issue(Box<IssueDetail>),
    Runs(Vec<Run>),
    Tasks(Vec<TaskRun>),
    Log(Vec<String>),
}

/// The most lines of a log that are kept: its end, where a failure says why.
pub const LOG_LINES: usize = 3000;

/// A log as Airflow sends it, as lines: its last [`LOG_LINES`], tabs as spaces, carriage returns
/// and terminal escapes out.
pub fn log_lines(text: &str) -> Vec<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\u{1b}' => {
                        // An escape sequence: up to and with its final letter.
                        for next in chars.by_ref() {
                            if next.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    '\t' => out.push_str("    "),
                    c if c.is_control() => {}
                    c => out.push(c),
                }
            }
            out
        })
        .collect();
    let skip = lines.len().saturating_sub(LOG_LINES);
    lines.into_iter().skip(skip).collect()
}

/// A page open on a view: what it is, what came back, the cursor (a list) or the scroll (text).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub ask: Ask,
    /// `None` while it is read.
    pub body: Option<Result<Body, String>>,
    pub cursor: usize,
    /// The first line shown; `usize::MAX` for the end, where a log opens.
    pub scroll: usize,
}

impl Page {
    pub fn new(ask: Ask) -> Page {
        let scroll = if matches!(ask, Ask::Log { .. }) { usize::MAX } else { 0 };
        Page { ask, body: None, cursor: 0, scroll }
    }

    /// How many rows the cursor walks: runs or tasks; 0 for text.
    pub fn rows(&self) -> usize {
        match &self.body {
            Some(Ok(Body::Runs(runs))) => runs.len(),
            Some(Ok(Body::Tasks(tasks))) => tasks.len(),
            _ => 0,
        }
    }

    /// ↑ ↓ and the rest: on a list the cursor moves, on text the scroll does. A log is scrolled
    /// from its end (`usize::MAX - n` is n lines above it), since its length on screen is the
    /// screen's to know.
    pub fn step(&mut self, delta: isize) {
        let rows = self.rows();
        if rows > 0 {
            self.cursor = (self.cursor as isize + delta).clamp(0, rows as isize - 1) as usize;
        } else if self.scroll > usize::MAX / 2 {
            let up = (usize::MAX - self.scroll) as isize - delta;
            self.scroll = usize::MAX - up.max(0) as usize;
        } else {
            self.scroll = (self.scroll as isize + delta).max(0) as usize;
        }
    }

    /// The run under the cursor of a DAG's runs.
    pub fn run_at_cursor(&self) -> Option<&Run> {
        match &self.body {
            Some(Ok(Body::Runs(runs))) => runs.get(self.cursor),
            _ => None,
        }
    }

    /// The task under the cursor of a run's tasks.
    pub fn task_at_cursor(&self) -> Option<&TaskRun> {
        match &self.body {
            Some(Ok(Body::Tasks(tasks))) => tasks.get(self.cursor),
            _ => None,
        }
    }
}

/// Where a page starts on screen: `scroll` of `len` lines in a window of `height`, the end
/// marker (`usize::MAX`, and steps up from it) resolved.
pub fn first_line(scroll: usize, len: usize, height: usize) -> usize {
    let last = len.saturating_sub(height);
    if scroll > usize::MAX / 2 {
        let up = usize::MAX - scroll;
        last.saturating_sub(up)
    } else {
        scroll.min(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_keeps_its_end_without_escapes() {
        let text = "a\tb\r\n\u{1b}[31mred\u{1b}[0m\nlast";
        assert_eq!(log_lines(text), ["a    b", "red", "last"]);
        let long: String = (0..LOG_LINES + 5).map(|i| format!("{i}\n")).collect();
        let lines = log_lines(&long);
        assert_eq!((lines.len(), lines[0].as_str()), (LOG_LINES, "5"));
    }

    #[test]
    fn a_log_opens_at_its_end_and_steps_up_from_there() {
        let mut page = Page::new(Ask::Log { dag: "d".into(), run: "r".into(), task: "t".into(), map_index: -1, attempt: 1 });
        page.body = Some(Ok(Body::Log((0..100).map(|i| i.to_string()).collect())));
        assert_eq!(first_line(page.scroll, 100, 20), 80);
        page.step(-1);
        assert_eq!(first_line(page.scroll, 100, 20), 79);
        page.step(-10);
        assert_eq!(first_line(page.scroll, 100, 20), 69);
        assert_eq!(first_line(0, 100, 20), 0);
        assert_eq!(first_line(500, 100, 20), 80, "past the end, the end");
    }

    #[test]
    fn on_a_list_the_cursor_moves_and_stays_inside() {
        let mut page = Page::new(Ask::Tasks { dag: "d".into(), run: "r".into() });
        page.body = Some(Ok(Body::Tasks(vec![TaskRun::default(), TaskRun::default(), TaskRun::default()])));
        page.step(1);
        page.step(5);
        assert_eq!(page.cursor, 2);
        page.step(-9);
        assert_eq!(page.cursor, 0);
    }
}
