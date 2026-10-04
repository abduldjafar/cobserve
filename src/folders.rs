//! The folders a new session can work in, for view 5's picker (`claude::Picker`): what is in
//! one, a search below it, or a path as typed. Every lookup runs on a thread of its own,
//! started by `main.rs`, and stops as soon as a newer one replaces it; the picker only asks
//! and draws what comes back.

use crate::pty;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A folder the picker can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    /// What its row says: its name; below where a search began, the path from there; for a
    /// typed path, the whole of it.
    pub shown: String,
    /// The part of `shown` the search matched, as a byte range, to light it up.
    pub hit: Option<(usize, usize)>,
    /// Its branch, when the folder is a git repository's own.
    pub branch: Option<String>,
}

/// What the picker asks: the folders in `dir`, or those `query` finds from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub generation: u64,
    pub dir: PathBuf,
    pub query: String,
}

/// What comes back; for a search, more than once as it goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub generation: u64,
    /// The folder looked in, as found on disk; `None` when it could not be.
    pub dir: Option<PathBuf>,
    /// Its own branch, for the row that opens a session in it.
    pub branch: Option<String>,
    pub folders: Vec<Folder>,
    /// Nothing more is coming for this lookup.
    pub done: bool,
    /// The search stopped at its limits, and folders further down were not read.
    pub cut_short: bool,
    pub error: Option<String>,
}

/// How far a search goes below where it starts — levels, folders read, time — and how many
/// of its matches are kept. Past that, going into a folder and searching from there is quicker.
const DEPTH: usize = 6;
const VISITS: usize = 40_000;
const BUDGET: Duration = Duration::from_secs(4);
const KEEP: usize = 300;
/// How often a search shows what it has so far.
const PROGRESS: Duration = Duration::from_millis(120);
/// Never searched into: thousands of folders, none of them where a session is opened.
const SKIP: [&str; 4] = ["node_modules", "target", "__pycache__", "venv"];
/// Not searched into from `/`: the kernel's own, not anybody's work.
const SYSTEM: [&str; 4] = ["proc", "sys", "dev", "run"];

/// Answer `lookup` through `send` unless a newer lookup has replaced it in `latest`.
pub fn look(lookup: &Lookup, latest: &AtomicU64, send: &dyn Fn(Found)) {
    let current = || latest.load(Ordering::SeqCst) == lookup.generation;
    let mut found = Found { generation: lookup.generation, ..Found::default() };
    let dir = match resolve(&lookup.dir) {
        Ok(dir) => dir,
        Err(why) => {
            found.error = Some(why);
            found.done = true;
            if current() {
                send(found);
            }
            return;
        }
    };
    found.branch = pty::git_branch(&dir);
    found.dir = Some(dir.clone());
    let query = lookup.query.trim();
    if query.is_empty() {
        match children(&dir, false) {
            Ok(folders) => found.folders = folders,
            Err(why) => found.error = Some(why),
        }
    } else if is_path(query) {
        found.folders = typed_path(&dir, query);
    } else {
        search(&dir, query, &current, &mut |folders, done, cut_short| {
            if current() {
                send(Found { folders, done, cut_short, ..found.clone() });
            }
        });
        return;
    }
    found.done = true;
    if current() {
        send(found);
    }
}

/// A query that is a path rather than a name: `~`, `/tmp`, `../other`, `work/co`.
fn is_path(query: &str) -> bool {
    query.starts_with('~') || query.contains('/') || matches!(query, "." | "..")
}

/// The folder a lookup starts from, which must be one.
fn resolve(dir: &Path) -> Result<PathBuf, String> {
    if dir.is_absolute() {
        if dir.is_dir() { Ok(dir.to_path_buf()) } else { Err(format!("no such folder: {}", pty::tilde(dir))) }
    } else {
        pty::resolve_dir(&dir.to_string_lossy())
    }
}

/// An error as a person would say it, without the OS's number.
fn plain(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "no permission to read it".to_string(),
        std::io::ErrorKind::NotFound => "it is gone".to_string(),
        _ => error.to_string(),
    }
}

/// Its branch, when `path` is a repository's own folder — not one somewhere inside it.
fn repo_branch(path: &Path) -> Option<String> {
    path.join(".git").exists().then(|| pty::git_branch(path)).flatten()
}

/// Where `needle` (lower case) is in `name`, as a byte range of `name`; `None` when it is not.
/// A name whose lower case is not the same length cannot be lit up by range, but still matches.
fn find(name: &str, needle: &str) -> Option<Option<(usize, usize)>> {
    let lower = name.to_lowercase();
    let at = lower.find(needle)?;
    Some((lower.len() == name.len()).then_some((at, at + needle.len())))
}

