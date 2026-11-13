// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod apps;
mod claude;
mod codex;
mod colors;
mod docker;
mod http;
mod media;
mod preview;
mod settings;
mod status;
mod tray;
mod update;
mod usage;
mod weather;
mod win;

use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::ShortcutState;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_SHIFT};

/// The floating bar keeps this far (logical px) from the screen edges...
const GAP: f64 = 6.0;
/// ...and this far from maximized windows above it.
const TOP_GAP: f64 = 2.0;
/// WebView2 start options. Tauri's own defaults plus: one renderer process for all bars and the popup instead of
/// one each (~30 MB each); with "Low memory mode" no GPU process either (~60-100 MB less, measured 2026-11-12).
fn browser_args_for(low_memory: bool) -> String {
    let args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --process-per-site --renderer-process-limit=1";
    if low_memory { format!("{args} --disable-gpu") } else { args.to_string() }
}

/// The start options for every Pane window, fixed at the first call: all windows must use the same ones (WebView2
/// refuses a second set in one process), so a changed "Low memory mode" applies after a restart.
pub(crate) fn browser_args() -> &'static str {
    static ARGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ARGS.get_or_init(|| browser_args_for(settings::get().low_memory))
}
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// True while Pane is shown and the Windows taskbar is hidden. Changed only on the main thread.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Makes window labels unique when the bars are rebuilt after a monitor change.
static GENERATION: AtomicUsize = AtomicUsize::new(0);

struct Bar {
    label: String,
    monitor: RECT,
    /// The monitor's DPI scale (1.0 = 96 dpi).
    scale: f64,
    /// Hidden because a fullscreen app (game, video) is in front on this monitor.
    fullscreen: bool,
    /// Auto-hide: slid away until the mouse touches the bottom edge.
    concealed: bool,
    /// Auto-hide: when the mouse left the bar (it hides a moment later).
    left_at: Option<Instant>,
}
struct Bars(Mutex<Vec<Bar>>);

/// Pane's popups (Pane menu, hidden icons, Docker) that are open now, by label (`{kind}-{bar label}`).
/// Auto-hide keeps a bar up while one of its popups is open; this replaced a single "hold" flag that could get
/// stuck on and keep the bar up for good.
static POPUPS_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Closed popups kept hidden for a quick reopen, with the time they closed. A new WebView page costs a few
/// hundred ms to build (on slower PCs much more), which made the menus feel slow; a kept one opens at once.
/// Destroyed after `KEEP` unused (a hidden page holds ~30 MB) or right away in "Low memory mode".
static POPUPS_KEPT: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
const KEEP: Duration = Duration::from_secs(120);
/// The popup kinds a bar can have (label prefix).
const POPUP_KINDS: [&str; 4] = ["overflow", "menu", "docker", "egg"];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A popup is (again) open: not kept for cleanup any more, and its bar stays up.
fn popup_opened(label: &str) {
    lock(&POPUPS_KEPT).retain(|(l, _)| l != label);
    let mut open = lock(&POPUPS_OPEN);
    if !open.iter().any(|l| l == label) {
        open.push(label.to_string());
    }
}

/// True while one of the bar's popups is open.
fn bar_has_popup(bar: &str) -> bool {
    lock(&POPUPS_OPEN).iter().any(|l| l.split_once('-').is_some_and(|(_, b)| b == bar))
}

/// Hides every open popup of `bar` (when the bar slides away or Pane is hidden). Main thread or any.
fn close_popups_of(app: &AppHandle, bar: &str) {
    let labels: Vec<String> = lock(&POPUPS_OPEN).iter().filter(|l| l.split_once('-').is_some_and(|(_, b)| b == bar)).cloned().collect();
    for label in labels {
        if let Some(w) = app.get_webview_window(&label) {
            close_popup(w);
        }
    }
}

/// Destroys kept popups that weren't reopened for `KEEP`.
fn drop_idle_popups(app: &AppHandle) {
    let idle: Vec<String> = {
        let mut kept = lock(&POPUPS_KEPT);
        let idle = kept.iter().filter(|(_, at)| at.elapsed() > KEEP).map(|(l, _)| l.clone()).collect();
        kept.retain(|(_, at)| at.elapsed() <= KEEP);
        idle
    };
    for label in idle {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.destroy();
        }
    }
}

impl Bar {
    /// The bar window's rect: the bar itself, or with "App colour under the bar" the whole strip (the page paints
    /// the strip in the app's colours and draws the floating bar inside it with the same margins).
    fn window(&self, s: &settings::Settings) -> RECT {
        let (strip, bar) = self.layout(s);
        if s.blend && !s.fill { strip } else { bar }
    }

    /// (strip reserved at the bottom of the monitor, the bar window's rect) in physical px.
    fn layout(&self, s: &settings::Settings) -> (RECT, RECT) {
        let px = |logical: f64| (logical * self.scale).round() as i32;
        let (gap, top_gap) = if s.fill { (0, 0) } else { (px(GAP), px(TOP_GAP)) };
        layout(self.monitor, px(s.size.height()), gap, top_gap)
    }
}

/// The bar `gap` px from the bottom and the sides; the reserved strip ends `top_gap` px above it.
fn layout(m: RECT, height: i32, gap: i32, top_gap: i32) -> (RECT, RECT) {
    let bar = RECT { left: m.left + gap, top: m.bottom - gap - height, right: m.right - gap, bottom: m.bottom - gap };
    (RECT { top: bar.top - top_gap, ..m }, bar)
}

fn bars(app: &AppHandle) -> MutexGuard<'_, Vec<Bar>> {
    app.state::<Bars>().inner().0.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shows or hides Pane and swaps the Windows taskbar the other way. Main thread only.
fn set_shown(app: &AppHandle, show: bool) {
    if SHOWN.swap(show, SeqCst) == show {
        return;
    }
    let dir = win::data_dir();
    if show {
        win::hide_taskbar(&dir);
        place_bars(app, true);
        let _ = app.emit("shown", ()); // plays the bar's entrance animation
        // Tray icons come to Pane only while it is shown; Explorer gets every message too.
        let h = app.clone();
        tray::start(move || push_tray(&h));
        // Explorer frees the taskbar's screen edge a moment after auto-hide turns on; place the bars again then.
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            on_main(&handle, |h| {
                if SHOWN.load(SeqCst) {
                    place_bars(h, true);
                }
            });
        });
    } else {
        preview::hide();
        let labels: Vec<String> = bars(app).iter().map(|b| b.label.clone()).collect();
        for label in labels {
            close_popups_of(app, &label);
        }
        tray::stop();
        place_bars(app, false);
        win::restore_taskbar(&dir);
    }
}

