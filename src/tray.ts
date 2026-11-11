import { invoke } from "@tauri-apps/api/core";

export type TrayIcon = { id: string; tip: string; icon: string; promoted: boolean };

/** Buttons for tray icons. Clicks go to the app that owns the icon, like on the Windows taskbar. */
export function renderTray(box: HTMLElement, icons: TrayIcon[]) {
  box.replaceChildren(
    ...icons.map((t) => {
      const b = document.createElement("button");
      b.className = "tray-icon";
      b.title = t.tip;
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
