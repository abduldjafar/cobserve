//! The screen, rendered through ratatui's `TestBackend` (DESIGN.md §10): every view at the
//! target size, the minimum and below it, the degradation order of §7, and the pieces each
//! screen must contain.

use super::*;
use crate::app::Event;
use crate::fake::FakeSource;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
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
    buffer_with(app, width, height, &theme)
}

fn buffer_with(app: &App, width: u16, height: u16, theme: &crate::theme::Theme) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|frame| draw_with(frame, app, theme)).expect("draw");
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

#[test]
fn the_screen_renders_at_the_target_size() {
    let app = app_after(20);
    let screen = render(&app, 120, 36);
    let lines: Vec<&str> = screen.lines().collect();

    // The masthead: the name, how the fleet is, the tabs, how fresh and the clock.
    assert!(lines[0].starts_with("  ◆ cobserve"), "{}", lines[0]);
    assert!(lines[0].contains("● live") || lines[0].contains("○ paused"));
    assert!(lines[0].contains("1 nodes") && lines[0].contains("5 sessions"), "the tabs: {}", lines[0]);
    assert!(lines[0].contains("✖ critical"), "the fake fleet is in trouble: {}", lines[0]);
    // The day line — no place known here — then the shelf, a row of air either side.
    assert!(lines[1].contains("PRAYER_CITY"), "how to say where: {}", lines[1]);
    assert!(lines[2].trim().is_empty() && lines[5].trim().is_empty(), "{screen}");
    assert!(lines[3].contains("FLEET") && lines[3].contains("mem ━") && lines[3].contains("cpu ━"), "{}", lines[3]);
    assert!(lines[4].contains("REDASH") && lines[4].contains("waiting"), "{}", lines[4]);

    let all = screen.replace('\n', " ");
    assert!(all.contains("clickhouse3"), "a node row");
    assert!(all.contains("/64 "), "denominators on the node row");
    assert!(all.contains("r_redash → grigol.gankava"), "a user row");
    assert!(all.contains("server · caches · merges"), "the closing row");
    assert!(all.contains("healthy nodes folded"), "the fold line");
    assert!(all.contains("USER → PERSON"), "the column header");
    assert!(all.contains("INSIGHTS"), "the insights panel");
    assert!(all.contains("move"), "footer verbs");
}

#[test]
fn every_view_renders_at_every_size_without_panicking() {
    let mut app = app_after(12);
    for view in ['1', '2', '3', '4'] {
        app.update(key(KeyCode::Char(view)));
        for (width, height) in [
            (200u16, 60u16),
            (120, 36),
            (100, 30),
            (80, 24),
            (60, 18),
            (40, 10),
            (20, 5),
            (10, 3),
            (3, 2),
            (1, 1),
        ] {
            let screen = render(&app, width, height);
            assert_eq!(screen.lines().count(), height as usize, "{view} at {width}x{height}");
        }
    }
}

#[test]
fn narrow_terminals_drop_columns_in_the_documented_order() {
    use super::nodes::grid_for;
    // 120: both bars, QUERIES and LONGEST.
    let wide = grid_for(120);
    assert!(wide.mem_bar >= super::nodes::MIN_BAR && wide.cpu_bar >= super::nodes::MIN_BAR);
    assert!(wide.queries && wide.longest);
    // 100 is still not narrow: both bars stay, the label gives a little.
    let hundred = grid_for(100);
    assert!(hundred.cpu_bar > 0, "{hundred:?}");
    // Below 100 the CPU bar goes first; the number stays.
    let no_bar = grid_for(96);
    assert_eq!(no_bar.cpu_bar, 0);
    assert!(no_bar.mem_bar > 0 && no_bar.longest && no_bar.queries, "{no_bar:?}");
    // Then LONGEST.
    let no_longest = grid_for(86);
    assert!(!no_longest.longest && no_longest.queries, "{no_longest:?}");
    // Then QUERIES.
    let bare = grid_for(74);
    assert!(!bare.queries && !bare.longest, "{bare:?}");

    let app = app_with_fake();
    let screen = render(&app, 120, 36);
    assert!(screen.contains("QUERIES") && screen.contains("LONGEST"));
    let screen = render(&app, 86, 36);
    assert!(!screen.contains("LONGEST"), "LONGEST is the second to go");
    assert!(screen.contains("QUERIES"));
    let screen = render(&app, 74, 36);
    assert!(!screen.contains("QUERIES"), "QUERIES goes last");
    assert!(screen.contains("r_redash"), "the rows themselves never disappear");
}

/// The user rows' numbers sit in the node row's columns: the `%` signs line up.
#[test]
fn every_level_of_the_tree_shares_one_grid() {
    let app = app_after(5);
    let screen = render(&app, 120, 36);
    let rows: Vec<&str> = screen
        .lines()
        .filter(|l| l.contains("▾ clickhouse3") || l.contains("├─ r_redash → grigol") || l.contains("└─ server · caches"))
        .collect();
    assert!(rows.len() >= 3, "{screen}");
    let first_pct = |line: &str| line.chars().position(|c| c == '%');
    let columns: Vec<Option<usize>> = rows.iter().map(|l| first_pct(l)).collect();
    assert!(columns.windows(2).all(|w| w[0] == w[1]), "{columns:?}\n{}", rows.join("\n"));
}

#[test]
fn short_terminals_shrink_the_drawer() {
    let app = app_with_fake();
    let tall = render(&app, 120, 36);
    assert!(tall.contains("shard"), "the drawer shows node detail");

    let short = render(&app, 120, 28);
    assert!(!short.contains("shard"), "below 30 rows the drawer shrinks to its title");
    assert!(short.contains("clickhouse3 · pay-ch-node-1"), "…which is still there");
}

#[test]
fn the_queue_view_lists_people_and_the_stitch_to_clickhouse() {
    let mut app = app_after(3);
    app.update(key(KeyCode::Char('2')));
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
    let footer = |screen: &str| screen.lines().last().unwrap_or_default().to_string();
    let mut fake = FakeSource::new();
    let mut app = App::new();
    for _ in 0..3 {
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
    }
    app.update(Event::Queue(Box::new(fake.queue())));
    app.update(key(KeyCode::Char('2')));
    render(&app, 140, 48);
    let job = app.selected_job().cloned().expect("the first running job under the cursor");
    assert_eq!(crate::app::job_words(&job), "#7438 Gateway transfers of grigol.gankava");
    assert!(footer(&render(&app, 140, 48)).contains("x  cancel job"));

    // x asks, about that job, and says what a cancel does to it.
    app.update(key(KeyCode::Char('x')));
    let screen = render(&app, 140, 48);
    assert!(screen.contains("cancel in Redash? · #7438 Gateway transfers of grigol.gankava"), "{screen}");
    assert!(screen.contains("its worker is freed — but its query runs on in ClickHouse on clickhouse3 until KILL QUERY stops it"), "{screen}");
    assert!(screen.contains(&format!("DELETE /api/jobs/{}", job.id)), "{screen}");
    let keys = footer(&screen);
    assert!(keys.contains("y  cancel it in Redash   esc  keep it   any other key keeps it"), "{keys}");
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
    app.update(key(KeyCode::Char('4')));
    let tape = render(&app, 160, 48);
    assert!(tape.contains("Redash job cancelled from here: #7438 Gateway transfers of grigol.gankava — its query runs on in ClickHouse on clickhouse3"), "{tape}");
    app.update(key(KeyCode::Char('2')));
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
    assert!(screen.contains("⊘ waiting") && screen.contains("it leaves the queue and never runs"), "{screen}");
    app.update(key(KeyCode::Esc));
    assert!(app.cancel_asked().is_none() && app.take_cancels().is_empty(), "esc keeps it");
}