/// The folders in `dir`, by name — hidden ones only when asked for.
fn children(dir: &Path, hidden: bool) -> Result<Vec<Folder>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("can't read {}: {}", pty::tilde(dir), plain(&e)))?;
    let mut folders: Vec<Folder> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            ((hidden || !name.starts_with('.')) && path.is_dir()).then(|| Folder {
                branch: repo_branch(&path),
                shown: name,
                hit: None,
                path,
            })
        })
        .collect();
    folders.sort_by_cached_key(|f| f.shown.to_lowercase());
    Ok(folders)
}

/// A path as typed, the way a shell completes one: in the folder before its last `/`, the
/// folders whose name has what follows — those that begin with it first — and when nothing
/// follows, that folder itself before them.
fn typed_path(dir: &Path, query: &str) -> Vec<Folder> {
    let (base, rest) = match query.rsplit_once('/') {
        Some(("", rest)) => ("/", rest),
        Some((base, rest)) => (base, rest),
        None => (query, ""),
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = match (base.strip_prefix('~'), home) {
        (Some(after), Some(home)) => home.join(after.trim_start_matches('/')),
        _ if Path::new(base).is_absolute() => PathBuf::from(base),
        _ => dir.join(base),
    };
    // `..` and `.` read as a person means them, not as segments to keep.
    let Ok(base) = base.canonicalize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if rest.is_empty() {
        out.push(Folder { shown: pty::tilde(&base), hit: None, branch: pty::git_branch(&base), path: base.clone() });
    }
    let needle = rest.to_lowercase();
    let mut matches: Vec<(bool, Folder)> = children(&base, rest.starts_with('.'))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|mut folder| {
            let hit = find(&folder.shown, &needle)?;
            let begins = hit.is_none_or(|(at, _)| at == 0);
            folder.shown = pty::tilde(&folder.path);
            let name_at = folder.shown.len() - folder.path.file_name().map_or(0, |n| n.to_string_lossy().len());
            folder.hit = hit.filter(|(a, b)| a < b).map(|(a, b)| (name_at + a, name_at + b));
            Some((!begins, folder))
        })
        .collect();
    matches.sort_by_key(|(later, _)| *later);
    out.extend(matches.into_iter().map(|(_, folder)| folder).take(KEEP));
    out
}

