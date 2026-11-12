//! Tiny blocking HTTPS GET with `ureq` (rustls, its own TLS).
//! Windows' WinRT HttpClient failed Open-Meteo with 0x80072F8F on the user's PC while Windows' own
//! curl reached it fine (2026-11-11), so Pane doesn't depend on that client any more.

use std::sync::OnceLock;
use std::time::Duration;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into())
}

/// Blocking GET; Ok(body) on 2xx, else a short reason ("HTTP 401", "timeout", ...).
/// Errors never contain the URL or header values, so tokens can't leak into the log.
pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<String, String> {
    let mut request = agent().get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.call().map_err(|e| reason(&e))?;
    response.body_mut().read_to_string().map_err(|e| reason(&e))
}

/// Blocking POST of a JSON body; Ok(response body) on 2xx. Same error rules as `get`.
pub fn post_json(url: &str, body: &serde_json::Value) -> Result<String, String> {
    let mut response = agent().post(url).header("Content-Type", "application/json").send(body.to_string()).map_err(|e| reason(&e))?;
    response.body_mut().read_to_string().map_err(|e| reason(&e))
}

fn reason(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        ureq::Error::Timeout(_) => "timeout".into(),
        ureq::Error::HostNotFound => "host not found".into(),
        ureq::Error::ConnectionFailed => "connection failed".into(),
        ureq::Error::Io(io) => format!("network error ({:?})", io.kind()),
        _ => "network error".into(),
    }
}
