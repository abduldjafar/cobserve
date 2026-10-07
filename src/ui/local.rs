//! View 0: this machine, as Activity Monitor shows it — and the band's LOCAL line.
//!
//! The machine first: its name, cores, memory and uptime; then its CPU and its memory as bars
//! with their denominators, the pressure the kernel reports and the swap; then every process
//! (or every program, `g`) by CPU or by memory (`s`), each with its cores of the machine's and its
//! resident memory of the machine's, closed by `the rest` — the CPU no row accounts for, so the
//! rows add up to the bar above them. The drawer says the rest of the row under the cursor.

use super::widgets::{rule, sparkline, thin_bar, Cells, PCT_SHAPE};
use crate::app::{scroll_into_view, App, Hit};
use crate::fmt;
use crate::local::{Local, Row, Sort};
use crate::severity::{self, Severity};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const GAP: usize = 2;

/// The colour of the memory bar: the pressure's, and none while it is normal — a full Mac is
/// a well one until the kernel says otherwise.
fn memory_severity(local: &Local) -> Severity {
    match local.latest.as_ref().and_then(|s| s.pressure).map(|p| p.severity()) {
        Some(Severity::Ok) | None => Severity::None,
        Some(sev) => sev,
    }
}

fn gib(bytes: u64) -> String {
    let g = fmt::gib(bytes);
    if g >= 100.0 { format!("{g:.0}") } else { format!("{g:.1}") }
}

/// `2.2/10 cores`
fn cores_of(busy: f64, cores: u32) -> String {
    format!("{busy:.1}/{cores} cores")
}

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let local = &app.local;
    let width = area.width as usize;
    let height = area.height as usize;
    let Some(sample) = local.latest.as_ref() else {
        let line = Line::from(Span::styled("  reading this machine…", theme.muted()));
        frame.render_widget(Paragraph::new(line), area);
        return;
    };
    if let (Some(why), false) = (&sample.error, local.shown()) {
        let line = Line::from(Span::styled(format!("  this machine could not be read: {why}"), theme.sev(Severity::Warn)));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mut lines = vec![title_line(app, theme, width), Line::from("")];
    lines.push(cpu_line(app, theme, width));
    lines.push(memory_line(app, theme, width));
    lines.push(pressure_line(local, theme, width));
    lines.push(Line::from(""));

    let rows = local.rows();
    let w = Widths::of(width);
    lines.push(list_rule(local, &rows, theme, width));
    lines.push(header(&w, sample.cores, sample.memory.map(|m| m.total), theme, width));
    let first_row = lines.len();
    for (index, row) in rows.iter().enumerate() {
        lines.push(row_line(row, index == local.selected, sample.cores, sample.memory.map(|m| m.total), &w, theme, width));
    }

    // The machine stays put; the list scrolls under it.
    let fixed = first_row.min(height);
    let room = height - fixed;
    let offset = scroll_into_view(app.viewport.local.get(), Some(local.selected), room, rows.len());
    app.viewport.local.set(offset);
    let mut hits = app.viewport.hits.borrow_mut();
    for index in offset..(offset + room).min(rows.len()) {
        let y = area.y + (fixed + index - offset) as u16;
        hits.push((Rect::new(area.x, y, area.width, 1), Hit::Process(index)));
    }
    drop(hits);
    let mut visible: Vec<Line<'static>> = lines[..fixed].to_vec();
    visible.extend(lines.into_iter().skip(first_row + offset).take(room));
    frame.render_widget(Paragraph::new(visible), area);
}

/// `sam-macbook.local · 10 cores · 32.0 GiB · up 31d02h · load 3.21 3.05 2.98          read 1s ago`
fn title_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let Some(sample) = app.local.latest.as_ref() else { return Line::from("") };
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.push(if sample.host.is_empty() { "this machine".to_string() } else { sample.host.clone() }, theme.strong());
    cells.push(format!(" · {} cores", sample.cores), theme.muted());
    if let Some(m) = sample.memory {
        cells.push(format!(" · {} GiB", gib(m.total)), theme.muted());
    }
    if let Some(up) = sample.uptime_s {
        cells.push(format!(" · up {}", fmt::dur(up as f64)), theme.muted());
    }
    if let Some([one, five, fifteen]) = sample.load {
        cells.push(format!(" · load {one:.2} {five:.2} {fifteen:.2}"), theme.muted());
    }
    let age = (crate::history::secs(app.clock) - sample.at).max(0.0);
    let right = format!("read {} ago ", fmt::dur(age));
    if cells.width() + 2 + fmt::width(&right) <= width {
        cells.pad_to(width - fmt::width(&right));
        cells.push(right, theme.faint());
    }
    cells.line(width, Style::default())
}

