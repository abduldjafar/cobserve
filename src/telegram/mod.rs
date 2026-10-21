//! The Telegram bot: cobserve on the phone, both ways. Asked anything, Claude Code answers from
//! a snapshot of what the screen knows — the fleet, the queries running, the Redash queue,
//! Airflow, Jira, the tape — and the conversation so far; asked to stop something, it proposes,
//! and a card with a button does it: `KILL QUERY` on a node, a Redash job cancelled. Unasked, the
//! bot says when something goes red.
//!
//! Pure, as `App` is: what comes in (`Incoming`, answers, results) goes through [`Bot`], and what
//! is to be done comes out as [`Out`] (messages for Telegram) and [`Work`] (Claude asked, a query
//! killed) for `main.rs` to send and run. `sources/telegram.rs` speaks to Telegram; `look.rs` is
//! what the messages look like, `html.rs` how text becomes Telegram's HTML.
//!
//! What leaves the machine: numbers, names of nodes, users, people, DAGs and tickets — never a
//! query's SQL to Claude; a card about to kill a query shows the first words of its SQL, in
//! Telegram only. Only the chats `TELEGRAM_CHAT_ID` names are answered, and nothing is killed or
//! cancelled without a press on the card's button, within a few minutes of the card.

pub mod html;
pub mod look;

use crate::app::App;
use crate::fmt;
use crate::model::JobState;
use crate::severity::Severity;
use html::esc;
use std::collections::{HashMap, VecDeque};

/// What Claude is told it is for, after Claude Code's own system prompt.
pub const RULES: &str = "You are cobserve's assistant on Telegram. cobserve watches Paysera's \
ClickHouse fleet, its Redash queue, Airflow and Jira for its owner, a data engineer, and you answer \
them from the snapshot of cobserve's screen given with every message — never from memory, never \
made up. When the snapshot does not say, say so, and say what would tell.
- Reply in the language of the person's own messages (often Indonesian); for a button's request \
(an alert to explain), in the language of their last question, else Indonesian.
- It is read on a phone, so short. Shape: line 1 is the answer in one short sentence (under 90 \
characters) with the number that decides it. Then two to four lines starting with \"• \", ONE fact \
each, under 80 characters, with its number and what it is of (memory 58.2/64 GiB · 91%) — split a \
long fact in two or drop it; no parentheses packed with numbers, no clauses chained with commas. \
Then, if there is something to do, one line starting with \"→ \", under 100 characters. Nothing \
else: no greeting, no heading, no closing line, no paragraph.
- Markup, only these: **bold** around the one name or number a line is about; `code` around query \
ids, job ids, DAG ids and ticket keys, never around prose; \"• \" and \"→ \" at line starts; \"> \" \
at a line start to quote a line of the snapshot. No Markdown headings, tables, links, code fences \
or italics.
- Severity marks, right after \"• \": 🔴 where the snapshot marks it ✖, 🟠 where ▲, 🟢 when it \
has recovered, nothing otherwise. No other emoji.
- Numbers as the snapshot gives them, every percentage with what it is of. Name nodes, people and \
DAGs exactly as the snapshot does.
- You run nothing yourself. When the person asks to stop something — or a runaway query is the \
plain answer to what they ask about — propose it, each on a line of its own at the very end, \
exactly:
ACTION kill <node> <query_id>
ACTION cancel <redash_job_id>
copying node, query_id and job id from the snapshot. `kill` is KILL QUERY on that ClickHouse node; \
`cancel` cancels a Redash job. The person gets a card with a button for each and nothing happens \
until they press it, so say what each would do — never that it is done. A Redash job that runs on \
ClickHouse goes on running there after a cancel: when the snapshot names its node and query_id, \
propose that kill too, or instead. Propose nothing the person did not ask for.
- The SQL of queries is left out of the snapshot on purpose; do not ask for it.";

/// A card's button works this long.
pub const CARD_S: i64 = 300;
/// A message older than this when cobserve starts came while it was not running: answering it
/// now could act on what was asked about something long gone.
const LATE_S: i64 = 120;
/// Exchanges remembered, and how much of an answer.
const REMEMBERED: usize = 8;
const ANSWER_KEPT: usize = 1200;
/// Telegram's limit is 4096 characters a message; the markup grows a little into HTML.
const MESSAGE_MAX: usize = 3500;
/// Alerts wait this long after the last, gathering what comes meanwhile into one message.
const ALERT_GAP_S: i64 = 30;
/// An issue is told only once it has gone on this long — what clears sooner is not news.
pub const PERSIST_S: i64 = 600;
/// An issue missing from the screen this long is over; a shorter gap is a flicker, and its
/// clock goes on.
const GONE_S: i64 = 90;
/// How long after a kill its query is watched for, to say it is gone.
const CONFIRM_S: i64 = 60;
const ACTIONS_MAX: usize = 5;

/// What comes from Telegram.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Message { chat: i64, text: String, at: i64 },
    /// A card's button: the message it is on, the callback to answer, what the button carries.
    Button { chat: i64, message: i64, callback: String, data: String },
}

pub type Buttons = Vec<(String, String)>;

/// What goes to Telegram: HTML, built with `html::esc`.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Send { chat: i64, text: String, buttons: Buttons },
    /// The message's new text; its buttons go unless given again.
    Edit { chat: i64, message: i64, text: String, buttons: Buttons },
    /// The toast a button press gets: plain text.
    Answer { callback: String, text: String },
    Typing { chat: i64 },
}

/// What reaches the bot: Telegram's updates, Claude's answers, the results of what it ran.
#[derive(Debug, Clone)]
pub enum BotEvent {
    Incoming(Incoming),
    Answered { chat: i64, question: String, result: Result<String, String> },
    Killed { chat: i64, message: i64, result: Result<String, String> },
    /// Telegram could not be reached, or refused: for the screen's notice line.
    Trouble(String),
}

/// What `main.rs` runs for the bot.
#[derive(Debug, Clone, PartialEq)]
pub enum Work {
    /// Claude Code asked `prompt`, under [`RULES`].
    Ask { chat: i64, question: String, prompt: String },
    /// `KILL QUERY` of `query_id` on `node`; the card's `message` gets the result.
    Kill { chat: i64, message: i64, node: String, query_id: String },
}

/// What a card does when its button is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Kill { node: String, query_id: String },
    Cancel { job: String },
    /// One of a browser's tab or extension processes, by its pid.
    Stop { pid: u32 },
}

/// How much the bot says unasked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alerts {
    Off,
    /// Red only — the default.
    Red,
    /// Amber and red.
    All,
}

#[derive(Debug, Clone)]
struct Card {
    action: Action,
    chat: i64,
    made: i64,
    facts: look::Facts,
}

/// Something wrong, watched until it has gone on for [`PERSIST_S`] — then told — or is over.
#[derive(Debug, Clone)]
struct Issue {
    group: look::Group,
    level: Severity,
    label: String,
    text: String,
    since: i64,
    seen: i64,
    told: bool,
}

/// Something wrong on the screen now: a key, where it goes, how bad, its name, what it is, and
/// since when, when the screen knows.
type Seen = (String, look::Group, Severity, String, String, Option<i64>);

/// What is wrong now on the fleet, on Airflow and on this Mac; `None` for a source not read.
type IssuesNow = (Option<Vec<Seen>>, Option<Vec<Seen>>, Option<Vec<Seen>>);

