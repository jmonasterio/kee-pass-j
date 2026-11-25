//! KDBX open/save/backup, entry and group model, search, stale flag.
//!
//! Wraps `keepass::Database`. Standard KDBX fields are `Title`, `UserName`, `Password`,
//! `URL`, `Notes`. Application fields are plain KDBX custom strings:
//! `KPJ_Stale`, `Recovery PIN` (protected), `Recovery Phone`, `Security Questions` (protected).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use keepass::config::{DatabaseConfig, DatabaseVersion, InnerCipherConfig, KdfConfig, VariantDictionary};
use keepass::db::{self, fields, AutoType, AutoTypeAssociation, Database, Entry, History, Times, Value};
use keepass::DatabaseKey;

pub const FIELD_STALE: &str = "KPJ_Stale";
pub const FIELD_RECOVERY_PIN: &str = "Recovery PIN";
pub const FIELD_RECOVERY_PHONE: &str = "Recovery Phone";
pub const FIELD_SECURITY_QUESTIONS: &str = "Security Questions";
const RECYCLE_BIN_NAME: &str = "Recycle Bin";
/// KeePass 2.x built-in icon index for the trash bin.
const RECYCLE_BIN_ICON: usize = 43;
const DEFAULT_HISTORY_MAX_ITEMS: usize = 10;
const BACKUP_EXT: &str = "kdbx";
/// Group custom-data key for the folder color shown in Our Logins; stored in the file.
const GROUP_COLOR_KEY: &str = "OurLogins.Color";
/// String field that marks Our Logins "document" entries (value `KIND_DOCUMENT`). A string
/// field like `KPJ_Stale`: entry custom data does not survive a save in keepass 0.17.
pub const FIELD_KIND: &str = "OurLogins.Kind";
const KIND_DOCUMENT: &str = "document";

/// Fields with a dedicated slot in `EntryView`; everything else is a custom field.
const RESERVED_FIELDS: [&str; 10] = [
    fields::TITLE,
    fields::USERNAME,
    fields::PASSWORD,
    fields::URL,
    fields::NOTES,
    FIELD_STALE,
    FIELD_RECOVERY_PIN,
    FIELD_RECOVERY_PHONE,
    FIELD_SECURITY_QUESTIONS,
    FIELD_KIND,
];

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("wrong password, or this database also needs a key file")]
    WrongKey,
    #[error("cannot open database: {0}")]
    Open(#[from] db::DatabaseOpenError),
    #[error("cannot serialize database: {0}")]
    Save(#[from] db::DatabaseSaveError),
    #[error("invalid key derivation settings: {0}")]
    Kdf(#[from] keepass::config::KdfConfigError),
    #[error("database file already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("database path has no file name: {0}")]
    InvalidPath(PathBuf),
    #[error("entry {0} not found")]
    EntryNotFound(EntryId),
    #[error("group {0} not found")]
    GroupNotFound(GroupId),
    #[error("attachment {0:?} not found")]
    AttachmentNotFound(String),
    #[error("entry {0} has no earlier version")]
    NoEarlierVersion(EntryId),
    #[error("the folder that version of entry {0} was in no longer exists")]
    VersionFolderGone(EntryId),
    #[error("could not keep {0:?} for the entry's history")]
    AttachmentNotKept(String),
    #[error("group name must not be empty")]
    EmptyGroupName,
    #[error("there is already a folder called \"{0}\" here")]
    DuplicateGroupName(String),
    #[error("the top group of a database cannot be deleted")]
    CannotDeleteRoot,
    #[error("this group holds the Recycle Bin; move the Recycle Bin out first")]
    GroupHoldsRecycleBin,
    #[error("database saved but backup failed at {path}: {source}")]
    Backup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn io_err(path: &Path, source: std::io::Error) -> VaultError {
    VaultError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(pub uuid::Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupId(pub uuid::Uuid);

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<db::EntryId> for EntryId {
    fn from(id: db::EntryId) -> Self {
        EntryId(id.uuid())
    }
}

impl From<EntryId> for db::EntryId {
    fn from(id: EntryId) -> Self {
        db::EntryId::from_uuid(id.0)
    }
}

impl From<db::GroupId> for GroupId {
    fn from(id: db::GroupId) -> Self {
        GroupId(id.uuid())
    }
}

impl From<GroupId> for db::GroupId {
    fn from(id: GroupId) -> Self {
        db::GroupId::from_uuid(id.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupView {
    pub id: GroupId,
    pub parent: Option<GroupId>,
    pub name: String,
    pub depth: usize,
    pub is_recycle_bin: bool,
    /// Color name chosen by the user (`OurLogins.Color` group custom data); `None` = automatic.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomField {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryView {
    pub id: EntryId,
    pub group: GroupId,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    pub tags: Vec<String>,
    /// Custom field `KPJ_Stale == "1"`.
    pub stale: bool,
    /// Custom field `Recovery PIN` (protected).
    pub recovery_pin: String,
    /// Custom field `Recovery Phone`.
    pub recovery_phone: String,
    /// Custom field `Security Questions` (protected, multiline).
    pub security_questions: String,
    /// Every other non-standard string field.
    pub custom: Vec<CustomField>,
    /// Attachment names.
    pub attachments: Vec<String>,
    /// The entry's own default sequence. `None` when it has none: the folder default
    /// (`autotype_group_sequence`) or else the app's default applies.
    pub autotype_sequence: Option<String>,
    /// Window associations, in file order.
    pub autotype_windows: Vec<AutoTypeWindow>,
    /// KeePass group `DefaultAutoTypeSequence` inherited by the entry: the nearest non-empty
    /// one from its folder up to the root.
    pub autotype_group_sequence: Option<String>,
    /// KeePass's effective Auto-Type switch: the entry's own `Enabled` flag, and the nearest
    /// folder up the tree that sets `EnableAutoType` does not set it to false.
    pub autotype_enabled: bool,
    /// A "document" entry (entry custom data `OurLogins.Kind = document`): text and files,
    /// no login. Other KeePass apps show it as a normal entry.
    pub document: bool,
    pub modified: DateTime<Utc>,
}

/// A KeePass Auto-Type window association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoTypeWindow {
    /// Window-title pattern (see `autotype::window_matches`).
    pub window: String,
    /// Sequence typed into a matching window; empty means the entry's default sequence.
    pub sequence: String,
}

impl EntryView {
    /// The sequence of the first window association whose pattern matches `window_title`,
    /// when that sequence is non-blank. KeePass types it instead of the entry's default.
    pub fn autotype_window_sequence(&self, window_title: &str) -> Option<&str> {
        self.autotype_windows
            .iter()
            .find(|w| crate::autotype::window_matches(&w.window, window_title))
            .map(|w| w.sequence.as_str())
            .filter(|s| !s.trim().is_empty())
    }

    /// The sequence KeePass types for this entry into the window titled `window_title`
    /// (`None`: no window known): the matching association's own sequence, otherwise the
    /// entry's own sequence, otherwise the inherited folder default. `None` means the app's
    /// default sequence applies.
    pub fn autotype_sequence_for(&self, window_title: Option<&str>) -> Option<&str> {
        window_title
            .and_then(|title| self.autotype_window_sequence(title))
            .or(self.autotype_sequence.as_deref())
            .or(self.autotype_group_sequence.as_deref())
    }
}

/// Everything the UI can change. `id: None` creates; `Some` updates in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryEdit {
    pub id: Option<EntryId>,
    pub group: GroupId,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub stale: bool,
    pub recovery_pin: String,
    pub recovery_phone: String,
    pub security_questions: String,
    pub custom: Vec<CustomField>,
    pub autotype_sequence: Option<String>,
    pub autotype_windows: Vec<String>,
    pub document: bool,
    /// Files to attach in the same change (name, bytes); a name the entry already has is replaced.
    pub attach: Vec<(String, Vec<u8>)>,
    /// Attachment names to remove in the same change (before `attach` is applied).
    pub detach: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPolicy {
    pub dir: PathBuf,
    /// Newest saves kept unconditionally (20).
    pub keep_recent: usize,
    /// Among older saves, one per calendar day for this many days (30).
    pub keep_daily_days: i64,
}

pub struct Vault {
    path: PathBuf,
    db: Database,
    /// Kept so `save` can re-encrypt. `DatabaseKey` zeroizes its contents on drop.
    key: DatabaseKey,
    dirty: bool,
    /// The file on disk has not been copied to the backups yet in this session. The first
    /// save copies it before replacing it.
    original_pending: bool,
    /// Opened as KDBX 3.1; the first save keeps a permanent copy of the 3.1 file.
    upgraded_from_kdbx3: bool,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("dirty", &self.dirty)
            .finish_non_exhaustive()
    }
}

fn file_stem(path: &Path) -> Result<String, VaultError> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| VaultError::InvalidPath(path.to_path_buf()))
}

fn build_key(password: &str, keyfile: Option<&Path>) -> Result<DatabaseKey, VaultError> {
    let mut key = DatabaseKey::new();
    if !password.is_empty() {
        key = key.with_password(password);
    }
    if let Some(kf) = keyfile {
        let mut f = File::open(kf).map_err(|e| io_err(kf, e))?;
        key = key.with_keyfile(&mut f).map_err(|e| io_err(kf, e))?;
    }
    Ok(key)
}

/// KDBX4 config: AES-256 outer cipher, ChaCha20 inner cipher, GZip, Argon2id with the
/// KeePass 2.x default work factor (2 iterations, 64 MiB, 2 lanes).
fn new_database_config() -> Result<DatabaseConfig, VaultError> {
    // `KdfConfig::Argon2id` carries an `argon2::Version` that keepass does not re-export, so
    // the variant is built through the crate's own KDF header parser.
    const KDF_ARGON2ID: [u8; 16] = [
        0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
    ];
    let mut vd = VariantDictionary::new();
    vd.set("$UUID", KDF_ARGON2ID.to_vec());
    vd.set("M", 64u64 * 1024 * 1024);
    vd.set("S", vec![0u8; 32]);
    vd.set("I", 2u64);
    vd.set("P", 2u32);
    vd.set("V", 0x13u32);
    let (kdf, _seed): (KdfConfig, Vec<u8>) = vd.try_into()?;
    let mut config = DatabaseConfig::default();
    config.kdf_config = kdf;
    Ok(config)
}

fn to_utc(t: Option<NaiveDateTime>) -> DateTime<Utc> {
    Utc.from_utc_datetime(&t.unwrap_or_else(Times::epoch))
}

fn entry_view(e: &db::EntryRef<'_>) -> EntryView {
    entry_view_in(e, e.parent().id().into())
}

/// `entry_view` with the folder given: for history versions, whose own folder keepass reads
/// with `unwrap` (it may have been deleted since).
fn entry_view_in(e: &db::EntryRef<'_>, group: GroupId) -> EntryView {
    let get = |k: &str| e.get(k).unwrap_or_default().to_owned();
    let mut custom: Vec<CustomField> = e
        .fields
        .iter()
        .filter(|(k, _)| !RESERVED_FIELDS.contains(&k.as_str()))
        .map(|(k, v)| CustomField {
            key: k.clone(),
            value: v.get().clone(),
            protected: v.is_protected(),
        })
        .collect();
    custom.sort_by(|a, b| a.key.cmp(&b.key));
    let mut attachments: Vec<String> = e.attachments_named().map(|(n, _)| n.to_owned()).collect();
    attachments.sort();
    let (autotype_sequence, autotype_windows, entry_enabled) = match &e.autotype {
        Some(at) => (
            non_blank(at.default_sequence.as_deref()),
            at.associations
                .iter()
                .map(|a| AutoTypeWindow { window: a.window.clone(), sequence: a.sequence.clone() })
                .collect(),
            at.enabled,
        ),
        None => (None, Vec::new(), true),
    };
    let (autotype_group_sequence, group_enabled) = group_autotype(e.database(), group.into());
    EntryView {
        id: e.id().into(),
        group,
        title: get(fields::TITLE),
        username: get(fields::USERNAME),
        password: get(fields::PASSWORD),
        url: get(fields::URL),
        notes: get(fields::NOTES),
        tags: e.tags.clone(),
        stale: e.get(FIELD_STALE) == Some("1"),
        recovery_pin: get(FIELD_RECOVERY_PIN),
        recovery_phone: get(FIELD_RECOVERY_PHONE),
        security_questions: get(FIELD_SECURITY_QUESTIONS),
        custom,
        attachments,
        autotype_sequence,
        autotype_windows,
        autotype_group_sequence,
        autotype_enabled: entry_enabled && group_enabled,
        document: e.get(FIELD_KIND) == Some(KIND_DOCUMENT),
        modified: to_utc(e.times.last_modification),
    }
}

fn non_blank(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty()).map(str::to_owned)
}

/// KeePass's inherited group Auto-Type settings, walking from `group` up to the root: the
/// nearest non-blank `DefaultAutoTypeSequence`, and the nearest explicit `EnableAutoType`
/// (unset all the way up means enabled). A group that no longer exists (a history version's
/// old folder) contributes nothing.
fn group_autotype(db: &Database, group: db::GroupId) -> (Option<String>, bool) {
    let mut sequence = None;
    let mut enabled = None;
    let mut current = Some(group);
    while let Some(id) = current {
        let Some(g) = db.group(id) else { break };
        if sequence.is_none() {
            sequence = non_blank(g.default_autotype_sequence.as_deref());
        }
        if enabled.is_none() {
            enabled = g.enable_autotype;
        }
        if sequence.is_some() && enabled.is_some() {
            break;
        }
        current = g.parent().map(|p| p.id());
    }
    (sequence, enabled.unwrap_or(true))
}

/// Stale entries last, then title A-Z (case-insensitive), then id for a stable order.
fn sort_views(views: &mut [EntryView]) {
    views.sort_by(|a, b| {
        a.stale
            .cmp(&b.stale)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.id.0.cmp(&b.id.0))
    });
}

fn set_or_remove(entry: &mut Entry, key: &str, value: &str, protected: bool) {
    if value.is_empty() {
        entry.fields.remove(key);
    } else if protected {
        entry.set_protected(key, value);
    } else {
        entry.set_unprotected(key, value);
    }
}

/// Keeps the newest `max` history versions and drops attachments only the removed
/// versions used.
fn trim_history(db: &mut Database, id: db::EntryId, max: usize) {
    let Some(mut entry) = db.entry_mut(id) else { return };
    let Some(history) = entry.history.as_ref() else { return };
    if history.get_entries().len() <= max {
        return;
    }
    let kept: Vec<Entry> = history.get_entries().iter().take(max).cloned().collect();
    let mut trimmed = History::default();
    for version in kept.into_iter().rev() {
        trimmed.add_entry(version);
    }
    entry.history = Some(trimmed);
    db.remove_unused_attachments();
}

/// Takes `names` off the live entry `e` while their data stays in the database, under the
/// same attachment ids, for the history versions that use them.
///
/// keepass 0.17 deletes an attachment's data as soon as the live entry lets go of it:
/// `remove_attachment_by_name`, and `add_attachment` over a name the entry has, drop every
/// reference of the entry, history versions included, and then the attachment. So each file
/// is put back under its old id (`add_attachment` takes the lowest free id; the free ids
/// below it are filled first) and the entry is set back to its state without those files.
/// The caller then rebuilds the references with `remove_unused_attachments`, once the new
/// history version is in place; that also drops the fillers.
fn detach_keeping_data(e: &mut db::EntryMut<'_>, names: &[&String]) -> Result<(), VaultError> {
    for &name in names {
        let Some((old_id, data)) = e.as_ref().attachment_by_name(name).map(|a| (a.id(), a.data.clone())) else { continue };
        e.remove_attachment_by_name(name);
        let without: Entry = (**e).clone();
        let used: HashSet<usize> = e.database_mut().iter_all_attachments().map(|a| a.id().id()).collect();
        for free in (0..old_id.id()).filter(|i| !used.contains(i)) {
            e.add_attachment(format!("ourlogins-filler-{free}"), Value::unprotected(Vec::new()));
        }
        let back = e.add_attachment(format!("ourlogins-kept-{}", old_id.id()), data).id();
        if back != old_id {
            return Err(VaultError::AttachmentNotKept(name.clone()));
        }
        **e = without;
    }
    Ok(())
}

/// The folder history version `index` was in, if it still exists. keepass reads a version's
/// folder with `unwrap`, which panics for a folder deleted since; that panic is caught.
fn version_folder(entry: &db::EntryRef<'_>, index: usize) -> Option<db::GroupId> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| entry.historical(index).map(|h| h.parent().id())))
        .ok()
        .flatten()
}

/// File names (sorted) with their attachment ids: the same name with another id is a
/// replaced file.
fn files_of(e: &db::EntryRef<'_>) -> Vec<(String, usize)> {
    let mut files: Vec<(String, usize)> = e.attachments_named().map(|(n, a)| (n.to_owned(), a.id().id())).collect();
    files.sort();
    files
}

/// An earlier version of an entry (see `Vault::versions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub modified: DateTime<Utc>,
    /// What the next newer version (or the current one) changed, e.g. "password, website
    /// changed", "file added: scan.pdf".
    pub changes: Vec<String>,
}

/// One difference between two versions of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// A standard field: its key in the page, and its name for people.
    Field(&'static str, &'static str),
    CustomAdded(String),
    CustomRemoved(String),
    CustomChanged(String),
    FileAdded(String),
    FileRemoved(String),
    FileReplaced(String),
    /// Marked not working (`true`) or working.
    Stale(bool),
    /// Made a document (`true`) or an account.
    Document(bool),
}

