//! Windows glue: hide and restore the taskbar, reserve the screen edge (AppBar), send keys.

use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::fs;
use std::mem::size_of_val;
use windows::core::{s, w, HSTRING, PCWSTR};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE,
    ABS_AUTOHIDE, APPBARDATA, DefSubclassProc, SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE, WINDOWPOS, WM_WINDOWPOSCHANGING};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, IsZoomed, ShowWindow, GWL_EXSTYLE, GWL_STYLE, SW_HIDE, SW_SHOWNA,
    SW_SHOWNORMAL, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};

/// Pane's data folder, %LOCALAPPDATA%\org.gruening.pane (the same one the uninstaller offers to delete).
pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("org.gruening.pane")
}

/// Appends a line to `pane.log` in the data folder (release builds have no console).
/// The file is started over once it passes 64 KB.
pub fn log(message: &str) {
    use std::io::Write;
    let path = data_dir().join("pane.log");
    let too_big = fs::metadata(&path).map_or(false, |m| m.len() > 64 * 1024);
    let file = fs::OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(&path);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut file) = file {
        let _ = writeln!(file, "{secs} {message}");
    }
}

/// Reads a JSON file, lets `change` edit it, and writes it back through a temp file + rename, so a crash
/// or a reader in between never sees half a file. Keys `change` doesn't touch stay as they were.
pub fn update_json(
    path: &Path,
    change: impl FnOnce(&mut serde_json::Value) -> Result<(), &'static str>,
) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|_| "can't read the login file")?;
    let mut root: serde_json::Value = serde_json::from_str(&text).map_err(|_| "login file isn't JSON")?;
    change(&mut root)?;
    let out = serde_json::to_string_pretty(&root).map_err(|_| "can't write the login file")?;
    let tmp = path.with_extension("pane-tmp");
    fs::write(&tmp, out).and_then(|_| fs::rename(&tmp, path)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("can't save the login file: {}", e.kind())
    })
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
        Err(e) => log(&format!("can't save taskbar setting, leaving auto-hide unchanged: {e}")),
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

/// Reserves `strip` (the bottom of a monitor) for `hwnd`, so maximized windows stop above it.
/// The strip is always taken as given: the shell's answer used to move the bar up when another bottom
/// AppBar (Explorer's hidden taskbar) still held space, e.g. after auto-hide was turned off again.
// shortcut: no AppBar notifications (uCallbackMessage = 0), so fullscreen apps aren't noticed yet; Phase 2.
pub fn reserve_edge(hwnd: HWND, strip: RECT) {
    let mut data = appbar(hwnd);
    data.uEdge = ABE_BOTTOM;
    data.rc = strip;
    unsafe {
        SHAppBarMessage(ABM_NEW, &mut data); // false if already registered, which is fine
        // The protocol asks first; the answer is ignored (Explorer's hidden taskbar still holds its
        // 48 px for a moment after auto-hide turns on, which would put the bar too high).
        SHAppBarMessage(ABM_QUERYPOS, &mut data);
        data.rc = strip;
        SHAppBarMessage(ABM_SETPOS, &mut data);
    }
}

/// Where each bar window must stay (hwnd, rect). Something outside Pane (seen 2026-11-12, most likely the
/// shell while the taskbar's auto-hide changes) moved the bar 50 px up for a moment, which is what
/// left the bar too high after auto-hide on â†’ off. `pin` makes the window refuse every other place.
static PINNED: std::sync::Mutex<Vec<(isize, RECT)>> = std::sync::Mutex::new(Vec::new());

/// Pins `hwnd` to `rc` (physical px), or unpins it with `None`. Main thread (the window's thread).
pub fn pin(hwnd: HWND, rc: Option<RECT>) {
    let mut pinned = PINNED.lock().unwrap_or_else(|e| e.into_inner());
    pinned.retain(|(h, _)| *h != hwnd.0 as isize);
    match rc {
        Some(rc) => pinned.push((hwnd.0 as isize, rc)),
        None => return,
    }
    drop(pinned);
    // Same proc and id again only updates the subclass, so calling this on every placement is fine.
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(keep_place), 1, 0);
    }
}

