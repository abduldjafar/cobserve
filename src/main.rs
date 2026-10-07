//! Terminal setup and the event loop (DESIGN.md §8).
//!
//! Sources are tokio tasks that push `Event`s down one channel; the loop applies them to
//! `App` and redraws. Key events come from crossterm's `EventStream`, a 1 s tick drives the
//! clock, and **the UI never blocks on the network**: a slow node, a dead Redash, or a
//! ClickHouse that never answers all show up as numbers that do not move.

mod airflow;
mod app;
mod assist;
mod attrib;
mod claude;
mod clock;
mod complete;
mod config;
mod console;
mod conversations;
mod desktop;
mod detail;
mod fake;
mod fmt;
mod folders;
mod history;
mod insight;
mod jira;
mod local;
mod model;
mod prayer;
mod saved;
mod pty;
mod severity;
mod sources;
mod sqltext;
mod tape;
mod theme;
mod tree;
mod ui;

use app::{App, Event};
use config::Config;
use crossterm::event::{Event as TermEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    // Before the terminal is touched: a configuration mistake should print a line, not leave
    // a mangled terminal behind.
    let args = match config::Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("cobserve: {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", config::USAGE);
        return Ok(());
    }
    if args.version {
        println!("cobserve {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let config = match Config::load(&args) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("cobserve: {e}");
            std::process::exit(2);
        }
    };
    // Before any source starts: every person they attribute is displayed with it (§6.4).
    attrib::set_home_domain(config.email_domain.clone());

    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(e) => {
            // Piping this binary into a file is a normal thing to do; a panic is not the answer.
            eprintln!("cobserve: no terminal to draw on ({e}). Run it in a terminal, or FAKE=1 with a TTY capture.");
            std::process::exit(1);
        }
    };
    install_panic_hook();
    // Pasted text arrives as one event, so a paste into Claude's pane is a paste and not a
    // stream of keys, each newline an Enter.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    // Clicks on the tabs and the sessions, the wheel over Claude. MOUSE=0 leaves the mouse to
    // the terminal, whose own selection then needs no modifier key.
    if config.mouse {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    }
    let modified_enter = tell_enter_apart();

    // The terminal's title is the monitor's while it runs, and the one it had after.
    {
        use std::io::Write;
        let _ = std::io::stdout().write_all(b"\x1b[22;0t");
    }
    let result = run(&mut terminal, config, args.claude, modified_enter).await;

    restore_terminal();
    result
}

/// Asked of the terminal before anything reads keys: where it speaks the kitty keyboard
/// protocol, Enter with shift or ⌘ comes through as itself rather than as Enter — as it does
/// for Claude Code and OpenCode run on their own — and they get it as a new line. Only the
/// escape codes are made unambiguous: keys are still pressed, never released.
fn tell_enter_apart() -> bool {
    if !crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        return false;
    }
    let flags = crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    let pushed = crossterm::execute!(std::io::stdout(), crossterm::event::PushKeyboardEnhancementFlags(flags)).is_ok();
    KEYS_PUSHED.store(pushed, Ordering::SeqCst);
    pushed
}

/// The keyboard protocol was asked for, and has to be given back on the way out.
static KEYS_PUSHED: AtomicBool = AtomicBool::new(false);

fn restore_terminal() {
    if KEYS_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste
    );
    {
        use std::io::Write;
        let _ = std::io::stdout().write_all(b"\x1b[23;0t");
    }
    ratatui::restore();
}

/// §12: the hook has to leave raw mode and the alternate screen, or a crash leaves the user's
/// shell unusable. Installed before anything enters raw mode.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    let _ = color_eyre::install();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

