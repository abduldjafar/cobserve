//! Today's views, drawn inside the new shell's zooms (`ui2/views`), through ratatui's
//! `TestBackend` (DESIGN.md §10): the queue, the sessions and their consoles, Airflow, Jira, the
//! tape, this machine. Ported from the old shell's tests when `src/ui/` went (redesign phase 6):
//! the views are the same code, reached by the new shell's keys (F1–F8 zoom a panel, F10 this
//! machine; the float's `ctrl+\` prefix for the sessions).
//!
//! Not ported, their subject gone with the old shell:
//! - the target size, every view at every size, narrow columns, short drawer: the old layout
//!   (`ui2::tests` has the grid, the zooms and the 500-size fuzz);
//! - the tree's grid, selecting a query and its SQL under it, J/K in the tree: view 1's tree is
//!   gone (the NODE zoom shows the chosen row's query, `ui2::tests`);
//! - every node a card under the band: the band and the cards are gone (FLEET and NODES);
//! - the day line and the prayer's reminder on it: the top bar draws the day now (`ui2::tests`);
//! - the header's tabs, the masthead's notice, the help overlay, the footer's filter: chrome that
//!   is the top bar, toasts, the palette's keys box and the status bar now;
//! - view 5 under the monitor's strip of nodes: the sessions are a zoom, the strip went with it;
//! - the map, the pivot, the insights and tab between them: replaced by NODES, the palette's
//!   people channel and the panels' summaries;
//! - a real fleet read in a few lines: the old band's own formatting;
//! - the AI in sight from every view: the old band's line (this machine's zoom keeps it);
//! - the dump and export helpers: `ui2::tests::dump` prints a clean frame.

use super::*;
use crate::app::{App, Event, Hit};
use crate::fake::FakeSource;
use crate::fmt;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// A panel zoomed by its F-key, as from anywhere: 2 the queue, 5 Airflow, 6 Jira, 7 the tape,
/// 8 the sessions, 10 this machine.
fn zoom(app: &mut App, f: u8) {
    app.update(key(KeyCode::F(f)));
}

/// The fake fleet after `polls` polls, so the sparklines and trends have history behind them.
fn app_after(polls: usize) -> App {
    let mut fake = FakeSource::new();
    let mut app = App::new();
    for i in 0..polls.max(1) {
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        if i % 3 == 0 {
            app.update(Event::Queue(Box::new(fake.queue())));
        }
    }
    app.update(Event::Queue(Box::new(fake.queue())));
    app
}

fn app_with_fake() -> App {
    app_after(1)
}

fn buffer(app: &App, width: u16, height: u16) -> Buffer {
    let theme = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|frame| crate::ui2::draw_with(frame, app, &theme)).expect("draw");
    terminal.backend().buffer().clone()
}

fn text_of(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|row| row.concat())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(app: &App, width: u16, height: u16) -> String {
    text_of(&buffer(app, width, height))
}

/// The sessions zoomed, a first one opened when there is none — as `7` was on the old shell.
fn sessions(app: &mut App) {
    app.open_claude();
    zoom(app, 8);
}

/// The float's prefix and a command: `ctrl+\` then `c`, the session floating first.
fn prefix(app: &mut App, c: char) {
    if app.shell.float.is_none() {
        crate::ui2::sessions::open(app);
    }
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char(c)));
}

/// A zoomed row without the zoom's frame and gutter.
fn inner(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    chars.get(2..chars.len().saturating_sub(2)).map(|c| c.iter().collect()).unwrap_or_default()
}

/// The status bar: the last line.
fn footer(screen: &str) -> String {
    screen.lines().last().unwrap_or_default().to_string()
}

#[test]
fn the_queue_view_lists_people_and_the_stitch_to_clickhouse() {
    let mut app = app_after(3);
    zoom(&mut app, 2);
    let screen = render(&app, 140, 48);
    assert!(screen.contains("RUNNING · 6 on a worker"), "{screen}");
    assert!(screen.contains("─ WAITING ·"), "{screen}");
    assert!(screen.contains("→ clickhouse3"), "the arrow into ClickHouse: {screen}");
    assert!(screen.contains("mysql · not ClickHouse"), "a job on another database says so: {screen}");
    assert!(screen.contains("runs inside Redash"), "Query Results is not a missing stitch: {screen}");
    assert!(screen.contains("4/4 busy"), "worker saturation");
    assert!(screen.contains("●●●●"), "workers drawn as slots");
    assert!(screen.contains("runaway ClickHouse query"), "the drawer explains why it is full: {screen}");
    assert!(!screen.contains("r_redash →"), "people, not the account Redash connects as: {screen}");
    assert!(!screen.contains("FAILED"), "no column the API cannot fill: {screen}");
    // Running jobs are listed before the waiting ones, and the waiting ones as waiting.
    let running = screen.find("─ RUNNING ·").expect("running section");
    let waiting = screen.find("─ WAITING ·").expect("waiting section");
    let simonyte = screen.find("#8093 July close pack").expect("r.simonyte waits");
    assert!(running < waiting && waiting < simonyte, "r.simonyte is in the WAITING list");
    // The cursor is on the first running job, and its SQL is open under it.
    assert!(screen.contains("▎ WITH BankRecord AS ("), "{screen}");
}

#[test]
fn x_then_y_cancels_a_job_in_redash_and_any_other_key_keeps_it() {
    let mut fake = FakeSource::new();
    let mut app = App::new();
    for _ in 0..3 {
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
    }
    app.update(Event::Queue(Box::new(fake.queue())));
    zoom(&mut app, 2);
    render(&app, 140, 48);
    let job = app.selected_job().cloned().expect("the first running job under the cursor");
    assert_eq!(crate::app::job_words(&job), "#7438 Gateway transfers of grigol.gankava");
    assert!(footer(&render(&app, 140, 48)).contains("x cancel job"));

    // x asks, about that job, in the dialog, and says what a cancel does to it.
    app.update(key(KeyCode::Char('x')));
    let screen = render(&app, 140, 48);
    assert!(screen.contains(&format!("Cancel Redash job {} · grigol.gankava?", job.id)), "{screen}");
    assert!(screen.contains("#7438 Gateway transfers · clickhouse3 · running"), "{screen}");
    assert!(screen.contains("Its query on clickhouse3 keeps running: Redash's cancel does not reach it"), "{screen}");
    assert!(screen.contains("[y] cancel the job     [esc] keep it"), "{screen}");
    // Any other key keeps it, and does nothing else: the cursor has not moved.
    app.update(key(KeyCode::Down));
    assert!(app.take_cancels().is_empty() && app.cancel_asked().is_none());
    assert_eq!(app.selected_job().map(|j| j.id.clone()), Some(job.id.clone()), "the key only answered");

    // x, then y: sent, and marked until Redash answers.
    app.update(key(KeyCode::Char('x')));
    app.update(key(KeyCode::Char('y')));
    assert_eq!(app.take_cancels(), std::slice::from_ref(&job.id));
    assert_eq!(app.notice(), Some("asking Redash to cancel #7438 Gateway transfers of grigol.gankava…"));
    assert!(render(&app, 140, 48).contains("⊘ cancelling… · → clickhouse3"));
    app.update(key(KeyCode::Char('x')));
    assert!(app.cancel_asked().is_none() && app.notice().is_some_and(|n| n.contains("already been asked")), "not twice");

    // Redash says yes: the query in ClickHouse is not stopped by that, and the notice says so.
    app.update(Event::Cancelled(job.id.clone(), Ok(())));
    assert_eq!(
        app.notice(),
        Some("Redash cancelled #7438 Gateway transfers of grigol.gankava — its query runs on in ClickHouse on clickhouse3 until KILL QUERY stops it")
    );
    assert!(render(&app, 140, 48).contains("⊘ cancelled · → clickhouse3"));
    // And it is on the tape, where it can be read later.
    zoom(&mut app, 7);
    let tape = render(&app, 160, 48);
    assert!(tape.contains("Redash job cancelled from here: #7438 Gateway transfers of grigol.gankava — its query runs on in ClickHouse on clickhouse3"), "{tape}");
    zoom(&mut app, 2);
    // Once the worker has let go, the job is gone from the list, and its mark with it.
    fake.cancel(&job.id).expect("the fake has it");
    app.update(Event::Queue(Box::new(fake.queue_now())));
    let screen = render(&app, 140, 48);
    assert!(!screen.contains("⊘") && !app.queue_rows().iter().any(|j| j.id == job.id), "{screen}");
    assert!(screen.contains("RUNNING · 5 on a worker"), "{screen}");

    // Redash refuses: said, and the mark goes.
    let other = app.selected_job().cloned().expect("another job under the cursor");
    app.update(key(KeyCode::Char('x')));
    app.update(key(KeyCode::Char('y')));
    app.take_cancels();
    app.update(Event::Cancelled(other.id.clone(), Err("HTTP 403 · the API key has to be an admin's".into())));
    let said = app.notice().unwrap_or_default().to_string();
    assert!(said.starts_with("Redash did not cancel ") && said.ends_with(": HTTP 403 · the API key has to be an admin's"), "{said}");
    assert!(app.cancelling(&other.id).is_none());

    // A waiting job leaves its queue.
    let first_waiting = app.queue_rows().iter().position(|j| j.state == crate::model::JobState::Queued).expect("a waiting job");
    let at = app.queue_rows().iter().position(|j| j.id == other.id).unwrap();
    for _ in at..first_waiting {
        app.update(key(KeyCode::Down));
    }
    app.update(key(KeyCode::Char('x')));
    let screen = render(&app, 140, 48);
    assert!(screen.contains("It leaves the queue and never runs"), "{screen}");
    app.update(key(KeyCode::Esc));
    assert!(app.cancel_asked().is_none() && app.take_cancels().is_empty(), "esc keeps it");
}

#[test]
fn j_and_k_scroll_the_sql_under_a_job() {
    let mut app = app_after(3);
    zoom(&mut app, 2);
    // A short screen, so the SQL under the first running job has to scroll.
    let before = render(&app, 120, 30);
    assert!(before.contains("lines 1–"), "{before}");
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT)));
    let after = render(&app, 120, 30);
    assert!(after.contains("lines 2–"), "{after}");
    // Another job starts at its first line again.
    app.update(key(KeyCode::Down));
    let next = render(&app, 120, 30);
    assert!(!next.contains("lines 2–"), "{next}");
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// The session on screen as `main.rs` leaves it once its program runs, and what it printed.
fn run_session(app: &mut App, output: &[u8]) -> u64 {
    let session = app.claude.current_mut().expect("a session");
    session.pane.state = crate::claude::PaneState::Running;
    session.pane.resize(29, 116);
    let id = session.id;
    app.update(Event::Pane(id, output.to_vec()));
    id
}

/// View 5 with a program that has printed something, as the pane would after its first output.
fn app_with_claude(output: &[u8]) -> App {
    let mut app = app_after(3);
    app.claude.default_dir = "~/work/cobserve".into();
    sessions(&mut app);
    run_session(&mut app, output);
    app
}

#[test]
fn sessions_are_a_list_beside_claude_and_can_be_renamed() {
    let mut app = app_with_claude(b"the first session\r\n");
    let first = app.claude.current().unwrap().id;
    // ctrl+\ n (the float's prefix) asks where, starting where the first works; ⏎ opens it there.
    prefix(&mut app, 'n');
    let asking = render(&app, 140, 40);
    assert!(asking.contains("New Claude session") && asking.contains("Open the session here"), "{asking}");
    app.update(key(KeyCode::Enter));
    run_session(&mut app, b"the second session\r\n");
    // ctrl+\ r: named.
    prefix(&mut app, 'r');
    for c in "infra".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    let typing = render(&app, 140, 40);
    assert!(typing.contains("infra▏"), "the name is typed in its place: {typing}");
    assert!(typing.contains("keep the name"), "{typing}");
    app.update(key(KeyCode::Enter));
    // The first one rings while the second is on screen.
    app.update(Event::Pane(first, b"\x07".to_vec()));

    let screen = render(&app, 140, 40);
    assert!(screen.contains("2 sessions") && screen.contains("1 needs you"), "{screen}");
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row("✻ 7 · cobserve").contains("! needs you"), "the first, by its folder, which rang: {screen}");
    assert!(row("✻ 8 · infra").contains("working"), "the second, by its name, printing: {screen}");
    assert_eq!(app.claude.current().map(|s| s.id), app.claude.list.get(1).map(|s| s.id), "the second is the chosen one");
    assert!(screen.contains("the second session") && screen.contains("the first session"), "each card shows its own screen: {screen}");
    assert!(screen.contains("n new session") && screen.contains("↻ past conversations"), "{screen}");
    // The zoom's keys are on the status bar.
    let keys = footer(&screen);
    assert!(keys.contains("↑↓ a session") && keys.contains("⏎ open it") && keys.contains("\\ float it"), "{keys}");
    // x twice closes the session on screen.
    app.update(key(KeyCode::Char('x')));
    assert!(render(&app, 140, 40).contains("x again closes the session"));
    app.update(key(KeyCode::Char('x')));
    assert_eq!(app.claude.list.len(), 1);
}

