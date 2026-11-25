import {
  api,
  errorText,
  FOLDER_COLORS,
  type AttachmentInfo,
  type CustomField,
  type Draft,
  type DraftEntry,
  type DraftFiles,
  type Duplicate,
  type EntryDetail,
  type EntrySummary,
  type FieldName,
  type Filter,
  type FolderColor,
  type FolderOption,
  type GenOptions,
  type GroupItem,
  type Id,
  type MainView,
  type TabItem,
  type VersionDetail,
  type VersionItem,
} from "../api";
import { confirmDialog, isOverlayOpen, openDialog, promptDialog } from "../dialogs";
import { button, debounce, el, field, icon, setChildren, type Child } from "../dom";
import { BACKUP_COPIES, FILE_ADVICE, formatBytes, LARGE_FILE, LARGE_TOTAL } from "../files";
import { renderMarkdown } from "../markdown";
import { passwordBox, setSecret } from "../password-box";
import { nav } from "../nav";
import { copyAndTell, toast } from "../toast";
import { openSettings } from "./settings";
import { checkForUpdates } from "../update";

type TextInputEl = HTMLElementTagNameMap["fluent-text-input"];
type TextAreaEl = HTMLElementTagNameMap["fluent-textarea"];

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
  renderDetail();
}

function build(root: HTMLElement): void {
  history = null;
  idleSaveTimer ??= window.setInterval(() => void saveWhileAway(), 5000);
  banner = el("div", { class: "save-banner" });
  sidebarScroll = el("div", { class: "sidebar-scroll" });

  searchInput = el(
    "fluent-text-input",
    {
      class: "search-input",
      placeholder: "Search",
      attrs: { title: "Search all accounts (Ctrl+K)", spellcheck: "false", autocomplete: "off" },
      on: { input: () => void searchFor(searchInput.value) },
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
        button(
          "Settings",
          async () => {
            if (await leaveDraft()) await openSettings();
          },
          { icon: "settings", appearance: "subtle", class: "nav-button" },
        ),
        button("About", () => void openAbout(), { icon: "info", appearance: "subtle", class: "nav-button" }),
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
            addMenu(),
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

function sameFilter(a: Filter, b: Filter): boolean {
  return a.kind === b.kind && (a.kind !== "group" || (b.kind === "group" && a.id === b.id));
}

function navRow(opts: {
  label: string;
  count?: number;
  selected: boolean;
  onSelect: () => void;
  lead?: Child;
  depth?: number;
  actions?: Child;
  extraClass?: string;
  title?: string;
}): HTMLElement {
  const select = () => {
    if (!opts.selected) opts.onSelect();
  };
  return el(
    "div",
    {
      class: "nav-row" + (opts.selected ? " selected" : "") + (opts.extraClass ? " " + opts.extraClass : ""),
      style: opts.depth ? { "--depth": String(opts.depth) } : undefined,
      attrs: { role: "button", tabindex: "0", "aria-current": opts.selected ? "true" : undefined, title: opts.title },
      on: {
        click: select,
        keydown: (event: KeyboardEvent) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            select();
          }
        },
      },
    },
    opts.lead ?? null,
    el("span", { class: "nav-text" }, opts.label),
    opts.count !== undefined && opts.count > 0 ? el("span", { class: "nav-count" }, String(opts.count)) : null,
    opts.actions ?? null,
  );
}

function renderSidebar(v: MainView): void {
  const tabs = v.tabs.map((tab, index) =>
    navRow({
      label: tab.name,
      selected: index === v.activeTab,
      onSelect: () => void leaveThen(() => api.selectTab(index)),
      title: tab.path,
      extraClass: "tab-row",
      actions: [
        tab.saving
          ? el("span", { class: "saving-label" }, "Saving…")
          : tab.unsaved
            ? el("span", { class: "dirty-dot", attrs: { title: "Not saved" } })
            : null,
        databaseMenu(index, tab),
        button(
          null,
          (event) => {
            event.stopPropagation();
            void lockTab(index);
          },
          { icon: "lock", appearance: "transparent", size: "small", title: `Lock ${tab.name}`, class: "row-action" },
        ),
      ],
    }),
  );

  const show = (filter: Filter) => () => void leaveThen(() => api.selectFilter(filter));
  const colorsOn = v.tileColors === "folder";
  const folders = v.folders.map((f) =>
    navRow({
      label: f.name,
      count: f.entryCount,
      depth: f.depth,
      selected: sameFilter(v.filter, { kind: "group", id: f.id }),
      onSelect: show({ kind: "group", id: f.id }),
      lead: colorsOn ? el("span", { class: "folder-dot", style: { "--hue": String(COLOR_HUES[folderColor(f)]) } }) : icon("folder"),
      extraClass: "folder-row",
      actions: folderMenu(v, f),
    }),
  );

  setChildren(
    sidebarScroll,
    el("div", { class: "section-head" }, el("span", null, "Databases")),
    el("div", { class: "nav-list", attrs: { role: "list" } }, tabs),
    button("Open…", () => void showUnlockPage(), { icon: "add", appearance: "subtle", class: "nav-button" }),
    // With several databases open, name the one the lists below belong to.
    v.tabs.length > 1 && v.tabs[v.activeTab]
      ? el(
          "div",
          { class: "section-head active-db-head", attrs: { title: v.tabs[v.activeTab].path } },
          el("span", null, `In ${v.tabs[v.activeTab].name}`),
        )
      : el("div", { class: "section-gap" }),
    el(
      "div",
      { class: "nav-list", attrs: { role: "list", "aria-label": "Show" } },
      navRow({
        label: "All accounts",
        count: v.counts.all,
        selected: v.filter.kind === "all",
        onSelect: show({ kind: "all" }),
        lead: icon("key"),
      }),
      v.counts.notWorking > 0 || v.filter.kind === "notWorking"
        ? navRow({
            label: "Not working",
            count: v.counts.notWorking,
            selected: v.filter.kind === "notWorking",
            onSelect: show({ kind: "notWorking" }),
            lead: icon("warning"),
          })
        : null,
      navRow({
        label: "Documents",
        count: v.counts.documents,
        selected: v.filter.kind === "documents",
        onSelect: show({ kind: "documents" }),
        lead: icon("document"),
      }),
    ),
    el(
      "div",
      { class: "section-head" },
      el("span", null, "Folders"),
      button(null, () => addGroup(v), { icon: "add", appearance: "transparent", size: "small", title: "New folder" }),
    ),
    v.folders.length === 0
      ? el("div", { class: "nav-empty" }, "No folders yet.")
      : el("div", { class: "nav-list", attrs: { role: "list", "aria-label": "Folders" } }, folders),
    el("div", { class: "section-gap" }),
    el(
      "div",
      { class: "nav-list", attrs: { role: "list" } },
      navRow({
        label: "Recycle Bin",
        count: v.counts.recycleBin,
        selected: v.filter.kind === "recycleBin",
        onSelect: show({ kind: "recycleBin" }),
        lead: icon("delete"),
        extraClass: "bin-row",
        actions:
          v.counts.recycleBin > 0
            ? button(
                null,
                (event) => {
                  event.stopPropagation();
                  void emptyRecycleBin(v);
                },
                { icon: "dismiss", appearance: "transparent", size: "small", title: "Empty Recycle Bin", class: "row-action" },
              )
            : null,
      }),
    ),
  );
}

function folderMenu(v: MainView, f: GroupItem): HTMLElement {
  const trigger = button(null, (event) => event.stopPropagation(), { icon: "more", appearance: "transparent", size: "small", title: `${f.name} options` });
  trigger.setAttribute("slot", "trigger");
  return el(
    "fluent-menu",
    { class: "row-action", on: { click: (event: MouseEvent) => event.stopPropagation() } },
    trigger,
    el(
      "fluent-menu-list",
      null,
      el("fluent-menu-item", { on: { click: () => renameGroup(f) } }, "Rename…"),
      el("fluent-menu-item", { on: { click: () => pickColor(f) } }, "Color…"),
      el("fluent-menu-item", { on: { click: () => addGroup(v, f) } }, "New folder inside"),
      el("fluent-menu-item", { class: "danger-item", on: { click: () => void deleteGroup(v, f) } }, "Delete folder…"),
    ),
  );
}

function databaseMenu(index: number, tab: TabItem): HTMLElement {
  const trigger = button(null, (event) => event.stopPropagation(), { icon: "more", appearance: "transparent", size: "small", title: `${tab.name} options` });
  trigger.setAttribute("slot", "trigger");
  return el(
    "fluent-menu",
    { class: "row-action", on: { click: (event: MouseEvent) => event.stopPropagation() } },
    trigger,
    el(
      "fluent-menu-list",
      null,
      el("fluent-menu-item", { on: { click: () => changeMasterPassword(index, tab) } }, "Change master password…"),
      el("fluent-menu-item", { on: { click: () => void keepVersions(index, tab) } }, `Keep versions (${tab.historyMaxItems})…`),
      el("fluent-menu-item", { on: { click: () => void lockTab(index) } }, "Lock"),
    ),
  );
}

/** How many earlier versions of each entry the database keeps: KDBX metadata, so it belongs
 *  to the file (KeePass and KeePassXC use the same number) and is set per database. */
async function keepVersions(index: number, tab: TabItem): Promise<void> {
  const choices = [0, 3, 5, 10, 20, 30, 50, 100];
  if (!choices.includes(tab.historyMaxItems)) choices.push(tab.historyMaxItems);
  const picker = el(
    "fluent-dropdown",
    { class: "clipboard-dropdown", attrs: { "aria-label": "Versions kept per entry" } },
    el(
      "fluent-listbox",
      null,
      choices
        .sort((a, b) => a - b)
        .map((n) => el("fluent-option", { attrs: { value: String(n), selected: n === tab.historyMaxItems } }, n === 0 ? "None" : plural(n, "version"))),
    ),
  );
  const save = button(
    "Save",
    async () => {
      const max = Number(picker.value);
      handle.close();
      if (max === tab.historyMaxItems) return;
      // The command works on the active database; switch to this one first.
      if (view?.activeTab !== index) {
        if (!(await leaveDraft())) return;
        await run(api.selectTab(index));
      }
      await run(api.setHistoryLimit(max));
      toast(`${tab.name} keeps ${max === 0 ? "no earlier versions" : plural(max, "earlier version")} of each entry.`);
    },
    { appearance: "primary" },
  );
  const handle = openDialog({
    title: "Keep versions",
    class: "narrow-dialog",
    body: el(
      "div",
      { class: "stack" },
      el(
        "p",
        { class: "dialog-text muted" },
        `Earlier versions kept for each entry in "${tab.name}". It is stored in the database file, so KeePass and KeePassXC keep the same number. A lower number deletes the older versions, and files only they had, right away.`,
      ),
      field("Versions per entry", picker),
    ),
    actions: [button("Cancel", () => handle.close()), save],
  });
}

function changeMasterPassword(index: number, tab: TabItem): void {
  const current = passwordBox({ placeholder: "Current master password" });
  const next = passwordBox({ placeholder: "New master password" });
  const repeat = passwordBox({ placeholder: "New master password again" });
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });
  let busy = false;
  const submit = async () => {
    if (busy) return;
    error.textContent = "";
    if (!current.value) return void (error.textContent = "Type the current password.");
    if (!next.value) return void (error.textContent = "Type the new password.");
    if (next.value !== repeat.value) return void (error.textContent = "The two new passwords are different.");
    busy = true;
    ok.disabled = true;
    ok.textContent = "Saving…";
    try {
      // The command works on the active database; make sure that is this one.
      if (view?.activeTab !== index) await api.selectTab(index);
      nav.main(await api.changeMasterPassword(current.value, next.value));
      handle.close();
      toast(`Master password of "${tab.name}" changed.`);
    } catch (err) {
      error.textContent = errorText(err);
      ok.textContent = "Change password";
    } finally {
      busy = false;
      ok.disabled = false;
    }
  };
  for (const input of [current, next, repeat]) {
    input.addEventListener("keydown", (event: KeyboardEvent) => {
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        void submit();
      }
    });
  }
  const ok = button("Change password", () => void submit(), { appearance: "primary" });
  const handle = openDialog({
    title: "Change master password",
    class: "narrow-dialog",
    body: el(
      "div",
      { class: "stack" },
      el("p", { class: "dialog-text muted" }, `For "${tab.name}". A key file used with it stays the same.`),
      field("Current password", current),
      field("New password", next),
      field("New password again", repeat),
      el(
        "p",
        { class: "dialog-text muted small" },
        "Everyone who opens this file needs the new password. Backups made before now still open with the old one. Quick unlock (Windows Hello / Touch ID) is set up again at the next unlock with the new password.",
      ),
      error,
    ),
    actions: [button("Cancel", () => handle.close()), ok],
  });
  current.focus();
}

