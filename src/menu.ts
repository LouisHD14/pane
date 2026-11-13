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
  volume_media: boolean;
  /** Glass lightness 0 (dark) ..= 100 (light), 50 = the original look. */
  glass: number;
  hide_delay_ms: number;
  slide_ms: number;
  size: "small" | "normal" | "large";
};

/** The on/off settings. */
export type Switch = { [K in keyof Settings]: Settings[K] extends boolean ? K : never }[keyof Settings];

/** A switch (`checked` set) or an action, a separator or a heading. Actions close the menu unless `stay`;
 * `more` marks an entry that opens the second page (Settings). */
type Item = { text: string; run: () => unknown; checked?: boolean; disabled?: boolean; stay?: boolean; more?: boolean };
type Entry = Item | "-" | { heading: string };

import { closeOnBlur } from "./popup";

const close = () => invoke("close_popup");

export async function menuPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("menu-popup");
  const list = Object.assign(document.createElement("div"), { className: "menu" });
  list.setAttribute("role", "menu");
  list.setAttribute("aria-label", "Pane");
  document.body.replaceChildren(list);
  closeOnBlur();
  // One popup, several pages: the main page, Settings, and a page per group of switches. Each page says where
  // its "Back" goes (PARENT), so Escape and Back always walk the same way out.
  type Page = "main" | "settings" | "bar" | "widgets" | "elements";
  const PARENT: Record<Page, Page | null> = { main: null, settings: "main", bar: "settings", widgets: "settings", elements: "settings" };
  let page: Page = "main";
  let go = (_: Page) => {};
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      const up = PARENT[page];
      return up ? go(up) : close();
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...list.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    const down = e.key === "ArrowDown";
    items[i < 0 ? (down ? 0 : items.length - 1) : (i + (down ? 1 : items.length - 1)) % items.length]?.focus();
  });

  try {
    const load = () =>
      Promise.all([
        invoke<Settings>("get_settings"),
        invoke<boolean>("autostart"),
        invoke<boolean>("codex_logged_in"),
        invoke<ClaudeUsage | null>("get_claude"),
        invoke<CodexUsage | null>("get_codex"),
      ]);
    let [s, auto, codexIn, claude, codex] = await load();
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
    const flip = (key: Switch, text: string): Entry => ({
      text,
      checked: s[key],
      run: () => invoke("set_setting", { key, value: !s[key] }), // the "settings" event redraws the menu
    });
    const back = (): Entry => ({ text: "Back", run: () => go(PARENT[page] ?? "main"), stay: true });
    // Three short pages instead of one long list: how the bar behaves, which widgets it carries, and which of
    // the standard parts it shows.
    const settingsPage = (): Entry[] => [
      back(),
      "-",
      { text: "Bar", run: () => go("bar"), stay: true, more: true },
      { text: "Widgets", run: () => go("widgets"), stay: true, more: true },
      { text: "Show on the bar", run: () => go("elements"), stay: true, more: true },
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
      "-",
      { text: "More settings…", run: () => invoke("open_settings") },
      { text: "Report a bug…", run: () => invoke("report_bug") },
    ];
    const barPage = (): Entry[] => [
      back(),
      "-",
      { heading: "Bar" },
      flip("autohide", "Auto-hide the bar"),
      flip("fill", "Fill the screen edge"),
      flip("adaptive_color", "Match the colour of apps"),
      flip("blend", "App colour under the bar"),
      flip("all_monitors", "Show apps from all monitors"),
    ];
    const widgetsPage = (): Entry[] => [
      back(),
      "-",
      { heading: "Widgets" },
      flip("show_media", "Now Playing"),
      flip("show_claude", "Claude usage"),
      flip("show_codex", "Codex usage"),
      flip("show_usage", "CPU / GPU / RAM"),
      flip("show_weather", "Weather"),
      flip("show_docker", "Docker (while it runs)"),
      "-",
      flip("volume_media", "Music controls at the volume icon"),
      flip("done_glow", "Glow when Claude / Codex finish"),
    ];
    const elementsPage = (): Entry[] => [
      back(),
      "-",
      { heading: "Show on the bar" },
      flip("show_start", "Start button"),
      flip("show_tray", "Tray icons"),
      flip("show_network", "Network"),
      flip("show_volume", "Volume"),
      flip("show_battery", "Battery"),
      flip("show_clock", "Time"),
      flip("show_date", "Date"),
    ];
    const PAGES: Record<Page, () => Entry[]> = {
      main: () => mainPage(),
      settings: settingsPage,
      bar: barPage,
      widgets: widgetsPage,
      elements: elementsPage,
    };
    const mainPage = (): Entry[] => [
      { text: "Settings", run: () => go("settings"), stay: true, more: true },
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
        ...PAGES[page]().map((e) => {
          if (e === "-") return Object.assign(document.createElement("div"), { className: "menu-sep", role: "separator" });
          if ("heading" in e) return Object.assign(document.createElement("div"), { className: "menu-heading", textContent: e.heading });
          const b = Object.assign(document.createElement("button"), { className: "menu-item", disabled: !!e.disabled });
          b.setAttribute("role", e.checked === undefined ? "menuitem" : "menuitemcheckbox");
          if (e.checked !== undefined) b.setAttribute("aria-checked", String(e.checked));
          const mark = Object.assign(document.createElement("span"), { className: "menu-check", textContent: e.checked ? "✓" : "" });
          mark.setAttribute("aria-hidden", "true");
          mark.textContent = e.checked ? "✓" : e.text === "Back" ? "‹" : "";
          b.append(mark, e.text);
          if (e.more) {
            b.setAttribute("aria-haspopup", "menu");
            b.append(Object.assign(document.createElement("span"), { className: "menu-more", textContent: "›" }));
          }
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
    // Layout sizes, not the drawn box: the opening animation starts at 80 % scale (which made the first
    // open too narrow), and the list may be cut to the window and scroll (see .menu).
    const fit = () => invoke("fit_popup", { width: list.offsetWidth, height: list.scrollHeight, centered: false });
    go = async (to) => {
      page = to;
      list.scrollTop = 0;
      render();
      await fit();
      list.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    };
    // A kept menu is opened again (Rust hides menus instead of destroying them, so they open at once):
    // start on the main page with fresh state and play the opening animation again.
    me.listen("popup-open", async () => {
      try {
        [s, auto, codexIn, claude, codex] = await load();
        update = { text: "Check for updates", run: checkUpdate, stay: true };
        list.style.animation = "none";
        void list.offsetWidth;
        list.style.animation = "";
        await go("main");
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