async fn run(terminal: &mut DefaultTerminal, config: Config, open_claude: bool, modified_enter: bool) -> color_eyre::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut app = App::new();
    app.poll_interval = config.poll;
    app.modified_enter = modified_enter;
    app.claude.commands = claude::Commands {
        claude: config.claude_command.clone(),
        opencode: config.opencode_command.clone(),
        terminal: config.shell_command.clone(),
    };
    app.claude.assistant = config.assistant;
    if let Ok(dir) = std::env::current_dir() {
        app.claude.default_dir = pty::tilde(&dir);
    }
    // The sessions of the last run, back where they were; each takes its conversation up when it
    // is first shown.
    app.claude.restore(&saved::load());
    let mut kept = app.claude.to_saved();
    if open_claude {
        app.open_claude();
    }
    if config.utc {
        app.time.shown = clock::Shown::Utc;
    }
    app.prayers = prayer::Prayers::new(None, config.prayer.remind_minutes);
    // The machine's zone, and the city prayer times are for when none was chosen: looked at
    // again every half minute, for a laptop that travels.
    let zone_table = clock::zone_table();
    let mut zone = clock::zone_now();
    app.update(Event::Zone(zone.clone(), prayer_place(&config, zone.as_deref(), zone_table.as_deref())));
    let mut zone_read = std::time::Instant::now();
    // View 5's programs, one per session. Dropping one ends it — on quit, all of them.
    let mut panes: HashMap<u64, pty::PtyProcess> = HashMap::new();
    // When the sessions' branches were last read: Claude, or you, can change them.
    let mut branches_read = std::time::Instant::now();
    // The folder picker's latest lookup: an older one still running sees it and stops.
    let latest_lookup = Arc::new(AtomicU64::new(0));
    // When the sessions' conversations were last asked after, and the sessions last kept.
    let mut conversations_read = std::time::Instant::now();
    let mut rounds = 0u32;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);

    // Query sessions' work, and where the fleet's servers are for it.
    let mut consoles = Consoles {
        targets: Arc::new(std::sync::Mutex::new(HashMap::new())),
        client: reqwest::Client::builder().build().unwrap_or_default(),
        tasks: HashMap::new(),
        fake: config.fake,
        assist_dir: assist::dir(),
    };
    // A Redash job cancelled from view 2 goes to the task that talks to Redash; `r` on views 5
    // and 6 to the task that reads Airflow or Jira.
    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel::<String>();
    let (airflow_tx, airflow_rx) = mpsc::unbounded_channel::<()>();
    let (jira_tx, jira_rx) = mpsc::unbounded_channel::<()>();
    let asks = Asks { cancels: cancel_rx, airflow: airflow_rx, jira: jira_rx };
    let details = spawn_sources(&config, tx.clone(), Arc::clone(&consoles.targets), asks);
    // Anything a source could not even start with belongs on screen, not in a log file.
    for warning in &config.warnings {
        app.update(Event::Notice(warning.clone()));
    }
    if let Some(config_error) = startup_problem(&config) {
        app.update(Event::Notice(config_error));
    }

    // crossterm's blocking read lives on its own thread and sends into the same channel: the
    // loop stays a plain select over two sources, and no extra dependency is needed.
    spawn_key_reader(tx.clone());

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    spawn_signal_watch(tx.clone());

    let mut shown_title = String::new();
    // Inside Cobserve.app the header is the app's: cobserve leaves its masthead out and says, after
    // a frame, what the header shows (`desktop.rs`).
    let inside_app = desktop::inside_the_app();
    app.outside_chrome = inside_app;
    let mut told: Option<desktop::State> = None;
    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if inside_app {
            let state = desktop::state(&app);
            if told.as_ref() != Some(&state) {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(desktop::sequence(&state).as_bytes());
                let _ = out.flush();
                told = Some(state);
            }
        }
        let title = ui::title(&app);
        if title != shown_title {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(&title));
            shown_title = title;
        }
        // After the frame: the panes now know the size they are drawn at.
        drive_panes(&mut app, &mut panes, &tx);
        drive_consoles(&mut app, &mut consoles, &tx);
        drive_picker(&mut app, &latest_lookup, &tx);
        drive_conversations(&mut app, home.as_deref(), &tx);
        if conversations_read.elapsed() >= Duration::from_secs(15) {
            conversations_read = std::time::Instant::now();
            // OpenCode is asked by running it, so once a minute rather than every round.
            rounds += 1;
            follow_conversations(&mut app, &panes, home.as_deref(), &tx, rounds % 4 == 1);
            // Kept as they change, not only on the way out: a killed terminal loses nothing.
            let now = app.claude.to_saved();
            if now != kept {
                kept = now;
                let _ = saved::store(&kept);
            }
        }
        if zone_read.elapsed() >= Duration::from_secs(30) {
            zone_read = std::time::Instant::now();
            let now = clock::zone_now();
            if now != zone {
                zone = now;
                app.update(Event::Zone(zone.clone(), prayer_place(&config, zone.as_deref(), zone_table.as_deref())));
            }
        }
        if branches_read.elapsed() >= Duration::from_secs(5) {
            branches_read = std::time::Instant::now();
            for session in &mut app.claude.list {
                if let Ok(dir) = pty::resolve_dir(&session.dir) {
                    session.branch = pty::git_branch(&dir);
                }
            }
        }

        tokio::select! {
            event = rx.recv() => match event {
                Some(event) => {
                    app.update(event);
                    // Whatever else is already waiting goes in before the next frame: Claude's
                    // output comes in many small pieces, and a frame for each is wasted work.
                    for _ in 0..512 {
                        match rx.try_recv() {
                            Ok(event) => app.update(event),
                            Err(_) => break,
                        }
                    }
                }
                // Every source is gone; nothing left to wait for.
                None => app.update(Event::Quit),
            },
            _ = tick.tick() => app.update(Event::Tick),
        }
        // Keys for Claude go out now, not a frame later.
        send_owed(&mut app, &mut panes);
        for text in app.take_notifications() {
            notify(config.notify, &text);
        }
        for text in app.take_clipboard() {
            copy_to_clipboard(&text);
        }
        for id in app.take_cancels() {
            if let Err(lost) = cancel_tx.send(id) {
                app.update(Event::Cancelled(lost.0, Err("the Redash source has stopped".into())));
            }
        }
        for feed in app.take_refreshes() {
            let _ = match feed {
                app::Feed::Airflow => airflow_tx.send(()),
                app::Feed::Jira => jira_tx.send(()),
            };
        }
        for url in app.take_opens() {
            open_in_browser(&url);
        }
        for ask in app.take_detail_asks() {
            details.read(ask, tx.clone());
        }

        if app.quit {
            break;
        }
    }
    // What each program is in at the end — Claude Code after a /clear is in another
    // conversation than it started with — kept for the next run.
    follow_conversations(&mut app, &panes, home.as_deref(), &tx, false);
    while let Ok(event) = rx.try_recv() {
        if matches!(event, Event::Conversation(..)) {
            app.update(event);
        }
    }
    let _ = saved::store(&app.claude.to_saved());
    Ok(())
}