/// Runs `f` on the main thread, where window calls can't deadlock.
fn on_main(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || f(&handle));
}

/// `set_shown` from any thread, returning once the main thread has done it.
fn shown_and_wait(app: &AppHandle, show: bool) {
    let (tx, rx) = std::sync::mpsc::channel();
    on_main(app, move |h| {
        set_shown(h, show);
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

fn request(app: &AppHandle, show: impl FnOnce(bool) -> bool + Send + 'static) {
    on_main(app, move |h| set_shown(h, show(SHOWN.load(SeqCst))));
}

/// Running auto-hide slides: bar label -> the slide that may still move it (newer slides and place_bars win).
static SLIDES: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);
static SLIDE_ID: AtomicUsize = AtomicUsize::new(0);

/// Auto-hide: slides the whole bar window (glass and icons together) between its place `rc` and just below the
/// monitor's bottom edge. Motion in the spirit of Nintendo's menus (decided with the user 2026-11-13): quick to
/// answer, eased, and with a little life at the ends, but never floaty.
/// - Up in `slide_ms` (default 260 ms): fast start, a ~5 % overshoot (a few px) and a settle (ease-out-back).
/// - Down in 80 % of that: a tiny lift first (anticipation), then accelerating away (ease-in-back). Pure ease-out
///   on the way down rushed off and left a thin strip lingering at the edge.
/// Time-based and synced to DWM's frames on its own thread (async window moves), so a slow PC drops frames
/// instead of stretching the slide, and a busy main thread can't stall it. The page plays its part too
/// ("slide" event: the contents settle in a short cascade). Hidden at the end of a slide down and parked back
/// at its place, so everything else still finds it where it belongs.
fn slide(app: &AppHandle, label: String, rc: RECT, below: i32, show: bool) {
    let ms = settings::get().slide_ms;
    let Some(hwnd) = app.get_webview_window(&label).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize) else { return };
    let id = SLIDE_ID.fetch_add(1, SeqCst);
    SLIDES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(label.clone(), id);
    let _ = app.emit_to(label.as_str(), "slide", serde_json::json!({ "show": show, "ms": ms }));
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd as _);
        let current = || SLIDES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|s| s.get(&label).copied()) == Some(id);
        let at = |offset: f64| {
            let top = rc.top + ((below - rc.top) as f64 * offset).round() as i32;
            RECT { top, bottom: top + rc.bottom - rc.top, ..rc }
        };
        let duration = Duration::from_millis(if show { ms } else { ms * 4 / 5 } as u64).as_secs_f64();
        if show {
            win::move_pinned_async(hwnd, at(1.0));
            win::show_async(hwnd, true);
        }
        let start = Instant::now();
        loop {
            if !current() {
                return; // a newer slide or place_bars took over
            }
            let t = if duration > 0.0 { (start.elapsed().as_secs_f64() / duration).min(1.0) } else { 1.0 };
            win::move_pinned_async(hwnd, at(slide_offset(show, t)));
            if t >= 1.0 {
                break;
            }
            win::next_frame();
        }
        if !show && current() {
            win::show_async(hwnd, false);
            win::move_pinned_async(hwnd, rc);
        }
        let mut slides = SLIDES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slides.as_mut().filter(|s| s.get(&label) == Some(&id)) {
            s.remove(&label);
        }
    });
}

/// How far down the bar is at time `t` (0..=1) of a slide: 0 at its place, 1 just below the screen.
/// Slightly below 0 means above its place (the overshoot going up, the lift before going down).
fn slide_offset(show: bool, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if show {
        // ease-out-back, s = 1.1: about 5 % past the place, then back.
        let (c1, u) = (1.1, t - 1.0);
        1.0 - (1.0 + (c1 + 1.0) * u.powi(3) + c1 * u.powi(2))
    } else {
        // ease-in-back, s = 0.9: lifts ~3 % first, then drops away faster and faster.
        let c1 = 0.9;
        (c1 + 1.0) * t.powi(3) - c1 * t.powi(2)
    }
}

