//! Bars (`03-glyphs.md` *Bars*): the share bar, `█` filled and `░` empty, at a width fixed per
//! column so rows compare.

/// `pct` of 100 as `█` then `░`, `width` cells; returns the two parts so each takes its colour.
pub fn share(pct: f64, width: usize) -> (String, String) {
    let fill = ((pct.clamp(0.0, 100.0) / 100.0 * width as f64).round() as usize).min(width);
    ("█".repeat(fill), "░".repeat(width - fill))
}

/// Largest-remainder rounding: `parts` (fractions of the whole) to cells that sum to `width`
/// exactly, so the rows of one node add up to the node's bar (§5.3 made visible).
pub fn largest_remainder(parts: &[f64], width: usize) -> Vec<usize> {
    let raw: Vec<f64> = parts.iter().map(|p| p.max(0.0) * width as f64).collect();
    let mut cells: Vec<usize> = raw.iter().map(|r| r.floor() as usize).collect();
    let total: f64 = parts.iter().map(|p| p.max(0.0)).sum();
    let target = ((total * width as f64).round() as usize).min(width);
    let mut left = target.saturating_sub(cells.iter().sum());
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| (raw[b] - raw[b].floor()).total_cmp(&(raw[a] - raw[a].floor())));
    for i in order {
        if left == 0 {
            break;
        }
        cells[i] += 1;
        left -= 1;
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_and_their_sum() {
        assert_eq!(share(93.0, 9), ("████████".into(), "░".into()));
        assert_eq!(share(0.0, 3), (String::new(), "░░░".into()));
        let cells = largest_remainder(&[0.255, 0.129, 0.055, 0.019, 0.474], 14);
        assert_eq!(cells.iter().sum::<usize>(), 13, "93.2% of 14 cells");
    }
}
