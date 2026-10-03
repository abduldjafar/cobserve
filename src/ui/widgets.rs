//! The small pieces every view is built from: a line assembled column by column, bars with
//! eighth-cell resolution, sparklines, keycaps, pills and titled rules.
//!
//! Widths are display cells (unicode-width), never bytes or chars, so a person's name or a
//! piece of SQL with wide characters cannot push a column out of line.

use crate::fmt;
use crate::history::{spark_glyph, Series, WINDOW_S};
use crate::insight::Tone;
use crate::severity::Severity;
use crate::theme::Theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A line built left to right, with its width tracked so columns can be placed exactly.
#[derive(Debug, Default, Clone)]
pub struct Cells {
    spans: Vec<Span<'static>>,
    width: usize,
}

impl Cells {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn push(&mut self, text: impl Into<String>, style: Style) -> &mut Self {
        let text = text.into();
        if text.is_empty() {
            return self;
        }
        self.width += fmt::width(&text);
        self.spans.push(Span::styled(text, style));
        self
    }

    pub fn span(&mut self, span: Span<'static>) -> &mut Self {
        self.width += fmt::width(&span.content);
        self.spans.push(span);
        self
    }

    pub fn spans(&mut self, spans: impl IntoIterator<Item = Span<'static>>) -> &mut Self {
        for span in spans {
            self.span(span);
        }
        self
    }

    /// Spaces up to column `col`. Nothing when already past it.
    pub fn pad_to(&mut self, col: usize) -> &mut Self {
        if col > self.width {
            let gap = " ".repeat(col - self.width);
            self.push(gap, Style::default());
        }
        self
    }

    pub fn gap(&mut self, cells: usize) -> &mut Self {
        if cells > 0 {
            self.push(" ".repeat(cells), Style::default());
        }
        self
    }

    /// `text` left-aligned in exactly `cells` cells.
    pub fn cell(&mut self, text: &str, cells: usize, style: Style) -> &mut Self {
        self.push(fmt::pad(text, cells), style)
    }

    /// `text` right-aligned in exactly `cells` cells.
    pub fn cell_right(&mut self, text: &str, cells: usize, style: Style) -> &mut Self {
        self.push(fmt::right(text, cells), style)
    }

    /// The line, cut to `max` cells with a `…` and padded to exactly `max`, so a row's
    /// background (the selection) runs edge to edge.
    pub fn line(self, max: usize, style: Style) -> Line<'static> {
        Line::from(fit(self.spans, max, true)).style(style)
    }

    /// Cut to `max` cells but not padded.
    pub fn line_unpadded(self, max: usize) -> Line<'static> {
        Line::from(fit(self.spans, max, false))
    }

    pub fn into_spans(self) -> Vec<Span<'static>> {
        self.spans
    }
}