/// A click where the last frame drew `hit`.
fn click_on(app: &mut App, hit: crate::app::Hit) {
    let at = app.viewport.hits.borrow().iter().find(|(_, h)| *h == hit).map(|(r, _)| *r);
    let at = at.unwrap_or_else(|| panic!("{hit:?} is not on screen"));
    app.update(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: at.x + at.width / 2,
        row: at.y,
        modifiers: KeyModifiers::NONE,
    }));
}

/// The picker's lookup answered with `folders` (name, branch) in the folder it is in, `hit` the
/// part of each name a search matched.
fn answer_picker(app: &mut App, branch: Option<&str>, folders: &[(&str, Option<&str>)], hit: Option<(usize, usize)>) {
    let crate::claude::Mode::Opening(picker) = &mut app.claude.mode else {
        panic!("not picking");
    };
    let lookup = picker.lookup().expect("a lookup");
    let folders = folders
        .iter()
        .map(|(name, branch)| crate::folders::Folder {
            path: lookup.dir.join(name),
            shown: name.to_string(),
            hit,
            branch: branch.map(String::from),
        })
        .collect();
    let found = crate::folders::Found {
        generation: lookup.generation,
        dir: Some(lookup.dir.clone()),
        branch: branch.map(String::from),
        folders,
        done: true,
        ..Default::default()
    };
    app.update(Event::Folders(found));
}

#[test]
fn a_new_session_s_folder_is_picked_with_clicks_like_in_an_explorer() {
    use crate::app::Hit;
    use crate::claude::{Mode, PickRow};
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let mut app = app_with_claude(b"one\r\n");
    prefix(&mut app, 'n');
    answer_picker(&mut app, Some("main"), &[("docs", None), ("src", None), ("tools", Some("dev"))], None);
    let screen = render(&app, 140, 40);
    for text in ["New Claude session", "where should it work?", "✕ cancel", "⌕", "search the folders below here", "3 folders"] {
        assert!(screen.contains(text), "{text}: {screen}");
    }
    let row = |screen: &str, text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row(&screen, "~ › work › cobserve").contains("⎇ main"), "where it is, and its branch: {screen}");
    assert!(row(&screen, "Open the session here").contains('▌'), "the cursor starts on it: {screen}");
    assert!(row(&screen, "↰  ..").contains("up to ~/work"), "{screen}");
    assert!(row(&screen, "▸  tools").contains("⎇ dev"), "a repository, with its branch: {screen}");

    // A click on a folder goes into it; one on a step of the path goes back up to it.
    click_on(&mut app, Hit::Pick(PickRow::Folder(0)));
    let Mode::Opening(picker) = &app.claude.mode else { panic!() };
    assert_eq!(picker.dir, home.join("work/cobserve/docs"));
    answer_picker(&mut app, None, &[], None);
    let inside = render(&app, 140, 40);
    assert!(inside.contains("~ › work › cobserve › docs") && inside.contains("no folders in here"), "{inside}");
    click_on(&mut app, Hit::Crumb(1));
    let Mode::Opening(picker) = &app.claude.mode else { panic!() };
    assert_eq!(picker.dir, home.join("work"));
    answer_picker(&mut app, None, &[("cobserve", Some("main")), ("pipelines", None)], None);

    // Typed, it searches below; what it found is listed with what matched.
    for c in "pipe".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    answer_picker(&mut app, None, &[("pipelines", None)], Some((0, 4)));
    let found = render(&app, 140, 40);
    assert!(found.contains("pipe▏") && found.contains("1 found") && !found.contains("Open the session here"), "{found}");
    assert!(row(&found, "▸  pipelines").contains('▌'), "the best match under the cursor: {found}");
    assert!(footer(&found).contains("esc clear the search"), "esc clears it first: {found}");

    // ⏎ opens the session in the folder under the cursor, and it is on screen.
    app.update(key(KeyCode::Enter));
    assert_eq!(app.claude.mode, Mode::Typing);
    assert_eq!((app.claude.list.len(), app.claude.current().unwrap().dir.as_str()), (2, "~/work/pipelines"));

    // A click on the first row opens it in the folder looked at; ✕ gives up.
    prefix(&mut app, 'n');
    answer_picker(&mut app, None, &[], None);
    render(&app, 140, 40);
    click_on(&mut app, Hit::Cancel);
    assert_eq!((app.claude.mode.clone(), app.claude.list.len()), (Mode::Typing, 2));
    prefix(&mut app, 'n');
    render(&app, 140, 40);
    click_on(&mut app, Hit::Pick(PickRow::Here));
    assert_eq!((app.claude.list.len(), app.claude.current().unwrap().dir.as_str()), (3, "~/work/pipelines"));
}

#[test]
fn opencode_and_a_terminal_sit_in_the_list_beside_claude() {
    use crate::app::Hit;
    use crate::claude::{Kind, Mode, PaneState};
    let mut app = app_with_claude(b"one\r\n");
    app.claude.open_new("~/work/notes", Kind::OpenCode);
    run_session(&mut app, b"opencode here\r\n");
    app.claude.open_new("~/work/scripts", Kind::Terminal);
    run_session(&mut app, b"$ ls\r\n");
    let screen = render(&app, 160, 40);
    let row = |screen: &str, text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row(&screen, "✻ 7 · cobserve").contains("▣ 8 · notes"), "Claude's mark, and OpenCode's beside it: {screen}");
    assert!(row(&screen, "❯ 9 · scripts").contains("❯"), "a shell's: {screen}");
    app.update(key(KeyCode::Enter));
    assert!(footer(&render(&app, 160, 40)).contains("every other key goes to the shell"), "opened full");
    app.update(ctrl('\\'));
    assert!(!app.shell.sessions_full, "ctrl+\\ is back on the list");

    // The picker offers the three, the one it will open lit; a click changes it.
    prefix(&mut app, 'n');
    let picking = render(&app, 160, 40);
    for text in ["New terminal", "✻ Claude", "▣ OpenCode", "❯ Terminal", "shift+tab switches"] {
        assert!(picking.contains(text), "{text}: {picking}");
    }
    click_on(&mut app, Hit::Kind(Kind::OpenCode));
    assert!(render(&app, 160, 40).contains("New OpenCode session"));
    app.update(key(KeyCode::Esc));

    // One that would not start says how to get it, for what it is.
    app.claude.select(1);
    app.claude.current_mut().unwrap().pane.state = PaneState::Failed("opencode is not installed here (not on PATH)".into());
    let failed = render(&app, 160, 40);
    assert!(row(&failed, "▣ 8 · notes").contains("✕ ended"), "{failed}");
    assert!(failed.contains("opencode is not installed here"), "the reason in its card: {failed}");
    // Opened full, the whole of how to get it.
    app.update(key(KeyCode::Enter));
    let full = render(&app, 160, 40);
    assert!(full.contains("opencode auth login") && full.contains("OPENCODE_CMD"), "{full}");
    assert!(footer(&full).contains("⏎ start OpenCode"), "opened full, it says how to start it");
    assert_eq!(app.claude.mode, Mode::Typing);
}

#[test]
fn the_terminal_s_title_says_how_the_fleet_is_and_the_next_prayer() {
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT);
    assert_eq!(crate::ui2::title(&app), "cobserve ✖ · Maghrib 17:50");
    at_moment(&mut app, MOMENT + 6504);
    assert_eq!(crate::ui2::title(&app), "cobserve ✖ · ◷ Maghrib in 10 min");
    at_moment(&mut app, MOMENT + 7090);
    assert_eq!(crate::ui2::title(&app), "cobserve ✖ · Maghrib now");
}

#[test]
fn the_clock_flips_to_utc_from_the_palette_or_a_click() {
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT);
    let clock = crate::action::ACTIONS.iter().find(|a| a.id == "clock").unwrap();
    (clock.run)(&mut app, "");
    let top = render(&app, 120, 36).lines().next().unwrap().to_string();
    assert!(top.contains("08:52:07 UTC"), "{top}");
    // Prayer times stay the place's own.
    assert!(top.contains("17:50"), "{top}");
    // A click on the clock flips it back.
    app.update(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 115,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("15:52:07 WIB"));
    // The tape's times are the clock's.
    zoom(&mut app, 7);
    assert!(render(&app, 120, 36).contains("TIME WIB"));
}

#[test]
fn sessions_of_the_last_run_wait_marked_and_a_conversation_can_be_taken_up() {
    use crate::app::Hit;
    use crate::claude::{Kind, Mode, PickRow};
    use crate::saved::{Saved, SavedKind, SavedSession};
    let mut app = app_after(3);
    app.claude.default_dir = "~/work/cobserve".into();
    let kept = |kind, dir: &str, conversation: Option<&str>| SavedSession { kind, name: None, dir: dir.into(), conversation: conversation.map(str::to_string), ..SavedSession::default() };
    app.claude.restore(&Saved {
        sessions: vec![kept(SavedKind::Claude, "~/work/billing", Some("aaa")), kept(SavedKind::OpenCode, "~/work/pipelines", None)],
        active: 0,
    });
    sessions(&mut app);
    let screen = render(&app, 140, 40);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row("▣ 8 · pipelines").contains("↻ kept"), "kept, not started: {screen}");
    assert!(!app.claude.list[0].waiting() && row("✻ 7 · billing").contains("◌ starting"), "the one on screen started: {screen}");
    assert!(screen.contains("↻ past conversations"), "{screen}");

    // The way to a conversation had elsewhere: every folder's, newest first.
    click_on(&mut app, Hit::Resume);
    let now = app.now();
    let Mode::Opening(picker) = &mut app.claude.mode else { panic!("not picking") };
    assert!(picker.everywhere);
    let lookup = picker.conversation_lookup().expect("asked for");
    assert_eq!((lookup.kind, lookup.dir), (Kind::Claude, None));
    let conversation = |id: &str, title: &str, dir: &str, ago: i64, open: bool| crate::conversations::Conversation {
        kind: Kind::Claude,
        id: id.into(),
        dir: dir.into(),
        title: title.into(),
        last: None,
        updated: now - ago,
        open,
    };
    picker.found_conversations(
        lookup.generation,
        Kind::Claude,
        true,
        vec![conversation("ccc", "stream the invoice export", "/work/billing", 300, true), conversation("ddd", "weekly totals", "/work/reports", 7200, false)],
    );
    let screen = render(&app, 140, 40);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(screen.contains("take a conversation up, from any folder"), "{screen}");
    assert!(row("↻  stream the invoice export").contains("/work/billing · 5m ago · open in another terminal"), "{screen}");
    assert!(row("↻  weekly totals").contains("/work/reports · 2h ago"), "{screen}");
    click_on(&mut app, Hit::Pick(PickRow::Conversation(1)));
    assert_eq!(app.claude.mode, Mode::Typing);
    let session = app.claude.current().unwrap();
    assert_eq!((session.dir.as_str(), session.conversation.as_deref()), ("/work/reports", Some("ddd")));
    assert_eq!(session.launch_args("claude", |_| true), ["--resume", "ddd"]);
}

#[test]
fn a_shell_scrolled_back_says_how_far() {
    use crate::claude::Kind;
    let mut app = app_with_claude(b"one\r\n");
    app.claude.open_new("~/work/scripts", Kind::Terminal);
    let lines: String = (1..=80).map(|n| format!("output {n}\r\n")).collect();
    run_session(&mut app, lines.as_bytes());
    // The pane takes the wheel opened full.
    app.update(key(KeyCode::Enter));
    render(&app, 160, 40);
    let pane = app.viewport.hits.borrow().iter().find(|(_, h)| *h == crate::app::Hit::Pane).map(|(r, _)| *r).unwrap();
    for _ in 0..4 {
        app.update(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: pane.x + 5,
            row: pane.y + 5,
            modifiers: KeyModifiers::NONE,
        }));
    }
    let screen = render(&app, 160, 40);
    assert!(screen.contains("↑ 12 lines back"), "{screen}");
    // A key goes to the shell, and the pane comes back down.
    app.update(key(KeyCode::Char('q')));
    assert!(!render(&app, 160, 40).contains("lines back"), "a key comes back down");
}

