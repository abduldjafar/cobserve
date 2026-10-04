//! Configuration (DESIGN.md §9): the environment, plus — only when the command line names one —
//! a credential file. Nothing else is read from disk, and no credential is ever printed, not
//! even in an error message.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A ClickHouse login. The password never leaves the request headers: not in `Debug`, not in
/// an error, not on screen (§9).
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .field("password", &"…")
            .finish()
    }
}

/// One server to start from: where it is, and how to log in to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// `scheme://host:port` without the login — the only form of it that is ever shown.
    pub url: String,
    pub host: String,
    pub port: u16,
    /// Its own login, from the credential file or from the URL; `None` → the default login.
    pub credentials: Option<Credentials>,
}

#[derive(Debug, Clone)]
pub struct ClickHouseConfig {
    /// The seeds. The fleet is these plus whatever discovery finds (§6.2).
    pub seeds: Vec<Seed>,
    pub cluster: String,
    /// The login for a seed without one of its own and for every host discovery adds:
    /// `default_login` in the credential file, or `CH_USER` / `CH_PASSWORD`. `None` when each
    /// seed brings its own.
    pub default_login: Option<Credentials>,
    /// The HTTP port assumed for discovered hosts that no seed covers.
    pub http_port: u16,
}

#[derive(Clone, Default)]
pub struct RedashConfig {
    pub url: Option<String>,
    pub admin_api_key: Option<String>,
    /// Read-only Redis for the names of waiting jobs (§6.3). Absent → counts only.
    pub redis_url: Option<String>,
}

impl std::fmt::Debug for RedashConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The API key is a credential, and a Redis URL carries its password (§9).
        f.debug_struct("RedashConfig")
            .field("url", &self.url)
            .field("admin_api_key", &self.admin_api_key.as_ref().map(|_| "…"))
            .field("redis_url", &self.redis_url.as_ref().map(|_| "…"))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub clickhouse: ClickHouseConfig,
    pub redash: RedashConfig,
    pub poll: Duration,
    /// Discovery runs every 60 s (§6.2).
    pub discover_every: Duration,
    pub fake: bool,
    /// The organisation's own e-mail domain: people there are shown by the local part alone
    /// (§6.4). `EMAIL_DOMAIN`, or `email_domain:` under `redash:` in the credential file.
    pub email_domain: Option<String>,
    /// Worth saying, not worth refusing to start over — shown in the footer.
    pub warnings: Vec<String>,
    /// What view 5's sessions run, each split like a shell would: `CLAUDE_CMD` (`claude` by
    /// default), `OPENCODE_CMD` (`opencode`), `SHELL_CMD` (`$SHELL`, else `/bin/sh`).
    pub claude_command: Vec<String>,
    pub opencode_command: Vec<String>,
    pub shell_command: Vec<String>,
    /// Clicks and the wheel (`MOUSE=0` turns them off, and with them the terminal's own
    /// selection comes back without a modifier key).
    pub mouse: bool,
    /// The prayer times on the day line.
    pub prayer: PrayerConfig,
    /// `TIME=utc`: the clock starts on UTC rather than the machine's own zone.
    pub utc: bool,
    /// How a prayer's reminder reaches you off the screen.
    pub notify: Notify,
}

/// Where and whether: `PRAYER_CITY` (a city by name) or `PRAYER_AT` (`lat,lon`), else the city
/// of the machine's time zone; `PRAYER=off` leaves them out. `PRAYER_REMIND` is how many minutes
/// ahead the reminder comes (10; 0 for none).
#[derive(Debug, Clone, PartialEq)]
pub struct PrayerConfig {
    pub off: bool,
    pub place: Option<crate::prayer::Place>,
    pub remind_minutes: u32,
}

/// `NOTIFY`: `off`, `bell` (the terminal's bell only) or, by default, `desktop` — the bell, the
/// terminal's own notification and the system's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    Off,
    Bell,
    Desktop,
}

pub const USAGE: &str = "\
Usage: cobserve [--credential FILE] [--claude]

  --credential FILE   the ClickHouse servers with a login for each (and optionally the
                      cluster and Redash) in YAML — see credentials.example.yaml. What the
                      file says wins over the environment.
  --claude            open on view 5: Claude Code in a pane, the monitor above it
  -h, --help          this text
  -V, --version       the version

Everything else comes from the environment (DESIGN.md §9): CH_SEED_URLS, CH_CLUSTER,
CH_USER, CH_PASSWORD, CH_HTTP_PORT, REDASH_URL, REDASH_ADMIN_API_KEY, REDIS_URL,
EMAIL_DOMAIN, POLL_MS, THEME, NO_COLOR, FAKE=1 for a generated fleet, CLAUDE_CMD,
OPENCODE_CMD and SHELL_CMD for what view 5's sessions run (default: claude, opencode, $SHELL),
MOUSE=0 to leave the mouse to the terminal, TIME=utc for a UTC clock (the machine's own
zone otherwise), PRAYER_CITY (e.g. Bandung) or PRAYER_AT (lat,lon) for where the prayer
times are for (the time zone's city otherwise), PRAYER_REMIND for how many minutes ahead
the reminder comes (10; 0 for none), PRAYER=off, and NOTIFY=bell or NOTIFY=off.
";

/// `variable` as a command and its arguments, quotes as a shell reads them; `default` when it
/// is not set.
fn command(variable: &'static str, raw: Option<String>, default: &str) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = raw.filter(|r| !r.trim().is_empty()) else {
        return Ok(vec![default.to_string()]);
    };
    match shell_words::split(&raw) {
        Ok(words) if !words.is_empty() => Ok(words),
        _ => Err(ConfigError::Bad(variable, "is not a command a shell could read (an unclosed quote?)".to_string())),
    }
}