/// Cut spans to `max` cells, ending in `…` when anything was cut; optionally pad to `max`.
pub fn fit(spans: Vec<Span<'static>>, max: usize, pad: bool) -> Vec<Span<'static>> {
    let mut used = 0usize;
    let mut out: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
    for span in spans {
        let w = fmt::width(&span.content);
        if used + w <= max {
            used += w;
            out.push(span);
            continue;
        }
        let room = max.saturating_sub(used);
        if room > 0 {
            let style = span.style;
            let text = fmt::truncate(&span.content, room);
            used += fmt::width(&text);
            out.push(Span::styled(text, style));
        } else if let Some(last) = out.pop() {
            // The cut fell exactly on a span boundary: the ellipsis takes the last cell of
            // the span before, so a cut line never looks complete.
            let w = fmt::width(&last.content);
            used -= w;
            let text = fmt::truncate(&format!("{}…", last.content), w);
            used += fmt::width(&text);
            out.push(Span::styled(text, last.style));
        }
        break;
    }
    if pad && used < max {
        out.push(Span::raw(" ".repeat(max - used)));
    }
    out
}

const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// A bar for a percentage, `cells` wide, with eighth-cell resolution: 25.3% of 16 cells is
/// four full blocks and a `▏`, not a rounding to four or five.
///
/// On a terminal that shows backgrounds the empty part is a coloured track; elsewhere it is
/// `░`, as DESIGN.md §7 drew it. A share that is not zero always shows at least an eighth, so
/// "some" never looks like "none".
pub fn bar(value: Option<f64>, cells: usize, fill: Color, theme: &Theme) -> Vec<Span<'static>> {
    if cells == 0 {
        return Vec::new();
    }
    let total = cells * 8;
    let mut eighths = value
        .map(|v| ((v / 100.0) * total as f64).round().clamp(0.0, total as f64) as usize)
        .unwrap_or(0);
    if eighths == 0 && value.is_some_and(|v| v > 0.0) {
        eighths = 1;
    }
    let full = eighths / 8;
    let rest = eighths % 8;
    let empty = cells - full - usize::from(rest > 0);

    let track_bg = if theme.paints_background() { theme.track } else { Color::Reset };
    let filled_style = Style::default().fg(fill).bg(track_bg);
    let mut spans = Vec::new();
    if full > 0 {
        spans.push(Span::styled("█".repeat(full), filled_style));
    }
    if rest > 0 {
        spans.push(Span::styled(EIGHTHS[rest], filled_style));
    }
    if empty > 0 {
        if theme.paints_background() {
            spans.push(Span::styled(" ".repeat(empty), Style::default().bg(theme.track)));
        } else {
            spans.push(Span::styled("░".repeat(empty), theme.faint()));
        }
    }
    spans
}

/// How a sparkline maps values to heights.
#[derive(Debug, Clone, Copy)]
pub enum Scale {
    /// A fixed range — counts that start at zero.
    Fixed(f64, f64),
    /// The range of the data itself, but never narrower than `min_span` (so noise is not
    /// drawn as drama) and never outside `[floor, ceil]`. The bar next to it already says how
    /// high the number is; the sparkline's job is the shape — climbing, sawing, flat.
    Auto { min_span: f64, floor: f64, ceil: f64 },
}

/// Percentages: the shape of the last minutes, at least 10 points tall, inside 0–100.
pub const PCT_SHAPE: Scale = Scale::Auto {
    min_span: 10.0,
    floor: 0.0,
    ceil: 100.0,
};

fn resolve(scale: Scale, values: &[Option<f64>]) -> (f64, f64) {
    match scale {
        Scale::Fixed(lo, hi) => (lo, hi),
        Scale::Auto { min_span, floor, ceil } => {
            let mut known = values.iter().flatten().copied();
            let Some(first) = known.next() else {
                return (floor, ceil);
            };
            let (mut lo, mut hi) = known.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
            if hi - lo < min_span {
                let mid = (hi + lo) / 2.0;
                lo = mid - min_span / 2.0;
                hi = mid + min_span / 2.0;
            }
            // Shift the window back inside the bounds rather than squashing it.
            if lo < floor {
                hi += floor - lo;
                lo = floor;
            }
            if hi > ceil {
                lo -= hi - ceil;
                hi = ceil;
            }
            (lo.max(floor), hi)
        }
    }
}

/// A sparkline of the last `WINDOW_S` of a series, `cells` wide. Each cell is coloured by what
/// its value means (`color_of`), so a history of amber and red reads as such at a glance.
/// Cells with no data are blank.
pub fn sparkline(
    series: Option<&Series>,
    now: f64,
    cells: usize,
    scale: Scale,
    color_of: impl Fn(f64) -> Color,
) -> Vec<Span<'static>> {
    let Some(series) = series else {
        return vec![Span::raw(" ".repeat(cells))];
    };
    let buckets = series.buckets(now, WINDOW_S, cells);
    let (lo, hi) = resolve(scale, &buckets);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_color: Option<Color> = None;
    for value in buckets {
        let (glyph, color) = match value {
            Some(v) => (spark_glyph(v, lo, hi), Some(color_of(v))),
            None => (' ', None),
        };
        if color != run_color && !run.is_empty() {
            spans.push(match run_color {
                Some(c) => Span::styled(std::mem::take(&mut run), Style::default().fg(c)),
                None => Span::raw(std::mem::take(&mut run)),
            });
        }
        run_color = color;
        run.push(glyph);
    }
    if !run.is_empty() {
        spans.push(match run_color {
            Some(c) => Span::styled(run, Style::default().fg(c)),
            None => Span::raw(run),
        });
    }
    spans
}