/// What a resource line says: its label, its share and how bad that is, of what, made of what.
struct Gauge {
    label: &'static str,
    pct: Option<f64>,
    sev: Severity,
    of: String,
    parts: Vec<(String, Style)>,
}

/// A resource line: label, bar, percentage, the denominator, what it is made of, its history.
fn resource(gauge: Gauge, series: &crate::history::Series, now: f64, theme: &Theme, width: usize) -> Line<'static> {
    let Gauge { label, pct, sev, of, parts } = gauge;
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.cell(label, 10, theme.section());
    let bar = if width >= 120 { 28 } else if width >= 90 { 20 } else { 12 };
    cells.spans(thin_bar(pct, bar, theme.bar_fill(sev), theme));
    cells.push(format!(" {:>6}", fmt::pct(pct)), theme.sev(sev).add_modifier(Modifier::BOLD));
    cells.push(format!("  {of}"), theme.text2());
    let mut tail = Cells::new();
    for (text, style) in parts {
        tail.push(text, style);
    }
    if cells.width() + 3 + tail.width() <= width {
        cells.gap(3);
        cells.spans(tail.into_spans());
    }
    let spark = width.saturating_sub(cells.width() + 3).min(48);
    if spark >= 8 && !series.is_empty() {
        cells.gap(3);
        cells.spans(sparkline(Some(series), now, spark, PCT_SHAPE, |v| theme.bar_fill(severity::node(Some(v)))));
    }
    cells.line(width, Style::default())
}

fn cpu_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let local = &app.local;
    let cores = local.latest.as_ref().map_or(0, |s| s.cores);
    let (pct, of, parts) = match local.cpu {
        Some(cpu) => (
            Some(cpu.busy_pct),
            cores_of(cpu.busy_cores, cores),
            vec![
                ("user ".to_string(), theme.faint()),
                (fmt::pct(Some(cpu.user_pct)), theme.text()),
                (" · system ".to_string(), theme.faint()),
                (fmt::pct(Some(cpu.system_pct)), theme.text()),
            ],
        ),
        None => (None, format!("—/{cores} cores"), vec![("from the second read".to_string(), theme.faint())]),
    };
    let sev = severity::node(pct);
    resource(Gauge { label: "CPU", pct, sev, of, parts }, &local.cpu_pct, crate::history::secs(app.clock), theme, width)
}

fn memory_line(app: &App, theme: &Theme, width: usize) -> Line<'static> {
    let local = &app.local;
    let Some(m) = local.latest.as_ref().and_then(|s| s.memory) else {
        let mut cells = Cells::new();
        cells.push(" ", Style::default());
        cells.cell("MEMORY", 10, theme.section());
        cells.push("not read", theme.muted());
        return cells.line(width, Style::default());
    };
    let mut parts = vec![("app ".to_string(), theme.faint()), (gib(m.app), theme.text())];
    if m.wired > 0 || m.compressed > 0 {
        parts.extend([
            (" · wired ".to_string(), theme.faint()),
            (gib(m.wired), theme.text()),
            (" · compressed ".to_string(), theme.faint()),
            (gib(m.compressed), theme.text()),
        ]);
    }
    parts.extend([(" · cached ".to_string(), theme.faint()), (format!("{} GiB", gib(m.cached)), theme.text())]);
    let of = format!("{}/{} GiB", gib(m.used()), gib(m.total));
    let gauge = Gauge { label: "MEMORY", pct: m.used_pct(), sev: memory_severity(local), of, parts };
    resource(gauge, &local.mem_pct, crate::history::secs(app.clock), theme, width)
}