/// Main thread only.
fn place_bars(app: &AppHandle, show: bool) {
    let settings = settings::get();
    // Settings or monitors changed: any running slide would put the bar back at an old place.
    SLIDES.lock().unwrap_or_else(|e| e.into_inner()).take();
    for bar in bars(app).iter_mut() {
        let Some(window) = app.get_webview_window(&bar.label) else { continue };
        let Ok(hwnd) = window.hwnd() else { continue };
        let hwnd = HWND(hwnd.0);
        if show {
            let (strip, rc) = (bar.layout(&settings).0, bar.window(&settings));
            if settings.autohide {
                // Floats over the apps: nothing is reserved, the bar takes the same place as otherwise.
                win::release_edge(hwnd);
            } else {
                win::reserve_edge(hwnd, strip);
            }
            // Square window corners when the window is the whole strip (fill, or the app-colour floor).
            win::set_round(hwnd, !(settings.fill || settings.blend));
            win::pin(hwnd, Some(rc));
            let _ = window.set_position(PhysicalPosition::new(rc.left, rc.top));
            let _ = window.set_size(PhysicalSize::new((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32));
            // Auto-hide starts shown; the cursor loop hides it once the mouse is elsewhere.
            (bar.concealed, bar.left_at) = (false, None);
            if !bar.fullscreen {
                // Without activating: taking the focus from the user's app made windows flicker.
                win::show_inactive(hwnd);
            }
        } else {
            win::release_edge(hwnd);
            win::hide(hwnd);
        }
    }
}

fn monitor_rects(app: &AppHandle) -> Vec<(RECT, f64)> {
    let monitors = app.available_monitors().unwrap_or_default();
    monitors
        .iter()
        .map(|m| {
            let (pos, size) = (m.position(), m.size());
            let rect = RECT { left: pos.x, top: pos.y, right: pos.x + size.width as i32, bottom: pos.y + size.height as i32 };
            (rect, m.scale_factor())
        })
        .collect()
}

/// One bar window per monitor. Main thread only.
fn create_bars(app: &AppHandle) -> tauri::Result<()> {
    let generation = GENERATION.fetch_add(1, SeqCst);
    let mut new = Vec::new();
    let tint = settings::get().tint();
    for (i, (monitor, scale)) in monitor_rects(app).into_iter().enumerate() {
        let label = format!("bar-{generation}-{i}");
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("Pane")
            .additional_browser_args(browser_args())
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .build()?;
        win::set_glass(HWND(window.hwnd()?.0), tint);
        win::no_transitions(HWND(window.hwnd()?.0));
        new.push(Bar { label, monitor, scale, fullscreen: false, concealed: false, left_at: None });
    }
    *bars(app) = new;
    Ok(())
}

/// Monitors were plugged in/out or changed resolution: replace all bars. Main thread only.
fn rebuild_bars(app: &AppHandle) {
    preview::hide();
    let old: Vec<String> = bars(app).iter().map(|b| b.label.clone()).collect();
    for label in old {
        if let Some(window) = app.get_webview_window(&label) {
            if let Ok(hwnd) = window.hwnd() {
                win::release_edge(HWND(hwnd.0));
                win::pin(HWND(hwnd.0), None);
            }
            let _ = window.destroy();
        }
        for kind in POPUP_KINDS {
            let popup_label = format!("{kind}-{label}");
            lock(&POPUPS_OPEN).retain(|l| *l != popup_label);
            lock(&POPUPS_KEPT).retain(|(l, _)| *l != popup_label);
            if let Some(popup) = app.get_webview_window(&popup_label) {
                let _ = popup.destroy();
            }
        }
    }
    if let Err(e) = create_bars(app) {
        crate::win::log(&format!("can't rebuild bars: {e}"));
    }
    if SHOWN.load(SeqCst) {
        place_bars(app, true);
    }
}

/// Hides the bar on monitors where a fullscreen app is in front, shows it again afterwards.
fn check_fullscreen(app: &AppHandle) {
    if !SHOWN.load(SeqCst) {
        return;
    }
    let changes: Vec<(String, bool, bool)> = bars(app)
        .iter_mut()
        .filter_map(|bar| {
            let fullscreen = win::fullscreen_on(bar.monitor);
            (fullscreen != bar.fullscreen).then(|| {
                bar.fullscreen = fullscreen;
                (bar.label.clone(), fullscreen, bar.concealed)
            })
        })
        .collect();
    for (label, fullscreen, concealed) in changes {
        let Some(window) = app.get_webview_window(&label) else { continue };
        if fullscreen {
            // Nothing of Pane's stays over the game: its menu or Docker popup goes with the bar.
            close_popups_of(app, &label);
            preview::hide();
            if let Ok(hwnd) = window.hwnd() {
                win::hide(HWND(hwnd.0));
            }
        } else if !concealed {
            if let Ok(hwnd) = window.hwnd() {
                win::show_inactive(HWND(hwnd.0));
            }
        }
    }
}

/// What each bar was sent last, so unchanged lists aren't sent again (every event re-renders the bar).
static LAST_APPS: Mutex<Option<HashMap<String, Vec<apps::AppItem>>>> = Mutex::new(None);

/// Sends each bar the apps of its own monitor, when they changed. Window events come in bursts
/// (focus, title, redraw); before, every one of them re-rendered every bar.
fn push_apps(app: &AppHandle) {
    let targets: Vec<(String, RECT)> = bars(app).iter().map(|b| (b.label.clone(), b.monitor)).collect();
    let all = settings::get().all_monitors;
    let mut last = LAST_APPS.lock().unwrap_or_else(|e| e.into_inner());
    let last = last.get_or_insert_with(HashMap::new);
    last.retain(|label, _| targets.iter().any(|(l, _)| l == label));
    for (label, monitor) in targets {
        let now = apps::list((!all).then_some(monitor));
        if last.get(&label) != Some(&now) {
            let _ = app.emit_to(label.as_str(), "apps", &now);
            preview::apps_changed(&label, &now);
            last.insert(label, now);
        }
    }
}

/// CPU / GPU / RAM and network speed once a second, only while Pane is shown and the meters are on.
fn usage_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            if SHOWN.load(SeqCst) && settings::get().show_usage {
                let now = usage::sample();
                if last.as_ref() != Some(&now) {
                    let _ = app.emit("usage", &now);
                    last = Some(now);
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Last Codex usage (None: not logged in to Codex with ChatGPT, or not fetched yet).
static CODEX: Mutex<Option<codex::CodexUsage>> = Mutex::new(None);

fn fetch_codex(app: &AppHandle) {
    let now = codex::fetch();
    if let Some(codex::CodexUsage::Error { note }) = &now {
        win::log(&format!("codex limits unavailable: {note}"));
    }
    *CODEX.lock().unwrap_or_else(|e| e.into_inner()) = now.clone();
    let _ = app.emit("codex", now);
}

/// Codex plan limits every 5 minutes, sent to the bars.
fn codex_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        fetch_codex(&app);
        std::thread::sleep(Duration::from_secs(5 * 60));
    });
}

/// Auto-hide: shows a bar when the mouse touches the bottom edge of its monitor and hides it `hide_delay_ms`
/// (default 0.7 s) after the mouse left it. Polls the cursor every 60 ms, only while auto-hide is on (a mouse
/// hook would run on every mouse move in every app).
fn autohide_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        let on = SHOWN.load(SeqCst) && settings::get().autohide;
        std::thread::sleep(Duration::from_millis(if on { 60 } else { 500 }));
        let Some((x, y)) = win::cursor().filter(|_| on) else { continue };
        let preview_open = preview::is_open();
        let settings = settings::get();
        let delay = Duration::from_millis(settings.hide_delay_ms as u64);
        let mut changes = Vec::new();
        for bar in bars(&app).iter_mut() {
            let m = bar.monitor;
            let inside = x >= m.left && x < m.right && y >= m.top && y < m.bottom;
            let over = inside && y >= bar.layout(&settings).0.top;
            if bar.concealed {
                if inside && y >= m.bottom - 2 && !bar.fullscreen {
                    (bar.concealed, bar.left_at) = (false, None);
                    changes.push((bar.label.clone(), bar.window(&settings), m.bottom, true));
                }
                continue;
            }
            // Up while anything opened from it is: a preview, Pane's popups, or a menu that hangs off the bar
            // (an app icon's or a tray icon's context menu), so no menu is left floating over a bar that slid
            // away. Menus from the bar open upwards from its top edge, so only ones reaching into this strip
            // count; a menu elsewhere on the monitor doesn't keep the bar up.
            let strip = bar.layout(&settings).0;
            let near_bar = RECT { top: strip.top - (48.0 * bar.scale) as i32, ..m };
            let hold = over || bar.fullscreen || preview_open || bar_has_popup(&bar.label) || win::menu_open_in(near_bar);
            if hold {
                bar.left_at = None;
            } else if bar.left_at.get_or_insert_with(Instant::now).elapsed() > delay {
                bar.concealed = true;
                changes.push((bar.label.clone(), bar.window(&settings), m.bottom, false));
            }
        }
        for (label, rc, below, show) in changes {
            slide(&app, label, rc, below, show);
        }
    });
}

