import { api, errorText, type RecentItem, type UnlockView } from "../api";
import { button, el, field, icon, setChildren } from "../dom";
import { passwordBox } from "../password-box";
import { nav } from "../nav";

let view: UnlockView | null = null;
let selected: string | null = null;
let passwordInput: HTMLElementTagNameMap["fluent-text-input"] | null = null;
let submit: (() => void) | null = null;

export function renderUnlock(root: HTMLElement, next: UnlockView): void {
  // Keep a typed password only while this page stays on screen and refreshes for the same
  // database (e.g. after picking a key file). Once the page was left, nothing carries over.
  const keptPassword = passwordInput?.isConnected && view && !view.pendingNew ? passwordInput.value : "";
  // Arriving at the page (app start, lock), as opposed to a re-render of the page already shown.
  const arriving = !passwordInput?.isConnected;
  const previous = selected;
  leaveUnlock();
  view = next;
  const paths = new Set(next.recent.map((r) => r.path));
  selected = next.selected ?? (previous && paths.has(previous) ? previous : null);

  // Opening a second database: say so, name what is already open, and offer the way back.
  const adding = next.openDbs.length > 0;
  const header = adding
    ? el(
        "div",
        { class: "unlock-header adding" },
        button(null, () => void back(), { icon: "back", appearance: "subtle", title: "Back to your open databases (Esc)" }),
        el(
          "div",
          null,
          el("h1", { class: "app-title" }, "Open another database"),
          el("div", { class: "muted" }, `${next.openDbs.join(", ")} stay${next.openDbs.length === 1 ? "s" : ""} open. This one opens in a new tab.`),
        ),
      )
    : el("div", { class: "unlock-header" }, el("h1", { class: "app-title" }, "Our Logins"));
  const body = next.pendingNew ? createForm(next.pendingNew) : openForm(next, selected === previous ? keptPassword : "", arriving && !adding);

  setChildren(
    root,
    el(
      "div",
      { class: "unlock-page" },
      el("div", { class: "drag-strip", attrs: { "data-tauri-drag-region": true } }),
      el("div", { class: "unlock-card" }, header, body),
    ),
  );
  (passwordInput as HTMLElement | null)?.focus();
}

/** Wipes the password boxes of the unlock page; called whenever the page is replaced. */
export function leaveUnlock(): void {
  for (const box of document.querySelectorAll<HTMLElementTagNameMap["fluent-text-input"]>(".unlock-card .password-box")) {
    box.value = "";
  }
  if (passwordInput) passwordInput.value = "";
  passwordInput = null;
  submit = null;
}

async function back(): Promise<void> {
  try {
    nav.main(await api.mainView());
  } catch (err) {
    showError(errorText(err));
  }
}

let errorNode: HTMLElement | null = null;
function showError(message: string): void {
  if (errorNode) errorNode.textContent = message;
}

async function refresh(call: Promise<UnlockView>): Promise<void> {
  try {
    renderUnlock(document.getElementById("app")!, await call);
  } catch (err) {
    showError(errorText(err));
  }
}

