//! The three-row digits of the two headline numbers (`03-glyphs.md` *Big digits*, `BIG` in the
//! simulation): three cells a digit, one cell between glyphs.

use crate::ui2::paint::Painter;
use ratatui::style::Style;

fn glyph(ch: char) -> [&'static str; 3] {
    match ch {
        '0' => ["█▀█", "█ █", "█▄█"],
        '1' => ["▄█ ", " █ ", "▄█▄"],
        '2' => ["▀▀█", "█▀▀", "█▄▄"],
        '3' => ["▀▀█", "▀▀█", "▄▄█"],
        '4' => ["█ █", "▀▀█", "  █"],
        '5' => ["█▀▀", "▀▀█", "▄▄█"],
        '6' => ["█▀▀", "█▀█", "█▄█"],
        '7' => ["▀▀█", "  █", "  █"],
        '8' => ["█▀█", "█▀█", "█▄█"],
        '9' => ["█▀█", "▀▀█", "▄▄█"],
        '%' => ["▀ █", " ▄▀", "█ ▄"],
        '.' => ["  ", "  ", "▄ "],
        _ => [" ", " ", " "],
    }
}

/// The width `text` takes in big digits.
pub fn width(text: &str) -> u16 {
    text.chars().map(|c| glyph(c)[0].chars().count() as u16 + 1).sum()
}

/// Draw `text` from (`x`, `y`); returns the column after the last glyph's gap, as `big()` does.
pub fn draw(p: &mut Painter<'_>, x: u16, y: u16, text: &str, style: Style) -> u16 {
    let mut x = x;
    for ch in text.chars() {
        let g = glyph(ch);
        for (r, row) in g.iter().enumerate() {
            p.put(x, y + r as u16, row, style);
        }
        x += g[0].chars().count() as u16 + 1;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn forty_two_percent_as_the_screen_draws_it() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        let mut p = Painter::new(&mut buf);
        assert_eq!(draw(&mut p, 0, 0, "42%", Style::default()), 12);
        let rows: Vec<String> = (0..3).map(|y| (0..12).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        assert_eq!(rows, ["█ █ ▀▀█ ▀ █ ", "▀▀█ █▀▀  ▄▀ ", "  █ █▄▄ █ ▄ "]);
        assert_eq!(width("16"), 8);
    }
}
