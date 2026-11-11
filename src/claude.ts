// Claude plan usage: 5-hour and weekly limit as two small bars; local token count when limits aren't available.

type Limit = { used_pct: number; resets_at: string | null };
export type ClaudeUsage =
  | { source: "limits"; five_hour: Limit; seven_day: Limit; plan: string }
  | { source: "local"; tokens_5h: number; note: string };

const GLYPH = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.5l1.9 6.2 6.1 1.8-6.1 1.9L12 18.6l-1.9-6.2L4 10.5l6.1-1.8z"/><path d="M18.5 15.5l.8 2.3 2.2.7-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.7z"/></svg>`;

const resetText = (iso: string | null) => {
  if (!iso) return "";
  const d = new Date(iso);
  const sameDay = d.toDateString() === new Date().toDateString();
  const fmt: Intl.DateTimeFormatOptions = sameDay ? { hour: "2-digit", minute: "2-digit" } : { weekday: "short", hour: "2-digit", minute: "2-digit" };
  return `, resets ${new Intl.DateTimeFormat(undefined, fmt).format(d)}`;
};

// --v goes on the fill itself: it is a registered property that doesn't inherit (so it can animate).
const bar = (label: string, l: Limit) => {
  const pct = Math.round(l.used_pct);
  return `<span class="limit ${pct >= 90 ? "high" : ""}"><span class="label">${label}</span><span class="fill" style="--v:${pct}%"></span><span class="pct">${pct}%</span></span>`;
};

/** The Claude logo from the installed Claude app; the drawn glyph until (or unless) it is found. */
let logo = "";
let last: { el: HTMLElement; u: ClaudeUsage | null } | null = null;

export function setClaudeLogo(url: string) {
  logo = url;
  if (last) renderClaude(last.el, last.u);
}

const mark = () => (logo ? `<img class="logo" src="${logo}" alt="" />` : GLYPH);

/** Compact token count: 1234567 → "1.2M". */
const tokens = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}K` : `${n}`);

export function renderClaude(el: HTMLElement, u: ClaudeUsage | null) {
  last = { el, u };
  el.hidden = !u;
  if (!u) return;
  if (u.source === "limits") {
    el.innerHTML = `${mark()}<span class="limits">${bar("5h", u.five_hour)}${bar("7d", u.seven_day)}</span>`;
    el.title = [
      `Claude ${u.plan.charAt(0).toUpperCase()}${u.plan.slice(1)}`,
      `5-hour limit: ${Math.round(u.five_hour.used_pct)} %${resetText(u.five_hour.resets_at)}`,
      `Weekly limit: ${Math.round(u.seven_day.used_pct)} %${resetText(u.seven_day.resets_at)}`,
    ].join("\n");
  } else {
    el.innerHTML = `${mark()}<span class="local">${tokens(u.tokens_5h)}<small>tokens · 5h</small></span>`;
    el.title = `Claude Code tokens in the last 5 hours: ${u.tokens_5h.toLocaleString()}\n${u.note}`;
  }
  el.setAttribute("aria-label", el.title.replace(/\n/g, ". "));
}
