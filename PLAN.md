# Pane – Plan

**Pane** is an own widget bar for Windows 11 that replaces the taskbar, inspired by lumabar.xyz (own design and name, not a copy). Written 2026-11-11, answers from the user the same day.

## Goal
Everything the taskbar does today keeps working, plus widgets:
- Windows/Start button
- Pinned favorites + running apps
- Tray icons (Discord, Steam, NVIDIA, …): **required**
- Spotify (Now Playing with controls)
- Claude usage (limits for claude.ai / Claude Code)
- Date, weekday, time, weather
- CPU/GPU/RAM usage, per-app volume (Windows mixer), network speed (Phase 6)

Budget: under ~100 MB RAM and ~0 % CPU when idle. If Pane crashes or quits, the normal taskbar must come back.

## Layout (decided 2026-11-11)
- Bottom edge, **on every monitor**.
- Glass theme. Start + favorites/running apps **centered**, like Windows 11.
- Right side: a glass block with the date and weekday, the time to the right of them, and the temperature with a weather icon below.

```
┌───────────────────────────────────────────────────────────────────────────┐
│ ♫ Song – Artist ⏯      [⊞] [app] [app] [app] [app]     ˄ tray  ┌───────────────────┐ │
│ Claude ▮▮▮▯ 62 %                                                │ Wed 11 Nov   16:45 │ │
│                                                                 │ ☁ 8°               │ │
│                                                                 └───────────────────┘ │
└───────────────────────────────────────────────────────────────────────────┘
```
Left side holds Spotify and Claude usage (a guess, open to change). Exact sizes come after Phase 1 on the real screen.

## Stack
- **Tauri 2** (Rust backend + web UI, same approach as LumaBar). UI in TypeScript + plain CSS; a framework only if the UI gets big.
- Rust `windows` crate for Win32/WinRT, `sysinfo` for CPU/RAM.
- Glass: the Windows Acrylic/Mica window effect (built into Tauri), no fake blur.
- Repo: private `LouisHD14/pane`; the folder stays `desktop/`.

## Languages and tools
| Part | Language / tool | Why |
| --- | --- | --- |
| Backend (Windows calls: taskbar, tray, windows, media, CPU/GPU) | **Rust** | Fast, small RAM, direct access to Windows APIs via Microsoft's `windows` crate |
| Bar UI (what you see) | **TypeScript + HTML/CSS** | Easy to make the glass design look good |
| Glue between both | **Tauri 2** | Uses the Edge WebView2 already in Windows 11, so no browser is bundled (unlike Electron) |
| UI build | **Vite** (Node.js) | Bundles the UI |
| Installer | **NSIS** (built by Tauri) | Normal `Pane-setup.exe` |

Needed on LOUISPC (checked 2026-11-11): Node.js 24 ✓, WebView2 ✓, **Rust (rustup) ✗**, **Visual Studio C++ Build Tools ✗**. Install both at the start of Phase 1.

## Install, uninstall, on/off (decided 2026-11-11)
Pane must be easy to try and easy to get rid of. The only Windows setting Pane touches is taskbar auto-hide (needed so the taskbar frees the screen edge). The old value is saved to `%LOCALAPPDATA%\org.gruening.pane\taskbar-autohide.txt` first and given back on quit, crash, uninstall and next start.
- **Install:** run `Pane-setup.exe` (per user, no admin needed). Pane goes into the Start menu and "Installed apps".
- **Uninstall:** Settings → Apps → Installed apps → Pane → Uninstall. The uninstaller closes Pane first, brings the taskbar back and asks whether to delete Pane's data too (`%LOCALAPPDATA%\org.gruening.pane`). The program itself lives in `%LOCALAPPDATA%\Pane`.
- **Turn off without uninstalling:** right-click Pane → "Quit Pane" (taskbar comes back at once), or a hotkey (**Ctrl+Alt+P**, toggles on/off).
- **Start with Windows:** a switch in Pane's settings, off by default.
- **Emergency:** if Pane hangs or is killed in Task Manager, a small watchdog brings the taskbar back. Last resort: run `"%LOCALAPPDATA%\Pane\pane.exe" --restore`, or turn off "Automatically hide the taskbar" in Settings → Personalization → Taskbar and restart Explorer.
- Phase 1 already includes the installer, so every phase can be tried and removed this way.

