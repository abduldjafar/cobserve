//! Suggestions for a query session's SQL as it is typed: what the cursor's place in the
//! statement calls for — a table after `FROM`, a table of `db.`, a column of the tables the
//! statement reads (by their aliases too), a function, a keyword, a format after `FORMAT` —
//! from what the server says it has (`Schema`, read from its system tables when a session
//! first runs on it) and what ClickHouse itself knows.
//!
//! Pure: the text, the cursor and the schema in; the suggestions out, the likeliest first.

/// What a server has, for suggestions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    pub databases: Vec<String>,
    pub tables: Vec<Table>,
    /// Each function's name, and whether it is an aggregate.
    pub functions: Vec<(String, bool)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub database: String,
    pub name: String,
    pub engine: String,
    /// Each column's name and type, in the table's order.
    pub columns: Vec<(String, String)>,
    /// About how many rows it holds, as the server counts them (`total_rows`), when it does.
    pub rows: Option<u64>,
    /// What its makers wrote about it, if anything.
    pub comment: String,
}

impl Table {
    /// How a statement names it: alone in `default`, else after its database.
    pub fn qualified(&self) -> String {
        if self.database == "default" || self.database.is_empty() { self.name.clone() } else { format!("{}.{}", self.database, self.name) }
    }
}

impl Schema {
    /// The table a statement names: `db.name`, or `name` alone — in `default` first.
    pub fn table(&self, database: Option<&str>, name: &str) -> Option<&Table> {
        let named = |t: &&Table| t.name == name || t.name.eq_ignore_ascii_case(name);
        match database {
            Some(db) => self.tables.iter().filter(named).find(|t| t.database == db || t.database.eq_ignore_ascii_case(db)),
            None => self.tables.iter().filter(named).min_by_key(|t| t.database != "default"),
        }
    }

    fn has_database(&self, name: &str) -> Option<&str> {
        self.databases.iter().find(|d| *d == name || d.eq_ignore_ascii_case(name)).map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Column,
    Table,
    Database,
    Function,
    Keyword,
    Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What takes the place of the word typed so far.
    pub text: String,
    pub what: What,
    /// A column's type, a table's engine, `aggregate`.
    pub detail: String,
    /// How far back from the end of `text` the cursor goes: into a function's parentheses.
    pub back: usize,
}

/// The suggestions at a cursor, for the word that starts at `from` on the cursor's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub from: usize,
    pub items: Vec<Suggestion>,
}

/// How many suggestions are kept.
pub const MAX: usize = 60;

