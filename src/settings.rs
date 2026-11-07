//! Persistent application settings stored as JSON in the per-user data directory.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::generator::GenOptions;
use crate::vault::BackupPolicy;

const MAX_RECENT: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("serialize settings: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentDb {
    pub path: PathBuf,
    #[serde(default)]
    pub keyfile: Option<PathBuf>,
}

/// What the letter tiles in the account list are coloured by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileColors {
    /// Each folder's colour (chosen by the user or picked automatically).
    #[default]
    Folder,
    /// One neutral colour for every tile.
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Most recent first, at most `MAX_RECENT` entries.
    pub recent: Vec<RecentDb>,
    pub reopen_last: bool,
    pub autotype_hotkey: String,
    pub autotype_default_sequence: String,
    pub generator: GenOptions,
    pub notes_monospace: bool,
    pub tile_colors: TileColors,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recent: Vec::new(),
            reopen_last: true,
            autotype_hotkey: "Ctrl+Alt+A".to_string(),
            autotype_default_sequence: "{USERNAME}{TAB}{PASSWORD}{ENTER}".to_string(),
            generator: GenOptions::default(),
            notes_monospace: false,
            tile_colors: TileColors::Folder,
        }
    }
}

impl Settings {
    /// Reads `settings_path()`. A missing or unreadable file yields `Default`. Moves the
    /// data directory over from the old KeePassJ name first.
    pub fn load() -> Settings {
        migrate_legacy_data_dir();
        Self::load_from(&settings_path())
    }

    fn load_from(path: &Path) -> Settings {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    /// Writes `settings_path()` atomically: `settings.json.tmp` then rename.
    pub fn save(&self) -> Result<(), SettingsError> {
        self.save_to(&settings_path())
    }

    fn save_to(&self, path: &Path) -> Result<(), SettingsError> {
        let io = |p: &Path| {
            let p = p.to_path_buf();
            move |source| SettingsError::Io { path: p, source }
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = tmp_path(path);
        {
            let mut f = fs::File::create(&tmp).map_err(io(&tmp))?;
            f.write_all(&json).map_err(io(&tmp))?;
            f.sync_all().map_err(io(&tmp))?;
        }
        // Windows refuses to rename over an existing file.
        if path.exists() {
            fs::remove_file(path).map_err(io(path))?;
        }
        fs::rename(&tmp, path).map_err(io(path))
    }

    /// Moves or inserts `path` at the front of the recent list, deduplicated by path,
    /// trimmed to `MAX_RECENT`.
    pub fn touch_recent(&mut self, path: &Path, keyfile: Option<&Path>) {
        self.recent.retain(|r| r.path != path);
        self.recent.insert(
            0,
            RecentDb { path: path.to_path_buf(), keyfile: keyfile.map(Path::to_path_buf) },
        );
        self.recent.truncate(MAX_RECENT);
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Per-user local data directory. Falls back to `./.ourlogins` when no home directory exists.
pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "logonpro", "OurLogins")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".ourlogins"))
}

/// The app was called KeePassJ up to version 0.2. On the first start under the new name, its
/// data directory (settings and backups) moves to `data_dir()`.
fn migrate_legacy_data_dir() {
    if let Some(old) = directories::ProjectDirs::from("com", "logonpro", "KeePassJ") {
        migrate_dir(old.data_local_dir(), &data_dir());
    }
}

/// Moves `old` to `new` unless `new` already exists. If the move fails (a file in use), the
/// settings file is copied so the recent databases and preferences carry over.
fn migrate_dir(old: &Path, new: &Path) {
    if new.exists() || !old.is_dir() {
        return;
    }
    if let Some(parent) = new.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::rename(old, new).is_ok() {
        // Windows keeps data under ...\logonpro\KeePassJ\data; drop the now-empty KeePassJ folder.
        #[cfg(windows)]
        if let Some(parent) = old.parent() {
            let _ = fs::remove_dir(parent);
        }
    } else if fs::create_dir_all(new).is_ok() {
        let _ = fs::copy(old.join("settings.json"), new.join("settings.json"));
    }
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn backup_policy() -> BackupPolicy {
    BackupPolicy { dir: data_dir().join("backups"), keep_recent: 20, keep_daily_days: 30 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert!(s.reopen_last);
        assert_eq!(s.autotype_hotkey, "Ctrl+Alt+A");
        assert_eq!(s.autotype_default_sequence, "{USERNAME}{TAB}{PASSWORD}{ENTER}");
        assert!(!s.notes_monospace);
        assert!(s.recent.is_empty());
    }

    #[test]
    fn round_trip_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let mut s = Settings { notes_monospace: true, ..Default::default() };
        s.touch_recent(Path::new("C:/db/a.kdbx"), Some(Path::new("C:/db/a.key")));
        s.generator.length = 24;
        s.save_to(&path).unwrap();
        assert!(!tmp_path(&path).exists());
        assert_eq!(Settings::load_from(&path), s);
        // Second save replaces the existing file.
        s.reopen_last = false;
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
    }

    #[test]
    fn missing_and_corrupt_files_give_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default());
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        // Unknown fields and missing fields are tolerated.
        fs::write(&path, br#"{"reopen_last": false, "future_field": 1}"#).unwrap();
        let s = Settings::load_from(&path);
        assert!(!s.reopen_last);
        assert_eq!(s.autotype_hotkey, "Ctrl+Alt+A");
    }

    #[test]
    fn touch_recent_dedupes_and_caps() {
        let mut s = Settings::default();
        for i in 0..12 {
            s.touch_recent(Path::new(&format!("db{i}.kdbx")), None);
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0].path, Path::new("db11.kdbx"));
        s.touch_recent(Path::new("db5.kdbx"), Some(Path::new("k.key")));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0].path, Path::new("db5.kdbx"));
        assert_eq!(s.recent[0].keyfile.as_deref(), Some(Path::new("k.key")));
        assert_eq!(s.recent.iter().filter(|r| r.path == Path::new("db5.kdbx")).count(), 1);
    }

    #[test]
    fn paths_and_policy() {
        assert_eq!(settings_path(), data_dir().join("settings.json"));
        let p = backup_policy();
        assert_eq!(p.dir, data_dir().join("backups"));
        assert_eq!(p.keep_recent, 20);
        assert_eq!(p.keep_daily_days, 30);
    }

    #[test]
    fn legacy_data_dir_moves_once() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("logonpro").join("KeePassJ").join("data");
        let new = tmp.path().join("logonpro").join("OurLogins").join("data");
        fs::create_dir_all(old.join("backups").join("db-1234")).unwrap();
        fs::write(old.join("settings.json"), b"{}").unwrap();
        fs::write(old.join("backups").join("db-1234").join("b.kdbx"), b"x").unwrap();

        migrate_dir(&old, &new);
        assert!(new.join("settings.json").is_file());
        assert!(new.join("backups").join("db-1234").join("b.kdbx").is_file());
        assert!(!old.exists());

        // Once the new directory exists, a reappearing old one is left alone.
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("settings.json"), b"{\"reopen_last\": false}").unwrap();
        migrate_dir(&old, &new);
        assert_eq!(fs::read(new.join("settings.json")).unwrap(), b"{}");
    }
}
