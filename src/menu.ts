// The bar's right-click menu, in its own glass popup window. A native menu always closes on a click; this one
// stays open while switches are flipped and closes on a click outside, Escape, or an action like "Quit Pane".

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { ClaudeUsage, CodexUsage } from "./claude";

export type Settings = {
  autohide: boolean;
  fill: boolean;
  adaptive_color: boolean;
  blend: boolean;
  all_monitors: boolean;
  low_memory: boolean;
  show_media: boolean;
  show_claude: boolean;
  show_codex: boolean;
  show_usage: boolean;
  show_weather: boolean;
  show_docker: boolean;
  show_start: boolean;
  show_tray: boolean;
  show_network: boolean;
  show_volume: boolean;
  show_battery: boolean;
  show_clock: boolean;
  show_date: boolean;
  done_glow: boolean;
  /** Bar elements in the user's order (arrange.ts). */
  element_order: string[];
  /** Glass lightness 0 (dark) ..= 100 (light), 50 = the original look. */
  glass: number;
  /** Glass opacity 0 (clear) ..= 100 (nearly solid), 50 = the original look. */
  opacity: number;
  /** Auto-hide: the bar comes up while the Start menu is open. */
  win_key: boolean;
  /** Double-click on an empty part of the bar: "none", "terminal", "explorer", "taskmgr", "desktop" or a program. */
  void_action: string;
  /** Scroll on the volume icon / an app icon. */
  wheel: boolean;
  show_awake: boolean;
  show_desktops: boolean;
  /** The user's own widget: a command and how often it runs (empty: off). */
  custom_command: string;
  custom_seconds: number;
  hide_delay_ms: number;
  slide_ms: number;
  size: "small" | "normal" | "large";
};

/** The on/off settings. */
export type Switch = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

/** An action (closes the menu) or a separator. */
type Item = { text: string; run: () => unknown };
type Entry = Item | "-";

import { closeOnBlur } from "./popup";

const close = () => invoke("close_popup");

/** Only actions: every setting lives in the settings window (asked for in ideas.md, 2026-11-16), so the
 * menu stays short and one place has them all. */
export async function menuPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("menu-popup");
  const list = Object.assign(document.createElement("div"), { className: "menu" });
  list.setAttribute("role", "menu");
  list.setAttribute("aria-label", "Pane");
  document.body.replaceChildren(list);
  closeOnBlur();
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") return close();
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...list.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    const down = e.key === "ArrowDown";
    items[i < 0 ? (down ? 0 : items.length - 1) : (i + (down ? 1 : items.length - 1)) % items.length]?.focus();
  });

  try {
    const load = () =>
      Promise.all([invoke<boolean>("codex_logged_in"), invoke<ClaudeUsage | null>("get_claude"), invoke<CodexUsage | null>("get_codex")]);
    let [codexIn, claude, codex] = await load();
    const entries = (): Entry[] => [
      { text: "Open terminal", run: () => invoke("open_terminal") },
      { text: "Task Manager", run: () => invoke("open_task_manager") },
      "-",
      { text: "Settings…", run: () => invoke("open_settings") },
      "-",
      ...(claude?.source === "local" ? [{ text: "Log in to Claude Code again…", run: () => invoke("open_claude_login") }] : []),
      {
        text: codexIn ? (codex?.source === "error" ? "Log in to Codex again…" : "Codex: switch account…") : "Add Codex login…",
        run: () => invoke("open_codex_login"),
      },
      "-",
      { text: "Hide Pane (Ctrl+Alt+P)", run: () => invoke("toggle") },
      { text: "Quit Pane", run: () => invoke("quit") },
    ];
    const render = () =>
      list.replaceChildren(
        ...entries().map((e) => {
          if (e === "-") return Object.assign(document.createElement("div"), { className: "menu-sep", role: "separator" });
          const b = Object.assign(document.createElement("button"), { className: "menu-item" });
          b.setAttribute("role", "menuitem");
          b.textContent = e.text;
          b.onclick = async () => {
            try {
              await e.run();
            } finally {
              close();
            }
          };
          return b;
        }),
      );
    // Layout sizes, not the drawn box: the opening animation starts at 80 % scale (which made the first
    // open too narrow), and the list may be cut to the window and scroll (see .menu).
    const fit = () => invoke("fit_popup", { width: list.offsetWidth, height: list.scrollHeight, centered: false });
    // A kept menu is opened again (Rust hides menus instead of destroying them, so they open at once): fresh
    // login state and the opening animation again.
    me.listen("popup-open", async () => {
      try {
        [codexIn, claude, codex] = await load();
        render();
        list.style.animation = "none";
        void list.offsetWidth;
        list.style.animation = "";
        await fit();
        list.querySelector<HTMLButtonElement>("button")?.focus();
      } catch {
        close();
      }
    });
    render();
    await fit();
  } catch {
    close(); // never leave an invisible menu holding the auto-hidden bar up
  }
}
