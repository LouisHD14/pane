import { invoke } from "@tauri-apps/api/core";
import { Menu, type MenuItemOptions, type CheckMenuItemOptions, type PredefinedMenuItemOptions } from "@tauri-apps/api/menu";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { renderStatus, type Status } from "./status";
import { renderTray, type TrayIcon } from "./tray";
import { mountMedia, renderMedia, type NowPlaying } from "./media";
import { renderWeather, type Weather } from "./weather";
import { renderUsage, type Usage } from "./usage";
import { once, replay } from "./anim";

type WinItem = { hwnd: number; title: string; active: boolean };
type AppItem = { id: string; name: string; icon: string; pinned: boolean; windows: WinItem[] };
type Item = MenuItemOptions | CheckMenuItemOptions | PredefinedMenuItemOptions;

const me = getCurrentWebviewWindow();
const $ = (id: string) => document.getElementById(id)!;

// The "hidden icons" popup loads the same page; its window label tells it apart from a bar.
if (me.label.startsWith("overflow-")) overflowPage();
else barPage();

/** Popup above the ^ arrow with the tray icons that aren't promoted to the bar. */
function overflowPage() {
  document.body.classList.add("overflow");
  document.body.replaceChildren(Object.assign(document.createElement("div"), { id: "hidden-icons", className: "hidden-icons" }));
  const show = (icons: TrayIcon[]) => {
    renderTray($("hidden-icons"), icons.filter((t) => !t.promoted));
    // Icons drop in one after another when the popup opens.
    [...$("hidden-icons").children].forEach((b, i) => ((b as HTMLElement).style.animationDelay = `${i * 25}ms`));
  };
  invoke<TrayIcon[]>("get_tray").then(show);
  me.listen<TrayIcon[]>("tray", (e) => show(e.payload));
  window.addEventListener("focus", () => replay(document.body, "open"));
  // Close like a Windows flyout: when focus goes elsewhere (also when an app opens its tray menu).
  window.addEventListener("blur", () => invoke("close_overflow"));
  document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("close_overflow"));
}

