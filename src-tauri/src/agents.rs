//! Notices when Claude Code or Codex finishes a task, for the short glow on their usage widgets.
//! Both write each conversation as a JSONL file (Claude Code: `~/.claude/projects/<project>/<session>.jsonl`,
//! Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`); Windows reports every change to them
//! (ReadDirectoryChangesW, no polling) and the end of the changed file says whether the turn is done:
//! - Claude Code: the last user/assistant line (not from a subagent) is an assistant message with
//!   `stop_reason: "end_turn"`.
//! - Codex: the last task event is `task_complete` (after `task_started`).
//! A finished turn counts once, and only if it finished just now (old conversations being touched don't glow).
//! Only the file ends are read, locally; nothing is sent anywhere or logged.

use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// A turn that ended longer ago than this when Pane sees it doesn't glow (Pane just started, an old chat changed).
const FRESH: i64 = 60;
/// How much of a file's end is read: a few of the last lines (tool results can be long).
const TAIL: u64 = 256 * 1024;

/// Watches both folders, each on its own thread; `done` runs once per finished turn.
pub fn watch(done: impl Fn(Agent) + Send + Sync + Clone + 'static) {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else { return };
    let claude = home.join(".claude").join("projects");
    let codex = home.join(".codex").join("sessions");
    let d = done.clone();
    std::thread::spawn(move || watch_dir(&claude, Agent::Claude, d));
    std::thread::spawn(move || watch_dir(&codex, Agent::Codex, done));
}

fn watch_dir(dir: &Path, agent: Agent, done: impl Fn(Agent)) {
    // The folder appears when the tool is first used; look again now and then until it's there.
    let handle = loop {
        if let Some(h) = open_dir(dir) {
            break h;
        }
        std::thread::sleep(Duration::from_secs(60));
    };
    // Per file: the timestamp of the finished turn that already counted.
    let mut counted: HashMap<PathBuf, String> = HashMap::new();
    let mut last_glow = SystemTime::UNIX_EPOCH;
    let mut buf = vec![0u32; 16 * 1024]; // 64 KB, DWORD-aligned as the API needs
    loop {
        let mut len = 0u32;
        let ok = unsafe {
            ReadDirectoryChangesW(
                handle,
                buf.as_mut_ptr() as _,
                (buf.len() * 4) as u32,
                true,
                FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE,
                Some(&mut len),
                None,
                None,
            )
        };
        if ok.is_err() {
            crate::win::log(&format!("{} activity can't be watched any more", agent.name()));
            unsafe {
                let _ = CloseHandle(handle);
            }
            return;
        }
        // Let the writer finish its burst of lines, then look at each changed file once.
        std::thread::sleep(Duration::from_millis(300));
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len as usize) };
        let mut files = changed_files(bytes);
        files.sort();
        files.dedup();
        for rel in files {
            if !counts(agent, &rel) {
                continue;
            }
            let path = dir.join(&rel);
            let Some(tail) = read_tail(&path) else { continue };
            let Some(stamp) = finished_turn(agent, &tail) else { continue };
            let fresh = iso_secs(&stamp).is_some_and(|t| now_secs() - t <= FRESH);
            if fresh && counted.get(&path) != Some(&stamp) {
                counted.insert(path, stamp);
                // One glow for the multi-line end of a single turn.
                if last_glow.elapsed().unwrap_or_default() > Duration::from_secs(3) {
                    last_glow = SystemTime::now();
                    done(agent);
                }
            }
        }
        if counted.len() > 500 {
            counted.clear();
        }
    }
}

