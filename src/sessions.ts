// The popup over the Claude widget (label "agents-<bar>"): the open Claude Code sessions, working or waiting,
// and the plan's limits. It opens when the mouse rests on the widget, without taking the focus, and closes when
// the mouse leaves (main.rs close_when_left). A click on a session brings its terminal to the front.
// It replaced the "done" glow and ✓ badge (the user's ideas.md, 2026-11-24).

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { claudeTip, type ClaudeUsage } from "./claude";

type Session = { name: string; project: string; busy: boolean; since: number; hwnd: number };

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

/** "just now", "5 min", "2 h", "3 d" since `ms` (Unix ms); "" if unknown. */
export function ago(ms: number, now = Date.now()): string {
  if (!ms) return "";
  const min = Math.floor((now - ms) / 60_000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min`;
  if (min < 24 * 60) return `${Math.floor(min / 60)} h`;
  return `${Math.floor(min / 1440)} d`;
}

export async function agentsPage() {
  const me = getCurrentWebviewWindow();
  const box = Object.assign(document.createElement("div"), { className: "docker-list agents-list" });
  box.setAttribute("role", "dialog");
  box.setAttribute("aria-label", "Claude Code sessions");
  document.body.replaceChildren(box);
  const close = () => invoke("close_popup");
  document.addEventListener("keydown", (e) => e.key === "Escape" && close());

  const render = async () => {
    const [list, usage] = await Promise.all([invoke<Session[]>("get_sessions"), invoke<ClaudeUsage | null>("get_claude")]);
    const working = list.filter((s) => s.busy).length;
    const head = `<div class="docker-head"><span><b>Claude Code</b> · ${
      list.length ? `${working} working, ${list.length - working} waiting` : "no sessions open"
    }</span></div>`;
    const rows = list
      .map((s) => {
        const state = s.busy ? "working" : `waiting${s.since ? ` · ${ago(s.since)}` : ""}`;
        const where = s.project && s.project !== s.name ? `${esc(s.project)} · ` : "";
        const label = `${s.name}, ${s.project}, ${state}${s.hwnd ? "" : ", no terminal window open"}`;
        return `<button class="docker-row agent-row ${s.busy ? "running busy-agent" : ""}" data-hwnd="${s.hwnd}"${
          s.hwnd ? "" : " disabled"
        } aria-label="${esc(label)}" title="${s.hwnd ? "Show its terminal" : "No terminal window open (background session)"}">
          <span class="dot" aria-hidden="true"></span>
          <span class="what"><span class="name">${esc(s.name)}</span><small>${where}${state}${s.hwnd ? "" : " · no window"}</small></span>
        </button>`;
      })
      .join("");
    const plan = usage ? `<div class="agents-plan">${claudeTip(usage).split("\n").map(esc).join("<br>")}</div>` : "";
    box.innerHTML = head + rows + plan;
    box.querySelectorAll<HTMLButtonElement>(".agent-row").forEach((b) => {
      b.onclick = () => {
        invoke("focus_window", { hwnd: Number(b.dataset.hwnd) });
        close();
      };
    });
    invoke("fit_popup", { width: box.offsetWidth, height: box.scrollHeight, centered: true });
  };

  // States change while it is open ("working" -> "waiting"): read again every 2 s, only while it shows.
  let timer = 0;
  const open = () => {
    clearInterval(timer);
    timer = window.setInterval(render, 2000);
    box.style.animation = "none";
    void box.offsetWidth;
    box.style.animation = "";
    render();
  };
  me.listen("popup-open", open);
  me.listen("popup-closed", () => clearInterval(timer));
  open();
}
