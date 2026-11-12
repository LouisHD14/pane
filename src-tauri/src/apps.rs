//! The taskbar's app icons: pinned apps + running windows, grouped by app, with click/close/launch and change events.

use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering::SeqCst};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use windows::core::{w, Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, PROPERTYKEY, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, GetObjectW, MonitorFromRect, ReleaseDC, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Storage::EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_FileDescription, PKEY_Link_TargetParsingPath};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    IShellItem2, IShellItemImageFactory, SHCreateItemFromIDList, SHCreateItemFromParsingName, SHLockShared,
    SHUnlockShared, ShellExecuteW, SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct AppItem {
    /// Stable key: lowercased exe path, or the AppUserModelID.
    pub id: String,
    pub name: String,
    /// "data:image/png;base64,..." (48x48), "" if none found.
    pub icon: String,
    pub pinned: bool,
    pub windows: Vec<WinItem>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct WinItem {
    pub hwnd: isize,
    pub title: String,
    pub active: bool,
    /// The app's overlay badge (ITaskbarList3::SetOverlayIcon, e.g. an unread count) as a PNG data URL, "" if none.
    pub badge: String,
    /// What the badge means ("3 unread messages"), "" if the app gave no text.
    pub badge_tip: String,
    /// The window flashes for attention (FlashWindowEx) and wasn't activated since.
    pub flashing: bool,
}

/// A taskbar pin, read from Explorer's pin list.
#[derive(Clone, Default)]
struct Pin {
    id: String,
    aumid: String,
    exe: String,
    name: String,
    icon: String,
    /// What `launch` runs: the .lnk path or "shell:AppsFolder\<aumid>".
    target: String,
}

/// A window that belongs on the taskbar.
#[derive(Clone, Default)]
struct Win {
    hwnd: isize,
    title: String,
    aumid: String,
    exe: String,
    active: bool,
    badge: Badge,
    flashing: bool,
    /// On the monitor being listed.
    here: bool,
}

impl Win {
    fn id(&self) -> String {
        if self.aumid.is_empty() { self.exe.clone() } else { self.aumid.clone() }
    }
}

const ICON_PX: i32 = 48;

/// Pinned apps first (in the user's Windows taskbar pin order), then running unpinned apps in order of first appearance.
/// Only windows on the monitor whose rcMonitor equals `monitor` are listed in `windows`; pinned apps appear on every
/// monitor, running unpinned apps only on monitors where they have windows.
pub fn list(monitor: RECT) -> Vec<AppItem> {
    com();
    let pins = pins();
    group(&pins, &windows(monitor), info)
}

/// Pins and windows -> taskbar items. `wins` come in first-seen order; `info` gives (name, icon) of an unpinned app.
fn group(pins: &[Pin], wins: &[Win], info: impl Fn(&Win) -> (String, String)) -> Vec<AppItem> {
    let item = |w: &Win| WinItem {
        hwnd: w.hwnd,
        title: w.title.clone(),
        active: w.active,
        badge: w.badge.png.clone(),
        badge_tip: w.badge.tip.clone(),
        flashing: w.flashing,
    };
    let pin_of = |w: &Win| pins.iter().position(|p| pin_matches(p, w));
    let mut items: Vec<AppItem> = pins
        .iter()
        .map(|p| AppItem { id: p.id.clone(), name: p.name.clone(), icon: p.icon.clone(), pinned: true, windows: vec![] })
        .collect();
    for w in wins {
        let at = match pin_of(w) {
            Some(i) => i,
            None => {
                let id = w.id();
                match items.iter().position(|a| !a.pinned && a.id == id) {
                    Some(i) => i,
                    None if !w.here => continue,
                    None => {
                        let (name, icon) = info(w);
                        let name = if name.is_empty() { w.title.clone() } else { name };
                        items.push(AppItem { id, name, icon, pinned: false, windows: vec![] });
                        items.len() - 1
                    }
                }
            }
        };
        if w.here {
            items[at].windows.push(item(w));
        }
    }
    items
}

fn pin_matches(p: &Pin, w: &Win) -> bool {
    if !p.aumid.is_empty() && p.aumid == w.aumid {
        return true;
    }
    // shortcut: Squirrel apps (Discord, ...) pin Update.exe, which starts app-<version>\<App>.exe next to it;
    // matched by folder. Reading the process's own AUMID would need the undocumented Explorer route.
    match p.exe.strip_suffix(r"\update.exe") {
        Some(dir) => w.exe.strip_prefix(dir).is_some_and(|rest| rest.starts_with(r"\app-")),
        None => !p.exe.is_empty() && p.exe == w.exe,
    }
}

/// Taskbar click on an app window: if it is the foreground window -> minimize; else restore if minimized and bring to front.
pub fn activate(hwnd: isize) {
    let h = HWND(hwnd as _);
    if foreground() == hwnd && !unsafe { IsIconic(h) }.as_bool() {
        unsafe {
            let _ = ShowWindow(h, SW_MINIMIZE);
        }
        return;
    }
    bring_to_front(hwnd);
}

/// Restores the window if minimized and puts it in front (a click on its preview: never minimizes).
pub fn bring_to_front(hwnd: isize) {
    let h = HWND(hwnd as _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if !SetForegroundWindow(h).as_bool() {
            // Windows only lets the foreground thread hand out focus; borrow its input state for the call.
            let fg = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let me = GetCurrentThreadId();
            let _ = AttachThreadInput(me, fg, true);
            let _ = SetForegroundWindow(h);
            let _ = AttachThreadInput(me, fg, false);
        }
    }
}

/// Asks the window to close, like its X button.
pub fn close(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Starts an app: a pin by its shortcut, else the exe path or AppUserModelID itself.
pub fn launch(id: &str) {
    com();
    let target = match pins().into_iter().find(|p| p.id == id) {
        Some(p) => p.target,
        None if id.contains('\\') => id.to_string(),
        None => format!("shell:AppsFolder\\{id}"),
    };
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(target), None, None, SW_SHOWNORMAL) };
}

