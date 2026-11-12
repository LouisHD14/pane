//! Claude plan usage limits, read with Claude Code's local login (user approved 2026-11-11).
//! Falls back to summing local Claude Code transcripts when the limits can't be fetched.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Limit {
    pub used_pct: f32,
    /// ISO 8601.
    pub resets_at: Option<String>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum ClaudeUsage {
    Limits { five_hour: Limit, seven_day: Limit, plan: String },
    /// Why limits aren't available is in `note`.
    Local { tokens_5h: u64, note: String },
}

/// The endpoint Claude Code's `/usage` uses. The token is sent only here.
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const WINDOW: Duration = Duration::from_secs(5 * 3600);

/// Blocking. Call every ~5 min from a background thread (and on demand).
pub fn fetch() -> ClaudeUsage {
    match limits() {
        Ok(u) => u,
        Err(note) => ClaudeUsage::Local { tokens_5h: local_tokens(), note },
    }
}

fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|h| Path::new(&h).join(".claude"))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

// No Debug derive: holds the token.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    claude_ai_oauth: OAuth,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuth {
    access_token: String,
    refresh_token: Option<String>,
    /// Unix ms.
    expires_at: i64,
    subscription_type: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

/// Claude Code's own OAuth app and token endpoint (from its CLI, 2026-11). Pane refreshes the way Claude Code
/// does and writes the new tokens back, so Claude Code stays logged in with them (approved 2026-11-12).
/// Before, Pane only read the file: when Claude Code wasn't run for a while (e.g. only the desktop app was
/// used), the token ran out and the widget fell back to the local token count for good.
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const RELOGIN: &str = "login expired — run `claude /login`";

fn credentials_path() -> Result<PathBuf, String> {
    Ok(claude_dir().ok_or("no user profile")?.join(".credentials.json"))
}

fn read_oauth() -> Result<OAuth, String> {
    let text = fs::read_to_string(credentials_path()?).map_err(|_| "not logged in to Claude Code")?;
    Ok(serde_json::from_str::<Credentials>(&text).map_err(|_| "not logged in to Claude Code")?.claude_ai_oauth)
}

/// Reads Claude Code's login, refreshes it when it ran out, and asks for the plan limits.
fn limits() -> Result<ClaudeUsage, String> {
    let mut oauth = read_oauth()?;
    // A minute of slack so the token doesn't run out mid-request.
    if expired(oauth.expires_at, now_secs() * 1000 + 60_000) {
        oauth = refresh(&oauth)?;
    }
    let plan = oauth.subscription_type.clone().unwrap_or_default();
    let body = match ask(&oauth.access_token) {
        // Revoked early, or Claude Code refreshed in the meantime: read the file again, refresh once, retry.
        Err(e) if e == "HTTP 401" => ask(&refresh(&read_oauth()?)?.access_token),
        // Seen 2026-11-12: "OAuth authentication is currently not allowed for this organization." A setting of the
        // Claude account, nothing Pane can change; the widget shows the local token count with this note.
        Err(e) if e == "HTTP 403" => Err("your Claude organisation doesn't allow reading the limits (HTTP 403)".to_string()),
        Err(e) if e == "HTTP 429" => Err("Claude is rate-limiting the limits request (HTTP 429); trying again in 5 minutes".to_string()),
        other => other,
    }?;
    parse(&body, plan)
}

/// http::get errors are "HTTP <code>" / "network error ...", never header values.
fn ask(token: &str) -> Result<String, String> {
    let auth = format!("Bearer {token}");
    crate::http::get(USAGE_URL, &[("Authorization", &auth), ("anthropic-beta", "oauth-2025-04-20")])
}