/** Hue for each folder color name; the CSS turns a hue into light/dark tile shades. */
const COLOR_HUES: Record<FolderColor, number> = {
  red: 4,
  orange: 28,
  yellow: 48,
  lime: 96,
  green: 150,
  teal: 178,
  cyan: 196,
  blue: 220,
  purple: 266,
  pink: 320,
};

/** The user's pick, or one derived from the folder id so it never changes on its own. */
function folderColor(f: GroupItem): FolderColor {
  if (f.color) return f.color;
  let hash = 0;
  for (const ch of f.id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return FOLDER_COLORS[hash % FOLDER_COLORS.length];
}

function pickColor(f: GroupItem): void {
  const current = folderColor(f);
  const swatch = (color: FolderColor) =>
    el(
      "button",
      {
        class: "swatch" + (color === current ? " selected" : ""),
        style: { "--hue": String(COLOR_HUES[color]) },
        attrs: { type: "button", title: color, "aria-label": color, "aria-pressed": String(color === current) },
        on: { click: () => void choose(color) },
      },
    );
  const choose = async (color: FolderColor | null) => {
    handle.close();
    await run(api.setGroupColor(f.id, color));
  };
  const body = el(
    "div",
    { class: "color-picker" },
    el("div", { class: "swatches" }, FOLDER_COLORS.map(swatch)),
    el("div", { class: "muted small" }, f.color ? "" : "This folder uses an automatic color."),
  );
  const handle = openDialog({
    title: `Color for ${f.name}`,
    class: "narrow-dialog",
    body,
    actions: [button("Automatic", () => void choose(null), { appearance: "subtle" }), button("Cancel", () => handle.close())],
  });
}

async function showUnlockPage(): Promise<void> {
  if (!(await leaveDraft())) return;
  try {
    nav.unlock(await api.unlockView());
  } catch (err) {
    toast(errorText(err));
  }
}

async function lockTab(index: number): Promise<void> {
  const tab = view?.tabs[index];
  if (!tab || !(await leaveDraft())) return;
  let next: MainView;
  try {
    next = await api.lockTab(index, false);
  } catch (err) {
    const ok = await confirmDialog({
      title: "Lock without saving?",
      message: `${errorText(err)} Changes to "${tab.name}" since its last successful save will be lost if you lock it now.`,
      confirm: "Lock anyway",
      danger: true,
    });
    if (!ok) return;
    try {
      next = await api.lockTab(index, true);
    } catch (err2) {
      toast(errorText(err2));
      return;
    }
  }
  try {
    if (next.tabs.length === 0) nav.unlock(await api.unlockView());
    else nav.main(next);
  } catch (err) {
    toast(errorText(err));
  }
}

/** New folder inside `parent`, or inside the folder being shown, or at the top level. */
function addGroup(v: MainView, parent?: GroupItem): void {
  const inside = parent ?? (v.filter.kind === "group" ? v.folders.find((f) => sameFilter(v.filter, { kind: "group", id: f.id })) : undefined);
  promptDialog({
    title: "New folder",
    subtitle: inside ? `Inside ${inside.name}` : undefined,
    label: "Name",
    placeholder: "Folder name",
    confirm: "Add folder",
    submit: async (name) => {
      if (!name) return "Give the folder a name first.";
      try {
        nav.main(await api.addGroup(inside?.id ?? null, name));
        return null;
      } catch (err) {
        return errorText(err);
      }
    },
  });
}

function renameGroup(group: GroupItem): void {
  promptDialog({
    title: "Rename folder",
    label: "Name",
    value: group.name,
    confirm: "Rename",
    submit: async (name) => {
      if (!name) return "Give the folder a name first.";
      if (name === group.name) return null;
      try {
        nav.main(await api.renameGroup(group.id, name));
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

async function deleteGroup(v: MainView, group: GroupItem): Promise<void> {
  // `folders` is pre-order, so the subtree is the run of deeper folders right after this one.
  const start = v.folders.indexOf(group);
  let end = start + 1;
  while (end < v.folders.length && v.folders[end].depth > group.depth) end++;
  const subtree = v.folders.slice(start, end);
  const accounts = subtree.reduce((sum, g) => sum + g.entryCount, 0);
  const parts = [accounts ? plural(accounts, "account") : "", subtree.length > 1 ? plural(subtree.length - 1, "folder") : ""];
  const contents = parts.filter(Boolean).join(", ");
  const what = contents ? `"${group.name}" and everything in it (${contents})` : `"${group.name}" (empty)`;
  const permanent = !v.recycleBinEnabled;

  const ok = await confirmDialog(
    permanent
      ? {
          title: "Delete permanently?",
          message: `This database has its Recycle Bin turned off, so ${what} will be deleted for good. This can't be undone.`,
          confirm: "Delete",
          danger: true,
        }
      : {
          title: "Delete folder?",
          message: `${what} will move to the Recycle Bin.`,
          confirm: "Move to Recycle Bin",
        },
  );
  if (!ok) return;
  try {
    nav.main(await api.deleteGroup(group.id));
    toast(permanent ? "Deleted" : "Moved to Recycle Bin");
  } catch (err) {
    toast(errorText(err));
  }
}

async function emptyRecycleBin(v: MainView): Promise<void> {
  const ok = await confirmDialog({
    title: "Empty Recycle Bin?",
    message: `${plural(v.counts.recycleBin, "item")} in the Recycle Bin will be deleted for good. This can't be undone.`,
    confirm: "Empty Recycle Bin",
    danger: true,
  });
  if (!ok) return;
  try {
    nav.main(await api.emptyRecycleBin());
    toast("Recycle Bin emptied");
  } catch (err) {
    toast(errorText(err));
  }
}

/** Tile for a row or the details: the first letter of an account, or a page for a document.
 *  Colored by the entry's folder unless tile colors are off; entries outside any folder stay neutral. */
function avatar(v: MainView, title: string, folder: Id | null, size: "small" | "large", document = false): HTMLElement {
  const letter = (title.match(/[\p{L}\p{N}]/u)?.[0] ?? "?").toUpperCase();
  const f = v.tileColors === "folder" && folder ? v.folders.find((g) => g.id === folder) : undefined;
  return el(
    "div",
    {
      class: `avatar ${size}` + (f ? "" : " neutral"),
      style: f ? { "--hue": String(COLOR_HUES[folderColor(f)]) } : undefined,
      attrs: { "aria-hidden": "true" },
    },
    document ? icon("document") : letter,
  );
}

function entryRow(v: MainView, entry: EntrySummary, selected: boolean): HTMLElement {
  // Inside a folder its name would repeat on every row; everywhere else it says where the account lives.
  const where = v.filter.kind === "group" && !v.search ? null : entry.folder && v.folders.find((f) => f.id === entry.folder)?.name;
  const first = entry.document
    ? entry.attachmentCount > 0
      ? plural(entry.attachmentCount, "file")
      : "Text"
    : entry.username || entry.url;
  const sub = [first, where].filter(Boolean).join(" · ");
  return el(
    "div",
    {
      class: "entry-row" + (selected ? " selected" : "") + (entry.stale ? " stale" : ""),
      dataset: { id: entry.id },
      attrs: { role: "option", "aria-selected": String(selected) },
      on: {
        click: () => {
          // The entry shown already, unless a new entry is being written over it.
          if (selected && !isNewDraft()) return;
          void leaveThen(() => api.selectEntry(entry.id));
        },
      },
    },
    avatar(v, entry.title, entry.folder, "small", entry.document),
    el(
      "div",
      { class: "entry-text" },
      el(
        "div",
        { class: "entry-title-line" },
        el("span", { class: "entry-title" }, entry.title || "(untitled)"),
        entry.stale ? el("fluent-badge", { appearance: "tint", color: "danger", size: "small" }, "not working") : null,
      ),
      el("div", { class: "entry-sub" }, sub),
    ),
  );
}

function emptyText(v: MainView): string {
  if (v.search) return "Nothing matches.";
  switch (v.filter.kind) {
    case "all":
      return "No accounts yet. Add one with + Add.";
    case "notWorking":
      return "Nothing is marked as not working.";
    case "documents":
      return "No documents yet. Use + Add → Document for ID scans, a will, or a long note.";
    case "recycleBin":
      return "The Recycle Bin is empty.";
    case "group":
      return "No accounts in this folder yet.";
  }
}

function renderList(v: MainView): void {
  if (v.entries.length === 0) {
    setChildren(entryList, el("div", { class: "empty-state" }, emptyText(v)));
    return;
  }
  const rows: Child[] = [];
  // Recycle Bin: accounts, then documents (the backend sorts them so), each under a heading.
  const binSections = v.filter.kind === "recycleBin" && !v.search;
  let dividerAdded = false;
  let section: boolean | null = null;
  for (const entry of v.entries) {
    if (binSections && entry.document !== section) {
      section = entry.document;
      const count = v.entries.filter((e) => e.document === entry.document).length;
      rows.push(el("div", { class: "section-label list-section" }, `${entry.document ? "Documents" : "Accounts"} (${count})`));
    } else if (!binSections && entry.stale && !dividerAdded) {
      if (rows.length > 0) rows.push(el("fluent-divider", { class: "stale-divider" }));
      dividerAdded = true;
    }
    rows.push(entryRow(v, entry, entry.id === v.selectedEntry));
  }
  setChildren(entryList, rows);
  entryList.querySelector(".entry-row.selected")?.scrollIntoView({ block: "nearest" });
}

// ---------------------------------------------------------------------------
// Details pane: read mode, and editing in place
// ---------------------------------------------------------------------------

type TextKey =
  | "title"
  | "username"
  | "password"
  | "url"
  | "recoveryPin"
  | "recoveryPhone"
  | "securityQuestions"
  | "notes"
  | "tags"
  | "group"
  | "autotypeSequence"
  | "autotypeWindows";
// Every pane row has a key: a TextKey, "custom@<index>" (read mode) or "custom:<uid>" (draft),
// "files", or "addCustom" (the Add field button).
const TEXT_KEYS: TextKey[] = [
  "title",
  "username",
  "password",
  "url",
  "recoveryPin",
  "recoveryPhone",
  "securityQuestions",
  "notes",
  "tags",
  "group",
  "autotypeSequence",
  "autotypeWindows",
];
const LOGIN_KEYS: TextKey[] = ["username", "password", "url", "recoveryPin", "recoveryPhone", "securityQuestions"];
/** Read mode copies and reveals these through `copy_field` / `reveal_field`. */
const FIELD_NAMES: Partial<Record<TextKey, FieldName>> = {
  username: "username",
  password: "password",
  url: "url",
  recoveryPin: "recoveryPin",
  recoveryPhone: "recoveryPhone",
  securityQuestions: "securityQuestions",
};
const PLACEHOLDERS: Partial<Record<TextKey, string>> = {
  url: "netflix.com",
  tags: "Comma separated",
  autotypeSequence: "{USERNAME}{TAB}{PASSWORD}{ENTER}",
};

interface CustomRow {
  uid: number;
  key: string;
  value: string;
  protected: boolean;
}

/** An entry being edited in the details pane. Nothing reaches the vault until Save. */
interface PaneDraft {
  /** The saved entry as shown when editing began; null for a new entry. */
  detail: EntryDetail | null;
  folders: FolderOption[];
  gen: GenOptions;
  /** Values as saved (blank for a new entry). */
  saved: DraftEntry;
  /** Values in the pane; `custom` lives in `custom` below instead. */
  values: DraftEntry;
  custom: CustomRow[];
  savedCustom: Map<number, CustomField>;
  /** Fields showing an input, with the value each had when it opened (Esc goes back to it). */
  open: Map<string, string | CustomRow>;
  /** Attachments removed on save. */
  removeFiles: Set<string>;
  /** Files picked to attach on save. */
  picked: DraftFiles | null;
  duplicate: Duplicate | null;
  /** Why saving it while the user was away did not happen; no new try until it changes. */
  idleSaveProblem: string | null;
}

let draft: PaneDraft | null = null;
let draftLoading = false;
let draftSaving = false;
/** Last value sent with `set_draft_pending`. */
let pendingSent = false;
/** The pane's DOM shows the draft; re-renders of the page leave it alone. */
let paneShowsDraft = false;
let customUid = 0;
let dupSeq = 0;
let draftBar: HTMLElement | null = null;
let doneButton: HTMLElement | null = null;
let dupSlot: HTMLElement | null = null;
let draftBarNote: HTMLElement | null = null;

/** A draft with changes is saved once the app has had no input for this long, never while
 *  the user is working. */
const IDLE_SAVE_MS = 2 * 60_000;
let lastInput = Date.now();
let idleSaveTimer: number | undefined;
/** Hides the "saved while you were away" note. */
let hideAwayNote: (() => void) | null = null;

/** Input in the main window (any key, click or mouse move). A key or click also puts away
 *  the "saved while you were away" note; a mouse move does not, or it would vanish unread. */
export function noteInput(dismissNote: boolean): void {
  lastInput = Date.now();
  if (dismissNote && hideAwayNote) {
    hideAwayNote();
    hideAwayNote = null;
  }
}

/** Saves the draft without asking when it has changes and the app has been idle for
 *  IDLE_SAVE_MS, like auto-lock does: an entry without a name becomes "Untitled", a duplicate
 *  title is kept as a separate entry. Only a failed write leaves the draft, with the reason in
 *  the unsaved bar. Not while a dialog is open: the user is in the middle of deciding
 *  something. */
async function saveWhileAway(): Promise<void> {
  const dr = draft;
  if (!dr || draftSaving || dr.idleSaveProblem || isOverlayOpen() || Date.now() - lastInput < IDLE_SAVE_MS || !hasPendingChanges()) return;
  const started = lastInput;
  draftSaving = true;
  try {
    const next = await api.draftSave(draftEntry(dr), null, true);
    // Typing that started during the save stays in the pane, to be saved later.
    if (draft !== dr || lastInput !== started) return;
    draft = null;
    // draft_save cleared the backend's flag.
    pendingSent = false;
    nav.main(next);
    const title = next.detail?.title || dr.values.title.trim();
    hideAwayNote = toast(`Saved your changes to ${title} while you were away.`, { label: "OK", run: () => {} }, true);
  } catch (err) {
    dr.idleSaveProblem = `Not saved while you were away: ${errorText(err)}`;
    if (draft === dr && draftBarNote) draftBarNote.textContent = dr.idleSaveProblem;
  } finally {
    draftSaving = false;
  }
}

function isNewDraft(): boolean {
  return draft !== null && draft.saved.id === null;
}

function isDocument(): boolean {
  return draft ? draft.values.document : (view?.detail?.document ?? false);
}

function customRow(key: string): CustomRow | undefined {
  return draft?.custom.find((row) => `custom:${row.uid}` === key);
}

function customChanged(row: CustomRow): boolean {
  const before = draft?.savedCustom.get(row.uid);
  return !before || before.key !== row.key || before.value !== row.value || before.protected !== row.protected;
}

/** A field of an existing entry whose pending value differs from the saved one. */
function fieldChanged(key: string): boolean {
  const dr = draft;
  if (!dr || dr.saved.id === null) return false;
  if (key === "files") return dr.removeFiles.size > 0 || (dr.picked?.files.length ?? 0) > 0;
  if (key.startsWith("custom:")) {
    const row = customRow(key);
    return row !== undefined && customChanged(row);
  }
  return (TEXT_KEYS as string[]).includes(key) && dr.values[key as TextKey] !== dr.saved[key as TextKey];
}

function hasPendingChanges(): boolean {
  const dr = draft;
  if (!dr) return false;
  if (dr.saved.id === null) return true;
  const customRemoved = [...dr.savedCustom.keys()].some((uid) => !dr.custom.some((row) => row.uid === uid));
  return customRemoved || dr.custom.some(customChanged) || TEXT_KEYS.some(fieldChanged) || fieldChanged("files");
}

/** Shows or hides the unsaved bar and the change markers, and tells the backend whether a
 *  window close must ask first. */
function syncPending(): void {
  const pending = hasPendingChanges();
  if (pending !== pendingSent) {
    pendingSent = pending;
    api.setDraftPending(pending).catch((err) => toast(errorText(err)));
  }
  if (draftBar) draftBar.hidden = !pending;
  if (doneButton) doneButton.hidden = pending;
  for (const node of detailPane.querySelectorAll<HTMLElement>("[data-key]")) node.classList.toggle("changed", fieldChanged(node.dataset.key ?? ""));
}

function beginDraft(data: Draft, detail: EntryDetail | null): PaneDraft {
  const custom = data.entry.custom.map((c) => ({ uid: ++customUid, ...c }));
  draft = {
    detail,
    folders: data.folders,
    gen: { ...data.gen, length: Math.min(64, Math.max(8, data.gen.length)) },
    saved: data.entry,
    values: { ...data.entry },
    custom,
    savedCustom: new Map(custom.map((row) => [row.uid, { key: row.key, value: row.value, protected: row.protected }])),
    open: new Map(),
    removeFiles: new Set(),
    picked: null,
    duplicate: null,
    idleSaveProblem: null,
  };
  return draft;
}

/** The draft for the selected entry, started on its first edit. */
async function ensureDraft(): Promise<PaneDraft | null> {
  if (draft) return draft;
  const detail = view?.detail;
  if (!detail || draftLoading) return null;
  draftLoading = true;
  try {
    return beginDraft(await api.draftOpen("edit"), detail);
  } catch (err) {
    toast(errorText(err));
    return null;
  } finally {
    draftLoading = false;
  }
}

function snapshotOf(key: string): string | CustomRow {
  const row = customRow(key);
  return row ? { ...row } : draft!.values[key as TextKey];
}

/** Turns one field into an input (pencil, double-click, F2). The first edit shows every field. */
async function startEdit(key: string): Promise<void> {
  const wasEditing = draft !== null;
  const dr = await ensureDraft();
  if (!dr) return;
  if (key.startsWith("custom@")) {
    const row = dr.custom[Number(key.slice("custom@".length))];
    if (!row) return;
    key = `custom:${row.uid}`;
  }
  if (key === "files" || key === "addCustom") return;
  if (!dr.open.has(key)) dr.open.set(key, snapshotOf(key));
  if (wasEditing) replaceRow(key, "editor");
  else renderPane(key);
}

/** Back to showing the value; the edit stays pending. An abandoned empty custom field goes. */
function closeField(key: string, keepFocus: boolean): void {
  const dr = draft;
  if (!dr || isNewDraft() || !dr.open.has(key)) return;
  dr.open.delete(key);
  const row = customRow(key);
  if (row && !row.key.trim() && !row.value) {
    removeCustom(row);
    return;
  }
  replaceRow(key, keepFocus ? "row" : null);
}

/** Esc: the field goes back to the value it had when its input opened. */
function revertField(key: string): void {
  const dr = draft;
  const before = dr?.open.get(key);
  if (!dr || before === undefined) return;
  if (typeof before === "string") dr.values[key as TextKey] = before;
  else Object.assign(customRow(key) ?? {}, { key: before.key, value: before.value, protected: before.protected });
  if (isNewDraft()) replaceRow(key, "editor");
  else closeField(key, true);
  syncPending();
}

function draftChanged(key: string): void {
  if (draft?.idleSaveProblem) {
    draft.idleSaveProblem = null;
    if (draftBarNote) draftBarNote.textContent = "";
  }
  syncPending();
  if (key === "title" || key === "url") checkDuplicate();
}

function addCustomField(): void {
  const dr = draft;
  if (!dr) return;
  const row: CustomRow = { uid: ++customUid, key: "", value: "", protected: false };
  dr.custom.push(row);
  dr.open.set(`custom:${row.uid}`, { ...row });
  renderPane(`custom:${row.uid}`);
  syncPending();
}

function removeCustom(row: CustomRow): void {
  const dr = draft;
  if (!dr) return;
  const key = `custom:${row.uid}`;
  dr.custom = dr.custom.filter((r) => r !== row);
  dr.open.delete(key);
  detailPane.querySelector(`[data-key="${CSS.escape(key)}"]`)?.remove();
  syncPending();
}

function discardDraft(): void {
  draft = null;
  syncPending();
  renderPane();
}

/** What `draft_save` gets: the pane's values, custom fields with something in them, removals. */
function draftEntry(dr: PaneDraft): DraftEntry {
  return {
    ...dr.values,
    custom: dr.custom.filter((row) => row.key.trim() !== "" || row.value !== "").map((row) => ({ key: row.key, value: row.value, protected: row.protected })),
    removeFiles: [...dr.removeFiles],
  };
}

/** Saves the draft in one backend call (one history version, one file save) and offers Undo. */
async function saveDraft(replace: Id | null = null): Promise<boolean> {
  const dr = draft;
  if (!dr || draftSaving) return false;
  if (!dr.values.title.trim()) {
    toast(`Give the ${dr.values.document ? "document" : "account"} a name first.`);
    void startEdit("title");
    return false;
  }
  const entry = draftEntry(dr);
  draftSaving = true;
  try {
    const next = await api.draftSave(entry, replace);
    draft = null;
    // draft_save cleared the backend's flag.
    pendingSent = false;
    nav.main(next);
    toast("Saved", { label: "Undo", run: () => void undoLastSave() });
    return true;
  } catch (err) {
    toast(errorText(err));
    return false;
  } finally {
    draftSaving = false;
  }
}

async function undoLastSave(): Promise<void> {
  if (draft) {
    toast("Save or discard the changes in the details first.");
    return;
  }
  try {
    nav.main(await api.undoSave());
    toast("Undone");
  } catch (err) {
    toast(errorText(err));
  }
}

function unsavedDialog(): Promise<"save" | "discard" | "stay"> {
  const name = draft?.values.title.trim() || (isDocument() ? "This document" : "This account");
  return new Promise((resolve) => {
    let choice: "save" | "discard" | "stay" = "stay";
    const pick = (c: typeof choice) => () => {
      choice = c;
      handle.close();
    };
    const save = button("Save", pick("save"), { appearance: "primary" });
    save.setAttribute("autofocus", "");
    const handle = openDialog({
      title: "Save your changes?",
      type: "alert",
      body: el("p", { class: "dialog-text" }, `${name} has changes that are not saved yet.`),
      actions: [button("Stay", pick("stay")), button("Discard", pick("discard")), save],
      onClose: () => resolve(choice),
    });
  });
}

/** Before the pane shows something else: unsaved edits get Save / Discard / Stay. True when
 *  the caller may go on. */
async function leaveDraft(): Promise<boolean> {
  if (!draft) return true;
  if (!hasPendingChanges()) {
    discardDraft();
    return true;
  }
  const choice = await unsavedDialog();
  if (choice === "save") return saveDraft();
  if (choice === "discard") discardDraft();
  return choice === "discard";
}

/** Runs a command that changes what the pane shows, once unsaved edits are dealt with. */
async function leaveThen(call: () => Promise<MainView>): Promise<void> {
  if (await leaveDraft()) await run(call());
}

/** The window's close button with unsaved edits (the backend held the close back). */
export async function closeRequested(): Promise<void> {
  if (await leaveDraft()) await attempt(api.quitApp());
}

/** Closes every open dialog; their callers see Cancel / Stay. */
function closeDialogs(): void {
  for (const dialog of document.querySelectorAll("fluent-dialog")) dialog.hide();
}

/** Auto-lock (idle, or the computer locked or slept): unsaved edits in the pane are saved
 *  without asking (a nameless entry as "Untitled", a duplicate title as a separate entry),
 *  dialogs close, every database locks and the unlock page shows. A failed write stays on
 *  screen until dismissed. */
export async function autoLock(): Promise<void> {
  const problems: string[] = [];
  const dr = draft;
  if (dr && hasPendingChanges()) {
    try {
      await api.draftSave(draftEntry(dr), null, true);
    } catch (err) {
      problems.push(`Auto-lock could not save your changes to ${dr.values.title.trim() || "a new entry"}: ${errorText(err)}`);
    }
  }
  history = null;
  draft = null;
  // lock_all clears the backend's flag.
  pendingSent = false;
  closeDialogs();
  try {
    problems.push(...(await api.lockAll()));
    nav.unlock(await api.unlockView());
  } catch (err) {
    problems.push(errorText(err));
  }
  if (problems.length > 0) toast(problems.join(" "), { label: "OK", run: () => {} }, true);
}

/** The backend locked everything without the page (it did not answer `auto-lock` in time). */
export async function autoLocked(): Promise<void> {
  const lost = draft !== null && hasPendingChanges();
  draft = null;
  pendingSent = false;
  closeDialogs();
  try {
    nav.unlock(await api.unlockView());
  } catch (err) {
    toast(errorText(err));
    return;
  }
  if (lost) toast("Auto-lock locked before your changes in the details could be saved.", { label: "OK", run: () => {} }, true);
}

async function startNew(mode: "add" | "addDocument"): Promise<void> {
  if (!view || view.tabs.length === 0) return;
  if (!(await leaveDraft())) return;
  let data: Draft;
  try {
    data = await api.draftOpen(mode);
  } catch (err) {
    toast(errorText(err));
    return;
  }
  const dr = beginDraft(data, null);
  // A new entry shows every field as an input, like a form.
  for (const key of paneKeys()) if (key !== "files" && key !== "addCustom") dr.open.set(key, snapshotOf(key));
  dr.open.set("title", "");
  renderPane("title");
  syncPending();
}

const checkDuplicate = debounce(async () => {
  const dr = draft;
  if (!dr) return;
  const seq = ++dupSeq;
  try {
    const dup = await api.findDuplicate(dr.values.title.trim(), dr.values.url.trim(), dr.saved.id);
    if (seq !== dupSeq || draft !== dr) return;
    dr.duplicate = dup;
  } catch {
    // Duplicate hints are advisory; a failed lookup just shows no hint.
    if (seq !== dupSeq || draft !== dr) return;
    dr.duplicate = null;
  }
  showDuplicate();
}, 200);

function showDuplicate(): void {
  const dup = draft?.duplicate;
  if (!dupSlot) return;
  if (!dup) {
    setChildren(dupSlot);
    return;
  }
  const replace = button("Replace", () => void saveDraft(dup.id), { size: "small" });
  replace.setAttribute("slot", "actions");
  setChildren(
    dupSlot,
    el(
      "fluent-message-bar",
      { intent: "warning", layout: "multiline" },
      `Already have "${dup.title}". Replace it? The old entry will be kept and marked not working.`,
      replace,
    ),
  );
}

function labelOf(key: string): string {
  switch (key as TextKey) {
    case "title":
      return isDocument() ? "Name" : "Site or service";
    case "username":
      return "Username";
    case "password":
      return "Password";
    case "url":
      return "Website";
    case "recoveryPin":
      return "Recovery PIN";
    case "recoveryPhone":
      return "Recovery phone";
    case "securityQuestions":
      return "Security questions";
    case "notes":
      return isDocument() ? "Text" : "Notes";
    case "tags":
      return "Tags";
    case "group":
      return "Folder";
    case "autotypeSequence":
      return "Auto-Type sequence";
    case "autotypeWindows":
      return "Auto-Type window titles";
  }
  return "Custom field";
}

/** A non-protected value as the pane shows it: the draft's, or the saved one. */
function shown(key: TextKey): string {
  if (draft) return draft.values[key];
  const d = view?.detail;
  if (!d) return "";
  switch (key) {
    case "title":
      return d.title;
    case "username":
      return d.username;
    case "url":
      return d.url;
    case "recoveryPhone":
      return d.recoveryPhone;
    case "notes":
      return d.notes;
    case "tags":
      return d.tags.join(", ");
    case "autotypeSequence":
      return d.autotypeSequence;
    case "autotypeWindows":
      return d.autotypeWindows.join("\n");
    default:
      // Protected values are fetched on demand (reveal_field).
      return "";
  }
}

function copyOf(key: TextKey): () => Promise<number> {
  return () => (draft ? api.copyText(draft.values[key]) : api.copyField(FIELD_NAMES[key] as FieldName));
}

function revealOf(key: TextKey): () => Promise<string> {
  return () => (draft ? Promise.resolve(draft.values[key]) : api.revealField(FIELD_NAMES[key] as FieldName));
}

/** Row keys in pane order. Read mode shows what is set; editing shows every field. */
function paneKeys(): string[] {
  const dr = draft;
  if (!dr) {
    const d = view?.detail;
    if (!d) return [];
    const keys: string[] = [];
    if (d.username) keys.push("username");
    if (d.hasPassword) keys.push("password");
    if (d.url) keys.push("url");
    if (d.hasRecoveryPin) keys.push("recoveryPin");
    if (d.recoveryPhone) keys.push("recoveryPhone");
    if (d.hasSecurityQuestions) keys.push("securityQuestions");
    d.custom.forEach((_, i) => keys.push(`custom@${i}`));
    if (d.document || d.notes) keys.push("notes");
    if (d.autotypeSequence) keys.push("autotypeSequence");
    if (d.autotypeWindows.length > 0) keys.push("autotypeWindows");
    if (d.tags.length > 0) keys.push("tags");
    keys.push("files");
    return keys;
  }
  // Documents show login fields only when the entry was an account with some set, and offer
  // neither new custom fields nor Auto-Type.
  const doc = dr.values.document;
  const keys: string[] = LOGIN_KEYS.filter((k) => !doc || dr.saved[k] !== "");
  keys.push(...dr.custom.map((row) => `custom:${row.uid}`));
  if (!doc) keys.push("addCustom");
  keys.push("notes");
  if (!doc) keys.push("autotypeSequence", "autotypeWindows");
  keys.push("group", "tags", "files");
  return keys;
}

function buildRow(key: string): HTMLElement {
  if (key === "title") return titleBlock();
  if (key === "files") return filesCard();
  if (key === "addCustom") {
    return el(
      "div",
      { class: "add-field", dataset: { key } },
      button("Add field", () => addCustomField(), { icon: "add", appearance: "subtle", class: "align-start" }),
    );
  }
  return draft?.open.has(key) ? editorRow(key) : displayRow(key);
}

/** Swaps one row for a fresh one, so the rest of the pane (and a click on its buttons) is kept. */
function replaceRow(key: string, focus: "editor" | "row" | null = null): void {
  const old = detailPane.querySelector(`[data-key="${CSS.escape(key)}"]`);
  if (!old) {
    renderPane(focus === "editor" ? key : undefined);
    return;
  }
  const next = buildRow(key);
  old.replaceWith(next);
  if (focus === "editor") focusIn(next);
  else if (focus === "row") next.focus();
}

/** Focuses the row's input with the caret after the text, as F2 does in a spreadsheet: typing
 *  adds to the value instead of replacing it. */
function focusIn(node: Element | null): void {
  const input = node?.querySelector<HTMLElement>("fluent-text-input, fluent-textarea, fluent-dropdown");
  input?.focus();
  const box = input?.shadowRoot?.querySelector<HTMLInputElement | HTMLTextAreaElement>("input, textarea");
  box?.setSelectionRange(box.value.length, box.value.length);
}

function emptyValue(text = "Not set"): HTMLElement {
  return el("span", { class: "empty-value" }, text);
}

function pencil(key: string): HTMLElement {
  return button(null, () => void startEdit(key), {
    icon: "edit",
    appearance: "subtle",
    size: "small",
    title: "Edit (F2 or double-click)",
    class: "edit-pencil",
  });
}

/** A field card: double-click or its pencil turns it into an input. */
function card(key: string, ...children: Child[]): HTMLElement {
  return el(
    "div",
    {
      class: "field-row" + (fieldChanged(key) ? " changed" : ""),
      dataset: { key },
      attrs: { tabindex: "-1" },
      on: {
        dblclick: (event: MouseEvent) => {
          if (event.target instanceof Element && event.target.closest("fluent-button, a, button")) return;
          void startEdit(key);
        },
      },
    },
    children,
  );
}

function valueCard(key: string, label: string, value: Child, actions: Child[], valueClass = ""): HTMLElement {
  return card(
    key,
    el("div", { class: "field-label" }, label),
    el(
      "div",
      { class: "field-line" },
      el("div", { class: "field-value " + valueClass }, value),
      el("div", { class: "field-actions" }, actions, pencil(key)),
    ),
  );
}

function blockCard(key: string, label: string, actions: Child[], block: Child): HTMLElement {
  return card(
    key,
    el("div", { class: "field-label-line" }, el("div", { class: "field-label" }, label), el("div", { class: "field-actions" }, actions, pencil(key))),
    block,
  );
}

function copyButton(copy: () => Promise<number>, shortcut?: string): HTMLElement {
  return button("Copy", () => void copyAndTell(copy()), {
    icon: "copy",
    appearance: "subtle",
    size: "small",
    title: shortcut ? `Copy (${shortcut})` : undefined,
  });
}

/** A masked value with its Show/Hide and Copy buttons. Hidden again on every re-render. */
function secretValue(reveal: () => Promise<string>, copy: () => Promise<number>, shortcut?: string): { value: HTMLElement; actions: HTMLElement[] } {
  const value = el("span", { class: "secret masked" }, MASK);
  let isShown = false;
  const toggle = button(
    "Show",
    async () => {
      if (isShown) {
        isShown = false;
        value.textContent = MASK;
        value.classList.add("masked");
        setChildren(toggle, icon("eye", "start"), "Show");
        return;
      }
      try {
        const revealed = await reveal();
        isShown = true;
        value.textContent = revealed;
        value.classList.remove("masked");
        setChildren(toggle, icon("eyeOff", "start"), "Hide");
      } catch (err) {
        toast(errorText(err));
      }
    },
    { icon: "eye", appearance: "subtle", size: "small" },
  );
  return { value, actions: [toggle, copyButton(copy, shortcut)] };
}

function secretCard(
  key: string,
  label: string,
  reveal: () => Promise<string>,
  copy: () => Promise<number>,
  opts: { multiline?: boolean; shortcut?: string } = {},
): HTMLElement {
  const secret = secretValue(reveal, copy, opts.shortcut);
  return valueCard(key, label, secret.value, secret.actions, opts.multiline ? "mono multiline" : "mono");
}

function notesBlock(notes: string, mono: boolean): HTMLElement {
  const lines = notes.split(/\r?\n/).map((line) =>
    el(
      "div",
      { class: "note-line" },
      el("span", { class: "note-text" }, line || "\u00a0"),
      line.trim()
        ? button("copy", () => void copyAndTell(api.copyText(line)), { appearance: "transparent", size: "small", class: "note-copy" })
        : null,
    ),
  );
  return el("div", { class: "notes-box" + (mono ? " mono" : "") }, lines);
}

function notesCard(): HTMLElement {
  const notes = shown("notes");
  const mono = view?.notesMonospace ?? false;
  if (!isDocument()) return blockCard("notes", "Notes", [], notes ? notesBlock(notes, mono) : emptyValue());
  // Document text: Markdown, with plain line breaks kept.
  const body = el("div", { class: "markdown" + (mono ? " mono" : "") });
  if (notes.trim()) renderMarkdown(notes, body, (url) => void attempt(api.openUrl(url)));
  else body.append(emptyValue("No text yet."));
  const copy = notes.trim() ? button("Copy text", () => void copyAndTell(api.copyText(notes)), { icon: "copy", appearance: "subtle", size: "small" }) : null;
  return blockCard("notes", "Text", [copy], body);
}

function displayRow(key: string): HTMLElement {
  if (key.startsWith("custom")) return customCard(key);
  const k = key as TextKey;
  const text = shown(k);
  const d = view?.detail;
  // Read mode only knows whether a protected value is set; a draft has the value.
  const isSet = (flag: boolean | undefined) => (draft ? draft.values[k] !== "" : (flag ?? false));
  switch (k) {
    case "password":
      return isSet(d?.hasPassword) ? secretCard(k, "Password", revealOf(k), copyOf(k), { shortcut: "Ctrl+C" }) : valueCard(k, "Password", emptyValue(), []);
    case "recoveryPin":
      return isSet(d?.hasRecoveryPin) ? secretCard(k, "Recovery PIN", revealOf(k), copyOf(k)) : valueCard(k, "Recovery PIN", emptyValue(), []);
    case "securityQuestions":
      return isSet(d?.hasSecurityQuestions)
        ? secretCard(k, "Security questions", revealOf(k), copyOf(k), { multiline: true })
        : valueCard(k, "Security questions", emptyValue(), []);
    case "url":
      if (!text) return valueCard(k, "Website", emptyValue(), []);
      return valueCard(
        k,
        "Website",
        el(
          "a",
          {
            class: "link",
            href: "#",
            on: {
              click: (event: MouseEvent) => {
                event.preventDefault();
                void attempt(api.openUrl(text));
              },
            },
          },
          text,
        ),
        [button("Open", () => void attempt(api.openUrl(text)), { icon: "open", appearance: "subtle", size: "small" }), copyButton(copyOf(k))],
      );
    case "notes":
      return notesCard();
    case "tags": {
      const tags = text
        .split(/[,;]/)
        .map((t) => t.trim())
        .filter(Boolean);
      return blockCard(
        k,
        "Tags",
        [],
        tags.length > 0 ? el("div", { class: "tags" }, tags.map((tag) => el("fluent-badge", { appearance: "tint", color: "brand" }, tag))) : emptyValue(),
      );
    }
    case "group":
      return valueCard(k, "Folder", draft?.folders.find((f) => f.id === text)?.name ?? "", []);
    case "autotypeSequence":
      return valueCard(k, labelOf(k), text || emptyValue("Folder or Settings default"), [], "mono");
    case "autotypeWindows":
      return valueCard(k, labelOf(k), text || emptyValue("Any window"), [], "mono multiline");
    default:
      return valueCard(k, labelOf(k), text || emptyValue(), text ? [copyButton(copyOf(k), k === "username" ? "Ctrl+B" : undefined)] : []);
  }
}

function customCard(key: string): HTMLElement {
  const row = customRow(key);
  if (row) {
    const label = row.key.trim() || "New field";
    if (row.protected && row.value) return secretCard(key, label, () => Promise.resolve(row.value), () => api.copyText(row.value));
    return valueCard(key, label, row.value || emptyValue(), row.value ? [copyButton(() => api.copyText(row.value))] : [], "multiline");
  }
  const c = view?.detail?.custom[Number(key.slice("custom@".length))];
  if (!c) return el("div", { dataset: { key } });
  const name: FieldName = `custom:${c.key}`;
  if (c.protected) return secretCard(key, c.key, () => api.revealField(name), () => api.copyField(name));
  return valueCard(key, c.key, c.value ?? "", [copyButton(() => api.copyField(name))], "multiline");
}

function textInput(value: string, opts: { placeholder?: string; class?: string } = {}): TextInputEl {
  return el("fluent-text-input", { class: opts.class, value, placeholder: opts.placeholder ?? "", attrs: { autocomplete: "off" } });
}

function textArea(value: string, opts: { placeholder?: string; class?: string; plain?: boolean } = {}): TextAreaEl {
  return el("fluent-textarea", {
    class: opts.class,
    value,
    placeholder: opts.placeholder ?? "",
    resize: "vertical",
    attrs: { spellcheck: opts.plain ? "false" : undefined, autocomplete: opts.plain ? "off" : undefined },
  });
}

/** Keeps a draft value in step with its input. */
function bind(input: TextInputEl | TextAreaEl, key: TextKey): void {
  const dr = draft!;
  input.addEventListener("input", () => {
    dr.values[key] = input.value;
    draftChanged(key);
  });
}

function checkbox(label: string, checked: boolean, onChange: (checked: boolean) => void): HTMLElement {
  const box = el("fluent-checkbox", { attrs: { checked }, on: { change: () => onChange(box.checked) } });
  return field(label, box, { position: "after" });
}

/** Password input with Regenerate, Copy and the generator (length, digits, symbols). */
function passwordEditor(dr: PaneDraft): HTMLElement {
  const box = passwordBox({ value: dr.values.password, large: true });
  bind(box, "password");
  let genSeq = 0;
  const regenerate = async () => {
    const seq = ++genSeq;
    try {
      const next = await api.generate({ ...dr.gen });
      if (seq !== genSeq || draft !== dr) return;
      box.value = next;
      dr.values.password = next;
      draftChanged("password");
    } catch (err) {
      if (seq === genSeq) toast(errorText(err));
    }
  };
  const lengthLabel = el("span", { class: "slider-value" }, String(dr.gen.length));
  const slider = el("fluent-slider", {
    class: "length-slider",
    attrs: { min: "8", max: "64", step: "1", value: String(dr.gen.length), "aria-label": "Password length" },
    on: {
      change: () => {
        const length = Number(slider.value);
        if (!Number.isFinite(length) || length === dr.gen.length) return;
        dr.gen.length = length;
        lengthLabel.textContent = String(length);
        void regenerate();
      },
    },
  });
  return el(
    "div",
    { class: "password-block" },
    box,
    el(
      "div",
      { class: "password-actions" },
      button("Regenerate", () => void regenerate(), { icon: "refresh", size: "small" }),
      button("Copy", () => void copyAndTell(api.copyText(box.value)), { icon: "copy", size: "small" }),
    ),
    el(
      "div",
      { class: "generator" },
      el("div", { class: "slider-line" }, el("span", { class: "generator-label" }, "Length"), slider, lengthLabel),
      el(
        "div",
        { class: "checkbox-line" },
        checkbox("Digits", dr.gen.digits, (on) => {
          dr.gen.digits = on;
          void regenerate();
        }),
        checkbox("Symbols", dr.gen.symbols, (on) => {
          dr.gen.symbols = on;
          void regenerate();
        }),
      ),
    ),
  );
}

function folderPicker(dr: PaneDraft): HTMLElement {
  const picker = el(
    "fluent-dropdown",
    {
      class: "group-dropdown",
      on: {
        change: () => {
          if (!picker.value) return;
          dr.values.group = picker.value;
          draftChanged("group");
        },
      },
    },
    el(
      "fluent-listbox",
      null,
      dr.folders.map((g) =>
        el(
          "fluent-option",
          {
            class: g.isRecycleBin ? "bin" : undefined,
            style: { "padding-inline-start": `${8 + g.depth * 16}px` },
            attrs: { value: g.id, selected: g.id === dr.values.group },
          },
          g.name,
        ),
      ),
    ),
  );
  picker.setAttribute("aria-label", "Folder");
  return picker;
}

function customEditor(row: CustomRow): HTMLElement {
  const name = textInput(row.key, { placeholder: "Name" });
  const value = passwordBox({ value: row.value, placeholder: "Value" });
  setSecret(value, row.protected);
  const prot = el("fluent-checkbox", { attrs: { checked: row.protected } });
  const key = `custom:${row.uid}`;
  name.addEventListener("input", () => {
    row.key = name.value;
    draftChanged(key);
  });
  value.addEventListener("input", () => {
    row.value = value.value;
    draftChanged(key);
  });
  prot.addEventListener("change", () => {
    row.protected = prot.checked;
    setSecret(value, prot.checked);
    draftChanged(key);
  });
  return el(
    "div",
    { class: "custom-row" },
    name,
    value,
    field("Protected", prot, { position: "after" }),
    button(null, () => removeCustom(row), { icon: "dismiss", appearance: "subtle", title: "Remove field" }),
  );
}

function textEditor(dr: PaneDraft, key: TextKey): HTMLElement {
  const mono = view?.notesMonospace ?? false;
  let input: TextInputEl | TextAreaEl;
  switch (key) {
    case "password":
      return passwordEditor(dr);
    case "group":
      return folderPicker(dr);
    case "recoveryPin":
      input = passwordBox({ value: dr.values.recoveryPin });
      break;
    case "notes":
      input = textArea(dr.values.notes, {
        class: [mono ? "mono" : "", dr.values.document ? "doc-text" : ""].join(" ").trim() || undefined,
        placeholder: dr.values.document ? "Plain text or Markdown: # headings, **bold**, - lists, [links](https://…)" : "",
      });
      break;
    case "securityQuestions":
      input = textArea(dr.values.securityQuestions, { plain: true });
      break;
    case "autotypeWindows":
      input = textArea(dr.values.autotypeWindows, { class: "mono", placeholder: "One window title pattern per line", plain: true });
      break;
    default:
      input = textInput(dr.values[key], {
        placeholder: key === "title" ? (dr.values.document ? "e.g. Passport" : "e.g. Netflix") : PLACEHOLDERS[key],
        class: key === "title" ? "title-input" : key === "autotypeSequence" ? "mono" : undefined,
      });
      if (key === "autotypeSequence") input.setAttribute("spellcheck", "false");
  }
  input.setAttribute("aria-label", labelOf(key));
  bind(input, key);
  return input;
}

/** Esc reverts the field; Enter closes a one-line field; leaving the row closes it. A new entry
 *  keeps every input open until Save or Discard. */
function wireEditor(row: HTMLElement, key: string): void {
  row.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      revertField(key);
    } else if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      // Text inputs take one line; text areas keep Enter for a new line.
      event.target instanceof HTMLElement &&
      event.target.tagName === "FLUENT-TEXT-INPUT" &&
      !isNewDraft()
    ) {
      event.preventDefault();
      closeField(key, true);
    }
  });
  if (isNewDraft()) return;
  row.addEventListener("focusout", () => {
    // After the focus has landed; a window that lost the focus keeps its input open. The pane
    // may have been redrawn meanwhile, so look the field up again.
    window.setTimeout(() => {
      if (!document.hasFocus() || detailPane.querySelector(`[data-key="${CSS.escape(key)}"]`)?.contains(document.activeElement)) return;
      closeField(key, false);
    }, 0);
  });
}

function editorRow(key: string): HTMLElement {
  const dr = draft!;
  const row = customRow(key);
  if (key === "title") {
    const wrap = el("div", { class: "detail-title-edit" + (fieldChanged(key) ? " changed" : ""), dataset: { key } }, textEditor(dr, "title"));
    wireEditor(wrap, key);
    return wrap;
  }
  const node = el(
    "div",
    { class: "field-row editing" + (fieldChanged(key) ? " changed" : ""), dataset: { key }, attrs: { tabindex: "-1" } },
    el("div", { class: "field-label" }, labelOf(key)),
    row ? customEditor(row) : textEditor(dr, key as TextKey),
  );
  wireEditor(node, key);
  return node;
}

let titleClickTimer: number | undefined;

function titleBlock(): HTMLElement {
  if (draft?.open.has("title")) return editorRow("title");
  const title = shown("title");
  const modified = draft ? draft.detail?.modified : view?.detail?.modified;
  // A click on the title of an entry with earlier versions opens its history; a double-click
  // edits it, so the click waits to see whether a second one follows.
  const versions = draft ? 0 : (view?.detail?.historyCount ?? 0);
  const tip = modified ? `Changed ${modified}` + (versions > 0 ? ` · click for its history (${plural(versions, "earlier version")})` : "") : undefined;
  return el(
    "div",
    {
      class: "detail-title-line" + (fieldChanged("title") ? " changed" : "") + (versions > 0 ? " has-history" : ""),
      dataset: { key: "title" },
      attrs: { tabindex: "-1" },
      on: {
        click: (event: MouseEvent) => {
          if (versions === 0 || event.detail !== 1 || (event.target instanceof Element && event.target.closest("fluent-button"))) return;
          titleClickTimer = window.setTimeout(() => void openHistory(), 300);
        },
        dblclick: () => {
          window.clearTimeout(titleClickTimer);
          void startEdit("title");
        },
      },
    },
    el("h2", { class: "detail-title", attrs: { title: tip } }, title || "(untitled)"),
    pencil("title"),
  );
}

/** "Save as…" for a file of the entry, or of its history version `version`. */
async function saveAttachment(name: string, version: number | null = null): Promise<void> {
  try {
    const saved = await api.saveAttachment(name, version);
    if (saved) toast("Saved");
  } catch (err) {
    toast(errorText(err));
  }
}

/** Remove (or keep after all) a saved file; it goes when the draft is saved. */
async function toggleRemoveFile(name: string): Promise<void> {
  const fresh = draft === null;
  const dr = await ensureDraft();
  if (!dr) return;
  if (dr.removeFiles.has(name)) dr.removeFiles.delete(name);
  else dr.removeFiles.add(name);
  if (fresh) renderPane();
  else replaceRow("files");
  syncPending();
}

/** "Add file…": pick, warn about the sizes when big, keep the files to attach on save. */
async function addFiles(): Promise<void> {
  const fresh = draft === null;
  const dr = await ensureDraft();
  if (!dr) return;
  const before = dr.picked?.files.length ?? 0;
  let files: DraftFiles;
  try {
    files = await api.draftPickFiles();
  } catch (err) {
    toast(errorText(err));
    return;
  }
  if (draft !== dr) return;
  const added = files.files.slice(before);
  if (added.length === 0) {
    // Picker cancelled: a draft started just for it goes again.
    if (fresh && !hasPendingChanges()) discardDraft();
    return;
  }
  const sum = (list: AttachmentInfo[]) => list.reduce((total, f) => total + f.size, 0);
  const removed = (dr.detail?.attachments ?? []).filter((a) => dr.removeFiles.has(a.name));
  const now = files.attachmentsBytes - sum(removed) + sum(files.files.slice(0, before));
  const after = now + sum(added);
  const big = added.filter((f) => f.size > LARGE_FILE);
  if (big.length > 0 || after > LARGE_TOTAL) {
    const reasons = [
      big.map((f) => `"${f.name}" is ${formatBytes(f.size)}.`).join(" "),
      `All files in this database: ${formatBytes(now)}, ${formatBytes(after)} with ${added.length === 1 ? "this one" : "these"}. Database file: ${formatBytes(files.fileBytes)}.`,
      `Every change saves the whole database again, and the backup folder keeps up to ${BACKUP_COPIES} copies of it (about ${formatBytes((files.fileBytes + sum(added)) * BACKUP_COPIES)}).`,
      after > LARGE_TOTAL ? "For many or large documents, a separate database (for example Documents.kdbx) keeps your passwords quick to save." : "",
      "If it's a photo of a paper, scanning it as a PDF usually makes it much smaller.",
    ];
    const ok = await confirmDialog({
      title: added.length === 1 ? "Add a large file?" : "Add large files?",
      message: reasons.filter(Boolean).join(" "),
      confirm: "Add anyway",
    });
    if (!ok) {
      try {
        for (let i = files.files.length - 1; i >= before; i--) files = await api.draftRemoveFile(i);
      } catch (err) {
        toast(errorText(err));
      }
    }
  }
  if (draft !== dr) return;
  dr.picked = files;
  if (fresh) renderPane();
  else replaceRow("files");
  syncPending();
}

async function unpickFile(index: number): Promise<void> {
  const dr = draft;
  if (!dr) return;
  try {
    dr.picked = await api.draftRemoveFile(index);
  } catch (err) {
    toast(errorText(err));
    return;
  }
  replaceRow("files");
  syncPending();
}

function filesCard(): HTMLElement {
  const dr = draft;
  const saved = dr ? (dr.detail?.attachments ?? []) : (view?.detail?.attachments ?? []);
  const picked = dr?.picked?.files ?? [];
  const kept = saved.filter((a) => !dr?.removeFiles.has(a.name));
  const bytes = [...kept, ...picked].reduce((sum, a) => sum + a.size, 0);
  const rows: Child[] = [
    ...saved.map((a) => {
      const removed = dr?.removeFiles.has(a.name) ?? false;
      return el(
        "div",
        { class: "attachment-row" + (removed ? " removed" : "") },
        el("span", { class: "attachment-name", attrs: { title: a.name } }, a.name),
        removed ? el("span", { class: "attachment-note" }, "removed on save") : null,
        el("span", { class: "attachment-size" }, formatBytes(a.size)),
        el(
          "div",
          { class: "attachment-actions" },
          button("Open", () => void attempt(api.openAttachment(a.name)), {
            icon: "open",
            appearance: "subtle",
            size: "small",
            title: "Open a temporary copy in its usual app. Copies are deleted when you lock or close Our Logins.",
          }),
          button("Save as…", () => void saveAttachment(a.name), { appearance: "subtle", size: "small" }),
          button(removed ? "Keep" : "Remove", () => void toggleRemoveFile(a.name), { appearance: "subtle", size: "small" }),
        ),
      );
    }),
    ...picked.map((f, index) =>
      el(
        "div",
        { class: "attachment-row added" },
        el("span", { class: "attachment-name", attrs: { title: f.name } }, f.name),
        el("span", { class: "attachment-note" }, "added on save"),
        el("span", { class: "attachment-size" }, formatBytes(f.size)),
        button(null, () => void unpickFile(index), { icon: "dismiss", appearance: "subtle", size: "small", title: `Don't add ${f.name}` }),
      ),
    ),
  ];
  return el(
    "div",
    { class: "field-row" + (fieldChanged("files") ? " changed" : ""), dataset: { key: "files" } },
    el(
      "div",
      { class: "field-label-line" },
      el("div", { class: "field-label" }, kept.length + picked.length > 0 ? `Files · ${formatBytes(bytes)}` : "Files"),
      button("Add file…", () => void addFiles(), { icon: "add", appearance: "subtle", size: "small" }),
    ),
    rows.length > 0 ? rows : el("div", { class: "muted small" }, isDocument() ? "No files yet. " + FILE_ADVICE : "No files attached."),
  );
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
      d.document
        ? null
        : el("fluent-menu-item", { on: { click: () => void run(api.toggleStale()) } }, d.stale ? "Mark working" : "Mark not working"),
      el(
        "fluent-menu-item",
        { on: { click: () => void run(api.setDocument(!d.document)) } },
        d.document ? "Show as an account (login fields)" : "Make this a document (text and files)",
      ),
      el("fluent-menu-item", { attrs: { disabled: d.historyCount === 0 }, on: { click: () => void openHistory() } }, `History (${d.historyCount})`),
      el("fluent-menu-item", { class: "danger-item", on: { click: () => void deleteEntry(d, permanent) } }, permanent ? "Delete permanently" : "Delete"),
    ),
  );
}