/// Query sessions' work: each query, each server's tables and each helper's answer a task of its
/// own, that `ctrl+c` — or its session closing — aborts: a query's request let go, so the server
/// stops it; a helper's program killed.
struct Consoles {
    /// Every server's address and login, as the ClickHouse source last found the fleet. Logins
    /// stay here, with the code that sends them — never in `App`.
    targets: Arc<std::sync::Mutex<HashMap<String, sources::clickhouse::ConsoleTarget>>>,
    client: reqwest::Client,
    tasks: HashMap<(u64, Work, u64), tokio::task::JoinHandle<()>>,
    fake: bool,
    /// Where helpers run: an empty folder of their own.
    assist_dir: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Work {
    Query,
    Ask,
}

/// What query sessions ask for, started; what they stop, stopped; the tables of the servers they
/// are on, read when they are not yet.
fn drive_consoles(app: &mut App, consoles: &mut Consoles, tx: &mpsc::UnboundedSender<Event>) {
    let (mut stops, mut queries, mut asks) = (Vec::new(), Vec::new(), Vec::new());
    for session in &mut app.claude.list {
        let Some(console) = session.console.as_deref_mut() else {
            continue;
        };
        if let Some(run) = console.take_cancel() {
            stops.push((session.id, Work::Query, run));
        }
        if let Some(ask) = console.take_ask_cancel() {
            stops.push((session.id, Work::Ask, ask));
        }
        if let Some(request) = console.take_request() {
            queries.push((session.id, request));
        }
        if let Some(ask) = console.take_ask() {
            asks.push((session.id, ask));
        }
    }
    for key in stops {
        if let Some(task) = consoles.tasks.remove(&key) {
            task.abort();
        }
    }
    // A closed session's work ends with it; finished work is let go of.
    consoles.tasks.retain(|(id, ..), task| {
        let open = app.claude.list.iter().any(|s| s.id == *id);
        if !open {
            task.abort();
        }
        open && !task.is_finished()
    });

    for (id, request) in queries {
        let tx = tx.clone();
        let run = request.id;
        let task = if consoles.fake {
            // The made-up fleet answers at once; a moment's wait makes it look like a server.
            let answer = fake::console_answer(app.snapshot(), &request.node, &request.sql);
            let wait = 120 + (request.sql.len() as u64 * 37) % 500;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                let answer = answer.map(|mut answer| {
                    answer.elapsed_ms = wait;
                    answer
                });
                let _ = tx.send(Event::ConsoleAnswer(id, run, answer));
            })
        } else {
            let target = consoles.targets.lock().ok().and_then(|targets| targets.get(&request.node).cloned());
            let client = consoles.client.clone();
            tokio::spawn(async move {
                let answer = match target {
                    Some(target) => sources::clickhouse::console_query(&client, &target, &request.node, &request.sql).await,
                    None => Err(format!("{} is not among the servers found — the next discovery may find it", request.node)),
                };
                let _ = tx.send(Event::ConsoleAnswer(id, run, answer));
            })
        };
        consoles.tasks.insert((id, Work::Query, run), task);
    }

    for (id, ask) in asks {
        let tx = tx.clone();
        let asked = ask.id;
        let task = if consoles.fake {
            let answer = fake::assist(&ask);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1400)).await;
                let _ = tx.send(Event::Assisted(id, asked, answer));
            })
        } else {
            let version = ask.node.as_deref().and_then(|node| app.snapshot()?.nodes.iter().find(|n| n.name == node)).map(|n| n.version.clone());
            let schema = app.schema_of(ask.node.as_deref()).cloned();
            let now = app.now();
            let today = format!("{}, {}", app.time.format(now, "%A %Y-%m-%d %H:%M"), app.time.label(now));
            let program = match ask.assistant {
                console::Assistant::Claude => app.claude.commands.claude.clone(),
                console::Assistant::OpenCode => app.claude.commands.opencode.clone(),
            };
            let dir = consoles.assist_dir.clone();
            tokio::spawn(async move {
                let answer = ask_helper(&ask, &program, schema.as_ref(), version.as_deref(), &today, &dir).await;
                let _ = tx.send(Event::Assisted(id, asked, answer));
            })
        };
        consoles.tasks.insert((id, Work::Ask, asked), task);
    }

    for node in app.schemas_wanted() {
        let tx = tx.clone();
        if consoles.fake {
            let _ = tx.send(Event::Schema(node, Ok(fake::schema())));
            continue;
        }
        let target = consoles.targets.lock().ok().and_then(|targets| targets.get(&node).cloned());
        let client = consoles.client.clone();
        tokio::spawn(async move {
            let schema = match target {
                Some(target) => sources::clickhouse::console_schema(&client, &target).await,
                None => Err("not among the servers found yet".to_string()),
            };
            let _ = tx.send(Event::Schema(node, schema));
        });
    }
}