fn issues_now(app: &App, now: i64) -> IssuesNow {
    let fleet = app.with_view(|_| ()).map(|_| {
        app.insights()
            .into_iter()
            .filter(|i| i.level.is_problem())
            .map(|i| {
                let text: String = i.parts.iter().map(|(t, _)| t.as_str()).collect();
                let text = match text.rfind("  +") {
                    Some(at) if text.ends_with(" more") => text[..at].to_string(),
                    _ => text,
                };
                (format!("i:{}", i.label), look::group_of(&i.subject), i.level, i.label.clone(), text, None)
            })
            .collect()
    });
    let airflow = app.airflow.day_read.then(|| {
        let mut latest: HashMap<&str, &crate::airflow::Run> = HashMap::new();
        for run in app.airflow.runs.iter().filter(|r| !r.state.is_live()) {
            let newer = latest.get(run.dag.as_str()).is_none_or(|l| run.at() > l.at());
            if newer {
                latest.insert(&run.dag, run);
            }
        }
        let mut out = Vec::new();
        // A DAG whose last run failed — until a run of it goes well.
        for (dag, run) in latest {
            if run.state == crate::airflow::RunState::Failed {
                let task = app.airflow.progress.get(&(run.dag.clone(), run.id.clone())).and_then(|p| p.failed.first().cloned());
                let text = match task {
                    Some(task) => format!("last run failed at {task} · {} run", run.kind),
                    None => format!("last run failed · {} run", run.kind),
                };
                out.push((format!("a:{dag}"), look::Group::Airflow, Severity::Crit, dag.to_string(), text, run.end.or(run.at())));
            }
        }
        // A run going on far too long.
        for run in app.airflow.runs.iter().filter(|r| r.state == crate::airflow::RunState::Running && r.severity(now).is_problem()) {
            let took = run.took(now).map(|s| fmt::dur(s as f64)).unwrap_or_default();
            let word = if run.is_stuck(now) { "stuck" } else { "running long" };
            out.push((format!("r:{}:{}", run.dag, run.id), look::Group::Airflow, run.severity(now), run.dag.clone(), format!("{word} · {took}"), run.start));
        }
        out
    });
    // This Mac: a browser held greedy (two reads over its line, as the tape says it).
    let mac = app.local.latest.is_some().then(|| {
        app.local
            .browsers
            .iter()
            .filter_map(|b| {
                let level = app.greed_of(&b.program);
                if !level.is_problem() {
                    return None;
                }
                let share = crate::browser::share(&app.local, b)?;
                let total = app.local.latest.as_ref().and_then(|s| s.memory).map_or(String::new(), |m| format!(" of {:.0} GiB", fmt::gib(m.total)));
                let tabs = b.parts.iter().find(|p| p.part == crate::local::Part::Tab).map_or(String::new(), |t| format!(" · {} tabs hold {}", t.count, fmt::bytes(t.rss)));
                Some((format!("l:{}", b.program), look::Group::Mac, level, b.program.clone(), format!("{} RSS · {:.1}%{total}{tabs}", fmt::bytes(b.rss), share), None))
            })
            .collect()
    });
    (fleet, airflow, mac)
}

/// A killed query watched until it leaves the list.
#[derive(Debug, Clone)]
struct Confirm {
    killing: look::Killing,
    text: String,
    made: i64,
    since: i64,
}


pub struct Bot {
    chats: Vec<i64>,
    started: i64,
    next: u64,
    cards: HashMap<u64, Card>,
    /// `Kill it on … too`, offered after a cancel: the chat, the kill and when.
    follows: HashMap<u64, (i64, Action, i64)>,
    /// Kills sent, by their card's message, for the result to be written under.
    kills: HashMap<(i64, i64), (look::Facts, i64)>,
    confirming: HashMap<(i64, i64), Confirm>,
    /// Redash jobs being cancelled from a card, and the card.
    cancelling: HashMap<String, (i64, i64, look::Facts)>,
    /// Alerts kept for their `explain` button, as Claude reads them.
    explains: HashMap<u64, String>,
    history: VecDeque<(String, String)>,
    /// The chat Claude is answering, since when, and what.
    asking: Option<(i64, i64, String)>,
    typed: i64,
    pub alerts: Alerts,
    primed: bool,
    /// What is wrong now, by what it is about, and since when.
    issues: HashMap<String, Issue>,
    owed: Vec<look::Line>,
    alerted: i64,
    pub out: Vec<Out>,
    pub work: Vec<Work>,
    /// For the screen's notice line.
    pub notices: Vec<String>,
    /// Tab processes being stopped from a card, by the card's message.
    stopping: HashMap<(i64, i64), look::Facts>,
    /// The chat that asked `/chrome` while its tabs were being read: they follow when read.
    chrome_waiting: Option<i64>,
}

impl Bot {
    pub fn new(chats: Vec<i64>, now: i64) -> Bot {
        Bot {
            chats,
            started: now,
            next: 1,
            cards: HashMap::new(),
            follows: HashMap::new(),
            kills: HashMap::new(),
            confirming: HashMap::new(),
            cancelling: HashMap::new(),
            explains: HashMap::new(),
            history: VecDeque::new(),
            asking: None,
            typed: 0,
            alerts: Alerts::Red,
            primed: false,
            issues: HashMap::new(),
            owed: Vec::new(),
            alerted: 0,
            out: Vec::new(),
            work: Vec::new(),
            notices: Vec::new(),
            stopping: HashMap::new(),
            chrome_waiting: None,
        }
    }

    fn allowed(&self, chat: i64) -> bool {
        self.chats.contains(&chat)
    }

    fn send(&mut self, chat: i64, html: impl Into<String>) {
        self.out.push(Out::Send { chat, text: html.into(), buttons: Vec::new() });
    }

    fn edit(&mut self, chat: i64, message: i64, html: String) {
        self.out.push(Out::Edit { chat, message, text: html, buttons: Vec::new() });
    }

    fn toast(&mut self, callback: String, text: &str) {
        self.out.push(Out::Answer { callback, text: text.to_string() });
    }