/// A key and what it does, for the footer: ` ⏎ ` on a raised cap, then the verb.
pub fn keycap(key: &str, label: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {key} "), theme.keycap()),
        Span::styled(format!(" {label}"), theme.muted()),
    ]
}

/// `▲ DEGRADED` on its colour.
pub fn pill(text: &str, sev: Severity, theme: &Theme) -> Span<'static> {
    Span::styled(format!(" {text} "), theme.pill(sev))
}

/// `─ TITLE ─────…` across `width` cells.
pub fn rule(width: usize, title: Vec<Span<'static>>, theme: &Theme) -> Line<'static> {
    let mut cells = Cells::new();
    cells.push("─", theme.rule());
    if !title.is_empty() {
        cells.push(" ", Style::default());
        cells.spans(title);
        cells.push(" ", Style::default());
    }
    let rest = width.saturating_sub(cells.width());
    cells.push("─".repeat(rest), theme.rule());
    cells.line_unpadded(width)
}

/// The spans of an insight or a tape line, coloured by what each piece is.
pub fn tone_spans(parts: &[(String, Tone)], theme: &Theme) -> Vec<Span<'static>> {
    parts
        .iter()
        .map(|(text, tone)| {
            let style = match tone {
                Tone::Plain => theme.text(),
                Tone::Strong => theme.strong(),
                Tone::Muted => theme.muted(),
                Tone::Node => theme.accent().add_modifier(Modifier::BOLD),
                Tone::Person => theme.person(),
                Tone::Sev(sev) => theme.sev(*sev).add_modifier(Modifier::BOLD),
            };
            Span::styled(text.clone(), style)
        })
        .collect()
}

