// Reordering the pinned app icons: drag them with the mouse, or Alt+Shift+Left/Right on a focused icon.
// The dragged icon follows the pointer, the others slide aside; Escape cancels. Only pinned apps move.

/** Pointer travel (px) before a press becomes a drag; less stays a click. */
const THRESHOLD = 5;

type Drag = { b: HTMLElement; pointer: number; x: number; on: boolean; items: HTMLElement[]; from: number; to: number; step: number; dx: number };

export type Hooks = {
  /** A drag began (close the hover preview). */
  start: () => void;
  /** It ended, dropped or cancelled (render what came in meanwhile). */
  end: () => void;
  /** New order of all pinned app ids, left to right. */
  save: (ids: string[]) => void;
};

/** Makes the `.app.pinned` buttons in `box` reorderable; `dragging()` tells whether a drag is under way. */
export function reorderPins(box: HTMLElement, hooks: Hooks) {
  let drag: Drag | null = null;
  // The click that ends a drag (or follows an Escape) must not launch / activate the app.
  let eatClick = false;
  const pins = () => [...box.querySelectorAll<HTMLElement>(".app.pinned:not(.leave)")];
  const save = () => hooks.save(pins().map((b) => b.dataset.id!));
  const move = (b: HTMLElement, to: HTMLElement, after: boolean) => (after ? to.after(b) : to.before(b));

  const finish = (drop: boolean) => {
    const d = drag;
    drag = null;
    if (!d?.on) return;
    eatClick = true;
    if (d.b.hasPointerCapture(d.pointer)) d.b.releasePointerCapture(d.pointer);
    d.b.classList.remove("dragging");
    for (const i of d.items) i.style.transform = ""; // the others (and, if nothing moved, the icon) slide back
    if (drop && d.to !== d.from) {
      // Move the icon in the DOM, keep it where it was dropped for one frame, then let it glide into its slot.
      box.classList.add("settle");
      d.b.style.transform = `translateX(${d.dx - (d.to - d.from) * d.step}px)`;
      move(d.b, d.items[d.to], d.to > d.from);
      void d.b.offsetWidth;
      box.classList.remove("settle");
      d.b.style.transform = "";
      save();
    }
    hooks.end();
  };

  box.addEventListener("pointerdown", (e) => {
    eatClick = false;
    const b = (e.target as HTMLElement).closest<HTMLElement>(".app.pinned:not(.leave)");
    if (e.button !== 0 || !b || drag) return;
    drag = { b, pointer: e.pointerId, x: e.clientX, on: false, items: [], from: 0, to: 0, step: 0, dx: 0 };
  });

  box.addEventListener("pointermove", (e) => {
    const d = drag;
    if (!d || e.pointerId !== d.pointer) return;
    if (!d.on) {
      if (Math.abs(e.clientX - d.x) < THRESHOLD) return;
      d.on = true;
      d.items = pins();
      d.from = d.to = d.items.indexOf(d.b);
      // All icons are the same width: one slot = distance between two neighbours.
      const left = d.items.map((i) => i.getBoundingClientRect().left);
      d.step = left.length > 1 ? left[1] - left[0] : 0;
      d.b.setPointerCapture(d.pointer);
      d.b.classList.add("dragging");
      hooks.start();
    }
    // Stays within the pinned icons; running unpinned apps keep their place after them.
    d.dx = Math.max(-d.from * d.step, Math.min((d.items.length - 1 - d.from) * d.step, e.clientX - d.x));
    d.to = d.step ? Math.round(d.from + d.dx / d.step) : d.from;
    d.items.forEach((it, i) => {
      const shift = it === d.b ? d.dx : i > d.from && i <= d.to ? -d.step : i < d.from && i >= d.to ? d.step : 0;
      it.style.transform = shift ? `translateX(${shift}px)` : "";
    });
  });

  box.addEventListener("pointerup", (e) => e.pointerId === drag?.pointer && finish(true));
  box.addEventListener("pointercancel", (e) => e.pointerId === drag?.pointer && finish(false));
  // Images are natively draggable; that would cancel the pointer drag.
  box.addEventListener("dragstart", (e) => e.preventDefault());
  box.addEventListener(
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
    eatClick = false;
    const b = (e.target as HTMLElement).closest?.<HTMLElement>(".app.pinned:not(.leave)");
    if (!b || !box.contains(b) || drag || !e.altKey || !e.shiftKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const items = pins();
    const to = items.indexOf(b) + (e.key === "ArrowLeft" ? -1 : 1);
    if (to < 0 || to >= items.length) return;
    move(b, items[to], e.key === "ArrowRight");
    b.focus(); // moving a node in the DOM drops its focus
    save();
  });

  return { dragging: () => !!drag?.on };
}
