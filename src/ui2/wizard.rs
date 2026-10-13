//! `:setup` (`02-panels.md` *Setup wizard*, `screens/120x36-wizard.txt`): five steps — ClickHouse,
//! Redash, Airflow, Jira, the day — one question each with the reason under it, a live check after
//! `⏎` (run by `main.rs` off the UI thread, `sources::setup_check`), and at the end the settings
//! kept by `setup.rs`: the plain ones in `config.toml`, the secrets in the Keychain. The
//! environment still wins over what is kept; what is kept takes effect at the next start.

use crate::app::App;
use crate::setup::Store;
use crate::severity::Severity;

pub struct Field {
    pub label: &'static str,
    /// The variable it sets (`DESIGN.md` §9).
    pub var: &'static str,
    pub secret: bool,
    pub note: &'static str,
}

pub struct Step {
    pub title: &'static str,
    pub question: &'static str,
    pub why: [&'static str; 2],
    pub fields: &'static [Field],
}

const fn f(label: &'static str, var: &'static str, secret: bool, note: &'static str) -> Field {
    Field { label, var, secret, note }
}

pub static STEPS: [Step; 5] = [
    Step {
        title: "ClickHouse",
        question: "Where is ClickHouse?",
        why: ["One or more HTTP addresses, comma-separated. cobserve reads system.clusters from them", "to discover the rest of the fleet. Nothing is ever written to ClickHouse."],
        fields: &[
            f("seeds", "CH_SEED_URLS", false, ""),
            f("cluster", "CH_CLUSTER", false, ""),
            f("user", "CH_USER", false, ""),
            f("password", "CH_PASSWORD", true, "→ macOS Keychain · service cobserve · account CH_PASSWORD"),
            f("timeout", "CH_TIMEOUT_MS", false, "ms · over the VPN a poll takes 0.6–5 s; 1500 marks nodes ↯"),
        ],
    },
    Step {
        title: "Redash",
        question: "Where is Redash?",
        why: ["The admin API key is on an admin's profile page. Redis is optional: without it the", "waiting jobs are a count, with it they have names and ages."],
        fields: &[
            f("url", "REDASH_URL", false, ""),
            f("api key", "REDASH_ADMIN_API_KEY", true, "→ Keychain · REDASH_ADMIN_API_KEY"),
            f("redis", "REDIS_URL", false, "optional · read-only · LRANGE + HGETALL"),
            f("domain", "EMAIL_DOMAIN", false, "people there are shown by name alone"),
        ],
    },
    Step {
        title: "Airflow",
        question: "Where is Airflow?",
        why: ["The login of its web UI: the API is signed into with the login form, because an Airflow", "whose auth_backends is the default refuses Basic auth. A refused login waits 5 minutes."],
        fields: &[f("url", "AIRFLOW_URL", false, ""), f("user", "AIRFLOW_USER", false, ""), f("password", "AIRFLOW_PASSWORD", true, "→ Keychain · AIRFLOW_PASSWORD")],
    },
    Step {
        title: "Jira",
        question: "Where is Jira?",
        why: ["A personal access token: Profile → Personal Access Tokens → Create token. GET only;", "nothing here can change a ticket."],
        fields: &[
            f("url", "JIRA_URL", false, ""),
            f("token", "JIRA_TOKEN", true, "→ Keychain · JIRA_TOKEN"),
            f("columns", "JIRA_STATUSES", false, "left to right · the last is where finished tickets go"),
        ],
    },
    Step {
        title: "The day",
        question: "Where are you?",
        why: ["Prayer times are computed here from the sun, by Kemenag's criteria. A city 300 km from", "the zone's is minutes off, so name yours."],
        fields: &[f("city", "PRAYER_CITY", false, "or PRAYER_AT=lat,lon"), f("remind", "PRAYER_REMIND", false, "minutes before · 0 for none"), f("notify", "NOTIFY", false, "desktop, bell or off")],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wizard {
    pub step: usize,
    pub field: usize,
    /// What each field holds; a secret starts empty — empty keeps the one already kept.
    pub values: Vec<Vec<String>>,
    /// Whether a secret is already set somewhere (the environment or the Keychain).
    pub kept: Vec<Vec<bool>>,
    pub message: Option<(String, Severity)>,
    pub checking: bool,
    /// The values the last check passed with: `⏎` again goes on.
    pub passed: Option<Vec<String>>,
}

impl Wizard {
    /// The wizard at `step`, its fields filled from the environment and what is kept.
    pub fn open(step: usize) -> Wizard {
        let store = Store::load();
        let values = STEPS
            .iter()
            .map(|s| s.fields.iter().map(|f| if f.secret { String::new() } else { std::env::var(f.var).ok().or_else(|| store.get(f.var)).unwrap_or_default() }).collect())
            .collect();
        let kept = STEPS.iter().map(|s| s.fields.iter().map(|f| f.secret && (std::env::var(f.var).is_ok() || store.has_secret(f.var))).collect()).collect();
        Wizard { step: step.min(STEPS.len() - 1), field: 0, values, kept, message: None, checking: false, passed: None }
    }

    pub fn current(&self) -> &'static Step {
        &STEPS[self.step]
    }

    /// The step's fields as `(variable, value)`, for the check.
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.current().fields.iter().zip(&self.values[self.step]).map(|(f, v)| (f.var.to_string(), v.clone())).collect()
    }

    fn next_step(&mut self) -> bool {
        self.message = None;
        self.passed = None;
        self.field = 0;
        if self.step + 1 < STEPS.len() {
            self.step += 1;
            true
        } else {
            false
        }
    }
}

fn toast(app: &mut App, text: impl Into<String>, error: bool) {
    let at = app.clock;
    app.shell.toast(text, error, at);
}

/// Keep everything typed: plain settings to `config.toml`, secrets to the Keychain.
fn finish(app: &mut App, w: &Wizard) {
    let mut values = std::collections::BTreeMap::new();
    let mut secrets = std::collections::BTreeMap::new();
    for (step, vals) in STEPS.iter().zip(&w.values) {
        for (field, value) in step.fields.iter().zip(vals) {
            if field.secret {
                secrets.insert(field.var.to_string(), value.clone());
            } else {
                values.insert(field.var.to_string(), value.clone());
            }
        }
    }
    match crate::setup::save(&values, &secrets) {
        Ok(place) => toast(app, format!("✔ setup saved · {place} · used from the next start"), false),
        Err(why) => toast(app, format!("setup not saved · {why}"), true),
    }
}

/// The answer of a live check.
pub fn checked(app: &mut App, step: usize, result: Result<String, String>) {
    let Some(w) = app.shell.wizard.as_mut() else { return };
    if w.step != step {
        return;
    }
    w.checking = false;
    match result {
        Ok(text) => {
            w.message = Some((format!("✔ {text} · ⏎ goes on"), Severity::Ok));
            w.passed = Some(w.values[step].clone());
        }
        Err(why) => {
            w.message = Some((format!("▲ {why} · ⏎ again checks, esc skips"), Severity::Warn));
            w.passed = None;
        }
    }
}

pub fn key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(mut w) = app.shell.wizard.take() else { return };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let n = w.current().fields.len();
    match key.code {
        KeyCode::Esc => {
            let title = w.current().title.to_lowercase();
            if w.next_step() {
                toast(app, format!("skipped · :setup {title} later"), false);
            } else {
                finish(app, &w);
                return;
            }
        }
        KeyCode::Char('d') if ctrl => {
            toast(app, "a generated fleet to look around: FAKE=1 cobserve", false);
            return;
        }
        KeyCode::Char('c') if ctrl => return,
        KeyCode::Tab | KeyCode::Down => w.field = (w.field + 1) % n,
        KeyCode::BackTab | KeyCode::Up => w.field = (w.field + n - 1) % n,
        KeyCode::Enter if w.checking => {}
        KeyCode::Enter => {
            if w.passed.as_ref() == Some(&w.values[w.step]) {
                if !w.next_step() {
                    finish(app, &w);
                    return;
                }
            } else {
                w.checking = true;
                w.message = Some(("checking…".to_string(), Severity::None));
                app.shell.setup_asks.push((w.step, w.pairs()));
            }
        }
        KeyCode::Backspace => {
            w.values[w.step][w.field].pop();
            w.passed = None;
        }
        KeyCode::Char(c) if !ctrl => {
            w.values[w.step][w.field].push(c);
            w.passed = None;
        }
        _ => {}
    }
    app.shell.wizard = Some(w);
}

/// A paste into the field.
pub fn paste(app: &mut App, text: &str) -> bool {
    let Some(w) = app.shell.wizard.as_mut() else { return false };
    let (step, field) = (w.step, w.field);
    w.values[step][field].push_str(text.lines().next().unwrap_or_default());
    w.passed = None;
    true
}

/// The palette's `setup · <step>` actions.
pub fn open(app: &mut App, step: usize) {
    app.shell.palette = None;
    app.shell.wizard = Some(Wizard::open(step));
}
