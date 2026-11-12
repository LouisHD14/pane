# Pane

Own taskbar for Windows 11: a glass bar at the bottom of every monitor with Start, apps and widgets. The plan and phases are in [`PLAN.md`](PLAN.md).

## Use it
- **Install:** run `Pane_<version>_x64-setup.exe` (no admin needed). Start Pane from the Start menu.
- **Apps:** your Windows taskbar pins, then open apps. Click to start / bring to front / minimize; right-click for its windows, "New window", "Close". Pin new apps from the Start menu (right-click → "Pin to taskbar").
- **Tray icons:** the icons Windows shows on the taskbar sit next to the status icons, the rest behind the ^ arrow. Click and right-click work like on the Windows taskbar. Which icons are visible follows Windows Settings → Personalization → Taskbar → Other system tray icons.
- **Now Playing (left):** cover, title, artist and play/pause/next for Spotify, YouTube in the browser or any app using Windows media controls.
- **Weather (under the time):** from your Windows location (Settings → Privacy & security → Location must allow desktop apps) and Open-Meteo. Click it for the Windows Widgets board.
- **Claude usage (left):** your 5-hour and weekly Claude limits with reset times (from Claude Code's login on this PC); click for the usage page. When the stored login has expired, Pane renews it like Claude Code does and saves the new token back to `~/.claude/.credentials.json`; if that is no longer possible the widget says "Log in" and a click opens `claude /login`.
- **Codex usage (left):** the same for Codex (ChatGPT plan), from the Codex CLI's login (`~/.codex/auth.json`, renewed the same way). Right-click the bar → "Add Codex login…" opens `codex login` in a terminal; the widget appears once you are logged in.
- **System load:** CPU / GPU / RAM meters with % (the same numbers as Task Manager: CPU utility, and GPU as the busiest engine, so video decoding on an Intel iGPU counts) and download/upload speed (hidden on narrow screens); click for Task Manager.
- **Settings:** right-click the bar → Settings: auto-hide (the bar floats over apps and slides away; touch the bottom edge of the screen to bring it back), match the colour of the apps above the bar, and show/hide each widget. Saved in `%LOCALAPPDATA%org.gruening.panesettings.json`.
- **Status and clock:** the network icon opens Quick Settings, the volume icon the per-app volume mixer, the clock notifications and the calendar.
- **Start with Windows:** right-click the bar → "Start with Windows".
- **Hide / show:** Ctrl+Alt+P, or right-click the bar → "Hide Pane". The Windows taskbar comes back while Pane is hidden.
- **Updates:** Pane checks GitHub releases 30 s after start and every 6 hours. A new version shows an "Update x.y.z" pill on the left; click it to install (the Windows taskbar comes back while the installer runs, then the new Pane starts). Right-click the bar → "Check for updates" to check now.
- **Quit:** right-click the bar → "Quit Pane".
- **Uninstall:** Settings → Apps → Installed apps → Pane → Uninstall. The taskbar comes back as it was.
- **If the taskbar ever stays hidden:** run `"%LOCALAPPDATA%\Pane\pane.exe" --restore`.

## Build
Needs Node.js 22+, Rust (rustup) and the Visual Studio C++ Build Tools.

```sh
npm install
npx tauri build      # installer in src-tauri/target/release/bundle/nsis/
npx tauri dev        # run with live reload of the UI
cd src-tauri && cargo test
```

Build with `npx tauri build`, not `cargo build --release`: plain cargo makes an exe that loads the UI from the dev server and shows an empty bar.

## Release
Needs the updater key at `%USERPROFILE%\.tauri\pane-updater.key` (see `CLAUDE.md`).

1. `tools/release.ps1 -Version x.y.z -Notes "What changed"`: bumps the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, builds the signed installer and writes `latest.json` next to it.
2. Commit and merge the version bump to `main`.
3. Run the `gh release create ...` command the script prints. Don't mark the release as prerelease: Pane reads `releases/latest/download/latest.json`, and GitHub's "latest" skips prereleases.

## Check
- `tools/smoke-test.ps1 -Exe <pane.exe>`: starts Pane, checks the taskbar is hidden, kills Pane hard, checks the watchdog brings the taskbar back. Prints RAM and idle CPU.
- `tools/install-test.ps1 -Setup <setup.exe>`: silent install → start → silent uninstall, checks nothing is left behind.

- `tools/fullscreen-test.ps1 -Exe <pane.exe>`: opens a borderless fullscreen window on the main monitor, checks only that bar hides and comes back.

- `tools/tray-test.ps1 -Exe <pane.exe> -Out <prefix>`: counts the bar's tray icons, clicks the ^ arrow and saves screenshots of the bar, the popup and Explorer's tray afterwards (uses `tools/eval.mjs` and the WebView2 debug port).

All of them hide your taskbar for a few seconds.

## Layout
```
index.html, src/          Bar UI (TypeScript, CSS)
src-tauri/src/main.rs     App: one bar per monitor, show/hide, watchdog, commands
src-tauri/src/win.rs      Windows calls: hide/restore taskbar, reserve the screen edge, fullscreen check, send keys
src-tauri/src/apps.rs     Pinned + running apps, icons, activate/close/launch, window events
src-tauri/src/status.rs   Network, volume, battery state
src-tauri/src/tray.rs     Tray icon host (own Shell_TrayWnd, forwards everything to Explorer)
src-tauri/src/media.rs    Now Playing (Windows media controls)
src-tauri/src/weather.rs  Windows location + Open-Meteo
src-tauri/src/usage.rs    CPU / GPU / RAM load and network speed
src-tauri/src/claude.rs   Claude plan limits (Claude Code login, renewed when expired) or local token count
src-tauri/src/codex.rs    Codex plan limits (Codex CLI login, renewed when expired)
src-tauri/src/settings.rs Right-click Settings (auto-hide, adaptive colour, widget switches)
src-tauri/src/http.rs     Small HTTPS GET/POST (ureq)
src-tauri/src/update.rs   Auto-update from GitHub releases
src-tauri/windows/        Uninstaller hook (closes Pane, restores the taskbar)
tools/                    Test scripts, release.ps1
```
