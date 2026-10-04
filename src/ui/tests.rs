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
    let screen = render(&app, 120, 36);
    assert!(screen.contains("WAITING"), "{screen}");
    assert!(screen.contains("RUNNING"), "{screen}");
    assert!(screen.contains("r_redash → r.simonyte"), "a waiting person");
    assert!(screen.contains("→ clickhouse3"), "the arrow into ClickHouse");
    assert!(screen.contains("6/6 busy"), "worker saturation");
    assert!(screen.contains("●●●●●●"), "workers drawn as slots");
    assert!(screen.contains("runaway ClickHouse query"), "the drawer explains why it is full: {screen}");
    // Waiting jobs are listed as waiting, not as running.
    let waiting = screen.find("WAITING ·").expect("waiting section");
    let running = screen.find("RUNNING ·").expect("running section");
    let simonyte = screen.find("r_redash → r.simonyte").expect("r.simonyte waits");
    assert!(waiting < simonyte && simonyte < running, "r.simonyte is in the WAITING half");
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
        "CREATE MATERIALIZED VIEW materialized_views.mv_gateway_bank_statement AS SELECT 1",
    )];
    let snapshot = crate::model::FleetSnapshot {
        taken_at: std::time::SystemTime::now(),
        nodes: vec![
            bi,
            ch3,
            ch2,
            crate::model::NodeSnapshot::unreachable(
                "clickhouse-posthog.example.net",
                "no access — r_datateam_sync needs SELECT on system.asynchronous_metrics",
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
            "clickhouse-posthog  no access · r_datateam_sync needs SELECT on system.asynchronous_metrics",
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
    assert!(panel[0].contains("✖ clickhouse-posthog  no access · r_datateam_sync needs SELECT"), "{screen}");
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
            let escaped = run.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            out.push_str(&format!(
                "<span style=\"color:{fg};background:{bg};{}\">{escaped}</span>",
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
            run.push_str(cell.symbol());
        }
        flush(&mut out, &mut run, &style);
        out.push_str("</div>");
    }
    out.push_str("</pre></body></html>\n");
    out
}

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

    let mut map = app_after(118);
    map.update(key(KeyCode::Char('3')));
    save("120x36-map", &map, 120, 36);

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
