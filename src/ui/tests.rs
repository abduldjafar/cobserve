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

    assert!(lines[0].contains("FLEETLENS"), "{}", lines[0]);
    assert!(lines[0].contains("LIVE") || lines[0].contains("PAUSED"));
    assert!(lines[0].contains("1 NODES"), "the tabs are in the header: {}", lines[0]);
    assert!(lines[0].contains("CRITICAL"), "the fake fleet is in trouble: {}", lines[0]);
    assert!(lines[1].contains("FLEET"), "{}", lines[1]);
    assert!(lines[2].contains("REDASH"), "{}", lines[2]);
    assert!(lines[2].contains("waiting"), "{}", lines[2]);

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
    app.update(key(KeyCode::Char('5')));
    run_session(&mut app, output);
    app
}

#[test]
fn sessions_are_a_list_beside_claude_and_can_be_renamed() {
    let mut app = app_with_claude(b"the first session\r\n");
    let first = app.claude.current().unwrap().id;
    // ctrl+\ n asks where; ⏎ opens it where the first works.
    app.update(ctrl('\\'));
    app.update(key(KeyCode::Char('n')));
    let asking = render(&app, 140, 40);
    assert!(asking.contains("new session, in:") && asking.contains("where should it work"), "{asking}");
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
    assert!(row("5 ✳ claude").contains('●'), "the first, which rang: {screen}");
    assert!(!row("6 ✳ infra").is_empty(), "the second, by its name: {screen}");
    assert!(screen.contains("the second session") && !screen.contains("the first session"), "only the session on screen: {screen}");
    assert!(screen.contains("+  new session"), "{screen}");
    assert!(screen.contains("5-9 session") && screen.contains("F1-F9"), "the keys, for when the mouse is not at hand: {screen}");

    app.update(ctrl('\\'));
    let bar = render(&app, 140, 40);
    assert!(bar.contains("which one?") && bar.contains("rename") && bar.contains("close"), "{bar}");
    app.update(key(KeyCode::Char('x')));
    let closing = render(&app, 140, 40);
    assert!(closing.contains("x again closes it"), "{closing}");
}

#[test]
fn a_narrow_terminal_puts_the_sessions_on_a_bar() {
    let app = app_with_claude(b"one\r\n");
    let screen = render(&app, 96, 30);
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[4].contains(" 5 claude ") && lines[4].contains(" + "), "{}", lines[4]);
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
    assert_eq!(under, " 4 TAPE ", "the click area is the tab itself");
    assert!(find(crate::app::Hit::Session(0)).is_some() && find(crate::app::Hit::NewSession).is_some());
    let pane = find(crate::app::Hit::Pane).expect("Claude's screen");
    assert_eq!(app.claude.pane_origin.get(), (pane.y, pane.x));
}

