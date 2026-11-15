//! Colours for the adaptive tint and the app-colour floor: the bottom edge of the app that sits on the bar.
//! Decided with the user 2026-11-12: only apps that reach down to the bar colour it (else the normal glass);
//! at most 3 calm zones per app (sidebar / content), small details ignored; a colour within one app is only
//! taken once it stays the same for a moment, so a playing video never makes the bar flicker.
//! Since 2026-11-14 each part of the bar takes the app that is really on top just above it: a small window
//! touching the bar colours only the part under it, the rest stays glass (it used to colour the whole bar).

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
/// Readings in a row that must agree before a new colour of the same app is shown (one every 0.75 s while nothing moves: ~1.5 s; a video never gets through).
/// Another app on the bar, or a bar that is away, takes its colour at once (Steady::feed).
const STEADY: u32 = 2;

/// Which app window is on top just above each cell of the bar (0: none, the desktop), with the rects of
/// those windows. Cheap (~0.5 ms: the window list, no pixels), so it is checked often; the bar is only read
/// again (`read`, ~20 ms) when this changes, or now and then for colour changes inside an app.
#[derive(PartialEq, Default, Debug)]
pub struct Layout {
    owner: Vec<isize>,
    rects: Vec<(isize, RECT)>,
}

impl Layout {
    /// The windows on the bar, left to right (for Steady::feed: another app recolours at once).
    pub fn ids(&self) -> Vec<isize> {
        let mut ids: Vec<isize> = self.owner.iter().copied().filter(|&id| id != 0).collect();
        ids.dedup();
        ids
    }
}

pub fn layout(strip: RECT) -> Layout {
    // In z-order, top first.
    let windows: Vec<(isize, RECT)> = crate::apps::taskbar_hwnds()
        .into_iter()
        .filter(|&h| !unsafe { IsIconic(h) }.as_bool())
        .filter_map(|h| bounds(h).map(|b| (h.0 as isize, b)))
        .collect();
    let owner = on_top(&windows, strip.left, strip.right, strip.top - 1 - TOUCH);
    let rects = windows.into_iter().filter(|w| owner.contains(&w.0)).collect();
    Layout { owner, rects }
}

/// What sits on the bar, part by part: `bands` colours left to right across `strip`, each the colour of the app
/// that is on top just above that part of the bar (`layout`), or None where no app reaches down to it (the
/// desktop: the normal glass). All None comes back as None.
pub fn read(strip: RECT, layout: &Layout, bands: usize) -> Option<Bands> {
    let (left, right, owner) = (strip.left, strip.right, &layout.owner);
    if owner.iter().all(|&id| id == 0) {
        return None; // nothing touches the bar: no need to read the screen
    }
    // One read of the rows just above the bar, all across it: reading the screen is the costly part (~20 ms
    // each however small, DWM copies the frame back), so never once per window. Apps that touch the bar end at
    // most TOUCH px above it, so these rows are theirs; an auto-hidden bar floats over the app, so they are
    // above the bar.
    let (w, h, px) = crate::win::screen_pixels(left, strip.top - ROWS, right - left, ROWS)?;
    let col = |x: i32| (((x - left).max(0) as usize) * w / (right - left).max(1) as usize).min(w);
    let mut cells: Vec<Option<Rgb>> = vec![None; CELLS];
    // One run of cells per window, grouped into its own calm zones.
    let mut i = 0;
    while i < CELLS {
        let id = owner[i];
        let end = (i..CELLS).find(|&j| owner[j] != id).unwrap_or(CELLS);
        if id != 0 {
            let run: Vec<Rgb> = (i..end)
                .map(|k| {
                    let (x0, x1) = (col(cell_x(left, right, k)), col(cell_x(left, right, k + 1)));
                    dominant(&px, w, h, x0, x1.max(x0 + 1))
                })
                .collect();
            for z in zones(&run) {
                (z.0..z.1).for_each(|k| cells[i + k] = Some(z.2));
            }
        }
        i = end;
    }
    to_bands(&cells, bands)
}

/// Px above the bar a window's bottom may end and still count as touching it.
const TOUCH: i32 = 2;

/// The left edge of cell `i` of `CELLS` across left..right (i = CELLS: the right edge).
fn cell_x(left: i32, right: i32, i: usize) -> i32 {
    left + ((right - left) as i64 * i as i64 / CELLS as i64) as i32
}

