//! Google Chrome, asked about itself: its open tabs (titles and addresses) through AppleScript,
//! and a tab's process stopped when the owner says so. Memory per tab is Chrome's alone (its Task
//! Manager); what cobserve knows of the processes comes from `ps` (`sources/local.rs`).
//!
//! AppleScript is only sent to a Chrome that is running — `tell application` would start one —
//! and macOS asks the owner once whether cobserve may ask Chrome. A refusal is said, with where
//! to allow it.

use crate::local::{self, Part};
use std::time::Duration;
use tokio::process::Command;

/// One open tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// Its window, counted from 1 as Chrome counts them.
    pub window: u32,
    pub title: String,
    pub url: String,
}

/// The tabs of every window, `window ⇥ url ⇥ title` a line.
const SCRIPT: &str = r#"tell application "Google Chrome"
  set out to ""
  repeat with w from 1 to count windows
    repeat with t in tabs of window w
      set out to out & w & tab & (URL of t) & tab & (title of t) & linefeed
    end repeat
  end repeat
  return out
end tell"#;

/// How long the owner has to answer macOS's question the first time; later reads take a moment.
const ASK_S: u64 = 120;

/// Chrome's open tabs, or why they could not be read.
pub async fn tabs(running: bool) -> Result<Vec<Tab>, String> {
    if !running {
        return Err("Chrome is not running".into());
    }
    let run = Command::new("osascript").args(["-e", SCRIPT]).env("LC_ALL", "C").kill_on_drop(true).output();
    let out = match tokio::time::timeout(Duration::from_secs(ASK_S), run).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("osascript: {e}")),
        Err(_) => return Err("Chrome did not answer within 2 minutes — is macOS asking whether cobserve may control it?".into()),
    };
    if !out.status.success() {
        return Err(refusal(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(parse_tabs(&String::from_utf8_lossy(&out.stdout)))
}

/// What osascript's complaint means for the owner.
pub fn refusal(stderr: &str) -> String {
    if stderr.contains("-1743") || stderr.to_ascii_lowercase().contains("not authorized") {
        "macOS did not let cobserve ask Chrome — allow it in System Settings › Privacy & Security › Automation".into()
    } else if stderr.contains("-600") || stderr.contains("isn’t running") || stderr.contains("isn't running") {
        "Chrome is not running".into()
    } else {
        let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no answer").trim();
        format!("Chrome did not say: {}", line.chars().take(160).collect::<String>())
    }
}

pub fn parse_tabs(text: &str) -> Vec<Tab> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let window = fields.next()?.trim().parse().ok()?;
            let url = fields.next()?.trim().to_string();
            let title = fields.next().unwrap_or_default().trim().to_string();
            Some(Tab { window, title, url })
        })
        .collect()
}

/// The site of an address: `mail.google.com` of `https://mail.google.com/mail/u/0/#inbox`,
/// `chrome://newtab` kept whole.
pub fn site(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) if scheme == "http" || scheme == "https" => {
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            host.split(':').next().unwrap_or(host).trim_start_matches("www.").to_string()
        }
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or(rest)),
        None => url.to_string(),
    }
}

/// Stop one of Chrome's tab processes: the tabs in it show "Aw, Snap!" and give back what they
/// held; a reload brings each back. Only a tab's or an extension's process, and only the very
/// process asked for — its command line is read again first, so a pid reused since the card was
/// shown is left alone.
pub async fn stop(pid: u32, command: &str) -> Result<String, String> {
    let out = Command::new("ps")
        .args(["-o", "pid=,etime=,args=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("ps: {e}"))?;
    let found = local::parse_args(&String::from_utf8_lossy(&out.stdout)).into_iter().find(|(p, _, _)| *p == pid);
    let Some((_, _, args)) = found else { return Err("already gone".into()) };
    if !args.starts_with(command) {
        return Err("that process is not the one shown any more — nothing was stopped".into());
    }
    let part = local::part_of(&args);
    if !part.killable() {
        return Err(match part {
            Part::Browser => "that is Chrome itself — stopping it closes every window".into(),
            Part::Gpu => "that is Chrome's GPU process — every tab draws through it".into(),
            _ => "only a tab's or an extension's process is stopped from here".into(),
        });
    }
    let status = Command::new("kill").args(["-TERM", &pid.to_string()]).status().await.map_err(|e| format!("kill: {e}"))?;
    if status.success() { Ok("stopped — its tabs show Aw, Snap!; reload one to get it back".into()) } else { Err(format!("kill exited with {status}")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_come_out_of_the_script_and_sites_out_of_their_addresses() {
        let tabs = parse_tabs("1\thttps://mail.google.com/mail/u/0/#inbox\tInbox (3) - me@x\n1\tchrome://newtab/\tNew Tab\n2\thttps://www.youtube.com/watch?v=1\tA video\twith a tab\n\nnot a line\n");
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[2], Tab { window: 2, title: "A video\twith a tab".into(), url: "https://www.youtube.com/watch?v=1".into() });
        assert_eq!(site(&tabs[0].url), "mail.google.com");
        assert_eq!(site(&tabs[1].url), "chrome://newtab");
        assert_eq!(site(&tabs[2].url), "youtube.com");
        assert_eq!(site("http://user:pw@localhost:5001/queries/1"), "localhost");
    }

    #[test]
    fn a_refusal_says_where_to_allow_it() {
        assert!(refusal("execution error: Not authorized to send Apple events to Google Chrome. (-1743)").contains("Automation"));
        assert_eq!(refusal("execution error: Google Chrome got an error: Application isn’t running. (-600)"), "Chrome is not running");
    }

    #[tokio::test]
    async fn a_stopped_chrome_is_not_asked() {
        assert_eq!(tabs(false).await, Err("Chrome is not running".into()));
    }
}
