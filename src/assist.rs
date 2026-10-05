//! A query session's helper (`ctrl+g`): Claude Code — signed in with the Pro or Max plan, as in
//! a terminal of its own — or OpenCode, asked to write the SQL the session's `--` comments ask
//! for, or to put right what failed.
//!
//! Built here, purely: what is asked — the rules, the server, the tables the text names and the
//! ones its words point at, with their columns, the text, why it failed — the command that asks
//! it, and what is made of the answer. `main.rs` runs the command with no tools, in a folder of
//! its own, without the monitor's secrets; what comes back goes into the session, which runs
//! nothing until asked.

use crate::complete::{Schema, Table};
use crate::console::{Ask, Assistant};

/// What the helper is told it is for, and how to answer.
pub const RULES: &str = "You write ClickHouse SQL for a read-only query console in a terminal. \
Answer with SQL alone: no Markdown, no code fences, nothing before or after it — whatever needs \
saying (an explanation, an answer to a question) goes in short -- comment lines above the SQL.
- One statement that only reads: SELECT (WITH … SELECT too), SHOW, DESCRIBE, EXISTS or EXPLAIN. \
It runs with readonly=1 and a 30 second limit, and only its first 1000 rows come back.
- Keep the request's -- comment lines at the top as they are.
- Use the tables and columns you are given and ClickHouse's own functions; never make a name up. \
When the question cannot be answered from them, answer with one -- comment line that says why.
- Prefer what is cheap: a LIMIT where many rows could come back.
- When asked about a part of the text, answer with what should take that part's place, and \
nothing else of the text.
- End a whole statement with a semicolon.";

/// What OpenCode may do while it answers: nothing — no shell, no files, no web. Applied over
/// whatever its own configuration allows.
pub const OPENCODE_PERMISSION: &str = r#"{"*":"deny","read":"deny","edit":"deny","bash":"deny","glob":"deny","grep":"deny","list":"deny","task":"deny","external_directory":"deny","todowrite":"deny","todoread":"deny","question":"deny","webfetch":"deny","websearch":"deny","codesearch":"deny","lsp":"deny","skill":"deny","doom_loop":"deny"}"#;

/// How long a helper may take.
pub const TIME_LIMIT_S: u64 = 120;

/// The folder helpers run in: an empty one of the user's own, where nothing of a project is
/// there to be read.
pub fn dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("cobserve").join("assist")
}

/// The most of the server's tables told in full, and of their columns each.
const TABLES_IN_FULL: usize = 10;
const COLUMNS_EACH: usize = 120;
/// The most of the other tables named.
const OTHERS_NAMED: usize = 300;

/// The command that asks `assistant`, after `program` (its command as the sessions run it): the
/// question goes on its standard input.
pub fn command(assistant: Assistant, program: &[String], dir: &str) -> Vec<String> {
    let mut command = program.to_vec();
    match assistant {
        // Printed, not a conversation: no tools, no servers of the user's, nothing kept. The
        // rules come after Claude Code's own system prompt, not in its place: the Pro and Max
        // plans are for Claude Code as it is.
        Assistant::Claude => command.extend(
            [
                "-p",
                "--output-format",
                "text",
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--append-system-prompt",
                RULES,
            ]
            .map(str::to_string),
        ),
        Assistant::OpenCode => command.extend(["run".to_string(), "--dir".to_string(), dir.to_string()]),
    }
    command
}

