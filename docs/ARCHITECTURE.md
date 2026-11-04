# Module contracts

Single crate `keepassj`: library (`src/lib.rs`) plus binary (`src/main.rs`). The library
has no Slint dependency; everything UI lives in `main.rs` and `ui/`. All public types below
are the integration contract between modules. Keep signatures as written; add helpers freely.

Error type per module: `thiserror` enum named `<Module>Error`. No `anyhow` in the library.

## `vault`

Wraps `keepass::Database` (crate `keepass` 0.17, feature `save_kdbx4`).

```rust
pub struct EntryId(pub uuid::Uuid);   // Copy, Eq, Hash, Display
pub struct GroupId(pub uuid::Uuid);   // Copy, Eq, Hash, Display

pub struct GroupView { pub id: GroupId, pub parent: Option<GroupId>, pub name: String, pub depth: usize, pub is_recycle_bin: bool }

pub struct EntryView {
    pub id: EntryId, pub group: GroupId,
    pub title: String, pub username: String, pub password: String, pub url: String, pub notes: String,
    pub tags: Vec<String>,
    pub stale: bool,                               // custom field KPJ_Stale == "1"
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
    /// Atomic write: `<path>.tmp` → fsync → rename over `<path>`. Then copy to backups and rotate.
    pub fn save(&mut self, backups: &BackupPolicy) -> Result<(), VaultError>;
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
    pub fn attachment(&self, id: EntryId, name: &str) -> Option<Vec<u8>>;
    pub fn add_attachment(&mut self, id: EntryId, name: &str, bytes: Vec<u8>) -> Result<(), VaultError>;
    pub fn remove_attachment(&mut self, id: EntryId, name: &str) -> Result<(), VaultError>;
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
pub fn generate(opts: &GenOptions) -> String;   // rand::rng(); guarantees at least one char from every enabled class when length >= enabled classes
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
pub fn data_dir() -> std::path::PathBuf;     // directories::ProjectDirs::from("com","logonpro","KeePassJ").data_local_dir()
pub fn settings_path() -> std::path::PathBuf; // data_dir()/settings.json
pub fn backup_policy() -> vault::BackupPolicy;  // dir = data_dir()/backups, 20, 30
```

## `autotype`

```rust
pub enum Token { Field(String), Key(enigo::Key), Char(char), Delay(u64), Modified(Modifiers, Box<Token>) }
pub struct Modifiers { pub shift: bool, pub ctrl: bool, pub alt: bool }
/// KeePass 2.x grammar: `{USERNAME}` `{PASSWORD}` `{URL}` `{TITLE}` `{NOTES}` `{S:Field Name}`,
/// `{TAB}` `{ENTER}` `{SPACE}` `{BACKSPACE}` `{DELETE}` `{ESC}` `{HOME}` `{END}` `{UP}` `{DOWN}` `{LEFT}` `{RIGHT}` `{F1}`..`{F12}`,
/// `{TAB 3}` repeat counts, `{DELAY 500}`, `{{}` `{}}` `{+}` `{^}` `{%}` escapes,
/// `+` `^` `%` prefix = shift/ctrl/alt applied to the next key or `(group)`.
/// Plain characters type literally.
pub fn parse_sequence(seq: &str) -> Result<Vec<Token>, AutotypeError>;
/// Types the sequence. `resolve` maps field names (upper-case standard names or `S:` custom names) to values.
pub fn perform(tokens: &[Token], resolve: &dyn Fn(&str) -> Option<String>) -> Result<(), AutotypeError>;
pub fn active_window_title() -> Option<String>;
/// KeePass window matching: pattern `//regex//` is a regex; otherwise `*` wildcards, case-insensitive, whole-title match.
pub fn window_matches(pattern: &str, title: &str) -> bool;
/// Default association: entry title (non-empty, >= 3 chars) appears in the window title, case-insensitive.
pub fn title_matches(entry_title: &str, window_title: &str) -> bool;
pub fn parse_hotkey(s: &str) -> Result<global_hotkey::hotkey::HotKey, AutotypeError>;  // "Ctrl+Alt+A"
```

`perform` inserts a 10 ms gap between keystrokes. Before the first keystroke it waits
`DELAY`-free; the caller is responsible for giving focus back to the target window
(hide our window, sleep ~250 ms).

## UI (`main.rs`, `ui/`)

Slint 1.18. One `AppWindow` with a `page` enum: `Unlock`, `Main`, `AddAccount`, `EditEntry`,
`Generator`, `Settings`. Databases are tabs; each tab holds a `Vault`. Models exposed to
Slint are `slint::VecModel` of flat structs (`GroupItem`, `EntryItem`, `TabItem`).
Every mutation goes: Slint callback → `Vault` method → `vault.save(&backup_policy())` →
rebuild models. Save failures show a persistent banner with the error; the app never
silently drops an edit.
