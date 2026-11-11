//! "Now Playing" from Windows' media controls (GSMTC): Spotify, browsers, any app that shows in the volume flyout.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props, GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct NowPlaying {
    /// Source app id ("Spotify.exe" or an AppUserModelID).
    pub app: String,
    pub title: String,
    pub artist: String,
    /// Data URL of the thumbnail as the app gives it (PNG or JPEG), "" if none.
    pub cover: String,
    pub playing: bool,
    /// Position at `updated_at_ms`.
    pub position_ms: u64,
    /// 0 if unknown.
    pub duration_ms: u64,
    /// Unix ms when the position was sampled, so the UI can count on while playing.
    pub updated_at_ms: u64,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Toggle,
    Next,
    Previous,
}

/// Latest state while `watch` runs, so `current()` doesn't query the apps.
static CACHE: OnceLock<Mutex<Option<NowPlaying>>> = OnceLock::new();
/// Last cover: (app|title|artist, data URL). Thumbnails are slow to read.
static COVER: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));

/// The current media session (the one Windows' flyout shows), None if nothing plays or is paused.
pub fn current() -> Option<NowPlaying> {
    match CACHE.get() {
        Some(cache) => cache.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        None => read(),
    }
}

/// Play/pause, next, previous on the current session. Doesn't wait for the app.
pub fn control(action: Action) {
    let Some(session) = manager().and_then(|m| m.GetCurrentSession().ok()) else { return };
    let _ = match action {
        Action::Toggle => session.TryTogglePlayPauseAsync(),
        Action::Next => session.TrySkipNextAsync(),
        Action::Previous => session.TrySkipPreviousAsync(),
    };
}

/// Calls `on_change` (debounced ~150 ms, from a background thread) when the current session, its track,
/// play state or position changes. Follows the current session when it switches apps.
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) {
    if CACHE.set(Mutex::new(read())).is_err() {
        return; // already watching
    }
    let Some(manager) = manager() else { return };
    // true = the current session changed, subscribe to the new one.
    let (tx, rx) = channel::<bool>();
    let session_tx = tx.clone();
    let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = session_tx.send(true);
        Ok(())
    }));
    let _ = tx.send(true);
    std::thread::spawn(move || {
        mta();
        let mut subscribed: Option<(Session, [i64; 3])> = None;
        while let Ok(first) = rx.recv() {
            std::thread::sleep(Duration::from_millis(150));
            let switched = rx.try_iter().fold(first, |a, b| a || b);
            if switched {
                if let Some((old, tokens)) = subscribed.take() {
                    let _ = old.RemoveMediaPropertiesChanged(tokens[0]);
                    let _ = old.RemovePlaybackInfoChanged(tokens[1]);
                    let _ = old.RemoveTimelinePropertiesChanged(tokens[2]);
                }
                subscribed = manager.GetCurrentSession().ok().and_then(|s| subscribe(&s, &tx).map(|t| (s, t)));
            }
            *CACHE.get().unwrap().lock().unwrap_or_else(|e| e.into_inner()) = read();
            on_change();
        }
    });
}

fn subscribe(session: &Session, tx: &Sender<bool>) -> Option<[i64; 3]> {
    fn notify<A: windows::core::RuntimeType + 'static>(tx: &Sender<bool>) -> TypedEventHandler<Session, A> {
        let tx = tx.clone();
        TypedEventHandler::new(move |_, _| {
            let _ = tx.send(false);
            Ok(())
        })
    }
    Some([
        session.MediaPropertiesChanged(&notify(tx)).ok()?,
        session.PlaybackInfoChanged(&notify(tx)).ok()?,
        session.TimelinePropertiesChanged(&notify(tx)).ok()?,
    ])
}

/// WinRT needs COM on the calling thread; an existing STA (Tauri's main thread) is fine too, the objects are agile.
fn mta() {
    thread_local!(static INIT: () = unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); });
    INIT.with(|_| {});
}

/// Kept for the whole run so the event registrations stay alive.
fn manager() -> Option<&'static Manager> {
    static MANAGER: OnceLock<Option<Manager>> = OnceLock::new();
    mta();
    MANAGER.get_or_init(|| Manager::RequestAsync().and_then(|op| op.join()).ok()).as_ref()
}

