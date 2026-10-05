//! Conversations Claude Code and OpenCode had elsewhere — in another terminal, an earlier run —
//! for a session here to take up again, and the one a running program is in now.
//!
//! - Claude Code keeps each conversation in `~/.claude/projects/<folder>/<id>.jsonl`, the folder
//!   being the directory it worked in with every character but a letter or a digit made `-`.
//!   Read here: the directory it worked in and its first prompt from the top of the file, its
//!   title (`/rename`) and last prompt from the end. A running one says which conversation it
//!   is in in `~/.claude/sessions/<pid>.json`.
//! - OpenCode answers `opencode session list --format json` in a directory, and anywhere a
//!   query of its own database through `opencode db`.
//!
//! Only what identifies a conversation is read: never what was said in it beyond those prompts,
//! and nothing is written. Every lookup runs on a thread of its own (`main.rs`).

use crate::claude::Kind;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub kind: Kind,
    /// What its program takes back: `claude --resume <id>`, `opencode --session <id>`.
    pub id: String,
    /// Where it worked.
    pub dir: PathBuf,
    /// Its name if it was given one, else its first prompt.
    pub title: String,
    /// What was asked last, when that is not the title.
    pub last: Option<String>,
    /// When it last changed, Unix seconds.
    pub updated: i64,
    /// A program has it open right now, in another terminal or here.
    pub open: bool,
}

/// How much of the start and of the end of a conversation file is read for its name.
const HEAD: u64 = 256 * 1024;
const TAIL: u64 = 256 * 1024;

/// Claude Code's folder for a directory: `/home/user/cobserve` → `-home-user-cobserve`.
pub fn claude_folder(dir: &Path) -> String {
    dir.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Claude Code's conversations, newest first, at most `limit`: those that worked in `dir`, or
/// all of them.
pub fn claude(home: &Path, dir: Option<&Path>, limit: usize) -> Vec<Conversation> {
    let projects = home.join(".claude").join("projects");
    let folders: Vec<PathBuf> = match dir {
        Some(dir) => vec![projects.join(claude_folder(dir))],
        None => std::fs::read_dir(&projects).map(|entries| entries.flatten().map(|e| e.path()).collect()).unwrap_or_default(),
    };
    let mut files: Vec<(i64, PathBuf)> = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|path| Some((modified(&path)?, path)))
        .collect();
    files.sort_by_key(|(updated, _)| std::cmp::Reverse(*updated));
    let open = claude_open(home);
    files
        .into_iter()
        .filter_map(|(updated, path)| {
            let mut conversation = claude_file(&path, updated)?;
            conversation.open = open.contains(&conversation.id);
            // A file whose directory is not the one asked about is another folder's that
            // happens to be named the same way.
            dir.is_none_or(|dir| conversation.dir == dir).then_some(conversation)
        })
        .take(limit)
        .collect()
}

/// One conversation file: `None` when it holds no prompt — a session opened and left at once.
fn claude_file(path: &Path, updated: i64) -> Option<Conversation> {
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    (&mut file).take(HEAD).read_to_end(&mut bytes).ok()?;
    // Cut in the middle of a character, the lines before it are still good.
    let head = String::from_utf8_lossy(&bytes).into_owned();
    let (mut dir, mut first) = (None, None);
    for line in head.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if dir.is_none()
            && let Some(cwd) = record.get("cwd").and_then(|c| c.as_str())
        {
            dir = Some(PathBuf::from(cwd));
        }
        if first.is_none() {
            first = prompt(&record);
        }
        if dir.is_some() && first.is_some() {
            break;
        }
    }
    let mut tail = String::new();
    if len > HEAD {
        file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
        let mut bytes = Vec::new();
        file.take(TAIL).read_to_end(&mut bytes).ok()?;
        tail = String::from_utf8_lossy(&bytes).into_owned();
    }
    let (mut title, mut last) = (None, None);
    for line in head.lines().chain(tail.lines()) {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match record.get("type").and_then(|t| t.as_str()) {
            Some("custom-title") => title = record.get("customTitle").and_then(|t| t.as_str()).map(one_line),
            Some("last-prompt") => last = record.get("lastPrompt").and_then(|t| t.as_str()).map(one_line),
            _ => {
                if let Some(text) = prompt(&record) {
                    last = Some(text);
                }
            }
        }
    }
    let first = first?;
    let title = title.filter(|t| !t.is_empty()).unwrap_or_else(|| first.clone());
    Some(Conversation {
        kind: Kind::Claude,
        id,
        dir: dir?,
        last: last.filter(|l| *l != title),
        title,
        updated,
        open: false,
    })
}

