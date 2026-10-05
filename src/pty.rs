//! The process behind view 5: `claude` in a pseudo-terminal (DESIGN.md §13).
//!
//! One per session of view 5, started from `main.rs` when the session asks for it. Its output
//! goes down the event channel as `Event::Pane` and its end as `Event::PaneExited`, both with
//! the session's id, from two threads of its own — reading a PTY blocks, and the loop must not.
//! Dropping the process ends it (SIGHUP, as a closed terminal window would).

use crate::app::Event;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

/// What the program does not get from the monitor's environment.
///
/// `ANTHROPIC_API_KEY` first: with it set, Claude Code bills the API instead of the plan the
/// user signed in with, and this pane is there to use the plan. Then the monitor's own
/// credentials (§9) — a seed URL can carry a password — which have no business in an agent's
/// environment, where one `env` would print them.
pub const NOT_PASSED_ON: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "CH_PASSWORD",
    "CH_SEED_URLS",
    "REDASH_ADMIN_API_KEY",
    "REDIS_URL",
    "AIRFLOW_PASSWORD",
    "JIRA_TOKEN",
];

pub struct PtyProcess {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    size: (u16, u16),
    pid: Option<u32>,
}

impl PtyProcess {
    /// Start `command` in a PTY of `rows` × `cols`, in `dir` — the project Claude works on.
    /// Its events carry `session`.
    pub fn spawn(session: u64, command: &[String], dir: &Path, rows: u16, cols: u16, tx: UnboundedSender<Event>) -> Result<Self, String> {
        let program = command.first().ok_or("no command to run")?;
        // How to install it is the session's to say: it knows what kind of program this is.
        if find_program(program).is_none() {
            return Err(format!("{program} is not installed here (not on PATH)"));
        }
        let size = PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| format!("no pseudo-terminal: {e}"))?;
        let child = pair
            .slave
            .spawn_command(command_for(command, dir))
            .map_err(|e| format!("{program} did not start: {e}"))?;
        // The program holds the only other end now: when it exits, reading sees the end.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let killer = child.clone_killer();
        let pid = child.process_id();

