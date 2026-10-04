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

/// The SQL laid out for reading in a block `width` cells wide: comments gone, a one-line query
/// broken before its clauses, every line wrapped at word boundaries, each piece coloured.
/// Lines keep the indentation the author gave them; a wrapped line starts at the margin.
pub fn layout(sql: &str, width: usize) -> Vec<Vec<(String, Token)>> {
    let width = width.max(8);
    let mut lines: Vec<Vec<(String, Token)>> = vec![Vec::new()];
    let mut col = 0usize;
    let mut line_start = true;
    let push = |lines: &mut Vec<Vec<(String, Token)>>, text: &str, token: Token| {
        let line = lines.last_mut().expect("there is always a line");
        match line.last_mut() {
            Some(last) if last.1 == token => last.0.push_str(text),
            _ => line.push((text.to_string(), token)),
        }
    };
    for (piece, token) in highlight(&readable(sql)) {
        for atom in atoms(&piece) {
            if atom == "\n" {
                lines.push(Vec::new());
                col = 0;
                line_start = true;
                continue;
            }
            if atom.starts_with([' ', '\t']) {
                if line_start {
                    let indent = atom.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>().min(width / 2);
                    push(&mut lines, &" ".repeat(indent), Token::Plain);
                    col += indent;
                } else if col > 0 && col < width {
                    push(&mut lines, " ", Token::Plain);
                    col += 1;
                }
                continue;
            }
            line_start = false;
            let mut rest = atom;
            while !rest.is_empty() {
                let w = crate::fmt::width(rest);
                if col + w > width && col > 0 {
                    trim_end(lines.last_mut().expect("there is always a line"));
                    lines.push(Vec::new());
                    col = 0;
                }
                if w <= width - col {
                    push(&mut lines, rest, token);
                    col += w;
                    break;
                }
                // Longer than a whole line: cut it where the line ends.
                let mut taken = String::new();
                let mut used = 0;
                for c in rest.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    if used + cw > width - col {
                        break;
                    }
                    taken.push(c);
                    used += cw;
                }
                if taken.is_empty() {
                    taken = rest.chars().next().map(String::from).unwrap_or_default();
                }
                push(&mut lines, &taken, token);
                col += used.max(1);
                rest = &rest[taken.len()..];
            }
        }
    }
    for line in &mut lines {
        trim_end(line);
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

fn trim_end(line: &mut Vec<(String, Token)>) {
    while let Some(last) = line.last_mut() {
        let trimmed = last.0.trim_end().len();
        if trimmed == 0 {
            line.pop();
        } else {
            last.0.truncate(trimmed);
            break;
        }
    }
}

/// A piece cut into what wrapping cares about: each line break, each run of blanks, each run
/// of anything else.
fn atoms(piece: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let kind = |c: char| match c {
        '\n' => 0,
        ' ' | '\t' => 1,
        _ => 2,
    };
    let chars: Vec<(usize, char)> = piece.char_indices().collect();
    for (n, &(at, c)) in chars.iter().enumerate() {
        let next = chars.get(n + 1);
        let ends = match next {
            None => true,
            Some(&(_, d)) => kind(c) == 0 || kind(c) != kind(d),
        };
        if ends {
            let end = next.map_or(piece.len(), |&(i, _)| i);
            out.push(&piece[start..end]);
            start = end;
        }
        let _ = at;
    }
    out
}

/// The query as text to read: its own line breaks when it has them (minus the indentation
/// every line shares), or — for a query sent as one long line — a break before each clause.
fn readable(sql: &str) -> String {
    let stripped = strip_comments(sql).replace("\r\n", "\n");
    let text = if stripped.trim().is_empty() { sql.replace("\r\n", "\n") } else { stripped };
    // The first line starts at the margin — what is left of it may be the blanks a stripped
    // comment left behind — and the others keep their indentation relative to each other.
    let lines: Vec<&str> = text.trim_start().lines().map(str::trim_end).collect();
    let content = lines.iter().filter(|l| !l.trim().is_empty()).count();
    if content <= 1 {
        return break_clauses(&collapse(&text));
    }
    let indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<&str> = Vec::new();
    let mut blank = false;
    for (n, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if blank {
            out.push("");
            blank = false;
        }
        out.push(if n == 0 { line } else { line.get(indent..).unwrap_or_else(|| line.trim_start()) });
    }
    out.join("\n")
}

/// A line break before each top-level clause of a one-line query: `SELECT`, `FROM`, `WHERE`,
/// `GROUP BY`, `ORDER BY`, `HAVING`, `LIMIT`, `UNION`, the joins, `SETTINGS`, `FORMAT` and a
/// `CREATE`'s `ENGINE` and `PARTITION BY`. Inside brackets and strings nothing moves, and a
/// function that shares a keyword's name (`left(…)`, `format(…)`) is left alone.
fn break_clauses(sql: &str) -> String {
    // (text, is a word, bracket depth)
    let mut pieces: Vec<(String, bool, i32)> = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut depth = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, '\'' | '"' | '`') {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
            pieces.push((chars[start..i.min(chars.len())].iter().collect(), false, depth));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            pieces.push((chars[start..i].iter().collect(), true, depth));
        } else {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            pieces.push((c.to_string(), false, depth));
            i += 1;
        }
    }
    let words: Vec<(usize, String)> = pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| p.1)
        .map(|(n, p)| (n, p.0.to_ascii_uppercase()))
        .collect();
    const JOIN_WORDS: &[&str] = &[
        "LEFT", "RIGHT", "INNER", "FULL", "CROSS", "OUTER", "GLOBAL", "ANY", "ALL", "SEMI", "ANTI", "ASOF", "ARRAY",
        "PASTE",
    ];
    let mut breaks: Vec<usize> = Vec::new();
    for (n, (index, word)) in words.iter().enumerate() {
        if n == 0 || pieces[*index].2 != 0 {
            continue;
        }
        // `left(AccNr, 2)` is a function; `FROM (SELECT …)` is a clause.
        let called = pieces.get(index + 1).is_some_and(|p| p.0 == "(");
        if called {
            continue;
        }
        let prev = words[n - 1].1.as_str();
        let next = words.get(n + 1).map(|w| w.1.as_str());
        let starts_join = |w: Option<&str>| w.is_some_and(|w| w == "JOIN" || JOIN_WORDS.contains(&w));
        let clause = match word.as_str() {
            "SELECT" | "FROM" | "WHERE" | "PREWHERE" | "HAVING" | "LIMIT" | "UNION" | "SETTINGS" | "FORMAT"
            | "ENGINE" => true,
            "GROUP" | "ORDER" | "PARTITION" => next == Some("BY"),
            "JOIN" => !JOIN_WORDS.contains(&prev),
            w if JOIN_WORDS.contains(&w) && w != "ALL" && w != "OUTER" => {
                starts_join(next) && !JOIN_WORDS.contains(&prev)
            }
            _ => false,
        };
        if clause {
            breaks.push(*index);
        }
    }
    let mut out = String::with_capacity(sql.len() + breaks.len());
    for (n, (text, _, _)) in pieces.iter().enumerate() {
        if breaks.contains(&n) {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
        }
        if out.ends_with('\n') && text.trim().is_empty() {
            continue;
        }
        out.push_str(text);
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
        assert_eq!(plain_lines(&layout(REDASH, 80))[0], "WITH BankRecord AS (");
        // A query that is only a comment is still a query: what there is, is shown.
        assert_eq!(plain_lines(&layout("/* only a comment */", 80)), ["/* only a comment */"]);
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

    fn plain_lines(lines: &[Vec<(String, Token)>]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|(t, _)| t.as_str()).collect()).collect()
    }

    #[test]
    fn a_one_line_query_is_broken_before_its_clauses() {
        let sql = "/* Username: j.petrova@example.net, Redash query_id: 8585 */ SELECT region, count() AS ops FROM accounting_lt.bank_record LEFT JOIN banks USING (Bank) WHERE EventDate >= today() - 30 AND left(AccNr, 2) = 'LT' GROUP BY region ORDER BY ops DESC LIMIT 10 SETTINGS max_threads = 2";
        assert_eq!(
            plain_lines(&layout(sql, 200)),
            [
                "SELECT region, count() AS ops",
                "FROM accounting_lt.bank_record",
                "LEFT JOIN banks USING (Bank)",
                "WHERE EventDate >= today() - 30 AND left(AccNr, 2) = 'LT'",
                "GROUP BY region",
                "ORDER BY ops DESC",
                "LIMIT 10",
                "SETTINGS max_threads = 2",
            ]
        );
    }

    #[test]
    fn brackets_and_strings_keep_their_clauses() {
        let sql = "INSERT INTO t SELECT a FROM (SELECT a FROM u WHERE x = 'FROM here') UNION ALL SELECT b FROM v";
        assert_eq!(
            plain_lines(&layout(sql, 200)),
            ["INSERT INTO t", "SELECT a", "FROM (SELECT a FROM u WHERE x = 'FROM here')", "UNION ALL", "SELECT b", "FROM v"]
        );
    }

    #[test]
    fn a_query_with_its_own_lines_keeps_them() {
        let lines = plain_lines(&layout(REDASH, 200));
        assert_eq!(
            lines,
            [
                "WITH BankRecord AS (",
                "  SELECT BillOpId, Bank FROM accounting_lt.bank_record",
                "  WHERE EventDate >= today() - 30",
                ")",
                "SELECT region, count() FROM BankRecord GROUP BY region",
            ],
            "the Redash comment is gone, the author's indentation stays"
        );
    }

    #[test]
    fn long_lines_wrap_at_words_and_long_words_are_cut() {
        let lines = layout("SELECT aaaa, bbbb, cccc, dddd FROM t", 12);
        let text = plain_lines(&lines);
        assert_eq!(text, ["SELECT aaaa,", "bbbb, cccc,", "dddd", "FROM t"]);
        assert!(text.iter().all(|l| crate::fmt::width(l) <= 12));
        let cut = plain_lines(&layout("SELECT a_very_long_identifier_indeed", 10));
        assert_eq!(cut, ["SELECT", "a_very_lon", "g_identifi", "er_indeed"]);
        // Colours survive the layout: the keyword is still a keyword.
        assert_eq!(lines[0][0], ("SELECT".to_string(), Token::Keyword));
    }
}
