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
- `git fetch origin` before creating worktrees (also for subagents): new worktrees start from the local `origin/main`, which is stale after a PR was merged on GitHub.
- Clicks sent from inside the page (DevTools) are not real input, so Windows refuses to bring apps to the front; test clicks with real mouse input (SetCursorPos + mouse_event). A test window must be TopMost to receive a test click.
- See the bar: start the exe and screenshot the bottom of the screen. Inspect the page: start with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and use the DevTools protocol (`http://127.0.0.1:9333/json`).