#[derive(Deserialize)]
struct Refreshed {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

/// Swaps the refresh token for a new access token and writes it back into Claude Code's credentials file.
/// Only `accessToken`, `refreshToken` and `expiresAt` change; the rest of the file is kept as it is.
fn refresh(oauth: &OAuth) -> Result<OAuth, String> {
    let token = oauth.refresh_token.as_deref().filter(|t| !t.is_empty()).ok_or(RELOGIN)?;
    let scopes = if oauth.scopes.is_empty() { "user:profile user:inference".to_string() } else { oauth.scopes.join(" ") };
    let body = serde_json::json!({ "grant_type": "refresh_token", "refresh_token": token, "client_id": CLIENT_ID, "scope": scopes });
    let reply = crate::http::post_json(TOKEN_URL, &body).map_err(|e| match e.as_str() {
        "HTTP 400" | "HTTP 401" => RELOGIN.to_string(),
        _ => e,
    })?;
    let new: Refreshed = serde_json::from_str(&reply).map_err(|_| "bad token response")?;
    let expires_at = now_secs() * 1000 + new.expires_in.unwrap_or(3600) * 1000;
    let refresh_token = new.refresh_token.or_else(|| oauth.refresh_token.clone());
    crate::win::update_json(&credentials_path()?, |root| {
        let o = root.get_mut("claudeAiOauth").and_then(|o| o.as_object_mut()).ok_or("no claudeAiOauth")?;
        o.insert("accessToken".into(), new.access_token.clone().into());
        if let Some(r) = &refresh_token {
            o.insert("refreshToken".into(), r.clone().into());
        }
        o.insert("expiresAt".into(), expires_at.into());
        Ok(())
    })?;
    Ok(OAuth {
        access_token: new.access_token,
        refresh_token,
        expires_at,
        subscription_type: oauth.subscription_type.clone(),
        scopes: oauth.scopes.clone(),
    })
}

fn expired(expires_at_ms: i64, now_ms: i64) -> bool {
    expires_at_ms <= now_ms
}

#[derive(Deserialize)]
struct Response {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    /// Percent, 0-100.
    utilization: Option<f32>,
    resets_at: Option<String>,
}

fn limit(w: Window) -> Limit {
    Limit { used_pct: w.utilization.unwrap_or(0.0), resets_at: w.resets_at }
}

fn parse(json: &str, plan: String) -> Result<ClaudeUsage, String> {
    let r = serde_json::from_str::<Response>(json).map_err(|_| "bad response")?;
    match (r.five_hour, r.seven_day) {
        (Some(f), Some(s)) => Ok(ClaudeUsage::Limits { five_hour: limit(f), seven_day: limit(s), plan }),
        _ => Err("bad response".into()),
    }
}

/// Tokens (input + output + cache) of Claude Code's last 5 h, from transcripts changed in that time.
// shortcut: rereads each recent file fully per call; keep byte offsets per file if this gets slow.
fn local_tokens() -> u64 {
    let Some(root) = claude_dir().map(|d| d.join("projects")) else { return 0 };
    let cutoff = SystemTime::now() - WINDOW;
    let mut files = Vec::new();
    recent_jsonl(&root, cutoff, &mut files);
    let since = now_secs() - WINDOW.as_secs() as i64;
    let mut seen = HashSet::new();
    files.iter().filter_map(|f| fs::read_to_string(f).ok()).map(|t| sum_tokens(t.lines(), since, &mut seen)).sum()
}

fn recent_jsonl(dir: &Path, cutoff: SystemTime, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            recent_jsonl(&path, cutoff, out);
        } else if path.extension().is_some_and(|x| x == "jsonl") && meta.modified().is_ok_and(|m| m >= cutoff) {
            out.push(path);
        }
    }
}

