//! View 5's sessions kept across runs: what each runs, where, under what name, and which
//! conversation it was in — so the next start shows them again, and each takes its conversation
//! up where it was left when it is opened.
//!
//! Kept in `$XDG_STATE_HOME/fleetlens/sessions.json` (`~/.local/state/fleetlens/` without it),
//! readable by its owner alone. No credential is in it, and nothing of the conversations
//! themselves: Claude Code and OpenCode keep those.

use crate::claude::Kind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub sessions: Vec<SavedSession>,
    /// The one that was on screen.
    #[serde(default)]
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    pub kind: SavedKind,
    #[serde(default)]
    pub name: Option<String>,
    pub dir: String,
    /// The conversation to take up: Claude Code's session id, OpenCode's.
    #[serde(default)]
    pub conversation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedKind {
    Claude,
    OpenCode,
    Terminal,
}

impl From<Kind> for SavedKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Claude => SavedKind::Claude,
            Kind::OpenCode => SavedKind::OpenCode,
            Kind::Terminal => SavedKind::Terminal,
        }
    }
}

impl From<SavedKind> for Kind {
    fn from(kind: SavedKind) -> Self {
        match kind {
            SavedKind::Claude => Kind::Claude,
            SavedKind::OpenCode => Kind::OpenCode,
            SavedKind::Terminal => Kind::Terminal,
        }
    }
}

/// Where the sessions are kept.
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state")))?;
    Some(state.join("fleetlens").join("sessions.json"))
}

/// The sessions of the last run; none when there were none, or the file is not one this reads.
pub fn load() -> Saved {
    path().and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Keep `saved` for the next run: written beside the old file and moved over it, so a crash
/// halfway leaves the old one whole.
pub fn store(saved: &Saved) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_survive_a_round_trip_and_a_file_from_elsewhere_is_ignored() {
        let saved = Saved {
            sessions: vec![
                SavedSession { kind: SavedKind::Claude, name: Some("billing export".into()), dir: "~/work/billing".into(), conversation: Some("aaa".into()) },
                SavedSession { kind: SavedKind::Terminal, name: None, dir: "~/work/reports".into(), conversation: None },
            ],
            active: 1,
        };
        let text = serde_json::to_string(&saved).unwrap();
        assert!(text.contains("\"kind\":\"claude\"") && text.contains("\"kind\":\"terminal\""), "{text}");
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
        // Older files, with less in them, still read.
        let old: Saved = serde_json::from_str(r#"{"sessions":[{"kind":"opencode","dir":"~/x"}]}"#).unwrap();
        assert_eq!((old.sessions[0].kind, old.active, old.sessions[0].conversation.clone()), (SavedKind::OpenCode, 0, None));
        assert!(serde_json::from_str::<Saved>(r#"{"sessions":[{"kind":"emacs","dir":"~"}]}"#).is_err());
    }
}