static EVENTS: OnceLock<Sender<()>> = OnceLock::new();
/// Last foreground window outside Pane, so clicking the bar doesn't count as the app losing focus.
static LAST_ACTIVE: Mutex<isize> = Mutex::new(0);

/// Calls `on_change` (debounced ~100 ms, from any thread) when windows open/close/rename/minimize, focus changes,
/// or an app sets a badge or flashes. Returns once the taskband window (see `taskband`) exists.
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) {
    let (tx, rx) = channel();
    if EVENTS.set(tx).is_err() {
        return; // already watching
    }
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(100));
            while rx.try_recv().is_ok() {}
            on_change();
        }
    });
    let (ready, created) = channel();
    std::thread::spawn(move || unsafe {
        let _ = ready.send(create_taskband());
        for (min, max) in [
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE), // destroy, show, hide
            (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        ] {
            SetWinEventHook(min, max, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
    if created.recv().unwrap_or(0) == 0 {
        crate::win::log("can't create the taskband window: no badges or flashing");
    }
}

fn changed() {
    if let Some(tx) = EVENTS.get() {
        let _ = tx.send(());
    }
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    if obj != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 || unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    if event == EVENT_SYSTEM_FOREGROUND {
        *LAST_ACTIVE.lock().unwrap() = hwnd.0 as isize;
        // Like Explorer: a window stops asking for attention once the user brings it to front.
        lock(&FLASHING).retain(|&w| w != hwnd.0 as isize);
    }
    changed();
}

// ---- badges and flashing: Pane's "taskband" window ----
//
// Apps set badges with ITaskbarList3::SetOverlayIcon. Shell32 looks up the window in the "TaskbandHWND" property of
// the first `Shell_TrayWnd` (Pane's own while Pane is shown, see tray.rs) and sends it WM_USER messages. This is
// how ManagedShell (Cairo, RetroBar) does it. The same window gets the shell hook messages (HSHELL_FLASH).

/// One window's overlay badge.
#[derive(Clone, Default, PartialEq, Debug)]
struct Badge {
    png: String,
    tip: String,
}

static BADGES: Mutex<Option<HashMap<isize, Badge>>> = Mutex::new(None);
static FLASHING: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static TASKBAND: AtomicIsize = AtomicIsize::new(0);
static SHELLHOOK: AtomicU32 = AtomicU32::new(0);

/// ITaskbarList messages (ManagedShell's list): wParam = the app's window, lParam = HICON / shared-memory text.
const TBL_SET_OVERLAY_ICON: u32 = WM_USER + 79;
const TBL_SET_OVERLAY_TIP: u32 = WM_USER + 85;
const HSHELL_RUDEAPPACTIVATED: u32 = 0x8004;
const HSHELL_FLASH: u32 = 0x8006;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The window ITaskbarList calls should reach (0 if it couldn't be made). tray.rs puts it on Pane's `Shell_TrayWnd`.
pub fn taskband() -> isize {
    TASKBAND.load(SeqCst)
}

/// Tells every taskbar window its button exists ("TaskbarButtonCreated", as Explorer does), so apps send their
/// badges again, now to Pane. Done each time Pane is shown, like ManagedShell does at start.
pub fn announce() {
    let msg = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
    for h in taskbar_hwnds() {
        unsafe {
            let _ = SendNotifyMessageW(h, msg, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn create_taskband() -> isize {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default().into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(taskband_proc),
            hInstance: instance,
            lpszClassName: w!("PaneTaskband"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // A hidden tool window, never on the bar (shell hooks are meant for top-level windows, not message-only ones).
        let Ok(hwnd) =
            CreateWindowExW(WS_EX_TOOLWINDOW, w!("PaneTaskband"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None)
        else {
            return 0;
        };
        SHELLHOOK.store(RegisterWindowMessageW(w!("SHELLHOOK")), SeqCst);
        let _ = RegisterShellHookWindow(hwnd);
        TASKBAND.store(hwnd.0 as isize, SeqCst);
        hwnd.0 as isize
    }
}

unsafe extern "system" fn taskband_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let target = wp.0 as isize;
    match msg {
        m if m != 0 && m == SHELLHOOK.load(SeqCst) => {
            let window = lp.0;
            let mut flashing = lock(&FLASHING);
            let before = flashing.len();
            match wp.0 as u32 {
                HSHELL_FLASH if window != foreground() && !flashing.contains(&window) => flashing.push(window),
                HSHELL_WINDOWACTIVATED | HSHELL_RUDEAPPACTIVATED | HSHELL_WINDOWDESTROYED => {
                    flashing.retain(|&w| w != window);
                    if wp.0 as u32 == HSHELL_WINDOWDESTROYED {
                        lock(&BADGES).get_or_insert_with(HashMap::new).remove(&window);
                    }
                }
                _ => {}
            }
            if flashing.len() != before {
                changed();
            }
            LRESULT(0)
        }
        TBL_SET_OVERLAY_ICON => {
            // Read now: the app may destroy the icon as soon as this returns.
            let png = if lp.0 == 0 { String::new() } else { crate::tray::icon_png(lp.0) };
            set_badge(target, |b| b.png = png);
            to_explorer(msg, wp, lp);
            LRESULT(0)
        }
        TBL_SET_OVERLAY_TIP => {
            let tip = shared_text(lp.0, pid_of(HWND(target as _)));
            set_badge(target, |b| b.tip = tip);
            to_explorer(msg, wp, lp);
            LRESULT(0)
        }
        // Other ITaskbarList calls (progress, thumbnail buttons, tabs): accepted and ignored, like ManagedShell.
        m if (WM_USER + 50..=WM_USER + 90).contains(&m) => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn set_badge(hwnd: isize, change: impl FnOnce(&mut Badge)) {
    let mut badges = lock(&BADGES);
    let badges = badges.get_or_insert_with(HashMap::new);
    let before = badges.get(&hwnd).cloned().unwrap_or_default();
    let mut now = before.clone();
    change(&mut now);
    if now == Badge::default() {
        badges.remove(&hwnd);
    } else {
        badges.insert(hwnd, now.clone());
    }
    if now != before {
        changed();
    }
}

/// Explorer's taskbar gets the badge too, so it is right when Pane quits and Explorer's taskbar comes back.
/// The icon is still valid: the app waits in SendMessage until this returns.
fn to_explorer(msg: u32, wp: WPARAM, lp: LPARAM) {
    let Some(tray) = crate::win::tray() else { return };
    let band = unsafe { GetPropW(tray, w!("TaskbandHWND")) };
    if band.is_invalid() || band.0 as isize == taskband() {
        return;
    }
    unsafe {
        let _ = SendMessageTimeoutW(HWND(band.0), msg, wp, lp, SMTO_ABORTIFHUNG, 1000, None);
    }
}

/// A string an app passed as shared memory (SHAllocShared), read for the app's process.
fn shared_text(handle: isize, pid: u32) -> String {
    if handle == 0 || pid == 0 {
        return String::new();
    }
    unsafe {
        let p = SHLockShared(HANDLE(handle as _), pid) as *const u16;
        if p.is_null() {
            return String::new();
        }
        // shortcut: reads at most 256 chars; overlay descriptions are short labels like "3 unread messages".
        let len = (0..256).take_while(|&i| *p.add(i) != 0).count();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        let _ = SHUnlockShared(p as _);
        text
    }
}

/// The foreground window, or the last one before Pane's bar took focus.
fn foreground() -> isize {
    let fg = unsafe { GetForegroundWindow() };
    if pid_of(fg) == std::process::id() { *LAST_ACTIVE.lock().unwrap() } else { fg.0 as isize }
}

/// Shell calls need COM on the calling thread; Tauri commands run on worker threads that may not have it yet.
fn com() {
    thread_local!(static INIT: () = unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); });
    INIT.with(|_| {});
}

// ---- windows ----

/// Explorer's rule for which windows get a taskbar button.
fn is_taskbar_window(visible: bool, cloaked: bool, owned: bool, ex_style: WINDOW_EX_STYLE, title: &str) -> bool {
    visible
        && !cloaked
        && (!owned || ex_style.contains(WS_EX_APPWINDOW))
        && !ex_style.contains(WS_EX_TOOLWINDOW)
        && !title.is_empty()
}

/// Every hwnd seen so far, in order of first appearance (EnumWindows order is z-order, which changes on every click).
static SEEN: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// The windows that get a taskbar button, in z-order.
pub(crate) fn taskbar_hwnds() -> Vec<HWND> {
    unsafe extern "system" fn add(h: HWND, all: LPARAM) -> windows::core::BOOL {
        unsafe { (*(all.0 as *mut Vec<HWND>)).push(h) };
        true.into()
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(add), LPARAM(&mut all as *mut _ as isize));
    }
    let me = std::process::id();
    all.retain(|&h| {
        let title = text(h);
        unsafe {
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, 4);
            let owned = GetWindow(h, GW_OWNER).is_ok();
            let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(h, GWL_EXSTYLE) as u32);
            is_taskbar_window(IsWindowVisible(h).as_bool(), cloaked != 0, owned, ex, &title)
                && !(pid_of(h) == me && title == "Pane")
        }
    });
    all
}

fn windows(monitor: RECT) -> Vec<Win> {
    let fg = foreground();
    let badges = lock(&BADGES).clone().unwrap_or_default();
    let flashing = lock(&FLASHING).clone();
    let mut wins: Vec<Win> = taskbar_hwnds()
        .into_iter()
        .map(|h| {
            let hwnd = h.0 as isize;
            let (exe, aumid) = identity(h, pid_of(h));
            Win {
                hwnd,
                title: text(h),
                aumid,
                exe,
                active: hwnd == fg,
                badge: badges.get(&hwnd).cloned().unwrap_or_default(),
                flashing: flashing.contains(&hwnd) && hwnd != fg,
                here: monitor_of(h) == monitor,
            }
        })
        .collect();
    let mut seen = SEEN.lock().unwrap();
    seen.retain(|s| wins.iter().any(|w| w.hwnd == *s));
    for w in &wins {
        if !seen.contains(&w.hwnd) {
            seen.push(w.hwnd);
        }
    }
    wins.sort_by_key(|w| seen.iter().position(|s| *s == w.hwnd));
    wins
}

/// (exe, AppUserModelID) per window, cached by (hwnd, pid). The list is rebuilt on every title change and focus
/// switch (several times a second while a browser loads or a terminal prints), and reading these for every
/// window each time (COM property store, process queries) was the main source of Pane's lag.
static IDENTITY: OnceLock<Mutex<HashMap<(isize, u32), (String, String)>>> = OnceLock::new();

fn identity(h: HWND, pid: u32) -> (String, String) {
    let cache = IDENTITY.get_or_init(Default::default);
    let key = (h.0 as isize, pid);
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let exe = exe_of(pid);
    let aumid = window_aumid(h).or_else(|| {
        // Store apps run inside ApplicationFrameHost; the app itself owns a child window.
        let pid = if exe.ends_with("\\applicationframehost.exe") { child_pid(h, pid) } else { pid };
        process_aumid(pid)
    });
    // Store frames get their app's child window a moment after they open: don't cache "no AUMID" for them.
    let found = (exe, aumid.unwrap_or_default());
    if !(found.1.is_empty() && found.0.ends_with("\\applicationframehost.exe")) {
        let mut map = cache.lock().unwrap();
        if map.len() > 512 {
            map.clear(); // closed windows; cheaper than tracking them
        }
        map.insert(key, found.clone());
    }
    found
}

fn text(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn child_pid(h: HWND, frame_pid: u32) -> u32 {
    unsafe extern "system" fn find(c: HWND, out: LPARAM) -> windows::core::BOOL {
        let (frame, found) = unsafe { &mut *(out.0 as *mut (u32, u32)) };
        let pid = pid_of(c);
        if pid != *frame {
            *found = pid;
            return false.into();
        }
        true.into()
    }
    let mut state = (frame_pid, frame_pid);
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(find), LPARAM(&mut state as *mut _ as isize));
    }
    state.1
}

/// The rcMonitor of the monitor the window is on (minimized windows: where they will restore to).
fn monitor_of(h: HWND) -> RECT {
    unsafe {
        let mut rc = RECT::default();
        if IsIconic(h).as_bool() {
            let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            let _ = GetWindowPlacement(h, &mut wp);
            rc = wp.rcNormalPosition;
        } else {
            let _ = GetWindowRect(h, &mut rc);
        }
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST), &mut mi);
        mi.rcMonitor
    }
}

pub(crate) fn exe_of(pid: u32) -> String {
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(p);
        if ok { String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() } else { String::new() }
    }
}

/// The AppUserModelID a window was given explicitly (Chrome profiles, UWP frames, ...).
fn window_aumid(h: HWND) -> Option<String> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(h) }.ok()?;
    let value = unsafe { store.GetValue(&PKEY_AppUserModel_ID) }.ok()?;
    nonempty(unsafe { PropVariantToStringAlloc(&value) }.ok()?)
}