/// What a helper writes for `ask`: asked once with every table of the server named and those the
/// text points at told with their columns — and, when it answers with `-- need: db.table, …`
/// instead, asked again with those tables' columns. Only names, columns and row counts go to it:
/// never a row of a table.
async fn ask_helper(
    ask: &console::Ask,
    program: &[String],
    schema: Option<&complete::Schema>,
    version: Option<&str>,
    today: &str,
    dir: &std::path::Path,
) -> Result<String, String> {
    let whole = ask.selected.is_none();
    let first = assist::prompt(ask, schema, version, today, None);
    let said = run_assistant(ask.assistant, program, first, dir).await?;
    match (schema, assist::needed(&said)) {
        (Some(schema), Some(names)) => {
            let second = assist::prompt(ask, Some(schema), version, today, Some(&names));
            let said = run_assistant(ask.assistant, program, second, dir).await?;
            if assist::needed(&said).is_some() {
                return Ok(format!("-- {} could not tell which tables answer it: looked at {}", ask.assistant.name(), names.join(", ")));
            }
            assist::clean(&said, whole)
        }
        _ => assist::clean(&said, whole),
    }
}

/// A helper's program, asked once: the question on its standard input, the answer on its
/// standard output — in a folder of its own, without the monitor's secrets (Claude Code then
/// signs in with the Pro or Max plan, as in a terminal), for at most `assist::TIME_LIMIT_S`.
/// Dropped — `ctrl+c` — the program is killed.
async fn run_assistant(assistant: console::Assistant, program: &[String], prompt: String, dir: &std::path::Path) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let name = assistant.name();
    let argv = assist::command(assistant, program, &dir.to_string_lossy());
    let Some(executable) = argv.first() else {
        return Err(format!("no command for {name}"));
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(&argv[1..])
        .current_dir(dir)
        .env("PWD", dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for variable in pty::NOT_PASSED_ON {
        command.env_remove(variable);
    }
    if assistant == console::Assistant::OpenCode {
        command.env("OPENCODE_PERMISSION", assist::OPENCODE_PERMISSION).env("OPENCODE_DISABLE_AUTOUPDATE", "1");
    }
    let mut child = command.spawn().map_err(|e| match (e.kind(), assistant) {
        (std::io::ErrorKind::NotFound, console::Assistant::Claude) => {
            "Claude Code is not installed here — npm install -g @anthropic-ai/claude-code, then `claude` once to sign in".to_string()
        }
        (std::io::ErrorKind::NotFound, console::Assistant::OpenCode) => {
            "OpenCode is not installed here — curl -fsSL https://opencode.ai/install | bash".to_string()
        }
        _ => format!("{name} would not start: {e}"),
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        });
    }
    let output = tokio::time::timeout(Duration::from_secs(assist::TIME_LIMIT_S), child.wait_with_output())
        .await
        .map_err(|_| format!("{name} did not answer within {} s", assist::TIME_LIMIT_S))?
        .map_err(|e| format!("{name}: {e}"))?;
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(stdout.into_owned())
    } else {
        Err(assist::failure(assistant, output.status.code(), &stderr, &stdout))
    }
}

