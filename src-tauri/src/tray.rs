//! Tray icons: Pane's own hidden `Shell_TrayWnd` sits above Explorer's, so apps' Shell_NotifyIcon calls reach Pane
//! first. Every message is passed on to Explorer unchanged, so Explorer's tray (and every AppBar) keeps working.
//! The RetroBar / ManagedShell technique.

use crate::{apps, win};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::{w, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_ROUTINE_FLAGS,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{
    SHAllocShared, SHFreeShared, SHGetKnownFolderPath, SHLockShared, SHUnlockShared, KF_FLAG_DEFAULT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct TrayIcon {
    /// Stable key: "hwnd:uid", or the icon's GUID.
    pub id: String,
    pub tip: String,
    /// "data:image/png;base64,..." at the size the app made it, "" if unreadable.
    pub icon: String,
    /// Shown on the bar (Windows 11 "Other system tray icons" switch on); else in the hidden-icons overflow.
    pub promoted: bool,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Click {
    Left,
    Right,
    Double,
}

/// One icon as its app announced it.
#[derive(Clone, Default, PartialEq, Debug)]
struct Icon {
    hwnd: isize,
    uid: u32,
    /// Uppercase, without braces.
    guid: Option<String>,
    callback: u32,
    version: u32,
    tip: String,
    png: String,
    hidden: bool,
    /// Lowercased exe path of the owner window's process.
    exe: String,
}

impl Icon {
    fn id(&self) -> String {
        self.guid.clone().unwrap_or_else(|| format!("{}:{}", self.hwnd, self.uid))
    }
}

/// The NOTIFYICONDATA fields Pane uses, as Shell_NotifyIcon sends them.
#[derive(Default, PartialEq, Debug)]
struct Nid {
    hwnd: u32,
    uid: u32,
    flags: u32,
    callback: u32,
    hicon: u32,
    tip: String,
    state: u32,
    state_mask: u32,
    version: u32,
    guid: Option<String>,
}

const SIGNATURE: u32 = 0x3475_3423;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x01;
const NIF_ICON: u32 = 0x02;
const NIF_TIP: u32 = 0x04;
const NIF_STATE: u32 = 0x08;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x01;
const NIN_SELECT: u32 = WM_USER;
/// Explorer's tray answers this one only when posted (ManagedShell's list).
const POSTED_TO_EXPLORER: u32 = WM_USER + 372;
const WM_STOP: u32 = WM_APP + 1;
const TIMER_TOPMOST: usize = 1;

/// SHELLTRAYDATA: `[u32 signature][u32 NIM_*][NOTIFYICONDATA]`. Shell32 always sends the full struct with 32-bit
/// handles (also from 64-bit apps): hWnd @4, uID @8, uFlags @12, uCallbackMessage @16, hIcon @20, szTip[128] @24,
/// dwState @280, dwStateMask @284, szInfo[256] @288, uVersion @800, szInfoTitle[64] @804, dwInfoFlags @932, guidItem @936.
fn parse(data: &[u8]) -> Option<(u32, Nid)> {
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if u32_at(0)? != SIGNATURE {
        return None;
    }
    let field = |off: usize| u32_at(8 + off).unwrap_or(0);
    let tip: Vec<u16> = data.get(8 + 24..8 + 280)?.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let flags = field(12);
    let guid = data.get(8 + 936..8 + 952).map(|g| {
        let d4: [u8; 8] = g[8..].try_into().unwrap();
        let u16_at = |at: usize| u16::from_le_bytes([g[at], g[at + 1]]);
        GUID::from_values(u32::from_le_bytes(g[..4].try_into().unwrap()), u16_at(4), u16_at(6), d4)
    });
    Some((
        u32_at(4)?,
        Nid {
            hwnd: field(4),
            uid: field(8),
            flags,
            callback: field(16),
            hicon: field(20),
            tip: String::from_utf16_lossy(&tip[..tip.iter().position(|&c| c == 0).unwrap_or(tip.len())]),
            state: field(280),
            state_mask: field(284),
            version: field(800),
            guid: guid.filter(|g| flags & NIF_GUID != 0 && *g != GUID::zeroed()).map(|g| format!("{g:?}")),
        },
    ))
}

/// Handles are 32 bits on the wire and sign-extended in 64-bit processes.
fn handle(h: u32) -> isize {
    h as i32 as isize
}

/// Applies one NIM_* message to the icon list, like Explorer does. Returns whether it was accepted.
/// `png` turns an HICON into a data URL, `exe` gives the owner's exe of a new icon.
fn apply(icons: &mut Vec<Icon>, msg: u32, nid: &Nid, png: impl Fn(isize) -> String, exe: impl Fn(isize) -> String) -> bool {
    let at = icons.iter().position(|i| match &nid.guid {
        Some(g) => i.guid.as_ref() == Some(g),
        None => i.guid.is_none() && i.hwnd == handle(nid.hwnd) && i.uid == nid.uid,
    });
    match msg {
        NIM_ADD | NIM_MODIFY => {
            let i = match at {
                Some(i) => i,
                // A modify for an icon Pane missed (e.g. added before Pane started) adds it, as ManagedShell does.
                None if nid.hwnd != 0 => {
                    let hwnd = handle(nid.hwnd);
                    icons.push(Icon { hwnd, uid: nid.uid, guid: nid.guid.clone(), exe: exe(hwnd), ..Default::default() });
                    icons.len() - 1
                }
                None => return false,
            };
            let icon = &mut icons[i];
            if nid.hwnd != 0 {
                icon.hwnd = handle(nid.hwnd);
                icon.uid = nid.uid;
            }
            if nid.flags & NIF_MESSAGE != 0 {
                icon.callback = nid.callback;
            }
            if nid.flags & NIF_ICON != 0 {
                icon.png = if nid.hicon == 0 { String::new() } else { png(handle(nid.hicon)) };
            }
            if nid.flags & NIF_TIP != 0 {
                icon.tip = nid.tip.clone();
            }
            if nid.flags & NIF_STATE != 0 && nid.state_mask & NIS_HIDDEN != 0 {
                icon.hidden = nid.state & NIS_HIDDEN != 0;
            }
            // uVersion shares its field with the balloon timeout, so only real version numbers count.
            if (1..=4).contains(&nid.version) {
                icon.version = nid.version;
            }
            true
        }
        NIM_DELETE => at.map(|i| icons.remove(i)).is_some(),
        NIM_SETVERSION => match at {
            Some(i) if nid.version <= 4 => {
                icons[i].version = nid.version;
                true
            }
            _ => false,
        },
        _ => true, // NIM_SETFOCUS
    }
}

// ---- the host window ----

struct Host {
    hwnd: isize,
    thread: JoinHandle<()>,
}

static HOST: Mutex<Option<Host>> = Mutex::new(None);
static ICONS: Mutex<Vec<Icon>> = Mutex::new(Vec::new());
static CHANGED: Mutex<Option<Sender<()>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the tray host on its own thread and asks running apps to send their icons again.
/// `on_change` is called (debounced ~100 ms, from another thread) when the icons change.
pub fn start(on_change: impl Fn() + Send + Sync + 'static) {
    let mut host = lock(&HOST);
    if host.is_some() {
        return;
    }
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(100));
            while rx.try_recv().is_ok() {}
            on_change();
        }
    });
    *lock(&CHANGED) = Some(tx);
    let (ready, created) = channel();
    let thread = std::thread::spawn(move || run(ready));
    match created.recv() {
        Ok(hwnd) => *host = Some(Host { hwnd, thread }),
        Err(_) => {
            crate::win::log("can't create the tray window");
            *lock(&CHANGED) = None;
        }
    }
}