impl Change {
    /// The pane's key for the field this changes ("custom:<name>", "files" for any file).
    fn key(&self) -> String {
        match self {
            Change::Field(key, _) => (*key).to_owned(),
            Change::CustomAdded(k) | Change::CustomRemoved(k) | Change::CustomChanged(k) => format!("custom:{k}"),
            Change::FileAdded(_) | Change::FileRemoved(_) | Change::FileReplaced(_) => "files".to_owned(),
            Change::Stale(_) => "stale".to_owned(),
            Change::Document(_) => "document".to_owned(),
        }
    }
}

type Version = (EntryView, Vec<(String, usize)>);

/// What changed from `old` to `new`.
fn changes(old: &Version, new: &Version) -> Vec<Change> {
    let (o, n) = (&old.0, &new.0);
    let fields = [
        ("title", "name", o.title != n.title),
        ("username", "username", o.username != n.username),
        ("password", "password", o.password != n.password),
        ("url", "website", o.url != n.url),
        ("recoveryPin", "recovery PIN", o.recovery_pin != n.recovery_pin),
        ("recoveryPhone", "recovery phone", o.recovery_phone != n.recovery_phone),
        ("securityQuestions", "security questions", o.security_questions != n.security_questions),
        ("notes", if n.document { "text" } else { "notes" }, o.notes != n.notes),
        ("tags", "tags", o.tags != n.tags),
        ("autotypeSequence", "Auto-Type sequence", o.autotype_sequence != n.autotype_sequence),
        ("autotypeWindows", "Auto-Type windows", o.autotype_windows != n.autotype_windows),
    ];
    let mut out: Vec<Change> = fields.iter().filter(|f| f.2).map(|f| Change::Field(f.0, f.1)).collect();
    for c in &n.custom {
        match o.custom.iter().find(|x| x.key == c.key) {
            None => out.push(Change::CustomAdded(c.key.clone())),
            Some(x) if x != c => out.push(Change::CustomChanged(c.key.clone())),
            Some(_) => {}
        }
    }
    out.extend(o.custom.iter().filter(|c| !n.custom.iter().any(|x| x.key == c.key)).map(|c| Change::CustomRemoved(c.key.clone())));
    for (name, id) in &new.1 {
        match old.1.iter().find(|(n, _)| n == name) {
            None => out.push(Change::FileAdded(name.clone())),
            Some((_, old_id)) if old_id != id => out.push(Change::FileReplaced(name.clone())),
            Some(_) => {}
        }
    }
    out.extend(old.1.iter().filter(|(name, _)| !new.1.iter().any(|(n, _)| n == name)).map(|(name, _)| Change::FileRemoved(name.clone())));
    if o.stale != n.stale {
        out.push(Change::Stale(n.stale));
    }
    if o.document != n.document {
        out.push(Change::Document(n.document));
    }
    out
}

