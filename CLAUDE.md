# Pane (desktop bar)

Pane: own Windows 11 taskbar replacement with widgets (Tauri 2, Rust + TypeScript). Plan and phases: `PLAN.md`. Root rules: `../CLAUDE.md`.

## Rules for this project
- The normal taskbar must always come back: on quit, on crash, on next start. Never ship a change that can leave the user without a taskbar.
- Same function as today's taskbar first (Start, favorites, running apps, tray icons), widgets second.
- Stay light: target under ~100 MB RAM, ~0 % CPU idle. Prefer event hooks over polling.
- Own design and name; LumaBar (lumabar.xyz) is only inspiration.
- Login tokens (Claude, GitHub, solar) are read locally, never logged, never committed.

## Build and test (learned 2026-11-11)
- Build with `npx tauri build`. `cargo build --release` alone makes an exe that loads the UI from `localhost:1420` and shows an empty bar.
- Rust needs `%USERPROFILE%\.cargo\bin` on PATH (new shells have it; old ones need it added).
- Before each PR: `cargo test` in `src-tauri/`, `npm run build`, then `tools/smoke-test.ps1` and `tools/install-test.ps1` (they hide the taskbar for a few seconds).
- The user runs Pane themselves now: stop their Pane before tests (single instance would hand over to it), start it again afterwards. `install-test.ps1` refuses to run while Pane is installed, because it ends with an uninstall (it removed the user's install once, 2026-11-11).
- `git fetch origin` before creating worktrees (also for subagents): new worktrees start from the local `origin/main`, which is stale after a PR was merged on GitHub.
- Clicks sent from inside the page (DevTools) are not real input, so Windows refuses to bring apps to the front; test clicks with real mouse input (SetCursorPos + mouse_event). A test window must be TopMost to receive a test click.
- HTTPS goes through `ureq` (rustls). WinRT `HttpClient` failed Open-Meteo with 0x80072F8F on the user's desktop too (not only in Claude's sandbox) while Windows' curl worked (2026-11-11).
- Processes Claude starts run in Claude's sandbox. Don't start the user's own Pane from Claude's shell; ask them to start it from the Start menu.
- Errors go to `%LOCALAPPDATA%\org.gruening.pane\pane.log`: read it first when something doesn't show up.
- Undecorated windows with `.shadow(true)` get invisible 7 px borders that shift the bar onto the next monitor: keep `shadow(false)`.
- Updater signing key (Tauri updater signature, not code signing, no password): `%USERPROFILE%\.tauri\pane-updater.key`. Never commit it, never print it; only the public key is in `tauri.conf.json`. Losing it means installed Panes can't update any more (they reject other signatures).
- Release: `tools/release.ps1 -Version x.y.z` bumps the versions, builds with the key and writes `latest.json`; it prints the `gh release create` command, which the user (or main session) runs. Never as prerelease (the endpoint is `releases/latest`).
- See the bar: start the exe and screenshot the bottom of the screen. Inspect the page: start with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and use the DevTools protocol (`http://127.0.0.1:9333/json`).