#[derive(Deserialize)]
struct Line {
    timestamp: String,
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    usage: Usage,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

/// Sums usage of assistant lines at or after `since` (unix s). Claude Code writes one line per
/// content block with the same message id and usage, so each id counts once.
fn sum_tokens<'a>(lines: impl Iterator<Item = &'a str>, since: i64, seen: &mut HashSet<String>) -> u64 {
    let mut total = 0;
    // Cheap text check first: most lines are big tool results we don't need to parse.
    for line in lines.filter(|l| l.contains(r#""type":"assistant""#) && l.contains(r#""usage""#)) {
        let Ok(l) = serde_json::from_str::<Line>(line) else { continue };
        if iso_secs(&l.timestamp).is_none_or(|t| t < since) {
            continue;
        }
        if let Some(id) = l.message.id {
            if !seen.insert(id) {
                continue;
            }
        }
        let u = l.message.usage;
        total += u.input_tokens + u.output_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
    }
    total
}

/// "2026-11-11T15:48:32.604Z" to unix seconds; the zone is assumed UTC (Claude Code writes Z).
fn iso_secs(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage() {
        let json = r#"{"five_hour":{"utilization":36.0,"resets_at":"2026-11-11T22:29:59.547056+00:00","limit_dollars":null},
            "seven_day":{"utilization":62,"resets_at":"2026-11-13T05:59:59.547073+00:00"},
            "seven_day_opus":null,"extra_usage":{"is_enabled":false},"limits":[1,2,3]}"#;
        assert_eq!(
            parse(json, "pro".into()),
            Ok(ClaudeUsage::Limits {
                five_hour: Limit { used_pct: 36.0, resets_at: Some("2026-11-11T22:29:59.547056+00:00".into()) },
                seven_day: Limit { used_pct: 62.0, resets_at: Some("2026-11-13T05:59:59.547073+00:00".into()) },
                plan: "pro".into(),
            })
        );
        // Unused window: null utilization and reset.
        let idle = r#"{"five_hour":{"utilization":null,"resets_at":null},"seven_day":{"utilization":0,"resets_at":null}}"#;
        assert!(matches!(parse(idle, "max".into()), Ok(ClaudeUsage::Limits { five_hour: Limit { used_pct: 0.0, resets_at: None }, .. })));
        assert!(parse("{}", "pro".into()).is_err());
        assert!(parse(r#"{"five_hour":null,"seven_day":null}"#, "pro".into()).is_err());
    }

    #[test]
    fn expired_token() {
        assert!(expired(1_000, 1_000));
        assert!(expired(999, 1_000));
        assert!(!expired(1_001, 1_000));
    }

    #[test]
    fn iso_to_unix() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_secs("2026-11-11T15:48:32.604Z"), Some(1_794_412_112));
        assert_eq!(iso_secs("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(iso_secs("garbage"), None);
    }

    #[test]
    fn sums_recent_usage() {
        let now = iso_secs("2026-11-11T20:00:00Z").unwrap();
        let lines = [
            // In window, written twice (two content blocks): counts once.
            r#"{"type":"assistant","timestamp":"2026-11-11T19:00:00.000Z","message":{"id":"a","usage":{"input_tokens":2,"output_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"service_tier":"standard"}}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-11T19:00:01.000Z","message":{"id":"a","usage":{"input_tokens":2,"output_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#,
            // In window, missing cache fields.
            r#"{"type":"assistant","timestamp":"2026-11-11T16:00:00Z","message":{"id":"b","usage":{"input_tokens":5,"output_tokens":7}}}"#,
            // Older than 5 h.
            r#"{"type":"assistant","timestamp":"2026-11-11T14:59:59Z","message":{"id":"c","usage":{"input_tokens":999,"output_tokens":999}}}"#,
            // User line, malformed line, assistant line without usage.
            r#"{"type":"user","timestamp":"2026-11-11T19:00:00Z","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","usage" broken"#,
            r#"{"type":"assistant","timestamp":"2026-11-11T19:00:00Z","message":{"id":"d"},"usage":1}"#,
        ];
        assert_eq!(sum_tokens(lines.into_iter(), now - 5 * 3600, &mut HashSet::new()), 1112 + 12);
    }

    #[test]
    #[ignore]
    fn claude_live() {
        println!("{:?}", fetch());
    }
}
