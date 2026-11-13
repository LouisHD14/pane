// Rearranging the bar's elements (widgets on the left; system load, tray, status icons and clock on the right):
// drag one along its side of the bar, as with the pinned app icons, or Alt+Shift+Left/Right while it has focus.
// The order is a CSS `order` on each `[data-move]` element, so the code that fills them never notices, and it is
// saved in the settings (element_order), which sends it to every bar.

/** Pointer travel (px) before a press becomes a drag; less stays a click (same as reorder.ts). */
const THRESHOLD = 5;

/** Every movable element, by side, in the built-in order. A key missing from a saved order goes back to its
 *  built-in neighbour, so elements added in later versions turn up where they belong. */
const SIDES = [
  ["media", "claude", "codex", "docker"],
  ["usage", "tray", "status", "clock"],
];

export type Hooks = {
  /** A drag began (close the hover preview and open popups). */
  start: () => void;
  /** New order of all keys, both sides, left to right. */
  save: (order: string[]) => void;
};

type Drag = {
  el: HTMLElement;
  pointer: number;
  x: number;
  on: boolean;
  /** The visible elements of that side, left to right, and their boxes when the drag began. */
  items: HTMLElement[];
  boxes: DOMRect[];
  from: number;
  to: number;
  /** How far the others step aside: the dragged element's width plus the gap to its neighbour. */
  step: number;
  dx: number;
};

/** Keys of one side in the saved order, built-in ones that are missing put back after their built-in predecessor. */
function sideOrder(defaults: string[], saved: string[]): string[] {
  const out = saved.filter((k, i) => defaults.includes(k) && saved.indexOf(k) === i);
  defaults.forEach((k, i) => {
    if (out.includes(k)) return;
    const before = defaults.slice(0, i).reverse().find((p) => out.includes(p));
    out.splice(before ? out.indexOf(before) + 1 : 0, 0, k);
  });
  return out;
}

/** How the others slide aside and everything glides into place (inline, so only while arranging). */
const GLIDE = "transform 0.2s var(--ease)";

const keyOf = (el: Element) => (el as HTMLElement).dataset.move!;
const visible = (el: HTMLElement) => el.getClientRects().length > 0;