/// Removes Pane's tray window; apps are told to send their icons to Explorer again.
pub fn stop() {
    let Some(host) = lock(&HOST).take() else { return };
    unsafe {
        let _ = PostMessageW(Some(HWND(host.hwnd as _)), WM_STOP, WPARAM(0), LPARAM(0));
    }
    let _ = host.thread.join();
    *lock(&CHANGED) = None;
}

/// The visible icons in the order they were added.
pub fn list() -> Vec<TrayIcon> {
    let icons = {
        let mut icons = lock(&ICONS);
        icons.retain(|i| unsafe { IsWindow(Some(HWND(i.hwnd as _))) }.as_bool());
        icons.clone()
    };
    let settings = settings();
    icons
        .into_iter()
        .filter(|i| !i.hidden)
        .map(|i| TrayIcon { id: i.id(), promoted: promoted(&settings, &i), tip: i.tip, icon: i.png })
        .collect()
}

/// Sends the click to the icon's app the way Explorer does, so its own menu or window opens.
/// `x`, `y`: the icon's screen position (apps place their menu there).
pub fn click(id: &str, kind: Click, x: i32, y: i32) {
    let Some(icon) = lock(&ICONS).iter().find(|i| i.id() == id).cloned() else { return };
    if icon.callback == 0 {
        return;
    }
    let hwnd = HWND(icon.hwnd as _);
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        // Pane is in front after the user's click; let the app take the focus for its menu.
        let _ = AllowSetForegroundWindow(pid);
        for &msg in messages(kind, icon.version) {
            let (wp, lp) = params(icon.version, icon.uid, x, y, msg);
            let _ = PostMessageW(Some(hwnd), icon.callback, wp, lp);
        }
    }
}