/// The conversation each running program is in: Claude Code says so for its process id
/// (`~/.claude/sessions`); OpenCode's is the newest in its folder begun after it started, asked
/// — when `opencode_too` — on a thread of its own until it is known.
fn follow_conversations(
    app: &mut App,
    panes: &HashMap<u64, pty::PtyProcess>,
    home: Option<&std::path::Path>,
    tx: &mpsc::UnboundedSender<Event>,
    opencode_too: bool,
) {
    let Some(home) = home else {
        return;
    };
    for session in &mut app.claude.list {
        let Some(pid) = panes.get(&session.id).and_then(pty::PtyProcess::pid) else {
            continue;
        };
        match session.kind {
            claude::Kind::Claude => {
                if let Some(id) = conversations::claude_live(home, pid) {
                    session.conversation = Some(id);
                }
            }
            claude::Kind::OpenCode if opencode_too && session.conversation.is_none() => {
                let (command, dir, started, id, tx) =
                    (app.claude.commands.opencode.clone(), pty::expand(&session.dir), session.started, session.id, tx.clone());
                std::thread::spawn(move || {
                    let newest = conversations::opencode(&command, &dir, 5)
                        .into_iter()
                        .filter(|c| c.updated >= started - 5)
                        .max_by_key(|c| c.updated);
                    if let Some(conversation) = newest {
                        let _ = tx.send(Event::Conversation(id, conversation.id));
                    }
                });
            }
            _ => {}
        }
    }
}

/// The picker's conversations to take up, listed on a thread of their own.
fn drive_conversations(app: &mut App, home: Option<&std::path::Path>, tx: &mpsc::UnboundedSender<Event>) {
    let claude::Mode::Opening(picker) = &mut app.claude.mode else {
        return;
    };
    let Some(lookup) = picker.conversation_lookup() else {
        return;
    };
    let (home, opencode, tx) = (home.map(std::path::Path::to_path_buf), app.claude.commands.opencode.clone(), tx.clone());
    std::thread::spawn(move || {
        let everywhere = lookup.dir.is_none();
        let limit = if everywhere { claude::CONVERSATIONS_EVERYWHERE } else { claude::CONVERSATIONS_HERE };
        let list = match (lookup.kind, &lookup.dir, &home) {
            (claude::Kind::Claude, dir, Some(home)) => conversations::claude(home, dir.as_deref(), limit),
            (claude::Kind::OpenCode, Some(dir), _) => conversations::opencode(&opencode, dir, limit),
            (claude::Kind::OpenCode, None, _) => conversations::opencode_all(&opencode, limit),
            _ => Vec::new(),
        };
        let _ = tx.send(Event::Conversations(lookup.generation, lookup.kind, everywhere, list));
    });
}

/// View 5's programs: each started when its session asks for it, all sized to what was drawn
/// (a session switched to is then the right size already), sent what is owed to them, and let
/// go of once they ended or their session was closed.
fn drive_panes(app: &mut App, panes: &mut HashMap<u64, pty::PtyProcess>, tx: &mpsc::UnboundedSender<Event>) {
    use claude::PaneState;
    let (rows, cols) = app.claude.want_size.get();
    let commands = app.claude.commands.clone();
    for session in &mut app.claude.list {
        match session.pane.state {
            PaneState::Starting => {
                // An earlier program of this session, if any, ends here.
                panes.remove(&session.id);
                let dir = match pty::resolve_dir(&session.dir) {
                    Ok(dir) => dir,
                    Err(why) => {
                        session.pane.state = PaneState::Failed(why);
                        continue;
                    }
                };
                session.branch = pty::git_branch(&dir);
                // Started again — after it ended, or ⏎ on a failure — a session takes up the
                // conversation it was in, not the one it was opened with.
                if session.started > 0 {
                    session.start = match (&session.conversation, session.kind) {
                        (Some(id), claude::Kind::Claude | claude::Kind::OpenCode) => claude::Start::Resume { id: id.clone(), fork: false },
                        (None, claude::Kind::OpenCode) => claude::Start::Continue,
                        _ => claude::Start::Fresh,
                    };
                }
                let mut command = commands.of(session.kind).to_vec();
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                let exists = |id: &str| home.as_deref().is_some_and(|home| conversations::claude_has(home, id));
                command.extend(session.launch_args(command.first().map_or("", String::as_str), exists));
                match pty::PtyProcess::spawn(session.id, &command, &dir, rows, cols, tx.clone()) {
                    Ok(process) => {
                        session.pane.resize(rows, cols);
                        session.pane.state = PaneState::Running;
                        session.started = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs() as i64);
                        session.pid = process.pid();
                        panes.insert(session.id, process);
                    }
                    Err(why) => session.pane.state = PaneState::Failed(why),
                }
            }
            PaneState::Running => {
                if let Some(process) = panes.get_mut(&session.id)
                    && (rows, cols) != process.size()
                    && rows > 0
                    && cols > 0
                {
                    process.resize(rows, cols);
                    session.pane.resize(rows, cols);
                }
            }
            PaneState::Idle | PaneState::Exited(_) | PaneState::Failed(_) => {
                panes.remove(&session.id);
            }
        }
    }
    // A closed session's program ends with it.
    panes.retain(|id, _| app.claude.list.iter().any(|s| s.id == *id));
    send_owed(app, panes);
}