#[test]
fn j_and_k_scroll_the_sql_under_a_job() {
    let mut app = app_after(3);
    app.update(key(KeyCode::Char('2')));
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
    app.update(key(KeyCode::Char('5')));
    run_session(&mut app, output);
    app
}

#[test]
fn sessions_are_a_list_beside_claude_and_can_be_renamed() {
    let mut app = app_with_claude(b"the first session\r\n");
    let first = app.claude.current().unwrap().id;
    // ctrl+\ n asks where, starting where the first works; ⏎ opens it there.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
    let asking = render(&app, 140, 40);
    assert!(asking.contains("New Claude session") && asking.contains("Open the session here"), "{asking}");
    app.update(key(KeyCode::Enter));
    run_session(&mut app, b"the second session\r\n");
    // ctrl+\ r: named.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('r')));
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
    assert!(screen.contains("SESSIONS"), "{screen}");
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row("✻  cobserve").contains("● 5"), "the first, by its folder, which rang: {screen}");
    assert!(row("✻  infra").contains('6'), "the second, by its name: {screen}");
    assert!(row("✻  infra").contains('▎') && !row("✻  cobserve").contains('▎'), "the one on screen is lit: {screen}");
    assert!(screen.contains("the second session") && !screen.contains("the first session"), "only the session on screen: {screen}");
    assert!(screen.contains("+  new session"), "{screen}");
    // The keys are the footer's, said once, and pressing ctrl+\ moves none of them.
    let footer = |screen: &str| screen.lines().last().unwrap_or_default().to_string();
    let typing = footer(&screen);
    assert!(typing.contains("ctrl+\\  then   1-4  views   5-9 ↑↓  sessions   /  find one   n  new   r  rename   x  close"), "{typing}");
    assert!(typing.contains("F1-F9  any tab"), "for a terminal that keeps ctrl+\\: {typing}");
    assert_eq!(screen.matches("5-9").count(), 1, "not in the list too: {screen}");

    app.update(ctrl('\\'));
    let bar = render(&app, 140, 40);
    assert!(bar.contains("which one?"), "{bar}");
    let lit = footer(&bar);
    let at = |line: &str, text: &str| line.find(text).unwrap_or_else(|| panic!("{text} in {line}"));
    assert_eq!(at(&lit, "1-4  views"), at(&typing, "1-4  views"), "{lit}");
    assert!(lit.contains("x  close   esc  back   ctrl+\\  monitor"), "the way back: {lit}");
    app.update(key(KeyCode::Char('x')));
    let closing = render(&app, 140, 40);
    assert!(closing.contains("x again closes it"), "{closing}");
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
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
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
    assert!(screen.contains("choose its folder →"), "the new session's card, framed: {screen}");

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
    assert!(found.lines().last().unwrap().contains("clear the search"), "esc clears it first: {found}");

    // ⏎ opens the session in the folder under the cursor, and it is on screen.
    app.update(key(KeyCode::Enter));
    assert_eq!(app.claude.mode, Mode::Typing);
    assert_eq!((app.claude.list.len(), app.claude.current().unwrap().dir.as_str()), (2, "~/work/pipelines"));

    // A click on the first row opens it in the folder looked at; ✕ gives up.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
    answer_picker(&mut app, None, &[], None);
    render(&app, 140, 40);
    click_on(&mut app, Hit::Cancel);
    assert_eq!((app.claude.mode.clone(), app.claude.list.len()), (Mode::Typing, 2));
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
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
    assert!(row(&screen, "✻  cobserve").contains('5'), "Claude's mark: {screen}");
    assert!(row(&screen, "▣  notes").contains('6'), "OpenCode's: {screen}");
    assert!(row(&screen, "❯  scripts").contains('7'), "a shell's: {screen}");
    assert!(screen.lines().last().unwrap().contains("every other key goes to the shell"), "{screen}");

    // The picker offers the three, the one it will open lit; a click changes it.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
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
    assert!(failed.contains("opencode auth login") && failed.contains("OPENCODE_CMD"), "{failed}");
    assert!(failed.lines().last().unwrap().contains("start OpenCode"), "{failed}");
    assert_eq!(app.claude.mode, Mode::Typing);
}

#[test]
fn every_node_has_a_card_under_the_band_and_a_click_opens_one() {
    use crate::app::Hit;
    let mut app = app_with_claude(b"one\r\n");
    let screen = render(&app, 160, 48);
    let lines: Vec<&str> = screen.lines().collect();
    let (names, mem, cpu) = (lines[6], lines[7], lines[8]);
    // The worst first, marked; each with its memory and its CPU, a bar and a share.
    assert!(names.starts_with("        ✖ clickhouse3 "), "{names}");
    assert!(mem.starts_with("   mem  ━") && cpu.starts_with("   cpu  ━"), "{mem}\n{cpu}");
    let cards = app.viewport.listed_nodes.borrow().len();
    assert!(cards >= 5, "{names}");
    assert_eq!(mem.matches('%').count(), cards + 1, "a share on every card, and how high the rest go: {mem}");
    assert_eq!(cpu.matches('%').count(), cards + 1, "{cpu}");
    assert!(names.contains("▲ clickhouse7 lag "), "amber for its lag, said in full: {names}");
    assert!(names.contains("● clickhouse2"), "a quiet one: {names}");
    let at = |name: &str| names.find(name).unwrap_or_else(|| panic!("{name} in {names}"));
    assert!(at("clickhouse3") < at("clickhouse7") && at("clickhouse7") < at("clickhouse2"), "trouble first: {names}");
    let fleet = app.snapshot().unwrap().nodes.len();
    assert!(names.contains(&format!("+{} more", fleet - cards)), "how many did not fit: {names}");
    assert!(mem.contains("≤ ") && cpu.contains("≤ "), "and how high they go: {mem}");
    assert!(lines[9].trim().is_empty() && lines[11].contains("SESSIONS"), "the sessions under the shelf: {screen}");
    assert!(!screen.lines().any(|l| l.trim_start().starts_with("NODES")), "the list beside is the sessions' alone: {screen}");

    // A node that does not answer says why.
    let mut down = app.snapshot().unwrap().clone();
    let name = down.nodes[1].name.clone();
    down.nodes[1] = crate::model::NodeSnapshot::unreachable(&name, "connection refused");
    app.update(Event::Snapshot(Box::new(down)));
    let screen = render(&app, 160, 48);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[6].contains(&format!("✖ {name}")), "{}", lines[6]);
    assert!(lines[7].contains("↯ unreachable") && lines[8].contains("connection refused"), "{}\n{}", lines[7], lines[8]);

    // A click anywhere on a card opens its node on view 1; the session goes on where it was.
    let index = app.viewport.listed_nodes.borrow().iter().position(|n| n == &name).expect("listed");
    let card = app.viewport.hits.borrow().iter().find(|(_, h)| *h == Hit::Node(index)).map(|(r, _)| *r).unwrap();
    assert_eq!(card.height, 3, "the name and both bars");
    app.update(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: card.x + 2,
        row: card.y + 2,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.view, View::Nodes);
    assert_eq!(app.selected(), Some(&crate::tree::RowId::Node(name)));

    // The card for the rest opens view 1 too.
    app.update(key(KeyCode::Char('5')));
    render(&app, 160, 48);
    let more = app.viewport.hits.borrow().iter().find(|(r, h)| *h == Hit::View(View::Nodes) && r.y == 6).map(|(r, _)| *r);
    let more = more.expect("the card for the rest");
    app.update(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: more.x,
        row: more.y + 1,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.view, View::Nodes);

    // Wide, every node; short, the line under the band carries them, both numbers each.
    app.update(key(KeyCode::Char('5')));
    let wide = render(&app, 250, 56);
    let names = wide.lines().nth(6).unwrap();
    assert!(!names.contains("more") && app.viewport.listed_nodes.borrow().len() == fleet, "{names}");
    let short = render(&app_with_claude(b"one\r\n"), 100, 26);
    let line = short.lines().nth(4).unwrap();
    assert!(line.starts_with("  ─ ✖ clickhouse3 mem ") && line.contains("% cpu "), "{line}");
    assert!(line.contains(" more ─"), "how many did not fit: {line}");
    assert!(!short.contains("mem  ━"), "no cards: {short}");
}

