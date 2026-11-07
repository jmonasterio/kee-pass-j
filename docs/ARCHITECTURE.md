# Module contracts

Cargo workspace. The root crate `ourlogins` is the library (`src/lib.rs`: vault, generator,
settings, autotype) and has no UI dependency. `src-tauri/` is the desktop app crate
(`ourlogins-app`, binary `OurLogins`), a Tauri 2 shell around the library. `web/` is the
frontend it embeds. All public types below are the integration contract between modules.
Keep signatures as written; add helpers freely.

Error type per module: `thiserror` enum named `<Module>Error`. No `anyhow` in the library.

## `vault`

Wraps `keepass::Database` (crate `keepass` 0.17, feature `save_kdbx4`).

```rust
pub struct EntryId(pub uuid::Uuid);   // Copy, Eq, Hash, Display
pub struct GroupId(pub uuid::Uuid);   // Copy, Eq, Hash, Display

pub struct GroupView { pub id: GroupId, pub parent: Option<GroupId>, pub name: String, pub depth: usize, pub is_recycle_bin: bool, pub color: Option<String> /* group custom data "OurLogins.Color" */ }

pub struct EntryView {
    pub id: EntryId, pub group: GroupId,
    pub title: String, pub username: String, pub password: String, pub url: String, pub notes: String,
    pub tags: Vec<String>,
    pub stale: bool,                               // custom field KPJ_Stale == "1"
    pub document: bool,                            // string field "OurLogins.Kind" == "document"
    pub recovery_pin: String,                      // custom field "Recovery PIN" (protected)
    pub recovery_phone: String,                    // custom field "Recovery Phone"
    pub security_questions: String,                // custom field "Security Questions" (protected, multiline)
    pub custom: Vec<CustomField>,                  // every other non-standard string field
    pub attachments: Vec<String>,                  // attachment names
    pub autotype_sequence: Option<String>,         // None = use default sequence
    pub autotype_windows: Vec<String>,             // window-title patterns
    pub modified: chrono::DateTime<chrono::Utc>,
}
pub struct CustomField { pub key: String, pub value: String, pub protected: bool }

/// Everything the UI can change. `id: None` creates; `Some` updates in place.
pub struct EntryEdit {
    pub id: Option<EntryId>, pub group: GroupId,
    pub title: String, pub username: String, pub password: String, pub url: String, pub notes: String,
    pub tags: Vec<String>, pub stale: bool,
    pub recovery_pin: String, pub recovery_phone: String, pub security_questions: String,
    pub custom: Vec<CustomField>,
    pub autotype_sequence: Option<String>, pub autotype_windows: Vec<String>,
}

pub struct BackupPolicy { pub dir: std::path::PathBuf, pub keep_recent: usize /*20*/, pub keep_daily_days: i64 /*30*/ }

pub struct Vault { /* path, db, key, dirty */ }
impl Vault {
    pub fn open(path: &Path, password: &str, keyfile: Option<&Path>) -> Result<Vault, VaultError>;
    pub fn create(path: &Path, password: &str) -> Result<Vault, VaultError>;   // new KDBX4, Argon2id, default groups like KeePass
    pub fn path(&self) -> &Path;
    pub fn name(&self) -> String;            // db name from meta, else file stem
    pub fn is_dirty(&self) -> bool;
    pub fn key_matches(&self, password: &str, keyfile: Option<&Path>) -> Result<bool, VaultError>;
    pub fn change_key(&mut self, password: &str, keyfile: Option<&Path>) -> Result<(), VaultError>;  // marks dirty; the next save re-encrypts
    /// `start_save` + `SaveJob::run`; on failure the vault stays dirty.
    pub fn save(&mut self, backups: &BackupPolicy) -> Result<(), VaultError>;
    /// Clones the database and key into a job and marks the vault clean. The job runs without the vault.
    pub fn start_save(&mut self) -> SaveJob;
    pub fn save_failed(&mut self, backed_up_original: bool);  // job failed: dirty again, original copy still owed
    pub fn root(&self) -> GroupId;
    pub fn groups(&self) -> Vec<GroupView>;  // pre-order, depth-first, root first
    pub fn entries_in(&self, group: GroupId) -> Vec<EntryView>;  // direct children only, stale last, then title A-Z
    pub fn all_entries(&self) -> Vec<EntryView>;                 // excludes recycle bin
    pub fn search(&self, query: &str) -> Vec<EntryView>;         // case-insensitive substring over title/username/url/notes/tags, every whitespace-separated term must match; stale last
    pub fn entry(&self, id: EntryId) -> Option<EntryView>;
    pub fn find_by_host(&self, host: &str) -> Vec<EntryView>;    // host of entry URL equals `host` (case-insensitive, `www.` stripped) or title contains host
    pub fn upsert_entry(&mut self, edit: EntryEdit) -> Result<EntryId, VaultError>;  // sets dirty, updates times
    pub fn delete_entry(&mut self, id: EntryId) -> Result<(), VaultError>;  // move to Recycle Bin group (create "Recycle Bin" if absent, like KeePass); if already in bin, delete permanently
    pub fn set_stale(&mut self, id: EntryId, stale: bool) -> Result<(), VaultError>;
    pub fn add_group(&mut self, parent: GroupId, name: &str) -> Result<GroupId, VaultError>;
    pub fn rename_group(&mut self, id: GroupId, name: &str) -> Result<(), VaultError>;
    pub fn delete_group(&mut self, id: GroupId) -> Result<(), VaultError>;  // whole subtree to the Recycle Bin; permanent when already in the bin, for the bin itself, or when the bin is disabled; root refused
    pub fn entry_count(&self, group: GroupId) -> usize;                     // direct entries only
    pub fn recycle_bin_enabled(&self) -> bool;                             // false when the metadata turns the bin off (deletes are permanent)
    pub fn recycle_bin_entries(&self) -> Vec<EntryView>;                   // everything in the bin, including inside deleted groups
    pub fn set_group_color(&mut self, id: GroupId, color: Option<&str>) -> Result<(), VaultError>;  // None = automatic
    pub fn set_document(&mut self, id: EntryId, document: bool) -> Result<(), VaultError>;  // false removes the field
    pub fn attachment(&self, id: EntryId, name: &str) -> Option<Vec<u8>>;
    pub fn attachment_size(&self, id: EntryId, name: &str) -> Option<u64>;
    pub fn attachments_total_bytes(&self) -> u64;  // every entry, Recycle Bin included; identical files counted once per entry (no dedup on save)
    pub fn add_attachment(&mut self, id: EntryId, name: &str, bytes: Vec<u8>) -> Result<(), VaultError>;
    pub fn remove_attachment(&mut self, id: EntryId, name: &str) -> Result<(), VaultError>;
}
impl SaveJob {
    /// Encrypts once into memory; `<path>.tmp` → fsync → rename over `<path>`; writes the same
    /// bytes to the backup folder (the file is never read back); rotates backups.
    pub fn run(self, backups: &BackupPolicy) -> Result<(), VaultError>;
    pub fn backs_up_original(&self) -> bool;
}
pub fn url_host(url: &str) -> Option<String>;  // "https://www.Foo.com/x" -> "foo.com"; bare "foo.com" accepted
pub fn list_backups(policy: &BackupPolicy, db_path: &Path) -> Vec<std::path::PathBuf>;  // newest first
```