/// What the folder picker wants read, on a thread of its own: a folder's folders come back at
/// once, a search as it goes. A newer lookup, or the picker closing, makes an older one stop.
fn drive_picker(app: &mut App, latest: &Arc<AtomicU64>, tx: &mpsc::UnboundedSender<Event>) {
    let claude::Mode::Opening(picker) = &mut app.claude.mode else {
        latest.store(0, Ordering::SeqCst);
        return;
    };
    let Some(lookup) = picker.lookup() else {
        return;
    };
    latest.store(lookup.generation, Ordering::SeqCst);
    let (latest, tx) = (Arc::clone(latest), tx.clone());
    std::thread::spawn(move || {
        folders::look(&lookup, &latest, &|found| {
            let _ = tx.send(Event::Folders(found));
        });
    });
}

/// Keys, pastes and answers owed to each program.
fn send_owed(app: &mut App, panes: &mut HashMap<u64, pty::PtyProcess>) {
    for session in &mut app.claude.list {
        if let Some(process) = panes.get_mut(&session.id) {
            let owed = session.pane.take_outbox();
            if !owed.is_empty() {
                process.write(&owed);
            }
        }
    }
}

/// Where prayer times are for: the place chosen, else the city of the machine's time zone.
fn prayer_place(config: &Config, zone: Option<&str>, table: Option<&str>) -> Option<prayer::Place> {
    if config.prayer.off {
        return None;
    }
    config.prayer.place.clone().or_else(|| prayer::of_time_zone(zone?, table))
}

/// A prayer's reminder, said beyond the screen: the terminal's bell, and the desktop's own
/// notification — through the terminal where it has one (iTerm2, Ghostty, WezTerm take OSC 9),
/// through the system where it does not (Terminal.app, tmux).
fn notify(how: config::Notify, text: &str) {
    use std::io::Write;
    if how == config::Notify::Off {
        return;
    }
    let text: String = text.chars().filter(|c| !c.is_control() && !matches!(c, '"' | '\\')).collect();
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    if how == config::Notify::Desktop {
        let terminal = std::env::var("TERM_PROGRAM").unwrap_or_default();
        if matches!(terminal.as_str(), "iTerm.app" | "ghostty" | "WezTerm") {
            let _ = write!(out, "\x1b]9;{text}\x1b\\");
        } else {
            system_notification(&text);
        }
    }
    let _ = out.flush();
}

/// Text on the clipboard: through the terminal (OSC 52 — iTerm2, kitty, WezTerm, Ghostty, tmux
/// with set-clipboard) and through the system's own tool where there is one, which works in a
/// terminal that does not take OSC 52.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };
    for (program, args) in tools {
        let child = std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A page in the browser — a ticket, a DAG's grid — from a program of its own that is not
/// waited for. Only the http(s) links views 5 and 6 make get this far.
fn open_in_browser(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return;
    };
    let child = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// The system's notification, from a program of its own that is not waited for.
fn system_notification(text: &str) {
    let command: Option<(&str, Vec<String>)> = if cfg!(target_os = "macos") {
        let script = format!("display notification \"{text}\" with title \"cobserve\" sound name \"Glass\"");
        Some(("osascript", vec!["-e".to_string(), script]))
    } else if cfg!(target_os = "linux") {
        Some(("notify-send", vec!["cobserve".to_string(), text.to_string()]))
    } else {
        None
    };
    let Some((program, args)) = command else {
        return;
    };
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// What is wrong before a single request is made, if anything. Shown in the footer area
/// instead of exiting, because the app still has to start.
fn startup_problem(config: &Config) -> Option<String> {
    if config.fake {
        return None;
    }
    if config.clickhouse.seeds.is_empty() {
        return Some("no CH_SEED_URLS".to_string());
    }
    None
}

/// The terminal closed (SIGHUP) or the process asked to end (SIGTERM): a quit like `q`, so the
/// sessions are kept for the next run before the programs in them end.
fn spawn_signal_watch(tx: mpsc::UnboundedSender<Event>) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut hangup), Ok(mut terminate)) = (signal(SignalKind::hangup()), signal(SignalKind::terminate())) else {
            return;
        };
        tokio::select! {
            _ = hangup.recv() => {}
            _ = terminate.recv() => {}
        }
        let _ = tx.send(Event::Quit);
    });
    #[cfg(not(unix))]
    drop(tx);
}

