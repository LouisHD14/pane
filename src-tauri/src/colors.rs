//! Colours for the adaptive tint and the app-colour floor: the bottom edge of the app that sits on the bar.
//! Decided with the user 2026-11-12: the top app window on each monitor, only when it reaches down to the
//! bar (else the normal glass); at most 3 calm zones (sidebar / content), small details ignored; a colour is
//! only taken once it stays the same for ~2 s, so a playing video never makes the bar flicker.

use std::mem::size_of;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::IsIconic;

pub type Rgb = [u8; 3];

/// Columns the app's bottom edge is cut into before they are grouped into zones.
const CELLS: usize = 32;
/// At most this many zones; smaller ones than `MIN_ZONE` cells are folded into a neighbour.
const MAX_ZONES: usize = 3;
const MIN_ZONE: usize = 5; // ~15 % of the width
/// Two colours count as the same when no channel differs by more than this.
const SAME: u8 = 24;
/// Rows read above the app's bottom edge. Browsers end in a 2-3 px grey frame line; over 12 rows the page's
/// own colour wins (with 3 rows the line won: black Netflix and GitHub's dark page came out grey, 2026-11-12).
const ROWS: i32 = 12;
/// Readings in a row that must agree before a new colour of the same app is shown (one every 0.5 s: ~1-1.5 s).
/// Another app on the bar, or a bar that is away, takes its colour at once (Steady::feed).
const STEADY: u32 = 3;

/// The top app window on `monitor` (0 = none) and its 8 band colours (left to right across `strip`),
/// or None when it doesn't reach down to the bar.
pub fn app_floor(monitor: RECT, strip: RECT, bands: usize) -> (isize, Option<Vec<Rgb>>) {
    let app = crate::apps::taskbar_hwnds().into_iter().find(|&h| {
        let b = bounds(h);
        // On this monitor (by its centre) and not minimized.
        b.is_some_and(|b| contains(monitor, (b.left + b.right) / 2, (b.top + b.bottom) / 2)) && !unsafe { IsIconic(h) }.as_bool()
    });
    let id = app.map_or(0, |h| h.0 as isize);
    (id, app.and_then(|app| floor_of(app, monitor, strip, bands)))
}

fn floor_of(app: HWND, monitor: RECT, strip: RECT, bands: usize) -> Option<Vec<Rgb>> {
    let b = bounds(app)?;
    if b.bottom < strip.top - 2 {
        return None; // a small window: the bar shows the normal glass
    }
    // Its own bottom rows, but never under the bar (an auto-hidden bar floats over the app).
    let (x0, x1, y) = (b.left.max(monitor.left), b.right.min(monitor.right), b.bottom.min(strip.top));
    let rows = ROWS.min(b.bottom - b.top);
    let (w, h, px) = crate::win::screen_pixels(x0, y - rows, x1 - x0, rows)?;
    let cells: Vec<Rgb> = (0..CELLS).map(|i| dominant(&px, w, h, i * w / CELLS, ((i + 1) * w / CELLS).max(i * w / CELLS + 1))).collect();
    Some(to_bands(&zones(&cells), x0, x1, strip.left, strip.right, bands))
}

/// The visible frame (without the invisible resize borders, which stick out of maximized windows).
fn bounds(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as _, size_of::<RECT>() as u32) }.ok()?;
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

fn contains(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn close(a: Rgb, b: Rgb, limit: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= limit)
}

/// The most common colour in columns x0..x1 of BGRA pixels (w × h), as a real colour that is on screen.
/// (Per-channel medians mixed channels of different pixels into colours that aren't there: purple tints.)
fn dominant(px: &[u8], w: usize, h: usize, x0: usize, x1: usize) -> Rgb {
    let mut counts: std::collections::HashMap<u16, (u32, [u32; 3])> = Default::default();
    for y in 0..h {
        for x in x0..x1.min(w) {
            let p = &px[(y * w + x) * 4..];
            let rgb = [p[2], p[1], p[0]];
            let key = (rgb[0] as u16 >> 4) << 8 | (rgb[1] as u16 >> 4) << 4 | rgb[2] as u16 >> 4;
            let e = counts.entry(key).or_default();
            e.0 += 1;
            (0..3).for_each(|i| e.1[i] += rgb[i] as u32);
        }
    }
    counts.into_values().max_by_key(|e| e.0).map_or([0; 3], |(n, s)| s.map(|v| (v / n.max(1)) as u8))
}

