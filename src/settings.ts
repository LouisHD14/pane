// The settings window: every setting of Pane, on six pages (the bar's right-click menu only has actions since
// 2026-11-16). Each row has an icon, a title, a line that says what it does, and for the visual switches a
// picture of off and on (pictures.ts). Every change goes to Rust at once and comes back to all pages as the
// "settings" event, so the bar, its menu and this window always agree.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { Settings, Switch } from "./menu";
import { icon, type IconName } from "./icons";
import { picture, type PictureKey } from "./pictures";

/** How often Pane's releases were downloaded (update.rs `Downloads`). */
type Downloads = { total: number; installers: number; this_version: number | null; newest: string; releases: number };

/** Creates an element with properties and children. */
function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: object = {}, ...kids: (Node | string)[]): HTMLElementTagNameMap[K] {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...kids);
  return e;
}

const set = (key: keyof Settings, value: boolean | number | string) => invoke("set_setting", { key, value });

/** Things to refresh when the settings change (here or from elsewhere). */
const updates: ((s: Settings) => void)[] = [];
const track = (u: (s: Settings) => void) => updates.push(u);

let ids = 0;

/** A row: icon, title with its explanation (and picture), and the control on the right. `labelFor` ties the
 * title to the control for screen readers and clicks. */
function item(name: IconName, title: string, about: string | Node, control: Node | null, extra: Node | null = null, labelFor?: string) {
  const id = `d${++ids}`;
  const heading = labelFor ? el("label", { className: "item-title", htmlFor: labelFor }, title) : el("span", { className: "item-title" }, title);
  const text = el("div", { className: "item-text" }, heading, el("span", { className: "item-about", id }, about));
  if (control instanceof HTMLElement) control.setAttribute("aria-describedby", id);
  const row = el("div", { className: "item" }, el("span", { className: "item-icon" }, icon(name)), text);
  if (control) row.append(el("div", { className: "item-control" }, control));
  return extra ? el("div", { className: "item-wrap" }, row, extra) : row;
}

/** An on/off switch row; `pic` adds the off/on drawing under it. */
function toggle(key: Switch, name: IconName, title: string, about: string, pic?: PictureKey) {
  const input = el("input", { type: "checkbox", className: "switch", id: `s-${key}` });
  input.setAttribute("role", "switch");
  input.onchange = () => set(key, input.checked);
  track((s) => (input.checked = s[key]));
  return item(name, title, about, input, pic ? picture(pic, track) : null, input.id);
}

type NumKey = "glass" | "opacity" | "hide_delay_ms" | "slide_ms" | "custom_seconds";

/** A slider row; `live` sends while dragging (for what is seen at once), otherwise once on release. */
function slider(key: NumKey, name: IconName, title: string, about: string, min: number, max: number, step: number, show: (v: number) => string, live = false) {
  const id = `s-${key}`;
  const input = el("input", { type: "range", id, min: String(min), max: String(max), step: String(step) });
  const out = el("output");
  out.setAttribute("for", id);
  const label = (v: number) => {
    out.textContent = show(v);
    input.setAttribute("aria-valuetext", show(v));
  };
  input.addEventListener(live ? "input" : "change", () => set(key, Number(input.value)));
  input.addEventListener("input", () => label(Number(input.value)));
  track((s) => {
    if (!input.matches(":active")) input.value = String(s[key]); // not under the dragging mouse
    label(s[key]);
  });
  return item(name, title, about, el("div", { className: "range" }, input, out), null, id);
}

/** The user's own widget: a command whose first line shows on the bar (settings.rs `custom_command`). */
function customWidget() {
  const input = el("input", { type: "text", className: "text", id: "s-custom", placeholder: "e.g. git -C C:\\code\\app branch --show-current", spellcheck: false });
  input.onchange = () => set("custom_command", input.value.trim());
  track((s) => {
    if (document.activeElement !== input) input.value = s.custom_command;
  });
  return [
    item("window_console", "Run a command", "A command that runs hidden in your user folder; its first line shows on the bar. Click it there to run it now. Empty: off.", null),
    el("div", { className: "item sub" }, el("label", { htmlFor: "s-custom", className: "item-title" }, "Command"), input),
    slider("custom_seconds", "timer", "Runs every", "How often the command runs again.", 2, 600, 1, (v) =>
      v < 60 ? `${v} s` : `${Math.floor(v / 60)} min${v % 60 ? ` ${v % 60} s` : ""}`,
    ),
  ];
}

