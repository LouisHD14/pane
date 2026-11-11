# Desktop Bar – Plan

Own widget bar for Windows 11 that replaces the taskbar, inspired by lumabar.xyz (own design and name, not a copy). Written 2026-11-11.

## Goal
Everything the taskbar does today keeps working, plus widgets:
- Windows/Start button
- Pinned favorites + running apps
- Spotify (Now Playing with controls)
- Claude usage (limits for claude.ai / Claude Code)
- More widgets later (see Phase 5)

Budget: under ~100 MB RAM and ~0 % CPU when idle. If the bar crashes or quits, the normal taskbar must come back.

## Stack
- **Tauri 2** (Rust backend + web UI, same approach as LumaBar). UI in TypeScript + plain CSS; a framework only if the UI gets big.
- Rust `windows` crate for Win32/WinRT calls, `sysinfo` for CPU/RAM.
- Repo: private `LouisHD14/desktop-bar` (name to confirm).

## How the Windows parts work
| Need | How |
| --- | --- |
| Reserve the screen edge | AppBar API (`SHAppBarMessage`): maximized windows stop at the bar |
| Hide/restore taskbar | Hide `Shell_TrayWnd` (+ secondary monitors); restore on quit, on crash (panic hook + watchdog) and on next start |
| Start button | Send the Win key (`SendInput`) → real Start menu opens |
| Running apps | `EnumWindows` + WinEvent hooks (window created/closed/focused), icons from the exe, click = focus/minimize, right-click = close |
| Favorites | User list of apps (drag to pin), launch via shell; merged with running apps like today's taskbar |
| Wi-Fi / volume / battery | Status icons; click opens Windows Quick Settings (Win+A) |
| Notifications / calendar | Clock click opens the Windows notification center (Win+N) |
| Spotify | Windows media API (`GlobalSystemMediaTransportControls`): title, artist, cover, play/pause/next. Works for Spotify, browsers, everything |
| Claude usage | See Phase 4 |
| Fullscreen | Hide bar when a fullscreen app/game is in front |

**Known gap:** tray icons of background apps (Discord, Steam, NVIDIA…). Becoming the real tray host is a lot of fragile work. Plan: those apps show up as running apps when they have a window; for the rest a button opens the hidden-icons area or we live without it. Decide after Phase 2.

## Phases
Each phase = one branch + PR. Gate = the user uses it daily before the next phase.

1. **Skeleton:** Tauri app, bar on the bottom edge, AppBar reservation, hide/restore taskbar (crash-safe), Start button, clock. Gate: the taskbar always comes back.
2. **Taskbar parity:** favorites + running apps (icons, active indicator, focus/minimize/close), Quick Settings + notification buttons, fullscreen hide, multi-monitor, autostart. Gate: the normal taskbar isn't missed for a week.
3. **Spotify / Now Playing:** cover, title, controls, small popup with progress.
4. **Claude usage:** 5-hour and weekly limit as bars + reset time.
   - Option A: use Claude Code's local login (`~/.claude/.credentials.json`) to read the usage endpoint that `/usage` shows. Needs checking that this endpoint is reachable and stable.
   - Option B: Claude Code token totals from local transcripts in `~/.claude/projects` (offline, always works, but no claude.ai limits).
   - Tokens stay local, read-only; never logged or sent anywhere else.
5. **Cool extras (pick):** CPU/RAM/GPU/temp, solar live (own API: PV, battery %), weather, Pomodoro/timer, GitHub PRs for `LouisHD14`, Stempelei stats, calendar next event.
6. **Polish:** glass/dark themes (Windows Mica/Acrylic first, wallpaper-bending glass later), settings page, signed installer, auto-update.

## Open questions
- Bar on top or bottom? All monitors or main only?
- Tray icons: how important (Discord, Steam, etc.)?
- Which extras first?
- Project/repo name?
