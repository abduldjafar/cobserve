//! The new shell through ratatui's `TestBackend`, against `docs/redesign/screens/`
//! (`06-build-order.md` *How the comparison works*).
//!
//! What is compared exactly: the chrome (the top bar's name and clock, the status bar's hints),
//! every frame's corners and sides (`frames()` — a panel or a tile in the wrong place fails), the
//! titles, and the words and positions of the labels. What is not: the numbers inside the panels.
//! The screens were drawn by the simulation from its own fleet, and the fake fleet (`fake.rs`) is
//! another one; a frozen copy of the simulation's numbers (`FAKE_SCENARIO=redesign`) would be a
//! second fake to keep in step with the first, for numbers the engine's own tests already prove.
//! Where a number is checked here it is checked against the engine (`fleet_totals`), so the panel
//! is held to the same §5 arithmetic.

use super::*;
use crate::app::Event;
use crate::caps::GraphTier;
use crate::fake::FakeSource;
use crate::model::fleet_totals;
use crate::ui2::layout::{Density, Preset};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// 2026-10-13 15:52:07 WIB, the simulation's instant.
const MOMENT: u64 = 1_791_881_527;

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ch(c: char) -> Event {
    key(KeyCode::Char(c))
}

/// The fake fleet after `polls` polls, the clock at the simulation's instant in
/// Jakarta, and block graphs — the tier the screens were drawn in.
fn app_after(polls: usize) -> App {
    // The polls end at the simulation's instant, so the history and the tape are of its hour.
    let start = std::time::UNIX_EPOCH + std::time::Duration::from_secs(MOMENT - 2 * polls.max(1) as u64);
    let mut fake = FakeSource::starting_at(start);
    let mut app = App::new();
    app.caps.graph = GraphTier::Block;
    for i in 0..polls.max(1) {
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        if i % 3 == 0 {
            app.update(Event::Queue(Box::new(fake.queue())));
        }
    }
    app.update(Event::Queue(Box::new(fake.queue())));
    app.update(Event::Airflow(Box::new(crate::fake::airflow(MOMENT as i64))));
    app.update(Event::Jira(Box::new(crate::fake::jira(MOMENT as i64))));
    app.time.zone = Some("Asia/Jakarta".into());
    app.time.pinned = chrono::FixedOffset::east_opt(7 * 3600);
    app.prayers = crate::prayer::Prayers::new(crate::prayer::city("Jakarta"), 0);
    app.tick_at(std::time::UNIX_EPOCH + std::time::Duration::from_secs(MOMENT));
    app.numbers_fresh_at_the_clock();
    app
}

fn render(app: &App, w: u16, h: u16) -> Vec<String> {
    let theme = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|frame| draw_with(frame, app, &theme)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect()
}

/// A screen file's rows, padded to its width (the files keep no trailing spaces).
fn screen(name: &str) -> Vec<String> {
    let path = format!("{}/docs/redesign/screens/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines().map(str::to_string).collect()
}

/// Where a row has frame glyphs: corners, sides, tees. Two rows with the same frames have their
/// panels, tiles and overlays in the same places.
fn frames(row: &str) -> Vec<(usize, char)> {
    row.chars().enumerate().filter(|(_, c)| "╭╮╰╯│├┤".contains(*c)).collect()
}

fn same_frames(got: &[String], want: &[String], rows: impl IntoIterator<Item = usize>, what: &str) {
    for y in rows {
        assert_eq!(frames(&got[y]), frames(&want[y]), "{what} row {y}:\n got  {}\n want {}", got[y], want[y]);
    }
}

/// The column where `needle` starts in `row`, in cells.
fn col(row: &str, needle: &str) -> Option<usize> {
    let byte = row.find(needle)?;
    Some(crate::fmt::width(&row[..byte]))
}

/// Cells `from..to` of a row.
fn cells(row: &str, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut x = 0;
    for c in row.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if x >= from && x + w <= to {
            out.push(c);
        }
        x += w;
    }
    out
}

fn trimmed(s: &str) -> &str {
    s.trim_end()
}

#[test]
fn the_120x36_dashboard_chrome_and_the_fleet_and_queue_row() {
    let app = app_after(30);
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash");

    // Row 0: the name at the left, the clock at the right, the day line between them.
    assert!(got[0].starts_with(" ❖ cobserve · prod    "), "{}", got[0]);
    assert!(trimmed(&got[0]).ends_with("● live   15:52:07 WIB"), "{}", got[0]);
    assert_eq!(col(&got[0], "● live"), col(&want[0], "● live"));
    assert!(got[0].contains("━●┄") && got[0].contains(" in "), "the day line, the present on it: {}", got[0]);
    assert_eq!(col(&got[0], ":").map(|c| c >= 22), Some(true), "the day line starts at column 22: {}", got[0]);

    // Rows 1–8: every frame where the screen has it, and the titles.
    same_frames(&got, &want, 1..=8, "120x36");
    assert!(got[1].starts_with("╭ 1 FLEET ─"), "{}", got[1]);
    assert_eq!(col(&got[1], "╭ 2 REDASH QUEUE ─"), Some(66));
    assert!(got[1][..got[1].find("╮").unwrap()].ends_with("hot · 9 nodes ─"), "the fleet's summary ends one cell before the corner: {}", got[1]);
    assert!(got[8].starts_with("╭ 3 NODES ─") && trimmed(&got[8]).ends_with("sorted by pressure · s ─╮"), "{}", got[8]);

    // The big number is the fleet's memory share, drawn as the BIG table draws it.
    let t = app.with_view(fleet_totals).unwrap();
    let pct = format!("{}%", t.mem_pct().unwrap().round() as i64);
    let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 20, 3));
    let end = glyph::big::draw(&mut paint::Painter::new(&mut buf), 0, 0, &pct, ratatui::style::Style::default());
    for r in 0..3u16 {
        let big: String = (0..end - 1).map(|x| buf[(x, r)].symbol().to_string()).collect();
        assert!(cells(&got[2 + r as usize], 2, 120).starts_with(&big), "big row {r}: {} / {big}", got[2 + r as usize]);
    }
    let beside = 2 + end as usize + 1;
    assert_eq!(col(&got[2], "cpu"), Some(beside), "{}", got[2]);
    assert_eq!(col(&got[3], &crate::fmt::pct(t.cpu_pct())), Some(beside), "{}", got[3]);
    assert!(got[5].contains("GiB memory") && got[6].contains(" queries · ") && got[6].contains(" ! ·…"), "{}\n{}", got[5], got[6]);
    assert_eq!(col(&got[6], "█ mem  • cpu  now"), col(&want[6], "█ mem  • cpu  now"), "the legend ends where the graph does");
    assert!(got[6].contains("-4m"), "the window printed: four minutes of history, stretched (`02-panels.md`): {}", got[6]);

    // The queue: waiting in big digits, the counts beside it, the workers under it.
    for (row, word) in [(2, "waiting"), (3, " running"), (4, " stale"), (5, " worker")] {
        assert!(cells(&got[row], 66, 120).contains(word), "row {row}: {}", got[row]);
    }
    assert_eq!(col(&got[6], "waiting jobs  now"), col(&want[6], "waiting jobs  now"));

    // Row 35: the hints of the Action table, then the poll line.
    let hints = " 1-8 panel   ⏎ open nodes   ←→↑↓ choose   e explain   p layout   : command";
    assert!(got[35].starts_with(hints), "{}", got[35]);
    assert!(want[35].starts_with(hints));
    assert!(trimmed(&got[35]).ends_with("· read-only") && got[35].contains("all · 9/9 nodes · "), "{}", got[35]);
}

