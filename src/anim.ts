/** Plays a CSS animation class again from the start (also when it is already set). */
export function replay(el: Element, cls: string) {
  el.classList.remove(cls);
  void (el as HTMLElement).offsetWidth; // force a style flush so the browser sees the class as new
  el.classList.add(cls);
}

/** Adds a one-shot animation class and drops it when the animation is done. */
export function once(el: Element, cls: string) {
  replay(el, cls);
  el.addEventListener("animationend", () => el.classList.remove(cls), { once: true });
}
