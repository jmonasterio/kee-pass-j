// Typed wrappers for every backend command and event (see the backend/frontend contract).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Every save re-encrypts the whole file (Argon2 key derivation), and unlocking derives the key
// too: a second or two, longer on a USB drive. A call still running after 250 ms shows the
// busy bar. Calls that wait on a native file dialog are left out, or the bar would run
// while the user picks a file.
const DIALOG_COMMANDS: Record<string, true> = {
  browse_open: true,
  browse_new: true,
  choose_keyfile: true,
  add_attachment: true,
  save_attachment: true,
};
let pending = 0;
let busyTimer: number | undefined;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (DIALOG_COMMANDS[cmd]) return tauriInvoke<T>(cmd, args);
  pending++;
  if (pending === 1) {
    busyTimer = window.setTimeout(() => {
      document.documentElement.dataset.busy = "true";
    }, 250);
  }
  try {
    return await tauriInvoke<T>(cmd, args);
  } finally {
    pending--;
    if (pending === 0) {
      window.clearTimeout(busyTimer);
      delete document.documentElement.dataset.busy;
    }
  }
}

export type Id = string;

export interface PlatformInfo {
  os: "windows" | "macos" | "linux";
  backdrop: boolean;
  /** Startup problem (e.g. the Auto-Type hotkey could not be registered) to show once. */
  notice: string | null;
}

export interface RecentItem {
  path: string;
  name: string;
  missing: boolean;
  keyfile: string | null;
  /** A password is kept behind Windows Hello / Touch ID for this database. */
  quickUnlock: boolean;
}
export interface UnlockView {
  recent: RecentItem[];
  selected: string | null;
  hasOpenDbs: boolean;
  pendingNew: string | null;
  /** "Touch ID" / "Windows Hello" when quick unlock is available and turned on. */
  quickUnlockLabel: string | null;
}

export interface TabItem {
  name: string;
  path: string;
  dirty: boolean;
}
/** A folder in the sidebar: every group except the root and the Recycle Bin subtree. */
export interface GroupItem {
  id: Id;
  parent: Id | null;
  name: string;
  /** 0 for top-level folders. */
  depth: number;
  /** Entries directly in this folder. */
  entryCount: number;
  /** Colour the user picked; null = automatic. */
  color: FolderColor | null;
}
export const FOLDER_COLORS = ["red", "orange", "yellow", "lime", "green", "teal", "cyan", "blue", "purple", "pink"] as const;
export type FolderColor = (typeof FOLDER_COLORS)[number];
export type TileColors = "folder" | "off";

export type Filter = { kind: "all" } | { kind: "notWorking" } | { kind: "recycleBin" } | { kind: "group"; id: Id };
export interface Counts {
  all: number;
  notWorking: number;
  recycleBin: number;
}
export interface EntrySummary {
  id: Id;
  title: string;
  username: string;
  url: string;
  stale: boolean;
  /** Folder id; null in the root or the Recycle Bin. */
  folder: Id | null;
}
export interface CustomFieldView {
  key: string;
  value: string | null;
  protected: boolean;
}
export interface EntryDetail {
  id: Id;
  title: string;
  username: string;
  url: string;
  notes: string;
  tags: string[];
  stale: boolean;
  inBin: boolean;
  folder: Id | null;
  /** "Email › Work"; "Recycle Bin" for deleted entries; null in the root. */
  folderPath: string | null;
  modified: string;
  hasPassword: boolean;
  hasRecoveryPin: boolean;
  recoveryPhone: string;
  hasSecurityQuestions: boolean;
  custom: CustomFieldView[];
  attachments: string[];
}
export interface MainView {
  tabs: TabItem[];
  activeTab: number;
  folders: GroupItem[];
  filter: Filter;
  counts: Counts;
  search: string;
  entries: EntrySummary[];
  selectedEntry: Id | null;
  detail: EntryDetail | null;
  saveError: string | null;
  notesMonospace: boolean;
  /** False when the database has its recycle bin turned off: every delete is permanent. */
  recycleBinEnabled: boolean;
  tileColors: TileColors;
}

export type FieldName =
  | "username"
  | "password"
  | "url"
  | "recoveryPin"
  | "recoveryPhone"
  | "securityQuestions"
  | `custom:${string}`;

export interface GenOptions {
  length: number;
  digits: boolean;
  symbols: boolean;
}
export interface CustomField {
  key: string;
  value: string;
  protected: boolean;
}
export interface FormGroup {
  id: Id;
  name: string;
  depth: number;
  isRecycleBin: boolean;
}
export interface EntryForm {
  id: Id | null;
  group: Id;
  title: string;
  url: string;
  username: string;
  password: string;
  recoveryPin: string;
  recoveryPhone: string;
  securityQuestions: string;
  notes: string;
  tags: string;
  autotypeSequence: string;
  autotypeWindows: string;
  custom: CustomField[];
}
export type FormMode = "add" | "edit";
export interface FormData {
  mode: FormMode;
  groups: FormGroup[];
  gen: GenOptions;
  entry: EntryForm;
}
export interface Duplicate {
  id: Id;
  title: string;
}

