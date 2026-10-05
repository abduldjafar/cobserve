//! View 5's query sessions: SQL typed here and run on one of the fleet's servers — read-only,
//! with limits — and what came back, as a table; suggestions as it is typed (`complete.rs`),
//! and Claude Code or OpenCode to write it when asked (`assist.rs`).
//!
//! This is the session's state, pure like the rest of `App`: the text and its cursor, what ran
//! before, the server it goes to, the run under way and the last answer, the suggestions open,
//! the helper's turn. The work is `main.rs`'s — a query to `sources/clickhouse.rs` (`fake.rs`
//! under `FAKE=1`), a helper's program — which takes what a session asks for once
//! (`take_request`, `take_ask`) and what it stops (`take_cancel`, `take_ask_cancel`).
//!
//! The keys are a SQL prompt's: `⏎` runs a statement that ends with `;` and otherwise starts a
//! new line (as `clickhouse-client` does), `ctrl+r` runs whatever is there, `↑` on the first
//! line goes back through what ran before, `tab` takes a suggestion, `ctrl+g` asks the helper
//! to write it, `ctrl+z` puts back what was there, `ctrl+c` stops a query or clears the text,
//! `shift+tab` goes to the answer — where the arrows move through it — and `ctrl+o` changes
//! the server.

use crate::complete::{self, Schema, Suggestion};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a query may take, as the session says it on screen; `sources/clickhouse.rs` sets them.
pub const TIME_LIMIT_S: u64 = 30;
pub const ROW_LIMIT: usize = 1000;
/// How much of what ran before is kept.
pub const HISTORY: usize = 100;

/// A query session.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Console {
    /// The server queries go to, by its name in the fleet.
    pub node: Option<String>,
    /// The SQL, a line per entry, and the cursor: a line and a character in it.
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    /// What ran, oldest first, and how far `↑` has gone back in it — with the text as it was
    /// before, for `↓` to come back to.
    pub history: Vec<String>,
    browsing: Option<usize>,
    draft: Option<Vec<String>>,
    pub state: RunState,
    /// The last answer, kept on screen while the next query runs.
    pub answer: Option<Answer>,
    pub focus: Focus,
    /// The first row and the first column of the answer on screen.
    pub scroll_row: usize,
    pub scroll_col: usize,
    /// The list of servers is open, with its cursor.
    pub choosing: Option<usize>,
    request: Option<Request>,
    cancel: Option<u64>,
    runs: u64,
    /// The suggestions open for the word at the cursor.
    pub suggest: Option<Suggest>,
    /// Where they were waved away (`esc`) — a line and a word's start — not to open again there.
    hush: Option<(usize, usize)>,
    /// Who writes the SQL when asked, and how that is going.
    pub assistant: Assistant,
    pub assisting: Assisting,
    ask: Option<Ask>,
    ask_cancel: Option<u64>,
    asks: u64,
    /// The text before its last change of all of it — a helper's answer written in, a clear —
    /// for `ctrl+z` to put back.
    undo: Option<Vec<String>>,
    /// What `y` or `Y` copied from the answer, for `main.rs` to put on the clipboard, once.
    clipboard: Option<(String, String)>,
}

/// Suggestions open under the cursor: for the word at `from` on line `row`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggest {
    pub row: usize,
    pub from: usize,
    pub items: Vec<Suggestion>,
    /// The one `tab` takes.
    pub at: usize,
    /// Chosen with ↑ ↓: `⏎` takes it too.
    pub moved: bool,
}

/// The program that writes SQL when asked: Claude Code, signed in with a Pro or Max plan, or
/// OpenCode, signed in as it is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Assistant {
    #[default]
    Claude,
    OpenCode,
}

impl Assistant {
    pub fn name(self) -> &'static str {
        match self {
            Assistant::Claude => "Claude",
            Assistant::OpenCode => "OpenCode",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Assistant::Claude => "✻",
            Assistant::OpenCode => "▣",
        }
    }

    pub fn other(self) -> Assistant {
        match self {
            Assistant::Claude => Assistant::OpenCode,
            Assistant::OpenCode => Assistant::Claude,
        }
    }
}

/// How the helper's turn is going.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Assisting {
    #[default]
    Idle,
    /// Asked at `since`, Unix seconds.
    Asking { id: u64, since: i64 },
    /// The text is what it wrote, not changed or run yet.
    Wrote,
    /// It could not — or there was nothing to ask it.
    Failed(String),
}

