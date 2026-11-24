//! The Claude Code sessions open on this PC, for the popup over the Claude widget (the user's ideas.md,
//! 2026-11-24: "instead of the glow, a popup on hover with my terminal sessions").
//!
//! Claude Code registers every running session as `~/.claude/sessions/<pid>.json` with its name, folder and
//! state (`status`: "busy" while it works, "idle" when it waits for the user). A file can outlive its process,
//! so a session only counts while that pid runs and was started at `procStart` (pids are reused).
//! Its terminal window is the one Claude Code titled with the session's name ("◐ fix the build" while busy).
//! Read only when the popup asks; nothing is watched, sent or logged.

use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub name: String,
    /// The folder's last part ("pane").
    pub project: String,
    pub busy: bool,
    /// When the state last changed, Unix ms (0 if unknown).
    pub since: i64,
    /// Its terminal window, 0 if none was found (a background job nobody has open).
    pub hwnd: isize,
}

/// Every live session, busy ones first, then the most recently changed. `windows`: (hwnd, title) of the open
/// app windows.
pub fn list(windows: &[(isize, String)]) -> Vec<Session> {
    let Some(dir) = std::env::var_os("USERPROFILE").map(|h| Path::new(&h).join(r".claude\sessions")) else { return vec![] };
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<Session> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|v| {
            let pid = v["pid"].as_u64().unwrap_or(0) as u32;
            let started = v["procStart"].as_str().and_then(|s| s.parse().ok());
            pid != 0 && alive(pid, started)
        })
        .filter_map(|v| session(&v, windows))
        .collect();
    out.sort_by_key(|s| (!s.busy, -s.since));
    out
}

/// One registry file -> a session; None for a spare (a pre-started process nobody uses yet).
fn session(v: &Value, windows: &[(isize, String)]) -> Option<Session> {
    if v["spare"].as_bool() == Some(true) {
        return None;
    }
    let name = v["name"].as_str().unwrap_or("").trim().to_string();
    let cwd = v["cwd"].as_str().unwrap_or("");
    let project = cwd.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("").to_string();
    let hwnd = windows.iter().find(|(_, t)| !name.is_empty() && bare(t) == name).map_or(0, |(h, _)| *h);
    Some(Session {
        name: if name.is_empty() { project.clone() } else { name },
        project,
        busy: v["status"].as_str() == Some("busy"),
        since: v["statusUpdatedAt"].as_i64().or_else(|| v["updatedAt"].as_i64()).unwrap_or(0),
        hwnd,
    })
}

/// A terminal title without Claude Code's state mark in front ("◐ fix the build" -> "fix the build").
fn bare(title: &str) -> &str {
    title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim_end()
}

/// The process runs, and (when known) is the one started at `started` (FILETIME), not a later one with its pid.
fn alive(pid: u32, started: Option<u64>) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let running = GetExitCodeProcess(h, &mut code).is_ok() && code == 259; // STILL_ACTIVE
        let (mut created, mut a, mut b, mut c) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let same = match started {
            Some(want) if GetProcessTimes(h, &mut created, &mut a, &mut b, &mut c).is_ok() => {
                ((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64) == want
            }
            _ => true,
        };
        let _ = CloseHandle(h);
        running && same
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_registry_file() {
        let v: Value = serde_json::from_str(
            r#"{"pid":1,"cwd":"C:\\dev\\pane","name":"read ideas.md","status":"busy","statusUpdatedAt":5,"updatedAt":3}"#,
        )
        .unwrap();
        let wins = [(7, "◐ read ideas.md".to_string()), (8, "read ideas.md – Notepad".to_string())];
        assert_eq!(session(&v, &wins), Some(Session { name: "read ideas.md".into(), project: "pane".into(), busy: true, since: 5, hwnd: 7 }));
        // No title match: no window; no name: the folder stands in.
        let v: Value = serde_json::from_str(r#"{"pid":1,"cwd":"C:\\dev\\solar\\","status":"idle"}"#).unwrap();
        assert_eq!(session(&v, &wins), Some(Session { name: "solar".into(), project: "solar".into(), busy: false, since: 0, hwnd: 0 }));
        let spare: Value = serde_json::from_str(r#"{"pid":1,"spare":true}"#).unwrap();
        assert_eq!(session(&spare, &wins), None);
    }

    #[test]
    fn titles_lose_the_state_mark() {
        assert_eq!(bare("✳ fix the build"), "fix the build");
        assert_eq!(bare("◑ read ideas.md and images"), "read ideas.md and images");
        assert_eq!(bare("plain"), "plain");
    }

    /// Lists this PC's live sessions (no windows matched). `cargo test -- --ignored lists_this_pc --nocapture`
    #[test]
    #[ignore]
    fn lists_this_pc() {
        for s in list(&[]) {
            println!("{} | {} | busy {}", s.name, s.project, s.busy);
        }
    }

    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::process::id(), None));
        assert!(!alive(std::process::id(), Some(1)));
    }
}