/** What a double-click on an empty part of the bar does (settings.rs `void_action`); "other" is a program. */
function voidAction() {
  const choices = [
    ["none", "Nothing"],
    ["terminal", "Open a terminal"],
    ["explorer", "Open File Explorer"],
    ["taskmgr", "Open Task Manager"],
    ["desktop", "Show the desktop"],
    ["other", "Open an app…"],
  ] as const;
  const known = choices.map(([v]) => v as string).filter((v) => v !== "other");
  const select = el("select", { id: "s-void", className: "select" }, ...choices.map(([value, text]) => el("option", { value }, text)));
  const program = el("input", { type: "text", className: "text", placeholder: "e.g. notepad, or C:\\Path\\App.exe", spellcheck: false });
  program.setAttribute("aria-label", "App to open");
  const programRow = el("div", { className: "item sub" }, el("span", { className: "item-about" }, "A program name, a path, or a command line"), program);
  const show = (v: string) => {
    const other = !known.includes(v);
    select.value = other ? "other" : v;
    programRow.hidden = !other;
    if (other && document.activeElement !== program) program.value = v;
  };
  select.onchange = () => {
    if (select.value === "other") {
      programRow.hidden = false;
      program.focus();
      if (program.value.trim()) set("void_action", program.value.trim());
    } else set("void_action", select.value);
  };
  program.onchange = () => program.value.trim() && set("void_action", program.value.trim());
  track((s) => show(s.void_action));
  return [item("cursor_click", "Double-click on the empty bar", "What happens when you double-click a free spot of the bar.", select, null, "s-void"), programRow];
}

function sizes() {
  const choices = [
    ["small", "Small"],
    ["normal", "Normal"],
    ["large", "Large"],
  ] as const;
  const radios = choices.map(([value, text]) => {
    const input = el("input", { type: "radio", name: "size", value });
    input.onchange = () => input.checked && set("size", value);
    track((s) => (input.checked = s.size === value));
    return el("label", { className: "choice" }, input, text);
  });
  const box = el("div", { className: "choices", role: "radiogroup" }, ...radios);
  box.setAttribute("aria-label", "Bar height");
  return item("arrow_autofit_height", "Bar height", "Small 40, normal 48 or large 56 px; the icons grow with it.", box);
}

function button(text: string, onclick: () => unknown) {
  return el("button", { className: "button", textContent: text, onclick });
}

const seconds = (ms: number) => `${(ms / 1000).toFixed(ms % 100 ? 2 : 1)} s`;

/** A heading inside a page, for a group of rows. */
const section = (title: string, ...rows: (Node | Node[])[]) =>
  el("section", { className: "section" }, el("h2", {}, title), el("div", { className: "s-card" }, ...rows.flat()));