// ---------------------------------------------------------------------------
// Entry history: earlier versions, read-only, with Restore and Delete
// ---------------------------------------------------------------------------

interface HistoryView {
  entryId: Id;
  versions: VersionItem[];
  /** Index of the version shown (0 = the newest earlier one). */
  selected: number;
  detail: VersionDetail | null;
}

/** The pane shows the selected entry's history instead of the entry. */
let history: HistoryView | null = null;

async function openHistory(): Promise<void> {
  if (!(await leaveDraft())) return;
  const id = view?.detail?.id;
  if (!id) return;
  try {
    const versions = await api.entryHistory();
    if (versions.length === 0) {
      toast("This entry has no earlier versions.");
      return;
    }
    history = { entryId: id, versions, selected: 0, detail: await api.entryVersion(0) };
  } catch (err) {
    toast(errorText(err));
    return;
  }
  renderPane();
}

function closeHistory(): void {
  history = null;
  renderPane();
}

async function selectVersion(index: number): Promise<void> {
  const h = history;
  if (!h || h.selected === index) return;
  try {
    const detail = await api.entryVersion(index);
    if (history !== h) return;
    h.selected = index;
    h.detail = detail;
  } catch (err) {
    toast(errorText(err));
    return;
  }
  renderPane();
}

/** After a version was deleted: the list again, or the entry when none is left. */
async function reloadHistory(h: HistoryView): Promise<void> {
  try {
    const versions = await api.entryHistory();
    if (history !== h) return;
    if (versions.length === 0) {
      history = null;
    } else {
      h.versions = versions;
      h.selected = Math.min(h.selected, versions.length - 1);
      h.detail = await api.entryVersion(h.selected);
    }
  } catch (err) {
    toast(errorText(err));
    history = null;
  }
  renderPane();
}