/// The AppUserModelID of a packaged (Store) app's process.
fn process_aumid(pid: u32) -> Option<String> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 256];
        let mut n = buf.len() as u32;
        let err = GetApplicationUserModelId(p, &mut n, Some(PWSTR(buf.as_mut_ptr())));
        let _ = CloseHandle(p);
        (err.0 == 0 && n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
    }
}

/// Takes a COM-allocated string; None if empty.
fn nonempty(s: PWSTR) -> Option<String> {
    let text = unsafe { s.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(s.0 as _)) };
    (!text.is_empty()).then_some(text)
}

// ---- pins ----

static PINS: Mutex<(Vec<u8>, Vec<Pin>)> = Mutex::new((Vec::new(), Vec::new()));

/// The user's taskbar pins in their order. Re-read only when Explorer's pin list changes.
fn pins() -> Vec<Pin> {
    let raw = favorites();
    let mut cache = PINS.lock().unwrap();
    if cache.0 != raw || cache.1.is_empty() && !raw.is_empty() {
        let pins = split_favorites(&raw).into_iter().filter_map(pin).collect();
        *cache = (raw, pins);
    }
    cache.1.clone()
}

/// Explorer's pin list, HKCU\...\Explorer\Taskband "Favorites".
fn favorites() -> Vec<u8> {
    let key = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Taskband");
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = buf.len() as u32;
    let err = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key, w!("Favorites"), RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as _), Some(&mut n))
    };
    buf.truncate(if err.0 == 0 { n as usize } else { 0 });
    buf
}