#[test]
fn claude_runs_in_view_five_under_the_monitor() {
    let app = app_with_claude(b"\x1b]0;\xe2\x9c\xb3 Tidy the README\x07\x1b[1mWelcome to Claude Code\x1b[m\r\n\r\n> fix the failing test\r\n");
    let screen = render(&app, 120, 36);
    assert!(screen.contains("Welcome to Claude Code"), "the program's screen: {screen}");
    assert!(screen.contains("> fix the failing test"), "{screen}");
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[1].contains("FLEET") && lines[2].contains("REDASH"), "the band stays: {screen}");
    assert!(lines[3].contains("✖ clickhouse3"), "the worst of the fleet under the band: {}", lines[3]);
    assert!(lines[0].contains("5 CLAUDE"), "a tab of its own: {}", lines[0]);
    assert!(screen.contains("ctrl+\\") && screen.contains("every other key goes to Claude"), "{screen}");
    assert!(screen.contains("✳ Tidy the README"), "Claude's task, from its title: {screen}");
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
    assert!(screen.contains("not in this pass"), "unbuilt keys stay out (§3)");
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
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("PAUSED"));
    app.update(key(KeyCode::Char('p')));
    app.clock = std::time::SystemTime::now() + std::time::Duration::from_secs(30);
    assert!(render(&app, 120, 36).lines().next().unwrap().contains("STALE"));
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
        let mut style: Option<(String, String, bool)> = None;
        let flush = |out: &mut String, run: &mut String, style: &Option<(String, String, bool)>| {
            if run.is_empty() {
                return;
            }
            let (fg, bg, bold) = style.clone().unwrap();
            out.push_str(&format!(
                "<span style=\"color:{fg};background:{bg};{}\">{run}</span>",
                if bold { "font-weight:bold;" } else { "" }
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
            let this = (fg, bg, cell.modifier.contains(Modifier::BOLD));
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

#[test]
#[ignore]
fn export_screens() {
    let Ok(dir) = std::env::var("SHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&dir).expect("shot dir");
    let save = |name: &str, app: &App, width: u16, height: u16| {
        let buf = buffer(app, width, height);
        std::fs::write(format!("{dir}/{name}.txt"), text_of(&buf) + "\n").expect("write txt");
        std::fs::write(format!("{dir}/{name}.html"), html_of(&buf, name)).expect("write html");
    };

    // Four minutes of history behind every sparkline.
    let mut app = app_after(118);
    save("120x36-nodes", &app, 120, 36);
    save("100x30-nodes", &app, 100, 30);
    save("160x48-nodes", &app, 160, 48);

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
    save("140x40-query", &app, 140, 40);

    let mut insights = app_after(118);
    insights.update(key(KeyCode::Tab));
    insights.update(key(KeyCode::Down));
    save("120x36-insights", &insights, 120, 36);

    let mut pivot = app_after(118);
    pivot.update(key(KeyCode::Char('u')));
    pivot.update(key(KeyCode::Enter));
    save("120x36-pivot", &pivot, 120, 36);

    let mut queue = app_after(118);
    queue.update(key(KeyCode::Char('2')));
    queue.update(key(KeyCode::Down));
    save("120x36-queue", &queue, 120, 36);
    save("160x48-queue", &queue, 160, 48);

    let mut quiet = app_after(118);
    quiet.update(Event::Queue(Box::new(quiet_redash_with_leftovers())));
    quiet.update(key(KeyCode::Char('2')));
    save("120x36-queue-leftovers", &quiet, 120, 36);

    let mut map = app_after(118);
    map.update(key(KeyCode::Char('3')));
    save("120x36-map", &map, 120, 36);

    // Three sessions in three projects: one named and on screen, one that rang while it was
    // not, one that Claude itself named by its task.
    let mut claude = app_after(118);
    claude.claude.default_dir = "~/work/billing".into();
    claude.update(key(KeyCode::Char('5')));
    let billing = run_session(&mut claude, CLAUDE_DEMO);
    claude.claude.rename_current("billing export");
    for (dir, title) in [
        ("~/work/pipelines", &b"\x1b]0;\xe2\x9c\xb3 late partitions\x07"[..]),
        ("~/work/reports", &b"\x1b]0;\xe2\x9c\xb3 weekly totals\x07"[..]),
    ] {
        claude.update(ctrl('\\'));
        claude.update(key(KeyCode::Char('n')));
        claude.update(ctrl('u'));
        for c in dir.chars() {
            claude.update(key(KeyCode::Char(c)));
        }
        claude.update(key(KeyCode::Enter));
        run_session(&mut claude, title);
    }
    claude.update(ctrl('\\'));
    claude.update(key(KeyCode::Char('5')));
    assert_eq!(claude.claude.current().unwrap().id, billing);
    claude.claude.list[0].branch = Some("main".into());
    claude.claude.list[1].branch = Some("fix-late-partitions".into());
    claude.claude.list[2].branch = Some("add-weekly-totals".into());
    claude.claude.list[1].pane.attention = true;
    save("120x36-claude", &claude, 120, 36);
    save("160x48-claude", &claude, 160, 48);

    // Long enough for clickhouse5 to join, ch6 to drop out and come back, and a kill.
    let mut tape = app_after(118);
    tape.update(key(KeyCode::Char('4')));
    save("120x36-tape", &tape, 120, 36);

    let mut help = app_after(10);
    help.update(key(KeyCode::Char('?')));
    save("120x36-help", &help, 120, 36);

    // THEME=light, and the 16-colour fallback, for terminals that are not dark or not modern.
    let light = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Light);
    let fleet = app_after(118);
    let buf = buffer_with(&fleet, 120, 36, &light);
    std::fs::write(format!("{dir}/120x36-light.html"), html_with(&buf, "light", &light)).expect("write html");
    let ansi = crate::theme::Theme::new(crate::theme::Depth::Ansi16, crate::theme::Variant::Dark);
    let buf = buffer_with(&fleet, 120, 36, &ansi);
    std::fs::write(format!("{dir}/120x36-ansi16.html"), html_with(&buf, "ansi16", &ansi)).expect("write html");
}