    fn token(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn on_event(&mut self, app: &mut App, event: BotEvent, now: i64) {
        match event {
            BotEvent::Incoming(Incoming::Message { chat, text, at }) => self.on_message(app, chat, text.trim(), at, now),
            BotEvent::Incoming(Incoming::Button { chat, message, callback, data }) => self.on_button(app, chat, message, callback, &data, now),
            BotEvent::Answered { chat, question, result } => self.on_answer(app, chat, question, result, now),
            BotEvent::Killed { chat, message, result } => self.on_killed(app, chat, message, result, now),
            BotEvent::Trouble(text) => self.notices.push(format!("Telegram: {text}")),
        }
    }

    fn on_message(&mut self, app: &mut App, chat: i64, text: &str, at: i64, now: i64) {
        if !self.allowed(chat) {
            if self.chats.is_empty() {
                self.send(chat, look::stranger(chat));
            }
            self.notices.push(format!("Telegram: a message from chat {chat}, which TELEGRAM_CHAT_ID does not name — not answered"));
            return;
        }
        if at < self.started - LATE_S {
            self.send(chat, look::late(text, now - at));
            return;
        }
        let (command, rest) = text.split_once(char::is_whitespace).map_or((text, ""), |(c, r)| (c, r.trim()));
        // `/status@cobserve_bot` in a group.
        let command = command.split('@').next().unwrap_or(command).to_ascii_lowercase();
        match command.as_str() {
            "/start" | "/help" => self.send(chat, look::HELP),
            "/status" => self.out.push(Out::Send { chat, text: look::status(app, now, look::Tab::Home), buttons: look::tabs(app, look::Tab::Home) }),
            "/forget" => {
                self.history.clear();
                self.send(chat, look::FORGOTTEN);
            }
            "/alerts" => {
                self.alerts = match rest.to_ascii_lowercase().as_str() {
                    "off" => Alerts::Off,
                    "all" => Alerts::All,
                    _ => Alerts::Red,
                };
                self.send(chat, look::alerts_set(self.alerts));
            }
            "/kill" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
                [node, query_id] => self.propose(app, chat, Action::Kill { node: node.to_string(), query_id: query_id.to_string() }, now),
                _ => self.send(chat, look::KILL_USAGE),
            },
            "/chrome" => self.chrome(app, chat, if rest.is_empty() { "Google Chrome" } else { rest }, now),
            "/cancel" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
                [job] => self.propose(app, chat, Action::Cancel { job: job.to_string() }, now),
                _ => self.send(chat, look::CANCEL_USAGE),
            },
            _ if command.starts_with('/') => self.send(chat, look::HELP),
            _ => self.ask(app, chat, text.to_string(), text.to_string(), now),
        }
    }

    /// Claude asked `question`; `shown` is how the bot speaks of it while it waits — what the
    /// person typed, or what the button they pressed is called.
    fn ask(&mut self, app: &App, chat: i64, question: String, shown: String, now: i64) {
        if let Some((_, since, asked)) = &self.asking {
            let line = look::thinking(asked, now - since);
            self.send(chat, line);
            return;
        }
        let prompt = prompt(app, &self.history, &question, now);
        self.asking = Some((chat, now, shown));
        self.typed = now;
        self.out.push(Out::Typing { chat });
        self.work.push(Work::Ask { chat, question, prompt });
    }

    fn on_answer(&mut self, app: &App, chat: i64, question: String, result: Result<String, String>, now: i64) {
        self.asking = None;
        match result {
            Ok(said) => {
                let (reply, actions) = parse_answer(&said);
                if reply.is_empty() && actions.is_empty() {
                    self.send(chat, look::SAID_NOTHING);
                    return;
                }
                for part in split(&reply) {
                    self.send(chat, html::answer(&part));
                }
                self.history.push_back((question, fmt::truncate(&reply, ANSWER_KEPT)));
                while self.history.len() > REMEMBERED {
                    self.history.pop_front();
                }
                for action in actions {
                    self.propose(app, chat, action, now);
                }
            }
            Err(why) => self.send(chat, look::claude_failed(&why)),
        }
    }

    /// A card for `action`, if what it is about is still there.
    fn propose(&mut self, app: &App, chat: i64, action: Action, now: i64) {
        let facts = match &action {
            Action::Kill { node, query_id } => {
                if !query_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')) {
                    self.send(chat, format!("❌ <b>No card</b> · “{}” is not a query id I would send to a server", esc(query_id)));
                    return;
                }
                look::kill_card(app, node, query_id)
            }
            Action::Cancel { job } => look::cancel_card(app, job),
            Action::Stop { pid } => look::stop_card(app, *pid),
        };
        match facts {
            Ok(facts) => {
                // The node as the screen names it, whatever the card was asked with.
                let action = match (&action, &facts.killing) {
                    (Action::Kill { .. }, Some(k)) => Action::Kill { node: k.node.clone(), query_id: k.query_id.clone() },
                    _ => action,
                };
                let token = self.token();
                let yes = match action {
                    Action::Kill { .. } => "🛑 Kill it",
                    Action::Cancel { .. } => "🛑 Cancel job",
                    Action::Stop { .. } => "🛑 Stop it",
                };
                self.out.push(Out::Send {
                    chat,
                    text: format!("{}\n\n{}", facts.html, look::HINT),
                    buttons: vec![(yes.to_string(), format!("y:{token}")), ("Leave it".to_string(), format!("n:{token}"))],
                });
                self.cards.insert(token, Card { action, chat, made: now, facts });
            }
            Err(why) => self.send(chat, why),
        }
    }

    fn on_button(&mut self, app: &mut App, chat: i64, message: i64, callback: String, data: &str, now: i64) {
        if !self.allowed(chat) {
            self.toast(callback, "Not for this chat");
            return;
        }
        // /status's tabs and Refresh, in place; an alert's Status, a new one.
        let tab = look::Tab::of(data).or_else(|| data.strip_prefix("r:").and_then(look::Tab::of));
        if let Some(tab) = tab {
            self.toast(callback, if data.starts_with("r:") { "Refreshed" } else { "" });
            self.out.push(Out::Edit { chat, message, text: look::status(app, now, tab), buttons: look::tabs(app, tab) });
            return;
        }
        if data == "C" {
            self.toast(callback, "");
            self.chrome(app, chat, "Google Chrome", now);
            return;
        }
        if data == "S" {
            self.toast(callback, "");
            self.out.push(Out::Send { chat, text: look::status(app, now, look::Tab::Home), buttons: look::tabs(app, look::Tab::Home) });
            return;
        }
        let Some((kind, token)) = data.split_once(':').and_then(|(k, t)| Some((k, t.parse::<u64>().ok()?))) else {
            self.toast(callback, "");
            return;
        };
        match kind {
            "e" => {
                if self.asking.is_some() {
                    self.toast(callback, "Still thinking — a moment");
                    return;
                }
                self.toast(callback, "");
                match self.explains.get(&token).cloned() {
                    Some(text) => {
                        let question = format!("Explain these alerts: what happened, how bad it is, what to do.\n{text}");
                        self.ask(app, chat, question, "explaining the alert".to_string(), now);
                    }
                    None => self.send(chat, look::EXPLAIN_GONE),
                }
                return;
            }
            "k" => {
                match self.follows.remove(&token) {
                    Some((owner, action, _)) if owner == chat => {
                        self.toast(callback, "On it");
                        self.propose(app, chat, action, now);
                    }
                    _ => self.toast(callback, "Already used"),
                }
                return;
            }
            _ => {}
        }
        let Some(card) = self.cards.remove(&token) else {
            self.toast(callback, "Already used");
            return;
        };
        if card.chat != chat {
            self.toast(callback, "Not for this chat");
            return;
        }
        let facts = card.facts.html.clone();
        let again = esc(&card.facts.again);
        if now - card.made > CARD_S {
            self.toast(callback, "Too old — ask again");
            let line = format!(
                "⌛ <b>Not done</b> · this card is {} minutes old, buttons work for {} · <i>{again} makes a fresh one</i>",
                (now - card.made) / 60,
                CARD_S / 60
            );
            self.edit(chat, message, format!("{facts}\n\n{line}"));
            return;
        }
        if kind != "y" {
            self.toast(callback, "Left alone");
            let still = match &card.action {
                Action::Kill { node, query_id } => app
                    .with_view(|v| v.nodes.iter().find(|n| n.name() == node)?.node.queries.iter().find(|q| q.query_id == *query_id).map(|q| format!(" · still running {}", fmt::dur(q.elapsed_s))))
                    .flatten()
                    .unwrap_or_default(),
                Action::Cancel { .. } => String::new(),
                Action::Stop { pid } => app.local.browsers.iter().find_map(|b| b.heaviest.iter().find(|h| h.pid == *pid)).map(|h| format!(" · still {}", fmt::bytes(h.rss))).unwrap_or_default(),
            };
            self.edit(chat, message, format!("{facts}\n\n— <b>Left alone</b>{still} · <i>{again} if you change your mind</i>"));
            return;
        }
        // Checked again: the screen may have moved on since the card was made.
        let gone = match &card.action {
            Action::Kill { node, query_id } => look::kill_card(app, node, query_id)
                .err()
                .map(|_| format!("✅ <b>Already gone</b> · <code>{}</code> is not running on {} any more — nothing to kill", esc(query_id), esc(&look::short(app, node)))),
            Action::Cancel { job } => look::cancel_card(app, job)
                .err()
                .map(|_| format!("✅ <b>Already gone</b> · Redash job <code>{}</code> is not in the queue any more — nothing to cancel", esc(job))),
            Action::Stop { pid } => look::stop_card(app, *pid)
                .err()
                .map(|_| format!("✅ <b>Already gone</b> · pid {pid} is not one of Chrome's processes any more — nothing to stop")),
        };
        if let Some(line) = gone {
            self.toast(callback, "It is gone already");
            self.edit(chat, message, format!("{facts}\n\n{line}"));
            return;
        }
        self.toast(callback, "On it");
        match card.action {
            Action::Kill { node, query_id } => {
                self.edit(chat, message, format!("{facts}\n\n⏳ KILL QUERY sent to {} …", esc(&look::short(app, &node))));
                self.notices.push(format!("Telegram: KILL QUERY {query_id} on {node}, pressed on the phone"));
                self.kills.insert((chat, message), (card.facts, now));
                self.work.push(Work::Kill { chat, message, node, query_id });
            }
            Action::Cancel { job } => {
                self.edit(chat, message, format!("{facts}\n\n⏳ Asking Redash to cancel it …"));
                app.cancel_from_elsewhere(&job);
                self.cancelling.insert(job, (chat, message, card.facts));
            }
            Action::Stop { pid } => {
                let helper = app.local.browsers.iter().find_map(|b| b.heaviest.iter().find(|h| h.pid == pid)).cloned();
                if let Some(helper) = helper {
                    self.edit(chat, message, format!("{facts}\n\n⏳ Stopping pid {pid} …"));
                    self.notices.push(format!("Telegram: stop {pid} ({} process), pressed on the phone", crate::browser::helper_word(helper.part)));
                    self.stopping.insert((chat, message), card.facts);
                    app.ask_stop(helper, crate::app::Asked::Phone { chat, message });
                }
            }
        }
    }

    /// `/chrome`: the browser's memory and tabs, a stop button for each of its heaviest tab
    /// processes. Tabs older than 30 s are read again and follow in a message of their own.
    fn chrome(&mut self, app: &mut App, chat: i64, program: &str, now: i64) {
        let (text, heavy) = look::chrome(app, now, program);
        let fresh = app.tabs.as_ref().is_some_and(|t| t.program == program && (now as f64 - t.at) < crate::browser::TABS_FRESH_S);
        if app.local.browser(program).is_some() && !fresh {
            app.tabs_for_phone = true;
            self.chrome_waiting = Some(chat);
        }
        let mut buttons = Vec::new();
        for h in heavy {
            let token = self.token();
            buttons.push((format!("🛑 Stop {} · {:.0} MiB", h.pid, h.rss as f64 / 1_048_576.0), format!("k:{token}")));
            buttons.push((String::new(), String::new()));
            self.follows.insert(token, (chat, Action::Stop { pid: h.pid }, now));
        }
        buttons.push(("↻ Refresh".into(), "C".into()));
        buttons.push(("‹ Overview".into(), "S".into()));
        self.out.push(Out::Send { chat, text, buttons });
    }

    /// The tabs `/chrome` was waiting for have been read: they follow in a message.
    pub fn on_tabs(&mut self, app: &mut App) {
        app.tabs_for_phone = false;
        let Some(chat) = self.chrome_waiting.take() else { return };
        let Some(read) = app.tabs.as_ref() else { return };
        let text = match &read.result {
            Ok(tabs) => {
                let mut text = format!("🗂 <b>{} tabs</b> · {} windows · {} sites\n", tabs.len(), read.windows(), read.sites.len());
                for site in read.sites.iter().take(8) {
                    text.push_str(&format!("{:>2} {}\n", site.tabs.len(), esc(&site.host)));
                }
                if read.sites.len() > 8 {
                    text.push_str(&format!("<i>+{} sites</i>\n", read.sites.len() - 8));
                }
                text.push_str("<i>a site's tabs share one process · which, only Chrome's Task Manager says</i>");
                text
            }
            Err(e) => format!("<i>tabs: {}</i>", esc(e)),
        };
        self.send(chat, text);
    }

    /// What came of a stop a card asked for.
    pub fn on_stopped(&mut self, stop: &crate::app::Stop, result: &Result<String, String>) {
        let crate::app::Asked::Phone { chat, message } = stop.asked else { return };
        let Some(facts) = self.stopping.remove(&(chat, message)) else { return };
        match result {
            Ok(_) => self.notices.push(format!("Telegram: ✔ stopped {}", stop.helper.pid)),
            Err(e) => self.notices.push(format!("Telegram: ✖ not stopped: {e}")),
        }
        self.edit(chat, message, format!("{}\n\n{}", facts.html, look::stopped(stop, result)));
    }

    fn on_killed(&mut self, app: &App, chat: i64, message: i64, result: Result<String, String>, now: i64) {
        let Some((facts, _)) = self.kills.remove(&(chat, message)) else {
            return;
        };
        match result {
            Ok(how) => {
                self.notices.push(format!("Telegram: ✔ {how}"));
                let Some(killing) = facts.killing else {
                    return;
                };
                let text = format!("{}\n\n{}", facts.html, look::killed(app, &how, &killing));
                self.edit(chat, message, text.clone());
                self.confirming.insert((chat, message), Confirm { killing, text, made: now, since: now });
            }
            Err(why) => {
                self.notices.push(format!("Telegram: ✖ not killed: {why}"));
                let line = format!("❌ <b>Not killed</b> — {}\n<i>{} asks again</i>", esc(&why), esc(&facts.again));
                self.edit(chat, message, format!("{}\n\n{line}", facts.html));
            }
        }
    }

    /// Redash's answer to a cancel a card asked for.
    pub fn on_cancelled(&mut self, app: &App, job: &str, result: &Result<(), String>) {
        let Some((chat, message, facts)) = self.cancelling.remove(job) else {
            return;
        };
        match result {
            Ok(()) => match facts.follow.clone() {
                Some((node, query_id, memory)) => {
                    let name = look::short(app, &node);
                    let mem = if memory > 0 { format!(" ({})", fmt::bytes(memory)) } else { String::new() };
                    let token = self.token();
                    self.follows.insert(token, (chat, Action::Kill { node, query_id: query_id.clone() }, app.now()));
                    self.out.push(Out::Edit {
                        chat,
                        message,
                        text: format!("{}\n\n✅ <b>Cancelled</b> in Redash · its query <code>{}</code> goes on on <b>{}</b>{mem}", facts.html, esc(&query_id), esc(&name)),
                        buttons: vec![(format!("🛑 Kill it on {name} too"), format!("k:{token}"))],
                    });
                }
                None => self.edit(chat, message, format!("{}\n\n✅ <b>Cancelled</b> in Redash", facts.html)),
            },
            Err(why) => self.edit(chat, message, format!("{}\n\n❌ <b>Not cancelled</b> — Redash said: {}", facts.html, esc(why))),
        }
    }

    /// Every second or so: the typing mark while Claude thinks, killed queries watched until
    /// they are gone, old cards let go, and alerts.
    pub fn tick(&mut self, app: &App, now: i64) {
        if let Some((chat, ..)) = self.asking
            && now - self.typed >= 5
        {
            self.typed = now;
            self.out.push(Out::Typing { chat });
        }
        let mut done = Vec::new();
        for (key, c) in &self.confirming {
            let running = app
                .with_view(|v| v.nodes.iter().any(|n| n.name() == c.killing.node && n.node.queries.iter().any(|q| q.query_id == c.killing.query_id)))
                .unwrap_or(true);
            if !running {
                let ran = c.killing.elapsed_s + (now - c.since) as f64;
                done.push((*key, Some(format!("{}\n{}", c.text, look::gone(app, &c.killing, ran)))));
            } else if now - c.made > CONFIRM_S {
                done.push((*key, None));
            }
        }
        for ((chat, message), text) in done {
            self.confirming.remove(&(chat, message));
            if let Some(text) = text {
                self.edit(chat, message, text);
            }
        }
        self.cards.retain(|_, card| now - card.made <= CARD_S * 4);
        self.follows.retain(|_, (_, _, made)| now - *made <= CARD_S * 4);
        self.watch(app, now);
    }

    /// What is wrong, kept with since when: told once it has gone on for [`PERSIST_S`], and
    /// again when it is over — what clears sooner is never told.
    fn watch(&mut self, app: &App, now: i64) {
        if !self.primed {
            // Nothing is watched until the fleet has been read once.
            if app.with_view(|_| ()).is_none() {
                return;
            }
            self.primed = true;
            for chat in self.chats.clone() {
                self.send(chat, look::up(app, self.alerts));
            }
        }
        let threshold = match self.alerts {
            Alerts::Off | Alerts::Red => Severity::Crit,
            Alerts::All => Severity::Warn,
        };
        let (fleet, airflow, mac) = issues_now(app, now);
        let started = self.started;
        // A source not read this time says nothing about its issues: they stay as they were.
        let unread: Vec<&str> = [(fleet.is_none(), "i:"), (airflow.is_none(), "a:"), (airflow.is_none(), "r:"), (mac.is_none(), "l:")].into_iter().filter(|(no, _)| *no).map(|(_, p)| p).collect();
        for issue in self.issues.iter_mut().filter(|(key, _)| unread.iter().any(|p| key.starts_with(p))) {
            issue.1.seen = now;
        }
        for (key, group, level, label, text, since) in fleet.into_iter().flatten().chain(airflow.into_iter().flatten()).chain(mac.into_iter().flatten()) {
            if level < threshold {
                continue;
            }
            let issue = self.issues.entry(key).or_insert_with(|| Issue { group, level, label: label.clone(), text: text.clone(), since: since.unwrap_or(now).clamp(started.min(now), now), seen: now, told: false });
            issue.seen = now;
            issue.level = issue.level.max(level);
            issue.text = text;
        }
        let over: Vec<String> = self.issues.iter().filter(|(_, i)| now - i.seen > GONE_S).map(|(k, _)| k.clone()).collect();
        let mut cleared = Vec::new();
        for key in over {
            if let Some(issue) = self.issues.remove(&key)
                && issue.told
            {
                cleared.push(look::cleared(issue.group, &issue.label, issue.seen - issue.since));
            }
        }
        let mut due: Vec<&mut Issue> = self.issues.values_mut().filter(|i| !i.told && now - i.since >= PERSIST_S).collect();
        due.sort_by_key(|i| (std::cmp::Reverse(i.level), i.since));
        let mut going = Vec::new();
        for issue in due {
            issue.told = true;
            going.push(look::ongoing(issue.group, issue.level, &issue.label, &issue.text, now - issue.since));
        }
        if self.alerts == Alerts::Off {
            return;
        }
        self.owed.extend(going);
        self.owed.extend(cleared);
        if self.owed.is_empty() || now - self.alerted < ALERT_GAP_S {
            return;
        }
        self.alerted = now;
        let lines: Vec<look::Line> = std::mem::take(&mut self.owed);
        let text = look::alerts(app, &lines, now);
        let token = self.token();
        self.explains.insert(token, lines.iter().map(|l| l.plain.clone()).collect::<Vec<_>>().join("\n"));
        if self.explains.len() > 30
            && let Some(oldest) = self.explains.keys().min().copied()
        {
            self.explains.remove(&oldest);
        }
        let mut buttons = if lines.iter().any(|l| l.ongoing) {
            vec![("🧠 Explain".to_string(), format!("e:{token}")), ("Status".to_string(), "S".to_string())]
        } else {
            Vec::new()
        };
        if lines.iter().any(|l| l.ongoing && l.group == look::Group::Mac) {
            buttons.push(("Chrome".to_string(), "C".to_string()));
        }
        for chat in self.chats.clone() {
            self.out.push(Out::Send { chat, text: text.clone(), buttons: buttons.clone() });
        }
    }
}

