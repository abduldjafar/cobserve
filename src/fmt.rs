//! How numbers are written (DESIGN.md §7), shared by the screen, the insights and the tape.
//!
//! Bytes are binary with one decimal (`1.2 GiB`), percentages one decimal (`25.3%`),
//! durations `12s`, `4m35s`, `1h02m`, and — for uptimes — `46d07h`. An unknown value is a
//! dash, never a zero (§2.1).

use std::time::{SystemTime, UNIX_EPOCH};

pub const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Bytes per second, for read and growth rates.
pub fn rate(bytes_per_s: f64) -> String {
    format!("{}/s", bytes(bytes_per_s.max(0.0) as u64))
}

pub fn gib(bytes: u64) -> f64 {
    bytes as f64 / GIB
}

/// `-0.0` is what an empty `f64` sum is; on screen it is just 0.
pub fn unsigned_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

pub fn pct(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{:.1}%", unsigned_zero(v)),
        None => "—".to_string(),
    }
}

/// A percentage with no decimals, for places that only need the size of it.
pub fn pct0(value: f64) -> String {
    format!("{:.0}%", unsigned_zero(value))
}

pub fn dur(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    if total < 60 {
        format!("{total}s")
    } else if total < 3600 {
        format!("{}m{:02}s", total / 60, total % 60)
    } else if total < 86_400 {
        format!("{}h{:02}m", total / 3600, (total % 3600) / 60)
    } else {
        format!("{}d{:02}h", total / 86_400, (total % 86_400) / 3600)
    }
}

pub fn opt_dur(seconds: Option<u64>) -> String {
    seconds
        .map(|s| dur(s as f64))
        .unwrap_or_else(|| "—".to_string())
}

/// A forecast: `~2m50s`. The tilde is the point — it is an extrapolation, not a promise, and
/// past ten minutes its seconds are noise: `~18m`.
pub fn eta(seconds: f64) -> String {
    if (600.0..3600.0).contains(&seconds) {
        return format!("~{}m", (seconds / 60.0).round() as u64);
    }
    format!("~{}", dur(seconds))
}

pub fn rows(rows: u64) -> String {
    if rows >= 1_000_000_000 {
        format!("{:.1}B", rows as f64 / 1e9)
    } else if rows >= 1_000_000 {
        format!("{:.1}M", rows as f64 / 1e6)
    } else if rows >= 1_000 {
        format!("{:.1}k", rows as f64 / 1e3)
    } else {
        rows.to_string()
    }
}

pub fn count(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// `1 query`, `3 queries`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// UTC, `HH:MM:SS`, without a date library (§7).
pub fn utc_clock(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

/// Seconds since the epoch back to a clock reading.
pub fn utc_clock_secs(t: f64) -> String {
    utc_clock(UNIX_EPOCH + std::time::Duration::from_secs_f64(t.max(0.0)))
}

/// Cut to `width` display cells with a trailing `…` (§7: truncate, never wrap).
pub fn truncate(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if unicode_width::UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Display width of a string, in terminal cells.
pub fn width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Left-align in `cells`, truncating when it does not fit.
pub fn pad(text: &str, cells: usize) -> String {
    let w = width(text);
    if w > cells {
        return truncate(text, cells);
    }
    format!("{text}{}", " ".repeat(cells - w))
}

/// Right-align in `cells`, truncating when it does not fit.
pub fn right(text: &str, cells: usize) -> String {
    let w = width(text);
    if w > cells {
        return truncate(text, cells);
    }
    format!("{}{text}", " ".repeat(cells - w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_rules_of_section_seven() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(pct(Some(25.34)), "25.3%");
        assert_eq!(pct(None), "—", "never a fake 0%");
        let empty: f64 = Vec::<f64>::new().into_iter().sum();
        assert_eq!(pct(Some(empty)), "0.0%", "an empty sum is -0.0, which is not news");
        assert_eq!(pct0(-0.0), "0%");
        assert_eq!(dur(12.0), "12s");
        assert_eq!(dur(275.0), "4m35s");
        assert_eq!(dur(3720.0), "1h02m");
        assert_eq!(dur(4_000_000.0), "46d07h", "uptimes read in days");
        assert_eq!(opt_dur(None), "—");
        assert_eq!(eta(170.0), "~2m50s");
        assert_eq!(eta(1081.0), "~18m", "a forecast that far out is minutes, not seconds");
        assert_eq!(eta(4000.0), "~1h06m");
        assert_eq!(rows(1_900_000_000), "1.9B");
        assert_eq!(count(1204), "1.2k");
        assert_eq!(rate(152.0 * 1024.0 * 1024.0), "152.0 MiB/s");
        assert_eq!(plural(1, "query", "queries"), "1 query");
        assert_eq!(plural(3, "query", "queries"), "3 queries");
    }

    #[test]
    fn the_clock_is_utc() {
        let at = UNIX_EPOCH + std::time::Duration::from_secs(12 * 3600 + 34 * 60 + 56);
        assert_eq!(utc_clock(at), "12:34:56");
        assert_eq!(utc_clock_secs(12.0 * 3600.0), "12:00:00");
    }

    #[test]
    fn padding_counts_cells_not_bytes() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(right("ab", 4), "  ab");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abc", 0), "");
        // A wide character takes two cells, so it is cut before it overflows.
        assert_eq!(width("日本"), 4);
        assert_eq!(truncate("日本語", 5), "日本…");
        assert_eq!(pad("→", 3), "→  ", "the arrow is one cell");
    }
}
