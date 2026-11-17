import { errorText } from "./api";
import { el, setChildren } from "./dom";

let node: HTMLElement | null = null;
let timer: number | undefined;
/** Bumped by every toast, so a closer only hides the toast it showed. */
let shownSeq = 0;

/** Small bottom-center pill that disappears after 2.5 s. With `action` it carries a button
 *  (Undo after a save) and stays 8 s; `sticky` keeps it until the button is pressed (news
 *  for someone who was away). Returns a function that hides this toast if it is still shown. */
export function toast(message: string, action?: { label: string; run: () => void }, sticky = false): () => void {
  if (!node) {
    // A manual popover lives in the top layer, so it also shows above open modal dialogs.
    node = el("div", { class: "toast", attrs: { role: "status", "aria-live": "polite", popover: "manual" } });
    document.body.appendChild(node);
  }
  const shown = node;
  setChildren(
    shown,
    message,
    action
      ? el(
          "button",
          {
            class: "toast-action",
            attrs: { type: "button" },
            on: {
              click: () => {
                window.clearTimeout(timer);
                shown.hidePopover();
                action.run();
              },
            },
          },
          action.label,
        )
      : null,
  );
  shown.classList.toggle("has-action", action !== undefined);
  // Re-showing moves the toast to the top of the top layer, above dialogs opened since.
  if (shown.matches(":popover-open")) shown.hidePopover();
  shown.showPopover();
  window.clearTimeout(timer);
  if (!sticky) timer = window.setTimeout(() => shown.hidePopover(), action ? 8000 : 2500);
  const seq = ++shownSeq;
  return () => {
    if (seq === shownSeq && shown.matches(":popover-open")) shown.hidePopover();
  };
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
