import { api, errorText, type EntryDetail, type EntrySummary, type FieldName, type GroupItem, type MainView } from "../api";
import { confirmDialog, isOverlayOpen, promptDialog } from "../dialogs";
import { button, el, icon, setChildren, type Child } from "../dom";
import { nav } from "../nav";
import { toast } from "../toast";
import { openForm } from "./form";
import { openSettings } from "./settings";

const MASK = "••••••••";

let view: MainView | null = null;
let page: HTMLElement | null = null;
let banner: HTMLElement;
let sidebarScroll: HTMLElement;
let searchInput: HTMLElementTagNameMap["fluent-text-input"];
let entryList: HTMLElement;
let detailPane: HTMLElement;
let searchSeq = 0;

/** Applies a MainView-returning command; errors become a toast. */
async function run(call: Promise<MainView>): Promise<void> {
  try {
    nav.main(await call);
  } catch (err) {
    toast(errorText(err));
  }
}

async function attempt(call: Promise<unknown>, done?: string): Promise<void> {
  try {
    await call;
    if (done) toast(done);
  } catch (err) {
    toast(errorText(err));
  }
}

export function renderMain(root: HTMLElement, next: MainView): void {
  view = next;
  if (!page || !root.contains(page)) build(root);
  renderBanner(next);
  renderSidebar(next);
  if (document.activeElement !== searchInput && searchInput.value !== next.search) searchInput.value = next.search;
  renderList(next);
  renderDetail(next);
}

function build(root: HTMLElement): void {
  banner = el("div", { class: "save-banner" });
  sidebarScroll = el("div", { class: "sidebar-scroll" });

  searchInput = el(
    "fluent-text-input",
    {
      class: "search-input",
      placeholder: "Search",
      attrs: { title: "Search all accounts (Ctrl+K)", spellcheck: "false", autocomplete: "off" },
      on: {
        input: () => {
          const seq = ++searchSeq;
          api.search(searchInput.value).then(
            (v) => {
              if (seq === searchSeq) nav.main(v);
            },
            (err) => toast(errorText(err)),
          );
        },
      },
    },
    icon("search", "start"),
  );

  entryList = el("div", { class: "entry-list", attrs: { role: "listbox", "aria-label": "Accounts", tabindex: "0" } });
  detailPane = el("section", { class: "detail-pane", attrs: { "aria-label": "Account details" } });

  page = el(
    "div",
    { class: "main-page" },
    el(
      "aside",
      { class: "sidebar" },
      el("div", { class: "sidebar-top", attrs: { "data-tauri-drag-region": true } }),
      sidebarScroll,
      el(
        "div",
        { class: "sidebar-bottom" },
        button("Settings", () => void openSettings(), { icon: "settings", appearance: "subtle", class: "nav-button" }),
      ),
    ),
    el(
      "div",
      { class: "content" },
      banner,
      el(
        "div",
        { class: "panes" },
        el(
          "section",
          { class: "list-pane" },
          el(
            "div",
            { class: "list-toolbar", attrs: { "data-tauri-drag-region": true } },
            searchInput,
            button("Add", () => void openForm("add", view?.notesMonospace ?? false), { appearance: "primary", icon: "add", title: "Add account (Ctrl+N)" }),
          ),
          entryList,
        ),
        detailPane,
      ),
    ),
  );
  setChildren(root, page);
}

function renderBanner(v: MainView): void {
  if (!v.saveError) {
    setChildren(banner);
    return;
  }
  const retry = button("Retry", () => void run(api.retrySave()), { size: "small" });
  retry.setAttribute("slot", "actions");
  setChildren(banner, el("fluent-message-bar", { intent: "error", layout: "multiline" }, v.saveError, retry));
}

