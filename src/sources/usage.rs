//! Reads what Claude Code and OpenCode used (view 0's AI panel), on a thread of its own every
//! minute — never on the UI's time. Usage numbers only: of a Claude Code transcript, the lines
//! that carry `usage`, and of those, the numbers, the model, the folder and the time; of OpenCode's
//! database, opened read-only, the same columns. Nothing of what was said is read into memory.
//!
//! Claude Code's transcripts run to hundreds of megabytes, so each file is read once, then only
//! what was written to it since.

use crate::app::Event;
use crate::usage::{self, Reply, Usage};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;

/// How far back is read: this month and the whole of the last, whatever the day.
const KEEP_S: i64 = 63 * 86_400;

pub fn spawn(tx: mpsc::UnboundedSender<Event>) {
    std::thread::spawn(move || {
        let mut reader = Reader::default();
        loop {
            let usage = reader.read();
            if tx.send(Event::Usage(Box::new(usage))).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

#[derive(Default)]
struct Reader {
    /// How far into each transcript has been read.
    offsets: HashMap<PathBuf, u64>,
    seen: HashSet<String>,
    claude: Vec<Reply>,
}

impl Reader {
    fn read(&mut self) -> Usage {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let mut notes = Vec::new();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            self.read_claude(&home.join(".claude/projects"), now);
        }
        self.claude.retain(|r| now - r.at <= KEEP_S);
        let mut replies = self.claude.clone();
        match opencode(now) {
            Ok(rows) => replies.extend(rows),
            Err(why) => notes.push(format!("OpenCode: {why}")),
        }
        Usage { replies, notes, read: true }
    }

    fn read_claude(&mut self, root: &Path, now: i64) {
        for path in transcripts(root) {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let modified = meta.modified().ok().and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            if now - modified > KEEP_S {
                continue;
            }
            let from = self.offsets.get(&path).copied().unwrap_or(0);
            // A file that got shorter was written anew: from the start.
            let from = if meta.len() < from { 0 } else { from };
            if meta.len() == from {
                continue;
            }
            let Ok(mut file) = std::fs::File::open(&path) else { continue };
            if file.seek(SeekFrom::Start(from)).is_err() {
                continue;
            }
            let mut reader = std::io::BufReader::new(file.by_ref());
            let mut read = from;
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    // A line still being written: read again from its start next time.
                    Ok(_) if !line.ends_with('\n') => break,
                    Ok(n) => {
                        read += n as u64;
                        if let Some((id, reply)) = usage::claude_line(&line)
                            && now - reply.at <= KEEP_S
                            && self.seen.insert(id)
                        {
                            self.claude.push(reply);
                        }
                    }
                }
            }
            self.offsets.insert(path, read);
        }
    }
}

/// Every transcript under `root`, subagents' included: `*.jsonl` two levels down at most.
fn transcripts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    stack.push((path, depth + 1));
                }
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

/// OpenCode's replies of the last days, from its database, read-only.
fn opencode(now: i64) -> Result<Vec<Reply>, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("no HOME")?;
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let db = data.join("opencode/opencode.db");
    if !db.exists() {
        return Ok(Vec::new());
    }
    let since_ms = (now - KEEP_S) * 1000;
    let sql = format!(
        "SELECT m.time_created, json_extract(m.data,'$.modelID'), json_extract(m.data,'$.cost'), \
         json_extract(m.data,'$.tokens.input'), json_extract(m.data,'$.tokens.output'), json_extract(m.data,'$.tokens.reasoning'), \
         json_extract(m.data,'$.tokens.cache.read'), json_extract(m.data,'$.tokens.cache.write'), s.directory, m.session_id \
         FROM message m JOIN session s ON s.id = m.session_id \
         WHERE m.time_created >= {since_ms} AND json_extract(m.data,'$.role') = 'assistant'"
    );
    let output = std::process::Command::new("sqlite3")
        .args(["-readonly", "-batch", "-separator", "\t"])
        .arg(&db)
        .arg(&sql)
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "sqlite3 not found".to_string() } else { e.to_string() })?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(said.lines().next().unwrap_or("its database could not be read").chars().take(120).collect());
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter_map(usage::opencode_row).collect())
}

#[cfg(test)]
mod tests {
    /// `cargo test --release live_usage -- --ignored --nocapture`: what view 0 would say of this
    /// machine's week, and how long the first read took. Numbers only.
    #[test]
    #[ignore]
    fn live_usage() {
        let started = std::time::Instant::now();
        let mut reader = super::Reader::default();
        let usage = reader.read();
        let first = started.elapsed();
        let started = std::time::Instant::now();
        let _ = reader.read();
        println!("first read {first:?}, the next {:?}; {} replies; notes {:?}", started.elapsed(), usage.replies.len(), usage.notes);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let s = usage.summary(now, 7 * 3600);
        for (tool, t) in &s.today {
            println!("today {:<12} {:>8} tokens  {}  ({} replies)", tool.name(), crate::usage::tokens(t.tokens), crate::usage::dollars(t.dollars), t.replies);
        }
        if let Some(w) = s.window {
            println!("window ends in {} min, {} tokens", (w.end - now) / 60, crate::usage::tokens(w.tally.tokens));
        }
        println!("week {} · days {:?}", crate::usage::tokens(s.week.tokens), s.days.iter().map(|d| crate::usage::tokens(d.values().sum())).collect::<Vec<_>>());
        for (name, by) in [(&s.month_names.0, &s.this_month), (&s.month_names.1, &s.last_month)] {
            println!("{name}: {:?}", by.iter().map(|(t, n)| format!("{} {} {}", t.name(), crate::usage::tokens(n.tokens), crate::usage::dollars(n.dollars))).collect::<Vec<_>>());
        }
        println!("models {:?}", s.models.iter().take(5).map(|m| (m.0.clone(), crate::usage::tokens(m.2.tokens))).collect::<Vec<_>>());
        println!("projects {:?}", s.projects.iter().take(5).map(|p| (p.0.clone(), p.1.iter().map(|(t, n)| format!("{} {} {}", t.name(), crate::usage::tokens(n.tokens), crate::usage::dollars(n.dollars))).collect::<Vec<_>>())).collect::<Vec<_>>());
    }
}
