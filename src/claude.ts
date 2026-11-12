// Claude and Codex plan usage: 5-hour and weekly limit as two small bars; for Claude, the local token count
// when the limits can't be read.

type Limit = { used_pct: number; resets_at: string | null };
type Limits = { source: "limits"; five_hour: Limit; seven_day: Limit; plan: string };
export type ClaudeUsage = Limits | { source: "local"; tokens_5h: number; note: string };
export type CodexUsage = Limits | { source: "error"; note: string };

const GLYPH = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.5l1.9 6.2 6.1 1.8-6.1 1.9L12 18.6l-1.9-6.2L4 10.5l6.1-1.8z"/><path d="M18.5 15.5l.8 2.3 2.2.7-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.7z"/></svg>`;
/** Codex: a terminal prompt (own drawing, no brand logo). */
const CODEX_GLYPH = `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.5" y="4" width="19" height="16" rx="4"/><path class="cut" d="M7 9.5l3 2.5-3 2.5M12.5 15h4.5"/></svg>`;

const resetText = (iso: string | null) => {
  if (!iso) return "";
  const d = new Date(iso);
  if (d.getTime() <= 0) return "";
  const sameDay = d.toDateString() === new Date().toDateString();
  const fmt: Intl.DateTimeFormatOptions = sameDay ? { hour: "2-digit", minute: "2-digit" } : { weekday: "short", hour: "2-digit", minute: "2-digit" };
  return `, resets ${new Intl.DateTimeFormat(undefined, fmt).format(d)}`;
};

// The fill is scaled, not resized: a transform is drawn by the compositor, no repaint of the bar.
const bar = (label: string, l: Limit) => {
  const pct = Math.round(l.used_pct);
  const share = Math.min(100, Math.max(0, pct)) / 100;
  return `<span class="limit ${pct >= 90 ? "high" : ""}"><span class="label">${label}</span><span class="fill"><i style="transform:scaleX(${share})"></i></span><span class="pct">${pct}%</span></span>`;
};

const title = (name: string, u: Limits) =>
  [
    `${name} ${u.plan.charAt(0).toUpperCase()}${u.plan.slice(1)}`.trim(),
    `5-hour limit: ${Math.round(u.five_hour.used_pct)} %${resetText(u.five_hour.resets_at)}`,
    `Weekly limit: ${Math.round(u.seven_day.used_pct)} %${resetText(u.seven_day.resets_at)}`,
  ].join("\n");

const limits = (u: Limits) => `<span class="limits">${bar("5h", u.five_hour)}${bar("7d", u.seven_day)}</span>`;

/** The Claude logo from the installed Claude app; the drawn glyph until (or unless) it is found. */
let logo = "";
let last: { el: HTMLElement; u: ClaudeUsage | null } | null = null;

export function setClaudeLogo(url: string) {
  logo = url;
  if (last) renderClaude(last.el, last.u);
}

/** The last Claude state, for the right-click menu ("log in again" when the login expired). */
export const claudeState = () => last?.u ?? null;

const mark = () => (logo ? `<img class="logo" src="${logo}" alt="" />` : GLYPH);

/** Compact token count: 1234567 → "1.2M". */
const tokens = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}K` : `${n}`);

/** Writes the markup only when it changed, so an unchanged refresh doesn't restart the fade. */
function put(el: HTMLElement, html: string, tip: string) {
  if (el.dataset.html !== html) {
    el.innerHTML = html;
    el.dataset.html = html;
  }
  el.title = tip;
  el.setAttribute("aria-label", tip.replace(/\n/g, ". "));
}

export function renderClaude(el: HTMLElement, u: ClaudeUsage | null) {
  last = { el, u };
  el.hidden = !u;
  if (!u) return;
  if (u.source === "limits") {
    put(el, `${mark()}${limits(u)}`, title("Claude", u));
  } else {
    // The login expired and couldn't be renewed: say so instead of a token count that looks like a limit.
    const relogin = u.note.includes("login");
    put(
      el,
      relogin
        ? `${mark()}<span class="local">Log in<small>Claude Code</small></span>`
        : `${mark()}<span class="local">${tokens(u.tokens_5h)}<small>tokens · 5h</small></span>`,
      relogin
        ? `Claude: ${u.note}\nClick to log in again`
        : `Claude Code tokens in the last 5 hours: ${u.tokens_5h.toLocaleString()}\n${u.note}`,
    );
  }
  el.classList.toggle("relogin", u.source === "local" && u.note.includes("login"));
}

let codexLast: CodexUsage | null = null;
export const codexState = () => codexLast;

export function renderCodex(el: HTMLElement, u: CodexUsage | null) {
  codexLast = u;
  el.hidden = !u;
  if (!u) return;
  if (u.source === "limits") put(el, `${CODEX_GLYPH}${limits(u)}`, title("Codex", u));
  else put(el, `${CODEX_GLYPH}<span class="local">Codex<small>${u.note.includes("login") ? "log in" : "offline"}</small></span>`, `Codex: ${u.note}`);
  el.classList.toggle("relogin", u.source === "error" && u.note.includes("login"));
}