#[test]
fn the_day_line_goes_from_subuh_to_isya_with_now_on_it() {
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT);
    let screen = render(&app, 160, 40);
    let day = screen.lines().nth(1).unwrap();
    let at = |text: &str| day.find(text).unwrap_or_else(|| panic!("{text} in {day}"));
    // Kemenag's times for Jakarta that day, in order; the next one says how far off it is.
    let stops = ["Subuh 04:21", "Terbit 05:33", "Dzuhur 11:45", "Ashar 14:48", "Maghrib 17:50 · in 1h58m", "Isya 18:59"];
    for pair in stops.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?}: {day}");
    }
    assert!(at("Ashar") < at("●") && at("●") < at("Maghrib"), "now, between them: {day}");
    assert!(day.trim_end().ends_with("Jakarta"), "where: {day}");
    assert!(screen.lines().next().unwrap().contains("15:52:07 WIB"), "the clock, in the zone: {screen}");
    // A Friday's noon prayer is Jumat.
    at_moment(&mut app, MOMENT - 2 * 86_400);
    assert!(render(&app, 160, 40).lines().nth(1).unwrap().contains("Jumat 11:46"), "{}", render(&app, 160, 40));
    // Narrow, the place goes first, then the times of what has passed; the next stays whole.
    at_moment(&mut app, MOMENT);
    let narrow = render(&app, 90, 30);
    let day = narrow.lines().nth(1).unwrap();
    assert!(day.contains("Maghrib 17:50 · in 1h58m") && !day.contains("Jakarta"), "{day}");
}

#[test]
fn ten_minutes_before_a_prayer_its_reminder_takes_the_line_until_waved_away() {
    use crate::app::Hit;
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT + 6504);
    let said = app.take_notifications();
    assert_eq!(said, ["Maghrib in 10 min · 17:50 · Jakarta"], "said once beyond the screen");
    let screen = render(&app, 120, 36);
    let banner = screen.lines().nth(1).unwrap();
    assert!(banner.contains("◷  Maghrib in 9:29  ·  17:50 WIB  ·  Jakarta"), "{banner}");
    assert!(banner.trim_end().ends_with("d  ✕ dismiss"), "{banner}");
    at_moment(&mut app, MOMENT + 6505);
    assert!(app.take_notifications().is_empty(), "not again");
    // In a session the key goes after ctrl+\; a click does it anywhere.
    app.update(key(KeyCode::Char('5')));
    assert!(render(&app, 120, 36).lines().nth(1).unwrap().contains("ctrl+\\ d  ✕ dismiss"));
    click_on(&mut app, Hit::Dismiss);
    let day = render(&app, 120, 36).lines().nth(1).unwrap().to_string();
    assert!(day.contains("Maghrib 17:50 · in 9:28") && !day.contains("◷"), "the line again: {day}");
    // Its time comes: nothing, it was waved away. The next one is reminded as this one was.
    at_moment(&mut app, MOMENT + 7070);
    assert!(app.take_notifications().is_empty());
    at_moment(&mut app, MOMENT + 10_800);
    let said = app.take_notifications();
    assert_eq!(said, ["Isya in 7 min · 18:59 · Jakarta"], "{said:?}");
    // On view 1, d waves it away.
    app.update(key(KeyCode::Char('1')));
    app.update(key(KeyCode::Char('d')));
    assert!(!render(&app, 120, 36).lines().nth(1).unwrap().contains("◷"));
}

#[test]
fn the_terminal_s_title_says_how_the_fleet_is_and_the_next_prayer() {
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT);
    assert_eq!(super::title(&app), "cobserve ✖ · Maghrib 17:50");
    at_moment(&mut app, MOMENT + 6504);
    assert_eq!(super::title(&app), "cobserve ✖ · ◷ Maghrib in 10 min");
    at_moment(&mut app, MOMENT + 7090);
    assert_eq!(super::title(&app), "cobserve ✖ · Maghrib now");
}

#[test]
fn the_clock_flips_to_utc_with_z_or_a_click() {
    use crate::app::Hit;
    let mut app = app_after(3);
    at_moment(&mut app, MOMENT);
    app.update(key(KeyCode::Char('z')));
    let top = render(&app, 120, 36).lines().next().unwrap().to_string();
    assert!(top.contains("08:52:07 UTC"), "{top}");
    // Prayer times stay the place's own.
    assert!(render(&app, 120, 36).lines().nth(1).unwrap().contains("Maghrib 17:50"));
    click_on(&mut app, Hit::Clock);
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("15:52:07 WIB"));
    // The tape's times are the clock's.
    app.update(key(KeyCode::Char('4')));
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
    app.update(key(KeyCode::Char('5')));
    let screen = render(&app, 140, 40);
    let row = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(row("▣  pipelines").contains("↻ 6"), "kept, not started: {screen}");
    assert!(!row("✻  billing").contains('↻'), "the one on screen started: {screen}");
    assert!(screen.contains("↻  past conversations"), "{screen}");

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
    app.update(key(KeyCode::Char('q')));
    assert!(!render(&app, 160, 40).contains("lines back"), "a key comes back down");
}

#[test]
fn a_narrow_terminal_puts_the_sessions_on_a_bar() {
    let app = app_with_claude(b"one\r\n");
    let screen = render(&app, 96, 30);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[6].contains("✖ clickhouse3") && lines[7].contains(" mem "), "the cards first: {screen}");
    assert!(lines[10].contains(" 5 ✻ cobserve ") && lines[10].contains(" + "), "{}", lines[10]);
    assert!(!screen.contains("SESSIONS"), "{screen}");
}

#[test]
fn the_header_s_tabs_and_the_sessions_can_be_clicked() {
    let app = app_with_claude(b"one\r\n");
    let screen = render(&app, 140, 40);
    let hits = app.viewport.hits.borrow().clone();
    let find = |hit: crate::app::Hit| hits.iter().find(|(_, h)| *h == hit).map(|(r, _)| *r);
    let tape = find(crate::app::Hit::View(View::Tape)).expect("the TAPE tab");
    let top: Vec<String> = screen.lines().next().unwrap().chars().map(String::from).collect();
    let under: String = top[tape.x as usize..(tape.x + tape.width) as usize].concat();
    assert_eq!(under, "4 tape", "the click area is the tab itself");
    assert!(find(crate::app::Hit::Session(0)).is_some() && find(crate::app::Hit::NewSession).is_some());
    let pane = find(crate::app::Hit::Pane).expect("Claude's screen");
    assert_eq!(app.claude.pane_origin.get(), (pane.y, pane.x));
}

#[test]
fn claude_runs_in_view_five_under_the_monitor() {
    let mut app = app_with_claude(b"\x1b]0;\xe2\x9c\xb3 Tidy the README\x07\x1b[1mWelcome to Claude Code\x1b[m\r\n\r\n> fix the failing test\r\n");
    let screen = render(&app, 120, 36);
    assert!(screen.contains("Welcome to Claude Code"), "the program's screen: {screen}");
    assert!(screen.contains("> fix the failing test"), "{screen}");
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[3].contains("FLEET") && lines[4].contains("REDASH"), "the band stays: {screen}");
    assert!(lines[6].contains("✖ clickhouse3"), "the worst of the fleet on the shelf: {}", lines[6]);
    assert!(lines[0].contains("5 sessions"), "a tab of its own: {}", lines[0]);
    assert!(screen.contains("✻  Tidy the README"), "Claude's task, from its title, in the list: {screen}");
    let footer = lines[lines.len() - 1];
    assert!(footer.contains("ctrl+\\  then") && !footer.contains("Tidy the README"), "keys only: {footer}");
    // The way to a new line in Claude's prompt: ctrl+j in any terminal, ⇧⏎ where it is told apart.
    assert!(footer.contains("x  close   ctrl+j  new line"), "{footer}");
    let wide = render(&app, 170, 40);
    assert!(wide.lines().last().unwrap().contains("new line   F1-F9  any tab   every other key goes to Claude"), "{wide}");
    app.modified_enter = true;
    let told_apart = render(&app, 140, 40);
    assert!(told_apart.lines().last().unwrap().contains("x  close   ⇧⏎  new line   F1-F9  any tab"), "{told_apart}");
    app.modified_enter = false;
    assert!(!screen.contains("─ job ·") && !screen.contains("INSIGHTS"), "no drawer, no insights: {screen}");
}

