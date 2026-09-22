//! Claude subscription usage — the rolling 5-hour window and the weekly one —
//! fetched by amux on a timer and shared with every Claude session.
//!
//! Each Claude Code session only learns its limits from its own last API
//! response, so a status line built from that shows whatever the session saw
//! last: a pane idle for an hour shows an hour-old number, and two panes
//! disagree. amux instead asks the endpoint Claude Code's `/usage` reads, every
//! five minutes, and writes the answer to one small file (`claude-usage.json`
//! in amux's cache directory) that every status line reads instead.
//!
//! The access token is read from Claude Code's own credentials file and only
//! ever sent to `api.anthropic.com`. amux never refreshes it: a refresh rotates
//! the token Claude Code itself holds. When it has expired the poll just fails,
//! and a later one picks up whatever a live Claude session refreshed it to.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// How often the usage is asked for.
pub const POLL_EVERY: Duration = Duration::from_secs(5 * 60);
/// A window that resets before the next regular poll gets one this long after
/// its reset, so the stale pre-reset number does not linger for minutes.
const AFTER_RESET: Duration = Duration::from_secs(20);
/// Never poll more often than this, whatever the reset times say.
const MIN_DELAY: Duration = Duration::from_secs(30);
/// Upper bound on a 429's `Retry-After`, so a hostile or broken header cannot
/// switch the meter off for days.
const MAX_BACKOFF: Duration = Duration::from_secs(60 * 60);

/// One limit window, in the same shape Claude Code hands a status line
/// (`rate_limits.five_hour` and friends), so a status line script reads either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// 0 to 100.
    pub used_percentage: f64,
    /// Unix epoch seconds when the window resets.
    pub resets_at: Option<i64>,
}

/// What `claude-usage.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Unix epoch seconds of the successful fetch, so a reader can tell a
    /// fresh file from one left behind by an amux that has since exited.
    pub fetched_at: i64,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

#[derive(Debug)]
pub enum PollError {
    /// No subscription token in Claude Code's credentials file.
    NoToken,
    /// The endpoint answered with a non-2xx status.
    Status { code: u16, retry_after: Option<Duration> },
    /// Network or TLS failure, or an unreadable body.
    Transport(String),
    /// A 2xx body that is not the usage JSON.
    Parse(String),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::NoToken => write!(f, "no Claude subscription token"),
            PollError::Status { code, .. } => write!(f, "usage endpoint returned {code}"),
            PollError::Transport(e) => write!(f, "usage request failed: {e}"),
            PollError::Parse(e) => write!(f, "usage response not understood: {e}"),
        }
    }
}

/// Where the shared usage file lives: `$AMUX_USAGE_FILE` if set (a development
/// instance or a test), otherwise `claude-usage.json` in amux's cache directory
/// (`$XDG_CACHE_HOME/amux`, falling back to `~/.cache/amux`).
pub fn usage_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AMUX_USAGE_FILE") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    Some(base.join("amux").join("claude-usage.json"))
}

/// Claude Code's credentials file (`$CLAUDE_CONFIG_DIR` or `~/.claude`).
fn credentials_path() -> Option<PathBuf> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|h| PathBuf::from(h).join(".claude"))
    })?;
    Some(dir.join(".credentials.json"))
}

