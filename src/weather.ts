// Weather row under the time: icon + temperature. Icons are small inline SVGs (24×24 viewBox).

export type Weather = {
  temp_c: number;
  code: number;
  is_day: boolean;
  kind: "clear" | "partlycloudy" | "cloudy" | "fog" | "drizzle" | "rain" | "snow" | "thunder";
};

const svg = (body: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`;
const SUN = `<circle cx="12" cy="12" r="4" class="sun"/><path class="sun" d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>`;
const MOON = `<path class="moon" d="M19 14.5A7.5 7.5 0 0 1 9.5 5a7.5 7.5 0 1 0 9.5 9.5z"/>`;
const CLOUD = `<path d="M7 18h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 18z"/>`;
const SMALL_SUN = `<circle cx="8" cy="8" r="3" class="sun"/><path class="sun" d="M8 2v1.3M2 8h1.3M3.8 3.8l.9.9M12.2 3.8l-.9.9"/>`;
const SMALL_MOON = `<path class="moon" d="M11 9.5A4.5 4.5 0 0 1 5.5 4a4.5 4.5 0 1 0 5.5 5.5z"/>`;
const drops = (d: string) => `<path class="rain" d="${d}"/>`;

const ICONS: Record<Weather["kind"], (day: boolean) => string> = {
  clear: (day) => (day ? SUN : MOON),
  partlycloudy: (day) => (day ? SMALL_SUN : SMALL_MOON) + `<path d="M9 19h8a3.5 3.5 0 0 0 .4-7 4.8 4.8 0 0 0-9.2 1.3A2.9 2.9 0 0 0 9 19z"/>`,
  cloudy: () => CLOUD,
  fog: () => `<path d="M4 9h16M6 13h12M4 17h16"/>`,
  drizzle: () => `<path d="M7 14h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 14z"/>` + drops("M9 17v1.5M13 17v1.5M11 20v1"),
  rain: () => `<path d="M7 14h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 14z"/>` + drops("M8.5 16.5l-1 3M12.5 16.5l-1 3M16.5 16.5l-1 3"),
  snow: () => `<path d="M7 14h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 14z"/><path class="snow" d="M9 18h.01M12 20h.01M15 18h.01"/>`,
  thunder: () => `<path d="M7 14h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 14z"/><path class="bolt" d="M12.5 14l-2 4h3l-2 4"/>`,
};

const LABEL: Record<Weather["kind"], string> = {
  clear: "Clear",
  partlycloudy: "Partly cloudy",
  cloudy: "Cloudy",
  fog: "Fog",
  drizzle: "Drizzle",
  rain: "Rain",
  snow: "Snow",
  thunder: "Thunderstorm",
};

/** The weather, or why there is none. */
export type Report = { now: Weather | null; note: string };

/** Why a reason is readable: "offline (timeout)" → "Offline". */
const reason = (note: string) => {
  const short = note.replace(/\s*\(.*\)\s*$/, "");
  return short.charAt(0).toUpperCase() + short.slice(1);
};

/** The weather row, or a dim placeholder saying why there is none (so a widget that is switched on is never
 * silently empty, which looked like a missing setting). Nothing read yet hides the row; the clock then sits
 * alone in the block. */
export function renderWeather(el: HTMLElement, r: Report | null) {
  const w = r?.now ?? null;
  el.hidden = !r || (!w && !r.note);
  el.classList.toggle("no-data", !w);
  if (el.hidden) return;
  if (!w) {
    el.innerHTML = `${svg(`<path d="M7 16h10a4 4 0 0 0 .5-8 5.5 5.5 0 0 0-10.6 1.5A3.3 3.3 0 0 0 7 16z"/><path d="M4 20 20 4"/>`)}<span>--</span>`;
    el.title = `No weather: ${reason(r!.note).toLowerCase()}\nPane keeps trying. Switch the weather off in Settings > Widgets.`;
    el.setAttribute("aria-label", `No weather, ${reason(r!.note).toLowerCase()}`);
    return;
  }
  const temp = `${Math.round(w.temp_c)}°`;
  el.innerHTML = `${svg(ICONS[w.kind](w.is_day))}<span>${temp}</span>`;
  el.title = `${LABEL[w.kind]}, ${temp}C`;
  el.setAttribute("aria-label", el.title);
}
