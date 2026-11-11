// CPU / GPU / RAM meters and network speed, refreshed once a second.

export type Usage = { cpu: number; ram: number; gpu: number | null; net_down: number; net_up: number };

/** 1234567 B/s → "1.2 MB/s". */
export function rate(bytes: number): string {
  if (bytes < 1000) return `${bytes} B/s`;
  if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} KB/s`;
  return `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB/s`;
}

const meter = (label: string, value: number) =>
  `<span class="meter" style="--v:${value}%"><span class="label">${label}</span><span class="fill"></span></span>`;

export function renderUsage(el: HTMLElement, u: Usage) {
  const meters = [meter("CPU", u.cpu), ...(u.gpu === null ? [] : [meter("GPU", u.gpu)]), meter("RAM", u.ram)];
  el.innerHTML = `<span class="meters">${meters.join("")}</span>
    <span class="net"><span>↓ ${rate(u.net_down)}</span><span>↑ ${rate(u.net_up)}</span></span>`;
  el.title = [`CPU ${u.cpu} %`, u.gpu === null ? "" : `GPU ${u.gpu} %`, `RAM ${u.ram} %`, `Download ${rate(u.net_down)}`, `Upload ${rate(u.net_up)}`]
    .filter(Boolean)
    .join("\n");
}
