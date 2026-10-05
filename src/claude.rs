//! View 5: sessions in panes (DESIGN.md §13) — Claude Code, OpenCode, or a shell.
//!
//! The monitor runs `claude` — Claude Code, signed in with the user's own Pro or Max plan —
//! `opencode`, or the user's shell in pseudo-terminals and draws one of them in the body of
//! view 5, under the header and the two band lines, so the fleet and the Redash queue stay in
//! sight while something else gets done. Nothing here talks to an API: each program does
//! everything itself, the way it would in a terminal tab of its own.
//!
//! There can be several sessions, each its own program with its own conversation, listed
//! beside the pane and numbered on from the header's tabs — the views are 1 to 4, the sessions
//! 5 to 9 — and a session can be renamed. `ctrl+\` is the one key a session does not get:
//! after it a number goes to that view or session, `n` `r` `x` open, rename and close
//! sessions (`c` `o` `t` open one of a kind), and `ctrl+\` again goes back to the monitor.
//!
//! This module is the sessions' state — emulated screens, bytes owed to each program, what each
//! program asked of its terminal — and the folder picker's, and is pure like the rest of `App`.
//! The processes (PTYs, children, the threads that read them) are `pty.rs`, the folders read for
//! the picker `folders.rs`, both driven from `main.rs`.

use crate::app::View;
use crate::conversations::Conversation;
use crate::folders::{Folder, Found, Lookup};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The number of the first session: the views are 1 to 4, so the sessions go on from 5, and
/// one number picks any tab, view or session — a digit the first five.
pub const FIRST_NUMBER: usize = 5;
/// The sessions a digit picks, 5 to 9.
pub const KEYED: usize = 10 - FIRST_NUMBER;
/// The most sessions there can be. Those after the first five are reached from the list beside
/// them, with the arrows after `ctrl+\`, or by what they are called (`ctrl+\ /`).
pub const MAX_SESSIONS: usize = 50;
/// A name longer than this would push the other tabs off the bar.
pub const NAME_MAX: usize = 24;

/// What a session runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    /// Claude Code, signed in with the user's own plan.
    #[default]
    Claude,
    /// OpenCode, signed in the way `opencode auth login` set it up.
    OpenCode,
    /// The user's own shell.
    Terminal,
    /// SQL on a server of the fleet, read-only (`console.rs`): no program, the session itself.
    Query,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Claude, Kind::OpenCode, Kind::Terminal, Kind::Query];

    /// Its mark in the list of sessions.
    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Claude => "✻",
            Kind::OpenCode => "▣",
            Kind::Terminal => "❯",
            Kind::Query => "▦",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Claude => "Claude",
            Kind::OpenCode => "OpenCode",
            Kind::Terminal => "Terminal",
            Kind::Query => "Query",
        }
    }

    /// Who the keys go to, for the footer.
    pub fn listener(self) -> &'static str {
        match self {
            Kind::Claude => "Claude",
            Kind::OpenCode => "OpenCode",
            Kind::Terminal => "the shell",
            Kind::Query => "the query",
        }
    }

    /// The variable that chooses its command.
    pub fn variable(self) -> &'static str {
        match self {
            Kind::Claude => "CLAUDE_CMD",
            Kind::OpenCode => "OPENCODE_CMD",
            Kind::Terminal => "SHELL_CMD",
            Kind::Query => "CH_SEED_URLS",
        }
    }

    /// How many lines scrolled off the top a pane keeps, for the wheel to bring back. A shell
    /// writes down the screen, so its past is worth keeping; Claude Code and OpenCode draw a
    /// screen of their own and scroll it themselves.
    pub fn scrollback(self) -> usize {
        match self {
            Kind::Terminal => 1000,
            Kind::Claude | Kind::OpenCode | Kind::Query => 0,
        }
    }

    /// The next kind round, for shift+tab in the picker.
    pub fn next(self) -> Kind {
        match self {
            Kind::Claude => Kind::OpenCode,
            Kind::OpenCode => Kind::Terminal,
            Kind::Terminal => Kind::Query,
            Kind::Query => Kind::Claude,
        }
    }

    /// What its program puts in its title before it has anything to say: its own name, which
    /// tells no two sessions apart.
    fn own_title(self, title: &str) -> bool {
        match self {
            Kind::Claude => title.eq_ignore_ascii_case("claude code") || title.eq_ignore_ascii_case("claude"),
            Kind::OpenCode => title.eq_ignore_ascii_case("opencode"),
            Kind::Terminal | Kind::Query => false,
        }
    }
}

/// The command each kind runs, with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commands {
    pub claude: Vec<String>,
    pub opencode: Vec<String>,
    pub terminal: Vec<String>,
}

impl Default for Commands {
    fn default() -> Self {
        Self { claude: vec!["claude".into()], opencode: vec!["opencode".into()], terminal: vec!["sh".into()] }
    }
}

impl Commands {
    pub fn of(&self, kind: Kind) -> &[String] {
        match kind {
            Kind::Claude => &self.claude,
            Kind::OpenCode => &self.opencode,
            Kind::Terminal => &self.terminal,
            Kind::Query => &[],
        }
    }
}

/// Where the pane is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneState {
    /// Nothing started yet: the first visit to view 5 starts it.
    Idle,
    /// Asked for: `main.rs` starts it before the next frame.
    Starting,
    Running,
    /// The program ended, and how.
    Exited(String),
    /// It could not be started at all — `claude` not installed, say.
    Failed(String),
}

/// What the program asked of its terminal while its output was parsed: answers to send back,
/// a title, a bell.
#[derive(Debug, Default)]
pub struct Replies {
    out: Vec<u8>,
    title: Option<String>,
    bell: bool,
}

impl vt100::Callbacks for Replies {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let title = String::from_utf8_lossy(title).trim().to_string();
        self.title = (!title.is_empty()).then_some(title);
    }

    /// The queries a full-screen program sends before it trusts its terminal: where the cursor
    /// is, and what kind of terminal this is. Unanswered, some wait for the answer.
    fn unhandled_csi(&mut self, screen: &mut vt100::Screen, i1: Option<u8>, _i2: Option<u8>, params: &[&[u16]], c: char) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            // DSR: the cursor position, 1-based.
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.out.extend(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            // DSR: the terminal is fine.
            (None, 'n', 5) => self.out.extend(b"\x1b[0n"),
            // DA1: a VT220 with ANSI colour; nothing that promises more than this pane does.
            (None, 'c', 0) => self.out.extend(b"\x1b[?62;22c"),
            // DA2.
            (Some(b'>'), 'c', 0) => self.out.extend(b"\x1b[>1;10;0c"),
            _ => {}
        }
    }

    /// OSC 10 / 11 `?`: the default colours, which some programs ask for to pick a light or a
    /// dark theme. The monitor is dark unless THEME=light says otherwise.
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if let [which @ (b"10" | b"11"), b"?"] = params {
            let light = std::env::var("THEME").is_ok_and(|t| t.eq_ignore_ascii_case("light"));
            let rgb = match (*which == b"10", light) {
                (true, false) | (false, true) => "dcdc/e2e2/ebeb",
                (true, true) | (false, false) => "0f0f/1212/1919",
            };
            let which = String::from_utf8_lossy(which);
            self.out.extend(format!("\x1b]{which};rgb:{rgb}\x1b\\").as_bytes());
        }
    }
}

