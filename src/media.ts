import { invoke } from "@tauri-apps/api/core";
import { replay } from "./anim";

export type NowPlaying = {
  app: string;
  title: string;
  artist: string;
  cover: string;
  playing: boolean;
  position_ms: number;
  duration_ms: number;
  updated_at_ms: number;
};

const icon = (d: string) => `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="${d}"/></svg>`;
const PLAY = icon("M5 3.5v9l7.5-4.5z");
const PAUSE = icon("M4.5 3.5h2.5v9H4.5zM9 3.5h2.5v9H9z");
const PREV = icon("M4 3.5h1.5v9H4zM12.5 3.5v9L6.5 8z");
const NEXT = icon("M10.5 3.5H12v9h-1.5zM3.5 3.5v9l6-4.5z");

let current: NowPlaying | null = null;
let timer = 0;

/** Builds the widget once; `renderMedia` only fills it in. */
export function mountMedia(box: HTMLElement) {
  box.innerHTML = `
    <img class="cover" alt="" />
    <div class="track"><span class="title"></span><span class="artist"></span></div>
    <div class="controls">
      <button data-action="previous" aria-label="Previous">${PREV}</button>
      <button data-action="toggle" class="toggle" aria-label="Play or pause">${PLAY}</button>
      <button data-action="next" aria-label="Next">${NEXT}</button>
    </div>
    <div class="progress"><div class="done"></div></div>`;
  box.querySelectorAll<HTMLButtonElement>("button").forEach((b) =>
    b.addEventListener("click", () => invoke("media_control", { action: b.dataset.action })),
  );
}

export function renderMedia(box: HTMLElement, np: NowPlaying | null) {
  // A new song slides in (not on pause/play or progress updates).
  if (np && current && (np.title !== current.title || np.artist !== current.artist)) replay(box, "change");
  current = np;
  box.hidden = !np;
  clearInterval(timer);
  if (!np) return;
  const cover = box.querySelector<HTMLImageElement>(".cover")!;
  cover.hidden = !np.cover;
  if (np.cover && cover.src !== np.cover) cover.src = np.cover;
  box.querySelector(".title")!.textContent = np.title;
  box.querySelector(".artist")!.textContent = np.artist;
  box.title = np.artist ? `${np.title} – ${np.artist}` : np.title;
  const toggle = box.querySelector(".toggle")!;
  toggle.innerHTML = np.playing ? PAUSE : PLAY;
  toggle.setAttribute("aria-label", np.playing ? "Pause" : "Play");
  drawProgress(box);
  // The position is a sample; move the bar along while playing (once a second is plenty for 1 px steps).
  if (np.playing && np.duration_ms) timer = window.setInterval(() => drawProgress(box), 1000);
}

function drawProgress(box: HTMLElement) {
  if (!current?.duration_ms) {
    box.querySelector<HTMLElement>(".progress")!.hidden = true;
    return;
  }
  const elapsed = current.playing ? Date.now() - current.updated_at_ms : 0;
  const share = Math.min(1, (current.position_ms + elapsed) / current.duration_ms);
  box.querySelector<HTMLElement>(".progress")!.hidden = false;
  box.querySelector<HTMLElement>(".done")!.style.width = `${(share * 100).toFixed(2)}%`;
}
