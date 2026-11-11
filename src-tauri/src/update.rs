//! Auto-update from GitHub releases (tauri-plugin-updater). Pane only tells the bars; the user clicks to install.

use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

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
