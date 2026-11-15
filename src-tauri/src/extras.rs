//! Small opt-in extras (all off unless switched on in the settings): keep awake, virtual desktop dots and the
//! user's own command widget. Ideas from Linux bars (GNOME's Caffeine, Waybar's workspaces and custom modules),
//! chosen with the user 2026-11-14.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ,
    REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_BINARY,
};

// ---------- Keep awake

/// The thread that holds the "don't sleep" request while keep awake is on (the request belongs to the thread
/// that made it, so it lives until that thread lets go). None: off.
static AWAKE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

/// Keeps the PC and the screen from going to sleep (on) or lets them again (off). Not saved: every start of
/// Pane begins with normal sleep, so a forgotten switch can't keep a laptop awake for days.
pub fn set_awake(on: bool) {
    let mut awake = AWAKE.lock().unwrap_or_else(|e| e.into_inner());
    if on == awake.is_some() {
        return;
    }
    if !on {
        *awake = None; // dropping the sender ends the thread, which clears the request
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) };
        let _ = rx.recv(); // until the sender is dropped
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    });
    *awake = Some(tx);
}

pub fn awake() -> bool {
    AWAKE.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

// ---------- Virtual desktops

/// The virtual desktops: how many there are and which one is shown (0-based).
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Desktops {
    pub count: usize,
    pub current: usize,
}

const DESKTOPS_KEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops");

fn binary(name: windows::core::PCWSTR) -> Vec<u8> {
    let mut buf = vec![0u8; 16 * 64];
    let mut n = buf.len() as u32;
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, DESKTOPS_KEY, name, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as _), Some(&mut n)) };
    if ok.0 != 0 {
        return Vec::new();
    }
    buf.truncate(n as usize);
    buf
}

/// Read from Explorer's own record (the documented API has no list of desktops). One desktop when Windows
/// hasn't written it yet (never made a second one).
pub fn desktops() -> Desktops {
    desktops_from(&binary(w!("VirtualDesktopIDs")), &binary(w!("CurrentVirtualDesktop")))
}

fn desktops_from(ids: &[u8], current: &[u8]) -> Desktops {
    let all: Vec<&[u8]> = ids.chunks_exact(16).collect();
    let current = all.iter().position(|id| *id == current).unwrap_or(0);
    Desktops { count: all.len().max(1), current }
}

/// Blocks until something in Explorer's desktops key changes (a desktop added, removed or switched to): an event
/// from the registry, so watching costs nothing while nothing happens. Returns at once (after a pause) if the key
/// can't be watched.
pub fn wait_for_desktop_change() {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, DESKTOPS_KEY, Some(0), KEY_NOTIFY | KEY_READ, &mut key).0 != 0 {
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        let _ = RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false);
        let _ = RegCloseKey(key);
    }
}

/// How many Ctrl+Win+Right (positive) or Left (negative) presses go from desktop `from` to `to`.
pub fn steps(from: usize, to: usize) -> i32 {
    to as i32 - from as i32
}

// ---------- The user's own command

/// Longest text shown on the bar; longer output is cut with "…".
const CUSTOM_MAX: usize = 48;
/// A command that runs longer than this is stopped (the bar shows "timed out").
const CUSTOM_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the command (through cmd, hidden) and returns the first non-empty line of what it printed, cut to fit.
pub fn run_custom(command: &str) -> String {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
    let child = Command::new("cmd")
        .raw_arg(format!("/d /c {command}"))
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return format!("can't run: {e}"),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > CUSTOM_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return "timed out".into();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return format!("failed: {e}"),
        }
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_end(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_end(&mut err);
    }
    let text = if out.iter().any(|b| !b.is_ascii_whitespace()) { out } else { err };
    first_line(&String::from_utf8_lossy(&text))
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > CUSTOM_MAX {
        format!("{}…", line.chars().take(CUSTOM_MAX - 1).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktops_are_read_from_explorers_ids() {
        let a = [1u8; 16];
        let b = [2u8; 16];
        let c = [3u8; 16];
        let ids = [a, b, c].concat();
        assert_eq!(desktops_from(&ids, &b), Desktops { count: 3, current: 1 });
        // Never made a second desktop: Windows may not have written anything yet.
        assert_eq!(desktops_from(&[], &[]), Desktops { count: 1, current: 0 });
        assert_eq!((steps(2, 0), steps(0, 2), steps(1, 1)), (-2, 2, 0));
    }

    #[test]
    fn custom_output_is_one_short_line() {
        assert_eq!(first_line("\r\n  main  \r\nmore\r\n"), "main");
        assert_eq!(first_line(""), "");
        let long = first_line(&"x".repeat(100));
        assert_eq!(long.chars().count(), CUSTOM_MAX);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_custom_command_prints_its_first_line() {
        assert_eq!(run_custom("echo hello& echo world"), "hello");
        assert_eq!(run_custom("echo oops 1>&2"), "oops", "only errors: show them");
    }

    #[test]
    fn keep_awake_switches() {
        set_awake(true);
        assert!(awake());
        set_awake(true);
        set_awake(false);
        assert!(!awake());
    }
}