/// Suggestions for the word before the cursor (`col` characters into line `row`), or none: in a
/// string or a comment, in the middle of a word, after `AS` or `LIMIT`, or — unless `forced`,
/// as `ctrl+space` asks — where nothing has been typed yet.
pub fn complete(lines: &[String], row: usize, col: usize, schema: Option<&Schema>, forced: bool) -> Option<Completion> {
    let line: Vec<char> = lines.get(row)?.chars().collect();
    let col = col.min(line.len());
    if line.get(col).is_some_and(|c| is_word(*c)) {
        return None;
    }
    let mut from = col;
    while from > 0 && is_word(line[from - 1]) {
        from -= 1;
    }
    let word: String = line[from..col].iter().collect();
    if word.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    // `db.`, `alias.`, `db.table.` before it.
    let mut start = from;
    let mut qualifier = Vec::new();
    while start > 0 && line[start - 1] == '.' {
        let end = start - 1;
        let mut begin = end;
        while begin > 0 && is_word(line[begin - 1]) {
            begin -= 1;
        }
        let part: String = line[begin..end].iter().collect();
        let part = if part.is_empty() && end > 0 && line[end - 1] == '`' {
            // A quoted name: `my db`.
            let close = end - 1;
            let open = line[..close].iter().rposition(|c| *c == '`')?;
            begin = open;
            line[open + 1..close].iter().collect()
        } else {
            part
        };
        if part.is_empty() {
            return None;
        }
        qualifier.insert(0, part);
        start = begin;
    }
    if word.is_empty() && qualifier.is_empty() && !forced {
        return None;
    }

    // The whole text, the cursor and the word's start in it.
    let text: Vec<char> = lines.join("\n").chars().collect();
    let offset: usize = lines[..row].iter().map(|l| l.chars().count() + 1).sum();
    let (at, word_at) = (offset + col, offset + start);
    let tokens = lex(&text);
    // In a string, a comment or a quoted name, nothing is suggested: a line's comment holds the
    // end of its line, the others end before their closing mark.
    let inside = |t: &Token| match t.kind {
        Kind::Comment if t.text == "--" => t.start < at && at <= t.end,
        Kind::Str | Kind::Comment | Kind::Quoted => t.start < at && (at < t.end || !t.closed),
        _ => false,
    };
    if tokens.iter().any(inside) {
        return None;
    }
    // The statement the cursor is in.
    let begin = tokens.iter().rposition(|t| t.end <= word_at && t.is(';')).map_or(0, |i| i + 1);
    let end = tokens.iter().skip(begin).position(|t| t.start >= at && t.is(';')).map_or(tokens.len(), |i| begin + i);
    let statement: Vec<&Token> = tokens[begin..end].iter().filter(|t| t.kind != Kind::Comment).collect();
    let before: Vec<&Token> = statement.iter().copied().filter(|t| t.end <= word_at).collect();
    let prev = before.last().copied();
    let first = statement.first().map(|t| t.upper()).unwrap_or_default();
    let references = references(&statement, at);
    let fallback = Schema::fallback();
    let schema = schema.unwrap_or(&fallback);

    let place = match prev.map(|t| (t.kind, t.upper())) {
        _ if !qualifier.is_empty() => Place::Qualified,
        Some((Kind::Word, word)) => match word.as_str() {
            "FROM" | "IN" if first == "SHOW" => Place::Database,
            "FROM" | "JOIN" | "INTO" | "TABLE" | "DESCRIBE" | "EXISTS" | "OPTIMIZE" => Place::Table,
            "DESC" if before.len() == 1 => Place::Table,
            "USE" | "DATABASE" => Place::Database,
            "FORMAT" => Place::Format,
            // A name being given, or a number.
            "AS" | "LIMIT" | "OFFSET" | "TOP" | "INTERVAL" => return None,
            w if VALUE_WORDS.contains(&w) => Place::Expression { after_value: true },
            w if is_keyword(w) => Place::Expression { after_value: false },
            _ => Place::Expression { after_value: true },
        },
        Some((Kind::Number | Kind::Str | Kind::Quoted, _)) => Place::Expression { after_value: true },
        Some((Kind::Punct, p)) => Place::Expression { after_value: p == ")" || p == "*" },
        _ => Place::Expression { after_value: false },
    };

    let keyword_case = |keyword: &str| -> String {
        if !word.is_empty() && word.chars().all(|c| !c.is_uppercase()) { keyword.to_lowercase() } else { keyword.to_string() }
    };
    // Each found with how well it matched, the rank of its kind here, and whether its case
    // differs from what was typed.
    let mut found: Vec<(u8, u8, bool, Suggestion)> = Vec::new();
    let mut offer = |quality: Option<u8>, rank: u8, suggestion: Suggestion| {
        if let Some(quality) = quality
            && suggestion.text != word
        {
            let inexact = !suggestion.text.starts_with(word.as_str());
            found.push((quality, rank, inexact, suggestion));
        }
    };
    let column = |name: &str, kind: &str, table: Option<&str>| Suggestion {
        text: name.to_string(),
        what: What::Column,
        detail: match table {
            Some(table) => format!("{kind} · {table}"),
            None => kind.to_string(),
        },
        back: 0,
    };
    let table_suggestion = |table: &Table, qualified: bool| Suggestion {
        text: if qualified { table.qualified() } else { table.name.clone() },
        what: What::Table,
        detail: table.engine.clone(),
        back: 0,
    };

    match place {
        Place::Qualified => {
            let last = qualifier.last().map(String::as_str).unwrap_or_default();
            let db = (qualifier.len() == 2).then(|| qualifier[0].as_str());
            // An alias or a table of the statement, else a table of the server: its columns.
            let referenced = references
                .iter()
                .find(|r| r.alias.as_deref().is_some_and(|a| a == last || a.eq_ignore_ascii_case(last)))
                .or_else(|| references.iter().find(|r| r.name.eq_ignore_ascii_case(last) && (db.is_none() || r.database.as_deref() == db)));
            let table = match referenced {
                Some(r) => schema.table(r.database.as_deref(), &r.name),
                // `db.` is a database's before it is a table's.
                None if db.is_none() && schema.has_database(last).is_some() => None,
                None if qualifier.len() <= 2 => schema.table(db, last),
                None => None,
            };
            if let Some(table) = table {
                for (name, kind) in &table.columns {
                    offer(quality(&word, name), 0, column(name, kind, None));
                }
            } else if let (1, Some(database)) = (qualifier.len(), schema.has_database(last)) {
                for table in schema.tables.iter().filter(|t| t.database == database) {
                    offer(quality(&word, &table.name), 0, table_suggestion(table, false));
                }
            }
        }
        Place::Table => {
            for database in &schema.databases {
                offer(quality(&word, database), 1, Suggestion { text: database.clone(), what: What::Database, detail: "database".into(), back: 0 });
            }
            for table in &schema.tables {
                let own = quality(&word, &table.name);
                let by_database = (own.is_none() && !word.is_empty() && starts_with(&table.database, &word)).then_some(2);
                offer(own.or(by_database), 0, table_suggestion(table, true));
            }
        }
        Place::Database => {
            for database in &schema.databases {
                offer(quality(&word, database), 0, Suggestion { text: database.clone(), what: What::Database, detail: "database".into(), back: 0 });
            }
        }
        Place::Format => {
            for format in FORMATS {
                offer(quality(&word, format), 0, Suggestion { text: format.to_string(), what: What::Format, detail: "format".into(), back: 0 });
            }
        }
        Place::Expression { after_value } => {
            let (columns, functions, keywords) = if after_value { (1, 2, 0) } else { (0, 1, 2) };
            let tables: Vec<&Table> = references.iter().filter_map(|r| schema.table(r.database.as_deref(), &r.name)).collect();
            let mut seen = std::collections::HashSet::new();
            for table in &tables {
                for (name, kind) in &table.columns {
                    if seen.insert(name.clone()) {
                        let from = (tables.len() > 1).then_some(table.name.as_str());
                        offer(quality(&word, name), columns, column(name, kind, from));
                    }
                }
            }
            for (name, aggregate) in &schema.functions {
                let suggestion = Suggestion {
                    text: format!("{name}()"),
                    what: What::Function,
                    detail: if *aggregate { "aggregate".into() } else { "function".into() },
                    back: 1,
                };
                offer(quality(&word, name), functions, suggestion);
            }
            for keyword in KEYWORDS {
                let head = keyword.split(' ').next().unwrap_or(keyword);
                let text = keyword_case(keyword);
                offer(quality(&word, head), keywords, Suggestion { text, what: What::Keyword, detail: String::new(), back: 0 });
            }
        }
    }
    // The best match, of the likeliest kind, the shortest; with nothing typed, in the order the
    // server gave them — a table's columns as the table has them.
    found.sort_by(|a, b| {
        (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)).then_with(|| {
            if word.is_empty() {
                std::cmp::Ordering::Equal
            } else {
                (a.3.text.chars().count(), &a.3.text).cmp(&(b.3.text.chars().count(), &b.3.text))
            }
        })
    });
    let mut items: Vec<Suggestion> = Vec::new();
    for (_, _, _, suggestion) in found {
        if !items.iter().any(|s| s.text == suggestion.text) {
            items.push(suggestion);
            if items.len() == MAX {
                break;
            }
        }
    }
    (!items.is_empty()).then_some(Completion { from, items })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// After `db.` or `alias.`.
    Qualified,
    Table,
    Database,
    Format,
    /// Anywhere else: a column, a function, a keyword — a keyword first after a value.
    Expression { after_value: bool },
}

