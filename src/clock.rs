//! The time on screen. The machine's own zone by default — followed as it changes, as a laptop
//! that travels or a zone changed in the settings does — or UTC, the servers' own: `z` flips
//! between them. Prayer times are always the place's own local time.

use chrono::{DateTime, FixedOffset, Local, Offset, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shown {
    #[default]
    Local,
    Utc,
}

/// How times are shown, and what the machine's zone is called.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clock {
    pub shown: Shown,
    /// The zone's name (`Asia/Jakarta`): `TZ` when it names one, else the system's. `None`
    /// until asked, and where neither says.
    pub zone: Option<String>,
    /// A zone that stands for the machine's — for the tests and the screenshots, which cannot
    /// depend on where they run.
    pub pinned: Option<FixedOffset>,
}

impl Clock {
    /// `HH:MM:SS` at `at` (Unix seconds), in the zone shown.
    pub fn hms(&self, at: i64) -> String {
        self.format(at, "%H:%M:%S")
    }

    /// `HH:MM` in the machine's own zone, whatever is shown — prayer times are local.
    pub fn local_hm(&self, at: i64) -> String {
        self.local(at, "%H:%M")
    }

    fn format(&self, at: i64, pattern: &str) -> String {
        match self.shown {
            Shown::Utc => Utc.timestamp_opt(at, 0).single().map(|t| t.format(pattern).to_string()).unwrap_or_default(),
            Shown::Local => self.local(at, pattern),
        }
    }

    /// `at` in the machine's zone, or the one standing for it.
    fn local(&self, at: i64, pattern: &str) -> String {
        match self.pinned {
            Some(zone) => zone.timestamp_opt(at, 0).single().map(|t| t.format(pattern).to_string()),
            None => local(at).map(|t| t.format(pattern).to_string()),
        }
        .unwrap_or_default()
    }

    /// What the zone shown is called at `at`: `UTC`, `WIB`, `WITA`, `WIT`, else its offset
    /// (`UTC+5:30`).
    pub fn label(&self, at: i64) -> String {
        match self.shown {
            Shown::Utc => "UTC".to_string(),
            Shown::Local => self.local_label(at),
        }
    }

    /// The machine's zone, by the name Indonesia gives it where it has one.
    pub fn local_label(&self, at: i64) -> String {
        let offset = match self.pinned {
            Some(zone) => zone.local_minus_utc(),
            None => local(at).map_or(0, |t| t.offset().fix().local_minus_utc()),
        };
        abbreviation(self.zone.as_deref(), offset)
    }

    /// The other one: local ↔ UTC.
    pub fn flip(&mut self) {
        self.shown = match self.shown {
            Shown::Local => Shown::Utc,
            Shown::Utc => Shown::Local,
        };
    }
}

fn local(at: i64) -> Option<DateTime<Local>> {
    Local.timestamp_opt(at, 0).single()
}

/// `Asia/Makassar` → `WITA`; a zone without a name here is its offset from UTC.
pub fn abbreviation(zone: Option<&str>, offset_s: i32) -> String {
    let named = match zone {
        Some("Asia/Jakarta" | "Asia/Pontianak") if offset_s == 7 * 3600 => Some("WIB"),
        Some("Asia/Makassar") if offset_s == 8 * 3600 => Some("WITA"),
        Some("Asia/Jayapura") if offset_s == 9 * 3600 => Some("WIT"),
        _ => None,
    };
    if let Some(name) = named {
        return name.to_string();
    }
    if offset_s == 0 {
        return "UTC".to_string();
    }
    let sign = if offset_s < 0 { '-' } else { '+' };
    let (hours, minutes) = (offset_s.abs() / 3600, offset_s.abs() % 3600 / 60);
    if minutes == 0 { format!("UTC{sign}{hours}") } else { format!("UTC{sign}{hours}:{minutes:02}") }
}

/// The machine's zone by name, as it is now: `TZ` when it names one (`Asia/Jakarta`, not
/// `WIB-7`), else what the system says.
pub fn zone_now() -> Option<String> {
    let from_env = std::env::var("TZ").ok().map(|tz| tz.trim_start_matches(':').to_string());
    if let Some(tz) = from_env.filter(|tz| tz.contains('/') && !tz.starts_with('/')) {
        return Some(tz);
    }
    iana_time_zone::get_timezone().ok()
}

/// The system's list of zones with the coordinates of each one's city, for prayer times where
/// none was chosen.
pub fn zone_table() -> Option<String> {
    ["/usr/share/zoneinfo/zone1970.tab", "/usr/share/zoneinfo/zone.tab", "/usr/share/zoneinfo.default/zone.tab"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indonesia_s_zones_have_their_names_and_the_rest_their_offsets() {
        assert_eq!(abbreviation(Some("Asia/Jakarta"), 7 * 3600), "WIB");
        assert_eq!(abbreviation(Some("Asia/Pontianak"), 7 * 3600), "WIB");
        assert_eq!(abbreviation(Some("Asia/Makassar"), 8 * 3600), "WITA");
        assert_eq!(abbreviation(Some("Asia/Jayapura"), 9 * 3600), "WIT");
        assert_eq!(abbreviation(Some("Asia/Kolkata"), 5 * 3600 + 1800), "UTC+5:30");
        assert_eq!(abbreviation(Some("America/New_York"), -4 * 3600), "UTC-4");
        assert_eq!(abbreviation(None, 0), "UTC");
        // A zone whose offset is not the one its name is known by is not called by that name.
        assert_eq!(abbreviation(Some("Asia/Jakarta"), 8 * 3600), "UTC+8");
    }

    #[test]
    fn utc_is_utc_and_flips_back() {
        let mut clock = Clock { shown: Shown::Utc, ..Clock::default() };
        assert_eq!(clock.hms(12 * 3600 + 34 * 60 + 56), "12:34:56");
        assert_eq!(clock.label(0), "UTC");
        clock.flip();
        assert_eq!(clock.shown, Shown::Local);
        clock.flip();
        assert_eq!(clock.shown, Shown::Utc);
    }

    #[test]
    fn a_pinned_zone_stands_for_the_machine_s() {
        let clock = Clock { shown: Shown::Local, zone: Some("Asia/Makassar".into()), pinned: FixedOffset::east_opt(8 * 3600) };
        assert_eq!(clock.hms(0), "08:00:00");
        assert_eq!(clock.label(0), "WITA");
        assert_eq!(clock.local_hm(3600), "09:00");
        let utc = Clock { shown: Shown::Utc, ..clock };
        assert_eq!((utc.hms(0).as_str(), utc.local_hm(0).as_str()), ("00:00:00", "08:00"), "prayer times stay local");
    }
}
