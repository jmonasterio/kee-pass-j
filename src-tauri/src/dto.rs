//! Data sent to and received from the web frontend. Field names are camelCase on the wire.
//! The list and detail views never carry passwords or other protected values; those only
//! cross the bridge through `reveal_field` (one value on demand) and the edit form.

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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockView {
    pub recent: Vec<RecentItem>,
    pub selected: Option<String>,
    pub has_open_dbs: bool,
    pub pending_new: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabItem {
    pub name: String,
    pub path: String,
    pub dirty: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupItem {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub depth: usize,
    pub is_recycle_bin: bool,
    /// Entries directly in this group.
    pub entry_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub stale: bool,
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
    pub modified: String,
    pub has_password: bool,
    pub has_recovery_pin: bool,
    pub recovery_phone: String,
    pub has_security_questions: bool,
    pub custom: Vec<CustomFieldView>,
    pub attachments: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainView {
    pub tabs: Vec<TabItem>,
    pub active_tab: usize,
    pub groups: Vec<GroupItem>,
    pub selected_group: Option<String>,
    pub search: String,
    pub entries: Vec<EntrySummary>,
    pub selected_entry: Option<String>,
    pub detail: Option<EntryDetail>,
    pub save_error: Option<String>,
    pub notes_monospace: bool,
    /// False when the database has its recycle bin turned off: every delete is permanent.
    pub recycle_bin_enabled: bool,
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
    pub generator: GeneratorSettings,
    pub backup_dir: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChooserItem {
    pub title: String,
    pub username: String,
    pub db: String,
}