async function restoreVersion(h: HistoryView): Promise<void> {
  try {
    const next = await api.restoreVersion(h.selected);
    history = null;
    nav.main(next);
    toast("Restored", { label: "Undo", run: () => void undoLastSave() });
  } catch (err) {
    toast(errorText(err));
  }
}

async function deleteVersion(h: HistoryView): Promise<void> {
  const item = h.versions[h.selected];
  const ok = await confirmDialog({
    title: "Delete this version?",
    message: `The version from ${item?.modified ?? "then"} will be deleted for good, with any file only it had. The entry itself stays as it is.`,
    confirm: "Delete version",
    danger: true,
  });
  if (!ok || history !== h) return;
  try {
    nav.main(await api.deleteVersion(h.selected));
  } catch (err) {
    toast(errorText(err));
    return;
  }
  await reloadHistory(h);
}

/** A read-only field of a version; `differs` marks a value other than the current one. */
function versionCard(label: string, value: Child, actions: Child[], differs: boolean, valueClass = ""): HTMLElement {
  return el(
    "div",
    { class: "field-row" + (differs ? " differs" : "") },
    el("div", { class: "field-label" }, label),
    el("div", { class: "field-line" }, el("div", { class: "field-value " + valueClass }, value), el("div", { class: "field-actions" }, actions)),
  );
}