#[test]
fn a_session_opened_full_has_the_others_on_a_bar() {
    let mut app = app_with_claude(b"one\r\n");
    app.update(key(KeyCode::Enter));
    let screen = render(&app, 96, 30);
    let bar = screen.lines().find(|l| l.contains(" 7 ✻ cobserve ")).unwrap_or_else(|| panic!("{screen}"));
    assert!(bar.contains(" + "), "{bar}");
    assert!(!screen.contains("╭ ✻ 7 · cobserve"), "no card: the pane edge to edge: {screen}");
}

#[test]
fn claude_not_installed_says_how_to_get_it() {
    let mut app = app_after(3);
    sessions(&mut app);
    app.claude.current_mut().unwrap().pane.state = crate::claude::PaneState::Failed(
        "claude is not installed here (not on PATH) — install Claude Code, then sign in once with /login and your Pro or Max account".into(),
    );
    let screen = render(&app, 120, 36);
    assert!(screen.contains("not installed"), "{screen}");
    assert!(screen.contains("no API key"), "it runs on the plan: {screen}");
    app.update(key(KeyCode::Enter));
    assert!(footer(&render(&app, 120, 36)).contains("⏎ start Claude"), "⏎ is offered once it is open");
}

#[test]
fn a_claude_that_ended_says_so_and_offers_to_start_again() {
    let mut app = app_with_claude(b"bye\r\n");
    let id = app.claude.current().unwrap().id;
    app.update(Event::PaneExited(id, "it exited".into()));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("bye"), "its last screen stays: {screen}");
    assert!(screen.contains("claude — it exited") && screen.contains("start it again"), "{screen}");
}

/// The Redash on the screenshot this view was rebuilt from: nothing waiting, one worker on
/// `queries` and that one idle, a scheduled refresh running — and RQ's started list holding
/// leftovers from months ago, which the old view listed as RUNNING for 177 days.
fn quiet_redash_with_leftovers() -> crate::model::QueueStatus {
    use crate::model::{Job, JobState, QueueRow, QueueStatus, Stale};
    let leftover = |id: &str, who: &str, query: u64, name: &str, source: &str, kind: &str, age_s: u64, why: Stale| {
        let mut job = Job::new(id, JobState::Stale(why), "queries");
        job.person = Some(who.to_string());
        job.redash_query_id = Some(query);
        job.query_name = Some(name.to_string());
        job.data_source = Some(source.to_string());
        job.data_source_type = Some(kind.to_string());
        job.age_s = age_s;
        job
    };
    let day = 86_400;
    let mut jobs = vec![
        leftover("z1", "grigol.gankava", 5120, "Transfers · weekly summary", "clickhouse-bi", "clickhouse", 177 * day + 36_000, Stale::OverADay),
        leftover("z2", "a.vaitkus", 6301, "Card margin · by day", "payments-mysql", "mysql", 173 * day + 61_200, Stale::Cancelled),
        leftover("z3", "j.petrova", 6302, "Ledger export · full", "clickhouse-bi", "clickhouse", 172 * day + 46_800, Stale::OverADay),
        leftover("o1", "d.zaleckas", 161, "Replica health check 2", "Query Results", "results", 772, Stale::NoWorker),
        leftover("o2", "d.zaleckas", 162, "Replica health check 1", "Query Results", "results", 742, Stale::NoWorker),
    ];
    let mut refresh = Job::new("s1", JobState::Started, "scheduled_queries");
    refresh.person = Some("r.simonyte".to_string());
    refresh.redash_query_id = Some(8091);
    refresh.query_name = Some("July close pack".to_string());
    refresh.data_source = Some("clickhouse7".to_string());
    refresh.data_source_type = Some("clickhouse".to_string());
    refresh.scheduled = true;
    refresh.age_s = 140;
    jobs.push(refresh);
    let row = |name: &str, running: u32, stale: u32, busy: u32, total: u32| QueueRow {
        name: name.to_string(),
        running,
        waiting: 0,
        oldest_wait_s: None,
        stale,
        workers_busy: busy,
        workers_total: total,
    };
    QueueStatus {
        reachable: true,
        error: None,
        queues: vec![
            row("default", 0, 0, 1, 2),
            row("emails", 0, 0, 1, 2),
            row("periodic", 0, 0, 1, 2),
            row("queries", 0, 5, 0, 1),
            row("scheduled_queries", 1, 0, 2, 6),
            row("schemas", 0, 0, 2, 6),
        ],
        jobs,
        names_available: false,
        workers_busy: 3,
        workers_total: 9,
        host: Some("redash.example.net".to_string()),
        version: Some("10.1.0".to_string()),
        taken_at: std::time::SystemTime::now(),
    }
}

#[test]
fn leftovers_in_the_started_list_are_not_shown_as_running() {
    let mut app = app_after(3);
    app.update(Event::Queue(Box::new(quiet_redash_with_leftovers())));
    zoom(&mut app, 2);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("REDASH QUEUE · redash.example.net · 10.1.0") && screen.contains("9 workers, 3 busy · esc back"), "{screen}");
    assert!(!screen.contains("oldest"), "no age without Redis, and no dash in its place: {screen}");
    assert!(screen.contains("RUNNING · 1 on a worker · scheduled_queries"), "{screen}");
    assert!(screen.contains("→ clickhouse7"), "the scheduled refresh is stitched: {screen}");
    assert!(screen.contains("STALE · 5 in RQ's started list that no worker runs · queries"), "{screen}");
    for why in ["over a day old", "cancelled", "no worker holds it"] {
        assert!(screen.contains(why), "{why}: {screen}");
    }
    assert!(!screen.contains("─ WAITING"), "nothing waits, so no list of nobody: {screen}");
    assert!(screen.contains("1 idle"), "the queries worker is idle, and says so: {screen}");
    let running = screen.find("─ RUNNING ·").unwrap();
    let stale = screen.find("─ STALE ·").unwrap();
    let months = screen.find("177d").expect("the oldest leftover is listed");
    assert!(stale < months && running < stale, "177 days is under STALE, not RUNNING: {screen}");
    // On the grid, the panel's frame says nothing waits; its big number is 0.
    app.update(key(KeyCode::Esc));
    let grid = render(&app, 140, 40);
    assert!(grid.contains("nothing waits") && grid.contains("1 running") && grid.contains("5 stale") && grid.contains("3/9 work"), "{grid}");
}

#[test]
fn an_unconfigured_redash_says_how_to_configure_it() {
    let mut app = App::new();
    app.update(Event::Queue(Box::new(crate::model::QueueStatus::unreachable(
        crate::model::QUEUE_NOT_CONFIGURED,
    ))));
    let screen = render(&app, 160, 36);
    assert!(screen.contains("not configured · REDASH_URL and REDASH_ADMIN_API_KEY"), "it says what turns it on: {screen}");
    assert!(!screen.contains("unreachable"), "not configured is not an outage: {screen}");
    zoom(&mut app, 2);
    let zoomed = render(&app, 160, 40);
    assert!(zoomed.contains("--credential file"), "it says where the settings go: {zoomed}");
}

#[test]
fn an_unreachable_queue_still_says_something() {
    let mut app = App::new();
    app.update(Event::Queue(Box::new(crate::model::QueueStatus::unreachable(
        crate::sources::redash::Error::Http(401).to_string(),
    ))));
    let grid = render(&app, 160, 36);
    assert!(grid.contains("unreachable · HTTP 401 · the API key has to be an admin"), "the panel says it: {grid}");
    zoom(&mut app, 2);
    let screen = render(&app, 160, 36);
    assert!(screen.contains("refused (HTTP 401 · the API key has to be an admin's)"), "{screen}");
}

#[test]
fn a_host_without_a_login_reads_not_polled() {
    let mut fake = FakeSource::new();
    let mut snapshot = fake.snapshot();
    snapshot
        .nodes
        .push(crate::model::NodeSnapshot::unreachable("ch-x", crate::model::NO_LOGIN));
    let mut app = App::new();
    app.update(Event::Snapshot(Box::new(snapshot)));
    let screen = render(&app, 160, 48);
    assert!(screen.contains("↯ not polled"), "the tile: {screen}");
    assert!(!screen.contains("unreachable"), "it was never asked, so it is not down: {screen}");
    zoom(&mut app, 3);
    let zoomed = render(&app, 160, 48);
    assert!(zoomed.contains("↯ not polled") && zoomed.contains("no login for this host"), "the zoomed tile says why: {zoomed}");
    assert_eq!(crate::model::NodeSnapshot::unreachable("ch-y", "HTTP 502").down_word(), "unreachable");
}

#[test]
fn the_tape_lists_what_happened() {
    let mut app = app_after(12);
    zoom(&mut app, 7);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("TAPE"), "{screen}");
    assert!(screen.contains("watching 8 nodes"), "{screen}");
    assert!(screen.contains("runaway"), "{screen}");
    assert!(screen.contains("joined the fleet"), "clickhouse5 appears after 20 s:\n{screen}");
}

#[test]
fn colours_carry_severity() {
    let app = app_after(3);
    let buf = buffer(&app, 120, 36);
    let theme = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark);
    // clickhouse3's tile is drawn in red: its frame and its bar.
    let width = buf.area.width as usize;
    let text = text_of(&buf);
    let row = text.lines().position(|l| l.contains("╭ clickhouse3 ")).expect("clickhouse3 tile");
    let bar = row + 1;
    let reds = (0..width).filter(|x| buf[(*x as u16, bar as u16)].fg == theme.crit).count();
    assert!(reds > 0, "a 90.9% node is drawn in red");
    // A quiet tile is not: colour means severity, nothing else.
    let quiet = text.lines().position(|l| l.contains("╭ ch9 ")).expect("ch9 tile");
    let col = text.lines().nth(quiet).unwrap().chars().position(|c| c == '╭').unwrap();
    assert_ne!(buf[(col as u16, quiet as u16)].fg, theme.crit, "{text}");
}

#[test]
fn a_paused_screen_says_so_and_a_stale_one_too() {
    let mut app = app_with_fake();
    app.update(key(KeyCode::Char(' ')));
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("○ paused"));
    app.update(key(KeyCode::Char(' ')));
    app.clock = std::time::SystemTime::now() + std::time::Duration::from_secs(30);
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("◌ stale"));
}

/// The fake fleet with FAKE=1's Airflow and Jira read at the screenshots' moment, on `view`.
fn app_on(view: char) -> App {
    let mut app = app_after(20);
    at_moment(&mut app, MOMENT);
    app.update(Event::Airflow(Box::new(crate::fake::airflow(MOMENT as i64))));
    app.update(Event::Jira(Box::new(crate::fake::jira(MOMENT as i64))));
    zoom(&mut app, view.to_digit(10).unwrap() as u8);
    app
}