/// Worker slots as dots: `●●●●○○` — busy in colour, idle hollow. Too many to draw becomes a
/// plain count.
pub fn dots(busy: u32, total: u32, color: Color, theme: &Theme) -> Vec<Span<'static>> {
    if total == 0 || total > 16 {
        return Vec::new();
    }
    let busy = busy.min(total);
    let mut spans = Vec::new();
    if busy > 0 {
        spans.push(Span::styled("●".repeat(busy as usize), Style::default().fg(color)));
    }
    if total > busy {
        spans.push(Span::styled("○".repeat((total - busy) as usize), theme.faint()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, Variant};

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn bars_have_eighth_cell_resolution() {
        let theme = Theme::new(Depth::Ansi16, Variant::Dark);
        // 25% of 16 cells = 4 cells exactly.
        assert_eq!(text(&bar(Some(25.0), 16, Color::Blue, &theme)), format!("{}{}", "█".repeat(4), "░".repeat(12)));
        // 26.6% of 16 cells = 4.25 cells = 34 eighths → four blocks and a quarter.
        assert_eq!(text(&bar(Some(26.6), 16, Color::Blue, &theme)), format!("{}▎{}", "█".repeat(4), "░".repeat(11)));
        assert_eq!(text(&bar(Some(100.0), 8, Color::Blue, &theme)), "█".repeat(8));
        assert_eq!(text(&bar(Some(250.0), 8, Color::Blue, &theme)), "█".repeat(8), "clamped");
        assert_eq!(text(&bar(None, 4, Color::Blue, &theme)), "░░░░", "unknown is an empty track");
        assert_eq!(text(&bar(Some(0.1), 4, Color::Blue, &theme)), "▏░░░", "some is never none");
        // Every bar is exactly as wide as asked.
        for v in [0.0, 3.3, 49.9, 50.0, 77.7, 99.9] {
            assert_eq!(fmt::width(&text(&bar(Some(v), 13, Color::Blue, &theme))), 13, "{v}");
        }
    }

    #[test]
    fn with_backgrounds_the_track_is_painted() {
        let theme = Theme::new(Depth::TrueColor, Variant::Dark);
        let spans = bar(Some(50.0), 4, Color::Blue, &theme);
        assert_eq!(text(&spans), "██  ");
        assert_eq!(spans.last().unwrap().style.bg, Some(theme.track));
    }

    #[test]
    fn cells_place_columns_exactly() {
        let mut cells = Cells::new();
        cells.push("ab", Style::default()).pad_to(5).cell_right("7", 3, Style::default());
        assert_eq!(cells.width(), 8);
        let line = cells.line(10, Style::default());
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "ab     7  ");
    }

    #[test]
    fn a_line_too_long_is_cut_with_an_ellipsis() {
        let mut cells = Cells::new();
        cells.push("hello ", Style::default()).push("world", Style::default());
        let line = cells.line(8, Style::default());
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hello w…");
    }

    #[test]
    fn a_cut_on_a_span_boundary_still_ends_in_an_ellipsis() {
        let mut cells = Cells::new();
        cells.push("abcd", Style::default()).push("efgh", Style::default());
        let line = cells.line_unpadded(4);
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "abc…");
    }

    #[test]
    fn a_rule_spans_the_width_with_its_title() {
        let theme = Theme::new(Depth::Mono, Variant::Dark);
        let line = rule(20, vec![Span::raw("INSIGHTS")], &theme);
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "─ INSIGHTS ─────────");
        assert_eq!(fmt::width(&joined), 20);
    }

    #[test]
    fn sparklines_are_as_wide_as_asked_and_blank_without_data() {
        let mut s = Series::default();
        for i in 0..120 {
            s.push(f64::from(i) * 2.0, f64::from(i % 100));
        }
        let now = s.last_t().unwrap();
        let spans = sparkline(Some(&s), now, 12, Scale::Fixed(0.0, 100.0), |_| Color::Blue);
        assert_eq!(fmt::width(&text(&spans)), 12);
        let none = sparkline(None, now, 6, PCT_SHAPE, |_| Color::Blue);
        assert_eq!(text(&none), "      ");
    }

    #[test]
    fn an_auto_scale_shows_the_shape_but_not_the_noise() {
        // 88 → 97: narrow, but more than noise, so it spans the whole height.
        let (lo, hi) = resolve(PCT_SHAPE, &[Some(88.0), Some(92.0), Some(97.0)]);
        assert!(lo <= 88.0 && hi >= 97.0 && hi - lo >= 10.0, "{lo} {hi}");
        assert!(hi <= 100.0, "never past 100%");
        // A flat line stays a flat line in the middle, not a cliff.
        let (lo, hi) = resolve(PCT_SHAPE, &[Some(36.0), Some(36.2)]);
        assert!((hi - lo - 10.0).abs() < 1e-9 && lo < 36.0 && hi > 36.2);
        // Near the floor the window shifts up instead of going negative.
        let (lo, hi) = resolve(PCT_SHAPE, &[Some(1.0), Some(2.0)]);
        assert_eq!((lo, hi), (0.0, 10.0));
        assert_eq!(resolve(Scale::Fixed(0.0, 5.0), &[Some(9.0)]), (0.0, 5.0));
        assert_eq!(resolve(PCT_SHAPE, &[None, None]), (0.0, 100.0));
    }

    #[test]
    fn worker_dots_show_busy_and_idle() {
        let theme = Theme::new(Depth::Mono, Variant::Dark);
        assert_eq!(text(&dots(4, 6, Color::Yellow, &theme)), "●●●●○○");
        assert!(dots(3, 40, Color::Yellow, &theme).is_empty(), "too many to draw");
    }
}
