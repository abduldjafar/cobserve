//! Redash attribution: the two regexes of DESIGN.md §6.4 and the display rule for a person.
//!
//! Redash writes a comment into every query it runs, so the ClickHouse user alone never says
//! who is actually behind a query — every query from Redash runs as the one account it
//! connects with. The SQL in §6.1 also extracts these server-side; the same regexes live here
//! for the fallback, FAKE mode and tests.

use regex::Regex;
use std::sync::OnceLock;

/// `Username: grigol.gankava@example.net,` → the address. The comma of the surrounding
/// comment is not part of the capture, which is why the character class stops at it.
fn username_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"Username:\s*([^,]+)").expect("static regex"))
}

/// `query_id: 7438` as Redash 10 writes it, `Query ID: 7438` as older ones did for scheduled
/// runs.
fn redash_query_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)query[ _]id:\s*(\d+)").expect("static regex"))
}

/// `Job ID: 4c8d…` — the RQ job that ran the query, which Redash adds to the comment as it
/// starts it. It is the one exact link between a job in the queue and a ClickHouse process.
fn redash_job_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)job[ _]id:\s*([A-Za-z0-9_-]+)").expect("static regex"))
}

/// The organisation's own e-mail domain — `EMAIL_DOMAIN`, or `email_domain:` under `redash:`
/// in the credential file. Its people are shown by the local part alone (`grigol.gankava`);
/// without it every address is shown whole. Set once at startup, before any source runs.
static HOME_DOMAIN: OnceLock<Option<String>> = OnceLock::new();

pub fn set_home_domain(domain: Option<String>) {
    let domain = domain
        .map(|d| d.trim().trim_start_matches('@').to_string())
        .filter(|d| !d.is_empty());
    let _ = HOME_DOMAIN.set(domain);
}

fn home_domain() -> Option<&'static str> {
    // Tests run against the fake fleet, whose people are at its domain — as FAKE=1 does.
    if cfg!(test) {
        return Some(crate::fake::DOMAIN);
    }
    HOME_DOMAIN.get().and_then(|d| d.as_deref())
}

/// The account Redash connects to ClickHouse with in the design's fleet (§6.4) — and in the
/// fake one. Attribution itself goes by the comment, whatever the account.
pub const REDASH_USER: &str = "r_redash";

/// The address Redash put in the comment, if any.
///
/// `[^,]+` is FleetLens' character class and it is right for the real comment, which always
/// continues `…, Redash query_id: N, …`. A comment with no comma after the address would
/// otherwise swallow the `*/` that ends it, so that is trimmed off here.
pub fn person_address(sql: &str) -> Option<String> {
    username_re()
        .captures(sql)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .trim()
                .trim_end_matches("*/")
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
}

/// The Redash query number from the comment. Queries run by hand have no `query_id:`.
pub fn redash_query_id(sql: &str) -> Option<u64> {
    redash_query_id_re()
        .captures(sql)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().trim().parse::<u64>().ok())
}

/// The RQ job id from the comment, when Redash wrote one.
pub fn redash_job_id(sql: &str) -> Option<&str> {
    redash_job_id_re().captures(sql).and_then(|c| c.get(1)).map(|m| m.as_str())
}

/// Display form of a person: the local part at the home domain, the whole address otherwise,
/// because a partner's domain is the only way to tell two grigols apart.
pub fn display_person(address: &str) -> String {
    display_person_at(address, home_domain())
}

fn display_person_at(address: &str, home: Option<&str>) -> String {
    match (address.split_once('@'), home) {
        (Some((local, domain)), Some(home)) if domain.eq_ignore_ascii_case(home) => local.to_string(),
        _ => address.to_string(),
    }
}