/// `PRESSURE  ● normal · the kernel counts 50% of memory free      SWAP  2.2/3.0 GiB`
fn pressure_line(local: &Local, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(" ", Style::default());
    cells.cell("PRESSURE", 10, theme.section());
    let sample = local.latest.as_ref();
    match sample.and_then(|s| s.pressure) {
        Some(p) => {
            cells.push("● ", theme.sev(p.severity()));
            cells.push(p.word(), theme.sev(p.severity()).add_modifier(Modifier::BOLD));
            cells.push(format!(" · the kernel counts {}% of memory free", p.free_pct), theme.muted());
        }
        None => {
            cells.push("not reported here", theme.muted());
        }
    }
    if let Some((used, total)) = sample.and_then(|s| s.swap) {
        cells.gap(5);
        cells.push("SWAP ", theme.section());
        cells.push(format!("{}/{} GiB", gib(used), gib(total)), theme.text2());
        if total > 0 {
            cells.push(format!(" {}", fmt::pct(Some(used as f64 / total as f64 * 100.0))), theme.muted());
        }
    }
    cells.line(width, Style::default())
}

fn list_rule(local: &Local, rows: &[Row], theme: &Theme, width: usize) -> Line<'static> {
    let counted = rows.iter().filter(|r| !r.rest).count();
    let what = if local.grouped { "PROGRAMS" } else { "PROCESSES" };
    let by = match local.sort {
        Sort::Cpu => "by CPU",
        Sort::Memory => "by memory",
    };
    rule(width, vec![Span::styled(what, theme.section()), Span::styled(format!(" · {counted} · {by}"), theme.muted())], theme)
}

/// Column widths, by what fits: the name takes what the numbers leave.
struct Widths {
    name: usize,
    pid: usize,
    user: usize,
    cores: usize,
    bar: usize,
    rss: usize,
    share: usize,
    time: usize,
}

impl Widths {
    fn of(width: usize) -> Widths {
        let mut w = Widths { name: 0, pid: 7, user: 12, cores: 6, bar: 12, rss: 10, share: 6, time: 9 };
        let fixed = |w: &Widths| 2 + [w.pid, w.user, w.cores, w.bar, w.rss, w.share, w.time].iter().filter(|c| **c > 0).map(|c| c + GAP).sum::<usize>();
        // What gives way first when the name would get too narrow.
        for drop in [5, 1, 6, 2, 4] {
            if width.saturating_sub(fixed(&w)) >= 28 {
                break;
            }
            match drop {
                5 => w.share = 0,
                1 => w.user = 0,
                6 => w.time = 0,
                2 => w.bar = 0,
                _ => w.pid = 0,
            }
        }
        w.name = width.saturating_sub(fixed(&w)).max(8);
        w
    }
}

fn header(w: &Widths, cores: u32, total: Option<u64>, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    let style = theme.faint().add_modifier(Modifier::BOLD);
    cells.push("  ", Style::default());
    cells.cell("NAME", w.name, style);
    if w.pid > 0 {
        cells.gap(GAP).cell_right("PID", w.pid, style);
    }
    if w.user > 0 {
        cells.gap(GAP).cell("USER", w.user, style);
    }
    cells.gap(GAP).cell_right("CORES", w.cores, style);
    if w.bar > 0 {
        cells.gap(GAP).cell(&format!("of {cores}"), w.bar, theme.faint());
    }
    cells.gap(GAP).cell_right("RSS", w.rss, style);
    if w.share > 0 {
        let of = total.map(|t| format!("/{}G", gib(t))).unwrap_or_default();
        cells.gap(GAP).cell_right(&of, w.share, theme.faint());
    }
    if w.time > 0 {
        cells.gap(GAP).cell_right("CPU TIME", w.time, style);
    }
    cells.line(width, theme.table_head())
}

