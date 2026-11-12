//! Live previews: hovering a running app's icon shows a small glass popup above it with a DWM thumbnail of each of
//! the app's windows. One worker thread owns the popup, so callers (Tauri commands, the main thread) never wait.

use crate::apps::AppItem;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::UI::WindowsAndMessaging::IsIconic;

/// One window in the popup. Sizes in CSS px; `body` (relative to the card) is where the thumbnail goes.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Card {
    pub hwnd: isize,
    pub title: String,
    pub active: bool,
    pub rect: [f64; 4],
    pub body: [f64; 4],
    /// DWM draws a live image; false for minimized windows (the page shows the app icon instead).
    pub live: bool,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Preview {
    pub icon: String,
    pub cards: Vec<Card>,
}

enum Cmd {
    /// `left`/`right`: the icon's edges in CSS px inside `bar`.
    Show { bar: WebviewWindow, monitor: RECT, app: AppItem, left: f64, right: f64 },
    Apps { bar: String, apps: Vec<AppItem> },
    Ready(String),
    Hide,
}

static TX: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();
/// What the popup shows (its label, the layout), for the page to fetch once it has loaded.
static SHOWING: Mutex<Option<(String, Preview)>> = Mutex::new(None);
static OPEN: AtomicBool = AtomicBool::new(false);
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Cursor check while the popup is open; it closes once the cursor was off icon and popup this long.
// shortcut: polls the cursor every 50 ms while a preview is open (like auto-hide); a mouse hook would be event-based
// but runs on every mouse move in every app.
const POLL: Duration = Duration::from_millis(50);
const GRACE: Duration = Duration::from_millis(300);

// Layout in CSS px (the page draws the same boxes).
const PAD: f64 = 8.0;
const GAP: f64 = 8.0;
const CARD_W: f64 = 216.0;
const INNER: f64 = 8.0;
const TITLE_H: f64 = 22.0;
const THUMB_H: f64 = 124.0;

pub fn start(app: AppHandle) {
    let (tx, rx) = channel();
    if TX.set(Mutex::new(tx)).is_ok() {
        std::thread::spawn(move || run(app, rx));
    }
}

fn send(cmd: Cmd) {
    if let Some(tx) = TX.get() {
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(cmd);
    }
}

pub fn show(bar: WebviewWindow, monitor: RECT, app: AppItem, left: f64, right: f64) {
    send(Cmd::Show { bar, monitor, app, left, right });
}

/// Always sent (cheap): a Show still queued must not open after this.
pub fn hide() {
    send(Cmd::Hide);
}

/// The bar's apps changed: an open preview follows (titles, closed windows).
pub fn apps_changed(bar: &str, apps: &[AppItem]) {
    if OPEN.load(SeqCst) {
        send(Cmd::Apps { bar: bar.to_string(), apps: apps.to_vec() });
    }
}

/// True while a preview is open (auto-hide keeps the bar up meanwhile).
pub fn is_open() -> bool {
    OPEN.load(SeqCst)
}

/// The popup's page has loaded and asks what to show; it is made visible now, so it never flashes empty.
pub fn ready(label: &str) -> Option<Preview> {
    send(Cmd::Ready(label.to_string()));
    let showing = SHOWING.lock().unwrap_or_else(|e| e.into_inner());
    showing.as_ref().filter(|(l, _)| l == label).map(|(_, p)| p.clone())
}

struct Open {
    bar: WebviewWindow,
    monitor: RECT,
    app: AppItem,
    left: f64,
    right: f64,
    popup: WebviewWindow,
    /// The page has loaded (asked for its content).
    ready: bool,
    visible: bool,
    thumbs: Vec<isize>,
    /// Physical px: the hovered icon and the popup.
    anchor: RECT,
    rect: RECT,
    away_since: Option<Instant>,
}