/// What Explorer sends for each click. NOTIFYICON_VERSION_4 apps also get NIN_SELECT / WM_CONTEXTMENU.
fn messages(kind: Click, version: u32) -> &'static [u32] {
    match (kind, version >= 4) {
        (Click::Left, true) => &[WM_LBUTTONDOWN, WM_LBUTTONUP, NIN_SELECT],
        (Click::Left, false) => &[WM_LBUTTONDOWN, WM_LBUTTONUP],
        (Click::Right, true) => &[WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU],
        (Click::Right, false) => &[WM_RBUTTONDOWN, WM_RBUTTONUP],
        (Click::Double, _) => &[WM_LBUTTONDBLCLK],
    }
}

/// Callback parameters: version 4 gets (MAKEWPARAM(x, y), MAKELPARAM(msg, uID)), older versions (uID, msg).
fn params(version: u32, uid: u32, x: i32, y: i32, msg: u32) -> (WPARAM, LPARAM) {
    if version >= 4 {
        let lo_hi = |lo: u32, hi: u32| (lo & 0xFFFF) | (hi & 0xFFFF) << 16;
        (WPARAM(lo_hi(x as u32, y as u32) as usize), LPARAM(lo_hi(msg, uid) as isize))
    } else {
        (WPARAM(uid as usize), LPARAM(msg as isize))
    }
}

/// The host thread: the window and its message loop. Sends the window back through `ready`.
fn run(ready: Sender<isize>) {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default().into();
        let class = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance, lpszClassName: w!("Shell_TrayWnd"), ..Default::default() };
        RegisterClassW(&class); // fails harmlessly if an earlier start() registered it
        // Topmost + created last puts it first in z-order, so FindWindow("Shell_TrayWnd") finds it before Explorer's.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("Shell_TrayWnd"),
            w!(""),
            WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return;
        };
        // shortcut: polls the z-order every 200 ms like ManagedShell (100 ms); an icon added in that gap reaches only
        // Explorer until the next TaskbarCreated. Upgrade if icons go missing.
        SetTimer(Some(hwnd), TIMER_TOPMOST, 200, None);
        // ITaskbarList (app badges) goes to the window named here on the first Shell_TrayWnd (see apps.rs).
        let band = apps::taskband();
        if band != 0 {
            let _ = SetPropW(hwnd, w!("TaskbandHWND"), Some(HANDLE(band as _)));
        }
        let _ = ready.send(hwnd.0 as isize);
        taskbar_created();
        if band != 0 {
            apps::announce();
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        let _ = UnregisterClassW(w!("Shell_TrayWnd"), Some(instance));
        lock(&ICONS).clear();
        taskbar_created();
    }
}