#[test]
fn airflow_shows_what_runs_waits_and_failed_and_every_dag_s_day() {
    let app = app_on('5');
    assert_eq!(app.view, View::Airflow);
    let screen = render(&app, 120, 36);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    assert!(row("AIRFLOW · airflow.example.net · 2.10.2").contains("scheduler ●"), "{screen}");
    assert!(row("last 24 h").contains("✖ 3"), "three failures in the day: {screen}");
    let now = row("NOW ·");
    assert!(now.contains("2 running") && now.contains("1 stuck") && now.contains("1 queued") && now.contains("3 failed today"), "{now}");
    assert!(row("stuck · running").contains("262d"), "the stuck run says for how long: {screen}");
    assert!(row("running 2h12m").contains("4 of 7"), "a run says how far its tasks are: {screen}");
    assert!(row("failed at check_consistency").contains("failed at build_cohorts"), "the failures, card beside card: {screen}");
    assert_eq!(app.airflow_selection(), app.airflow.sections(app.now()).keys().first(), "the cursor starts on the first card");
    assert!(row("╭ ✖ test_clickhouse_connection").contains("╭ ▸ statement_daily_agg_reload"), "{screen}");
    // What the stuck run's task waits for, and its tasks under it once read.
    assert!(screen.contains("↻ ping_clickhouse waits for a ret"), "{screen}");
    assert!(row("╭ test_clickhouse_connection · manual").contains("running 262d"), "the run under the cursor: {screen}");
    assert!(footer(&screen).contains("⏎ task log") && footer(&screen).contains("o in Airflow"), "{screen}");

    // The day's timeline, under hours of the clock on screen.
    let mut app = app;
    for _ in 0..30 {
        app.update(key(KeyCode::Down));
    }
    let screen = render(&app, 120, 36);
    let header = screen.lines().find(|l| l.contains("┃ now")).unwrap_or_else(|| panic!("{screen}"));
    assert!(header.contains("18") && header.contains("00") && header.contains("06") && header.contains("12"), "{header}");
    // Its schedule as Airflow words it (in the DAG's zone), its failure on its timeline.
    let kyc = screen.lines().find(|l| l.contains("kyc_onboarding_tables") && l.contains("05:30")).unwrap_or_else(|| panic!("{screen}"));
    assert!(kyc.contains('✖') && kyc.contains("✖1"), "{kyc}");
    assert!(screen.contains("· ↑ 3 more"), "the threads above the window are counted: {screen}");
    let tall = render(&app, 120, 60);
    let fifteen = tall.lines().find(|l| l.contains("clickhouse_replication_check")).unwrap_or_else(|| panic!("{tall}"));
    assert!(fifteen.contains("every 15m") && fifteen.contains("▪▪▪▪"), "{fifteen}");

    for (w, h) in [(100, 30), (80, 24), (60, 16), (200, 50)] {
        let screen = render(&app, w, h);
        assert_eq!(screen.lines().count(), h as usize, "{w}×{h}");
    }
}

#[test]
fn jira_shows_your_tickets_in_the_board_s_columns() {
    let app = app_on('6');
    assert_eq!(app.view, View::Jira);
    let screen = render(&app, 120, 36);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    assert!(row("jira.example.net").contains("Sam Example"), "{screen}");
    // The lanes' titles: each column's count, and what is late in it.
    let lanes = row("IN PROGRESS 4");
    assert!(lanes.contains("IN REVIEW 4") && lanes.contains("FEEDBACK 1") && lanes.contains("DONE 8 in 7 days"), "{lanes}");
    assert!(lanes.contains("✖ 1 overdue") && lanes.contains("▲ 1 due soon"), "{lanes}");
    assert!(row("DATA-2647").contains("ASAP"), "{screen}");
    assert!(screen.contains("4d · overdue 2d · 7h12m"), "{screen}");
    let summary = screen.lines().find(|l| l.contains("Client account balances")).unwrap_or_else(|| panic!("{screen}"));
    assert!(!summary.contains("DE - "), "the team's tag is split off on the board: {summary}");
    assert!(screen.contains("12h · due tomorrow"), "{screen}");
    // The cursor on the first ticket, and the drawer on it.
    assert!(row("▌").contains("DATA-2207"), "High before no priority, as the board ranks: {screen}");
    assert!(screen.contains("https://jira.example.net/browse/DATA-2207"), "{screen}");

    for (w, h) in [(100, 30), (80, 24), (60, 16), (200, 50)] {
        let screen = render(&app, w, h);
        assert_eq!(screen.lines().count(), h as usize, "{w}×{h}");
    }
}

#[test]
fn a_ticket_or_a_run_opens_in_the_browser_with_o_and_r_reads_again() {
    let mut app = app_on('6');
    render(&app, 120, 36);
    app.update(key(KeyCode::Down));
    let key_now = app.jira_selection().map(str::to_string);
    app.update(key(KeyCode::Char('o')));
    assert_eq!(app.take_opens(), [format!("https://jira.example.net/browse/{}", key_now.clone().unwrap())]);
    app.update(key(KeyCode::Char('y')));
    assert_eq!(app.take_clipboard().len(), 1);
    app.update(key(KeyCode::Char('r')));
    assert_eq!(app.take_refreshes(), [crate::app::Feed::Jira]);
    assert!(render(&app, 120, 36).contains("reading…"), "until the answer comes");
    // The answer comes with the tickets in another order: the cursor stays on its ticket.
    let mut board = crate::fake::jira(MOMENT as i64 + 60);
    board.tickets.reverse();
    app.update(Event::Jira(Box::new(board)));
    assert_eq!(app.jira_selection().map(str::to_string), key_now);
    assert!(!render(&app, 120, 36).contains("reading…"));
    // A read that fails keeps the tickets, and says why they are not new.
    app.update(Event::Jira(Box::new(crate::jira::Board::unreachable("no answer within 15 s"))));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("DATA-2647") && screen.contains("no answer within 15 s"), "{screen}");

    // A click on a run puts the cursor there; a second opens its tasks here.
    let mut app = app_on('5');
    render(&app, 120, 36);
    click_on(&mut app, Hit::AirflowRow(2));
    assert_eq!(app.airflow_rows().get(2), app.airflow_selection());
    render(&app, 120, 36);
    click_on(&mut app, Hit::AirflowRow(2));
    assert!(matches!(app.page().map(|p| &p.ask), Some(crate::detail::Ask::Tasks { .. })), "{:?}", app.page());
    app.update(key(KeyCode::Char('o')));
    let opened = app.take_opens();
    assert!(opened.len() == 1 && opened[0].starts_with("https://airflow.example.net/dags/"), "{opened:?}");
}

/// The answers a page asked for, as FAKE=1 gives them.
fn answer_pages(app: &mut App) {
    for ask in app.take_detail_asks() {
        let answer = crate::fake::detail(&ask, MOMENT as i64);
        app.update(Event::Detail(ask, answer));
    }
}

#[test]
fn enter_on_a_ticket_shows_it_in_full_here() {
    let mut app = app_on('6');
    for _ in 0..4 {
        app.update(key(KeyCode::Down));
    }
    assert_eq!(app.jira_selection(), Some("DATA-2647"));
    app.update(key(KeyCode::Enter));
    assert!(render(&app, 120, 36).contains("reading…"), "until the ticket comes");
    answer_pages(&mut app);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("◂ DATA-2647 · Client account balances"), "{screen}");
    assert!(screen.contains("─ DESCRIPTION") && screen.contains("Asked for") && screen.contains("• one row per day"), "{screen}");
    assert!(!screen.contains("h3.") && !screen.contains("{code"), "the markup is gone: {screen}");
    assert!(screen.contains("SELECT toDate(created_at)"), "a code block's lines: {screen}");
    for _ in 0..30 {
        app.update(key(KeyCode::Down));
    }
    let screen = render(&app, 120, 36);
    assert!(screen.contains("─ COMMENTS · 2") && screen.contains("Jurgita Petrova"), "scrolled to the comments: {screen}");
    assert!(footer(&screen).contains("esc back"), "{screen}");
    app.update(key(KeyCode::Esc));
    assert!(app.page().is_none());
    assert!(render(&app, 120, 36).contains("IN REVIEW 4"), "back on the board");
}

#[test]
fn a_dag_s_runs_a_run_s_tasks_and_a_failed_task_s_log_open_one_over_the_other() {
    let mut app = app_on('5');
    // The DAG whose day has a failure, from its line in ACTIVITY.
    let at = app.airflow_rows().iter().position(|k| *k == crate::airflow::RowKey::Dag("kyc_onboarding_tables".into())).expect("its line");
    app.update(key(KeyCode::Home));
    for _ in 0..at {
        app.update(key(KeyCode::Down));
    }
    app.update(key(KeyCode::Enter));
    answer_pages(&mut app);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("◂ kyc_onboarding_tables › its runs"), "{screen}");
    assert!(screen.contains("✖ scheduled · 12:30") && screen.contains("failed"), "{screen}");
    // Its failed run: the run's tasks, the failed one red and marked.
    app.update(key(KeyCode::Enter));
    answer_pages(&mut app);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("✖ build_cohorts") && screen.contains("upstream failed"), "{screen}");
    let failed = match &app.page().unwrap().body {
        Some(Ok(crate::detail::Body::Tasks(tasks))) => tasks.iter().position(|t| t.state == "failed").unwrap(),
        other => panic!("{other:?}"),
    };
    for _ in 0..failed {
        app.update(key(KeyCode::Down));
    }
    assert!(render(&app, 120, 36).contains("⏎ its log"), "the drawer says where ⏎ goes");
    app.update(key(KeyCode::Enter));
    answer_pages(&mut app);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("build_cohorts · log of try 1"), "{screen}");
    assert!(screen.contains("AirflowException: 3 table(s) need a look"), "it opens at the end, where the failure is: {screen}");
    // esc, three times: back to the tasks, the runs, the view.
    app.update(key(KeyCode::Esc));
    assert!(matches!(app.page().map(|p| &p.ask), Some(crate::detail::Ask::Tasks { .. })));
    app.update(key(KeyCode::Esc));
    app.update(key(KeyCode::Esc));
    assert!(app.page().is_none());
    // A task that has not run has no log to open.
    for (w, h) in [(80, 24), (200, 50)] {
        assert_eq!(render(&app, w, h).lines().count(), h as usize);
    }
}

#[test]
fn jira_charts_the_hours_logged_this_month_day_by_day() {
    let app = app_on('6');
    let screen = render(&app, 120, 36);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    // FAKE=1 on Sunday the 4th: Thursday 6h30 + 2h, Friday 3h + 3h + 2h; nothing at the weekend.
    let head = row("LOGGED · October 2026");
    assert!(head.contains("16h30m in 2 working days") && head.contains("8h15m a working day") && head.contains("today 0h"), "{head}");
    let values = screen.lines().find(|l| l.contains("8½")).unwrap_or_else(|| panic!("{screen}"));
    assert!(values.contains("8½") && values.contains(" 8"), "the hours over each day: {values}");
    assert!(row(" 9h │").contains("▇▇"), "the scale in whole hours, 8½ of 9 under it: {screen}");
    assert!(screen.lines().any(|l| inner(l).trim_start().starts_with("│██ ██")), "the bars' lower cells full: {screen}");
    let days = row("  1  2  3  4  5");
    assert!(days.contains("31"), "every day of the month: {days}");
    let bars = screen.lines().filter(|l| inner(l).split('│').nth(1).and_then(|after| after.chars().next()).is_some_and(|c| "▁▂▃▄▅▆▇█".contains(c))).count();
    assert_eq!(bars, 2, "two rows of bars where the body is under 30 rows: {screen}");
}

#[test]
fn unconfigured_airflow_and_jira_say_how_to_set_them_up() {
    let mut app = app_after(3);
    app.update(Event::Airflow(Box::new(crate::airflow::Activity::unreachable(crate::airflow::NOT_CONFIGURED))));
    app.update(Event::Jira(Box::new(crate::jira::Board::unreachable(crate::jira::NOT_CONFIGURED))));
    zoom(&mut app, 5);
    assert!(render(&app, 120, 36).contains("AIRFLOW_URL, AIRFLOW_USER and AIRFLOW_PASSWORD"));
    zoom(&mut app, 6);
    assert!(render(&app, 120, 36).contains("JIRA_URL and JIRA_TOKEN"));
    app.update(Event::Jira(Box::new(crate::jira::Board::unreachable("HTTP 401 · the token was refused"))));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("Jira could not be read") && screen.contains("HTTP 401"), "{screen}");
}