/// How well `word` names `name`: 0 its start, in any case; 1 the start of a part of it (`log`
/// in `query_log`, `hour` in `toStartOfHour`); `None` not at all. Anything goes for nothing
/// typed.
fn quality(word: &str, name: &str) -> Option<u8> {
    if word.is_empty() || starts_with(name, word) {
        return Some(0);
    }
    if word.chars().count() >= 2 {
        let chars: Vec<char> = name.chars().collect();
        for i in 1..chars.len() {
            let boundary = chars[i - 1] == '_' || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
            if boundary && starts_with(&chars[i..].iter().collect::<String>(), word) {
                return Some(1);
            }
        }
    }
    None
}

fn starts_with(name: &str, word: &str) -> bool {
    name.len() >= word.len() && name.is_char_boundary(word.len()) && name[..word.len()].eq_ignore_ascii_case(word)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|k| k.split(' ').any(|part| part == word)) || ["BY", "NOT", "NULL", "IS"].contains(&word)
}

/// A table a statement reads, and the alias it goes by there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reference {
    database: Option<String>,
    name: String,
    alias: Option<String>,
}

/// The tables after `FROM`, `JOIN` and the commas between them — not the name the cursor is in,
/// which is still being typed.
fn references(statement: &[&Token], at: usize) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < statement.len() {
        let upper = statement[i].upper();
        if statement[i].kind != Kind::Word || !(upper == "FROM" || upper == "JOIN") {
            i += 1;
            continue;
        }
        i += 1;
        loop {
            let name_at = |j: usize| statement.get(j).filter(|t| matches!(t.kind, Kind::Word | Kind::Quoted));
            let Some(first) = name_at(i) else {
                break;
            };
            let (database, name, next) = if statement.get(i + 1).is_some_and(|t| t.is('.')) {
                match name_at(i + 2) {
                    Some(table) => (Some(first.text.clone()), table, i + 3),
                    // `db.`, its table still to be typed.
                    None => {
                        i += 2;
                        break;
                    }
                }
            } else {
                (None, first, i + 1)
            };
            // A table function, or the name the cursor is in.
            if statement.get(next).is_some_and(|t| t.is('(')) || (name.start <= at && at <= name.end) {
                i = next;
                break;
            }
            let mut j = next;
            if statement.get(j).is_some_and(|t| t.upper() == "AS") {
                j += 1;
            }
            let alias = name_at(j).filter(|t| t.kind == Kind::Quoted || !is_keyword(&t.upper())).map(|t| t.text.clone());
            if alias.is_some() {
                j += 1;
            }
            found.push(Reference { database, name: name.text.clone(), alias });
            if statement.get(j).is_some_and(|t| t.is(',')) {
                i = j + 1;
            } else {
                i = j;
                break;
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    /// A name between backquotes or double quotes.
    Quoted,
    Number,
    Str,
    Comment,
    Punct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: Kind,
    /// Where it is in the text, in characters: `start..end`.
    start: usize,
    end: usize,
    /// A word, a name without its quotes, a punctuation mark.
    text: String,
    /// A string or a comment that ends before the text does.
    closed: bool,
}

impl Token {
    fn upper(&self) -> String {
        if self.kind == Kind::Word { self.text.to_ascii_uppercase() } else { self.text.clone() }
    }

    fn is(&self, c: char) -> bool {
        self.kind == Kind::Punct && self.text.len() == 1 && self.text.starts_with(c)
    }
}

fn lex(text: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        let start = i;
        let token = |kind, end: usize, value: String, closed| Token { kind, start, end, text: value, closed };
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && text.get(i + 1) == Some(&'-') {
            while i < text.len() && text[i] != '\n' {
                i += 1;
            }
            tokens.push(token(Kind::Comment, i, "--".into(), i < text.len()));
        } else if c == '/' && text.get(i + 1) == Some(&'*') {
            i += 2;
            let mut closed = false;
            while i < text.len() {
                if text[i] == '*' && text.get(i + 1) == Some(&'/') {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            tokens.push(token(Kind::Comment, i, "/*".into(), closed));
        } else if matches!(c, '\'' | '`' | '"') {
            i += 1;
            let mut closed = false;
            let mut value = String::new();
            while i < text.len() {
                if text[i] == '\\' {
                    i += 2;
                    continue;
                }
                if text[i] == c {
                    // A doubled quote is the quote itself.
                    if text.get(i + 1) == Some(&c) {
                        value.push(c);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                value.push(text[i]);
                i += 1;
            }
            let i = i.min(text.len());
            tokens.push(token(if c == '\'' { Kind::Str } else { Kind::Quoted }, i, value, closed));
        } else if c.is_ascii_digit() {
            while i < text.len() && (text[i].is_alphanumeric() || text[i] == '.' || text[i] == '_') {
                i += 1;
            }
            tokens.push(token(Kind::Number, i, text[start..i].iter().collect(), true));
        } else if is_word(c) {
            while i < text.len() && is_word(text[i]) {
                i += 1;
            }
            tokens.push(token(Kind::Word, i, text[start..i].iter().collect(), true));
        } else {
            i += 1;
            tokens.push(token(Kind::Punct, i, c.to_string(), true));
        }
    }
    tokens
}

/// Keywords after which a value has been given, as after a name.
const VALUE_WORDS: &[&str] = &["END", "NULL", "TRUE", "FALSE", "DAY", "HOUR", "MINUTE", "SECOND", "WEEK", "MONTH", "YEAR", "ASC", "DESC", "FINAL"];

/// The keywords suggested, the ones of two words by their first.
const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "PREWHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "LIMIT BY", "OFFSET", "WITH", "AS",
    "AND", "OR", "NOT", "IN", "NOT IN", "GLOBAL IN", "IS NULL", "IS NOT NULL", "NULL", "BETWEEN", "LIKE", "ILIKE",
    "NOT LIKE", "JOIN", "LEFT JOIN", "RIGHT JOIN", "INNER JOIN", "FULL JOIN", "CROSS JOIN", "ARRAY JOIN",
    "LEFT ARRAY JOIN", "ANY JOIN", "ON", "USING", "UNION ALL", "UNION DISTINCT", "DISTINCT", "CASE", "WHEN", "THEN",
    "ELSE", "END", "INTERVAL", "ASC", "DESC", "NULLS FIRST", "NULLS LAST", "WITH TOTALS", "WITH FILL", "FINAL", "SAMPLE",
    "SETTINGS", "FORMAT", "OVER", "PARTITION BY", "WINDOW", "QUALIFY", "EXPLAIN", "EXPLAIN PIPELINE", "EXPLAIN SYNTAX",
    "EXPLAIN ESTIMATE", "EXPLAIN indexes = 1", "DESCRIBE TABLE", "SHOW TABLES", "SHOW DATABASES", "SHOW CREATE TABLE",
    "SHOW PROCESSLIST", "EXISTS", "TRUE", "FALSE", "DAY", "HOUR", "MINUTE", "SECOND", "WEEK", "MONTH", "YEAR",
];

/// The formats that come back as text a session can show.
const FORMATS: &[&str] = &[
    "Pretty", "PrettyCompact", "PrettyCompactMonoBlock", "PrettySpace", "Vertical", "Markdown", "TabSeparated",
    "TabSeparatedWithNames", "TSV", "TSVWithNames", "CSV", "CSVWithNames", "JSON", "JSONEachRow", "JSONCompact",
    "JSONCompactEachRow", "Values", "XML", "Null",
];

/// The functions suggested before the server has said which it knows, and whether each is an
/// aggregate.
const FUNCTIONS: &[(&str, bool)] = &[
    ("count", true), ("sum", true), ("avg", true), ("min", true), ("max", true), ("any", true), ("anyLast", true),
    ("argMax", true), ("argMin", true), ("uniq", true), ("uniqExact", true), ("uniqCombined", true), ("groupArray", true),
    ("groupUniqArray", true), ("quantile", true), ("quantiles", true), ("quantileExact", true), ("median", true),
    ("countIf", true), ("sumIf", true), ("avgIf", true), ("topK", true), ("if", false), ("multiIf", false),
    ("coalesce", false), ("ifNull", false), ("nullIf", false), ("toDate", false), ("toDateTime", false),
    ("toDateTime64", false), ("toStartOfMinute", false), ("toStartOfFiveMinutes", false), ("toStartOfHour", false),
    ("toStartOfDay", false), ("toStartOfWeek", false), ("toStartOfMonth", false), ("toYYYYMM", false),
    ("toYYYYMMDD", false), ("now", false), ("today", false), ("yesterday", false), ("dateDiff", false),
    ("formatDateTime", false), ("toUnixTimestamp", false), ("fromUnixTimestamp", false), ("toString", false),
    ("toUInt64", false), ("toInt64", false), ("toFloat64", false), ("toFloat64OrZero", false), ("toUInt64OrZero", false),
    ("toDecimal64", false), ("parseDateTimeBestEffort", false), ("length", false), ("lower", false), ("upper", false),
    ("concat", false), ("substring", false), ("replaceAll", false), ("replaceRegexpAll", false), ("extract", false),
    ("match", false), ("position", false), ("splitByChar", false), ("arrayJoin", false), ("arrayMap", false),
    ("arrayFilter", false), ("has", false), ("hasAny", false), ("indexOf", false), ("round", false), ("floor", false),
    ("ceil", false), ("abs", false), ("intDiv", false), ("greatest", false), ("least", false),
    ("formatReadableSize", false), ("formatReadableQuantity", false), ("formatReadableTimeDelta", false),
    ("JSONExtractString", false), ("JSONExtractInt", false), ("JSONExtractRaw", false), ("cityHash64", false),
    ("runningDifference", false), ("bar", false), ("hostName", false), ("version", false), ("uptime", false),
    ("currentDatabase", false), ("currentUser", false), ("isNull", false), ("isNotNull", false), ("empty", false),
    ("notEmpty", false), ("toTypeName", false), ("clusterAllReplicas", false),
];

/// The system tables every server has, suggested before it has said what else it has.
const SYSTEM_TABLES: &[&str] = &[
    "processes", "query_log", "query_thread_log", "parts", "part_log", "tables", "columns", "databases", "merges",
    "mutations", "replicas", "replication_queue", "clusters", "metrics", "events", "asynchronous_metrics", "disks",
    "settings", "functions", "users", "errors", "dictionaries", "text_log",
];

impl Schema {
    /// What every server has: its system tables (no columns yet), ClickHouse's common functions.
    pub fn fallback() -> Schema {
        Schema {
            databases: vec!["default".into(), "system".into()],
            tables: SYSTEM_TABLES
                .iter()
                .map(|name| Table { database: "system".into(), name: name.to_string(), engine: "System".into(), columns: Vec::new(), ..Default::default() })
                .collect(),
            functions: FUNCTIONS.iter().map(|(name, aggregate)| (name.to_string(), *aggregate)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let columns = |list: &[(&str, &str)]| list.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect();
        Schema {
            databases: vec!["default".into(), "system".into(), "wallet".into()],
            tables: vec![
                Table {
                    database: "system".into(),
                    name: "processes".into(),
                    engine: "SystemProcesses".into(),
                    columns: columns(&[("query_id", "String"), ("user", "String"), ("elapsed", "Float64"), ("memory_usage", "Int64"), ("query", "String")]),
                    ..Default::default()
                },
                Table {
                    database: "system".into(),
                    name: "query_log".into(),
                    engine: "MergeTree".into(),
                    columns: columns(&[("event_time", "DateTime"), ("query_duration_ms", "UInt64"), ("user", "String")]),
                    ..Default::default()
                },
                Table {
                    database: "wallet".into(),
                    name: "ledger".into(),
                    engine: "ReplicatedMergeTree".into(),
                    columns: columns(&[("merchant_id", "UInt64"), ("amount", "Decimal(18, 2)"), ("month", "UInt8")]),
                    ..Default::default()
                },
                Table { database: "default".into(), name: "events".into(), engine: "MergeTree".into(), columns: columns(&[("ts", "DateTime")]), ..Default::default() },
            ],
            functions: vec![("count".into(), true), ("countIf".into(), true), ("toStartOfHour".into(), false), ("formatReadableSize".into(), false)],
        }
    }

    /// The suggestions where `|` is, as texts.
    fn at(text: &str) -> Vec<String> {
        at_with(text, Some(&schema()), false)
    }

    fn at_with(text: &str, schema: Option<&Schema>, forced: bool) -> Vec<String> {
        let marked = text.find('|').expect("a cursor");
        let before = &text[..marked];
        let lines: Vec<String> = text.replace('|', "").split('\n').map(str::to_string).collect();
        let row = before.matches('\n').count();
        let col = before.rsplit('\n').next().unwrap_or(before).chars().count();
        complete(&lines, row, col, schema, forced).map(|c| c.items.into_iter().map(|s| s.text).collect()).unwrap_or_default()
    }

    #[test]
    fn after_from_come_the_tables_and_after_a_database_its_own() {
        assert_eq!(at("SELECT * FROM proc|"), ["system.processes"]);
        assert_eq!(at("SELECT * FROM led|")[0], "wallet.ledger");
        // A part of the name will do: `log` finds query_log.
        assert_eq!(at("SELECT * FROM log|"), ["system.query_log"]);
        // The database first when it is what is being typed, its tables after.
        let sys = at("SELECT * FROM sys|");
        assert_eq!(sys[0], "system");
        assert!(sys.contains(&"system.processes".to_string()), "{sys:?}");
        assert_eq!(at("SELECT * FROM system.|"), ["processes", "query_log"]);
        assert_eq!(at("SELECT * FROM system.q|"), ["query_log"]);
        assert_eq!(at("SELECT * FROM ev|"), ["events"], "a table of default goes by its name alone");
        assert_eq!(at("SHOW TABLES FROM wa|"), ["wallet"]);
        assert_eq!(at("SELECT 1 FORMAT Pre|")[0], "Pretty");
    }

    #[test]
    fn columns_come_from_the_tables_the_statement_reads_by_name_or_alias() {
        // The table is after the cursor: the statement is read whole.
        let user = at("SELECT us| FROM system.processes");
        assert_eq!(user[0], "user");
        assert_eq!(at("SELECT p.mem| FROM system.processes AS p"), ["memory_usage"]);
        assert_eq!(at("SELECT l.| FROM wallet.ledger l WHERE l.month = 7"), ["merchant_id", "amount", "month"]);
        assert_eq!(at("SELECT processes.que| FROM system.processes"), ["query", "query_id"]);
        // Two tables: a column says which it is from.
        let lines = vec!["SELECT ev| FROM system.query_log, events".to_string()];
        let both = complete(&lines, 0, 9, Some(&schema()), false).unwrap();
        assert_eq!((both.items[0].text.as_str(), both.items[0].detail.as_str()), ("event_time", "DateTime · query_log"));
        assert_eq!(both.from, 7);
    }

    #[test]
    fn functions_open_their_parentheses_and_keywords_follow_the_case_typed() {
        let lines = vec!["SELECT cou".to_string()];
        let found = complete(&lines, 0, 10, Some(&schema()), false).unwrap();
        assert_eq!((found.items[0].text.as_str(), found.items[0].back, found.items[0].what), ("count()", 1, What::Function));
        assert_eq!(found.items[1].text, "countIf()");
        // `Hour` finds toStartOfHour too, by the part of its name — after what starts with it.
        assert_eq!(at("SELECT Hour| FROM events"), ["HOUR", "toStartOfHour()"]);
        // After a value a keyword is likelier than a column or a function.
        let mut with_from = schema();
        with_from.functions.push(("fromUnixTimestamp".into(), false));
        assert_eq!(at_with("SELECT user FR| system.processes", Some(&with_from), false)[0], "FROM");
        assert_eq!(at_with("SELECT FR|", Some(&with_from), false)[0], "fromUnixTimestamp()", "after SELECT, a function");
        assert_eq!(at("select user fr|")[0], "from", "lower case typed, lower case given");
        assert_eq!(at("SELECT count() FROM events GRO|"), ["GROUP BY"]);
        assert_eq!(at("SELECT * FROM events WHERE ts > now() AN|")[0], "AND");
        assert_eq!(at("SELECT * FROM events ORDER BY ts DESC LIM|")[0], "LIMIT");
    }

    #[test]
    fn nothing_is_suggested_where_nothing_could_go() {
        assert!(at("SELECT 'proc|'").is_empty(), "in a string");
        assert!(at("SELECT 1 -- proc|").is_empty(), "in a comment");
        assert!(at("SELECT count() AS c|").is_empty(), "a name being given");
        assert!(at("SELECT * FROM events LIMIT 1|").is_empty(), "a number");
        assert!(at("SELECT * FROM events LIMIT |").is_empty());
        assert!(at("SELECT us|er FROM system.processes").is_empty(), "inside a word");
        assert!(at("SELECT |").is_empty(), "nothing typed, nothing asked");
        assert!(!at_with("SELECT * FROM |", Some(&schema()), true).is_empty(), "ctrl+space asks");
        // What is typed exactly is not offered again.
        assert!(!at("SELECT * FROM system.processes|").contains(&"processes".to_string()));
        // The statement is the one the cursor is in.
        assert_eq!(at("SELECT 1 FROM wallet.ledger;\nSELECT merch| FROM events"), Vec::<String>::new());
        assert_eq!(at("SELECT merch| FROM wallet.ledger;\nSELECT 1 FROM events"), ["merchant_id"]);
    }

    #[test]
    fn before_the_server_has_said_what_it_has_the_system_tables_and_common_functions_do() {
        assert_eq!(at_with("SELECT * FROM system.proc|", None, false), ["processes"]);
        assert_eq!(at_with("SELECT formatReadableS|", None, false), ["formatReadableSize()"]);
        assert_eq!(quality("log", "query_log"), Some(1));
        assert_eq!(quality("hour", "toStartOfHour"), Some(1));
        assert_eq!(quality("TOS", "toStartOfHour"), Some(0));
        assert_eq!(quality("x", "toStartOfHour"), None);
    }
}
