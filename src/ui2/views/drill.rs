//! A browser opened from this machine (`⏎` on its row in view 0): its parts and its tabs over
//! one list of its tab and extension processes and its tabs by site, and a card for the row under
//! the cursor (the Chrome spec, `120x36-zoom-chrome`). Everything drawn here was worked out when a
//! read came in — `Local::browser`, `Drill::rows`, `TabsRead::sites` — so a frame only places it.

use super::kit::{self, Card};
use super::widgets::Cells;
use crate::app::App;
use crate::browser::{self, Drill, Row, TabsRead};
use crate::fmt;
use crate::local::{Breakdown, Helper, Part};
use crate::severity::Severity;
use crate::theme::Theme;
use crate::ui2::paint::{lp, rp, Painter};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Span;

/// The paragraph that says what can and cannot be known of tabs and processes.
const HONEST: &str = "a site's tabs share one; iframes, workers and prerendered pages get their own. Which tab is in which process only Chrome's Task Manager says (⇧esc).";

pub fn draw(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, drill: &Drill) {
    let Some(b) = app.local.browser(&drill.program) else {
        kit::message(p, theme, i, ("◌", theme.muted()), &format!("{} is not running", drill.program), &["its processes have gone since the last read", "esc back"]);
        return;
    };
    let total = app.local.latest.as_ref().and_then(|s| s.memory).map(|m| m.total);
    let tabs = app.tabs.as_ref().filter(|t| t.program == drill.program);
    let wide = i.width >= 100 && i.height >= 28;
    let mut y = i.y;
    if wide {
        let h = 8;
        let left_w = 64.min(i.width - 40);
        hero_parts(p, theme, Rect::new(i.x, y, left_w, h), b, total);
        hero_tabs(p, app, theme, Rect::new(i.x + left_w, y, i.width - left_w, h), b, tabs);
        y += h;
    } else {
        y = plain_lines(p, app, theme, i, b, total, tabs);
    }
    let card_h = match i.height {
        30.. => 6,
        20.. => 3,
        _ => 0,
    };
    let list = Rect::new(i.x, y, i.width, (i.bottom() - card_h).saturating_sub(y));
    draw_list(p, app, theme, list, drill, b, total, tabs);
    if card_h > 0 {
        detail(p, app, theme, Rect::new(i.x, i.bottom() - card_h, i.width, card_h), drill, b, total, tabs);
    }
}

/// `of 32 GiB`: the Mac's memory, the denominator of every share here.
fn of_mac(total: Option<u64>) -> String {
    total.map_or("of RAM".into(), |t| format!("of {:.0} GiB", fmt::gib(t)))
}

fn pct_of(part: u64, whole: Option<u64>) -> String {
    whole.filter(|w| *w > 0).map_or("—".into(), |w| fmt::pct(Some(part as f64 / w as f64 * 100.0)))
}

/// The parts the card shows: the utility helpers folded into one row where there is no room.
fn folded(b: &Breakdown) -> Vec<(&'static str, usize, u64)> {
    let mut rows: Vec<(&'static str, usize, u64)> = Vec::new();
    let mut utility = (0usize, 0u64);
    for s in &b.parts {
        match s.part {
            Part::Network | Part::Storage | Part::Audio | Part::Other => {
                utility.0 += s.count;
                utility.1 += s.rss;
            }
            part => rows.push((browser::part_word(part), s.count, s.rss)),
        }
    }
    if utility.0 > 0 {
        rows.push(("utility", utility.0, utility.1));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));
    rows
}