/// The text of a prompt the user typed: a `user` record of the main conversation whose content
/// is text — not a tool's result, not a command's echo.
fn prompt(record: &serde_json::Value) -> Option<String> {
    if record.get("type").and_then(|t| t.as_str()) != Some("user") || record.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
        return None;
    }
    let content = record.get("message")?.get("content")?;
    let text = match content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = one_line(&text);
    (!text.is_empty() && !text.starts_with('<') && !text.starts_with("Caveat:")).then_some(text)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The conversations running Claude Codes have open: `~/.claude/sessions/<pid>.json`.
fn claude_open(home: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home.join(".claude").join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter_map(|record| record.get("sessionId").and_then(|s| s.as_str()).map(str::to_string))
        .collect()
}

/// The conversation the Claude Code with this process id is in now — after a `/clear` or a
/// `/resume` inside it, not the one it started with.
pub fn claude_live(home: &Path, pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".claude").join("sessions").join(format!("{pid}.json"))).ok()?;
    let record: serde_json::Value = serde_json::from_str(&text).ok()?;
    record.get("sessionId")?.as_str().map(str::to_string)
}

/// Whether Claude Code has a conversation by this id anywhere — one never typed into has no file,
/// and `--resume` would refuse it.
pub fn claude_has(home: &Path, id: &str) -> bool {
    let projects = home.join(".claude").join("projects");
    std::fs::read_dir(projects)
        .map(|entries| entries.flatten().any(|e| e.path().join(format!("{id}.jsonl")).is_file()))
        .unwrap_or(false)
}

/// OpenCode's conversations in `dir`, newest first: what `opencode session list` says there.
pub fn opencode(command: &[String], dir: &Path, limit: usize) -> Vec<Conversation> {
    let Some(text) = run(command, dir, &["session", "list", "--format", "json", "-n", &limit.to_string()]) else {
        return Vec::new();
    };
    parse_opencode(&text, Some(dir))
}

/// OpenCode's conversations anywhere, newest first, from its own database.
pub fn opencode_all(command: &[String], limit: usize) -> Vec<Conversation> {
    let query = format!(
        "select id, title, directory, time_updated as updated from session where parent_id is null and time_archived is null order by time_updated desc limit {limit}"
    );
    let here = std::env::temp_dir();
    let Some(text) = run(command, &here, &["db", &query, "--format", "json"]) else {
        return Vec::new();
    };
    parse_opencode(&text, None)
}

fn parse_opencode(text: &str, dir: Option<&Path>) -> Vec<Conversation> {
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return Vec::new();
    };
    // A query session's questions to OpenCode are not conversations to take up.
    let helpers = crate::assist::dir();
    rows.iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.to_string();
            let directory = row.get("directory").and_then(|d| d.as_str()).map(PathBuf::from).or_else(|| dir.map(Path::to_path_buf))?;
            Some(Conversation {
                kind: Kind::OpenCode,
                id,
                dir: directory,
                title: row.get("title").and_then(|t| t.as_str()).map(one_line).unwrap_or_default(),
                last: None,
                updated: row.get("updated").and_then(|u| u.as_i64()).map_or(0, |ms| ms / 1000),
                open: false,
            })
        })
        .filter(|conversation| !conversation.dir.starts_with(&helpers))
        .collect()
}