/// The command line. It says where the credential file is and which view to open on; the rest
/// is environment.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub credential: Option<PathBuf>,
    /// Open on view 5, with Claude started.
    pub claude: bool,
    pub help: bool,
    pub version: bool,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ConfigError> {
        let mut parsed = Args::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => parsed.help = true,
                "-V" | "--version" => parsed.version = true,
                "--claude" => parsed.claude = true,
                "-c" | "--credential" | "--credentials" => {
                    let path = args.next().filter(|p| !p.is_empty()).ok_or_else(|| {
                        ConfigError::Usage(format!("{arg} needs a file, e.g. {arg} credentials.yaml"))
                    })?;
                    parsed.credential = Some(PathBuf::from(path));
                }
                other => match other
                    .strip_prefix("--credential=")
                    .or_else(|| other.strip_prefix("--credentials="))
                {
                    Some(path) if !path.is_empty() => parsed.credential = Some(PathBuf::from(path)),
                    // A flag is repeated back; anything else might be a password typed in the
                    // wrong place, so it is not.
                    _ if other.starts_with('-') => {
                        return Err(ConfigError::Usage(format!("unknown option {other}")));
                    }
                    _ => return Err(ConfigError::Usage("unexpected argument".to_string())),
                },
            }
        }
        Ok(parsed)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Missing(&'static str),
    Bad(&'static str, String),
    Usage(String),
    /// The credential file (as named on the command line) and what is wrong with it.
    File(String, String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing(name) => {
                writeln!(f, "{name} is required (DESIGN.md §9); no value is assumed.")?;
                match *name {
                    "CH_SEED_URLS" => writeln!(f, "Or list the servers under clickhouse: servers: in a --credential file.")?,
                    "CH_CLUSTER" => writeln!(f, "Or set clickhouse: cluster: in the --credential file.")?,
                    "CH_USER" | "CH_PASSWORD" => writeln!(
                        f,
                        "CH_USER and CH_PASSWORD go together: they are the login for every server without one of its own (in the --credential file, or http://user:password@host:8123) and for the hosts discovery finds."
                    )?,
                    _ => {}
                }
                // Saying what to run beats saying what is missing: the ways in are the build
                // order's step 1, the local rig, and the fleet — with a login per server from
                // a file, or one login for every server.
                let ways: [(&str, &str); 4] = [
                    ("FAKE=1 cobserve", "generated fleet, no network"),
                    (
                        "eval \"$(./dev/local-rig.sh env)\" && cobserve",
                        "the local CH 24.10 rig",
                    ),
                    (
                        "cobserve --credential credentials.yaml",
                        "the fleet, a login per server (credentials.example.yaml)",
                    ),
                    (
                        "CH_SEED_URLS=http://host:8123 CH_CLUSTER=ch_cluster CH_USER=u CH_PASSWORD=p cobserve",
                        "the fleet, one login for every server",
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
            ConfigError::Usage(message) => write!(f, "{message}\n\n{USAGE}"),
            ConfigError::File(path, why) => write!(f, "--credential {path}: {why}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Where variables come from: the process environment, or a table in a test.
struct Env<'a>(&'a dyn Fn(&str) -> Option<String>);

impl Env<'_> {
    fn get(&self, name: &str) -> Option<String> {
        (self.0)(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    fn required(&self, name: &'static str) -> Result<String, ConfigError> {
        self.get(name).ok_or(ConfigError::Missing(name))
    }

    fn number<T: std::str::FromStr>(&self, name: &'static str, default: T) -> Result<T, ConfigError> {
        match self.get(name) {
            None => Ok(default),
            Some(raw) => raw
                .parse()
                .map_err(|_| ConfigError::Bad(name, format!("{raw:?} is not a number"))),
        }
    }
}

/// Why a seed does not parse. None of them repeats the seed: it may hold a password, or a
/// piece of one cut off at a comma.
const NO_SCHEME: &str = "has no scheme — write it as http://host:8123 or http://user:password@host:8123";
const NO_HOST: &str = "has no host";
const NO_USER: &str = "has a login without a user name";
const BAD_PORT: &str =
    "has a port that is not a number (in a password, / , and % have to be written %2F, %2C and %25)";

/// `http://user:password@host:8123/…` → the URL without its login, the host, the port and the
/// login. A password may hold `:` and `@` as they are; `/`, `,` and `%` have to be
/// percent-encoded (`%2F`, `%2C`, `%25`) — the first ends the host part, the second separates
/// seeds, the third starts an escape. A seed without a port is assumed on `http_port`.
pub fn parse_seed(seed: &str, http_port: u16) -> Result<Seed, &'static str> {
    let (scheme, rest) = seed.trim().split_once("://").ok_or(NO_SCHEME)?;
    if scheme.is_empty() {
        return Err(NO_SCHEME);
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let (login, host_port) = match authority.rsplit_once('@') {
        Some((login, host_port)) => (Some(login), host_port),
        None => (None, authority),
    };
    let (host, port) = if host_port.ends_with(']') {
        (host_port, http_port)
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().map_err(|_| BAD_PORT)?),
            None => (host_port, http_port),
        }
    };
    if host.is_empty() {
        return Err(NO_HOST);
    }
    let credentials = match login {
        None => None,
        Some(login) => {
            let (user, password) = login.split_once(':').unwrap_or((login, ""));
            if user.is_empty() {
                return Err(NO_USER);
            }
            Some(Credentials {
                user: percent_decode(user),
                password: percent_decode(password),
            })
        }
    };
    Ok(Seed {
        url: format!("{scheme}://{host_port}"),
        host: host.to_string(),
        port,
        credentials,
    })
}

/// `%40` → `@`. Anything that is not a whole escape is kept as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

// -- the credential file ------------------------------------------------------------------
//
// The file as written. None of these derive `Debug`: they hold passwords. Every value is read
// as the text it is in the file — `password: 007` is "007", not 7 — so a password only needs
// quotes where YAML itself would read it differently (a leading `"`, `'`, `#`, `{`, `[`, `&`,
// `*`, `!`, `|`, `>`, `%` or `@`, or ` #` inside it).

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "clickhouse: and redash: sections")]
struct FileContents {
    clickhouse: Option<FileClickHouse>,
    redash: Option<FileRedash>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "servers:, and optionally cluster: and default_login:")]
struct FileClickHouse {
    cluster: Option<String>,
    #[serde(default)]
    servers: Vec<FileServer>,
    default_login: Option<FileLogin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "a server: url:, user: and password:")]
struct FileServer {
    url: String,
    user: Option<String>,
    password: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "a login: user: and password:")]
