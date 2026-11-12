import { invoke } from "@tauri-apps/api/core";
import { Menu, type MenuItemOptions, type PredefinedMenuItemOptions } from "@tauri-apps/api/menu";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { renderStatus, type Status } from "./status";
import { renderTray, type TrayIcon } from "./tray";
import { mountMedia, renderMedia, type NowPlaying } from "./media";
import { renderWeather, type Weather } from "./weather";
import { renderUsage, type Usage } from "./usage";
import { once, replay } from "./anim";
import { renderClaude, renderCodex, setClaudeLogo, type ClaudeUsage, type CodexUsage } from "./claude";
import { applyTint } from "./tint";
import { menuPage, type Settings } from "./menu";
import { reorderPins } from "./reorder";
import { settingsPage } from "./settings";

type Progress = { state: "normal" | "paused" | "error" | "indeterminate"; value: number };
type WinItem = {
  hwnd: number;
  title: string;
  active: boolean;
  badge: string;
  badge_tip: string;
  flashing: boolean;
  progress: Progress | null;
};
type AppItem = { id: string; name: string; icon: string; pinned: boolean; windows: WinItem[] };
type Item = MenuItemOptions | PredefinedMenuItemOptions;

/** Widget switches hide their widget with a class on <body> (the widget keeps updating, it's just not shown). */
const WIDGETS = ["media", "claude", "codex", "usage", "weather"] as const;
let settings: Settings | null = null;
function applySettings(s: Settings) {
  settings = s;
  for (const w of WIDGETS) document.body.classList.toggle(`no-${w}`, !s[`show_${w}`]);
  document.body.classList.toggle("fill", s.fill);
  document.body.classList.toggle("blend", s.blend && !s.fill);
  if (!s.adaptive_color && !s.blend) applyTint(null);
}

/** Bar contents scale with the bar height (settings.rs `Size::height`; 48 px is what the CSS sizes are for). */
const ZOOM = { small: 40 / 48, normal: 1, large: 56 / 48 };

/** Glass lightness (50 = the original look; Rust changes the acrylic tint) and bar size, on every page. */
function applyLook(s: Settings) {
  const root = document.documentElement.style;
  // Lighter: a white sheen over the glass. Darker: only the app-colour floor's slab needs a shade (no acrylic under it).
  root.setProperty("--wash", String((Math.max(0, s.glass - 50) / 50) * 0.2));
  root.setProperty("--shade", String((Math.max(0, 50 - s.glass) / 50) * 0.4));
  root.setProperty("--bar-zoom", String(ZOOM[s.size] ?? 1));
}

const me = getCurrentWebviewWindow();
const $ = (id: string) => document.getElementById(id)!;

invoke<Settings>("get_settings").then(applyLook);
me.listen<Settings>("settings", (e) => applyLook(e.payload));

// The popups ("hidden icons", Pane menu, window preview) and the settings window load the same page; the window
// label tells them apart from a bar.
if (me.label.startsWith("overflow-")) overflowPage();
else if (me.label.startsWith("menu-")) menuPage();
else if (me.label.startsWith("preview-")) previewPage();
else if (me.label === "settings") settingsPage();
else barPage();

type ThumbButton = { id: number; icon: string; tip: string; enabled: boolean; inert: boolean; flat: boolean; dismiss: boolean };
type Card = {
  hwnd: number;
  title: string;
  active: boolean;
  rect: number[];
  body: number[];
  live: boolean;
  buttons: ThumbButton[];
  bar: number[];
};
type Preview = { icon: string; cards: Card[] };

