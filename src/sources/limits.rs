//! Reads the Claude plan's limits every five minutes (view 0's AI panel, the sessions' list):
//! asked of the endpoint Claude Code's own `/usage` asks, with Claude Code's login, read from the
//! macOS Keychain where Claude Code keeps it. One GET; the token is never shown, logged, kept or
//! passed on, and never refreshed — that would sign Claude Code out. When it has run out, or the
//! answer does not come, Claude Code's last copy (`~/.claude.json`) stands in, with its date.

use crate::app::Event;
use crate::limits::{self, PlanLimits};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;

const EVERY: Duration = Duration::from_secs(300);
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

pub async fn run(tx: mpsc::UnboundedSender<Event>) {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(15)).build() {
        Ok(client) => client,
        Err(_) => return,
    };
    let mut ticker = tokio::time::interval(EVERY);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if let Some(plan) = read(&client).await
            && tx.send(Event::Limits(Box::new(plan))).is_err()
        {
            return;
        }
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Claude Code's login: its access token, when it runs out (Unix ms), the plan.
struct Login {
    token: String,
    expires_ms: i64,
    plan: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login").field("plan", &self.plan).field("expires_ms", &self.expires_ms).finish_non_exhaustive()
    }
}

async fn login() -> Option<Login> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = tokio::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let oauth = value.get("claudeAiOauth")?;
    let text = |key: &str| oauth.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    Some(Login {
        token: oauth.get("accessToken")?.as_str()?.to_string(),
        expires_ms: oauth.get("expiresAt").and_then(serde_json::Value::as_i64).unwrap_or(0),
        plan: limits::plan_name(&text("subscriptionType"), &text("rateLimitTier")),
    })
}

/// The limits: live when the login is good and the endpoint answers, else Claude Code's copy.
async fn read(client: &reqwest::Client) -> Option<PlanLimits> {
    let login = login().await;
    let plan = login.as_ref().map(|l| l.plan.clone()).unwrap_or_default();
    let why = match &login {
        None => "Claude Code's login is not in the Keychain".to_string(),
        Some(l) if l.expires_ms > 0 && l.expires_ms / 1000 <= now() => "Claude Code's login has run out until it next runs".to_string(),
        Some(l) => match ask(client, &l.token).await {
            Ok(answer) => {
                let found = limits::read(&answer);
                if !found.is_empty() {
                    return Some(PlanLimits { plan, limits: found, as_of: now(), live: true, note: None });
                }
                "the answer had no limits".to_string()
            }
            Err(why) => why,
        },
    };
    cached(plan, why)
}

async fn ask(client: &reqwest::Client, token: &str) -> Result<serde_json::Value, String> {
    let response = client
        .get(ENDPOINT)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "cobserve")
        .send()
        .await
        .map_err(|e| super::transport(e, Duration::from_secs(15)))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json().await.map_err(|_| "an answer that is not the usage".to_string())
}

/// Claude Code's last copy of the figures, from when it last asked.
fn cached(plan: String, why: String) -> Option<PlanLimits> {
    let home = std::env::var_os("HOME")?;
    let text = std::fs::read_to_string(std::path::Path::new(&home).join(".claude.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let copy = value.get("cachedUsageUtilization")?;
    let found = limits::read(copy.get("utilization")?);
    let plan = if plan.is_empty() {
        let account = value.get("oauthAccount");
        let tier = account.and_then(|a| a.get("organizationRateLimitTier")).and_then(|t| t.as_str()).unwrap_or("");
        let kind = account.and_then(|a| a.get("organizationType")).and_then(|t| t.as_str()).unwrap_or("");
        limits::plan_name(kind.strip_prefix("claude_").unwrap_or(kind), tier)
    } else {
        plan
    };
    let as_of = copy.get("fetchedAtMs").and_then(serde_json::Value::as_i64).unwrap_or(0) / 1000;
    (!found.is_empty()).then_some(PlanLimits { plan, limits: found, as_of, live: false, note: Some(why) })
}

#[cfg(test)]
mod tests {
    /// `cargo test live_limits -- --ignored --nocapture`: this Mac's Claude plan and its limits,
    /// read as the app reads them. The plan and the figures only.
    #[tokio::test]
    #[ignore]
    async fn live_limits() {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().unwrap();
        match super::read(&client).await {
            Some(plan) => {
                println!("plan {:?} · live {} · note {:?}", plan.plan, plan.live, plan.note);
                for limit in &plan.limits {
                    println!("  {:<18} {:>5.1}%  resets {:?}", limit.label, limit.percent, limit.resets_at);
                }
            }
            None => println!("no limits"),
        }
    }
}
