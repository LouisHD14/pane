//! Pane's settings (right-click menu switches and the settings window), kept in `settings.json` in Pane's data folder.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// The bar floats over apps and slides away; it comes back when the mouse touches the bottom edge
    /// (like Windows' "Automatically hide the taskbar"). Off: the bar reserves its screen edge.
    pub autohide: bool,
    /// The bar fills its screen edge from side to side with square corners, like Windows' own taskbar.
    /// Off: a rounded glass bar floating a few px from the edges.
    pub fill: bool,
    /// Tint the glass with the colours of the apps just above the bar.
    pub adaptive_color: bool,
    /// Paint the strip around the floating bar in the colours of the app's bottom edge, so the bar
    /// seems to sit on the app instead of on the wallpaper. No effect with `fill` (no strip around it).
    pub blend: bool,
    /// With several monitors: every bar shows the apps of all monitors. Off: each bar only the apps on its own
    /// monitor (pinned apps are on every bar either way).
    pub all_monitors: bool,
    /// Draw the bars without the GPU (WebView2 --disable-gpu): ~60-100 MB less memory, a little CPU while
    /// something animates (measured 2026-11-12). Read once at start, so it applies after a restart.
    pub low_memory: bool,
    pub show_media: bool,
    pub show_claude: bool,
    pub show_codex: bool,
    pub show_usage: bool,
    pub show_weather: bool,
    /// The user's own order of the pinned apps (app ids), set by dragging them in the bar. Only Pane's order:
    /// Windows' own taskbar pins stay as they are. Empty: Windows' pin order.
    pub pin_order: Vec<String>,
    /// How light the glass is: 0 dark ..= 100 light, 50 is Pane's original look. Darker adds acrylic tint
    /// (`tint`), lighter removes it and the pages add a white sheen (main.ts `applyLook`).
    pub glass: u8,
    /// Auto-hide: how long after the mouse left the bar it slides away (ms).
    pub hide_delay_ms: u32,
    /// Auto-hide: how long the slide takes (ms).
    pub slide_ms: u32,
    pub size: Size,
}

/// Bar height. The pages scale the bar's contents to match (main.ts `ZOOM`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Small,
    Normal,
    Large,
}

impl Size {
    /// Bar height in logical px; normal is the Windows 11 taskbar's.
    pub fn height(self) -> f64 {
        match self {
            Size::Small => 40.0,
            Size::Normal => 48.0,
            Size::Large => 56.0,
        }
    }
}

const GLASS_MAX: f64 = 100.0;
const HIDE_DELAY: (f64, f64) = (100.0, 3000.0);
const SLIDE: (f64, f64) = (0.0, 400.0);

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autohide: false,
            fill: false,
            adaptive_color: false,
            blend: false,
            all_monitors: false,
            low_memory: false,
            show_media: true,
            show_claude: true,
            show_codex: true,
            show_usage: true,
            show_weather: true,
            pin_order: Vec::new(),
            glass: 50,
            hide_delay_ms: 700,
            slide_ms: 120,
            size: Size::Normal,
        }
    }
}

impl Settings {
    fn switch(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            "autohide" => &mut self.autohide,
            "fill" => &mut self.fill,
            "adaptive_color" => &mut self.adaptive_color,
            "blend" => &mut self.blend,
            "all_monitors" => &mut self.all_monitors,
            "low_memory" => &mut self.low_memory,
            "show_media" => &mut self.show_media,
            "show_claude" => &mut self.show_claude,
            "show_codex" => &mut self.show_codex,
            "show_usage" => &mut self.show_usage,
            "show_weather" => &mut self.show_weather,
            _ => return None,
        })
    }

    /// Sets one field from a page's value; numbers are rounded and clamped. None: unknown key or wrong type.
    fn apply(&mut self, key: &str, value: &Value) -> Option<()> {
        if let Some(field) = self.switch(key) {
            *field = value.as_bool()?;
            return Some(());
        }
        let num = |(lo, hi): (f64, f64)| value.as_f64().map(|n| n.round().clamp(lo, hi));
        match key {
            "glass" => self.glass = num((0.0, GLASS_MAX))? as u8,
            "hide_delay_ms" => self.hide_delay_ms = num(HIDE_DELAY)? as u32,
            "slide_ms" => self.slide_ms = num(SLIDE)? as u32,
            "size" => self.size = Size::deserialize(value).ok()?,
            _ => return None,
        }
        Some(())
    }

    /// A hand-edited file gets the same limits as the settings window.
    fn clamped(mut self) -> Self {
        self.glass = self.glass.min(GLASS_MAX as u8);
        self.hide_delay_ms = (self.hide_delay_ms as f64).clamp(HIDE_DELAY.0, HIDE_DELAY.1) as u32;
        self.slide_ms = (self.slide_ms as f64).clamp(SLIDE.0, SLIDE.1) as u32;
        self
    }

    /// Acrylic tint (0xAABBGGRR) for the glass lightness: 50 is the original 0x40_1A_12_10 (dark blue-grey over
    /// ~25 % of the blur); towards 0 it covers up to 75 %, towards 100 it fades out.
    pub fn tint(&self) -> u32 {
        let light = self.glass.min(100) as u32;
        let alpha = if light <= 50 { 0x40 + (50 - light) * 0x80 / 50 } else { 0x40 * (100 - light) / 50 };
        alpha << 24 | 0x1A_12_10
    }
}