/// For each cell across left..right, the topmost window (of `windows`, top first) covering the cell's centre
/// at height `y`, or 0 for none.
fn on_top(windows: &[(isize, RECT)], left: i32, right: i32, y: i32) -> Vec<isize> {
    (0..CELLS)
        .map(|i| {
            let x = (cell_x(left, right, i) + cell_x(left, right, i + 1)) / 2;
            windows.iter().find(|(_, r)| contains(*r, x, y)).map_or(0, |w| w.0)
        })
        .collect()
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

/// The cells as `n` band colours, each band taking the cell at its middle. None when no cell has an app.
fn to_bands(cells: &[Option<Rgb>], n: usize) -> Option<Bands> {
    cells.iter().any(Option::is_some).then(|| (0..n).map(|i| cells[((2 * i + 1) * cells.len() / (2 * n)).min(cells.len() - 1)]).collect())
}

/// One reading: a colour per band, None where the bar shows plain glass.
pub type Bands = Vec<Option<Rgb>>;

/// Shows a colour only once it held still for `STEADY` readings; "no app" (None) counts the same way.
/// A different app window, or `at_once` (the bar is slid away, so nobody sees a flicker), skips the wait:
/// switching apps recolours the bar right away, and a bar that comes up already has the right colours.
#[derive(Default)]
pub struct Steady {
    shown: Option<Option<Bands>>,
    candidate: Option<Option<Bands>>,
    count: u32,
    apps: Vec<isize>,
}

impl Steady {
    /// Feeds one reading of the windows `apps`; returns what to show when that changes.
    pub fn feed(&mut self, apps: Vec<isize>, now: Option<Bands>, at_once: bool) -> Option<Option<Bands>> {
        let switched = std::mem::replace(&mut self.apps, apps.clone()) != apps;
        let same = |a: &Option<Bands>, b: &Option<Bands>| match (a, b) {
            (Some(a), Some(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(x, y)| match (x, y) {
                        (Some(x), Some(y)) => close(*x, *y, SAME / 2),
                        (None, None) => true,
                        _ => false,
                    })
            }
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
        let mut c = vec![None; CELLS];
        z.iter().for_each(|z| (z.0..z.1).for_each(|k| c[k] = Some(z.2)));
        let b = to_bands(&c, 8).unwrap();
        assert!(close(b[0].unwrap(), DARK, 0) && close(b[1].unwrap(), DARK, 0) && close(b[7].unwrap(), LIGHT, 12));
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
    fn a_small_window_colours_only_the_part_of_the_bar_under_it() {
        let r = |l, t, r, b| RECT { left: l, top: t, right: r, bottom: b };
        // The bar is 0..3200 at y 1000. A small window touching it over 800..1600 on top of a maximized one that
        // ends exactly at the bar (left half), and a window floating above that touches nothing.
        let floating = (3, r(0, 100, 3200, 700));
        let small = (2, r(800, 600, 1600, 1000));
        let maximized = (1, r(0, 0, 1600, 1000));
        let owner = on_top(&[floating, small, maximized], 0, 3200, 1000 - 1 - TOUCH);
        assert_eq!(owner[0], 1, "left: the maximized window");
        assert_eq!(owner[9], 2, "middle: the small window on top of it");
        assert_eq!(owner[CELLS - 1], 0, "right: nothing reaches the bar, plain glass");
        // A window ending 10 px above the bar doesn't touch it.
        assert_eq!(on_top(&[(4, r(0, 0, 3200, 990))], 0, 3200, 1000 - 1 - TOUCH), vec![0; CELLS]);
        // Bands: glass (None) where no app is, and no colours at all without one.
        let mut cells = vec![None; CELLS];
        (0..8).for_each(|k| cells[k] = Some(DARK));
        let b = to_bands(&cells, 16).unwrap();
        assert_eq!((b[0], b[3], b[4], b[15]), (Some(DARK), Some(DARK), None, None));
        assert_eq!(to_bands(&vec![None; CELLS], 16), None);
    }

    #[test]
    fn only_steady_colours_are_shown() {
        let mut s = Steady::default();
        let a = Some(vec![Some(DARK); 8]);
        let b = Some(vec![Some(LIGHT); 8]);
        // The first app on the bar: at once.
        assert_eq!(s.feed(vec![1], a.clone(), false), Some(a.clone()));
        assert_eq!(s.feed(vec![1], a.clone(), false), None, "unchanged: nothing to send");
        // A video in the same app: a different colour every reading never gets through.
        for i in 0..10u8 {
            assert_eq!(s.feed(vec![1], Some(vec![Some([i * 25, 255 - i * 25, 90]); 8]), false), None);
        }
        // The same app changing colour for good: once it held still.
        assert_eq!(s.feed(vec![1], b.clone(), false), None);
        assert_eq!(s.feed(vec![1], b.clone(), false), Some(b.clone()));
        // Another app (or one more on the bar): at once.
        assert_eq!(s.feed(vec![2], a.clone(), false), Some(a.clone()));
        let half = Some([vec![Some(DARK); 4], vec![None; 4]].concat());
        assert_eq!(s.feed(vec![2, 3], half.clone(), false), Some(half));
        // The bar is slid away: at once too, so it comes up in the right colours.
        assert_eq!(s.feed(vec![2, 3], b.clone(), true), Some(b.clone()));
        // No app on the bar any more: the glass, at once (the windows changed).
        assert_eq!(s.feed(vec![], None, false), Some(None));
    }
}

#[cfg(test)]
mod timing {
    /// How long one colour check takes on this screen (run by hand: cargo test --release timing -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn one_check() {
        use windows::Win32::Foundation::RECT;
        let strip = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
        let t = std::time::Instant::now();
        for _ in 0..20 {
            let _ = super::read(strip, &super::layout(strip), 16);
        }
        let all = t.elapsed() / 20;
        let t = std::time::Instant::now();
        for _ in 0..20 {
            let _ = crate::apps::taskbar_hwnds();
        }
        let enumerate = t.elapsed() / 20;
        let t = std::time::Instant::now();
        for _ in 0..20 {
            let _ = crate::win::screen_pixels(0, 1020, 1920, 12);
        }
        println!("check {all:?}, of it: windows {enumerate:?}, screen read {:?}", t.elapsed() / 20);
    }
}