function versionRows(d: VersionDetail): HTMLElement[] {
  const index = d.index;
  const differs = (key: string) => d.differs.includes(key);
  const shownIf = (key: string, set: boolean) => set || differs(key);
  const copy = (field: FieldName) => copyButton(() => api.copyField(field, index));
  const secret = (key: FieldName, label: string, set: boolean, multiline = false) => {
    if (!set) return versionCard(label, emptyValue(), [], differs(key));
    const s = secretValue(() => api.revealField(key, index), () => api.copyField(key, index));
    return versionCard(label, s.value, s.actions, differs(key), multiline ? "mono multiline" : "mono");
  };
  const rows: HTMLElement[] = [versionCard(d.document ? "Name" : "Site or service", d.title || emptyValue(), [], differs("title"))];
  if (shownIf("username", d.username !== "")) rows.push(versionCard("Username", d.username || emptyValue(), d.username ? [copy("username")] : [], differs("username")));
  if (shownIf("password", d.hasPassword)) rows.push(secret("password", "Password", d.hasPassword));
  if (shownIf("url", d.url !== "")) rows.push(versionCard("Website", d.url || emptyValue(), d.url ? [copy("url")] : [], differs("url")));
  if (shownIf("recoveryPin", d.hasRecoveryPin)) rows.push(secret("recoveryPin", "Recovery PIN", d.hasRecoveryPin));
  if (shownIf("recoveryPhone", d.recoveryPhone !== "")) {
    rows.push(versionCard("Recovery phone", d.recoveryPhone || emptyValue(), d.recoveryPhone ? [copy("recoveryPhone")] : [], differs("recoveryPhone")));
  }
  if (shownIf("securityQuestions", d.hasSecurityQuestions)) rows.push(secret("securityQuestions", "Security questions", d.hasSecurityQuestions, true));
  for (const c of d.custom) {
    const field: FieldName = `custom:${c.key}`;
    rows.push(c.protected ? secret(field, c.key, true) : versionCard(c.key, c.value ?? "", [copy(field)], differs(field), "multiline"));
  }
  // Custom fields the current version has and this one had not.
  for (const key of d.differs.filter((k) => k.startsWith("custom:") && !d.custom.some((c) => `custom:${c.key}` === k))) {
    rows.push(versionCard(key.slice("custom:".length), emptyValue("Not in this version"), [], true));
  }
  if (shownIf("notes", d.notes !== "")) {
    const mono = view?.notesMonospace ?? false;
    let body: HTMLElement;
    if (!d.notes) {
      body = emptyValue();
    } else if (d.document) {
      body = el("div", { class: "markdown" + (mono ? " mono" : "") });
      renderMarkdown(d.notes, body, (url) => void attempt(api.openUrl(url)));
    } else {
      body = notesBlock(d.notes, mono);
    }
    rows.push(
      el(
        "div",
        { class: "field-row" + (differs("notes") ? " differs" : "") },
        el(
          "div",
          { class: "field-label-line" },
          el("div", { class: "field-label" }, d.document ? "Text" : "Notes"),
          d.notes ? button("Copy text", () => void copyAndTell(api.copyText(d.notes)), { icon: "copy", appearance: "subtle", size: "small" }) : null,
        ),
        body,
      ),
    );
  }
  if (shownIf("autotypeSequence", d.autotypeSequence !== "")) {
    rows.push(versionCard("Auto-Type sequence", d.autotypeSequence || emptyValue("Folder or Settings default"), [], differs("autotypeSequence"), "mono"));
  }
  if (shownIf("autotypeWindows", d.autotypeWindows.length > 0)) {
    rows.push(versionCard("Auto-Type window titles", d.autotypeWindows.join("\n") || emptyValue("Any window"), [], differs("autotypeWindows"), "mono multiline"));
  }
  if (shownIf("stale", d.stale)) rows.push(versionCard("Status", d.stale ? "Not working" : "Working", [], differs("stale")));
  if (shownIf("tags", d.tags.length > 0)) {
    const tags = d.tags.length > 0 ? el("div", { class: "tags" }, d.tags.map((tag) => el("fluent-badge", { appearance: "tint", color: "brand" }, tag))) : emptyValue();
    rows.push(versionCard("Tags", tags, [], differs("tags")));
  }
  if (shownIf("files", d.attachments.length > 0)) {
    rows.push(
      el(
        "div",
        { class: "field-row" + (differs("files") ? " differs" : "") },
        el("div", { class: "field-label" }, "Files"),
        d.attachments.length === 0
          ? emptyValue("No files")
          : d.attachments.map((a) =>
              el(
                "div",
                { class: "attachment-row" },
                el("span", { class: "attachment-name", attrs: { title: a.name } }, a.name),
                el("span", { class: "attachment-size" }, formatBytes(a.size)),
                el(
                  "div",
                  { class: "attachment-actions" },
                  button("Open", () => void attempt(api.openAttachment(a.name, index)), { icon: "open", appearance: "subtle", size: "small" }),
                  button("Save as…", () => void saveAttachment(a.name, index), { appearance: "subtle", size: "small" }),
                ),
              ),
            ),
      ),
    );
  }
  return rows;
}