/// What is asked: for OpenCode, which has no system prompt here, the rules first.
pub fn prompt(ask: &Ask, schema: Option<&Schema>, version: Option<&str>) -> String {
    let mut out = String::new();
    if ask.assistant == Assistant::OpenCode {
        out.push_str(RULES);
        out.push_str("\n\n");
    }
    match (&ask.node, version.filter(|v| !v.is_empty())) {
        (Some(node), Some(version)) => out.push_str(&format!("Server: {node}, ClickHouse {version}.\n\n")),
        (Some(node), None) => out.push_str(&format!("Server: {node}.\n\n")),
        _ => {}
    }
    if let Some(schema) = schema {
        let chosen = tables_for(&ask.sql, schema);
        if !chosen.is_empty() {
            out.push_str("Tables, with their columns:\n");
            for table in &chosen {
                let columns: Vec<String> = table.columns.iter().take(COLUMNS_EACH).map(|(name, kind)| format!("{name} {kind}")).collect();
                let more = table.columns.len().saturating_sub(COLUMNS_EACH);
                let more = if more > 0 { format!(", … {more} more") } else { String::new() };
                out.push_str(&format!("{}.{} ({}): {}{more}\n", table.database, table.name, table.engine, columns.join(", ")));
            }
            out.push('\n');
        }
        let others: Vec<String> = schema
            .tables
            .iter()
            .filter(|t| t.database != "system" && !chosen.iter().any(|c| std::ptr::eq(*c, *t)))
            .take(OTHERS_NAMED)
            .map(|t| format!("{}.{}", t.database, t.name))
            .collect();
        if !others.is_empty() {
            out.push_str(&format!("Other tables: {}.\n\n", others.join(", ")));
        }
    }
    out.push_str("The console's text:\n-----\n");
    out.push_str(ask.sql.trim_end());
    out.push_str("\n-----\n\n");
    if let Some(part) = &ask.selected {
        out.push_str("The part of it selected:\n-----\n");
        out.push_str(part.trim_end());
        out.push_str("\n-----\n\n");
    }
    if let Some(error) = &ask.error {
        out.push_str("It ran, and the server said:\n");
        out.push_str(error);
        out.push_str("\n\n");
    }
    let asked = ask.instruction.trim();
    match (&ask.selected, asked.is_empty(), &ask.error) {
        (Some(_), false, _) => out.push_str(&format!("Asked, of the selected part: {asked}\nAnswer with what should take that part's place.")),
        (Some(_), true, Some(_)) => out.push_str("Put the selected part right. Answer with what should take its place."),
        (Some(_), true, None) => out.push_str("Finish or correct the selected part. Answer with what should take its place."),
        (None, false, _) => out.push_str(&format!("Asked: {asked}")),
        (None, true, Some(_)) => out.push_str("Put it right."),
        (None, true, None) => out.push_str("Write the query its comments ask for — or finish, or correct, the SQL that is there."),
    }
    out
}

/// The tables told in full: those the text names, then those its words point at, else the
/// system tables a question about the server is likeliest to need.
fn tables_for<'a>(sql: &str, schema: &'a Schema) -> Vec<&'a Table> {
    let lower = sql.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).filter(|w| !w.is_empty()).collect();
    let mut chosen: Vec<&Table> = Vec::new();
    fn add<'a>(table: &'a Table, chosen: &mut Vec<&'a Table>) {
        if chosen.len() < TABLES_IN_FULL && !chosen.iter().any(|c| std::ptr::eq(*c, table)) {
            chosen.push(table);
        }
    }
    // Named: `db.table`, or a table's own name.
    for word in &words {
        for table in &schema.tables {
            let qualified = format!("{}.{}", table.database, table.name).to_lowercase();
            if *word == qualified || (word.len() >= 3 && *word == table.name.to_lowercase()) {
                add(table, &mut chosen);
            }
        }
    }
    // Pointed at: a word of four letters or more in a table's name, or in its columns'.
    let telling: Vec<&str> = words.iter().copied().filter(|w| w.len() >= 4 && !w.contains('.') && !STOP_WORDS.contains(w)).collect();
    let stem = |w: &str| w.strip_suffix('s').filter(|s| s.len() >= 4).unwrap_or(w).to_string();
    let telling: Vec<String> = telling.iter().map(|w| stem(w)).collect();
    let mut scored: Vec<(usize, &Table)> = schema
        .tables
        .iter()
        .map(|table| {
            let name = table.name.to_lowercase();
            let score = telling
                .iter()
                .map(|w| {
                    let in_name = if name.contains(w.as_str()) { 3 } else { 0 };
                    let in_columns = table.columns.iter().filter(|(c, _)| c.to_lowercase().contains(w.as_str())).count().min(2);
                    in_name + in_columns
                })
                .sum::<usize>();
            (score, table)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.database.cmp(&b.1.database)).then(a.1.name.cmp(&b.1.name)));
    for (_, table) in scored.into_iter().take(6) {
        add(table, &mut chosen);
    }
    if chosen.is_empty() {
        for name in ["processes", "query_log", "parts", "tables"] {
            if let Some(table) = schema.table(Some("system"), name) {
                add(table, &mut chosen);
            }
        }
    }
    chosen
}

/// Words too common to point at a table.
const STOP_WORDS: &[&str] = &[
    "select", "from", "where", "group", "order", "limit", "with", "that", "this", "what", "which", "show", "list", "give", "find",
    "most", "many", "much", "each", "every", "last", "first", "than", "more", "less", "over", "into", "only", "have", "their",
    "them", "they", "about", "right", "today", "query", "queries", "table", "tables", "count", "number", "top", "biggest",
    "largest", "using", "used", "uses", "per", "the", "and", "for",
];