fn row_line(row: &Row, selected: bool, cores: u32, total: Option<u64>, w: &Widths, theme: &Theme, width: usize) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push(if selected { "▌ " } else { "  " }, theme.accent());
    if row.rest {
        cells.cell("the rest", w.name, theme.muted().add_modifier(Modifier::ITALIC));
        if w.pid > 0 {
            cells.gap(GAP + w.pid);
        }
        if w.user > 0 {
            cells.gap(GAP + w.user);
        }
        cells.gap(GAP).cell_right(&format!("{:.2}", row.cores), w.cores, theme.muted());
        if w.bar > 0 {
            cells.gap(GAP);
            cells.spans(thin_bar(Some(row.cores / f64::from(cores.max(1)) * 100.0), w.bar, theme.bar_fill(Severity::None), theme));
        }
        cells.gap(GAP);
        cells.push("kernel, short-lived", theme.faint());
        return cells.line(width, if selected { theme.selected() } else { Style::default() });
    }
    let mut name = Cells::new();
    name.push(row.name.clone(), if selected { theme.strong() } else { theme.text() });
    if row.count > 1 {
        name.push(format!(" ×{}", row.count), theme.faint());
    }
    let name_cells = name.width();
    cells.spans(super::widgets::fit(name.into_spans(), w.name, false));
    cells.gap(w.name.saturating_sub(name_cells));
    if w.pid > 0 {
        cells.gap(GAP).cell_right(&row.pid.to_string(), w.pid, theme.faint());
    }
    if w.user > 0 {
        cells.gap(GAP).cell(&fmt::truncate(&row.user, w.user), w.user, theme.text2());
    }
    let pct = row.cores / f64::from(cores.max(1)) * 100.0;
    cells.gap(GAP).cell_right(&format!("{:.2}", row.cores), w.cores, theme.strong());
    if w.bar > 0 {
        cells.gap(GAP);
        cells.spans(thin_bar(Some(pct), w.bar, theme.bar_fill(Severity::None), theme));
    }
    cells.gap(GAP).cell_right(&fmt::bytes(row.rss), w.rss, theme.text());
    if w.share > 0 {
        let share = total.filter(|t| *t > 0).map(|t| fmt::pct(Some(row.rss as f64 / t as f64 * 100.0))).unwrap_or_default();
        cells.gap(GAP).cell_right(&share, w.share, theme.muted());
    }
    if w.time > 0 {
        cells.gap(GAP).cell_right(&fmt::dur(row.cpu_time_s), w.time, theme.muted());
    }
    cells.line(width, if selected { theme.selected() } else { Style::default() })
}

/// The drawer: the row under the cursor, in full.
pub fn drawer(app: &App, theme: &Theme, width: usize) -> (Vec<Span<'static>>, Vec<Line<'static>>) {
    let local = &app.local;
    let rows = local.rows();
    let Some(row) = rows.get(local.selected) else {
        return (vec![Span::styled("this machine", theme.strong())], vec![Line::from(Span::styled("reading…", theme.muted()))]);
    };
    let sample = local.latest.as_ref();
    let cores = sample.map_or(0, |s| s.cores);
    let total = sample.and_then(|s| s.memory).map(|m| m.total);
    let mut title = vec![Span::styled(row.name.clone(), theme.strong())];
    if row.rest {
        title.push(Span::styled(" · what no process accounts for", theme.muted()));
        let mut cells = Cells::new();
        cells.push(format!("{:.2} of {cores} cores", row.cores), theme.text());
        cells.push(
            " — the machine's busy cores less every process's: the kernel (kernel_task is not in ps), and processes that started and ended between two reads",
            theme.muted(),
        );
        return (title, vec![cells.line_unpadded(width)]);
    }
    if row.count > 1 {
        title.push(Span::styled(format!(" · {} processes", row.count), theme.muted()));
    } else {
        title.push(Span::styled(format!(" · pid {}", row.pid), theme.muted()));
    }

    let mut numbers = Cells::new();
    numbers.push(format!("{:.2}", row.cores), theme.strong());
    numbers.push(format!(" of {cores} cores"), theme.muted());
    if cores > 0 {
        numbers.push(format!(" ({} of the machine)", fmt::pct(Some(row.cores / f64::from(cores) * 100.0))), theme.faint());
    }
    numbers.push(" · ", theme.faint());
    numbers.push(fmt::bytes(row.rss), theme.strong());
    if let Some(t) = total.filter(|t| *t > 0) {
        numbers.push(format!(" resident of {} GiB ({})", gib(t), fmt::pct(Some(row.rss as f64 / t as f64 * 100.0))), theme.muted());
    }
    numbers.push(" · CPU time ", theme.faint());
    numbers.push(fmt::dur(row.cpu_time_s), theme.text());
    if !row.user.is_empty() {
        numbers.push(" · user ", theme.faint());
        numbers.push(row.user.clone(), theme.text2());
    }
    let mut lines = vec![numbers.line_unpadded(width)];

    let mut who = Cells::new();
    if row.count > 1 {
        who.push(format!("the busiest is pid {}", row.pid), theme.text2());
    } else {
        who.push(format!("parent pid {}", row.ppid), theme.text2());
    }
    who.push(" · ", theme.faint());
    who.push(row.command.clone(), theme.muted());
    lines.push(who.line_unpadded(width));
    (title, lines)
}

