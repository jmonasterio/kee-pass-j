// Quick Access: the small window the global hotkey opens over whatever app is in front.
// It lists the accounts matching that app's window (when there are several or none) and
// searches every open database. Enter types the account into that app; it never shows passwords.

import { api, errorText, onQuickOpen, type QuickItem, type QuickOpen } from "../api";
import { button, el, icon, setChildren } from "../dom";

let state: QuickOpen = { locked: false, windowTitle: "", matches: [] };
let query = "";
let results: QuickItem[] = [];
let selected = 0;
let searchSeq = 0;

let root: HTMLElement;
let search: HTMLElementTagNameMap["fluent-text-input"];
let context: HTMLElement;
let list: HTMLElement;
let status: HTMLElement;

function shown(): QuickItem[] {
  return query ? results : state.matches;
}

function fail(err: unknown): void {
  status.textContent = errorText(err);
}

async function typeItem(item: QuickItem): Promise<void> {
  try {
    await api.quickType(item.db, item.id);
  } catch (err) {
    fail(err);
  }
}

async function copy(item: QuickItem, field: "username" | "password"): Promise<void> {
  try {
    await api.quickCopy(item.db, item.id, field);
  } catch (err) {
    fail(err);
  }
}

function row(item: QuickItem, index: number): HTMLElement {
  const isSelected = index === selected;
  const sub = [item.username, item.folder, item.dbName].filter(Boolean).join(" · ");
  return el(
    "div",
    {
      class: "quick-row" + (isSelected ? " selected" : "") + (item.stale ? " stale" : ""),
      attrs: { role: "option", "aria-selected": String(isSelected) },
      on: {
        click: () => {
          selected = index;
          render();
        },
        dblclick: () => void typeItem(item),
      },
    },
    el(
      "div",
      { class: "quick-text" },
      el("div", { class: "entry-title" }, item.title || "(untitled)", item.stale ? el("span", { class: "quick-stale" }, " · not working") : null),
      el("div", { class: "entry-sub" }, sub || item.url),
    ),
    isSelected
      ? el(
          "div",
          { class: "quick-actions" },
          button("Type", () => void typeItem(item), { appearance: "primary", size: "small", title: "Type into the app behind (Enter)" }),
          button("Password", () => void copy(item, "password"), { icon: "copy", size: "small", title: "Copy password (Ctrl+Enter)" }),
          item.username ? button("Username", () => void copy(item, "username"), { icon: "copy", size: "small", title: "Copy username (Ctrl+B)" }) : null,
        )
      : null,
  );
}

function render(): void {
  status.textContent = "";
  if (state.locked) {
    context.textContent = "No database is unlocked.";
    setChildren(
      list,
      el(
        "div",
        { class: "quick-empty" },
        el("p", null, "Unlock a database in Our Logins first, then press the hotkey again."),
        button("Open Our Logins", () => void api.quickOpenMain().catch(fail), { appearance: "primary" }),
      ),
    );
    return;
  }
  const items = shown();
  selected = Math.min(selected, Math.max(0, items.length - 1));
  const target = state.windowTitle ? `“${state.windowTitle}”` : "the active window";
  context.textContent = query
    ? `${items.length || "No"} account${items.length === 1 ? "" : "s"} found`
    : state.matches.length
      ? `Accounts for ${target}`
      : `No account matches ${target}. Type to search all accounts.`;
  setChildren(list, items.map(row));
  list.querySelector(".quick-row.selected")?.scrollIntoView({ block: "nearest" });
}

function onKey(event: KeyboardEvent): void {
  if (event.isComposing) return;
  const items = shown();
  const item = items[selected];
  const mod = event.ctrlKey || event.metaKey;
  if (event.key === "Escape") {
    event.preventDefault();
    void api.quickHide();
  } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (items.length === 0) return;
    selected = (selected + (event.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
    render();
  } else if (event.key === "Enter" && item) {
    event.preventDefault();
    void (mod ? copy(item, "password") : typeItem(item));
  } else if (mod && event.key.toLowerCase() === "b" && item) {
    event.preventDefault();
    void copy(item, "username");
  }
}

export async function startQuick(container: HTMLElement): Promise<void> {
  root = container;
  document.documentElement.classList.add("quick-window");
  search = el(
    "fluent-text-input",
    {
      class: "quick-search",
      placeholder: "Search accounts",
      attrs: { spellcheck: "false", autocomplete: "off", "aria-label": "Search accounts" },
      on: {
        input: () => {
          query = search.value.trim();
          selected = 0;
          const seq = ++searchSeq;
          if (!query) {
            results = [];
            render();
            return;
          }
          api.quickSearch(query).then((items) => {
            if (seq !== searchSeq) return;
            results = items;
            render();
          }, fail);
        },
      },
    },
    icon("search", "start"),
  );
  context = el("div", { class: "quick-context" });
  list = el("div", { class: "quick-list", attrs: { role: "listbox", "aria-label": "Accounts" } });
  status = el("div", { class: "quick-status", attrs: { role: "alert" } });
  setChildren(
    root,
    el(
      "div",
      { class: "quick" },
      el("div", { class: "quick-head", attrs: { "data-tauri-drag-region": true } }, search),
      context,
      list,
      status,
      el("div", { class: "quick-foot" }, "Enter types it · Ctrl+Enter copies the password · Ctrl+B copies the username · Esc closes"),
    ),
  );
  document.addEventListener("keydown", onKey);

  await onQuickOpen((payload) => {
    state = payload;
    query = "";
    results = [];
    selected = 0;
    search.value = "";
    render();
    // The window is shown right after the event; focus once it is visible.
    requestAnimationFrame(() => search.focus());
  });
  render();
}