/// Queries the current session live (blocking, a few ms; the cover only when the track changed).
fn read() -> Option<NowPlaying> {
    let session = manager()?.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let app = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let status = session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).unwrap_or(PlaybackStatus::Closed);
    let playing = status == PlaybackStatus::Playing;

    let now = unix_ms(SystemTime::now());
    let (mut position_ms, mut duration_ms) = (0, 0);
    if let Ok(t) = session.GetTimelineProperties() {
        let (start, end) = (t.StartTime().map_or(0, |t| t.Duration), t.EndTime().map_or(0, |t| t.Duration));
        duration_ms = ticks_to_ms(end - start);
        let sampled = t.LastUpdatedTime().map_or(0, |d| filetime_to_unix_ms(d.UniversalTime));
        position_ms = position_at(ticks_to_ms(t.Position().map_or(0, |p| p.Duration)), sampled, now, playing, duration_ms);
    }

    let key = format!("{app}|{title}|{artist}");
    let cached = { let c = COVER.lock().unwrap_or_else(|e| e.into_inner()); (c.0 == key).then(|| c.1.clone()) };
    let cover = cached.unwrap_or_else(|| {
        let url = cover(&props).unwrap_or_default();
        // shortcut: an empty cover isn't cached, so a missing thumbnail is retried on each event; fine while events are rare.
        if !url.is_empty() {
            *COVER.lock().unwrap_or_else(|e| e.into_inner()) = (key, url.clone());
        }
        url
    });
    Some(NowPlaying { app, title, artist, cover, playing, position_ms, duration_ms, updated_at_ms: now })
}

fn cover(props: &Props) -> Option<String> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    let n = reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0; n as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{b64}", mime_of(&bytes)))
}

/// Thumbnails are PNG or JPEG; the stream's ContentType is often empty, so look at the bytes.
fn mime_of(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") { "image/png" } else { "image/jpeg" }
}

/// WinRT TimeSpan (100 ns ticks) to ms; negative becomes 0.
fn ticks_to_ms(ticks: i64) -> u64 {
    (ticks.max(0) / 10_000) as u64
}

/// WinRT DateTime (100 ns ticks since 1601) to unix ms; 0 stays 0 (unknown).
fn filetime_to_unix_ms(ticks: i64) -> u64 {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    if ticks <= EPOCH_DIFF { 0 } else { ((ticks - EPOCH_DIFF) / 10_000) as u64 }
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Apps report the position only when it jumps (track start, seek), so count on from when they sampled it.
fn position_at(position_ms: u64, sampled_ms: u64, now_ms: u64, playing: bool, duration_ms: u64) -> u64 {
    let pos = if playing && sampled_ms > 0 { position_ms + now_ms.saturating_sub(sampled_ms) } else { position_ms };
    if duration_ms > 0 { pos.min(duration_ms) } else { pos }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_conversions() {
        assert_eq!(ticks_to_ms(12_345_678), 1234);
        assert_eq!(ticks_to_ms(-5), 0);
        assert_eq!(filetime_to_unix_ms(116_444_736_000_000_000 + 20_000), 2);
        assert_eq!(filetime_to_unix_ms(0), 0);
    }

    #[test]
    fn position_counts_on_while_playing() {
        assert_eq!(position_at(10_000, 1_000, 4_000, true, 0), 13_000);
        assert_eq!(position_at(10_000, 1_000, 4_000, false, 0), 10_000);
        assert_eq!(position_at(10_000, 0, 4_000, true, 0), 10_000); // unknown sample time
        assert_eq!(position_at(10_000, 1_000, 900_000, true, 200_000), 200_000); // clamped to the end
    }

    #[test]
    fn cover_type() {
        assert_eq!(mime_of(b"\x89PNG\r\n"), "image/png");
        assert_eq!(mime_of(b"\xff\xd8\xff"), "image/jpeg");
    }

    #[test]
    #[ignore]
    fn media_live() {
        match current() {
            Some(n) => println!(
                "app={} title={:?} artist={:?} playing={} position={}ms duration={}ms cover={} chars ({})",
                n.app, n.title, n.artist, n.playing, n.position_ms, n.duration_ms, n.cover.len(), &n.cover[..n.cover.len().min(22)]
            ),
            None => println!("nothing playing"),
        }
    }
}
