//! Windows glue: hide and restore the taskbar, reserve the screen edge (AppBar), send keys.

use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::fs;
use std::mem::size_of_val;
use windows::core::{s, w, HSTRING, PCWSTR};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE,
    ABS_AUTOHIDE, APPBARDATA, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNA, SW_SHOWNORMAL,
};

/// Pane's data folder, %LOCALAPPDATA%\org.gruening.pane (the same one the uninstaller offers to delete).
pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("org.gruening.pane")
}

/// Holds the taskbar's auto-hide setting from before Pane changed it. Exists only while Pane hides the taskbar.
fn state_file(dir: &Path) -> PathBuf {
    dir.join("taskbar-autohide.txt")
}

fn read_saved(dir: &Path) -> Option<u32> {
    parse_state(&fs::read_to_string(state_file(dir)).ok()?)
}

fn parse_state(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

/// The setting to give back later. A value left over from a run that didn't clean up wins:
/// the current setting is then Pane's own (auto-hide on), not the user's.
fn setting_to_restore(saved: Option<u32>, current: u32) -> u32 {
    saved.unwrap_or(current)
}

fn appbar(hwnd: HWND) -> APPBARDATA {
    APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() }
}

/// Explorer's main taskbar window; skips Pane's own `Shell_TrayWnd` (the tray icon host in tray.rs).
pub fn tray() -> Option<HWND> {
    let me = std::process::id();
    let mut prev = None;
    while let Ok(h) = unsafe { FindWindowExW(None, prev, w!("Shell_TrayWnd"), PCWSTR::null()) } {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if pid != me {
            return Some(h);
        }
        prev = Some(h);
    }
    None
}

/// Changes when Explorer restarts (new taskbar window).
pub fn tray_id() -> isize {
    tray().map_or(0, |h| h.0 as isize)
}

/// The main taskbar plus the ones on other monitors.
fn taskbars() -> Vec<HWND> {
    let mut all: Vec<HWND> = tray().into_iter().collect();
    let mut prev = None;
    while let Ok(h) = unsafe { FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), PCWSTR::null()) } {
        all.push(h);
        prev = Some(h);
    }
    all
}

fn autohide_setting() -> u32 {
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut appbar(HWND::default())) as u32 }
}

fn set_autohide_setting(state: u32) {
    let Some(t) = tray() else { return };
    let mut data = appbar(t);
    data.lParam = LPARAM(state as isize);
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
}

/// Hides the taskbar. Auto-hide is turned on so the taskbar frees its screen edge for Pane;
/// the old setting is written to disk first so it can always be given back.
pub fn hide_taskbar(dir: &Path) {
    let original = setting_to_restore(read_saved(dir), autohide_setting());
    let saved = fs::create_dir_all(dir).and_then(|_| fs::write(state_file(dir), original.to_string()));
    match saved {
        Ok(()) => set_autohide_setting(original | ABS_AUTOHIDE),
        // Without the file the setting couldn't be restored, so leave it alone and only hide the windows.
        Err(e) => eprintln!("pane: can't save taskbar setting, leaving auto-hide unchanged: {e}"),
    }
    keep_hidden();
}

/// Hides the taskbar windows again if Explorer showed them.
pub fn keep_hidden() {
    for t in taskbars() {
        unsafe {
            if IsWindowVisible(t).as_bool() {
                let _ = ShowWindow(t, SW_HIDE);
            }
        }
    }
}

/// Brings the taskbar back as it was. Safe to call any number of times.
pub fn restore_taskbar(dir: &Path) {
    if let Some(original) = read_saved(dir) {
        set_autohide_setting(original);
        let _ = fs::remove_file(state_file(dir));
    }
    for t in taskbars() {
        unsafe {
            let _ = ShowWindow(t, SW_SHOWNA);
        }
    }
}

/// Reserves `height` px at the bottom of `monitor` for `hwnd`, so maximized windows stop above it.
/// Returns the rect the bar should take.
// shortcut: no AppBar notifications (uCallbackMessage = 0), so fullscreen apps aren't noticed yet; Phase 2.
pub fn reserve_edge(hwnd: HWND, monitor: RECT, height: i32) -> RECT {
    let mut data = appbar(hwnd);
    data.uEdge = ABE_BOTTOM;
    data.rc = monitor;
    unsafe {
        SHAppBarMessage(ABM_NEW, &mut data); // false if already registered, which is fine
        SHAppBarMessage(ABM_QUERYPOS, &mut data);
        data.rc.top = data.rc.bottom - height;
        SHAppBarMessage(ABM_SETPOS, &mut data);
    }
    data.rc
}