/// "Favorites" is a list of `[0x00][u32 size][item id list of `size` bytes]`, ended by 0xFF.
fn split_favorites(raw: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while raw.get(at) == Some(&0) && at + 5 <= raw.len() {
        let size = u32::from_le_bytes(raw[at + 1..at + 5].try_into().unwrap()) as usize;
        let Some(pidl) = raw.get(at + 5..at + 5 + size) else { break };
        out.push(pidl);
        at += 5 + size;
    }
    out
}

fn pin(pidl: &[u8]) -> Option<Pin> {
    let mut owned = pidl.to_vec();
    owned.extend_from_slice(&[0, 0]); // make sure the list is terminated
    let item: IShellItem2 = unsafe { SHCreateItemFromIDList(owned.as_ptr() as *const ITEMIDLIST) }.ok()?;
    let prop = |key: &PROPERTYKEY| unsafe { item.GetString(key) }.ok().and_then(nonempty).unwrap_or_default();
    let aumid = prop(&PKEY_AppUserModel_ID);
    let exe = full_exe(&prop(&PKEY_Link_TargetParsingPath).to_lowercase());
    let lnk = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok().and_then(nonempty).unwrap_or_default();
    let id = [&aumid, &exe, &lnk].into_iter().find(|s| !s.is_empty())?.clone();
    Some(Pin {
        name: unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok().and_then(nonempty).unwrap_or_default(),
        icon: icon(&item.cast().ok()?),
        target: if lnk.is_empty() { format!("shell:AppsFolder\\{aumid}") } else { lnk },
        id,
        aumid,
        exe,
    })
}

