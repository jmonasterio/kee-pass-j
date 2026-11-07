//! Tauri commands: the whole surface the web frontend can call (see docs/ARCHITECTURE.md).
//! Commands are `async` so they run off the main thread: Argon2 key derivation and the
//! blocking native file dialogs must not freeze the window.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

use ourlogins::autotype;
use ourlogins::generator::{self, GenOptions};
use ourlogins::settings;
use ourlogins::vault::{self, CustomField, EntryEdit, EntryView, Vault};

use crate::dto::*;
use crate::state::{filter_shows, folders, ordered_groups, parse_entry_id, parse_filter, parse_group_id, AppState, Filter, OpenDb, Shared};
use crate::typing;

type R<T> = Result<T, String>;

/// Folder colours the user can pick; the frontend maps each name to a light and a dark shade.
const FOLDER_COLORS: [&str; 10] = ["red", "orange", "yellow", "lime", "green", "teal", "cyan", "blue", "purple", "pink"];

pub struct Platform {
    pub backdrop: bool,
    /// Problem found during startup (hotkey registration) to show once the page is up.
    pub notice: Option<String>,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn file_dialog(app: &AppHandle) -> FileDialogBuilder<tauri::Wry> {
    let d = app.dialog().file();
    match app.get_webview_window("main") {
        Some(w) => d.set_parent(&w),
        None => d,
    }
}

fn kdbx_dialog(app: &AppHandle) -> FileDialogBuilder<tauri::Wry> {
    file_dialog(app).add_filter("KeePass database", &["kdbx"])
}

fn into_path(p: Option<tauri_plugin_dialog::FilePath>) -> Option<PathBuf> {
    p.and_then(|p| p.into_path().ok())
}

fn open_external(target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let clean = target.replace('"', "");
        std::process::Command::new("cmd")
            .raw_arg(format!("/C start \"\" \"{clean}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(target).spawn().map(|_| ())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(target).spawn().map(|_| ())
    }
}

fn add_open_db(app: &AppHandle, st: &mut AppState, vault: Vault, keyfile: Option<PathBuf>) -> MainView {
    let path = vault.path().to_path_buf();
    st.dbs.push(OpenDb {
        vault,
        filter: Filter::All,
        selected_entry: None,
    });
    st.active_tab = st.dbs.len() - 1;
    st.search.clear();
    st.settings.touch_recent(&path, keyfile.as_deref());
    if !st.picker.contains(&path) {
        st.picker.insert(0, path.clone());
    }
    st.picker_selected = Some(path);
    if let Err(e) = st.save_settings() {
        typing::toast(app, &e);
    }
    st.main_view()
}

/// Stores `keyfile` for `path`, adding it to the recent list when it was only picked this
/// session (Open file…).
fn set_recent_keyfile(app: &AppHandle, st: &mut AppState, path: &Path, keyfile: Option<PathBuf>) -> UnlockView {
    match st.settings.recent.iter_mut().find(|r| r.path == path) {
        Some(r) => r.keyfile = keyfile,
        None => st.settings.touch_recent(path, keyfile.as_deref()),
    }
    st.picker_selected = Some(path.to_path_buf());
    if let Err(e) = st.save_settings() {
        typing::toast(app, &e);
    }
    st.unlock_view()
}

fn selected_entry(st: &AppState) -> R<EntryView> {
    let db = st.active().ok_or("No database is open.")?;
    let id = db.selected_entry.ok_or("No account is selected.")?;
    db.vault.entry(id).ok_or_else(|| "That account no longer exists.".into())
}

fn field_value(e: &EntryView, field: &str) -> R<String> {
    let v = match field {
        "username" => e.username.clone(),
        "password" => e.password.clone(),
        "url" => e.url.clone(),
        "recoveryPin" => e.recovery_pin.clone(),
        "recoveryPhone" => e.recovery_phone.clone(),
        "securityQuestions" => e.security_questions.clone(),
        f => match f.strip_prefix("custom:") {
            Some(key) => e
                .custom
                .iter()
                .find(|c| c.key == key)
                .map(|c| c.value.clone())
                .ok_or_else(|| format!("No field named \"{key}\"."))?,
            None => return Err(format!("Unknown field {f}.")),
        },
    };
    Ok(v)
}

fn split_tags(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

fn gen_options(st: &AppState, opts: GenOptionsDto) -> GenOptions {
    GenOptions {
        length: opts.length.clamp(4, 128),
        digits: opts.digits,
        symbols: opts.symbols,
        uppercase: st.settings.generator.uppercase,
        exclude_ambiguous: st.settings.generator.exclude_ambiguous,
    }
}

/// Runs a vault mutation on the active database, saves, and returns the new view.
fn mutate<T>(st: &mut AppState, what: &str, f: impl FnOnce(&mut OpenDb) -> Result<T, vault::VaultError>) -> R<MainView> {
    let db = st.active_or_err()?;
    f(db).map_err(|e| format!("{what}: {e}"))?;
    st.save_active();
    Ok(st.main_view())
}

// ---------------------------------------------------------------------------
// platform / unlock
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn platform_info(platform: State<'_, Platform>) -> R<PlatformInfo> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    Ok(PlatformInfo { os, backdrop: platform.backdrop, notice: platform.notice.clone() })
}

#[tauri::command]
pub async fn unlock_view(state: State<'_, Shared>) -> R<UnlockView> {
    Ok(state.lock().unlock_view())
}

#[tauri::command]
pub async fn unlock(app: AppHandle, state: State<'_, Shared>, path: String, password: String) -> R<MainView> {
    let path = PathBuf::from(path);
    let keyfile = {
        let mut st = state.lock();
        st.picker_selected = Some(path.clone());
        if st.dbs.iter().any(|db| db.vault.path() == path) {
            return Err("That database is already open.".into());
        }
        st.settings
            .recent
            .iter()
            .find(|r| r.path == path)
            .and_then(|r| r.keyfile.clone())
    };
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }
    // Key derivation takes a second or more; keep the state unlocked meanwhile.
    let vault = Vault::open(&path, &password, keyfile.as_deref()).map_err(|e| format!("Could not unlock: {e}"))?;
    let mut st = state.lock();
    if st.dbs.iter().any(|db| db.vault.path() == path) {
        return Err("That database is already open.".into());
    }
    Ok(add_open_db(&app, &mut st, vault, keyfile))
}

#[tauri::command]
pub async fn browse_open(app: AppHandle, state: State<'_, Shared>) -> R<UnlockView> {
    let picked = into_path(kdbx_dialog(&app).blocking_pick_file());
    let mut st = state.lock();
    if let Some(path) = picked {
        st.picker.retain(|p| p != &path);
        st.picker.insert(0, path.clone());
        st.picker_selected = Some(path);
        st.pending_new_db = None;
    }
    Ok(st.unlock_view())
}

#[tauri::command]
pub async fn browse_new(app: AppHandle, state: State<'_, Shared>) -> R<UnlockView> {
    let picked = into_path(kdbx_dialog(&app).set_file_name("Passwords.kdbx").blocking_save_file());
    let mut st = state.lock();
    if let Some(mut path) = picked {
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("kdbx")) {
            path.set_extension("kdbx");
        }
        st.pending_new_db = Some(path);
    }
    Ok(st.unlock_view())
}

#[tauri::command]
pub async fn create_database(app: AppHandle, state: State<'_, Shared>, password: String, confirm: String) -> R<MainView> {
    if password.is_empty() {
        return Err("The master password cannot be empty.".into());
    }
    if password != confirm {
        return Err("The two passwords differ.".into());
    }
    let path = state
        .lock()
        .pending_new_db
        .clone()
        .ok_or("Choose where to save the new database first.")?;
    let vault = Vault::create(&path, &password).map_err(|e| format!("Could not create the database: {e}"))?;
    let mut st = state.lock();
    st.pending_new_db = None;
    Ok(add_open_db(&app, &mut st, vault, None))
}

#[tauri::command]
pub async fn cancel_create(state: State<'_, Shared>) -> R<UnlockView> {
    let mut st = state.lock();
    st.pending_new_db = None;
    Ok(st.unlock_view())
}

#[tauri::command]
pub async fn forget_recent(app: AppHandle, state: State<'_, Shared>, path: String) -> R<UnlockView> {
    let path = PathBuf::from(path);
    let mut st = state.lock();
    st.picker.retain(|p| p != &path);
    st.settings.recent.retain(|r| r.path != path);
    if st.picker_selected.as_ref() == Some(&path) {
        st.picker_selected = st.picker.first().cloned();
    }
    if let Err(e) = st.save_settings() {
        typing::toast(&app, &e);
    }
    Ok(st.unlock_view())
}

#[tauri::command]
pub async fn choose_keyfile(app: AppHandle, state: State<'_, Shared>, path: String) -> R<UnlockView> {
    let picked = into_path(file_dialog(&app).set_title("Choose key file").blocking_pick_file());
    let mut st = state.lock();
    match picked {
        Some(keyfile) => Ok(set_recent_keyfile(&app, &mut st, Path::new(&path), Some(keyfile))),
        None => Ok(st.unlock_view()),
    }
}

#[tauri::command]
pub async fn clear_keyfile(app: AppHandle, state: State<'_, Shared>, path: String) -> R<UnlockView> {
    let mut st = state.lock();
    Ok(set_recent_keyfile(&app, &mut st, Path::new(&path), None))
}

// ---------------------------------------------------------------------------
// main page
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn main_view(state: State<'_, Shared>) -> R<MainView> {
    Ok(state.lock().main_view())
}

#[tauri::command]
pub async fn select_tab(state: State<'_, Shared>, index: usize) -> R<MainView> {
    let mut st = state.lock();
    if index < st.dbs.len() {
        st.active_tab = index;
        st.search.clear();
    }
    Ok(st.main_view())
}

#[tauri::command]
pub async fn lock_tab(state: State<'_, Shared>, index: usize) -> R<MainView> {
    let mut st = state.lock();
    if index >= st.dbs.len() {
        return Ok(st.main_view());
    }
    st.dbs.remove(index);
    st.save_error = match st.save_error.take() {
        Some((t, _)) if t == index => None,
        Some((t, e)) if t > index => Some((t - 1, e)),
        other => other,
    };
    if st.active_tab > index || st.active_tab >= st.dbs.len() {
        st.active_tab = st.active_tab.saturating_sub(1);
    }
    st.search.clear();
    Ok(st.main_view())
}

#[tauri::command]
pub async fn select_filter(state: State<'_, Shared>, filter: FilterDto) -> R<MainView> {
    let filter = parse_filter(&filter)?;
    let mut st = state.lock();
    st.active_or_err()?.filter = filter;
    st.search.clear();
    Ok(st.main_view())
}

/// Adds a folder inside `parent`, or at the top level when `parent` is `None`, and shows it.
#[tauri::command]
pub async fn add_group(state: State<'_, Shared>, parent: Option<String>, name: String) -> R<MainView> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the folder a name.".into());
    }
    let parent = parent.as_deref().map(parse_group_id).transpose()?;
    let mut st = state.lock();
    mutate(&mut st, "Could not add the folder", |db| {
        let groups = ordered_groups(&db.vault);
        let parent = parent.filter(|p| folders(&groups).iter().any(|f| f.id == *p)).unwrap_or(db.vault.root());
        let id = db.vault.add_group(parent, &name)?;
        db.filter = Filter::Group(id);
        Ok(())
    })
}