fn mark(sev: Severity) -> &'static str {
    match sev {
        Severity::Crit => "✖",
        Severity::Warn => "▲",
        _ => "·",
    }
}

/// A long text in Telegram-sized pieces, cut at lines where it can be.
pub fn split(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    for line in text.lines() {
        let mut line = line.to_string();
        while line.chars().count() > MESSAGE_MAX {
            let head: String = line.chars().take(MESSAGE_MAX).collect();
            line = line.chars().skip(MESSAGE_MAX).collect();
            if !part.is_empty() {
                parts.push(std::mem::take(&mut part));
            }
            parts.push(head);
        }
        if part.chars().count() + line.chars().count() + 1 > MESSAGE_MAX {
            parts.push(std::mem::take(&mut part));
        }
        if !part.is_empty() {
            part.push('\n');
        }
        part.push_str(&line);
    }
    if !part.trim().is_empty() {
        parts.push(part);
    }
    parts
}

/// Claude's answer without its `ACTION` lines, and the actions they propose.
pub fn parse_answer(said: &str) -> (String, Vec<Action>) {
    let mut actions = Vec::new();
    let mut kept = Vec::new();
    for line in said.lines() {
        let words: Vec<&str> = line.trim().trim_matches('`').split_whitespace().collect();
        match words[..] {
            [first, "kill", node, query_id] if first.eq_ignore_ascii_case("ACTION") => {
                actions.push(Action::Kill { node: node.to_string(), query_id: query_id.to_string() });
            }
            [first, "cancel", job] if first.eq_ignore_ascii_case("ACTION") => actions.push(Action::Cancel { job: job.to_string() }),
            _ => kept.push(line),
        }
    }
    actions.dedup();
    actions.truncate(ACTIONS_MAX);
    (kept.join("\n").trim().to_string(), actions)
}