function renderHistory(v: MainView, h: HistoryView): void {
  const current = v.detail;
  const d = h.detail;
  const list = el(
    "div",
    { class: "history-list", attrs: { role: "listbox", "aria-label": "Earlier versions" } },
    h.versions.map((item) =>
      el(
        "button",
        {
          class: "history-item" + (item.index === h.selected ? " selected" : ""),
          attrs: { type: "button", role: "option", "aria-selected": String(item.index === h.selected) },
          on: { click: () => void selectVersion(item.index) },
        },
        el("span", { class: "history-date" }, item.modified),
        el("span", { class: "history-changes" }, item.changes.join("; ")),
      ),
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
        avatar(v, current?.title ?? "", current?.folder ?? null, "large", current?.document ?? false),
        el(
          "div",
          { class: "detail-title-block" },
          el("div", { class: "history-heading" }, `History of ${current?.title || "(untitled)"}`),
          el("h2", { class: "detail-title" }, d ? `Version from ${d.modified}` : "Earlier versions"),
        ),
        el(
          "div",
          { class: "detail-actions" },
          button("Restore this version", () => void restoreVersion(h), {
            appearance: "primary",
            title: "Make this version the current one. The current one is kept in the history.",
          }),
          button("Delete version", () => void deleteVersion(h)),
          button("Close", () => closeHistory(), { title: "Back to the entry (Esc)" }),
        ),
      ),
      el("div", { class: "history-split" }, list, el("div", { class: "fields history-version" }, d ? versionRows(d) : null)),
    ),
  );
}