/** Window previews above an app icon. Rust lays out the cards and DWM draws the live images into each `body`. */
function previewPage() {
  document.body.classList.add("preview");
  const px = (el: HTMLElement, [x, y, w, h]: number[]) =>
    Object.assign(el.style, { left: `${x}px`, top: `${y}px`, width: `${w}px`, height: `${h}px` });
  const closeWindow = (c: Card) => invoke("close_window", { hwnd: c.hwnd });
  // The app's own buttons (play/pause/next...), like under Windows' previews; a click reaches the app as WM_COMMAND.
  const toolbar = (c: Card) => {
    const bar = Object.assign(document.createElement("div"), { className: "card-buttons" });
    bar.setAttribute("role", "toolbar");
    bar.setAttribute("aria-label", `${c.title} controls`);
    px(bar, c.bar);
    bar.append(
      ...c.buttons.map((t) => {
        const b = Object.assign(document.createElement("button"), { className: "thumb-button", title: t.tip, disabled: !t.enabled });
        b.setAttribute("aria-label", t.tip || `Button ${t.id}`);
        b.classList.toggle("flat", t.flat);
        if (t.icon) b.append(Object.assign(document.createElement("img"), { src: t.icon, alt: "" }));
        if (t.inert) {
          b.classList.add("inert");
          b.tabIndex = -1;
        } else {
          b.onclick = () => {
            invoke("thumb_button", { hwnd: c.hwnd, id: t.id });
            if (t.dismiss) invoke("hide_preview");
          };
        }
        return b;
      }),
    );
    return bar;
  };
  const show = (p: Preview | null) => {
    if (!p) return;
    document.body.replaceChildren(
      ...p.cards.map((c) => {
        const card = Object.assign(document.createElement("div"), { className: "card" });
        card.classList.toggle("active", c.active);
        px(card, c.rect);
        const main = Object.assign(document.createElement("button"), { className: "card-main", title: c.title });
        main.setAttribute("aria-label", `Switch to ${c.title}`);
        const title = Object.assign(document.createElement("span"), { className: "card-title", textContent: c.title });
        // Height of the title row = where the image starts.
        Object.assign(title.style, { height: `${c.body[1]}px`, lineHeight: `${c.body[1]}px` });
        const body = document.createElement("div");
        body.className = c.live ? "card-body" : "card-body minimized";
        px(body, c.body);
        if (!c.live && p.icon) body.append(Object.assign(document.createElement("img"), { src: p.icon, alt: "" }));
        main.append(title, body);
        main.onclick = () => invoke("focus_window", { hwnd: c.hwnd });
        const close = Object.assign(document.createElement("button"), { className: "card-close", textContent: "✕", title: "Close window" });
        close.setAttribute("aria-label", `Close ${c.title}`);
        close.onclick = () => closeWindow(c);
        card.append(main, close);
        if (c.buttons.length) card.append(toolbar(c));
        // Middle-click closes the window, like on Windows' previews (and never starts autoscroll).
        card.addEventListener("mousedown", (e) => e.button === 1 && e.preventDefault());
        card.addEventListener("auxclick", (e) => e.button === 1 && closeWindow(c));
        return card;
      }),
    );
  };
  // Listen first: Rust may send the layout right after get_preview answered "nothing yet".
  me.listen<Preview>("preview", (e) => show(e.payload)).then(() => invoke<Preview | null>("get_preview").then(show));
  document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("hide_preview"));
}

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
  window.addEventListener("blur", () => invoke("close_popup"));
  document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("close_popup"));
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
  let pending: AppItem[] | null = null; // came in during a drag, shown when it ends
  const renderApps = (list: AppItem[]) => {
    if (reorder.dragging()) return void (pending = list);
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
      b.onmouseenter = () => hoverApp(b);
      b.onmouseleave = () => clearTimeout(hoverTimer);
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
  // Pinned apps are dragged into Pane's own order (Rust saves it and sends it to every bar).
  const reorder = reorderPins($("apps"), {
    start: () => closePreview(),
    end: () => {
      const list = pending;
      pending = null;
      if (list) renderApps(list);
    },
    save: (order) => invoke("set_pin_order", { order }),
  });
  invoke<AppItem[]>("get_apps").then(renderApps);
  me.listen<AppItem[]>("apps", (e) => renderApps(e.payload));

  // --- Live preview after a short hover on a running app; quicker when one is already open (moving along the
  // icons). Rust closes it once the mouse has left icon and popup for a moment.
  let hoverTimer = 0;
  let previewOpen = false;
  const hoverApp = (b: HTMLElement) => {
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      const app = apps.find((a) => a.id === b.dataset.id);
      if (!app?.windows.length || !b.matches(":hover") || reorder.dragging()) return;
      const r = b.getBoundingClientRect();
      previewOpen = true;
      invoke("show_preview", { id: app.id, left: r.left, right: r.right });
    }, previewOpen ? 120 : 400);
  };
  me.listen("preview-closed", () => (previewOpen = false));
  const closePreview = () => {
    clearTimeout(hoverTimer);
    if (previewOpen) invoke("hide_preview");
  };
  $("apps").addEventListener("click", closePreview);

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

  // --- Claude plan usage (5-hour and weekly limit); a click opens the usage page on claude.ai.
  invoke<ClaudeUsage | null>("get_claude").then((u) => renderClaude($("claude"), u));
  invoke<string>("claude_icon").then((url) => url && setClaudeLogo(url));
  me.listen<ClaudeUsage | null>("claude", (e) => renderClaude($("claude"), e.payload));
  $("claude").addEventListener("click", () => invoke($("claude").classList.contains("relogin") ? "open_claude_login" : "open_claude_usage"));

  // --- Codex plan usage (shown when the Codex CLI is logged in with ChatGPT); a click opens chatgpt.com's usage.
  invoke<CodexUsage | null>("get_codex").then((u) => renderCodex($("codex"), u));
  me.listen<CodexUsage | null>("codex", (e) => renderCodex($("codex"), e.payload));
  $("codex").addEventListener("click", () => invoke($("codex").classList.contains("relogin") ? "open_codex_login" : "open_codex_usage"));

  // --- Settings (right-click menu) and the adaptive colour of the apps above the bar.
  invoke<Settings>("get_settings").then(applySettings);
  me.listen<Settings>("settings", (e) => applySettings(e.payload));
  me.listen<[number, number, number][] | null>("tint", (e) => applyTint(settings?.adaptive_color || settings?.blend ? e.payload : null));

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

  // --- Update: a pill when GitHub has a newer Pane; click installs it (Pane restarts).
  me.listen<{ version: string; notes: string }>("update", (e) => showUpdate(e.payload.version, e.payload.notes));
  $("update").addEventListener("click", async () => {
    const pill = $("update");
    const text = pill.textContent;
    pill.textContent = "Updating…";
    await invoke("install_update").catch(() => (pill.textContent = text));
  });

  // --- Right-click: app menu on an app icon, Pane menu (own popup, menu.ts) everywhere else (tray icons handle their own).
  document.addEventListener("contextmenu", async (e) => {
    e.preventDefault();
    if (reorder.dragging()) return;
    closePreview();
    const id =(e.target as HTMLElement).closest<HTMLElement>(".app")?.dataset.id;
    const app = apps.find((a) => a.id === id);
    if (!app) return invoke("open_menu", { clientX: e.clientX });
    const menu = await Menu.new({ items: appMenu(app) });
    // Auto-hide must not slide the bar away while its menu is open.
    await invoke("hold_bar", { on: true });
    try {
      await menu.popup();
    } finally {
      invoke("hold_bar", { on: false });
    }
  });
}

