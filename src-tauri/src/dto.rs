//! Data sent to and received from the web frontend. Field names are camelCase on the wire.
//! The list and detail views never carry passwords or other protected values; those only
//! cross the bridge through `reveal_field` (one value on demand) and the edit form.

use ourlogins::settings::TileColors;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub os: &'static str,
    pub backdrop: bool,
    pub notice: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentItem {
    pub path: String,
    pub name: String,
    pub missing: bool,
    pub keyfile: Option<String>,
    /// A password is stored for this database: "Unlock with Touch ID / Windows Hello" works.
    pub quick_unlock: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockView {
    pub recent: Vec<RecentItem>,
    pub selected: Option<String>,
    pub has_open_dbs: bool,
    pub pending_new: Option<String>,
    /// "Touch ID" / "Windows Hello" when quick unlock is available and turned on.
    pub quick_unlock_label: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabItem {
    pub name: String,
    pub path: String,
    pub dirty: bool,
}

/// A folder in the sidebar (any group except the root and the Recycle Bin subtree).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupItem {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    /// 0 for folders directly under the root.
    pub depth: usize,
    /// Entries directly in this folder.
    pub entry_count: usize,
    /// Colour name the user picked; `None` = automatic.
    pub color: Option<String>,
}

/// What the account list shows when the search box is empty.
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilterDto {
    All,
    NotWorking,
    Documents,
    RecycleBin,
    Group { id: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub all: usize,
    pub not_working: usize,
    pub documents: usize,
    pub recycle_bin: usize,
}

/// Space the open database takes, shown next to attachments.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    /// All attachments in the file.
    pub attachments_bytes: u64,
    /// The `.kdbx` file on disk as last saved.
    pub file_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub name: String,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub stale: bool,
    /// Folder id; `None` when the entry sits in the root (or the Recycle Bin).
    pub folder: Option<String>,
    /// A document entry (text and files) rather than a login.
    pub document: bool,
    pub attachment_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldView {
    pub key: String,
    pub value: Option<String>,
    pub protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub stale: bool,
    pub in_bin: bool,
    /// Folder id (see `EntrySummary::folder`).
    pub folder: Option<String>,
    /// "Email › Work"; "Recycle Bin" for deleted entries; `None` in the root.
    pub folder_path: Option<String>,
    pub modified: String,
    pub has_password: bool,
    pub has_recovery_pin: bool,
    pub recovery_phone: String,
    pub has_security_questions: bool,
    pub custom: Vec<CustomFieldView>,
    pub attachments: Vec<AttachmentInfo>,
    pub document: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainView {
    pub tabs: Vec<TabItem>,
    pub active_tab: usize,
    pub folders: Vec<GroupItem>,
    pub filter: FilterDto,
    pub counts: Counts,
    pub search: String,
    pub entries: Vec<EntrySummary>,
    pub selected_entry: Option<String>,
    pub detail: Option<EntryDetail>,
    pub save_error: Option<String>,
    pub notes_monospace: bool,
    /// False when the database has its recycle bin turned off: every delete is permanent.
    pub recycle_bin_enabled: bool,
    pub tile_colors: TileColors,
    pub storage: Storage,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct GenOptionsDto {
    pub length: usize,
    pub digits: bool,
    pub symbols: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldDto {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormGroup {
    pub id: String,
    pub name: String,
    pub depth: usize,
    pub is_recycle_bin: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryForm {
    pub id: Option<String>,
    pub group: String,
    pub title: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub recovery_pin: String,
    pub recovery_phone: String,
    pub security_questions: String,
    pub notes: String,
    pub tags: String,
    pub autotype_sequence: String,
    pub autotype_windows: String,
    pub custom: Vec<CustomFieldDto>,
    pub document: bool,
}

/// A file the user picked to attach, before it is added (sizes in bytes).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPick {
    pub name: String,
    pub size: u64,
    /// Attachments already in the database.
    pub attachments_bytes: u64,
    /// The database file now.
    pub file_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    pub version: &'static str,
    pub repository: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormData {
    pub mode: &'static str,
    pub groups: Vec<FormGroup>,
    pub gen: GenOptionsDto,
    pub entry: EntryForm,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Duplicate {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorSettings {
    pub length: usize,
    pub digits: bool,
    pub symbols: bool,
    pub uppercase: bool,
    pub exclude_ambiguous: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub autotype_hotkey: String,
    pub autotype_default_sequence: String,
    pub reopen_last: bool,
    pub notes_monospace: bool,
    pub tile_colors: TileColors,
    pub generator: GeneratorSettings,
    pub backup_dir: String,
    pub quick_unlock: bool,
    /// "Touch ID" / "Windows Hello" when this machine has it set up (read-only); `None` hides the option.
    pub quick_unlock_name: Option<String>,
}

/// One account in Quick Access. Never carries the password.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuickItem {
    /// Index of the open database (tab).
    pub db: usize,
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub stale: bool,
    pub folder: Option<String>,
    /// Database name, only when more than one database is open.
    pub db_name: Option<String>,
}

/// Sent to the Quick Access window when the global hotkey opens it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuickOpen {
    /// No database is unlocked: Quick Access can only offer to open the app.
    pub locked: bool,
    /// Title of the window that was active when the hotkey was pressed.
    pub window_title: String,
    /// Accounts matching that window (none or several; exactly one is typed straight away).
    pub matches: Vec<QuickItem>,
}