/// One `claude`: its screen, what is owed to it, what it asked for.
pub struct ClaudePane {
    pub state: PaneState,
    parser: vt100::Parser<Replies>,
    /// Bytes for the program — keys, pastes, answers to its queries. `main.rs` sends them.
    outbox: Vec<u8>,
    /// The program rang its bell while it was not on screen — Claude Code does when it is done
    /// or needs an answer. Its tab shows it until it is looked at.
    pub attention: bool,
    /// What the program put in its terminal title: Claude Code writes what it is working on.
    pub title: Option<String>,
}

impl Default for ClaudePane {
    fn default() -> Self {
        Self {
            state: PaneState::Idle,
            parser: vt100::Parser::new_with_callbacks(24, 80, 0, Replies::default()),
            outbox: Vec::new(),
            attention: false,
            title: None,
        }
    }
}

impl ClaudePane {
    pub fn is_running(&self) -> bool {
        self.state == PaneState::Running
    }

    /// Ask for the program to be started — again, after it ended — on a clean screen of
    /// `rows` × `cols` that keeps `scrollback` lines of what scrolls off it.
    pub fn start(&mut self, (rows, cols): (u16, u16), scrollback: usize) {
        self.parser = vt100::Parser::new_with_callbacks(rows.max(1), cols.max(1), scrollback, Replies::default());
        self.outbox.clear();
        self.title = None;
        self.state = PaneState::Starting;
    }

    /// The program's output: the screen changes, and whatever it asked is answered.
    /// `visible` says whether this pane is on screen, for the bell.
    pub fn feed(&mut self, bytes: &[u8], visible: bool) {
        self.parser.process(bytes);
        let replies = self.parser.callbacks_mut();
        self.outbox.append(&mut replies.out);
        if let Some(title) = replies.title.take() {
            self.title = Some(title);
        }
        if std::mem::take(&mut replies.bell) && !visible {
            self.attention = true;
        }
    }

    /// A key for the program `kind` runs, as a terminal would send it — and, as a terminal
    /// does, back down to the bottom from wherever the wheel had scrolled.
    pub fn key(&mut self, key: &KeyEvent, kind: Kind) {
        self.parser.screen_mut().set_scrollback(0);
        let bytes = match kind {
            // Enter with shift, ⌘, ctrl or ⌥ is a new line in the prompt: sent as ctrl+j, the
            // line feed Claude Code and OpenCode both take as one, whatever the terminal is.
            Kind::Claude | Kind::OpenCode if is_new_line(key) => b"\n".to_vec(),
            _ => encode_key(key, self.parser.screen().application_cursor()),
        };
        self.outbox.extend(bytes);
    }

    /// How many lines up the wheel has scrolled the pane's own past; 0 at the bottom.
    pub fn scrolled_back(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// The pane's own past, `lines` further up (or down, negative), as far as there is one.
    fn scroll_back(&mut self, lines: isize) {
        let at = self.parser.screen().scrollback().saturating_add_signed(lines);
        // The emulator stops at what it kept.
        self.parser.screen_mut().set_scrollback(at);
    }

    /// Pasted text: bracketed when the program asked for it, so a pasted newline is text and
    /// not a press of Enter.
    pub fn paste(&mut self, text: &str) {
        self.parser.screen_mut().set_scrollback(0);
        if self.parser.screen().bracketed_paste() {
            self.outbox.extend(b"\x1b[200~");
            self.outbox.extend(text.as_bytes());
            self.outbox.extend(b"\x1b[201~");
        } else {
            self.outbox.extend(text.replace('\n', "\r").as_bytes());
        }
    }

    /// What is owed to the program, taken.
    pub fn take_outbox(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.outbox)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

/// How a session's program starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Start {
    /// A new conversation — Claude Code's under the id in `Session::conversation`, chosen here so
    /// it can be found again.
    #[default]
    Fresh,
    /// Take a conversation up again by its id; `fork`: as a copy of it, the original being open
    /// in another terminal still.
    Resume { id: String, fork: bool },
    /// OpenCode's last conversation in the folder: its id was never seen.
    Continue,
}

/// A session of view 5: one program, where it works, and the name it goes by.
pub struct Session {
    /// Stable for the session's life; the PTY's events carry it.
    pub id: u64,
    pub kind: Kind,
    /// What the user called it (`r`). Until then the list shows what Claude is working on,
    /// from its title.
    pub name: Option<String>,
    /// The directory its program runs in, as typed — `~` is fine; `main.rs` resolves it.
    pub dir: String,
    /// The git branch checked out there, kept fresh by `main.rs`.
    pub branch: Option<String>,
    pub pane: ClaudePane,
    /// The conversation it is in as far as is known: Claude Code's session id — chosen here for
    /// a new one, read back from Claude Code while it runs — or OpenCode's, once seen.
    pub conversation: Option<String>,
    /// How its program starts the next time it does.
    pub start: Start,
    /// When its program last started, Unix seconds: OpenCode's conversation is the one begun
    /// after it.
    pub started: i64,
    /// Its program's process id while it runs, for Claude Code to say what it is in.
    pub pid: Option<u32>,
    /// A query session's SQL, server and answer; `None` for the others.
    pub console: Option<Box<crate::console::Console>>,
}

impl Session {
    fn new(id: u64, kind: Kind, dir: String) -> Self {
        Session {
            id,
            kind,
            name: None,
            dir,
            branch: None,
            pane: ClaudePane::default(),
            conversation: (kind == Kind::Claude).then(|| uuid::Uuid::new_v4().to_string()),
            start: Start::Fresh,
            started: 0,
            pid: None,
            console: (kind == Kind::Query).then(|| Box::new(crate::console::Console::new(None))),
        }
    }

    /// Kept from the last run, not started yet: it is, when it is first shown. A query session
    /// has no program to start.
    pub fn waiting(&self) -> bool {
        self.console.is_none() && self.pane.state == PaneState::Idle
    }

