//! What Claude Code and OpenCode used on this machine: tokens, by model and project, over the last
//! week — view 0's AI panel. `sources/usage.rs` reads it (Claude Code's transcripts, OpenCode's
//! database, the usage numbers only); this is the arithmetic, pure.
//!
//! - **tokens** of a reply = input + output + cache written + cache read, as the API counts them.
//! - **cost**: OpenCode's own, as it recorded it per reply. Claude Code on a Pro or Max plan has
//!   no per-token price, so it is only ever *≈ at API prices*: input, output and cache reads at
//!   the model's list price, cache writes at 1.25× input (5 minutes) or 2× (an hour); a model
//!   not in the table has no price, and its tokens still count.
//! - **the 5-hour window** — what a Pro or Max plan's limits are counted over: it starts on the
//!   hour of the first reply after the last one ended, and lasts five hours.
//! - **a day** is the clock's.

use std::collections::BTreeMap;

/// Which program a reply came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    Claude,
    OpenCode,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::OpenCode => "OpenCode",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Tool::Claude => "✻",
            Tool::OpenCode => "▣",
        }
    }
}

/// One reply of a model, and what it took.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub tool: Option<Tool>,
    /// When, Unix seconds.
    pub at: i64,
    pub model: String,
    /// The folder's name the session worked in.
    pub project: String,
    /// The conversation it is of: Claude Code's session id, OpenCode's — what a session of view 7
    /// knows its own by.
    pub session: String,
    pub input: u64,
    pub output: u64,
    /// Cache written, for five minutes and for an hour.
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    /// What it cost as its program recorded it (OpenCode), in dollars.
    pub cost: Option<f64>,
}

impl Reply {
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    /// Dollars: recorded, or at the model's API price (Claude Code); `None` without either.
    pub fn dollars(&self) -> Option<f64> {
        if self.cost.is_some() {
            return self.cost;
        }
        let (input, output, read) = price(&self.model)?;
        let per = |tokens: u64, rate: f64| tokens as f64 / 1e6 * rate;
        Some(per(self.input, input) + per(self.output, output) + per(self.cache_read, read) + per(self.cache_write_5m, input * 1.25) + per(self.cache_write_1h, input * 2.0))
    }
}

/// A model's list price, dollars per million tokens: input, output, cache read.
pub fn price(model: &str) -> Option<(f64, f64, f64)> {
    let m = model.to_lowercase();
    let table: [(&str, (f64, f64, f64)); 10] = [
        ("fable-5-1", (10.0, 50.0, 0.25)),
        ("mythos-5-1", (10.0, 50.0, 0.25)),
        ("fable-5", (10.0, 50.0, 1.0)),
        ("opus-5-5", (4.0, 20.0, 0.20)),
        ("opus-5", (5.0, 25.0, 0.50)),
        ("opus-4", (5.0, 25.0, 0.50)),
        ("sonnet-5-5", (2.0, 10.0, 0.20)),
        ("sonnet-5", (2.0, 10.0, 0.20)),
        ("sonnet-4", (3.0, 15.0, 0.30)),
        ("haiku-4", (1.0, 5.0, 0.10)),
    ];
    table.iter().find(|(name, _)| m.contains(name)).map(|(_, p)| *p)
}

/// `claude-fable-5-1` → `fable-5-1`, `deepseek-v4.1-flash` stays: what fits a narrow column.
pub fn short_model(model: &str) -> String {
    model.strip_prefix("claude-").unwrap_or(model).to_string()
}

/// What was read, and when.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub replies: Vec<Reply>,
    /// What could not be read, by program: `OpenCode: sqlite3 not found`.
    pub notes: Vec<String>,
    pub read: bool,
}

/// A program's day: its tokens, its dollars, how many replies.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tally {
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cache: u64,
    pub dollars: f64,
    /// Some of it had no price: the dollars are a floor.
    pub unpriced: bool,
    pub replies: usize,
}

impl Tally {
    fn add(&mut self, reply: &Reply) {
        self.tokens += reply.tokens();
        self.input += reply.input;
        self.output += reply.output;
        self.cache += reply.cache_read + reply.cache_write_5m + reply.cache_write_1h;
        match reply.dollars() {
            Some(d) => self.dollars += d,
            None => self.unpriced = true,
        }
        self.replies += 1;
    }
}

/// The Pro or Max plan's current window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
    pub tally: Tally,
}

/// What the panel shows.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub today: BTreeMap<Tool, Tally>,
    /// Claude Code's window going now, if a reply is in one.
    pub window: Option<Window>,
    /// The last seven days, oldest first, each day's tokens by program.
    pub days: Vec<BTreeMap<Tool, u64>>,
    pub week: Tally,
    /// Today's tokens by model and by project, the most first.
    pub models: Vec<(String, Tool, u64)>,
    pub projects: Vec<(String, u64)>,
}

