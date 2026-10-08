//! Where the data comes from. Each source is a tokio task that pushes `Event`s into the
//! loop; nothing here touches `App` or the UI.

pub mod airflow;
pub mod clickhouse;
pub mod jira;
pub mod limits;
pub mod local;
pub mod usage;
pub mod redash;

use std::time::{Duration, UNIX_EPOCH};

/// Why a request got no answer, in the words the screen uses for every source. reqwest's own
/// text is `error sending request for url (…)` for each of these; the cause is further down its
/// source chain. Never with the URL: a query string can carry what is not to be shown (§9).
pub fn transport(error: reqwest::Error, timeout: Duration) -> String {
    let mut chain = String::new();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        chain.push_str(&cause.to_string());
        chain.push(' ');
        source = cause.source();
    }
    let timed_out = error.is_timeout().then_some(timeout);
    reason(&chain, timed_out, error.is_connect())
        .unwrap_or_else(|| error.without_url().to_string().chars().take(200).collect())
}

/// The cause behind a transport error, from the text of its source chain: `timeout` when the
/// request ran out of time, and how long it had.
pub fn reason(chain: &str, timeout: Option<Duration>, connect: bool) -> Option<String> {
    let text = chain.to_ascii_lowercase();
    if let Some(limit) = timeout {
        let secs = limit.as_secs_f64();
        return Some(if secs.fract() == 0.0 {
            format!("no answer within {secs:.0} s")
        } else {
            format!("no answer within {secs:.1} s")
        });
    }
    let said = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    if said(&["dns error", "failed to lookup address", "name or service not known", "nodename nor servname", "no such host"]) {
        return Some("name does not resolve from here (DNS)".to_string());
    }
    if said(&["connection refused"]) {
        return Some("connection refused — nothing listening on that port".to_string());
    }
    if said(&["no route to host", "network is unreachable", "host is unreachable"]) {
        return Some("no route to host".to_string());
    }
    if said(&["connection reset"]) {
        return Some("connection reset".to_string());
    }
    if said(&["certificate", "tls", "ssl"]) {
        return Some("TLS handshake failed".to_string());
    }
    connect.then(|| "cannot connect".to_string())
}

/// One path segment of a URL: everything but letters, digits and `-._~` escaped, so a DAG run's
/// id (`scheduled__2026-10-05T06:00:00+00:00`) stays one segment and its `+` a plus.
pub fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A timestamp as Jira (`2026-10-02T04:15:43.000+0300`) or Airflow
/// (`2026-10-05T15:15:37.660939+00:00`) writes it, in Unix seconds.
pub fn unix(text: &str) -> Option<i64> {
    let at = redash::parse_rfc3339_seconds(text)?;
    at.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// The host of a base URL, for a title line: `https://jira.example.net/` → `jira.example.net`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    authority.rsplit_once('@').map_or(authority, |(_, host)| host).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_stays_one_segment() {
        assert_eq!(segment("scheduled__2026-10-05T06:00:00+00:00"), "scheduled__2026-10-05T06%3A00%3A00%2B00%3A00");
        assert_eq!(segment("ACCOUNTING_MARTS_MV"), "ACCOUNTING_MARTS_MV");
        assert_eq!(segment("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn both_ways_of_writing_a_zone_are_read() {
        // 2026-10-02 01:15:43 UTC, written by Jira in +03:00 without the colon.
        assert_eq!(unix("2026-10-02T04:15:43.000+0300"), Some(1_790_903_743));
        assert_eq!(unix("2026-10-02T01:15:43.123456+00:00"), Some(1_790_903_743));
        assert_eq!(unix("2026-10-02T01:15:43Z"), Some(1_790_903_743));
        assert_eq!(unix("not a time"), None);
    }

    #[test]
    fn a_timeout_says_how_long_it_waited() {
        assert_eq!(reason("", Some(Duration::from_millis(1500)), false).as_deref(), Some("no answer within 1.5 s"));
        assert_eq!(reason("", Some(Duration::from_secs(15)), false).as_deref(), Some("no answer within 15 s"));
        assert_eq!(reason("dns error: failed to lookup address", None, true).as_deref(), Some("name does not resolve from here (DNS)"));
        assert_eq!(reason("", None, false), None);
    }

    #[test]
    fn the_host_is_what_the_title_shows() {
        assert_eq!(host_of("https://jira.example.net/"), "jira.example.net");
        assert_eq!(host_of("https://airflow.example.net:8443/x"), "airflow.example.net:8443");
        assert_eq!(host_of("jira.example.net"), "jira.example.net");
    }
}
