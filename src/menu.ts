// The bar's right-click menu, in its own glass popup window. A native menu always closes on a click; this one
// stays open while switches are flipped and closes on a click outside, Escape, or an action like "Quit Pane".

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { ClaudeUsage, CodexUsage } from "./claude";

export type Settings = {
  autohide: boolean;
  fill: boolean;
  adaptive_color: boolean;
  show_media: boolean;
  show_claude: boolean;
  show_codex: boolean;
  show_usage: boolean;
  show_weather: boolean;
};

/** A switch (`checked` set) or an action, a separator or a heading. Actions close the menu unless `stay`. */
type Item = { text: string; run: () => unknown; checked?: boolean; disabled?: boolean; stay?: boolean };
type Entry = Item | "-" | { heading: string };

const close = () => invoke("close_popup");

export async function menuPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("menu-popup");
  const list = Object.assign(document.createElement("div"), { className: "menu" });
  list.setAttribute("role", "menu");
  list.setAttribute("aria-label", "Pane");
  document.body.replaceChildren(list);
  window.addEventListener("blur", close);
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
    let [s, auto, codexIn, claude, codex] = await Promise.all([
      invoke<Settings>("get_settings"),
      invoke<boolean>("autostart"),
      invoke<boolean>("codex_logged_in"),
      invoke<ClaudeUsage | null>("get_claude"),
      invoke<CodexUsage | null>("get_codex"),
    ]);
    const checkUpdate = async () => {
      update = { text: "Checking…", disabled: true, run: () => {} };
      render();
      // A found update also arrives in the bar as the "Update x.y.z" pill.
      const version = await invoke<string | null>("check_update").catch(() => null);
      update = version
        ? { text: `Install update ${version}`, run: () => invoke("install_update") }
        : { text: "Up to date", disabled: true, run: () => {} };
      render();
    };
    let update: Item = { text: "Check for updates", run: checkUpdate, stay: true };
    const flip = (key: keyof Settings, text: string): Entry => ({
      text,
      checked: s[key],
      run: () => invoke("set_setting", { key, on: !s[key] }), // the "settings" event redraws the menu
    });
    const entries = (): Entry[] => [
      { heading: "Settings" },
      flip("autohide", "Auto-hide the bar"),
      flip("fill", "Fill the screen edge"),
      flip("adaptive_color", "Match the colour of apps"),
      "-",
      flip("show_media", "Show Now Playing"),
      flip("show_claude", "Show Claude usage"),
      flip("show_codex", "Show Codex usage"),
      flip("show_usage", "Show CPU / GPU / RAM"),
      flip("show_weather", "Show weather"),
      "-",
      {
        text: "Start with Windows",
        checked: auto,
        run: async () => {
          await invoke("set_autostart", { on: !auto });
          auto = await invoke<boolean>("autostart");
          render();
        },
      },
      update,
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
    const render = () => {
      const focused = [...list.querySelectorAll("button")].indexOf(document.activeElement as HTMLButtonElement);
      list.replaceChildren(
        ...entries().map((e) => {
          if (e === "-") return Object.assign(document.createElement("div"), { className: "menu-sep", role: "separator" });
          if ("heading" in e) return Object.assign(document.createElement("div"), { className: "menu-heading", textContent: e.heading });
          const b = Object.assign(document.createElement("button"), { className: "menu-item", disabled: !!e.disabled });
          b.setAttribute("role", e.checked === undefined ? "menuitem" : "menuitemcheckbox");
          if (e.checked !== undefined) b.setAttribute("aria-checked", String(e.checked));
          const mark = Object.assign(document.createElement("span"), { className: "menu-check", textContent: e.checked ? "✓" : "" });
          mark.setAttribute("aria-hidden", "true");
          b.append(mark, e.text);
          b.onclick = async () => {
            const stay = e.stay || e.checked !== undefined;
            try {
              await e.run();
            } finally {
              if (!stay) close();
            }
          };
          return b;
        }),
      );
      if (focused >= 0) list.querySelectorAll("button")[focused]?.focus();
    };

    me.listen<Settings>("settings", (e) => {
      s = e.payload;
      render();
    });
    render();
    const r = list.getBoundingClientRect();
    await invoke("fit_menu", { width: Math.ceil(r.width), height: Math.ceil(r.height) });
  } catch {
    close(); // never leave an invisible menu holding the auto-hidden bar up
  }
}