/// The real Airflow and Jira, each read once and drawn, with `AIRFLOW_URL`, `AIRFLOW_USER`,
/// `AIRFLOW_PASSWORD`, `JIRA_URL` and `JIRA_TOKEN` set:
/// `cargo test live_airflow_and_jira -- --ignored --nocapture`. Ignored by default: it goes on
/// the network, and reads only.
#[tokio::test]
#[ignore]
async fn live_airflow_and_jira() {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let mut app = app_after(3);
    let airflow = crate::config::AirflowConfig { url: env("AIRFLOW_URL"), user: env("AIRFLOW_USER"), password: env("AIRFLOW_PASSWORD") };
    if let Some(mut source) = crate::sources::airflow::AirflowSource::new(&airflow) {
        let started = std::time::Instant::now();
        let first = source.now_only().await;
        println!(
            "airflow, what runs now: {:.1} s · reachable {} · {} live runs · day read {} · error {:?}",
            started.elapsed().as_secs_f64(),
            first.reachable,
            first.runs.len(),
            first.day_read,
            first.error
        );
        for round in 1..=2 {
            let started = std::time::Instant::now();
            let activity = source.activity(false).await;
            println!(
                "airflow read {round}: {:.1} s · reachable {} · {} runs · {} dags · {} live tasks · error {:?}",
                started.elapsed().as_secs_f64(),
                activity.reachable,
                activity.runs.len(),
                activity.dags.len(),
                activity.tasks.len(),
                activity.error
            );
            app.update(Event::Airflow(Box::new(activity)));
        }
    }
    let jira = crate::config::JiraConfig {
        url: env("JIRA_URL"),
        token: env("JIRA_TOKEN"),
        statuses: crate::jira::DEFAULT_STATUSES.iter().map(|s| s.to_string()).collect(),
        done_days: crate::jira::DEFAULT_DONE_DAYS,
    };
    if let Some(mut source) = crate::sources::jira::JiraSource::new(&jira) {
        for round in 1..=2 {
            let started = std::time::Instant::now();
            let mut board = source.board().await;
            let since = board.tickets.iter().filter(|t| t.status_since.is_some()).count();
            if source.time_due() {
                let started = std::time::Instant::now();
                let read = source.time(&mut board).await;
                println!("jira time of the month: {read} in {:.1} s", started.elapsed().as_secs_f64());
            }
            println!(
                "jira read {round}: {:.1} s · reachable {} · {} tickets ({since} with their status's date) · error {:?}",
                started.elapsed().as_secs_f64(),
                board.reachable,
                board.tickets.len(),
                board.error
            );
            app.update(Event::Jira(Box::new(board)));
        }
    }
    app.tick_at(std::time::SystemTime::now());
    zoom(&mut app, 5);
    println!("{}", render(&app, 160, 48));
    zoom(&mut app, 6);
    println!("{}", render(&app, 160, 48));
    println!(
        "worklogs this month: {} · with a summary: {} · tickets: {}",
        app.jira.worklogs.len(),
        app.jira.worklogs.iter().filter(|w| !w.summary.is_empty()).count(),
        app.time_by_ticket().1.len()
    );
    app.update(key(KeyCode::Char('t')));
    println!("{}", render(&app, 160, 48));
    app.update(key(KeyCode::Esc));

    // The pages: a ticket in full; the runs of a DAG that failed, its failed run's tasks, a log.
    if let Some(source) = crate::sources::jira::JiraSource::new(&jira)
        && let Some(key) = app.jira.rows().first().map(|t| t.key.clone())
    {
        let started = std::time::Instant::now();
        let issue = source.details().issue(&key).await;
        println!("jira page {key}: {:.1} s · {:?}", started.elapsed().as_secs_f64(), issue.as_ref().map(|i| (i.description.len(), i.comments.len(), i.subtasks.len(), i.links.len())));
        if let Ok(issue) = issue {
            let answer = key_enter_page(&mut app, crate::detail::Ask::Issue(key), crate::detail::Body::Issue(Box::new(issue)));
            app.update(answer);
            println!("{}", render(&app, 160, 48));
        }
    }
    if let Some(mut source) = crate::sources::airflow::AirflowSource::new(&airflow) {
        let activity = source.now_only().await;
        let details = source.details();
        let now = app.now();
        let failed = app.airflow.sections(now).failed.first().map(|r| (r.dag.clone(), r.id.clone()));
        let _ = activity;
        if let Some((dag, run)) = failed {
            let runs = details.runs(&dag).await;
            println!("airflow page runs of {dag}: {:?}", runs.as_ref().map(Vec::len));
            let tasks = details.tasks(&dag, &run).await;
            println!("airflow page tasks of {run}: {:?}", tasks.as_ref().map(|t| t.iter().map(|t| (t.id.clone(), t.state.clone(), t.try_number)).collect::<Vec<_>>()));
            if let Some(task) = tasks.ok().and_then(|t| t.into_iter().find(|t| t.state == "failed")) {
                let started = std::time::Instant::now();
                let log = details.log(&dag, &run, &task.id, task.map_index, task.try_number).await;
                let lines = log.as_ref().map(|t| crate::detail::log_lines(t));
                println!("airflow page log of {}: {:.1} s · {:?} lines", task.id, started.elapsed().as_secs_f64(), lines.as_ref().map(Vec::len));
                if let Ok(lines) = lines {
                    for line in lines.iter().rev().take(6).rev() {
                        println!("  | {}", fmt::truncate(line, 150));
                    }
                }
            }
        }
    }
}

/// A page put on screen with its answer, as the loop would.
fn key_enter_page(app: &mut App, ask: crate::detail::Ask, body: crate::detail::Body) -> Event {
    app.update(key(KeyCode::Enter));
    app.take_detail_asks();
    if let Some(page) = app.jira_page.as_mut() {
        page.ask = ask.clone();
    }
    Event::Detail(ask, Ok(body))
}

/// The fake fleet on view 6 later in the month — the 22nd, a Thursday — so the month's time has
/// three weeks in it.
fn app_late_in_the_month() -> App {
    let at = MOMENT + 18 * 86_400;
    let mut app = app_after(5);
    at_moment(&mut app, at);
    app.update(Event::Jira(Box::new(crate::fake::jira(at as i64))));
    zoom(&mut app, 6);
    app.numbers_fresh_at_the_clock();
    app
}

#[test]
fn t_shows_which_tickets_took_the_time_day_by_day() {
    let mut app = app_late_in_the_month();
    app.update(key(KeyCode::Char('t')));
    assert!(app.jira_time.is_some());
    let screen = render(&app, 120, 36);
    let row = |screen: &str, text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    assert!(row(&screen, "◂ Time logged · October 2026").contains("working days"), "{screen}");
    assert!(screen.contains("─ BY TICKET · 6 tickets"), "{screen}");
    assert!(screen.contains("─ THU 22 OCT"), "today chosen: {screen}");
    // The day columns line up: a ticket's cell for the 1st under the chart's 1.
    let days = row(&screen, "  1  2  3  4  5");
    let first = row(&screen, "DATA-2647");
    let at = days.find(" 1 ").unwrap() + 1;
    assert!(first.chars().nth(at).is_some_and(|c| c != ' '), "the 1st's cell under its day:\n{days}\n{first}");

    // ← to the 21st, a Wednesday: its tickets and their share of the day.
    app.update(key(KeyCode::Left));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("─ WED 21 OCT · 8h00m on 2 tickets") || screen.contains("─ WED 21 OCT ·"), "{screen}");
    assert!(screen.contains('%'), "each ticket's share: {screen}");
    // To a Sunday: nothing, said why.
    for _ in 0..3 {
        app.update(key(KeyCode::Left));
    }
    assert!(render(&app, 120, 36).contains("a weekend day — nothing logged"));

    // ⏎ on a ticket opens it in full over the page; esc comes back to the page, esc again out.
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Enter));
    assert!(matches!(app.page().map(|p| &p.ask), Some(crate::detail::Ask::Issue(_))));
    app.update(key(KeyCode::Esc));
    assert!(app.page().is_none() && app.jira_time.is_some());
    assert!(render(&app, 120, 36).contains("◂ Time logged · October 2026"), "back on the month's time");
    app.update(key(KeyCode::Esc));
    assert!(app.jira_time.is_none());
    for (w, h) in [(80, 24), (100, 30), (200, 50)] {
        let mut app = app_late_in_the_month();
        app.update(key(KeyCode::Char('t')));
        assert_eq!(render(&app, w, h).lines().count(), h as usize, "{w}×{h}");
    }
}

// ---------------------------------------------------------------------------
// Screenshots: `SHOT_DIR=docs/screenshots cargo test export_screens -- --ignored`
// writes every view as plain text and as colour HTML (what dev/screenshots.sh turns into
// PNGs). Ignored by default: it writes files.
// ---------------------------------------------------------------------------

/// 15:52:07 WIB on Sunday 4 October 2026.
#[test]
fn a_click_puts_the_cursor_on_a_row_and_a_second_opens_it() {
    use crate::app::Hit;
    let mut app = app_after(5);
    // A job of the zoomed queue: a click selects it, a second opens it (its node, zoomed).
    zoom(&mut app, 2);
    render(&app, 120, 36);
    click_on(&mut app, Hit::Job(1));
    assert_eq!(app.queue_selection(), Some(1));
    render(&app, 120, 36);
    click_on(&mut app, Hit::Job(0));
    render(&app, 120, 36);
    click_on(&mut app, Hit::Job(0));
    assert_eq!(app.shell.zoom, Some(crate::ui2::PanelId::Node), "a second click opens the job's node");
    // A line of the tape, the same way.
    zoom(&mut app, 7);
    render(&app, 120, 36);
    if app.tape.len() > 1 {
        click_on(&mut app, Hit::TapeLine(1));
        assert_eq!(app.tape_selection(), 1);
    }
}

#[test]
fn five_sessions_open_still_leave_room_for_a_new_one() {
    use crate::claude::Kind;
    let mut app = app_after(5);
    sessions(&mut app);
    for i in 1..5 {
        app.claude.open_new(&format!("~/work/project{i}"), Kind::Claude);
    }
    assert_eq!(app.claude.list.len(), 5);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("+ new session") && screen.contains("past conversations"), "a free cell offers one: {screen}");
    assert!(!screen.contains("of 5"), "not full at five: {screen}");
    // A click on it, or ctrl+\ n, asks where the sixth should work.
    click_on(&mut app, crate::app::Hit::NewSession);
    assert!(matches!(app.claude.mode, crate::claude::Mode::Opening(_)), "{:?}", app.claude.mode);
}

#[test]
fn fifty_sessions_fit_the_list_which_follows_the_one_on_screen_and_finds_one_by_name() {
    use crate::claude::{Kind, MAX_SESSIONS};
    let mut app = app_after(5);
    sessions(&mut app);
    for i in 1..MAX_SESSIONS {
        let kind = [Kind::Claude, Kind::OpenCode, Kind::Terminal][i % 3];
        app.claude.open_new(&format!("~/work/project{i:02}"), kind);
    }
    assert_eq!(app.claude.list.len(), 50);
    let screen = render(&app, 140, 40);
    let row = |screen: &str, text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    let lit = |screen: &str, text: &str| screen.lines().any(|l| l.contains(text) && l.contains('▌'));
    assert!(screen.contains("50 sessions"), "{screen}");
    // A line each beside the chosen one's screen, the chosen — the last — lit, and how many are
    // above it.
    assert!(lit(&screen, "project49"), "{screen}");
    assert!(screen.contains("╭ ▣ 56 · project49"), "the preview: {screen}");
    assert!(row(&screen, "↑ ").contains(" more"), "{screen}");
    assert!(!screen.contains("project01"), "the first are above, out of sight: {screen}");
    assert!(screen.contains("50 of 50 · x x closes one"), "{screen}");

    // ↑ walks back through them on the zoomed list; the list follows.
    for _ in 0..30 {
        app.update(key(KeyCode::Up));
    }
    assert_eq!(app.claude.active, 19);
    let screen = render(&app, 140, 40);
    assert!(lit(&screen, "project19") && row(&screen, "↑ ").contains(" more"), "{screen}");

    // / finds one by what it is called; ⏎ puts it on screen.
    app.update(key(KeyCode::Char('/')));
    for c in "ject07".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    let screen = render(&app, 140, 40);
    assert!(row(&screen, "⌕ /").contains("/ject07▏") && row(&screen, "⌕ /").contains("1 of 1"), "{screen}");
    assert!(lit(&screen, "project07"), "{screen}");
    app.update(key(KeyCode::Enter));
    assert_eq!((app.claude.active, &app.claude.mode), (7, &crate::claude::Mode::Typing));
    // By its number too.
    prefix(&mut app, '/');
    app.update(key(KeyCode::Char('4')));
    app.update(key(KeyCode::Char('0')));
    app.update(key(KeyCode::Enter));
    assert_eq!(crate::claude::Sessions::number_of(app.claude.active), 40);

    // Fifty is as many as there can be.
    prefix(&mut app, 'n');
    assert!(app.notice().is_some_and(|n| n.contains("50 sessions is as many as there can be")), "{:?}", app.notice());

    // Opened full on a narrow terminal, the tabs round the one on screen, and how many either
    // side.
    app.shell.sessions_full = true;
    app.shell.sessions_spread = true;
    let narrow = render(&app, 90, 30);
    let bar = row(&narrow, " 40 ");
    assert!(bar.contains("‹ ") && bar.contains(" ›"), "{narrow}");
}