#[test]
fn claude_not_installed_says_how_to_get_it() {
    let mut app = app_after(3);
    app.update(key(KeyCode::Char('5')));
    app.claude.current_mut().unwrap().pane.state = crate::claude::PaneState::Failed(
        "claude is not installed here (not on PATH) — install Claude Code, then sign in once with /login and your Pro or Max account".into(),
    );
    let screen = render(&app, 120, 36);
    assert!(screen.contains("not installed"), "{screen}");
    assert!(screen.contains("no API key"), "it runs on the plan: {screen}");
    assert!(screen.contains("start Claude"), "⏎ is offered: {screen}");
}

#[test]
fn a_notice_on_the_border_makes_room_and_never_runs_into_the_counts() {
    let mut app = app_after(3);
    let said = "discovery via clickhouse-events.example.net: no access — r_reports_daily needs SELECT on system.clusters";
    app.update(Event::Notice(said.into()));
    let bottom = |width: u16| render(&app, width, 30).lines().last().unwrap_or_default().to_string();
    let wide = bottom(240);
    assert!(wide.contains(said) && wide.contains("↑↓  move"), "room for the keys and the notice: {wide}");
    assert!(!wide.contains("nodes polled"), "the counts make way for it: {wide}");
    let mid = bottom(150);
    assert!(mid.contains("▲ discovery via") && mid.trim_end().ends_with('…'), "cut, and says so: {mid}");
    assert!(mid.contains("↑↓  move") && !mid.contains("q  quit"), "the last keys make way: {mid}");
    let narrow = bottom(90);
    assert!(narrow.contains("▲ discovery via"), "{narrow}");
    // Gone, the counts come back.
    app.update(Event::Notice(String::new()));
    let quiet = render(&app, 240, 30).lines().last().unwrap_or_default().to_string();
    assert!(quiet.contains("nodes polled") && quiet.contains("read-only"), "{quiet}");
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
    app.update(key(KeyCode::Char('2')));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("REDASH  0 waiting · 1 running · ●●●○○○○○○ 3/9 workers busy · 5 stale"), "{screen}");
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
}

#[test]
fn an_unconfigured_redash_says_how_to_configure_it() {
    let mut app = App::new();
    app.update(Event::Queue(Box::new(crate::model::QueueStatus::unreachable(
        crate::model::QUEUE_NOT_CONFIGURED,
    ))));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("REDASH  not configured"), "{screen}");
    assert!(screen.contains("--credential file"), "it says where the settings go: {screen}");
    assert!(!screen.contains("unreachable"), "not configured is not an outage: {screen}");
}

#[test]
fn an_unreachable_queue_still_says_something() {
    let mut app = App::new();
    app.update(Event::Queue(Box::new(crate::model::QueueStatus::unreachable(
        crate::sources::redash::Error::Http(401).to_string(),
    ))));
    let screen = render(&app, 120, 36);
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
    assert!(screen.contains("↯ not polled"), "the row: {screen}");
    assert!(screen.contains("not polled · no login for this host"), "the insight: {screen}");
    assert!(!screen.contains("unreachable"), "it was never asked, so it is not down: {screen}");
    assert_eq!(crate::model::NodeSnapshot::unreachable("ch-y", "HTTP 502").down_word(), "unreachable");
}

#[test]
fn the_map_draws_one_tile_per_node() {
    let mut app = app_after(5);
    app.update(key(KeyCode::Char('3')));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("MAP"), "{screen}");
    for node in ["clickhouse3", "clickhouse-bi", "clickhouse7", "ch4"] {
        assert!(screen.contains(node), "{node} has a tile:\n{screen}");
    }
    assert!(screen.contains("MEM") && screen.contains("CPU"));
}

#[test]
fn the_tape_lists_what_happened() {
    let mut app = app_after(12);
    app.update(key(KeyCode::Char('4')));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("TAPE"), "{screen}");
    assert!(screen.contains("watching 8 nodes"), "{screen}");
    assert!(screen.contains("runaway"), "{screen}");
    assert!(screen.contains("joined the fleet"), "clickhouse5 appears after 20 s:\n{screen}");
}

#[test]
fn the_pivot_lists_people_first() {
    let mut app = app_after(3);
    app.update(key(KeyCode::Char('u')));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("USER → PERSON · NODE"), "{screen}");
    let first_row = screen
        .lines()
        .find(|l| l.contains("▸ ") || l.contains("▾ "))
        .unwrap_or_default()
        .to_string();
    assert!(first_row.contains("r_redash →"), "{first_row}");
}

#[test]
fn the_help_overlay_lists_the_keymap() {
    let mut app = app_with_fake();
    app.help = true;
    let screen = render(&app, 120, 36);
    assert!(screen.contains("keys"), "{screen}");
    assert!(screen.contains("pivot"), "{screen}");
    assert!(screen.contains("filter"), "{screen}");
    assert!(screen.contains("runaway query"), "the glyph legend");
    assert!(screen.contains("the clock") && screen.contains("prayer's reminder"), "the new keys: {screen}");
}

#[test]
fn a_filter_is_shown_in_the_footer_while_typing() {
    let mut app = app_with_fake();
    app.update(key(KeyCode::Char('/')));
    for c in "petrova".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    let screen = render(&app, 120, 36);
    assert!(screen.contains("petrova▏"), "{screen}");
    assert!(screen.contains("clear"), "{screen}");
    assert!(screen.contains("matching rows"), "{screen}");
}

#[test]
fn selecting_a_query_shows_its_detail() {
    let mut app = app_after(4);
    // Walk down to the first query row: ⏎ on a user opens it, then the cursor reaches it.
    for _ in 0..8 {
        app.update(key(KeyCode::Enter));
        if app
            .selected_row()
            .map(|(_, id)| matches!(id, crate::tree::RowId::Query { .. }))
            .unwrap_or(false)
        {
            break;
        }
        app.update(key(KeyCode::Down));
    }
    assert!(
        app.selected_row()
            .map(|(_, id)| matches!(id, crate::tree::RowId::Query { .. }))
            .unwrap_or(false),
        "landed on a query row"
    );

    let screen = render(&app, 140, 40);
    assert!(screen.contains("cores"), "the drawer describes the query");
    assert!(screen.contains("rows"), "and its read volume");
    assert!(screen.contains("of its"), "memory against its own limit:\n{screen}");
    assert!(screen.contains("PROGRESS"), "and how far it is:\n{screen}");
    assert!(screen.contains("SELECT ·") || screen.contains("INSERT ·"), "query rows say what they read");
}

