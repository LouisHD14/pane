![Pane: a glass taskbar with widgets for Windows 11](docs/banner.png)

<div align="center">

**Pane replaces the Windows 11 taskbar with a glass bar that keeps every taskbar habit and adds widgets worth the space.**

Start, pins, running apps, tray icons and window previews work as you expect — plus now playing, Claude and Codex usage, CPU/GPU/RAM, weather and Docker, in about 110 MB of RAM and ~0 % CPU when idle.

[![Latest release](https://img.shields.io/github/v/release/LouisHD14/pane?label=download&color=1d8bff)](https://github.com/LouisHD14/pane/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/LouisHD14/pane/total?color=1d8bff)](https://github.com/LouisHD14/pane/releases)
[![Windows 11](https://img.shields.io/badge/Windows%2011-x64-1d8bff)](https://github.com/LouisHD14/pane/releases/latest)
[![MIT](https://img.shields.io/badge/license-MIT-1d8bff)](LICENSE)

</div>

![The Pane bar at the bottom of the screen](docs/bar.png)

## Install

1. Download **`Pane_x.y.z_x64-setup.exe`** from [the latest release](https://github.com/LouisHD14/pane/releases/latest).
2. Run it. Per user, no admin needed. SmartScreen may warn because the installer isn't code-signed — **More info → Run anyway**.
3. Start Pane from the Start menu. It replaces the taskbar straight away and starts with Windows from then on.

Don't like it? **Right-click the bar → Quit Pane**, and the Windows taskbar is back instantly. Uninstalling leaves nothing behind.

## Contents

- [What's on the bar](#whats-on-the-bar)
- [Settings](#settings)
- [Turning it off](#turning-it-off)
- [Known limits](#known-limits)
- [Your taskbar always comes back](#your-taskbar-always-comes-back)
- [Privacy](#privacy)
- [Build from source](#build-from-source)

## What's on the bar

| | What it does |
| --- | --- |
| **Start** | Opens the real Start menu. |
| **Pinned and running apps** | Your Windows pins, then open apps, centred like Windows 11. Click to start, focus or minimise; drag to reorder; right-click for windows, "New window" and "Close". Badges, progress bars and attention flashing like the real taskbar. |
| **Window previews** | Hover a running app for live thumbnails of its windows, with close buttons and the app's own media buttons. |
| **Tray icons** | The icons Windows shows, the rest behind the `^` arrow, with clicks and right-clicks passed through to the app. Follows *Settings → Personalization → Taskbar*. Windows' own volume, network and battery icons are left out — Pane draws those itself. |
| **Now playing** | Cover, title, artist and play/pause/next for Spotify, a browser, anything using the Windows media controls. Compact controls also pop out of the volume icon on hover. |
| **Claude usage** | Your 5-hour and weekly limits with reset times, read from Claude Code's login on this PC. An orange glow marks a finished task until you look at it. |
| **Codex usage** | The same for a ChatGPT plan, from the Codex CLI's login. Blue glow. |
| **CPU / GPU / RAM** | Small meters matching Task Manager, plus network speed. Click for Task Manager. |
| **Weather** | Temperature and an icon under the clock, from your Windows location and Open-Meteo. Says so plainly when it can't be read. |
| **Docker** | While Docker runs: how many containers are up, and a popup to start, stop or restart each. Hidden otherwise. |
| **Date and time** | Click for notifications and the calendar. |
| **Auto-update** | A pill appears when a newer release exists; one click installs it. |

Pane puts a bar on **every monitor**, each showing that monitor's windows, and gets out of the way when a game or video goes fullscreen.

## Settings

**Right-click the bar → Settings** for three short pages:

- **Bar** — auto-hide, fill the screen edge, match the colour of the apps above it, show apps from all monitors
- **Widgets** — now playing, Claude, Codex, system load, weather, Docker, the music controls, the finished-task glow
- **Show on the bar** — Start, tray, network, volume, battery, time, date

**More settings…** opens a window with glass lightness, bar height (small / normal / large), auto-hide timing (with a "Taskbar-like" preset), low-memory mode, release download counts and **Report a bug…**.

**Rearrange the bar** by dragging: pinned apps, the widgets on the left, and system load, tray, status icons and clock on the right each move along their side, like icons on the Windows taskbar. `Alt+Shift+←/→` moves a focused one.

**Auto-hide** floats the bar over your apps and slides it away; touch the bottom edge to bring it back. It stays up while a menu or preview is open.

## Turning it off

- **Quit:** right-click the bar → *Quit Pane*. The taskbar returns at once.
- **Hide / show:** `Ctrl+Alt+P`. The taskbar returns while Pane is hidden.
- **Uninstall:** *Settings → Apps → Installed apps → Pane*. The uninstaller closes Pane, restores the taskbar as it was, and asks whether to delete Pane's data too.

## Known limits

- **Apps running as administrator** (installers, elevated tools) take no clicks from Pane: Windows doesn't let a normal program post messages to an elevated window. Their tray icons and "Close window" do nothing — use the window itself. Pane says so once with a "Needs admin rights" pill.

## Your taskbar always comes back

- The only Windows setting Pane touches is taskbar auto-hide. The old value is saved first and given back on quit, crash, uninstall and next start.
- A watchdog process restores the taskbar if Pane crashes or is ended in Task Manager.
- If it ever stays hidden:
  ```
  "%LOCALAPPDATA%\Pane\pane.exe" --restore
  ```
  or untick *Automatically hide the taskbar* in *Settings → Personalization → Taskbar*.

## Privacy

Nothing leaves your PC except the requests below. Settings and logs stay in `%LOCALAPPDATA%\org.gruening.pane`.

- **Claude and Codex usage** read the logins those tools already keep on this PC (`~/.claude/.credentials.json`, `~/.codex/auth.json`). They're sent only to Anthropic and OpenAI to read your usage, and renewed the same way the CLIs do. Pane never logs them.
- **The finished-task glow** reads the end of Claude Code's and Codex's own conversation files (`~/.claude/projects`, `~/.codex/sessions`) when they change. Nothing from them is sent or logged.
- **Weather** sends your rounded location to [Open-Meteo](https://open-meteo.com). No key, no account.
- **Docker** talks to Docker's local pipe only.
- **Updates and download counts** are read from GitHub.
- **Report a bug…** opens GitHub in your browser with the version filled in; nothing is sent until you submit it.

## Build from source

Needs Node.js 22+, Rust (rustup) and the Visual Studio C++ Build Tools.

```sh
npm install
npx tauri build      # installer in src-tauri/target/release/bundle/nsis/
npx tauri dev        # live reload of the UI
cd src-tauri && cargo test
```

Use `npx tauri build`, not `cargo build --release`: plain cargo produces an exe that loads the UI from the dev server and shows an empty bar. Without the release signing key the build ends with an updater error — the exe and installer are still produced.

<details>
<summary><b>Test scripts</b></summary>

These start Pane and hide your taskbar for a few seconds. Quit your own Pane first.

- `tools/smoke-test.ps1 -Exe <pane.exe>` — taskbar hidden, Pane killed hard, watchdog restores it. Prints RAM and idle CPU.
- `tools/install-test.ps1 -Setup <setup.exe>` — silent install, start, silent uninstall; checks nothing is left behind.
- `tools/fullscreen-test.ps1 -Exe <pane.exe>` — the bar hides for a fullscreen window and comes back.
- `tools/tray-test.ps1 -Exe <pane.exe> -Out <prefix>` — counts tray icons, screenshots the tray popup.
- `tools/memory.ps1` — per-process private working set.

</details>

<details>
<summary><b>Where things live</b></summary>

```
index.html, src/          Bar UI, popups and settings window (TypeScript, CSS)
src-tauri/src/main.rs     App: one bar per monitor, show/hide, watchdog, commands
src-tauri/src/win.rs      Windows calls: hide/restore taskbar, reserve the edge, fullscreen check, send keys
src-tauri/src/apps.rs     Pinned + running apps, icons, activate/close/launch, window events
src-tauri/src/preview.rs  Live window previews (DWM thumbnails)
src-tauri/src/tray.rs     Tray icon host (own Shell_TrayWnd, forwards everything to Explorer)
src-tauri/src/status.rs   Network, volume, battery state
src-tauri/src/media.rs    Now Playing (Windows media controls)
src-tauri/src/weather.rs  Windows location + Open-Meteo
src-tauri/src/usage.rs    CPU / GPU / RAM load and network speed
src-tauri/src/claude.rs   Claude plan limits (Claude Code login, renewed when expired)
src-tauri/src/codex.rs    Codex plan limits (Codex CLI login, renewed when expired)
src-tauri/src/agents.rs   Notices when Claude Code or Codex finishes a task
src-tauri/src/docker.rs   Containers, over Docker's local pipe
src-tauri/src/colors.rs   The colours of the app above the bar (adaptive colour)
src-tauri/src/settings.rs Settings, checked and clamped
src-tauri/src/http.rs     Small HTTPS GET/POST (ureq)
src-tauri/src/update.rs   Auto-update and release download counts
src-tauri/windows/        Uninstaller hook (closes Pane, restores the taskbar)
docs/                     README images; banner.html renders banner.png
tools/                    Test scripts, release.ps1
```

Design notes and the running plan: [`PLAN.md`](PLAN.md).

</details>

<details>
<summary><b>Releasing (maintainer)</b></summary>

Needs the updater key at `%USERPROFILE%\.tauri\pane-updater.key`.

1. `tools/release.ps1 -Version x.y.z -Notes "What changed"` bumps the versions, builds the signed installer and writes `latest.json`.
2. Merge the version bump to `main`.
3. Run the `gh release create …` command it prints. Never as a prerelease: Pane reads `releases/latest`.

</details>

## License

MIT — see [LICENSE](LICENSE).

## Credits

- Inspired by [LumaBar](https://lumabar.xyz). Pane has its own design and code.
- The tray icon host follows the technique of [ManagedShell](https://github.com/cairoshell/ManagedShell) and [RetroBar](https://github.com/dremin/RetroBar).
- Built with [Tauri](https://tauri.app). Weather by [Open-Meteo](https://open-meteo.com).