/// Keys on a dedicated OS thread: `crossterm::event::read` blocks, and the loop must not.
fn spawn_key_reader(tx: mpsc::UnboundedSender<Event>) {
    std::thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(TermEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                if tx.send(Event::Key(key)).is_err() {
                    return;
                }
            }
            // A click, a drag or the wheel; a mouse that only moves is not news.
            Ok(TermEvent::Mouse(mouse)) if mouse.kind != crossterm::event::MouseEventKind::Moved => {
                if tx.send(Event::Mouse(mouse)).is_err() {
                    return;
                }
            }
            Ok(TermEvent::Paste(text)) => {
                if tx.send(Event::Paste(text)).is_err() {
                    return;
                }
            }
            // A new size is drawn straight away: Claude's pane follows it.
            Ok(TermEvent::Resize(..)) => {
                if tx.send(Event::Tick).is_err() {
                    return;
                }
            }
            // A mouse report is not a key press, nor is a key released.
            Ok(_) => {}
            Err(_) => {
                let _ = tx.send(Event::Quit);
                return;
            }
        }
    });
}

/// What the screen asks of the sources: a Redash job cancelled, Airflow or Jira read again.
struct Asks {
    cancels: mpsc::UnboundedReceiver<String>,
    airflow: mpsc::UnboundedReceiver<()>,
    jira: mpsc::UnboundedReceiver<()>,
}

/// What reads the pages of views 5 and 6 when they ask: each ask a task of its own, so a page
/// never waits for a poll, nor the screen for a page.
struct Details {
    airflow: Option<sources::airflow::AirflowDetails>,
    jira: Option<sources::jira::JiraDetails>,
    fake: bool,
}

impl Details {
    fn read(&self, ask: detail::Ask, tx: mpsc::UnboundedSender<Event>) {
        use detail::{Ask, Body};
        if self.fake {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                let answer = fake::detail(&ask, now);
                let _ = tx.send(Event::Detail(ask, answer));
            });
            return;
        }
        let (airflow, jira) = (self.airflow.clone(), self.jira.clone());
        tokio::spawn(async move {
            let answer = match (&ask, airflow, jira) {
                (Ask::Issue(key), _, Some(jira)) => jira.issue(key).await.map(|i| Body::Issue(Box::new(i))),
                (Ask::Runs(dag), Some(airflow), _) => airflow.runs(dag).await.map(Body::Runs),
                (Ask::Tasks { dag, run }, Some(airflow), _) => airflow.tasks(dag, run).await.map(Body::Tasks),
                (Ask::Log { dag, run, task, map_index, attempt }, Some(airflow), _) => {
                    airflow.log(dag, run, task, *map_index, *attempt).await.map(|text| Body::Log(detail::log_lines(&text)))
                }
                (Ask::Issue(_), _, None) => Err("Jira is not configured".to_string()),
                (_, None, _) => Err("Airflow is not configured".to_string()),
            };
            let _ = tx.send(Event::Detail(ask, answer));
        });
    }
}