static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    crate::win::data_dir().join("settings.json")
}

pub fn get() -> Settings {
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    current
        .get_or_insert_with(|| {
            let saved: Option<Settings> = fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok());
            saved.unwrap_or_default().clamped()
        })
        .clone()
}

/// Sets one setting by its field name; returns the new settings, or None for an unknown name or a wrong value.
pub fn set(key: &str, value: &Value) -> Option<Settings> {
    let mut s = get();
    s.apply(key, value)?;
    Some(save(s))
}

/// Saves the pinned apps' order (app ids, first = leftmost).
pub fn set_pin_order(order: Vec<String>) -> Settings {
    save(Settings { pin_order: order, ..get() })
}

fn save(s: Settings) -> Settings {
    let dir = crate::win::data_dir();
    let saved = fs::create_dir_all(&dir).and_then(|_| fs::write(path(), serde_json::to_string_pretty(&s).unwrap_or_default()));
    if let Err(e) = saved {
        crate::win::log(&format!("can't save settings: {e}"));
    }
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_or_partial_files_get_defaults() {
        let s: Settings = serde_json::from_str(r#"{"autohide":true}"#).unwrap();
        assert!(s.autohide && s.show_media && !s.adaptive_color && !s.blend && !s.all_monitors && !s.low_memory && s.pin_order.is_empty());
        assert_eq!((s.glass, s.hide_delay_ms, s.slide_ms, s.size), (50, 700, 120, Size::Normal));
    }

    #[test]
    fn defaults_keep_todays_look() {
        let s = Settings::default();
        assert_eq!(s.tint(), 0x40_1A_12_10);
        assert_eq!(s.size.height(), 48.0);
        assert_eq!(Settings { glass: 0, ..s.clone() }.tint(), 0xC0_1A_12_10);
        assert_eq!(Settings { glass: 100, ..s }.tint(), 0x00_1A_12_10);
    }

    #[test]
    fn values_are_checked_and_clamped() {
        let mut s = Settings::default();
        assert!(s.apply("glass", &json!(250)).is_some() && s.glass == 100);
        assert!(s.apply("glass", &json!(-4)).is_some() && s.glass == 0);
        assert!(s.apply("glass", &json!(33.6)).is_some() && s.glass == 34);
        assert!(s.apply("hide_delay_ms", &json!(0)).is_some() && s.hide_delay_ms == 100);
        assert!(s.apply("slide_ms", &json!(99999)).is_some() && s.slide_ms == 400);
        assert!(s.apply("size", &json!("large")).is_some() && s.size == Size::Large);
        assert!(s.apply("size", &json!("huge")).is_none() && s.size == Size::Large);
        assert!(s.apply("autohide", &json!(true)).is_some() && s.autohide);
        assert!(s.apply("autohide", &json!(1)).is_none());
        assert!(s.apply("glass", &json!("light")).is_none());
        assert!(s.apply("nope", &json!(true)).is_none());
        let edited: Settings = serde_json::from_str(r#"{"glass":200,"hide_delay_ms":5,"slide_ms":5000}"#).unwrap();
        let edited = edited.clamped();
        assert_eq!((edited.glass, edited.hide_delay_ms, edited.slide_ms), (100, 100, 400));
    }
}