/// The subscription access token (`claudeAiOauth.accessToken`), if any. Its
/// `expiresAt` is not consulted: the server is the authority on expiry.
pub fn read_token(credentials: &Path) -> Option<String> {
    let raw = fs::read_to_string(credentials).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let token = value.get("claudeAiOauth")?.get("accessToken")?.as_str()?.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// The endpoint's JSON as a [`Usage`]. A window with no usage number is left
/// out rather than guessed.
pub fn parse(body: &str, fetched_at: i64) -> Result<Usage, PollError> {
    let value: Value = serde_json::from_str(body).map_err(|e| PollError::Parse(e.to_string()))?;
    if !value.is_object() {
        return Err(PollError::Parse("not a JSON object".into()));
    }
    Ok(Usage {
        fetched_at,
        five_hour: window(value.get("five_hour")),
        seven_day: window(value.get("seven_day")),
    })
}

fn window(value: Option<&Value>) -> Option<Window> {
    let value = value?;
    let used = value
        .get("utilization")
        .and_then(Value::as_f64)
        .or_else(|| value.get("used_percentage").and_then(Value::as_f64))?;
    Some(Window { used_percentage: used.clamp(0.0, 100.0), resets_at: reset_epoch(value.get("resets_at")) })
}

/// `resets_at` arrives as an RFC 3339 string (`2026-09-22T03:50:00.910052+00:00`)
/// from this endpoint and as epoch seconds from Claude Code; accept both, and
/// epoch milliseconds too.
fn reset_epoch(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => {
            let n = n.as_f64()?;
            Some(if n > 1e10 { (n / 1000.0) as i64 } else { n as i64 })
        }
        Value::String(s) => time::OffsetDateTime::parse(s.trim(), &time::format_description::well_known::Rfc3339)
            .ok()
            .map(|t| t.unix_timestamp()),
        _ => None,
    }
}

/// Ask the endpoint once. Returns the raw body on a 2xx.
pub fn fetch(token: &str) -> Result<String, PollError> {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(tls)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", concat!("amux/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| PollError::Transport(e.to_string()))?;
    let code = response.status().as_u16();
    if !(200..300).contains(&code) {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(PollError::Status { code, retry_after });
    }
    response.body_mut().read_to_string().map_err(|e| PollError::Transport(e.to_string()))
}

/// Replace the usage file in one step (write aside, then rename), so a status
/// line never reads half a file. The side file is per process: two amux
/// instances sharing the file must not write into each other's.
pub fn write(path: &Path, usage: &Usage) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(usage)?)?;
    fs::rename(&tmp, path)
}

/// When to ask next: the regular interval, or shortly after the earliest
/// window reset if that comes sooner — right after a reset the old number is
/// simply wrong.
pub fn next_delay(usage: &Usage, now: i64) -> Duration {
    let earliest_reset = [&usage.five_hour, &usage.seven_day]
        .into_iter()
        .flatten()
        .filter_map(|w| w.resets_at)
        .filter(|&at| at > now)
        .min();
    let delay = match earliest_reset {
        Some(at) => POLL_EVERY.min(Duration::from_secs((at - now) as u64) + AFTER_RESET),
        None => POLL_EVERY,
    };
    delay.max(MIN_DELAY)
}

fn now_epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// One round: read the token, ask, write the file. Returns how long to wait
/// before the next round.
fn poll_once(credentials: &Path, target: &Path) -> Duration {
    let result = read_token(credentials)
        .ok_or(PollError::NoToken)
        .and_then(|token| fetch(&token))
        .and_then(|body| parse(&body, now_epoch()));
    match result {
        Ok(usage) => {
            if let Err(e) = write(target, &usage) {
                tracing::warn!("claude usage: could not write {}: {e}", target.display());
            }
            next_delay(&usage, now_epoch())
        }
        // Asked too often: wait as long as the server says, within reason.
        Err(PollError::Status { code: 429, retry_after }) => {
            retry_after.unwrap_or(POLL_EVERY).clamp(POLL_EVERY, MAX_BACKOFF)
        }
        // No subscription on this machine is not worth a log line every five
        // minutes; everything else is.
        Err(PollError::NoToken) => POLL_EVERY,
        Err(e) => {
            tracing::warn!("claude usage: {e}");
            POLL_EVERY
        }
    }
}