/// Folders below `root` whose name has `query` in it, best first: the name itself, then
/// names that begin with it, then the rest, the nearest first in each. Breadth-first, so what
/// is near is found first, within `DEPTH` levels, `VISITS` folders and `BUDGET` — `report`ed
/// as it goes and once at the end.
fn search(root: &Path, query: &str, current: &dyn Fn() -> bool, report: &mut dyn FnMut(Vec<Folder>, bool, bool)) {
    let needle = query.to_lowercase();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let started = Instant::now();
    let mut shown_at = started;
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
    // (how well, how deep, the folder)
    let mut matches: Vec<(u8, usize, Folder)> = Vec::new();
    let mut visits = 0;
    let mut cut_short = false;
    let best = |matches: &mut Vec<(u8, usize, Folder)>| {
        matches.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then_with(|| a.2.shown.to_lowercase().cmp(&b.2.shown.to_lowercase())));
        matches.truncate(KEEP);
        matches.iter().map(|(_, _, folder)| folder.clone()).collect::<Vec<_>>()
    };
    while let Some((dir, depth)) = queue.pop_front() {
        if !current() {
            return;
        }
        if visits >= VISITS || started.elapsed() >= BUDGET {
            cut_short = true;
            break;
        }
        visits += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            // A link to a folder is offered, never followed: links can go round in circles.
            if !(kind.is_dir() || (kind.is_symlink() && path.is_dir())) {
                continue;
            }
            if let Some(hit) = find(&name, &needle) {
                let rank = match hit {
                    _ if name.to_lowercase() == needle => 0,
                    Some((0, _)) => 1,
                    _ => 2,
                };
                let shown = path.strip_prefix(root).unwrap_or(&path).display().to_string();
                let name_at = shown.len() - name.len();
                let hit = hit.map(|(a, b)| (name_at + a, name_at + b));
                matches.push((rank, depth + 1, Folder { branch: repo_branch(&path), shown, hit, path: path.clone() }));
            }
            let skipped = SKIP.contains(&name.as_str())
                || (home.as_deref() == Some(dir.as_path()) && name == "Library")
                || (dir == Path::new("/") && SYSTEM.contains(&name.as_str()));
            if kind.is_dir() && depth + 1 < DEPTH && !skipped {
                queue.push_back((path, depth + 1));
            }
        }
        if matches.len() > 4 * KEEP {
            best(&mut matches);
        }
        if shown_at.elapsed() >= PROGRESS {
            shown_at = Instant::now();
            report(best(&mut matches), false, false);
        }
    }
    report(best(&mut matches), true, cut_short);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A tree to look in, removed when dropped.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("pay_monitoring-folders-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for dir in ["alpha/.git", "beta/deep/cobweb", "Cobalt", ".hidden/cobra", "node_modules/cobfoo"] {
                std::fs::create_dir_all(root.join(dir)).unwrap();
            }
            std::fs::write(root.join("alpha/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::write(root.join("notes.txt"), "not a folder").unwrap();
            Self(root)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ask(dir: &Path, query: &str) -> Vec<Found> {
        let latest = AtomicU64::new(7);
        let got = RefCell::new(Vec::new());
        look(&Lookup { generation: 7, dir: dir.to_path_buf(), query: query.into() }, &latest, &|found| got.borrow_mut().push(found));
        got.into_inner()
    }

    fn shown(found: &Found) -> Vec<&str> {
        found.folders.iter().map(|f| f.shown.as_str()).collect()
    }

    #[test]
    fn a_folder_lists_its_folders_by_name_with_their_branches() {
        let tree = Tree::new("list");
        let answers = ask(&tree.0, "");
        assert_eq!(answers.len(), 1);
        let found = &answers[0];
        assert!(found.done && found.error.is_none());
        assert_eq!(found.dir.as_deref(), Some(tree.0.as_path()));
        assert_eq!(shown(found), ["alpha", "beta", "Cobalt", "node_modules"], "no files, nothing hidden");
        assert_eq!(found.folders[0].branch.as_deref(), Some("main"), "alpha is a repository");
        assert_eq!(found.folders[1].branch, None, "beta is not one");
    }

    #[test]
    fn a_search_finds_names_below_best_and_nearest_first() {
        let tree = Tree::new("search");
        let answers = ask(&tree.0, "COB");
        let last = answers.last().unwrap();
        assert!(last.done && !last.cut_short);
        // node_modules is not searched into, a hidden folder is not either.
        assert_eq!(shown(last), ["Cobalt", "beta/deep/cobweb"]);
        assert_eq!(last.folders[0].hit, Some((0, 3)));
        assert_eq!(&last.folders[1].shown[10..13], "cob", "the match, to light up");
        assert_eq!(last.folders[1].hit, Some((10, 13)));
        let exact = ask(&tree.0, "deep");
        assert_eq!(shown(exact.last().unwrap()), ["beta/deep"]);
    }

    #[test]
    fn a_typed_path_is_completed_like_a_shell_does() {
        let tree = Tree::new("path");
        let found = ask(&tree.0, "beta/").pop().unwrap();
        let beta = tree.0.join("beta").canonicalize().unwrap();
        assert_eq!(found.folders[0].path, beta, "the folder itself first");
        assert_eq!(found.folders[1].path, beta.join("deep"));
        let found = ask(&tree.0, "beta/DE").pop().unwrap();
        assert_eq!(found.folders.len(), 1);
        let deep = &found.folders[0];
        assert!(deep.shown.ends_with("/beta/deep"), "the whole path: {}", deep.shown);
        let (a, b) = deep.hit.unwrap();
        assert_eq!(&deep.shown[a..b], "de");
        let up = ask(&tree.0.join("beta"), "../Cob").pop().unwrap();
        assert_eq!(up.folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), [tree.0.canonicalize().unwrap().join("Cobalt")]);
    }

    #[test]
    fn a_lookup_replaced_by_a_newer_one_says_nothing() {
        let tree = Tree::new("stale");
        let latest = AtomicU64::new(8);
        let got = RefCell::new(Vec::new());
        look(&Lookup { generation: 7, dir: tree.0.clone(), query: "cob".into() }, &latest, &|found| got.borrow_mut().push(found));
        assert!(got.borrow().is_empty());
    }

    #[test]
    fn a_folder_that_is_not_there_says_so() {
        let found = ask(Path::new("/no/such/place"), "").pop().unwrap();
        assert!(found.done && found.dir.is_none());
        assert!(found.error.unwrap().contains("no such folder"));
    }
}