function openForm(v: UnlockView, keptPassword: string, autoQuick: boolean): HTMLElement {
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });
  errorNode = error;
  const password = passwordBox({ placeholder: "Master password" });
  password.value = keptPassword;
  passwordInput = password;

  const unlockButton = button("Unlock", () => void doUnlock(), { appearance: "primary" });
  const keyfileButton = button("Key file…", () => {
    if (selected) void refresh(api.chooseKeyfile(selected));
  });
  // "Unlock with Touch ID" / "Unlock with Windows Hello", shown when a password is stored.
  const quickButton = v.quickUnlockLabel
    ? button(`Unlock with ${v.quickUnlockLabel}`, () => void doQuickUnlock(), { appearance: "primary", icon: "key", class: "quick-unlock" })
    : null;

  const rows = new Map<string, HTMLElement>();
  const current = (): RecentItem | undefined => v.recent.find((r) => r.path === selected);
  const updateSelection = () => {
    for (const [path, row] of rows) {
      const on = path === selected;
      row.classList.toggle("selected", on);
      row.setAttribute("aria-selected", String(on));
    }
    const item = current();
    unlockButton.disabled = !item || item.missing;
    keyfileButton.disabled = !item;
    if (quickButton) {
      const usable = !!item && item.quickUnlock && !item.missing;
      quickButton.hidden = !usable;
      // With quick unlock on offer, the password button steps back to the secondary look.
      unlockButton.setAttribute("appearance", usable ? "outline" : "primary");
    }
  };

  const select = (path: string) => {
    if (path !== selected) password.value = "";
    selected = path;
    error.textContent = "";
    updateSelection();
    password.focus();
  };

  const recentRow = (item: RecentItem): HTMLElement => {
    const row = el(
      "div",
      {
        class: "recent-row" + (item.missing ? " missing" : ""),
        attrs: { role: "option", tabindex: "0" },
        on: {
          click: () => select(item.path),
          keydown: (event: KeyboardEvent) => {
            if (event.key === " " || (event.key === "Enter" && event.target === row)) {
              event.preventDefault();
              event.stopPropagation();
              select(item.path);
            }
          },
        },
      },
      el(
        "div",
        { class: "recent-text" },
        el(
          "div",
          { class: "recent-name-line" },
          el("span", { class: "recent-name" }, item.name),
          item.missing ? el("fluent-badge", { appearance: "tint", color: "danger", size: "small" }, "file not found") : null,
        ),
        el("div", { class: "recent-path" }, item.path),
        item.keyfile
          ? el(
              "div",
              { class: "recent-keyfile" },
              el("span", null, "Key file: ", el("span", { class: "keyfile-name" }, item.keyfile)),
              button(
                "Clear",
                (event) => {
                  event.stopPropagation();
                  void refresh(api.clearKeyfile(item.path));
                },
                { appearance: "transparent", size: "small" },
              ),
            )
          : null,
      ),
      button(
        null,
        (event) => {
          event.stopPropagation();
          void refresh(api.forgetRecent(item.path));
        },
        { icon: "dismiss", appearance: "subtle", size: "small", title: "Remove from list", class: "recent-remove" },
      ),
    );
    rows.set(item.path, row);
    return row;
  };

  let busy = false;
  const doUnlock = async () => {
    const item = current();
    if (busy || !item || item.missing) return;
    busy = true;
    unlockButton.disabled = true;
    unlockButton.textContent = "Unlocking…";
    error.textContent = "";
    try {
      const main = await api.unlock(item.path, password.value);
      password.value = "";
      nav.main(main);
    } catch (err) {
      error.textContent = errorText(err);
      unlockButton.textContent = "Unlock";
      updateSelection();
      password.select();
      password.focus();
    } finally {
      busy = false;
    }
  };
  const doQuickUnlock = async () => {
    const item = current();
    if (busy || !item || !item.quickUnlock || !quickButton) return;
    busy = true;
    quickButton.disabled = true;
    error.textContent = "";
    try {
      nav.main(await api.unlockQuick(item.path));
    } catch (err) {
      // A changed master password removes the stored one: re-render so the button goes away,
      // then show the message on the new page.
      const message = errorText(err);
      await refresh(api.unlockView());
      showError(message);
    } finally {
      busy = false;
    }
  };
  // Enter with an empty password field uses quick unlock when it is on offer.
  submit = () => void (password.value === "" && current()?.quickUnlock && quickButton ? doQuickUnlock() : doUnlock());

  const list =
    v.recent.length > 0
      ? el("div", { class: "recent-list", attrs: { role: "listbox", "aria-label": "Recent databases" } }, v.recent.map(recentRow))
      : el("p", { class: "muted empty-recent" }, v.openDbs.length > 0 ? "No other recent databases. Open a file or create a new one." : "No recent databases. Open a file or create a new one.");

  updateSelection();

  // Show the Touch ID / Windows Hello prompt right away on arrival, once. A cancel or error
  // re-renders the page without arriving, so it does not prompt again; the button stays.
  const auto = current();
  if (autoQuick && quickButton && auto?.quickUnlock && !auto.missing) {
    const fire = () => {
      if (quickButton.isConnected && selected === auto.path) void doQuickUnlock();
    };
    // Wait for the window to have focus so the prompt does not open behind other apps.
    if (document.hasFocus()) setTimeout(fire, 0);
    else window.addEventListener("focus", fire, { once: true });
  }

  return el(
    "div",
    { class: "stack" },
    el("div", { class: "section-label" }, v.openDbs.length > 0 ? "Other recent databases" : "Recent databases"),
    list,
    quickButton,
    field("Master password", password),
    el(
      "div",
      { class: "button-row" },
      keyfileButton,
      el("div", { class: "spacer" }),
      unlockButton,
    ),
    error,
    el("fluent-divider", null),
    el(
      "div",
      { class: "button-row" },
      button("Open file…", () => void refresh(api.browseOpen()), { icon: "folder" }),
      button("New database…", () => void refresh(api.browseNew()), { icon: "add" }),
    ),
  );
}

function createForm(path: string): HTMLElement {
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });
  errorNode = error;
  const password = passwordBox({ placeholder: "Master password" });
  const repeat = passwordBox({ placeholder: "Repeat the password" });
  passwordInput = password;
  const createButton = button("Create database", () => void doCreate(), { appearance: "primary" });

  let busy = false;
  const doCreate = async () => {
    if (busy) return;
    busy = true;
    createButton.disabled = true;
    error.textContent = "";
    try {
      const main = await api.createDatabase(password.value, repeat.value);
      password.value = "";
      repeat.value = "";
      nav.main(main);
    } catch (err) {
      error.textContent = errorText(err);
    } finally {
      busy = false;
      createButton.disabled = false;
    }
  };
  submit = () => void doCreate();

  return el(
    "div",
    { class: "stack" },
    el("div", { class: "section-label" }, "New database"),
    el("div", { class: "create-path" }, icon("folder"), el("span", null, path)),
    el(
      "fluent-message-bar",
      { intent: "warning", layout: "multiline" },
      "Write this password down somewhere safe. If you forget it, nobody can recover your passwords.",
    ),
    field("Master password", password),
    field("Repeat password", repeat),
    error,
    el(
      "div",
      { class: "button-row end" },
      button("Cancel", () => void refresh(api.cancelCreate())),
      createButton,
    ),
  );
}

/** Page keyboard handling: Enter submits, Esc goes back or cancels creation. */
export function unlockKeydown(event: KeyboardEvent): void {
  if (!view || event.isComposing) return;
  if (event.key === "Escape") {
    if (view.pendingNew) {
      event.preventDefault();
      void refresh(api.cancelCreate());
    } else if (view.openDbs.length > 0) {
      event.preventDefault();
      void back();
    }
    return;
  }
  if (event.key === "Enter" && submit) {
    const target = event.target as HTMLElement | null;
    // Buttons and recent rows handle Enter themselves.
    if (target && (target.tagName === "FLUENT-BUTTON" || target.classList.contains("recent-row"))) return;
    event.preventDefault();
    submit();
  }
}