/// Some shortcuts name a bare exe ("explorer"); those live in the Windows folder.
// shortcut: only the Windows folder is searched, not the whole PATH; enough for the built-in pins.
fn full_exe(exe: &str) -> String {
    if exe.is_empty() || exe.contains('\\') {
        return exe.to_string();
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
    let ext = if exe.ends_with(".exe") { "" } else { ".exe" };
    format!(r"{root}\{exe}{ext}")
}

// ---- names and icons ----

static INFO: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();

/// (name, icon) of a running app, cached per id: the Start menu entry for an AppUserModelID, else the exe.
fn info(w: &Win) -> (String, String) {
    let cache = INFO.get_or_init(Default::default);
    let id = w.id();
    if let Some(hit) = cache.lock().unwrap().get(&id) {
        return hit.clone();
    }
    let from = |path: String, key: Option<&PROPERTYKEY>| -> Option<(String, String)> {
        let item: IShellItem2 = unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }.ok()?;
        let name = match key {
            Some(k) => unsafe { item.GetString(k) }.ok().and_then(nonempty),
            None => unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok().and_then(nonempty),
        };
        Some((name.unwrap_or_default(), icon(&item.cast().ok()?)))
    };
    let found = (!w.aumid.is_empty())
        .then(|| from(format!("shell:AppsFolder\\{}", w.aumid), None))
        .flatten()
        .or_else(|| from(w.exe.clone(), Some(&PKEY_FileDescription)))
        .unwrap_or_default();
    cache.lock().unwrap().insert(id, found.clone());
    found
}