export interface GeneratorSettings {
  length: number;
  digits: boolean;
  symbols: boolean;
  uppercase: boolean;
  excludeAmbiguous: boolean;
}
export interface SettingsView {
  autotypeHotkey: string;
  autotypeDefaultSequence: string;
  reopenLast: boolean;
  notesMonospace: boolean;
  tileColors: TileColors;
  generator: GeneratorSettings;
  backupDir: string;
  quickUnlock: boolean;
  /** "Touch ID" / "Windows Hello" when the machine has it set up; null hides the option. */
  quickUnlockName: string | null;
}
/** One account in Quick Access (never the password). */
export interface QuickItem {
  /** Index of the open database. */
  db: number;
  id: Id;
  title: string;
  username: string;
  url: string;
  stale: boolean;
  folder: string | null;
  /** Only set when more than one database is open. */
  dbName: string | null;
}
export interface QuickOpen {
  locked: boolean;
  windowTitle: string;
  matches: QuickItem[];
}

/** Text of a rejected invoke: the backend rejects with a user-readable string. */
export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

export const api = {
  platformInfo: () => invoke<PlatformInfo>("platform_info"),

  unlockView: () => invoke<UnlockView>("unlock_view"),
  unlock: (path: string, password: string) => invoke<MainView>("unlock", { path, password }),
  unlockQuick: (path: string) => invoke<MainView>("unlock_quick", { path }),
  browseOpen: () => invoke<UnlockView>("browse_open"),
  browseNew: () => invoke<UnlockView>("browse_new"),
  createDatabase: (password: string, confirm: string) =>
    invoke<MainView>("create_database", { password, confirm }),
  cancelCreate: () => invoke<UnlockView>("cancel_create"),
  forgetRecent: (path: string) => invoke<UnlockView>("forget_recent", { path }),
  chooseKeyfile: (path: string) => invoke<UnlockView>("choose_keyfile", { path }),
  clearKeyfile: (path: string) => invoke<UnlockView>("clear_keyfile", { path }),

  mainView: () => invoke<MainView>("main_view"),
  selectTab: (index: number) => invoke<MainView>("select_tab", { index }),
  lockTab: (index: number) => invoke<MainView>("lock_tab", { index }),
  selectFilter: (filter: Filter) => invoke<MainView>("select_filter", { filter }),
  addGroup: (parent: Id | null, name: string) => invoke<MainView>("add_group", { parent, name }),
  setGroupColor: (id: Id, color: FolderColor | null) => invoke<MainView>("set_group_color", { id, color }),
  selectEntry: (id: Id) => invoke<MainView>("select_entry", { id }),
  search: (text: string) => invoke<MainView>("search", { text }),
  toggleStale: () => invoke<MainView>("toggle_stale"),
  deleteEntry: () => invoke<MainView>("delete_entry"),
  deleteGroup: (id: Id) => invoke<MainView>("delete_group", { id }),
  emptyRecycleBin: () => invoke<MainView>("empty_recycle_bin"),
  addAttachment: () => invoke<MainView>("add_attachment"),
  saveAttachment: (name: string) => invoke<string | null>("save_attachment", { name }),
  removeAttachment: (name: string) => invoke<MainView>("remove_attachment", { name }),
  revealField: (field: FieldName) => invoke<string>("reveal_field", { field }),
  copyField: (field: FieldName) => invoke<void>("copy_field", { field }),
  copyText: (text: string) => invoke<void>("copy_text", { text }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  retrySave: () => invoke<MainView>("retry_save"),

  formOpen: (mode: FormMode) => invoke<FormData>("form_open", { mode }),
  generate: (opts: GenOptions) => invoke<string>("generate", { opts }),
  findDuplicate: (title: string, url: string, editing: Id | null) =>
    invoke<Duplicate | null>("find_duplicate", { title, url, editing }),
  formSave: (entry: EntryForm, replace: Id | null) => invoke<MainView>("form_save", { entry, replace }),

  settingsGet: () => invoke<SettingsView>("settings_get"),
  settingsSave: (settings: SettingsView) => invoke<void>("settings_save", { settings }),
  openBackupFolder: () => invoke<void>("open_backup_folder"),

  autotypeSelected: () => invoke<void>("autotype_selected"),
  quickSearch: (text: string) => invoke<QuickItem[]>("quick_search", { text }),
  quickType: (db: number, id: Id) => invoke<void>("quick_type", { db, id }),
  quickCopy: (db: number, id: Id, field: "username" | "password") => invoke<void>("quick_copy", { db, id, field }),
  quickHide: () => invoke<void>("quick_hide"),
  quickOpenMain: () => invoke<void>("quick_open_main"),
};

export function onToast(handler: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("toast", (event) => handler(event.payload));
}

export function onQuickOpen(handler: (payload: QuickOpen) => void): Promise<UnlistenFn> {
  return listen<QuickOpen>("quick-open", (event) => handler(event.payload));
}
