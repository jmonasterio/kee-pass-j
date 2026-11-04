//! KDBX open/save/backup, entry and group model, search, stale flag.
//!
//! Wraps `keepass::Database`. Standard KDBX fields are `Title`, `UserName`, `Password`,
//! `URL`, `Notes`. Application fields are plain KDBX custom strings:
//! `KPJ_Stale`, `Recovery PIN` (protected), `Recovery Phone`, `Security Questions` (protected).

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use keepass::config::{DatabaseConfig, DatabaseVersion, InnerCipherConfig, KdfConfig, VariantDictionary};
use keepass::db::{self, fields, AutoTypeAssociation, Database, Entry, History, Times, Value};
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

/// Fields with a dedicated slot in `EntryView`; everything else is a custom field.
const RESERVED_FIELDS: [&str; 9] = [
    fields::TITLE,
    fields::USERNAME,
    fields::PASSWORD,
    fields::URL,
    fields::NOTES,
    FIELD_STALE,
    FIELD_RECOVERY_PIN,
    FIELD_RECOVERY_PHONE,
    FIELD_SECURITY_QUESTIONS,
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
    #[error("group name must not be empty")]
    EmptyGroupName,
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
    /// `None` means the default sequence applies.
    pub autotype_sequence: Option<String>,
    /// Window-title patterns.
    pub autotype_windows: Vec<String>,
    pub modified: DateTime<Utc>,
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
    let (autotype_sequence, autotype_windows) = match &e.autotype {
        Some(at) => (
            at.default_sequence.clone().filter(|s| !s.is_empty()),
            at.associations.iter().map(|a| a.window.clone()).collect(),
        ),
        None => (None, Vec::new()),
    };
    EntryView {
        id: e.id().into(),
        group: e.parent().id().into(),
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
        modified: to_utc(e.times.last_modification),
    }
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
        if let DatabaseVersion::KDB3(_) = db.config.version {
            db.config.version = DatabaseVersion::KDB4(1);
            db.config.inner_cipher_config = InnerCipherConfig::ChaCha20;
        }
        Ok(Vault {
            path: path.to_path_buf(),
            db,
            key,
            dirty: false,
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
        db.meta.generator = Some("KeePassJ".to_owned());
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

    /// Replace the master key (KeePass "Change Master Key"). Marks the vault dirty; the
    /// next `save` re-encrypts the file with the new key. The backup copies written before
    /// this call keep the old key.
    pub fn change_key(&mut self, password: &str, keyfile: Option<&Path>) -> Result<(), VaultError> {
        self.key = build_key(password, keyfile)?;
        self.db.meta.master_key_changed = Some(keepass::db::Times::now());
        self.dirty = true;
        Ok(())
    }

    /// Atomic write: `<path>.tmp` → fsync → rename over `<path>`. Then copy to backups and rotate.
    /// A serialization failure leaves the existing file untouched.
    pub fn save(&mut self, backups: &BackupPolicy) -> Result<(), VaultError> {
        let tmp = tmp_path(&self.path);
        if let Err(e) = self.write_tmp(&tmp) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        let backup_target = backup_file_path(backups, &self.path, Local::now())?;
        let mut backup_written = false;
        if let Err(rename_err) = fs::rename(&tmp, &self.path) {
            // Windows refuses the replacing rename while another process holds the file
            // open without FILE_SHARE_DELETE. The backup copy is taken from the tmp file
            // first so the new content exists on disk before the original is removed.
            let recoverable = cfg!(windows) && self.path.exists();
            if !recoverable {
                let _ = fs::remove_file(&tmp);
                return Err(io_err(&self.path, rename_err));
            }
            copy_backup(&tmp, &backup_target)?;
            backup_written = true;
            if let Err(e) = fs::remove_file(&self.path).and_then(|_| fs::rename(&tmp, &self.path)) {
                return Err(io_err(&self.path, e));
            }
        }
        #[cfg(unix)]
        if let Some(dir) = self.path.parent() {
            if let Ok(d) = File::open(dir) {
                let _ = d.sync_all();
            }
        }
        self.dirty = false;
        if !backup_written {
            copy_backup(&self.path, &backup_target)?;
        }
        rotate_backups_at(backups, &self.path, Local::now())?;
        Ok(())
    }

    fn write_tmp(&self, tmp: &Path) -> Result<(), VaultError> {
        let file = File::create(tmp).map_err(|e| io_err(tmp, e))?;
        let mut writer = BufWriter::new(file);
        self.db.save(&mut writer, self.key.clone())?;
        writer.flush().map_err(|e| io_err(tmp, e))?;
        writer
            .into_inner()
            .map_err(|e| io_err(tmp, e.into_error()))?
            .sync_all()
            .map_err(|e| io_err(tmp, e))?;
        Ok(())
    }

    pub fn root(&self) -> GroupId {
        self.db.root().id().into()
    }

    fn recycle_bin_id(&self) -> Option<db::GroupId> {
        self.db.recycle_bin().map(|g| g.id())
    }

    /// True when `group` is the recycle bin or lies under it.
    fn in_recycle_bin(&self, group: db::GroupId) -> bool {
        let Some(bin) = self.recycle_bin_id() else { return false };
        let mut cur = Some(group);
        while let Some(id) = cur {
            if id == bin {
                return true;
            }
            cur = self.db.group(id).and_then(|g| g.parent().map(|p| p.id()));
        }
        false
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
    /// on every call. Updates keep the previous version in the entry history.
    pub fn upsert_entry(&mut self, edit: EntryEdit) -> Result<EntryId, VaultError> {
        let group: db::GroupId = edit.group.into();
        if self.db.group(group).is_none() {
            return Err(VaultError::GroupNotFound(edit.group));
        }
        let id: db::EntryId = match edit.id {
            Some(id) => {
                let id: db::EntryId = id.into();
                if self.db.entry(id).is_none() {
                    return Err(VaultError::EntryNotFound(id.into()));
                }
                id
            }
            None => self.db.group_mut(group).map(|mut g| g.add_entry().id()).ok_or(VaultError::GroupNotFound(edit.group))?,
        };
        let mut entry = self.db.entry_mut(id).ok_or(VaultError::EntryNotFound(id.into()))?;
        let is_update = edit.id.is_some();
        let mut tracked = entry.track_changes();
        {
            let mut e = tracked.as_mut();
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

            let mut at = e.autotype.take().unwrap_or_default();
            at.enabled = true;
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

            let now = Times::now();
            e.times.last_modification = Some(now);
            e.times.last_access = Some(now);
        }
        drop(tracked);
        if is_update {
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
        let bin_disabled = self.db.meta.recyclebin_enabled == Some(false);
        if bin_disabled || self.in_recycle_bin(parent) {
            let mut entry = self.db.entry_mut(kid).ok_or(VaultError::EntryNotFound(id))?;
            entry.track_changes().remove();
        } else {
            let bin = self.ensure_recycle_bin();
            let mut entry = self.db.entry_mut(kid).ok_or(VaultError::EntryNotFound(id))?;
            entry.move_to(bin).map_err(|_| VaultError::GroupNotFound(bin.into()))?;
            entry.times.location_changed = Some(Times::now());
        }
        self.dirty = true;
        Ok(())
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

    pub fn add_group(&mut self, parent: GroupId, name: &str) -> Result<GroupId, VaultError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(VaultError::EmptyGroupName);
        }
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
        let mut g = self.db.group_mut(id.into()).ok_or(VaultError::GroupNotFound(id))?;
        g.name = name.to_owned();
        g.times.last_modification = Some(Times::now());
        self.dirty = true;
        Ok(())
    }

    pub fn attachment(&self, id: EntryId, name: &str) -> Option<Vec<u8>> {
        let entry = self.db.entry(id.into())?;
        let att = entry.attachment_by_name(name)?;
        Some(att.data.get().clone())
    }

    /// Adds or replaces the attachment with this name.
    pub fn add_attachment(&mut self, id: EntryId, name: &str, bytes: Vec<u8>) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        let mut tracked = entry.track_changes();
        tracked.add_attachment(name, Value::protected(bytes));
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

    pub fn remove_attachment(&mut self, id: EntryId, name: &str) -> Result<(), VaultError> {
        let mut entry = self.db.entry_mut(id.into()).ok_or(VaultError::EntryNotFound(id))?;
        if entry.attachment_by_name_mut(name).is_none() {
            return Err(VaultError::AttachmentNotFound(name.to_owned()));
        }
        let mut tracked = entry.track_changes();
        {
            let mut e = tracked.as_mut();
            e.remove_attachment_by_name(name);
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

fn backup_file_path(policy: &BackupPolicy, db_path: &Path, at: DateTime<Local>) -> Result<PathBuf, VaultError> {
    let (dir, stem) = backup_subdir(policy, db_path)?;
    Ok(dir.join(format!("{}-{}.{}", stem, at.format("%Y%m%d-%H%M%S"), BACKUP_EXT)))
}

fn copy_backup(from: &Path, to: &Path) -> Result<(), VaultError> {
    let backup_err = |e: std::io::Error| VaultError::Backup {
        path: to.to_path_buf(),
        source: e,
    };
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(backup_err)?;
    }
    fs::copy(from, to).map_err(backup_err)?;
    Ok(())
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
        }
    }

    /// KDBX 3.1 fixture from keepass-rs (MIT), password "demopass", two entries with attachments.
    #[test]
    fn kdbx3_wrong_password_and_upgrade_on_save() {
        let tmp = tempfile::tempdir().unwrap();
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
        v.save(&pol).unwrap();

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

    #[test]
    fn change_key_locks_out_old_password() {
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
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
        let bank = v.upsert_entry(rich).unwrap();
        v.add_attachment(bank, "card.txt", b"front: 1234".to_vec()).unwrap();

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
        assert_eq!(b.autotype_windows, vec!["*MyBank*".to_string(), "//Bank.*//".to_string()]);
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
        let tmp = tempfile::tempdir().unwrap();
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
        assert!(matches!(v.upsert_entry(edit(GroupId(uuid::Uuid::new_v4()), "x")), Err(VaultError::GroupNotFound(_))));
    }

    #[test]
    fn search_and_find_by_host() {
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("d.kdbx");
        let mut v = Vault::create(&path, PW).unwrap();
        let root = v.root();
        let id = v.upsert_entry(edit(root, "Doomed")).unwrap();
        v.add_attachment(id, "a.bin", vec![1, 2, 3]).unwrap();
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
    fn stale_flag_and_attachments() {
        let tmp = tempfile::tempdir().unwrap();
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

        v.add_attachment(id, "f", vec![9]).unwrap();
        v.add_attachment(id, "f", vec![8, 7]).unwrap();
        assert_eq!(v.attachment(id, "f").unwrap(), vec![8, 7]);
        assert_eq!(v.entry(id).unwrap().attachments, vec!["f".to_string()]);
        v.remove_attachment(id, "f").unwrap();
        assert!(v.attachment(id, "f").is_none());
        assert!(matches!(v.remove_attachment(id, "f"), Err(VaultError::AttachmentNotFound(_))));
        v.db.remove_unused_attachments();
        assert_eq!(v.db.num_attachments(), 0);
    }

    #[test]
    fn backup_rotation_keeps_recent_and_dailies() {
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
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