    /// The arguments its program starts with, after its command's own: a conversation to take
    /// up or the id of a new one — for Claude Code and OpenCode themselves, not for a command
    /// of the user's that only runs them. `exists` says whether Claude Code has a conversation by
    /// an id: one never typed into cannot be resumed, only started under that id.
    pub fn launch_args(&self, program: &str, exists: impl Fn(&str) -> bool) -> Vec<String> {
        let name = Path::new(program).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fork = |fork: bool, flag: &str| fork.then(|| flag.to_string());
        match (self.kind, name.as_str()) {
            (Kind::Claude, "claude") => match &self.start {
                Start::Resume { id, fork: f } if exists(id) => {
                    [Some("--resume".to_string()), Some(id.clone()), fork(*f, "--fork-session")].into_iter().flatten().collect()
                }
                _ => self.conversation.iter().flat_map(|id| ["--session-id".to_string(), id.clone()]).collect(),
            },
            (Kind::OpenCode, "opencode") => match &self.start {
                Start::Resume { id, fork: f } => {
                    [Some("--session".to_string()), Some(id.clone()), fork(*f, "--fork")].into_iter().flatten().collect()
                }
                Start::Continue => vec!["--continue".to_string()],
                Start::Fresh => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The last part of its directory: `cobserve` for `~/work/cobserve`.
    pub fn dir_label(&self) -> String {
        let trimmed = self.dir.trim_end_matches('/');
        trimmed.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(trimmed).to_string()
    }

    /// The tab's text: the name, else what the program says it is working on (its title
    /// without a leading glyph), else the folder — a title that is only the program's own
    /// name tells no two sessions apart.
    pub fn label(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        if let Some(console) = &self.console {
            return console.node.clone().unwrap_or_else(|| "no server".to_string());
        }
        let title = self.pane.title.as_deref().map(|t| t.trim_start_matches(|c: char| !c.is_alphanumeric()).trim());
        match title {
            Some(title) if !title.is_empty() && !self.kind.own_title(title) => title.to_string(),
            _ => self.dir_label(),
        }
    }
}

/// Where a new session will work, chosen as in a file explorer: a folder at a time — a click
/// goes in, the path above it goes back up — or a search below it. The folders are read by
/// `folders.rs` on a thread `main.rs` starts; this is what was asked and what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    /// What the session will run.
    pub kind: Kind,
    /// The folder looked at: absolute, as the disk has it once the first answer is in.
    pub dir: PathBuf,
    /// The search, as typed: a name to find below `dir`, or a path.
    pub query: String,
    pub folders: Vec<Folder>,
    /// The branch of `dir` itself.
    pub branch: Option<String>,
    /// An answer for what is on screen is still coming.
    pub looking: bool,
    /// The last search stopped at its limits.
    pub cut_short: bool,
    pub error: Option<String>,
    /// The row the cursor is on, in `rows()`.
    pub cursor: usize,
    /// The first row on screen, kept by the drawing so the cursor stays in sight.
    pub scroll: Cell<usize>,
    generation: u64,
    asked: u64,
    /// Conversations of `kind` to take up: those had in `dir`, or — `everywhere` — anywhere.
    pub conversations: Vec<Conversation>,
    /// Listing every folder's conversations rather than this folder's folders.
    pub everywhere: bool,
    /// What the conversations on screen are for, and what was last asked: a lookup per change.
    conversations_for: Option<(u64, Kind, bool)>,
    conversations_asked: Option<(u64, Kind, bool)>,
    /// For a query session, the fleet's servers to choose from — kept fresh by `App` — and
    /// whether each answers.
    pub servers: Vec<(String, bool)>,
}

/// What `main.rs` should list for the picker: conversations of `kind`, in `dir` or everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationLookup {
    pub generation: u64,
    pub kind: Kind,
    /// `None`: every folder's.
    pub dir: Option<PathBuf>,
}

/// How many of a folder's conversations the picker lists; everywhere, how many in all.
pub const CONVERSATIONS_HERE: usize = 4;
pub const CONVERSATIONS_EVERYWHERE: usize = 30;

/// A row of the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickRow {
    /// Open the session in the folder looked at.
    Here,
    /// One of `conversations`: take it up.
    Conversation(usize),
    /// List every folder's conversations.
    Everywhere,
    /// For a query session: one of `servers`.
    Server(usize),
    /// The folder above.
    Up,
    /// One of `folders`.
    Folder(usize),
}

/// Lookups are numbered across every picker, so an answer from one closed a moment ago is
/// never taken for the new one's.
fn next_generation() -> u64 {
    static GENERATIONS: AtomicU64 = AtomicU64::new(0);
    GENERATIONS.fetch_add(1, Ordering::Relaxed) + 1
}

impl Picker {
    pub fn new(dir: PathBuf, kind: Kind) -> Self {
        Self {
            kind,
            dir,
            query: String::new(),
            folders: Vec::new(),
            branch: None,
            looking: true,
            cut_short: false,
            error: None,
            cursor: 0,
            scroll: Cell::new(0),
            generation: next_generation(),
            asked: 0,
            conversations: Vec::new(),
            everywhere: false,
            conversations_for: None,
            conversations_asked: None,
            servers: Vec::new(),
        }
    }

    /// The servers a query session can be opened on, those the search names.
    pub fn servers_shown(&self) -> Vec<usize> {
        let wanted = self.query.trim().to_lowercase();
        (0..self.servers.len()).filter(|&i| wanted.is_empty() || self.servers[i].0.to_lowercase().contains(&wanted)).collect()
    }

    /// Whether the kind on screen keeps conversations to take up: a shell does not.
    fn keeps_conversations(&self) -> bool {
        matches!(self.kind, Kind::Claude | Kind::OpenCode)
    }

    /// The conversations to ask `main.rs` for, once per folder and kind.
    pub fn conversation_lookup(&mut self) -> Option<ConversationLookup> {
        let wanted = (self.generation, self.kind, self.everywhere);
        if !self.keeps_conversations() || self.searching() || self.conversations_asked == Some(wanted) {
            return None;
        }
        self.conversations_asked = Some(wanted);
        Some(ConversationLookup { generation: self.generation, kind: self.kind, dir: (!self.everywhere).then(|| self.dir.clone()) })
    }

    /// Conversations listed, taken when they are for what is on screen.
    pub fn found_conversations(&mut self, generation: u64, kind: Kind, everywhere: bool, list: Vec<Conversation>) {
        if (generation, kind, everywhere) != (self.generation, self.kind, self.everywhere) {
            return;
        }
        self.conversations = list;
        self.conversations_for = Some((generation, kind, everywhere));
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// The conversations on screen are the ones for the folder and kind shown — not those of a
    /// kind switched away from.
    pub fn conversations_shown(&self) -> &[Conversation] {
        if self.conversations_for == Some((self.generation, self.kind, self.everywhere)) { &self.conversations } else { &[] }
    }

    /// Still listing them.
    pub fn looking_for_conversations(&self) -> bool {
        self.keeps_conversations() && !self.searching() && self.conversations_for != Some((self.generation, self.kind, self.everywhere))
    }

    /// Every folder's conversations, or back to the folders.
    pub fn show_everywhere(&mut self, everywhere: bool) {
        if self.everywhere != everywhere {
            self.everywhere = everywhere;
            self.cursor = 0;
            self.scroll.set(0);
        }
    }

    /// Searching rather than looking at one folder.
    pub fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// What the list shows: in a folder, the way to open the session there and the way up
    /// before its folders; searching, what was found.
    pub fn rows(&self) -> Vec<PickRow> {
        if self.kind == Kind::Query {
            return self.servers_shown().into_iter().map(PickRow::Server).collect();
        }
        let conversations = (0..self.conversations_shown().len()).map(PickRow::Conversation);
        if self.everywhere {
            return conversations.collect();
        }
        let mut rows = Vec::new();
        if !self.searching() {
            rows.push(PickRow::Here);
            rows.extend(conversations);
            if self.keeps_conversations() {
                rows.push(PickRow::Everywhere);
            }
            if self.dir.parent().is_some() {
                rows.push(PickRow::Up);
            }
        }
        rows.extend((0..self.folders.len()).map(PickRow::Folder));
        rows
    }

    /// The row under the cursor.
    pub fn at_cursor(&self) -> Option<PickRow> {
        self.rows().get(self.cursor).copied()
    }

    /// The folder a row stands for.
    pub fn path_of(&self, row: PickRow) -> Option<PathBuf> {
        match row {
            PickRow::Here => Some(self.dir.clone()),
            PickRow::Up => self.dir.parent().map(Path::to_path_buf),
            PickRow::Folder(index) => self.folders.get(index).map(|f| f.path.clone()),
            PickRow::Conversation(index) => self.conversations_shown().get(index).map(|c| c.dir.clone()),
            PickRow::Everywhere | PickRow::Server(_) => None,
        }
    }

    /// What `main.rs` should look up for what is on screen now — once.
    pub fn lookup(&mut self) -> Option<Lookup> {
        // A query session works on a server, not in a folder.
        if self.kind == Kind::Query {
            self.looking = false;
            return None;
        }
        (self.asked != self.generation).then(|| {
            self.asked = self.generation;
            Lookup { generation: self.generation, dir: self.dir.clone(), query: self.query.clone() }
        })
    }

    /// An answer, taken when it is for what is on screen.
    pub fn found(&mut self, found: Found) {
        if found.generation != self.generation {
            return;
        }
        if let Some(dir) = found.dir {
            self.dir = dir;
        }
        self.branch = found.branch;
        self.folders = found.folders;
        self.looking = !found.done;
        self.cut_short = found.cut_short;
        self.error = found.error;
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// Something to look up changed: the cursor goes back to the top.
    fn changed(&mut self) {
        self.generation = next_generation();
        self.looking = true;
        self.cut_short = false;
        self.error = None;
        self.cursor = 0;
        self.scroll.set(0);
    }

    /// Into `dir`, with the search cleared.
    pub fn go_to(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.query.clear();
        self.folders.clear();
        self.branch = None;
        self.changed();
    }

    /// Up a folder, where there is one.
    pub fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.go_to(parent);
        }
    }