fn hero_parts(p: &mut Painter<'_>, theme: &Theme, r: Rect, b: &Breakdown, total: Option<u64>) {
    let inner = Card::new(theme.border()).title(vec![Span::styled("parts", theme.strong())]).right(vec![Span::styled("the rows add up to Chrome", theme.faint())]).draw(p, r);
    if inner.height == 0 {
        return;
    }
    let mac = of_mac(total);
    let head = format!("{}{}{}  {}{}{}", rp("PART", 11), lp("PROCS", 5), lp("RSS", 10), rp("", 10), lp("of Chrome", 10), lp(&mac, 10));
    p.put(inner.x, inner.y, &head, theme.table_head());
    let rows = folded(b);
    let shares: Vec<f64> = rows.iter().map(|r| if b.rss > 0 { r.2 as f64 / b.rss as f64 } else { 0.0 }).collect();
    let cells = crate::ui2::glyph::bar::largest_remainder(&shares, 10);
    for (k, ((word, count, rss), fill)) in rows.iter().zip(cells).enumerate().take(inner.height.saturating_sub(1) as usize) {
        let y = inner.y + 1 + k as u16;
        let mut c = Cells::new();
        c.push(rp(word, 11), theme.text());
        c.push(lp(&count.to_string(), 5), theme.text2());
        c.push(lp(&fmt::bytes(*rss), 10), theme.strong());
        c.push("  ", theme.text());
        c.push("█".repeat(fill), ratatui::style::Style::default().fg(theme.bar_fill(Severity::None)));
        c.push("░".repeat(10 - fill), theme.faint());
        c.push(lp(&pct_of(*rss, Some(b.rss)), 10), theme.text2());
        c.push(lp(&pct_of(*rss, total), 10), theme.muted());
        kit::put_cells(p, inner.x, y, c, inner.width as usize);
    }
}

fn hero_tabs(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, b: &Breakdown, tabs: Option<&TabsRead>) {
    let age = tabs.map(|t| std::time::Duration::from_secs_f64((app.now() as f64 - t.at).max(0.0)));
    let right = kit::freshness(age, None, app.tabs_reading(), theme, 24);
    let inner = Card::new(theme.border()).title(vec![Span::styled("tabs", theme.strong())]).right(right.into_spans()).draw(p, r);
    if inner.height == 0 {
        return;
    }
    let w = inner.width as usize;
    let mut lines: Vec<(String, ratatui::style::Style)> = Vec::new();
    let tab_procs = b.parts.iter().find(|s| s.part == Part::Tab).map_or(0, |s| s.count);
    match tabs.map(|t| &t.result) {
        None => {
            lines.push(("◌ asking Chrome…".into(), theme.accent()));
            lines.push(("macOS may ask whether cobserve may control".into(), theme.muted()));
            lines.push((format!("{} — answer OK (up to 2 minutes)", b.program), theme.muted()));
        }
        Some(Err(e)) => {
            lines.push((format!("✖ {e}"), theme.sev(Severity::Warn)));
            lines.push(("r asks again".into(), theme.faint()));
        }
        Some(Ok(list)) if list.is_empty() => lines.push((format!("0 tabs · 0 windows — {} is open with no window", browser::short(&b.program)), theme.muted())),
        Some(Ok(list)) => {
            let read = tabs.expect("matched");
            lines.push((format!("{} tabs · {} windows · {} sites", list.len(), read.windows(), read.sites.len()), theme.strong()));
            if let Some(tab) = read.on_screen() {
                lines.push((format!("● open now: {} · {}", tab.title, crate::sources::chrome::site(&tab.url)), theme.text()));
            }
            let text = format!("{tab_procs} tab processes for {} tabs: {HONEST}", list.len());
            for line in wrap(&text, w) {
                lines.push((line, theme.muted()));
            }
        }
    }
    let room = inner.height as usize;
    let last = (room.saturating_sub(1), "r reads the tabs again · every 30 s while open");
    for (k, (line, style)) in lines.iter().enumerate().take(room.saturating_sub(1)) {
        kit::put_spans(p, inner.x, inner.y + k as u16, vec![Span::styled(line.clone(), *style)], w);
    }
    kit::put_spans(p, inner.x, inner.y + last.0 as u16, vec![Span::styled(last.1, theme.faint())], w);
}

/// Words to lines of `w` cells.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && fmt::width(&line) + 1 + fmt::width(word) > w {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// Where there is no room for the cards: their facts in three lines.
fn plain_lines(p: &mut Painter<'_>, app: &App, theme: &Theme, i: Rect, b: &Breakdown, total: Option<u64>, tabs: Option<&TabsRead>) -> u16 {
    let w = i.width as usize;
    let share = browser::share(&app.local, b);
    let sev = browser::share_severity(share, app.chrome_lines);
    let cores = app.local.latest.as_ref().map_or(0, |s| s.cores);
    let mut first = Cells::new();
    first.push(format!(" {} RSS", fmt::bytes(b.rss)), theme.strong());
    first.push(format!(" = {} {}", fmt::pct(share), of_mac(total)), sev_style(theme, sev));
    first.push(format!(" · {} processes · {:.1} of {cores} cores", b.count, b.cores), theme.muted());
    kit::put_cells(p, i.x, i.y, first, w);
    let mut second = Cells::new();
    second.push(" ", theme.text());
    for (k, (word, count, rss)) in folded(b).into_iter().enumerate() {
        if k > 0 {
            second.push(" · ", theme.faint());
        }
        second.push(format!("{word} {count} "), theme.text2());
        second.push(fmt::bytes(rss), theme.text());
    }
    kit::put_cells(p, i.x, i.y + 1, second, w);
    let third = match tabs.map(|t| (&t.result, t)) {
        Some((Ok(list), read)) => format!(" {} tabs · {} windows · {} sites · tab↔process: ⇧esc in Chrome", list.len(), read.windows(), read.sites.len()),
        Some((Err(e), _)) => format!(" ✖ {e}"),
        None => " ◌ asking Chrome for its tabs…".into(),
    };
    kit::put_spans(p, i.x, i.y + 2, vec![Span::styled(third, theme.muted())], w);
    i.y + 3
}