/// Tells all apps the tray (re)appeared, so they add their icons again.
fn taskbar_created() {
    unsafe {
        let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COPYDATA if lp.0 != 0 => {
                let cds = &*(lp.0 as *const COPYDATASTRUCT);
                let tray = cds.dwData == 1;
                let ours = tray && !cds.lpData.is_null() && {
                    let data = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                    parse(data).is_some_and(|(m, nid)| on_icon_message(m, &nid))
                };
                // dwData 0 is SHAppBarMessage (also Pane's own AppBar), 3 Shell_NotifyIconGetRect: Explorer answers.
                let explorer = if cds.dwData == 0 { forward_appbar(wp, cds) } else { forward(msg, wp, lp) };
                if tray {
                    LRESULT((ours || explorer.is_some_and(|r| r.0 != 0)) as isize)
                } else {
                    explorer.unwrap_or_default()
                }
            }
            // Explorer's own WM_USER messages (tools send these to "the taskbar"); WM_APP+ and broadcasts stay here.
            m if m == WM_COMMAND || (WM_USER..WM_APP).contains(&m) => forward(msg, wp, lp).unwrap_or_default(),
            WM_TIMER if wp.0 == TIMER_TOPMOST => {
                if FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok() != Some(hwnd) {
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                LRESULT(0)
            }
            WM_WINDOWPOSCHANGED => {
                // Tools that show "the taskbar" by class name must not make this window visible.
                if (*(lp.0 as *const WINDOWPOS)).flags.contains(SWP_SHOWWINDOW) {
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_VISIBLE.0 as isize));
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_STOP => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => LRESULT(0), // only stop() closes it
            WM_DESTROY => {
                let _ = RemovePropW(hwnd, w!("TaskbandHWND"));
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Passes a message on to Explorer's taskbar; None if there is none or it didn't answer.
fn forward(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let explorer = win::tray()?;
    unsafe {
        if msg == POSTED_TO_EXPLORER {
            let _ = PostMessageW(Some(explorer), msg, wp, lp);
            return Some(LRESULT(0));
        }
        let mut result = 0usize;
        let ok = SendMessageTimeoutW(explorer, msg, wp, lp, SMTO_ABORTIFHUNG, 3000, Some(&mut result));
        (ok.0 != 0).then_some(LRESULT(result as isize))
    }
}

/// SHAppBarMessage (APPBARMSGDATAV3: APPBARDATA[40], dwMessage, hSharedMemory @48, dwProcessId @56) passes its
/// APPBARDATA in shared memory made for the tray window's process, i.e. Pane, so Explorer can't open it (and Pane's
/// own calls even get a plain pointer). Hand Explorer a copy made for Explorer and copy its answer back.
unsafe fn forward_appbar(wp: WPARAM, cds: &COPYDATASTRUCT) -> Option<LRESULT> {
    let me = std::process::id();
    let mut data = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }.to_vec();
    let lp = LPARAM(cds as *const _ as isize);
    let at = |i: usize| data.get(i..i + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).unwrap_or(0);
    let (shared, owner) = (at(48), at(56) as u32);
    let explorer = win::tray()?;
    let mut explorer_pid = 0;
    unsafe { GetWindowThreadProcessId(explorer, Some(&mut explorer_pid)) };
    if shared == 0 || owner != me || explorer_pid == 0 {
        return forward(WM_COPYDATA, wp, lp);
    }
    unsafe {
        let mine = SHLockShared(HANDLE(shared as _), me);
        if mine.is_null() {
            return forward(WM_COPYDATA, wp, lp);
        }
        let size = (*(mine as *const u32)).min(256); // the APPBARDATA's own cbSize
        let theirs = SHAllocShared(Some(mine), size, explorer_pid);
        data[48..56].copy_from_slice(&(theirs.0 as u64).to_le_bytes());
        data[56..60].copy_from_slice(&explorer_pid.to_le_bytes());
        let copy = COPYDATASTRUCT { dwData: 0, cbData: data.len() as u32, lpData: data.as_mut_ptr() as _ };
        let result = forward(WM_COPYDATA, wp, LPARAM(&copy as *const _ as isize));
        let answer = SHLockShared(theirs, explorer_pid);
        if !answer.is_null() {
            std::ptr::copy_nonoverlapping(answer as *const u8, mine as *mut u8, size as usize);
            let _ = SHUnlockShared(answer);
        }
        let _ = SHFreeShared(theirs, explorer_pid);
        let _ = SHUnlockShared(mine);
        result
    }
}

fn on_icon_message(msg: u32, nid: &Nid) -> bool {
    let mut icons = lock(&ICONS);
    let before = icons.clone();
    let ok = apply(&mut icons, msg, nid, icon_png, owner_exe);
    if *icons != before {
        if let Some(tx) = lock(&CHANGED).as_ref() {
            let _ = tx.send(());
        }
    }
    ok
}

fn owner_exe(hwnd: isize) -> String {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(HWND(hwnd as _), Some(&mut pid)) };
    apps::exe_of(pid)
}

/// The icon as a PNG data URL at its own size (the app picks it for the tray's DPI), "" if unreadable.
// shortcut: monochrome icons (no color bitmap) give ""; no tray app seen uses them.
pub(crate) fn icon_png(hicon: isize) -> String {
    unsafe {
        let mut ii = ICONINFO::default();
        if GetIconInfo(HICON(hicon as _), &mut ii).is_err() {
            return String::new();
        }
        let color = if ii.hbmColor.is_invalid() { None } else { apps::pixels(ii.hbmColor) };
        let mask = apps::pixels(ii.hbmMask);
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        let Some((w, h, px)) = color else { return String::new() };
        let mask = mask.filter(|m| (m.0, m.1) == (w, h)).map(|m| m.2);
        apps::encode_png(w, h, &icon_rgba(px, mask.as_deref())).map_or(String::new(), apps::data_url)
    }
}

/// Icon bitmaps are straight (not premultiplied) BGRA; icons without alpha use the AND mask (white = transparent).
fn icon_rgba(mut px: Vec<u8>, mask: Option<&[u8]>) -> Vec<u8> {
    let has_alpha = px.chunks(4).any(|p| p[3] != 0);
    for (i, p) in px.chunks_mut(4).enumerate() {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = if mask.is_some_and(|m| m[i * 4] != 0) { 0 } else { 255 };
        }
    }
    px
}

// ---- promoted (shown on the bar) or overflow ----

/// One entry of HKCU\Control Panel\NotifyIconSettings, where Windows 11 keeps each icon's "show in taskbar" switch.
#[derive(Default)]
struct Setting {
    exe: String,
    uid: Option<u32>,
    guid: Option<String>,
    promoted: bool,
}

/// Matched by owner exe plus GUID (icons that have one) or uID, the same keys Windows uses.
fn promoted(settings: &[Setting], icon: &Icon) -> bool {
    settings
        .iter()
        .find(|s| s.exe == icon.exe && if icon.guid.is_some() { s.guid == icon.guid } else { s.uid == Some(icon.uid) })
        .is_some_and(|s| s.promoted)
}

fn settings() -> Vec<Setting> {
    let mut out = Vec::new();
    let mut folders = HashMap::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, w!(r"Control Panel\NotifyIconSettings"), Some(0), KEY_READ, &mut key).0 != 0 {
            return out;
        }
        for index in 0.. {
            let mut name = [0u16; 64];
            let mut n = name.len() as u32;
            if RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None).0 != 0 {
                break;
            }
            let sub = PCWSTR(name.as_ptr());
            let text = |value: PCWSTR| {
                let mut buf = [0u16; 1024];
                let mut size = (buf.len() * 2) as u32;
                let err = RegGetValueW(key, sub, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size));
                (err.0 == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
            };
            let dword = |value: PCWSTR| {
                let (mut v, mut size) = (0u32, 4u32);
                let flags: REG_ROUTINE_FLAGS = RRF_RT_REG_DWORD;
                let err = RegGetValueW(key, sub, value, flags, None, Some(&mut v as *mut _ as _), Some(&mut size));
                (err.0 == 0).then_some(v)
            };
            let Some(path) = text(w!("ExecutablePath")) else { continue };
            out.push(Setting {
                exe: expand(&path, &mut folders),
                uid: dword(w!("UID")),
                guid: text(w!("IconGuid")).map(|g| g.trim_matches(['{', '}']).to_uppercase()),
                promoted: dword(w!("IsPromoted")) == Some(1),
            });
        }
        let _ = RegCloseKey(key);
    }
    out
}

