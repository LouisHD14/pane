//! Notification counts for the app icons: how many of an app's notifications sit in Windows' notification centre,
//! or the badge a Store app set on its tile (WhatsApp, Teams, Mail). Apps that use ITaskbarList3 overlay badges
//! are covered by apps.rs already; this adds every app that only sends toasts, and Store apps, whose badges have
//! no documented reading API (decided 2026-11-16, the user's ideas.md).
//!
//! Both live in Windows' own notification database, %LOCALAPPDATA%\Microsoft\Windows\Notifications\
//! wpndatabase.db (SQLite, WAL). It is opened read-only with the SQLite that ships with Windows (winsqlite3.dll,
//! no dependency) and read again only when Windows writes to that folder (ReadDirectoryChangesW, no polling).
//! Only app ids, types and badge values are read; notification texts are never read, logged or sent.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

#[link(name = "winsqlite3", kind = "raw-dylib")]
unsafe extern "system" {
    fn sqlite3_open_v2(file: *const c_char, db: *mut *mut c_void, flags: c_int, vfs: *const c_char) -> c_int;
    fn sqlite3_busy_timeout(db: *mut c_void, ms: c_int) -> c_int;
    fn sqlite3_prepare_v2(db: *mut c_void, sql: *const c_char, len: c_int, stmt: *mut *mut c_void, tail: *mut *const c_char) -> c_int;
    fn sqlite3_step(stmt: *mut c_void) -> c_int;
    fn sqlite3_column_text(stmt: *mut c_void, col: c_int) -> *const c_char;
    fn sqlite3_column_int64(stmt: *mut c_void, col: c_int) -> i64;
    fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    fn sqlite3_close(db: *mut c_void) -> c_int;
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_OPEN_READONLY: c_int = 1;

/// Lowercased app id (AppUserModelID) -> what its icon shows: "3", "99+" or "•" (a Store badge glyph).
pub type Notes = HashMap<String, String>;

static NOTES: Mutex<Option<Notes>> = Mutex::new(None);

/// The counts as last read (empty until the first read, or when the database can't be read).
pub fn current() -> Notes {
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// What an app's icon shows: the note of its own id or of one of its windows' AppUserModelIDs, "" for none.
pub fn note_for<'a>(notes: &Notes, id: &'a str, aumids: impl IntoIterator<Item = &'a str>) -> String {
    std::iter::once(id)
        .chain(aumids)
        .filter(|k| !k.is_empty())
        .find_map(|k| notes.get(&k.to_lowercase()))
        .cloned()
        .unwrap_or_default()
}

fn folder() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Microsoft\Windows\Notifications"))
}

/// Reads the counts now and again whenever Windows writes to its notification folder; `changed` runs after a
/// read that came out different (apps.rs then pushes the app lists again).
pub fn watch(changed: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let Some(dir) = folder() else { return };
        let refresh = || {
            let now = read(&dir.join("wpndatabase.db")).unwrap_or_default();
            let mut last = NOTES.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() != Some(&now) {
                *last = Some(now);
                drop(last);
                changed();
            }
        };
        refresh();
        let handle = unsafe {
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
        let Ok(handle) = handle else {
            crate::win::log("notification counts: the notification folder can't be watched");
            return;
        };
        let mut buf = vec![0u32; 1024];
        loop {
            let mut len = 0u32;
            let ok = unsafe {
                ReadDirectoryChangesW(
                    handle,
                    buf.as_mut_ptr() as _,
                    (buf.len() * 4) as u32,
                    false,
                    FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE,
                    Some(&mut len),
                    None,
                    None,
                )
            };
            if ok.is_err() {
                crate::win::log("notification counts: watching the notification folder stopped");
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return;
            }
            // Windows writes the database in a burst (WAL, shm, temp file): read once it has settled.
            std::thread::sleep(Duration::from_millis(400));
            refresh();
        }
    });
}

/// The notification centre's toasts and Store badges per app, read-only.
fn read(db_path: &std::path::Path) -> Option<Notes> {
    let path = CString::new(db_path.to_str()?).ok()?;
    let sql = c"SELECT h.PrimaryId, n.Type, CAST(n.Payload AS TEXT), n.ExpiryTime \
                FROM Notification n JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
                WHERE n.Type IN ('toast', 'badge')";
    let mut rows = Vec::new();
    unsafe {
        let mut db = std::ptr::null_mut();
        let opened = sqlite3_open_v2(path.as_ptr(), &mut db, SQLITE_OPEN_READONLY, std::ptr::null());
        if opened != SQLITE_OK {
            sqlite3_close(db); // also frees a handle that failed to open
            return None;
        }
        sqlite3_busy_timeout(db, 500); // Windows may be writing right now
        let mut stmt = std::ptr::null_mut();
        if sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) == SQLITE_OK {
            let text = |col| {
                let p = sqlite3_column_text(stmt, col);
                if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            while sqlite3_step(stmt) == SQLITE_ROW {
                rows.push(Row { app: text(0), kind: text(1), payload: text(2), expires: sqlite3_column_int64(stmt, 3) });
            }
        }
        sqlite3_finalize(stmt);
        sqlite3_close(db);
    }
    Some(notes(&rows, filetime_now()))
}