fn sev_style(theme: &Theme, sev: Severity) -> ratatui::style::Style {
    if sev.is_problem() { theme.sev(sev).add_modifier(Modifier::BOLD) } else { theme.text() }
}

/// A line of the list as drawn: a section's rule or header, a row of `Drill::rows`, a gap.
enum Line {
    Rule,
    Header,
    Row(usize),
    Gap,
}

#[allow(clippy::too_many_arguments)]
fn draw_list(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, drill: &Drill, b: &Breakdown, total: Option<u64>, tabs: Option<&TabsRead>) {
    if r.height < 2 {
        return;
    }
    let procs = drill.processes();
    let mut lines: Vec<Line> = vec![Line::Rule, Line::Header];
    lines.extend((0..procs).map(Line::Row));
    let has_tabs = drill.rows.len() > procs;
    if has_tabs && r.height >= 14 {
        lines.push(Line::Gap);
        lines.push(Line::Rule);
        lines.extend((procs..drill.rows.len()).map(Line::Row));
    }
    let h = r.height as usize;
    let at = lines.iter().position(|l| matches!(l, Line::Row(k) if *k == drill.selected)).unwrap_or(0);
    let first = at.saturating_sub(h.saturating_sub(1)).min(lines.len().saturating_sub(h));
    let shown = &lines[first..lines.len().min(first + h)];
    let procs_shown = shown.iter().filter(|l| matches!(l, Line::Row(k) if *k < procs)).count();
    let sites_shown = shown.iter().filter(|l| matches!(l, Line::Row(k) if *k >= procs)).count();
    let mut section = 0;
    let w = r.width as usize;
    let cols = Cols::of(r.width);
    for (k, line) in lines.iter().enumerate() {
        if matches!(line, Line::Rule) {
            section += 1;
        }
        if k < first || k >= first + h {
            continue;
        }
        let y = r.y + (k - first) as u16;
        match line {
            Line::Rule if section == 1 => {
                let more = procs.saturating_sub(procs_shown);
                let mut title = vec![Span::styled("TAB AND EXTENSION PROCESSES", theme.section()), Span::styled(format!(" · {procs} · heaviest first"), theme.muted())];
                if more > 0 {
                    title.push(Span::styled(format!(" · {procs_shown} shown · ↓ {more} more"), theme.faint()));
                }
                kit::rule(p, theme, r.x, y, r.width, title);
            }
            Line::Rule => {
                let read = tabs.expect("tab rows come from a read");
                let sites = read.sites.len();
                let rows = drill.rows.len() - procs;
                let more = rows.saturating_sub(sites_shown);
                let mut title = vec![
                    Span::styled("TABS", theme.section()),
                    Span::styled(format!(" · {} in {} windows · by site · ⏎ opens a site", read.tabs().len(), read.windows()), theme.muted()),
                ];
                if more > 0 {
                    title.push(Span::styled(format!(" · {sites_shown} of {sites} shown · ↓ {more} more"), theme.faint()));
                }
                kit::rule(p, theme, r.x, y, r.width, title);
            }
            Line::Header => {
                p.put(r.x, y, &cols.header(total), theme.table_head());
            }
            Line::Gap => {}
            Line::Row(index) => {
                let selected = *index == drill.selected;
                if selected {
                    kit::band(p, theme, r, y);
                }
                match &drill.rows[*index] {
                    Row::Process(h) => process_row(p, app, theme, r.x, y, w, &cols, h, b, total),
                    Row::Site { host, count } => site_row(p, theme, r.x, y, w, host, *count, tabs, drill.open.contains(host)),
                    Row::Tab(i) => {
                        if let Some(tab) = tabs.and_then(|t| t.tabs().get(*i)) {
                            let mut c = Cells::new();
                            c.push(format!("{}{} w{}  ", " ".repeat(8), if tab.active { "●" } else { " " }, tab.window), if tab.active { theme.text() } else { theme.faint() });
                            c.push(tab.title.clone(), if tab.active { theme.strong() } else { theme.text() });
                            if tab.active {
                                c.push(if tab.window == 1 { "  · open now" } else { "  · shown in its window" }, theme.faint());
                            }
                            kit::put_cells(p, r.x, y, c, w);
                        }
                    }
                }
                if selected {
                    p.put(r.x, y, "▌", theme.accent());
                }
            }
        }
    }
}