## How the Windows parts work
| Need | How |
| --- | --- |
| Reserve the screen edge | AppBar API (`SHAppBarMessage`) per monitor: maximized windows stop at the bar |
| Hide/restore taskbar | Hide `Shell_TrayWnd` and `Shell_SecondaryTrayWnd`; restore on quit, on crash (panic hook + watchdog) and on next start |
| Start button | Send the Win key (`SendInput`) → the real Start menu opens |
| Running apps | `EnumWindows` + WinEvent hooks (created/closed/focused), icons from the exe, click = focus/minimize, right-click = close. Each monitor shows its own windows |
| Favorites | User list of apps (drag to pin), merged with running apps like today |
| Tray icons | See "Tray" below |
| Wi-Fi / volume / battery | Status icons; click opens Windows Quick Settings (Win+A) |
| Notifications / calendar | Clicking the date/time opens the Windows notification center + calendar (Win+N) |
| Spotify | Windows media API (`GlobalSystemMediaTransportControls`): title, artist, cover, play/pause/next. Works for Spotify, browsers, everything |
| Weather | See "Weather" below |
| Fullscreen | Hide Pane on the monitor where a fullscreen app/game is in front |

### Tray (required)
Apps announce tray icons by sending messages to the window named `Shell_TrayWnd`. Pane creates its own `Shell_TrayWnd` window on top, so it receives those messages first (icon, tooltip, add/change/remove) and passes each one on to Explorer, so nothing breaks. At start it broadcasts `TaskbarCreated`, so running apps send their icons again. Clicks and right-clicks go back to the app the way Explorer does it, so the app's own menus open.
- Proven approach: RetroBar and Cairo Shell work this way (their open-source ManagedShell library is the reference).
- Hardest part of the project → its own phase, tested with Discord, Steam, NVIDIA, OneDrive, Bluetooth, Windows Security.
- "Hidden icons" overflow arrow like today.

### Weather
Windows has no public weather API: the taskbar's weather comes from MSN, and there is no official way to read it. Plan:
- Location from Windows itself (the Windows location service, the same one Windows weather uses).
- Temperature + condition from **Open-Meteo** (free, no account, no key), refreshed every 30 min.
- Clicking the weather opens the **Windows Widgets board** (Win+W), so the full Windows weather is one click away.

## Phases
Each phase = one branch + PR. Gate = the user uses it daily before the next phase.

1. **Skeleton:** install Rust + C++ Build Tools; Pane at the bottom of every monitor, glass, AppBar reservation, hide/restore taskbar (crash-safe), Start button, date/weekday/time block; Quit menu, Ctrl+Alt+P toggle, installer + clean uninstall. Gate: the taskbar always comes back, and install → uninstall leaves nothing behind.
   - **Built 2026-11-11.** Checked on LOUISPC (2 monitors, 2560×1440): taskbar back after a hard kill (watchdog) and after uninstall; install → uninstall leaves no folder, no Installed-apps entry, no state file; 86 MB private RAM for both bars, 0 % CPU idle; installer 1.5 MB. `tools/smoke-test.ps1` and `tools/install-test.ps1` repeat these checks. Waiting for the user's daily-use OK.
2. **Taskbar parity:** favorites + running apps per monitor, Quick Settings + notification buttons, fullscreen hide, autostart.
   - **Built 2026-11-11** (v0.2.0). Favorites are the user's own Windows taskbar pins in their order (read from Explorer's `Taskband` registry list, also Store-app pins like Spotify/Claude); new pins via Start menu → "Pin to taskbar" show up live. Running apps grouped by exe / AppUserModelID, each bar shows its own monitor's windows. Click: start / bring to front / minimize / cycle windows; right-click: window list, new window, close. Status icons (network, volume, battery) → Quick Settings (Win+A). Fullscreen app hides only that monitor's bar. Monitor plug/unplug and resolution changes rebuild the bars. "Start with Windows" in the right-click menu (off by default; uninstall removes it).
   - Checked on LOUISPC: real mouse clicks front/minimize/restore; fullscreen window hides only the primary bar and it returns; autostart Run entry added/removed; kill → taskbar back; install → uninstall clean; 94 MB private RAM, 0 % CPU idle. Tests: `tools/fullscreen-test.ps1` plus the phase 1 scripts.
   - Not done: unpinning from Pane (Windows blocks the unpin verb for other apps; unpin via Start menu for now), drag to reorder, window thumbnails, Wi-Fi signal bars (always full; needs the WLAN API).