/// A query session on clickhouse3 of the fake fleet, on screen, its tables known.
fn app_with_query() -> App {
    let mut app = app_after(118);
    app.claude.open_query(Some("clickhouse3"));
    sessions(&mut app);
    // Opened full: every key is the query's.
    app.update(key(KeyCode::Enter));
    app.update(Event::Schema("clickhouse3".into(), Ok(crate::fake::schema())));
    app
}

fn type_into(app: &mut App, text: &str) {
    for c in text.chars() {
        match c {
            '\n' => app.update(Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL))),
            c => app.update(key(KeyCode::Char(c))),
        }
    }
}

/// What the session asked of `main.rs`, done the way `main.rs` does it under `FAKE=1`.
fn answer_console(app: &mut App) {
    let snapshot = app.snapshot().cloned();
    let session = app.claude.current().map(|s| s.id).expect("a session");
    let console = app.claude.current_mut().and_then(|s| s.console.as_deref_mut()).expect("a query session");
    if let Some(ask) = console.take_ask() {
        let answer = crate::fake::assist(&ask);
        app.update(Event::Assisted(session, ask.id, answer));
        return;
    }
    let request = console.take_request().expect("a query");
    let mut answer = crate::fake::console_answer(snapshot.as_ref(), &request.node, &request.sql);
    if let Ok(answer) = answer.as_mut() {
        answer.elapsed_ms = 184;
    }
    app.update(Event::ConsoleAnswer(session, request.id, answer));
}

#[test]
fn a_query_session_suggests_as_it_is_typed_and_shows_its_answer_as_a_table() {
    let mut app = app_with_query();
    let screen = render(&app, 140, 40);
    assert!(screen.contains("▦ Query  on   ● clickhouse3 ▾") && screen.contains("✻ Claude ⇄"), "{screen}");
    assert!(screen.contains("read-only · 30 s · 1000 rows · 9 tables"), "{screen}");
    assert!(screen.contains("ctrl+k: tell Claude what it should show — it writes the SQL"), "{screen}");

    type_into(&mut app, "SELECT user, count() AS queries\nFROM system.proc");
    let screen = render(&app, 140, 40);
    assert!(screen.contains("▌▦ processes"), "the table under the word: {screen}");
    app.update(key(KeyCode::Tab));
    type_into(&mut app, "\nGROUP BY user;");
    let sql = app.claude.current().and_then(|s| s.console.as_ref()).unwrap().sql();
    assert_eq!(sql, "SELECT user, count() AS queries\nFROM system.processes\nGROUP BY user;");
    app.update(key(KeyCode::Enter));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("running on clickhouse3"), "{screen}");
    answer_console(&mut app);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("✔ ") && screen.contains(" rows · 184 ms") && screen.contains("on clickhouse3"), "{screen}");
    assert!(screen.contains("user") && screen.contains("queries") && screen.contains("r_redash"), "{screen}");
    // The bar over it says what it is on.
    assert!(screen.contains("▦ clickhouse3"), "{screen}");

    // A write is refused by the server, and the helper is offered to put it right.
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    type_into(&mut app, "DROP TABLE wallet.ledger;");
    app.update(key(KeyCode::Enter));
    answer_console(&mut app);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("✖ clickhouse3 · Code 164 · monitor: Cannot execute query in readonly mode."), "{screen}");
    assert!(screen.contains("✻ ctrl+k: Claude puts it right, told what the server said"), "{screen}");
}

#[test]
fn ctrl_k_has_the_helper_write_the_query_the_comment_asks_for() {
    let mut app = app_with_query();
    type_into(&mut app, "-- the ten users using the most memory right now");
    // A click on "ctrl+k asks Claude" opens the line to say what, as the key does; ⏎ with
    // nothing typed asks for what the comment says.
    render(&app, 140, 40);
    click_on(&mut app, crate::app::Hit::ConsoleAsk);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("ask Claude ▸ what it should do — or just ⏎: what its -- comments ask"), "{screen}");
    app.update(key(KeyCode::Enter));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("Claude is writing it") && screen.contains("ctrl+c stops"), "{screen}");
    answer_console(&mut app);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("✻ Claude wrote this — read it first"), "{screen}");
    assert!(screen.contains("FROM system.processes"), "{screen}");
    let console = app.claude.current().and_then(|s| s.console.as_ref()).unwrap();
    assert!(console.take_request_peek().is_none(), "nothing runs until asked");
    // ⏎ at the end runs what it wrote; the helper's chip switches to OpenCode with a click.
    app.update(key(KeyCode::Enter));
    answer_console(&mut app);
    assert!(render(&app, 140, 40).contains("✔ "));
    click_on(&mut app, crate::app::Hit::ConsoleAssistant);
    assert!(render(&app, 140, 40).contains("▣ OpenCode ⇄"));
    assert_eq!(app.claude.assistant, crate::console::Assistant::OpenCode, "and new sessions ask it too");
}

#[test]
fn a_drag_over_the_text_selects_it_and_ctrl_k_asks_about_that_part_alone() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app_with_query();
    app.update(Event::Paste("SELECT user\nFROM system.processes\nWHERE elapsed > 10;".into()));
    render(&app, 140, 40);
    let text = app.viewport.console_text.get().expect("the text on screen");
    let mouse = |kind, column, row| Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
    // From the W of WHERE to just before its ;.
    app.update(mouse(MouseEventKind::Down(MouseButton::Left), text.text_x, text.top + 2));
    app.update(mouse(MouseEventKind::Drag(MouseButton::Left), text.text_x + 9, text.top + 2));
    app.update(mouse(MouseEventKind::Drag(MouseButton::Left), text.text_x + 18, text.top + 2));
    app.update(mouse(MouseEventKind::Up(MouseButton::Left), text.text_x + 18, text.top + 2));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("18 characters selected · ⌫ deletes it · ctrl+k asks Claude about it · ctrl+r runs it"), "{screen}");

    // ctrl+k: the line to say what, about the selection.
    app.update(ctrl('k'));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("ask Claude about the selection ▸ make it faster, explain it, fix it…"), "{screen}");
    type_into(&mut app, "over a minute");
    app.update(key(KeyCode::Enter));
    assert!(render(&app, 140, 40).contains("Claude is writing it"));
    answer_console(&mut app);
    // What came back took that part's place alone — marked, to be seen — and ctrl+z undoes it.
    let screen = render(&app, 140, 40);
    assert!(screen.contains("✻ Claude rewrote the part marked — read it first: ctrl+r runs all of it"), "{screen}");
    let console = app.claude.current().and_then(|s| s.console.as_ref()).unwrap();
    assert_eq!(console.sql(), "SELECT user\nFROM system.processes\n/* FAKE=1: as it was */ WHERE elapsed > 10;");
    assert!(console.selection().is_none());
    app.update(ctrl('z'));
    let console = app.claude.current().and_then(|s| s.console.as_ref()).unwrap();
    assert_eq!(console.sql(), "SELECT user\nFROM system.processes\nWHERE elapsed > 10;");

    // ctrl+a, then ⌫: all of it gone at once.
    app.update(ctrl('a'));
    app.update(key(KeyCode::Backspace));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("ctrl+k: tell Claude what it should show"), "empty again: {screen}");
}

#[test]
fn a_drag_past_the_bottom_of_a_long_text_scrolls_it_along() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app_with_query();
    let long: Vec<String> = (1..=30).map(|n| format!("-- line {n}")).collect();
    app.update(Event::Paste(long.join("\n")));
    // To the top: the text from its first line.
    app.update(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)));
    for _ in 0..40 {
        app.update(key(KeyCode::Up));
    }
    render(&app, 140, 40);
    let text = app.viewport.console_text.get().expect("the text on screen");
    assert_eq!(text.first, 0);
    let mouse = |kind, row| Event::Mouse(MouseEvent { kind, column: text.text_x, row, modifiers: KeyModifiers::NONE });
    // A click on the field's last line leaves the text where it is.
    app.update(mouse(MouseEventKind::Down(MouseButton::Left), text.top + text.height - 1));
    render(&app, 140, 40);
    assert_eq!(app.viewport.console_text.get().unwrap().first, 0, "what was under the mouse stays put");
    // Past the bottom, a line further each move.
    for _ in 0..3 {
        app.update(mouse(MouseEventKind::Drag(MouseButton::Left), text.top + text.height + 1));
        render(&app, 140, 40);
    }
    app.update(mouse(MouseEventKind::Up(MouseButton::Left), text.top + text.height + 1));
    assert_eq!(app.viewport.console_text.get().unwrap().first, 3);
    let console = app.claude.current().and_then(|s| s.console.as_ref()).unwrap();
    let last = text.height as usize - 1;
    assert_eq!(console.selection(), Some(((last, 0), (last + 3, 0))), "from the line clicked to three further");
}

#[test]
fn a_query_session_is_opened_on_a_server_chosen_from_the_fleet() {
    let mut app = app_after(5);
    sessions(&mut app);
    prefix(&mut app, 'q');
    let screen = render(&app, 140, 40);
    assert!(screen.contains("New query session") && screen.contains("which server should it run on?"), "{screen}");
    assert!(screen.contains("●  clickhouse3") && screen.contains("search the servers"), "{screen}");
    assert!(footer(&screen).contains("⏎ open it on that server"), "{screen}");
    type_into(&mut app, "bi");
    let screen = render(&app, 140, 40);
    assert!(screen.contains("1 server") && screen.contains("clickhouse-bi"), "{screen}");
    app.update(key(KeyCode::Enter));
    let session = app.claude.current().unwrap();
    assert_eq!(session.console.as_ref().and_then(|c| c.node.as_deref()), Some("clickhouse-bi"));
    assert!(render(&app, 140, 40).contains("● clickhouse-bi ▾"));
}

const MOMENT: u64 = 1_791_103_927;

/// The clock at `at` in Jakarta, with the prayer times for it.
fn at_moment(app: &mut App, at: u64) {
    app.time.zone = Some("Asia/Jakarta".into());
    app.time.pinned = chrono::FixedOffset::east_opt(7 * 3600);
    if app.prayers.place.is_none() {
        app.prayers = crate::prayer::Prayers::new(crate::prayer::city("Jakarta"), 10);
    }
    app.tick_at(std::time::UNIX_EPOCH + std::time::Duration::from_secs(at));
}

fn app_with_local() -> App {
    let mut app = app_after(20);
    let at = crate::history::secs(app.clock);
    for step in 1..=6 {
        app.update(Event::Local(Box::new(crate::fake::local(step, at - 12.0 + 2.0 * step as f64))));
    }
    app
}

