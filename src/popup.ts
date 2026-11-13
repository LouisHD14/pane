// Shared behaviour of Pane's own popups (hidden icons, Pane menu, Docker).

import { invoke } from "@tauri-apps/api/core";

/** Closes the popup once the focus has really left it.
 *
 * A plain `blur` handler closed popups that were being opened at that very moment: clicking the bar button of
 * an open popup first blurs the popup, and the click then reopens it, so the late blur shut the fresh one again
 * and the popup "just didn't show" (reported 2026-11-13). Waiting a moment and asking whether the window is
 * focused again settles that race. */
export function closeOnBlur(): () => void {
  let timer = 0;
  const close = () => invoke("close_popup");
  window.addEventListener("blur", () => {
    clearTimeout(timer);
    timer = window.setTimeout(() => {
      if (!document.hasFocus()) close();
    }, 150);
  });
  window.addEventListener("focus", () => clearTimeout(timer));
  return close;
}