    pub fn type_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.query.push_str(text);
            self.changed();
        }
    }

    /// The last character of the search off; `false` when there was none.
    pub fn backspace(&mut self) -> bool {
        let had = self.query.pop().is_some();
        if had {
            self.changed();
        }
        had
    }

    pub fn clear_query(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.changed();
        }
    }

    /// The cursor `delta` rows on, within the list.
    pub fn step(&mut self, delta: isize) {
        let last = self.rows().len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    /// The way back up, `~ › work › cobserve`: each step's name and the folder it opens.
    pub fn crumbs(&self) -> Vec<(String, PathBuf)> {
        let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.parent().is_some());
        let (mut at, rest, first) = match home {
            Some(home) if self.dir.starts_with(&home) => {
                let rest = self.dir.strip_prefix(&home).unwrap_or(Path::new("")).to_path_buf();
                (home, rest, "~")
            }
            _ => {
                let rest = self.dir.strip_prefix("/").unwrap_or(&self.dir).to_path_buf();
                (PathBuf::from("/"), rest, "/")
            }
        };
        let mut crumbs = vec![(first.to_string(), at.clone())];
        for part in rest.components() {
            at.push(part);
            crumbs.push((part.as_os_str().to_string_lossy().to_string(), at.clone()));
        }
        crumbs
    }
}

/// What keys on view 5 do besides going to Claude.
// One of these is held, by `Sessions`: the picker's size costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Every key but `ctrl+\` goes to the session on screen.
    Typing,
    /// After `ctrl+\`: a number (1–4 a view, 5–9 a session), `n`, `r`, `x`, `esc`.
    Bar,
    /// Renaming the session on screen; the text so far.
    Naming(String),
    /// Opening a session: choosing the folder it will work in.
    Opening(Picker),
    /// Looking for a session by what it is called (`ctrl+\ /`): what is typed so far, and the
    /// one the cursor is on among those it names.
    Finding { query: String, at: usize },
}

/// Every session of view 5, and which one is on screen.
pub struct Sessions {
    pub list: Vec<Session>,
    pub active: usize,
    pub mode: Mode,
    /// What each kind runs: `CLAUDE_CMD`, `OPENCODE_CMD`, `SHELL_CMD`.
    pub commands: Commands,
    /// The size view 5 last drew a pane at; `main.rs` sizes every PTY to it, so a session
    /// switched to is already the right size.
    pub want_size: Cell<(u16, u16)>,
    /// The view the monitor goes back to.
    pub back_to: View,
    /// `x` was pressed once: the next `x` closes the session on screen.
    pub closing: bool,
    /// Where a new session works unless told otherwise: the monitor's own directory.
    pub default_dir: String,
    /// Where the pane was last drawn, for the mouse.
    pub pane_origin: Cell<(u16, u16)>,
    /// Wheel ticks not yet sent as a page: three make one, so a turn of the wheel is a page
    /// and not three.
    wheel: i8,
    next_id: u64,
    /// Who writes a new query session's SQL when asked: `ASSISTANT`, or the last one chosen.
    pub assistant: crate::console::Assistant,
    /// The first session the list beside them shows, kept by the drawing so the one on screen
    /// stays in sight when there are more than fit.
    pub list_scroll: Cell<usize>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            active: 0,
            mode: Mode::Typing,
            commands: Commands::default(),
            want_size: Cell::new((24, 80)),
            back_to: View::Nodes,
            closing: false,
            default_dir: ".".to_string(),
            pane_origin: Cell::new((0, 0)),
            wheel: 0,
            next_id: 1,
            assistant: crate::console::Assistant::Claude,
            list_scroll: Cell::new(0),
        }
    }
}

impl Sessions {
    pub fn current(&self) -> Option<&Session> {
        self.list.get(self.active)
    }

    pub fn current_mut(&mut self) -> Option<&mut Session> {
        self.list.get_mut(self.active)
    }