        let output = tx.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if output.send(Event::Pane(session, buffer[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let mut child = child;
        std::thread::spawn(move || {
            let how = match child.wait() {
                Ok(status) if status.success() => "it exited".to_string(),
                Ok(status) => match status.signal() {
                    Some(signal) => format!("it ended on {signal}"),
                    None => format!("it exited with status {}", status.exit_code()),
                },
                Err(e) => format!("it could not be waited for: {e}"),
            };
            let _ = tx.send(Event::PaneExited(session, how));
        });

        Ok(Self { master: pair.master, writer, killer, size: (rows, cols), pid })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        // A program that has just ended cannot be written to; its end is reported on its own.
        let _ = self.writer.write_all(bytes).and_then(|()| self.writer.flush());
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// The program's process id, while the system says it.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let size = PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 };
        if self.master.resize(size).is_ok() {
            self.size = (rows, cols);
        }
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

/// The command, in `dir`, with a terminal it can believe and without what [`NOT_PASSED_ON`]
/// lists.
fn command_for(command: &[String], dir: &Path) -> CommandBuilder {
    let mut builder = CommandBuilder::new(&command[0]);
    builder.args(&command[1..]);
    // Without this the PTY starts the program in the home directory.
    builder.cwd(dir);
    scrub(&mut builder);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder
}

fn scrub(builder: &mut CommandBuilder) {
    for name in NOT_PASSED_ON {
        builder.env_remove(name);
    }
}

/// A session's directory as typed — `~` for home, relative to the monitor's own — as a path
/// that exists.
pub fn resolve_dir(typed: &str) -> Result<PathBuf, String> {
    let path = expand(typed);
    if path.is_dir() { Ok(path) } else { Err(format!("no such directory: {}", typed.trim())) }
}

/// A directory as typed, made absolute — `~` is home, anything relative is from here — without
/// asking the disk whether it is there.
pub fn expand(typed: &str) -> PathBuf {
    let typed = typed.trim();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = match (typed.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(typed),
    };
    let path = match std::env::current_dir() {
        Ok(here) if path.is_relative() => here.join(path),
        _ => path,
    };
    // `./x` and a trailing `/.` are the same folder without them.
    path.components().collect()
}

/// `~/work/cobserve` for a path under home: how a directory is shown, and typed.
pub fn tilde(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => {
            let rest = path.strip_prefix(&home).unwrap_or(path);
            if rest.as_os_str().is_empty() { "~".to_string() } else { format!("~/{}", rest.display()) }
        }
        _ => path.display().to_string(),
    }
}

/// The branch checked out in `dir` or the repository around it, read from git's own files —
/// no `git` to run, and a worktree's `.git` file is followed. A detached head is its hash, cut.
pub fn git_branch(dir: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(here) = at {
        let dot_git = here.join(".git");
        let git_dir = if dot_git.is_dir() {
            Some(dot_git)
        } else if dot_git.is_file() {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim().to_string();
            let target = PathBuf::from(target);
            Some(if target.is_relative() { here.join(target) } else { target })
        } else {
            None
        };
        if let Some(git_dir) = git_dir {
            let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
            let head = head.trim();
            return Some(match head.strip_prefix("ref: refs/heads/") {
                Some(branch) => branch.to_string(),
                None => head.chars().take(7).collect(),
            });
        }
        at = here.parent();
    }
    None
}

/// Where `program` would be run from: itself when it names a path, else the first match on
/// `PATH`.
fn find_program(program: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn neither_an_api_key_nor_the_monitors_credentials_are_passed_on() {
        let mut builder = CommandBuilder::new("claude");
        builder.env("ANTHROPIC_API_KEY", "sk-test");
        builder.env("CH_SEED_URLS", "http://monitor:secret@ch1:8123");
        builder.env("PATH", "/usr/bin");
        scrub(&mut builder);
        assert!(builder.get_env("ANTHROPIC_API_KEY").is_none(), "the plan, not the API");
        assert!(builder.get_env("CH_SEED_URLS").is_none());
        assert!(builder.get_env("PATH").is_some(), "everything else is");
    }

    #[test]
    fn a_branch_is_read_from_git_s_own_files() {
        let root = std::env::temp_dir().join(format!("cobserve-git-{}", std::process::id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/fix-late-partitions\n").unwrap();
        assert_eq!(git_branch(&repo.join("src/deep")).as_deref(), Some("fix-late-partitions"), "from anywhere inside");
        // A worktree's .git is a file pointing at its own git directory.
        let worktree = root.join("worktree");
        std::fs::create_dir_all(root.join("gitdirs/wt")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), format!("gitdir: {}\n", root.join("gitdirs/wt").display())).unwrap();
        std::fs::write(root.join("gitdirs/wt/HEAD"), "4c8d1b5e2f3a4e1b9d7c\n").unwrap();
        assert_eq!(git_branch(&worktree).as_deref(), Some("4c8d1b5"), "a detached head is its hash, cut");
        assert!(resolve_dir(&repo.display().to_string()).is_ok());
        let err = resolve_dir("/no/such/place").unwrap_err();
        assert!(err.contains("no such directory"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_program_that_is_not_installed_says_so() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let error = PtyProcess::spawn(1, &["no-such-claude-here".into()], Path::new("."), 24, 80, tx).err().unwrap();
        assert!(error.contains("not installed"), "{error}");
        assert!(find_program("sh").is_some());
    }

    /// What the program printed, and how it ended if it did, until `done` says enough or a
    /// few seconds pass. Its end can arrive before its last output: they come from two threads.
    fn collect(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>, done: impl Fn(&str, Option<&str>) -> bool) -> (String, Option<String>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut seen, mut ended) = (String::new(), None);
        while Instant::now() < deadline && !done(&seen, ended.as_deref()) {
            match rx.try_recv() {
                Ok(Event::Pane(_, bytes)) => seen.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Event::PaneExited(_, how)) => ended = Some(how),
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        (seen, ended)
    }

    #[cfg(unix)]
    #[test]
    fn a_program_runs_in_the_pty_and_answers_its_keys() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let script = r#"printf 'size %s\n' "$(stty size)"; printf 'key=[%s] ' "${ANTHROPIC_API_KEY-unset}"; printf 'ready> '; read line; printf 'got:%s\n' "$line""#;
        let dir = std::env::temp_dir();
        let script = format!("printf 'in %s\\n' \"$(pwd -P)\"; {script}");
        let mut process = PtyProcess::spawn(7, &["sh".into(), "-c".into(), script], &dir, 30, 100, tx).expect("sh starts");
        let (seen, _) = collect(&mut rx, |seen, _| seen.contains("ready>"));
        assert!(seen.contains("size 30 100"), "the PTY has the pane's size: {seen}");
        let real = std::fs::canonicalize(&dir).unwrap();
        assert!(seen.contains(&format!("in {}", real.display())), "it runs in its session's directory: {seen}");
        assert!(seen.contains("key=[unset]"), "{seen}");
        process.write(b"hello\r");
        let (seen, ended) = collect(&mut rx, |seen, ended| seen.contains("got:hello") && ended.is_some());
        assert!(seen.contains("got:hello"), "{seen}");
        assert_eq!(ended.as_deref(), Some("it exited"), "and its end is reported");
    }
}