Backup layout: `<policy.dir>/<sanitized db file stem>-<8 hex of path hash>/<stem>-YYYYMMDD-HHMMSS.kdbx`.
Rotation after each save: keep the newest `keep_recent`; among older files keep the newest
one per calendar day for the last `keep_daily_days` days; delete the rest.

Standard KDBX field names: `Title`, `UserName`, `Password`, `URL`, `Notes`. Custom fields
listed above are plain KDBX custom strings so KeePass 2.x / KeePassXC show them.

## `generator`

```rust
pub const AMBIGUOUS: &str = "0Oo1lI|5S8B2Z";
pub const SYMBOLS: &str = "!@#$%&*?-_=+";          // readable subset only
pub struct GenOptions { pub length: usize, pub digits: bool, pub symbols: bool, pub uppercase: bool, pub exclude_ambiguous: bool }
impl Default for GenOptions  // length 16, digits true, symbols false, uppercase true, exclude_ambiguous true
pub fn generate(opts: &GenOptions) -> String;   // OS random source (getrandom) with rejection sampling; guarantees at least one char from every enabled class when length >= enabled classes
pub fn alphabet(opts: &GenOptions) -> String;   // the exact charset used
```

## `settings`

```rust
pub struct RecentDb { pub path: std::path::PathBuf, pub keyfile: Option<std::path::PathBuf> }
pub struct Settings {
    pub recent: Vec<RecentDb>,          // most recent first, max 10
    pub reopen_last: bool,              // default true
    pub autotype_hotkey: String,        // default "Ctrl+Alt+A"
    pub autotype_default_sequence: String,  // default "{USERNAME}{TAB}{PASSWORD}{ENTER}"
    pub generator: generator::GenOptions,
    pub notes_monospace: bool,          // default false
}
impl Settings {
    pub fn load() -> Settings;                       // missing/corrupt file -> Default
    pub fn save(&self) -> Result<(), SettingsError>; // atomic write
    pub fn touch_recent(&mut self, path: &Path, keyfile: Option<&Path>);  // move/insert to front
}
pub fn data_dir() -> std::path::PathBuf;     // directories::ProjectDirs::from("com","logonpro","OurLogins").data_local_dir(); Settings::load() first moves the old "KeePassJ" directory here once
pub fn settings_path() -> std::path::PathBuf; // data_dir()/settings.json
pub fn backup_policy() -> vault::BackupPolicy;  // dir = data_dir()/backups, 20, 30
```