function renderSidebar(v: MainView): void {
  const tabs = v.tabs.map((tab, index) =>
    el(
      "div",
      {
        class: "nav-row tab-row" + (index === v.activeTab ? " selected" : ""),
        attrs: { role: "button", tabindex: "0", "aria-current": index === v.activeTab ? "true" : undefined, title: tab.path },
        on: {
          click: () => {
            if (index !== v.activeTab) void run(api.selectTab(index));
          },
          keydown: (event: KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              if (index !== v.activeTab) void run(api.selectTab(index));
            }
          },
        },
      },
      el("span", { class: "nav-text" }, tab.name),
      tab.dirty ? el("span", { class: "dirty-dot", attrs: { title: "Not saved" } }) : null,
      button(
        null,
        (event) => {
          event.stopPropagation();
          void lockTab(index);
        },
        { icon: "lock", appearance: "transparent", size: "small", title: `Lock ${tab.name}`, class: "row-action" },
      ),
    ),
  );

  const groups = v.groups.map((group) =>
    el(
      "div",
      {
        class: "nav-row group-row" + (group.id === v.selectedGroup ? " selected" : "") + (group.isRecycleBin ? " bin" : ""),
        style: { "--depth": String(group.depth) },
        attrs: { role: "treeitem", tabindex: "0", "aria-selected": String(group.id === v.selectedGroup), "aria-level": group.depth + 1 },
        on: {
          click: () => void run(api.selectGroup(group.id)),
          keydown: (event: KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              void run(api.selectGroup(group.id));
            }
          },
        },
      },
      el("span", { class: "nav-text" }, group.name),
      group.depth > 0
        ? button(
            null,
            (event) => {
              event.stopPropagation();
              void deleteGroup(v, group);
            },
            {
              icon: "delete",
              appearance: "transparent",
              size: "small",
              title: group.isRecycleBin ? "Empty Recycle Bin" : `Delete ${group.name}`,
              class: "row-action",
            },
          )
        : null,
    ),
  );

  setChildren(
    sidebarScroll,
    el("div", { class: "section-head" }, el("span", null, "Databases")),
    el("div", { class: "nav-list", attrs: { role: "list" } }, tabs),
    button("Open…", () => void showUnlockPage(), { icon: "add", appearance: "subtle", class: "nav-button" }),
    el(
      "div",
      { class: "section-head" },
      el("span", null, "Groups"),
      button(null, () => addGroup(v), { icon: "add", appearance: "transparent", size: "small", title: "Add group" }),
    ),
    el("div", { class: "nav-list", attrs: { role: "tree", "aria-label": "Groups" } }, groups),
  );
}

async function showUnlockPage(): Promise<void> {
  try {
    nav.unlock(await api.unlockView());
  } catch (err) {
    toast(errorText(err));
  }
}

async function lockTab(index: number): Promise<void> {
  const tab = view?.tabs[index];
  if (!tab) return;
  if (tab.dirty) {
    const ok = await confirmDialog({
      title: "Lock without saving?",
      message: `The last save of "${tab.name}" failed. Changes made since then will be lost if you lock it now.`,
      confirm: "Lock anyway",
      danger: true,
    });
    if (!ok) return;
  }
  try {
    const next = await api.lockTab(index);
    if (next.tabs.length === 0) nav.unlock(await api.unlockView());
    else nav.main(next);
  } catch (err) {
    toast(errorText(err));
  }
}

