//! The severity table of DESIGN.md §7, shared by the screen, the insights and the event tape.
//!
//! It used to live in the UI. It moved here once the insights and the tape started asking
//! the same question ("is 78% of memory a problem?"): one table, one answer, wherever the
//! number is shown or reasoned about.

/// Ordered from harmless to worst, so `max` picks the worst of several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Nothing to say about it.
    None,
    /// Explicitly fine — a recovery, a healthy summary.
    Ok,
    /// Worth knowing, not worth acting on.
    Info,
    /// Amber (§7).
    Warn,
    /// Red (§7).
    Crit,
}

impl Severity {
    /// The glyph that carries the severity when colour cannot (NO_COLOR, a screenshot in a
    /// ticket). All of them are one cell wide in every terminal font we have seen; `⚠` is
    /// not used because several terminals draw it as a two-cell emoji and shift the row.
    pub fn glyph(self) -> &'static str {
        match self {
            Severity::None => " ",
            Severity::Ok => "✔",
            Severity::Info => "●",
            Severity::Warn => "▲",
            Severity::Crit => "✖",
        }
    }

    /// The mark after a number in a table: ` ▲` or ` ✖`, nothing when fine.
    pub fn mark(self) -> &'static str {
        match self {
            Severity::Warn => " ▲",
            Severity::Crit => " ✖",
            _ => "",
        }
    }

    pub fn is_problem(self) -> bool {
        self >= Severity::Warn
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::None | Severity::Ok | Severity::Info => "HEALTHY",
            Severity::Warn => "DEGRADED",
            Severity::Crit => "CRITICAL",
        }
    }
}

/// Node memory and CPU: amber at 75, red at 90 (§7).
pub fn node(value: Option<f64>) -> Severity {
    match value {
        Some(v) if v >= 90.0 => Severity::Crit,
        Some(v) if v >= 75.0 => Severity::Warn,
        _ => Severity::None,
    }
}

/// A user's share of one node: amber at 20, red at 40 (§7).
pub fn user_share(value: Option<f64>) -> Severity {
    match value {
        Some(v) if v >= 40.0 => Severity::Crit,
        Some(v) if v >= 20.0 => Severity::Warn,
        _ => Severity::None,
    }
}

/// Query elapsed: amber at 5 s, red at 30 s — which is also the runaway mark of §5.4.
pub fn elapsed(seconds: f64) -> Severity {
    if seconds >= 30.0 {
        Severity::Crit
    } else if seconds >= 5.0 {
        Severity::Warn
    } else {
        Severity::None
    }
}

/// Replica lag: amber at 10 s, red at 60 s (§7).
pub fn lag(seconds: u64) -> Severity {
    if seconds >= 60 {
        Severity::Crit
    } else if seconds >= 10 {
        Severity::Warn
    } else {
        Severity::None
    }
}

/// Queue waits: amber at 60 s or when every worker is busy, red at 180 s (§7).
pub fn wait(seconds: u64, saturated: bool) -> Severity {
    if seconds >= 180 {
        Severity::Crit
    } else if seconds >= 60 || saturated {
        Severity::Warn
    } else {
        Severity::None
    }
}

/// A query's memory against its own limit: amber from the runaway fraction (§5.4), red when
/// ClickHouse is about to kill it.
pub fn limit_share(fraction: f64) -> Severity {
    if fraction >= 0.95 {
        Severity::Crit
    } else if fraction >= crate::model::RUNAWAY_MEMORY_FRACTION {
        Severity::Warn
    } else {
        Severity::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_matches_the_table() {
        assert_eq!(node(Some(74.9)), Severity::None);
        assert_eq!(node(Some(75.0)), Severity::Warn);
        assert_eq!(node(Some(90.0)), Severity::Crit);
        assert_eq!(node(None), Severity::None, "unknown is not a problem we can see");
        assert_eq!(user_share(Some(19.9)), Severity::None);
        assert_eq!(user_share(Some(20.0)), Severity::Warn);
        assert_eq!(user_share(Some(40.0)), Severity::Crit);
        assert_eq!(elapsed(4.9), Severity::None);
        assert_eq!(elapsed(5.0), Severity::Warn);
        assert_eq!(elapsed(30.0), Severity::Crit);
        assert_eq!(lag(9), Severity::None);
        assert_eq!(lag(10), Severity::Warn);
        assert_eq!(lag(60), Severity::Crit);
        assert_eq!(wait(59, false), Severity::None);
        assert_eq!(wait(10, true), Severity::Warn, "all workers busy is amber");
        assert_eq!(wait(180, false), Severity::Crit);
        assert_eq!(limit_share(0.79), Severity::None);
        assert_eq!(limit_share(0.8), Severity::Warn);
        assert_eq!(limit_share(0.96), Severity::Crit);
    }

    #[test]
    fn the_worst_of_several_is_the_max() {
        let worst = [Severity::Info, Severity::Crit, Severity::Warn]
            .into_iter()
            .max()
            .unwrap();
        assert_eq!(worst, Severity::Crit);
        assert!(Severity::Warn.is_problem());
        assert!(!Severity::Info.is_problem());
        assert_eq!(Severity::Crit.label(), "CRITICAL");
    }
}