#[test]
fn this_machine_has_a_status_line_and_a_zoom() {
    let mut app = app_with_local();
    // From 140 columns, this machine joins the status bar's right end.
    let line = footer(&render(&app, 140, 40));
    assert!(line.contains("this Mac · cpu ") && line.contains(" · mem 27/32 GiB"), "{line}");
    assert!(!footer(&render(&app, 139, 40)).contains("this Mac"), "not under 140 columns");

    zoom(&mut app, 10);
    assert_eq!(app.view, View::Local);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("sam-macbook.local · 10 cores · 32.0 GiB · up 31d02h · load 3.21 3.05 2.98"), "{screen}");
    assert!(screen.contains("PRESSURE  ● normal · the kernel counts 50% of memory free"), "{screen}");
    assert!(screen.contains("PROCESSES · 18 · by CPU"), "{screen}");
    let rows: Vec<&str> = screen.lines().skip_while(|l| !l.contains("NAME")).skip(1).collect();
    assert!(inner(rows[0]).starts_with("▌ WindowServer") && rows[0].contains("164"), "the busiest first, the cursor on it: {}", rows[0]);
    let rest = rows.iter().find(|l| l.contains("the rest")).expect("the closing row");
    assert!(rest.contains("kernel, short-lived"), "{rest}");
    assert!(screen.contains("of 10 cores (") && screen.contains("resident of 32.0 GiB"), "the drawer: {screen}");

    // Programs: Chrome's processes in one row; by memory, it is first.
    app.update(key(KeyCode::Char('g')));
    app.update(key(KeyCode::Char('s')));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("PROGRAMS · 9 · by memory"), "{screen}");
    let rows: Vec<&str> = screen.lines().skip_while(|l| !l.contains("NAME")).skip(1).collect();
    assert!(rows[0].contains("Google Chrome ×8"), "{}", rows[0]);

    // ⏎ on Chrome: its parts, its tab and extension processes, the cursor on the heaviest.
    app.update(key(KeyCode::Enter));
    let screen = render(&app, 120, 36);
    assert!(screen.lines().nth(1).is_some_and(|l| l.starts_with("╭ 0 Google Chrome · sam-macbook.local") && l.contains("8 processes · 3.7 GiB RSS · 11.5% of 32 GiB")), "{screen}");
    assert!(screen.contains("dashboard › this machine › Google Chrome"), "{screen}");
    assert!(screen.contains("╭ parts") && screen.contains("the rows add up to Chrome") && screen.contains("─ TAB AND EXTENSION PROCESSES · 5 · heaviest first"), "{screen}");
    assert!(screen.contains("◌ asking Chrome…"), "before the tabs are read: {screen}");
    assert!(screen.lines().any(|l| inner(l).starts_with("▌") && l.contains("15564") && l.contains("834.0 MiB")), "{screen}");
    // The tabs are due (the drill is open) once, not again while one is read.
    let (program, running) = app.wants_tabs(app.now() as f64).expect("asked for when the drill opens");
    assert!(program == "Google Chrome" && running && app.wants_tabs(app.now() as f64).is_none());
    app.update(Event::Tabs(Box::new(crate::browser::TabsRead::new(program, app.now() as f64 - 12.0, Ok(crate::fake::tabs())))));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("12 tabs · 2 windows · 6 sites") && screen.contains("─ TABS · 12 in 2 windows · by site"), "{screen}");
    assert!(screen.lines().any(|l| l.contains("4   docs.google.com") && l.contains("Q4 metrics plan")), "{screen}");
    assert!(app.wants_tabs(app.now() as f64).is_none(), "fresh for 30 s");
    // x on a process asks, anything but y keeps it, y sends it.
    app.update(key(KeyCode::Char('x')));
    assert!(render(&app, 120, 36).contains("Stop Chrome tab process 15564 · 834.0 MiB?"));
    app.update(key(KeyCode::Char('n')));
    assert!(app.stop_asked().is_none() && app.take_stops().is_empty(), "kept");
    app.update(key(KeyCode::Char('x')));
    app.update(key(KeyCode::Char('y')));
    let stops = app.take_stops();
    assert_eq!(stops.len(), 1);
    assert_eq!((stops[0].helper.pid, stops[0].asked), (15564, crate::app::Asked::Screen));
    // ⏎ on a site opens its tabs; esc closes the drill, esc again the machine.
    let site = app.drill.as_ref().unwrap().processes();
    app.drill.as_mut().unwrap().select(site);
    app.update(key(KeyCode::Enter));
    assert!(render(&app, 120, 36).contains("w1  Q4 metrics plan"));
    for (w, h) in [(40, 12), (60, 20), (80, 24), (200, 60), (10, 4)] {
        render(&app, w, h);
    }
    app.update(key(KeyCode::Esc));
    assert!(app.drill.is_none() && render(&app, 120, 36).contains("╭ 0 LOCAL ·"));
    for (w, h) in [(40, 12), (60, 20), (80, 24), (200, 60), (10, 4)] {
        render(&app, w, h);
    }
}

#[test]
fn wide_jira_is_a_board_of_lanes_and_the_arrows_cross_them() {
    let mut app = app_on('6');
    let screen = render(&app, 172, 48);
    let titles = screen.lines().find(|l| l.contains("IN PROGRESS 4")).unwrap_or_else(|| panic!("{screen}"));
    assert!(titles.contains("IN REVIEW 4 ✖ 1 overdue") && titles.contains("FEEDBACK 1") && titles.contains("DONE 8 in 7 days"), "{titles}");
    // The first ticket's block: its key and priority, its summary in two lines, its age.
    let first = screen.lines().find(|l| l.contains("▌DATA-2207")).unwrap_or_else(|| panic!("{screen}"));
    assert!(first.contains("High") && first.contains("DATA-2647") && first.contains("ASAP"), "lane beside lane: {first}");
    assert!(screen.contains("4d · overdue 2d · 7h12m logged") && screen.contains("12h · due tomorrow"), "{screen}");
    assert!(screen.contains("✔ 2h"), "a finished one says when: {screen}");
    assert!(footer(&screen).contains("←→ a lane"), "{screen}");

    // → to the next lane at the same height, ↓ down it, ← back.
    app.update(key(KeyCode::Right));
    assert_eq!(app.jira_selection(), Some("DATA-2647"));
    app.update(key(KeyCode::Down));
    assert_eq!(app.jira_selection(), Some("DATA-1817"));
    app.update(key(KeyCode::Right));
    assert_eq!(app.jira_selection(), Some("DATA-1289"), "feedback has one: the cursor takes it");
    app.update(key(KeyCode::Left));
    app.update(key(KeyCode::Left));
    assert_eq!(app.jira_selection(), Some("DATA-2207"));
    // A click on a block chooses it; ⏎ still opens it.
    let screen = render(&app, 172, 48);
    assert!(screen.contains("▌DATA-2207"), "{screen}");
    let index = app.jira.rows().iter().position(|t| t.key == "DATA-2737").unwrap();
    click_on(&mut app, Hit::Ticket(index));
    assert_eq!(app.jira_selection(), Some("DATA-2737"));
    // Narrower, still the lanes: ← → cross them at every width.
    let narrow = render(&app, 120, 36);
    assert!(narrow.contains("IN REVIEW 4"), "{narrow}");
    for (w, h) in [(140, 30), (172, 20), (250, 60)] {
        assert_eq!(render(&app, w, h).lines().count(), h as usize);
    }
}

#[test]
fn wide_airflow_tells_its_knots_and_weaves_the_day() {
    let mut app = app_on('5');
    let screen = render(&app, 172, 48);
    let now = screen.lines().find(|l| l.contains("NOW ·")).unwrap_or_else(|| panic!("{screen}"));
    assert!(now.contains("2 running · 1 stuck · 1 queued · 3 failed today"), "{now}");
    // Four cards a row: the stuck run, told — how long, what its task waits for — beside the
    // running one and the queued one, and the first failure.
    let titles = screen.lines().find(|l| l.contains("╭ ✖ test_clickhouse_connection")).unwrap_or_else(|| panic!("{screen}"));
    assert!(titles.contains("statement_daily_agg_reload") && titles.contains("gateway_transfers_backfill") && titles.contains("replication_app_consistency"), "{titles}");
    let first = screen.lines().find(|l| l.contains("stuck · running 262d")).unwrap_or_else(|| panic!("{screen}"));
    assert!(first.contains("4 of 7") && first.contains("failed at check_consistency"), "{first}");
    assert!(screen.contains("↻ ping_clickhouse waits for a retry"), "{screen}");
    // The day: hours over the threads, each thread's rhythm after it.
    let hours = screen.lines().find(|l| l.contains("┃ now") && l.contains("00") && l.contains("12")).unwrap_or_else(|| panic!("{screen}"));
    assert!(!hours.contains("SCHEDULE"), "no table header: {hours}");
    let fifteen = screen.lines().find(|l| l.contains("clickhouse_replication_check")).unwrap_or_else(|| panic!("{screen}"));
    assert!(fifteen.contains("▪▪▪▪▪▪") && fifteen.contains("every 15m"), "{fifteen}");
    let cbk = screen.lines().find(|l| l.contains("cbk_accounts_report") && l.contains("every 2h")).unwrap_or_else(|| panic!("{screen}"));
    assert!(cbk.contains('✖') && cbk.contains("✖1"), "{cbk}");

    // The cursor walks the same rows as the narrow view, and a click on a knot opens its tasks.
    for _ in 0..3 {
        app.update(key(KeyCode::Down));
    }
    let screen = render(&app, 172, 48);
    assert_eq!(app.airflow_selection(), app.airflow.sections(app.now()).keys().get(3), "{screen}");
    assert!(screen.contains("╭ replication_app_consistency_check · scheduled"), "the Gantt follows the cursor: {screen}");
    click_on(&mut app, Hit::AirflowRow(3));
    assert!(matches!(app.page().map(|p| &p.ask), Some(crate::detail::Ask::Tasks { .. })), "{:?}", app.page());
    app.update(key(KeyCode::Esc));
    for (w, h) in [(140, 30), (172, 20), (250, 60)] {
        assert_eq!(render(&app, w, h).lines().count(), h as usize);
    }
}

#[test]
fn many_sessions_sit_by_project_a_line_each_with_the_way_to_find_one() {
    use crate::claude::Kind;
    let mut app = app_after(5);
    sessions(&mut app);
    let sessions = [
        ("~/work/airflow-dags", Kind::OpenCode, "airflow-agent"),
        ("~/work/airflow-dags", Kind::Claude, "MetricsAtomic"),
        ("~/code/apitap-lib", Kind::OpenCode, "apitap-opencode"),
        ("~/work/airflow-dags", Kind::Claude, "airflow-dq"),
        ("~/code/apitap-lib", Kind::Claude, "apitap-lib-qa"),
        ("~/code/cobserve", Kind::Claude, "integrate-jira-api"),
        ("~/code/apitap-lib", Kind::Terminal, "shell"),
    ];
    for (dir, kind, name) in sessions {
        app.claude.open_new(dir, kind);
        let session = app.claude.current_mut().unwrap();
        session.name = Some(name.into());
        session.branch = Some(if dir.ends_with("dags") { "docs-move".into() } else { "main".into() });
    }
    // The first, 7, opened with the view.
    app.claude.list[4].pane.attention = true;
    app.claude.select(6);
    // Eight are more than 120×36 holds as cards: the list, beside the chosen one's screen.
    let screen = render(&app, 120, 36);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    assert!(screen.contains("8 sessions · 1 needs you") && screen.contains("/ find one"), "{screen}");
    // Projects with more than one session are groups, who rang and their shared branch with them.
    assert!(row("airflow-dags 3").contains("! 1  ⎇ docs-move"), "who rang, then the shared branch: {screen}");
    assert!(row("apitap-lib 3").contains("⎇ main"), "{screen}");
    // The lone ones together at the end; the one on screen lit, where it works in its preview.
    assert!(!row("other 2").is_empty(), "{screen}");
    assert!(screen.lines().any(|l| l.contains("integrate-jira-api") && l.contains('▌')), "{screen}");
    assert!(row("╭ ✻ 13 · integrate-jira-api").contains("○ idle") || row("╭ ✻ 13 · integrate-jira-api").contains("starting"), "{screen}");
    assert!(screen.contains("~/code/cobserve  ⎇ main"), "{screen}");
    let lines: Vec<&str> = screen.lines().collect();
    // A line each: the airflow ones sit together although opened apart.
    let dq = lines.iter().position(|l| l.contains("airflow-dq")).unwrap();
    let agent = lines.iter().position(|l| l.contains("airflow-agent")).unwrap();
    assert_eq!(dq - agent, 2, "{screen}");

    // ↓ on the zoomed list walks them as they are shown: by project.
    app.claude.select(1);
    app.update(key(KeyCode::Down));
    assert_eq!(app.claude.current().unwrap().name.as_deref(), Some("MetricsAtomic"));
    app.update(key(KeyCode::Down));
    assert_eq!(app.claude.current().unwrap().name.as_deref(), Some("airflow-dq"), "the next airflow one, not the next opened");
    for (w, h) in [(100, 20), (160, 14), (250, 60)] {
        assert_eq!(render(&app, w, h).lines().count(), h as usize);
    }
}