/// Start the background poller: once now, then on [`next_delay`]'s schedule,
/// for the life of the process. A plain thread, like the other sweepers here —
/// the work is one blocking HTTP request every few minutes.
pub fn start_poller() {
    let (Some(credentials), Some(target)) = (credentials_path(), usage_path()) else {
        tracing::warn!("claude usage: no home directory; the usage meter stays off");
        return;
    };
    std::thread::Builder::new()
        .name("claude-usage".into())
        .spawn(move || loop {
            let delay = poll_once(&credentials, &target);
            std::thread::sleep(delay);
        })
        .expect("spawn claude usage poller");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real answer of the endpoint (2026-09-22).
    const BODY: &str = r#"{
        "five_hour": {"utilization": 22.0, "resets_at": "2026-09-22T03:50:00.910052+00:00", "limit_dollars": null},
        "seven_day": {"utilization": 69.0, "resets_at": "2026-09-24T00:00:00.910073+00:00"},
        "seven_day_opus": null,
        "extra_usage": {"is_enabled": false}
    }"#;

    #[test]
    fn parses_the_endpoint_answer_into_status_line_shape() {
        let usage = parse(BODY, 1_790_050_000).unwrap();
        assert_eq!(usage.fetched_at, 1_790_050_000);
        // Same instants Claude Code itself reported to a status line that day.
        assert_eq!(usage.five_hour, Some(Window { used_percentage: 22.0, resets_at: Some(1_790_049_000) }));
        assert_eq!(usage.seven_day, Some(Window { used_percentage: 69.0, resets_at: Some(1_790_208_000) }));
    }

    #[test]
    fn missing_or_odd_windows_are_left_out_not_guessed() {
        let usage = parse(r#"{"five_hour": null, "seven_day": {"resets_at": "2026-09-24T00:00:00Z"}}"#, 0).unwrap();
        assert_eq!(usage.five_hour, None);
        assert_eq!(usage.seven_day, None, "no usage number, no window");
        let clamped = parse(r#"{"five_hour": {"utilization": 130, "resets_at": 1790049000000}}"#, 0).unwrap();
        assert_eq!(clamped.five_hour, Some(Window { used_percentage: 100.0, resets_at: Some(1_790_049_000) }));
        assert!(parse("[]", 0).is_err());
        assert!(parse("<html>", 0).is_err());
    }

    #[test]
    fn polls_soon_after_a_reset_but_never_too_often() {
        let usage = |five: i64, week: i64| Usage {
            fetched_at: 0,
            five_hour: Some(Window { used_percentage: 1.0, resets_at: Some(five) }),
            seven_day: Some(Window { used_percentage: 1.0, resets_at: Some(week) }),
        };
        let now = 1_000_000;
        assert_eq!(next_delay(&usage(now + 3600, now + 86400), now), POLL_EVERY);
        assert_eq!(next_delay(&usage(now + 100, now + 86400), now), Duration::from_secs(120));
        assert_eq!(next_delay(&usage(now + 1, now + 86400), now), MIN_DELAY);
        // A reset already in the past says nothing about the next one.
        assert_eq!(next_delay(&usage(now - 5, now + 86400), now), POLL_EVERY);
    }

    #[test]
    fn token_comes_from_the_claude_oauth_entry_only() {
        let dir = std::env::temp_dir().join(format!("amux-usage-token-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".credentials.json");
        fs::write(&file, r#"{"claudeAiOauth": {"accessToken": "tok-123", "expiresAt": 1}}"#).unwrap();
        assert_eq!(read_token(&file).as_deref(), Some("tok-123"));
        fs::write(&file, r#"{"claudeAiOauth": {"accessToken": "  "}}"#).unwrap();
        assert_eq!(read_token(&file), None);
        fs::write(&file, r#"{"somethingElse": {}}"#).unwrap();
        assert_eq!(read_token(&file), None);
        assert_eq!(read_token(&dir.join("missing.json")), None);
    }

    #[test]
    fn written_file_reads_back_whole() {
        let dir = std::env::temp_dir().join(format!("amux-usage-write-{}", std::process::id()));
        let file = dir.join("nested").join("claude-usage.json");
        let usage = parse(BODY, 42).unwrap();
        write(&file, &usage).unwrap();
        let back: Usage = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, usage);
        let leftovers: Vec<_> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "side file left behind: {leftovers:?}");
    }

    /// Not a correctness test: one real request with this machine's Claude
    /// token, to check the TLS and header path end to end. Prints the parsed
    /// windows, never the token. `cargo test -p amux-core --features usage
    /// usage::tests::real_endpoint -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_endpoint() {
        let token = read_token(&credentials_path().unwrap()).expect("a Claude subscription token");
        let body = fetch(&token).unwrap();
        let usage = parse(&body, now_epoch()).unwrap();
        println!("{usage:?}");
        assert!(usage.five_hour.is_some() || usage.seven_day.is_some());
    }
}
