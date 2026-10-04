//! Terminal setup and the event loop (DESIGN.md §8).
//!
//! Sources are tokio tasks that push `Event`s down one channel; the loop applies them to
//! `App` and redraws. Key events come from crossterm's `EventStream`, a 1 s tick drives the
//! clock, and **the UI never blocks on the network**: a slow node, a dead Redash, or a
//! ClickHouse that never answers all show up as numbers that do not move.

mod app;
mod attrib;
mod claude;
mod clock;
mod config;
mod conversations;
mod fake;
mod fmt;
mod folders;
mod history;
mod insight;
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
use std::sync::atomic::{AtomicU64, Ordering};
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

    // The terminal's title is the monitor's while it runs, and the one it had after.
    {
        use std::io::Write;
        let _ = std::io::stdout().write_all(b"\x1b[22;0t");
    }
    let result = run(&mut terminal, config, args.claude).await;

    restore_terminal();
    result
}

fn restore_terminal() {
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

async fn run(terminal: &mut DefaultTerminal, config: Config, open_claude: bool) -> color_eyre::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut app = App::new();
    app.poll_interval = config.poll;
    app.claude.commands = claude::Commands {
        claude: config.claude_command.clone(),
        opencode: config.opencode_command.clone(),
        terminal: config.shell_command.clone(),
    };
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

    spawn_sources(&config, tx.clone());
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
    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;
        let title = ui::title(&app);
        if title != shown_title {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(&title));
            shown_title = title;
        }
        // After the frame: the panes now know the size they are drawn at.
        drive_panes(&mut app, &mut panes, &tx);
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

fn spawn_sources(config: &Config, tx: mpsc::UnboundedSender<Event>) {
    if config.fake {
        tokio::spawn(fake_loop(config.poll, tx));
        return;
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
                redash.run(queue_tx).await;
            });
        }
        // Redash is optional (§9): the strip says so instead of the app refusing to start.
        None => {
            let _ = queue_tx.send(Event::Queue(Box::new(model::QueueStatus::unreachable(
                model::QUEUE_NOT_CONFIGURED,
            ))));
        }
    }
}

/// FAKE=1: generated data on the same timers the real sources use, so the UI cannot tell the
/// difference (DESIGN.md §8, step 1 of the build order).
async fn fake_loop(poll: Duration, tx: mpsc::UnboundedSender<Event>) {
    let mut fake = fake::FakeSource::new();
    let mut polling = tokio::time::interval(poll);
    polling.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // §6.3: the queue moves every 3 s, the fleet every POLL_MS.
    let mut queue = tokio::time::interval(Duration::from_secs(3));
    queue.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = polling.tick() => {
                if tx.send(Event::Snapshot(Box::new(fake.snapshot()))).is_err() {
                    return;
                }
            }
            _ = queue.tick() => {
                if tx.send(Event::Queue(Box::new(fake.queue()))).is_err() {
                    return;
                }
            }
        }
    }
}