unsafe extern "system" fn keep_place(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    if msg == WM_WINDOWPOSCHANGING {
        let rc = PINNED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(h, _)| *h == hwnd.0 as isize).map(|p| p.1);
        if let Some(rc) = rc {
            let pos = &mut *(lp.0 as *mut WINDOWPOS);
            if !pos.flags.contains(SWP_NOMOVE) {
                (pos.x, pos.y) = (rc.left, rc.top);
            }
            if !pos.flags.contains(SWP_NOSIZE) {
                (pos.cx, pos.cy) = (rc.right - rc.left, rc.bottom - rc.top);
            }
        }
    }
    DefSubclassProc(hwnd, msg, wp, lp)
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
    }
    set_round(hwnd, true);
}

/// Windows 11 rounded corners also for borderless windows (they clip the glass too), or square ones.
pub fn set_round(hwnd: HWND, round: bool) {
    let corners = if round { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corners as *const _ as _, size_of_val(&corners) as u32);
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
/// A maximized window doesn't count, also not with auto-hide on, where no edge is reserved and it covers the monitor.
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
        // Screen overlays (Snipping Tool's Win+Shift+S, screen recorders) cover the monitor too, but only for a
        // moment: hiding the bar for them made it blink and left it out of the screenshot.
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(fg, GWL_EXSTYLE) as u32);
        if is_overlay(ex, &crate::apps::exe_of(pid)) {
            return false;
        }
        let mut rect = RECT::default();
        let style = WINDOW_STYLE(GetWindowLongPtrW(fg, GWL_STYLE) as u32);
        GetWindowRect(fg, &mut rect).is_ok() && is_fullscreen(rect, monitor, IsZoomed(fg).as_bool(), style)
    }
}

/// Covers the monitor and isn't just maximized. A maximized window keeps its frame (caption or resize border);
/// fullscreen apps drop it (browsers' F11 and video remove both, games use borderless popups).
fn is_fullscreen(rect: RECT, monitor: RECT, zoomed: bool, style: WINDOW_STYLE) -> bool {
    let framed = style.contains(WS_CAPTION) || style.contains(WS_THICKFRAME);
    covers(rect, monitor) && !(zoomed && framed)
}

/// Tool windows never are fullscreen apps; the Snipping Tool's overlay is known by its process.
fn is_overlay(ex: WINDOW_EX_STYLE, exe: &str) -> bool {
    const OVERLAYS: [&str; 4] = ["screenclippinghost.exe", "snippingtool.exe", "screensketch.exe", "shellexperiencehost.exe"];
    let name = exe.rsplit('\\').next().unwrap_or(exe);
    ex.contains(WS_EX_TOOLWINDOW) || OVERLAYS.contains(&name)
}

/// Shows a window without activating it, so the app the user works in keeps the keyboard focus.
pub fn show_inactive(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNA);
    }
}

/// Hides a bar. Use this, not Tauri's `hide()`, for windows shown with `show_inactive`: Tauri only hides a
/// window it showed itself (it still thinks it is hidden), so its `hide()` did nothing.
pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rc = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rc) }.ok().map(|_| rc)
}

/// Opens a console window running `command` (e.g. "codex login"); it stays open so the user sees the result.
pub fn run_in_terminal(command: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    let result = std::process::Command::new("cmd").args(["/k", command]).creation_flags(CREATE_NEW_CONSOLE).spawn();
    if let Err(e) = result {
        log(&format!("can't open a terminal: {e}"));
    }
}

/// The cursor's screen position in physical pixels.
pub fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
}

