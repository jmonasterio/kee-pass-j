import {
  api,
  errorText,
  FOLDER_COLORS,
  type EntryDetail,
  type EntrySummary,
  type FieldName,
  type Filter,
  type FolderColor,
  type GroupItem,
  type Id,
  type MainView,
} from "../api";
import { confirmDialog, isOverlayOpen, openDialog, promptDialog } from "../dialogs";
import { button, el, icon, setChildren, type Child } from "../dom";
import { renderMarkdown } from "../markdown";
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
      onSelect: () => void run(api.selectTab(index)),
      title: tab.path,
      extraClass: "tab-row",
      actions: [
        tab.dirty ? el("span", { class: "dirty-dot", attrs: { title: "Not saved" } }) : null,
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

  const show = (filter: Filter) => () => void run(api.selectFilter(filter));
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
    el("div", { class: "section-gap" }),
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
      el("fluent-menu-item", { on: { click: () => pickColor(f) } }, "Colour…"),
      el("fluent-menu-item", { on: { click: () => addGroup(v, f) } }, "New folder inside"),
      el("fluent-menu-item", { class: "danger-item", on: { click: () => void deleteGroup(v, f) } }, "Delete folder…"),
    ),
  );
}

/** Hue for each folder colour name; the CSS turns a hue into light/dark tile shades. */
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
    el("div", { class: "muted small" }, f.color ? "" : "This folder uses an automatic colour."),
  );
  const handle = openDialog({
    title: `Colour for ${f.name}`,
    class: "narrow-dialog",
    body,
    actions: [button("Automatic", () => void choose(null), { appearance: "subtle" }), button("Cancel", () => handle.close())],
  });
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
    message: `${plural(v.counts.recycleBin, "account")} in the Recycle Bin will be deleted for good. This can't be undone.`,
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
 *  Coloured by the entry's folder unless tile colours are off; entries outside any folder stay neutral. */
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
      on: { click: () => void run(api.selectEntry(entry.id)) },
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
  let dividerAdded = false;
  for (const entry of v.entries) {
    if (entry.stale && !dividerAdded) {
      if (rows.length > 0) rows.push(el("fluent-divider", { class: "stale-divider" }));
      dividerAdded = true;
    }
    rows.push(entryRow(v, entry, entry.id === v.selectedEntry));
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

async function removeAttachment(d: EntryDetail, name: string): Promise<void> {
  const ok = await confirmDialog({
    title: "Remove file?",
    message: `"${name}" will be removed from "${d.title || "(untitled)"}".`,
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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** One file above this gets a warning before it is added. */
const LARGE_FILE = 5 * 1024 * 1024;
/** All files together above this get a warning: every save rewrites them, every backup copies them. */
const LARGE_TOTAL = 50 * 1024 * 1024;
/** Full copies the backup folder keeps at most: the newest 20 plus one per day for 30 days. */
const BACKUP_COPIES = 50;

const FILE_ADVICE =
  "Good fits: scans of IDs and passports, a will, insurance papers — a few MB each. Photo collections and videos make every save slow and every backup large; keep those elsewhere.";

/** "Add file…": pick, show the sizes (warn when big), then attach. */
async function addFile(): Promise<void> {
  let pick;
  try {
    pick = await api.pickAttachment();
  } catch (err) {
    toast(errorText(err));
    return;
  }
  if (!pick) return;
  const after = pick.attachmentsBytes + pick.size;
  if (pick.size > LARGE_FILE || after > LARGE_TOTAL) {
    const reasons = [
      pick.size > LARGE_FILE ? `"${pick.name}" is ${formatBytes(pick.size)}.` : "",
      `Files in this database would grow from ${formatBytes(pick.attachmentsBytes)} to ${formatBytes(after)}.`,
      `Every change saves the whole database again, and the backup folder keeps up to ${BACKUP_COPIES} copies of it (about ${formatBytes((pick.fileBytes + pick.size) * BACKUP_COPIES)}).`,
      after > LARGE_TOTAL ? "For many or large documents, a separate database (for example Documents.kdbx) keeps your passwords quick to save." : "",
      "If it's a photo of a paper, scanning it as a PDF usually makes it much smaller.",
    ];
    const ok = await confirmDialog({
      title: "Add a large file?",
      message: reasons.filter(Boolean).join(" "),
      confirm: "Add anyway",
    });
    if (!ok) {
      void api.cancelAttachment();
      return;
    }
  }
  try {
    nav.main(await api.attachPicked());
    toast(`Added ${pick.name} (${formatBytes(pick.size)}). Files in this database: ${formatBytes(after)}.`);
  } catch (err) {
    toast(errorText(err));
  }
}

function attachmentsCard(v: MainView, d: EntryDetail): HTMLElement {
  const entryBytes = d.attachments.reduce((sum, a) => sum + a.size, 0);
  return el(
    "div",
    { class: "field-row" },
    el(
      "div",
      { class: "field-label-line" },
      el("div", { class: "field-label" }, d.attachments.length ? `Files · ${formatBytes(entryBytes)}` : "Files"),
      button("Add file…", () => void addFile(), { icon: "add", appearance: "subtle", size: "small" }),
    ),
    d.attachments.length === 0
      ? el("div", { class: "muted small" }, d.document ? "No files yet. " + FILE_ADVICE : "No files attached.")
      : d.attachments.map((a) =>
          el(
            "div",
            { class: "attachment-row" },
            el("span", { class: "attachment-name", attrs: { title: a.name } }, a.name),
            el("span", { class: "attachment-size" }, formatBytes(a.size)),
            button("Open", () => void attempt(api.openAttachment(a.name)), {
              icon: "open",
              appearance: "subtle",
              size: "small",
              title: "Open a temporary copy in its usual app. Copies are deleted when you lock or close Our Logins.",
            }),
            button("Save as…", () => void saveAttachment(a.name), { appearance: "subtle", size: "small" }),
            button("Remove", () => void removeAttachment(d, a.name), { appearance: "subtle", size: "small" }),
          ),
        ),
    el(
      "div",
      { class: "storage-line muted small" },
      `All files in this database: ${formatBytes(v.storage.attachmentsBytes)} · database file: ${formatBytes(v.storage.fileBytes)}`,
      v.storage.attachmentsBytes > LARGE_TOTAL ? el("span", { class: "storage-warn" }, " · large: consider a separate Documents database") : null,
    ),
  );
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
      el("fluent-menu-item", { class: "danger-item", on: { click: () => void deleteEntry(d, permanent) } }, permanent ? "Delete permanently" : "Delete"),
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
      el("fluent-menu-item", { on: { click: () => void openForm("add", view?.notesMonospace ?? false) } }, "Account"),
      el("fluent-menu-item", { on: { click: () => void openForm("addDocument", view?.notesMonospace ?? false) } }, "Document"),
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
    "Stay small and local: no cloud, no account, no network access.",
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
    actions: [button("Close", () => handle.close(), { appearance: "primary" })],
  });
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
  if (d.document) {
    // Document text: Markdown, with plain line breaks kept.
    const body = el("div", { class: "markdown" + (v.notesMonospace ? " mono" : "") });
    if (d.notes.trim()) renderMarkdown(d.notes, body, (url) => void attempt(api.openUrl(url)));
    else body.append(el("span", { class: "muted" }, "No text. Use Edit to write or paste some."));
    rows.push(
      el(
        "div",
        { class: "field-row" },
        el(
          "div",
          { class: "field-label-line" },
          el("div", { class: "field-label" }, "Text"),
          d.notes.trim() ? button("Copy text", () => void attempt(api.copyText(d.notes), "Copied"), { icon: "copy", appearance: "subtle", size: "small" }) : null,
        ),
        body,
      ),
    );
  } else if (d.notes) {
    rows.push(el("div", { class: "field-row" }, el("div", { class: "field-label" }, "Notes"), notesBlock(d.notes, v.notesMonospace)));
  }

  rows.push(attachmentsCard(v, d));

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
        avatar(v, d.title, d.folder, "large", d.document),
        el(
          "div",
          { class: "detail-title-block" },
          d.folderPath
            ? el(
                "button",
                {
                  class: "detail-folder",
                  attrs: { type: "button", title: d.folder ? "Show this folder" : undefined, disabled: !d.folder },
                  on: { click: () => d.folder && void run(api.selectFilter({ kind: "group", id: d.folder })) },
                },
                d.inBin ? icon("delete") : icon("folder"),
                d.folderPath,
              )
            : null,
          el("h2", { class: "detail-title" }, d.title || "(untitled)"),
          el(
            "div",
            { class: "detail-badges" },
            d.stale ? el("fluent-badge", { appearance: "tint", color: "danger" }, "not working") : null,
          ),
        ),
        el(
          "div",
          { class: "detail-actions" },
          button("Edit", () => void openForm("edit", v.notesMonospace), { appearance: "primary" }),
          d.document ? null : button("Auto-Type", () => void attempt(api.autotypeSelected())),
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

/** Page shortcuts: Ctrl+K/F search, Ctrl+N add account (Ctrl+Shift+N document), Up/Down select, Esc clears search. */
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
    // In the Documents list, or with Shift, a new entry is a document.
    void openForm(event.shiftKey || view.filter.kind === "documents" ? "addDocument" : "add", view.notesMonospace);
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