pub fn release_edge(hwnd: HWND) {
    unsafe { SHAppBarMessage(ABM_REMOVE, &mut appbar(hwnd)) };
}

/// Blurred, tinted glass behind a window that stays on when the window isn't focused.
/// Windows 11's own Mica/Acrylic backdrops turn flat grey on inactive windows, and Pane's bar is never
/// the active window, so this uses the older accent policy (the same call TranslucentTB uses).
/// `tint` is 0xAABBGGRR: alpha = how much tint covers the blur.
// shortcut: SetWindowCompositionAttribute is undocumented; if Windows drops it the bar is just transparent.
pub fn set_glass(hwnd: HWND, tint: u32) {
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Attribute {
        kind: u32,
        data: *mut std::ffi::c_void,
        size: usize,
    }
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
    const WCA_ACCENT_POLICY: u32 = 19;
    type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut Attribute) -> i32;
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let set: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut accent = AccentPolicy { state: ACCENT_ENABLE_ACRYLICBLURBEHIND, flags: 0, gradient: tint, animation: 0 };
        let mut attribute = Attribute {
            kind: WCA_ACCENT_POLICY,
            data: &mut accent as *mut _ as _,
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut attribute);
        // Windows 11 rounded corners also for borderless windows; they clip the glass too.
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, size_of_val(&round) as u32);
    }
}

/// Opens a file, app or URI like double-clicking it, e.g. "ms-settings:apps-volume".
pub fn open(target: &str) {
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Presses the keys together and releases them in reverse order, e.g. Win+N.
pub fn press(keys: &[VIRTUAL_KEY]) {
    let input = |vk, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: flags, ..Default::default() } },
    };
    let all: Vec<INPUT> = keys
        .iter()
        .map(|&k| input(k, KEYBD_EVENT_FLAGS(0)))
        .chain(keys.iter().rev().map(|&k| input(k, KEYEVENTF_KEYUP)))
        .collect();
    unsafe { SendInput(&all, size_of::<INPUT>() as i32) };
}

/// True when the foreground window covers the whole `monitor` (a game or video in fullscreen).
/// A maximized window doesn't count: it stops at the work area above Pane.
pub fn fullscreen_on(monitor: RECT) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        // The desktop (wallpaper) window covers the monitor too.
        let mut class = [0u16; 32];
        let len = GetClassNameW(fg, &mut class).max(0) as usize;
        if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW") {
            return false;
        }
        let mut rect = RECT::default();
        GetWindowRect(fg, &mut rect).is_ok() && covers(rect, monitor)
    }
}

fn covers(r: RECT, m: RECT) -> bool {
    r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
}

/// Blocks until process `pid` has ended (returns at once if it is already gone).
pub fn wait_for_process(pid: u32) {
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, INFINITE);
            let _ = CloseHandle(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftover_setting_wins_over_current() {
        // Earlier run crashed with auto-hide forced on (3); the user's real setting was 2.
        assert_eq!(setting_to_restore(Some(2), 3), 2);
        assert_eq!(setting_to_restore(None, 2), 2);
    }

    #[test]
    fn fullscreen_means_covering_the_whole_monitor() {
        let monitor = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        // Borderless fullscreen game, and one on the second monitor (left of the first).
        assert!(covers(RECT { left: 0, top: 0, right: 2560, bottom: 1440 }, monitor));
        assert!(!covers(RECT { left: -2560, top: 0, right: 0, bottom: 1440 }, monitor));
        // Maximized window: overhangs 8 px but stops above Pane's 48 px edge.
        assert!(!covers(RECT { left: -8, top: -8, right: 2568, bottom: 1400 }, monitor));
    }

    #[test]
    fn state_file_parsing() {
        assert_eq!(parse_state("2\r\n"), Some(2));
        assert_eq!(parse_state("garbage"), None);
    }
}
