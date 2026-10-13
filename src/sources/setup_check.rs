//! The setup wizard's live checks (`ui2/wizard.rs`): one read-only request to what a step names,
//! off the UI thread, and a line saying what answered. Nothing is written anywhere: ClickHouse is
//! asked `SELECT version()`, Redash its queue status, Airflow its health, Jira who the token is.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A field left empty for a secret means "keep the one there": the environment's, or the kept one.
fn value(fields: &HashMap<String, String>, name: &str) -> Option<String> {
    fields
        .get(name)
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .or_else(|| crate::setup::Store::load().get(name))
}

fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder().timeout(timeout).build().unwrap_or_default()
}

pub async fn check(step: usize, pairs: Vec<(String, String)>) -> Result<String, String> {
    let fields: HashMap<String, String> = pairs.into_iter().collect();
    match step {
        0 => clickhouse(&fields).await,
        1 => redash(&fields).await,
        2 => airflow(&fields).await,
        3 => jira(&fields).await,
        _ => day(&fields),
    }
}

async fn clickhouse(fields: &HashMap<String, String>) -> Result<String, String> {
    let seeds = value(fields, "CH_SEED_URLS").ok_or("no seed address")?;
    let first = seeds.split(',').map(str::trim).find(|s| !s.is_empty()).ok_or("no seed address")?;
    let seed = crate::config::parse_seed(first, 8123).map_err(|e| format!("the first seed: {e}"))?;
    let timeout = value(fields, "CH_TIMEOUT_MS").and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(1500);
    let (user, password) = match &seed.credentials {
        Some(c) => (c.user.clone(), c.password.clone()),
        None => (value(fields, "CH_USER").unwrap_or_else(|| "default".into()), value(fields, "CH_PASSWORD").unwrap_or_default()),
    };
    let started = Instant::now();
    let answer = client(Duration::from_millis(timeout))
        .get(format!("{}/", seed.url.trim_end_matches('/')))
        .query(&[("query", "SELECT version()")])
        .basic_auth(&user, Some(&password))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { format!("no answer within {timeout} ms") } else { format!("{}: not reachable", seed.host) })?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{} answered HTTP {} — the login?", seed.host, status.as_u16()));
    }
    let cluster = value(fields, "CH_CLUSTER").map(|c| format!(" · cluster {c} is read at the start")).unwrap_or_default();
    Ok(format!("ClickHouse {} on {} answered in {:.1} s{cluster}", text.trim(), seed.host, started.elapsed().as_secs_f64()))
}

async fn redash(fields: &HashMap<String, String>) -> Result<String, String> {
    let url = value(fields, "REDASH_URL").ok_or("no address")?;
    let key = value(fields, "REDASH_ADMIN_API_KEY").ok_or("no admin API key")?;
    let answer = client(Duration::from_secs(10))
        .get(format!("{}/api/admin/queries/rq_status", url.trim_end_matches('/')))
        .header("Authorization", format!("Key {key}"))
        .send()
        .await
        .map_err(|_| "not reachable".to_string())?;
    let status = answer.status();
    if !status.is_success() {
        return Err(format!("HTTP {} — an admin's key is needed", status.as_u16()));
    }
    let body: serde_json::Value = answer.json().await.map_err(|_| "an answer that is not Redash's".to_string())?;
    let queues = body.get("queues").and_then(|q| q.as_object()).map_or(0, |q| q.len());
    let workers = body.get("workers").and_then(|w| w.as_array()).map_or(0, |w| w.len());
    let redis = if value(fields, "REDIS_URL").is_some() { " · Redis is read at the start" } else { " · no Redis: waiting jobs are a count" };
    Ok(format!("Redash answered · {queues} queues · {workers} workers{redis}"))
}

async fn airflow(fields: &HashMap<String, String>) -> Result<String, String> {
    let url = value(fields, "AIRFLOW_URL").ok_or("no address")?;
    let answer = client(Duration::from_secs(10))
        .get(format!("{}/health", url.trim_end_matches('/')))
        .send()
        .await
        .map_err(|_| "not reachable".to_string())?;
    if !answer.status().is_success() {
        return Err(format!("/health answered HTTP {}", answer.status().as_u16()));
    }
    let body: serde_json::Value = answer.json().await.unwrap_or_default();
    let scheduler = body.pointer("/scheduler/status").and_then(|s| s.as_str()).unwrap_or("unknown");
    let login = if value(fields, "AIRFLOW_USER").is_some() && value(fields, "AIRFLOW_PASSWORD").is_some() { "the login is tried at the first read" } else { "no login yet" };
    Ok(format!("Airflow answered · scheduler {scheduler} · {login}"))
}

async fn jira(fields: &HashMap<String, String>) -> Result<String, String> {
    let url = value(fields, "JIRA_URL").ok_or("no address")?;
    let token = value(fields, "JIRA_TOKEN").ok_or("no token")?;
    let answer = client(Duration::from_secs(10))
        .get(format!("{}/rest/api/2/myself", url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "not reachable".to_string())?;
    if !answer.status().is_success() {
        return Err(format!("HTTP {} — the token?", answer.status().as_u16()));
    }
    let body: serde_json::Value = answer.json().await.unwrap_or_default();
    let name = body.get("displayName").and_then(|n| n.as_str()).unwrap_or("someone");
    Ok(format!("Jira answered · the token is {name}'s"))
}

fn day(fields: &HashMap<String, String>) -> Result<String, String> {
    let city = value(fields, "PRAYER_CITY").ok_or("no city · PRAYER_AT=lat,lon works too")?;
    let place = crate::prayer::city(&city).ok_or_else(|| format!("{city} is not a city cobserve knows · PRAYER_AT=lat,lon"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let schedule = crate::prayer::Schedule::around(&place, now);
    let next = schedule.next_prayer(now).map(|m| format!(" · next {} in {}", m.name(), crate::fmt::countdown(m.at - now))).unwrap_or_default();
    Ok(format!("{}{next}", place.name))
}