/// Lines for the version list: "password, website changed", "file added: scan.pdf".
fn describe_changes(changes: &[Change]) -> Vec<String> {
    let changed: Vec<String> = changes
        .iter()
        .filter_map(|c| match c {
            Change::Field(_, name) => Some((*name).to_owned()),
            Change::CustomChanged(key) => Some(format!("\"{key}\"")),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    if !changed.is_empty() {
        out.push(format!("{} changed", changed.join(", ")));
    }
    for c in changes {
        match c {
            Change::CustomAdded(k) => out.push(format!("field added: {k}")),
            Change::CustomRemoved(k) => out.push(format!("field removed: {k}")),
            Change::FileAdded(n) => out.push(format!("file added: {n}")),
            Change::FileRemoved(n) => out.push(format!("file removed: {n}")),
            Change::FileReplaced(n) => out.push(format!("file replaced: {n}")),
            Change::Stale(true) => out.push("marked not working".to_owned()),
            Change::Stale(false) => out.push("marked working".to_owned()),
            Change::Document(true) => out.push("made a document".to_owned()),
            Change::Document(false) => out.push("made an account".to_owned()),
            Change::Field(..) | Change::CustomChanged(_) => {}
        }
    }
    if out.is_empty() {
        out.push("no field changes".to_owned());
    }
    out
}

impl Vault {
    pub fn open(path: &Path, password: &str, keyfile: Option<&Path>) -> Result<Vault, VaultError> {
        let key = build_key(password, keyfile)?;
        let mut file = File::open(path).map_err(|e| io_err(path, e))?;
        let mut db = Database::open(&mut file, key.clone()).map_err(|e| match e {
            // KDBX 3.1 decrypts the whole payload before checking the key, so a wrong key
            // surfaces as a padding error from AES-CBC instead of IncorrectKey.
            db::DatabaseOpenError::Key(keepass::error::DatabaseKeyError::IncorrectKey)
            | db::DatabaseOpenError::Cryptography(keepass::error::CryptographyError::InvalidPadding(_)) => {
                VaultError::WrongKey
            }
            other => VaultError::Open(other),
        })?;
        // keepass 0.17 can only write KDBX 4. A KDBX 3.1 file is converted in memory the way
        // KeePass 2.35+ does it: same outer cipher and AES-KDF rounds, inner stream cipher
        // ChaCha20. The file on disk changes format on the first save; KeePass 2.35+ and
        // KeePassXC 2.3+ open the result.
        let upgraded_from_kdbx3 = matches!(db.config.version, DatabaseVersion::KDB3(_));
        if upgraded_from_kdbx3 {
            db.config.version = DatabaseVersion::KDB4(1);
            db.config.inner_cipher_config = InnerCipherConfig::ChaCha20;
        }
        // The writer also rejects KDBX 4.0 (`dump.rs` requires exactly `KDB4(1)`), so 4.0
        // files are saved as 4.1. KeePass 2.48+ and KeePassXC 2.7+ open 4.1.
        if matches!(db.config.version, DatabaseVersion::KDB4(minor) if minor != 1) {
            db.config.version = DatabaseVersion::KDB4(1);
        }
        Ok(Vault {
            path: path.to_path_buf(),
            db,
            key,
            dirty: false,
            original_pending: true,
            upgraded_from_kdbx3,
        })
    }

    /// New KDBX4 database with a root group named after the file stem and a "Recycle Bin"
    /// group registered in the metadata, as KeePass 2.x creates it. Nothing is written until
    /// the first `save`. Refuses to shadow an existing file.
    pub fn create(path: &Path, password: &str) -> Result<Vault, VaultError> {
        if path.exists() {
            return Err(VaultError::AlreadyExists(path.to_path_buf()));
        }
        let stem = file_stem(path)?;
        let key = build_key(password, None)?;
        let mut db = Database::with_config(new_database_config()?);
        let now = Times::now();
        db.meta.generator = Some("Our Logins".to_owned());
        db.meta.database_name = Some(stem.clone());
        db.meta.database_name_changed = Some(now);
        db.meta.memory_protection = Some(db::MemoryProtection::default());
        db.meta.maintenance_history_days = Some(365);
        db.meta.master_key_changed = Some(now);
        db.meta.master_key_change_rec = Some(-1);
        db.meta.master_key_change_force = Some(-1);
        db.meta.history_max_items = Some(DEFAULT_HISTORY_MAX_ITEMS as isize);
        db.meta.history_max_size = Some(6 * 1024 * 1024);
        db.meta.recyclebin_enabled = Some(true);
        db.meta.settings_changed = Some(now);
        db.root_mut().name = stem;
        let mut vault = Vault {
            path: path.to_path_buf(),
            db,
            key,
            dirty: true,
            original_pending: false,
            upgraded_from_kdbx3: false,
        };
        vault.ensure_recycle_bin();
        Ok(vault)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Database name from the metadata, else the file stem.
    pub fn name(&self) -> String {
        self.db
            .meta
            .database_name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| file_stem(&self.path).ok())
            .unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// True when `password` and `keyfile` make the key this vault was opened (or last re-keyed) with.
    pub fn key_matches(&self, password: &str, keyfile: Option<&Path>) -> Result<bool, VaultError> {
        Ok(build_key(password, keyfile)? == self.key)
    }

    /// Replace the master key (KeePass "Change Master Key"). Marks the vault dirty; the
    /// next `save` re-encrypts the file with the new key. The backup copies written before
    /// this call keep the old key.
    pub fn change_key(&mut self, password: &str, keyfile: Option<&Path>) -> Result<(), VaultError> {
        self.key = build_key(password, keyfile)?;
        self.db.meta.master_key_changed = Some(keepass::db::Times::now());
        self.dirty = true;
        Ok(())
    }

    /// Saves now: `start_save` followed by `SaveJob::run`. A failure leaves the existing file
    /// untouched and the vault still marked as changed.
    pub fn save(&mut self, backups: &BackupPolicy) -> Result<(), VaultError> {
        let job = self.start_save();
        let original = job.backup_original;
        job.run(backups).inspect_err(|_| self.save_failed(original))
    }

    /// Takes a copy of the database for saving and marks the vault saved. The slow part,
    /// `SaveJob::run`, needs no access to the vault, so edits can continue meanwhile. If the
    /// job fails, call `save_failed` with `SaveJob::backs_up_original`.
    pub fn start_save(&mut self) -> SaveJob {
        let job = SaveJob {
            db: self.db.clone(),
            key: self.key.clone(),
            path: self.path.clone(),
            backup_original: self.original_pending,
            upgraded_from_kdbx3: self.upgraded_from_kdbx3,
        };
        self.dirty = false;
        self.original_pending = false;
        job
    }

    /// A job from `start_save` failed: its changes still need saving, and the copy of the
    /// file as it was on disk is still owed if that job was to make it.
    pub fn save_failed(&mut self, backed_up_original: bool) {
        self.dirty = true;
        self.original_pending |= backed_up_original;
    }

    pub fn root(&self) -> GroupId {
        self.db.root().id().into()
    }

    fn recycle_bin_id(&self) -> Option<db::GroupId> {
        self.db.recycle_bin().map(|g| g.id())
    }

    /// True when `group` is the recycle bin or lies under it.
    fn in_recycle_bin(&self, group: db::GroupId) -> bool {
        self.recycle_bin_id().is_some_and(|bin| self.is_ancestor(bin, group))
    }

    /// Returns the recycle bin group, creating "Recycle Bin" under the root when the
    /// metadata names none.
    fn ensure_recycle_bin(&mut self) -> db::GroupId {
        if let Some(id) = self.recycle_bin_id() {
            return id;
        }
        let now = Times::now();
        let mut root = self.db.root_mut();
        let mut bin = root.add_group();
        bin.name = RECYCLE_BIN_NAME.to_owned();
        bin.set_icon_builtin(RECYCLE_BIN_ICON);
        bin.enable_autotype = Some(false);
        bin.enable_searching = Some(false);
        bin.is_expanded = false;
        let id = bin.id();
        self.db.meta.recyclebin_uuid = Some(id.uuid());
        self.db.meta.recyclebin_enabled = Some(true);
        self.db.meta.recyclebin_changed = Some(now);
        self.dirty = true;
        id
    }

    /// Pre-order, depth-first, root first.
    pub fn groups(&self) -> Vec<GroupView> {
        let bin = self.recycle_bin_id();
        let mut out = Vec::with_capacity(self.db.num_groups());
        let mut stack: Vec<(db::GroupId, usize)> = vec![(self.db.root().id(), 0)];
        while let Some((gid, depth)) = stack.pop() {
            let Some(g) = self.db.group(gid) else { continue };
            out.push(GroupView {
                id: g.id().into(),
                parent: g.parent().map(|p| p.id().into()),
                name: g.name.clone(),
                depth,
                is_recycle_bin: Some(g.id()) == bin,
                color: match g.custom_data.get(GROUP_COLOR_KEY).and_then(|c| c.value.as_ref()) {
                    Some(db::CustomDataValue::String(s)) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                },
            });
            let children: Vec<db::GroupId> = g.group_ids().collect();
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        out
    }

    /// Direct children only, stale last, then title A-Z.
    pub fn entries_in(&self, group: GroupId) -> Vec<EntryView> {
        let mut views: Vec<EntryView> = match self.db.group(group.into()) {
            Some(g) => g.entries().map(|e| entry_view(&e)).collect(),
            None => Vec::new(),
        };
        sort_views(&mut views);
        views
    }

    /// Every entry outside the recycle bin, stale last, then title A-Z.
    pub fn all_entries(&self) -> Vec<EntryView> {
        let mut views: Vec<EntryView> = self
            .db
            .iter_all_entries()
            .filter(|e| !self.in_recycle_bin(e.parent().id()))
            .map(|e| entry_view(&e))
            .collect();
        sort_views(&mut views);
        views
    }

    /// Every entry in the recycle bin, including inside deleted groups; stale last, then title A-Z.
    pub fn recycle_bin_entries(&self) -> Vec<EntryView> {
        let mut views: Vec<EntryView> = self
            .db
            .iter_all_entries()
            .filter(|e| self.in_recycle_bin(e.parent().id()))
            .map(|e| entry_view(&e))
            .collect();
        sort_views(&mut views);
        views
    }

    /// Sets or clears (`None`) the group's color, kept in the file so it travels with the database.
    pub fn set_group_color(&mut self, id: GroupId, color: Option<&str>) -> Result<(), VaultError> {
        let mut g = self.db.group_mut(id.into()).ok_or(VaultError::GroupNotFound(id))?;
        match color {
            Some(c) => {
                g.custom_data.insert(
                    GROUP_COLOR_KEY.to_owned(),
                    db::CustomDataItem {
                        value: Some(db::CustomDataValue::String(c.to_owned())),
                        last_modification_time: Some(Times::now()),
                    },
                );
            }
            None => {
                g.custom_data.remove(GROUP_COLOR_KEY);
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// Case-insensitive substring match over title, username, URL, notes and tags. Every
    /// whitespace-separated term must match. Stale last. The recycle bin is excluded.
    pub fn search(&self, query: &str) -> Vec<EntryView> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut views = self.all_entries();
        if terms.is_empty() {
            return views;
        }
        views.retain(|v| {
            let hay = format!(
                "{}\n{}\n{}\n{}\n{}",
                v.title,
                v.username,
                v.url,
                v.notes,
                v.tags.join("\n")
            )
            .to_lowercase();
            terms.iter().all(|t| hay.contains(t.as_str()))
        });
        views
    }

    pub fn entry(&self, id: EntryId) -> Option<EntryView> {
        self.db.entry(id.into()).map(|e| entry_view(&e))
    }

    /// Entries whose URL host equals `host` (case-insensitive, `www.` stripped) or whose
    /// title contains it. Stale last. The recycle bin is excluded.
    pub fn find_by_host(&self, host: &str) -> Vec<EntryView> {
        let wanted = normalize_host(host);
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut views = self.all_entries();
        views.retain(|v| {
            url_host(&v.url).as_deref() == Some(wanted.as_str()) || v.title.to_lowercase().contains(&wanted)
        });
        views
    }

    /// Creates or updates an entry. Sets dirty; creation time on insert, modification time
    /// on every call. Updates keep the previous version in the entry history: one version for
    /// the whole edit, attachments added and removed included.
    pub fn upsert_entry(&mut self, mut edit: EntryEdit) -> Result<EntryId, VaultError> {
        let group: db::GroupId = edit.group.into();
        if self.db.group(group).is_none() {
            return Err(VaultError::GroupNotFound(edit.group));
        }
        let id: db::EntryId = match edit.id {
            Some(id) => {
                let id: db::EntryId = id.into();
                let existing = self.db.entry(id).ok_or(VaultError::EntryNotFound(id.into()))?;
                if let Some(missing) = edit.detach.iter().find(|n| existing.attachment_by_name(n).is_none()) {
                    return Err(VaultError::AttachmentNotFound(missing.clone()));
                }
                id
            }
            None => {
                if let Some(missing) = edit.detach.first() {
                    return Err(VaultError::AttachmentNotFound(missing.clone()));
                }
                self.db.group_mut(group).map(|mut g| g.add_entry().id()).ok_or(VaultError::GroupNotFound(edit.group))?
            }
        };
        let attach = std::mem::take(&mut edit.attach);
        let mut entry = self.db.entry_mut(id).ok_or(VaultError::EntryNotFound(id.into()))?;
        let is_update = edit.id.is_some();
        let mut tracked = entry.track_changes();
        {
            let mut e = tracked.as_mut();
            // Files removed, or replaced under the same name, stay in the database for the
            // history version this edit creates (and older ones).
            let replaced: Vec<String> = attach
                .iter()
                .filter(|(n, _)| e.as_ref().attachment_by_name(n).is_some())
                .map(|(n, _)| n.clone())
                .collect();
            let dropped: Vec<&String> = edit.detach.iter().chain(&replaced).collect();
            detach_keeping_data(&mut e, &dropped)?;
            if e.parent_mut().id() != group {
                e.move_to(group).map_err(|_| VaultError::GroupNotFound(edit.group))?;
                e.times.location_changed = Some(Times::now());
            }
            e.set_unprotected(fields::TITLE, edit.title.as_str());
            e.set_unprotected(fields::USERNAME, edit.username.as_str());
            e.set_protected(fields::PASSWORD, edit.password.as_str());
            e.set_unprotected(fields::URL, edit.url.as_str());
            e.set_unprotected(fields::NOTES, edit.notes.as_str());
            set_or_remove(&mut e, FIELD_STALE, if edit.stale { "1" } else { "" }, false);
            set_or_remove(&mut e, FIELD_RECOVERY_PIN, &edit.recovery_pin, true);
            set_or_remove(&mut e, FIELD_RECOVERY_PHONE, &edit.recovery_phone, false);
            set_or_remove(&mut e, FIELD_SECURITY_QUESTIONS, &edit.security_questions, true);
            set_or_remove(&mut e, FIELD_KIND, if edit.document { KIND_DOCUMENT } else { "" }, false);

            let keep: Vec<&str> = edit.custom.iter().map(|c| c.key.as_str()).collect();
            e.fields
                .retain(|k, _| RESERVED_FIELDS.contains(&k.as_str()) || keep.contains(&k.as_str()));
            for c in &edit.custom {
                if RESERVED_FIELDS.contains(&c.key.as_str()) || c.key.is_empty() {
                    continue;
                }
                if c.protected {
                    e.set_protected(c.key.as_str(), c.value.as_str());
                } else {
                    e.set_unprotected(c.key.as_str(), c.value.as_str());
                }
            }

            e.tags = edit.tags.iter().filter(|t| !t.is_empty()).cloned().collect();

            // An entry without Auto-Type settings gets KeePass's defaults (enabled); an existing
            // `Enabled = False` from KeePass stays as it is.
            let mut at = e.autotype.take().unwrap_or_else(|| AutoType { enabled: true, ..AutoType::default() });
            at.default_sequence = edit.autotype_sequence.clone().filter(|s| !s.is_empty());
            let previous: HashMap<String, String> = at
                .associations
                .drain(..)
                .map(|a| (a.window, a.sequence))
                .collect();
            at.associations = edit
                .autotype_windows
                .iter()
                .filter(|w| !w.is_empty())
                .map(|w| AutoTypeAssociation {
                    window: w.clone(),
                    sequence: previous.get(w).cloned().unwrap_or_default(),
                })
                .collect();
            e.autotype = Some(at);

            for (name, bytes) in attach {
                e.add_attachment(name, Value::protected(bytes));
            }

            let now = Times::now();
            e.times.last_modification = Some(now);
            e.times.last_access = Some(now);
        }
        drop(tracked);
        if is_update {
            // Rebuilds the attachment references (history versions now use the detached
            // files) and drops what nothing uses.
            self.db.remove_unused_attachments();
            let max = self
                .db
                .meta
                .history_max_items
                .filter(|n| *n >= 0)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_HISTORY_MAX_ITEMS);
            trim_history(&mut self.db, id, max);
        } else if let Some(mut e) = self.db.entry_mut(id) {
            // A freshly created entry has no previous version worth keeping.
            e.history = Some(History::default());
        }
        self.dirty = true;
        Ok(id.into())
    }

    /// Moves the entry to the recycle bin (creating "Recycle Bin" like KeePass). Entries
    /// already in the bin are deleted permanently, as are all entries when the bin is
    /// disabled in the database metadata.
    pub fn delete_entry(&mut self, id: EntryId) -> Result<(), VaultError> {
        let kid: db::EntryId = id.into();
        let parent = self
            .db
            .entry(kid)
            .map(|e| e.parent().id())
            .ok_or(VaultError::EntryNotFound(id))?;
        if !self.recycle_bin_enabled() || self.in_recycle_bin(parent) {
            return self.remove_entry(id);
        }
        let bin = self.ensure_recycle_bin();
        let mut entry = self.db.entry_mut(kid).ok_or(VaultError::EntryNotFound(id))?;
        entry.move_to(bin).map_err(|_| VaultError::GroupNotFound(bin.into()))?;
        entry.times.location_changed = Some(Times::now());
        self.dirty = true;
        Ok(())
    }

    /// Deletes an entry for good, wherever it is (undoing the creation of a new entry).
    pub fn remove_entry(&mut self, id: EntryId) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        entry.track_changes().remove();
        self.dirty = true;
        Ok(())
    }

    /// Puts history version `index` (0 = the newest) back (KeePass "Restore"). The current
    /// version goes into the history first, so nothing is lost. With `keep_folder` the entry
    /// stays in the folder it is in now; otherwise it goes back to the folder that version was
    /// in (Undo of a save that moved it), which must still exist.
    pub fn restore_version(&mut self, id: EntryId, index: usize, keep_folder: bool) -> Result<(), VaultError> {
        let kid: db::EntryId = id.into();
        let (previous, version_folder) = {
            let entry = self.db.entry(kid).ok_or(VaultError::EntryNotFound(id))?;
            let previous = entry.history.as_ref().and_then(|h| h.get_entries().get(index)).cloned().ok_or(VaultError::NoEarlierVersion(id))?;
            (previous, version_folder(&entry, index))
        };
        // The version's own record names its folder, so the entry passes through it.
        let version_folder = version_folder.ok_or(VaultError::VersionFolderGone(id))?;
        let mut entry = self.db.entry_mut(kid).ok_or(VaultError::EntryNotFound(id))?;
        let mut tracked = entry.track_changes();
        let current_folder = tracked.as_mut().parent_mut().id();
        let moved = !keep_folder && current_folder != version_folder;
        if moved {
            tracked.move_to(version_folder).map_err(|_| VaultError::GroupNotFound(version_folder.into()))?;
        }
        let mut restored = previous;
        restored.history = tracked.history.take();
        let now = Times::now();
        restored.times.last_modification = Some(now);
        if moved {
            restored.times.location_changed = Some(now);
        }
        *tracked = restored;
        if keep_folder && current_folder != version_folder {
            // The restored record says `version_folder`; the entry is listed in `current_folder`.
            tracked.as_mut().move_to(current_folder).map_err(|_| VaultError::GroupNotFound(current_folder.into()))?;
        }
        drop(tracked);
        let max = self.history_max_items();
        trim_history(&mut self.db, kid, max);
        // The entry now uses the old version's attachments again; rebuild the references.
        self.db.remove_unused_attachments();
        self.dirty = true;
        Ok(())
    }

    /// Deletes history version `index` (0 = the newest). Files only that version used go.
    pub fn delete_version(&mut self, id: EntryId, index: usize) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        let versions = entry.history.as_ref().map(|h| h.get_entries().clone()).unwrap_or_default();
        if index >= versions.len() {
            return Err(VaultError::NoEarlierVersion(id));
        }
        let mut history = History::default();
        // `add_entry` puts each version first: add the oldest first.
        for (_, version) in versions.into_iter().enumerate().rev().filter(|(i, _)| *i != index) {
            history.add_entry(version);
        }
        entry.history = Some(history);
        self.db.remove_unused_attachments();
        self.dirty = true;
        Ok(())
    }

    /// Number of earlier versions kept for the entry.
    pub fn history_len(&self, id: EntryId) -> usize {
        self.db
            .entry(id.into())
            .and_then(|e| e.history.as_ref().map(|h| h.get_entries().len()))
            .unwrap_or(0)
    }

    /// The entry's earlier versions, newest first, each with what the next newer version
    /// (or the current one) changed.
    pub fn versions(&self, id: EntryId) -> Vec<VersionInfo> {
        let Some(entry) = self.db.entry(id.into()) else { return Vec::new() };
        let group: GroupId = entry.parent().id().into();
        let count = entry.history.as_ref().map_or(0, |h| h.get_entries().len());
        let mut newer = (entry_view(&entry), files_of(&entry));
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let Some(version) = entry.historical(index) else { break };
            let older = (entry_view_in(&version, group), files_of(&version));
            out.push(VersionInfo {
                modified: older.0.modified,
                changes: describe_changes(&changes(&older, &newer)),
            });
            newer = older;
        }
        out
    }

    /// History version `index` of the entry (0 = the newest), and which fields differ from
    /// the current version (keys as in `Change::key`). Its `group` is the entry's current
    /// folder: keepass cannot read an old version's folder once that folder is gone.
    pub fn version(&self, id: EntryId, index: usize) -> Option<(EntryView, Vec<String>)> {
        let entry = self.db.entry(id.into())?;
        let version = entry.historical(index)?;
        let current = (entry_view(&entry), files_of(&entry));
        let old = (entry_view_in(&version, entry.parent().id().into()), files_of(&version));
        let mut differs: Vec<String> = changes(&old, &current).iter().map(Change::key).collect();
        differs.dedup();
        Some((old.0, differs))
    }

    /// The bytes of a file of history version `index`.
    pub fn version_attachment(&self, id: EntryId, index: usize, name: &str) -> Option<Vec<u8>> {
        let entry = self.db.entry(id.into())?;
        let version = entry.historical(index)?;
        let att = version.attachment_by_name(name)?;
        Some(att.data.get().clone())
    }

    /// Size in bytes of a file of history version `index`.
    pub fn version_attachment_size(&self, id: EntryId, index: usize, name: &str) -> Option<u64> {
        let entry = self.db.entry(id.into())?;
        let version = entry.historical(index)?;
        let att = version.attachment_by_name(name)?;
        Some(att.data.get().len() as u64)
    }

    /// Versions kept per entry (KDBX metadata, so it travels with the file); 10 when unset.
    pub fn history_max_items(&self) -> usize {
        self.db
            .meta
            .history_max_items
            .filter(|n| *n >= 0)
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_HISTORY_MAX_ITEMS)
    }

    pub fn set_stale(&mut self, id: EntryId, stale: bool) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        let mut tracked = entry.track_changes();
        {
            let mut e = tracked.as_mut();
            set_or_remove(&mut e, FIELD_STALE, if stale { "1" } else { "" }, false);
            e.times.last_modification = Some(Times::now());
        }
        drop(tracked);
        let max = self
            .db
            .meta
            .history_max_items
            .filter(|n| *n >= 0)
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_HISTORY_MAX_ITEMS);
        trim_history(&mut self.db, id.into(), max);
        self.dirty = true;
        Ok(())
    }

    /// Turns an entry into a document entry (text and files) or back into an account.
    pub fn set_document(&mut self, id: EntryId, document: bool) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        set_or_remove(&mut entry, FIELD_KIND, if document { KIND_DOCUMENT } else { "" }, false);
        entry.times.last_modification = Some(Times::now());
        self.dirty = true;
        Ok(())
    }

    /// Size in bytes of one attachment of an entry.
    pub fn attachment_size(&self, id: EntryId, name: &str) -> Option<u64> {
        let entry = self.db.entry(id.into())?;
        let att = entry.attachment_by_name(name)?;
        Some(att.data.get().len() as u64)
    }

    /// Bytes of all attachments stored in the file. Two entries holding identical files count
    /// twice: the `keepass` crate stores each attachment separately.
    pub fn attachments_total_bytes(&self) -> u64 {
        self.db.iter_all_attachments().map(|a| a.data.get().len() as u64).sum()
    }

    /// Fails when a sibling folder already has this name (ignoring case), so two folders
    /// with the same name never sit side by side.
    fn check_group_name(&self, parent: db::GroupId, name: &str, except: Option<db::GroupId>) -> Result<(), VaultError> {
        let p = self.db.group(parent).ok_or(VaultError::GroupNotFound(parent.into()))?;
        let taken = p.group_ids().filter(|c| Some(*c) != except).any(|c| {
            self.db.group(c).is_some_and(|g| g.name.trim().to_lowercase() == name.to_lowercase())
        });
        if taken {
            return Err(VaultError::DuplicateGroupName(name.to_owned()));
        }
        Ok(())
    }

    pub fn add_group(&mut self, parent: GroupId, name: &str) -> Result<GroupId, VaultError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(VaultError::EmptyGroupName);
        }
        self.check_group_name(parent.into(), name, None)?;
        let mut p = self
            .db
            .group_mut(parent.into())
            .ok_or(VaultError::GroupNotFound(parent))?;
        let mut g = p.add_group();
        g.name = name.to_owned();
        let id = g.id();
        self.dirty = true;
        Ok(id.into())
    }

    pub fn rename_group(&mut self, id: GroupId, name: &str) -> Result<(), VaultError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(VaultError::EmptyGroupName);
        }
        let parent = self.db.group(id.into()).ok_or(VaultError::GroupNotFound(id))?.parent().map(|p| p.id());
        if let Some(parent) = parent {
            self.check_group_name(parent, name, Some(id.into()))?;
        }
        let mut g = self.db.group_mut(id.into()).ok_or(VaultError::GroupNotFound(id))?;
        if g.name == name {
            return Ok(());
        }
        g.name = name.to_owned();
        g.times.last_modification = Some(Times::now());
        self.dirty = true;
        Ok(())
    }

    /// Number of entries directly in `group` (subgroups not counted).
    pub fn entry_count(&self, group: GroupId) -> usize {
        self.db.group(group.into()).map_or(0, |g| g.entries().count())
    }

    /// False when the database metadata turns the recycle bin off; deletes are then permanent.
    pub fn recycle_bin_enabled(&self) -> bool {
        self.db.meta.recyclebin_enabled != Some(false)
    }

    /// Moves the group, with all its subgroups and entries, to the recycle bin (creating
    /// "Recycle Bin" like KeePass). A group already in the bin, the bin itself, and every
    /// group when the bin is disabled are deleted permanently.
    pub fn delete_group(&mut self, id: GroupId) -> Result<(), VaultError> {
        let gid: db::GroupId = id.into();
        if self.db.group(gid).is_none() {
            return Err(VaultError::GroupNotFound(id));
        }
        if gid == self.db.root().id() {
            return Err(VaultError::CannotDeleteRoot);
        }
        if !self.recycle_bin_enabled() || self.in_recycle_bin(gid) {
            let mut group = self.db.group_mut(gid).ok_or(VaultError::GroupNotFound(id))?;
            group.track_changes().remove().map_err(|_| VaultError::CannotDeleteRoot)?;
        } else {
            // The bin may sit inside this group; moving the group into it would be a cycle.
            if self.recycle_bin_id().is_some_and(|bin| self.is_ancestor(gid, bin)) {
                return Err(VaultError::GroupHoldsRecycleBin);
            }
            let bin = self.ensure_recycle_bin();
            let mut group = self.db.group_mut(gid).ok_or(VaultError::GroupNotFound(id))?;
            group.track_changes().move_to(bin).map_err(|_| VaultError::GroupNotFound(bin.into()))?;
        }
        self.dirty = true;
        Ok(())
    }

    /// True when `ancestor` is `group` or one of its parents.
    fn is_ancestor(&self, ancestor: db::GroupId, group: db::GroupId) -> bool {
        let mut cur = Some(group);
        while let Some(id) = cur {
            if id == ancestor {
                return true;
            }
            cur = self.db.group(id).and_then(|g| g.parent().map(|p| p.id()));
        }
        false
    }

    pub fn attachment(&self, id: EntryId, name: &str) -> Option<Vec<u8>> {
        let entry = self.db.entry(id.into())?;
        let att = entry.attachment_by_name(name)?;
        Some(att.data.get().clone())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

fn normalize_host(host: &str) -> String {
    let h = host.trim().to_lowercase();
    let h = h.strip_prefix("www.").unwrap_or(&h);
    h.trim_end_matches('.').to_owned()
}

/// Title for an entry saved without a name: "Untitled", or "Untitled 2", "Untitled 3"… when
/// `taken` (the database's titles) already has it, ignoring case.
pub fn untitled_name(taken: &[String]) -> String {
    let is_taken = |name: &str| taken.iter().any(|t| t.trim().eq_ignore_ascii_case(name));
    let mut name = "Untitled".to_owned();
    let mut n = 2;
    while is_taken(&name) {
        name = format!("Untitled {n}");
        n += 1;
    }
    name
}

/// `"https://www.Foo.com/x"` → `"foo.com"`; a bare `"foo.com"` is accepted. Userinfo and
/// port are dropped; the leading `www.` is stripped. Returns `None` for an empty or
/// whitespace-containing host.
pub fn url_host(url: &str) -> Option<String> {
    let s = url.trim();
    if s.is_empty() {
        return None;
    }
    let rest = match s.find("://") {
        Some(i) => &s[i + 3..],
        None => s,
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let mut authority = &rest[..end];
    if let Some(i) = authority.rfind('@') {
        authority = &authority[i + 1..];
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return None;
    }
    let host = normalize_host(host);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn sanitize_stem(stem: &str) -> String {
    let s: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if s.is_empty() {
        "db".to_owned()
    } else {
        s
    }
}

/// FNV-1a over the absolute path. Stable across Rust releases and platforms, unlike
/// `DefaultHasher`. The path is not canonicalized: canonicalization depends on whether the
/// file exists yet and on Windows adds the `\\?\` prefix, which would change the hash
/// between the first save and later ones. Windows paths are compared case-insensitively,
/// so they are lowercased.
fn path_hash(db_path: &Path) -> u32 {
    let abs = std::path::absolute(db_path).unwrap_or_else(|_| db_path.to_path_buf());
    let mut text = abs.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h ^ (h >> 32)) as u32
}

/// `<policy.dir>/<sanitized stem>-<8 hex of path hash>`.
fn backup_subdir(policy: &BackupPolicy, db_path: &Path) -> Result<(PathBuf, String), VaultError> {
    let stem = sanitize_stem(&file_stem(db_path)?);
    let dir = policy.dir.join(format!("{}-{:08x}", stem, path_hash(db_path)));
    Ok((dir, stem))
}

/// `<stem>-YYYYMMDD-HHMMSS.kdbx` for `at`, moved one second later while that name is taken.
/// Names have one-second resolution: the pre-save copy of a file written in the same second
/// as the save (a file copied in just before opening, or two saves in one second) would
/// otherwise be overwritten by the next backup.
fn backup_file_path(policy: &BackupPolicy, db_path: &Path, at: DateTime<Local>) -> Result<PathBuf, VaultError> {
    let (dir, stem) = backup_subdir(policy, db_path)?;
    let mut at = at;
    loop {
        let path = dir.join(format!("{}-{}.{}", stem, at.format("%Y%m%d-%H%M%S"), BACKUP_EXT));
        if !path.exists() {
            return Ok(path);
        }
        at += chrono::Duration::seconds(1);
    }
}

/// A copy of a vault taken by `Vault::start_save`; `run` writes it.
pub struct SaveJob {
    db: Database,
    key: DatabaseKey,
    path: PathBuf,
    backup_original: bool,
    upgraded_from_kdbx3: bool,
}

impl SaveJob {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pass to `Vault::save_failed` when `run` fails.
    pub fn backs_up_original(&self) -> bool {
        self.backup_original
    }

    /// Encrypts once into memory, then writes `<path>.tmp` → fsync → rename over `<path>`,
    /// and the same bytes into the backup folder, so the file is never read back (a full
    /// read on a network drive costs as much as the write). Rotates the backups.
    /// A failure before the rename leaves the existing file untouched.
    pub fn run(self, backups: &BackupPolicy) -> Result<(), VaultError> {
        let mut bytes = Vec::new();
        self.db.save(&mut bytes, self.key.clone())?;
        let tmp = tmp_path(&self.path);
        let staged = write_synced(&tmp, &bytes).and_then(|()| {
            if self.backup_original {
                backup_original(backups, &self.path, self.upgraded_from_kdbx3)?;
            }
            Ok(())
        });
        if let Err(e) = staged {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        let backup_target = backup_file_path(backups, &self.path, Local::now())?;
        if let Err(rename_err) = fs::rename(&tmp, &self.path) {
            // Windows refuses the replacing rename while another process holds the file
            // open without FILE_SHARE_DELETE. The backup is written first so the new content
            // exists outside the tmp file before the original is removed.
            let recoverable = cfg!(windows) && self.path.exists();
            if !recoverable {
                let _ = fs::remove_file(&tmp);
                return Err(io_err(&self.path, rename_err));
            }
            write_backup(&bytes, &backup_target)?;
            fs::remove_file(&self.path)
                .and_then(|_| fs::rename(&tmp, &self.path))
                .map_err(|e| io_err(&self.path, e))?;
        } else {
            write_backup(&bytes, &backup_target)?;
        }
        #[cfg(unix)]
        if let Some(dir) = self.path.parent() {
            if let Ok(d) = File::open(dir) {
                let _ = d.sync_all();
            }
        }
        rotate_backups_at(backups, &self.path, Local::now())?;
        Ok(())
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let mut file = File::create(path).map_err(|e| io_err(path, e))?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| io_err(path, e))
}

/// Copies the file currently on disk into the backup folder before the session's first
/// save replaces it, so edits made by other apps are kept. The copy is named after the
/// file's modification time and rotates like any other backup. A file opened as KDBX 3.1
/// also gets `<stem>-kdbx3-original.kdbx`, which rotation never deletes; it is written once.
fn backup_original(backups: &BackupPolicy, path: &Path, upgraded_from_kdbx3: bool) -> Result<(), VaultError> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(()); // nothing on disk yet
    };
    let mtime: DateTime<Local> = meta.modified().map(DateTime::from).unwrap_or_else(|_| Local::now());
    copy_backup(path, &backup_file_path(backups, path, mtime)?)?;
    if upgraded_from_kdbx3 {
        let (dir, stem) = backup_subdir(backups, path)?;
        let keep = dir.join(format!("{stem}-kdbx3-original.{BACKUP_EXT}"));
        if !keep.exists() {
            copy_backup(path, &keep)?;
        }
    }
    Ok(())
}

fn backup_err(to: &Path) -> impl Fn(std::io::Error) -> VaultError + '_ {
    move |e| VaultError::Backup { path: to.to_path_buf(), source: e }
}

