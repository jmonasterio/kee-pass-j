import type { Dialog } from "@fluentui/web-components";
import { button, el, field, type Child } from "./dom";

export interface DialogHandle {
  dialog: Dialog;
  close(): void;
}

interface ToggleDetail {
  newState?: string;
}

/** Calls `onClosed` once when a Fluent dialog/drawer reports it closed (Esc, light dismiss, hide()). */
export function onClosed(node: HTMLElement, onClosed: () => void): void {
  const listener = (event: Event) => {
    if (event.target !== node) return;
    const detail = (event as CustomEvent<ToggleDetail>).detail;
    if (detail?.newState !== "closed") return;
    node.removeEventListener("toggle", listener);
    node.remove();
    onClosed();
  };
  node.addEventListener("toggle", listener);
}

/** True while any modal dialog or drawer is on screen; page shortcuts stand down then. */
export function isOverlayOpen(): boolean {
  return document.querySelector("fluent-dialog, fluent-drawer") !== null;
}

export function openDialog(opts: {
  title: string;
  body: Child;
  actions: HTMLElement[];
  type?: "modal" | "alert";
  class?: string;
  onClose?: () => void;
}): DialogHandle {
  for (const action of opts.actions) action.setAttribute("slot", "action");
  const dialog = el(
    "fluent-dialog",
    { class: opts.class, attrs: { "aria-label": opts.title } },
    el("fluent-dialog-body", null, el("h2", { class: "dialog-title", attrs: { slot: "title" } }, opts.title), opts.body, opts.actions),
  );
  dialog.type = opts.type ?? "modal";
  onClosed(dialog, () => opts.onClose?.());
  document.body.appendChild(dialog);
  dialog.show();
  return { dialog, close: () => dialog.hide() };
}

/** Yes/no confirmation. Resolves true only when the confirm button is pressed. */
export function confirmDialog(opts: { title: string; message: string; confirm: string; danger?: boolean }): Promise<boolean> {
  return new Promise((resolve) => {
    let confirmed = false;
    const ok = button(
      opts.confirm,
      () => {
        confirmed = true;
        handle.close();
      },
      { appearance: "primary", class: opts.danger ? "danger" : undefined },
    );
    ok.setAttribute("autofocus", "");
    const handle = openDialog({
      title: opts.title,
      type: "alert",
      body: el("p", { class: "dialog-text" }, opts.message),
      actions: [button("Cancel", () => handle.close()), ok],
      onClose: () => resolve(confirmed),
    });
  });
}

/**
 * Single text input dialog. `submit` receives the trimmed text and returns an error message to
 * show inline, or null to close the dialog.
 */
export function promptDialog(opts: {
  title: string;
  subtitle?: string;
  label: string;
  placeholder?: string;
  /** Text already in the field, selected (e.g. the current name when renaming). */
  value?: string;
  confirm: string;
  submit: (text: string) => Promise<string | null>;
}): void {
  const input = el("fluent-text-input", { placeholder: opts.placeholder ?? "", value: opts.value ?? "", attrs: { autofocus: true } });
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });
  let busy = false;
  const submit = async () => {
    if (busy) return;
    const text = input.value.trim();
    busy = true;
    ok.disabled = true;
    const problem = await opts.submit(text);
    busy = false;
    ok.disabled = false;
    if (problem === null) handle.close();
    else error.textContent = problem;
  };
  input.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      void submit();
    }
  });
  const ok = button(opts.confirm, () => void submit(), { appearance: "primary" });
  const handle = openDialog({
    title: opts.title,
    class: "narrow-dialog",
    body: el("div", { class: "stack" }, opts.subtitle ? el("p", { class: "dialog-text muted" }, opts.subtitle) : null, field(opts.label, input), error),
    actions: [button("Cancel", () => handle.close()), ok],
  });
}
