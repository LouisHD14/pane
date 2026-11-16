// Small drawings for the settings window: what a switch looks like off and on, side by side, with the current
// state outlined (asked for in ideas.md, 2026-11-16: "use pictures to tell better what the setting does").
// Own drawings of a screen, an app and the bar; decoration only (aria-hidden), the row's text says it all.

import type { Settings } from "./menu";

export type PictureKey = "fill" | "adaptive_color" | "blend" | "autohide" | "all_monitors" | "done_glow";

// Colours of the drawings: wallpaper, the bar's dark glass, an app with a green sidebar on a cream page.
const WALL = `<rect width="112" height="64" rx="4" fill="#23406f"/><circle cx="84" cy="18" r="22" fill="#2f5aa0" opacity=".55"/>`;
const GLASS = "rgba(18,22,32,.82)";
const SIDE = "#2e7d5b";
const PAGE = "#efe7d4";

/** The bar with three icons: floating (margins, round) or filling the bottom edge. */
const bar = (fill: string, edge = false, y = 52) =>
  edge
    ? `<rect x="0" y="${y + 2}" width="112" height="10" fill="${fill}"/>${icons(y + 5)}`
    : `<rect x="14" y="${y}" width="84" height="9" rx="3" fill="${fill}"/>${icons(y + 2.5)}`;

const icons = (y: number) =>
  [46, 53, 60].map((x, i) => `<rect x="${x}" y="${y}" width="5" height="4" rx="1" fill="${["#5aa9ff", "#ffb347", "#e5e5e5"][i]}"/>`).join("");

/** An app window from the top down to `bottom`, with a green sidebar. */
const app = (bottom: number) =>
  `<rect x="0" y="0" width="112" height="${bottom}" rx="4" fill="${PAGE}"/><rect x="0" y="0" width="30" height="${bottom}" rx="4" fill="${SIDE}"/>` +
  `<rect x="38" y="10" width="50" height="4" rx="2" fill="#cfc4ab"/><rect x="38" y="19" width="62" height="3" rx="1.5" fill="#ddd3bd"/><rect x="38" y="26" width="40" height="3" rx="1.5" fill="#ddd3bd"/>`;

/** The bar tinted in the colours of the app above it: green under the sidebar, cream under the page. */
const TINT = `<defs><linearGradient id="tint" x1="0" x2="1"><stop offset=".22" stop-color="${SIDE}"/><stop offset=".3" stop-color="#b9ae94"/></linearGradient></defs>`;

/** Two small monitors with a bar each, showing `left` and `right` app dots. */
const monitors = (left: string[], right: string[]) => {
  const one = (x: number, apps: string[]) =>
    `<rect x="${x}" y="10" width="50" height="34" rx="3" fill="#23406f"/><rect x="${x + 6}" y="36" width="38" height="6" rx="2" fill="${GLASS}"/>` +
    apps.map((c, i) => `<rect x="${x + 25 - apps.length * 3 + i * 6}" y="37.5" width="4" height="3" rx="1" fill="${c}"/>`).join("") +
    `<rect x="${x + 21}" y="44" width="8" height="6" fill="#3a3f4a"/><rect x="${x + 15}" y="50" width="20" height="2" rx="1" fill="#3a3f4a"/>`;
  return one(3, left) + one(59, right);
};

/** The Claude usage widget, with the ✓ badge when `done`. */
const widget = (done: boolean) =>
  `<rect width="112" height="64" rx="4" fill="#1d2433"/><rect x="18" y="20" width="76" height="24" rx="6" fill="rgba(255,255,255,.08)"/>` +
  `<rect x="24" y="25" width="14" height="14" rx="3" fill="#d97757"/>` +
  `<rect x="46" y="27" width="30" height="3" rx="1.5" fill="rgba(255,255,255,.25)"/><rect x="46" y="27" width="6" height="3" rx="1.5" fill="#d97757"/>` +
  `<rect x="46" y="34" width="30" height="3" rx="1.5" fill="rgba(255,255,255,.25)"/><rect x="46" y="34" width="12" height="3" rx="1.5" fill="#d97757"/>` +
  (done
    ? `<circle cx="38" cy="24" r="5.5" fill="#ff8a4c" stroke="#1d2433" stroke-width="1.5"/><path d="M35.6 24.1l1.6 1.6 3-3.2" fill="none" stroke="#1a1206" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>`
    : "");

const DRAW: Record<PictureKey, [off: string, on: string]> = {
  fill: [WALL + bar(GLASS), WALL + bar(GLASS, true)],
  adaptive_color: [app(50) + bar(GLASS), TINT + app(50) + bar("url(#tint)")],
  blend: [
    app(50) + bar(GLASS),
    app(50) + `<rect x="0" y="50" width="34" height="14" fill="${SIDE}"/><rect x="34" y="50" width="78" height="14" fill="#cdbf9f"/>` + bar(GLASS),
  ],
  autohide: [
    WALL + app(50) + bar(GLASS),
    WALL + app(64) + `<rect x="40" y="62" width="32" height="2" rx="1" fill="rgba(255,255,255,.6)"/>` +
      `<path d="M56 50l-3.5 4h2.3v4h2.4v-4h2.3z" fill="#fff" stroke="#000" stroke-width=".6"/>`,
  ],
  all_monitors: [monitors(["#5aa9ff"], ["#ffb347"]), monitors(["#5aa9ff", "#ffb347"], ["#5aa9ff", "#ffb347"])],
  done_glow: [widget(false), widget(true)],
};

/** "Off | On" drawings for a switch; `track` registers the outline update for when the setting changes. */
export function picture(key: PictureKey, track: (update: (s: Settings) => void) => void): HTMLElement {
  const ns = "http://www.w3.org/2000/svg";
  const fig = document.createElement("div");
  fig.className = "pics";
  fig.setAttribute("aria-hidden", "true");
  const cards = DRAW[key].map((markup, i) => {
    const card = document.createElement("div");
    card.className = "pic";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 112 64");
    svg.innerHTML = markup;
    const caption = document.createElement("span");
    caption.textContent = i ? "On" : "Off";
    card.append(svg, caption);
    return card;
  });
  fig.append(...cards);
  track((s) => cards.forEach((c, i) => c.classList.toggle("now", s[key] === (i === 1))));
  return fig;
}