function addGroup(v: MainView): void {
  const parent = v.groups.find((g) => g.id === v.selectedGroup) ?? v.groups[0];
  if (!parent) return;
  promptDialog({
    title: "New group",
    subtitle: `Inside ${parent.name}`,
    label: "Name",
    placeholder: "Group name",
    confirm: "Add group",
    submit: async (name) => {
      if (!name) return "Give the group a name first.";
      try {
        nav.main(await api.addGroup(parent.id, name));
        return null;
      } catch (err) {
        return errorText(err);
      }
    },
  });
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** True when `group` is the Recycle Bin or lies inside it. */
function inRecycleBin(v: MainView, group: GroupItem): boolean {
  const byId = new Map(v.groups.map((g) => [g.id, g]));
  for (let g: GroupItem | undefined = group; g; g = g.parent ? byId.get(g.parent) : undefined) {
    if (g.isRecycleBin) return true;
  }
  return false;
}

async function deleteGroup(v: MainView, group: GroupItem): Promise<void> {
  // `groups` is pre-order, so the subtree is the run of deeper groups right after this one.
  const start = v.groups.indexOf(group);
  let end = start + 1;
  while (end < v.groups.length && v.groups[end].depth > group.depth) end++;
  const subtree = v.groups.slice(start, end);
  const accounts = subtree.reduce((sum, g) => sum + g.entryCount, 0);
  const parts = [accounts ? plural(accounts, "account") : "", subtree.length > 1 ? plural(subtree.length - 1, "group") : ""];
  const contents = parts.filter(Boolean).join(", ");
  const what = contents ? `"${group.name}" and everything in it (${contents})` : `"${group.name}" (empty)`;
  const permanent = !v.recycleBinEnabled || inRecycleBin(v, group);

  const ok = await confirmDialog(
    group.isRecycleBin
      ? {
          title: "Empty Recycle Bin?",
          message: contents
            ? `Everything in the Recycle Bin (${contents}) will be deleted for good. This can't be undone.`
            : "The Recycle Bin is already empty. It will be removed and created again on the next delete.",
          confirm: "Empty Recycle Bin",
          danger: true,
        }
      : permanent
        ? {
            title: "Delete permanently?",
            message: `${what} will be deleted for good. This can't be undone.`,
            confirm: "Delete",
            danger: true,
          }
        : {
            title: "Delete group?",
            message: `${what} will move to the Recycle Bin.`,
            confirm: "Move to Recycle Bin",
          },
  );
  if (!ok) return;
  try {
    nav.main(await api.deleteGroup(group.id));
    toast(group.isRecycleBin ? "Recycle Bin emptied" : permanent ? "Deleted" : "Moved to Recycle Bin");
  } catch (err) {
    toast(errorText(err));
  }
}

/** Distinct hues for letter tiles; raw `hash % 360` bunches similar names together. */
const AVATAR_HUES = [4, 28, 48, 96, 150, 178, 204, 228, 266, 312];

/** Letter tile coloured from the title, so accounts are told apart at a glance. */
function avatar(title: string, size: "small" | "large"): HTMLElement {
  const letter = (title.match(/[\p{L}\p{N}]/u)?.[0] ?? "?").toUpperCase();
  let hash = 0;
  for (const ch of title.toLowerCase()) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  const hue = AVATAR_HUES[hash % AVATAR_HUES.length];
  return el("div", { class: `avatar ${size}`, style: { "--hue": String(hue) }, attrs: { "aria-hidden": "true" } }, letter);
}

function entryRow(entry: EntrySummary, selected: boolean): HTMLElement {
  return el(
    "div",
    {
      class: "entry-row" + (selected ? " selected" : "") + (entry.stale ? " stale" : ""),
      dataset: { id: entry.id },
      attrs: { role: "option", "aria-selected": String(selected) },
      on: { click: () => void run(api.selectEntry(entry.id)) },
    },
    avatar(entry.title, "small"),
    el(
      "div",
      { class: "entry-text" },
      el(
        "div",
        { class: "entry-title-line" },
        el("span", { class: "entry-title" }, entry.title || "(untitled)"),
        entry.stale ? el("fluent-badge", { appearance: "tint", color: "danger", size: "small" }, "not working") : null,
      ),
      el("div", { class: "entry-sub" }, entry.username || entry.url),
    ),
  );
}

function renderList(v: MainView): void {
  if (v.entries.length === 0) {
    setChildren(entryList, el("div", { class: "empty-state" }, v.search ? "Nothing matches." : "No accounts in this group yet."));
    return;
  }
  const rows: Child[] = [];
  let dividerAdded = false;
  for (const entry of v.entries) {
    if (entry.stale && !dividerAdded) {
      if (rows.length > 0) rows.push(el("fluent-divider", { class: "stale-divider" }));
      dividerAdded = true;
    }
    rows.push(entryRow(entry, entry.id === v.selectedEntry));
  }
  setChildren(entryList, rows);
  entryList.querySelector(".entry-row.selected")?.scrollIntoView({ block: "nearest" });
}

function fieldRow(label: string, value: Child, actions: Child[] = [], valueClass = ""): HTMLElement {
  return el(
    "div",
    { class: "field-row" },
    el("div", { class: "field-label" }, label),
    el("div", { class: "field-line" }, el("div", { class: "field-value " + valueClass }, value), el("div", { class: "field-actions" }, actions)),
  );
}

function copyButton(field: FieldName): HTMLElement {
  return button("Copy", () => void attempt(api.copyField(field), "Copied"), { icon: "copy", appearance: "subtle", size: "small" });
}

/** Masked value with Show/Hide (reveal_field) and Copy (copy_field). Hidden again on every re-render. */
function secretRow(label: string, field: FieldName, multiline = false): HTMLElement {
  const value = el("span", { class: "secret masked" }, MASK);
  let shown = false;
  const toggle = button(
    "Show",
    async () => {
      if (shown) {
        shown = false;
        value.textContent = MASK;
        value.classList.add("masked");
        setChildren(toggle, icon("eye", "start"), "Show");
        return;
      }
      try {
        const revealed = await api.revealField(field);
        shown = true;
        value.textContent = revealed;
        value.classList.remove("masked");
        setChildren(toggle, icon("eyeOff", "start"), "Hide");
      } catch (err) {
        toast(errorText(err));
      }
    },
    { icon: "eye", appearance: "subtle", size: "small" },
  );
  return fieldRow(label, value, [toggle, copyButton(field)], multiline ? "mono multiline" : "mono");
}

function notesBlock(notes: string, mono: boolean): HTMLElement {
  const lines = notes.split(/\r?\n/).map((line) =>
    el(
      "div",
      { class: "note-line" },
      el("span", { class: "note-text" }, line || "\u00a0"),
      line.trim()
        ? button("copy", () => void attempt(api.copyText(line), "Copied"), { appearance: "transparent", size: "small", class: "note-copy" })
        : null,
    ),
  );
  return el("div", { class: "notes-box" + (mono ? " mono" : "") }, lines);
}

async function deleteEntry(d: EntryDetail, permanent: boolean): Promise<void> {
  if (permanent) {
    const ok = await confirmDialog({
      title: "Delete permanently?",
      message: d.inBin
        ? `"${d.title || "(untitled)"}" will be deleted for good. This can't be undone.`
        : `This database has its Recycle Bin turned off, so "${d.title || "(untitled)"}" will be deleted for good. This can't be undone.`,
      confirm: "Delete",
      danger: true,
    });
    if (!ok) return;
  }
  try {
    nav.main(await api.deleteEntry());
    toast(permanent ? "Deleted" : "Moved to Recycle Bin");
  } catch (err) {
    toast(errorText(err));
  }
}

async function removeAttachment(name: string): Promise<void> {
  const ok = await confirmDialog({
    title: "Remove attachment?",
    message: `"${name}" will be removed from this account.`,
    confirm: "Remove",
    danger: true,
  });
  if (ok) await run(api.removeAttachment(name));
}

async function saveAttachment(name: string): Promise<void> {
  try {
    const saved = await api.saveAttachment(name);
    if (saved) toast("Saved");
  } catch (err) {
    toast(errorText(err));
  }
}

function overflowMenu(d: EntryDetail, v: MainView): HTMLElement {
  const permanent = d.inBin || !v.recycleBinEnabled;
  const trigger = button(null, () => {}, { icon: "more", appearance: "subtle", title: "More actions" });
  trigger.setAttribute("slot", "trigger");
  return el(
    "fluent-menu",
    null,
    trigger,
    el(
      "fluent-menu-list",
      null,
      el("fluent-menu-item", { on: { click: () => void run(api.toggleStale()) } }, d.stale ? "Mark working" : "Mark not working"),
      el("fluent-menu-item", { class: "danger-item", on: { click: () => void deleteEntry(d, permanent) } }, permanent ? "Delete permanently" : "Delete"),
    ),
  );
}

function renderDetail(v: MainView): void {
  const d = v.detail;
  if (!d) {
    setChildren(detailPane, el("div", { class: "detail-scroll" }, el("div", { class: "empty-state" }, "Select an account")));
    return;
  }

  const rows: Child[] = [];
  if (d.username) rows.push(fieldRow("Username", d.username, [copyButton("username")]));
  if (d.hasPassword) rows.push(secretRow("Password", "password"));
  if (d.url)
    rows.push(
      fieldRow(
        "Website",
        el(
          "a",
          {
            class: "link",
            href: "#",
            on: {
              click: (event: MouseEvent) => {
                event.preventDefault();
                void attempt(api.openUrl(d.url));
              },
            },
          },
          d.url,
        ),
        [button("Open", () => void attempt(api.openUrl(d.url)), { icon: "open", appearance: "subtle", size: "small" }), copyButton("url")],
      ),
    );
  if (d.hasRecoveryPin) rows.push(secretRow("Recovery PIN", "recoveryPin"));
  if (d.recoveryPhone) rows.push(fieldRow("Recovery phone", d.recoveryPhone, [copyButton("recoveryPhone")]));
  if (d.hasSecurityQuestions) rows.push(secretRow("Security questions", "securityQuestions", true));
  for (const custom of d.custom) {
    const fieldName: FieldName = `custom:${custom.key}`;
    if (custom.protected) rows.push(secretRow(custom.key, fieldName));
    else rows.push(fieldRow(custom.key, custom.value ?? "", [copyButton(fieldName)], "multiline"));
  }
  if (d.notes) rows.push(el("div", { class: "field-row" }, el("div", { class: "field-label" }, "Notes"), notesBlock(d.notes, v.notesMonospace)));

  rows.push(
    el(
      "div",
      { class: "field-row" },
      el(
        "div",
        { class: "field-label-line" },
        el("div", { class: "field-label" }, "Attachments"),
        button("Add file…", () => void run(api.addAttachment()), { icon: "add", appearance: "subtle", size: "small" }),
      ),
      d.attachments.length === 0
        ? el("div", { class: "muted small" }, "No files attached.")
        : d.attachments.map((name) =>
            el(
              "div",
              { class: "attachment-row" },
              el("span", { class: "attachment-name" }, name),
              button("Save as…", () => void saveAttachment(name), { appearance: "subtle", size: "small" }),
              button("Remove", () => void removeAttachment(name), { appearance: "subtle", size: "small" }),
            ),
          ),
    ),
  );

  if (d.tags.length > 0)
    rows.push(
      el(
        "div",
        { class: "field-row" },
        el("div", { class: "field-label" }, "Tags"),
        el("div", { class: "tags" }, d.tags.map((tag) => el("fluent-badge", { appearance: "tint", color: "brand" }, tag))),
      ),
    );

  setChildren(
    detailPane,
    el(
      "div",
      { class: "detail-scroll" },
      el(
        "div",
        { class: "detail-header", attrs: { "data-tauri-drag-region": true } },
        avatar(d.title, "large"),
        el(
          "div",
          { class: "detail-title-block" },
          el("h2", { class: "detail-title" }, d.title || "(untitled)"),
          el(
            "div",
            { class: "detail-badges" },
            d.stale ? el("fluent-badge", { appearance: "tint", color: "danger" }, "not working") : null,
            d.inBin ? el("fluent-badge", { appearance: "tint", color: "subtle" }, "In Recycle Bin") : null,
          ),
        ),
        el(
          "div",
          { class: "detail-actions" },
          button("Edit", () => void openForm("edit", v.notesMonospace), { appearance: "primary" }),
          button("Auto-Type", () => void attempt(api.autotypeSelected())),
          overflowMenu(d, v),
        ),
      ),
      el("div", { class: "fields" }, rows),
      el("div", { class: "modified muted" }, `Changed ${d.modified}`),
    ),
  );
}

function isTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "FLUENT-TEXT-INPUT" || tag === "FLUENT-TEXTAREA" || target.isContentEditable;
}