impl Usage {
    /// Today's tokens and dollars by conversation, for view 7's list.
    pub fn by_session(&self, now: i64, offset_s: i64) -> std::collections::HashMap<String, Tally> {
        let day_of = |at: i64| (at + offset_s).div_euclid(86_400);
        let today = day_of(now);
        let mut out: std::collections::HashMap<String, Tally> = std::collections::HashMap::new();
        for reply in self.replies.iter().filter(|r| !r.session.is_empty() && day_of(r.at) == today) {
            out.entry(reply.session.clone()).or_default().add(reply);
        }
        out
    }

    /// The panel's numbers at `now`, days on a clock `offset_s` east of UTC.
    pub fn summary(&self, now: i64, offset_s: i64) -> Summary {
        let day_of = |at: i64| (at + offset_s).div_euclid(86_400);
        let today = day_of(now);
        let mut summary = Summary { days: vec![BTreeMap::new(); 7], ..Summary::default() };
        let mut models: BTreeMap<(String, Tool), u64> = BTreeMap::new();
        let mut projects: BTreeMap<String, u64> = BTreeMap::new();
        for reply in &self.replies {
            let Some(tool) = reply.tool else { continue };
            let ago = today - day_of(reply.at);
            if !(0..7).contains(&ago) {
                continue;
            }
            *summary.days[6 - ago as usize].entry(tool).or_default() += reply.tokens();
            summary.week.add(reply);
            if ago == 0 {
                summary.today.entry(tool).or_default().add(reply);
                *models.entry((short_model(&reply.model), tool)).or_default() += reply.tokens();
                *projects.entry(reply.project.clone()).or_default() += reply.tokens();
            }
        }
        summary.window = window(&self.replies, now);
        let mut models: Vec<(String, Tool, u64)> = models.into_iter().map(|((m, t), n)| (m, t, n)).collect();
        models.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        let mut projects: Vec<(String, u64)> = projects.into_iter().collect();
        projects.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        summary.models = models;
        summary.projects = projects;
        summary
    }
}

/// Claude Code's 5-hour window holding `now`: each starts on the hour of the first reply after
/// the one before ended.
pub fn window(replies: &[Reply], now: i64) -> Option<Window> {
    let mut times: Vec<&Reply> = replies.iter().filter(|r| r.tool == Some(Tool::Claude) && r.at <= now).collect();
    times.sort_by_key(|r| r.at);
    let mut current: Option<Window> = None;
    for reply in times {
        match &mut current {
            Some(w) if reply.at < w.end => w.tally.add(reply),
            _ => {
                let start = reply.at - reply.at.rem_euclid(3600);
                let mut tally = Tally::default();
                tally.add(reply);
                current = Some(Window { start, end: start + 5 * 3600, tally });
            }
        }
    }
    current.filter(|w| now < w.end)
}

/// `4.2M`, `850k`, `312` — tokens in a few characters.
pub fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// `$18.40`, `$0.42`, `$0.003`.
pub fn dollars(d: f64) -> String {
    if d >= 0.01 || d == 0.0 { format!("${d:.2}") } else { format!("${d:.3}") }
}

// ---------------------------------------------------------------------------
// Reading what the programs wrote
// ---------------------------------------------------------------------------

/// One line of a Claude Code transcript, if it is an assistant's reply with its usage: the reply,
/// and its id (`message.id` + `requestId`) — the same reply is written more than once.
pub fn claude_line(line: &str) -> Option<(String, Reply)> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    let n = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_u64).unwrap_or(0);
    let id = format!("{}:{}", message.get("id")?.as_str()?, value.get("requestId").and_then(|r| r.as_str()).unwrap_or(""));
    let at = crate::sources::unix(value.get("timestamp")?.as_str()?)?;
    let (write_5m, write_1h) = match usage.get("cache_creation") {
        Some(split) => (n(split.get("ephemeral_5m_input_tokens")), n(split.get("ephemeral_1h_input_tokens"))),
        None => (n(usage.get("cache_creation_input_tokens")), 0),
    };
    let cwd = value.get("cwd").and_then(|c| c.as_str()).unwrap_or("");
    let model = message.get("model").and_then(|m| m.as_str()).unwrap_or("");
    if model == "<synthetic>" {
        return None;
    }
    Some((
        id,
        Reply {
            tool: Some(Tool::Claude),
            at,
            model: model.to_string(),
            project: project_of(cwd),
            session: value.get("sessionId").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            input: n(usage.get("input_tokens")),
            output: n(usage.get("output_tokens")),
            cache_write_5m: write_5m,
            cache_write_1h: write_1h,
            cache_read: n(usage.get("cache_read_input_tokens")),
            cost: None,
        },
    ))
}

/// One row of OpenCode's replies, as `sources/usage.rs` asks for them (tab-separated): when (ms),
/// model, cost, input, output, reasoning, cache read, cache write, folder, session.
pub fn opencode_row(row: &str) -> Option<Reply> {
    let f: Vec<&str> = row.split('\t').collect();
    if f.len() < 9 {
        return None;
    }
    let n = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
    Some(Reply {
        tool: Some(Tool::OpenCode),
        at: f[0].trim().parse::<i64>().ok()? / 1000,
        model: f[1].to_string(),
        cost: f[2].trim().parse::<f64>().ok(),
        input: n(f[3]),
        // Reasoning is output, as the providers bill it.
        output: n(f[4]) + n(f[5]),
        cache_read: n(f[6]),
        cache_write_5m: n(f[7]),
        cache_write_1h: 0,
        project: project_of(f[8]),
        session: f.get(9).map(|s| s.trim().to_string()).unwrap_or_default(),
    })
}

