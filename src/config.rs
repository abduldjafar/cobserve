//! Environment-only configuration (DESIGN.md §9). Nothing is read from disk and no
//! credential is ever printed — not even in an error message.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ClickHouseConfig {
    /// `http://host:8123` seeds. The fleet is these plus whatever discovery finds (§6.2).
    pub seeds: Vec<String>,
    pub cluster: String,
    pub user: String,
    pub password: String,
    /// The HTTP port assumed for discovered hosts that no seed covers.
    pub http_port: u16,
}

#[derive(Debug, Clone, Default)]
pub struct RedashConfig {
    pub url: Option<String>,
    pub admin_api_key: Option<String>,
    /// Read-only Redis for the names of waiting jobs (§6.3). Absent → counts only.
    pub redis_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub clickhouse: ClickHouseConfig,
    pub redash: RedashConfig,
    pub poll: Duration,
    /// Discovery runs every 60 s (§6.2).
    pub discover_every: Duration,
    pub fake: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    Missing(&'static str),
    Bad(&'static str, String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing(name) => {
                writeln!(f, "{name} is required (DESIGN.md §9); no value is assumed.")?;
                // Saying what to run beats saying what is missing: the three ways in are the
                // build order's step 1, the local rig, and the fleet.
                let ways: [(&str, &str); 3] = [
                    ("FAKE=1 pay_monitoring", "generated fleet, no network"),
                    (
                        "eval \"$(./dev/local-rig.sh env)\" && pay_monitoring",
                        "the local CH 24.10 rig",
                    ),
                    (
                        &fleet_example(name),
                        "the fleet",
                    ),
                ];
                let width = ways.iter().map(|(command, _)| command.len()).max().unwrap_or(0);
                writeln!(f, "Try one of:")?;
                for (command, note) in ways {
                    writeln!(f, "  {command:<width$}  {note}")?;
                }
                Ok(())
            }
            ConfigError::Bad(name, value) => write!(f, "{name} is not usable: {value}"),
        }
    }
}

/// The fleet line, built from whichever variable was missing so the hint names it.
fn fleet_example(name: &str) -> String {
    format!("{name}=http://host:8123 CH_CLUSTER=ch_paysera CH_USER=u CH_PASSWORD=p pay_monitoring")
}

impl std::error::Error for ConfigError {}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    env(name).ok_or(ConfigError::Missing(name))
}

fn number<T: std::str::FromStr>(name: &'static str, default: T) -> Result<T, ConfigError> {
    match env(name) {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| ConfigError::Bad(name, format!("{raw:?} is not a number"))),
    }
}

impl Config {
    /// Read the environment. `FAKE=1` needs none of the ClickHouse variables: step 1 of the
    /// build order has to run with no network and no credentials at all.
    pub fn from_env() -> Result<Self, ConfigError> {
        let fake = env("FAKE").as_deref() == Some("1");

        let clickhouse = if fake {
            ClickHouseConfig {
                seeds: Vec::new(),
                cluster: String::new(),
                user: String::new(),
                password: String::new(),
                http_port: 8123,
            }
        } else {
            let seeds: Vec<String> = required("CH_SEED_URLS")?
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if seeds.is_empty() {
                return Err(ConfigError::Missing("CH_SEED_URLS"));
            }
            ClickHouseConfig {
                seeds,
                cluster: required("CH_CLUSTER")?,
                user: required("CH_USER")?,
                password: required("CH_PASSWORD")?,
                http_port: number("CH_HTTP_PORT", 8123u16)?,
            }
        };

        Ok(Config {
            clickhouse,
            redash: RedashConfig {
                url: env("REDASH_URL"),
                admin_api_key: env("REDASH_ADMIN_API_KEY"),
                redis_url: env("REDIS_URL"),
            },
            poll: Duration::from_millis(number("POLL_MS", 2000u64)?),
            discover_every: Duration::from_secs(60),
            fake,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, not three: the process environment is shared by every test thread, so cases
    /// that set and unset the same variables have to run in sequence.
    #[test]
    fn the_environment_is_read_and_nothing_is_assumed() {
        // SAFETY: this module's tests are the only ones touching these variables.
        unsafe {
            std::env::remove_var("FAKE");
            std::env::remove_var("POLL_MS");
            std::env::remove_var("CH_SEED_URLS");
        }

        let err = Config::from_env().unwrap_err().to_string();
        assert!(err.contains("CH_SEED_URLS"), "{err}");
        assert!(err.contains("§9"), "the message points at the spec: {err}");
        // The hint has to name the three ways in: fake data, the local rig, the fleet.
        assert!(err.contains("FAKE=1"), "{err}");
        assert!(err.contains("dev/local-rig.sh"), "{err}");
        assert!(err.contains("CH_CLUSTER"), "{err}");

        // With the seeds satisfied, a bad number is still an error rather than a default.
        unsafe {
            std::env::set_var("FAKE", "1");
            std::env::set_var("POLL_MS", "soon");
        }
        let err = Config::from_env().unwrap_err().to_string();
        assert!(err.contains("POLL_MS"), "a bad number is an error: {err}");
        unsafe { std::env::remove_var("POLL_MS") };

        let config = Config::from_env().expect("FAKE=1 runs with nothing configured");
        assert!(config.fake);
        assert!(config.clickhouse.seeds.is_empty());
        assert_eq!(config.poll, Duration::from_millis(2000), "the default from §9");
        assert!(config.redash.url.is_none(), "Redash is optional");
        unsafe { std::env::remove_var("FAKE") };
    }
}