fn run(app: AppHandle, rx: Receiver<Cmd>) {
    let mut open: Option<Open> = None;
    loop {
        let cmd = if open.is_some() {
            match rx.recv_timeout(POLL) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        match cmd {
            Some(Cmd::Show { bar, monitor, app: item, left, right }) => {
                // Same bar: reuse the popup (no new page). Another bar may have another DPI: start over.
                let reuse = open.take_if(|o| o.bar.label() == bar.label());
                close(&app, open.take());
                open = match reuse {
                    // Hovered the same icon again: already showing it.
                    Some(o) if o.app == item && o.left == left => Some(o),
                    reuse => place(&app, reuse, bar, monitor, item, left, right),
                };
            }
            Some(Cmd::Apps { bar, apps }) => {
                if let Some(o) = open.take_if(|o| o.bar.label() == bar) {
                    match apps.into_iter().find(|a| a.id == o.app.id && !a.windows.is_empty()) {
                        Some(a) if a.windows == o.app.windows => open = Some(o),
                        Some(a) => {
                            let (bar, monitor, left, right) = (o.bar.clone(), o.monitor, o.left, o.right);
                            open = place(&app, Some(o), bar, monitor, a, left, right);
                        }
                        None => close(&app, Some(o)),
                    }
                }
            }
            Some(Cmd::Ready(label)) => {
                if let Some(o) = open.as_mut().filter(|o| o.popup.label() == label) {
                    o.ready = true;
                    reveal(o);
                }
            }
            Some(Cmd::Hide) => close(&app, open.take()),
            None => {}
        }
        if let Some(o) = open.as_mut() {
            let here = crate::win::cursor().is_some_and(|p| inside(p, o.anchor, o.rect));
            if here {
                o.away_since = None;
            } else if o.away_since.get_or_insert_with(Instant::now).elapsed() > GRACE {
                close(&app, open.take());
            }
        }
        OPEN.store(open.is_some(), SeqCst);
    }
}

/// Shows (or updates) the popup for `item` above its icon. None if there is nothing to show.
fn place(
    app: &AppHandle,
    reuse: Option<Open>,
    bar: WebviewWindow,
    monitor: RECT,
    item: AppItem,
    left: f64,
    right: f64,
) -> Option<Open> {
    if item.windows.is_empty() {
        close(app, reuse);
        return None;
    }
    let mut o = match reuse {
        Some(mut o) => {
            unregister(&mut o.thumbs);
            o
        }
        None => {
            let popup = match create(app) {
                Ok(p) => p,
                Err(e) => {
                    crate::win::log(&format!("can't open the preview: {e}"));
                    return None;
                }
            };
            Open {
                bar: bar.clone(),
                monitor,
                app: item.clone(),
                left,
                right,
                popup,
                ready: false,
                visible: false,
                thumbs: Vec::new(),
                anchor: RECT::default(),
                rect: RECT::default(),
                away_since: None,
            }
        }
    };
    let (Ok(dest), Ok(bar_pos), Ok(bar_size)) = (o.popup.hwnd(), bar.outer_position(), bar.outer_size()) else {
        close(app, Some(o));
        return None;
    };
    let dest = HWND(dest.0);
    // Minimized windows have no current image (DWM would show an empty frame): those get the app icon.
    let thumbs: Vec<Option<isize>> = item
        .windows
        .iter()
        .map(|w| {
            let src = HWND(w.hwnd as _);
            if unsafe { IsIconic(src) }.as_bool() {
                return None;
            }
            unsafe { DwmRegisterThumbnail(dest, src) }.ok()
        })
        .collect();
    let sizes: Vec<Option<(f64, f64)>> = thumbs
        .iter()
        .map(|t| {
            let size = unsafe { DwmQueryThumbnailSourceSize((*t)?) }.ok()?;
            (size.cx > 0 && size.cy > 0).then_some((size.cx as f64, size.cy as f64))
        })
        .collect();
    o.thumbs = thumbs.iter().flatten().copied().collect();

    let scale = bar.scale_factor().unwrap_or(1.0);
    let margin = (8.0 * scale).round() as i32;
    let max_w = ((monitor.right - monitor.left - 2 * margin) as f64 / scale).max(CARD_W);
    let (w, h, slots) = layout(&sizes, max_w);
    let (pw, ph) = ((w * scale).round() as i32, (h * scale).round() as i32);
    let anchor = RECT {
        left: bar_pos.x + (left * scale).round() as i32,
        right: bar_pos.x + (right * scale).round() as i32,
        top: bar_pos.y,
        bottom: bar_pos.y + bar_size.height as i32,
    };
    let x = ((anchor.left + anchor.right) / 2 - pw / 2).clamp(monitor.left + margin, (monitor.right - margin - pw).max(monitor.left));
    let y = bar_pos.y - ph - margin;
    let _ = o.popup.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = o.popup.set_position(PhysicalPosition::new(x, y));

    let px = |v: f64| (v * scale).round() as i32;
    for (t, slot) in thumbs.iter().zip(&slots) {
        let (Some(t), Some([tx, ty, tw, th])) = (t, slot.thumb) else { continue };
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT { left: px(tx), top: px(ty), right: px(tx + tw), bottom: px(ty + th) },
            opacity: 255,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
            ..Default::default()
        };
        let _ = unsafe { DwmUpdateThumbnailProperties(*t, &props) };
    }

    let preview = Preview {
        icon: item.icon.clone(),
        cards: item
            .windows
            .iter()
            .zip(slots)
            .map(|(w, s)| Card {
                hwnd: w.hwnd,
                title: w.title.clone(),
                active: w.active,
                rect: s.card,
                body: s.body,
                live: s.thumb.is_some(),
            })
            .collect(),
    };
    let label = o.popup.label().to_string();
    *SHOWING.lock().unwrap_or_else(|e| e.into_inner()) = Some((label.clone(), preview.clone()));
    let _ = app.emit_to(label.as_str(), "preview", &preview);
    (o.bar, o.monitor, o.app, o.left, o.right) = (bar, monitor, item, left, right);
    (o.anchor, o.rect, o.away_since) = (anchor, RECT { left: x, top: y, right: x + pw, bottom: y + ph }, None);
    reveal(&mut o);
    Some(o)
}