/// What a helper answered, as SQL for the session: what was between fences when it fenced it,
/// without a terminal's colours — a whole statement ended with `;` so one `⏎` runs it. Nothing
/// at all is said so.
pub fn clean(answer: &str, whole: bool) -> Result<String, String> {
    let plain = strip_ansi(answer);
    let text = plain.trim();
    let text = fenced(text).unwrap_or(text).trim();
    if text.is_empty() {
        return Err("it answered nothing".into());
    }
    // A long line of SQL is broken before its clauses, to be read in the session's field.
    let mut sql = text
        .lines()
        .map(|line| {
            let long = line.chars().count() > 80 && !line.trim_start().starts_with("--");
            if long { crate::sqltext::break_clauses(line.trim()) } else { line.to_string() }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A whole statement ends with `;`, so one ⏎ runs it; a part takes its place as it came.
    let only_comments = sql.lines().all(|l| l.trim().is_empty() || l.trim_start().starts_with("--"));
    if whole && !only_comments && !sql.trim_end().ends_with(';') {
        sql.push(';');
    }
    Ok(sql)
}

/// The inside of the first fenced block — ```sql … ``` — if there is one.
fn fenced(text: &str) -> Option<&str> {
    let open = text.find("```")?;
    let after = &text[open + 3..];
    let body_at = after.find('\n')? + 1;
    let body = &after[body_at..];
    let close = body.find("```").unwrap_or(body.len());
    Some(&body[..close])
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for inner in chars.by_ref() {
                    if inner.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Why a helper did not answer, in a line: what it printed on its way out, with the way to
/// sign in when that is what it says.
pub fn failure(assistant: Assistant, status: Option<i32>, stderr: &str, stdout: &str) -> String {
    let said = strip_ansi(if stderr.trim().is_empty() { stdout } else { stderr });
    // The line that says what went wrong: one that says `error`, else the last — not OpenCode's
    // `> build · model` header.
    let lines: Vec<&str> = said.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("> ")).collect();
    let line = lines.iter().find(|l| l.to_lowercase().contains("error")).or(lines.last()).copied().unwrap_or_default();
    let lower = said.to_lowercase();
    let signed_out = ["login", "log in", "sign in", "api key", "unauthorized", "authentication", "401"].iter().any(|w| lower.contains(w));
    match assistant {
        Assistant::Claude if signed_out => "not signed in — run `claude` once and /login with your Pro or Max plan".into(),
        Assistant::OpenCode if signed_out => "not signed in — run `opencode auth login` once".into(),
        Assistant::Claude if lower.contains("unknown option") => format!("this Claude Code is too old for it — `claude update` ({line})"),
        _ if !line.is_empty() => fmt_line(line),
        _ => match status {
            Some(code) => format!("{} ended with status {code} and said nothing", assistant.name()),
            None => format!("{} was stopped", assistant.name()),
        },
    }
}

fn fmt_line(line: &str) -> String {
    line.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let table = |db: &str, name: &str, columns: &[&str]| Table {
            database: db.into(),
            name: name.into(),
            engine: "MergeTree".into(),
            columns: columns.iter().map(|c| (c.to_string(), "String".to_string())).collect(),
        };
        Schema {
            databases: vec!["system".into(), "wallet".into(), "gateway".into()],
            tables: vec![
                table("system", "processes", &["query_id", "user", "memory_usage"]),
                table("system", "query_log", &["event_time", "query_duration_ms"]),
                table("system", "parts", &["table", "rows"]),
                table("system", "tables", &["database", "name"]),
                table("wallet", "ledger", &["merchant_id", "amount"]),
                table("gateway", "transfers", &["ts", "status", "amount"]),
            ],
            functions: Vec::new(),
        }
    }

    fn ask(sql: &str, error: Option<&str>, assistant: Assistant) -> Ask {
        Ask { id: 1, assistant, node: Some("clickhouse3".into()), sql: sql.into(), error: error.map(str::to_string), instruction: String::new(), selected: None }
    }

    #[test]
    fn the_question_has_the_server_the_tables_it_points_at_and_what_failed() {
        let text = prompt(&ask("-- failed transfers per hour today", None, Assistant::Claude), Some(&schema()), Some("24.11.1"));
        assert!(text.starts_with("Server: clickhouse3, ClickHouse 24.11.1."), "{text}");
        assert!(text.contains("gateway.transfers (MergeTree): ts String, status String, amount String"), "{text}");
        assert!(text.contains("Other tables: wallet.ledger."), "named, not told in full: {text}");
        assert!(text.contains("-----\n-- failed transfers per hour today\n-----"));
        assert!(text.ends_with("the SQL that is there."));
        assert!(!text.contains(RULES), "Claude has them with its system prompt");

        let fix = prompt(&ask("SELECT usr FROM system.processes", Some("Code 47 · Unknown identifier usr"), Assistant::OpenCode), Some(&schema()), None);
        assert!(fix.starts_with(RULES), "OpenCode has them first");
        assert!(fix.contains("system.processes (MergeTree): query_id String, user String"));
        assert!(fix.contains("the server said:\nCode 47 · Unknown identifier usr\n\nPut it right."));
        // A question typed after ctrl+k, about a selected part.
        let mut part = ask("SELECT user FROM system.processes WHERE elapsed > 10", None, Assistant::Claude);
        part.instruction = "only queries over a minute".into();
        part.selected = Some("WHERE elapsed > 10".into());
        let text = prompt(&part, Some(&schema()), None);
        assert!(text.contains("The part of it selected:\n-----\nWHERE elapsed > 10\n-----"), "{text}");
        assert!(text.ends_with("Asked, of the selected part: only queries over a minute\nAnswer with what should take that part's place."), "{text}");
        let mut asked = ask("", None, Assistant::Claude);
        asked.instruction = "top 10 users by memory".into();
        assert!(prompt(&asked, Some(&schema()), None).ends_with("Asked: top 10 users by memory"));
        // Nothing points anywhere: the server's own tables.
        let schema = schema();
        let vague = tables_for("-- how is it doing", &schema);
        assert_eq!(vague.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["processes", "query_log", "parts", "tables"]);
    }

    #[test]
    fn the_commands_ask_without_tools_and_the_answer_is_made_sql() {
        let claude = command(Assistant::Claude, &["claude".to_string()], "/tmp/x");
        assert_eq!(&claude[..4], ["claude", "-p", "--output-format", "text"]);
        let tools = claude.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(claude[tools + 1], "", "no tools at all");
        assert!(claude.contains(&"--no-session-persistence".to_string()) && claude.contains(&"--strict-mcp-config".to_string()));
        assert!(!claude.contains(&"--bare".to_string()), "--bare would not read the Pro or Max login");
        assert!(claude.contains(&"--append-system-prompt".to_string()) && !claude.contains(&"--system-prompt".to_string()));
        assert_eq!(command(Assistant::OpenCode, &["opencode".to_string()], "/tmp/x"), ["opencode", "run", "--dir", "/tmp/x"]);
        assert!(serde_json::from_str::<serde_json::Value>(OPENCODE_PERMISSION).unwrap()["bash"] == "deny");

        assert_eq!(clean("```sql\nSELECT 1\n```\nThis counts.", true).unwrap(), "SELECT 1;");
        assert_eq!(clean("\x1b[1m-- top users\nSELECT user FROM system.processes;\x1b[0m\n", true).unwrap(), "-- top users\nSELECT user FROM system.processes;");
        assert_eq!(clean("-- no table has refunds", true).unwrap(), "-- no table has refunds", "a comment alone is not ended with ;");
        let long = "SELECT database, name, total_rows FROM system.tables WHERE database = 'testing' ORDER BY total_rows DESC LIMIT 5";
        assert_eq!(
            clean(&format!("-- the five biggest\n{long}"), true).unwrap(),
            "-- the five biggest\nSELECT database, name, total_rows\nFROM system.tables\nWHERE database = 'testing'\nORDER BY total_rows DESC\nLIMIT 5;"
        );
        assert!(clean("  \n", true).is_err());
        assert_eq!(clean("WHERE ts > now() - INTERVAL 1 DAY", false).unwrap(), "WHERE ts > now() - INTERVAL 1 DAY", "a part takes no ;");
        assert!(failure(Assistant::Claude, Some(1), "Invalid API key · Please run /login", "").contains("/login"));
        assert_eq!(failure(Assistant::OpenCode, Some(1), "", "Error: model not found\n"), "Error: model not found");
        assert_eq!(failure(Assistant::Claude, None, "", ""), "Claude was stopped");
        let blocked = "\n> build · some-model\n\nError: Forbidden: request blocked by the network\n";
        assert_eq!(failure(Assistant::OpenCode, Some(1), blocked, ""), "Error: Forbidden: request blocked by the network");
    }
}