/// The process rows' columns, by what fits.
struct Cols {
    cpu_time: bool,
    of_mac: bool,
    age: bool,
}

impl Cols {
    fn of(width: u16) -> Cols {
        Cols { cpu_time: width >= 100, of_mac: width >= 90, age: width >= 70 }
    }

    fn header(&self, total: Option<u64>) -> String {
        let mut s = format!("    {}  {}{}{}", lp("PID", 7), rp("PART", 10), lp("RSS", 10), lp("of Chrome", 11));
        if self.of_mac {
            s.push_str(&lp(&of_mac(total), 12));
        }
        s.push_str(&lp("CORES", 8));
        if self.cpu_time {
            s.push_str(&lp("CPU TIME", 10));
        }
        if self.age {
            s.push_str(&lp("AGE", 9));
        }
        s
    }
}

#[allow(clippy::too_many_arguments)]
fn process_row(p: &mut Painter<'_>, app: &App, theme: &Theme, x: u16, y: u16, w: usize, cols: &Cols, h: &Helper, b: &Breakdown, total: Option<u64>) {
    let mut c = Cells::new();
    c.push("  ", theme.text());
    let running = h.cores >= 0.10;
    c.push(if running { format!("{} ", kit::spinner(app)) } else { "  ".into() }, theme.text());
    c.push(lp(&h.pid.to_string(), 7), theme.text2());
    c.push("  ", theme.text());
    c.push(rp(browser::helper_word(h.part), 10), theme.text());
    c.push(lp(&fmt::bytes(h.rss), 10), theme.strong());
    c.push(lp(&pct_of(h.rss, Some(b.rss)), 11), theme.text2());
    if cols.of_mac {
        c.push(lp(&pct_of(h.rss, total), 12), theme.muted());
    }
    c.push(lp(&format!("{:.2}", h.cores), 8), if running { theme.text() } else { theme.muted() });
    if cols.cpu_time {
        c.push(lp(&fmt::dur(h.cpu_time_s), 10), theme.muted());
    }
    if cols.age {
        let age = h.started.map_or("—".into(), |s| fmt::dur(app.now() as f64 - s));
        c.push(lp(&age, 9), theme.muted());
    }
    kit::put_cells(p, x, y, c, w);
}

#[allow(clippy::too_many_arguments)]
fn site_row(p: &mut Painter<'_>, theme: &Theme, x: u16, y: u16, w: usize, host: &str, count: usize, tabs: Option<&TabsRead>, open: bool) {
    let mut c = Cells::new();
    c.push(lp(&count.to_string(), 8), theme.text2());
    let shown = tabs.and_then(|t| t.sites.iter().find(|s| s.host == host).map(|s| s.tabs.iter().any(|&i| t.tabs().get(i).is_some_and(|tab| tab.active)))).unwrap_or(false);
    c.push(if open { " ▾" } else { "  " }, theme.faint());
    c.push(if shown { "● " } else { "  " }, theme.text());
    c.push(rp(host, 21), if shown { theme.strong() } else { theme.text2() });
    let titles: Vec<String> = tabs
        .and_then(|t| t.sites.iter().find(|s| s.host == host).map(|s| s.tabs.iter().filter_map(|&i| t.tabs().get(i).map(|tab| tab.title.clone())).collect()))
        .unwrap_or_default();
    // The titles cut at the row's end, and how many were not named always said.
    let room = w.saturating_sub(c.width() + 1);
    let mut named = String::new();
    let mut n = 0;
    for title in &titles {
        let left = titles.len() - n - 1;
        let tail = if left > 0 { format!(" · +{left}") } else { String::new() };
        let next = if named.is_empty() { title.clone() } else { format!("{named} · {title}") };
        if fmt::width(&next) + fmt::width(&tail) > room {
            break;
        }
        named = next;
        n += 1;
    }
    if n < titles.len() {
        named.push_str(&format!("{}+{}", if named.is_empty() { "" } else { " · " }, titles.len() - n));
    }
    c.push(" ", theme.text());
    c.push(named, theme.muted());
    kit::put_cells(p, x, y, c, w);
}