/// A run of cells `start..end` with its (mean) colour.
type Zone = (usize, usize, Rgb);

fn mean(cells: &[Rgb], start: usize, end: usize) -> Rgb {
    let n = (end - start).max(1) as u32;
    let mut s = [0u32; 3];
    for c in &cells[start..end] {
        (0..3).for_each(|i| s[i] += c[i] as u32);
    }
    s.map(|v| (v / n) as u8)
}

/// Neighbouring cells of a similar colour become one zone; small zones fold into their closest neighbour
/// until there are at most `MAX_ZONES`.
fn zones(cells: &[Rgb]) -> Vec<Zone> {
    let mut z: Vec<Zone> = Vec::new();
    for (i, &c) in cells.iter().enumerate() {
        match z.last_mut() {
            Some(last) if close(last.2, c, SAME) => {
                last.1 = i + 1;
                last.2 = mean(cells, last.0, last.1);
            }
            _ => z.push((i, i + 1, c)),
        }
    }
    loop {
        // Neighbours that became alike (a detail between them was folded away) join first.
        if let Some(i) = (1..z.len()).find(|&i| close(z[i - 1].2, z[i].2, SAME)) {
            z[i - 1] = (z[i - 1].0, z[i].1, mean(cells, z[i - 1].0, z[i].1));
            z.remove(i);
            continue;
        }
        let too_many = z.len() > MAX_ZONES;
        let smallest = (0..z.len()).min_by_key(|&i| z[i].1 - z[i].0);
        let Some(i) = smallest.filter(|&i| too_many || z[i].1 - z[i].0 < MIN_ZONE) else { break };
        if z.len() == 1 {
            break;
        }
        // Into the neighbour with the closer colour (the only one at the ends).
        let diff = |j: usize| (0..3).map(|k| z[i].2[k].abs_diff(z[j].2[k]) as u32).sum::<u32>();
        let j = match (i.checked_sub(1), (i + 1 < z.len()).then_some(i + 1)) {
            (Some(l), Some(r)) => if diff(l) <= diff(r) { l } else { r },
            (Some(l), None) => l,
            (None, Some(r)) => r,
            (None, None) => break,
        };
        let (a, b) = (i.min(j), i.max(j));
        z[a] = (z[a].0, z[b].1, mean(cells, z[a].0, z[b].1));
        z.remove(b);
    }
    z
}

/// The zones (measured over the app's x0..x1) as `n` band colours across the strip; parts of the strip
/// beside the app take the nearest zone.
fn to_bands(z: &[Zone], x0: i32, x1: i32, left: i32, right: i32, n: usize) -> Vec<Rgb> {
    (0..n)
        .map(|i| {
            let x = left as f64 + (i as f64 + 0.5) * (right - left) as f64 / n as f64;
            let cell = ((x - x0 as f64) / (x1 - x0).max(1) as f64 * CELLS as f64).clamp(0.0, CELLS as f64 - 1.0) as usize;
            z.iter().find(|z| cell < z.1).or(z.last()).map_or([0; 3], |z| z.2)
        })
        .collect()
}

/// Shows a colour only once it held still for `STEADY` readings; "no app" (None) counts the same way.
/// A different app window, or `at_once` (the bar is slid away, so nobody sees a flicker), skips the wait:
/// switching apps recolours the bar right away, and a bar that comes up already has the right colours.
#[derive(Default)]
pub struct Steady {
    shown: Option<Option<Vec<Rgb>>>,
    candidate: Option<Option<Vec<Rgb>>>,
    count: u32,
    app: isize,
}