/// One node running one long query with `sql`, and the cursor on that query (through its
/// insight, as on call would get there).
fn app_on_query(sql: &str) -> App {
    const GIB: u64 = 1024 * 1024 * 1024;
    let mut query = crate::model::QueryRow::new("q-long", "analyst");
    query.elapsed_s = 95.0;
    query.memory_bytes = GIB;
    query.sql = sql.into();
    let node = crate::model::NodeSnapshot {
        name: "ch-a".into(),
        host: "ch-a".into(),
        port: 9000,
        shard: 1,
        replica: 1,
        version: "24.10".into(),
        reachable: true,
        mem_total: Some(64 * GIB),
        mem_used: 10 * GIB,
        cores: Some(16.0),
        cpu_busy_cores: Some(2.0),
        running: 1,
        lag_s: 0,
        active_parts: 10,
        queries: vec![query],
        uptime_s: Some(100),
        server_cpu_time_us: None,
        max_memory_usage: Some(9 * GIB),
        unreachable_reason: None,
        poll_ms: Some(20),
    };
    let mut app = App::new();
    app.update(Event::Snapshot(Box::new(crate::model::FleetSnapshot {
        taken_at: std::time::SystemTime::now(),
        nodes: vec![node],
    })));
    app.update(key(KeyCode::Tab));
    app.update(key(KeyCode::Enter));
    assert!(
        matches!(app.selected_row().map(|(_, id)| id), Some(crate::tree::RowId::Query { .. })),
        "the long query's insight lands on it"
    );
    app
}

#[test]
fn the_selected_query_shows_its_sql_right_under_it() {
    let app = app_on_query("/* Username: someone@example.net */ SELECT region, count() AS ops FROM accounting.bank_record WHERE day = today() GROUP BY region");
    let screen = render(&app, 140, 44);
    let lines: Vec<&str> = screen.lines().collect();
    let row = lines.iter().position(|l| l.contains("▌")).expect("the cursor's row");
    assert!(lines[row + 1].contains("▎ SELECT region, count() AS ops"), "{screen}");
    assert!(lines[row + 2].contains("▎ FROM accounting.bank_record"), "a clause a line: {screen}");
    assert!(lines[row + 3].contains("▎ WHERE day = today()"), "{screen}");
    assert!(lines[row + 4].contains("▎ GROUP BY region"), "{screen}");
    assert!(!screen.contains("Username"), "Redash's comment is not part of what to read: {screen}");
    assert!(!screen.contains("lines 1–"), "four lines fit: no scrolling\n{screen}");
    let drawer: String = lines[lines.len() - 6..].join("\n");
    assert!(!drawer.contains("bank_record"), "the drawer no longer repeats the SQL:\n{drawer}");

    // With the cursor in the insights, the tree closes the SQL again.
    let mut app = app;
    app.update(key(KeyCode::Tab));
    assert!(!render(&app, 140, 44).contains("▎ SELECT region"));
}

#[test]
fn a_query_longer_than_its_room_scrolls_with_j_and_k() {
    let columns: Vec<String> = (1..=20).map(|n| format!("  column_{n:02},")).collect();
    let sql = format!("SELECT\n{}\n  last_column\nFROM wide_table", columns.join("\n"));
    let mut app = app_on_query(&sql);
    let window = |app: &App| -> (usize, usize, usize) {
        let screen = render(app, 120, 34);
        let re = regex::Regex::new(r"lines (\d+)–(\d+) of (\d+) · J K or shift ↑↓ to scroll").unwrap();
        let caps = re.captures(&screen).unwrap_or_else(|| panic!("no scroll footer:\n{screen}"));
        let n = |i: usize| caps[i].parse::<usize>().unwrap();
        (n(1), n(2), n(3))
    };
    let (first, last, total) = window(&app);
    assert_eq!((first, total), (1, 23), "23 lines, from the top");
    assert!(last < total);
    assert!(render(&app, 120, 34).contains("┃"), "a scrollbar");

    app.update(key(KeyCode::Char('J')));
    app.update(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT)));
    assert_eq!(window(&app).0, 3, "J and shift ↓ each scroll a line");

    for _ in 0..40 {
        app.update(key(KeyCode::Char('J')));
    }
    let (_, last, total) = window(&app);
    assert_eq!(last, total, "it stops at the last line");
    for _ in 0..40 {
        app.update(key(KeyCode::Char('K')));
    }
    assert_eq!(window(&app).0, 1, "and at the first");
    assert!(
        matches!(app.selected_row().map(|(_, id)| id), Some(crate::tree::RowId::Query { .. })),
        "scrolling the SQL does not move the cursor"
    );
}

#[test]
fn the_insights_name_the_trouble() {
    let app = app_after(20);
    let insights = insights_of(&app);
    let screen = render(&app, 140, 44);
    assert!(!insights.is_empty());
    // The first insight is on screen: what it is about, and its line.
    let line: String = insights[0].parts.iter().map(|(text, _)| text.as_str()).collect();
    let prefix: String = line.chars().take(30).collect();
    let row = screen
        .lines()
        .find(|row| row.contains(&prefix))
        .unwrap_or_else(|| panic!("{prefix}\n{screen}"));
    assert!(row.contains(&insights[0].label), "{row}");
}

#[test]
fn tab_highlights_the_chosen_insight() {
    let mut app = app_after(4);
    app.update(key(KeyCode::Tab));
    let screen = render(&app, 120, 36);
    assert!(screen.contains("⏎ go there"), "{screen}");
}

#[test]
fn colours_carry_severity() {
    let app = app_after(3);
    let buf = buffer(&app, 120, 36);
    let theme = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark);
    // Somewhere on the clickhouse3 row the critical red is used.
    let width = buf.area.width as usize;
    let text = text_of(&buf);
    let row = text.lines().position(|l| l.contains("▾ clickhouse3")).expect("clickhouse3 row");
    let reds = (0..width)
        .filter(|x| buf[(*x as u16, row as u16)].fg == theme.crit)
        .count();
    assert!(reds > 0, "a 90.9% node is drawn in red");
}

#[test]
fn a_paused_screen_says_so_and_a_stale_one_too() {
    let mut app = app_with_fake();
    app.update(key(KeyCode::Char('p')));
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("○ paused"));
    app.update(key(KeyCode::Char('p')));
    app.clock = std::time::SystemTime::now() + std::time::Duration::from_secs(30);
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("◌ stale"));
}