/// Adaptive colour and the app-colour floor: every 0.75 s, the colours of the app sitting on each bar
/// (colors.rs), sent to that bar once they held still. Only while one of the settings is on and the bar is visible.
fn tint_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut steady: HashMap<String, colors::Steady> = HashMap::new();
        let mut was_on = false;
        loop {
            std::thread::sleep(Duration::from_millis(750));
            let s = settings::get();
            let on = SHOWN.load(SeqCst) && (s.adaptive_color || s.blend);
            if !on {
                if was_on {
                    let _ = app.emit("tint", None::<Vec<[u8; 3]>>);
                    steady.clear();
                }
                was_on = false;
                continue;
            }
            was_on = true;
            let targets: Vec<(String, RECT, RECT)> = bars(&app)
                .iter()
                .filter(|b| !b.concealed && !b.fullscreen)
                .map(|b| (b.label.clone(), b.layout(&s).0, b.monitor))
                .collect();
            for (label, strip, monitor) in targets {
                let now = colors::app_floor(monitor, strip, 8);
                if let Some(show) = steady.entry(label.clone()).or_default().feed(now) {
                    let _ = app.emit_to(label.as_str(), "tint", show);
                }
            }
        }
    });
}

/// Last Claude usage (None until the first fetch is done).
static CLAUDE: Mutex<Option<claude::ClaudeUsage>> = Mutex::new(None);

/// Claude plan limits every 5 minutes, sent to the bars.
fn claude_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        let now = claude::fetch();
        if let claude::ClaudeUsage::Local { note, .. } = &now {
            win::log(&format!("claude limits unavailable: {note}"));
        }
        *CLAUDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(now.clone());
        let _ = app.emit("claude", now);
        std::thread::sleep(Duration::from_secs(5 * 60));
    });
}

/// Last weather, with the reason when there is none (no location, offline, not fetched yet).
static WEATHER: Mutex<Option<weather::Report>> = Mutex::new(None);

/// Fetches the weather now and every 30 minutes, and sends it to the bars. After a failure it tries again in a
/// minute for the first few tries (a timeout right after the PC woke up or Pane started shouldn't cost half an
/// hour of weather), then every 5 minutes.
fn weather_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut failures = 0u32;
        loop {
            let result = weather::fetch();
            let wait = match &result {
                Ok(_) => {
                    failures = 0;
                    30 * 60
                }
                Err(e) => {
                    win::log(&format!("no weather: {e}"));
                    failures += 1;
                    if failures <= 3 {
                        60
                    } else {
                        5 * 60
                    }
                }
            };
            let now = match result {
                Ok(w) => weather::Report { now: Some(w), note: String::new() },
                Err(note) => weather::Report { now: None, note },
            };
            *lock(&WEATHER) = Some(now.clone());
            let _ = app.emit("weather", now);
            std::thread::sleep(Duration::from_secs(wait));
        }
    });
}

/// Sends all tray icons to every bar and popup (bars show the promoted ones, popups the rest).
fn push_tray(app: &AppHandle) {
    static LAST: Mutex<Option<Vec<tray::TrayIcon>>> = Mutex::new(None);
    let now = tray::list();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    // Apps re-send their icons often with nothing changed; each send rebuilt the tray in every bar.
    if last.as_ref() != Some(&now) {
        let _ = app.emit("tray", &now);
        *last = Some(now);
    }
}

/// Starts `Pane.exe --watchdog <pid>`: it waits until this process ends (also by crash or Task Manager)
/// and then brings the taskbar back.
fn start_watchdog() {
    let Ok(exe) = std::env::current_exe() else { return };
    let pid = std::process::id().to_string();
    if let Err(e) = std::process::Command::new(exe).args(["--watchdog", &pid]).creation_flags(CREATE_NO_WINDOW).spawn() {
        crate::win::log(&format!("watchdog didn't start: {e}"));
    }
}

/// Once a second: keep Explorer's taskbar hidden, notice Explorer restarts, monitor changes and fullscreen apps.
// shortcut: polling instead of TaskbarCreated / WM_DISPLAYCHANGE messages; costs ~0 CPU, fine until Phase 3 adds a message window.
fn watch_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut tray = win::tray_id();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            drop_idle_popups(&app);
            if !SHOWN.load(SeqCst) {
                continue;
            }
            win::keep_hidden();
            let now = win::tray_id();
            if now != tray {
                tray = now;
                on_main(&app, |h| place_bars(h, true));
            }
            let current = monitor_rects(&app);
            let known: Vec<(RECT, f64)> = bars(&app).iter().map(|b| (b.monitor, b.scale)).collect();
            if current != known {
                on_main(&app, |h| {
                    rebuild_bars(h);
                    push_apps(h);
                });
            }
            check_fullscreen(&app);
        }
    });
}

/// Sends the status icons' state to all bars when it changes.
fn status_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let now = status::read();
            if last.as_ref() != Some(&now) {
                let _ = app.emit("status", &now);
                last = Some(now);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[tauri::command]
fn open_start() {
    win::press(&[VK_LWIN]);
}

#[tauri::command]
fn open_notifications() {
    win::press(&[VK_LWIN, VIRTUAL_KEY(b'N' as u16)]);
}

#[tauri::command]
fn open_quick_settings() {
    win::press(&[VK_LWIN, VIRTUAL_KEY(b'A' as u16)]);
}

#[tauri::command]
fn toggle(app: AppHandle) {
    request(&app, |shown| !shown);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0); // RunEvent::Exit below brings the taskbar back
}

/// The apps for the bar that asks (each bar shows its own monitor's windows).
#[tauri::command]
async fn get_apps(app: AppHandle, window: WebviewWindow) -> Vec<apps::AppItem> {
    let monitor = bars(&app).iter().find(|b| b.label == window.label()).map(|b| b.monitor);
    let all = settings::get().all_monitors;
    monitor.map(|m| apps::list((!all).then_some(m))).unwrap_or_default()
}

#[tauri::command]
async fn get_status() -> status::Status {
    status::read()
}

#[tauri::command]
fn activate(hwnd: isize) {
    apps::activate(hwnd);
}