    pub fn by_id_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.list.iter_mut().find(|s| s.id == id)
    }

    /// Whether the session on screen has a program to type into.
    pub fn is_running(&self) -> bool {
        self.current().is_some_and(|s| s.pane.is_running())
    }

    /// A new session of `kind` working in `dir` (the monitor's own directory when empty),
    /// started and put on screen. `None` when every number is taken.
    pub fn open_new(&mut self, dir: &str, kind: Kind) -> Option<u64> {
        if self.list.len() >= MAX_SESSIONS {
            return None;
        }
        if kind == Kind::Query {
            return self.open_query(None);
        }
        let dir = if dir.trim().is_empty() { self.default_dir.clone() } else { dir.trim().to_string() };
        let session = Session::new(self.take_id(), kind, dir);
        Some(self.add(session))
    }

    /// A new query session on `node`, on screen.
    pub fn open_query(&mut self, node: Option<&str>) -> Option<u64> {
        if self.list.len() >= MAX_SESSIONS {
            return None;
        }
        let mut session = Session::new(self.take_id(), Kind::Query, self.default_dir.clone());
        let mut console = crate::console::Console::new(node.map(str::to_string));
        console.assistant = self.assistant;
        session.console = Some(Box::new(console));
        Some(self.add(session))
    }

    /// A new session that takes up a conversation had elsewhere — in another terminal, an
    /// earlier run — in the folder it worked in; as a copy when it is still open there.
    pub fn open_conversation(&mut self, conversation: &Conversation) -> Option<u64> {
        if self.list.len() >= MAX_SESSIONS {
            return None;
        }
        let mut session = Session::new(self.take_id(), conversation.kind, crate::pty::tilde(&conversation.dir));
        session.conversation = Some(conversation.id.clone());
        session.start = Start::Resume { id: conversation.id.clone(), fork: conversation.open };
        Some(self.add(session))
    }

    /// The sessions of the last run, back in the list as they were, none started: each is when
    /// it is first shown.
    pub fn restore(&mut self, saved: &crate::saved::Saved) {
        for kept in saved.sessions.iter().take(MAX_SESSIONS.saturating_sub(self.list.len())) {
            let kind = Kind::from(kept.kind);
            let mut session = Session::new(self.take_id(), kind, kept.dir.clone());
            session.name = kept.name.clone();
            if let Some(console) = session.console.as_mut() {
                console.node = kept.node.clone();
                console.history = kept.history.clone();
                console.set_sql(kept.sql.as_deref().unwrap_or_default());
                console.assistant = self.assistant;
            }
            session.start = match (kind, &kept.conversation) {
                (Kind::Claude | Kind::OpenCode, Some(id)) => Start::Resume { id: id.clone(), fork: false },
                (Kind::OpenCode, None) => Start::Continue,
                _ => Start::Fresh,
            };
            if kind == Kind::Claude {
                session.conversation = kept.conversation.clone().or(session.conversation);
            } else {
                session.conversation = kept.conversation.clone();
            }
            self.list.push(session);
        }
        self.active = saved.active.min(self.list.len().saturating_sub(1));
    }

    /// The sessions as they are now, to be kept for the next run.
    pub fn to_saved(&self) -> crate::saved::Saved {
        crate::saved::Saved {
            sessions: self
                .list
                .iter()
                .map(|s| crate::saved::SavedSession {
                    kind: s.kind.into(),
                    name: s.name.clone(),
                    dir: s.dir.clone(),
                    conversation: match s.kind {
                        Kind::Terminal | Kind::Query => None,
                        // Claude Code's conversation is known from the start; one never typed
                        // into is started under the same id next time.
                        _ => s.conversation.clone(),
                    },
                    node: s.console.as_ref().and_then(|c| c.node.clone()),
                    sql: s.console.as_ref().map(|c| c.sql()).filter(|sql| !sql.trim().is_empty()),
                    history: s.console.as_ref().map(|c| c.history.clone()).unwrap_or_default(),
                })
                .collect(),
            active: self.active,
        }
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// `session` started and put on screen.
    fn add(&mut self, mut session: Session) -> u64 {
        if session.console.is_none() {
            session.pane.start(self.want_size.get(), session.kind.scrollback());
        }
        let id = session.id;
        self.list.push(session);
        self.select(self.list.len() - 1);
        id
    }

    /// Where the next session would work: the directory of the one on screen, else the
    /// monitor's.
    pub fn next_dir(&self) -> String {
        self.current().map_or_else(|| self.default_dir.clone(), |s| s.dir.clone())
    }

    /// What the next session would run: what the one on screen runs, else Claude.
    pub fn next_kind(&self) -> Kind {
        self.current().map_or(Kind::Claude, |s| s.kind)
    }

    /// The command a session of `kind` runs.
    pub fn command_of(&self, kind: Kind) -> &[String] {
        self.commands.of(kind)
    }

    /// The mouse over the pane, for the program on screen: as the program asked for it when it
    /// asked for mouse reports. Else the wheel does what it would in a terminal of its own:
    /// for Claude Code a turn is a page, what it scrolls its conversation with; a full-screen
    /// program (`less`, `vim` in a shell) gets the arrow keys; and a shell's screen scrolls
    /// back through what went past.
    pub fn mouse(&mut self, event: &MouseEvent) {
        let (top, left) = self.pane_origin.get();
        let wheel = match event.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => 0,
        };
        let Some(session) = self.list.get_mut(self.active).filter(|s| s.pane.is_running()) else {
            return;
        };
        let (row, col) = (event.row.saturating_sub(top), event.column.saturating_sub(left));
        if let Some(bytes) = encode_mouse(session.pane.screen(), event, row, col) {
            session.pane.outbox.extend(bytes);
            return;
        }
        if wheel == 0 {
            return;
        }
        if session.kind != Kind::Claude {
            if session.pane.screen().alternate_screen() {
                let up = KeyEvent::new(if wheel < 0 { KeyCode::Up } else { KeyCode::Down }, KeyModifiers::NONE);
                session.pane.key(&up, session.kind);
            } else {
                session.pane.scroll_back(-3 * isize::from(wheel));
            }
            return;
        }
        if self.wheel.signum() != wheel {
            self.wheel = 0;
        }
        self.wheel += wheel;
        if self.wheel.abs() >= 3 {
            self.wheel = 0;
            session.pane.outbox.extend(if wheel < 0 { b"\x1b[5~" } else { b"\x1b[6~" });
        }
    }

    /// The session on screen ends, and its neighbour takes its place.
    pub fn close_current(&mut self) {
        if self.active < self.list.len() {
            self.list.remove(self.active);
        }
        self.active = self.active.min(self.list.len().saturating_sub(1));
        self.closing = false;
    }

    /// Put session `index` on screen; what it rang for has now been seen, and one kept from the
    /// last run starts — takes its conversation up — now.
    pub fn select(&mut self, index: usize) {
        if index < self.list.len() {
            self.active = index;
            let size = self.want_size.get();
            let session = &mut self.list[index];
            session.pane.attention = false;
            if session.waiting() {
                session.pane.start(size, session.kind.scrollback());
            }
        }
    }

    /// The next or the previous session, round the bar.
    pub fn step(&mut self, forward: bool) {
        let len = self.list.len();
        if len > 0 {
            self.select(if forward { (self.active + 1) % len } else { (self.active + len - 1) % len });
        }
    }

    /// Name the session on screen; an empty name gives the default back.
    pub fn rename_current(&mut self, name: &str) {
        let name: String = name.trim().chars().take(NAME_MAX).collect();
        if let Some(session) = self.current_mut() {
            session.name = (!name.is_empty()).then_some(name);
        }
    }

    /// Some session rang while it was not on screen.
    pub fn calling(&self) -> bool {
        self.list.iter().any(|s| s.pane.attention)
    }

    /// The tab number of session `index`.
    pub fn number_of(index: usize) -> usize {
        FIRST_NUMBER + index
    }

    /// The sessions `query` names — in what they are called, what they run, their folder or
    /// server, or by their number — in the list's order; every one when nothing is typed.
    pub fn matching(&self, query: &str) -> Vec<usize> {
        let wanted = query.trim().to_lowercase();
        let has = |text: &str| text.to_lowercase().contains(&wanted);
        self.list
            .iter()
            .enumerate()
            .filter(|(index, session)| {
                wanted.is_empty()
                    || Sessions::number_of(*index).to_string() == wanted
                    || has(&session.label())
                    || has(&session.dir)
                    || has(session.kind.title())
                    || session.console.as_ref().and_then(|c| c.node.as_deref()).is_some_and(has)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The session a tab number picks, if there is one.
    pub fn index_of(&self, number: usize) -> Option<usize> {
        number.checked_sub(FIRST_NUMBER).filter(|i| *i < self.list.len())
    }
}

/// `ctrl+\` — the one key that does not go to the program: after it, a number picks a view or a
/// session; pressed again, it goes to the monitor. Claude Code has no use for it, and terminals send it as
/// `^\` (crossterm reports that byte as `ctrl+4`).
pub fn is_switch_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('4'))
}

/// A mouse event as the program asked to get them (`\e[?1000h` and friends), at `row`, `col`
/// of the pane; `None` when it did not ask.
fn encode_mouse(screen: &vt100::Screen, event: &MouseEvent, row: u16, col: u16) -> Option<Vec<u8>> {
    use vt100::{MouseProtocolEncoding, MouseProtocolMode};
    let mode = screen.mouse_protocol_mode();
    let (button, release) = match event.kind {
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::Down(button) => (button_code(button), false),
        MouseEventKind::Up(button) if mode != MouseProtocolMode::Press => (button_code(button), true),
        MouseEventKind::Drag(button) if matches!(mode, MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion) => {
            (button_code(button) + 32, false)
        }
        _ => return None,
    };
    if mode == MouseProtocolMode::None {
        return None;
    }
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    Some(match screen.mouse_protocol_encoding() {
        MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes(),
        _ => {
            // The old encoding: one byte each, offset by 32; a release is button 3.
            let b = if release { 3 } else { button };
            let byte = |v: u32| u8::try_from((v + 32).min(255)).unwrap_or(255);
            vec![0x1b, b'[', b'M', byte(b), byte(x), byte(y)]
        }
    })
}

