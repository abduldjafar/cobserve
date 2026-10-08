//! A Claude plan's limits — the session, the week, a model's own week — as Claude Code's `/usage`
//! shows them: how much of each is used and when it resets. `sources/limits.rs` reads them; this
//! is the shape and the reading of the answer, pure.
//!
//! The percentage is the plan's own figure, of a limit Anthropic does not publish: the
//! denominator is "the limit", and each line says which one.

/// One limit of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    /// `session`, `this week`, `Fable this week`.
    pub label: String,
    /// Used, 0–100, as the plan counts it.
    pub percent: f64,
    /// When it starts again, Unix seconds.
    pub resets_at: Option<i64>,
}

/// The plan and its limits, and where they came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlanLimits {
    /// `Max 5x`, `Max 20x`, `Pro` — from Claude Code's login.
    pub plan: String,
    pub limits: Vec<Limit>,
    /// When the figures were taken, Unix seconds.
    pub as_of: i64,
    /// Asked of Anthropic just now, or Claude Code's last copy (`~/.claude.json`).
    pub live: bool,
    /// Why they are not live, when they are not.
    pub note: Option<String>,
}

impl PlanLimits {
    /// The worst of the limits, as the panel colours it: §7's 75 and 90.
    pub fn severity(&self) -> crate::severity::Severity {
        crate::severity::node(self.limits.iter().map(|l| l.percent).reduce(f64::max))
    }

    /// The session's and the week's percentages, for a line: `session 11% · week 70%`.
    pub fn short(&self) -> String {
        let find = |label: &str| self.limits.iter().find(|l| l.label == label).map(|l| format!("{:.0}%", l.percent));
        match (find("session"), find("this week")) {
            (Some(s), Some(w)) => format!("session {s} · week {w}"),
            (Some(s), None) => format!("session {s}"),
            (None, Some(w)) => format!("week {w}"),
            _ => String::new(),
        }
    }
}

/// `max` + `default_claude_max_5x` → `Max 5x`; `pro` → `Pro`.
pub fn plan_name(subscription: &str, tier: &str) -> String {
    let base = match subscription {
        "max" => "Max",
        "pro" => "Pro",
        "team" => "Team",
        "enterprise" => "Enterprise",
        "" => return String::new(),
        other => other,
    };
    match tier.rsplit('_').next().filter(|t| t.ends_with('x') && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit())) {
        Some(times) if base == "Max" => format!("{base} {times}"),
        _ => base.to_string(),
    }
}

/// The limits in an answer of Claude's usage endpoint — or in Claude Code's copy of one, which
/// has the same shape under `utilization`. Its `limits` list when there is one, else the session
/// and the week from `five_hour` and `seven_day`.
pub fn read(answer: &serde_json::Value) -> Vec<Limit> {
    let at = |v: &serde_json::Value| v.get("resets_at").and_then(|r| r.as_str()).and_then(crate::sources::unix);
    if let Some(list) = answer.get("limits").and_then(|l| l.as_array()).filter(|l| !l.is_empty()) {
        return list
            .iter()
            .filter_map(|l| {
                let percent = l.get("percent").and_then(serde_json::Value::as_f64)?;
                let model = l.pointer("/scope/model/display_name").and_then(|m| m.as_str());
                let label = match (l.get("group").and_then(|g| g.as_str()), model) {
                    (Some("session"), _) => "session".to_string(),
                    (Some("weekly"), Some(model)) => format!("{model} this week"),
                    (Some("weekly"), None) => "this week".to_string(),
                    (Some(group), _) => group.to_string(),
                    (None, _) => l.get("kind").and_then(|k| k.as_str()).unwrap_or("limit").to_string(),
                };
                Some(Limit { label, percent, resets_at: at(l) })
            })
            .collect();
    }
    [("five_hour", "session"), ("seven_day", "this week")]
        .into_iter()
        .filter_map(|(key, label)| {
            let v = answer.get(key)?;
            Some(Limit { label: label.to_string(), percent: v.get("utilization")?.as_f64()?, resets_at: at(v) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_and_its_limits_read_from_the_answer() {
        assert_eq!(plan_name("max", "default_claude_max_5x"), "Max 5x");
        assert_eq!(plan_name("max", "default_claude_max_20x"), "Max 20x");
        assert_eq!(plan_name("pro", "default_claude_ai"), "Pro");
        let answer: serde_json::Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":11.0,"resets_at":"2026-10-08T13:39:59.712049+00:00"},
                "seven_day":{"utilization":70.0,"resets_at":"2026-10-11T12:59:59.712076+00:00"},
                "limits":[
                  {"kind":"session","group":"session","percent":11,"resets_at":"2026-10-08T13:39:59.712049+00:00","scope":null},
                  {"kind":"weekly_all","group":"weekly","percent":70,"resets_at":"2026-10-11T12:59:59.712076+00:00","scope":null},
                  {"kind":"weekly_scoped","group":"weekly","percent":29,"resets_at":"2026-10-11T12:59:59.712311+00:00","scope":{"model":{"id":null,"display_name":"Fable"}}}]}"#,
        )
        .unwrap();
        let limits = read(&answer);
        let labels: Vec<(&str, f64)> = limits.iter().map(|l| (l.label.as_str(), l.percent)).collect();
        assert_eq!(labels, [("session", 11.0), ("this week", 70.0), ("Fable this week", 29.0)]);
        assert_eq!(limits[0].resets_at, Some(1_791_466_799));
        // An older copy with no list: the session and the week from their own keys.
        let old: serde_json::Value = serde_json::from_str(r#"{"five_hour":{"utilization":19,"resets_at":null},"seven_day":{"utilization":100,"resets_at":null}}"#).unwrap();
        assert_eq!(read(&old).iter().map(|l| l.percent).collect::<Vec<_>>(), [19.0, 100.0]);
        let plan = PlanLimits { plan: "Max 5x".into(), limits, as_of: 0, live: true, note: None };
        assert_eq!(plan.short(), "session 11% · week 70%");
        assert_eq!(plan.severity(), crate::severity::Severity::None);
    }
}