struct FileLogin {
    user: String,
    password: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "url:, api_key:, redis_url: and email_domain:")]
struct FileRedash {
    url: Option<String>,
    api_key: Option<String>,
    redis_url: Option<String>,
    email_domain: Option<String>,
}

/// A credential file that has been read and parsed, and the name it was given by.
struct CredentialFile {
    shown: String,
    contents: FileContents,
}

impl CredentialFile {
    fn error(&self, why: impl Into<String>) -> ConfigError {
        ConfigError::File(self.shown.clone(), why.into())
    }
}

fn parse_credential_file(text: &str) -> Result<FileContents, String> {
    match serde_norway::from_str::<Option<FileContents>>(text) {
        Ok(Some(contents)) => Ok(contents),
        Ok(None) => Err("there is nothing in it — see credentials.example.yaml".to_string()),
        Err(e) => Err(without_values(&e.to_string())),
    }
}

/// A YAML error with every value from the file taken out — the value might be a password in
/// the wrong place. Keys and positions stay: they are what point at the mistake.
fn without_values(message: &str) -> String {
    let quoted = regex::Regex::new(r#""(?:[^"\\]|\\.)*""#).expect("a valid pattern");
    let typed = regex::Regex::new(r"(integer|floating point|boolean|character) `[^`]*`").expect("a valid pattern");
    let message = quoted.replace_all(message, "\"…\"");
    typed.replace_all(&message, "$1 …").into_owned()
}

/// The file's own word for a server or a login that is not complete.
const NO_PASSWORD: &str = "has a user but no password (write password: \"\" for a user without one)";
const NO_FILE_USER: &str = "has a password but no user";
const TWO_LOGINS: &str = "has a login both in its url and in user/password — keep one";

fn file_seed(server: &FileServer, http_port: u16) -> Result<Seed, &'static str> {
    let mut seed = parse_seed(&server.url, http_port)?;
    match (&server.user, &server.password) {
        (None, None) => {}
        (Some(_), _) if seed.credentials.is_some() => return Err(TWO_LOGINS),
        (Some(user), Some(password)) if !user.is_empty() => {
            seed.credentials = Some(Credentials {
                user: user.clone(),
                password: password.clone(),
            });
        }
        (Some(user), Some(_)) if user.is_empty() => return Err(NO_USER),
        (Some(_), _) => return Err(NO_PASSWORD),
        (None, Some(_)) => return Err(NO_FILE_USER),
    }
    Ok(seed)
}

fn file_login(login: &FileLogin) -> Result<Credentials, &'static str> {
    match &login.password {
        _ if login.user.is_empty() => Err(NO_USER),
        Some(password) => Ok(Credentials {
            user: login.user.clone(),
            password: password.clone(),
        }),
        None => Err(NO_PASSWORD),
    }
}