/// "{known folder GUID}\rest" -> "c:\program files\rest", lowercased like `apps::exe_of`.
fn expand(path: &str, folders: &mut HashMap<String, String>) -> String {
    let lower = path.to_lowercase();
    let Some((id, rest)) = lower.strip_prefix('{').and_then(|p| p.split_once('}')) else { return lower };
    let folder = folders.entry(id.to_string()).or_insert_with(|| {
        let Ok(guid) = GUID::try_from(id) else { return String::new() };
        unsafe { SHGetKnownFolderPath(&guid, KF_FLAG_DEFAULT, None) }.map_or(String::new(), |p| {
            let s = unsafe { p.to_string() }.unwrap_or_default().to_lowercase();
            unsafe { CoTaskMemFree(Some(p.0 as _)) };
            s
        })
    });
    format!("{folder}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SHELLTRAYDATA as shell32 builds it.
    fn message(msg: u32, flags: u32, tip: &str, guid: Option<GUID>) -> Vec<u8> {
        let mut b = vec![0u8; 8 + 956];
        let mut put = |at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(0, SIGNATURE);
        put(4, msg);
        put(8, 956);
        put(8 + 4, 0x0001_02A4); // hWnd
        put(8 + 8, 7); // uID
        put(8 + 12, flags);
        put(8 + 16, WM_APP + 5); // uCallbackMessage
        put(8 + 20, 0xFFFF_8001); // hIcon, sign-extended in 64-bit processes
        put(8 + 280, NIS_HIDDEN);
        put(8 + 284, NIS_HIDDEN);
        put(8 + 800, 4);
        for (i, c) in tip.encode_utf16().enumerate() {
            b[8 + 24 + i * 2..8 + 26 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        if let Some(g) = guid {
            b[8 + 936..8 + 940].copy_from_slice(&g.data1.to_le_bytes());
            b[8 + 940..8 + 942].copy_from_slice(&g.data2.to_le_bytes());
            b[8 + 942..8 + 944].copy_from_slice(&g.data3.to_le_bytes());
            b[8 + 944..8 + 952].copy_from_slice(&g.data4);
        }
        b
    }

    #[test]
    fn parses_shelltraydata() {
        let guid = GUID::from_u128(0xBA82E2DC_F405_47AD_B032_CF0FAA0E3933);
        let (msg, nid) = parse(&message(NIM_ADD, NIF_TIP | NIF_GUID, "Windows Security", Some(guid))).unwrap();
        assert_eq!(msg, NIM_ADD);
        assert_eq!((nid.hwnd, nid.uid, nid.callback, nid.version), (0x0001_02A4, 7, WM_APP + 5, 4));
        assert_eq!(handle(nid.hicon), -0x7FFF);
        assert_eq!(nid.tip, "Windows Security");
        assert_eq!(nid.guid.as_deref(), Some("BA82E2DC-F405-47AD-B032-CF0FAA0E3933"));
        // A GUID without NIF_GUID doesn't count; other signatures aren't tray messages.
        assert_eq!(parse(&message(NIM_ADD, 0, "", Some(guid))).unwrap().1.guid, None);
        let mut bad = message(NIM_ADD, 0, "", None);
        bad[0] = 0;
        assert_eq!(parse(&bad), None);
        assert_eq!(parse(&bad[..100]), None);
    }

    #[test]
    fn add_modify_hide_delete() {
        let mut icons = Vec::new();
        let png = |h: isize| format!("png{h}");
        let exe = |_| "c:\\app.exe".to_string();
        let (_, mut nid) = parse(&message(NIM_ADD, NIF_MESSAGE | NIF_ICON | NIF_TIP, "One", None)).unwrap();
        assert!(apply(&mut icons, NIM_ADD, &nid, png, exe));
        assert_eq!(icons[0].id(), format!("{}:7", 0x0001_02A4));
        assert_eq!((icons[0].tip.as_str(), icons[0].png.as_str(), icons[0].exe.as_str()), ("One", "png-32767", "c:\\app.exe"));
        nid.flags = NIF_STATE | NIF_TIP;
        nid.tip = "Two".into();
        assert!(apply(&mut icons, NIM_MODIFY, &nid, png, exe));
        assert_eq!(icons.len(), 1);
        assert!(icons[0].hidden && icons[0].tip == "Two" && icons[0].png == "png-32767");
        nid.uid = 8;
        assert!(!apply(&mut icons, NIM_DELETE, &nid, png, exe)); // other uID: unknown
        nid.uid = 7;
        assert!(apply(&mut icons, NIM_DELETE, &nid, png, exe));
        assert!(icons.is_empty());
        nid.hwnd = 0;
        assert!(!apply(&mut icons, NIM_MODIFY, &nid, png, exe)); // no window to add it for
    }

    #[test]
    fn click_params_per_version() {
        assert_eq!(params(4, 7, 100, -2, WM_LBUTTONUP), (WPARAM(0xFFFE_0064), LPARAM(0x0007_0202)));
        assert_eq!(params(0, 7, 100, 200, WM_LBUTTONUP), (WPARAM(7), LPARAM(0x0202)));
        assert_eq!(messages(Click::Right, 4).last(), Some(&WM_CONTEXTMENU));
        assert_eq!(messages(Click::Left, 3), [WM_LBUTTONDOWN, WM_LBUTTONUP]);
    }

    #[test]
    fn icon_without_alpha_uses_mask() {
        let px = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let mask = [0, 0, 0, 0, 255, 255, 255, 0];
        assert_eq!(icon_rgba(px, Some(&mask)), [3, 2, 1, 255, 6, 5, 4, 0]);
        assert_eq!(icon_rgba(vec![1, 2, 3, 128], None), [3, 2, 1, 128]); // straight alpha kept
    }

    #[test]
    fn promoted_by_exe_and_guid_or_uid() {
        let settings = [
            Setting { exe: "c:\\a.exe".into(), uid: Some(1), promoted: true, ..Default::default() },
            Setting { exe: "c:\\a.exe".into(), uid: Some(2), ..Default::default() },
            Setting { exe: "c:\\b.exe".into(), guid: Some("G".into()), promoted: true, ..Default::default() },
        ];
        let icon = |exe: &str, uid, guid: Option<&str>| Icon { exe: exe.into(), uid, guid: guid.map(Into::into), ..Default::default() };
        assert!(promoted(&settings, &icon("c:\\a.exe", 1, None)));
        assert!(!promoted(&settings, &icon("c:\\a.exe", 2, None)));
        assert!(promoted(&settings, &icon("c:\\b.exe", 0, Some("G"))));
        assert!(!promoted(&settings, &icon("c:\\c.exe", 1, None)));
    }

    #[test]
    fn known_folder_paths_expand() {
        let mut cache = HashMap::new();
        let windows = expand(r"{F38BF404-1D43-42F2-9305-67DE0B28FC23}\explorer.exe", &mut cache);
        assert!(windows.ends_with(r"\windows\explorer.exe") && windows.contains(':'), "{windows}");
        assert_eq!(expand(r"C:\X\App.exe", &mut cache), r"c:\x\app.exe");
    }

    /// Hosts the tray for 5 s, prints the icons and checks Explorer still gets icons and AppBar messages:
    /// `cargo test -- --ignored tray_live --nocapture`
    #[test]
    #[ignore]
    fn tray_live() {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::Shell::*;
        let owner = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, None, None) }.unwrap();
        // Reads AppBar answers: state, taskbar rect, and where an AppBar at the bottom may go (data in and out).
        let appbar = || unsafe {
            let mut data = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: owner, ..Default::default() };
            let state = SHAppBarMessage(ABM_GETSTATE, &mut data);
            SHAppBarMessage(ABM_GETTASKBARPOS, &mut data);
            let taskbar = data.rc;
            SHAppBarMessage(ABM_NEW, &mut data);
            data.uEdge = ABE_BOTTOM;
            data.rc = RECT { left: 0, top: 0, right: taskbar.right, bottom: taskbar.bottom };
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            SHAppBarMessage(ABM_REMOVE, &mut data);
            (state, taskbar, data.rc)
        };
        let first = || unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.map_or(0, |h| h.0 as isize);
        // A test icon of our own, added and removed with the real Shell_NotifyIcon.
        let mut nid = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: owner, uID: 1, ..Default::default() };
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_APP + 9;
        nid.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap();
        nid.szTip[..14].copy_from_slice(&"Pane tray test".encode_utf16().collect::<Vec<_>>());
        let notify = |m| unsafe { Shell_NotifyIconW(m, &nid) }.as_bool();

        println!("before: appbar {:?}, explorer tray {:?}", appbar(), win::tray());
        start(|| println!("  (changed)"));
        let ours = lock(&HOST).as_ref().map(|h| h.hwnd);
        println!("ours {ours:?}, FindWindow finds {}", first());
        assert_eq!(ours, Some(first()), "apps would still find Explorer's tray first");
        std::thread::sleep(Duration::from_secs(5));
        let during = appbar();
        println!("during: appbar {during:?} (forwarded to Explorer)");
        assert!(notify(NIM_ADD), "Shell_NotifyIcon add failed through Pane's window");
        std::thread::sleep(Duration::from_millis(300));
        for i in list() {
            println!("{} promoted={} icon={}B {:?}", i.id, i.promoted, i.icon.len(), i.tip);
        }
        assert!(list().iter().any(|i| i.tip == "Pane tray test" && i.icon.starts_with("data:image/png")));
        stop();
        let after = appbar();
        println!("after: appbar {after:?}, FindWindow finds {} (explorer {:?})", first(), win::tray());
        assert_ne!(during.1, RECT::default(), "SHAppBarMessage got no answer through Pane's window");
        assert_eq!(during, after);
        // Explorer got the forwarded add: it accepts a modify (refused for unknown icons) and the delete.
        let (modify, delete) = (notify(NIM_MODIFY), notify(NIM_DELETE));
        println!("Explorer after stop: modify {modify}, delete {delete}");
        assert!(modify && delete, "Explorer's tray didn't get the icon");
        assert!(!notify(NIM_MODIFY), "Explorer accepts modify for unknown icons, so the check above proves nothing");
        unsafe { DestroyWindow(owner) }.unwrap();
    }
}
