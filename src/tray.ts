import { invoke } from "@tauri-apps/api/core";

export type TrayIcon = { id: string; tip: string; icon: string; promoted: boolean };

/** Icon ids each box has shown, so only icons that are new get the pop-in animation. */
const seen = new WeakMap<HTMLElement, Set<string>>();

/** Buttons for tray icons. Clicks go to the app that owns the icon, like on the Windows taskbar. */
export function renderTray(box: HTMLElement, icons: TrayIcon[]) {
  const known = seen.get(box);
  seen.set(box, new Set(icons.map((t) => t.id)));
  box.replaceChildren(
    ...icons.map((t) => {
      const b = document.createElement("button");
      b.className = known && !known.has(t.id) ? "tray-icon enter" : "tray-icon";
      b.title = t.tip;
      b.dataset.id = t.id; // the icon's GUID or "hwnd:uid", also for test scripts
      b.setAttribute("aria-label", t.tip || "Tray icon");
      const img = document.createElement("img");
      img.src = t.icon;
      img.alt = "";
      b.append(img);
      // Screen position in physical pixels, so the app opens its menu right at the icon.
      const send = (kind: "left" | "right" | "double", e: MouseEvent) =>
        invoke("tray_click", {
          id: t.id,
          kind,
          x: Math.round(e.screenX * devicePixelRatio),
          y: Math.round(e.screenY * devicePixelRatio),
        });
      b.addEventListener("click", (e) => send("left", e));
      b.addEventListener("dblclick", (e) => send("double", e));
      b.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        e.stopPropagation();
        send("right", e);
      });
      return b;
    }),
  );
}