/// The band's LOCAL line: `LOCAL   cpu ━━━━ 22.1% 2.2/10 cores ▁▂▃   mem ━━━━ 82.1% 26.3/32 GiB ·
/// pressure normal · swap 2.2 GiB   top opencode 0.47` — what gives way first is the end.
pub fn band_line(app: &App, theme: &Theme, label: usize, width: usize) -> Line<'static> {
    let local = &app.local;
    let mut cells = Cells::new();
    cells.cell("LOCAL", label, theme.section());
    let Some(sample) = local.latest.as_ref() else {
        cells.push("reading this machine…", theme.muted());
        return cells.line(width, theme.text());
    };
    if let (Some(why), false) = (&sample.error, local.shown()) {
        cells.push(format!("could not be read: {why}"), theme.sev(Severity::Warn));
        return cells.line(width, theme.text());
    }
    let hint = "[0] local";
    let room = width.saturating_sub(fmt::width(hint) + 2);
    let cpu_pct = local.cpu.map(|c| c.busy_pct);
    let cpu_sev = severity::node(cpu_pct);
    let mem = sample.memory;
    let mem_sev = memory_severity(local);
    let now = crate::history::secs(app.clock);
    // From the widest to the narrowest: the sparkline goes, then the swap and the busiest
    // process, then the bars shorten; the denominators last.
    let shapes = [
        (16, true, 10, 2),
        (12, true, 8, 2),
        (12, true, 0, 2),
        (12, true, 0, 1),
        (12, true, 0, 0),
        (8, true, 0, 0),
        (0, true, 0, 0),
        (0, false, 0, 0),
    ];
    let mut line = Cells::new();
    for (bar, absolute, spark, extra) in shapes {
        let mut pieces = Cells::new();
        pieces.push("cpu ", theme.faint());
        if bar > 0 {
            pieces.spans(thin_bar(cpu_pct, bar, theme.bar_fill(cpu_sev), theme));
            pieces.push(" ", theme.text());
        }
        pieces.push(fmt::pct(cpu_pct), theme.sev(cpu_sev).add_modifier(Modifier::BOLD));
        if absolute {
            let busy = local.cpu.map_or("—".to_string(), |c| format!("{:.1}", c.busy_cores));
            pieces.push(format!(" {busy}/{} cores", sample.cores), theme.text2());
        }
        if spark > 0 && !local.cpu_pct.is_empty() {
            pieces.push(" ", theme.text());
            pieces.spans(sparkline(Some(&local.cpu_pct), now, spark, PCT_SHAPE, |v| theme.bar_fill(severity::node(Some(v)))));
        }
        if let Some(m) = mem {
            pieces.push("   mem ", theme.faint());
            if bar > 0 {
                pieces.spans(thin_bar(m.used_pct(), bar, theme.bar_fill(mem_sev), theme));
                pieces.push(" ", theme.text());
            }
            pieces.push(fmt::pct(m.used_pct()), theme.sev(mem_sev).add_modifier(Modifier::BOLD));
            if absolute {
                pieces.push(format!(" {}/{} GiB", gib(m.used()), gib(m.total)), theme.text2());
            }
        }
        if let Some(p) = sample.pressure {
            pieces.push(" · ", theme.faint());
            pieces.push("pressure ", theme.faint());
            let sev = p.severity();
            pieces.push(p.word(), if sev == Severity::Ok { theme.text2() } else { theme.sev(sev).add_modifier(Modifier::BOLD) });
        }
        // 2: the swap and the busiest process; 1: the busiest only.
        if extra == 2 && let Some((used, _)) = sample.swap.filter(|s| s.0 > 0) {
            pieces.push(" · swap ", theme.faint());
            pieces.push(format!("{} GiB", gib(used)), theme.text2());
        }
        if extra >= 1 && let Some((top, cores)) = local.top() {
            pieces.push("   top ", theme.faint());
            pieces.push(top.name().to_string(), theme.text());
            pieces.push(format!(" {cores:.2}"), theme.text2());
        }
        line = pieces;
        if line.width() <= room.saturating_sub(cells.width()) {
            break;
        }
    }
    cells.spans(line.into_spans());
    if cells.width() + 2 + fmt::width(hint) <= width {
        cells.pad_to(width - fmt::width(hint));
        cells.push(hint, theme.faint());
    }
    cells.line(width, theme.text())
}