#[tauri::command]
fn close_window(hwnd: isize) {
    apps::close(hwnd);
}

/// A click on a button of a window's thumbnail toolbar (in its preview).
#[tauri::command]
fn thumb_button(hwnd: isize, id: u32) {
    apps::click_thumb_button(hwnd, id);
}

/// Hover preview of an app's windows above its icon; `left`/`right` are the icon's edges in CSS px in the bar.
#[tauri::command]
async fn show_preview(app: AppHandle, window: WebviewWindow, id: String, left: f64, right: f64) {
    let Some(monitor) = bars(&app).iter().find(|b| b.label == window.label()).map(|b| b.monitor) else { return };
    let item = LAST_APPS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|all| all.get(window.label()).and_then(|list| list.iter().find(|a| a.id == id).cloned()))
        // Nothing sent to this bar yet (just rebuilt after a monitor change): ask for its apps now, so the
        // first hover after that still gets a preview.
        .or_else(|| {
            let all = settings::get().all_monitors;
            apps::list((!all).then_some(monitor)).into_iter().find(|a| a.id == id)
        });
    if let Some(item) = item {
        preview::show(window, monitor, item, left, right);
    }
}

// Async (not on the main thread) like every preview call, so they never wait on it.
#[tauri::command]
async fn hide_preview() {
    preview::hide();
}

/// The preview page asks what to show once it has loaded.
#[tauri::command]
async fn get_preview(window: WebviewWindow) -> Option<preview::Preview> {
    preview::current(window.label())
}

/// The preview page has drawn its cards: it may be shown now.
#[tauri::command]
async fn preview_drawn(window: WebviewWindow) {
    preview::drawn(window.label());
}

/// The mouse came onto the apps: get a preview page ready before the first hover needs it.
#[tauri::command]
async fn warm_preview() {
    preview::warm();
}

/// Last Docker state (None: Docker isn't running), sent to the bars and the Docker popups when it changes.
static DOCKER: Mutex<Option<docker::Docker>> = Mutex::new(None);

/// Docker containers every 3 s while Docker runs (one cheap request over its local pipe), every 10 s otherwise
/// (just a failed pipe open), and only while the widget is on.
fn docker_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        let on = SHOWN.load(SeqCst) && settings::get().show_docker;
        let now = if on { docker::read() } else { None };
        let changed = {
            let mut last = lock(&DOCKER);
            let changed = *last != now;
            *last = now.clone();
            changed
        };
        if changed {
            let _ = app.emit("docker", &now);
        }
        std::thread::sleep(Duration::from_secs(if now.is_some() { 3 } else { 10 }));
    });
}

#[tauri::command]
async fn get_docker() -> Option<docker::Docker> {
    lock(&DOCKER).clone()
}

/// Start, stop or restart a container from the Docker popup; the new state follows at once.
#[tauri::command]
async fn docker_action(app: AppHandle, id: String, action: docker::Action) -> Result<(), String> {
    let result = docker::act(&id, action);
    let now = docker::read();
    *lock(&DOCKER) = now.clone();
    let _ = app.emit("docker", &now);
    result
}

#[tauri::command]
async fn open_docker_desktop() {
    docker::open_desktop();
}

/// "Report a bug": the issue form on GitHub with Pane's version and the Windows build filled in. Nothing else
/// is sent; the log stays local (`open_log_folder`), the user can attach it themselves.
#[tauri::command]
async fn report_bug(app: AppHandle) {
    let version = app.package_info().version.to_string();
    let body = format!(
        "**What happened?**\n\n\n**What did you expect?**\n\n\n**Steps to reproduce**\n1. \n\n---\nPane {version}, Windows {}\n",
        win::windows_build()
    );
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    win::open(&format!("https://github.com/{}/issues/new?labels=bug&body={}", update::REPO, enc(&body)));
}

/// Opens Pane's data folder (pane.log, settings.json) in Explorer.
#[tauri::command]
async fn open_log_folder() {
    let dir = win::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    win::open(&dir.to_string_lossy());
}