#[test]
fn the_80x24_and_160x48_dashboards_have_their_frames_where_the_screens_do() {
    let app = app_after(30);
    let got = render(&app, 80, 24);
    let want = screen("80x24-dash");
    assert!(got[0].starts_with(" ❖ cobserve · prod") && trimmed(&got[0]).ends_with("● live   15:52:07 WIB"), "{}", got[0]);
    assert!(got[0].contains("☾ Maghrib in "), "under 60 free columns the chip: {}", got[0]);
    assert_eq!(col(&got[0], "☾ Maghrib"), col(&want[0], "☾ Maghrib"));
    same_frames(&got, &want, 1..=5, "80x24");
    assert!(got[2].starts_with("│ mem ") && got[3].starts_with("│ cpu ") && got[4].contains(" q · "), "plain lines under 60 columns:\n{}\n{}\n{}", got[2], got[3], got[4]);
    assert!(cells(&got[2], 44, 80).contains(" waiting"), "{}", got[2]);
    assert!(got[23].starts_with(" 1-8 panel   ⏎ open nodes   ←→↑↓ choose  "), "{}", got[23]);

    let got = render(&app, 160, 48);
    let want = screen("160x48-dash");
    assert!(trimmed(&got[0]).ends_with("● live   15:52:07 WIB"));
    assert!(got[0].contains("Maghrib 17:") || got[0].contains("Maghrib 18:"), "names from 80 free columns: {}", got[0]);
    same_frames(&got, &want, 1..=10, "160x48");
}