/// What Claude is asked: the moment, the snapshot, the conversation so far, the question.
pub fn prompt(app: &App, history: &VecDeque<(String, String)>, question: &str, now: i64) -> String {
    let mut out = format!("Now: {}, {}.\n\n<snapshot>\n", app.time.format(now, "%A %Y-%m-%d %H:%M:%S"), app.time.label(now));
    out.push_str(&context(app, now));
    out.push_str("</snapshot>\n\n");
    if !history.is_empty() {
        out.push_str("The conversation so far:\n");
        for (asked, said) in history {
            out.push_str(&format!("Person: {asked}\nYou: {said}\n"));
        }
        out.push('\n');
    }
    out.push_str(&format!("Person: {question}\n"));
    out
}

/// What the screen knows, as text for Claude: every panel, the queries and the jobs without
/// their SQL.
pub fn context(app: &App, now: i64) -> String {
    let mut out = String::new();
    out.push_str("== Panels (✖ red, ▲ amber)\n");
    out.push_str(&crate::plain::summary(app));
    out.push('\n');

    let fleet = app.with_view(|view| {
        let mut s = String::from("== ClickHouse nodes\n");
        for n in &view.nodes {
            let node = n.node;
            if !node.reachable {
                s.push_str(&format!("{} · unreachable: {}\n", node.name, node.unreachable_reason.as_deref().unwrap_or("no answer")));
                continue;
            }
            let mem = match node.mem_total {
                Some(total) => format!("{} of {} · {}", fmt::bytes(node.mem_used), fmt::bytes(total), fmt::pct(n.mem_pct)),
                None => format!("{} of unknown", fmt::bytes(node.mem_used)),
            };
            let cpu = match (n.busy_cores, node.cores) {
                (Some(busy), Some(cores)) => format!("{busy:.1} of {cores:.0} cores · {}", fmt::pct(n.cpu_pct)),
                _ => "unknown".to_string(),
            };
            s.push_str(&format!(
                "{} · shard {} replica {} · ClickHouse {} · memory {mem} · cpu {cpu} · {} queries · replication lag {} s · {} active parts · up {} · memory limit a query {}{}\n",
                node.name,
                node.shard,
                node.replica,
                node.version,
                node.running,
                node.lag_s,
                node.active_parts,
                fmt::opt_dur(node.uptime_s),
                fmt::bytes(n.mem_limit),
                if n.runaways > 0 { format!(" · {} RUNAWAY", n.runaways) } else { String::new() }
            ));
        }
        let mut queries: Vec<(&str, &crate::model::QueryStat<'_>)> =
            view.nodes.iter().flat_map(|n| n.users.iter().flat_map(move |u| u.queries.iter().map(move |q| (n.name(), q)))).collect();
        queries.sort_by_key(|q| std::cmp::Reverse(q.1.query.memory_bytes));
        s.push_str(&format!("\n== Queries running ({} in all; the 40 using most memory) — node · query_id · user → person · what · time · memory · cores\n", queries.len()));
        for (node, q) in queries.iter().take(40) {
            let row = q.query;
            let who = crate::attrib::user_label(&row.user, row.person.as_deref());
            let redash = row.redash_query_id.map(|id| format!(" · Redash #{id}")).unwrap_or_default();
            let progress = q.progress.map(|p| format!(" · {:.0}% read", p * 100.0)).unwrap_or_default();
            s.push_str(&format!(
                "{node} · {} · {who}{redash} · {} · {} · {} of {} · {:.1} cores · read {} rows{progress}{}\n",
                row.query_id,
                row.kind.as_deref().unwrap_or("query"),
                fmt::dur(row.elapsed_s),
                fmt::bytes(row.memory_bytes),
                fmt::bytes(q.limit),
                q.cores,
                fmt::rows(row.read_rows),
                if q.runaway { " · RUNAWAY" } else { "" }
            ));
        }
        s
    });
    match fleet {
        Some(s) => out.push_str(&s),
        None => out.push_str("== ClickHouse: not read yet\n"),
    }

    let q = &app.queue;
    out.push_str("\n== Redash queue\n");
    if !q.reachable {
        out.push_str(&format!("unreachable: {}\n", q.error.as_deref().unwrap_or("no answer")));
    } else {
        out.push_str(&format!("workers {} busy of {}\n", q.workers_busy, q.workers_total));
        for row in &q.queues {
            out.push_str(&format!(
                "queue {} · {} running · {} waiting{} · {} stale · workers {} of {}\n",
                row.name,
                row.running,
                row.waiting,
                row.oldest_wait_s.map(|s| format!(" (oldest {})", fmt::dur(s as f64))).unwrap_or_default(),
                row.stale,
                row.workers_busy,
                row.workers_total
            ));
        }
        if !q.jobs.is_empty() {
            out.push_str("jobs — job id · state · queue · person · query · data source · age · where it runs on ClickHouse\n");
        }
        for j in q.jobs.iter().take(50) {
            let state = match j.state {
                JobState::Queued => "waiting".to_string(),
                JobState::Started => "running".to_string(),
                JobState::Stale(why) => format!("stale ({})", why.reason()),
            };
            let on = j.clickhouse_target().map(|(n, id)| format!(" · on {n} as {id}")).unwrap_or_default();
            let by = if j.scheduled { " (scheduled)" } else { "" };
            out.push_str(&format!(
                "{} · {state} · {} · {}{by} · {} · {} · {}{on}\n",
                j.id,
                j.queue,
                j.label(),
                j.query_label(),
                j.data_source.as_deref().unwrap_or("—"),
                fmt::dur(j.age_s as f64)
            ));
        }
    }

    let a = &app.airflow;
    out.push_str("\n== Airflow\n");
    if !a.reachable {
        out.push_str(&format!("not read: {}\n", a.error.as_deref().unwrap_or("no answer")));
    } else {
        let beat = |name: &str, b: &crate::airflow::Beat| b.status.as_ref().map(|s| format!("{name} {s}"));
        let health: Vec<String> = [beat("scheduler", &a.health.scheduler), beat("triggerer", &a.health.triggerer), beat("dag processor", &a.health.dag_processor)]
            .into_iter()
            .flatten()
            .chain(a.health.metadatabase.as_ref().map(|m| format!("metadatabase {m}")))
            .collect();
        out.push_str(&format!("health: {}\n", if health.is_empty() { "—".to_string() } else { health.join(" · ") }));
        if let Some(errors) = a.import_errors.filter(|e| *e > 0) {
            out.push_str(&format!("{errors} DAG import errors\n"));
        }
        let c = a.counts(now);
        out.push_str(&format!("last 24 h: {} runs · {} ok · {} failed · {} running · {} queued · {} DAGs ({} paused)\n", c.runs, c.ok, c.failed, c.running, c.queued, c.dags, c.paused));
        let mut runs: Vec<&crate::airflow::Run> = a.runs.iter().filter(|r| r.state != crate::airflow::RunState::Success).collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.at()));
        for r in runs.iter().take(40) {
            let when = r.at().map(|t| format!("{} ago", fmt::dur((now - t).max(0) as f64))).unwrap_or_default();
            let took = r.took(now).map(|s| format!(" · took {}", fmt::dur(s as f64))).unwrap_or_default();
            let mut line = format!("{} · {} · run {} ({}) · {when}{took}", r.dag, r.state.word(), r.id, r.kind);
            if let Some(p) = a.progress.get(&(r.dag.clone(), r.id.clone())) {
                line.push_str(&format!(" · tasks {} done of {}", p.done, p.total));
                if !p.failed.is_empty() {
                    line.push_str(&format!(" · failed tasks: {}", p.failed.join(", ")));
                }
                if !p.running.is_empty() {
                    line.push_str(&format!(" · running tasks: {}", p.running.join(", ")));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    let b = &app.jira;
    out.push_str("\n== Jira (the owner's board)\n");
    if !b.reachable {
        out.push_str(&format!("not read: {}\n", b.error.as_deref().unwrap_or("no answer")));
    } else {
        for column in b.columns() {
            out.push_str(&format!("{} ({}):\n", column.status, column.tickets.len()));
            for t in column.tickets.iter().take(25) {
                let due = t.due.as_ref().map(|d| format!(" · due {d}")).unwrap_or_default();
                let since = t.in_status(now).map(|s| format!(" · in {} for {}", column.status, fmt::dur(s as f64))).unwrap_or_default();
                out.push_str(&format!("  {} · {} · {}{due}{since}\n", t.key, t.summary, t.priority.as_deref().unwrap_or("no priority")));
            }
        }
    }

    out.push_str("\n== Tape (what changed, newest first)\n");
    for e in app.tape.newest_first().take(25) {
        let text: String = e.parts.iter().map(|(t, _)| t.as_str()).collect();
        out.push_str(&format!("{} {} {} {text}\n", app.time.hms(e.at as i64), mark(e.level), e.kind.label()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        crate::ui2::tests::app_after(10)
    }

    fn sent(bot: &Bot) -> Vec<(String, Buttons)> {
        bot.out.iter().filter_map(|o| if let Out::Send { text, buttons, .. } = o { Some((text.clone(), buttons.clone())) } else { None }).collect()
    }

    fn button(bot: &Bot) -> String {
        bot.out.iter().rev().find_map(|o| if let Out::Send { buttons, .. } = o { buttons.first().map(|b| b.1.clone()) } else { None }).expect("a card")
    }

    fn press(bot: &mut Bot, app: &mut App, chat: i64, message: i64, data: &str, now: i64) {
        bot.on_event(app, BotEvent::Incoming(Incoming::Button { chat, message, callback: "c".into(), data: data.to_string() }), now);
    }

    fn running(app: &App) -> (String, String) {
        app.with_view(|v| {
            let node = v.nodes.iter().find(|n| !n.node.queries.is_empty()).unwrap();
            (node.node.name.clone(), node.node.queries[0].query_id.clone())
        })
        .unwrap()
    }

    #[test]
    fn chrome_shows_its_parts_and_a_stop_button_that_makes_a_card() {
        let mut app = app();
        let now = app.now();
        app.update(crate::app::Event::Local(Box::new(crate::fake::local(1, now as f64 - 2.0))));
        app.update(crate::app::Event::Local(Box::new(crate::fake::local(2, now as f64))));
        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: "/chrome".into(), at: now }), now);
        let (text, buttons) = sent(&bot).pop().expect("the message");
        assert!(text.contains("<b>Google Chrome</b> · 3.7 GiB RSS") && text.contains("11.5% of 32 GiB · 8 processes"), "{text}");
        assert!(text.contains("<pre>part") && text.contains("tabs: reading…") && text.contains("Heaviest tab processes"), "{text}");
        assert!(crate::telegram::html::plain_of(&text).lines().all(|l| l.chars().count() <= 52), "phone-wide: {text}");
        assert!(buttons.iter().any(|b| b.0 == "🛑 Stop 15564 · 834 MiB") && !buttons.iter().any(|b| b.0.contains("14897")), "never the browser: {buttons:?}");
        assert!(app.tabs_for_phone, "its tabs are read for the phone");
        // The tabs, when read, follow in a message of their own.
        app.update(crate::app::Event::Tabs(Box::new(crate::browser::TabsRead::new("Google Chrome".into(), now as f64, Ok(crate::fake::tabs())))));
        bot.out.clear();
        bot.on_tabs(&mut app);
        assert!(sent(&bot)[0].0.contains("🗂 <b>12 tabs</b> · 2 windows · 6 sites") && !app.tabs_for_phone);
        // A stop button makes a card; y stops it, asked on the phone; the result comes under it.
        let data = buttons.iter().find(|b| b.0.starts_with("🛑 Stop 15564")).unwrap().1.clone();
        bot.out.clear();
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Button { chat: 7, message: 3, callback: "c".into(), data }), now);
        assert!(sent(&bot)[0].0.contains("Stop this Chrome tab process?") && sent(&bot)[0].0.contains("Aw, Snap!"));
        let yes = button(&bot);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Button { chat: 7, message: 9, callback: "c".into(), data: yes }), now + 5);
        let stops = app.take_stops();
        assert_eq!(stops.len(), 1);
        assert_eq!((stops[0].helper.pid, stops[0].asked), (15564, crate::app::Asked::Phone { chat: 7, message: 9 }));
        bot.out.clear();
        bot.on_stopped(&stops[0], &Ok("stopped".into()));
        assert!(matches!(&bot.out[..], [Out::Edit { message: 9, text, .. }] if text.contains("✅ <b>Stopped</b> · pid 15564")), "{:?}", bot.out);
    }

    #[test]
    fn actions_are_taken_out_of_the_answer() {
        let said = "Node 3 is full: one query holds 7.9 GiB of 9.0 GiB.\nI'd kill it.\nACTION kill ch-3 abc-123\n`ACTION cancel 9f1e`";
        let (reply, actions) = parse_answer(said);
        assert_eq!(reply, "Node 3 is full: one query holds 7.9 GiB of 9.0 GiB.\nI'd kill it.");
        assert_eq!(actions, [Action::Kill { node: "ch-3".into(), query_id: "abc-123".into() }, Action::Cancel { job: "9f1e".into() }]);
        assert_eq!(parse_answer("no action here").1, []);
    }

    #[test]
    fn long_answers_are_cut_for_telegram_at_lines() {
        let text = (0..400).map(|i| format!("line {i} of a long answer")).collect::<Vec<_>>().join("\n");
        let parts = split(&text);
        assert!(parts.len() >= 2 && parts.iter().all(|p| p.chars().count() <= MESSAGE_MAX), "{}", parts.len());
        assert_eq!(parts.join("\n"), text);
        assert_eq!(split("").len(), 0);
    }

    #[test]
    fn a_stranger_is_told_only_their_chat_id() {
        let mut app = app();
        let now = app.now();
        let mut bot = Bot::new(Vec::new(), now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 42, text: "which node is full?".into(), at: now }), now);
        assert!(bot.work.is_empty());
        assert!(matches!(&sent(&bot)[..], [(text, _)] if text.contains("TELEGRAM_CHAT_ID=42")), "{:?}", bot.out);

        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 42, text: "/status".into(), at: now }), now);
        assert!(bot.out.is_empty() && bot.work.is_empty());
    }

    #[test]
    fn a_question_goes_to_claude_with_the_snapshot_and_without_sql_and_the_answer_comes_back_as_html() {
        let mut app = app();
        let now = app.now();
        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: "node mana yang paling penuh?".into(), at: now }), now);
        let Some(Work::Ask { prompt, question, .. }) = bot.work.first().cloned() else { panic!("{:?}", bot.work) };
        assert!(prompt.contains("== ClickHouse nodes") && prompt.contains("== Redash queue") && prompt.ends_with("Person: node mana yang paling penuh?\n"));
        let sqls: Vec<String> = app.with_view(|v| v.nodes.iter().flat_map(|n| n.node.queries.iter().map(|q| q.sql.clone())).filter(|s| s.len() > 20).collect()).unwrap();
        assert!(!sqls.is_empty());
        for sql in sqls {
            assert!(!prompt.contains(&sql), "{sql}");
        }
        // A second question while the first is answered waits its turn, and says for what.
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: "and now?".into(), at: now }), now + 4);
        assert_eq!(bot.work.len(), 1);
        assert!(sent(&bot).last().unwrap().0.contains("still on <i>node mana yang paling penuh?</i> · 4 s"));
        bot.out.clear();
        bot.on_event(&mut app, BotEvent::Answered { chat: 7, question, result: Ok("**clickhouse3** is full <3\n• 🔴 memory 91%".into()) }, now + 9);
        assert_eq!(sent(&bot)[0].0, "<b>clickhouse3 is full &lt;3</b>\n\n• 🔴 memory 91%");
    }

    #[test]
    fn a_kill_needs_the_press_of_a_button_and_a_query_still_running() {
        let mut app = app();
        let now = app.now();
        let (node, query_id) = running(&app);
        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Answered { chat: 7, question: "kill it".into(), result: Ok(format!("It's the heaviest.\nACTION kill {node} {query_id}")) }, now);
        assert!(bot.work.is_empty(), "nothing is run before the button");
        let (card, buttons) = sent(&bot).pop().unwrap();
        assert!(card.starts_with("🛑 <b>Kill this query?</b>") && card.contains(&format!("<code>{query_id}</code>")) && card.contains("<pre>"), "{card}");
        assert!(card.contains("<b>who</b>") && card.contains("<b>memory</b>") && card.contains("this query "), "{card}");
        assert_eq!(buttons[0].0, "🛑 Kill it");
        let token = buttons[0].1.clone();
        // Another chat's press does nothing.
        press(&mut bot, &mut app, 8, 1, &token, now);
        assert!(bot.work.is_empty());
        press(&mut bot, &mut app, 7, 1, &token, now + 5);
        assert_eq!(bot.work.last(), Some(&Work::Kill { chat: 7, message: 1, node: node.clone(), query_id: query_id.clone() }));
        // The result goes under the card's facts.
        bot.out.clear();
        bot.on_event(&mut app, BotEvent::Killed { chat: 7, message: 1, result: Ok("asked".into()) }, now + 6);
        assert!(matches!(&bot.out[..], [Out::Edit { message: 1, text, .. }] if text.starts_with("🛑 <b>Kill this query?</b>") && text.contains("✅ <b>Killed</b>")), "{:?}", bot.out);
        // A card works once.
        bot.work.clear();
        press(&mut bot, &mut app, 7, 1, &token, now + 7);
        assert!(bot.work.is_empty());

        // A query that is not running gets no card; an old card does nothing.
        bot.out.clear();
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: format!("/kill {node} no-such-query"), at: now }), now);
        assert!(matches!(&sent(&bot)[..], [(text, buttons)] if buttons.is_empty() && text.contains("not running")), "{:?}", bot.out);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: format!("/kill {node} {query_id}"), at: now }), now);
        let token = button(&bot);
        bot.out.clear();
        press(&mut bot, &mut app, 7, 2, &token, now + CARD_S + 61);
        assert!(bot.work.is_empty());
        assert!(matches!(&bot.out[..], [Out::Answer { .. }, Out::Edit { text, .. }] if text.contains("⌛ <b>Not done</b>")), "{:?}", bot.out);
    }

    #[test]
    fn a_cancel_from_a_card_goes_to_redash_and_its_answer_back_to_the_card() {
        let mut app = app();
        let now = app.now();
        let job = app.queue.jobs.first().expect("the fake queue has jobs").id.clone();
        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: format!("/cancel {job}"), at: now }), now);
        let token = button(&bot);
        press(&mut bot, &mut app, 7, 3, &token, now);
        assert_eq!(app.take_cancels(), std::slice::from_ref(&job));
        bot.out.clear();
        bot.on_cancelled(&app, &job, &Ok(()));
        assert!(matches!(&bot.out[..], [Out::Edit { message: 3, text, .. }] if text.contains("✅ <b>Cancelled</b> in Redash")), "{:?}", bot.out);
    }

    #[test]
    fn status_s_tabs_edit_it_in_place() {
        let mut app = app();
        let now = app.now();
        let mut bot = Bot::new(vec![7], now);
        bot.on_event(&mut app, BotEvent::Incoming(Incoming::Message { chat: 7, text: "/status".into(), at: now }), now);
        let (text, buttons) = sent(&bot).pop().unwrap();
        assert!(text.contains("<b>cobserve</b>"), "{text}");
        assert_eq!(buttons, look::tabs(&app, look::Tab::Home));
        bot.out.clear();
        press(&mut bot, &mut app, 7, 9, "t:f", now + 2);
        assert!(
            matches!(&bot.out[..], [Out::Answer { .. }, Out::Edit { message: 9, text, buttons, .. }] if text.contains("<b>Fleet</b>") && *buttons == look::tabs(&app, look::Tab::Fleet)),
            "{:?}",
            bot.out
        );
    }

    /// A fleet of one healthy node, read at `at`: nothing wrong on it.
    fn quiet(at: i64) -> App {
        const GIB: u64 = 1 << 30;
        let node = crate::model::NodeSnapshot {
            name: "ch1".into(),
            host: "ch1".into(),
            port: 9000,
            shard: 1,
            replica: 1,
            version: "24.10".into(),
            reachable: true,
            mem_total: Some(64 * GIB),
            mem_used: 10 * GIB,
            cores: Some(16.0),
            cpu_busy_cores: Some(2.0),
            running: 0,
            lag_s: 0,
            active_parts: 10,
            queries: vec![],
            uptime_s: Some(100),
            server_cpu_time_us: None,
            max_memory_usage: Some(9 * GIB),
            unreachable_reason: None,
            poll_ms: Some(20),
        };
        let taken_at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(at as u64);
        let mut app = App::new();
        app.update(crate::app::Event::Snapshot(Box::new(crate::model::FleetSnapshot { taken_at, nodes: vec![node] })));
        app.tick_at(taken_at);
        app
    }

    #[test]
    fn an_issue_is_told_once_it_has_gone_on_for_ten_minutes_and_again_when_it_clears() {
        let app = app();
        let now = app.now();
        let mut bot = Bot::new(vec![7], now);
        bot.tick(&app, now);
        assert!(matches!(&sent(&bot)[..], [(text, _)] if text.starts_with("🟢 <b>cobserve is up</b>")), "only the hello: {:?}", bot.out);
        bot.out.clear();
        // Red from the start — some of it for hours before the bot was — but the bot has not
        // seen it for ten minutes yet: nothing.
        bot.tick(&app, now + 300);
        bot.tick(&app, now + PERSIST_S - 1);
        assert!(bot.out.is_empty(), "{:?}", bot.out);
        bot.tick(&app, now + PERSIST_S);
        let (text, buttons) = sent(&bot).pop().expect("the alert");
        assert!(text.starts_with("🔔 <b>") && text.contains("going on for 10+ min") && text.contains("<b>clickhouse3</b> · for 10m"), "{text}");
        assert!(buttons[0].1.starts_with("e:"));
        // Told once.
        bot.out.clear();
        bot.tick(&app, now + PERSIST_S + 60);
        assert!(bot.out.is_empty(), "{:?}", bot.out);
        // A fleet not read says nothing; a flicker shorter than GONE_S does not end it.
        let t = now + PERSIST_S + 60;
        bot.tick(&App::new(), t + 30);
        bot.tick(&quiet(t + 60), t + 60);
        assert!(bot.out.is_empty(), "{:?}", bot.out);
        // Gone for longer: it is over, and that is told.
        bot.tick(&quiet(t + 60 + GONE_S + 1), t + 60 + GONE_S + 1);
        assert!(sent(&bot).iter().any(|(text, _)| text.contains("🟢 <b>clickhouse3</b> · cleared after 11m")), "{:?}", bot.out);
    }

    #[test]
    fn what_clears_within_ten_minutes_is_never_told() {
        let app = app();
        let now = app.now();
        let mut bot = Bot::new(vec![7], now);
        bot.tick(&app, now);
        bot.out.clear();
        bot.tick(&app, now + 300);
        bot.tick(&quiet(now + 300 + GONE_S + 1), now + 300 + GONE_S + 1);
        bot.tick(&quiet(now + 2 * PERSIST_S), now + 2 * PERSIST_S);
        // Airflow, not read by the quiet app, keeps its own; nothing of the fleet is told.
        assert!(sent(&bot).iter().all(|(text, _)| !text.contains("<b>clickhouse3</b>") && !text.contains("<b>Redash</b>")), "{:?}", bot.out);
        assert!(bot.issues.keys().all(|k| !k.starts_with("i:")), "{:?}", bot.issues.keys());
    }

}