/// How often Pane's releases were downloaded (settings window, on demand).
#[tauri::command]
async fn get_downloads(app: AppHandle) -> Result<update::Downloads, String> {
    let version = app.package_info().version.to_string();
    // Blocking HTTP, so off the async runtime's threads.
    tauri::async_runtime::spawn_blocking(move || update::downloads(&version))
        .await
        .unwrap_or_else(|_| Err("couldn't ask GitHub".into()))
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// A click on a preview: that window to the front (never minimizes, unlike a click on the icon).
#[tauri::command]
fn focus_window(hwnd: isize) {
    apps::bring_to_front(hwnd);
    preview::hide();
}

#[tauri::command]
async fn launch(id: String) {
    apps::launch(&id);
}

#[tauri::command]
async fn get_media() -> Option<media::NowPlaying> {
    media::current()
}

#[tauri::command]
async fn media_control(action: media::Action) {
    media::control(action);
}

#[tauri::command]
async fn get_weather() -> Option<weather::Report> {
    lock(&WEATHER).clone()
}

#[tauri::command]
async fn get_claude() -> Option<claude::ClaudeUsage> {
    CLAUDE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The Claude logo, taken from the Claude desktop app installed on this PC ("" if it isn't installed).
// shortcut: fixed package name of the Microsoft Store / MSIX Claude app; other installs fall back to the drawn glyph.
#[tauri::command]
async fn claude_icon() -> String {
    apps::icon_of("shell:AppsFolder\\Claude_pzs8sxrjxfjjc!Claude")
}

#[tauri::command]
async fn open_claude_usage() {
    win::open("https://claude.ai/settings/usage");
}

#[tauri::command]
async fn get_codex() -> Option<codex::CodexUsage> {
    CODEX.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn modified(path: Option<std::path::PathBuf>) -> Option<std::time::SystemTime> {
    std::fs::metadata(path?).and_then(|m| m.modified()).ok()
}

/// Opens a terminal with `codex login`; the widget appears once Codex wrote its login (checked for 5 minutes).
#[tauri::command]
fn open_codex_login(app: AppHandle) {
    win::run_in_terminal("codex login");
    std::thread::spawn(move || {
        let before = modified(codex::auth_path());
        for _ in 0..100 {
            std::thread::sleep(Duration::from_secs(3));
            let now = modified(codex::auth_path());
            if now.is_some() && now != before {
                std::thread::sleep(Duration::from_secs(1)); // let Codex finish writing
                return fetch_codex(&app);
            }
        }
    });
}

#[tauri::command]
async fn open_codex_usage() {
    win::open("https://chatgpt.com/codex/settings/usage");
}

#[tauri::command]
async fn codex_logged_in() -> bool {
    codex::logged_in()
}

/// Opens a terminal with Claude Code's login, for when its stored login can't be refreshed any more.
#[tauri::command]
fn open_claude_login() {
    win::run_in_terminal("claude /login");
}

#[tauri::command]
async fn get_settings() -> settings::Settings {
    settings::get()
}

/// Sets one setting (a switch's bool, a slider's number, the size's name; checked and clamped in settings.rs)
/// and applies it live.
#[tauri::command]
async fn set_setting(app: AppHandle, key: String, value: serde_json::Value) -> Option<settings::Settings> {
    let now = settings::set(&key, &value)?;
    let _ = app.emit("settings", &now);
    if matches!(key.as_str(), "autohide" | "fill" | "blend" | "size") && SHOWN.load(SeqCst) {
        on_main(&app, |h| place_bars(h, true));
    }
    if key == "glass" {
        let tint = now.tint();
        on_main(&app, move |h| {
            // Bars and open popups; the settings window has its own (Mica) backdrop.
            for (label, window) in h.webview_windows() {
                if let (true, Ok(hwnd)) = (label != SETTINGS, window.hwnd()) {
                    win::set_glass(HWND(hwnd.0), tint);
                }
            }
        });
    }
    if key == "all_monitors" {
        push_apps(&app);
    }
    if key == "show_codex" && now.show_codex {
        let h = app.clone();
        std::thread::spawn(move || fetch_codex(&h));
    }
    Some(now)
}

/// The pinned apps' new order (app ids, left to right) after a drag in one bar; every bar gets it.
/// Pane's own order only: Windows' taskbar pins are not touched.
#[tauri::command]
async fn set_pin_order(app: AppHandle, order: Vec<String>) {
    settings::set_pin_order(order.into_iter().filter(|id| id.len() <= 2048).take(1024).collect());
    push_apps(&app);
}

#[tauri::command]
async fn open_volume_mixer() {
    win::open("ms-settings:apps-volume");
}

#[tauri::command]
fn open_task_manager() {
    win::press(&[VK_CONTROL, VK_SHIFT, VK_ESCAPE]);
}

#[tauri::command]
fn open_widgets() {
    win::press(&[VK_LWIN, VIRTUAL_KEY(b'W' as u16)]);
}

#[tauri::command]
async fn get_tray() -> Vec<tray::TrayIcon> {
    tray::list()
}

#[tauri::command]
fn tray_click(id: String, kind: tray::Click, x: i32, y: i32) {
    tray::click(&id, kind, x, y);
}

/// A hidden glass popup (tray overflow, Pane menu, Docker) for a bar. It loads the same page, which tells itself
/// apart by the label. Built without focus: it takes the focus when it is shown, so a focus change while its
/// page is still loading can't count as "clicked away" and close it before it ever showed.
fn popup(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    let popup = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Pane")
        .additional_browser_args(browser_args())
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .build()?;
    win::set_glass(HWND(popup.hwnd()?.0), settings::get().tint());
    Ok(popup)
}

/// Opens the "hidden icons" popup above the arrow of the bar that asks. `client_x` is the arrow's
/// centre in CSS pixels inside that bar.
#[tauri::command]
async fn open_overflow(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    let label = format!("overflow-{}", window.label());
    let popup = match app.get_webview_window(&label) {
        Some(popup) => popup,
        None => popup(&app, &label)?,
    };
    popup_opened(&label);
    // Grid of 40 px cells, 5 per row, 8 px padding (same as the CSS).
    let count = tray::list().iter().filter(|t| !t.promoted).count().max(1);
    let (cols, rows) = (count.min(5) as f64, count.div_ceil(5) as f64);
    let scale = window.scale_factor()?;
    let (w, h) = ((cols * 40.0 + 18.0) * scale, (rows * 40.0 + 18.0) * scale);
    let bar = window.outer_position()?;
    let x = bar.x as f64 + client_x * scale - w / 2.0;
    let y = bar.y as f64 - h - 8.0 * scale;
    popup.set_size(PhysicalSize::new(w.round() as u32, h.round() as u32))?;
    popup.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))?;
    popup.show()?;
    popup.set_focus()?;
    // A kept popup was hidden: its page plays the opening animation again.
    popup.emit_to(label.as_str(), "popup-open", ())
}

/// Where a self-sizing popup (Pane menu, Docker) opens: the click's x in CSS px inside its bar, by popup label.
static POPUP_X: Mutex<Option<HashMap<String, f64>>> = Mutex::new(None);

/// Opens (or reopens a kept) popup that sizes itself: its page measures itself and calls `fit_popup`, which
/// shows it. A kept page is told with "popup-open" to refresh and measure again.
async fn open_fitted(app: &AppHandle, label: String, client_x: f64) -> tauri::Result<()> {
    lock(&POPUP_X).get_or_insert_with(HashMap::new).insert(label.clone(), client_x);
    let result = match app.get_webview_window(&label) {
        // Open and on screen: just bring it forward.
        Some(open) if lock(&POPUPS_OPEN).contains(&label) && open.is_visible().unwrap_or(false) => return open.set_focus(),
        // Kept (or open but never shown): its page refreshes, measures itself and shows.
        Some(kept) => {
            popup_opened(&label);
            kept.emit_to(label.as_str(), "popup-open", ())
        }
        None => {
            popup_opened(&label);
            popup(app, &label).map(|_| ())
        }
    };
    if result.is_err() {
        lock(&POPUPS_OPEN).retain(|l| *l != label);
        return result;
    }
    // A page that never managed to show itself must not keep the bar up for good, nor swallow the next
    // right-click (0.9.0: an invisible menu stayed "open", so right-clicks only focused it).
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let Some(w) = app.get_webview_window(&label) else {
            lock(&POPUPS_OPEN).retain(|l| *l != label);
            return;
        };
        if lock(&POPUPS_OPEN).contains(&label) && !w.is_visible().unwrap_or(false) {
            win::log(&format!("popup {label} didn't show itself, closed"));
            lock(&POPUPS_OPEN).retain(|l| *l != label);
            let _ = w.destroy();
        }
    });
    Ok(())
}

/// Right-click on the bar: the Pane menu, an own popup so it stays open while settings are switched
/// (a native menu always closes on a click).
#[tauri::command]
async fn open_menu(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    open_fitted(&app, format!("menu-{}", window.label()), client_x).await
}

