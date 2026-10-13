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
        let rows = render(&app, w, h);
        assert_eq!(rows.len(), h as usize);
    }
    let rows = render(&app, 39, 12);
    assert!(rows[0].starts_with("cobserve · too small"), "{}", rows[0]);
}
