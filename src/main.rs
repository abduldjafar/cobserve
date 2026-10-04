//! Terminal setup and the event loop (DESIGN.md §8).
//!
//! Sources are tokio tasks that push `Event`s down one channel; the loop applies them to
//! `App` and redraws. Key events come from crossterm's `EventStream`, a 1 s tick drives the
//! clock, and **the UI never blocks on the network**: a slow node, a dead Redash, or a
//! ClickHouse that never answers all show up as numbers that do not move.

mod app;
mod attrib;
mod claude;
mod config;
mod fake;
mod fmt;
mod history;
mod insight;
mod model;
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
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    // Before the terminal is touched: a configuration mistake should print a line, not leave
    // a mangled terminal behind.
    let args = match config::Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("pay_monitoring: {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", config::USAGE);
        return Ok(());
    }
    if args.version {
        println!("pay_monitoring {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let config = match Config::load(&args) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("pay_monitoring: {e}");
            std::process::exit(2);
        }
    };
    // Before any source starts: every person they attribute is displayed with it (§6.4).
    attrib::set_home_domain(config.email_domain.clone());

    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(e) => {
            // Piping this binary into a file is a normal thing to do; a panic is not the answer.
            eprintln!("pay_monitoring: no terminal to draw on ({e}). Run it in a terminal, or FAKE=1 with a TTY capture.");
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
    app.claude.command = config.claude_command.clone();
    if let Ok(dir) = std::env::current_dir() {
        app.claude.default_dir = pty::tilde(&dir);
    }
    if open_claude {
        app.open_claude();
    }
    // View 5's programs, one per session. Dropping one ends it — on quit, all of them.
    let mut panes: HashMap<u64, pty::PtyProcess> = HashMap::new();
    // When the sessions' branches were last read: Claude, or you, can change them.
    let mut branches_read = std::time::Instant::now();

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

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;
        // After the frame: the panes now know the size they are drawn at.
        drive_panes(&mut app, &mut panes, &tx);
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

        if app.quit {
            break;
        }
    }
    Ok(())
}

/// View 5's programs: each started when its session asks for it, all sized to what was drawn
/// (a session switched to is then the right size already), sent what is owed to them, and let
/// go of once they ended or their session was closed.
fn drive_panes(app: &mut App, panes: &mut HashMap<u64, pty::PtyProcess>, tx: &mpsc::UnboundedSender<Event>) {
    use claude::PaneState;
    let (rows, cols) = app.claude.want_size.get();
    let command = app.claude.command.clone();
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
                match pty::PtyProcess::spawn(session.id, &command, &dir, rows, cols, tx.clone()) {
                    Ok(process) => {
                        session.pane.resize(rows, cols);
                        session.pane.state = PaneState::Running;
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