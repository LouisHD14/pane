//! Tiny blocking HTTPS GET over the WinRT HttpClient (no extra crates).

use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

/// Blocking GET; Ok(body) on 2xx, Err("HTTP <code>") or Err("network error 0x...").
/// Errors never contain the URL or header values, so tokens can't leak into logs.
// shortcut: no request timeout (WinRT default), add a cancel timer if a fetch ever hangs its thread.
pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<String, String> {
    let (status, body) = (|| {
        let client = HttpClient::new()?;
        let h = client.DefaultRequestHeaders()?;
        for (k, v) in headers {
            h.TryAppendWithoutValidation(&HSTRING::from(*k), &HSTRING::from(*v))?;
        }
        let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.join()?;
        let status = resp.StatusCode()?.0;
        let body = resp.Content()?.ReadAsStringAsync()?.join()?;
        Ok::<_, windows::core::Error>((status, body.to_string()))
    })()
    .map_err(|e| format!("network error 0x{:08X}", e.code().0))?;
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(format!("HTTP {status}"))
    }
}