fn copy_backup(from: &Path, to: &Path) -> Result<(), VaultError> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(backup_err(to))?;
    }
    fs::copy(from, to).map_err(backup_err(to))?;
    Ok(())
}

fn write_backup(bytes: &[u8], to: &Path) -> Result<(), VaultError> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(backup_err(to))?;
    }
    fs::write(to, bytes).map_err(backup_err(to))
}

/// Timestamp parsed from `<stem>-YYYYMMDD-HHMMSS.kdbx`.
fn backup_timestamp(stem: &str, file_name: &str) -> Option<NaiveDateTime> {
    let rest = file_name.strip_prefix(stem)?.strip_prefix('-')?;
    let rest = rest.strip_suffix(&format!(".{}", BACKUP_EXT))?;
    NaiveDateTime::parse_from_str(rest, "%Y%m%d-%H%M%S").ok()
}

/// Backup files with their timestamps, newest first.
fn backups_with_times(policy: &BackupPolicy, db_path: &Path) -> Vec<(NaiveDateTime, PathBuf)> {
    let Ok((dir, stem)) = backup_subdir(policy, db_path) else {
        return Vec::new();
    };
    let Ok(read) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(NaiveDateTime, PathBuf)> = read
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name();
            let ts = backup_timestamp(&stem, name.to_str()?)?;
            Some((ts, e.path()))
        })
        .collect();
    out.sort_by(|a, b| b.cmp(a));
    out
}

