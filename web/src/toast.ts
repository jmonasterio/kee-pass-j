import { errorText } from "./api";
import { el } from "./dom";

let node: HTMLElement | null = null;
let timer: number | undefined;

/** Small bottom-center pill that disappears after 2.5 s. */
export function toast(message: string): void {
  if (!node) {
    // A manual popover lives in the top layer, so it also shows above open modal dialogs.
    node = el("div", { class: "toast", attrs: { role: "status", "aria-live": "polite", popover: "manual" } });
    document.body.appendChild(node);
  }
  node.textContent = message;
  // Re-showing moves the toast to the top of the top layer, above dialogs opened since.
  if (node.matches(":popover-open")) node.hidePopover();
  node.showPopover();
  window.clearTimeout(timer);
  timer = window.setTimeout(() => node?.hidePopover(), 2500);
}

/** Runs a copy command (which answers the seconds until the clipboard is cleared, 0 = never)
 *  and says what happened. */
export async function copyAndTell(call: Promise<number>): Promise<void> {
  try {
    const seconds = await call;
    toast(seconds > 0 ? `Copied. Cleared from the clipboard in ${seconds} s.` : "Copied");
  } catch (err) {
    toast(errorText(err));
  }
}
