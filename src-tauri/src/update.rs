//! Auto-update from GitHub releases (tauri-plugin-updater). Pane only tells the bars; the user clicks to install.

use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// The GitHub repository Pane is released from (also where "Report a bug" opens a new issue).
pub const REPO: &str = "LouisHD14/pane";

/// How often Pane's releases were downloaded, for keeping track of how far it has spread. Read from the public
/// GitHub API when the settings window asks for it (never on a timer); the request carries no account, token or
/// anything about this PC, only a plain "Pane" user agent.
#[derive(serde::Serialize, Clone, PartialEq, Debug, Default)]
pub struct Downloads {
    /// Every release asset together (installers and the updater bundles).
    pub total: u64,
    /// Just the installers (`-setup.exe`), which is as close to "installs" as GitHub can say.
    pub installers: u64,
    /// This version's installers, when it has a release.
    pub this_version: Option<u64>,
    pub newest: String,
    pub releases: usize,
}

/// Reads the download counts of all releases. `version` is the running Pane's (its own line in the result).
pub fn downloads(version: &str) -> Result<Downloads, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let body = crate::http::get(&url, &[("User-Agent", "Pane"), ("Accept", "application/vnd.github+json")])?;
    parse_downloads(&body, version)
}

fn parse_downloads(body: &str, version: &str) -> Result<Downloads, String> {
    let list: Vec<serde_json::Value> = serde_json::from_str(body).map_err(|_| "unreadable answer from GitHub".to_string())?;
    let mut out = Downloads { releases: list.len(), ..Default::default() };
    for release in &list {
        let tag = release.get("tag_name").and_then(|t| t.as_str()).unwrap_or_default();
        if out.newest.is_empty() && release.get("draft").and_then(|d| d.as_bool()) != Some(true) {
            out.newest = tag.to_string();
        }
        let mut installers = 0;
        for asset in release.get("assets").and_then(|a| a.as_array()).into_iter().flatten() {
            let count = asset.get("download_count").and_then(|c| c.as_u64()).unwrap_or(0);
            out.total += count;
            if asset.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.to_lowercase().ends_with("-setup.exe")) {
                installers += count;
            }
        }
        out.installers += installers;
        if tag.trim_start_matches('v') == version {
            out.this_version = Some(installers);
        }
    }
    Ok(out)
}

#[derive(Clone, serde::Serialize)]
pub struct Available {
    version: String,
    notes: String,
}

/// Checks GitHub for a newer version; tells the bars ("update" event) if there is one.
pub async fn check(app: &AppHandle) -> Option<String> {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    match found {
        Ok(Some(update)) => {
            let notes = update.body.clone().unwrap_or_default();
            let _ = app.emit("update", Available { version: update.version.clone(), notes });
            Some(update.version)
        }
        Ok(None) => None,
        Err(e) => {
            crate::win::log(&format!("update check failed: {e}"));
            None
        }
    }
}

/// First check ~30 s after start, then every 6 hours.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        loop {
            tauri::async_runtime::block_on(check(&app));
            std::thread::sleep(Duration::from_secs(6 * 60 * 60));
        }
    });
}

/// Downloads the update, brings the Windows taskbar back and starts the installer, which ends Pane
/// and starts the new version. Returns only on failure (then Pane goes back to how it was).
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else { return Err("no update".into()) };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    // The installer replaces pane.exe and the plugin ends Pane with process::exit (no RunEvent::Exit),
    // so swap the taskbar back first and wait until that is done.
    let was_shown = crate::SHOWN.load(std::sync::atomic::Ordering::SeqCst);
    crate::shown_and_wait(app, false);
    let result = update.install(bytes).map_err(|e| e.to_string());
    crate::shown_and_wait(app, was_shown);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape GitHub really answers with (cut down from the live releases, 2026-11-13).
    const RELEASES: &str = r#"[
      {"tag_name":"v0.9.0","draft":false,"assets":[
        {"name":"latest.json","download_count":10},
        {"name":"Pane_0.9.0_x64-setup.exe","download_count":3},
        {"name":"Pane_0.9.0_x64-setup.exe.sig","download_count":1}]},
      {"tag_name":"v0.8.1","draft":false,"assets":[
        {"name":"latest.json","download_count":23},
        {"name":"Pane_0.8.1_x64-setup.exe","download_count":1},
        {"name":"Pane_0.8.1_x64-setup.exe.sig","download_count":1}]},
      {"tag_name":"v0.6.2","draft":false,"assets":[
        {"name":"Pane_0.6.2_x64-setup.exe","download_count":1}]}
    ]"#;

    #[test]
    fn counts_installers_apart_from_update_checks() {
        let d = parse_downloads(RELEASES, "0.9.0").unwrap();
        // Installers only: latest.json is the updater asking, .sig is its signature, neither is an install.
        assert_eq!((d.installers, d.this_version), (5, Some(3)));
        assert_eq!((d.total, d.releases, d.newest.as_str()), (40, 3, "v0.9.0"));
        // A version without a release of its own says so instead of claiming zero.
        assert_eq!(parse_downloads(RELEASES, "0.9.1").unwrap().this_version, None);
        assert!(parse_downloads("not json", "0.9.0").is_err());
        assert_eq!(parse_downloads("[]", "0.9.0").unwrap(), Downloads::default());
    }
}