/** "+ Add" with a choice: an account (Ctrl+N) or a document (Ctrl+Shift+N). */
function addMenu(): HTMLElement {
  const trigger = button("Add", () => {}, { appearance: "primary", icon: "add", title: "Add an account (Ctrl+N) or a document (Ctrl+Shift+N)" });
  trigger.setAttribute("slot", "trigger");
  return el(
    "fluent-menu",
    null,
    trigger,
    el(
      "fluent-menu-list",
      null,
      el("fluent-menu-item", { on: { click: () => void startNew("add") } }, "Account"),
      el("fluent-menu-item", { on: { click: () => void startNew("addDocument") } }, "Document"),
    ),
  );
}

async function openAbout(): Promise<void> {
  let info;
  try {
    info = await api.aboutInfo();
  } catch (err) {
    toast(errorText(err));
    return;
  }
  const repo = info.repository;
  const goals = [
    "Record a new account in under ten seconds, and flag old ones as not working instead of losing them.",
    "Keep the family's passwords and important documents in standard KeePass files that KeePass 2, KeePassXC and phone apps still open.",
    "Never lose an edit: save on every change, with rolling backups away from the database drive.",
    "Type logins into other apps with one hotkey (Auto-Type and Quick Access).",
    "Stay small and local: no cloud, no account. The only network use is checking GitHub for a newer version.",
  ];
  const handle = openDialog({
    title: "About Our Logins",
    class: "about-dialog",
    body: el(
      "div",
      { class: "stack" },
      el("div", { class: "muted" }, `Version ${info.version}`),
      el("ul", { class: "about-goals" }, goals.map((g) => el("li", null, g))),
      el(
        "div",
        null,
        "Source code: ",
        el(
          "a",
          {
            class: "link",
            href: "#",
            on: {
              click: (event: MouseEvent) => {
                event.preventDefault();
                void attempt(api.openUrl(repo));
              },
            },
          },
          repo,
        ),
      ),
    ),
    actions: [
      button("Check for updates", () => {
        handle.close();
        void checkForUpdates(true);
      }),
      button("Close", () => handle.close(), { appearance: "primary" }),
    ],
  });
}