/// Group or others can open the file: ssh refuses a key like that; this only says so.
#[cfg(unix)]
fn readable_by_others(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
fn readable_by_others(_: &Path) -> bool {
    false
}

impl Config {
    /// The configuration for this run: the credential file the command line names, if any,
    /// and the environment.
    pub fn load(args: &Args) -> Result<Self, ConfigError> {
        Self::load_with(args, &|name| std::env::var(name).ok())
    }

    fn load_with(args: &Args, env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut warnings = Vec::new();
        let file = match &args.credential {
            None => None,
            Some(path) => {
                let shown = path.display().to_string();
                let text = std::fs::read_to_string(path)
                    .map_err(|e| ConfigError::File(shown.clone(), format!("cannot read it ({e})")))?;
                if readable_by_others(path) {
                    // The name first: the footer cuts a long path, and the name is what matters.
                    let name = path.file_name().map_or_else(|| shown.clone(), |n| n.to_string_lossy().into_owned());
                    warnings.push(format!("{name} can be read by other users — chmod 600 {shown}"));
                }
                let contents = parse_credential_file(&text).map_err(|why| ConfigError::File(shown.clone(), why))?;
                Some(CredentialFile { shown, contents })
            }
        };
        let mut config = Self::assemble(file.as_ref(), &Env(env))?;
        warnings.append(&mut config.warnings);
        config.warnings = warnings;
        Ok(config)
    }

    /// The file and the environment together. The file wins where both say something;
    /// `CH_SEED_URLS` adds its servers to the file's.
    fn assemble(file: Option<&CredentialFile>, env: &Env<'_>) -> Result<Self, ConfigError> {
        let fake = env.get("FAKE").as_deref() == Some("1");
        let from_file = file.and_then(|f| f.contents.clickhouse.as_ref().map(|section| (f, section)));

        let clickhouse = if fake {
            ClickHouseConfig {
                seeds: Vec::new(),
                cluster: String::new(),
                default_login: None,
                http_port: 8123,
            }
        } else {
            let http_port = env.number("CH_HTTP_PORT", 8123u16)?;
            let mut seeds = Vec::new();
            if let Some((file, section)) = from_file {
                let total = section.servers.len();
                for (i, server) in section.servers.iter().enumerate() {
                    let seed = file_seed(server, http_port)
                        .map_err(|why| file.error(format!("server {} of {total} {why}", i + 1)))?;
                    seeds.push(seed);
                }
            }
            if let Some(raw) = env.get("CH_SEED_URLS") {
                let typed: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                for (i, seed) in typed.iter().enumerate() {
                    let seed = parse_seed(seed, http_port).map_err(|why| {
                        ConfigError::Bad("CH_SEED_URLS", format!("seed {} of {} {why}", i + 1, typed.len()))
                    })?;
                    seeds.push(seed);
                }
            }
            if seeds.is_empty() {
                return Err(ConfigError::Missing("CH_SEED_URLS"));
            }

            let cluster = match from_file.and_then(|(_, s)| s.cluster.as_deref()).map(str::trim) {
                Some(cluster) if !cluster.is_empty() => cluster.to_string(),
                _ => env.required("CH_CLUSTER")?,
            };

            // A server can carry its own login when the servers do not share one. The default
            // login is then for a server without one and for the hosts discovery adds, and is
            // required only when some server has none. Of CH_USER and CH_PASSWORD, one is never
            // completed with a guess for the other.
            let default_login = match from_file.and_then(|(file, s)| s.default_login.as_ref().map(|l| (file, l))) {
                Some((file, login)) => Some(file_login(login).map_err(|why| file.error(format!("default_login {why}")))?),
                None => match (env.get("CH_USER"), env.get("CH_PASSWORD")) {
                    (Some(user), Some(password)) => Some(Credentials { user, password }),
                    (None, None) if seeds.iter().all(|seed| seed.credentials.is_some()) => None,
                    (None, _) => return Err(ConfigError::Missing("CH_USER")),
                    (Some(_), None) => return Err(ConfigError::Missing("CH_PASSWORD")),
                },
            };
            ClickHouseConfig {
                seeds,
                cluster,
                default_login,
                http_port,
            }
        };

        let mut warnings = Vec::new();
        let redash = file.and_then(|f| f.contents.redash.as_ref());
        let pick = |in_file: Option<&String>, name: &str| {
            in_file
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| env.get(name))
        };
        Ok(Config {
            clickhouse,
            redash: RedashConfig {
                url: pick(redash.and_then(|r| r.url.as_ref()), "REDASH_URL"),
                admin_api_key: pick(redash.and_then(|r| r.api_key.as_ref()), "REDASH_ADMIN_API_KEY"),
                redis_url: pick(redash.and_then(|r| r.redis_url.as_ref()), "REDIS_URL"),
            },
            poll: Duration::from_millis(env.number("POLL_MS", 2000u64)?),
            discover_every: Duration::from_secs(60),
            fake,
            // The fake fleet's people are at a domain of its own.
            email_domain: pick(redash.and_then(|r| r.email_domain.as_ref()), "EMAIL_DOMAIN")
                .or_else(|| fake.then(|| crate::fake::DOMAIN.to_string())),
            claude_command: command("CLAUDE_CMD", env.get("CLAUDE_CMD"), "claude")?,
            opencode_command: command("OPENCODE_CMD", env.get("OPENCODE_CMD"), "opencode")?,
            // A terminal runs the shell the user logs in with.
            shell_command: command(
                "SHELL_CMD",
                env.get("SHELL_CMD"),
                env.get("SHELL").filter(|s| !s.trim().is_empty()).as_deref().unwrap_or("/bin/sh"),
            )?,
            mouse: !matches!(
                env.get("MOUSE").map(|m| m.trim().to_ascii_lowercase()).as_deref(),
                Some("0" | "off" | "false" | "no")
            ),
            prayer: prayer_config(env, &mut warnings)?,
            utc: env.get("TIME").is_some_and(|t| t.eq_ignore_ascii_case("utc")),
            notify: match env.get("NOTIFY").map(|n| n.to_ascii_lowercase()).as_deref() {
                None | Some("desktop" | "on" | "1") => Notify::Desktop,
                Some("bell") => Notify::Bell,
                Some("off" | "0" | "none" | "no") => Notify::Off,
                Some(other) => return Err(ConfigError::Bad("NOTIFY", format!("{other:?} is not off, bell or desktop"))),
            },
            warnings,
        })
    }
}