#[test]
fn view_zero_says_what_claude_code_and_opencode_used() {
    let mut app = app_with_local();
    let now = app.now();
    app.update(Event::Usage(Box::new(crate::fake::usage(now))));
    zoom(&mut app, 10);
    let screen = render(&app, 172, 48);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    // Beside the machine: each program's day, its parts, the plan's window, the week, the breakdowns.
    assert!(row("AI · today").contains("│ AI · today"), "a panel of its own: {screen}");
    let claude = row("✻ Claude Code");
    assert!(claude.contains(" tokens") && claude.contains("≈ $"), "{claude}");
    assert!(row("▣ OpenCode").contains(" tokens") && !row("▣ OpenCode").contains('≈'), "its own cost, no ≈");
    assert!(row(" replies").contains("in ") && row(" replies").contains("cache "), "{screen}");
    assert!(row("5h window").contains(" left · "), "{screen}");
    assert!(!row("last 7 days").is_empty() && screen.contains("by model") && screen.contains("by project"), "{screen}");
    // By project, each program in a column of its own, the day's all under them.
    let head = row("by project");
    assert!(head.contains("✻ Claude Code") && head.contains("▣ OpenCode") && head.contains("total"), "{head}");
    let project = |name: &str| screen.lines().find(|l| l.contains(&format!("  {name} ")) && inner(l).split('│').nth(1).is_some_and(|p| p.contains(name))).unwrap_or_else(|| panic!("{name}: {screen}")).to_string();
    let cobserve = project("cobserve");
    assert!(cobserve.contains("≈ $") && cobserve.contains("$0.01"), "both programs on one project: {cobserve}");
    assert!(project("airflow-dags").contains('—'), "a program that did not work there: {screen}");
    assert!(row("  all ").contains("≈ $"), "{screen}");
    assert!(row("by model").contains("tokens") && row("by model").contains("cost"), "{screen}");
    // This month so far and the last, in the same columns.
    assert!(row("by month").contains("✻ Claude Code") && row(" to the ").contains("≈ $"), "{screen}");
    assert!(screen.contains("≈ Claude Code at API prices"), "what ≈ means: {screen}");
    // Narrow: a line under the pressure.
    let narrow = render(&app, 120, 36);
    let ai = narrow.lines().find(|l| inner(l).trim_start().starts_with("AI ")).unwrap_or_else(|| panic!("{narrow}"));
    assert!(ai.contains("✻ ") && ai.contains(" today ≈ $") && ai.contains("▣ "), "{ai}");
    // The app's status bar has it too.
    let state = crate::desktop::state(&app);
    let chip = state.chips.iter().find(|c| c.id == "ai").expect("an AI chip");
    assert!(chip.text.starts_with("today ") && chip.text.contains(" tokens"), "{chip:?}");
    for (w, h) in [(140, 30), (250, 60), (60, 20)] {
        assert_eq!(render(&app, w, h).lines().count(), h as usize);
    }
}

#[test]
fn the_session_list_says_what_each_session_used_today() {
    use crate::usage::{Reply, Tool, Usage};
    let mut app = app_with_claude(b"hello\r\n");
    let now = app.now();
    app.claude.list[0].conversation = Some("conv-a".into());
    app.claude.list[0].name = Some("integrate-jira-api".into());
    let reply = |tool, session: &str, read: u64| Reply {
        tool: Some(tool),
        at: now - 600,
        model: "claude-opus-5-5".into(),
        project: "cobserve".into(),
        session: session.into(),
        input: 10,
        output: 500,
        cache_read: read,
        cost: (tool == Tool::OpenCode).then_some(0.42),
        ..Reply::default()
    };
    app.update(Event::Usage(Box::new(Usage {
        replies: vec![reply(Tool::Claude, "conv-a", 1_200_000), reply(Tool::Claude, "elsewhere", 3_000_000), reply(Tool::OpenCode, "ses_9", 50_000)],
        notes: Vec::new(),
        read: true,
        ..Default::default()
    })));
    let screen = render(&app, 160, 48);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    // Over the list: each program today, Claude Code's window with it.
    assert!(row("✻ 4.2M today").contains("≈ $") && row("✻ 4.2M today").contains("· 5h "), "{screen}");
    assert!(row("▣ 51k today").contains("$0.42"), "{screen}");
    // On the session's row: what it used, its own conversation only.
    assert!(row("~/work/cobserve").contains("1.2M today"), "{screen}");
}

#[test]
fn the_claude_plan_s_limits_show_where_its_usage_does() {
    let mut app = app_with_local();
    let now = app.now();
    app.update(Event::Usage(Box::new(crate::fake::usage(now))));
    app.update(Event::Limits(Box::new(crate::fake::limits(now))));
    zoom(&mut app, 10);
    let screen = render(&app, 172, 48);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("{text}: {screen}")).to_string();
    // The plan by name, and a line a limit: its share, when it resets.
    assert!(row("✻ Claude Code · Max 5x").contains(" tokens"), "{screen}");
    let session = row("session  ");
    assert!(session.contains(" 11% of it") && session.contains("resets "), "{session}");
    assert!(row("this week  ").contains(" 70% of it"), "{screen}");
    assert!(row("Fable this week").contains(" 29% of it"), "{screen}");
    assert!(!screen.contains("5h window"), "the plan's own figures in its place");
    // Over the sessions' list and in the app's chip: the session and the week.
    zoom(&mut app, 8);
    let list = render(&app, 160, 48);
    assert!(list.contains("✻ 2.9M today · ≈ $3.68 · session"), "the plan beside the day, cut to the list's width: {list}");
    zoom(&mut app, 10);
    let chip = crate::desktop::state(&app).chips.into_iter().find(|c| c.id == "ai").unwrap();
    assert!(chip.text.contains("session 11% · week 70%") && chip.sev == "none", "{chip:?}");
    // Not live: from when, and why.
    let mut stale = crate::fake::limits(now - 4 * 86_400);
    stale.live = false;
    stale.note = Some("Claude Code's login has run out until it next runs".into());
    app.update(Event::Limits(Box::new(stale)));
    assert!(render(&app, 172, 48).contains("as Claude Code last saw them, 4d00h ago"), "{}", render(&app, 172, 48));
}

#[test]
fn the_run_under_the_cursor_has_its_tasks_read_once_for_the_gantt_without_a_page() {
    let mut app = app_on('5');
    // A tick reads the wall clock: put the moment back after it.
    app.update(Event::Tick);
    at_moment(&mut app, MOMENT);
    let asks = app.take_detail_asks();
    let [crate::detail::Ask::Tasks { dag, run }] = asks.as_slice() else { panic!("{asks:?}") };
    assert_eq!(dag, "test_clickhouse_connection");
    // Not asked again before the answer, however often the clock ticks.
    app.update(Event::Tick);
    at_moment(&mut app, MOMENT);
    assert!(app.take_detail_asks().is_empty());
    let ask = asks[0].clone();
    let body = crate::fake::detail(&ask, app.now());
    app.update(Event::Detail(ask, body));
    assert!(app.page().is_none(), "no page opened");
    assert!(app.airflow_tasks.contains_key(&(dag.clone(), run.clone())));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("┃ running") || screen.contains("┃ done"), "the Gantt draws its tasks: {screen}");
    // A DAG's thread asks nothing.
    for _ in 0..12 {
        app.update(key(KeyCode::Down));
    }
    assert!(matches!(app.airflow_selection(), Some(crate::airflow::RowKey::Dag(_))));
    let asked = app.take_detail_asks();
    assert!(asked.iter().all(|a| matches!(a, crate::detail::Ask::Tasks { .. })), "{asked:?}");
    app.update(Event::Tick);
    assert!(app.take_detail_asks().is_empty(), "nothing for a DAG");
}

#[test]
fn twenty_sessions_are_a_list_with_a_preview_and_six_a_grid() {
    use crate::ui2::views::claude::{cap, shape, Shape};
    // What each size holds as cards (the inside of the zoom: two columns and two rows less).
    for (w, h, want) in [(76, 20, 4), (96, 26, 4), (116, 32, 6), (136, 36, 6), (156, 44, 9), (196, 56, 12)] {
        assert_eq!(cap(w, h), want, "{w}×{h}");
    }
    assert!(matches!(shape(116, 32, 6), Shape::Grid { cols: 2, rows: 3, .. }));
    assert!(matches!(shape(116, 32, 7), Shape::List { list_w: 38, preview_w: Some(76) }));
    assert!(matches!(shape(76, 20, 20), Shape::List { list_w: 76, preview_w: None }), "no room for a preview at 80 columns");

    let mut app = app_after(5);
    crate::ui2::tests::with_many_sessions(&mut app, 20);
    zoom(&mut app, 8);
    let screen = render(&app, 120, 36);
    // Every one on screen at once, by project, the chosen one's screen beside them.
    for name in ["billing export", "late partitions", "airflow-dq", "weekly notes", "clickhouse-bi"] {
        assert!(screen.contains(name), "{name}: {screen}");
    }
    assert!(!screen.contains(" more"), "twenty fit without a window: {screen}");
    assert!(screen.contains("airflow-dags 4  ⎇ docs-move") && screen.contains("pipelines 4  ! 1"), "{screen}");
    assert!(screen.contains("20 sessions") && screen.contains("1 needs you"), "{screen}");
    assert!(screen.contains("╭ ✻ 14 · airflow-dq"), "the preview of the chosen one: {screen}");
    // ← → a project at a time; PgDn a page; r renames.
    let name = |app: &App| app.claude.current().unwrap().label();
    app.update(key(KeyCode::Right));
    assert_eq!(name(&app), "apitap-opencode", "the next project's first");
    app.update(key(KeyCode::Left));
    assert_eq!(name(&app), "airflow-agent", "back to the start of the project before");
    app.update(key(KeyCode::Left));
    assert_eq!(name(&app), "clickhouse3");
    app.update(key(KeyCode::PageDown));
    assert_eq!(app.claude.order().last(), Some(&app.claude.active), "a page reaches the end");
    app.update(key(KeyCode::Char('r')));
    // Its name to edit: away with it, then the new one.
    for _ in 0..20 {
        app.update(key(KeyCode::Backspace));
    }
    for c in "notes".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    assert!(render(&app, 120, 36).contains("notes▏"));
    app.update(key(KeyCode::Enter));
    assert_eq!(name(&app), "notes");
    // At 80 columns the list is the zoom, every line with the last thing it printed.
    let narrow = render(&app, 80, 24);
    assert!(!narrow.contains("╭ ✻ 14"), "{narrow}");

    // Down to six, the grid — on the key that closed the seventh.
    while app.claude.list.len() > 6 {
        app.update(key(KeyCode::Char('x')));
        app.update(key(KeyCode::Char('x')));
    }
    let grid = render(&app, 120, 36);
    assert!(grid.lines().filter(|l| l.matches("╭ ").count() >= 2).count() >= 3, "three rows of two cards: {grid}");
    // ← → in the grid: the card beside.
    let order = app.claude.order();
    app.claude.select(order[0]);
    app.update(key(KeyCode::Right));
    assert_eq!(app.claude.active, order[3], "a column of three along");
    for (w, h) in [(40, 12), (80, 24), (250, 70)] {
        render(&app, w, h);
    }
}

#[test]
fn enter_types_in_the_preview_beside_the_list_and_f_opens_it_edge_to_edge() {
    let mut app = app_after(5);
    crate::ui2::tests::with_many_sessions(&mut app, 20);
    zoom(&mut app, 8);
    app.shell.last_size.set((120, 36));
    render(&app, 120, 36);
    // ⏎: the program has the keys, the list stays in sight beside it.
    app.update(key(KeyCode::Enter));
    assert!(app.shell.sessions_full && !app.shell.sessions_spread);
    let screen = render(&app, 120, 36);
    assert!(screen.contains("keys go to airflow-dq") && screen.contains("weekly notes"), "the list beside it: {screen}");
    assert!(screen.contains("typing here · ctrl+\\ the list"), "{screen}");
    assert!(app.viewport.hits.borrow().iter().any(|(_, h)| *h == crate::app::Hit::Pane), "the preview takes the mouse");
    let (rows, cols) = app.claude.want_size.get();
    assert!(cols < 80 && rows > 20, "the program sized to the preview: {rows}×{cols}");
    // A key is the program's: `f` does not leave.
    app.update(key(KeyCode::Char('f')));
    assert!(app.shell.sessions_full && !app.shell.sessions_spread);
    // ctrl+\ back to the list; `f` there opens it edge to edge.
    app.update(ctrl('\\'));
    assert!(!app.shell.sessions_full);
    app.update(key(KeyCode::Char('f')));
    assert!(app.shell.sessions_full && app.shell.sessions_spread);
    let screen = render(&app, 120, 36);
    assert!(!screen.contains("weekly notes ") || screen.contains("‹"), "edge to edge, the others on a bar: {screen}");
    // On the grid there is no preview: ⏎ opens it edge to edge.
    app.update(ctrl('\\'));
    let mut few = app_after(5);
    crate::ui2::tests::with_many_sessions(&mut few, 3);
    zoom(&mut few, 8);
    few.shell.last_size.set((120, 36));
    few.update(key(KeyCode::Enter));
    assert!(few.shell.sessions_full && few.shell.sessions_spread);
}