/// Shows the popup without taking the focus, once its page is there.
fn reveal(o: &mut Open) {
    if o.ready && !o.visible {
        if let Ok(h) = o.popup.hwnd() {
            crate::win::show_inactive(HWND(h.0));
            o.visible = true;
        }
    }
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let label = format!("preview-{}", COUNTER.fetch_add(1, SeqCst));
    let popup = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Pane")
        .additional_browser_args(crate::BROWSER_ARGS)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .build()?;
    crate::win::set_glass(HWND(popup.hwnd()?.0), crate::GLASS_TINT);
    Ok(popup)
}

/// Thumbnails off, popup gone (destroyed, not hidden: a hidden page keeps ~30 MB, like the tray popup).
fn close(app: &AppHandle, open: Option<Open>) {
    let Some(mut o) = open else { return };
    unregister(&mut o.thumbs);
    let _ = o.popup.destroy();
    *SHOWING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let _ = app.emit_to(o.bar.label(), "preview-closed", ());
}

fn unregister(thumbs: &mut Vec<isize>) {
    for t in thumbs.drain(..) {
        let _ = unsafe { DwmUnregisterThumbnail(t) };
    }
}

/// The cursor counts as "still here" over the icon, the popup, and the gap between them.
fn inside((x, y): (i32, i32), anchor: RECT, popup: RECT) -> bool {
    let within = |r: RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    let bridge = RECT { left: anchor.left.max(popup.left), right: anchor.right.min(popup.right), top: popup.bottom, bottom: anchor.top };
    within(anchor) || within(popup) || within(bridge)
}

#[derive(Debug, PartialEq)]
struct Slot {
    /// x, y, w, h in the popup.
    card: [f64; 4],
    /// The thumbnail area, relative to the card.
    body: [f64; 4],
    /// The live image, relative to the popup (aspect kept, centred in the body), if the window has one.
    thumb: Option<[f64; 4]>,
}

/// Cards side by side for windows of the given source sizes; shrunk to fit `max_w`. Returns (width, height, slots).
// shortcut: one row only; dozens of windows get small cards. A second row would need a smarter layout.
fn layout(sizes: &[Option<(f64, f64)>], max_w: f64) -> (f64, f64, Vec<Slot>) {
    let n = sizes.len() as f64;
    let natural = 2.0 * PAD + n * CARD_W + (n - 1.0).max(0.0) * GAP;
    let s = (max_w / natural).min(1.0);
    let card_h = 2.0 * INNER + TITLE_H + THUMB_H;
    let (bw, bh) = (CARD_W - 2.0 * INNER, THUMB_H);
    let slots = sizes
        .iter()
        .enumerate()
        .map(|(i, size)| {
            let (x, y) = (PAD + i as f64 * (CARD_W + GAP), PAD);
            let (bx, by) = (INNER, INNER + TITLE_H);
            let thumb = size.map(|(sw, sh)| {
                let k = (bw / sw).min(bh / sh).min(1.0);
                let (tw, th) = (sw * k, sh * k);
                [x + bx + (bw - tw) / 2.0, y + by + (bh - th) / 2.0, tw, th].map(|v| v * s)
            });
            Slot { card: [x, y, CARD_W, card_h].map(|v| v * s), body: [bx, by, bw, bh].map(|v| v * s), thumb }
        })
        .collect();
    (natural * s, (2.0 * PAD + card_h) * s, slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_keep_aspect_and_fit() {
        let (w, h, slots) = layout(&[Some((1600.0, 900.0)), None], 10_000.0);
        assert_eq!((w, h), (2.0 * PAD + 2.0 * CARD_W + GAP, 2.0 * PAD + 2.0 * INNER + TITLE_H + THUMB_H));
        // 16:9 in a 200 x 124 box: full width, centred vertically.
        let [x, y, tw, th] = slots[0].thumb.unwrap();
        assert_eq!((x, tw, th), (PAD + INNER, 200.0, 112.5));
        assert_eq!(y, PAD + INNER + TITLE_H + (124.0 - 112.5) / 2.0);
        assert_eq!(slots[1].card[0], PAD + CARD_W + GAP);
        assert_eq!(slots[1].thumb, None, "minimized: no live image");
        // Small windows aren't blown up.
        assert_eq!(layout(&[Some((100.0, 50.0))], 10_000.0).2[0].thumb.unwrap()[2..], [100.0, 50.0]);
        // Too many for the screen: everything shrinks to fit.
        let (w, _, slots) = layout(&vec![Some((800.0, 600.0)); 20], 1000.0);
        assert!((w - 1000.0).abs() < 1e-9 && slots[19].card[0] + slots[19].card[2] < 1000.0);
    }

    #[test]
    fn cursor_may_cross_the_gap() {
        let anchor = RECT { left: 100, top: 500, right: 140, bottom: 548 };
        let popup = RECT { left: 0, top: 300, right: 300, bottom: 490 };
        assert!(inside((120, 520), anchor, popup)); // on the icon
        assert!(inside((10, 310), anchor, popup)); // on the popup
        assert!(inside((120, 495), anchor, popup)); // between them
        assert!(!inside((10, 495), anchor, popup)); // beside the gap
        assert!(!inside((200, 520), anchor, popup)); // next icon
    }
}