/// What to ask a helper, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub id: u64,
    pub assistant: Assistant,
    pub node: Option<String>,
    /// The text in the session.
    pub sql: String,
    /// What the server said, when the text is what ran last and it said no.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum RunState {
    #[default]
    Idle,
    /// Sent to `node` at `since` (Unix seconds).
    Running { id: u64, node: String, since: i64 },
    /// The server said no, or could not be reached.
    Failed { node: String, error: String },
    /// Stopped with `ctrl+c` before an answer came.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Editor,
    Answer,
}

/// A query to send, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub node: String,
    pub sql: String,
}

/// What a query answered.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Answer {
    pub node: String,
    /// Each column's name and type.
    pub columns: Vec<(String, String)>,
    /// The rows as text; `None` is NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// The time the answer took, as measured here.
    pub elapsed_ms: u64,
    /// What the server read for it, when it says.
    pub read_rows: Option<u64>,
    pub read_bytes: Option<u64>,
    /// There were more rows than the session keeps.
    pub cut: bool,
    /// The answer in a format of the query's own (`FORMAT Pretty`), as it came.
    pub text: Option<String>,
}

/// What a key did that the session around the console has to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Handled,
    /// Not the console's: the key goes on to the session's own handling.
    Ignored,
}

impl Console {
    pub fn new(node: Option<String>) -> Self {
        Console { node, lines: vec![String::new()], ..Console::default() }
    }

    /// The SQL as one text.
    pub fn sql(&self) -> String {
        self.lines.join("\n")
    }

