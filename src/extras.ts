// Opt-in extras on the bar (each off until switched on in the settings): keep awake, virtual desktop dots and
// the user's own command widget, plus scroll-wheel steps. Rust side: extras.rs and main.rs.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

type Desktops = { count: number; current: number };

const svg = (body: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`;
const CUP = svg(`<path d="M5 9h11v5a5 5 0 0 1-5 5h-1a5 5 0 0 1-5-5z"/><path d="M16 11h1.5a2.5 2.5 0 0 1 0 5H16"/><path d="M8 3.5c0 1 1 1.5 1 2.5M11.5 3.5c0 1 1 1.5 1 2.5" class="steam"/>`);

/** Calls `step(+1 | -1)` once per wheel notch over `el` (down = +1), however finely a touchpad scrolls, and at
 * most every 90 ms so one flick doesn't race through everything. Only while `on(e)` (the page scrolls otherwise). */
export function wheelSteps(el: HTMLElement, on: (e: WheelEvent) => boolean, step: (dir: 1 | -1, e: WheelEvent) => void) {
  let sum = 0;
  let last = 0;
  el.addEventListener(
    "wheel",
    (e) => {
      if (!on(e)) return;
      e.preventDefault();
      sum += e.deltaMode === 1 ? e.deltaY * 40 : e.deltaY; // lines → px
      if (Math.abs(sum) < 50) return;
      const now = performance.now();
      const dir = sum > 0 ? 1 : -1;
      sum = 0;
      if (now - last < 90) return;
      last = now;
      step(dir, e);
    },
    { passive: false },
  );
}

export function mountExtras(wheelOn: () => boolean) {
  const me = getCurrentWebviewWindow();
  const $ = (id: string) => document.getElementById(id)!;

  // --- Keep awake: a cup, lit while the PC isn't allowed to sleep. Not saved (Rust): each start is normal sleep.
  const awake = $("awake");
  awake.innerHTML = CUP;
  const showAwake = (on: boolean) => {
    awake.classList.toggle("on", on);
    awake.setAttribute("aria-pressed", String(on));
    awake.title = on ? "Keep awake is on: the PC and screen don't sleep. Click to allow sleep again." : "Keep awake: click so the PC and screen don't go to sleep";
    awake.setAttribute("aria-label", on ? "Keep awake, on" : "Keep awake, off");
  };
  invoke<boolean>("get_awake").then(showAwake);
  me.listen<boolean>("awake", (e) => showAwake(e.payload));
  awake.addEventListener("click", () => invoke("set_awake", { on: !awake.classList.contains("on") }));

  // --- Virtual desktop dots: the shown one is long; click one to go there, scroll to step.
  const box = $("desktops");
  let desktops: Desktops = { count: 1, current: 0 };
  const showDesktops = (d: Desktops) => {
    desktops = d;
    if (box.children.length !== d.count) {
      box.replaceChildren(
        ...Array.from({ length: d.count }, (_, i) => {
          const b = Object.assign(document.createElement("button"), { className: "desk" });
          b.dataset.index = String(i);
          return b;
        }),
      );
    }
    [...box.children].forEach((b, i) => {
      b.classList.toggle("current", i === d.current);
      b.setAttribute("aria-label", `Desktop ${i + 1} of ${d.count}${i === d.current ? ", shown" : ""}`);
      (b as HTMLElement).title = `Desktop ${i + 1}`;
    });
  };
  invoke<Desktops>("get_desktops").then(showDesktops);
  me.listen<Desktops>("desktops", (e) => showDesktops(e.payload));
  box.addEventListener("click", (e) => {
    const i = (e.target as HTMLElement).closest<HTMLElement>(".desk")?.dataset.index;
    if (i !== undefined) invoke("go_to_desktop", { index: Number(i) });
  });
  wheelSteps(box, () => wheelOn(), (dir) => {
    const to = Math.min(desktops.count - 1, Math.max(0, desktops.current + dir));
    if (to !== desktops.current) invoke("go_to_desktop", { index: to });
  });

  // --- The user's own command: its first line as a pill; a click runs it again now.
  const custom = $("custom");
  const showCustom = (text: string | null) => {
    custom.hidden = !text;
    custom.textContent = text ?? "";
  };
  invoke<string | null>("get_custom").then(showCustom);
  me.listen<string | null>("custom", (e) => showCustom(e.payload));
  custom.addEventListener("click", () => {
    custom.classList.add("running");
    invoke("refresh_custom");
  });
  me.listen("custom", () => custom.classList.remove("running"));
  custom.title = "Your command (Settings › Your own widget). Click to run it now.";

  // --- Scroll on the volume icon: Windows' own volume keys (its flyout shows the level).
  wheelSteps(
    $("status"),
    (e) => wheelOn() && !!(e.target as HTMLElement).closest(".vol-wrap"),
    (dir) => invoke("volume_step", { up: dir < 0 }),
  );
}