export function arrangeElements(bar: HTMLElement, hooks: Hooks) {
  let order = SIDES.map((d) => [...d]);
  let drag: Drag | null = null;
  // The click that ends a drag must not open what was dragged (Claude popup, Task Manager, a tray app, …).
  let eatClick = false;

  const element = (key: string) => bar.querySelector<HTMLElement>(`[data-move="${key}"]`);
  const apply = () => order.forEach((side) => side.forEach((k, i) => element(k)?.style.setProperty("order", String(i + 1))));
  const sideOf = (key: string) => order.findIndex((s) => s.includes(key));
  /** Visible elements of a side, left to right as shown. */
  const shown = (side: number) => order[side].map(element).filter((e): e is HTMLElement => !!e && visible(e));

  /** Puts `key` just before (or after) `target` on its side, then saves. */
  const place = (key: string, target: string, after: boolean) => {
    const side = order[sideOf(key)];
    side.splice(side.indexOf(key), 1);
    side.splice(side.indexOf(target) + (after ? 1 : 0), 0, key);
    apply();
    hooks.save(order.flat());
  };

  const finish = (drop: boolean) => {
    const d = drag;
    drag = null;
    if (!d?.on) return;
    eatClick = true;
    if (d.el.hasPointerCapture(d.pointer)) d.el.releasePointerCapture(d.pointer);
    d.el.classList.remove("moving");
    // Where everything is drawn right now (with the shifts), then the new order, then glide from there.
    const was = d.items.map((i) => i.getBoundingClientRect().left);
    for (const i of d.items) {
      i.style.transition = "none";
      i.style.transform = "";
    }
    if (drop && d.to !== d.from) place(keyOf(d.el), keyOf(d.items[d.to]), d.to > d.from);
    d.items.forEach((i, n) => {
      const off = was[n] - i.getBoundingClientRect().left;
      if (off) i.style.transform = `translateX(${off}px)`;
    });
    void bar.offsetWidth;
    for (const i of d.items) {
      i.style.transition = GLIDE;
      i.style.transform = "";
    }
    bar.classList.remove("arranging");
    // Then back to the elements' own transitions (hover colours etc.).
    setTimeout(() => d.items.forEach((i) => drag?.items.includes(i) || (i.style.transition = "")), 250);
  };

  bar.addEventListener("pointerdown", (e) => {
    eatClick = false;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-move]");
    if (e.button !== 0 || !el || drag) return;
    drag = { el, pointer: e.pointerId, x: e.clientX, on: false, items: [], boxes: [], from: 0, to: 0, step: 0, dx: 0 };
  });

  bar.addEventListener("pointermove", (e) => {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    if (!d.on) {
      if (Math.abs(e.clientX - d.x) < THRESHOLD) return;
      d.items = shown(sideOf(keyOf(d.el)));
      d.from = d.to = d.items.indexOf(d.el);
      if (d.from < 0 || d.items.length < 2) return void (drag = null);
      d.on = true;
      d.boxes = d.items.map((i) => i.getBoundingClientRect());
      const me = d.boxes[d.from];
      const next = d.boxes[d.from + 1] ?? d.boxes[d.from - 1];
      const gap = d.boxes[d.from + 1] ? next.left - me.right : me.left - next.right;
      d.step = me.width + Math.max(0, gap);
      for (const i of d.items) i.style.transition = i === d.el ? "none" : GLIDE;
      d.el.setPointerCapture(d.pointer);
      d.el.classList.add("moving");
      bar.classList.add("arranging");
      hooks.start();
    }
    // Stays on its own side of the bar.
    const first = d.boxes[0].left - d.boxes[d.from].left;
    const last = d.boxes[d.items.length - 1].right - d.boxes[d.from].right;
    d.dx = Math.max(first, Math.min(last, e.clientX - d.x));
    // It takes a neighbour's slot once its middle has passed that neighbour's middle.
    const mid = (d.boxes[d.from].left + d.boxes[d.from].right) / 2 + d.dx;
    d.to = d.from;
    d.boxes.forEach((b, i) => {
      const centre = (b.left + b.right) / 2;
      if ((i > d.from && mid > centre) || (i < d.from && mid < centre && i < d.to)) d.to = i;
    });
    d.items.forEach((it, i) => {
      const shift = it === d.el ? d.dx : i > d.from && i <= d.to ? -d.step : i < d.from && i >= d.to ? d.step : 0;
      it.style.transform = shift ? `translateX(${shift}px)` : "";
    });
  });

  bar.addEventListener("pointerup", (e) => e.pointerId === drag?.pointer && finish(true));
  bar.addEventListener("pointercancel", (e) => e.pointerId === drag?.pointer && finish(false));
  bar.addEventListener("dragstart", (e) => (e.target as HTMLElement).closest?.("[data-move]") && e.preventDefault());
  bar.addEventListener(
    "click",
    (e) => {
      if (!eatClick) return;
      eatClick = false;
      e.stopPropagation();
      e.preventDefault();
    },
    true,
  );

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && drag?.on) return finish(false);
    const el = (e.target as HTMLElement).closest?.<HTMLElement>("[data-move]");
    if (!el || !bar.contains(el) || drag || !e.altKey || !e.shiftKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const items = shown(sideOf(keyOf(el)));
    const to = items.indexOf(el) + (e.key === "ArrowLeft" ? -1 : 1);
    if (to < 0 || to >= items.length) return;
    place(keyOf(el), keyOf(items[to]), e.key === "ArrowRight");
  });

  return {
    dragging: () => !!drag?.on,
    /** The saved order (from the settings); ignored mid-drag, the drop saves anyway. */
    set(saved: string[]) {
      if (drag?.on) return;
      order = SIDES.map((d) => sideOrder(d, saved ?? []));
      apply();
    },
  };
}
