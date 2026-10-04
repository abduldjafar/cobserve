//! Terminal setup and the event loop (DESIGN.md §8).
//!
//! Sources are tokio tasks that push `Event`s down one channel; the loop applies them to
//! `App` and redraws. Key events come from crossterm's `EventStream`, a 1 s tick drives the
//! clock, and **the UI never blocks on the network**: a slow node, a dead Redash, or a
//! ClickHouse that never answers all show up as numbers that do not move.

mod app;
mod attrib;
mod config;
mod fake;
mod fmt;
mod history;
mod insight;
mod model;
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

    let result = run(&mut terminal, config).await;

    ratatui::restore();
    result
}

/// §12: the hook has to leave raw mode and the alternate screen, or a crash leaves the user's
/// shell unusable. Installed before anything enters raw mode.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    let _ = color_eyre::install();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}

async fn run(terminal: &mut DefaultTerminal, config: Config) -> color_eyre::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut app = App::new();
    app.poll_interval = config.poll;

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

        tokio::select! {
            event = rx.recv() => match event {
                Some(event) => app.update(event),
                // Every source is gone; nothing left to wait for.
                None => app.update(Event::Quit),
            },
            _ = tick.tick() => app.update(Event::Tick),
        }

        if app.quit {
            break;
        }
    }
    Ok(())
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
            // Resizes redraw on the next event; a mouse report is not a key press.
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
                        let _ = tx.send(Event::Notice(format!("discovery: {error}")));
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