fn open_dir(dir: &Path) -> Option<HANDLE> {
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(dir.as_os_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    h.ok().filter(|h| !h.is_invalid())
}

/// The relative file names in a FILE_NOTIFY_INFORMATION list.
fn changed_files(bytes: &[u8]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    while let (Some(next), Some(name_len)) = (u32_at(at), u32_at(at + 8)) {
        let start = at + 12;
        if let Some(name) = bytes.get(start..start + name_len as usize) {
            let wide: Vec<u16> = name.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            out.push(PathBuf::from(String::from_utf16_lossy(&wide)));
        }
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    out
}

/// Which changed files are conversations: Claude Code's main session files sit right in a project folder
/// (subagents' files are deeper and their turns aren't the user's task); Codex's rollouts anywhere below.
fn counts(agent: Agent, rel: &Path) -> bool {
    let jsonl = rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl"));
    match agent {
        Agent::Claude => jsonl && rel.components().count() == 2,
        Agent::Codex => jsonl && rel.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rollout-")),
    }
}

fn read_tail(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(size.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The timestamp of the turn that just finished, when the conversation's latest state is "finished".
fn finished_turn(agent: Agent, tail: &str) -> Option<String> {
    // Newest line first; the first (cut) line of the tail doesn't parse and is skipped.
    for line in tail.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let stamp = || v.get("timestamp").and_then(Value::as_str).map(str::to_string);
        match agent {
            Agent::Claude => {
                if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                match v.get("type").and_then(Value::as_str) {
                    Some("assistant") => {
                        let stop = v.pointer("/message/stop_reason").and_then(Value::as_str);
                        return if stop == Some("end_turn") { stamp() } else { None };
                    }
                    Some("user") => return None, // a new prompt or a tool result: still working
                    _ => continue,               // titles, hooks, queue notes: not about the turn
                }
            }
            Agent::Codex => {
                if v.get("type").and_then(Value::as_str) != Some("event_msg") {
                    continue;
                }
                match v.pointer("/payload/type").and_then(Value::as_str) {
                    Some("task_complete") => return stamp(),
                    Some("task_started") => return None,
                    _ => continue,
                }
            }
        }
    }
    None
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// "2026-11-12T17:00:30.722Z" → Unix seconds (UTC; both tools write UTC with a Z).
fn iso_secs(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if s.as_bytes().get(10) != Some(&b'T') || !(1..=12).contains(&m) {
        return None;
    }
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_turns_end_with_end_turn() {
        let done = r#"{"type":"user","message":{"content":"hi"}}
{"type":"assistant","timestamp":"2026-11-13T10:00:01.000Z","message":{"stop_reason":"tool_use"}}
{"type":"user","message":{"content":[{"type":"tool_result"}]}}
{"type":"assistant","timestamp":"2026-11-13T10:00:05.000Z","message":{"stop_reason":"end_turn"}}
{"type":"system","subtype":"stop_hook_summary"}
{"type":"queue-operation"}"#;
        assert_eq!(finished_turn(Agent::Claude, done).as_deref(), Some("2026-11-13T10:00:05.000Z"));
        let working = format!("{done}\n{}", r#"{"type":"user","message":{"content":"next"}}"#);
        assert_eq!(finished_turn(Agent::Claude, &working), None);
        // A subagent finishing inside the main conversation isn't the end of the user's task.
        let sub = r#"{"type":"assistant","timestamp":"2026-11-13T10:00:01.000Z","message":{"stop_reason":"tool_use"}}
{"type":"assistant","isSidechain":true,"timestamp":"2026-11-13T10:00:02.000Z","message":{"stop_reason":"end_turn"}}"#;
        assert_eq!(finished_turn(Agent::Claude, sub), None);
    }

    #[test]
    fn codex_turns_end_with_task_complete() {
        let log = r#"{"timestamp":"2026-11-12T17:00:25.351Z","type":"event_msg","payload":{"type":"task_started"}}
{"timestamp":"2026-11-12T17:00:28.000Z","type":"response_item","payload":{"type":"message"}}
{"timestamp":"2026-11-12T17:00:30.722Z","type":"event_msg","payload":{"type":"task_complete"}}
{"timestamp":"2026-11-12T17:00:30.800Z","type":"event_msg","payload":{"type":"token_count"}}"#;
        assert_eq!(finished_turn(Agent::Codex, log).as_deref(), Some("2026-11-12T17:00:30.722Z"));
        let again = format!("{log}\n{}", r#"{"timestamp":"2026-11-12T17:01:00.000Z","type":"event_msg","payload":{"type":"task_started"}}"#);
        assert_eq!(finished_turn(Agent::Codex, &again), None);
    }

    #[test]
    fn only_conversation_files_count() {
        assert!(counts(Agent::Claude, Path::new("C--proj\\abc.jsonl")));
        assert!(!counts(Agent::Claude, Path::new("C--proj\\abc\\subagents\\agent-1.jsonl")));
        assert!(!counts(Agent::Claude, Path::new("C--proj\\notes.txt")));
        assert!(counts(Agent::Codex, Path::new("2026\\11\\12\\rollout-2026-11-12T18-00-21-x.jsonl")));
        assert!(!counts(Agent::Codex, Path::new("2026\\11\\12\\other.jsonl")));
    }

    #[test]
    fn reads_utc_timestamps() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_secs("2026-11-12T17:00:30.722Z"), Some(1_794_502_830));
        assert_eq!(iso_secs("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(iso_secs("garbage"), None);
    }

    #[test]
    fn parses_change_lists() {
        // Two entries: "a.jsonl" then "b\\c.jsonl".
        let entry = |next: u32, name: &str| {
            let wide: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            let mut e = Vec::new();
            e.extend(next.to_le_bytes());
            e.extend(3u32.to_le_bytes());
            e.extend((wide.len() as u32).to_le_bytes());
            e.extend(wide);
            while e.len() % 4 != 0 {
                e.push(0);
            }
            e
        };
        let first = entry(0, "a.jsonl");
        let mut bytes = entry(first.len() as u32, "a.jsonl");
        bytes.extend(entry(0, "b\\c.jsonl"));
        assert_eq!(changed_files(&bytes), [PathBuf::from("a.jsonl"), PathBuf::from("b\\c.jsonl")]);
    }
}