## `autotype`

```rust
pub enum Token { Field(String), Key(Key), Char(char), Delay(u64), Modified(Modifiers, Box<Token>) }
pub enum Key { Tab, Enter, Space, Backspace, Delete, Escape, Home, End, Up, Down, Left, Right, PageUp, PageDown, Insert, F(u8) }  // Insert fails on macOS (no such key)
pub struct Modifiers { pub shift: bool, pub ctrl: bool, pub alt: bool }
/// KeePass 2.x grammar: `{USERNAME}` `{PASSWORD}` `{URL}` `{TITLE}` `{NOTES}` `{S:Field Name}`,
/// `{TAB}` `{ENTER}` `{SPACE}` `{BACKSPACE}` `{DELETE}` `{ESC}` `{HOME}` `{END}` `{UP}` `{DOWN}` `{LEFT}` `{RIGHT}` `{F1}`..`{F12}`,
/// `{TAB 3}` repeat counts, `{DELAY 500}`, `{{}` `{}}` `{+}` `{^}` `{%}` escapes,
/// `+` `^` `%` prefix = shift/ctrl/alt applied to the next key or `(group)`.
/// Plain characters type literally.
pub fn parse_sequence(seq: &str) -> Result<Vec<Token>, AutotypeError>;
/// Types the sequence. `resolve` maps field names (upper-case standard names or `S:` custom names) to values.
pub fn perform(tokens: &[Token], resolve: &dyn Fn(&str) -> Option<String>) -> Result<(), AutotypeError>;
pub fn active_window_title() -> Option<String>;  // src/input.rs: Windows GetForegroundWindow; macOS frontmost app's layer-0 window via CGWindowListCopyWindowInfo (empty without Screen Recording permission)
/// KeePass window matching: pattern `//regex//` is a regex; otherwise `*` wildcards, case-insensitive, whole-title match.
pub fn window_matches(pattern: &str, title: &str) -> bool;
/// Default association: entry title (non-empty, >= 3 chars) appears in the window title, case-insensitive.
pub fn title_matches(entry_title: &str, window_title: &str) -> bool;
pub fn parse_hotkey(s: &str) -> Result<global_hotkey::hotkey::HotKey, AutotypeError>;  // "Ctrl+Alt+A"
```

`perform` inserts a 10 ms gap between keystrokes. Before the first keystroke it waits
`DELAY`-free; the caller is responsible for giving focus back to the target window
(hide our window, sleep ~250 ms).

## UI (`src-tauri/`, `web/`)

Tauri 2 with the system webview (WebView2 on Windows, WKWebView on macOS). The frontend is
vanilla TypeScript plus Fluent UI Web Components v3 (`web/src`), bundled by esbuild
(`web/build.mjs`) into `web/dist`. The `custom-protocol` feature is on by default, so
`cargo build --release -p ourlogins-app` compiles `web/dist` into one self-contained
`OurLogins.exe`. Run `npm run build` first.

- `src-tauri/src/state.rs`: `AppState` (open databases as tabs, each with a list filter
  — All, Not working, Recycle Bin or a folder — and a selected entry; search; recent list;
  pending new database; last save error) behind one `Mutex`, plus `main_view()` /
  `unlock_view()` that build the page data. `main_view()` drops the selected entry when it is
  not in the list shown, so the details always belong to a visible row.
- `src-tauri/src/commands.rs`: every `#[tauri::command]` the page can call. Mutating commands
  go: `Vault` method → `save_active()` (records a failure in `save_error`) → return the fresh
  `MainView`. Save failures show a persistent banner with Retry; an edit is never dropped
  silently. Commands are `async`, so Argon2, the blocking native file dialogs and the Windows
  Hello / Touch ID prompts run off the main thread.
