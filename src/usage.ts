// CPU / GPU / RAM meters and network speed, refreshed once a second.

export type Usage = { cpu: number; ram: number; gpu: number | null; net_down: number; net_up: number };

/** 1234567 B/s → "1.2 MB/s". */
export function rate(bytes: number): string {
  if (bytes < 1000) return `${bytes} B/s`;
  if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} KB/s`;
  return `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB/s`;
}

const METERS = ["cpu", "gpu", "ram"] as const;

/** Builds the meters once and then only moves them, so the fills glide between values. */
export function renderUsage(el: HTMLElement, u: Usage) {
  if (!el.dataset.ready) {
    el.dataset.ready = "1";
    el.innerHTML = `<span class="meters">${METERS.map(
      (m) => `<span class="meter" data-m="${m}"><span class="label">${m.toUpperCase()}</span><span class="fill"></span></span>`,
    ).join("")}</span><span class="net"><span class="down"></span><span class="up"></span></span>`;
  }
  for (const m of METERS) {
    const value = u[m];
    const meter = el.querySelector<HTMLElement>(`[data-m="${m}"]`)!;
    meter.hidden = value === null;
    meter.querySelector<HTMLElement>(".fill")!.style.setProperty("--v", `${value ?? 0}%`);
  }
  el.querySelector(".down")!.textContent = `↓ ${rate(u.net_down)}`;
  el.querySelector(".up")!.textContent = `↑ ${rate(u.net_up)}`;
  el.title = [`CPU ${u.cpu} %`, u.gpu === null ? "" : `GPU ${u.gpu} %`, `RAM ${u.ram} %`, `Download ${rate(u.net_down)}`, `Upload ${rate(u.net_up)}`]
    .filter(Boolean)
    .join("\n");
}