/// Sets a folder's colour (one of `FOLDER_COLORS`), or back to automatic with `None`.
#[tauri::command]
pub async fn set_group_color(state: State<'_, Shared>, id: String, color: Option<String>) -> R<MainView> {
    let gid = parse_group_id(&id)?;
    if let Some(c) = color.as_deref() {
        if !FOLDER_COLORS.contains(&c) {
            return Err(format!("Unknown colour {c}."));
        }
    }
    let mut st = state.lock();
    mutate(&mut st, "Could not change the colour", |db| db.vault.set_group_color(gid, color.as_deref()))
}

#[tauri::command]
pub async fn select_entry(state: State<'_, Shared>, id: String) -> R<MainView> {
    let eid = parse_entry_id(&id)?;
    let mut st = state.lock();
    st.active_or_err()?.selected_entry = Some(eid);
    Ok(st.main_view())
}

#[tauri::command]
pub async fn search(state: State<'_, Shared>, text: String) -> R<MainView> {
    let mut st = state.lock();
    st.search = text;
    Ok(st.main_view())
}

#[tauri::command]
pub async fn toggle_stale(state: State<'_, Shared>) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    mutate(&mut st, "Could not change the flag", |db| db.vault.set_stale(e.id, !e.stale))
}

#[tauri::command]
pub async fn delete_entry(state: State<'_, Shared>) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    mutate(&mut st, "Could not delete", |db| {
        db.vault.delete_entry(e.id)?;
        db.selected_entry = None;
        Ok(())
    })
}