#[allow(clippy::too_many_arguments)]
fn detail(p: &mut Painter<'_>, app: &App, theme: &Theme, r: Rect, drill: &Drill, b: &Breakdown, total: Option<u64>, tabs: Option<&TabsRead>) {
    let Some(row) = drill.current() else {
        Card::new(theme.border()).title(vec![Span::styled("nothing to choose", theme.muted())]).draw(p, r);
        return;
    };
    let short = browser::short(&b.program);
    let (title, right, lines): (String, String, Vec<(String, ratatui::style::Style)>) = match row {
        Row::Process(h) => {
            let word = browser::helper_word(h.part);
            let age = h.started.map_or("age unknown".into(), |s| format!("started {} ago", fmt::dur(app.now() as f64 - s)));
            let numbers = format!(
                "{} RSS · {} of {short}'s {} · {} {} · {:.2} cores · CPU time {} · {age}",
                fmt::bytes(h.rss),
                pct_of(h.rss, Some(b.rss)),
                fmt::bytes(b.rss),
                pct_of(h.rss, total),
                of_mac(total),
                h.cores,
                fmt::dur(h.cpu_time_s)
            );
            let exe = h.command.rsplit('/').next().unwrap_or(&h.command).to_string();
            let what = if h.part == Part::Extension {
                format!("an extension's process · {exe} · parent {}", h.ppid)
            } else {
                format!("a renderer: the tabs of one site, or an iframe, a worker or a page being prerendered — which, only {short}'s Task Manager says (⇧esc) · {exe} · parent {}", h.ppid)
            };
            let does = if h.part == Part::Extension {
                "x stops it: the extension stops until Chrome starts it again (it does when it is next used); its memory is freed".to_string()
            } else {
                "x stops it: its tabs show Aw, Snap! — reload one to get it back; its memory is freed · never the browser, GPU".to_string()
            };
            (format!("pid {} · {word} process · {}", h.pid, fmt::bytes(h.rss)), "x stops it · esc back".into(), vec![(numbers, theme.text()), (what, theme.muted()), (does, theme.text2())])
        }
        Row::Site { host, count } => {
            let read = tabs.expect("a site comes from a read");
            let site = read.sites.iter().find(|s| &s.host == host);
            let mut lines: Vec<(String, ratatui::style::Style)> = Vec::new();
            let ids: Vec<usize> = site.map(|s| s.tabs.clone()).unwrap_or_default();
            for &i in ids.iter().take(3) {
                if let Some(tab) = read.tabs().get(i) {
                    lines.push((format!("{} w{}  {}", if tab.active { "●" } else { " " }, tab.window, tab.title), theme.text()));
                }
            }
            if ids.len() > 3 {
                lines.push((format!("… and {} more", ids.len() - 3), theme.faint()));
            }
            (format!("{host} · {count} tabs"), if drill.open.contains(host) { "⏎ folds it".into() } else { "⏎ opens it".into() }, lines)
        }
        Row::Tab(i) => {
            let tab = tabs.and_then(|t| t.tabs().get(*i));
            match tab {
                Some(tab) => (format!("w{} · {}", tab.window, tab.title), crate::sources::chrome::site(&tab.url), vec![(tab.url.clone(), theme.muted())]),
                None => ("a tab".into(), String::new(), Vec::new()),
            }
        }
    };
    let inner = Card::new(theme.border()).title(vec![Span::styled(title, theme.strong())]).right(vec![Span::styled(right, theme.faint())]).draw(p, r);
    let w = inner.width as usize;
    let mut y = inner.y;
    for (text, style) in lines {
        for line in wrap(&text, w) {
            if y >= inner.bottom() {
                return;
            }
            kit::put_spans(p, inner.x, y, vec![Span::styled(line, style)], w);
            y += 1;
        }
    }
}
