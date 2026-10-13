//! Text placed by cell on a ratatui `Buffer`, the way the simulation's `Buf` does it
//! (`docs/research/simulasi-konsep-ui.html`): `put` at a column, `putr` ending at one, `bg` and
//! `clear` over a rectangle. Everything is clipped to the buffer and to the painter's clip, by
//! display width, so a string can never cross a frame or the edge of the screen.

use crate::fmt;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

pub struct Painter<'a> {
    pub buf: &'a mut Buffer,
    clip: Rect,
}

impl<'a> Painter<'a> {
    pub fn new(buf: &'a mut Buffer) -> Painter<'a> {
        let clip = buf.area;
        Painter { buf, clip }
    }

    /// The clip in force: nothing is drawn outside it.
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Run `f` with the clip narrowed to `area` (and the old clip), then put it back.
    pub fn within<R>(&mut self, area: Rect, f: impl FnOnce(&mut Painter<'_>) -> R) -> R {
        let old = self.clip;
        self.clip = old.intersection(area);
        let out = f(self);
        self.clip = old;
        out
    }

    /// `text` from column `x` of row `y`; returns the column after it (as if nothing were
    /// clipped, so a caller can keep laying out).
    pub fn put(&mut self, x: u16, y: u16, text: &str, style: Style) -> u16 {
        let width = fmt::width(text) as u16;
        let c = self.clip;
        if text.is_empty() || y < c.y || y >= c.bottom() || x >= c.right() {
            return x.saturating_add(width);
        }
        let mut col = x;
        for ch in text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            if w == 0 {
                continue;
            }
            if col >= c.x && col + w <= c.right() {
                let mut tmp = [0u8; 4];
                if let Some(cell) = self.buf.cell_mut((col, y)) {
                    cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
                }
                if w == 2
                    && let Some(cell) = self.buf.cell_mut((col + 1, y))
                {
                    cell.set_symbol("").set_style(style);
                }
            }
            col = col.saturating_add(w);
            if col >= c.right() {
                break;
            }
        }
        x.saturating_add(width)
    }

    /// `text` ending just before column `end` (the simulation's `putr`); returns where it starts.
    pub fn putr(&mut self, end: u16, y: u16, text: &str, style: Style) -> u16 {
        let width = fmt::width(text) as u16;
        let x = end.saturating_sub(width);
        // Cut from the left when it would start before column 0.
        if (end as usize) < width as usize {
            let keep: String = cut_left(text, end as usize);
            self.put(0, y, &keep, style);
            return 0;
        }
        self.put(x, y, text, style);
        x
    }

    /// Patch the style of a rectangle, keeping the text (a background, a modifier).
    pub fn style(&mut self, area: Rect, style: Style) {
        let area = self.clip.intersection(area);
        self.buf.set_style(area, style);
    }

    /// Only the background of a rectangle.
    pub fn bg(&mut self, area: Rect, color: Color) {
        self.style(area, Style::default().bg(color));
    }

    /// Blank a rectangle with `style`: spaces, the style's colours.
    pub fn clear(&mut self, area: Rect, style: Style) {
        let area = self.clip.intersection(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = self.buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    }

    /// A run of `ch` from `x`, `n` cells long.
    pub fn run(&mut self, x: u16, y: u16, ch: char, n: u16, style: Style) {
        if n == 0 {
            return;
        }
        let s: String = std::iter::repeat_n(ch, n as usize).collect();
        self.put(x, y, &s, style);
    }
}

/// The last `cells` cells of `text`.
fn cut_left(text: &str, cells: usize) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut w = 0;
    for ch in text.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > cells {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.into_iter().rev().collect()
}

/// `s` cut to `w` cells with `…` (the simulation's `tr`).
pub fn tr(s: &str, w: usize) -> String {
    fmt::truncate(s, w)
}

/// `s` cut and padded to exactly `w` cells (`rp`).
pub fn rp(s: &str, w: usize) -> String {
    fmt::pad(&tr(s, w), w)
}

/// `s` cut and right-aligned in exactly `w` cells (`lp`).
pub fn lp(s: &str, w: usize) -> String {
    fmt::right(&tr(s, w), w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    #[test]
    fn put_and_putr_clip_by_display_width() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        let mut p = Painter::new(&mut buf);
        assert_eq!(p.put(7, 0, "abcdef", Style::default()), 13);
        p.putr(10, 1, "xyz", Style::default());
        p.within(Rect::new(0, 0, 3, 2), |p| {
            p.put(0, 0, "日本語", Style::default());
        });
        assert!(row(&buf, 0).ends_with("abc"), "{:?}", row(&buf, 0));
        assert!(row(&buf, 0).starts_with("日"), "a wide char that would cross the clip is left out: {:?}", row(&buf, 0));
        assert!(row(&buf, 1).ends_with("xyz"));
        let mut p = Painter::new(&mut buf);
        p.putr(2, 1, "hello", Style::default());
        assert!(row(&buf, 1).starts_with("lo"), "cut from the left: {:?}", row(&buf, 1));
    }

    #[test]
    fn tr_rp_lp() {
        assert_eq!(tr("clickhouse3", 6), "click…");
        assert_eq!(rp("ab", 4), "ab  ");
        assert_eq!(lp("ab", 4), "  ab");
    }
}