#[test]
fn the_palette_its_typed_form_and_the_keys_box() {
    let mut app = app_after(30);
    app.update(ch(':'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-palette");
    // The box, its separators and its borders, over the grid.
    for y in 7..=19 {
        let g: Vec<(usize, char)> = frames(&got[y]).into_iter().filter(|(x, _)| (21..=98).contains(x)).collect();
        let w: Vec<(usize, char)> = frames(&want[y]).into_iter().filter(|(x, _)| (21..=98).contains(x)).collect();
        assert_eq!(g, w, "palette row {y}:\n got  {}\n want {}", got[y], want[y]);
    }
    assert_eq!(col(&got[7], "╭─ :  ─"), Some(21), "{}", got[7]);
    assert!(got[8].contains("│ › █type two or three letters · tab for nodes, people, queries, jobs…"), "{}", got[8]);
    for (y, row) in (10..18).zip(["dashboard", "fleet", "queue", "nodes", "node", "airflow", "jira", "tape"]) {
        assert_eq!(cells(&got[y], 21, 99), cells(&want[y], 21, 99), "the panels first, each with its digit: row {y}");
        assert!(got[y].contains(row));
    }
    assert!(got[19].contains("╰─ ↑↓ choose   ⏎ run   tab channel ›actions───nodes───people───queries───"), "{}", got[19]);

    for c in "exp".chars() {
        app.update(ch(c));
    }
    let got = render(&app, 120, 36);
    assert!(got[10].contains("│ › exp█"), "{}", got[10]);
    // The screen lists `export incident report` and `setup · airflow` too: those actions arrive
    // with the tape's report (phase 3) and the wizard (phase 5).
    assert!(got[12].contains("│▌ explain this number            the number under the cursor · its §5 …   e │"), "{}", got[12]);

    app.update(key(KeyCode::Esc));
    assert!(app.shell.palette.is_none(), "esc closes it");
    app.update(ch('?'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-palette-help");
    assert_eq!(col(&got[7], "╭─ ? keys · dash ─"), Some(21), "{}", got[7]);
    assert_eq!(cells(&got[8], 21, 99), cells(&want[8], 21, 99), "the query line");
    app.update(key(KeyCode::Esc));

    // ⏎ runs the chosen row: `fleet`, the second, zooms panel 1.
    app.update(ch(':'));
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Enter));
    assert_eq!(app.shell.zoom, Some(PanelId::Fleet));
    // A channel: `tab` to the nodes, type a name, ⏎ opens that node.
    app.update(ch(':'));
    app.update(key(KeyCode::Tab));
    for c in "bi".chars() {
        app.update(ch(c));
    }
    app.update(key(KeyCode::Enter));
    assert_eq!((app.shell.zoom, app.shell.chosen.as_deref()), (Some(PanelId::Node), Some("clickhouse-bi")));
}

#[test]
fn density_and_presets() {
    let mut app = app_after(30);
    app.update(ch('+'));
    assert_eq!(app.shell.density, Density::Compact);
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash-density-compact");
    assert!(got[0].contains("☾ Maghrib in ") && !got[0].contains("━●┄"), "compact: the chip instead of the day line: {}", got[0]);
    assert_eq!(col(&got[0], "☾ Maghrib"), col(&want[0], "☾ Maghrib"));
    same_frames(&got, &want, 1..=8, "compact");

    app.update(ch('+'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash-density-focus");
    assert!(got[0].starts_with("╭ 1 FLEET ─"), "focus: no top bar: {}", got[0]);
    same_frames(&got, &want, 0..=7, "focus");
    assert_eq!(&got[35][..col(&got[35], "  ").unwrap()], &want[35][..col(&want[35], "  ").unwrap()]);

    app.update(ch('+'));
    app.update(ch('p'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash-preset-fleet");
    same_frames(&got, &want, 1..=8, "preset fleet");
    assert!(got[35].contains("fleet · 9/9 nodes"), "{}", got[35]);
    app.update(ch('p'));
    assert!(render(&app, 120, 36)[35].contains("ops · 9/9 nodes"));
}

#[test]
fn focus_tab_zoom_and_back() {
    let mut app = app_after(5);
    let got = render(&app, 120, 36);
    assert!(got[8].starts_with("╭ 3 NODES ─"), "{}", got[8]);
    app.update(key(KeyCode::Tab));
    assert_eq!(app.shell.focus, PanelId::Node);
    app.update(ch('2'));
    app.update(key(KeyCode::Enter));
    let got = render(&app, 120, 36);
    assert!(got[0].contains("dashboard › queue") && got[0].contains("esc back "), "{}", got[0]);
    assert!(got[1].starts_with("╭ 2 REDASH QUEUE · ") && got[34].starts_with("╰"), "one frame over the body: {}", got[1]);
    assert!(got[35].starts_with(" esc back"), "{}", got[35]);
    app.update(ch('6'));
    assert_eq!(app.shell.zoom, Some(PanelId::Jira), "a digit switches the zoom");
    app.update(key(KeyCode::Esc));
    assert_eq!((app.shell.zoom, app.shell.focus), (None, PanelId::Jira));
    app.update(key(KeyCode::Esc));
    assert!(!app.quit, "esc never quits");
}

#[test]
fn a_later_phase_s_key_says_so_in_a_toast() {
    let mut app = app_after(3);
    app.update(ch('W'));
    let got = render(&app, 120, 36);
    assert!(got[34].contains(" wall — not built yet ▕"), "bottom-right, one column in from the edge: {}", got[34]);
}

/// Frames of a row, leaving out the cells listed: where the screen's simulation drew over a corner.
fn frames_but(row: &str, skip: &[usize]) -> Vec<(usize, char)> {
    frames(row).into_iter().filter(|(x, _)| !skip.contains(x)).collect()
}

#[test]
fn the_nodes_and_node_panels_on_the_dashboards() {
    let app = app_after(30);
    for (w, h, rows) in [(120u16, 36u16, 8..=28), (80, 24, 6..=19), (160, 48, 10..=40)] {
        let got = render(&app, w, h);
        let want = screen(&format!("{w}x{h}-dash"));
        for y in rows {
            // The simulation writes ` clickhouse5 NEW ` over its tile's corner when the name is
            // as long as the tile allows; this shell cuts the name instead (`clickhouse5 N…`) and
            // keeps the corner — so that one cell is not compared.
            let skip: Vec<usize> = frames(&want[y]).iter().filter(|(x, _)| want[y].chars().nth(x.saturating_sub(1)) == Some(' ') && cells(&want[y], x.saturating_sub(4), *x).contains("NEW")).map(|(x, _)| *x).collect();
            let skip_got: Vec<usize> = frames(&got[y]).iter().filter(|(x, c)| *c == '╮' && cells(&got[y], x.saturating_sub(3), *x).contains('…')).map(|(x, _)| *x).collect();
            let all: Vec<usize> = skip.iter().chain(&skip_got).copied().collect();
            assert_eq!(frames_but(&got[y], &all), frames_but(&want[y], &all), "{w}x{h} row {y}:\n got  {}\n want {}", got[y], want[y]);
        }
    }
    let got = render(&app, 120, 36);
    // The tiles: name, bar and share, the sparkline, queries and the longest.
    assert!(got[9].contains("╭ clickhouse3 ───╮") && got[10].contains("%"), "{}\n{}", got[9], got[10]);
    assert!(cells(&got[12], 2, 20).contains("4q !2"), "queries and runaways: {}", got[12]);
    // NODE: one row a person, then the server, on one scale.
    assert!(got[21].starts_with("│ grigol.ga… ") && got[25].starts_with("│ server · … "), "{}\n{}", got[21], got[25]);
    let filled: usize = (21..=25).map(|y| cells(&got[y], 13, 27).chars().filter(|c| *c == '█').count()).sum();
    let node = app.with_view(|v| v.nodes.iter().find(|n| n.name() == "clickhouse3").and_then(|n| n.mem_pct)).flatten().unwrap();
    assert_eq!(filled, (node / 100.0 * 14.0).round() as usize, "the rows' cells add up to the node's (§5.3)");
}

#[test]
fn the_zoomed_nodes_and_node() {
    let mut app = app_after(30);
    app.update(key(KeyCode::Enter));
    let got = render(&app, 120, 36);
    let want = screen("120x36-zoom-nodes");
    same_frames(&got, &want, 1..=34, "zoom nodes");
    assert_eq!(trimmed(&got[1]), trimmed(&want[1]), "the zoom's title and summary");
    assert!(got[2].contains("╭ clickhouse7 ─── lag 12s ╮"), "{}", got[2]);
    assert!(got[4].contains("│ mem 93.3% 59.7/64 GiB") && got[6].contains("│ cpu "), "{}\n{}", got[4], got[6]);
    assert_eq!(trimmed(&got[35]).split("   ").take(6).collect::<Vec<_>>(), trimmed(&want[35]).split("   ").take(6).collect::<Vec<_>>(), "the zoom's hints");

    // → moves the chosen tile, ⏎ opens it.
    app.update(key(KeyCode::Right));
    assert_eq!(app.shell.chosen.as_deref(), Some("clickhouse-bi"));
    app.update(key(KeyCode::Left));
    app.update(key(KeyCode::Enter));
    assert_eq!(app.shell.zoom, Some(PanelId::Node));
    let got = render(&app, 120, 36);
    let want = screen("120x36-zoom-node");
    assert!(got[0].contains("dashboard › node › clickhouse3"), "{}", got[0]);
    // The rows, the divider, the two graphs and their labels; the graphs' heights are the screen's.
    // Row 32: the simulation's line (`… = 0.2× a 24 GiB query`) runs over the frame; this one is
    // clipped to it, and says only what §5 traces (lag, parts, free memory).
    same_frames(&got, &want, (1..=31).chain(33..=34), "zoom node");
    for (y, text) in [(2, "│ WHO HOLDS IT  memory, share of the node · the rows add up to the │MEMORY · 4 min"), (3, "│ grigol.gankava   "), (7, "│ server · caches… ")] {
        assert!(got[y].starts_with(text), "row {y}: {}", got[y]);
    }
    assert!(got[3].contains("25.4%  16.3G  3.10c│"), "pct · GiB · cores: {}", got[3]);
    assert_eq!(col(&got[15], "-4m"), col(&want[15], "-60m"), "the window printed where the screen prints its hour");
    assert_eq!(col(&got[17], "CPU · 4 min"), col(&want[17], "CPU · 60 min"));
    assert!(got[32].contains("│lag 0 s · 1.2k parts · free "), "{}", got[32]);
    assert!(got[9].contains(" · Redash #7438 · ") && got[11].contains("│ ▎ "), "the chosen row's query and its SQL:\n{}\n{}", got[9], got[11]);
    assert_eq!(trimmed(&got[35]).split("   ").take(6).collect::<Vec<_>>(), trimmed(&want[35]).split("   ").take(6).collect::<Vec<_>>());

    // ↓ to the server, its own words.
    for _ in 0..4 {
        app.update(key(KeyCode::Down));
    }
    let got = render(&app, 120, 36);
    assert!(got[9].contains("the server itself"), "{}", got[9]);
}

#[test]
fn explain_a_node_and_a_person() {
    let mut app = app_after(30);
    app.update(ch('e'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-explain-node");
    // The title says which of the subject's numbers this is, and that `e` steps to the next.
    assert_eq!(col(&got[12], "╭─ explain · 93.3% · 1 of 2 · e next ─"), col(&want[12], "╭─ explain · 93.1% ─"));
    // The screen writes the inputs `node.mem_used`; the provenance names them as its input rows do.
    assert!(got[13].contains("│ node.mem_pct = mem_used ÷ mem_total × 100   "), "the formula: {}", got[13]);
    assert!(want[13].contains("│ node.mem_pct = node.mem_used ÷ node.mem_total × 100"));
    assert!(got[14].contains("│              = 59.7 GiB ÷ 64.0 GiB × 100 = 93.3%"), "the formula with the values: {}", got[14]);
    assert!(got[16].contains("│ mem_used        59.7 GiB   system.asynchronous_metrics · MemoryResident"), "{}", got[16]);
    // The screen's popover lists the node's CPU under the memory inputs too; the provenance of a
    // share lists the inputs of its formula only, so this one is a row shorter.
    let bottom = got.iter().position(|r| r.contains("╰─ ⏎ ask Claude about it · y copy · esc ─")).unwrap();
    assert_eq!(bottom, 21);
    app.update(ch('y'));
    assert!(app.shell.explain.is_none() && app.take_clipboard().first().is_some_and(|t| t.contains("mem_used ÷ mem_total × 100")), "y copies it");

    app.update(ch('4'));
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Down));
    app.update(ch('e'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-explain-user");
    assert_eq!(col(&got[12], "╭─ explain · 12.8% · 1 of 4 · e next ─"), col(&want[12], "╭─ explain · 25.5% ─"));
    assert!(got[13].contains("│ user.mem_pct = user_memory ÷ mem_total × 100"), "{}", got[13]);
    let bottom = got.iter().position(|r| r.contains("╰─ ⏎ ask Claude about it · y copy · esc ─")).unwrap();
    assert_eq!(bottom, 22, "as tall as the screen's");
    // e again: the person's CPU, then their longest query's cores, then whether it is a runaway.
    for quantity in ["user.cpu_pct", "query.cores", "query.runaway", "user.mem_pct"] {
        app.update(ch('e'));
        assert!(render(&app, 120, 36)[13].contains(&format!("│ {quantity} = ")), "{quantity}");
    }
    app.update(key(KeyCode::Esc));
    assert!(app.shell.explain.is_none() && app.shell.zoom == Some(PanelId::Node), "esc closes the popover only");
}

#[test]
fn sort_and_filter_the_tiles() {
    let mut app = app_after(5);
    app.update(ch('s'));
    assert_eq!(app.tree.sort, crate::model::SortKey::Mem);
    assert!(render(&app, 120, 36)[8].contains("sorted by memory · s ─╮"));
    app.update(ch('/'));
    for c in "ch".chars() {
        app.update(ch(c));
    }
    let got = render(&app, 120, 36);
    assert!(got[35].contains("/ ch█ · esc clears"), "{}", got[35]);
    assert!(got[9].contains("╭ ch4 ") && !got[9].contains("clickhouse3"), "only the matching tiles: {}", got[9]);
    app.update(key(KeyCode::Enter));
    assert!(render(&app, 120, 36)[35].contains("/ ch · esc clears"));
    app.update(key(KeyCode::Esc));
    assert!(app.shell.filter.is_none() && render(&app, 120, 36)[9].contains("clickhouse"), "esc clears it");
}

#[test]
fn the_airflow_jira_and_tape_panels_on_the_dashboard() {
    let app = app_after(30);
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash");
    same_frames(&got, &want, 20..=34, "120x36 middle and bottom");
    // AIRFLOW: the day in numbers, the strip, the hours, then the rows in view 5's order.
    // The screen's line runs into the frame (`↻ 1││`); here the retries give way instead.
    assert!(cells(&got[21], 48, 86).starts_with("│ 217 runs  ✔ 211  ✖ 3  ▸ 2  ◌ 1 "), "{:?}", cells(&got[21], 48, 86));
    assert!(cells(&got[22], 48, 86).contains("▸━") && cells(&got[23], 48, 86).contains("now"), "{}\n{}", got[22], got[23]);
    for (y, start) in [(24, "│ ✖ test_clickhouse_con…"), (25, "│ ▸ statement_daily_agg…"), (26, "│ ◌ gateway_transfers_b…"), (27, "│ ✖ replication_app_con…")] {
        assert_eq!(cells(&got[y], 48, 86).split("  ").next(), cells(&want[y], 48, 86).split("  ").next(), "row {y}");
        assert!(cells(&got[y], 48, 86).starts_with(start));
    }
    // JIRA: the flow and what is overdue, the hours, the tickets with their due or age.
    assert!(cells(&got[21], 86, 120).contains(" › ") && cells(&got[21], 86, 120).contains("✖ 1 overdue │"), "{}", got[21]);
    assert!(cells(&got[23], 86, 120).starts_with("│ DATA-2207 ") && cells(&got[23], 86, 120).ends_with("11d │"), "{}", got[23]);
    assert!(cells(&got[25], 86, 120).contains("due tomorrow │") && cells(&got[27], 86, 120).contains("overdue 2d │"), "{}\n{}", got[25], got[27]);
    // TAPE: the two density rows beside the newest lines.
    assert!(got[30].starts_with("│ ✖ ▁") && got[31].starts_with("│ ▲ ▁"), "{}\n{}", got[30], got[31]);
    assert_eq!(col(&got[30], "▲ 15:5"), col(&want[30], "▲ 15:5"), "the lines start where the screen's do");
}

#[test]
fn the_zoomed_queue_and_the_cancel_dialog() {
    let mut app = app_after(30);
    app.update(ch('2'));
    app.update(key(KeyCode::Enter));
    let got = render(&app, 120, 36);
    let want = screen("120x36-zoom-queue");
    assert!(got[0].contains("dashboard › queue"));
    assert_eq!(trimmed(&got[1]), trimmed(&want[1]), "the zoom's title: the instance and its workers");
    same_frames(&got, &want, 1..=34, "zoom queue");
    let text = got.join("\n");
    // The cards, the sections, the stitch, the job's card.
    for part in ["╭ waiting ", "╭ queues ", "4/4 busy", "─ RUNNING · 6 on a worker ─", "→ clickhouse3", "16.3G  3.1c", "─ WAITING · ", "% of 3 min", "╭ #7438 Gateway transfers · grigol.gankava"] {
        assert!(text.contains(part), "{part}:\n{text}");
    }
    assert_eq!(trimmed(&got[35]).split("   ").take(4).collect::<Vec<_>>(), trimmed(&want[35]).split("   ").take(4).collect::<Vec<_>>());

    // x asks, n keeps, x then y sends.
    app.update(key(KeyCode::Down));
    app.update(ch('x'));
    let got = render(&app, 120, 36);
    let want = screen("120x36-cancel-dialog");
    let top = got.iter().position(|r| r.contains("╭─ Cancel Redash job ")).expect("the dialog");
    assert_eq!(Some(top), want.iter().position(|r| r.contains("╭─ Cancel Redash job ")));
    assert_eq!(col(&got[top], "╭─ Cancel"), col(&want[top], "╭─ Cancel"));
    assert!(got[top].contains("· j.petrova? ─"), "{}", got[top]);
    assert!(got[top + 2].contains("#8585 AML dashboard · clickhouse-bi · running "), "{}", got[top + 2]);
    assert!(got[top + 3].contains("Its query on clickhouse-bi keeps running"), "{}", got[top + 3]);
    assert!(got[top + 7].contains("[y] cancel the job     [esc] keep it"), "{}", got[top + 7]);
    app.update(ch('n'));
    assert!(app.cancel_asked().is_none() && app.take_cancels().is_empty(), "anything but y keeps it");
    app.update(ch('x'));
    app.update(ch('y'));
    assert_eq!(app.take_cancels().len(), 1, "y sends it");

    // ⏎ on a running job opens its node, with the person chosen.
    app.update(key(KeyCode::Up));
    app.update(key(KeyCode::Enter));
    assert_eq!(app.shell.zoom, Some(PanelId::Node));
    assert_eq!(app.shell.chosen.as_deref(), Some("clickhouse3"));
    assert!(app.shell.person.as_deref().is_some_and(|p| p.contains("grigol.gankava")), "{:?}", app.shell.person);
}

#[test]
fn the_zoomed_airflow_jira_and_tape_are_today_s_views() {
    let mut app = app_after(30);
    for (digit, name, parts) in [
        ('5', "120x36-zoom-airflow", vec!["NOW ·", "╭ ✖ test_clickhouse_connection", "THE DAY", "kyc_onboarding_tables", "╭ test_clickhouse_connection · manual"]),
        ('6', "120x36-zoom-jira", vec!["DATA-2207", "DATA-2647", "IN PROGRESS"]),
        ('7', "120x36-zoom-tape", vec!["TAPE", "runaway m.kairys on clickhouse-bi"]),
    ] {
        app.update(key(KeyCode::Esc));
        app.update(ch(digit));
        app.update(key(KeyCode::Enter));
        let got = render(&app, 120, 36);
        let want = screen(name);
        assert_eq!(got[0].split("    ").nth(1).map(str::trim), want[0].split("    ").nth(1).map(str::trim), "the path");
        assert_eq!(cells(&got[1], 0, 9), cells(&want[1], 0, 9), "{name}: the title");
        assert!(trimmed(&got[1]).ends_with("esc back ─╮"), "{}", got[1]);
        assert_eq!(frames(&got[2]).first(), Some(&(0, '│')));
        let text = got.join("\n");
        for part in parts {
            assert!(text.to_uppercase().contains(&part.to_uppercase()), "{name}: {part}\n{text}");
        }
        let hints: Vec<&str> = trimmed(&want[35]).split("   ").take(3).collect();
        assert_eq!(trimmed(&got[35]).split("   ").take(3).collect::<Vec<_>>(), hints, "{name}: the zoom's hints");
    }
    // In the jira zoom, ⏎ opens the ticket's page and esc closes it before the zoom.
    app.update(key(KeyCode::Esc));
    app.update(ch('6'));
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Enter));
    assert!(app.page().is_some() || app.jira_page.is_some(), "the ticket in full");
    app.update(key(KeyCode::Esc));
    assert_eq!(app.shell.zoom, Some(PanelId::Jira), "esc closed the page, not the zoom");
    assert!(app.jira_page.is_none());
}

#[test]
fn a_note_goes_on_the_tape() {
    let mut app = app_after(5);
    app.update(ch('7'));
    app.update(key(KeyCode::Enter));
    app.update(ch('n'));
    for c in "ch3 hot".chars() {
        app.update(ch(c));
    }
    let got = render(&app, 120, 36);
    let want = screen("120x36-note");
    assert!(got[35].starts_with(" note › ch3 hot█"), "{}", got[35]);
    assert!(trimmed(&got[35]).ends_with("⏎ keeps it on the tape · esc") && trimmed(&want[35]).ends_with("⏎ keeps it on the tape · esc"));
    app.update(key(KeyCode::Enter));
    assert!(app.shell.note.is_none());
    let newest: String = app.tape.newest_first().next().unwrap().parts.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(newest, "✎ ch3 hot");
}

/// Four sessions, one of each kind, none of them started (the tests run no program).
fn with_sessions(app: &mut App) {
    use crate::claude::Kind;
    for (kind, dir, name) in [(Kind::Claude, "~/work/billing", "billing export"), (Kind::OpenCode, "~/work/pipelines", "late partitions"), (Kind::Terminal, "~/work/reports", "make totals")] {
        app.claude.open_new(dir, kind);
        app.claude.rename_current(name);
    }
    app.claude.open_query(Some("clickhouse3"));
    app.claude.select(0);
}

/// A session's program up and having printed `bytes`, as `main.rs` leaves it.
fn run_pane(app: &mut App, index: usize, bytes: &[u8], working: bool) {
    let clock = app.clock;
    let session = &mut app.claude.list[index];
    session.pane.start((33, 116), 1000);
    session.pane.state = crate::claude::PaneState::Running;
    session.pane.feed(bytes, false);
    session.pane.last_output = if working { Some(clock) } else { clock.checked_sub(std::time::Duration::from_secs(60)) };
}

/// The sessions of `with_sessions`, their programs running: what a screenshot of the zoom shows.
fn with_running_sessions(app: &mut App) {
    with_sessions(app);
    app.claude.list[0].branch = Some("main".into());
    app.claude.list[1].branch = Some("fix/late-parts".into());
    app.claude.list[2].branch = Some("main".into());
    run_pane(app, 0, CLAUDE_DEMO, true);
    run_pane(app, 1, OPENCODE_DEMO, false);
    run_pane(app, 2, SHELL_DEMO, false);
    app.claude.list[1].pane.attention = true;
}

/// `n` sessions on seven projects, as the list + preview mockups have them; the chosen one's
/// program running.
pub(crate) fn with_many_sessions(app: &mut App, n: usize) {
    use crate::claude::Kind;
    const SET: [(&str, Kind, &str); 20] = [
        ("~/work/billing", Kind::Claude, "billing export"),
        ("~/work/pipelines", Kind::OpenCode, "late partitions"),
        ("~/work/reports", Kind::Terminal, "make totals"),
        ("", Kind::Query, "clickhouse3"),
        ("~/work/airflow-dags", Kind::OpenCode, "airflow-agent"),
        ("~/work/airflow-dags", Kind::Claude, "MetricsAtomic"),
        ("~/code/apitap-lib", Kind::OpenCode, "apitap-opencode"),
        ("~/work/airflow-dags", Kind::Claude, "airflow-dq"),
        ("~/code/apitap-lib", Kind::Claude, "apitap-lib-qa"),
        ("~/code/cobserve", Kind::Claude, "integrate-jira-api"),
        ("~/code/apitap-lib", Kind::Terminal, "shell"),
        ("~/work/billing", Kind::Claude, "invoice tests"),
        ("~/work/billing", Kind::Terminal, "shell"),
        ("~/work/pipelines", Kind::Claude, "backfill 03:00"),
        ("~/work/pipelines", Kind::Claude, "dq checks"),
        ("~/work/pipelines", Kind::Terminal, "shell"),
        ("~/work/airflow-dags", Kind::Claude, "docs-move"),
        ("", Kind::Query, "clickhouse-bi"),
        ("~/code/cobserve", Kind::Claude, "zoom redesign"),
        ("~/notes", Kind::Claude, "weekly notes"),
    ];
    for k in 0..n {
        let (dir, kind, name) = SET[k % SET.len()];
        let name = if k < SET.len() { name.to_string() } else { format!("{name} {}", k / SET.len() + 1) };
        if kind == Kind::Query {
            app.claude.open_query(Some(&name));
        } else {
            let dir = if k < SET.len() { dir.to_string() } else { format!("{dir}-{}", k / SET.len() + 1) };
            app.claude.open_new(&dir, kind);
            app.claude.rename_current(&name);
        }
    }
    for i in 0..app.claude.list.len() {
        if app.claude.list[i].dir.contains("airflow-dags") {
            app.claude.list[i].branch = Some("docs-move".into());
        }
    }
    let chosen = 7.min(n.saturating_sub(1));
    run_pane(app, chosen, CLAUDE_DEMO, true);
    if n > 1 {
        app.claude.list[1].pane.attention = true;
    }
    app.claude.select(chosen);
}

const OPENCODE_DEMO: &[u8] = b"\x1b[1m\xe2\x96\xa3 opencode\x1b[m \x1b[2m~/work/pipelines\x1b[m\r\n\r\n> find why the 03:00 partitions land late\r\n\r\nI checked the scheduler log: the merge of\r\n2026-10-12 waited on parts_to_delay_insert.\r\n\r\n\x1b[33mPermission needed to run:\x1b[m\r\n  clickhouse-client --query \"SELECT ...\"\r\n\r\n  [a]llow  [d]eny  [e]dit\r\n";

const SHELL_DEMO: &[u8] = b"\x1b[32m$\x1b[m make totals\r\npython3 totals.py --month 2026-10\r\nreading 31 files... done\r\nwriting totals-2026-10.csv (12,418 rows)\r\n\x1b[32m$\x1b[m wc -l totals-2026-10.csv\r\n12419 totals-2026-10.csv\r\n\x1b[32m$\x1b[m ";

#[test]
fn the_sessions_panel_and_zoom() {
    let mut app = app_after(5);
    with_sessions(&mut app);
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash");
    same_frames(&got, &want, 29..=34, "sessions panel");
    assert!(got[29].contains("╭ 8 SESSIONS ─") && trimmed(&got[29]).ends_with("4 · \\ floats one ─╮"), "{}", got[29]);
    for (y, glyph, number) in [(30, "✻", "7"), (31, "▣", "8"), (32, "❯", "9"), (33, "▦", "10")] {
        assert!(cells(&got[y], 74, 90).starts_with(&format!("{glyph} {number}")), "row {y}: {}", got[y]);
    }
    assert!(got[30].contains("billing export"), "{}", got[30]);

    app.update(ch('8'));
    app.update(key(KeyCode::Enter));
    let got = render(&app, 120, 36);
    let want = screen("120x36-zoom-sessions");
    assert!(got[0].contains("dashboard › sessions"));
    assert_eq!(cells(&got[1], 0, 17), cells(&want[1], 0, 17), "{}", got[1]);
    let hints: Vec<&str> = trimmed(&want[35]).split("   ").take(6).collect();
    assert_eq!(trimmed(&got[35]).split("   ").take(6).collect::<Vec<_>>(), hints, "the zoom's hints");
    assert!(got.join("\n").contains("billing export"), "today's list beside the screen");
    // ↓ chooses the next session; ⏎ opens it full; ctrl+\\ comes back to the list.
    app.update(key(KeyCode::Down));
    assert_eq!(app.claude.active, 1);
    app.update(key(KeyCode::Enter));
    assert!(app.shell.sessions_full);
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    assert!(!app.shell.sessions_full && app.shell.zoom == Some(PanelId::Sessions));
    app.update(key(KeyCode::Esc));
    assert_eq!(app.shell.zoom, None);
}

#[test]
fn a_session_floats_over_the_grid_with_its_prefix() {
    let mut app = app_after(5);
    with_sessions(&mut app);
    app.update(ch('\\'));
    assert!(app.shell.float.is_some());
    let got = render(&app, 120, 36);
    let want = screen("120x36-float-session");
    assert_eq!(col(&got[2], "╭─ 7 claude · ~/work/billing"), col(&want[2], "╭─ 7 claude · ~/work/billing"), "{}", got[2]);
    assert!(got[2].contains("8 ▣ opencode · 9 ❯ terminal · 10 ▦ query · + new"), "the others in the title: {}", got[2]);
    assert_eq!(col(&got[24], "╰─ ctrl+\\ hide · ctrl+\\ 1-6 a panel"), Some(12), "{}", got[24]);
    assert_eq!(frames(&got[10]).iter().filter(|(x, _)| *x == 12 || *x == 107).count(), 2, "the box's sides: {}", got[10]);

    // ctrl+\\: the prefix, then the which-key box once 250 ms have passed.
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    let got = render(&app, 120, 36);
    assert!(got[24].contains("ctrl+\\ · then: 1-6 a panel"), "{}", got[24]);
    assert!(!got.join("\n").contains("ctrl+\\ then ─"), "a fast hand never sees the box");
    let later = app.clock + std::time::Duration::from_millis(300);
    app.tick_at(later);
    let got = render(&app, 120, 36);
    let want = screen("120x36-float-session-prefix");
    assert_eq!(col(&got[12], "╭─ ctrl+\\ then ─"), col(&want[12], "╭─ ctrl+\\ then ─"), "{}", got[12]);
    // ↓ in the prefix: the next session.
    app.update(key(KeyCode::Down));
    assert_eq!(app.claude.active, 1);
    // ctrl+\\ twice hides the pane.
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    assert!(app.shell.float.is_none());
    // F-keys work without the prefix: F3 hides it and zooms NODES.
    app.update(ch('\\'));
    app.update(key(KeyCode::F(3)));
    assert!(app.shell.float.is_none() && app.shell.zoom == Some(PanelId::Nodes));
}

#[test]
fn claude_gets_the_row_or_the_number_in_its_prompt() {
    let mut app = app_after(5);
    app.update(ch('4'));
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Enter));
    assert!(app.shell.float.is_some(), "⏎ on a person floats Claude");
    let (_, prompt, _) = app.shell.pending_prompt.clone().expect("a prompt waits for the session");
    assert!(prompt.contains("on clickhouse3, grigol.gankava holds") && prompt.contains("What should I look at?"), "{prompt}");
    assert_eq!(app.claude.current().map(|s| s.kind), Some(crate::claude::Kind::Claude));
    // c: a SQL session on the chosen node, floating.
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    app.update(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    app.update(ch('c'));
    assert!(app.shell.float.is_some());
    assert_eq!(app.claude.current().and_then(|s| s.console.as_ref()).and_then(|c| c.node.as_deref()), Some("clickhouse3"));
}

fn click(col: u16, row: u16) -> Event {
    Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: col, row, modifiers: KeyModifiers::NONE })
}

#[test]
fn a_click_focuses_a_second_opens() {
    let mut app = app_after(5);
    render(&app, 120, 36);
    app.update(click(100, 25));
    assert_eq!(app.shell.focus, PanelId::Jira);
    app.update(click(100, 25));
    assert_eq!(app.shell.zoom, Some(PanelId::Jira));
    app.update(click(5, 0));
    assert_eq!(app.shell.zoom, None, "❖ cobserve goes to the grid");
    // A tile: the second one in the first row.
    app.update(click(25, 11));
    assert_eq!((app.shell.focus, app.shell.chosen.as_deref()), (PanelId::Nodes, Some("clickhouse-bi")));
}

#[test]
fn the_setup_wizard_asks_checks_and_skips() {
    let mut app = app_after(5);
    crate::ui2::wizard::open(&mut app, 0);
    let got = render(&app, 120, 36);
    let want = screen("120x36-wizard");
    let top = got.iter().position(|r| r.contains("╭─ setup · 1 of 5 · ClickHouse ─")).expect("the box");
    assert_eq!(Some(top), want.iter().position(|r| r.contains("╭─ setup · 1 of 5 · ClickHouse ─")));
    assert_eq!(col(&got[top], "╭─ setup"), col(&want[top], "╭─ setup"));
    for (dy, text) in [(2, "Where is ClickHouse?"), (3, "One or more HTTP addresses, comma-separated."), (6, "seeds     › "), (9, "password  › "), (10, "timeout   › ")] {
        assert_eq!(col(&got[top + dy], text), col(&want[top + dy], text), "row {dy}: {}", got[top + dy]);
    }
    assert!(got[top + 15].contains("⏎ next: Redash    tab next field    esc skip this step"), "{}", got[top + 15]);
    // Typing goes into the field; a secret is never shown.
    app.update(key(KeyCode::Tab));
    app.update(key(KeyCode::Tab));
    app.update(key(KeyCode::Tab));
    for c in "s3cret".chars() {
        app.update(ch(c));
    }
    let got = render(&app, 120, 36);
    assert!(got[top + 9].contains("password  › ••••••█") && !got.join("").contains("s3cret"), "{}", got[top + 9]);
    // ⏎ asks main for a live check; its answer is shown, and ⏎ again goes on.
    app.update(key(KeyCode::Enter));
    assert_eq!(app.shell.setup_asks.len(), 1);
    assert_eq!(app.shell.setup_asks[0].0, 0);
    app.update(Event::SetupChecked(0, Ok("ClickHouse 24.10 on node answered in 0.3 s".into())));
    assert!(render(&app, 120, 36).join("\n").contains("✔ ClickHouse 24.10 on node answered in 0.3 s · ⏎ goes on"));
    app.update(key(KeyCode::Enter));
    assert_eq!(app.shell.wizard.as_ref().map(|w| w.step), Some(1), "on to Redash");
    // esc skips a step; the setup closes without saving when nothing is finished here.
    app.update(key(KeyCode::Esc));
    assert_eq!(app.shell.wizard.as_ref().map(|w| w.step), Some(2));
    app.shell.wizard = None;
}

#[test]
fn the_tour_lights_a_row_and_advances() {
    let mut app = app_after(5);
    app.shell.tour = Some(0);
    let got = render(&app, 120, 36);
    let want = screen("120x36-tour");
    assert_eq!(col(&got[2], "╭─ tour · 1 of 6 ─"), col(&want[2], "╭─ tour · 1 of 6 ─"), "{}", got[2]);
    assert!(got[3].contains("One screen, eight panels, numbered 1–8."), "{}", got[3]);
    assert!(got[6].contains("╰─ any key advances · esc skips ─"), "{}", got[6]);
    app.update(ch('x'));
    assert_eq!(app.shell.tour, Some(1));
    let got = render(&app, 120, 36);
    assert!(got.iter().any(|r| r.contains("FLEET and REDASH")), "the second box");
    app.shell.tour = Some(5);
    app.shell.toasts.clear();
    app.shell.tour = None;
}

#[test]
fn replay_steps_back_and_comes_back_live() {
    let mut app = app_after(120);
    let live = render(&app, 120, 36);
    app.update(ch('['));
    app.update(ch('['));
    assert_eq!(app.shell.replay, 2);
    let got = render(&app, 120, 36);
    let want = screen("120x36-dash-replay");
    assert!(trimmed(&got[0]).ends_with("· replay −2m"), "{}", got[0]);
    assert!(want[0].contains("◷ 15:40 · replay −12m"));
    assert!(got[0].contains("◷ 15:50 · replay −2m"), "the minute's clock: {}", got[0]);
    assert_ne!(got[2..5], live[2..5], "the big number is the minute's");
    app.update(ch(']'));
    assert_eq!(app.shell.replay, 1);
    app.update(ch('t'));
    assert_eq!(app.shell.replay, 0);
    assert_eq!(render(&app, 120, 36)[0], live[0]);
    for _ in 0..10 {
        app.update(ch('['));
    }
    assert!(app.shell.replay <= 4, "no further back than the history reaches: {}", app.shell.replay);
}

#[test]
fn this_machine_zoomed_and_on_the_status_bar() {
    let mut app = app_after(5);
    app.update(Event::Local(Box::new(crate::fake::local(1, MOMENT as f64 - 2.0))));
    app.update(Event::Local(Box::new(crate::fake::local(2, MOMENT as f64))));
    let wide = render(&app, 160, 48);
    assert!(wide[47].contains("this Mac · cpu ") && wide[47].contains(" GiB   all · "), "{}", wide[47]);
    assert!(!render(&app, 120, 36)[35].contains("this Mac"), "not under 140 columns");
    app.update(ch('0'));
    assert!(app.shell.local);
    let got = render(&app, 120, 36);
    assert!(got[1].starts_with("╭ 0 LOCAL · "), "{}", got[1]);
    app.update(key(KeyCode::Esc));
    assert!(!app.shell.local);
}

#[test]
fn theme_and_prayer_from_the_palette() {
    let mut app = app_after(3);
    for name in ["theme", "prayer"] {
        let a = crate::action::ACTIONS.iter().find(|a| a.id == name).unwrap();
        (a.run)(&mut app, "");
    }
    assert!(app.shell.light.is_some());
    assert!(app.prayers.place.is_none() && !render(&app, 120, 36)[0].contains("━●┄"), "the day line is off");
    let a = crate::action::ACTIONS.iter().find(|a| a.id == "prayer").unwrap();
    (a.run)(&mut app, "");
    assert!(app.prayers.place.is_some());
}

/// `cargo test ui2::tests::dump -- --ignored --nocapture`: the screen at a size, to look at.
#[test]
#[ignore]
fn dump() {
    let (w, h) = std::env::var("SIZE").ok().and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((120, 36));
    let mut app = app_after(30);
    app.shell.last_size.set((w, h));
    match std::env::var("SESSIONS").ok().and_then(|n| n.parse::<usize>().ok()) {
        Some(4) => with_running_sessions(&mut app),
        Some(n) => with_many_sessions(&mut app, n),
        None => {}
    }
    for c in std::env::var("KEYS").unwrap_or_default().chars() {
        app.update(match c {
            '⏎' => key(KeyCode::Enter),
            '⎋' => key(KeyCode::Esc),
            '⇥' => key(KeyCode::Tab),
            '↓' => key(KeyCode::Down),
            '↑' => key(KeyCode::Up),
            '←' => key(KeyCode::Left),
            '→' => key(KeyCode::Right),
            c => ch(c),
        });
    }
    answer_asks(&mut app);
    println!("{}", render(&app, w, h).join("\n"));
}

/// What the screen asked to read — a run's tasks for the Gantt, a page — answered by the fake,
/// as the loop does.
fn answer_asks(app: &mut App) {
    let now = app.now();
    for ask in app.take_detail_asks() {
        let body = crate::fake::detail(&ask, now);
        app.update(Event::Detail(ask, body));
    }
}

#[test]
fn every_size_renders_without_a_panic() {
    let mut app = app_after(3);
    let mut seed: u64 = 0x2026_1013;
    let mut next = |m: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % m
    };
    for round in 0..500 {
        let w = 20 + next(231) as u16;
        let h = 8 + next(73) as u16;
        app.shell.zoom = match round % 10 {
            0..=1 => None,
            k => Some(PanelId::ALL[k - 2]),
        };
        app.shell.preset = [Preset::All, Preset::Fleet, Preset::Ops][round % 3];
        app.shell.density = [Density::Comfortable, Density::Compact, Density::Focus][(round / 3) % 3];
        app.shell.palette = (round % 7 == 0).then(Default::default);
        app.shell.explain = None;
        if round % 11 == 0 {
            nav::explain_now(&mut app);
        }
        app.shell.filter = (round % 13 == 0).then(|| crate::ui2::state::Filter { query: "ch".into(), typing: true });
        app.shell.note = (round % 17 == 0).then(|| "a note".to_string());
        app.shell.float = (round % 19 == 0).then(|| crate::ui2::state::Float { full: round % 2 == 0, ..Default::default() });
        app.shell.wizard = (round % 23 == 0).then(|| crate::ui2::wizard::Wizard::open(round % 5));
        app.shell.tour = (round % 29 == 0).then_some(round % 6);
        app.shell.local = round % 31 == 0;
        app.shell.replay = (round % 37 == 0) as u8;
        views::sync(&mut app);
        let rows = render(&app, w, h);
        assert_eq!(rows.len(), h as usize);
    }
    let rows = render(&app, 39, 12);
    assert!(rows[0].starts_with("cobserve · too small"), "{}", rows[0]);
}

// ---------------------------------------------------------------------------
// Screenshots: `SHOT_DIR=docs/screenshots cargo test ui2::tests::export_screens -- --ignored`
// writes the screens of docs/redesign from the fake fleet as text and as colour HTML (what
// dev/screenshots.sh turns into PNGs). Ignored by default: it writes files.
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

fn html_with(buf: &ratatui::buffer::Buffer, title: &str, theme: &crate::theme::Theme) -> String {
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


/// What a session in the floating pane looks like in the screenshot, written by hand — the
/// real one is whatever Claude Code draws.
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

fn shot_buffer(app: &App, w: u16, h: u16, theme: &crate::theme::Theme) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|frame| draw_with(frame, app, theme)).expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
#[ignore]
fn export_screens() {
    let Ok(dir) = std::env::var("SHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&dir).expect("shot dir");
    let dark = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Dark);
    let light = crate::theme::Theme::new(crate::theme::Depth::TrueColor, crate::theme::Variant::Light);
    let write = |name: &str, app: &App, w: u16, h: u16, theme: &crate::theme::Theme| {
        let buf = shot_buffer(app, w, h, theme);
        let text: Vec<String> = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect();
        std::fs::write(format!("{dir}/{name}.txt"), text.join("\n") + "\n").expect("text");
        std::fs::write(format!("{dir}/{name}.html"), html_with(&buf, name, theme)).expect("html");
    };
    let fresh = || {
        let mut app = app_after(120);
        app.caps.graph = crate::caps::GraphTier::Braille;
        app.update(Event::Local(Box::new(crate::fake::local(1, MOMENT as f64 - 2.0))));
        app.update(Event::Local(Box::new(crate::fake::local(2, MOMENT as f64))));
        app
    };
    for (w, h) in [(120, 36), (80, 24), (160, 48)] {
        write(&format!("{w}x{h}-dash"), &fresh(), w, h, &dark);
    }
    for panel in PanelId::ALL {
        let mut app = fresh();
        if panel == PanelId::Sessions {
            with_running_sessions(&mut app);
        }
        app.update(ch(char::from(b'0' + panel.number())));
        app.update(key(KeyCode::Enter));
        answer_asks(&mut app);
        write(&format!("120x36-zoom-{}", panel.slug()), &app, 120, 36, &dark);
    }
    let mut app = fresh();
    with_many_sessions(&mut app, 20);
    app.update(ch('8'));
    app.update(key(KeyCode::Enter));
    write("120x36-zoom-sessions-list", &app, 120, 36, &dark);
    let mut app = fresh();
    app.update(ch(':'));
    write("120x36-palette", &app, 120, 36, &dark);
    let mut app = fresh();
    app.update(ch('e'));
    write("120x36-explain-node", &app, 120, 36, &dark);
    let mut app = fresh();
    app.update(ch('2'));
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Down));
    app.update(ch('x'));
    write("120x36-cancel-dialog", &app, 120, 36, &dark);
    let mut app = fresh();
    with_sessions(&mut app);
    app.update(ch('\\'));
    if let Some(session) = app.claude.current_mut() {
        session.branch = Some("main".into());
        session.pane.start((23, 92), 1000);
        session.pane.state = crate::claude::PaneState::Running;
        session.pane.feed(CLAUDE_DEMO, true);
    }
    write("120x36-float-session", &app, 120, 36, &dark);
    let mut app = fresh();
    crate::ui2::wizard::open(&mut app, 0);
    write("120x36-wizard", &app, 120, 36, &dark);
    let mut app = fresh();
    app.shell.tour = Some(0);
    write("120x36-tour", &app, 120, 36, &dark);
    let mut app = fresh();
    app.update(ch('0'));
    write("120x36-local", &app, 120, 36, &dark);
    write("120x36-light", &fresh(), 120, 36, &light);
}