    /// The text replaced, the cursor at its end.
    pub fn set_sql(&mut self, sql: &str) {
        self.lines = sql.split('\n').map(str::to_string).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].chars().count();
    }

    pub fn running(&self) -> bool {
        matches!(self.state, RunState::Running { .. })
    }

    /// What `main.rs` should send, once.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    #[cfg(test)]
    pub fn take_request_peek(&self) -> Option<&Request> {
        self.request.as_ref()
    }

    /// The run `main.rs` should stop, once.
    pub fn take_cancel(&mut self) -> Option<u64> {
        self.cancel.take()
    }

    /// What was copied — the text, and what it is — once.
    pub fn take_clipboard(&mut self) -> Option<(String, String)> {
        self.clipboard.take()
    }

    /// The row under the answer's cursor, or the whole answer with its column names, as
    /// tab-separated text.
    fn copy(&mut self, all: bool) {
        let Some(answer) = &self.answer else {
            return;
        };
        let line = |row: &[Option<String>]| -> String {
            row.iter().map(|v| v.as_deref().unwrap_or("NULL").replace(['\t', '\n'], " ")).collect::<Vec<_>>().join("\t")
        };
        let (text, what) = if all || answer.columns.is_empty() {
            let mut lines = vec![answer.columns.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("\t")];
            lines.extend(answer.rows.iter().map(|row| line(row)));
            if answer.columns.is_empty() {
                (answer.text.clone().unwrap_or_default(), "the answer".to_string())
            } else {
                (lines.join("\n"), format!("{} with their names", if answer.rows.len() == 1 { "1 row".to_string() } else { format!("{} rows", answer.rows.len()) }))
            }
        } else {
            match answer.rows.get(self.scroll_row) {
                Some(row) => (line(row), format!("row {}", self.scroll_row + 1)),
                None => return,
            }
        };
        self.clipboard = Some((text, what));
    }

    /// What `main.rs` should ask a helper, once.
    pub fn take_ask(&mut self) -> Option<Ask> {
        self.ask.take()
    }

    /// The question `main.rs` should withdraw — its program ended — once.
    pub fn take_ask_cancel(&mut self) -> Option<u64> {
        self.ask_cancel.take()
    }

    pub fn asking(&self) -> bool {
        matches!(self.assisting, Assisting::Asking { .. })
    }

    /// Ask the helper to write what the text asks for — its `--` comments — or to put right
    /// what failed. What it writes takes the text's place; nothing runs until asked.
    pub fn ask_for_sql(&mut self, now: i64) {
        if self.asking() {
            return;
        }
        let sql = self.sql();
        if sql.trim().is_empty() {
            self.assisting = Assisting::Failed("say what the query should do after --, then ctrl+k".into());
            return;
        }
        let error = match &self.state {
            RunState::Failed { error, .. } if self.history.last().map(|h| h.trim()) == Some(sql.trim()) => Some(error.clone()),
            _ => None,
        };
        self.asks += 1;
        self.ask = Some(Ask { id: self.asks, assistant: self.assistant, node: self.node.clone(), sql, error });
        self.assisting = Assisting::Asking { id: self.asks, since: now };
        self.suggest = None;
    }

    /// The question withdrawn.
    pub fn stop_asking(&mut self) {
        if let Assisting::Asking { id, .. } = self.assisting {
            self.ask_cancel = Some(id);
            self.assisting = Assisting::Idle;
        }
    }

    /// The helper's answer, taken when it is for the question asked: SQL in the text's place —
    /// `ctrl+z` puts the text back — or why not.
    pub fn assisted(&mut self, id: u64, result: Result<String, String>) {
        if !matches!(self.assisting, Assisting::Asking { id: asked, .. } if asked == id) {
            return;
        }
        match result {
            Ok(sql) => {
                self.undo = Some(self.lines.clone());
                self.set_sql(&sql);
                self.focus = Focus::Editor;
                self.suggest = None;
                self.assisting = Assisting::Wrote;
            }
            Err(error) => self.assisting = Assisting::Failed(error),
        }
    }

    /// The text before its last change of all of it, back — and what it was, kept to come back
    /// to with another `ctrl+z`.
    pub fn undo(&mut self) {
        if let Some(before) = self.undo.take() {
            let now = std::mem::replace(&mut self.lines, if before.is_empty() { vec![String::new()] } else { before });
            self.undo = Some(now);
            self.row = self.lines.len() - 1;
            self.col = self.line_len();
            self.suggest = None;
            self.edited();
        }
    }

    /// The text cleared, to come back with `ctrl+z`.
    fn clear(&mut self) {
        if !self.sql().is_empty() {
            self.undo = Some(self.lines.clone());
        }
        self.set_sql("");
        self.edited();
    }

    /// The text changed by hand: what the helper said of it no longer holds.
    fn edited(&mut self) {
        if matches!(self.assisting, Assisting::Wrote | Assisting::Failed(_)) {
            self.assisting = Assisting::Idle;
        }
    }

    /// Run what is typed on the server chosen.
    pub fn run(&mut self, now: i64) {
        let sql = self.sql();
        let sql = sql.trim().trim_end_matches(';').trim().to_string();
        if sql.is_empty() || self.running() {
            return;
        }
        self.suggest = None;
        self.edited();
        let Some(node) = self.node.clone() else {
            self.state = RunState::Failed { node: String::new(), error: "no server to run it on — ctrl+o chooses one".into() };
            return;
        };
        self.runs += 1;
        let id = self.runs;
        self.request = Some(Request { id, node: node.clone(), sql });
        self.state = RunState::Running { id, node, since: now };
        let text = self.sql().trim().to_string();
        if self.history.last() != Some(&text) {
            self.history.push(text);
            let over = self.history.len().saturating_sub(HISTORY);
            self.history.drain(..over);
        }
        self.browsing = None;
        self.draft = None;
    }

    /// The query under way, stopped: the server is told by the request going away.
    pub fn stop(&mut self) {
        if let RunState::Running { id, .. } = self.state {
            self.cancel = Some(id);
            self.state = RunState::Stopped;
        }
    }

    /// An answer, taken when it is for the run under way.
    pub fn answered(&mut self, id: u64, result: Result<Answer, String>) {
        let RunState::Running { id: running, node, .. } = &self.state else {
            return;
        };
        if *running != id {
            return;
        }
        match result {
            Ok(answer) => {
                self.answer = Some(answer);
                self.state = RunState::Idle;
                self.scroll_row = 0;
                self.scroll_col = 0;
            }
            Err(error) => self.state = RunState::Failed { node: node.clone(), error },
        }
    }

    /// Queries go to `node` from now on.
    pub fn connect(&mut self, node: &str) {
        self.node = Some(node.to_string());
        self.choosing = None;
        if matches!(self.state, RunState::Failed { .. }) {
            self.state = RunState::Idle;
        }
    }

    /// A key, with the fleet's servers for the list of them, `now` for a run's start, and what
    /// the server has for the suggestions.
    pub fn key(&mut self, key: &KeyEvent, servers: &[String], now: i64, schema: Option<&Schema>) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(at) = self.choosing {
            match key.code {
                KeyCode::Up => self.choosing = Some(at.saturating_sub(1)),
                KeyCode::Down => self.choosing = Some((at + 1).min(servers.len().saturating_sub(1))),
                KeyCode::Enter => {
                    if let Some(node) = servers.get(at) {
                        self.connect(node);
                    }
                }
                KeyCode::Esc => self.choosing = None,
                KeyCode::Char('o') if ctrl => self.choosing = None,
                _ => return Outcome::Handled,
            }
            return Outcome::Handled;
        }
        if self.focus == Focus::Editor && self.suggestion_key(key) {
            return Outcome::Handled;
        }
        let was_open = self.suggest.take().is_some();
        let outcome = self.edit_key(key, servers, now, schema);
        // The suggestions follow the word as it is typed.
        if self.focus == Focus::Editor {
            match key.code {
                KeyCode::Char(c) if !ctrl && (c.is_alphanumeric() || c == '_' || c == '.') => self.refresh(schema, false),
                KeyCode::Backspace | KeyCode::Delete if was_open => self.refresh(schema, false),
                KeyCode::Char(' ') if ctrl => {
                    self.hush = None;
                    self.refresh(schema, true);
                }
                _ => {}
            }
        }
        outcome
    }

    /// A key for the suggestions open, if it is theirs: `tab` takes one — `⏎` too once one was
    /// chosen — ↑ ↓ choose, `esc` closes them.
    fn suggestion_key(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(open) = self.suggest.as_mut() else {
            return false;
        };
        let len = open.items.len().max(1);
        match key.code {
            KeyCode::Tab => self.accept(),
            KeyCode::Enter if open.moved => self.accept(),
            KeyCode::Down => (open.at, open.moved) = ((open.at + 1) % len, true),
            KeyCode::Up => (open.at, open.moved) = ((open.at + len - 1) % len, true),
            KeyCode::Char('n') if ctrl => (open.at, open.moved) = ((open.at + 1) % len, true),
            KeyCode::Char('p') if ctrl => (open.at, open.moved) = ((open.at + len - 1) % len, true),
            KeyCode::PageDown => (open.at, open.moved) = ((open.at + 8).min(len - 1), true),
            KeyCode::PageUp => (open.at, open.moved) = (open.at.saturating_sub(8), true),
            KeyCode::Esc => {
                self.hush = Some((open.row, open.from));
                self.suggest = None;
            }
            _ => return false,
        }
        true
    }

    /// The suggestion chosen, in place of the word typed so far — a function with the cursor
    /// in its parentheses.
    pub fn accept(&mut self) {
        let Some(open) = self.suggest.take() else {
            return;
        };
        let Some(item) = open.items.get(open.at) else {
            return;
        };
        if open.row != self.row || open.from > self.col {
            return;
        }
        let (start, end) = (self.byte_at(open.from), self.byte_at(self.col));
        self.lines[self.row].replace_range(start..end, &item.text);
        self.col = open.from + item.text.chars().count().saturating_sub(item.back);
        self.browsing = None;
        self.edited();
    }

    /// The suggestions for the word at the cursor, now: none where there are none, nor where
    /// they were waved away — unless `forced`.
    pub fn refresh(&mut self, schema: Option<&Schema>, forced: bool) {
        let found = complete::complete(&self.lines, self.row, self.col, schema, forced);
        if let Some(found) = &found
            && self.hush.is_some_and(|hush| hush != (self.row, found.from))
        {
            self.hush = None;
        }
        self.suggest = found
            .filter(|found| forced || self.hush != Some((self.row, found.from)))
            .map(|found| Suggest { row: self.row, from: found.from, items: found.items, at: 0, moved: false });
    }

    /// `tab` with no suggestions open: the only one taken, several shown, or an indent. After
    /// `FROM ` and the like, with nothing typed yet, it lists what can go there.
    fn tab(&mut self, schema: Option<&Schema>) {
        let names = |what: complete::What| matches!(what, complete::What::Table | complete::What::Database | complete::What::Format);
        let found = complete::complete(&self.lines, self.row, self.col, schema, false)
            .or_else(|| complete::complete(&self.lines, self.row, self.col, schema, true).filter(|found| found.items.iter().all(|i| names(i.what))));
        match found {
            Some(found) if found.items.len() == 1 => {
                self.suggest = Some(Suggest { row: self.row, from: found.from, items: found.items, at: 0, moved: false });
                self.accept();
            }
            Some(found) => self.suggest = Some(Suggest { row: self.row, from: found.from, items: found.items, at: 0, moved: false }),
            None => {
                self.insert(' ');
                self.insert(' ');
            }
        }
    }

    fn edit_key(&mut self, key: &KeyEvent, servers: &[String], now: i64, schema: Option<&Schema>) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Char('o'), true) => {
                let at = self.node.as_ref().and_then(|n| servers.iter().position(|s| s == n)).unwrap_or(0);
                self.choosing = Some(at);
            }
            (KeyCode::Char('r'), true) => self.run(now),
            // ctrl+k — ctrl+g too, where nothing outside the terminal takes it first.
            (KeyCode::Char('k' | 'g'), true) => self.ask_for_sql(now),
            (KeyCode::Char('t'), true) => self.assistant = self.assistant.other(),
            (KeyCode::Char('z'), true) => self.undo(),
            (KeyCode::Char('c'), true) => {
                if self.running() {
                    self.stop();
                } else if self.asking() {
                    self.stop_asking();
                } else {
                    self.clear();
                    self.focus = Focus::Editor;
                }
            }
            (KeyCode::Char(' '), true) => {}
            (KeyCode::BackTab, _) if self.focus == Focus::Editor => {
                if self.answer.is_some() {
                    self.focus = Focus::Answer;
                }
            }
            (KeyCode::Tab, _) if self.focus == Focus::Editor => self.tab(schema),
            (KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc, _) if self.focus == Focus::Answer => self.focus = Focus::Editor,
            (KeyCode::PageUp, _) => self.scroll(-10, 0),
            (KeyCode::PageDown, _) => self.scroll(10, 0),
            _ if self.focus == Focus::Answer => match key.code {
                KeyCode::Up => self.scroll(-1, 0),
                KeyCode::Down => self.scroll(1, 0),
                KeyCode::Left => self.scroll(0, -1),
                KeyCode::Right => self.scroll(0, 1),
                KeyCode::Home => self.scroll_row = 0,
                KeyCode::End => self.scroll(isize::MAX / 2, 0),
                KeyCode::Char('y') if !ctrl => self.copy(false),
                KeyCode::Char('Y') if !ctrl => self.copy(true),
                // Typing goes back to the text, and is typed.
                KeyCode::Char(c) if !ctrl => {
                    self.focus = Focus::Editor;
                    self.type_char(c);
                }
                _ => {}
            },
            (KeyCode::Enter, _) => {
                if self.sql().trim_end().ends_with(';') {
                    self.run(now);
                } else {
                    self.newline();
                }
            }
            (KeyCode::Char('j'), true) => self.newline(),
            (KeyCode::Char('u'), true) => self.clear(),
            (KeyCode::Char('a'), true) | (KeyCode::Home, _) => self.col = 0,
            (KeyCode::Char('e'), true) | (KeyCode::End, _) => self.col = self.line_len(),
            (KeyCode::Char(c), false) => self.type_char(c),
            (KeyCode::Backspace, _) => self.backspace(),
            (KeyCode::Delete, _) => self.delete(),
            (KeyCode::Left, _) => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.line_len();
                }
            }
            (KeyCode::Right, _) => {
                if self.col < self.line_len() {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            (KeyCode::Up, _) => {
                if self.row == 0 {
                    self.back_in_history();
                } else {
                    self.row -= 1;
                    self.col = self.col.min(self.line_len());
                }
            }
            (KeyCode::Down, _) => {
                if self.row + 1 >= self.lines.len() {
                    self.on_in_history();
                } else {
                    self.row += 1;
                    self.col = self.col.min(self.line_len());
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Handled
    }

    /// Pasted text, typed where the cursor is — a newline a new line.
    pub fn paste(&mut self, text: &str) {
        self.focus = Focus::Editor;
        for c in text.chars() {
            match c {
                '\n' => self.newline(),
                '\r' => {}
                '\t' => {
                    self.insert(' ');
                    self.insert(' ');
                }
                c if !c.is_control() => self.insert(c),
                _ => {}
            }
        }
    }

    /// The answer moved by rows and columns, within it.
    pub fn scroll(&mut self, rows: isize, cols: isize) {
        let Some(answer) = &self.answer else {
            return;
        };
        let last_row = answer.rows.len().saturating_sub(1);
        let last_col = answer.columns.len().saturating_sub(1);
        self.scroll_row = self.scroll_row.saturating_add_signed(rows).min(last_row);
        self.scroll_col = self.scroll_col.saturating_add_signed(cols).min(last_col);
    }

    fn line_len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    fn byte_at(&self, col: usize) -> usize {
        let line = &self.lines[self.row];
        line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
    }

    /// A character typed: a `)` before one already there steps over it, as after a function
    /// a suggestion opened.
    fn type_char(&mut self, c: char) {
        if c == ')' && self.lines[self.row].chars().nth(self.col) == Some(')') {
            self.col += 1;
            return;
        }
        self.insert(c);
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.col);
        self.lines[self.row].insert(at, c);
        self.col += 1;
        self.browsing = None;
        self.edited();
    }

    fn newline(&mut self) {
        let at = self.byte_at(self.col);
        let rest = self.lines[self.row].split_off(at);
        // The new line starts as indented as this one.
        let indent: String = self.lines[self.row].chars().take_while(|c| *c == ' ').collect();
        self.row += 1;
        self.col = indent.chars().count();
        self.lines.insert(self.row, format!("{indent}{rest}"));
        self.browsing = None;
        self.edited();
    }

    fn backspace(&mut self) {
        self.edited();
        if self.col > 0 {
            self.col -= 1;
            let at = self.byte_at(self.col);
            self.lines[self.row].remove(at);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len();
            self.lines[self.row].push_str(&line);
        }
    }

    fn delete(&mut self) {
        self.edited();
        if self.col < self.line_len() {
            let at = self.byte_at(self.col);
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn back_in_history(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let at = match self.browsing {
            None => {
                self.draft = Some(self.lines.clone());
                self.history.len() - 1
            }
            Some(0) => return,
            Some(at) => at - 1,
        };
        self.browsing = Some(at);
        let text = self.history[at].clone();
        self.set_sql(&text);
        self.browsing = Some(at);
        self.row = 0;
        self.col = self.col.min(self.line_len());
    }

    fn on_in_history(&mut self) {
        let Some(at) = self.browsing else {
            return;
        };
        if at + 1 < self.history.len() {
            let text = self.history[at + 1].clone();
            self.set_sql(&text);
            self.browsing = Some(at + 1);
        } else {
            let draft = self.draft.take().unwrap_or_else(|| vec![String::new()]);
            self.lines = draft;
            self.row = self.lines.len() - 1;
            self.col = self.line_len();
            self.browsing = None;
        }
    }
}

/// A value as a cell shows it: numbers right, text left — `true` for numbers.
pub fn is_number(kind: &str) -> bool {
    let kind = kind.trim_start_matches("Nullable(").trim_start_matches("LowCardinality(");
    ["Int", "UInt", "Float", "Decimal"].iter().any(|prefix| kind.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(console: &mut Console, text: &str) {
        for c in text.chars() {
            console.key(&key(KeyCode::Char(c)), &[], 0, None);
        }
    }

    #[test]
    fn enter_runs_a_statement_that_ends_with_a_semicolon_and_otherwise_starts_a_line() {
        let mut console = Console::new(Some("clickhouse3".into()));
        typed(&mut console, "SELECT user");
        console.key(&key(KeyCode::Enter), &[], 100, None);
        assert_eq!(console.lines, ["SELECT user", ""]);
        assert!(console.take_request().is_none());
        typed(&mut console, "FROM system.processes;");
        console.key(&key(KeyCode::Enter), &[], 100, None);
        let request = console.take_request().expect("sent");
        assert_eq!((request.node.as_str(), request.sql.as_str()), ("clickhouse3", "SELECT user\nFROM system.processes"));
        assert!(matches!(console.state, RunState::Running { since: 100, .. }));
        assert!(console.take_request().is_none(), "once");
        assert_eq!(console.history, ["SELECT user\nFROM system.processes;"]);

        // Its answer, and an answer for an older run, which is not taken.
        let answer = Answer { node: "clickhouse3".into(), columns: vec![("user".into(), "String".into())], ..Answer::default() };
        console.answered(request.id + 1, Ok(answer.clone()));
        assert!(console.running());
        console.answered(request.id, Ok(answer));
        assert_eq!(console.state, RunState::Idle);
        assert_eq!(console.answer.as_ref().unwrap().columns[0].0, "user");
    }

    #[test]
    fn ctrl_r_runs_what_is_there_and_ctrl_c_stops_it_or_clears() {
        let mut console = Console::new(Some("ch1".into()));
        typed(&mut console, "SELECT 1");
        console.key(&ctrl('r'), &[], 5, None);
        let request = console.take_request().unwrap();
        assert_eq!(request.sql, "SELECT 1");
        console.key(&ctrl('c'), &[], 6, None);
        assert_eq!(console.state, RunState::Stopped);
        assert_eq!(console.take_cancel(), Some(request.id), "the request goes away");
        console.answered(request.id, Ok(Answer::default()));
        assert!(console.answer.is_none(), "a stopped run's answer is not taken");
        console.key(&ctrl('c'), &[], 7, None);
        assert_eq!(console.lines, [""], "idle, ctrl+c clears");
        // No server: said, not sent.
        let mut nowhere = Console::new(None);
        typed(&mut nowhere, "SELECT 1;");
        nowhere.key(&key(KeyCode::Enter), &[], 0, None);
        assert!(nowhere.take_request().is_none());
        assert!(matches!(&nowhere.state, RunState::Failed { error, .. } if error.contains("ctrl+o")));
    }

    #[test]
    fn up_on_the_first_line_goes_back_through_what_ran_and_down_comes_back() {
        let mut console = Console::new(Some("ch1".into()));
        for sql in ["SELECT 1;", "SELECT 2;"] {
            typed(&mut console, sql);
            console.key(&key(KeyCode::Enter), &[], 0, None);
            let id = console.take_request().unwrap().id;
            console.answered(id, Ok(Answer::default()));
            console.set_sql("");
        }
        typed(&mut console, "SEL");
        // With suggestions open the arrows are theirs; once they are closed, the history's.
        assert!(console.suggest.is_some());
        console.key(&key(KeyCode::Esc), &[], 0, None);
        console.key(&key(KeyCode::Up), &[], 0, None);
        assert_eq!(console.sql(), "SELECT 2;");
        console.key(&key(KeyCode::Up), &[], 0, None);
        assert_eq!(console.sql(), "SELECT 1;");
        console.key(&key(KeyCode::Up), &[], 0, None);
        assert_eq!(console.sql(), "SELECT 1;", "the oldest stays");
        console.key(&key(KeyCode::Down), &[], 0, None);
        console.key(&key(KeyCode::Down), &[], 0, None);
        assert_eq!(console.sql(), "SEL", "back to what was being typed");
    }

    #[test]
    fn editing_works_across_lines_and_characters_of_any_width() {
        let mut console = Console::new(None);
        typed(&mut console, "  WHERE x = 'é'");
        console.key(&key(KeyCode::Enter), &[], 0, None);
        assert_eq!(console.lines, ["  WHERE x = 'é'", "  "], "the new line keeps the indent");
        console.key(&key(KeyCode::Backspace), &[], 0, None);
        console.key(&key(KeyCode::Backspace), &[], 0, None);
        console.key(&key(KeyCode::Backspace), &[], 0, None);
        assert_eq!(console.lines, ["  WHERE x = 'é'"], "joined back");
        console.key(&key(KeyCode::Left), &[], 0, None);
        console.key(&key(KeyCode::Left), &[], 0, None);
        console.key(&key(KeyCode::Delete), &[], 0, None);
        assert_eq!(console.lines, ["  WHERE x = ''"]);
        console.paste("\nAND y = 1\r\n\tLIMIT 5");
        assert_eq!(console.lines, ["  WHERE x = '", "  AND y = 1", "    LIMIT 5'"]);
        console.key(&ctrl('u'), &[], 0, None);
        assert_eq!(console.sql(), "");
    }

    #[test]
    fn ctrl_o_lists_the_servers_and_tab_moves_through_the_answer() {
        let servers = ["ch1".to_string(), "ch2".to_string(), "ch3".to_string()];
        let mut console = Console::new(Some("ch2".into()));
        console.key(&ctrl('o'), &servers, 0, None);
        assert_eq!(console.choosing, Some(1), "on the one in use");
        console.key(&key(KeyCode::Down), &servers, 0, None);
        console.key(&key(KeyCode::Enter), &servers, 0, None);
        assert_eq!((console.node.as_deref(), console.choosing), (Some("ch3"), None));

        console.answer = Some(Answer {
            columns: vec![("a".into(), "UInt8".into()), ("b".into(), "String".into())],
            rows: (0..50).map(|n| vec![Some(n.to_string()), None]).collect(),
            ..Answer::default()
        });
        console.key(&key(KeyCode::BackTab), &servers, 0, None);
        assert_eq!(console.focus, Focus::Answer);
        console.key(&key(KeyCode::Down), &servers, 0, None);
        console.key(&key(KeyCode::Right), &servers, 0, None);
        console.key(&key(KeyCode::Right), &servers, 0, None);
        assert_eq!((console.scroll_row, console.scroll_col), (1, 1), "within the answer");
        console.key(&key(KeyCode::End), &servers, 0, None);
        assert_eq!(console.scroll_row, 49);
        console.key(&key(KeyCode::Char('x')), &servers, 0, None);
        assert_eq!((console.focus, console.sql().as_str()), (Focus::Editor, "x"), "typing goes back to the text");
        assert!(is_number("Nullable(UInt64)") && is_number("Float32") && !is_number("String"));
    }

    #[test]
    fn tab_takes_a_suggestion_and_a_function_opens_its_parentheses() {
        let mut console = Console::new(Some("ch1".into()));
        typed(&mut console, "SELECT * FROM system.proc");
        let open = console.suggest.as_ref().expect("suggestions as it is typed");
        assert_eq!(open.items[0].text, "processes");
        console.key(&key(KeyCode::Tab), &[], 0, None);
        assert_eq!(console.sql(), "SELECT * FROM system.processes");
        assert!(console.suggest.is_none());

        // ⏎ is the line's until a suggestion is chosen with the arrows.
        let mut console = Console::new(Some("ch1".into()));
        typed(&mut console, "SELECT cou");
        assert!(console.suggest.is_some());
        console.key(&key(KeyCode::Down), &[], 0, None);
        console.key(&key(KeyCode::Up), &[], 0, None);
        console.key(&key(KeyCode::Enter), &[], 0, None);
        assert_eq!((console.sql().as_str(), console.col), ("SELECT count()", 13), "the cursor in the parentheses");
        typed(&mut console, ")");
        assert_eq!(console.sql(), "SELECT count()", "a ) steps over the one there");
        assert_eq!(console.col, 14);

        // esc closes them for the word; the next word has its own.
        let mut console = Console::new(None);
        typed(&mut console, "SELECT cou");
        console.key(&key(KeyCode::Esc), &[], 0, None);
        typed(&mut console, "n");
        assert!(console.suggest.is_none(), "not again for that word");
        typed(&mut console, "t() FROM sys");
        assert!(console.suggest.is_some());
        console.key(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL), &[], 0, None);
        assert!(console.suggest.is_some(), "ctrl+space asks for them");
        // tab with nothing to take indents — after FROM it lists the tables.
        let mut console = Console::new(None);
        console.key(&key(KeyCode::Tab), &[], 0, None);
        assert_eq!(console.sql(), "  ");
        let mut console = Console::new(None);
        typed(&mut console, "SELECT * FROM ");
        console.key(&key(KeyCode::Esc), &[], 0, None);
        console.key(&key(KeyCode::Tab), &[], 0, None);
        assert!(console.suggest.as_ref().is_some_and(|s| s.items.iter().any(|i| i.text == "system.processes")), "{:?}", console.suggest);
    }

    #[test]
    fn ctrl_k_asks_the_helper_and_what_it_writes_takes_the_text_s_place_until_ctrl_z() {
        let mut console = Console::new(Some("ch1".into()));
        console.key(&ctrl('k'), &[], 10, None);
        assert!(matches!(&console.assisting, Assisting::Failed(why) if why.contains("--")), "nothing to ask");
        assert!(console.take_ask().is_none());

        typed(&mut console, "-- the ten users using the most memory");
        console.key(&ctrl('k'), &[], 10, None);
        let ask = console.take_ask().expect("asked");
        assert_eq!((ask.assistant, ask.node.as_deref(), ask.error.as_deref()), (Assistant::Claude, Some("ch1"), None));
        assert!(ask.sql.starts_with("-- the ten users"));
        assert!(console.asking());
        console.assisted(ask.id + 1, Ok("SELECT 2;".into()));
        assert!(console.asking(), "an answer to another question is not taken");
        console.assisted(ask.id, Ok("-- the ten users using the most memory\nSELECT user FROM system.processes;".into()));
        assert_eq!(console.assisting, Assisting::Wrote);
        assert_eq!(console.lines.len(), 2);
        assert!(console.take_request().is_none(), "nothing runs until asked");
        console.key(&ctrl('z'), &[], 11, None);
        assert_eq!(console.sql(), "-- the ten users using the most memory", "ctrl+z puts the text back");
        console.key(&ctrl('z'), &[], 11, None);
        assert!(console.sql().ends_with("system.processes;"), "and again, what it wrote");

        // What failed is asked about with what the server said.
        let id = {
            console.key(&key(KeyCode::Enter), &[], 12, None);
            console.take_request().unwrap().id
        };
        console.answered(id, Err("Code 47 · Unknown identifier".into()));
        // ctrl+g asks as ctrl+k does.
        console.key(&ctrl('g'), &[], 13, None);
        let ask = console.take_ask().unwrap();
        assert_eq!(ask.error.as_deref(), Some("Code 47 · Unknown identifier"));
        // ctrl+c withdraws the question; ctrl+t is the other helper.
        console.key(&ctrl('c'), &[], 14, None);
        assert_eq!((console.take_ask_cancel(), console.asking()), (Some(ask.id), false));
        console.key(&ctrl('t'), &[], 14, None);
        assert_eq!(console.assistant, Assistant::OpenCode);
        // A clear comes back with ctrl+z too.
        console.key(&ctrl('c'), &[], 15, None);
        assert_eq!(console.sql(), "");
        console.key(&ctrl('z'), &[], 15, None);
        assert!(console.sql().contains("system.processes"));
    }

    #[test]
    fn in_the_answer_y_copies_the_row_and_capital_y_all_of_it() {
        let mut console = Console::new(Some("ch1".into()));
        console.answer = Some(Answer {
            columns: vec![("user".into(), "String".into()), ("n".into(), "UInt64".into())],
            rows: vec![vec![Some("r_redash".into()), Some("2".into())], vec![Some("airflow".into()), None]],
            ..Answer::default()
        });
        console.key(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), &[], 0, None);
        console.key(&key(KeyCode::Down), &[], 0, None);
        console.key(&key(KeyCode::Char('y')), &[], 0, None);
        assert_eq!(console.take_clipboard(), Some(("airflow\tNULL".to_string(), "row 2".to_string())));
        console.key(&key(KeyCode::Char('Y')), &[], 0, None);
        assert_eq!(console.take_clipboard().unwrap().0, "user\tn\nr_redash\t2\nairflow\tNULL");
        assert_eq!(console.focus, Focus::Answer, "still in the answer");
    }
}