fn spawn_sources(
    config: &Config,
    tx: mpsc::UnboundedSender<Event>,
    console_targets: Arc<std::sync::Mutex<HashMap<String, sources::clickhouse::ConsoleTarget>>>,
    asks: Asks,
) -> Details {
    let Asks { mut cancels, airflow: airflow_asks, jira: jira_asks } = asks;
    let mut details = Details { airflow: None, jira: None, fake: config.fake };
    if config.fake {
        tokio::spawn(fake_loop(config.poll, tx, cancels, airflow_asks, jira_asks));
        return details;
    }

    // This machine, read as often as the fleet: no configuration, nothing on the network.
    tokio::spawn(sources::local::run(config.poll, tx.clone()));

    // Airflow and Jira are optional (§9, like Redash): unconfigured, their views say how.
    match sources::airflow::AirflowSource::new(&config.airflow) {
        Some(airflow) => {
            details.airflow = Some(airflow.details());
            tokio::spawn(airflow.run(tx.clone(), airflow_asks));
        }
        None => {
            let _ = tx.send(Event::Airflow(Box::new(airflow::Activity::unreachable(airflow::NOT_CONFIGURED))));
        }
    }
    match sources::jira::JiraSource::new(&config.jira) {
        Some(jira) => {
            details.jira = Some(jira.details());
            tokio::spawn(jira.run(tx.clone(), jira_asks));
        }
        None => {
            let _ = tx.send(Event::Jira(Box::new(jira::Board::unreachable(jira::NOT_CONFIGURED))));
        }
    }

    let clickhouse = config.clickhouse.clone();
    let poll = config.poll;
    let discover_every = config.discover_every;
    let queue_tx = tx.clone();
    tokio::spawn(async move {
        let mut source = match sources::clickhouse::ClickHouseSource::new(&clickhouse) {
            Ok(source) => source,
            Err(e) => {
                let _ = tx.send(Event::Notice(format!("ClickHouse source: {e}")));
                return;
            }
        };
        // Query sessions go where the fleet is: the seeds now, every server discovery finds.
        let publish = |source: &sources::clickhouse::ClickHouseSource| {
            if let Ok(mut targets) = console_targets.lock() {
                *targets = source.console_targets();
            }
        };
        publish(&source);
        // Nothing is printed here: stderr is the terminal ratatui draws on, and a line written
        // there stays on screen in whatever cells the next frame does not change. The bottom
        // border already says how many nodes are polled.

        // §6.2: discovery at start, then every 60 s.
        let mut discovery = tokio::time::interval(discover_every);
        discovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut polling = tokio::time::interval(poll);
        polling.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // Biased so that, at start, discovery runs before the first poll: otherwise the
            // first snapshot can carry a seed's raw address and every node discovery then
            // names would be flagged NEW.
            tokio::select! {
                biased;
                _ = discovery.tick() => {
                    for error in source.discover().await {
                        let _ = tx.send(Event::Notice(format!("discovery via {error}")));
                    }
                    publish(&source);
                }
                _ = polling.tick() => {
                    let snapshot = source.poll().await;
                    if tx.send(Event::Snapshot(Box::new(snapshot))).is_err() {
                        return;
                    }
                }
            }
        }
    });

    match sources::redash::RedashSource::new(&config.redash) {
        Some(redash) => {
            tokio::spawn(async move {
                redash.run(queue_tx, cancels).await;
            });
        }
        // Redash is optional (§9): the strip says so instead of the app refusing to start.
        None => {
            let _ = queue_tx.send(Event::Queue(Box::new(model::QueueStatus::unreachable(
                model::QUEUE_NOT_CONFIGURED,
            ))));
            tokio::spawn(async move {
                while let Some(id) = cancels.recv().await {
                    let _ = queue_tx.send(Event::Cancelled(id, Err("Redash is not configured".into())));
                }
            });
        }
    }
    details
}

/// FAKE=1: generated data on the same timers the real sources use, so the UI cannot tell the
/// difference (DESIGN.md §8, step 1 of the build order).
async fn fake_loop(
    poll: Duration,
    tx: mpsc::UnboundedSender<Event>,
    mut cancels: mpsc::UnboundedReceiver<String>,
    mut airflow_asks: mpsc::UnboundedReceiver<()>,
    mut jira_asks: mpsc::UnboundedReceiver<()>,
) {
    let mut fake = fake::FakeSource::new();
    let mut local_step = 0;
    let mut polling = tokio::time::interval(poll);
    polling.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // §6.3: the queue moves every 3 s, the fleet every POLL_MS; Airflow is read every 15 s and
    // Jira every minute, as their sources do.
    let mut queue = tokio::time::interval(Duration::from_secs(3));
    queue.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut airflow = tokio::time::interval(Duration::from_secs(15));
    airflow.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut jira = tokio::time::interval(Duration::from_secs(60));
    jira.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);

    loop {
        tokio::select! {
            _ = polling.tick() => {
                if tx.send(Event::Snapshot(Box::new(fake.snapshot()))).is_err() {
                    return;
                }
                local_step += 1;
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                let _ = tx.send(Event::Local(Box::new(fake::local(local_step, at))));
            }
            _ = queue.tick() => {
                if tx.send(Event::Queue(Box::new(fake.queue()))).is_err() {
                    return;
                }
            }
            // A moment's wait when asked, so `r` looks like a read.
            Some(()) = airflow_asks.recv() => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = tx.send(Event::Airflow(Box::new(fake::airflow(now()))));
            }
            Some(()) = jira_asks.recv() => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = tx.send(Event::Jira(Box::new(fake::jira(now()))));
            }
            _ = airflow.tick() => {
                if tx.send(Event::Airflow(Box::new(fake::airflow(now())))).is_err() {
                    return;
                }
            }
            _ = jira.tick() => {
                if tx.send(Event::Jira(Box::new(fake::jira(now())))).is_err() {
                    return;
                }
            }
            Some(id) = cancels.recv() => {
                let result = fake.cancel(&id);
                let _ = tx.send(Event::Cancelled(id, result));
                if tx.send(Event::Queue(Box::new(fake.queue_now()))).is_err() {
                    return;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        let cases: [(&[u8], &str); 5] = [(b"", ""), (b"f", "Zg=="), (b"fo", "Zm8="), (b"foo", "Zm9v"), (b"foobar", "Zm9vYmFy")];
        for (bytes, encoded) in cases {
            assert_eq!(super::base64(bytes), encoded);
        }
    }
}