/// Newest first.
pub fn list_backups(policy: &BackupPolicy, db_path: &Path) -> Vec<PathBuf> {
    backups_with_times(policy, db_path).into_iter().map(|(_, p)| p).collect()
}

/// Keeps the newest `keep_recent`; among older files keeps the newest one per calendar day
/// for the last `keep_daily_days` days; deletes the rest. Returns the deleted paths.
pub fn rotate_backups(policy: &BackupPolicy, db_path: &Path) -> Result<Vec<PathBuf>, VaultError> {
    rotate_backups_at(policy, db_path, Local::now())
}

fn rotate_backups_at(policy: &BackupPolicy, db_path: &Path, now: DateTime<Local>) -> Result<Vec<PathBuf>, VaultError> {
    let files = backups_with_times(policy, db_path);
    let oldest_daily: NaiveDate = now.date_naive() - chrono::Duration::days(policy.keep_daily_days.max(0));
    let mut deleted = Vec::new();
    let mut last_day_kept: Option<NaiveDate> = None;
    for (ts, path) in files.into_iter().skip(policy.keep_recent) {
        let day = ts.date();
        let within_window = day >= oldest_daily;
        let first_of_day = last_day_kept != Some(day);
        if within_window && first_of_day {
            last_day_kept = Some(day);
            continue;
        }
        fs::remove_file(&path).map_err(|e| VaultError::Backup {
            path: path.clone(),
            source: e,
        })?;
        deleted.push(path);
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    const PW: &str = "correct horse";

    fn policy(dir: &Path) -> BackupPolicy {
        BackupPolicy {
            dir: dir.join("backups"),
            keep_recent: 20,
            keep_daily_days: 30,
        }
    }

    fn edit(group: GroupId, title: &str) -> EntryEdit {
        EntryEdit {
            id: None,
            group,
            title: title.to_owned(),
            username: format!("{}@example.com", title.to_lowercase()),
            password: format!("pw-{}", title),
            url: String::new(),
            notes: String::new(),
            tags: Vec::new(),
            stale: false,
            recovery_pin: String::new(),
            recovery_phone: String::new(),
            security_questions: String::new(),
            custom: Vec::new(),
            autotype_sequence: None,
            autotype_windows: Vec::new(),
            document: false,
            attach: Vec::new(),
            detach: Vec::new(),
        }
    }

    /// The entry as it is now, ready to change and save again.
    fn edit_of(v: &Vault, id: EntryId) -> EntryEdit {
        let e = v.entry(id).unwrap();
        EntryEdit {
            id: Some(id),
            group: e.group,
            title: e.title,
            username: e.username,
            password: e.password,
            url: e.url,
            notes: e.notes,
            tags: e.tags,
            stale: e.stale,
            recovery_pin: e.recovery_pin,
            recovery_phone: e.recovery_phone,
            security_questions: e.security_questions,
            custom: e.custom,
            autotype_sequence: e.autotype_sequence,
            autotype_windows: e.autotype_windows.into_iter().map(|w| w.window).collect(),
            document: e.document,
            attach: Vec::new(),
            detach: Vec::new(),
        }
    }

    fn history_len(v: &Vault, id: EntryId) -> usize {
        v.db.entry(id.into()).unwrap().history.as_ref().map_or(0, |h| h.get_entries().len())
    }

    /// KDBX 3.1 fixture from keepass-rs (MIT), password "demopass", two entries with attachments.
    #[test]
    fn kdbx3_wrong_password_and_upgrade_on_save() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("old.kdbx");
        fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kdbx3_with_attachments.kdbx"), &path).unwrap();
        let pol = policy(tmp.path());

        assert!(matches!(Vault::open(&path, "wrong", None), Err(VaultError::WrongKey)));

        let mut v = Vault::open(&path, "demopass", None).unwrap();
        let before: Vec<(String, Vec<String>, Vec<Vec<u8>>)> = v
            .all_entries()
            .into_iter()
            .map(|e| {
                let data = e.attachments.iter().map(|a| v.attachment(e.id, a).unwrap()).collect();
                (e.title, e.attachments, data)
            })
            .collect();
        assert!(before.iter().any(|(_, a, _)| !a.is_empty()), "fixture should have attachments");
        let root = v.root();
        v.upsert_entry(edit(root, "Added")).unwrap();
        let original = fs::read(&path).unwrap();
        v.save(&pol).unwrap();
        v.upsert_entry(edit(root, "Second")).unwrap();
        v.save(&pol).unwrap();
        let (dir, stem) = backup_subdir(&pol, &path).unwrap();
        let kept = dir.join(format!("{stem}-kdbx3-original.{BACKUP_EXT}"));
        assert_eq!(fs::read(&kept).unwrap(), original, "KDBX 3.1 original must be kept byte for byte");
        assert!(
            list_backups(&pol, &path).iter().any(|p| fs::read(p).unwrap() == original),
            "pre-save copy must be in the rotating backups"
        );

        let mut header = [0u8; 12];
        std::io::Read::read_exact(&mut File::open(&path).unwrap(), &mut header).unwrap();
        assert_eq!(u16::from_le_bytes([header[10], header[11]]), 4, "saved file must be KDBX 4");

        let v2 = Vault::open(&path, "demopass", None).unwrap();
        for (title, names, data) in &before {
            let e = v2.all_entries().into_iter().find(|e| &e.title == title).unwrap();
            assert_eq!(&e.attachments, names);
            let got: Vec<Vec<u8>> = names.iter().map(|a| v2.attachment(e.id, a).unwrap()).collect();
            assert_eq!(&got, data);
        }
        assert!(v2.all_entries().iter().any(|e| e.title == "Added"));
    }

    /// KDBX 4.0 fixture from keepass-rs (MIT), password "demopass". keepass 0.17 reads 4.0
    /// but only writes 4.1; saving used to fail with "Unsupported database version".
    #[test]
    fn kdbx40_saves_as_kdbx41() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("v40.kdbx");
        fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kdbx40_argon2.kdbx"), &path).unwrap();
        let header = |p: &Path| {
            let mut h = [0u8; 12];
            std::io::Read::read_exact(&mut File::open(p).unwrap(), &mut h).unwrap();
            (u16::from_le_bytes([h[10], h[11]]), u16::from_le_bytes([h[8], h[9]]))
        };
        assert_eq!(header(&path), (4, 0), "fixture must be KDBX 4.0");

        let mut v = Vault::open(&path, "demopass", None).unwrap();
        let root = v.root();
        v.upsert_entry(edit(root, "Added")).unwrap();
        v.save(&policy(tmp.path())).unwrap();

        assert_eq!(header(&path), (4, 1));
        let v2 = Vault::open(&path, "demopass", None).unwrap();
        assert!(v2.all_entries().iter().any(|e| e.title == "Added"));
    }

    #[test]
    fn change_key_locks_out_old_password() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("k.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        v.upsert_entry(edit(root, "Bank")).unwrap();
        v.save(&pol).unwrap();
        v.change_key("new pass", None).unwrap();
        assert!(v.is_dirty());
        v.save(&pol).unwrap();
        assert!(Vault::open(&path, PW, None).is_err());
        let v2 = Vault::open(&path, "new pass", None).unwrap();
        assert_eq!(v2.all_entries().len(), 1);
    }

    #[test]
    fn create_save_reopen_roundtrip() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("Family Vault.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        assert!(v.is_dirty());
        assert_eq!(v.name(), "Family Vault");
        let root = v.root();
        let groups = v.groups();
        assert_eq!(groups[0].id, root);
        assert_eq!(groups[0].name, "Family Vault");
        assert_eq!(groups[0].depth, 0);
        assert!(groups.iter().any(|g| g.is_recycle_bin && g.name == "Recycle Bin" && g.depth == 1));

        let mut rich = edit(root, "Bank");
        rich.url = "https://www.MyBank.com/login".into();
        rich.notes = "line one\nline two".into();
        rich.tags = vec!["money".into(), "important".into()];
        rich.recovery_pin = "1234".into();
        rich.recovery_phone = "+1 555 0100".into();
        rich.security_questions = "Q: pet?\nA: rex".into();
        rich.custom = vec![
            CustomField { key: "Account No".into(), value: "987".into(), protected: false },
            CustomField { key: "Secret Word".into(), value: "swordfish".into(), protected: true },
        ];
        rich.autotype_sequence = Some("{USERNAME}{ENTER}{DELAY 500}{PASSWORD}{ENTER}".into());
        rich.autotype_windows = vec!["*MyBank*".into(), "//Bank.*//".into()];
        rich.attach = vec![("card.txt".into(), b"front: 1234".to_vec())];
        let bank = v.upsert_entry(rich).unwrap();

        let mut old = edit(root, "Amazon");
        old.stale = true;
        let amazon_old = v.upsert_entry(old).unwrap();
        let zoo = v.upsert_entry(edit(root, "zoo tickets")).unwrap();

        v.save(&pol).unwrap();
        assert!(!v.is_dirty());
        assert!(path.exists());
        assert!(!tmp_path(&path).exists());
        assert_eq!(list_backups(&pol, &path).len(), 1);

        let r = Vault::open(&path, PW, None).unwrap();
        assert!(!r.is_dirty());
        assert_eq!(r.name(), "Family Vault");
        let b = r.entry(bank).unwrap();
        assert_eq!(b.group, root);
        assert_eq!(b.title, "Bank");
        assert_eq!(b.username, "bank@example.com");
        assert_eq!(b.password, "pw-Bank");
        assert_eq!(b.url, "https://www.MyBank.com/login");
        assert_eq!(b.notes, "line one\nline two");
        assert_eq!(b.tags, vec!["money".to_string(), "important".to_string()]);
        assert!(!b.stale);
        assert_eq!(b.recovery_pin, "1234");
        assert_eq!(b.recovery_phone, "+1 555 0100");
        assert_eq!(b.security_questions, "Q: pet?\nA: rex");
        assert_eq!(
            b.custom,
            vec![
                CustomField { key: "Account No".into(), value: "987".into(), protected: false },
                CustomField { key: "Secret Word".into(), value: "swordfish".into(), protected: true },
            ]
        );
        assert_eq!(b.attachments, vec!["card.txt".to_string()]);
        assert_eq!(r.attachment(bank, "card.txt").unwrap(), b"front: 1234");
        assert_eq!(b.autotype_sequence.as_deref(), Some("{USERNAME}{ENTER}{DELAY 500}{PASSWORD}{ENTER}"));
        let windows: Vec<&str> = b.autotype_windows.iter().map(|w| w.window.as_str()).collect();
        assert_eq!(windows, vec!["*MyBank*", "//Bank.*//"]);
        assert!(b.autotype_enabled);
        assert!(b.modified > Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap());

        let a = r.entry(amazon_old).unwrap();
        assert!(a.stale);
        assert!(a.autotype_sequence.is_none());
        assert!(a.autotype_windows.is_empty());
        assert!(a.custom.is_empty());
        assert!(a.attachments.is_empty());

        // Stale last, then title A-Z.
        let titles: Vec<String> = r.entries_in(root).into_iter().map(|e| e.title).collect();
        assert_eq!(titles, vec!["Bank", "zoo tickets", "Amazon"]);
        assert_eq!(r.all_entries().len(), 3);
        assert_eq!(r.entry(zoo).unwrap().title, "zoo tickets");

        // Protected flags survive in the file.
        let raw = r.db.entry(bank.into()).unwrap();
        assert!(raw.fields[fields::PASSWORD].is_protected());
        assert!(raw.fields[FIELD_RECOVERY_PIN].is_protected());
        assert!(raw.fields[FIELD_SECURITY_QUESTIONS].is_protected());
        assert!(!raw.fields[FIELD_RECOVERY_PHONE].is_protected());
    }

    #[test]
    fn update_keeps_id_and_times() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("t.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let id = v.upsert_entry(edit(root, "One")).unwrap();
        let created = v.db.entry(id.into()).unwrap().times.creation;
        assert!(created.is_some());
        let sub = v.add_group(root, "Sub").unwrap();
        let mut e = edit(sub, "One renamed");
        e.id = Some(id);
        e.custom = vec![CustomField { key: "k".into(), value: "v".into(), protected: false }];
        let id2 = v.upsert_entry(e).unwrap();
        assert_eq!(id, id2);
        let view = v.entry(id).unwrap();
        assert_eq!(view.title, "One renamed");
        assert_eq!(view.group, sub);
        assert_eq!(v.entries_in(root).len(), 0);
        assert_eq!(v.entries_in(sub).len(), 1);
        let raw = v.db.entry(id.into()).unwrap();
        assert_eq!(raw.times.creation, created);
        assert_eq!(raw.history.as_ref().unwrap().get_entries().len(), 1);
        // Removing the custom field drops it from the entry.
        let mut e = edit(sub, "One renamed");
        e.id = Some(id);
        v.upsert_entry(e).unwrap();
        assert!(v.entry(id).unwrap().custom.is_empty());
        for _ in 0..15 {
            let mut e = edit(sub, "One renamed");
            e.id = Some(id);
            v.upsert_entry(e).unwrap();
        }
        assert_eq!(v.db.entry(id.into()).unwrap().history.as_ref().unwrap().get_entries().len(), 10);
        v.rename_group(sub, "Renamed").unwrap();
        assert!(v.groups().iter().any(|g| g.id == sub && g.name == "Renamed"));
        assert!(matches!(v.add_group(root, "  "), Err(VaultError::EmptyGroupName)));
        // Sibling names are unique ignoring case; the same name under another parent is fine,
        // and renaming a folder to its own name (or a case change of it) is allowed.
        assert!(matches!(v.add_group(root, " renamed "), Err(VaultError::DuplicateGroupName(_))));
        let other = v.add_group(root, "Other").unwrap();
        assert!(matches!(v.rename_group(other, "RENAMED"), Err(VaultError::DuplicateGroupName(_))));
        v.add_group(sub, "Other").unwrap();
        v.rename_group(sub, "RENAMED").unwrap();
        assert!(matches!(v.upsert_entry(edit(GroupId(uuid::Uuid::new_v4()), "x")), Err(VaultError::GroupNotFound(_))));
    }

    /// KeePass order: a matching association's own sequence, then the entry's sequence, then
    /// the nearest non-blank folder default up the tree, then the app default (`None`).
    #[test]
    fn autotype_sequence_resolution() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("at.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let outer = v.add_group(root, "Outer").unwrap();
        let inner = v.add_group(outer, "Inner").unwrap();
        v.db.group_mut(outer.into()).unwrap().default_autotype_sequence = Some("{USERNAME}~{PASSWORD}~".into());
        // A blank folder default in between does not stop the inheritance.
        v.db.group_mut(inner.into()).unwrap().default_autotype_sequence = Some(" ".into());

        let mut bank = edit(inner, "Bank");
        bank.autotype_windows = vec!["*Bank Login*".into(), "Bank Home*".into()];
        let bank = v.upsert_entry(bank).unwrap();
        // The editor sets only window patterns; KeePass also sets a sequence per window.
        v.db.entry_mut(bank.into()).unwrap().autotype.as_mut().unwrap().associations[0].sequence =
            "{PASSWORD}{ENTER}".into();
        let plain = v.upsert_entry(edit(root, "Plain")).unwrap();
        v.save(&pol).unwrap();

        let mut r = Vault::open(&path, PW, None).unwrap();
        let b = r.entry(bank).unwrap();
        assert_eq!(b.autotype_sequence, None);
        assert_eq!(b.autotype_group_sequence.as_deref(), Some("{USERNAME}~{PASSWORD}~"));
        assert_eq!(b.autotype_window_sequence("My Bank Login - Browser"), Some("{PASSWORD}{ENTER}"));
        assert_eq!(b.autotype_sequence_for(Some("My Bank Login - Browser")), Some("{PASSWORD}{ENTER}"));
        // A matching association with an empty sequence falls back to the entry's default,
        // here inherited from Outer through Inner.
        assert_eq!(b.autotype_window_sequence("Bank Home"), None);
        assert_eq!(b.autotype_sequence_for(Some("Bank Home")), Some("{USERNAME}~{PASSWORD}~"));
        assert_eq!(b.autotype_sequence_for(Some("Elsewhere")), Some("{USERNAME}~{PASSWORD}~"));
        assert_eq!(b.autotype_sequence_for(None), Some("{USERNAME}~{PASSWORD}~"));
        // Nothing set anywhere: the app default.
        let p = r.entry(plain).unwrap();
        assert_eq!(p.autotype_group_sequence, None);
        assert_eq!(p.autotype_sequence_for(Some("Plain")), None);

        // The entry's own sequence overrides the folder default but not the association's.
        // Editing keeps the association's sequence.
        let mut e = edit_of(&r, bank);
        e.autotype_sequence = Some("{TITLE}{ENTER}".into());
        r.upsert_entry(e).unwrap();
        let b = r.entry(bank).unwrap();
        assert_eq!(b.autotype_sequence_for(Some("Elsewhere")), Some("{TITLE}{ENTER}"));
        assert_eq!(b.autotype_sequence_for(Some("Bank Home")), Some("{TITLE}{ENTER}"));
        assert_eq!(b.autotype_sequence_for(Some("My Bank Login - Browser")), Some("{PASSWORD}{ENTER}"));

        // The nearest non-blank folder default wins.
        r.db.group_mut(inner.into()).unwrap().default_autotype_sequence = Some("{TAB}".into());
        let mut e = edit_of(&r, bank);
        e.autotype_sequence = None;
        r.upsert_entry(e).unwrap();
        assert_eq!(r.entry(bank).unwrap().autotype_sequence_for(Some("Elsewhere")), Some("{TAB}"));
    }

    /// KeePass's switches: the entry's `Enabled`, and the nearest folder that sets
    /// `EnableAutoType`. Editing an entry keeps its switch.
    #[test]
    fn autotype_enabled_flags() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("en.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let off = v.add_group(root, "Off").unwrap();
        let below = v.add_group(off, "Below").unwrap();
        v.db.group_mut(off.into()).unwrap().enable_autotype = Some(false);
        let a = v.upsert_entry(edit(off, "A")).unwrap();
        let b = v.upsert_entry(edit(below, "B")).unwrap();
        let c = v.upsert_entry(edit(root, "C")).unwrap();
        assert!(!v.entry(a).unwrap().autotype_enabled);
        assert!(!v.entry(b).unwrap().autotype_enabled, "inherited from Off");
        assert!(v.entry(c).unwrap().autotype_enabled);
        v.db.group_mut(below.into()).unwrap().enable_autotype = Some(true);
        assert!(v.entry(b).unwrap().autotype_enabled, "Below turns it back on");

        v.db.entry_mut(c.into()).unwrap().autotype.as_mut().unwrap().enabled = false;
        let mut e = edit_of(&v, c);
        e.title = "C renamed".into();
        v.upsert_entry(e).unwrap();
        v.save(&pol).unwrap();

        let r = Vault::open(&path, PW, None).unwrap();
        assert!(!r.entry(a).unwrap().autotype_enabled);
        assert!(r.entry(b).unwrap().autotype_enabled);
        let c = r.entry(c).unwrap();
        assert_eq!(c.title, "C renamed");
        assert!(!c.autotype_enabled, "editing must not turn Auto-Type back on");
    }

    #[test]
    fn search_and_find_by_host() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("s.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let mut a = edit(root, "GitHub work");
        a.url = "https://www.github.com/login".into();
        a.tags = vec!["dev".into()];
        v.upsert_entry(a).unwrap();
        let mut b = edit(root, "GitHub personal");
        b.url = "github.com".into();
        b.stale = true;
        v.upsert_entry(b).unwrap();
        let mut c = edit(root, "Gitlab");
        c.url = "https://gitlab.com".into();
        c.notes = "dev account for work".into();
        v.upsert_entry(c).unwrap();
        let mut d = edit(root, "Mentions GitHub.com in title");
        d.url = String::new();
        v.upsert_entry(d).unwrap();

        let titles = |views: Vec<EntryView>| views.into_iter().map(|e| e.title).collect::<Vec<_>>();
        assert_eq!(titles(v.search("github")), vec!["GitHub work", "Mentions GitHub.com in title", "GitHub personal"]);
        assert_eq!(titles(v.search("DEV work")), vec!["GitHub work", "Gitlab"]);
        assert_eq!(titles(v.search("nothing-here")), Vec::<String>::new());
        assert_eq!(v.search("   ").len(), 4);
        assert_eq!(titles(v.find_by_host("WWW.GitHub.com")), vec!["GitHub work", "Mentions GitHub.com in title", "GitHub personal"]);
        assert_eq!(titles(v.find_by_host("gitlab.com")), vec!["Gitlab"]);
        assert!(v.find_by_host("").is_empty());
    }

    #[test]
    fn delete_moves_to_bin_then_permanent() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("d.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let mut doomed = edit(root, "Doomed");
        doomed.attach = vec![("a.bin".into(), vec![1, 2, 3])];
        let id = v.upsert_entry(doomed).unwrap();
        v.delete_entry(id).unwrap();
        let bin = v.groups().into_iter().find(|g| g.is_recycle_bin).unwrap().id;
        assert_eq!(v.entry(id).unwrap().group, bin);
        assert_eq!(v.entries_in(bin).len(), 1);
        assert!(v.all_entries().is_empty());
        assert!(v.search("doomed").is_empty());
        v.delete_entry(id).unwrap();
        assert!(v.entry(id).is_none());
        assert_eq!(v.db.num_entries(), 0);
        assert_eq!(v.db.num_attachments(), 0);
        assert!(matches!(v.delete_entry(id), Err(VaultError::EntryNotFound(_))));

        // A database without a bin gets one on first delete.
        v.db.meta.recyclebin_uuid = None;
        let id2 = v.upsert_entry(edit(root, "Second")).unwrap();
        v.delete_entry(id2).unwrap();
        let bins: Vec<_> = v.groups().into_iter().filter(|g| g.is_recycle_bin).collect();
        assert_eq!(bins.len(), 1);
        assert_eq!(v.entry(id2).unwrap().group, bins[0].id);
    }

    #[test]
    fn group_color_survives_save_and_bin_entries_include_deleted_groups() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("c.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let bank = v.add_group(root, "Banking").unwrap();
        v.set_group_color(bank, Some("green")).unwrap();
        let a = v.upsert_entry(edit(bank, "A")).unwrap();
        let b = v.upsert_entry(edit(root, "B")).unwrap();
        v.save(&policy(tmp.path())).unwrap();

        let mut v = Vault::open(&path, PW, None).unwrap();
        let color = |v: &Vault| v.groups().into_iter().find(|g| g.id == bank).unwrap().color;
        assert_eq!(color(&v).as_deref(), Some("green"));
        v.set_group_color(bank, None).unwrap();
        assert_eq!(color(&v), None);

        v.delete_entry(b).unwrap();
        v.delete_group(bank).unwrap();
        let mut in_bin: Vec<_> = v.recycle_bin_entries().into_iter().map(|e| e.id).collect();
        in_bin.sort_by_key(|id| id.0);
        let mut expected = vec![a, b];
        expected.sort_by_key(|id| id.0);
        assert_eq!(in_bin, expected, "entries of a deleted group count as in the bin");
        assert!(v.all_entries().is_empty());
    }

    #[test]
    fn delete_group_moves_subtree_to_bin_then_permanent() {
        let tmp = crate::test_util::TempDir::new();
        let mut v = Vault::create(&tmp.path().join("g.kdbx"), PW).unwrap();
        let root = v.root();
        let mail = v.add_group(root, "Mail").unwrap();
        let work = v.add_group(mail, "Work").unwrap();
        let a = v.upsert_entry(edit(mail, "A")).unwrap();
        let b = v.upsert_entry(edit(work, "B")).unwrap();
        assert!(matches!(v.delete_group(root), Err(VaultError::CannotDeleteRoot)));

        v.delete_group(mail).unwrap();
        let bin = v.groups().into_iter().find(|g| g.is_recycle_bin).unwrap().id;
        assert_eq!(v.groups().into_iter().find(|g| g.id == mail).unwrap().parent, Some(bin));
        assert_eq!(v.entry(b).unwrap().group, work, "the subtree moves intact");
        assert!(v.all_entries().is_empty());

        // Already in the bin: gone for good, with everything inside.
        v.delete_group(mail).unwrap();
        assert!(v.entry(a).is_none() && v.entry(b).is_none());
        assert!(v.groups().iter().all(|g| g.id != mail && g.id != work));

        // Deleting the bin itself empties it; the next delete creates a new bin.
        let c = v.upsert_entry(edit(root, "C")).unwrap();
        v.delete_entry(c).unwrap();
        v.delete_group(bin).unwrap();
        assert!(v.entry(c).is_none());
        assert!(v.groups().iter().all(|g| !g.is_recycle_bin));
        let d = v.upsert_entry(edit(root, "D")).unwrap();
        v.delete_entry(d).unwrap();
        assert!(v.groups().iter().any(|g| g.is_recycle_bin && g.id == v.entry(d).unwrap().group));

        // A group that contains the bin cannot be moved into it.
        let outer = v.add_group(root, "Outer").unwrap();
        let new_bin = v.entry(d).unwrap().group;
        v.db.group_mut(new_bin.into()).unwrap().move_to(outer.into()).unwrap();
        assert!(matches!(v.delete_group(outer), Err(VaultError::GroupHoldsRecycleBin)));

        // With the bin turned off, deletes are permanent.
        v.db.meta.recyclebin_enabled = Some(false);
        let temp = v.add_group(root, "Temp").unwrap();
        let e = v.upsert_entry(edit(temp, "E")).unwrap();
        v.delete_group(temp).unwrap();
        assert!(v.entry(e).is_none());
        assert!(v.groups().iter().all(|g| g.id != temp));
    }

    #[test]
    fn document_flag_and_attachment_sizes_survive_save() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("doc.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let mut e = edit(root, "Passport");
        e.document = true;
        e.notes = "# Passport\nNumber **X123**\nExpires 2031".into();
        e.attach = vec![("scan.pdf".into(), vec![7; 3000])];
        let doc = v.upsert_entry(e).unwrap();
        let mut bank = edit(root, "Bank");
        bank.attach = vec![("copy.pdf".into(), vec![7; 3000]), ("other.txt".into(), vec![1; 500])];
        let acct = v.upsert_entry(bank).unwrap();
        v.save(&policy(tmp.path())).unwrap();

        let mut v = Vault::open(&path, PW, None).unwrap();
        assert!(v.entry(doc).unwrap().document);
        assert!(!v.entry(acct).unwrap().document);
        assert_eq!(v.entry(doc).unwrap().notes, "# Passport\nNumber **X123**\nExpires 2031");
        assert_eq!(v.attachment_size(doc, "scan.pdf"), Some(3000));
        assert_eq!(v.attachments_total_bytes(), 6500);

        v.set_document(doc, false).unwrap();
        v.set_document(acct, true).unwrap();
        assert!(!v.entry(doc).unwrap().document);
        assert!(v.entry(acct).unwrap().document);
    }

    #[test]
    fn stale_flag_and_attachments() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("a.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let id = v.upsert_entry(edit(root, "X")).unwrap();
        v.set_stale(id, true).unwrap();
        assert!(v.entry(id).unwrap().stale);
        assert_eq!(v.db.entry(id.into()).unwrap().get(FIELD_STALE), Some("1"));
        v.set_stale(id, false).unwrap();
        assert!(!v.entry(id).unwrap().stale);
        assert_eq!(v.db.entry(id.into()).unwrap().get(FIELD_STALE), None);

        let mut e = edit_of(&v, id);
        e.attach = vec![("f".into(), vec![9])];
        v.upsert_entry(e).unwrap();
        let mut e = edit_of(&v, id);
        e.attach = vec![("f".into(), vec![8, 7])];
        v.upsert_entry(e).unwrap();
        assert_eq!(v.attachment(id, "f").unwrap(), vec![8, 7]);
        assert_eq!(v.entry(id).unwrap().attachments, vec!["f".to_string()]);
        let mut e = edit_of(&v, id);
        e.detach = vec!["f".into()];
        v.upsert_entry(e).unwrap();
        assert!(v.attachment(id, "f").is_none());
        let mut e = edit_of(&v, id);
        e.detach = vec!["f".into()];
        assert!(matches!(v.upsert_entry(e), Err(VaultError::AttachmentNotFound(_))));
        let mut e = edit(root, "New");
        e.detach = vec!["f".into()];
        assert!(matches!(v.upsert_entry(e), Err(VaultError::AttachmentNotFound(_))));
        assert_eq!(v.all_entries().len(), 1, "a refused edit must not create an entry");
        v.db.remove_unused_attachments();
        // Both earlier contents of "f" stay for the history versions that have them.
        assert_eq!(v.db.num_attachments(), 2);
        let raw = v.db.entry(id.into()).unwrap();
        assert_eq!(raw.historical(0).unwrap().attachment_by_name("f").unwrap().data.get(), &vec![8, 7]);
        assert_eq!(raw.historical(1).unwrap().attachment_by_name("f").unwrap().data.get(), &vec![9]);
        assert!(raw.historical(2).unwrap().attachment_by_name("f").is_none());
    }

    #[test]
    fn edit_with_files_is_one_history_version() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("h.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let mut e = edit(root, "Passport");
        e.attach = vec![("old.pdf".into(), vec![1; 10])];
        let id = v.upsert_entry(e).unwrap();
        assert_eq!(history_len(&v, id), 0);

        let mut e = edit_of(&v, id);
        e.title = "Passport 2031".into();
        e.detach = vec!["old.pdf".into()];
        e.attach = vec![("new.pdf".into(), vec![2; 20]), ("back.pdf".into(), vec![3; 30])];
        v.upsert_entry(e).unwrap();

        assert_eq!(history_len(&v, id), 1);
        let now = v.entry(id).unwrap();
        assert_eq!(now.title, "Passport 2031");
        assert_eq!(now.attachments, vec!["back.pdf".to_string(), "new.pdf".to_string()]);
        let raw = v.db.entry(id.into()).unwrap();
        let before = raw.historical(0).unwrap();
        assert_eq!(before.get(fields::TITLE), Some("Passport"));
        assert_eq!(before.attachments_named().map(|(n, _)| n.to_owned()).collect::<Vec<_>>(), vec!["old.pdf".to_string()]);

        v.save(&policy(tmp.path())).unwrap();
        let v = Vault::open(&path, PW, None).unwrap();
        assert_eq!(v.attachment(id, "new.pdf").unwrap(), vec![2; 20]);
        assert_eq!(history_len(&v, id), 1);
    }

    #[test]
    fn removed_and_replaced_files_stay_in_history() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("files.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        // A file deleted for good before the edits leaves a free id below a.pdf's.
        let mut gone = edit(root, "Gone");
        gone.attach = vec![("g".into(), vec![9; 5])];
        let gone = v.upsert_entry(gone).unwrap();
        let other = v.upsert_entry(edit(root, "Other")).unwrap();
        // Bytes that do not compress, so the file size shows whether they are stored.
        let mut x = 0x2545_f491u32;
        let scan: Vec<u8> = (0..40_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let mut e = edit(root, "Will");
        e.attach = vec![("a.pdf".into(), scan.clone()), ("keep.txt".into(), vec![1; 10])];
        let id = v.upsert_entry(e).unwrap();
        let mut o = edit_of(&v, other);
        o.attach = vec![("x".into(), vec![2; 5])];
        v.upsert_entry(o).unwrap();
        v.remove_entry(gone).unwrap();
        v.save(&pol).unwrap();

        let mut e = edit_of(&v, id);
        e.detach = vec!["a.pdf".into()];
        v.upsert_entry(e).unwrap();
        let mut o = edit_of(&v, other);
        o.detach = vec!["x".into()];
        v.upsert_entry(o).unwrap();
        let mut e = edit_of(&v, id);
        e.attach = vec![("keep.txt".into(), vec![3; 10])];
        v.upsert_entry(e).unwrap();
        v.save(&pol).unwrap();

        let v = Vault::open(&path, PW, None).unwrap();
        assert_eq!(v.entry(id).unwrap().attachments, vec!["keep.txt".to_string()]);
        assert_eq!(v.attachment(id, "keep.txt").unwrap(), vec![3; 10]);
        assert_eq!(v.version_attachment(id, 0, "keep.txt").unwrap(), vec![1; 10]);
        assert!(v.version_attachment(id, 0, "a.pdf").is_none());
        assert_eq!(v.version_attachment(id, 1, "a.pdf").unwrap(), scan);
        assert_eq!(v.version_attachment(other, 0, "x").unwrap(), vec![2; 5]);
    }

    #[test]
    fn restore_version_undoes_the_last_edit() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("u.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let sub = v.add_group(root, "Sub").unwrap();
        let id = v.upsert_entry(edit(root, "Bank")).unwrap();
        // A new entry has no earlier version.
        assert!(matches!(v.restore_version(id, 0, false), Err(VaultError::NoEarlierVersion(_))));

        let mut e = edit_of(&v, id);
        e.title = "Bank renamed".into();
        e.password = "changed".into();
        e.group = sub;
        e.attach = vec![("card.txt".into(), vec![5; 5])];
        v.upsert_entry(e).unwrap();

        // Undo of that save: back to the old content and the old folder.
        v.restore_version(id, 0, false).unwrap();
        let back = v.entry(id).unwrap();
        assert_eq!(back.title, "Bank");
        assert_eq!(back.password, "pw-Bank");
        assert_eq!(back.group, root);
        assert!(back.attachments.is_empty());
        assert_eq!(v.entries_in(root).len(), 1);
        assert!(v.entries_in(sub).is_empty());
        // The undone version stays in the history, with its file.
        assert_eq!(v.history_len(id), 2);
        assert_eq!(v.version_attachment(id, 0, "card.txt").unwrap(), vec![5; 5]);
        let raw = v.db.entry(id.into()).unwrap();
        assert_eq!(raw.historical(0).unwrap().get(fields::TITLE), Some("Bank renamed"));

        v.save(&policy(tmp.path())).unwrap();
        let mut v = Vault::open(&path, PW, None).unwrap();
        assert_eq!(v.entry(id).unwrap().title, "Bank");
        assert_eq!(v.entry(id).unwrap().group, root);

        v.remove_entry(id).unwrap();
        assert!(v.entry(id).is_none());
        assert!(v.is_dirty());
    }

    #[test]
    fn restore_and_delete_versions_by_index() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("idx.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let sub = v.add_group(root, "Sub").unwrap();
        let id = v.upsert_entry(edit(root, "V1")).unwrap();
        for title in ["V2", "V3"] {
            let mut e = edit_of(&v, id);
            e.title = title.into();
            v.upsert_entry(e).unwrap();
        }
        // Move the entry; restoring from the viewer keeps it where it is now.
        let mut e = edit_of(&v, id);
        e.title = "V4".into();
        e.group = sub;
        v.upsert_entry(e).unwrap();
        // Versions newest first: V3, V2, V1.
        let titles = |v: &Vault| (0..v.history_len(id)).map(|i| v.version(id, i).unwrap().0.title).collect::<Vec<_>>();
        assert_eq!(titles(&v), ["V3", "V2", "V1"]);

        v.restore_version(id, 2, true).unwrap();
        let now = v.entry(id).unwrap();
        assert_eq!(now.title, "V1");
        assert_eq!(now.group, sub);
        assert_eq!(v.entries_in(sub).len(), 1);
        assert!(v.entries_in(root).is_empty());
        assert_eq!(titles(&v), ["V4", "V3", "V2", "V1"]);

        v.delete_version(id, 1).unwrap();
        assert_eq!(titles(&v), ["V4", "V2", "V1"]);
        assert!(matches!(v.delete_version(id, 3), Err(VaultError::NoEarlierVersion(_))));
        assert_eq!(v.entry(id).unwrap().title, "V1");
        v.save(&policy(tmp.path())).unwrap();
        let v = Vault::open(&path, PW, None).unwrap();
        assert_eq!(titles(&v), ["V4", "V2", "V1"]);
    }

    #[test]
    fn version_summaries_and_differences() {
        let tmp = crate::test_util::TempDir::new();
        let mut v = Vault::create(&tmp.path().join("s.kdbx"), PW).unwrap();
        let root = v.root();
        let mut e = edit(root, "Bank");
        e.attach = vec![("old.pdf".into(), vec![1]), ("same.txt".into(), vec![2])];
        e.custom = vec![CustomField { key: "PIN".into(), value: "1".into(), protected: true }];
        let id = v.upsert_entry(e).unwrap();

        let mut e = edit_of(&v, id);
        e.password = "new".into();
        e.url = "bank.example".into();
        e.custom = vec![CustomField { key: "Account".into(), value: "9".into(), protected: false }];
        e.detach = vec!["old.pdf".into()];
        e.attach = vec![("scan.pdf".into(), vec![3]), ("same.txt".into(), vec![4])];
        v.upsert_entry(e).unwrap();
        v.set_stale(id, true).unwrap();

        let versions = v.versions(id);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].changes, ["marked not working"]);
        assert_eq!(
            versions[1].changes,
            [
                "password, website changed",
                "field added: Account",
                "field removed: PIN",
                "file replaced: same.txt",
                "file added: scan.pdf",
                "file removed: old.pdf",
            ]
        );
        // Version 1 against the current version.
        let (old, differs) = v.version(id, 1).unwrap();
        assert_eq!(old.password, "pw-Bank");
        assert_eq!(differs, ["password", "url", "custom:Account", "custom:PIN", "files", "stale"]);
        let (_, differs) = v.version(id, 0).unwrap();
        assert_eq!(differs, ["stale"]);
    }

    #[test]
    fn backup_rotation_keeps_recent_and_dailies() {
        let tmp = crate::test_util::TempDir::new();
        let db_path = tmp.path().join("rot.kdbx");
        let pol = policy(tmp.path());
        let (dir, stem) = backup_subdir(&pol, &db_path).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let now = Local.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 11, 4).unwrap().and_hms_opt(12, 0, 0).unwrap()).unwrap();
        let today = now.date_naive();
        let mut written = Vec::new();
        // 25 saves today, one per minute: newest 20 kept, then one more as today's daily.
        for i in 0..25 {
            let ts = today.and_time(NaiveTime::from_hms_opt(10, i, 0).unwrap());
            let p = dir.join(format!("{}-{}.{}", stem, ts.format("%Y%m%d-%H%M%S"), BACKUP_EXT));
            fs::write(&p, b"x").unwrap();
            written.push(p);
        }
        // Three saves each on days 1, 5, 29, 31, 40 ago.
        for days in [1i64, 5, 29, 31, 40] {
            for h in [8u32, 13, 18] {
                let ts = (today - chrono::Duration::days(days)).and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap());
                let p = dir.join(format!("{}-{}.{}", stem, ts.format("%Y%m%d-%H%M%S"), BACKUP_EXT));
                fs::write(&p, b"x").unwrap();
            }
        }
        fs::write(dir.join("unrelated.txt"), b"x").unwrap();
        assert_eq!(list_backups(&pol, &db_path).len(), 40);

        let deleted = rotate_backups_at(&pol, &db_path, now).unwrap();
        let remaining = list_backups(&pol, &db_path);
        // 20 recent + 1 today daily + 3 dailies (1, 5, 29 days) = 24; days 31 and 40 are outside the window.
        assert_eq!(remaining.len(), 24, "{remaining:?}");
        assert_eq!(deleted.len(), 16);
        assert!(dir.join("unrelated.txt").exists());
        // Newest first; the newest 20 of today are the 10:24 .. 10:05 files.
        assert_eq!(remaining[0], written[24]);
        assert_eq!(remaining[19], written[5]);
        // The daily survivor of today among the older files is the newest one, 10:04.
        assert_eq!(remaining[20], written[4]);
        for days in [1i64, 5, 29] {
            let ts = (today - chrono::Duration::days(days)).and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
            assert!(remaining.contains(&dir.join(format!("{}-{}.{}", stem, ts.format("%Y%m%d-%H%M%S"), BACKUP_EXT))));
        }
        // Rotation is idempotent.
        assert!(rotate_backups_at(&pol, &db_path, now).unwrap().is_empty());
    }

    #[test]
    fn failed_save_keeps_original() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("keep.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        v.upsert_entry(edit(root, "Safe")).unwrap();
        v.save(&pol).unwrap();
        let before = fs::read(&path).unwrap();

        v.upsert_entry(edit(root, "Lost")).unwrap();
        v.path = tmp.path().join("missing-dir").join("keep.kdbx");
        assert!(matches!(v.save(&pol), Err(VaultError::Io { .. })));
        assert!(v.is_dirty());
        assert_eq!(fs::read(&path).unwrap(), before);
        let r = Vault::open(&path, PW, None).unwrap();
        assert_eq!(r.all_entries().len(), 1);

        // Second save over an existing file replaces it in place and adds a backup.
        v.path = path.clone();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        v.save(&pol).unwrap();
        assert_eq!(Vault::open(&path, PW, None).unwrap().all_entries().len(), 2);
        assert_eq!(list_backups(&pol, &path).len(), 2);
        assert!(!tmp_path(&path).exists());
    }

    #[test]
    fn wrong_password_and_existing_file() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("w.kdbx");
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        v.save(&pol).unwrap();
        assert!(matches!(Vault::open(&path, "nope", None), Err(VaultError::WrongKey)));
        assert!(matches!(Vault::create(&path, PW), Err(VaultError::AlreadyExists(_))));
        assert!(matches!(Vault::open(&tmp.path().join("none.kdbx"), PW, None), Err(VaultError::Io { .. })));
    }

    #[test]
    fn keyfile_roundtrip() {
        let tmp = crate::test_util::TempDir::new();
        let path = tmp.path().join("k.kdbx");
        let kf = tmp.path().join("key.bin");
        fs::write(&kf, b"some random key material that is not 32 or 64 bytes long").unwrap();
        let pol = policy(tmp.path());
        let mut v = Vault::create(&path, PW).unwrap();
        v.key = build_key(PW, Some(&kf)).unwrap();
        v.save(&pol).unwrap();
        assert!(Vault::open(&path, PW, Some(&kf)).is_ok());
        assert!(Vault::open(&path, PW, None).is_err());
    }

    #[test]
    fn untitled_names_skip_taken_ones() {
        assert_eq!(untitled_name(&[]), "Untitled");
        assert_eq!(untitled_name(&["Bank".into()]), "Untitled");
        assert_eq!(untitled_name(&["untitled".into()]), "Untitled 2");
        assert_eq!(untitled_name(&["Untitled".into(), "Untitled 2".into(), "Untitled 4".into()]), "Untitled 3");
    }

    #[test]
    fn url_host_cases() {
        assert_eq!(url_host("https://www.Foo.com/x").as_deref(), Some("foo.com"));
        assert_eq!(url_host("foo.com").as_deref(), Some("foo.com"));
        assert_eq!(url_host("WWW.foo.com").as_deref(), Some("foo.com"));
        assert_eq!(url_host("http://user:pw@foo.com:8080/p?q#f").as_deref(), Some("foo.com"));
        assert_eq!(url_host("foo.com:443").as_deref(), Some("foo.com"));
        assert_eq!(url_host("https://foo.com?x=1").as_deref(), Some("foo.com"));
        assert_eq!(url_host("https://[::1]:8080/").as_deref(), Some("::1"));
        assert_eq!(url_host("  https://a.b.c.  ").as_deref(), Some("a.b.c"));
        assert_eq!(url_host(""), None);
        assert_eq!(url_host("   "), None);
        assert_eq!(url_host("https:///path"), None);
        assert_eq!(url_host("not a url"), None);
        assert_eq!(url_host("www."), None);
    }
}
