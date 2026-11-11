# Pane (desktop bar)

Pane: own Windows 11 taskbar replacement with widgets (Tauri 2, Rust + TypeScript). Plan and phases: `PLAN.md`. Root rules: `../CLAUDE.md`.

## Rules for this project
- The normal taskbar must always come back: on quit, on crash, on next start. Never ship a change that can leave the user without a taskbar.
- Same function as today's taskbar first (Start, favorites, running apps, tray icons), widgets second.
- Stay light: target under ~100 MB RAM, ~0 % CPU idle. Prefer event hooks over polling.
- Own design and name; LumaBar (lumabar.xyz) is only inspiration.
- Login tokens (Claude, GitHub, solar) are read locally, never logged, never committed.
