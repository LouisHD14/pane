# Pane

Own taskbar for Windows 11: a glass bar at the bottom of every monitor with Start, apps and widgets. The plan and phases are in [`PLAN.md`](PLAN.md).

## Use it
- **Install:** run `Pane_<version>_x64-setup.exe` (no admin needed). Start Pane from the Start menu.
- **Hide / show:** Ctrl+Alt+P, or right-click the bar → "Hide Pane". The Windows taskbar comes back while Pane is hidden.
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

## Check
- `tools/smoke-test.ps1 -Exe <pane.exe>`: starts Pane, checks the taskbar is hidden, kills Pane hard, checks the watchdog brings the taskbar back. Prints RAM and idle CPU.
- `tools/install-test.ps1 -Setup <setup.exe>`: silent install → start → silent uninstall, checks nothing is left behind.

Both hide your taskbar for a few seconds.

## Layout
```
index.html, src/          Bar UI (TypeScript, CSS)
src-tauri/src/main.rs     App: one bar per monitor, show/hide, watchdog, commands
src-tauri/src/win.rs      Windows calls: hide/restore taskbar, reserve the screen edge, send keys
src-tauri/windows/        Uninstaller hook (closes Pane, restores the taskbar)
tools/                    Test scripts
```