export async function settingsPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("settings");
  document.documentElement.classList.add("settings");
  // close_popup destroys the calling window (its hold release is a no-op here: menus close when this takes focus).
  document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("close_popup"));

  // --- General: things that are actions or about Pane itself.
  const autostart = el("input", { type: "checkbox", className: "switch", id: "s-autostart" });
  autostart.setAttribute("role", "switch");
  autostart.onchange = async () => {
    await invoke("set_autostart", { on: autostart.checked });
    autostart.checked = await invoke<boolean>("autostart");
  };

  const status = el("span", { textContent: "Pane looks for updates by itself and shows a pill on the bar when one is ready." });
  status.setAttribute("role", "status");
  const check = button("Check for updates", () => {});
  const checkUpdate = async () => {
    check.disabled = true;
    status.textContent = "Checking…";
    const version = await invoke<string | null>("check_update").catch(() => null);
    status.textContent = version ? `Pane ${version} is available.` : "Pane is up to date.";
    if (version) {
      check.textContent = `Install ${version}`;
      check.onclick = () => {
        check.disabled = true;
        status.textContent = "Updating… Pane restarts.";
        invoke("install_update").catch(() => {
          status.textContent = "The update failed.";
          check.disabled = false;
        });
      };
    }
    check.disabled = false;
  };
  check.onclick = checkUpdate;

  // How often Pane's releases were downloaded, read from GitHub when the General page is first shown. Keeps
  // nothing and sends nothing about this PC; it only reads the public release page.
  const counts = el("span", { textContent: "How often Pane was downloaded from GitHub." });
  counts.setAttribute("role", "status");
  const refresh = button("Refresh", () => loadDownloads());
  let downloadsLoaded = false;
  const loadDownloads = async () => {
    downloadsLoaded = true;
    refresh.disabled = true;
    counts.textContent = "Reading GitHub…";
    try {
      const d = await invoke<Downloads>("get_downloads");
      const n = (v: number) => v.toLocaleString();
      const mine = d.this_version === null ? "" : ` · ${n(d.this_version)} on this version`;
      counts.textContent = `${n(d.installers)} installers downloaded${mine} · ${n(d.total)} files over ${d.releases} release${d.releases === 1 ? "" : "s"}`;
    } catch (e) {
      counts.textContent = `Couldn't read the download counts (${e}).`;
    }
    refresh.disabled = false;
  };

  const version = el("span");
  invoke<string>("app_version").then((v) => (version.textContent = `Pane ${v}. The log (pane.log) helps with bug reports.`));

  const lightness = (v: number) => (v === 50 ? "Default" : v < 50 ? `Darker ${50 - v}` : `Lighter ${v - 50}`);
  const opacity = (v: number) => (v === 50 ? "Default" : v === 0 ? "Clear" : v === 100 ? "Solid" : v < 50 ? `Clearer ${50 - v}` : `More solid ${v - 50}`);

  const PAGES: { id: string; title: string; icon: IconName; build: () => Node[] }[] = [
    {
      id: "look",
      title: "Look",
      icon: "paint_brush",
      build: () => [
        section(
          "Shape",
          toggle("fill", "full_screen_maximize", "Fill the screen edge", "The bar spans the whole bottom edge with square corners, instead of floating.", "fill"),
          sizes(),
        ),
        section(
          "Colour",
          toggle("adaptive_color", "color", "Match the colour of apps", "The glass takes on the colours of the app right above the bar.", "adaptive_color"),
          toggle("blend", "color_fill", "App colour under the bar", "The app's colours fill the strip under the bar; the bar floats on top. Not with “Fill the screen edge”.", "blend"),
          slider("glass", "brightness_high", "Glass lightness", "Darker or lighter glass.", 0, 100, 5, lightness, true),
          slider("opacity", "transparency_square", "Glass opacity", "How much of what's behind shows through.", 0, 100, 5, opacity, true),
        ),
      ],
    },
    {
      id: "autohide",
      title: "Auto-hide",
      icon: "eye_off",
      build: () => [
        section(
          "Auto-hide",
          toggle("autohide", "eye_off", "Auto-hide the bar", "The bar slides away when the mouse leaves it and comes back at the bottom edge. Apps get the whole screen.", "autohide"),
          toggle("win_key", "keyboard", "Come up with the Windows key", "While Start or Search is open, the bar shows."),
        ),
        section(
          "Timing",
          item("panel_bottom", "Like the Windows taskbar", "Comes up at once, waits briefly, slides fast.", button("Taskbar-like", () => {
            set("hide_delay_ms", 250);
            set("slide_ms", 150);
          })),
          slider("hide_delay_ms", "timer", "Stays after the mouse leaves", "How long the bar waits before it slides away.", 100, 3000, 50, (v) => seconds(v) + (v === 700 ? " (default)" : "")),
          slider("slide_ms", "top_speed", "Slide duration", "How long the slide takes. 0 is instant.", 0, 600, 10, (v) => (v ? `${v} ms` : "Instant") + (v === 260 ? " (default)" : "")),
        ),
      ],
    },
    {
      id: "items",
      title: "Taskbar items",
      icon: "apps_list",
      build: () => [
        section(
          "Apps",
          toggle("all_monitors", "window_multiple", "Show apps from all monitors", "Each bar lists the windows of every monitor, not only its own. Pinned apps are on every bar either way.", "all_monitors"),
        ),
        section(
          "Show on the bar",
          toggle("show_start", "grid", "Start button", "Opens the Start menu."),
          toggle("show_tray", "chevron_up", "Tray icons", "Icons of apps running in the background (Discord, Steam, …)."),
          toggle("show_network", "wifi_1", "Network", "Opens Quick Settings."),
          toggle("show_volume", "speaker_2", "Volume", "Opens the volume mixer."),
          toggle("show_battery", "battery_6", "Battery", "Only on PCs with a battery."),
          toggle("show_clock", "clock", "Time", "The clock on the right."),
          toggle("show_date", "calendar_ltr", "Date", "Weekday and date next to the time."),
          toggle("show_awake", "drink_coffee", "Keep-awake button", "While it's on, the PC and screen don't go to sleep."),
          toggle("show_desktops", "tab_desktop_multiple", "Virtual desktops", "A dot per desktop: click one to switch, or scroll over them."),
        ),
      ],
    },
    {
      id: "widgets",
      title: "Widgets",
      icon: "board",
      build: () => [
        section(
          "Widgets",
          toggle("show_media", "music_note_2", "Now Playing", "Song, artist and play/pause/next for Spotify, browsers and other players."),
          toggle("show_claude", "sparkle", "Claude usage", "Your 5-hour and weekly Claude limits. Click it to open the usage page."),
          toggle("show_codex", "code", "Codex usage", "The same for Codex, when the Codex CLI is logged in."),
          toggle("show_usage", "data_usage", "CPU / GPU / RAM", "Live load and network speed."),
          toggle("show_weather", "weather_partly_cloudy_day", "Weather", "Temperature under the time. Click it for the forecast."),
          toggle("show_docker", "box_multiple", "Docker", "Running containers, only while Docker runs."),
        ),
        section("Your own widget", customWidget()),
      ],
    },
    {
      id: "input",
      title: "Mouse & keyboard",
      icon: "cursor_click",
      build: () => [
        section(
          "Mouse",
          toggle("wheel", "arrow_bidirectional_up_down", "Scroll wheel", "Scroll on the volume icon to change the volume, on an app to step through its windows."),
          voidAction(),
        ),
        section(
          "Keyboard",
          item("keyboard", "Hide or show Pane", "Ctrl + Alt + P switches between Pane and the Windows taskbar.", null),
        ),
      ],
    },
    {
      id: "general",
      title: "General",
      icon: "settings",
      build: () => {
        if (!downloadsLoaded) loadDownloads();
        return [
          section(
            "Windows",
            item("power", "Start with Windows", "Also under “Startup apps” in Task Manager; switching it off there switches it off here.", autostart, null, "s-autostart"),
            toggle("low_memory", "developer_board", "Low memory mode", "No GPU for Pane's windows: about 20 MB less. Applies after a restart."),
          ),
          section(
            "Updates",
            item("arrow_sync", "Updates", status, check),
            item("arrow_download", "Downloads", counts, refresh),
          ),
          section(
            "Help",
            item("bug", "Report a bug", "Opens a bug report on GitHub with Pane's version filled in.", button("Report a bug…", () => invoke("report_bug"))),
            item("folder_open", "Log folder", version, button("Open log folder", () => invoke("open_log_folder"))),
          ),
        ];
      },
    },
  ];
  const nav = el("nav", { className: "nav" });
  nav.setAttribute("aria-label", "Settings pages");
  const content = el("main", { className: "content", tabIndex: -1 });
  const built = new Map<string, HTMLElement>();
  const show = (id: string) => {
    const page = PAGES.find((p) => p.id === id) ?? PAGES[0];
    if (!built.has(page.id)) {
      const body = el("div", { className: "page" }, el("h1", {}, page.title), ...page.build());
      built.set(page.id, body);
      if (current) apply(current);
    }
    content.replaceChildren(built.get(page.id)!);
    content.scrollTop = 0;
    nav.querySelectorAll("button").forEach((b) => {
      if (b.dataset.page === page.id) b.setAttribute("aria-current", "page");
      else b.removeAttribute("aria-current");
    });
    try {
      localStorage.setItem("settings-page", page.id);
    } catch {
      /* storage may be off; the window then opens on the first page */
    }
  };
  for (const p of PAGES) {
    const b = el("button", { className: "nav-item", type: "button" }, icon(p.icon), el("span", {}, p.title));
    b.dataset.page = p.id;
    b.setAttribute("aria-label", p.title); // the text hides when the window is narrow
    b.onclick = () => show(p.id);
    nav.append(b);
  }
  document.body.replaceChildren(el("div", { className: "settings-shell" }, el("div", { className: "nav-wrap" }, el("div", { className: "brand" }, "Pane settings"), nav), content));

  let current: Settings | null = null;
  const apply = (s: Settings) => {
    current = s;
    updates.forEach((u) => u(s));
  };
  me.listen<Settings>("settings", (e) => apply(e.payload));
  let first = "look";
  try {
    first = localStorage.getItem("settings-page") ?? first;
  } catch {
    /* see above */
  }
  const [s, auto] = await Promise.all([invoke<Settings>("get_settings"), invoke<boolean>("autostart")]);
  current = s;
  show(first);
  apply(s);
  autostart.checked = auto;
}