/// A folder's last part: what the session list calls the project.
pub fn project_of(dir: &str) -> String {
    let trimmed = dir.trim_end_matches('/');
    trimmed.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("~").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_446_400 + 10 * 3600; // 2026-10-08 10:00 UTC

    fn reply(tool: Tool, at: i64, model: &str, project: &str, input: u64, output: u64, read: u64) -> Reply {
        Reply { tool: Some(tool), at, model: model.into(), project: project.into(), input, output, cache_read: read, ..Reply::default() }
    }

    #[test]
    fn a_claude_code_line_reads_and_its_price_is_the_api_s() {
        let line = r#"{"type":"assistant","sessionId":"5f0c","requestId":"req_1","timestamp":"2026-10-05T15:09:14.684Z","cwd":"/Users/me/code/cobserve","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":18033,"cache_read_input_tokens":25653,"output_tokens":371,"cache_creation":{"ephemeral_1h_input_tokens":18033,"ephemeral_5m_input_tokens":0}}}}"#;
        let (id, r) = claude_line(line).unwrap();
        assert_eq!((id.as_str(), r_session(line)), ("msg_1:req_1", "5f0c".to_string()));
        assert_eq!((r.input, r.output, r.cache_write_1h, r.cache_read, r.project.as_str()), (2, 371, 18033, 25653, "cobserve"));
        assert_eq!(r.tokens(), 2 + 371 + 18033 + 25653);
        // $4 in, $20 out, $0.20 cache read, 1h writes at 2× input.
        let expected = (2.0 * 4.0 + 371.0 * 20.0 + 25653.0 * 0.20 + 18033.0 * 8.0) / 1e6;
        assert!((r.dollars().unwrap() - expected).abs() < 1e-9);
        assert_eq!(claude_line(r#"{"type":"user","message":{"content":"hi"}}"#), None);
        assert_eq!(price("deepseek-v4.1-flash"), None);
    }

    fn r_session(line: &str) -> String {
        claude_line(line).unwrap().1.session
    }

    #[test]
    fn an_opencode_row_keeps_its_own_cost() {
        let r = opencode_row("1791440645097\tdeepseek-v4.1-flash\t0.002185542\t529\t442\t500\t513664\t0\t/Users/me/code/apitap-lib\tses_42").unwrap();
        assert_eq!(r.session, "ses_42");
        assert_eq!((r.at, r.input, r.output, r.cache_read, r.project.as_str()), (1_791_440_645, 529, 942, 513_664, "apitap-lib"));
        assert_eq!(r.dollars(), Some(0.002185542));
        assert_eq!(opencode_row("short\trow"), None);
    }

    #[test]
    fn today_the_window_and_the_week() {
        let usage = Usage {
            replies: vec![
                reply(Tool::Claude, NOW - 3 * 3600 - 600, "claude-opus-5-5", "cobserve", 1000, 2000, 1_000_000), // 06:50: opens 06:00–11:00
                reply(Tool::Claude, NOW - 3600, "claude-fable-5-1", "airflow-dags", 10, 500, 3_000_000),
                reply(Tool::OpenCode, NOW - 1800, "deepseek-v4.1-flash", "apitap-lib", 500, 400, 500_000),
                reply(Tool::Claude, NOW - 3 * 86_400, "claude-opus-5-5", "cobserve", 0, 0, 2_000_000),
                reply(Tool::Claude, NOW - 9 * 86_400, "claude-opus-5-5", "old", 0, 0, 9_000_000),
            ],
            notes: Vec::new(),
            read: true,
        };
        let s = usage.summary(NOW, 0);
        let claude = s.today[&Tool::Claude];
        assert_eq!((claude.tokens, claude.replies), (4_003_510, 2));
        assert_eq!(s.today[&Tool::OpenCode].tokens, 500_900);
        let w = s.window.unwrap();
        assert_eq!((w.start, w.end), (NOW - 4 * 3600, NOW + 3600), "on the hour of its first reply, five hours");
        assert_eq!(w.tally.replies, 2);
        assert_eq!(s.days.len(), 7);
        assert_eq!(s.days[6][&Tool::Claude], 4_003_510);
        assert_eq!(s.days[3][&Tool::Claude], 2_000_000, "three days ago");
        assert_eq!(s.week.tokens, 4_003_510 + 500_900 + 2_000_000, "nine days ago is not in the week");
        assert_eq!(s.models[0].0, "fable-5-1");
        assert_eq!(s.projects[0], ("airflow-dags".to_string(), 3_000_510));
        assert_eq!(window(&usage.replies, NOW + 2 * 3600), None, "the window has ended");
        assert_eq!((tokens(4_003_510), tokens(850_000), tokens(312)), ("4.0M".to_string(), "850k".to_string(), "312".to_string()));
        assert_eq!((dollars(18.4), dollars(0.0021)), ("$18.40".to_string(), "$0.002".to_string()));
    }
}