/// The prayer settings. A city nobody here knows is worth a word in the footer, not a refusal to
/// start: the time zone's city stands in.
fn prayer_config(env: &Env<'_>, warnings: &mut Vec<String>) -> Result<PrayerConfig, ConfigError> {
    use crate::prayer::{self, Place, PlaceFrom};
    let off = env.get("PRAYER").is_some_and(|p| matches!(p.to_ascii_lowercase().as_str(), "off" | "0" | "no" | "false"));
    let city = env.get("PRAYER_CITY");
    let at = env.get("PRAYER_AT");
    let place = match (&city, &at) {
        (_, Some(raw)) => match prayer::coordinates(raw) {
            Some((lat, lon)) => Some(Place {
                name: city.clone().unwrap_or_else(|| format!("{lat:.2}, {lon:.2}")),
                lat,
                lon,
                from: PlaceFrom::Chosen,
            }),
            None => {
                warnings.push("PRAYER_AT is not lat,lon (e.g. -6.91,107.61) — the time zone's city stands in".to_string());
                None
            }
        },
        (Some(name), None) => {
            let place = prayer::city(name);
            if place.is_none() {
                warnings.push(format!("PRAYER_CITY {name} is not a city known here — PRAYER_AT=lat,lon gives any place"));
            }
            place
        }
        (None, None) => None,
    };
    Ok(PrayerConfig { off, place, remind_minutes: env.number("PRAYER_REMIND", 10u32)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn table(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| vars.get(name).cloned()
    }

    fn from(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::assemble(None, &Env(&table(pairs)))
    }

    fn with_file(yaml: &str, pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let contents = parse_credential_file(yaml).map_err(|why| ConfigError::File("creds.yaml".into(), why))?;
        let file = CredentialFile {
            shown: "creds.yaml".into(),
            contents,
        };
        Config::assemble(Some(&file), &Env(&table(pairs)))
    }

    fn users(config: &Config) -> Vec<Option<&str>> {
        config
            .clickhouse
            .seeds
            .iter()
            .map(|seed| seed.credentials.as_ref().map(|c| c.user.as_str()))
            .collect()
    }

    fn password_of(config: &Config, seed: usize) -> &str {
        &config.clickhouse.seeds[seed].credentials.as_ref().unwrap().password
    }

    #[test]
    fn the_environment_is_read_and_nothing_is_assumed() {
        let err = from(&[]).unwrap_err().to_string();
        assert!(err.contains("CH_SEED_URLS"), "{err}");
        assert!(err.contains("§9"), "the message points at the spec: {err}");
        // The hint has to name the ways in: fake data, the local rig, the fleet.
        assert!(err.contains("FAKE=1"), "{err}");
        assert!(err.contains("dev/local-rig.sh"), "{err}");
        assert!(err.contains("CH_CLUSTER"), "{err}");
        assert!(err.contains("--credential credentials.yaml"), "and a login per server: {err}");

        let err = from(&[("FAKE", "1"), ("POLL_MS", "soon")]).unwrap_err().to_string();
        assert!(err.contains("POLL_MS"), "a bad number is an error rather than a default: {err}");

        let config = from(&[("FAKE", "1")]).expect("FAKE=1 runs with nothing configured");
        assert!(config.fake);
        assert!(config.clickhouse.seeds.is_empty());
        assert_eq!(config.email_domain.as_deref(), Some(crate::fake::DOMAIN), "the fake fleet's own");
        assert_eq!(config.poll, Duration::from_millis(2000), "the default from §9");
        assert!(config.redash.url.is_none(), "Redash is optional");
    }

    #[test]
    fn a_login_per_server_in_the_seed_urls_needs_no_shared_one() {
        let seeds = "http://mon1:Zq9-one@clickhouse1.example.net:8123, http://mon2:Zq9-two@clickhouse2.example.net:8123";
        let config = from(&[("CH_SEED_URLS", seeds), ("CH_CLUSTER", "ch_cluster")]).unwrap();
        assert_eq!(users(&config), [Some("mon1"), Some("mon2")]);
        assert_eq!(config.clickhouse.seeds[0].url, "http://clickhouse1.example.net:8123");
        assert!(config.clickhouse.default_login.is_none());
        assert!(!format!("{config:?}").contains("Zq9"), "Debug never shows a password");

        // One seed without a login: the shared one is required again.
        let mixed = "http://mon1:Zq9-one@clickhouse1:8123,http://clickhouse2:8123";
        let err = from(&[("CH_SEED_URLS", mixed), ("CH_CLUSTER", "ch_cluster")]).unwrap_err().to_string();
        assert!(err.contains("CH_USER is required"), "{err}");
        assert!(!err.contains("Zq9"), "a password is never echoed: {err}");

        // CH_USER without CH_PASSWORD is not completed with a guess.
        let err = from(&[("CH_SEED_URLS", mixed), ("CH_CLUSTER", "ch_cluster"), ("CH_USER", "monitor")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("CH_PASSWORD is required"), "{err}");

        // With both, the seed without a login uses them and the other keeps its own.
        let config = from(&[
            ("CH_SEED_URLS", mixed),
            ("CH_CLUSTER", "ch_cluster"),
            ("CH_USER", "monitor"),
            ("CH_PASSWORD", "Zq9-shared"),
            ("REDASH_ADMIN_API_KEY", "Zq9-key"),
            ("REDIS_URL", "redis://:Zq9-redis@redis:6379/0"),
        ])
        .unwrap();
        assert_eq!(users(&config), [Some("mon1"), None]);
        let shown = format!("{config:?}");
        assert!(!shown.contains("Zq9"), "nothing secret survives Debug, Redash's included: {shown}");
        assert!(shown.contains("monitor") && shown.contains("mon1"), "the users are fine to show: {shown}");
    }

    #[test]
    fn a_seed_that_does_not_parse_is_refused_without_repeating_it() {
        // A comma in a password has cut the seed in two.
        let err = from(&[("CH_SEED_URLS", "http://mon1:Zq9,x@clickhouse1:8123"), ("CH_CLUSTER", "c")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("CH_SEED_URLS is not usable: seed 1 of 2"), "{err}");
        assert!(err.contains("%2C"), "it says how to write a comma in a password: {err}");
        assert!(!err.contains("Zq9") && !err.contains("mon1"), "{err}");

        assert_eq!(parse_seed("ch1:8123", 8123), Err(NO_SCHEME));
        assert_eq!(parse_seed("monitor:pw@ch1:8123", 8123), Err(NO_SCHEME));
        assert_eq!(parse_seed("http://mon1:pa", 8123), Err(BAD_PORT), "a password cut at a comma");
        assert_eq!(parse_seed("http://mon1:pa/ss@ch1:8123", 8123), Err(BAD_PORT), "or at a slash");
        assert_eq!(parse_seed("http://:pw@ch1:8123", 8123), Err(NO_USER));
        assert_eq!(parse_seed("http://u:pw@:8123", 8123), Err(NO_HOST));
    }

    #[test]
    fn a_seed_url_says_where_the_server_is_and_how_to_log_in() {
        let seed = parse_seed("http://monitor:p@ss:w%2Fx@clickhouse1.example.net:8123/", 8123).unwrap();
        assert_eq!(seed.url, "http://clickhouse1.example.net:8123", "the login is never part of the URL");
        assert_eq!((seed.host.as_str(), seed.port), ("clickhouse1.example.net", 8123));
        let login = seed.credentials.unwrap();
        assert_eq!((login.user.as_str(), login.password.as_str()), ("monitor", "p@ss:w/x"));

        let plain = parse_seed(" http://ch-b:8124 ", 8123).unwrap();
        assert_eq!((plain.url.as_str(), plain.port), ("http://ch-b:8124", 8124));
        assert_eq!(plain.credentials, None);
        assert_eq!(parse_seed("https://ch1", 8125).unwrap().port, 8125, "no port: CH_HTTP_PORT");

        let v6 = parse_seed("http://u:p@[::1]:8126", 8123).unwrap();
        assert_eq!((v6.url.as_str(), v6.host.as_str(), v6.port), ("http://[::1]:8126", "[::1]", 8126));
        let passwordless = parse_seed("http://monitor@ch1:8123", 8123).unwrap();
        assert_eq!(passwordless.credentials.unwrap().password, "", "a user with no password");
    }

    #[test]
    fn percent_escapes_are_decoded_and_anything_else_is_kept() {
        assert_eq!(percent_decode("p%40ss%2C%2F%25"), "p@ss,/%");
        assert_eq!(percent_decode("50%off"), "50%off");
        assert_eq!(percent_decode("%C3%A9t%C3%A9"), "été");
        assert_eq!(percent_decode("%€"), "%€", "no panic in the middle of a character");
        assert_eq!(percent_decode("cut%4"), "cut%4");
        assert_eq!(percent_decode("%FF"), "%FF", "not UTF-8 once decoded: kept as typed");
    }

    const FILE: &str = r#"
# a comment
clickhouse:
  cluster: ch_cluster
  servers:
    - url: http://clickhouse1.example.net:8123
      user: mon1
      password: "Zq9-one"
    - url: http://clickhouse2.example.net:8123
      user: mon2
      password: Zq9-two
redash:
  url: https://redash.example
  api_key: Zq9-key
  email_domain: example.org
"#;

    #[test]
    fn a_credential_file_gives_each_server_its_own_login() {
        let config = with_file(FILE, &[]).unwrap();
        assert_eq!(users(&config), [Some("mon1"), Some("mon2")]);
        assert_eq!(password_of(&config, 0), "Zq9-one");
        assert_eq!(password_of(&config, 1), "Zq9-two");
        assert_eq!(config.clickhouse.seeds[1].url, "http://clickhouse2.example.net:8123");
        assert_eq!(config.clickhouse.cluster, "ch_cluster", "the file can name the cluster");
        assert!(config.clickhouse.default_login.is_none(), "nobody needs one");
        assert_eq!(config.redash.url.as_deref(), Some("https://redash.example"));
        assert_eq!(config.redash.admin_api_key.as_deref(), Some("Zq9-key"));
        assert_eq!(config.email_domain.as_deref(), Some("example.org"));
        assert!(!format!("{config:?}").contains("Zq9"));
    }

    #[test]
    fn the_example_file_is_a_working_file() {
        let config = with_file(include_str!("../credentials.example.yaml"), &[]).unwrap();
        assert_eq!(users(&config), [Some("monitor_ch1"), Some("monitor_ch2")]);
        assert_eq!(config.clickhouse.cluster, "ch_cluster");
        assert!(config.clickhouse.default_login.is_none() && config.redash.url.is_none(), "the rest is commented out");
    }

    #[test]
    fn the_file_wins_and_the_environment_fills_in() {
        let yaml = "clickhouse:\n  servers:\n    - url: http://ch1:8123\n      user: mon1\n      password: p1\n    - url: http://ch2:8123\n";
        let env = [
            ("CH_SEED_URLS", "http://ch3:8123"),
            ("CH_CLUSTER", "from-env"),
            ("CH_USER", "shared"),
            ("CH_PASSWORD", "p"),
            ("REDASH_URL", "https://redash.env"),
            ("EMAIL_DOMAIN", "example.com"),
        ];
        let config = with_file(yaml, &env).unwrap();
        let hosts: Vec<&str> = config.clickhouse.seeds.iter().map(|s| s.host.as_str()).collect();
        assert_eq!(hosts, ["ch1", "ch2", "ch3"], "CH_SEED_URLS adds to the file's servers");
        assert_eq!(users(&config), [Some("mon1"), None, None]);
        assert_eq!(config.clickhouse.default_login.as_ref().unwrap().user, "shared");
        assert_eq!(config.clickhouse.cluster, "from-env");
        assert_eq!(config.redash.url.as_deref(), Some("https://redash.env"));
        assert_eq!(config.email_domain.as_deref(), Some("example.com"));

        let yaml = format!("{yaml}  cluster: from-file\n  default_login:\n    user: file-default\n    password: p\n");
        let config = with_file(&yaml, &env).unwrap();
        assert_eq!(config.clickhouse.cluster, "from-file");
        assert_eq!(config.clickhouse.default_login.as_ref().unwrap().user, "file-default");

        // A server without a login and no default anywhere is still an error.
        let err = with_file("clickhouse:\n  cluster: c\n  servers:\n    - url: http://ch2:8123\n", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("CH_USER is required"), "{err}");
    }

    #[test]
    fn passwords_in_the_file_are_read_exactly_as_written() {
        let yaml = r#"clickhouse:
  cluster: c
  servers:
    - { url: "http://a:8123", user: u, password: 007 }
    - { url: "http://b:8123", user: u, password: 0x1F }
    - url: http://c:8123
      user: u
      password: p@ss:w/rd,%
    - url: http://d:8123
      user: u
      password: a#b
    - url: http://e:8123
      user: u
      password: 'it''s # not a comment'
    - url: http://f:8123
      user: u
      password: ""
"#;
        let config = with_file(yaml, &[]).unwrap();
        let passwords: Vec<&str> = (0..6).map(|i| password_of(&config, i)).collect();
        assert_eq!(passwords, ["007", "0x1F", "p@ss:w/rd,%", "a#b", "it's # not a comment", ""]);
    }

    #[test]
    fn mistakes_in_the_file_are_named_without_their_values() {
        let cases: [(&str, &str); 8] = [
            ("clickhouse:\n  default_login: Zq9-secret\n", "default_login"),
            ("clickhouse:\n  default_login: 12345\n", "default_login"),
            ("clickhouse:\n  servers:\n    - url: http://a:8123\n      user: u\n      pasword: Zq9\n", "unknown field `pasword`"),
            ("clickhouse:\n  servers:\n    - url: http://a:8123\n      user: u\n       password: Zq9 x\n", "line 5"),
            ("clickhouse:\n  servers:\n    - url: http://a:8123\n      user: u\n", "server 1 of 1 has a user but no password"),
            ("clickhouse:\n  servers:\n    - url: http://a:8123\n      password: Zq9\n", "server 1 of 1 has a password but no user"),
            ("clickhouse:\n  servers:\n    - url: http://u:Zq9@a:8123\n      user: u\n      password: Zq9\n", "both in its url and in user/password"),
            ("# nothing but a comment\n", "there is nothing in it"),
        ];
        for (yaml, says) in cases {
            let err = with_file(yaml, &[("CH_CLUSTER", "c")]).unwrap_err().to_string();
            assert!(err.starts_with("--credential creds.yaml: "), "{err}");
            assert!(err.contains(says), "{yaml:?} → {err}");
            assert!(!err.contains("Zq9") && !err.contains("12345"), "{yaml:?} → {err}");
        }
    }

    #[test]
    fn values_are_taken_out_of_yaml_errors() {
        assert_eq!(
            without_values(r#"c: invalid type: string "Zq9 \"x\"", expected a login at line 1 column 4"#),
            r#"c: invalid type: string "…", expected a login at line 1 column 4"#
        );
        assert_eq!(
            without_values("c: invalid type: integer `12345`, expected a login"),
            "c: invalid type: integer …, expected a login"
        );
        assert_eq!(without_values("c: unknown field `pasword`"), "c: unknown field `pasword`", "keys stay");
    }

    #[test]
    fn the_command_line_names_the_credential_file() {
        let parse = |args: &[&str]| Args::parse(args.iter().map(|a| a.to_string()));
        assert_eq!(parse(&[]).unwrap(), Args::default());
        for args in [&["--credential", "c.yaml"][..], &["--credential=c.yaml"], &["-c", "c.yaml"], &["--credentials", "c.yaml"]] {
            assert_eq!(parse(args).unwrap().credential, Some(PathBuf::from("c.yaml")), "{args:?}");
        }
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-V"]).unwrap().version);
        let both = parse(&["--claude", "--credential", "c.yaml"]).unwrap();
        assert!(both.claude && both.credential.is_some(), "--claude opens on view 5");

        let err = parse(&["--credential"]).unwrap_err().to_string();
        assert!(err.contains("needs a file") && err.contains("Usage:"), "{err}");
        let err = parse(&["--credentail", "c.yaml"]).unwrap_err().to_string();
        assert!(err.contains("unknown option --credentail"), "a flag is repeated back: {err}");
        let err = parse(&["Zq9-secret"]).unwrap_err().to_string();
        assert!(!err.contains("Zq9"), "anything else might be a password: {err}");
    }

    #[test]
    fn a_session_s_command_is_split_like_a_shell_would() {
        let claude = |raw: Option<&str>| command("CLAUDE_CMD", raw.map(String::from), "claude");
        assert_eq!(claude(None).unwrap(), ["claude"]);
        assert_eq!(claude(Some("  ")).unwrap(), ["claude"]);
        assert_eq!(
            claude(Some("claude --model opus --add-dir '/srv/my repo'")).unwrap(),
            ["claude", "--model", "opus", "--add-dir", "/srv/my repo"]
        );
        let err = claude(Some("claude 'Zq9")).unwrap_err().to_string();
        assert!(err.contains("CLAUDE_CMD") && !err.contains("Zq9"), "the value is not repeated: {err}");
    }

    #[test]
    fn opencode_and_the_terminal_have_commands_of_their_own() {
        let env = [("FAKE", "1"), ("SHELL", "/bin/zsh")];
        let config = from(&env).unwrap();
        assert_eq!((config.opencode_command, config.shell_command), (vec!["opencode".to_string()], vec!["/bin/zsh".to_string()]));
        let config = from(&[env[0], ("OPENCODE_CMD", "opencode --model x/y"), ("SHELL_CMD", "bash -l")]).unwrap();
        assert_eq!(config.opencode_command, ["opencode", "--model", "x/y"]);
        assert_eq!(config.shell_command, ["bash", "-l"], "SHELL_CMD wins, and without SHELL…");
        assert_eq!(from(&env[..1]).unwrap().shell_command, ["/bin/sh"], "…/bin/sh is there");
    }

    #[test]
    fn the_named_file_is_read_and_a_readable_one_is_flagged() {
        let dir = std::env::temp_dir().join(format!("cobserve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("creds.yaml");
        std::fs::write(&path, FILE).unwrap();
        let args = Args {
            credential: Some(path.clone()),
            ..Args::default()
        };
        let none = |_: &str| None;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let config = Config::load_with(&args, &none).unwrap();
            assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
            assert!(config.warnings[0].contains("chmod 600"), "{:?}", config.warnings);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let config = Config::load_with(&args, &none).unwrap();
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(users(&config), [Some("mon1"), Some("mon2")]);

        let missing = Args {
            credential: Some(dir.join("nope.yaml")),
            ..Args::default()
        };
        let err = Config::load_with(&missing, &none).unwrap_err().to_string();
        assert!(err.contains("nope.yaml: cannot read it"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