function barPage() {
  // --- Clock: Windows locale decides the format, e.g. "Mi., 11. Nov." / "Wed, Nov 11".
  const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });
  const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
  // --- Entrance: the bar rises and a light sweeps over the glass, at start and whenever Pane is shown again.
  const enter = () => {
    replay(document.querySelector(".bar")!, "enter");
    replay(document.body, "shine");
  };
  enter();
  me.listen("shown", enter);

  const tick = () => {
    const now = new Date();
    $("date").textContent = dateFmt.format(now);
    const time = timeFmt.format(now);
    if ($("time").textContent && $("time").textContent !== time) once($("time"), "tick");
    $("time").textContent = time;
    // Wake up again right after the next full minute, not every second.
    setTimeout(tick, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
  };
  tick();

  // --- Apps: pinned + running, each bar shows the windows on its own monitor.
  let apps: AppItem[] = [];
  const renderApps = (list: AppItem[]) => {
    const box = $("apps");
    const firstPaint = !box.children.length;
    apps = list;
    // Reuse the buttons (keyed by app id) so indicators slide and only new/closed apps animate.
    const current = [...box.querySelectorAll<HTMLElement>(".app:not(.leave)")];
    const old = new Map(current.map((b) => [b.dataset.id!, b]));
    const next = list.map((app) => {
      const b = old.get(app.id) ?? document.createElement("button");
      if (!old.delete(app.id) && !firstPaint) once(b, "enter");
      updateAppButton(b, app);
      return b;
    });
    // Closed apps shrink away at their old spot, then go.
    for (const b of old.values()) {
      next.splice(Math.min(current.indexOf(b), next.length), 0, b);
      b.classList.add("leave");
      b.addEventListener("animationend", () => b.remove(), { once: true });
    }
    // Touch the DOM only if the order changed: re-inserting restarts running animations.
    if (next.length !== box.children.length || next.some((b, i) => box.children[i] !== b)) box.replaceChildren(...next);
  };
  invoke<AppItem[]>("get_apps").then(renderApps);
  me.listen<AppItem[]>("apps", (e) => renderApps(e.payload));

  // --- Tray: promoted icons on the bar, the rest behind the ^ arrow.
  const showTray = (icons: TrayIcon[]) => {
    renderTray($("tray"), icons.filter((t) => t.promoted));
    $("overflow").hidden = !icons.some((t) => !t.promoted);
  };
  invoke<TrayIcon[]>("get_tray").then(showTray);
  me.listen<TrayIcon[]>("tray", (e) => showTray(e.payload));
  $("overflow").addEventListener("click", () => {
    const r = $("overflow").getBoundingClientRect();
    invoke("open_overflow", { clientX: r.left + r.width / 2 });
  });

  // --- Now Playing (Spotify, browsers, any app using Windows media controls).
  mountMedia($("media"));
  invoke<NowPlaying | null>("get_media").then((np) => renderMedia($("media"), np));
  me.listen<NowPlaying | null>("media", (e) => renderMedia($("media"), e.payload));

  // --- Weather under the time; a click opens the Windows Widgets board with the full forecast.
  invoke<Weather | null>("get_weather").then((w) => renderWeather($("weather"), w));
  me.listen<Weather | null>("weather", (e) => renderWeather($("weather"), e.payload));
  $("weather").addEventListener("click", () => invoke("open_widgets"));

  // --- Status icons (network, volume, battery) open Windows Quick Settings.
  invoke<Status>("get_status").then((s) => renderStatus($("status"), s));
  me.listen<Status>("status", (e) => renderStatus($("status"), e.payload));

  $("start").addEventListener("click", () => invoke("open_start"));
  $("status").addEventListener("click", (e) => {
    const open = (e.target as HTMLElement).closest<HTMLElement>("[data-open]")?.dataset.open;
    if (open === "volume-mixer") invoke("open_volume_mixer");
    else if (open) invoke("open_quick_settings");
  });

  // --- CPU / GPU / RAM and network speed; a click opens Task Manager.
  me.listen<Usage>("usage", (e) => renderUsage($("usage"), e.payload));
  $("usage").addEventListener("click", () => invoke("open_task_manager"));
  $("clock").addEventListener("click", () => invoke("open_notifications"));

  // --- Right-click: app menu on an app icon, Pane menu everywhere else (tray icons handle their own).
  document.addEventListener("contextmenu", async (e) => {
    e.preventDefault();
    const id = (e.target as HTMLElement).closest<HTMLElement>(".app")?.dataset.id;
    const app = apps.find((a) => a.id === id);
    const menu = await Menu.new({ items: app ? appMenu(app) : await paneMenu() });
    await menu.popup();
  });
}

/** Fills a (new or reused) app button with the app's current state. */
function updateAppButton(b: HTMLElement, app: AppItem) {
  b.classList.add("app");
  b.dataset.id = app.id;
  b.title = app.windows.length === 1 ? app.windows[0].title : app.name;
  b.setAttribute("aria-label", app.name);
  b.classList.toggle("running", app.windows.length > 0);
  b.classList.toggle("active", app.windows.some((w) => w.active));
  const img = b.querySelector("img");
  if (app.icon && img?.src !== app.icon) {
    b.replaceChildren(Object.assign(document.createElement("img"), { src: app.icon, alt: "" }));
  } else if (!app.icon) {
    b.textContent = app.name.slice(0, 1).toUpperCase();
  }
  b.onclick = () => clickApp(app, b);
}

/** No window: start it (icon bounces). One window: front / minimize. Several: cycle through them. */
function clickApp(app: AppItem, b: HTMLElement) {
  const wins = app.windows;
  if (!wins.length) {
    once(b, "launch");
    return invoke("launch", { id: app.id });
  }
  const i = wins.findIndex((w) => w.active);
  const next = wins.length === 1 ? wins[0] : wins[(i + 1) % wins.length];
  invoke("activate", { hwnd: next.hwnd });
}

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