/// What a user row shows in its first column: `r_redash → grigol.gankava`, or just the user
/// when attribution finds nothing and the user is not the shared Redash account.
pub fn user_label(user: &str, person: Option<&str>) -> String {
    match person.map(display_person) {
        Some(p) if !p.is_empty() => format!("{user} → {p}"),
        _ => user.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Attribution as §6.4 stores it on a row is `model::attribution_from_sql`, which combines
    // these regexes with what the server already extracted. The pieces are tested here.

    const REDASH_COMMENT: &str = "/* Application: Redash */ /* Username: grigol.gankava@example.net, Redash query_id: 7438, Redash: */";

    #[test]
    fn person_from_a_real_shaped_comment() {
        assert_eq!(
            person_address(REDASH_COMMENT).as_deref(),
            Some("grigol.gankava@example.net")
        );
        assert_eq!(display_person("grigol.gankava@example.net"), "grigol.gankava");
    }

    #[test]
    fn person_stops_at_the_trailing_comma() {
        let sql = "/* Username: j.petrova@example.net, Redash query_id: 8585 */ SELECT 1";
        assert_eq!(person_address(sql).as_deref(), Some("j.petrova@example.net"));
    }

    #[test]
    fn person_without_a_comma_still_parses() {
        let sql = "/* Username: m.kairys@example.net */";
        assert_eq!(person_address(sql).as_deref(), Some("m.kairys@example.net"));
    }

    #[test]
    fn without_a_home_domain_every_address_is_whole() {
        assert_eq!(display_person_at("grigol.gankava@example.net", None), "grigol.gankava@example.net");
        assert_eq!(display_person_at("grigol.gankava@Example.NET", Some("example.net")), "grigol.gankava");
        assert_eq!(display_person_at("grigol.gankava", Some("example.net")), "grigol.gankava", "no @ at all");
    }

    #[test]
    fn foreign_domain_keeps_the_whole_address() {
        assert_eq!(
            display_person("kontsultant@partner.example"),
            "kontsultant@partner.example"
        );
        assert_eq!(
            person_address("/* Username: erik@partner.example, */"),
            Some("erik@partner.example".to_string())
        );
    }

    #[test]
    fn no_username_means_no_person() {
        assert_eq!(person_address("SELECT count() FROM wallet_log"), None);
        assert_eq!(person_address("/* Username:   */"), None);
    }

    #[test]
    fn redash_query_id_present_and_absent() {
        assert_eq!(redash_query_id(REDASH_COMMENT), Some(7438));
        assert_eq!(redash_query_id("/* Username: j.petrova@example.net */"), None);
        assert_eq!(redash_query_id("SELECT 1"), None);
    }

    /// What Redash 10 actually prepends: the job id is in it, and scheduled runs say so.
    const REDASH_10: &str = "/* Username: j.petrova@example.net, query_id: 8585, Queue: queries, Job ID: 4c8d1b5e-2f3a-4e1b-9d7c-0a1b2c3d4e5f, Query Hash: 1a2b3c, Scheduled: False */ SELECT 1";

    #[test]
    fn the_job_id_and_both_spellings_of_the_query_id() {
        assert_eq!(redash_job_id(REDASH_10), Some("4c8d1b5e-2f3a-4e1b-9d7c-0a1b2c3d4e5f"));
        assert_eq!(redash_query_id(REDASH_10), Some(8585));
        assert_eq!(person_address(REDASH_10).as_deref(), Some("j.petrova@example.net"));
        assert_eq!(redash_query_id("/* Query ID: 7438, Username: Scheduled */"), Some(7438));
        assert_eq!(redash_job_id(REDASH_COMMENT), None, "the design's comment has no job id");
        assert_eq!(redash_job_id("SELECT 1"), None);
    }

    #[test]
    fn labels_fall_back_to_the_bare_user() {
        assert_eq!(user_label("r_redash", Some("grigol.gankava")), "r_redash → grigol.gankava");
        assert_eq!(user_label("airflow", None), "airflow");
        assert_eq!(user_label("r_redash", None), "r_redash");
    }

    #[test]
    fn multiple_username_comments_take_the_first() {
        let sql = "/* Username: first@example.net, */ /* Username: second@example.net, */";
        assert_eq!(person_address(sql).as_deref(), Some("first@example.net"));
    }
}