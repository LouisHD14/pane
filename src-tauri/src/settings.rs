//! The switches in the bar's right-click "Settings" menu, kept in `settings.json` in Pane's data folder.

use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// The bar floats over apps and slides away; it comes back when the mouse touches the bottom edge
    /// (like Windows' "Automatically hide the taskbar"). Off: the bar reserves its screen edge.
    pub autohide: bool,
    /// Tint the glass with the colours of the apps just above the bar.
    pub adaptive_color: bool,
    pub show_media: bool,
    pub show_claude: bool,
    pub show_codex: bool,
    pub show_usage: bool,
    pub show_weather: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autohide: false,
            adaptive_color: false,
            show_media: true,
            show_claude: true,
            show_codex: true,
            show_usage: true,
            show_weather: true,
        }
    }
}

static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    crate::win::data_dir().join("settings.json")
}

pub fn get() -> Settings {
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    current
        .get_or_insert_with(|| fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
        .clone()
}

/// Sets one switch by its field name; returns the new settings, or None for an unknown name.
pub fn set(key: &str, on: bool) -> Option<Settings> {
    let mut s = get();
    let field = match key {
        "autohide" => &mut s.autohide,
        "adaptive_color" => &mut s.adaptive_color,
        "show_media" => &mut s.show_media,
        "show_claude" => &mut s.show_claude,
        "show_codex" => &mut s.show_codex,
        "show_usage" => &mut s.show_usage,
        "show_weather" => &mut s.show_weather,
        _ => return None,
    };
    *field = on;
    let dir = crate::win::data_dir();
    let saved = fs::create_dir_all(&dir).and_then(|_| fs::write(path(), serde_json::to_string_pretty(&s).unwrap_or_default()));
    if let Err(e) = saved {
        crate::win::log(&format!("can't save settings: {e}"));
    }
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_or_partial_files_get_defaults() {
        let s: Settings = serde_json::from_str(r#"{"autohide":true}"#).unwrap();
        assert!(s.autohide && s.show_media && !s.adaptive_color);
    }
}
