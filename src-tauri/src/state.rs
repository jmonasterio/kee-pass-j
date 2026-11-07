//! Application state behind the commands, and the views built from it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ourlogins::settings::{self, Settings};
use ourlogins::vault::{EntryId, EntryView, GroupId, GroupView, Vault};

use crate::dto::*;

pub struct OpenDb {
    pub vault: Vault,
    pub selected_group: Option<GroupId>,
    pub selected_entry: Option<EntryId>,
}

pub struct AppState {
    pub settings: Settings,
    pub dbs: Vec<OpenDb>,
    pub active_tab: usize,
    pub search: String,
    /// Databases offered on the unlock page: settings.recent plus files picked this session.
    pub picker: Vec<PathBuf>,
    /// Path preselected on the unlock page.
    pub picker_selected: Option<PathBuf>,
    pub pending_new_db: Option<PathBuf>,
    /// (db index, entry) pairs behind the Auto-Type chooser dialog.
    pub chooser: Vec<(usize, EntryId)>,
    /// Tab whose last save failed, with the error text; Retry saves this one.
    pub save_error: Option<(usize, String)>,
}

pub struct Shared(pub Mutex<AppState>);

impl Shared {
    pub fn new() -> Shared {
        let settings = Settings::load();
        let picker: Vec<PathBuf> = settings.recent.iter().map(|r| r.path.clone()).collect();
        let picker_selected = if settings.reopen_last { picker.first().cloned() } else { None };
        Shared(Mutex::new(AppState {
            settings,
            dbs: Vec::new(),
            active_tab: 0,
            search: String::new(),
            picker,
            picker_selected,
            pending_new_db: None,
            chooser: Vec::new(),
            save_error: None,
        }))
    }

    /// A panic while holding the lock must not brick the app: the state is still usable.
    pub fn lock(&self) -> MutexGuard<'_, AppState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl AppState {
    pub fn active(&self) -> Option<&OpenDb> {
        self.dbs.get(self.active_tab)
    }

    pub fn active_mut(&mut self) -> Option<&mut OpenDb> {
        self.dbs.get_mut(self.active_tab)
    }

    pub fn active_or_err(&mut self) -> Result<&mut OpenDb, String> {
        self.active_mut().ok_or_else(|| "No database is open.".to_string())
    }

    /// Saves tab `tab`, recording or clearing the persistent error. Returns true on success.
    pub fn save_tab(&mut self, tab: usize) -> bool {
        let policy = settings::backup_policy();
        let Some(db) = self.dbs.get_mut(tab) else { return true };
        match db.vault.save(&policy) {
            Ok(()) => {
                if self.save_error.as_ref().is_some_and(|(t, _)| *t == tab) {
                    self.save_error = None;
                }
                true
            }
            Err(e) => {
                self.save_error = Some((tab, e.to_string()));
                false
            }
        }
    }

    pub fn save_active(&mut self) -> bool {
        self.save_tab(self.active_tab)
    }

    pub fn save_settings(&self) -> Result<(), String> {
        self.settings.save().map_err(|e| format!("Could not save settings: {e}"))
    }

