//! Codex (ChatGPT plan) usage limits, read with the Codex CLI's local login (`~/.codex/auth.json`).
//! The same endpoint Codex's `/status` uses. Tokens are refreshed like Codex does and written back.

use crate::claude::Limit;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum CodexUsage {
    Limits { five_hour: Limit, seven_day: Limit, plan: String },
    /// Logged in, but the limits couldn't be read; why is in `note`.
    Error { note: String },
}

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// Codex CLI's OAuth app (from its binary, 2026-11).
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const RELOGIN: &str = "login expired — run `codex login`";

pub fn auth_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex").join("auth.json"))
}

/// True when the Codex CLI is logged in with a ChatGPT account (API-key logins have no plan limits).
pub fn logged_in() -> bool {
    read().is_ok()
}

/// Blocking. None when Codex isn't logged in with ChatGPT (the widget stays hidden).
pub fn fetch() -> Option<CodexUsage> {
    let tokens = read().ok()?;
    Some(limits(tokens).unwrap_or_else(|note| CodexUsage::Error { note }))
}

// No Debug derive: holds tokens.
#[derive(Deserialize)]
struct Auth {
    tokens: Option<Tokens>,
}

#[derive(Deserialize, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    account_id: Option<String>,
}

fn read() -> Result<Tokens, String> {
    let text = fs::read_to_string(auth_path().ok_or("no user profile")?).map_err(|_| "not logged in to Codex")?;
    let auth: Auth = serde_json::from_str(&text).map_err(|_| "not logged in to Codex")?;
    auth.tokens.filter(|t| !t.access_token.is_empty()).ok_or_else(|| "not logged in to Codex".into())
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn limits(mut t: Tokens) -> Result<CodexUsage, String> {
    if jwt_exp(&t.access_token).is_some_and(|exp| exp <= now_secs() + 60) {
        t = refresh(&t)?;
    }
    let body = match ask(&t) {
        Err(e) if e == "HTTP 401" => ask(&refresh(&read()?)?),
        other => other,
    }?;
    parse(&body)
}

fn ask(t: &Tokens) -> Result<String, String> {
    let auth = format!("Bearer {}", t.access_token);
    let account = t.account_id.clone().unwrap_or_default();
    crate::http::get(USAGE_URL, &[("Authorization", &auth), ("ChatGPT-Account-Id", &account), ("User-Agent", "pane")])
}

#[derive(Deserialize)]
struct Refreshed {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

/// Gets new tokens and writes them into Codex's auth.json (only the token fields and `last_refresh` change).
fn refresh(t: &Tokens) -> Result<Tokens, String> {
    let token = t.refresh_token.as_deref().filter(|r| !r.is_empty()).ok_or(RELOGIN)?;
    let body = serde_json::json!({
        "client_id": CLIENT_ID, "grant_type": "refresh_token", "refresh_token": token, "scope": "openid profile email"
    });
    let reply = crate::http::post_json(TOKEN_URL, &body).map_err(|e| match e.as_str() {
        "HTTP 400" | "HTTP 401" => RELOGIN.to_string(),
        _ => e,
    })?;
    let new: Refreshed = serde_json::from_str(&reply).map_err(|_| "bad token response")?;
    let mut out = t.clone();
    if let Some(a) = &new.access_token {
        out.access_token = a.clone();
    }
    if new.refresh_token.is_some() {
        out.refresh_token = new.refresh_token.clone();
    }
    let path = auth_path().ok_or("no user profile")?;
    let stamp = iso_now();
    crate::win::update_json(&path, |root| {
        let tokens = root.get_mut("tokens").and_then(|v| v.as_object_mut()).ok_or("no tokens")?;
        for (key, value) in [("access_token", &new.access_token), ("refresh_token", &new.refresh_token), ("id_token", &new.id_token)] {
            if let Some(v) = value {
                tokens.insert(key.into(), v.clone().into());
            }
        }
        root["last_refresh"] = stamp.into();
        Ok(())
    })?;
    Ok(out)
}

/// The `exp` (unix s) of a JWT, without checking its signature (only used to know when to refresh).
fn jwt_exp(jwt: &str) -> Option<i64> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?.get("exp")?.as_i64()
}

fn iso_now() -> String {
    iso_of(now_secs())
}

#[derive(Deserialize)]
struct Response {
    plan_type: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: Option<f32>,
    /// Unix s.
    reset_at: Option<i64>,
}

fn limit(w: Option<Window>) -> Limit {
    let w = w.unwrap_or(Window { used_percent: None, reset_at: None });
    Limit { used_pct: w.used_percent.unwrap_or(0.0), resets_at: w.reset_at.map(iso_of) }
}

/// Unix seconds to "2026-11-12T09:04:40Z" (civil date from days, Howard Hinnant's algorithm).
fn iso_of(s: i64) -> String {
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn parse(json: &str) -> Result<CodexUsage, String> {
    let r: Response = serde_json::from_str(json).map_err(|_| "bad response")?;
    let rl = r.rate_limit.ok_or("bad response")?;
    if rl.primary_window.is_none() && rl.secondary_window.is_none() {
        return Err("bad response".into());
    }
    Ok(CodexUsage::Limits { five_hour: limit(rl.primary_window), seven_day: limit(rl.secondary_window), plan: r.plan_type.unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wham_usage() {
        let json = r#"{"plan_type":"plus","rate_limit":{"allowed":false,"limit_reached":true,
            "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_after_seconds":18000,"reset_at":1794492281},
            "secondary_window":{"used_percent":100,"limit_window_seconds":604800,"reset_after_seconds":300913,"reset_at":0}},"credits":null}"#;
        assert_eq!(
            parse(json),
            Ok(CodexUsage::Limits {
                five_hour: Limit { used_pct: 12.0, resets_at: Some(iso_of(1_794_492_281)) },
                seven_day: Limit { used_pct: 100.0, resets_at: Some("1970-01-01T00:00:00Z".into()) },
                plan: "plus".into(),
            })
        );
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"rate_limit":{}}"#).is_err());
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso_of(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_of(1_709_208_000), "2024-02-29T12:00:00Z");
        assert_eq!(iso_of(1_794_412_112), "2026-11-11T15:48:32Z");
    }

    #[test]
    fn reads_jwt_expiry() {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1794500000,"sub":"x"}"#);
        assert_eq!(jwt_exp(&format!("h.{payload}.sig")), Some(1_794_500_000));
        assert_eq!(jwt_exp("garbage"), None);
    }

    #[test]
    #[ignore]
    fn codex_live() {
        println!("{:?}", fetch());
    }
}