struct Row {
    app: String,
    kind: String,
    /// The badge's XML (`<badge value="3"/>`); only read for badges.
    payload: String,
    /// FILETIME; 0 = never.
    expires: i64,
}

fn filetime_now() -> i64 {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (unix.as_nanos() / 100) as i64 + 116_444_736_000_000_000
}

/// Rows -> notes. A Store badge says what the app itself counts (its unread messages), so it wins over the number
/// of toasts; a badge of 0 or "none" clears it. Expired toasts are already gone from the notification centre.
fn notes(rows: &[Row], now: i64) -> Notes {
    let mut toasts: HashMap<String, u32> = HashMap::new();
    let mut badges: Notes = HashMap::new();
    for r in rows {
        let app = r.app.to_lowercase();
        match r.kind.as_str() {
            "toast" if r.expires == 0 || r.expires > now => *toasts.entry(app).or_default() += 1,
            "badge" => {
                if let Some(b) = badge(&r.payload) {
                    badges.insert(app, b);
                }
            }
            _ => {}
        }
    }
    for (app, n) in toasts {
        badges.entry(app).or_insert_with(|| count(n));
    }
    badges
}

/// `<badge value="3"/>` -> "3"; a glyph ("newMessage", "attention", ...) -> "•"; 0, "none" or no value -> None.
fn badge(xml: &str) -> Option<String> {
    let start = xml.find("value=\"")? + 7;
    let value = &xml[start..start + xml[start..].find('"')?];
    match value.parse::<u32>() {
        Ok(0) => None,
        Ok(n) => Some(count(n)),
        Err(_) if value.is_empty() || value == "none" => None,
        Err(_) => Some("•".into()),
    }
}

fn count(n: u32) -> String {
    if n > 99 { "99+".into() } else { n.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(app: &str, kind: &str, payload: &str, expires: i64) -> Row {
        Row { app: app.into(), kind: kind.into(), payload: payload.into(), expires }
    }

    #[test]
    fn counts_toasts_and_badges_per_app() {
        let rows = [
            row("com.squirrel.Discord.Discord", "toast", "", 200),
            row("com.squirrel.Discord.Discord", "toast", "", 0),
            row("com.squirrel.Discord.Discord", "toast", "", 50), // expired
            row("WhatsApp!App", "toast", "", 200),
            row("WhatsApp!App", "badge", r#"<badge value="7"/>"#, 0), // the app's own count wins
            row("Teams!App", "badge", r#"<badge value="0"/>"#, 0),
            row("Mail!App", "badge", r#"<badge value="newMessage"/>"#, 0),
            row("Mail!App", "tile", "", 0),
        ];
        let n = notes(&rows, 100);
        assert_eq!(n.get("com.squirrel.discord.discord").map(String::as_str), Some("2"));
        assert_eq!(n.get("whatsapp!app").map(String::as_str), Some("7"));
        assert_eq!(n.get("mail!app").map(String::as_str), Some("•"));
        assert!(!n.contains_key("teams!app"));
    }

    #[test]
    fn badge_values() {
        assert_eq!(badge(r#"<badge value="150"/>"#).as_deref(), Some("99+"));
        assert_eq!(badge(r#"<badge value="none"/>"#), None);
        assert_eq!(badge("<badge/>"), None);
    }

    #[test]
    fn matches_by_id_or_window_aumid() {
        let notes: Notes = [("whatsapp!app".to_string(), "3".to_string())].into();
        assert_eq!(note_for(&notes, "c:\\whatsapp.exe", ["", "WhatsApp!App"]), "3");
        assert_eq!(note_for(&notes, "WhatsApp!App", []), "3");
        assert_eq!(note_for(&notes, "c:\\other.exe", [""]), "");
    }

    /// Reads the real database of this PC (only counts are printed). `cargo test -- --ignored reads_this_pc`
    #[test]
    #[ignore]
    fn reads_this_pc() {
        let n = read(&folder().unwrap().join("wpndatabase.db")).expect("database readable");
        println!("{} apps with notifications: {:?}", n.len(), n.values().collect::<Vec<_>>());
    }
}