/// The screen one fleet showed, which the insights were rebuilt for: a node that answers but
/// refuses the monitor's login a grant, a 27-minute CREATE far past the limit its settings
/// show, a long report, and three quiet nodes. Three short lines, nothing twice, no raw error.
#[test]
fn a_real_fleet_reads_in_a_few_short_lines() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let node = |name: &str, total_gib: u64, used_gib: f64, cores: f64, busy: f64| crate::model::NodeSnapshot {
        name: name.into(),
        host: name.into(),
        port: 9000,
        shard: 1,
        replica: 1,
        version: "24.11.1.2557".into(),
        reachable: true,
        mem_total: Some(total_gib * GIB),
        mem_used: (used_gib * GIB as f64) as u64,
        cores: Some(cores),
        cpu_busy_cores: Some(busy),
        running: 0,
        lag_s: 0,
        active_parts: 100,
        queries: vec![],
        uptime_s: Some(1000),
        server_cpu_time_us: None,
        max_memory_usage: Some(6_000_000_000),
        unreachable_reason: None,
        poll_ms: Some(40),
    };
    let query = |id: &str, user: &str, elapsed: f64, gib: f64, sql: &str| {
        let mut q = crate::model::QueryRow::new(id, user);
        q.elapsed_s = elapsed;
        q.memory_bytes = (gib * GIB as f64) as u64;
        q.memory_limit = Some(6_000_000_000);
        q.sql = sql.into();
        q
    };
    let mut bi = node("clickhouse-bi.example.net", 252, 58.8, 16.0, 8.7);
    bi.queries = vec![query("b1", "metabase", 2.0, 0.4, "SELECT 1"), query("b2", "metabase", 1.0, 0.2, "SELECT 2")];
    let mut ch3 = node("clickhouse3.example.net", 227, 3.5, 8.0, 3.9);
    ch3.queries = vec![
        query("c1", "airflow", 166.0, 0.9, "INSERT INTO statistics.daily SELECT 1"),
        query("c2", "airflow", 3.0, 0.1, "SELECT 1"),
        query("c3", "airflow", 1.0, 0.1, "SELECT 2"),
    ];
    let mut ch2 = node("clickhouse2.example.net", 453, 112.3, 16.0, 3.3);
    ch2.queries = vec![query(
        "m1",
        "ch_user",
        1670.0,
        71.5,
        "CREATE MATERIALIZED VIEW materialized_views.mv_invoice_monthly_totals AS SELECT 1",
    )];
    let snapshot = crate::model::FleetSnapshot {
        taken_at: std::time::SystemTime::now(),
        nodes: vec![
            bi,
            ch3,
            ch2,
            crate::model::NodeSnapshot::unreachable(
                "clickhouse-metrics.example.net",
                "no access — r_reports_daily needs SELECT on system.asynchronous_metrics",
            ),
            node("clickhouse4.example.net", 128, 2.0, 16.0, 0.4),
            node("clickhouse1.example.net", 128, 3.0, 16.0, 0.4),
        ],
    };
    let mut app = App::new();
    app.update(Event::Snapshot(Box::new(snapshot)));
    app.update(Event::Queue(Box::new(crate::model::QueueStatus::unreachable(crate::model::QUEUE_NOT_CONFIGURED))));

    let lines: Vec<String> = insights_of(&app).iter().map(|i| i.text()).collect();
    assert_eq!(
        lines,
        [
            "clickhouse-metrics  no access · r_reports_daily needs SELECT on system.asynchronous_metrics",
            "clickhouse2  long query · ch_user, 27m50s",
            "clickhouse3  long query · airflow, 2m46s",
        ]
    );
    let screen = render(&app, 160, 40);
    let panel: Vec<&str> = screen
        .lines()
        .skip_while(|l| !l.contains("INSIGHTS"))
        .skip(1)
        .take(3)
        .collect();
    assert!(panel[0].contains("✖ clickhouse-metrics  no access · r_reports_daily needs SELECT"), "{screen}");
    assert!(!screen.contains("1279%") && !screen.contains("DB::Exception"), "{screen}");
}

#[test]
fn dump_screen() {
    let app = app_after(30);
    println!("{}", render(&app, 120, 36));
}

// ---------------------------------------------------------------------------
// Screenshots: `SHOT_DIR=docs/screenshots cargo test export_screens -- --ignored`
// writes every view as plain text and as colour HTML (what dev/screenshots.sh turns into
// PNGs). Ignored by default: it writes files.
// ---------------------------------------------------------------------------

fn css(color: ratatui::style::Color, fallback: &str) -> String {
    use ratatui::style::Color;
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => fallback.to_string(),
        Color::Black => "#000000".into(),
        Color::Red => "#cd3131".into(),
        Color::Green => "#0dbc79".into(),
        Color::Yellow => "#e5e510".into(),
        Color::Blue => "#2472c8".into(),
        Color::Magenta => "#bc3fbc".into(),
        Color::Cyan => "#11a8cd".into(),
        Color::Gray => "#e5e5e5".into(),
        Color::DarkGray => "#666666".into(),
        Color::LightRed => "#f14c4c".into(),
        Color::LightGreen => "#23d18b".into(),
        Color::LightYellow => "#f5f543".into(),
        Color::LightBlue => "#3b8eea".into(),
        Color::LightMagenta => "#d670d6".into(),
        Color::LightCyan => "#29b8db".into(),
        Color::White => "#ffffff".into(),
        Color::Indexed(_) => fallback.to_string(),
    }
}

fn html_of(buf: &Buffer, title: &str) -> String {
    html_with(buf, title, &crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark))
}

fn html_with(buf: &Buffer, title: &str, theme: &crate::theme::Theme) -> String {
    use ratatui::style::Modifier;
    let fg0 = css(theme.fg, "#dce2eb");
    let bg0 = css(theme.bg, "#0f1219");
    let mut out = String::new();
    out.push_str(&format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         body{{margin:0;background:{bg0};}}\
         pre{{margin:0;padding:14px 16px;font:15px 'DejaVu Sans Mono',monospace;color:{fg0};background:{bg0};}}\
         div{{height:18px;line-height:18px;white-space:pre;overflow:hidden}}\
         span{{white-space:pre}}\
         i{{display:inline-block;width:1ch;font-style:normal}}\
         </style></head><body><pre>"
    ));
    let width = buf.area.width;
    for y in 0..buf.area.height {
        out.push_str("<div>");
        let mut run = String::new();
        let mut style: Option<(String, String, bool, bool)> = None;
        let flush = |out: &mut String, run: &mut String, style: &Option<(String, String, bool, bool)>| {
            if run.is_empty() {
                return;
            }
            let (fg, bg, bold, underline) = style.clone().unwrap();
            out.push_str(&format!(
                "<span style=\"color:{fg};background:{bg};{}{}\">{run}</span>",
                if bold { "font-weight:bold;" } else { "" },
                if underline { "text-decoration:underline;text-underline-offset:4px;" } else { "" }
            ));
            run.clear();
        };
        for x in 0..width {
            let cell = &buf[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (mut fg, mut bg) = (css(cell.fg, &fg0), css(cell.bg, &bg0));
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.modifier.contains(Modifier::DIM) {
                fg = "#7a8596".into();
            }
            let this = (fg, bg, cell.modifier.contains(Modifier::BOLD), cell.modifier.contains(Modifier::UNDERLINED));
            if style.as_ref() != Some(&this) {
                flush(&mut out, &mut run, &style);
                style = Some(this);
            }
            let symbol = cell.symbol().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            let lines_and_blocks = symbol.chars().all(|c| ('\u{2500}'..='\u{259f}').contains(&c));
            if !symbol.is_ascii() && !lines_and_blocks && unicode_width::UnicodeWidthStr::width(cell.symbol()) == 1 {
                // A glyph the font lacks (⎇, ✳) comes from a fallback font with its own width;
                // held to one column, it no longer pushes the rest of the line along. Lines and
                // blocks are the font's own and come in long runs, where the browser's rounding
                // of each held column would add up.
                run.push_str(&format!("<i>{symbol}</i>"));
            } else {
                run.push_str(&symbol);
            }
        }
        flush(&mut out, &mut run, &style);
        out.push_str("</div>");
    }
    out.push_str("</pre></body></html>\n");
    out
}

/// A sample screen for view 5's screenshot: what a session in the pane looks like, written by
/// hand — the real one is whatever Claude Code draws.
const CLAUDE_DEMO: &[u8] = b"\x1b]0;\xe2\x9c\xb3 Stream the invoice export\x07\
\x1b[38;2;215;119;87m\xe2\x9c\xbb\x1b[m \x1b[1mClaude Code\x1b[m  \x1b[2m~/work/billing\x1b[m\r\n\r\n\
\x1b[2m>\x1b[m make the invoice export stream its rows instead of building one big Vec\r\n\r\n\
\x1b[38;2;215;119;87m\xe2\x97\x8f\x1b[m I'll read the exporter first.\r\n\r\n\
\x1b[32m\xe2\x97\x8f\x1b[m \x1b[1mRead\x1b[m(src/export/invoice.rs)\r\n\
\x20\x20\x1b[2m\xe2\x94\x94  214 lines\x1b[m\r\n\r\n\
\x1b[32m\xe2\x97\x8f\x1b[m \x1b[1mUpdate\x1b[m(src/export/invoice.rs)\r\n\
\x20\x20\x1b[2m\xe2\x94\x94  18 lines added, 31 removed\x1b[m\r\n\r\n\
\x1b[38;2;215;119;87m\xe2\x9c\xbb\x1b[m Running the tests\xe2\x80\xa6 \x1b[2m(esc to interrupt)\x1b[m\r\n\r\n\
\x1b[2m\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\
\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\x1b[m\r\n\
\xe2\x9d\xaf \r\n\
\x1b[2m\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\
\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\x1b[m\r\n";

