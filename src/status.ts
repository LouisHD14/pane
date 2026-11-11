// Status icons: network, volume, battery. Shapes drawn as small inline SVGs (24×24 viewBox).

export type Status = {
  network: "none" | "ethernet" | "wifi";
  wifi_signal: number | null;
  volume: number;
  muted: boolean;
  battery: { percent: number; charging: boolean } | null;
};

const svg = (body: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`;

function networkIcon(s: Status): [string, string] {
  if (s.network === "ethernet") {
    return [svg(`<rect x="4" y="4" width="16" height="11" rx="2"/><path d="M12 15v5M8 20h8"/>`), "Ethernet connected"];
  }
  if (s.network === "wifi") {
    const bars = s.wifi_signal ?? 4;
    // Arcs light up from the inside out with the signal strength (0-4).
    const arcs = ["M9.5 16.5a3.5 3.5 0 0 1 5 0", "M7 14a7 7 0 0 1 10 0", "M4.5 11.5a10.5 10.5 0 0 1 15 0", "M2 9a14 14 0 0 1 20 0"];
    const paths = arcs.map((d, i) => `<path d="${d}" class="${i < Math.max(bars, 1) ? "" : "dim"}"/>`).join("");
    return [svg(`${paths}<circle cx="12" cy="19.5" r="1.2" class="fill"/>`), `Wi-Fi, signal ${bars} of 4`];
  }
  return [svg(`<circle cx="12" cy="12" r="8"/><path d="M5 19 19 5"/>`), "No internet"];
}

function volumeIcon(s: Status): [string, string] {
  const speaker = `<path d="M4 9h3l5-4v14l-5-4H4z" class="fill"/>`;
  if (s.muted || s.volume === 0) return [svg(`${speaker}<path d="M16 9l5 6M21 9l-5 6"/>`), "Muted"];
  const waves = [
    s.volume > 0 ? `<path d="M15 9.5a3.5 3.5 0 0 1 0 5"/>` : "",
    s.volume > 33 ? `<path d="M17.5 7a7 7 0 0 1 0 10"/>` : "",
    s.volume > 66 ? `<path d="M20 4.5a10.5 10.5 0 0 1 0 15"/>` : "",
  ].join("");
  return [svg(speaker + waves), `Volume ${s.volume} %`];
}

function batteryIcon(b: NonNullable<Status["battery"]>): [string, string] {
  const width = Math.round((14 * b.percent) / 100);
  const bolt = b.charging ? `<path d="M11 8l-2 4h3l-1 4" class="bolt"/>` : "";
  const body = `<rect x="2.5" y="7" width="17" height="10" rx="2"/><path d="M21.5 10.5v3"/><rect x="4" y="8.5" width="${width}" height="7" rx="1" class="fill"/>`;
  return [svg(body + bolt), `Battery ${b.percent} %${b.charging ? ", charging" : ""}`];
}

export function renderStatus(el: HTMLElement, s: Status) {
  const parts = [networkIcon(s), volumeIcon(s), ...(s.battery ? [batteryIcon(s.battery)] : [])];
  el.innerHTML = parts.map(([icon]) => icon).join("");
  el.title = parts.map(([, label]) => label).join("\n");
}
