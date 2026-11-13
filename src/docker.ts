// Docker: a small widget with the running containers while Docker runs (hidden otherwise), and a popup above it
// with every container, its state and start / stop / restart.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

export type Container = {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  project: string | null;
  ports: string[];
};
export type Docker = { running: number; containers: Container[] };

/** Stacked boxes (own drawing, no brand logo). */
const GLYPH = `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="11" width="5" height="5" rx="1"/><rect x="9.5" y="11" width="5" height="5" rx="1"/><rect x="16" y="11" width="5" height="5" rx="1"/><rect x="9.5" y="4.5" width="5" height="5" rx="1"/><path d="M2.5 18.5c3 2 15 2 19 0"/></svg>`;

export function renderDocker(el: HTMLElement, d: Docker | null) {
  el.hidden = !d;
  if (!d) return;
  const total = d.containers.length;
  const html = `${GLYPH}<span class="count">${d.running}</span>`;
  if (el.dataset.html !== html) {
    el.innerHTML = html;
    el.dataset.html = html;
  }
  el.classList.toggle("idle", d.running === 0);
  const tip = `Docker: ${d.running} of ${total} container${total === 1 ? "" : "s"} running`;
  el.title = tip;
  el.setAttribute("aria-label", `${tip}. Show containers`);
}

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

const icon = (d: string) => `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="${d}"/></svg>`;
const STOP = icon("M4.5 4.5h7v7h-7z");
const START = icon("M5 3.5v9l7.5-4.5z");
const RESTART = icon("M12.5 8a4.5 4.5 0 1 1-1.3-3.2M12.5 3v2.8H9.7");

/** The popup above the widget (label "docker-<bar>"). Sizes itself, like the Pane menu. */
export async function dockerPage() {
  const me = getCurrentWebviewWindow();
  document.body.classList.add("docker-popup");
  const box = Object.assign(document.createElement("div"), { className: "docker-list" });
  box.setAttribute("role", "dialog");
  box.setAttribute("aria-label", "Docker containers");
  document.body.replaceChildren(box);
  const close = () => invoke("close_popup");
  window.addEventListener("blur", close);
  document.addEventListener("keydown", (e) => e.key === "Escape" && close());

  let busy = new Set<string>();
  let error = "";
  const fit = () => invoke("fit_popup", { width: box.offsetWidth, height: box.scrollHeight, centered: true });

  const render = (d: Docker | null) => {
    if (!d) return close(); // Docker stopped
    const head = `<div class="docker-head"><span><b>Docker</b> · ${d.running} running</span><button class="docker-open">Open Docker Desktop</button></div>`;
    const rows = d.containers.length
      ? d.containers
          .map((c) => {
            const running = c.state === "running";
            const where = [c.project, c.image].filter(Boolean).map((t) => esc(t!)).join(" · ");
            const ports = c.ports.length ? ` · ${esc(c.ports.join(", "))}` : "";
            const wait = busy.has(c.id) ? " disabled" : "";
            return `<div class="docker-row ${esc(c.state)}" data-id="${esc(c.id)}">
              <span class="dot" aria-hidden="true"></span>
              <span class="what"><span class="name">${esc(c.name)}</span><small>${where}${ports}</small></span>
              <small class="when">${esc(c.status)}</small>
              <span class="acts">
                ${running ? `<button data-act="restart" title="Restart" aria-label="Restart ${esc(c.name)}"${wait}>${RESTART}</button>` : ""}
                <button data-act="${running ? "stop" : "start"}" title="${running ? "Stop" : "Start"}" aria-label="${running ? "Stop" : "Start"} ${esc(c.name)}"${wait}>${running ? STOP : START}</button>
              </span>
            </div>`;
          })
          .join("")
      : `<div class="docker-empty">No containers</div>`;
    box.innerHTML = head + rows + (error ? `<div class="docker-error" role="alert">${esc(error)}</div>` : "");
    box.querySelector<HTMLButtonElement>(".docker-open")!.onclick = () => {
      invoke("open_docker_desktop");
      close();
    };
    box.querySelectorAll<HTMLButtonElement>("[data-act]").forEach((b) => {
      b.onclick = async () => {
        const id = b.closest<HTMLElement>(".docker-row")!.dataset.id!;
        busy.add(id);
        error = "";
        b.closest(".docker-row")!.classList.add("busy");
        b.disabled = true;
        await invoke("docker_action", { id, action: b.dataset.act }).catch((e) => (error = String(e)));
        busy.delete(id);
        render(await invoke<Docker | null>("get_docker"));
      };
    });
    fit();
  };

  me.listen<Docker | null>("docker", (e) => render(e.payload));
  // A kept popup opened again: fresh state, opening animation again.
  me.listen("popup-open", async () => {
    busy = new Set();
    error = "";
    box.style.animation = "none";
    void box.offsetWidth;
    box.style.animation = "";
    render(await invoke<Docker | null>("get_docker"));
  });
  render(await invoke<Docker | null>("get_docker"));
}
