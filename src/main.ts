import { invoke } from "@tauri-apps/api/core";
import { Menu, type MenuItemOptions, type CheckMenuItemOptions, type PredefinedMenuItemOptions } from "@tauri-apps/api/menu";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { renderStatus, type Status } from "./status";

type WinItem = { hwnd: number; title: string; active: boolean };
type AppItem = { id: string; name: string; icon: string; pinned: boolean; windows: WinItem[] };
type Item = MenuItemOptions | CheckMenuItemOptions | PredefinedMenuItemOptions;

const me = getCurrentWebviewWindow();
const $ = (id: string) => document.getElementById(id)!;

// --- Clock: Windows locale decides the format, e.g. "Mi., 11. Nov." / "Wed, Nov 11".
const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

function tick() {
  const now = new Date();
  $("date").textContent = dateFmt.format(now);
  $("time").textContent = timeFmt.format(now);
  // Wake up again right after the next full minute, not every second.
  setTimeout(tick, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
}
tick();

// --- Apps: pinned + running, each bar shows the windows on its own monitor.
let apps: AppItem[] = [];

function renderApps(list: AppItem[]) {
  apps = list;
  const box = $("apps");
  box.replaceChildren(
    ...list.map((app) => {
      const b = document.createElement("button");
      b.className = "app";
      b.dataset.id = app.id;
      b.title = app.windows.length === 1 ? app.windows[0].title : app.name;
      b.setAttribute("aria-label", app.name);
      b.classList.toggle("running", app.windows.length > 0);
      b.classList.toggle("active", app.windows.some((w) => w.active));
      if (app.icon) {
        const img = document.createElement("img");
        img.src = app.icon;
        img.alt = "";
        b.append(img);
      } else {
        b.textContent = app.name.slice(0, 1).toUpperCase();
      }
      b.addEventListener("click", () => clickApp(app));
      return b;
    }),
  );
}

/** No window: start it. One window: front / minimize. Several: cycle through them. */
function clickApp(app: AppItem) {
  const wins = app.windows;
  if (!wins.length) return invoke("launch", { id: app.id });
  const i = wins.findIndex((w) => w.active);
  const next = wins.length === 1 ? wins[0] : wins[(i + 1) % wins.length];
  invoke("activate", { hwnd: next.hwnd });
}

invoke<AppItem[]>("get_apps").then(renderApps);
me.listen<AppItem[]>("apps", (e) => renderApps(e.payload));

// --- Status icons (network, volume, battery) open Windows Quick Settings.
invoke<Status>("get_status").then((s) => renderStatus($("status"), s));
me.listen<Status>("status", (e) => renderStatus($("status"), e.payload));

$("start").addEventListener("click", () => invoke("open_start"));
$("status").addEventListener("click", () => invoke("open_quick_settings"));
$("clock").addEventListener("click", () => invoke("open_notifications"));

// --- Right-click: app menu on an app icon, Pane menu everywhere else.
document.addEventListener("contextmenu", async (e) => {
  e.preventDefault();
  const id = (e.target as HTMLElement).closest<HTMLElement>(".app")?.dataset.id;
  const app = apps.find((a) => a.id === id);
  const menu = await Menu.new({ items: app ? appMenu(app) : await paneMenu() });
  await menu.popup();
});

function appMenu(app: AppItem): Item[] {
  const items: Item[] = app.windows.map((w) => ({ text: w.title, action: () => invoke("activate", { hwnd: w.hwnd }) }));
  if (items.length) items.push({ item: "Separator" });
  items.push({ text: app.windows.length ? "New window" : `Open ${app.name}`, action: () => invoke("launch", { id: app.id }) });
  if (app.windows.length) {
    const label = app.windows.length > 1 ? "Close all windows" : "Close window";
    items.push({ text: label, action: () => app.windows.forEach((w) => invoke("close_window", { hwnd: w.hwnd })) });
  }
  return items;
}

async function paneMenu(): Promise<Item[]> {
  const auto = await invoke<boolean>("autostart");
  return [
    { text: "Start with Windows", checked: auto, action: () => invoke("set_autostart", { on: !auto }) },
    { item: "Separator" },
    { text: "Hide Pane (Ctrl+Alt+P)", action: () => invoke("toggle") },
    { text: "Quit Pane", action: () => invoke("quit") },
  ];
}