- `src-tauri/src/dto.rs`: wire types (camelCase JSON), mirrored by `web/src/api.ts`.
- `src-tauri/src/typing.rs`: global hotkey (`global-hotkey`, manager owned by the main
  thread) and Auto-Type. Typing runs on the main thread. One account matching the active
  window is typed at once; otherwise the hotkey opens Quick Access, a second always-on-top
  window (`quick` in `tauri.conf.json`, same bundle loaded with `?quick`, `web/src/views/quick.ts`).
  Typing from Quick Access or the main window hides that window, waits 250 ms so the app
  behind gets the focus back, then types.
- `src-tauri/src/biometric.rs`: quick unlock. Windows: the password is encrypted with a key
  derived from a Windows Hello signature (KeePassXC's scheme). macOS: login Keychain item read
  only after a Touch ID check (the module doc explains why the Keychain itself cannot require
  Touch ID for an ad-hoc signed app). The first password unlock of a database stores it; a
  wrong stored password (changed elsewhere) is forgotten.
- Folder colours live in the file as group custom data `OurLogins.Color`.
- Documents are entries with the string field `OurLogins.Kind = document` (entry custom
  data does not survive a save with the `keepass` crate 0.17). Their notes render as
  Markdown through `web/src/markdown.ts`: markdown-it tokens (raw HTML off) become DOM
  nodes; links open only for http, https and mailto; images show as their alt text.
- Adding a file takes two commands: `pick_attachment` shows the native dialog and returns
  name, size and current totals (the path stays in Rust as `pending_attachment`), then
  `attach_picked` or `cancel_attachment`. `open_attachment` writes a copy to
  `<data dir>/open/<random>/<name>` and opens it with the default app; the `open` folder
  is deleted when the last database locks, at exit and at startup.
- The Add document form keeps chosen files as paths in `AppState.form_files`
  (`form_pick_files`, `form_remove_file`; `form_open` clears them). `form_save` reads them
  before changing the database and attaches them to the saved entry; equal names get " (2)".
- `src-tauri/src/saver.rs`: background saving. A command that changes a database calls
  `save_tab`, which starts one thread per file (`AppState.saving` lists them). The thread
  takes `Vault::start_save` under the lock, runs the job without it, repeats while the vault
  is dirty again, records `save_error` (keyed by path) and emits `saved`; the page then
  re-reads `main_view`. `lock_tab`, `change_master_password` and exit use `wait_idle` and
  then `save_now` on the calling thread. `lock_tab(discard)` locks despite a failing save
  only after the page asked "Lock without saving?".
- `change_master_password` checks the current password with `key_matches`, re-keys, saves at
  once (and restores the old key if that save fails), and forgets the quick-unlock password.
- Sibling folder names are unique ignoring case (`VaultError::DuplicateGroupName`, checked by
  `add_group` and `rename_group`).

Security rules for the webview:

- Vault text reaches the DOM only through text nodes and properties. `web/build.mjs` fails
  the build if `web/src` uses `innerHTML`, `outerHTML`, `insertAdjacentHTML`,
  `document.write`, `eval` or `new Function`.
- The CSP in `tauri.conf.json` allows only the app's own scripts, with no inline scripts or
  `eval`; `connect-src` is limited to Tauri IPC. A navigation guard keeps the webview on the
  app's own pages; links open in the browser through `open_url`.
- List and detail data never include passwords or other protected values. `copy_field`
  writes the clipboard from Rust (`clipboard.rs`, arboard without image support; marked to
  stay out of Windows clipboard history and cloud sync, and out of macOS clipboard-history
  apps). `reveal_field` returns one value when the user presses Show. The edit form
  necessarily carries the entry's values.
- The window runs `incognito`. On Windows, WebView2 general autofill and password autosave
  are off, and release builds disable browser shortcuts (reload, print, find, devtools).
- Capabilities (`src-tauri/capabilities/default.json`) grant both windows only event
  listening and window dragging. File dialogs and the clipboard are used from Rust only.
  Quick Access receives account titles and usernames, never passwords.