/// Colours of the screen row just above `y` from `left` to `right`, in `n` equal parts (each part's median,
/// so the 1 px borders between windows don't tint it). Reads 3 rows; None when the screen can't be read
/// (secure desktop, locked PC).
pub fn screen_colors(left: i32, right: i32, y: i32, n: usize) -> Option<Vec<[u8; 3]>> {
    let (w, h) = (right - left, 3);
    if w < n as i32 || n == 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), left, y - h, SRCCOPY).is_ok();
        SelectObject(mem, old);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let px = if copied { crate::apps::pixels(bmp) } else { None };
        let _ = DeleteObject(bmp.into());
        let (pw, ph, px) = px?;
        Some(segment_medians(&px, pw as usize, ph as usize, n))
    }
}

/// Splits BGRA pixels into `n` column bands and returns each band's per-channel median as RGB.
fn segment_medians(px: &[u8], w: usize, h: usize, n: usize) -> Vec<[u8; 3]> {
    (0..n)
        .map(|i| {
            let (x0, x1) = (i * w / n, ((i + 1) * w / n).max(i * w / n + 1));
            let mut ch: [Vec<u8>; 3] = Default::default();
            for y in 0..h {
                for x in x0..x1.min(w) {
                    let p = &px[(y * w + x) * 4..];
                    ch[0].push(p[2]);
                    ch[1].push(p[1]);
                    ch[2].push(p[0]);
                }
            }
            ch.map(|mut c| {
                c.sort_unstable();
                c.get(c.len() / 2).copied().unwrap_or(0)
            })
        })
        .collect()
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
    fn maximized_window_over_whole_monitor_is_not_fullscreen() {
        let monitor = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        // Auto-hide: nothing reserved, so a maximized window overhangs the whole monitor.
        let maximized = RECT { left: -8, top: -8, right: 2568, bottom: 1448 };
        let framed = WS_CAPTION | WS_THICKFRAME;
        assert!(!is_fullscreen(maximized, monitor, true, framed));
        // Browser F11 from a maximized window: still zoomed, but the frame is gone.
        assert!(is_fullscreen(monitor, monitor, true, WINDOW_STYLE(0)));
        // Borderless game, not zoomed.
        assert!(is_fullscreen(monitor, monitor, false, WINDOW_STYLE(0x8000_0000)));
        // A framed window the user dragged over the whole monitor isn't maximized: treated as fullscreen like before.
        assert!(is_fullscreen(maximized, monitor, false, framed));
    }

    #[test]
    fn snipping_overlay_is_not_fullscreen_app() {
        assert!(is_overlay(WINDOW_EX_STYLE(0), r"c:\windows\systemapps\x\screenclippinghost.exe"));
        assert!(is_overlay(WINDOW_EX_STYLE(0), r"c:\program files\windowsapps\snip\snippingtool.exe"));
        assert!(is_overlay(WS_EX_TOOLWINDOW, r"c:\tools\recorder.exe"));
        assert!(!is_overlay(WINDOW_EX_STYLE(0), r"c:\games\game.exe"));
    }

    #[test]
    fn colour_bands_ignore_thin_borders() {
        // 10 px wide, 3 rows: left half red with a white 1 px border column at x=2, right half blue.
        let mut px = Vec::new();
        for _ in 0..3 {
            for x in 0..10 {
                let bgra = if x == 2 { [255, 255, 255, 255] } else if x < 5 { [0, 0, 200, 255] } else { [200, 0, 0, 255] };
                px.extend_from_slice(&bgra);
            }
        }
        assert_eq!(segment_medians(&px, 10, 3, 2), vec![[200, 0, 0], [0, 0, 200]]);
    }

    #[test]
    fn json_update_keeps_other_keys() {
        let dir = std::env::temp_dir().join(format!("pane-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("creds.json");
        fs::write(&file, r#"{"other":{"keep":1},"login":{"token":"old","extra":true}}"#).unwrap();
        update_json(&file, |v| {
            v["login"]["token"] = "new".into();
            Ok(())
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["login"]["token"], "new");
        assert_eq!(v["login"]["extra"], true);
        assert_eq!(v["other"]["keep"], 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn state_file_parsing() {
        assert_eq!(parse_state("2\r\n"), Some(2));
        assert_eq!(parse_state("garbage"), None);
    }
}
