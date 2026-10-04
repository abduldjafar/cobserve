//! Reading SQL for people: what a query *is* in a few words, and its text without the noise.
//!
//! A query row has room for about twenty characters. `c3e51cb5` says nothing to on call;
//! `SELECT · accounting_lt.bank_record` says which table is being scanned, which is usually
//! the whole story. None of this parses SQL properly — it does not need to: it only has to be
//! right about the first verb and the first real table, and fall back to nothing otherwise.

/// `/* … */` and `-- …` removed. Redash prefixes every query with a comment that §6.4 already
/// turned into the person column, so showing it again in the SQL preview is noise.
pub fn strip_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_string: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(quote) = in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == quote {
                in_string = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                in_string = Some(c);
                out.push(c);
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for inner in chars.by_ref() {
                    if prev == '*' && inner == '/' {
                        break;
                    }
                    prev = inner;
                }
                out.push(' ');
            }
            '-' if chars.peek() == Some(&'-') => {
                for inner in chars.by_ref() {
                    if inner == '\n' {
                        break;
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Whitespace runs become one space (§2.3).
pub fn collapse(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The SQL as the drawer shows it: no comments, one line.
pub fn preview(sql: &str) -> String {
    let stripped = collapse(&strip_comments(sql));
    if stripped.is_empty() {
        // A query that is only a comment is still a query; show what there is.
        collapse(sql)
    } else {
        stripped
    }
}

/// What a query is: its verb and the first real table it touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub verb: &'static str,
    pub target: Option<String>,
}

impl Summary {
    /// `SELECT · accounting_lt.bank_record`, or just `SELECT`.
    pub fn label(&self) -> String {
        match &self.target {
            Some(target) => format!("{} · {target}", self.verb),
            None => self.verb.to_string(),
        }
    }
}

const VERBS: &[(&str, &str)] = &[
    ("SELECT", "SELECT"),
    ("WITH", "SELECT"),
    ("INSERT", "INSERT"),
    ("EXPLAIN", "EXPLAIN"),
    ("CREATE", "CREATE"),
    ("ALTER", "ALTER"),
    ("OPTIMIZE", "OPTIMIZE"),
    ("SYSTEM", "SYSTEM"),
    ("DROP", "DROP"),
    ("TRUNCATE", "TRUNCATE"),
    ("SHOW", "SHOW"),
    ("DESCRIBE", "DESCRIBE"),
    ("DESC", "DESCRIBE"),
    ("KILL", "KILL"),
    ("DELETE", "DELETE"),
    ("RENAME", "RENAME"),
];

/// Words: identifiers (with their dots and quotes) and single punctuation characters. Strings
/// are skipped whole so a `'FROM'` inside a literal is never taken for a clause.
fn words(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = sql.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '\'' {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == '\\' {
                    chars.next();
                } else if inner == '\'' {
                    break;
                }
            }
            out.push("''".to_string());
        } else if c.is_alphanumeric() || c == '_' || c == '`' || c == '"' || c == '.' {
            let mut word = String::new();
            while let Some(&inner) = chars.peek() {
                if inner.is_alphanumeric() || inner == '_' || inner == '`' || inner == '"' || inner == '.' {
                    word.push(inner);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(word);
        } else {
            out.push(c.to_string());
            chars.next();
        }
    }
    out
}

fn unquote(identifier: &str) -> String {
    identifier.replace(['`', '"'], "")
}

pub fn summary(sql: &str) -> Summary {
    let words = words(&strip_comments(sql));
    let first = words
        .iter()
        .find(|w| w.chars().next().is_some_and(char::is_alphabetic))
        .map(|w| w.to_ascii_uppercase());
    let verb = first
        .as_deref()
        .and_then(|w| VERBS.iter().find(|(word, _)| *word == w))
        .map(|(_, verb)| *verb)
        .unwrap_or("QUERY");

    // Names defined by WITH … AS ( are not tables; the table is whatever they read from.
    let mut ctes: Vec<String> = Vec::new();
    for window in words.windows(3) {
        if window[1].eq_ignore_ascii_case("AS") && window[2] == "(" {
            ctes.push(unquote(&window[0]).to_ascii_lowercase());
        }
    }

    let clause = if verb == "INSERT" { "INTO" } else { "FROM" };
    let mut target = None;
    for (i, word) in words.iter().enumerate() {
        if !word.eq_ignore_ascii_case(clause) {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            break;
        };
        if next == "(" || next == "''" {
            continue; // a subquery or a literal; its own FROM comes later
        }
        let name = unquote(next);
        if name.is_empty() || ctes.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        // `numbers(…)`, `clusterAllReplicas(…)`, `s3(…)`: a table function. Its name says what
        // is being read; its arguments would not fit anyway.
        let is_function = words.get(i + 2).is_some_and(|w| w == "(");
        target = Some(if is_function { format!("{name}()") } else { name });
        break;
    }
    Summary { verb, target }
}

/// How a piece of SQL is drawn in the drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Number,
    Plain,
}

const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "PREWHERE", "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET",
    "WITH", "AS", "AND", "OR", "NOT", "IN", "IS", "NULL", "JOIN", "LEFT", "RIGHT", "INNER",
    "OUTER", "FULL", "CROSS", "ON", "USING", "UNION", "ALL", "DISTINCT", "INSERT", "INTO",
    "VALUES", "CREATE", "TABLE", "ALTER", "DROP", "OPTIMIZE", "FINAL", "SETTINGS", "FORMAT",
    "CASE", "WHEN", "THEN", "ELSE", "END", "INTERVAL", "LIKE", "ILIKE", "BETWEEN", "EXPLAIN",
    "SAMPLE", "ARRAY", "GLOBAL", "ANY", "ASC", "DESC", "SYSTEM", "KILL", "QUERY", "DELETE",
    "UPDATE", "DAY", "HOUR", "MINUTE", "SECOND", "WEEK", "MONTH", "YEAR",
];

/// SQL cut into coloured pieces. Joining the pieces gives back the input exactly.
pub fn highlight(sql: &str) -> Vec<(String, Token)> {
    let mut out: Vec<(String, Token)> = Vec::new();
    let mut push = |text: String, token: Token| {
        if let Some(last) = out.last_mut()
            && last.1 == token
        {
            last.0.push_str(&text);
            return;
        }
        out.push((text, token));
    };
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '\'' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            push(chars[start..i.min(chars.len())].iter().collect(), Token::String);
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            push(chars[start..i].iter().collect(), Token::Number);
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let token = if KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(&word)) {
                Token::Keyword
            } else {
                Token::Plain
            };
            push(word, token);
        } else {
            push(c.to_string(), Token::Plain);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDASH: &str = "/* Application: Redash */ /* Username: grigol.gankava@example.net, Redash query_id: 7438, Redash: 10.1.0 */ WITH BankRecord AS (\n  SELECT BillOpId, Bank FROM accounting_lt.bank_record\n  WHERE EventDate >= today() - 30\n)\nSELECT region, count() FROM BankRecord GROUP BY region";

    #[test]
    fn comments_are_stripped_but_strings_are_not() {
        assert_eq!(
            collapse(&strip_comments("/* a */ SELECT '/* not a comment */' -- tail\nFROM t")),
            "SELECT '/* not a comment */' FROM t"
        );
        assert_eq!(preview(REDASH).split_whitespace().next(), Some("WITH"));
        assert_eq!(preview("/* only a comment */"), "/* only a comment */");
    }

    #[test]
    fn the_summary_finds_the_real_table_behind_a_cte() {
        let s = summary(REDASH);
        assert_eq!(s.verb, "SELECT");
        assert_eq!(s.target.as_deref(), Some("accounting_lt.bank_record"));
        assert_eq!(s.label(), "SELECT · accounting_lt.bank_record");
    }

    #[test]
    fn the_summary_of_inserts_and_table_functions() {
        let insert = summary("INSERT INTO statistics.daily_rollup SELECT d FROM accounting.raw");
        assert_eq!((insert.verb, insert.target.as_deref()), ("INSERT", Some("statistics.daily_rollup")));

        let function = summary(
            "SELECT host_name FROM clusterAllReplicas('ch_cluster', system.parts) GROUP BY host_name",
        );
        assert_eq!(function.target.as_deref(), Some("clusterAllReplicas()"));

        let explain = summary("EXPLAIN SELECT count() FROM payments.not_initiated");
        assert_eq!((explain.verb, explain.target.as_deref()), ("EXPLAIN", Some("payments.not_initiated")));

        let quoted = summary("SELECT * FROM `wallet`.\"ledger\"");
        assert_eq!(quoted.target.as_deref(), Some("wallet.ledger"), "quotes are dropped");

        let subquery = summary("SELECT * FROM (SELECT * FROM wallet.ledger) WHERE 1");
        assert_eq!(subquery.target.as_deref(), Some("wallet.ledger"));

        let nothing = summary("SELECT 1");
        assert_eq!((nothing.verb, nothing.target), ("SELECT", None));
        assert_eq!(summary("").verb, "QUERY");
    }

    #[test]
    fn a_from_inside_a_string_is_not_a_clause() {
        let s = summary("SELECT 'FROM fake.table' AS x FROM real.table");
        assert_eq!(s.target.as_deref(), Some("real.table"));
    }

    #[test]
    fn highlighting_is_lossless_and_finds_keywords() {
        let sql = "SELECT count() FROM t WHERE x = 'it''s' AND n > 42";
        let pieces = highlight(sql);
        let joined: String = pieces.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, sql);
        assert!(pieces.iter().any(|(t, k)| t == "SELECT" && *k == Token::Keyword));
        assert!(pieces.iter().any(|(t, k)| t == "42" && *k == Token::Number));
        assert!(pieces.iter().any(|(_, k)| *k == Token::String));
        assert!(pieces.iter().any(|(t, k)| t.contains("count") && *k == Token::Plain));
    }
}