impl Steady {
    /// Feeds one reading of window `app`; returns what to show when that changes.
    pub fn feed(&mut self, app: isize, now: Option<Vec<Rgb>>, at_once: bool) -> Option<Option<Vec<Rgb>>> {
        let switched = std::mem::replace(&mut self.app, app) != app;
        let same = |a: &Option<Vec<Rgb>>, b: &Option<Vec<Rgb>>| match (a, b) {
            (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(*x, *y, SAME / 2)),
            (None, None) => true,
            _ => false,
        };
        if self.candidate.as_ref().is_some_and(|c| same(c, &now)) {
            self.count += 1;
        } else {
            (self.candidate, self.count) = (Some(now.clone()), 1);
        }
        if switched || at_once {
            self.count = STEADY; // this reading counts as settled
        }
        if self.count < STEADY || self.shown.as_ref().is_some_and(|s| same(s, &now)) {
            return None;
        }
        self.shown = Some(now.clone());
        Some(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK: Rgb = [32, 32, 32];
    const LIGHT: Rgb = [240, 238, 230];

    #[test]
    fn sidebar_and_content_are_two_zones_and_details_vanish() {
        let mut cells = vec![DARK; 8];
        cells.extend(vec![LIGHT; 24]);
        cells[20] = [200, 40, 40]; // a red button in the content
        let z = zones(&cells);
        assert_eq!(z.len(), 2, "{z:?}");
        assert_eq!((z[0].0, z[0].1), (0, 8));
        assert!(close(z[1].2, LIGHT, 12));
        let b = to_bands(&z, 0, 3200, 0, 3200, 8);
        assert!(close(b[0], DARK, 0) && close(b[1], DARK, 0) && close(b[7], LIGHT, 12));
    }

    #[test]
    fn a_gradient_becomes_at_most_three_zones() {
        let cells: Vec<Rgb> = (0..32).map(|i| [(i * 8) as u8, 0, 255 - (i * 8) as u8]).collect();
        let z = zones(&cells);
        assert!(z.len() <= MAX_ZONES && z.len() >= 2, "{z:?}");
    }

    #[test]
    fn dominant_takes_a_real_colour_not_a_channel_mix() {
        // Half orange, half white: a per-channel median gave a colour that is neither.
        let (w, h) = (10, 1);
        let mut px = Vec::new();
        for x in 0..w {
            let rgb: Rgb = if x < 6 { [230, 120, 40] } else { [255, 255, 255] };
            px.extend([rgb[2], rgb[1], rgb[0], 255]);
        }
        assert_eq!(dominant(&px, w, h, 0, w), [230, 120, 40]);
    }

    #[test]
    fn only_steady_colours_are_shown() {
        let mut s = Steady::default();
        let a = Some(vec![DARK; 8]);
        let b = Some(vec![LIGHT; 8]);
        // The first app on the bar: at once.
        assert_eq!(s.feed(1, a.clone(), false), Some(a.clone()));
        assert_eq!(s.feed(1, a.clone(), false), None, "unchanged: nothing to send");
        // A video in the same app: a different colour every reading never gets through.
        for i in 0..10u8 {
            assert_eq!(s.feed(1, Some(vec![[i * 25, 255 - i * 25, 90]; 8]), false), None);
        }
        // The same app changing colour for good: once it held still.
        assert_eq!(s.feed(1, b.clone(), false), None);
        assert_eq!(s.feed(1, b.clone(), false), None);
        assert_eq!(s.feed(1, b.clone(), false), Some(b.clone()));
        // Another app: at once.
        assert_eq!(s.feed(2, a.clone(), false), Some(a.clone()));
        // The bar is slid away: at once too, so it comes up in the right colours.
        assert_eq!(s.feed(2, b.clone(), true), Some(b.clone()));
        // No app on the bar any more: the glass, at once (the window changed).
        assert_eq!(s.feed(0, None, false), Some(None));
    }
}
