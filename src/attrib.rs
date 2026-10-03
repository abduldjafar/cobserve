//! Redash attribution: the two regexes of DESIGN.md §6.4 and the display rule for a person.
//!
//! Redash writes a comment into every query it runs, so the ClickHouse user alone never says
//! who is actually behind a query — everything in this fleet is `r_redash`. The SQL in §6.1
//! also extracts these server-side; the same regexes live here for FAKE mode and tests.

use regex::Regex;
use std::sync::OnceLock;

/// `Username: grigol.gankava@paysera.net,` → the address. The comma of the surrounding
/// comment is not part of the capture, which is why the character class stops at it.
fn username_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"Username:\s*([^,]+)").expect("static regex"))
}

fn redash_query_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"query_id:\s*(\d+)").expect("static regex"))
}

/// Paysera's own domain, shown as the bare local part (`grigol.gankava`).
const PAYSERA_DOMAIN: &str = "paysera.net";

/// The ClickHouse user that means "somebody through Redash" — the only one worth trying to
/// resolve to a person at all (§6.4).
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

/// Display form of a person: the local part for @paysera.net, the whole address otherwise,
/// because a partner's domain is the only way to tell two grigols apart.
pub fn display_person(address: &str) -> String {
    match address.split_once('@') {
        Some((local, domain)) if domain.eq_ignore_ascii_case(PAYSERA_DOMAIN) => {
            local.to_string()
        }
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

    const REDASH_COMMENT: &str = "/* Application: Redash */ /* Username: grigol.gankava@paysera.net, Redash query_id: 7438, Redash: */";

    #[test]
    fn person_from_a_real_shaped_comment() {
        assert_eq!(
            person_address(REDASH_COMMENT).as_deref(),
            Some("grigol.gankava@paysera.net")
        );
        assert_eq!(display_person("grigol.gankava@paysera.net"), "grigol.gankava");
    }

    #[test]
    fn person_stops_at_the_trailing_comma() {
        let sql = "/* Username: j.petrova@paysera.net, Redash query_id: 8585 */ SELECT 1";
        assert_eq!(person_address(sql).as_deref(), Some("j.petrova@paysera.net"));
    }

    #[test]
    fn person_without_a_comma_still_parses() {
        let sql = "/* Username: m.kairys@paysera.net */";
        assert_eq!(person_address(sql).as_deref(), Some("m.kairys@paysera.net"));
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
        assert_eq!(redash_query_id("/* Username: j.petrova@paysera.net */"), None);
        assert_eq!(redash_query_id("SELECT 1"), None);
    }


    #[test]
    fn labels_fall_back_to_the_bare_user() {
        assert_eq!(user_label("r_redash", Some("grigol.gankava")), "r_redash → grigol.gankava");
        assert_eq!(user_label("airflow", None), "airflow");
        assert_eq!(user_label("r_redash", None), "r_redash");
    }

    #[test]
    fn multiple_username_comments_take_the_first() {
        let sql = "/* Username: first@paysera.net, */ /* Username: second@paysera.net, */";
        assert_eq!(person_address(sql).as_deref(), Some("first@paysera.net"));
    }
}