/** Page re-renders leave a pane that shows a draft alone: its inputs keep their focus. */
function renderDetail(): void {
  if (!(draft && paneShowsDraft)) renderPane();
}

function renderPane(focus?: string): void {
  const v = view;
  if (!v) return;
  const dr = draft;
  paneShowsDraft = dr !== null;
  draftBar = doneButton = dupSlot = draftBarNote = null;
  // The history belongs to the entry it was opened for.
  if (history && (dr || v.detail?.id !== history.entryId)) history = null;
  if (history) {
    renderHistory(v, history);
    return;
  }
  const d = dr ? dr.detail : v.detail;
  if (!dr && !d) {
    setChildren(detailPane, el("div", { class: "detail-scroll" }, el("div", { class: "empty-state" }, "Select an account")));
    return;
  }
  const doc = isDocument();
  const title = shown("title");
  const folder = dr ? (v.folders.some((f) => f.id === dr.values.group) ? dr.values.group : null) : (d?.folder ?? null);
  let actions: Child[];
  if (dr) {
    const saveKey = document.documentElement.dataset.os === "macos" ? "⌘S" : "Ctrl+S";
    draftBar = el(
      "div",
      { class: "draft-bar", attrs: { role: "region", "aria-label": "Unsaved changes" } },
      el("span", { class: "draft-bar-text" }, dr.saved.id ? "Unsaved changes" : `New ${doc ? "document" : "account"}, not saved yet`),
      button(`Save (${saveKey})`, () => void saveDraft(), { appearance: "primary", size: "small" }),
      button("Discard", () => discardDraft(), { appearance: "subtle", size: "small" }),
      (draftBarNote = el("span", { class: "draft-bar-note", attrs: { role: "status" } }, dr.idleSaveProblem ?? "")),
    );
    dupSlot = el("div", { class: "dup-slot" });
    doneButton = button("Done", () => discardDraft(), { title: "Stop editing" });
    actions = [doneButton];
  } else {
    actions = [
      button("Edit", () => void startEdit("title"), { appearance: "primary", title: "Edit (F2)" }),
      d!.document ? null : button("Auto-Type", () => void attempt(api.autotypeSelected())),
      overflowMenu(d!, v),
    ];
  }
  setChildren(
    detailPane,
    draftBar,
    el(
      "div",
      { class: "detail-scroll" },
      el(
        "div",
        { class: "detail-header", attrs: { "data-tauri-drag-region": true } },
        avatar(v, title, folder, "large", doc),
        el(
          "div",
          { class: "detail-title-block" },
          d?.folderPath
            ? el(
                "button",
                {
                  class: "detail-folder",
                  attrs: { type: "button", title: d.folder ? "Show this folder" : undefined, disabled: !d.folder },
                  on: { click: () => d.folder && void leaveThen(() => api.selectFilter({ kind: "group", id: d.folder! })) },
                },
                d.inBin ? icon("delete") : icon("folder"),
                el("span", { class: "detail-folder-text", attrs: { title: d.folderPath } }, d.folderPath),
              )
            : null,
          titleBlock(),
          el("div", { class: "detail-badges" }, d?.stale ? el("fluent-badge", { appearance: "tint", color: "danger" }, "not working") : null),
        ),
        el("div", { class: "detail-actions" }, actions),
      ),
      dupSlot,
      el("div", { class: "fields" }, paneKeys().map(buildRow)),
    ),
  );
  showDuplicate();
  syncPending();
  if (focus) focusIn(detailPane.querySelector(`[data-key="${CSS.escape(focus)}"]`));
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
  const id = view.entries[next].id;
  void leaveThen(() => api.selectEntry(id));
}

/** Search, after unsaved edits are dealt with (Stay puts the old text back). */
async function searchFor(text: string): Promise<void> {
  if (draft && !(await leaveDraft())) {
    searchInput.value = view?.search ?? "";
    return;
  }
  const seq = ++searchSeq;
  try {
    const next = await api.search(text);
    if (seq === searchSeq) nav.main(next);
  } catch (err) {
    toast(errorText(err));
  }
}

/** F2: edit the field that has the focus, or the title. */
function editFocusedField(target: EventTarget | null): void {
  const row = target instanceof Element && detailPane.contains(target) ? target.closest<HTMLElement>("[data-key]") : null;
  const key = row?.dataset.key;
  if (key && key !== "files" && key !== "addCustom") {
    if (!draft?.open.has(key)) void startEdit(key);
    return;
  }
  if (draft || view?.detail) void startEdit("title");
}

/** Page shortcuts: Ctrl+K/F search, Ctrl+N add account (Ctrl+Shift+N document), Ctrl+B/C copy the
 *  username/password and Ctrl+U opens the website (as in KeePass), Up/Down select, Esc clears
 *  search, F2 edits a field, Ctrl+S saves the edits. */
export function mainKeydown(event: KeyboardEvent): void {
  if (!view || isOverlayOpen() || event.isComposing) return;
  const mod = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  if (event.key === "Escape" && history && !isTextTarget(event.target)) {
    event.preventDefault();
    closeHistory();
    return;
  }
  if (mod && !event.altKey && !event.shiftKey && key === "s") {
    event.preventDefault();
    if (draft) void saveDraft();
    return;
  }
  if (event.key === "F2" && !mod && !event.altKey && !event.shiftKey) {
    event.preventDefault();
    editFocusedField(event.target);
    return;
  }
  if (mod && !event.altKey && (key === "k" || key === "f")) {
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
    return;
  }
  if (mod && !event.altKey && key === "n") {
    event.preventDefault();
    // In the Documents list, or with Shift, a new entry is a document.
    void startNew(event.shiftKey || view.filter.kind === "documents" ? "addDocument" : "add");
    return;
  }
  // KeePass shortcuts for the entry shown: Ctrl+B copies the username, Ctrl+C the password,
  // Ctrl+U opens the URL. Text boxes keep their own Ctrl+C when text is selected in them.
  if (mod && !event.altKey && !event.shiftKey && (key === "b" || key === "c" || key === "u") && (draft || view.detail)) {
    const inSearch = event.target === searchInput;
    if (!inSearch && isTextTarget(event.target)) return;
    if (key === "c") {
      const box = inSearch ? searchInput.shadowRoot?.querySelector("input") : null;
      const textSelected = box ? box.selectionStart !== box.selectionEnd : !(document.getSelection()?.isCollapsed ?? true);
      if (textSelected) return;
    }
    event.preventDefault();
    const d = view.detail;
    if (key === "b") {
      if (draft ? draft.values.username : d?.username) void copyAndTell(copyOf("username")());
      else toast("This entry has no username.");
    } else if (key === "c") {
      if (draft ? draft.values.password : d?.hasPassword) void copyAndTell(copyOf("password")());
      else toast("This entry has no password.");
    } else {
      const url = shown("url");
      if (url) void attempt(api.openUrl(url));
      else toast("This entry has no website.");
    }
    return;
  }
  if (event.key === "Escape" && event.target === searchInput) {
    if (searchInput.value) {
      event.preventDefault();
      searchInput.value = "";
      void searchFor("");
    }
    return;
  }
  if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !mod && !event.altKey) {
    if (event.target !== searchInput && isTextTarget(event.target)) return;
    // Let the overflow menu and other popups and controls handle their own arrows.
    if (event.target instanceof HTMLElement && event.target.closest("fluent-menu, fluent-dropdown, fluent-slider")) return;
    event.preventDefault();
    moveSelection(event.key === "ArrowDown" ? 1 : -1);
  }
}