/// The four redrawn zooms at sizes from tiny to huge, the cursor first, in the middle and last,
/// the sessions with none, a grid's worth, a list's and the most there can be: never a panic.
#[test]
fn the_zooms_render_at_every_size_without_a_panic() {
    let mut seed: u64 = 0x2026_1015;
    let mut next = |m: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % m
    };
    for sessions in [0usize, 4, 20, 50] {
        let mut app = app_after(30);
        if sessions == 4 {
            with_running_sessions(&mut app);
        } else if sessions > 0 {
            with_many_sessions(&mut app, sessions);
        }
        for digit in ['2', '5', '6', '8'] {
            app.update(key(KeyCode::Esc));
            app.update(key(KeyCode::Esc));
            app.update(ch(digit));
            app.update(key(KeyCode::Enter));
            answer_asks(&mut app);
            for round in 0..40 {
                let w = 20 + next(231) as u16;
                let h = 8 + next(73) as u16;
                let moves = match round % 3 {
                    0 => 0,
                    1 => 6,
                    _ => 60,
                };
                for _ in 0..moves {
                    app.update(key(KeyCode::Down));
                }
                let screen = render(&app, w, h);
                assert_eq!(screen.len(), h as usize, "zoom {digit}, {sessions} sessions, {w}×{h}");
                if round % 3 == 2 {
                    app.update(key(KeyCode::Home));
                }
            }
        }
    }
}
