// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(dead_code)] // wired into the UI next
mod apps;
mod win;

use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::Duration;
use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::ShortcutState;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_LWIN};

/// Bar height in logical pixels, the same as the Windows 11 taskbar.
const BAR_HEIGHT: f64 = 48.0;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// True while Pane is shown and the Windows taskbar is hidden. Changed only on the main thread.
static SHOWN: AtomicBool = AtomicBool::new(false);

struct Bar {
    label: String,
    monitor: RECT,
    height: i32,
}
struct Bars(Vec<Bar>);

/// Shows or hides Pane and swaps the Windows taskbar the other way. Main thread only.
fn set_shown(app: &AppHandle, show: bool) {
    if SHOWN.swap(show, SeqCst) == show {
        return;
    }
    let dir = win::data_dir();
    if show {
        win::hide_taskbar(&dir);
        place_bars(app, true);
        // Explorer frees the taskbar's screen edge a moment after auto-hide turns on; place the bars again then.
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                if SHOWN.load(SeqCst) {
                    place_bars(&h, true);
                }
            });
        });
    } else {
        place_bars(app, false);
        win::restore_taskbar(&dir);
    }
}

/// Runs `set_shown` on the main thread, where window calls can't deadlock.
fn request(app: &AppHandle, show: impl FnOnce(bool) -> bool + Send + 'static) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || set_shown(&handle, show(SHOWN.load(SeqCst))));
}

fn place_bars(app: &AppHandle, show: bool) {
    for bar in &app.state::<Bars>().0 {
        let Some(window) = app.get_webview_window(&bar.label) else { continue };
        let Ok(hwnd) = window.hwnd() else { continue };
        let hwnd = HWND(hwnd.0);
        if show {
            let rc = win::reserve_edge(hwnd, bar.monitor, bar.height);
            let _ = window.set_position(PhysicalPosition::new(rc.left, rc.top));
            let _ = window.set_size(PhysicalSize::new((rc.right - rc.left) as u32, bar.height as u32));
            let _ = window.show();
        } else {
            win::release_edge(hwnd);
            let _ = window.hide();
        }
    }
}

// shortcut: one bar per monitor found at start; plugging monitors in/out needs a restart until Phase 2.
fn create_bars(app: &tauri::App) -> tauri::Result<()> {
    let mut bars = Vec::new();
    for (i, monitor) in app.available_monitors()?.iter().enumerate() {
        let label = format!("bar-{i}");
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
        let (pos, size) = (monitor.position(), monitor.size());
        bars.push(Bar {
            label,
            monitor: RECT { left: pos.x, top: pos.y, right: pos.x + size.width as i32, bottom: pos.y + size.height as i32 },
            height: (BAR_HEIGHT * monitor.scale_factor()).round() as i32,
        });
    }
    app.manage(Bars(bars));
    Ok(())
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

/// Explorer sometimes shows its taskbar again, and after an Explorer restart it forgets Pane's screen edges.
// shortcut: checks once a second instead of listening for the TaskbarCreated message; fine until Phase 3 adds the tray window.
fn keep_taskbar_hidden(app: AppHandle) {
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
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || place_bars(&handle, true));
            }
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
fn toggle(app: AppHandle) {
    request(&app, |shown| !shown);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0); // RunEvent::Exit below brings the taskbar back
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
        .invoke_handler(tauri::generate_handler![open_start, open_notifications, toggle, quit])
        .setup(|app| {
            create_bars(app)?;
            start_watchdog();
            set_shown(app.handle(), true);
            keep_taskbar_hidden(app.handle().clone());
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
