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

/// The fake fleet after `polls` polls, the new shell on, the clock at the simulation's instant in
/// Jakarta, and block graphs — the tier the screens were drawn in.
fn app_after(polls: usize) -> App {
    let mut fake = FakeSource::new();
    let mut app = App::new();
    app.shell.on = true;
    app.caps.graph = GraphTier::Block;
    for i in 0..polls.max(1) {
        app.update(Event::Snapshot(Box::new(fake.snapshot())));
        if i % 3 == 0 {
            app.update(Event::Queue(Box::new(fake.queue())));
        }
    }
    app.update(Event::Queue(Box::new(fake.queue())));
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
    assert!(got[1].starts_with("╭ 2 REDASH QUEUE ─") && got[34].starts_with("╰"), "one frame over the body: {}", got[1]);
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
    assert_eq!(col(&got[12], "╭─ explain · 93.3% ─"), col(&want[12], "╭─ explain · 93.1% ─"));
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
    assert_eq!(col(&got[12], "╭─ explain · 12.8% ─"), col(&want[12], "╭─ explain · 25.5% ─"));
    assert!(got[13].contains("│ user.mem_pct = user_memory ÷ mem_total × 100"), "{}", got[13]);
    let bottom = got.iter().position(|r| r.contains("╰─ ⏎ ask Claude about it · y copy · esc ─")).unwrap();
    assert_eq!(bottom, 22, "as tall as the screen's");
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

/// `cargo test ui2::tests::dump -- --ignored --nocapture`: the screen at a size, to look at.
#[test]
#[ignore]
fn dump() {
    let (w, h) = std::env::var("SIZE").ok().and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((120, 36));
    let mut app = app_after(30);
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
    println!("{}", render(&app, w, h).join("\n"));
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
        let rows = render(&app, w, h);
        assert_eq!(rows.len(), h as usize);
    }
    let rows = render(&app, 39, 12);
    assert!(rows[0].starts_with("cobserve · too small"), "{}", rows[0]);
}
