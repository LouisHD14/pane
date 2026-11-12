// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod claude;
mod codex;
mod colors;
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

/// Auto-hide: keeps the bar up while its menu or the hidden-icons popup is open.
static HOLD: AtomicBool = AtomicBool::new(false);

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
/// monitor's bottom edge in `slide_ms` (default 120 ms), no bounce (decided with the user 2026-11-12).
/// Up: fast, then settling (ease-out); down: slow start, then away (ease-in). Ease-out on the way down rushed off and left a thin strip lingering at
/// the edge, which looked like the bar disappeared weirdly. Hidden at the end of
/// a slide down and parked back at its place, so everything else still finds it where it belongs.
fn slide(app: &AppHandle, label: String, rc: RECT, below: i32, show: bool) {
    let frames = slide_frames(settings::get().slide_ms);
    let id = SLIDE_ID.fetch_add(1, SeqCst);
    SLIDES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(label.clone(), id);
    let app = app.clone();
    std::thread::spawn(move || {
        let (from, to) = if show { (below, rc.top) } else { (rc.top, below) };
        for i in 0..=frames {
            let t = i as f64 / frames as f64;
            let eased = if show { 1.0 - (1.0 - t).powi(3) } else { t.powi(3) };
            let top = from + ((to - from) as f64 * eased).round() as i32;
            let label = label.clone();
            on_main(&app, move |h| {
                let current = SLIDES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|s| s.get(&label).copied());
                if current != Some(id) {
                    return; // a newer slide or place_bars took over
                }
                let Some(hwnd) = h.get_webview_window(&label).and_then(|w| w.hwnd().ok()).map(|h| HWND(h.0)) else { return };
                win::pin(hwnd, Some(RECT { top, bottom: top + rc.bottom - rc.top, ..rc }));
                win::move_to(hwnd, rc.left, top);
                if show && i == 0 {
                    win::show_inactive(hwnd);
                }
                if !show && i == frames {
                    win::hide(hwnd);
                    win::pin(hwnd, Some(rc));
                    win::move_to(hwnd, rc.left, rc.top);
                }
            });
            std::thread::sleep(Duration::from_millis(FRAME_MS));
        }
    });
}

const FRAME_MS: u64 = 15;

