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
mod model;
mod sources;
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
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Before the terminal is touched: a configuration mistake should print a line,
            // not leave a mangled terminal behind.
            eprintln!("pay_monitoring: {e}");
            std::process::exit(2);
        }
    };

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

    spawn_sources(&config, tx.clone());
    // Anything a source could not even start with belongs on screen, not in a log file.
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

/// What the app is about to poll, for the log line at startup. No credential is ever in it.
fn fleet_summary(config: &Config, source: &sources::clickhouse::ClickHouseSource) -> String {
    let names: Vec<&str> = source
        .targets()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    format!(
        "polling {} node(s): {} · cluster {} · every {} ms",
        names.len(),
        names.join(" "),
        if config.clickhouse.cluster.is_empty() {
            "—"
        } else {
            &config.clickhouse.cluster
        },
        config.poll.as_millis()
    )
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
    let summary_config = config.clone();
    tokio::spawn(async move {
        let mut source = match sources::clickhouse::ClickHouseSource::new(&clickhouse) {
            Ok(source) => source,
            Err(e) => {
                let _ = tx.send(Event::Notice(format!("ClickHouse source: {e}")));
                return;
            }
        };
        eprintln!("{}", fleet_summary(&summary_config, &source));

        // §6.2: discovery at start, then every 60 s.
        let mut discovery = tokio::time::interval(discover_every);
        discovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut polling = tokio::time::interval(poll);
        polling.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
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
            let _ = queue_tx.send(Event::Notice(
                "no REDASH_URL / REDASH_ADMIN_API_KEY — the queue strip stays empty".to_string(),
            ));
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