/// A click on the Docker widget: the containers popup above it.
#[tauri::command]
async fn open_docker(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    open_fitted(&app, format!("docker-{}", window.label()), client_x).await
}

#[tauri::command]
async fn open_egg(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    open_fitted(&app, format!("egg-{}", window.label()), client_x).await
}

/// `egg.txt` in Pane's data folder: one line per rule, `#` lines and blank lines skipped. The file is the
/// reader's own and is never part of Pane (nothing is shipped or committed with it); without it the page
/// shows its own lines. A first line starting with `# title:` replaces the headline.
#[tauri::command]
async fn egg_rules() -> Option<(String, Vec<String>)> {
    let text = std::fs::read_to_string(win::data_dir().join("egg.txt")).ok()?;
    let mut title = String::new();
    let mut rules = Vec::new();
    for line in text.lines().map(str::trim) {
        match line.strip_prefix("# title:") {
            Some(t) => title = t.trim().to_string(),
            None if line.is_empty() || line.starts_with('#') => {}
            None => rules.push(line.to_string()),
        }
    }
    (!rules.is_empty()).then_some((title, rules))
}

/// Creates `egg.txt` with a short header if it isn't there and opens it in the editor.
#[tauri::command]
async fn open_egg_file() {
    let path = win::data_dir().join("egg.txt");
    if !path.exists() {
        let _ = std::fs::create_dir_all(win::data_dir());
        let header = "# One rule per line. Lines starting with # are ignored.\r\n\
                      # A line like \"# title: ...\" replaces the headline.\r\n\
                      # This file stays on this PC: it is not part of Pane and is never committed.\r\n\r\n";
        let _ = std::fs::write(&path, header);
    }
    win::open(&path.to_string_lossy());
}

/// A popup page measured itself (CSS px): put it above its bar at the click, inside the monitor, and show it.
#[tauri::command]
async fn fit_popup(app: AppHandle, window: WebviewWindow, width: f64, height: f64, centered: bool) -> tauri::Result<()> {
    let label = window.label().to_string();
    let Some(bar_label) = label.split_once('-').map(|(_, b)| b.to_string()) else { return Ok(()) };
    let monitor = bars(&app).iter().find(|b| b.label == bar_label).map(|b| b.monitor);
    // Its bar is gone (monitors changed), or it was closed meanwhile: don't show it.
    let (Some(bar), Some(monitor)) = (app.get_webview_window(&bar_label), monitor) else {
        close_popup(window);
        return Ok(());
    };
    if !lock(&POPUPS_OPEN).contains(&label) {
        return Ok(());
    }
    let scale = bar.scale_factor()?;
    let at = bar.outer_position()?;
    let gap = (8.0 * scale).round() as i32;
    // Never taller than the space above the bar: the list scrolls instead of running off the screen.
    let room = at.y - gap - monitor.top - gap;
    let (w, h) = ((width * scale).round() as i32, ((height * scale).round() as i32).min(room));
    let client_x = lock(&POPUP_X).as_ref().and_then(|m| m.get(&label).copied()).unwrap_or(0.0);
    let left = at.x + (client_x * scale).round() as i32 - if centered { w / 2 } else { 0 };
    let x = left.min(monitor.right - w).max(monitor.left);
    let y = at.y - h - gap;
    window.set_size(PhysicalSize::new(w as u32, h as u32))?;
    window.set_position(PhysicalPosition::new(x, y))?;
    window.show()?;
    window.set_focus()
}

/// Closes a popup (tray overflow, Pane menu, Docker) or the settings window (Escape). Popups are hidden and
/// kept for a quick reopen (see `POPUPS_KEPT`); the settings window and, in "Low memory mode", popups are destroyed.
#[tauri::command]
fn close_popup(window: WebviewWindow) {
    let label = window.label().to_string();
    let was_open = {
        let mut open = lock(&POPUPS_OPEN);
        let was = open.contains(&label);
        open.retain(|l| *l != label);
        was
    };
    if label == SETTINGS || settings::get().low_memory {
        let _ = window.destroy();
        return;
    }
    let _ = window.hide();
    if was_open {
        lock(&POPUPS_KEPT).push((label.clone(), Instant::now()));
        let _ = window.emit_to(label.as_str(), "popup-closed", ());
    }
}

const SETTINGS: &str = "settings";

/// "More settings…": the settings window, a normal window with a title bar, centred on the monitor of the bar
/// whose menu asked. One at a time (focused if already open); destroyed when closed, like the popups.
#[tauri::command]
async fn open_settings(app: AppHandle, window: WebviewWindow) -> tauri::Result<()> {
    if let Some(open) = app.get_webview_window(SETTINGS) {
        let _ = open.unminimize();
        return open.set_focus();
    }
    let bar_label = window.label().strip_prefix("menu-").unwrap_or(window.label()).to_string();
    let place = {
        let bars = bars(&app);
        bars.iter().find(|b| b.label == bar_label).or(bars.first()).map(|b| (b.monitor, b.scale))
    };
    let settings = WebviewWindowBuilder::new(&app, SETTINGS, WebviewUrl::App("index.html".into()))
        .title("Pane settings")
        .additional_browser_args(browser_args())
        .transparent(true)
        // Windows 11's own glass for normal windows; dark like the bar.
        .effects(tauri::utils::config::WindowEffectsConfig {
            effects: vec![tauri::window::Effect::Mica],
            ..Default::default()
        })
        .theme(Some(tauri::Theme::Dark))
        .min_inner_size(360.0, 320.0)
        .visible(false)
        .build()?;
    if let Some((m, scale)) = place {
        // 440 x 680 logical px, at most 90 % of the monitor.
        let w = ((440.0 * scale) as i32).min((m.right - m.left) * 9 / 10);
        let h = ((680.0 * scale) as i32).min((m.bottom - m.top) * 9 / 10);
        // Position first: moving onto a monitor with another DPI rescales the window, the size then fixes it.
        settings.set_position(PhysicalPosition::new(m.left + (m.right - m.left - w) / 2, m.top + (m.bottom - m.top - h) / 2))?;
        settings.set_size(PhysicalSize::new(w as u32, h as u32))?;
    }
    settings.show()?;
    settings.set_focus()
}