/// Moves a folder with everything in it to the Recycle Bin, or deletes it for good when it is
/// already there (frontend confirms first). A filter on the deleted folder moves to its parent.
#[tauri::command]
pub async fn delete_group(state: State<'_, Shared>, id: String) -> R<MainView> {
    let gid = parse_group_id(&id)?;
    let mut st = state.lock();
    mutate(&mut st, "Could not delete the group", |db| {
        let parent = db.vault.groups().into_iter().find(|g| g.id == gid).and_then(|g| g.parent);
        db.vault.delete_group(gid)?;
        if db.filter == Filter::Group(gid) {
            db.filter = match parent {
                Some(p) if p != db.vault.root() => Filter::Group(p),
                _ => Filter::All,
            };
        }
        Ok(())
    })
}

/// Deletes everything in the Recycle Bin for good (frontend confirms first).
#[tauri::command]
pub async fn empty_recycle_bin(state: State<'_, Shared>) -> R<MainView> {
    let mut st = state.lock();
    mutate(&mut st, "Could not empty the Recycle Bin", |db| {
        if let Some(bin) = db.vault.groups().into_iter().find(|g| g.is_recycle_bin) {
            db.vault.delete_group(bin.id)?;
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn add_attachment(app: AppHandle, state: State<'_, Shared>) -> R<MainView> {
    let id = selected_entry(&state.lock())?.id;
    let Some(path) = into_path(file_dialog(&app).blocking_pick_file()) else {
        return Ok(state.lock().main_view());
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read the file: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into());
    let mut st = state.lock();
    mutate(&mut st, "Could not attach", |db| db.vault.add_attachment(id, &name, bytes))
}

#[tauri::command]
pub async fn save_attachment(app: AppHandle, state: State<'_, Shared>, name: String) -> R<Option<String>> {
    let bytes = {
        let st = state.lock();
        let e = selected_entry(&st)?;
        st.active()
            .and_then(|db| db.vault.attachment(e.id, &name))
            .ok_or("Attachment data is missing.")?
    };
    let Some(target) = into_path(file_dialog(&app).set_file_name(&name).blocking_save_file()) else {
        return Ok(None);
    };
    std::fs::write(&target, bytes).map_err(|e| format!("Could not write the file: {e}"))?;
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
pub async fn remove_attachment(state: State<'_, Shared>, name: String) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    mutate(&mut st, "Could not remove", |db| db.vault.remove_attachment(e.id, &name))
}

#[tauri::command]
pub async fn reveal_field(state: State<'_, Shared>, field: String) -> R<String> {
    field_value(&selected_entry(&state.lock())?, &field)
}

#[tauri::command]
pub async fn copy_field(app: AppHandle, state: State<'_, Shared>, field: String) -> R<()> {
    let value = field_value(&selected_entry(&state.lock())?, &field)?;
    app.clipboard().write_text(value).map_err(|e| format!("Could not copy: {e}"))
}

#[tauri::command]
pub async fn copy_text(app: AppHandle, text: String) -> R<()> {
    app.clipboard().write_text(text).map_err(|e| format!("Could not copy: {e}"))
}

#[tauri::command]
pub async fn open_url(url: String) -> R<()> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(());
    }
    let target = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    open_external(&target).map_err(|e| format!("Could not open the browser: {e}"))
}

#[tauri::command]
pub async fn retry_save(state: State<'_, Shared>) -> R<MainView> {
    let mut st = state.lock();
    let tab = st.save_error.as_ref().map_or(st.active_tab, |(t, _)| *t);
    st.save_tab(tab);
    Ok(st.main_view())
}

// ---------------------------------------------------------------------------
// add / edit form
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn form_open(state: State<'_, Shared>, mode: String) -> R<FormData> {
    let st = state.lock();
    let db = st.active().ok_or("No database is open.")?;
    let groups = ordered_groups(&db.vault);
    let g = &st.settings.generator;
    let gen = GenOptionsDto { length: g.length, digits: g.digits, symbols: g.symbols };
    let form_groups = groups
        .iter()
        .map(|g| FormGroup {
            id: g.id.to_string(),
            name: if g.depth == 0 { "No folder".to_string() } else { g.name.clone() },
            depth: g.depth,
            is_recycle_bin: g.is_recycle_bin,
        })
        .collect();
    let entry = match mode.as_str() {
        "add" => {
            let group = match db.filter {
                Filter::Group(g) => g,
                _ => db.vault.root(),
            };
            EntryForm {
                id: None,
                group: group.to_string(),
                title: String::new(),
                url: String::new(),
                username: String::new(),
                password: generator::generate(&st.settings.generator),
                recovery_pin: String::new(),
                recovery_phone: String::new(),
                security_questions: String::new(),
                notes: String::new(),
                tags: String::new(),
                autotype_sequence: String::new(),
                autotype_windows: String::new(),
                custom: Vec::new(),
            }
        }
        "edit" => {
            let e = selected_entry(&st)?;
            EntryForm {
                id: Some(e.id.to_string()),
                group: e.group.to_string(),
                title: e.title,
                url: e.url,
                username: e.username,
                password: e.password,
                recovery_pin: e.recovery_pin,
                recovery_phone: e.recovery_phone,
                security_questions: e.security_questions,
                notes: e.notes,
                tags: e.tags.join(", "),
                autotype_sequence: e.autotype_sequence.unwrap_or_default(),
                autotype_windows: e.autotype_windows.join("\n"),
                custom: e
                    .custom
                    .into_iter()
                    .map(|c| CustomFieldDto { key: c.key, value: c.value, protected: c.protected })
                    .collect(),
            }
        }
        other => return Err(format!("Unknown form mode {other}.")),
    };
    Ok(FormData {
        mode: if entry.id.is_some() { "edit" } else { "add" },
        groups: form_groups,
        gen,
        entry,
    })
}

#[tauri::command]
pub async fn generate(state: State<'_, Shared>, opts: GenOptionsDto) -> R<String> {
    let st = state.lock();
    Ok(generator::generate(&gen_options(&st, opts)))
}

#[tauri::command]
pub async fn find_duplicate(state: State<'_, Shared>, title: String, url: String, editing: Option<String>) -> R<Option<Duplicate>> {
    let host = vault::url_host(url.trim())
        .or_else(|| vault::url_host(title.trim()))
        .filter(|h| h.len() >= 3);
    let Some(host) = host else { return Ok(None) };
    let editing = editing.as_deref().map(parse_entry_id).transpose()?;
    let st = state.lock();
    let Some(db) = st.active() else { return Ok(None) };
    Ok(db
        .vault
        .find_by_host(&host)
        .into_iter()
        .find(|e| Some(e.id) != editing && !e.stale)
        .map(|e| Duplicate { id: e.id.to_string(), title: e.title }))
}

#[tauri::command]
pub async fn form_save(state: State<'_, Shared>, entry: EntryForm, replace: Option<String>) -> R<MainView> {
    let title = entry.title.trim().to_string();
    if title.is_empty() {
        return Err("Give the account a name first.".into());
    }
    let id = entry.id.as_deref().map(parse_entry_id).transpose()?;
    let replace = replace.as_deref().map(parse_entry_id).transpose()?;
    let group = parse_group_id(&entry.group)?;
    let mut st = state.lock();
    let db = st.active_or_err()?;
    let group = if db.vault.groups().iter().any(|g| g.id == group) { group } else { db.vault.root() };
    let stale = id.and_then(|id| db.vault.entry(id)).is_some_and(|e| e.stale);
    let sequence = entry.autotype_sequence.trim().to_string();
    let edit = EntryEdit {
        id,
        group,
        title,
        username: entry.username.trim().to_string(),
        password: entry.password,
        url: entry.url.trim().to_string(),
        notes: entry.notes,
        tags: split_tags(&entry.tags),
        stale,
        recovery_pin: entry.recovery_pin.trim().to_string(),
        recovery_phone: entry.recovery_phone.trim().to_string(),
        security_questions: entry.security_questions,
        custom: entry
            .custom
            .into_iter()
            .filter(|c| !c.key.trim().is_empty())
            .map(|c| CustomField { key: c.key.trim().to_string(), value: c.value, protected: c.protected })
            .collect(),
        autotype_sequence: if sequence.is_empty() { None } else { Some(sequence) },
        autotype_windows: entry
            .autotype_windows
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    };
    let new_id = db
        .vault
        .upsert_entry(edit)
        .and_then(|new_id| {
            if let Some(old) = replace.filter(|old| *old != new_id) {
                db.vault.set_stale(old, true)?;
            }
            Ok(new_id)
        })
        .map_err(|e| format!("Could not save the account: {e}"))?;
    db.selected_entry = Some(new_id);
    // Keep the current list if the saved account shows in it; otherwise show its folder.
    let groups = ordered_groups(&db.vault);
    if let Some(saved) = db.vault.entry(new_id) {
        if !filter_shows(db.filter, &groups, &saved) {
            db.filter = if saved.group == db.vault.root() { Filter::All } else { Filter::Group(saved.group) };
        }
    }
    st.search.clear();
    st.save_active();
    Ok(st.main_view())
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn settings_get(state: State<'_, Shared>) -> R<SettingsView> {
    let st = state.lock();
    let s = &st.settings;
    Ok(SettingsView {
        autotype_hotkey: s.autotype_hotkey.clone(),
        autotype_default_sequence: s.autotype_default_sequence.clone(),
        reopen_last: s.reopen_last,
        notes_monospace: s.notes_monospace,
        tile_colors: s.tile_colors,
        generator: GeneratorSettings {
            length: s.generator.length,
            digits: s.generator.digits,
            symbols: s.generator.symbols,
            uppercase: s.generator.uppercase,
            exclude_ambiguous: s.generator.exclude_ambiguous,
        },
        backup_dir: settings::backup_policy().dir.display().to_string(),
    })
}

#[tauri::command]
pub async fn settings_save(app: AppHandle, state: State<'_, Shared>, settings: SettingsView) -> R<()> {
    let hotkey = settings.autotype_hotkey.trim().to_string();
    autotype::parse_hotkey(&hotkey).map_err(|e| format!("Hotkey not understood: {e}"))?;
    let sequence = settings.autotype_default_sequence.trim().to_string();
    autotype::parse_sequence(&sequence).map_err(|e| format!("Sequence not understood: {e}"))?;
    {
        let mut st = state.lock();
        let s = &mut st.settings;
        s.autotype_hotkey = hotkey;
        s.autotype_default_sequence = sequence;
        s.reopen_last = settings.reopen_last;
        s.notes_monospace = settings.notes_monospace;
        s.tile_colors = settings.tile_colors;
        let g = settings.generator;
        s.generator = GenOptions {
            length: g.length.clamp(4, 128),
            digits: g.digits,
            symbols: g.symbols,
            uppercase: g.uppercase,
            exclude_ambiguous: g.exclude_ambiguous,
        };
        s.save().map_err(|e| format!("Could not write settings: {e}"))?;
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = typing::register_hotkey(&handle) {
            typing::toast(&handle, &e);
        }
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_backup_folder() -> R<()> {
    let dir = settings::backup_policy().dir;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create the folder: {e}"))?;
    open_external(&dir.display().to_string()).map_err(|e| format!("Could not open the folder: {e}"))
}

// ---------------------------------------------------------------------------
// Auto-Type
// ---------------------------------------------------------------------------

/// Select-entry mode: type the selected entry into whichever window was active before ours.
#[tauri::command]
pub async fn autotype_selected(app: AppHandle, state: State<'_, Shared>) -> R<()> {
    let (tab, id) = {
        let st = state.lock();
        (st.active_tab, selected_entry(&st)?.id)
    };
    typing::perform_hidden(&app, "main", tab, id, true)
}

// ---------------------------------------------------------------------------
// Quick Access (the popup the global hotkey opens)
// ---------------------------------------------------------------------------

/// Searches every open database; an empty query returns nothing (the popup keeps its matches).
#[tauri::command]
pub async fn quick_search(state: State<'_, Shared>, text: String) -> R<Vec<QuickItem>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let st = state.lock();
    let mut items = Vec::new();
    for (i, db) in st.dbs.iter().enumerate() {
        for e in db.vault.search(text) {
            items.push(typing::quick_item(&st, i, &e));
        }
    }
    items.sort_by_key(|item| item.stale);
    items.truncate(50);
    Ok(items)
}

fn quick_entry(st: &AppState, db: usize, id: &str) -> R<EntryView> {
    let id = parse_entry_id(id)?;
    st.dbs
        .get(db)
        .and_then(|d| d.vault.entry(id))
        .ok_or_else(|| "That account no longer exists.".into())
}

/// Hides Quick Access so the window behind it gets the focus back, then types the account.
#[tauri::command]
pub async fn quick_type(app: AppHandle, state: State<'_, Shared>, db: usize, id: String) -> R<()> {
    let entry = quick_entry(&state.lock(), db, &id)?;
    typing::perform_hidden(&app, "quick", db, entry.id, false)
}

/// Copies one field of the account and closes Quick Access.
#[tauri::command]
pub async fn quick_copy(app: AppHandle, state: State<'_, Shared>, db: usize, id: String, field: String) -> R<()> {
    let value = field_value(&quick_entry(&state.lock(), db, &id)?, &field)?;
    app.clipboard().write_text(value).map_err(|e| format!("Could not copy: {e}"))?;
    if let Some(w) = app.get_webview_window("quick") {
        let _ = w.hide();
    }
    Ok(())
}

#[tauri::command]
pub async fn quick_hide(app: AppHandle) -> R<()> {
    if let Some(w) = app.get_webview_window("quick") {
        let _ = w.hide();
    }
    Ok(())
}

/// "Open Our Logins" from Quick Access (e.g. to unlock a database first).
#[tauri::command]
pub async fn quick_open_main(app: AppHandle) -> R<()> {
    if let Some(w) = app.get_webview_window("quick") {
        let _ = w.hide();
    }
    typing::bring_main_to_front(&app);
    Ok(())
}