fn button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// Enter with a modifier — shift, ⌘, ctrl or ⌥ — which the terminal can tell apart from Enter
/// alone when it speaks the kitty keyboard protocol (⌥ as Meta it always can).
pub fn is_new_line(key: &KeyEvent) -> bool {
    let modifiers = KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::META | KeyModifiers::HYPER;
    key.code == KeyCode::Enter && key.modifiers.intersects(modifiers)
}

/// A key as an xterm sends it.
pub fn encode_key(key: &KeyEvent, application_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let esc = |rest: &str| format!("\x1b{rest}").into_bytes();
    let cursor = |letter: char| {
        if modifier > 1 {
            esc(&format!("[1;{modifier}{letter}"))
        } else if application_cursor {
            esc(&format!("O{letter}"))
        } else {
            esc(&format!("[{letter}"))
        }
    };
    let tilde = |number: u8| {
        if modifier > 1 {
            esc(&format!("[{number};{modifier}~"))
        } else {
            esc(&format!("[{number}~"))
        }
    };
    let with_alt = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    match key.code {
        KeyCode::Char(c) if ctrl => {
            let byte = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                'A'..='Z' => c as u8 - b'A' + 1,
                ' ' | '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '7' | '/' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return with_alt(c.to_string().into_bytes()),
            };
            with_alt(vec![byte])
        }
        KeyCode::Char(c) => with_alt(c.to_string().into_bytes()),
        // ⌥ as Meta puts an escape before it; shift, ctrl and ⌘ an xterm leaves out.
        KeyCode::Enter if alt => b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => esc("[Z"),
        KeyCode::Backspace if ctrl => with_alt(vec![0x08]),
        KeyCode::Backspace => with_alt(vec![0x7f]),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => match modifier {
            1 => esc(&format!("O{}", (b'P' + n - 1) as char)),
            _ => esc(&format!("[1;{modifier}{}", (b'P' + n - 1) as char)),
        },
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_are_sent_the_way_an_xterm_sends_them() {
        let plain = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let send = |code, modifiers| encode_key(&key(code, modifiers), false);
        assert_eq!(send(KeyCode::Char('q'), plain), b"q");
        assert_eq!(send(KeyCode::Char('é'), plain), "é".as_bytes());
        assert_eq!(send(KeyCode::Char('c'), ctrl), [0x03], "ctrl+c interrupts Claude, not the monitor");
        assert_eq!(send(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
        assert_eq!(send(KeyCode::Enter, plain), b"\r");
        assert_eq!(send(KeyCode::Enter, KeyModifiers::SHIFT), b"\r", "as an xterm sends it");
        assert_eq!(send(KeyCode::Enter, KeyModifiers::ALT), b"\x1b\r", "⌥ as Meta");
        assert_eq!(send(KeyCode::Backspace, plain), [0x7f]);
        assert_eq!(send(KeyCode::Esc, plain), [0x1b]);
        assert_eq!(send(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z", "shift+tab cycles Claude's modes");
        assert_eq!(send(KeyCode::Up, plain), b"\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Up, plain), true), b"\x1bOA", "application cursor mode");
        assert_eq!(send(KeyCode::Right, ctrl), b"\x1b[1;5C");
        assert_eq!(send(KeyCode::PageDown, plain), b"\x1b[6~");
        assert_eq!(send(KeyCode::Delete, KeyModifiers::SHIFT), b"\x1b[3;2~");
        assert_eq!(send(KeyCode::F(1), plain), b"\x1bOP");
        assert_eq!(send(KeyCode::F(12), plain), b"\x1b[24~");
    }

    #[test]
    fn enter_with_a_modifier_is_a_new_line_for_claude_and_opencode_and_enter_for_a_shell() {
        let mut pane = ClaudePane::default();
        let enter = |modifiers| key(KeyCode::Enter, modifiers);
        // ⇧⏎, ⌘⏎ (the kitty keyboard protocol's super), ctrl+⏎ and ⌥⏎: ctrl+j, a new line.
        for modifiers in [KeyModifiers::SHIFT, KeyModifiers::SUPER, KeyModifiers::CONTROL, KeyModifiers::ALT] {
            for kind in [Kind::Claude, Kind::OpenCode] {
                pane.key(&enter(modifiers), kind);
                assert_eq!(pane.take_outbox(), b"\n", "{modifiers:?} in {kind:?}");
            }
        }
        pane.key(&enter(KeyModifiers::NONE), Kind::Claude);
        assert_eq!(pane.take_outbox(), b"\r", "Enter alone sends the prompt");
        pane.key(&key(KeyCode::Char('j'), KeyModifiers::CONTROL), Kind::OpenCode);
        assert_eq!(pane.take_outbox(), b"\n", "ctrl+j itself, in any terminal");
        // A shell gets what an xterm sends: Enter is Enter, ⌥ puts an escape before it.
        pane.key(&enter(KeyModifiers::SHIFT), Kind::Terminal);
        assert_eq!(pane.take_outbox(), b"\r");
        pane.key(&enter(KeyModifiers::ALT), Kind::Terminal);
        assert_eq!(pane.take_outbox(), b"\x1b\r");
    }

    #[test]
    fn ctrl_backslash_is_the_switch_however_the_terminal_reports_it() {
        assert!(is_switch_key(&key(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
        assert!(is_switch_key(&key(KeyCode::Char('4'), KeyModifiers::CONTROL)), "crossterm's name for ^\\");
        assert!(!is_switch_key(&key(KeyCode::Char('\\'), KeyModifiers::NONE)));
        assert!(!is_switch_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn the_screen_follows_the_output_and_queries_are_answered() {
        let mut pane = ClaudePane::default();
        pane.start((24, 80), 0);
        pane.state = PaneState::Running;
        pane.feed(b"\x1b]0;\xe2\x9c\xb3 Fix the parser\x07hello \x1b[31mred\x1b[m\r\n> ", true);
        let screen = pane.screen();
        assert_eq!(screen.contents().lines().next(), Some("hello red"));
        assert_eq!(screen.cell(0, 6).unwrap().fgcolor(), vt100::Color::Idx(1));
        assert_eq!(pane.title.as_deref(), Some("✳ Fix the parser"), "Claude Code's task, from its title");

        pane.feed(b"\x1b[6n\x1b[c", true);
        assert_eq!(pane.take_outbox(), b"\x1b[2;3R\x1b[?62;22c", "the cursor is on row 2, column 3");
        assert!(pane.take_outbox().is_empty(), "taken once");
    }

    #[test]
    fn a_bell_off_screen_asks_for_attention() {
        let mut pane = ClaudePane::default();
        pane.feed(b"\x07", true);
        assert!(!pane.attention, "on screen, the bell is seen");
        pane.feed(b"done\x07", false);
        assert!(pane.attention);
    }

    #[test]
    fn a_paste_is_bracketed_when_the_program_asked() {
        let mut pane = ClaudePane::default();
        pane.paste("a\nb");
        assert_eq!(pane.take_outbox(), b"a\rb");
        pane.feed(b"\x1b[?2004h", true);
        pane.paste("a\nb");
        assert_eq!(pane.take_outbox(), b"\x1b[200~a\nb\x1b[201~");
    }

    #[test]
    fn starting_again_begins_on_a_clean_screen_of_the_drawn_size() {
        let mut pane = ClaudePane::default();
        pane.feed(b"old output", true);
        pane.start((30, 100), 0);
        assert_eq!(pane.state, PaneState::Starting);
        assert_eq!(pane.screen().size(), (30, 100));
        assert!(pane.screen().contents().trim().is_empty());
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn the_wheel_scrolls_claude_a_page_per_turn_unless_it_asked_for_the_mouse() {
        let mut sessions = Sessions::default();
        sessions.open_new("", Kind::Claude).unwrap();
        sessions.current_mut().unwrap().pane.state = PaneState::Running;
        sessions.pane_origin.set((5, 30));
        // Three ticks of the wheel are one turn: one page.
        for _ in 0..3 {
            sessions.mouse(&mouse(MouseEventKind::ScrollUp, 40, 10));
        }
        assert_eq!(sessions.current_mut().unwrap().pane.take_outbox(), b"\x1b[5~");
        for _ in 0..2 {
            sessions.mouse(&mouse(MouseEventKind::ScrollDown, 40, 10));
        }
        assert!(sessions.current_mut().unwrap().pane.take_outbox().is_empty(), "not yet a turn");
        // A program that asks for SGR mouse reports gets them, at its own coordinates.
        sessions.current_mut().unwrap().pane.feed(b"\x1b[?1000h\x1b[?1006h", true);
        sessions.mouse(&mouse(MouseEventKind::ScrollDown, 40, 10));
        sessions.mouse(&mouse(MouseEventKind::Down(MouseButton::Left), 31, 5));
        sessions.mouse(&mouse(MouseEventKind::Up(MouseButton::Left), 31, 5));
        assert_eq!(sessions.current_mut().unwrap().pane.take_outbox(), b"\x1b[<65;11;6M\x1b[<0;2;1M\x1b[<0;2;1m");
    }

    #[test]
    fn the_picker_asks_once_per_change_and_takes_only_its_own_answers() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let work = home.join("work");
        let folder = |name: &str| Folder { path: work.join(name), shown: name.into(), hit: None, branch: None };
        let mut picker = Picker::new(work.clone(), Kind::Claude);
        let first = picker.lookup().expect("the first look");
        assert_eq!((first.dir.as_path(), first.query.as_str()), (work.as_path(), ""));
        assert!(picker.lookup().is_none(), "asked once");
        assert_eq!(
            picker.rows(),
            [PickRow::Here, PickRow::Everywhere, PickRow::Up],
            "the way to open it here, to every folder's conversations and up, before anything is read"
        );
        let answer = |generation, folders| Found { generation, dir: Some(work.clone()), folders, done: true, ..Found::default() };
        picker.found(answer(first.generation, vec![folder("airflow"), folder("cobserve")]));
        assert_eq!(picker.rows().len(), 5);
        assert!(!picker.looking);

        // Typing searches; an answer to the question before comes too late to be taken.
        picker.type_text("cob");
        let search = picker.lookup().expect("a search");
        assert_eq!(search.query, "cob");
        picker.found(answer(first.generation, Vec::new()));
        assert_eq!(picker.folders.len(), 2, "dropped");
        picker.found(answer(search.generation, vec![folder("cobserve")]));
        assert_eq!(picker.rows(), [PickRow::Folder(0)], "searching, the list is what was found");
        assert_eq!(picker.path_of(PickRow::Folder(0)), Some(work.join("cobserve")));

        // In and up again, the search cleared on the way; the path back up.
        picker.go_to(work.join("cobserve"));
        assert!(picker.query.is_empty() && picker.looking && picker.cursor == 0);
        let names = |picker: &Picker| picker.crumbs().into_iter().map(|(name, _)| name).collect::<Vec<_>>();
        assert_eq!(names(&picker), ["~", "work", "cobserve"]);
        assert_eq!(picker.crumbs()[1].1, work);
        picker.up();
        assert_eq!(picker.dir, work);
        picker.step(-5);
        assert_eq!(picker.cursor, 0, "the cursor stays in the list");
        picker.step(isize::MAX);
        assert_eq!(picker.cursor, picker.rows().len() - 1);
        assert_eq!(names(&Picker::new(PathBuf::from("/opt/tools"), Kind::Terminal)), ["/", "opt", "tools"]);
    }

    #[test]
    fn a_session_starts_its_conversation_by_id_and_takes_one_up_again() {
        let mut sessions = Sessions::default();
        sessions.open_new("~/work/billing", Kind::Claude).unwrap();
        let session = sessions.current().unwrap();
        let id = session.conversation.clone().expect("Claude Code's id, chosen here");
        assert_eq!(session.launch_args("claude", |_| false), ["--session-id", id.as_str()]);
        assert_eq!(session.launch_args("/usr/local/bin/claude", |_| false), ["--session-id", id.as_str()]);
        assert!(session.launch_args("my-claude-wrapper", |_| false).is_empty(), "a command of the user's own gets nothing added");
        sessions.rename_current("billing export");
        sessions.open_new("~/work/pipelines", Kind::OpenCode).unwrap();
        sessions.open_new("~/work/reports", Kind::Terminal).unwrap();
        assert!(sessions.list[1].launch_args("opencode", |_| true).is_empty(), "a new OpenCode conversation");

        // Kept, and back in the next run: none started until shown.
        let saved = sessions.to_saved();
        assert_eq!(saved.sessions.len(), 3);
        assert_eq!(saved.sessions[0].name.as_deref(), Some("billing export"));
        let mut next = Sessions::default();
        next.restore(&saved);
        assert!(next.list.iter().all(Session::waiting));
        assert_eq!(next.active, 2, "the one on screen");
        assert_eq!(next.list[0].launch_args("claude", |_| true), ["--resume", id.as_str()]);
        assert_eq!(next.list[0].launch_args("claude", |_| false), ["--session-id", id.as_str()], "never typed into: started under its id");
        assert_eq!(next.list[1].launch_args("opencode", |_| true), ["--continue"], "its id unseen: the folder's last");
        assert!(next.list[2].launch_args("bash", |_| true).is_empty());
        next.select(0);
        assert_eq!(next.list[0].pane.state, PaneState::Starting, "shown, it takes its conversation up");
        assert!(next.list[1].waiting(), "the others wait");

        // OpenCode's id once seen; a conversation from another terminal, still open there, as a
        // copy of it.
        next.list[1].conversation = Some("ses_9".into());
        assert_eq!(next.to_saved().sessions[1].conversation.as_deref(), Some("ses_9"));
        let elsewhere = Conversation {
            kind: Kind::Claude,
            id: "abc".into(),
            dir: PathBuf::from("/work/x"),
            title: "late partitions".into(),
            last: None,
            updated: 0,
            open: true,
        };
        next.open_conversation(&elsewhere).unwrap();
        let taken = next.current().unwrap();
        assert_eq!((taken.dir.as_str(), taken.pane.state.clone()), ("/work/x", PaneState::Starting));
        assert_eq!(taken.launch_args("claude", |_| true), ["--resume", "abc", "--fork-session"]);
        let opencode = Conversation { kind: Kind::OpenCode, id: "ses_1".into(), open: false, ..elsewhere };
        next.open_conversation(&opencode).unwrap();
        assert_eq!(next.current().unwrap().launch_args("opencode", |_| true), ["--session", "ses_1"]);
    }

    #[test]
    fn a_session_runs_what_its_kind_says_and_goes_by_what_it_is_on() {
        let commands = Commands { opencode: vec!["opencode".into(), "--model".into(), "x/y".into()], ..Commands::default() };
        let mut sessions = Sessions { default_dir: "~/work/cobserve".into(), commands, ..Sessions::default() };
        assert_eq!(sessions.next_kind(), Kind::Claude, "with nothing open, Claude");
        sessions.open_new("", Kind::OpenCode).unwrap();
        assert_eq!(sessions.next_kind(), Kind::OpenCode, "a new one runs what the one on screen runs");
        assert_eq!(sessions.command_of(Kind::OpenCode), ["opencode", "--model", "x/y"]);
        assert_eq!(sessions.command_of(Kind::Terminal), ["sh"]);
        // OpenCode's title before a task is only its name; a shell's says what runs in it.
        sessions.current_mut().unwrap().pane.feed(b"\x1b]0;OpenCode\x07", true);
        assert_eq!(sessions.current().unwrap().label(), "cobserve");
        sessions.open_new("", Kind::Terminal).unwrap();
        sessions.current_mut().unwrap().pane.feed(b"\x1b]0;vim notes.md\x07", true);
        assert_eq!(sessions.current().unwrap().label(), "vim notes.md");
        assert_eq!(Kind::ALL.map(Kind::glyph), ["✻", "▣", "❯", "▦"]);
        assert_eq!(Kind::ALL.map(Kind::next), [Kind::OpenCode, Kind::Terminal, Kind::Query, Kind::Claude]);
    }

    #[test]
    fn the_wheel_scrolls_a_shell_back_and_gives_a_full_screen_program_its_arrows() {
        let mut sessions = Sessions::default();
        sessions.open_new("", Kind::Terminal).unwrap();
        let pane = &mut sessions.current_mut().unwrap().pane;
        pane.state = PaneState::Running;
        pane.resize(10, 40);
        let output: String = (1..=50).map(|n| format!("line {n}\r\n")).collect();
        pane.feed(output.as_bytes(), true);
        sessions.pane_origin.set((0, 0));
        sessions.mouse(&mouse(MouseEventKind::ScrollUp, 5, 5));
        sessions.mouse(&mouse(MouseEventKind::ScrollUp, 5, 5));
        let pane = &mut sessions.current_mut().unwrap().pane;
        assert_eq!(pane.scrolled_back(), 6, "three lines a tick, as a terminal scrolls");
        assert!(pane.take_outbox().is_empty(), "nothing typed into the shell — no stray ~");
        assert!(pane.screen().contents().starts_with("line 36\n"), "six lines up from line 42: {}", pane.screen().contents());
        pane.key(&key(KeyCode::Char('l'), KeyModifiers::NONE), Kind::Terminal);
        assert_eq!(pane.scrolled_back(), 0, "a key goes back down");
        pane.take_outbox();
        // less, vim: a screen of their own, which the arrows move.
        pane.feed(b"\x1b[?1049h", true);
        sessions.mouse(&mouse(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(sessions.current_mut().unwrap().pane.take_outbox(), b"\x1b[B");
    }

    #[test]
    fn sessions_open_switch_rename_and_close() {
        let mut sessions = Sessions::default();
        sessions.want_size.set((30, 100));
        sessions.default_dir = "~/work/cobserve".into();
        let first = sessions.open_new("", Kind::Claude).unwrap();
        let second = sessions.open_new(" ~/work/airflow ", Kind::Claude).unwrap();
        assert_eq!(sessions.list[0].dir, "~/work/cobserve", "the monitor's own directory by default");
        assert_eq!((sessions.list[1].dir.as_str(), sessions.list[1].dir_label().as_str()), ("~/work/airflow", "airflow"));
        assert_eq!(sessions.next_dir(), "~/work/airflow", "a new one starts where the one on screen works");
        assert_ne!(first, second);
        assert_eq!(sessions.active, 1, "a new session is put on screen");
        assert_eq!(sessions.current().unwrap().pane.state, PaneState::Starting);
        assert_eq!(sessions.current().unwrap().pane.screen().size(), (30, 100));

        // Unnamed, a tab says what Claude is on; named, what the user said.
        sessions.current_mut().unwrap().pane.feed(b"\x1b]0;\xe2\x9c\xb3 Tidy the README\x07", true);
        assert_eq!(sessions.current().unwrap().label(), "Tidy the README");
        sessions.rename_current("  infra on-call  ");
        assert_eq!(sessions.current().unwrap().label(), "infra on-call");
        sessions.rename_current(&"x".repeat(40));
        assert_eq!(sessions.current().unwrap().label().len(), NAME_MAX);
        sessions.rename_current("");
        assert_eq!(sessions.current().unwrap().label(), "Tidy the README", "an empty name gives the default back");
        assert_eq!(sessions.list[0].label(), "cobserve", "no title yet: the folder");
        sessions.list[0].pane.feed(b"\x1b]0;\xe2\x9c\xb3 Claude Code\x07", true);
        assert_eq!(sessions.list[0].label(), "cobserve", "Claude's title before a task is only its name");
        // Numbered on from the views: the first session is tab 5.
        assert_eq!((Sessions::number_of(0), Sessions::number_of(1)), (5, 6));
        assert_eq!((sessions.index_of(5), sessions.index_of(6), sessions.index_of(7), sessions.index_of(4)), (Some(0), Some(1), None, None));

        sessions.step(true);
        assert_eq!(sessions.active, 0, "round the bar");
        sessions.step(false);
        assert_eq!(sessions.active, 1);

        // A bell on the session not on screen marks it; looking at it clears it.
        sessions.by_id_mut(first).unwrap().pane.feed(b"\x07", false);
        assert!(sessions.calling());
        sessions.select(0);
        assert!(!sessions.calling());

        sessions.close_current();
        assert_eq!(sessions.list.len(), 1);
        assert_eq!(sessions.current().unwrap().id, second);
        sessions.close_current();
        assert!(sessions.current().is_none());
        for _ in 0..MAX_SESSIONS {
            sessions.open_new("", Kind::Claude).unwrap();
        }
        assert!(sessions.open_new("", Kind::Claude).is_none(), "fifty at most");
        assert_eq!(sessions.list.len(), 50);
        assert_eq!((sessions.index_of(54), sessions.index_of(55)), (Some(49), None), "numbered on to 54");
    }

    #[test]
    fn a_session_is_found_by_its_name_folder_kind_server_or_number() {
        let mut sessions = Sessions::default();
        sessions.open_new("~/work/billing", Kind::Claude);
        sessions.open_new("~/work/reports", Kind::Terminal);
        sessions.open_query(Some("clickhouse3"));
        sessions.list[0].name = Some("invoice export".into());
        assert_eq!(sessions.matching(""), [0, 1, 2]);
        assert_eq!(sessions.matching("INVOICE"), [0]);
        assert_eq!(sessions.matching("reports"), [1]);
        assert_eq!(sessions.matching("terminal"), [1]);
        assert_eq!(sessions.matching("house3"), [2]);
        assert_eq!(sessions.matching("6"), [1], "by its number");
        assert!(sessions.matching("nothing like it").is_empty());
    }
}
