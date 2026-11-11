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
- Extras (Phase 6)

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

1. **Skeleton:** Pane at the bottom of every monitor, glass, AppBar reservation, hide/restore taskbar (crash-safe), Start button, date/weekday/time block. Gate: the taskbar always comes back.
2. **Taskbar parity:** favorites + running apps per monitor, Quick Settings + notification buttons, fullscreen hide, autostart.
3. **Tray icons:** own tray host as above, overflow arrow. Gate: the normal taskbar isn't missed for a week.
4. **Spotify + weather:** Now Playing with cover/controls and a small popup with progress; temperature + icon under the time.
5. **Claude usage:** 5-hour and weekly limits as bars + reset time.
   - Option A: use Claude Code's local login (`~/.claude/.credentials.json`) to read the usage that `/usage` shows. Needs checking that this is reachable and stable.
   - Option B (fallback): Claude Code token totals from the local transcripts in `~/.claude/projects`.
   - Tokens stay local, read-only, never logged or sent anywhere else.
6. **Extras** (suggestions, the user picks):
   - **Solar live:** PV watts, battery %, today's kWh from the own solar API.
   - **System:** CPU/GPU load and temperature, RAM; tiny graph on hover.
   - **Claude Code jobs:** shows running background sessions, turns green when one is done.
   - **GitHub:** open PRs and CI status for `LouisHD14` repos.
   - **Stempelei:** new sign-ups / stamps today.
   - **Focus timer:** Pomodoro that turns on Windows Do Not Disturb while it runs.
   - **Mic/camera indicator:** shows when an app uses them; click mutes the mic.
   - **Per-app volume** in a popup on the volume icon.
   - **Next calendar event** with a countdown.
   - **Network speed:** live up/down.
7. **Polish:** themes (light/dark/clear glass), settings page, signed installer, auto-update.

## Open questions
- Which extras first?
- Spotify + Claude on the left side: OK?