    pub fn unlock_view(&self) -> UnlockView {
        let recent = self
            .picker
            .iter()
            .map(|p| RecentItem {
                path: p.display().to_string(),
                name: file_stem(p),
                missing: !p.is_file(),
                keyfile: self
                    .settings
                    .recent
                    .iter()
                    .find(|r| &r.path == p)
                    .and_then(|r| r.keyfile.as_deref())
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned()),
            })
            .collect();
        let selected = self
            .picker_selected
            .as_ref()
            .filter(|p| self.picker.contains(p))
            .map(|p| p.display().to_string());
        UnlockView {
            recent,
            selected,
            has_open_dbs: !self.dbs.is_empty(),
            pending_new: self.pending_new_db.as_ref().map(|p| p.display().to_string()),
        }
    }

    /// Builds the whole main page; also repairs selections that no longer exist.
    pub fn main_view(&mut self) -> MainView {
        let search = self.search.trim().to_string();
        let notes_monospace = self.settings.notes_monospace;
        let save_error = self.save_error.as_ref().map(|(_, e)| e.clone());
        let tabs = self
            .dbs
            .iter()
            .map(|db| TabItem {
                name: db.vault.name(),
                path: db.vault.path().display().to_string(),
                dirty: db.vault.is_dirty(),
            })
            .collect();
        let active_tab = self.active_tab;
        let Some(db) = self.dbs.get_mut(active_tab) else {
            return MainView {
                tabs,
                active_tab,
                groups: Vec::new(),
                selected_group: None,
                search,
                entries: Vec::new(),
                selected_entry: None,
                detail: None,
                save_error,
                notes_monospace,
                recycle_bin_enabled: true,
            };
        };

        let groups = ordered_groups(&db.vault);
        if db.selected_group.is_none_or(|sel| !groups.iter().any(|g| g.id == sel)) {
            db.selected_group = Some(db.vault.root());
        }
        let mut list = if search.is_empty() {
            db.selected_group.map(|g| db.vault.entries_in(g)).unwrap_or_default()
        } else {
            db.vault.search(&search)
        };
        // The vault already orders stale entries last; keep that invariant here too.
        list.sort_by_key(|e| e.stale);

        let detail = db
            .selected_entry
            .and_then(|id| db.vault.entry(id))
            .map(|e| entry_detail(&e, &groups));
        if detail.is_none() {
            db.selected_entry = None;
        }

        MainView {
            tabs,
            active_tab,
            groups: groups
                .iter()
                .map(|g| GroupItem {
                    id: g.id.to_string(),
                    parent: g.parent.map(|p| p.to_string()),
                    name: g.name.clone(),
                    depth: g.depth,
                    is_recycle_bin: g.is_recycle_bin,
                    entry_count: db.vault.entry_count(g.id),
                })
                .collect(),
            selected_group: db.selected_group.map(|g| g.to_string()),
            search,
            entries: list
                .iter()
                .map(|e| EntrySummary {
                    id: e.id.to_string(),
                    title: e.title.clone(),
                    username: e.username.clone(),
                    url: e.url.clone(),
                    stale: e.stale,
                })
                .collect(),
            selected_entry: db.selected_entry.map(|e| e.to_string()),
            detail,
            save_error,
            notes_monospace,
            recycle_bin_enabled: db.vault.recycle_bin_enabled(),
        }
    }
}

pub fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Pre-order groups with the Recycle Bin subtree moved to the end.
pub fn ordered_groups(vault: &Vault) -> Vec<GroupView> {
    let all = vault.groups();
    let mut main = Vec::with_capacity(all.len());
    let mut bin = Vec::new();
    let mut bin_depth: Option<usize> = None;
    for g in all {
        match bin_depth {
            Some(d) if g.depth > d => bin.push(g),
            _ => {
                if g.is_recycle_bin {
                    bin_depth = Some(g.depth);
                    bin.push(g);
                } else {
                    bin_depth = None;
                    main.push(g);
                }
            }
        }
    }
    main.extend(bin);
    main
}

/// True when `group` is the Recycle Bin or inside it. `groups` must come from `ordered_groups`,
/// which places the whole bin subtree after the first bin group.
pub fn in_bin(groups: &[GroupView], group: GroupId) -> bool {
    let Some(start) = groups.iter().position(|g| g.is_recycle_bin) else { return false };
    groups[start..].iter().any(|g| g.id == group)
}

fn entry_detail(e: &EntryView, groups: &[GroupView]) -> EntryDetail {
    EntryDetail {
        id: e.id.to_string(),
        title: e.title.clone(),
        username: e.username.clone(),
        url: e.url.clone(),
        notes: e.notes.clone(),
        tags: e.tags.clone(),
        stale: e.stale,
        in_bin: in_bin(groups, e.group),
        modified: e
            .modified
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        has_password: !e.password.is_empty(),
        has_recovery_pin: !e.recovery_pin.is_empty(),
        recovery_phone: e.recovery_phone.clone(),
        has_security_questions: !e.security_questions.trim().is_empty(),
        custom: e
            .custom
            .iter()
            .map(|c| CustomFieldView {
                key: c.key.clone(),
                value: if c.protected { None } else { Some(c.value.clone()) },
                protected: c.protected,
            })
            .collect(),
        attachments: e.attachments.clone(),
    }
}

pub fn parse_entry_id(s: &str) -> Result<EntryId, String> {
    uuid::Uuid::parse_str(s).map(EntryId).map_err(|_| format!("Bad entry id: {s}"))
}

pub fn parse_group_id(s: &str) -> Result<GroupId, String> {
    uuid::Uuid::parse_str(s).map(GroupId).map_err(|_| format!("Bad group id: {s}"))
}