/// "Start with Windows": an entry named "Pane" under HKCU ...\CurrentVersion\Run, which is also the row
/// Task Manager shows under "Startup apps". Switching that row off there writes Windows' own "disabled" mark
/// (…\Explorer\StartupApproved\Run), which this reads too, so Pane's switch says what Task Manager says;
/// turning it on here clears that mark again (both in `tauri-plugin-autostart`).
#[tauri::command]
async fn autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// New installs start with "Start with Windows" on, so Pane is there after a restart and has its row in Task
/// Manager. Done once (`autostart_set`): after that it is the user's switch, here or in Task Manager.
/// Never from a build folder, which would autostart a copy that is about to be overwritten.
fn autostart_first_run(app: &AppHandle) {
    if settings::get().autostart_set {
        return;
    }
    let from_build_folder = std::env::current_exe()
        .map(|p| p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
        .unwrap_or(true);
    if from_build_folder {
        return; // and the flag stays off, so the installed Pane still does this once
    }
    match app.autolaunch().enable() {
        Ok(()) => {
            settings::set("autostart_set", &serde_json::Value::Bool(true));
        }
        Err(e) => win::log(&format!("can't turn on start with Windows: {e}")),
    }
}

#[tauri::command]
async fn set_autostart(app: AppHandle, on: bool) {
    let launcher = app.autolaunch();
    let result = if on { launcher.enable() } else { launcher.disable() };
    if let Err(e) = result {
        crate::win::log(&format!("can't change autostart: {e}"));
    }
}

/// Manual check from the Pane menu: the new version, or None when Pane is up to date.
#[tauri::command]
async fn check_update(app: AppHandle) -> Option<String> {
    update::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let result = update::install(&app).await;
    if let Err(e) = &result {
        crate::win::log(&format!("update failed: {e}"));
    }
    result
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--watchdog") => {
            if let Some(pid) = args.get(2).and_then(|p| p.parse().ok()) {
                win::wait_for_process(pid);
            }
            return win::restore_taskbar(&win::data_dir());
        }
        // Used by the uninstaller.
        Some("--restore") => return win::restore_taskbar(&win::data_dir()),
        _ => {}
    }

    tauri::Builder::default()
        // Starting Pane again while it runs (hidden) shows it instead of opening a second copy.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| request(app, |_| true)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["ctrl+alt+p"])
                .expect("valid shortcut")
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        request(app, |shown| !shown);
                    }
                })
                .build(),
        )
        // "Start with Windows" (HKCU Run key, off until the user turns it on).
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Bars(Mutex::new(Vec::new())))
        .invoke_handler(tauri::generate_handler![
            open_start,
            open_notifications,
            open_quick_settings,
            toggle,
            quit,
            get_apps,
            get_status,
            activate,
            close_window,
            thumb_button,
            show_preview,
            hide_preview,
            get_preview,
            focus_window,
            launch,
            get_media,
            media_control,
            get_weather,
            open_widgets,
            open_volume_mixer,
            get_claude,
            claude_icon,
            open_claude_usage,
            open_task_manager,
            get_tray,
            tray_click,
            open_overflow,
            open_menu,
            close_popup,
            autostart,
            set_autostart,
            check_update,
            install_update,
            get_codex,
            open_codex_login,
            open_codex_usage,
            codex_logged_in,
            open_claude_login,
            get_settings,
            set_setting,
            set_pin_order,
            open_settings,
            fit_popup,
            warm_preview,
            preview_drawn,
            get_docker,
            docker_action,
            open_docker,
            open_docker_desktop,
            open_egg,
            egg_rules,
            open_egg_file,
            report_bug,
            open_log_folder,
            get_downloads,
            app_version
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            create_bars(&handle)?;
            start_watchdog();
            preview::start(handle.clone());
            // Before showing: the tray window points app badges at the window this creates.
            let h = handle.clone();
            apps::watch(move || {
                push_apps(&h);
                check_fullscreen(&h);
            });
            set_shown(&handle, true);
            watch_loop(handle.clone());
            status_loop(handle.clone());
            weather_loop(handle.clone());
            usage_loop(handle.clone());
            claude_loop(handle.clone());
            codex_loop(handle.clone());
            autohide_loop(handle.clone());
            tint_loop(handle.clone());
            docker_loop(handle.clone());
            let h = handle.clone();
            std::thread::spawn(move || autostart_first_run(&h));
            let h = handle.clone();
            agents::watch(move |agent| {
                if SHOWN.load(SeqCst) && settings::get().done_glow {
                    let _ = h.emit("agent-done", agent.name());
                }
            });
            update::watch(handle.clone());
            let m = handle.clone();
            media::watch(move || {
                let _ = m.emit("media", media::current());
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Pane failed to start")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                set_shown(app, false);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_memory_mode_only_adds_no_gpu() {
        assert!(!browser_args_for(false).contains("--disable-gpu"));
        assert!(browser_args_for(true).ends_with(" --disable-gpu"));
        assert!(browser_args_for(true).starts_with(&browser_args_for(false)), "the other options stay the same");
    }

    #[test]
    fn reserved_strip_matches_the_drawn_bar() {
        let m = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        // Floating: 6 px from the edges, the reserved strip ends 2 px above the bar.
        let (strip, bar) = layout(m, 48, 6, 2);
        assert_eq!((bar.left, bar.top, bar.right, bar.bottom), (6, 1386, 2554, 1434));
        assert_eq!((strip.left, strip.top, strip.right, strip.bottom), (0, 1384, 2560, 1440));
        // Fill: the bar is exactly the reserved strip.
        let (strip, bar) = layout(m, 48, 0, 0);
        assert_eq!((bar.left, bar.top, bar.right, bar.bottom), (0, 1392, 2560, 1440));
        assert_eq!((strip.top, strip.bottom), (bar.top, bar.bottom));
    }

    #[test]
    fn slides_start_and_end_in_place_with_a_small_overshoot() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // Up: from below to its place, passing it by a few % on the way.
        assert!(close(slide_offset(true, 0.0), 1.0) && close(slide_offset(true, 1.0), 0.0));
        let over = (0..=100).map(|i| slide_offset(true, i as f64 / 100.0)).fold(f64::MAX, f64::min);
        assert!(over < -0.03 && over > -0.08, "overshoot {over}");
        // Fast start: most of the way up in the first third.
        assert!(slide_offset(true, 0.33) < 0.25);
        // Down: a small lift first, then away.
        assert!(close(slide_offset(false, 0.0), 0.0) && close(slide_offset(false, 1.0), 1.0));
        let lift = (0..=100).map(|i| slide_offset(false, i as f64 / 100.0)).fold(f64::MAX, f64::min);
        assert!(lift < -0.01 && lift > -0.06, "lift {lift}");
        assert!(slide_offset(false, 0.9) > 0.6);
    }
}