/// Icon of any shell item (an exe or .lnk path, or "shell:AppsFolder\<AppUserModelID>") as a PNG
/// data URL, "" if there is none.
pub fn icon_of(path: &str) -> String {
    com();
    let item: Option<IShellItem2> = unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }.ok();
    item.and_then(|i| i.cast().ok()).map_or(String::new(), |f| icon(&f))
}

/// The item's icon as a PNG data URL, "" if there is none.
fn icon(factory: &IShellItemImageFactory) -> String {
    let Ok(bmp) = (unsafe { factory.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY) }) else {
        return String::new();
    };
    let png = pixels(bmp).and_then(|(w, h, px)| encode_png(w, h, &to_rgba(px)));
    unsafe {
        let _ = DeleteObject(bmp.into());
    }
    png.map_or(String::new(), data_url)
}

pub(crate) fn data_url(png: Vec<u8>) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

/// Pixels of a bitmap as 32-bit BGRA, top row first.
pub(crate) fn pixels(bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = BITMAP::default();
        GetObjectW(bmp.into(), size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as _));
        let (w, h) = (info.bmWidth, info.bmHeight.abs());
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let rows = GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if rows != h || w == 0 {
            return None;
        }
        Some((w as u32, h as u32, px))
    }
}

/// Shell bitmaps are premultiplied BGRA; icons without any alpha are fully opaque.
fn to_rgba(mut px: Vec<u8>) -> Vec<u8> {
    let has_alpha = px.chunks(4).any(|p| p[3] != 0);
    for p in px.chunks_mut(4) {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
        } else if p[3] != 0 && p[3] != 255 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255) / a).min(255) as u8;
            }
        }
    }
    px
}