/** Fills a (new or reused) app button with the app's current state. */
function updateAppButton(b: HTMLElement, app: AppItem) {
  b.classList.add("app");
  b.classList.toggle("pinned", app.pinned);
  if (app.pinned) b.setAttribute("aria-keyshortcuts", "Alt+Shift+ArrowLeft Alt+Shift+ArrowRight"); // move it (reorder.ts)
  else b.removeAttribute("aria-keyshortcuts");
  b.dataset.id = app.id;
  // The app's badge (unread count / dot) from its first window that has one, like the Windows taskbar.
  const badged = app.windows.find((w) => w.badge);
  const tip = badged?.badge_tip ? ` (${badged.badge_tip})` : "";
  // Progress (a download, a copy) from its first window that has one, like the Windows taskbar.
  const progress = app.windows.find((w) => w.progress)?.progress;
  b.title = (app.windows.length === 1 ? app.windows[0].title : app.name) + tip;
  b.setAttribute("aria-label", app.name + tip + (progress ? `, ${progressText(progress)}` : ""));
  b.classList.toggle("running", app.windows.length > 0);
  b.classList.toggle("active", app.windows.some((w) => w.active));
  // Asks for attention (FlashWindowEx) until the window is brought to front.
  const flashing = app.windows.some((w) => w.flashing);
  // replay, not once: the badge's pop-in ending (animationend bubbles) would cut the pulses short.
  if (flashing && !b.classList.contains("flashing")) replay(b, "flash");
  b.classList.toggle("flashing", flashing);
  if (!flashing) b.classList.remove("flash");
  const img = b.querySelector<HTMLImageElement>("img.icon");
  if (app.icon && img?.src !== app.icon) {
    b.replaceChildren(Object.assign(document.createElement("img"), { className: "icon", src: app.icon, alt: "" }));
  } else if (!app.icon) {
    b.textContent = app.name.slice(0, 1).toUpperCase();
  }
  let badge = b.querySelector<HTMLImageElement>("img.badge");
  if (badged && !badge) {
    badge = b.appendChild(Object.assign(document.createElement("img"), { className: "badge", alt: "" }));
    once(badge, "pop-in");
  }
  if (badged && badge!.src !== badged.badge) badge!.src = badged.badge;
  if (!badged) badge?.remove();
  // A thin bar under the icon in place of the running line; CSS colours it by state (data-state).
  let bar = b.querySelector<HTMLElement>(".progress");
  if (progress) {
    bar ??= b.appendChild(Object.assign(document.createElement("span"), { className: "progress" }));
    bar.dataset.state = progress.state;
    bar.style.setProperty("--p", String(progress.value));
  } else bar?.remove();
  b.classList.toggle("progressing", !!progress);
  b.onclick = () => clickApp(app, b);
}

function progressText(p: Progress): string {
  const pct = `${Math.round(p.value * 100)} %`;
  return { normal: pct, paused: `paused at ${pct}`, error: `error at ${pct}`, indeterminate: "busy" }[p.state];
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

/** Shows the "Update x.y.z" pill (pops in the first time). */
function showUpdate(version: string, notes: string) {
  const pill = $("update");
  pill.textContent = `Update ${version}`;
  pill.title = notes || `Install Pane ${version} and restart`;
  if (pill.hidden) {
    pill.hidden = false;
    once(pill, "pop-in");
  }
}
