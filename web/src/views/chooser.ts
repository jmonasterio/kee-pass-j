import { api, errorText, type ChooserItem } from "../api";
import { openDialog, type DialogHandle } from "../dialogs";
import { button, el } from "../dom";
import { toast } from "../toast";

let open: { handle: DialogHandle; settle: () => void } | null = null;

/** Shows the "which account?" list after the global hotkey matched several entries. */
export function openChooser(items: ChooserItem[]): void {
  // A newer request supersedes one still on screen; the backend only waits for the latest.
  if (open) {
    open.settle();
    open.handle.close();
  }

  let settled = false;
  let active = 0;
  const settle = () => {
    settled = true;
  };
  const choose = (index: number) => {
    if (settled) return;
    settled = true;
    handle.close();
    api.autotypeChoose(index).catch((err) => toast(errorText(err)));
  };

  const rows = items.map((item, index) =>
    el(
      "div",
      {
        class: "chooser-row",
        attrs: { role: "option", "aria-selected": String(index === 0) },
        on: {
          click: () => choose(index),
          mouseenter: () => highlight(index),
        },
      },
      el("div", { class: "chooser-text" }, el("div", { class: "entry-title" }, item.title || "(untitled)"), el("div", { class: "entry-sub" }, item.username)),
      el("div", { class: "chooser-db muted" }, item.db),
    ),
  );
  const highlight = (index: number) => {
    active = index;
    rows.forEach((row, i) => {
      row.classList.toggle("selected", i === index);
      row.setAttribute("aria-selected", String(i === index));
    });
    rows[index]?.scrollIntoView({ block: "nearest" });
  };

  const list = el("div", { class: "chooser-list", attrs: { role: "listbox", tabindex: "0", autofocus: true, "aria-label": "Matching accounts" } }, rows);
  list.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      highlight(Math.min(items.length - 1, Math.max(0, active + (event.key === "ArrowDown" ? 1 : -1))));
    } else if (event.key === "Enter" && items.length > 0) {
      event.preventDefault();
      choose(active);
    }
  });

  const handle = openDialog({
    title: "Which account?",
    class: "chooser-dialog",
    body: el("div", { class: "stack" }, el("p", { class: "dialog-text muted" }, "Several accounts match the active window. Pick one to type."), list),
    actions: [button("Cancel", () => handle.close())],
    onClose: () => {
      if (open?.handle === handle) open = null;
      if (settled) return;
      settled = true;
      api.autotypeCancel().catch((err) => toast(errorText(err)));
    },
  });
  open = { handle, settle };
  highlight(0);
}
