// The settings window ("More settings…" in the bar's menu): every switch plus glass lightness, auto-hide timing
// and bar size. Every change goes to Rust at once and comes back to all pages as the "settings" event.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { Settings, Switch } from "./menu";

/** How often Pane's releases were downloaded (update.rs `Downloads`). */
type Downloads = { total: number; installers: number; this_version: number | null; newest: string; releases: number };

/** Creates an element with properties and children. */
function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: object = {}, ...kids: (Node | string)[]): HTMLElementTagNameMap[K] {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...kids);
  return e;
}

const set = (key: keyof Settings, value: boolean | number | string) => invoke("set_setting", { key, value });

/** Things to refresh when the settings change (here or from the bar's menu). */
const updates: ((s: Settings) => void)[] = [];

function toggle(key: Switch, text: string) {
  const input = el("input", { type: "checkbox", className: "switch" });
  input.setAttribute("role", "switch");
  input.onchange = () => set(key, input.checked);
  updates.push((s) => (input.checked = s[key]));
  return el("label", { className: "row" }, el("span", {}, text), input);
}

type NumKey = "glass" | "hide_delay_ms" | "slide_ms";

/** A slider; `live` sends while dragging (for what is seen at once), otherwise once on release. */
function slider(key: NumKey, text: string, min: number, max: number, step: number, show: (v: number) => string, live = false) {
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
  updates.push((s) => {
    if (!input.matches(":active")) input.value = String(s[key]); // not under the dragging mouse
    label(s[key]);
  });
  return el("div", { className: "row slider" }, el("label", { htmlFor: id }, text), out, input);
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
    updates.push((s) => (input.checked = s.size === value));
    return el("label", { className: "choice" }, input, text);
  });
  return el("fieldset", { className: "row choices" }, el("legend", {}, "Bar height"), el("div", {}, ...radios));
}

function group(title: string, ...rows: Node[]) {
  return el("fieldset", { className: "group" }, el("legend", {}, title), ...rows);
}

const seconds = (ms: number) => `${(ms / 1000).toFixed(ms % 100 ? 2 : 1)} s`;

export async function settingsPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("settings");
  document.documentElement.classList.add("settings");
  // close_popup destroys the calling window (its hold release is a no-op here: menus close when this takes focus).
  document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("close_popup"));

  const autostart = el("input", { type: "checkbox", className: "switch" });
  autostart.setAttribute("role", "switch");
  autostart.onchange = async () => {
    await invoke("set_autostart", { on: autostart.checked });
    autostart.checked = await invoke<boolean>("autostart");
  };

  const status = el("span", { className: "note" });
  status.setAttribute("role", "status");
  const check = el("button", { className: "button", textContent: "Check for updates" });
  check.onclick = async () => {
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

  const version = el("span", { className: "note" });

  // How often Pane's releases were downloaded, read from GitHub when this window opens. Keeps nothing and
  // sends nothing about this PC; it only reads the public release page.
  const counts = el("span", { className: "note" });
  counts.setAttribute("role", "status");
  const refresh = el("button", { className: "button", textContent: "Refresh" });
  const loadDownloads = async () => {
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
  refresh.onclick = loadDownloads;
  const lightness = (v: number) => (v === 50 ? "Default" : v < 50 ? `Darker ${50 - v}` : `Lighter ${v - 50}`);
  document.body.replaceChildren(
    el(
      "main",
      { className: "settings-page" },
      el("h1", {}, "Pane settings"),
      group(
        "Bar",
        toggle("autohide", "Auto-hide the bar"),
        toggle("fill", "Fill the screen edge"),
        toggle("adaptive_color", "Match the colour of apps"),
        toggle("blend", "App colour under the bar"),
        toggle("all_monitors", "Show apps from all monitors"),
        sizes(),
        slider("glass", "Glass lightness", 0, 100, 5, lightness, true),
      ),
      group(
        "Auto-hide",
        el(
          "div",
          { className: "row" },
          el("span", { className: "note" }, "Quick like Windows' own taskbar: comes up at once, waits briefly, slides fast."),
          el("button", {
            className: "button",
            textContent: "Taskbar-like",
            onclick: () => {
              set("hide_delay_ms", 250);
              set("slide_ms", 150);
            },
          }),
        ),
        slider("hide_delay_ms", "Stays after the mouse leaves", 100, 3000, 50, (v) => seconds(v) + (v === 700 ? " (default)" : "")),
        slider("slide_ms", "Slide duration", 0, 600, 10, (v) => (v ? `${v} ms` : "Instant") + (v === 260 ? " (default)" : "")),
      ),
      group(
        "Widgets",
        toggle("show_media", "Now Playing"),
        toggle("show_claude", "Claude usage"),
        toggle("show_codex", "Codex usage"),
        toggle("show_usage", "CPU / GPU / RAM"),
        toggle("show_weather", "Weather"),
        toggle("show_docker", "Docker (only while Docker runs)"),
        toggle("done_glow", "Glow when Claude Code / Codex finish a task"),
      ),
      group(
        "Show on the bar",
        toggle("show_start", "Start button"),
        toggle("show_tray", "Tray icons"),
        toggle("show_network", "Network"),
        toggle("show_volume", "Volume"),
        toggle("volume_media", "Music controls at the volume icon"),
        toggle("show_battery", "Battery"),
        toggle("show_clock", "Time"),
        toggle("show_date", "Date"),
      ),
      group("Memory", toggle("low_memory", "Low memory mode (no GPU, applies after restart)")),
      group(
        "Windows",
        el("label", { className: "row" }, el("span", {}, "Start with Windows"), autostart),
        el(
          "div",
          { className: "row" },
          el("span", { className: "note" }, "Also under “Startup apps” in Task Manager — switching it off there switches it off here."),
        ),
      ),
      group("Updates", el("div", { className: "row" }, status, check)),
      group("Downloads", el("div", { className: "row" }, counts, refresh)),
      group(
        "Help",
        el(
          "div",
          { className: "row" },
          el("span", { className: "note" }, "Something wrong? Opens a bug report on GitHub with Pane's version filled in."),
          el("button", { className: "button", textContent: "Report a bug…", onclick: () => invoke("report_bug") }),
        ),
        el(
          "div",
          { className: "row" },
          version,
          el("button", { className: "button", textContent: "Open log folder", onclick: () => invoke("open_log_folder") }),
        ),
      ),
    ),
  );
  invoke<string>("app_version").then((v) => (version.textContent = `Pane ${v} · the log (pane.log) helps with bug reports`));
  loadDownloads();

  const apply = (s: Settings) => updates.forEach((u) => u(s));
  me.listen<Settings>("settings", (e) => apply(e.payload));
  const [s, auto] = await Promise.all([invoke<Settings>("get_settings"), invoke<boolean>("autostart")]);
  apply(s);
  autostart.checked = auto;
  document.querySelector<HTMLInputElement>("input")?.focus();
}