/// 15:52:07 WIB on Sunday 4 October 2026.
#[test]
fn a_click_puts_the_cursor_on_a_row_and_a_second_opens_it() {
    use crate::app::Hit;
    let mut app = app_after(5);
    render(&app, 120, 36);
    // The third row of the tree: a click selects it, a second opens it.
    let third = app.viewport.hits.borrow().iter().filter_map(|(_, h)| matches!(h, Hit::Row(_)).then_some(*h)).nth(2).unwrap();
    click_on(&mut app, third);
    let Hit::Row(index) = third else { unreachable!() };
    let id = app.with_rows(|_, rows| rows[index].id.clone()).unwrap();
    assert_eq!(app.selected(), Some(&id));
    let before = app.with_rows(|_, rows| rows.len()).unwrap();
    render(&app, 120, 36);
    click_on(&mut app, Hit::Row(index));
    let after = app.with_rows(|_, rows| rows.len()).unwrap();
    assert_ne!(before, after, "opened (or closed) like ⏎");

    // A tile of the map, a line of the tape, the same way.
    app.update(key(KeyCode::Char('3')));
    render(&app, 120, 36);
    click_on(&mut app, Hit::Tile(2));
    assert_eq!(app.map_selection(), 2);
    render(&app, 120, 36);
    click_on(&mut app, Hit::Tile(2));
    assert_eq!(app.view, View::Nodes, "a second click opens the node in view 1");
    app.update(key(KeyCode::Char('4')));
    render(&app, 120, 36);
    if app.tape.len() > 1 {
        click_on(&mut app, Hit::TapeLine(1));
        assert_eq!(app.tape_selection(), 1);
    }
    // An insight: the first click goes into the list, the second goes there.
    app.update(key(KeyCode::Char('1')));
    render(&app, 120, 36);
    click_on(&mut app, Hit::Insight(0));
    assert_eq!(app.focus, crate::app::Focus::Insights);
}

#[test]
fn five_sessions_open_still_leave_room_for_a_new_one() {
    use crate::claude::Kind;
    let mut app = app_after(5);
    app.update(key(KeyCode::Char('5')));
    for i in 1..5 {
        app.claude.open_new(&format!("~/work/project{i}"), Kind::Claude);
    }
    assert_eq!(app.claude.list.len(), 5);
    let screen = render(&app, 140, 40);
    assert!(screen.contains("+  new session") && screen.contains("past conversations"), "{screen}");
    assert!(!screen.contains("of 5"), "not full at five: {screen}");
    // A click on it, or ctrl+\ n, asks where the sixth should work.
    click_on(&mut app, crate::app::Hit::NewSession);
    assert!(matches!(app.claude.mode, crate::claude::Mode::Opening(_)), "{:?}", app.claude.mode);
}

#[test]
fn fifty_sessions_fit_the_list_which_follows_the_one_on_screen_and_finds_one_by_name() {
    use crate::claude::{Kind, MAX_SESSIONS};
    let mut app = app_after(5);
    app.update(key(KeyCode::Char('5')));
    for i in 1..MAX_SESSIONS {
        let kind = [Kind::Claude, Kind::OpenCode, Kind::Terminal][i % 3];
        app.claude.open_new(&format!("~/work/project{i:02}"), kind);
    }
    assert_eq!(app.claude.list.len(), 50);
    let screen = render(&app, 140, 40);
    let row = |screen: &str, text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default().to_string();
    assert!(screen.contains("SESSIONS  50"), "{screen}");
    // A line each, the one on screen — the last — lit, and how many are above it.
    assert!(row(&screen, "▣  project49").contains('▎'), "{screen}");
    assert!(row(&screen, "↑ ").contains(" more"), "{screen}");
    assert!(!screen.contains("project01"), "the first are above, out of sight: {screen}");
    assert!(screen.contains("50 of 50 · x closes one"), "{screen}");

    // ctrl+\ then ↑ walks back through them; the list follows.
    app.update(ctrl('\\'));
    for _ in 0..30 {
        app.update(key(KeyCode::Up));
    }
    assert_eq!(app.claude.active, 19);
    let screen = render(&app, 140, 40);
    assert!(row(&screen, "▣  project19").contains('▎') && row(&screen, "↓ ").contains(" more"), "{screen}");

    // ctrl+\ / finds one by what it is called; ⏎ puts it on screen.
    app.update(key(KeyCode::Char('/')));
    for c in "ject07".chars() {
        app.update(key(KeyCode::Char(c)));
    }
    let screen = render(&app, 140, 40);
    assert!(row(&screen, "SESSIONS").contains("/ject07▏") && row(&screen, "SESSIONS").contains("1 of 1"), "{screen}");
    assert!(row(&screen, "  project07 ").contains('▎'), "{screen}");
    app.update(key(KeyCode::Enter));
    assert_eq!((app.claude.active, &app.claude.mode), (7, &crate::claude::Mode::Typing));
    // By its number too.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('/')));
    app.update(key(KeyCode::Char('4')));
    app.update(key(KeyCode::Char('0')));
    app.update(key(KeyCode::Enter));
    assert_eq!(crate::claude::Sessions::number_of(app.claude.active), 40);

    // Fifty is as many as there can be.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
    assert!(app.notice().is_some_and(|n| n.contains("50 sessions is as many as there can be")), "{:?}", app.notice());

    // On a narrow terminal, the tabs round the one on screen, and how many either side.
    let narrow = render(&app, 90, 30);
    let bar = row(&narrow, " 40 ");
    assert!(bar.contains("‹ ") && bar.contains(" ›"), "{narrow}");
}

/// A query session on clickhouse3 of the fake fleet, on screen, its tables known.
fn app_with_query() -> App {
    let mut app = app_after(118);
    app.claude.open_query(Some("clickhouse3"));
    app.open_claude();
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
    // The list beside it says what it is on and how it went.
    assert!(screen.contains("▦  clickhouse3"), "{screen}");

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
    assert!(screen.contains("⌫  delete it   ctrl+k  ask about it") && screen.contains("esc  unselect"), "the keys for it below: {screen}");

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
    app.update(key(KeyCode::Char('5')));
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('q')));
    let screen = render(&app, 140, 40);
    assert!(screen.contains("New query session") && screen.contains("which server should it run on?"), "{screen}");
    assert!(screen.contains("●  clickhouse3") && screen.contains("search the servers"), "{screen}");
    assert!(screen.contains("choose its server →") && screen.contains("⏎  open it on that server"), "{screen}");
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