pub(crate) fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(hwnd: isize, exe: &str, aumid: &str, here: bool) -> Win {
        Win { hwnd, title: format!("w{hwnd}"), exe: exe.into(), aumid: aumid.into(), here, ..Default::default() }
    }

    #[test]
    fn pins_first_then_running_apps_here() {
        let pins = vec![
            Pin { id: "chrome".into(), aumid: "Chrome".into(), exe: "c:\\chrome.exe".into(), ..Default::default() },
            Pin { id: "Spotify!App".into(), aumid: "Spotify!App".into(), ..Default::default() },
        ];
        let wins = vec![
            win(1, "c:\\code.exe", "", true),
            win(2, "c:\\chrome.exe", "", true),    // matches the pin by exe
            win(3, "c:\\chrome.exe", "Chrome", false), // pinned app, other monitor: not listed here
            win(4, "c:\\notes.exe", "", false),    // unpinned, other monitor: no icon here
            win(5, "c:\\code.exe", "", true),
        ];
        let items = group(&pins, &wins, |w| (format!("name of {}", w.exe), String::new()));
        let ids: Vec<_> = items.iter().map(|a| (a.id.as_str(), a.windows.iter().map(|w| w.hwnd).collect::<Vec<_>>())).collect();
        assert_eq!(ids, [("chrome", vec![2]), ("Spotify!App", vec![]), ("c:\\code.exe", vec![1, 5])]);
        assert!(items[0].pinned && !items[2].pinned);
        assert_eq!(items[2].name, "name of c:\\code.exe");
    }

    #[test]
    fn badges_set_and_clear_per_window() {
        let badge = |h| lock(&BADGES).as_ref().and_then(|b| b.get(&h).cloned());
        set_badge(-7, |b| b.png = "data:1".into());
        set_badge(-7, |b| b.tip = "3 unread".into());
        assert_eq!(badge(-7), Some(Badge { png: "data:1".into(), tip: "3 unread".into() }));
        set_badge(-7, |b| b.png = String::new());
        set_badge(-7, |b| b.tip = String::new());
        assert_eq!(badge(-7), None, "a cleared badge is forgotten");
        // The badge reaches the bar on the window's item.
        let mut w = win(1, "c:\\chat.exe", "", true);
        (w.badge, w.flashing) = (Badge { png: "data:2".into(), tip: "1 new".into() }, true);
        let items = group(&[], &[w], |_| ("Chat".into(), String::new()));
        let item = &items[0].windows[0];
        assert_eq!((item.badge.as_str(), item.badge_tip.as_str(), item.flashing), ("data:2", "1 new", true));
    }

    #[test]
    fn squirrel_pin_matches_versioned_exe() {
        let p = Pin { exe: r"c:\d\update.exe".into(), ..Default::default() };
        assert!(pin_matches(&p, &win(1, r"c:\d\app-1.0.9\discord.exe", "", true)));
        assert!(!pin_matches(&p, &win(1, r"c:\other\app-1\x.exe", "", true)));
    }

    #[test]
    fn favorites_blob_split() {
        let raw = [0, 2, 0, 0, 0, 0xAA, 0xBB, 0, 1, 0, 0, 0, 0xCC, 0xFF];
        assert_eq!(split_favorites(&raw), [&[0xAA, 0xBB][..], &[0xCC][..]]);
    }

    #[test]
    fn window_filter() {
        assert!(is_taskbar_window(true, false, false, WINDOW_EX_STYLE(0), "Doc"));
        assert!(is_taskbar_window(true, false, true, WS_EX_APPWINDOW, "Doc"));
        assert!(!is_taskbar_window(true, false, true, WINDOW_EX_STYLE(0), "Dialog"));
        assert!(!is_taskbar_window(true, false, false, WS_EX_TOOLWINDOW, "Tool"));
        assert!(!is_taskbar_window(true, true, false, WINDOW_EX_STYLE(0), "Other desktop"));
        assert!(!is_taskbar_window(true, false, false, WINDOW_EX_STYLE(0), ""));
    }

    /// Prints what the taskbar would show on the primary monitor: `cargo test -- --ignored print_apps --nocapture`
    #[test]
    #[ignore]
    fn print_apps() {
        let primary = monitor_of(unsafe { GetDesktopWindow() });
        for a in list(primary) {
            let wins: Vec<_> = a.windows.iter().map(|w| format!("{:?}{}", w.title, if w.active { "*" } else { "" })).collect();
            println!("{} {:?} [{}] icon={}B {:?}", if a.pinned { "PIN" } else { "RUN" }, a.name, a.id, a.icon.len(), wins);
        }
    }
}
