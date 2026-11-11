// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
#[allow(dead_code)]
mod claude;
mod http;
mod media;
mod status;
mod tray;
mod usage;
mod weather;
mod win;

use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::ShortcutState;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_SHIFT};

/// Bar height in logical pixels, the same as the Windows 11 taskbar.
const BAR_HEIGHT: f64 = 48.0;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// True while Pane is shown and the Windows taskbar is hidden. Changed only on the main thread.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Makes window labels unique when the bars are rebuilt after a monitor change.
static GENERATION: AtomicUsize = AtomicUsize::new(0);

struct Bar {
    label: String,
    monitor: RECT,
    height: i32,
    /// Hidden because a fullscreen app (game, video) is in front on this monitor.
    fullscreen: bool,
}
struct Bars(Mutex<Vec<Bar>>);

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

fn request(app: &AppHandle, show: impl FnOnce(bool) -> bool + Send + 'static) {
    on_main(app, move |h| set_shown(h, show(SHOWN.load(SeqCst))));
}

/// Main thread only.
fn place_bars(app: &AppHandle, show: bool) {
    for bar in bars(app).iter() {
        let Some(window) = app.get_webview_window(&bar.label) else { continue };
        let Ok(hwnd) = window.hwnd() else { continue };
        let hwnd = HWND(hwnd.0);
        if show {
            let rc = win::reserve_edge(hwnd, bar.monitor, bar.height);
            let _ = window.set_position(PhysicalPosition::new(rc.left, rc.top));
            let _ = window.set_size(PhysicalSize::new((rc.right - rc.left) as u32, bar.height as u32));
            if !bar.fullscreen {
                let _ = window.show();
            }
        } else {
            win::release_edge(hwnd);
            let _ = window.hide();
        }
    }
}

fn monitor_rects(app: &AppHandle) -> Vec<(RECT, i32)> {
    let monitors = app.available_monitors().unwrap_or_default();
    monitors
        .iter()
        .map(|m| {
            let (pos, size) = (m.position(), m.size());
            let rect = RECT { left: pos.x, top: pos.y, right: pos.x + size.width as i32, bottom: pos.y + size.height as i32 };
            (rect, (BAR_HEIGHT * m.scale_factor()).round() as i32)
        })
        .collect()
}

/// One bar window per monitor. Main thread only.
fn create_bars(app: &AppHandle) -> tauri::Result<()> {
    let generation = GENERATION.fetch_add(1, SeqCst);
    let mut new = Vec::new();
    for (i, (monitor, height)) in monitor_rects(app).into_iter().enumerate() {
        let label = format!("bar-{generation}-{i}");
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("Pane")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            // Dark, see-through acrylic tint (the default tint is a flat light grey).
            .effects(EffectsBuilder::new().effect(Effect::Acrylic).color(Color(16, 18, 26, 140)).build())
            .build()?;
        new.push(Bar { label, monitor, height, fullscreen: false });
    }
    *bars(app) = new;
    Ok(())
}

/// Monitors were plugged in/out or changed resolution: replace all bars. Main thread only.
fn rebuild_bars(app: &AppHandle) {
    let old: Vec<String> = bars(app).iter().map(|b| b.label.clone()).collect();
    for label in old {
        if let Some(window) = app.get_webview_window(&label) {
            if let Ok(hwnd) = window.hwnd() {
                win::release_edge(HWND(hwnd.0));
            }
            let _ = window.destroy();
        }
        if let Some(popup) = app.get_webview_window(&format!("overflow-{label}")) {
            let _ = popup.destroy();
        }
    }
    if let Err(e) = create_bars(app) {
        eprintln!("pane: can't rebuild bars: {e}");
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
    let changes: Vec<(String, bool)> = bars(app)
        .iter_mut()
        .filter_map(|bar| {
            let fullscreen = win::fullscreen_on(bar.monitor);
            (fullscreen != bar.fullscreen).then(|| {
                bar.fullscreen = fullscreen;
                (bar.label.clone(), fullscreen)
            })
        })
        .collect();
    for (label, fullscreen) in changes {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = if fullscreen { window.hide() } else { window.show() };
        }
    }
}

/// Sends each bar the apps of its own monitor.
fn push_apps(app: &AppHandle) {
    let targets: Vec<(String, RECT)> = bars(app).iter().map(|b| (b.label.clone(), b.monitor)).collect();
    for (label, monitor) in targets {
        let _ = app.emit_to(label.as_str(), "apps", apps::list(monitor));
    }
}

/// CPU / GPU / RAM and network speed once a second, only while Pane is shown.
fn usage_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        if SHOWN.load(SeqCst) {
            let _ = app.emit("usage", usage::sample());
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

/// Last weather (None: no location, offline, or not fetched yet).
static WEATHER: Mutex<Option<weather::Weather>> = Mutex::new(None);

/// Fetches the weather now and every 30 minutes (5 minutes after a failure) and sends it to the bars.
fn weather_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        let result = weather::fetch();
        if let Err(e) = &result {
            eprintln!("pane: no weather: {e}");
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
    let _ = app.emit("tray", tray::list());
}

/// Starts `Pane.exe --watchdog <pid>`: it waits until this process ends (also by crash or Task Manager)
/// and then brings the taskbar back.
fn start_watchdog() {
    let Ok(exe) = std::env::current_exe() else { return };
    let pid = std::process::id().to_string();
    if let Err(e) = std::process::Command::new(exe).args(["--watchdog", &pid]).creation_flags(CREATE_NO_WINDOW).spawn() {
        eprintln!("pane: watchdog didn't start: {e}");
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
            let known: Vec<(RECT, i32)> = bars(&app).iter().map(|b| (b.monitor, b.height)).collect();
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
    monitor.map(apps::list).unwrap_or_default()
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

/// Opens the "hidden icons" popup above the arrow of the bar that asks. `client_x` is the arrow's
/// centre in CSS pixels inside that bar.
#[tauri::command]
async fn open_overflow(app: AppHandle, window: WebviewWindow, client_x: f64) -> tauri::Result<()> {
    let label = format!("overflow-{}", window.label());
    let popup = match app.get_webview_window(&label) {
        Some(popup) => popup,
        None => WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
            .title("Pane")
            .decorations(false)
            .transparent(true)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible(false)
            .effects(EffectsBuilder::new().effect(Effect::Acrylic).color(Color(16, 18, 26, 170)).build())
            .build()?,
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
    popup.set_focus()
}

#[tauri::command]
fn close_overflow(window: WebviewWindow) {
    let _ = window.hide();
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
        eprintln!("pane: can't change autostart: {e}");
    }
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
            launch,
            get_media,
            media_control,
            get_weather,
            open_widgets,
            open_volume_mixer,
            open_task_manager,
            get_tray,
            tray_click,
            open_overflow,
            close_overflow,
            autostart,
            set_autostart
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            create_bars(&handle)?;
            start_watchdog();
            set_shown(&handle, true);
            watch_loop(handle.clone());
            status_loop(handle.clone());
            weather_loop(handle.clone());
            usage_loop(handle.clone());
            let m = handle.clone();
            media::watch(move || {
                let _ = m.emit("media", media::current());
            });
            let h = handle.clone();
            apps::watch(move || {
                push_apps(&h);
                check_fullscreen(&h);
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