#[test]
#[ignore]
fn export_screens() {
    let Ok(dir) = std::env::var("SHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&dir).expect("shot dir");
    // One place and one moment for every screen: Jakarta, Sunday 4 October 2026, 15:52 WIB —
    // an hour into Ashar, Maghrib two hours off.
    let save = |name: &str, app: &mut App, width: u16, height: u16| {
        at_moment(app, MOMENT);
        let buf = buffer(app, width, height);
        std::fs::write(format!("{dir}/{name}.txt"), text_of(&buf) + "\n").expect("write txt");
        std::fs::write(format!("{dir}/{name}.html"), html_of(&buf, name)).expect("write html");
    };

    // Four minutes of history behind every sparkline.
    let mut app = app_after(118);
    save("120x36-nodes", &mut app, 120, 36);
    save("100x30-nodes", &mut app, 100, 30);
    save("160x48-nodes", &mut app, 160, 48);

    // A user opened and a query selected: the drawer at its fullest.
    for _ in 0..8 {
        app.update(key(KeyCode::Enter));
        if app
            .selected_row()
            .map(|(_, id)| matches!(id, crate::tree::RowId::Query { .. }))
            .unwrap_or(false)
        {
            break;
        }
        app.update(key(KeyCode::Down));
    }
    save("140x40-query", &mut app, 140, 40);

    let mut insights = app_after(118);
    insights.update(key(KeyCode::Tab));
    insights.update(key(KeyCode::Down));
    save("120x36-insights", &mut insights, 120, 36);

    let mut pivot = app_after(118);
    pivot.update(key(KeyCode::Char('u')));
    pivot.update(key(KeyCode::Enter));
    save("120x36-pivot", &mut pivot, 120, 36);

    let mut queue = app_after(118);
    queue.update(key(KeyCode::Char('2')));
    queue.update(key(KeyCode::Down));
    save("120x36-queue", &mut queue, 120, 36);
    save("160x48-queue", &mut queue, 160, 48);
    // `x` on the oldest running job: what a cancel will do to it, before anything is sent.
    queue.update(key(KeyCode::Char('x')));
    save("120x36-queue-cancel", &mut queue, 120, 36);
    queue.update(key(KeyCode::Esc));

    let mut quiet = app_after(118);
    quiet.update(Event::Queue(Box::new(quiet_redash_with_leftovers())));
    quiet.update(key(KeyCode::Char('2')));
    save("120x36-queue-leftovers", &mut quiet, 120, 36);

    let mut map = app_after(118);
    map.update(key(KeyCode::Char('3')));
    save("120x36-map", &mut map, 120, 36);

    // Four sessions in three projects: Claude named and on screen, OpenCode that rang while it
    // was not, Claude named by its task, and a shell.
    let mut claude = app_after(118);
    claude.claude.default_dir = "~/work/billing".into();
    claude.update(key(KeyCode::Char('5')));
    let billing = run_session(&mut claude, CLAUDE_DEMO);
    claude.claude.rename_current("billing export");
    use crate::claude::Kind;
    for (dir, kind, title) in [
        ("~/work/pipelines", Kind::OpenCode, &b"\x1b]0;late partitions\x07"[..]),
        ("~/work/reports", Kind::Claude, &b"\x1b]0;\xe2\x9c\xb3 weekly totals\x07"[..]),
        ("~/work/reports", Kind::Terminal, &b"\x1b]0;make totals\x07"[..]),
    ] {
        claude.claude.open_new(dir, kind);
        run_session(&mut claude, title);
    }
    claude.update(ctrl('\\'));
    claude.update(key(KeyCode::Char('5')));
    assert_eq!(claude.claude.current().unwrap().id, billing);
    claude.claude.list[0].branch = Some("main".into());
    claude.claude.list[1].branch = Some("fix-late-partitions".into());
    claude.claude.list[2].branch = Some("add-weekly-totals".into());
    claude.claude.list[3].branch = Some("add-weekly-totals".into());
    claude.claude.list[1].pane.attention = true;
    save("120x36-claude", &mut claude, 120, 36);
    save("160x48-claude", &mut claude, 160, 48);

    // A fourth one's folder being picked, a folder up from billing, among the other projects.
    claude.update(ctrl('\\'));
    claude.update(key(KeyCode::Char('n')));
    claude.update(key(KeyCode::Left));
    let projects = [
        ("billing", Some("main")),
        ("dotfiles", Some("master")),
        ("infra-notes", None),
        ("pipelines", Some("fix-late-partitions")),
        ("reports", Some("add-weekly-totals")),
        ("sandbox", None),
        ("website", Some("redesign")),
    ];
    answer_picker(&mut claude, None, &projects, None);
    for _ in 0..5 {
        claude.update(key(KeyCode::Down));
    }
    save("160x48-claude-new", &mut claude, 160, 48);

    // Two dozen sessions: a line each, the list following the one on screen; then one found
    // by name.
    let mut many = app_after(118);
    many.claude.default_dir = "~/work".into();
    many.update(key(KeyCode::Char('5')));
    let projects = ["billing", "pipelines", "reports", "website", "infra-notes", "dotfiles", "sandbox", "exports"];
    for i in 1..24 {
        let kind = [Kind::Claude, Kind::OpenCode, Kind::Terminal, Kind::Claude][i % 4];
        many.claude.open_new(&format!("~/work/{}", projects[i % projects.len()]), kind);
        let session = many.claude.current_mut().unwrap();
        session.pane.state = crate::claude::PaneState::Running;
        session.name = Some(format!("{} {}", ["fix", "check", "tidy", "look at", "rerun"][i % 5], ["late partitions", "the exports", "weekly totals", "slow merges", "a dashboard", "the README"][i % 6]));
    }
    many.claude.list[3].pane.attention = true;
    many.claude.select(16);
    run_session(&mut many, CLAUDE_DEMO);
    many.claude.list[16].name = Some("stream the invoice export".into());
    save("140x40-claude-many", &mut many, 140, 40);
    many.update(ctrl('\\'));
    many.update(key(KeyCode::Char('/')));
    for c in "export".chars() {
        many.update(key(KeyCode::Char(c)));
    }
    save("140x40-claude-find", &mut many, 140, 40);

    // A query session: what Claude wrote for a comment, run, its answer under it; a line of it
    // selected and Claude about to be asked about that line alone; and another one being
    // typed, the tables of `system.` under the cursor.
    let mut query = app_with_query();
    type_into(&mut query, "-- the ten users using the most memory right now");
    query.update(ctrl('k'));
    query.update(key(KeyCode::Enter));
    answer_console(&mut query);
    query.update(key(KeyCode::Enter));
    answer_console(&mut query);
    save("140x40-query-session", &mut query, 140, 40);
    {
        let console = query.claude.current_mut().and_then(|s| s.console.as_deref_mut()).expect("a query session");
        let row = console.lines.iter().position(|l| l.starts_with("ORDER BY")).expect("an ORDER BY");
        (console.anchor, console.row, console.col) = (Some((row, 0)), row, console.lines[row].chars().count());
    }
    query.update(ctrl('k'));
    type_into(&mut query, "by how many queries instead");
    save("140x40-query-ask", &mut query, 140, 40);
    query.update(key(KeyCode::Esc));
    query.update(key(KeyCode::Esc));
    query.update(ctrl('u'));
    type_into(&mut query, "SELECT database, table, sum(rows) AS rows\nFROM system.pa");
    save("140x40-query-suggest", &mut query, 140, 40);

    // Long enough for clickhouse5 to join, ch6 to drop out and come back, and a kill.
    let mut tape = app_after(118);
    tape.update(key(KeyCode::Char('4')));
    save("120x36-tape", &mut tape, 120, 36);

    let mut help = app_after(10);
    help.update(key(KeyCode::Char('?')));
    save("120x36-help", &mut help, 120, 36);

    // Nine and a half minutes before Maghrib: its reminder in the day line's place.
    let mut reminder = app_after(118);
    at_moment(&mut reminder, MOMENT + 6504);
    let buf = buffer(&reminder, 120, 36);
    std::fs::write(format!("{dir}/120x36-reminder.txt"), text_of(&buf) + "\n").expect("write txt");
    std::fs::write(format!("{dir}/120x36-reminder.html"), html_of(&buf, "120x36-reminder")).expect("write html");

    // THEME=light, and the 16-colour fallback, for terminals that are not dark or not modern.
    let light = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Light);
    let mut fleet = app_after(118);
    at_moment(&mut fleet, MOMENT);
    let buf = buffer_with(&fleet, 120, 36, &light);
    std::fs::write(format!("{dir}/120x36-light.html"), html_with(&buf, "light", &light)).expect("write html");
    let ansi = crate::theme::Theme::new(crate::theme::Depth::Ansi16, crate::theme::Variant::Dark);
    let buf = buffer_with(&fleet, 120, 36, &ansi);
    std::fs::write(format!("{dir}/120x36-ansi16.html"), html_with(&buf, "ansi16", &ansi)).expect("write html");
}
