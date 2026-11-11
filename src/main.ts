import { invoke } from "@tauri-apps/api/core";
import { Menu } from "@tauri-apps/api/menu";

const dateEl = document.getElementById("date")!;
const timeEl = document.getElementById("time")!;

// Windows locale decides the format, e.g. "Mi., 11. Nov." / "Wed, Nov 11".
const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

function tick() {
  const now = new Date();
  dateEl.textContent = dateFmt.format(now);
  timeEl.textContent = timeFmt.format(now);
  // Wake up again right after the next full minute, not every second.
  setTimeout(tick, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
}
tick();

document.getElementById("start")!.addEventListener("click", () => invoke("open_start"));
document.getElementById("clock")!.addEventListener("click", () => invoke("open_notifications"));

document.addEventListener("contextmenu", async (e) => {
  e.preventDefault();
  const menu = await Menu.new({
    items: [
      { id: "hide", text: "Hide Pane (Ctrl+Alt+P)", action: () => invoke("toggle") },
      { id: "quit", text: "Quit Pane", action: () => invoke("quit") },
    ],
  });
  await menu.popup();
});