3. **Tray icons:** own tray host as above, overflow arrow. Gate: the normal taskbar isn't missed for a week.
   - **Built 2026-11-11** (v0.3.0). `tray.rs` (subagent): own topmost `Shell_TrayWnd` while Pane is shown, every message also forwarded to Explorer; AppBar messages (dwData 0) are copied into memory Explorer can read and the answer copied back (plain forwarding broke Pane's own AppBar). Visible vs overflow follows Windows' own `NotifyIconSettings\IsPromoted`. On hide/quit Pane removes its window and broadcasts `TaskbarCreated`, so apps go back to Explorer.
   - Checked on LOUISPC: 11 icons found (Windows Security + Bluetooth promoted, 9 in the overflow popup); real right-click on Bluetooth opens Bluetooth's own menu at the icon; Explorer's tray has its icons after Pane quits; kill/fullscreen/install tests pass; 98 MB private RAM; idle CPU 0.6 % of one core (topmost re-check every 200 ms). `tools/tray-test.ps1` repeats the tray check.
   - Not done: Explorer's own system icons (e.g. Windows Update) aren't app icons and don't come to Pane; monochrome icons are skipped; the 200 ms topmost check could become event-based if CPU matters.
4. **Spotify + weather:** Now Playing with cover/controls and a small popup with progress; temperature + icon under the time.
   - **Built 2026-11-11** (v0.4.0). `media.rs` (subagent): Windows media controls (Spotify, browsers, any app), cover cached, play/pause/previous/next; shown on the left with a thin progress line instead of a popup (simpler, always visible). `weather.rs` (subagent): Windows location (cached 6 h, only lat/lon rounded to ~1 km leave the PC) + Open-Meteo every 30 min via WinRT HttpClient (no extra HTTP crate); shown under the time, click opens the Windows Widgets board (Win+W); hidden when there is no location or no connection.
   - Checked on LOUISPC: Now Playing showed the paused YouTube video in Chrome with cover, title, artist, controls and progress. Weather: location works, but the HTTPS request failed with 0x80072F8F (secure channel) in Claude's sandboxed test shell while curl worked — **to confirm on the user's normal desktop**; if it still fails there, switch to `ureq` + rustls. `tools/widgets-test.ps1` repeats the check.
   - Budget: 105.6 MB private RAM (over the 100 MB target by ~6 %), idle CPU ~1.3 % of one core (~0.06 % of the whole CPU). Trim in Phase 7 (e.g. fewer WebView2 processes, smaller cover images).
5. **Claude usage:** 5-hour and weekly limits as bars + reset time.
   - Option A: use Claude Code's local login (`~/.claude/.credentials.json`) to read the usage that `/usage` shows. Needs checking that this is reachable and stable.
   - Option B (fallback): Claude Code token totals from the local transcripts in `~/.claude/projects`.
   - Tokens stay local, read-only, never logged or sent anywhere else.
   - **Built 2026-11-11** (v0.6.0), option A approved by the user. `claude.rs` (subagent): `GET https://api.anthropic.com/api/oauth/usage` with the Claude Code token (`anthropic-beta: oauth-2025-04-20`), reads `five_hour` / `seven_day` utilization + reset time, plan from the credentials file; every 5 min. Expired token / 401 → local token count of the last 5 h (B). Never refreshes or writes the credentials. `http.rs`: shared WinRT GET (also used by weather). Widget left next to Now Playing; click opens claude.ai/settings/usage.
   - Checked on LOUISPC: "5h 46 %, 7d 64 %, resets 23:30 / Fri 07:00" on the bar. Undocumented endpoint: may change.
   - v0.6.0 checks: kill, fullscreen, install → uninstall pass. RAM 149 MB private (glass layers + animations raise WebView2 GPU memory): **first job of Phase 7 is to bring it back under 100 MB.**

### User feedback 2026-11-11 (after installing v0.5.0), fixed in v0.6.0
- Date hover covered only the text → the whole date/time/weather block lights up.
- No glass visible: Windows 11's Acrylic/Mica turn flat grey on inactive windows (the bar is never active) → accent-policy acrylic (stays on) + DWM rounded corners + CSS sheen.
- Bar should float free → 6 px gap to the screen edges and above maximized windows.
- Start icon too small/unclear → 24 px gradient glyph.
- Animations for everything → entrance, hover/press, app pop in/out, sliding indicators, launch bounce, tray/popup, song change, gliding meters/progress; off with Windows' reduced motion.
- Answers: floating style "full width with gaps"; Claude login OK; repo public + auto-update; no paid code signing.
6. **Extras (picked by the user 2026-11-11):**
   - **CPU / GPU / RAM usage:** small % meters; CPU/RAM via `sysinfo`, GPU load via the Windows GPU performance counters (the ones Task Manager shows; works for the RTX 3060 without NVIDIA tools). Refresh every 2 s, paused while no monitor is on.
   - **Per-app volume:** click on the volume icon opens the Windows volume mixer (`ms-settings:apps-volume`), where every app's volume is set. Later maybe an own popup with sliders (Windows audio session API).
   - **Network speed:** live up/down (KB/s, MB/s) from the network adapter counters, every 1 s.
   - Not picked for now (could come later): solar live, Claude Code jobs, GitHub, Stempelei, focus timer, mic/camera indicator, calendar event.
   - **Built 2026-11-11** (v0.5.0, before Phase 5, which waits for the user's OK on reading the Claude login). `usage.rs` (subagent): CPU via GetSystemTimes, RAM via GlobalMemoryStatusEx, GPU via PDH 3D-engine counters, network via GetIfTable2 deltas; no `sysinfo` crate; ~1.5 ms per sample, every 1 s, only while Pane is shown. Meters + ↓/↑ speed left of the tray; click opens Task Manager. Volume icon is its own button → Windows volume mixer.
   - Checked on LOUISPC: CPU 1 %, GPU 1 %, RAM 51 %, ↓ 12 KB/s ↑ 3 KB/s shown; pane.exe used 0.016 s CPU over 5 s; kill and install tests pass. GPU path not checked under load yet. RAM now 115 MB private (PDH loads extra DLLs into pane.exe): Phase 7 should trim it back under 100 MB.
7. **Polish:** themes (light/dark/clear glass), settings page, signed installer, auto-update.
   - **Auto-update built 2026-11-12** (for v0.7.0): `tauri-plugin-updater`, endpoint `releases/latest/download/latest.json` on GitHub (repo must be public). `update.rs` checks 30 s after start and every 6 h, sends "update" to the bars → accent pill "Update x.y.z" at the end of the left side; click → download, taskbar back (`set_shown(false)` on the main thread, waited for), passive NSIS install, Pane restarts. Pane menu: "Check for updates" / "Up to date". Signature key in `%USERPROFILE%\.tauri\pane-updater.key`; `tools/release.ps1` builds + writes `latest.json`. No code signing (SmartScreen may warn on first install).
   - Not tested end to end yet: needs a first public release to update from.
   - **Public repo 2026-11-12:** secret scan of all 45 commits clean; history rewritten to the GitHub noreply email; old repo renamed to private `LouisHD14/pane-archive` (keeps PRs #1-#11, whose commits still carry the old email); new public `LouisHD14/pane` with the clean history, v0.6.2 re-released there.
   - **Memory 2026-11-12:** one shared WebView2 renderer (`--process-per-site --renderer-process-limit=1`) and the tray popup is destroyed when it closes (an opened popup kept ~30 MB). Result ~160-175 MB private for two bars (was 189 MB with the popup opened). The 100 MB target is not reachable with two hardware-drawn transparent pages: JS heap is only 3.6 MB per page, CSS effects change ±10 MB; `--enable-low-end-device-mode` (89 MB) and `--disable-gpu` (82 MB) both draw an empty bar. Accepted: ~0.5 % of 32 GB, idle CPU ~0.
   - Not done (not asked for): themes, settings page. Paid code signing: skipped by the user.

### Feedback from a second PC (i5-10310U laptop, Intel UHD, 1920×1080 at 150 %), 2026-11-12
- **Lag:** apps, tray and usage were sent to every bar on every event even when nothing changed, and each send rebuilt that part of the page; the meters and the song progress animated a gradient / width (repaint of the whole glass layer every frame, all the time while music played). Now: Rust only sends what changed, window identity (exe, AppUserModelID) is cached per window, meters and progress use `transform: scaleX` (compositor only).
- **Claude widget showed "138.7M tokens":** the stored Claude Code login had expired 3.6 days earlier and Pane never refreshed it. Now refreshed like Claude Code (`platform.claude.com/v1/oauth/token`, Claude Code's client id) and written back to `~/.claude/.credentials.json` (only the three token fields; temp file + rename). If renewal fails the widget says "Log in" and opens `claude /login`. This changes Phase 5's "never refreshes or writes the credentials", approved by the user.
- **Codex usage widget** (asked for): Codex CLI login (`~/.codex/auth.json`), `chatgpt.com/backend-api/wham/usage` (the endpoint behind Codex's `/status`), refreshed and written back the same way. Hidden when Codex isn't logged in with ChatGPT; right-click → "Add Codex login…" opens `codex login`.
- **Right-click Settings:** auto-hide, adaptive colour, show/hide each widget (`settings.rs`, `settings.json` in the data folder).
- **Snipping Tool made the bar blink:** Win+Shift+S puts a fullscreen overlay in front, which Pane took for a fullscreen app and hid the bar. Overlays (tool windows, Snipping Tool / ShellExperienceHost) no longer count; bars are shown without activation (`SW_SHOWNA`).
- **Meters "didn't work":** CPU from `% Processor Utility` (Task Manager's number; plain busy time read much lower on a turbo laptop CPU), GPU as the busiest engine type per adapter (3D only showed ~0 % on Intel iGPUs during video); % shown next to each meter.
- **Auto-hide** (asked for, like Windows' "automatically hide the taskbar"): no screen edge reserved, the bar floats over apps, hides 0.7 s after the mouse leaves, comes back when the mouse touches the bottom 2 px; stays while its menu or the tray popup is open. Cursor polled every 60 ms only while auto-hide is on.
- **Adaptive colour** (asked for): every 0.75 s the 3 pixel rows just above the bar (above the gap, so the bar itself isn't read) are split in 8 bands; each band's median colour (so 1 px window borders don't count) tints the glass, dark text over light apps. Sent only when it changes.
- **Date wrapped onto two lines and pushed the weather out of the bar** on a 1280 CSS px wide bar: right column now always gets its full width, the date never wraps, the left side gives way first (Now Playing shortens), and narrow bars drop the network speed (< 1400 px) and the song text (< 1000 px).

### Bar layout and settings menu, 2026-11-12 (branch `bar-layout-settings`)
- **Auto-hide over maximized apps:** with no edge reserved a maximized window covers the whole monitor and was taken for a fullscreen app, so the bar never came back. Now a window that is maximized (`IsZoomed`) and still has its frame (caption or resize border) doesn't count; F11 / video / games drop the frame and still hide the bar.
- **"Fill the screen edge"** setting (`fill` in `settings.json`, off by default): the bar is the whole reserved strip, square corners (DWM `DONOTROUND`), no margin; the tint covers it all. Applies live.
- **Reserved strip = what is drawn:** floating: bar + 6 px below + 2 px above (was 6 px above); fill: exactly the bar. The bar's place no longer follows the shell's `ABM_QUERYPOS` answer, which moved it up when another bottom AppBar (likely Explorer's hidden taskbar) still held space, seen after auto-hide on → off. Such mismatches and a work area that doesn't end at the strip are written to `pane.log`.
- **Pane menu stays open:** it is an own glass popup (`menu.ts`, window `menu-<bar>`, sized by the page via `fit_menu`) instead of a native menu, which closes on every click. Switches and "Check for updates" keep it open; other actions, a click outside or Escape close it. The app icons' menu stays native.
- **Bar pinned to its place (2026-11-12, live-tested):** something outside Pane (most likely the shell while the taskbar's auto-hide changes) moved the bar 50 px up for about a second at start and after auto-hide on → off. `win::pin` subclasses each bar and rejects every other position in `WM_WINDOWPOSCHANGING`. Live check on LOUISPC (2560×1440): floating bar 1386–1434, work area ends 1384; fill 1392–1440, work area 1392; auto-hide on → off returns to the same place.
- **"App colour under the bar"** (`blend` in `settings.json`, 2026-11-12, asked for with a screenshot): the bar window covers the whole reserved strip; the tint layer becomes an opaque floor in the 8 colours of the app's bottom edge and the bar floats on it as its own glass slab with the usual margins. Uses the adaptive-colour sampling (runs when either switch is on). No effect with "Fill the screen edge".
- **Which colours (decided with the user 2026-11-12, `colors.rs`):** the top app window on each monitor (z-order, minimized skipped), only when its visible frame reaches the bar, else the normal glass. Its own bottom 3 rows are read in 32 cells; each cell takes its most common real colour (the old per-channel median mixed channels of different pixels into colours that weren't there: the purple tints). Alike neighbours join, zones under ~15 % fold into their closest neighbour, at most 3 zones (sidebar / content). A colour is only shown after 3 matching readings (~2 s), so a playing video keeps the last calm colour. The adaptive tint of the glass uses the same colours.

### App badges and window previews, 2026-11-12 (branch `app-badges-previews`, waiting for a live test)
- **Badges** (user chose "badges on app icons"): apps set them with `ITaskbarList3::SetOverlayIcon`; shell32 sends that to the window in the `TaskbandHWND` property of the first `Shell_TrayWnd`. Pane's tray window now carries that property, pointing at a hidden "taskband" window in `apps.rs` (the ManagedShell/RetroBar way). Overlay icon + description are also passed on to Explorer's taskband, so Explorer is right when Pane quits. When Pane is shown it sends `TaskbarButtonCreated` to every taskbar window, so apps send their badges again. Shown small at the icon's bottom-right.
- **Attention flashing:** the taskband window is also a shell hook window; `HSHELL_FLASH` gives the icon a warm tile (pulses 3×, then stays) until the window comes to front.
- Not possible: Store-app badges (`BadgeUpdateManager`, e.g. WhatsApp from the Store): no documented way to read another app's badge. Other ITaskbarList calls (tabs, thumbnail clip/tooltip) are accepted and ignored; progress and thumbnail buttons since 2026-11-12, see below.
- **Live previews:** 400 ms hover on a running app → glass popup above the icon with a DWM thumbnail (`DwmRegisterThumbnail`) and title per window; click = that window to front; closes 0.3 s after the mouse left icon + popup (cursor polled every 50 ms only while open). One worker thread (`preview.rs`) owns the popup; it is reused while moving along the icons and destroyed on close (like the tray popup). Minimized windows show the app icon. Shown without activation; auto-hide holds the bar while it is open.

### Reorder pinned apps, 2026-11-12 (branch `pin-reorder`, waiting for a live test)
- **Drag to reorder** (asked for; decided with the user: the order lives in Pane only, Windows' own taskbar pins are not changed). `pin_order` (app ids) in `settings.json`; `apps::list` applies it on top of Windows' pin order: saved ids first, pins Pane doesn't know yet after them in Windows' order, unpinned ids dropped (unit test). A drop calls `set_pin_order`, which pushes the apps to every bar (with "Show apps from all monitors" on or off: pins are on every bar either way).
- `reorder.ts`: pointer drag after 5 px (shorter presses stay clicks, right-click menus unchanged), the icon follows the pointer within the pins, the others slide aside, Escape cancels, the click after a drag is swallowed; no hover preview while dragging; app updates that come in during a drag are shown when it ends. Keyboard: Alt+Shift+Left/Right on a focused pinned icon. Running unpinned apps stay after the pins and can't be dragged.
- Checked: `cargo test`, `npm run build`, and a headless Edge self-check of `reorder.ts` with CDP mouse/key input (click, drag, clamp, Escape, keyboard). Not checked live in the bar yet.

### Progress bars and preview controls, 2026-11-12 (branch `taskband-progress`, waiting for a live test)
- **Progress on app icons:** `SetProgressState` / `SetProgressValue` arrive at the taskband as `WM_USER+65` / `+64` (posted, not sent; wParam = window, lParam = TBPFLAG or 0..0xFFFE, ManagedShell/LiteStep). A value turns "none"/"indeterminate" into normal, error and paused stay (like Explorer). Thin bar under the icon in place of the running line: green, yellow paused, red error, a sliding light while indeterminate (the only animation, only then; still with reduced motion). In 1 % steps so a download doesn't rebuild the bar on every call. Cleared on `TBPF_NOPROGRESS` and when the window is destroyed. Passed on to Explorer like the badge.
- **Preview cards:** close button (✕, top-right, on hover or keyboard focus, `aria-label "Close <title>"`) and middle-click close the window (`close_window`, WM_CLOSE). Cards are now a `div` with a switch button, the close button and the toolbar (buttons can't nest).
- **Thumbnail toolbars** (play/pause/next of media players): `ThumbBarAddButtons` / `UpdateButtons` / `SetImageList` = `WM_USER+76/77/78`, lParam = SHAllocShared block. Only LiteStep documents the layout, so it was checked with real shell32 calls on a private desktop (`cargo test -- --ignored thumb_bar_from_shell32`, nothing of the user's sees it): `[u32 count]` + 540-byte records with 32-bit fields also on 64-bit (mask, id, bitmap, HICON copy, tip `[u16; 260]`, flags); the image list is `[u32 size]` + an `ImageList_Write` stream (read with `ImageList_Read`). Blocks are bounded by `VirtualQuery`, at most 7 buttons. Shown centred in a row under each card's image (the row only exists when a window has buttons); a click posts `WM_COMMAND(MAKEWPARAM(id, THBN_CLICKED))` to the app, like Explorer; DISMISSONCLICK closes the preview. Progress and badge changes don't redraw an open preview.

### Settings window, 2026-11-12 (branch `settings-window`, waiting for a live test)
- **Asked for (item 12):** a window with all switches grouped (Bar, Auto-hide, Widgets, Windows, Updates), glass lightness, auto-hide timing and bar height. The right-click menu keeps its quick switches and gets "More settings…".
- **Window:** label `settings`, same `index.html` (`settings.ts`), a normal window with a title bar, Windows 11 Mica (dark) behind a transparent page, centred on the monitor of the bar whose menu asked. One at a time (focused if open), destroyed when closed. Real labels, native checkboxes (drawn as switches, `role="switch"`), sliders and radio buttons, so keyboard and screen readers work; Escape closes it.
- **New settings** (`settings.rs`, serde defaults = the look before): `glass` 0-100 (50), `hide_delay_ms` 100-3000 (700), `slide_ms` 0-400 (120, 8 frames x 15 ms), `size` small / normal / large (40 / 48 / 56 logical px). `set_setting(key, value)` takes bools, numbers and the size name, rounds and clamps numbers, refuses wrong types; a hand-edited file is clamped too. Everything applies live through the existing `settings` event.
- **Glass lightness mapping:** darker raises the acrylic tint's alpha from 0x40 to 0xC0 (same dark blue-grey); lighter fades it to 0 and the pages add a white sheen up to 20 %. On the app-colour floor (no acrylic under the slab) darker adds a black shade up to 40 %, lighter the same white sheen. Bars and open popups are re-tinted at once.
- **Bar height:** the window height and the reserved strip follow `size` (gaps stay 6 / 2 px); the bar's contents scale with CSS `zoom` (one factor for every px size written for 48 px).

## Open questions
- None. (Spotify + Claude on the left: used since Phase 4, no objection from the user.)

### Memory, 2026-11-12 (branch `low-memory-setting`)
- Real use is ~105-125 MB in Task Manager (private working set), not the ~400 MB summed working sets suggested; the 100 MB goal is close.
- Tried and dropped (no clear gain in alternating A/B runs): WebView2 MemoryUsageTargetLevel Low for the bars, network service in-process, no crash reporter, V8 lite mode.
- Kept: "Low memory mode (no GPU, applies after restart)" in the settings window, off by default (user's choice): `--disable-gpu` for all windows, ~20 MB less in Task Manager and ~70 MB less private memory, measured twice each; looks the same; costs a little CPU while something animates.