/// `command` with `args` in `dir`, its output when it succeeded — OpenCode's own way of saying
/// what it keeps, rather than its files read behind its back.
fn run(command: &[String], dir: &Path, args: &[&str]) -> Option<String> {
    let program = command.first()?;
    let output = std::process::Command::new(program)
        .args(&command[1..])
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn modified(path: &Path) -> Option<i64> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("conversations-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    fn write(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn claude_s_folder_names_are_the_directory_with_every_other_character_a_dash() {
        assert_eq!(claude_folder(Path::new("/home/user/cobserve")), "-home-user-cobserve");
        assert_eq!(claude_folder(Path::new("/Users/me/work/my.app_2")), "-Users-me-work-my-app-2");
    }

    #[test]
    fn claude_s_conversations_are_named_by_their_title_or_first_prompt_newest_first() {
        use serde_json::json;
        let home = home();
        let work = Path::new("/work/reports");
        let folder = home.join(".claude/projects").join(claude_folder(work));
        let user = |text: &str| json!({"type": "user", "cwd": "/work/reports", "isSidechain": false, "message": {"role": "user", "content": text}});
        write(&folder.join("aaa.jsonl"), &[
            json!({"type": "queue-operation", "operation": "enqueue"}),
            user("make the totals weekly"),
            json!({"type": "assistant", "cwd": "/work/reports", "message": {"content": [{"type": "text", "text": "Sure"}]}}),
            json!({"type": "user", "cwd": "/work/reports", "message": {"content": [{"type": "tool_result", "content": "ok"}]}}),
            user("now run   the tests"),
        ]);
        write(&folder.join("bbb.jsonl"), &[
            user("<command-name>/clear</command-name>"),
            user("fix the late partitions"),
            json!({"type": "custom-title", "customTitle": "late partitions"}),
        ]);
        // Opened and left: nothing to take up.
        write(&folder.join("ccc.jsonl"), &[json!({"type": "mode", "mode": "default"})]);
        // Running in another terminal.
        std::fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        std::fs::write(home.join(".claude/sessions/4242.json"), json!({"pid": 4242, "sessionId": "bbb"}).to_string()).unwrap();
        // bbb is the newer.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(folder.join("aaa.jsonl")).unwrap().set_modified(old).unwrap();

        let found = claude(&home, Some(work), 10);
        assert_eq!(found.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["bbb", "aaa"]);
        assert_eq!((found[0].title.as_str(), found[0].open), ("late partitions", true));
        assert_eq!(found[0].last.as_deref(), Some("fix the late partitions"));
        assert_eq!((found[1].title.as_str(), found[1].open), ("make the totals weekly", false));
        assert_eq!(found[1].last.as_deref(), Some("now run the tests"));
        assert_eq!(found[1].dir, work);
        assert_eq!(claude(&home, None, 1).len(), 1, "everywhere, as many as asked");
        assert!(claude(&home, Some(Path::new("/elsewhere")), 10).is_empty());
        assert_eq!(claude_live(&home, 4242).as_deref(), Some("bbb"));
        assert!(claude_has(&home, "aaa") && !claude_has(&home, "zzz"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn opencode_s_list_is_read_as_it_prints_it() {
        let text = r#"[{"id":"ses_1","title":"Fix  the job","updated":1791153186337,"created":1791153186000,"projectId":"p","directory":"/work/pipelines"}]"#;
        let found = parse_opencode(text, None);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].title.as_str(), found[0].updated), ("ses_1", "Fix the job", 1_791_153_186));
        assert_eq!(found[0].dir, Path::new("/work/pipelines"));
        assert!(parse_opencode("not json", None).is_empty());
        assert!(parse_opencode("", None).is_empty());
    }
}
