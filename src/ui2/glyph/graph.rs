//! Graphs on a fixed scale (`03-glyphs.md` *Graphs*, `graph()` in the simulation): one sample a
//! column (two in braille), newest at the right, never auto-scaled. Each column takes the colour
//! of its own value's severity; a second series is a dot `•` a column on the same axes.

use crate::caps::GraphTier;
use crate::ui2::paint::Painter;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// The eight heights of a block cell.
pub const SPK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const ASCII: [char; 7] = ['.', ':', '-', '=', '+', '*', '#'];

/// Samples a column takes in this tier.
pub fn per_column(tier: GraphTier) -> usize {
    match tier {
        GraphTier::Braille => 2,
        _ => 1,
    }
}

/// Fill the slices a poll did not land in with the value before them, so a 4-minute history
/// stretched over a wide graph reads as a line, not a comb. Before the first sample stays empty:
/// the graph does not invent a past it never saw.
pub fn stretch(mut samples: Vec<Option<f64>>) -> Vec<Option<f64>> {
    let mut last = None;
    for s in samples.iter_mut() {
        match s {
            Some(v) => last = Some(*v),
            None => *s = last,
        }
    }
    samples
}

/// Eighths of the column `v` fills on `h` rows of a `max` scale: `round(v / max × h × 8)`, at
/// least one, so a known zero still shows as the floor.
fn eighths(v: f64, max: f64, h: u16) -> u32 {
    let lvl = (v / max * f64::from(h) * 8.0).round();
    (lvl.max(1.0) as u32).min(u32::from(h) * 8)
}

/// The filled area of `samples` in `area`. `samples` holds `area.width × per_column(tier)`
/// values, oldest first; a missing one leaves a gap.
pub fn area(p: &mut Painter<'_>, tier: GraphTier, rect: Rect, samples: &[Option<f64>], max: f64, color: &dyn Fn(f64) -> Style) {
    if rect.width == 0 || rect.height == 0 || max <= 0.0 {
        return;
    }
    let h = rect.height;
    let per = per_column(tier);
    for c in 0..rect.width {
        let i = c as usize * per;
        let x = rect.x + c;
        match tier {
            GraphTier::Braille => {
                let a = samples.get(i).copied().flatten();
                let b = samples.get(i + 1).copied().flatten();
                if a.is_none() && b.is_none() {
                    continue;
                }
                // Four dots a row: the level in quarter rows.
                let quarter = |v: Option<f64>| v.map_or(0, |v| ((v / max * f64::from(h) * 4.0).round().max(1.0) as u32).min(u32::from(h) * 4));
                let (qa, qb) = (quarter(a), quarter(b));
                let worst = a.into_iter().chain(b).fold(f64::MIN, f64::max);
                let style = color(worst);
                for r in 0..h {
                    let na = qa.saturating_sub(u32::from(r) * 4).min(4);
                    let nb = qb.saturating_sub(u32::from(r) * 4).min(4);
                    if na == 0 && nb == 0 {
                        continue;
                    }
                    p.put(x, rect.y + h - 1 - r, &braille(na, nb).to_string(), style);
                }
            }
            GraphTier::Block | GraphTier::Ascii => {
                let Some(v) = samples.get(i).copied().flatten() else { continue };
                let lvl = eighths(v, max, h);
                let style = color(v);
                for r in 0..h {
                    let r32 = u32::from(r);
                    let ch = if lvl >= (r32 + 1) * 8 {
                        if tier == GraphTier::Block { '█' } else { '#' }
                    } else if lvl > r32 * 8 {
                        let part = (lvl - r32 * 8) as usize;
                        if tier == GraphTier::Block { SPK[part - 1] } else { ASCII[(part - 1).min(6)] }
                    } else {
                        continue;
                    };
                    p.put(x, rect.y + h - 1 - r, &ch.to_string(), style);
                }
            }
        }
    }
}

/// A braille cell with `left` and `right` dots filled from the bottom (0–4 each).
fn braille(left: u32, right: u32) -> char {
    const L: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
    const R: [u32; 4] = [0x80, 0x20, 0x10, 0x08];
    let mut bits = 0;
    for bit in L.iter().take(left.min(4) as usize) {
        bits |= bit;
    }
    for bit in R.iter().take(right.min(4) as usize) {
        bits |= bit;
    }
    char::from_u32(0x2800 + bits).unwrap_or(' ')
}

/// The second series: a `•` a column at its height, over whatever is drawn there.
pub fn dots(p: &mut Painter<'_>, rect: Rect, samples: &[Option<f64>], max: f64, style: Style) {
    if rect.height == 0 || max <= 0.0 {
        return;
    }
    let h = rect.height;
    for (c, v) in samples.iter().take(rect.width as usize).enumerate() {
        let Some(v) = v else { continue };
        let r = ((v / max * f64::from(h)).floor().max(0.0) as u16).min(h - 1);
        p.put(rect.x + c as u16, rect.y + h - 1 - r, "•", style);
    }
}

/// A sparkline on `max`: 8 levels, a gap as a space (`sparkStr` in the simulation).
pub fn sparkline(samples: &[Option<f64>], max: f64) -> String {
    samples
        .iter()
        .map(|v| match v {
            Some(v) if max > 0.0 => SPK[((v / max * 8.0).floor().max(0.0) as usize).min(7)],
            Some(_) => SPK[0],
            None => ' ',
        })
        .collect()
}

/// One slice in `per` of `samples`, the worst of each group: `width` columns from
/// `width × per` samples for a series drawn at one sample a column.
pub fn worst_of(samples: &[Option<f64>], per: usize) -> Vec<Option<f64>> {
    samples
        .chunks(per.max(1))
        .map(|c| c.iter().flatten().copied().fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    fn rows(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect()
    }

    #[test]
    fn block_columns_fill_eighths_on_a_fixed_scale() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        let mut p = Painter::new(&mut buf);
        let s = [Some(100.0), Some(50.0), Some(0.0), None];
        area(&mut p, GraphTier::Block, Rect::new(0, 0, 4, 2), &s, 100.0, &|_| Style::default());
        assert_eq!(rows(&buf), ["█   ", "██▁ "], "100 → two rows, 50 → one, 0 → the floor, none → a gap");
    }

    #[test]
    fn braille_takes_two_samples_a_column() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let mut p = Painter::new(&mut buf);
        area(&mut p, GraphTier::Braille, Rect::new(0, 0, 1, 1), &[Some(100.0), Some(25.0)], 100.0, &|_| Style::default());
        assert_eq!(rows(&buf), ["⣇"]);
        assert_eq!(stretch(vec![None, Some(1.0), None, Some(2.0), None]), [None, Some(1.0), Some(1.0), Some(2.0), Some(2.0)]);
        assert_eq!(sparkline(&[Some(0.0), Some(99.0), None], 100.0), "▁█ ");
        assert_eq!(worst_of(&[Some(1.0), Some(3.0), None, None], 2), [Some(3.0), None]);
    }
}