/// Steps of a slide that takes `ms` (at least one: 0 ms jumps straight there).
fn slide_frames(ms: u32) -> i32 {
    ((ms as f64 / FRAME_MS as f64).round() as i32).max(1)
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
        for kind in ["overflow", "menu"] {
            if let Some(popup) = app.get_webview_window(&format!("{kind}-{label}")) {
                // Its page can't call close_popup any more, so let the bar auto-hide again here.
                HOLD.store(false, SeqCst);
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
        let hold = HOLD.load(SeqCst) || preview::is_open();
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
            } else if over || hold || bar.fullscreen {
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

/// Last weather (None: no location, offline, or not fetched yet).
static WEATHER: Mutex<Option<weather::Weather>> = Mutex::new(None);

/// Fetches the weather now and every 30 minutes (5 minutes after a failure) and sends it to the bars.
fn weather_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        let result = weather::fetch();
        if let Err(e) = &result {
            crate::win::log(&format!("no weather: {e}"));
        }
        let wait = if result.is_ok() { 30 } else { 5 };
        let now = result.ok();
        *WEATHER.lock().unwrap_or_else(|e| e.into_inner()) = now.clone();
        let _ = app.emit("weather", now);
        std::thread::sleep(Duration::from_secs(wait * 60));
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
    let item = LAST_APPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|all| {
        all.get(window.label()).and_then(|list| list.iter().find(|a| a.id == id).cloned())
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
    preview::ready(window.label())
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
fn get_weather() -> Option<weather::Weather> {
    WEATHER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
fn get_claude() -> Option<claude::ClaudeUsage> {
    CLAUDE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The Claude logo, taken from the Claude desktop app installed on this PC ("" if it isn't installed).
// shortcut: fixed package name of the Microsoft Store / MSIX Claude app; other installs fall back to the drawn glyph.
#[tauri::command]
async fn claude_icon() -> String {
    apps::icon_of("shell:AppsFolder\\Claude_pzs8sxrjxfjjc!Claude")
}

#[tauri::command]
fn open_claude_usage() {
    win::open("https://claude.ai/settings/usage");
}

#[tauri::command]
fn get_codex() -> Option<codex::CodexUsage> {
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
fn open_codex_usage() {
    win::open("https://chatgpt.com/codex/settings/usage");
}

#[tauri::command]
fn codex_logged_in() -> bool {
    codex::logged_in()
}

/// Opens a terminal with Claude Code's login, for when its stored login can't be refreshed any more.
#[tauri::command]
fn open_claude_login() {
    win::run_in_terminal("claude /login");
}

#[tauri::command]
fn get_settings() -> settings::Settings {
    settings::get()
}

/// Sets one setting (a switch's bool, a slider's number, the size's name; checked and clamped in settings.rs)
/// and applies it live.
#[tauri::command]
fn set_setting(app: AppHandle, key: String, value: serde_json::Value) -> Option<settings::Settings> {
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

/// Keeps an auto-hidden bar up while its right-click menu is open.
#[tauri::command]
fn hold_bar(on: bool) {
    HOLD.store(on, SeqCst);
}

#[tauri::command]
fn open_volume_mixer() {
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
fn get_tray() -> Vec<tray::TrayIcon> {
    tray::list()
}

#[tauri::command]
fn tray_click(id: String, kind: tray::Click, x: i32, y: i32) {
    tray::click(&id, kind, x, y);
}

/// A hidden glass popup (tray overflow, Pane menu) for a bar. It loads the same page, which tells itself
/// apart by the label.
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
    HOLD.store(true, SeqCst);
    popup.set_focus()
}

/// Where the Pane menu opens: the right-click's x in CSS px inside its bar (one menu at a time).
static MENU_X: Mutex<f64> = Mutex::new(0.0);

/// Right-click on the bar: the Pane menu, an own popup so it stays open while settings are switched
/// (a native menu always closes on a click). It shows itself once the page knows its size (`fit_menu`).
#[tauri::command]
async fn open_menu(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    let label = format!("menu-{}", window.label());
    // Normally gone already: it closes when it loses focus (and when it can't build itself, see main.ts).
    if let Some(open) = app.get_webview_window(&label) {
        return open.set_focus();
    }
    *MENU_X.lock().unwrap_or_else(|e| e.into_inner()) = client_x;
    HOLD.store(true, SeqCst);
    popup(&app, &label).map(|_| ()).inspect_err(|_| HOLD.store(false, SeqCst))
}

/// The menu page measured itself (CSS px): put it above its bar at the right-click, inside the monitor.
#[tauri::command]
async fn fit_menu(app: AppHandle, window: WebviewWindow, width: f64, height: f64) -> tauri::Result<()> {
    let Some(bar_label) = window.label().strip_prefix("menu-") else { return Ok(()) };
    let monitor = bars(&app).iter().find(|b| b.label == bar_label).map(|b| b.monitor);
    // Its bar is gone (monitors changed): close, so a hidden menu doesn't keep holding the bar up.
    let (Some(bar), Some(monitor)) = (app.get_webview_window(bar_label), monitor) else {
        close_popup(window);
        return Ok(());
    };
    let scale = bar.scale_factor()?;
    let at = bar.outer_position()?;
    let gap = (8.0 * scale).round() as i32;
    // Never taller than the space above the bar: the list scrolls instead of running off the screen.
    let room = at.y - gap - monitor.top - gap;
    let (w, h) = ((width * scale).round() as i32, ((height * scale).round() as i32).min(room));
    let client_x = *MENU_X.lock().unwrap_or_else(|e| e.into_inner());
    let x = (at.x + (client_x * scale).round() as i32).min(monitor.right - w).max(monitor.left);
    let y = at.y - h - gap;
    window.set_size(PhysicalSize::new(w as u32, h as u32))?;
    window.set_position(PhysicalPosition::new(x, y))?;
    window.show()?;
    window.set_focus()
}

/// Closes a popup (tray overflow or Pane menu) or the settings window (Escape).
#[tauri::command]
fn close_popup(window: WebviewWindow) {
    HOLD.store(false, SeqCst);
    // Destroyed, not hidden: a hidden popup keeps its ~30 MB page alive. Reopening builds it again (~0.1 s).
    let _ = window.destroy();
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

#[tauri::command]
fn autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) {
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
            fit_menu,
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
            hold_bar
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
    fn slide_keeps_todays_steps() {
        assert_eq!(slide_frames(120), 8); // the default: 8 frames x 15 ms, as before the setting
        assert_eq!(slide_frames(0), 1);
        assert_eq!(slide_frames(400), 27);
    }
}
