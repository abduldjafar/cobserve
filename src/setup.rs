//! What `:setup` keeps (`docs/redesign/02-panels.md` *Setup wizard*): the plain settings in
//! `~/.config/cobserve/config.toml`, the secrets in the macOS Keychain (service `cobserve`, the
//! account named as the variable) or, where there is no Keychain, in `secrets.toml` beside it at
//! mode 0600 — which the wizard says before writing. The environment still wins over both:
//! `Config::load` asks the environment first, then this store (`DESIGN.md` §9, "environment-first").
//!
//! The file is the variables themselves, one `NAME = "value"` a line, so what the wizard writes is
//! what a person would export.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The variables that are secrets: never in `config.toml`, never on screen.
pub const SECRETS: [&str; 4] = ["CH_PASSWORD", "REDASH_ADMIN_API_KEY", "AIRFLOW_PASSWORD", "JIRA_TOKEN"];

/// Where the secrets went, as the file records it.
const SECRETS_AT: &str = "COBSERVE_SECRETS";
const KEYCHAIN: &str = "keychain";
const FILE: &str = "file";
const SERVICE: &str = "cobserve";

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/cobserve"))
}

pub fn config_path() -> Option<PathBuf> {
    dir().map(|d| d.join("config.toml"))
}

fn secrets_path() -> Option<PathBuf> {
    dir().map(|d| d.join("secrets.toml"))
}

/// `NAME = "value"` lines; `#` comments and blank lines skipped. A value is a TOML basic string
/// with `\"` and `\\` escaped.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else { continue };
        let mut unescaped = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => unescaped.push('\n'),
                    Some('t') => unescaped.push('\t'),
                    Some(other) => unescaped.push(other),
                    None => {}
                }
            } else {
                unescaped.push(c);
            }
        }
        out.insert(name.trim().to_string(), unescaped);
    }
    out
}

/// The lines of `values`, as `parse` reads them back.
pub fn render(header: &str, values: &BTreeMap<String, String>) -> String {
    let mut text = String::new();
    for line in header.lines() {
        text.push_str(&format!("# {line}\n"));
    }
    for (name, value) in values {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        text.push_str(&format!("{name} = \"{escaped}\"\n"));
    }
    text
}

/// The plain settings, and where the secrets are.
#[derive(Debug, Clone, Default)]
pub struct Store {
    pub values: BTreeMap<String, String>,
}

impl Store {
    pub fn load() -> Store {
        let values = config_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default();
        Store { values }
    }

    /// A setting by its variable's name: a plain one from the file, a secret from where the file
    /// says the secrets went.
    pub fn get(&self, name: &str) -> Option<String> {
        if SECRETS.contains(&name) {
            return match self.values.get(SECRETS_AT).map(String::as_str) {
                Some(KEYCHAIN) => keychain::get(SERVICE, name),
                Some(FILE) => secrets_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse(&t).remove(name)),
                _ => None,
            };
        }
        self.values.get(name).filter(|v| !v.is_empty()).cloned()
    }

    /// Whether a secret is kept, without reading it.
    pub fn has_secret(&self, name: &str) -> bool {
        self.values.get(&format!("{name}_SET")).is_some_and(|v| v == "yes")
    }
}

/// Keep `values` (plain) and `secrets` (only the ones typed: an empty one keeps what was there).
/// Returns where the secrets went, in words for the toast.
pub fn save(values: &BTreeMap<String, String>, secrets: &BTreeMap<String, String>) -> Result<String, String> {
    let dir = dir().ok_or("no HOME to keep the settings under")?;
    save_in(&dir, values, secrets, true)
}

