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

/** The small play/pause/next pill that slides out of the volume icon while music plays (setting
 * "volume_media"). It opens on hover over the icon and stays while the mouse is on it. */
export function mountMiniMedia(wrap: HTMLElement) {
  const pill = document.createElement("div");
  pill.className = "mini-media";
  pill.setAttribute("role", "toolbar");
  pill.setAttribute("aria-label", "Music");
  pill.innerHTML = `
    <button data-action="previous" aria-label="Previous" tabindex="-1">${PREV}</button>
    <button data-action="toggle" class="toggle" aria-label="Play or pause" tabindex="-1">${PLAY}</button>
    <button data-action="next" aria-label="Next" tabindex="-1">${NEXT}</button>`;
  pill.querySelectorAll<HTMLButtonElement>("button").forEach((b) =>
    b.addEventListener("click", (e) => {
      e.stopPropagation(); // not the volume mixer behind it
      invoke("media_control", { action: b.dataset.action });
    }),
  );
  wrap.append(pill);
  let timer = 0;
  const open = (on: boolean) => {
    clearTimeout(timer);
    if (on && !wrap.classList.contains("music")) return;
    wrap.classList.toggle("open", on);
    pill.querySelectorAll("button").forEach((b) => (b.tabIndex = on ? 0 : -1));
  };
  wrap.addEventListener("mouseenter", () => {
    clearTimeout(timer);
    timer = window.setTimeout(() => open(true), 120); // not on a quick pass over the icon
  });
  wrap.addEventListener("mouseleave", () => {
    clearTimeout(timer);
    timer = window.setTimeout(() => open(false), 350);
  });
  wrap.addEventListener("focusin", () => open(true));
  wrap.addEventListener("focusout", (e) => !wrap.contains(e.relatedTarget as Node) && open(false));
}

let miniTrack = "";

export function renderMiniMedia(wrap: HTMLElement, np: NowPlaying | null, on: boolean) {
  const music = !!np && on;
  wrap.classList.toggle("music", music);
  wrap.classList.toggle("playing", music && np!.playing);
  if (!music) {
    wrap.classList.remove("open");
    miniTrack = "";
    return;
  }
  const toggle = wrap.querySelector(".mini-media .toggle")!;
  toggle.innerHTML = np!.playing ? PAUSE : PLAY;
  toggle.setAttribute("aria-label", np!.playing ? "Pause" : "Play");
  const track = `${np!.title}\u0000${np!.artist}`;
  // A new song: peek out for a moment, so the controls are found (not when the mouse is on them).
  if (miniTrack && track !== miniTrack && np!.playing && !wrap.matches(":hover")) {
    wrap.classList.add("open");
    window.setTimeout(() => !wrap.matches(":hover") && wrap.classList.remove("open"), 2200);
  }
  miniTrack = track;
}

function drawProgress(box: HTMLElement) {
  if (!current?.duration_ms) {
    box.querySelector<HTMLElement>(".progress")!.hidden = true;
    return;
  }
  const elapsed = current.playing ? Date.now() - current.updated_at_ms : 0;
  const share = Math.min(1, (current.position_ms + elapsed) / current.duration_ms);
  box.querySelector<HTMLElement>(".progress")!.hidden = false;
  box.querySelector<HTMLElement>(".done")!.style.transform = `scaleX(${share.toFixed(4)})`;
}
