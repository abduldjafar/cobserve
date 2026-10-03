//! View 4: the tape — what changed, newest first (`crate::tape`).

use super::widgets::{tone_spans, Cells};
use crate::app::{scroll_into_view, App};
use crate::fmt;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let (crit, warn) = app.tape.counts();

    let mut title = Cells::new();
    title.push(" TAPE", theme.section());
    title.push(format!(" · {}", fmt::plural(app.tape.len(), "event", "events")), theme.muted());
    if crit > 0 {
        title.push(format!(" · {crit} {}", Severity::Crit.glyph()), theme.sev(Severity::Crit));
    }
    if warn > 0 {
        title.push(format!(" · {warn} {}", Severity::Warn.glyph()), theme.sev(Severity::Warn));
    }
    title.push(" · newest first · ↑↓ scroll · ⏎ go to it", theme.faint());
    frame.render_widget(
        Paragraph::new(title.line(width, Style::default())),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height < 2 {
        return;
    }

    let mut header = Cells::new();
    header.push("   ", Style::default());
    header.cell("TIME UTC", 10, theme.section());
    header.cell("KIND", 7, theme.section());
    header.push("WHAT HAPPENED", theme.section());
    frame.render_widget(
        Paragraph::new(header.line(width, Style::default())),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );

    let body = Rect::new(area.x, area.y + 2, area.width, area.height.saturating_sub(2));
    if app.tape.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "   nothing has happened yet — events appear here as the fleet changes",
                theme.muted(),
            )),
            body,
        );
        return;
    }
    let height = body.height as usize;
    let selected = app.tape_selection();
    let offset = scroll_into_view(app.viewport.tape.get(), Some(selected), height, app.tape.len());
    app.viewport.tape.set(offset);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height);
    let mut previous_minute: Option<u64> = None;
    for (i, event) in app.tape.newest_first().enumerate().skip(offset).take(height) {
        let is_selected = i == selected;
        let mut cells = Cells::new();
        cells.push(if is_selected { "▌" } else { " " }, theme.accent());
        cells.push(format!("{} ", event.level.glyph()), theme.sev(event.level).add_modifier(Modifier::BOLD));
        // The time is printed in full once per minute and dimmed after, so a burst of events
        // reads as one moment.
        let minute = (event.at / 60.0) as u64;
        let clock = fmt::utc_clock_secs(event.at);
        let style = if previous_minute == Some(minute) { theme.faint() } else { theme.text2() };
        previous_minute = Some(minute);
        cells.cell(&clock, 10, style);
        cells.cell(event.kind.label(), 7, theme.muted());
        cells.spans(tone_spans(&event.parts, theme));
        lines.push(cells.line(width, if is_selected { theme.selected() } else { Style::default() }));
    }
    frame.render_widget(Paragraph::new(lines), body);
}