/// `save` into `dir`; `keychain` false keeps the secrets in the 0600 file.
fn save_in(dir: &std::path::Path, values: &BTreeMap<String, String>, secrets: &BTreeMap<String, String>, keychain: bool) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config_path = dir.join("config.toml");
    let secrets_path = dir.join("secrets.toml");
    let mut store = std::fs::read_to_string(&config_path).ok().map(|t| parse(&t)).unwrap_or_default();
    for (k, v) in values {
        if v.trim().is_empty() {
            store.remove(k);
        } else {
            store.insert(k.clone(), v.trim().to_string());
        }
    }
    let typed: Vec<(&String, &String)> = secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
    let mut place = match store.get(SECRETS_AT).map(String::as_str) {
        Some(FILE) => FILE,
        _ if !keychain => FILE,
        _ => KEYCHAIN,
    };
    if !typed.is_empty() && place == KEYCHAIN {
        for (name, secret) in &typed {
            if keychain::set(SERVICE, name, secret).is_err() {
                place = FILE;
                break;
            }
        }
    }
    if !typed.is_empty() && place == FILE {
        let path = secrets_path.clone();
        let mut kept = std::fs::read_to_string(&path).ok().map(|t| parse(&t)).unwrap_or_default();
        for (name, secret) in &typed {
            kept.insert((*name).clone(), (*secret).clone());
        }
        write_private(&path, &render("cobserve's secrets, written by :setup — readable by you alone (0600)", &kept))?;
    }
    for (name, _) in &typed {
        store.insert(format!("{name}_SET"), "yes".into());
    }
    store.insert(SECRETS_AT.into(), place.into());
    let path = config_path;
    let header = "cobserve's settings, written by :setup.\nThe environment wins over every line here; the secrets are not here.";
    std::fs::write(&path, render(header, &store)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(match place {
        KEYCHAIN => format!("{} · secrets in the Keychain", tilde(&path)),
        _ => format!("{} · secrets in {} (0600)", tilde(&path), tilde(&secrets_path)),
    })
}

fn tilde(path: &std::path::Path) -> String {
    crate::pty::tilde(path)
}

/// Write `text` to `path` readable by its owner alone.
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(target_os = "macos")]
mod keychain {
    pub fn get(service: &str, account: &str) -> Option<String> {
        let bytes = security_framework::passwords::get_generic_password(service, account).ok()?;
        String::from_utf8(bytes).ok()
    }

    pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(service, account, secret.as_bytes()).map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub fn get(_: &str, _: &str) -> Option<String> {
        None
    }

    pub fn set(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err("no Keychain here".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_the_variables_and_reads_back() {
        let mut values = BTreeMap::new();
        values.insert("CH_SEED_URLS".to_string(), "http://a:8123,http://b:8123".to_string());
        values.insert("JIRA_STATUSES".to_string(), "In progress, \"Done\"".to_string());
        let text = render("a header", &values);
        assert!(text.starts_with("# a header\n"));
        assert_eq!(parse(&text), values);
        assert_eq!(parse("# x\n\nA = \"1\"\nnot a line\nB=\"two \\\\ three\""), BTreeMap::from([("A".into(), "1".into()), ("B".into(), "two \\ three".into())]));
    }

    #[test]
    fn saved_settings_and_a_private_secrets_file() {
        let dir = std::env::temp_dir().join(format!("cobserve-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let values = BTreeMap::from([("CH_SEED_URLS".to_string(), "http://a:8123".to_string()), ("CH_CLUSTER".to_string(), String::new())]);
        let secrets = BTreeMap::from([("CH_PASSWORD".to_string(), "pw\"1".to_string()), ("JIRA_TOKEN".to_string(), String::new())]);
        let said = save_in(&dir, &values, &secrets, false).unwrap();
        assert!(said.contains("(0600)"), "{said}");
        let config = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let parsed = parse(&config);
        assert_eq!(parsed.get("CH_SEED_URLS").map(String::as_str), Some("http://a:8123"));
        assert!(!parsed.contains_key("CH_CLUSTER"), "an empty setting is not kept");
        assert!(!config.contains("pw"), "no secret in config.toml");
        assert_eq!(parsed.get("CH_PASSWORD_SET").map(String::as_str), Some("yes"));
        assert!(!parsed.contains_key("JIRA_TOKEN_SET"), "an empty secret keeps whatever was there");
        let kept = parse(&std::fs::read_to_string(dir.join("secrets.toml")).unwrap());
        assert_eq!(kept.get("CH_PASSWORD").map(String::as_str), Some("pw\"1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("secrets.toml")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_secret_is_never_a_plain_setting() {
        let store = Store { values: BTreeMap::from([("CH_PASSWORD".into(), "leaked".into())]) };
        assert_eq!(store.get("CH_PASSWORD"), None, "a secret is read from where the secrets are, never from the file");
    }
}