function moveSelection(delta: number): void {
  if (!view || view.entries.length === 0) return;
  const index = view.entries.findIndex((e) => e.id === view!.selectedEntry);
  const next = index < 0 ? (delta > 0 ? 0 : view.entries.length - 1) : Math.min(view.entries.length - 1, Math.max(0, index + delta));
  if (index === next) return;
  void run(api.selectEntry(view.entries[next].id));
}

/** Page shortcuts: Ctrl+K/F search, Ctrl+N add, Up/Down select, Esc clears search. */
export function mainKeydown(event: KeyboardEvent): void {
  if (!view || isOverlayOpen() || event.isComposing) return;
  const mod = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  if (mod && !event.altKey && (key === "k" || key === "f")) {
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
    return;
  }
  if (mod && !event.altKey && key === "n") {
    event.preventDefault();
    void openForm("add", view.notesMonospace);
    return;
  }
  if (event.key === "Escape" && event.target === searchInput) {
    if (searchInput.value) {
      event.preventDefault();
      searchInput.value = "";
      const seq = ++searchSeq;
      api.search("").then(
        (v) => {
          if (seq === searchSeq) nav.main(v);
        },
        (err) => toast(errorText(err)),
      );
    }
    return;
  }
  if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !mod && !event.altKey) {
    if (event.target !== searchInput && isTextTarget(event.target)) return;
    // Let the overflow menu and other popups handle their own arrows.
    if (event.target instanceof HTMLElement && event.target.closest("fluent-menu, fluent-dropdown")) return;
    event.preventDefault();
    moveSelection(event.key === "ArrowDown" ? 1 : -1);
  }
}
