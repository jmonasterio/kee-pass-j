//! Tauri commands: the whole surface the web frontend can call (see docs/ARCHITECTURE.md).
//! Commands are `async` so they run off the main thread: Argon2 key derivation and the
//! blocking native file dialogs must not freeze the window.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

use ourlogins::autotype;
use ourlogins::generator::{self, GenOptions};
use ourlogins::settings;
use ourlogins::vault::{self, CustomField, EntryEdit, EntryView, Vault};

use crate::autolock;
use crate::biometric;
use crate::clipboard;
use crate::dto::*;
use crate::saver;
use crate::state::{filter_shows, folders, ordered_groups, parse_entry_id, parse_filter, parse_group_id, AppState, Filter, LastSave, OpenDb, Shared};
use crate::typing;

type R<T> = Result<T, String>;

/// Folder colors the user can pick; the frontend maps each name to a light and a dark shade.
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
    st.note_activity();
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

/// The selected entry, or its history version `version` (0 = the newest earlier one).
fn shown_entry(st: &AppState, version: Option<usize>) -> R<EntryView> {
    let e = selected_entry(st)?;
    let Some(index) = version else { return Ok(e) };
    st.active()
        .and_then(|db| db.vault.version(e.id, index))
        .map(|(v, _)| v)
        .ok_or_else(|| "That version no longer exists.".into())
}

/// The bytes of a file of the selected entry, or of its history version `version`.
fn attachment_bytes(st: &AppState, version: Option<usize>, name: &str) -> R<Vec<u8>> {
    let e = selected_entry(st)?;
    let db = st.active().ok_or("No database is open.")?;
    match version {
        None => db.vault.attachment(e.id, name),
        Some(index) => db.vault.version_attachment(e.id, index, name),
    }
    .ok_or_else(|| "Attachment data is missing.".into())
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

/// Gives keyboard focus to the main window's web content. At launch WebView2 does not have
/// it, so `focus()` on the password box marks it focused but typed keys go nowhere.
#[tauri::command]
pub async fn focus_window(window: tauri::WebviewWindow) -> R<()> {
    window.set_focus().map_err(|e| e.to_string())?;
    window.as_ref().set_focus().map_err(|e| e.to_string())
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
    let (view, offer_quick_unlock) = {
        let mut st = state.lock();
        if st.dbs.iter().any(|db| db.vault.path() == path) {
            return Err("That database is already open.".into());
        }
        let offer = st.quick_unlock_label().is_some() && !biometric::has(&path);
        (add_open_db(&app, &mut st, vault, keyfile), offer)
    };
    // First unlock with the password on a machine with Windows Hello / Touch ID: keep the
    // password behind it for next time. Windows shows its prompt here.
    if offer_quick_unlock {
        if let Err(e) = biometric::store(&path, &password, parent_window(&app)) {
            typing::toast(&app, &format!("Quick unlock was not set up: {e}"));
        }
    }
    Ok(view)
}

/// Unlocks with the password stored behind Windows Hello / Touch ID.
#[tauri::command]
pub async fn unlock_quick(app: AppHandle, state: State<'_, Shared>, path: String) -> R<MainView> {
    let path = PathBuf::from(path);
    let (keyfile, name) = {
        let mut st = state.lock();
        st.picker_selected = Some(path.clone());
        if st.dbs.iter().any(|db| db.vault.path() == path) {
            return Err("That database is already open.".into());
        }
        let keyfile = st.settings.recent.iter().find(|r| r.path == path).and_then(|r| r.keyfile.clone());
        (keyfile, crate::state::file_stem(&path))
    };
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }
    let password = biometric::retrieve(&path, &format!("unlock {name}"), parent_window(&app))?;
    let vault = match Vault::open(&path, &password, keyfile.as_deref()) {
        Ok(v) => v,
        Err(vault::VaultError::WrongKey) => {
            // The master password was changed elsewhere; the stored one is useless now.
            biometric::forget(&path);
            return Err("The master password has changed. Unlock with the new password once to set up quick unlock again.".into());
        }
        Err(e) => return Err(format!("Could not unlock: {e}")),
    };
    let mut st = state.lock();
    if st.dbs.iter().any(|db| db.vault.path() == path) {
        return Err("That database is already open.".into());
    }
    Ok(add_open_db(&app, &mut st, vault, keyfile))
}

/// Native handle of the main window, so the Windows Hello prompt opens in front of it.
fn parent_window(app: &AppHandle) -> Option<isize> {
    #[cfg(windows)]
    {
        app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
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
    biometric::forget(&path);
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
    // Key files usually sit next to the database, so the dialog starts in its folder.
    let mut dialog = file_dialog(&app).set_title("Choose key file");
    if let Some(dir) = Path::new(&path).parent().filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let picked = into_path(dialog.blocking_pick_file());
    let mut st = state.lock();
    match picked {
        Some(keyfile) => Ok(set_recent_keyfile(&app, &mut st, Path::new(&path), Some(keyfile))),
        None => Ok(st.unlock_view()),
    }
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

/// Locks tab `index` after its changes are on disk. With `discard`, a failing save does not
/// stop the lock (the window has asked "Lock without saving?").
#[tauri::command]
pub async fn lock_tab(state: State<'_, Shared>, index: usize, discard: bool) -> R<MainView> {
    let Some(path) = state.lock().dbs.get(index).map(|d| d.vault.path().to_path_buf()) else {
        return Ok(state.lock().main_view());
    };
    let mut st = saver::wait_idle(state.inner(), &path);
    let Some(index) = st.dbs.iter().position(|d| d.vault.path() == path) else {
        return Ok(st.main_view());
    };
    if let Err(e) = saver::save_now(&mut st, index) {
        if !discard {
            return Err(format!("Could not save before locking: {e}"));
        }
    }
    close_db(&mut st, index);
    Ok(st.main_view())
}

/// Takes database `index` out of the open list (its changes are saved, or given up, by the
/// caller) and forgets what belonged to it.
pub fn close_db(st: &mut AppState, index: usize) {
    let db = st.dbs.remove(index);
    let path = db.vault.path();
    if st.save_error.as_ref().is_some_and(|(p, _)| p == path) {
        st.save_error = None;
    }
    if st.active_tab > index || st.active_tab >= st.dbs.len() {
        st.active_tab = st.active_tab.saturating_sub(1);
    }
    st.search.clear();
    st.draft_files.clear();
    if st.last_save.as_ref().is_some_and(|l| l.path == path) {
        st.last_save = None;
    }
    if st.dbs.is_empty() {
        // No database open: remove temporary copies of opened attachments.
        clear_open_dir();
    }
}

/// Auto-lock, after the page has saved its unsaved edits: saves and locks every open
/// database. Returns what could not be saved (those databases are locked anyway).
#[tauri::command]
pub async fn lock_all(state: State<'_, Shared>) -> R<Vec<String>> {
    Ok(autolock::lock_everything(state.inner()))
}

/// Input in one of the windows (the page reports it at most every 30 s): auto-lock counts
/// from now.
#[tauri::command]
pub async fn note_activity(state: State<'_, Shared>) -> R<()> {
    state.lock().note_activity();
    Ok(())
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

#[tauri::command]
pub async fn rename_group(state: State<'_, Shared>, id: String, name: String) -> R<MainView> {
    let gid = parse_group_id(&id)?;
    let mut st = state.lock();
    mutate(&mut st, "Could not rename the folder", |db| db.vault.rename_group(gid, &name))
}

/// Sets a folder's color (one of `FOLDER_COLORS`), or back to automatic with `None`.
#[tauri::command]
pub async fn set_group_color(state: State<'_, Shared>, id: String, color: Option<String>) -> R<MainView> {
    let gid = parse_group_id(&id)?;
    if let Some(c) = color.as_deref() {
        if !FOLDER_COLORS.contains(&c) {
            return Err(format!("Unknown color {c}."));
        }
    }
    let mut st = state.lock();
    mutate(&mut st, "Could not change the color", |db| db.vault.set_group_color(gid, color.as_deref()))
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

/// Folder for temporary copies of attachments opened in other apps. Deleted when the last
/// database is locked, when the app quits, and at the next start (in case it crashed).
pub fn open_dir() -> PathBuf {
    settings::data_dir().join("open")
}

pub fn clear_open_dir() {
    let _ = std::fs::remove_dir_all(open_dir());
}

/// Opens an attachment in its usual app (PDF reader, image viewer…) from a temporary copy.
/// `version`: a file of that history version of the entry.
#[tauri::command]
pub async fn open_attachment(state: State<'_, Shared>, name: String, version: Option<usize>) -> R<()> {
    let bytes = attachment_bytes(&state.lock(), version, &name)?;
    // Only the last path component: an attachment named "..\x" must not escape the folder.
    let file_name = Path::new(&name).file_name().ok_or("The attachment has no usable file name.")?;
    let dir = open_dir().join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create a temporary folder: {e}"))?;
    let target = dir.join(file_name);
    std::fs::write(&target, bytes).map_err(|e| format!("Could not write a temporary copy: {e}"))?;
    open_external(&target.display().to_string()).map_err(|e| format!("Could not open the file: {e}"))
}

/// Turns the selected entry into a document (text and files) or back into an account.
#[tauri::command]
pub async fn set_document(state: State<'_, Shared>, document: bool) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    mutate(&mut st, "Could not change the entry", |db| db.vault.set_document(e.id, document))
}

#[tauri::command]
pub async fn about_info() -> R<AboutInfo> {
    Ok(AboutInfo {
        version: env!("CARGO_PKG_VERSION"),
        repository: "https://github.com/jmonasterio/kee-pass-j",
    })
}

#[tauri::command]
pub async fn save_attachment(app: AppHandle, state: State<'_, Shared>, name: String, version: Option<usize>) -> R<Option<String>> {
    let bytes = attachment_bytes(&state.lock(), version, &name)?;
    let Some(target) = into_path(file_dialog(&app).set_file_name(&name).blocking_save_file()) else {
        return Ok(None);
    };
    std::fs::write(&target, bytes).map_err(|e| format!("Could not write the file: {e}"))?;
    Ok(Some(target.display().to_string()))
}

/// One field of the selected entry, or of its history version `version`.
#[tauri::command]
pub async fn reveal_field(state: State<'_, Shared>, field: String, version: Option<usize>) -> R<String> {
    field_value(&shown_entry(&state.lock(), version)?, &field)
}

/// Copies one field of the selected entry (or of its history version `version`). Returns
/// the seconds until the clipboard is cleared (0 = it is not).
#[tauri::command]
pub async fn copy_field(state: State<'_, Shared>, field: String, version: Option<usize>) -> R<u32> {
    let (value, clear) = {
        let mut st = state.lock();
        st.note_activity();
        (field_value(&shown_entry(&st, version)?, &field)?, st.settings.clipboard_clear_seconds)
    };
    clipboard::copy(&value, clear)?;
    Ok(clear)
}

/// Copies text from the page (a generated password, a notes line). Returns the seconds until
/// the clipboard is cleared (0 = it is not).
#[tauri::command]
pub async fn copy_text(state: State<'_, Shared>, text: String) -> R<u32> {
    let clear = {
        let mut st = state.lock();
        st.note_activity();
        st.settings.clipboard_clear_seconds
    };
    clipboard::copy(&text, clear)?;
    Ok(clear)
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
    let failed = st.save_error.as_ref().and_then(|(p, _)| st.dbs.iter().position(|d| d.vault.path() == p));
    let tab = failed.unwrap_or(st.active_tab);
    st.save_tab(tab);
    Ok(st.main_view())
}

/// Changes the master password of the active database and saves at once, so the result is
/// known before the dialog closes. The key file, if any, stays the same. Backups made
/// before the change still open with the old password.
#[tauri::command]
pub async fn change_master_password(state: State<'_, Shared>, current: String, new: String) -> R<MainView> {
    if new.is_empty() {
        return Err("Type the new password.".into());
    }
    let Some(path) = state.lock().active().map(|d| d.vault.path().to_path_buf()) else {
        return Err("No database is open.".into());
    };
    let mut st = saver::wait_idle(state.inner(), &path);
    let keyfile = st.settings.recent.iter().find(|r| r.path == path).and_then(|r| r.keyfile.clone());
    let keyfile = keyfile.as_deref();
    let key_err = |e: vault::VaultError| format!("Could not use the key file: {e}");
    let tab = st.dbs.iter().position(|d| d.vault.path() == path).ok_or("That database is no longer open.")?;
    let vault = &mut st.dbs[tab].vault;
    if !vault.key_matches(&current, keyfile).map_err(key_err)? {
        return Err("The current password is not right.".into());
    }
    vault.change_key(&new, keyfile).map_err(key_err)?;
    if let Err(e) = saver::save_now(&mut st, tab) {
        // Keep the file and the open copy on the same password.
        st.dbs[tab].vault.change_key(&current, keyfile).map_err(key_err)?;
        return Err(format!("The password was not changed; saving failed: {e}"));
    }
    // A password kept for Windows Hello / Touch ID is the old one now. The next unlock with
    // the new password stores it again.
    biometric::forget(&path);
    Ok(st.main_view())
}

// ---------------------------------------------------------------------------
// editing in the details pane
// ---------------------------------------------------------------------------

/// Starts an edit of the selected entry (`edit`) or a new one (`add`, `addDocument`): the
/// values to edit, the folders and generator options. Clears the files picked for the last draft.
#[tauri::command]
pub async fn draft_open(state: State<'_, Shared>, mode: String) -> R<Draft> {
    let mut st = state.lock();
    st.draft_files.clear();
    let db = st.active().ok_or("No database is open.")?;
    let groups = ordered_groups(&db.vault);
    let g = &st.settings.generator;
    let gen = GenOptionsDto { length: g.length, digits: g.digits, symbols: g.symbols };
    let folders = groups
        .iter()
        .map(|g| FolderOption {
            id: g.id.to_string(),
            name: if g.depth == 0 { "No folder".to_string() } else { g.name.clone() },
            depth: g.depth,
            is_recycle_bin: g.is_recycle_bin,
        })
        .collect();
    let entry = match mode.as_str() {
        "add" | "addDocument" => {
            let document = mode == "addDocument";
            let group = match db.filter {
                Filter::Group(g) => g,
                _ => db.vault.root(),
            };
            DraftEntry {
                id: None,
                group: group.to_string(),
                title: String::new(),
                url: String::new(),
                username: String::new(),
                password: if document { String::new() } else { generator::generate(&st.settings.generator) },
                recovery_pin: String::new(),
                recovery_phone: String::new(),
                security_questions: String::new(),
                notes: String::new(),
                tags: String::new(),
                autotype_sequence: String::new(),
                autotype_windows: String::new(),
                custom: Vec::new(),
                document,
                remove_files: Vec::new(),
            }
        }
        "edit" => {
            let e = selected_entry(&st)?;
            DraftEntry {
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
                autotype_windows: e.autotype_windows.iter().map(|w| w.window.as_str()).collect::<Vec<_>>().join("\n"),
                custom: e
                    .custom
                    .into_iter()
                    .map(|c| CustomFieldDto { key: c.key, value: c.value, protected: c.protected })
                    .collect(),
                document: e.document,
                remove_files: Vec::new(),
            }
        }
        other => return Err(format!("Unknown draft mode {other}.")),
    };
    Ok(Draft { folders, gen, entry })
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

/// Saves the draft: the edited values, files picked for it and files it removes, as one change
/// (one history version) and one file save. `replace` marks that entry not working. With
/// `automatic` (saved while the user was away, or by auto-lock) an entry without a name is
/// called "Untitled" ("Untitled 2"… when taken). Remembers the save so `undo_save` can take
/// it back.
#[tauri::command]
pub async fn draft_save(state: State<'_, Shared>, entry: DraftEntry, replace: Option<String>, automatic: bool) -> R<MainView> {
    let id = entry.id.as_deref().map(parse_entry_id).transpose()?;
    let replace = replace.as_deref().map(parse_entry_id).transpose()?;
    let group = parse_group_id(&entry.group)?;
    let mut st = state.lock();
    let db = st.active_or_err()?;
    let mut title = entry.title.trim().to_string();
    if title.is_empty() {
        if !automatic {
            return Err("Give it a name first.".into());
        }
        let taken: Vec<String> = db.vault.all_entries().into_iter().chain(db.vault.recycle_bin_entries()).map(|e| e.title).collect();
        title = vault::untitled_name(&taken);
    }
    let group = if db.vault.groups().iter().any(|g| g.id == group) { group } else { db.vault.root() };
    let before = match id {
        Some(id) => Some(db.vault.entry(id).ok_or("That entry no longer exists.")?),
        None => None,
    };
    let sequence = entry.autotype_sequence.trim().to_string();
    // Files picked for the draft: read them before touching the database, so a missing file
    // stops the save with the edits still in the pane. A name the entry keeps, or another
    // picked file has, gets a number: "scan.pdf", "scan (2).pdf".
    let kept: Vec<&String> = before
        .iter()
        .flat_map(|e| e.attachments.iter())
        .filter(|n| !entry.remove_files.contains(n))
        .collect();
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(st.draft_files.len());
    for path in &st.draft_files {
        let bytes = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        let base = file_name(path);
        let mut name = base.clone();
        let mut n = 2;
        while kept.contains(&&name) || files.iter().any(|(f, _)| *f == name) {
            name = numbered_name(&base, n);
            n += 1;
        }
        files.push((name, bytes));
    }
    let edit = EntryEdit {
        id,
        group,
        title,
        username: entry.username.trim().to_string(),
        password: entry.password,
        url: entry.url.trim().to_string(),
        notes: entry.notes,
        tags: split_tags(&entry.tags),
        stale: before.as_ref().is_some_and(|e| e.stale),
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
        document: entry.document,
        attach: files,
        detach: entry.remove_files,
    };
    let path = st.active().map(|d| d.vault.path().to_path_buf()).ok_or("No database is open.")?;
    let db = st.active_or_err()?;
    let replaced = replace.filter(|old| Some(*old) != id);
    let new_id = db
        .vault
        .upsert_entry(edit)
        .and_then(|new_id| {
            if let Some(old) = replaced {
                db.vault.set_stale(old, true)?;
            }
            Ok(new_id)
        })
        .map_err(|e| format!("Could not save: {e}"))?;
    db.selected_entry = Some(new_id);
    // Keep the current list if the saved entry shows in it; otherwise show its folder.
    let groups = ordered_groups(&db.vault);
    let saved = db.vault.entry(new_id).ok_or("The saved entry is missing.")?;
    if !filter_shows(db.filter, &groups, &saved) {
        db.filter = if saved.group == db.vault.root() { Filter::All } else { Filter::Group(saved.group) };
    }
    st.last_save = Some(LastSave {
        path,
        saved,
        created: before.is_none(),
        replaced,
    });
    st.draft_files.clear();
    st.draft_pending = false;
    st.search.clear();
    st.save_active();
    Ok(st.main_view())
}

/// Undo for the last `draft_save` or `restore_version`: an edit goes back to the entry's
/// newest history version (the undone one stays in the history) and its folder, a new entry
/// is deleted, and an entry it replaced is marked working again. Refused when the entry has
/// changed since.
#[tauri::command]
pub async fn undo_save(state: State<'_, Shared>) -> R<MainView> {
    let mut st = state.lock();
    let last = st.last_save.take().ok_or("There is nothing to undo.")?;
    let tab = st
        .dbs
        .iter()
        .position(|d| d.vault.path() == last.path)
        .ok_or("That database is no longer open.")?;
    let db = &mut st.dbs[tab];
    let id = last.saved.id;
    if db.vault.entry(id).as_ref() != Some(&last.saved) {
        return Err("It has changed since that save, so it was not undone.".into());
    }
    let undone = if last.created { db.vault.remove_entry(id) } else { db.vault.restore_version(id, 0, false) };
    undone
        .and_then(|()| match last.replaced {
            Some(old) if db.vault.entry(old).is_some() => db.vault.set_stale(old, false),
            _ => Ok(()),
        })
        .map_err(|e| format!("Could not undo: {e}"))?;
    let shown = if last.created { last.replaced } else { Some(id) };
    let groups = ordered_groups(&db.vault);
    if let Some(entry) = shown.and_then(|e| db.vault.entry(e)) {
        if !filter_shows(db.filter, &groups, &entry) {
            db.filter = if entry.group == db.vault.root() { Filter::All } else { Filter::Group(entry.group) };
        }
    }
    db.selected_entry = shown;
    st.save_tab(tab);
    Ok(st.main_view())
}

// ---------------------------------------------------------------------------
// entry history
// ---------------------------------------------------------------------------

fn local_time(t: chrono::DateTime<chrono::Utc>) -> String {
    t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()
}

/// The selected entry's earlier versions, newest first, with what each next save changed.
#[tauri::command]
pub async fn entry_history(state: State<'_, Shared>) -> R<Vec<VersionItem>> {
    let st = state.lock();
    let e = selected_entry(&st)?;
    let db = st.active().ok_or("No database is open.")?;
    Ok(db
        .vault
        .versions(e.id)
        .into_iter()
        .enumerate()
        .map(|(index, v)| VersionItem { index, modified: local_time(v.modified), changes: v.changes })
        .collect())
}

/// History version `index` of the selected entry, without protected values (`reveal_field`
/// and `copy_field` with `version` get those), and the fields that differ from now.
#[tauri::command]
pub async fn entry_version(state: State<'_, Shared>, index: usize) -> R<VersionDetail> {
    let st = state.lock();
    let e = selected_entry(&st)?;
    let db = st.active().ok_or("No database is open.")?;
    let (v, differs) = db.vault.version(e.id, index).ok_or("That version no longer exists.")?;
    Ok(VersionDetail {
        index,
        title: v.title,
        username: v.username,
        url: v.url,
        notes: v.notes,
        tags: v.tags,
        stale: v.stale,
        modified: local_time(v.modified),
        has_password: !v.password.is_empty(),
        has_recovery_pin: !v.recovery_pin.is_empty(),
        recovery_phone: v.recovery_phone,
        has_security_questions: !v.security_questions.trim().is_empty(),
        custom: v
            .custom
            .into_iter()
            .map(|c| CustomFieldView { value: (!c.protected).then_some(c.value), key: c.key, protected: c.protected })
            .collect(),
        attachments: v
            .attachments
            .iter()
            .map(|name| AttachmentInfo {
                size: db.vault.version_attachment_size(e.id, index, name).unwrap_or(0),
                name: name.clone(),
            })
            .collect(),
        document: v.document,
        autotype_sequence: v.autotype_sequence.unwrap_or_default(),
        autotype_windows: v.autotype_windows.into_iter().map(|w| w.window).collect(),
        differs,
    })
}

/// Makes history version `index` the current one (the current one goes into the history;
/// the entry stays in its folder). Undo takes it back.
#[tauri::command]
pub async fn restore_version(state: State<'_, Shared>, index: usize) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    let path = st.active().map(|d| d.vault.path().to_path_buf()).ok_or("No database is open.")?;
    let db = st.active_or_err()?;
    db.vault.restore_version(e.id, index, true).map_err(|err| format!("Could not restore: {err}"))?;
    let saved = db.vault.entry(e.id).ok_or("The restored entry is missing.")?;
    st.last_save = Some(LastSave { path, saved, created: false, replaced: None });
    st.save_active();
    Ok(st.main_view())
}

/// Deletes history version `index` of the selected entry, and the files only it used.
#[tauri::command]
pub async fn delete_version(state: State<'_, Shared>, index: usize) -> R<MainView> {
    let mut st = state.lock();
    let e = selected_entry(&st)?;
    mutate(&mut st, "Could not delete the version", |db| db.vault.delete_version(e.id, index))
}

/// The details pane has edits that are not saved; closing the window then asks first.
#[tauri::command]
pub async fn set_draft_pending(state: State<'_, Shared>, pending: bool) -> R<()> {
    state.lock().draft_pending = pending;
    Ok(())
}

/// Quits after the page has dealt with unsaved edits (the window's close was held back for
/// it). The exit handler saves every open database first.
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> R<()> {
    app.exit(0);
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into())
}

/// "scan.pdf", 2 → "scan (2).pdf"; "notes", 3 → "notes (3)".
fn numbered_name(name: &str, n: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

fn draft_files_view(st: &AppState) -> DraftFiles {
    let db = st.active();
    DraftFiles {
        files: st
            .draft_files
            .iter()
            .map(|p| AttachmentInfo { name: file_name(p), size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) })
            .collect(),
        attachments_bytes: db.map_or(0, |d| d.vault.attachments_total_bytes()),
        file_bytes: db.and_then(|d| std::fs::metadata(d.vault.path()).ok()).map_or(0, |m| m.len()),
    }
}

/// "Add file…" while editing: picks files (several at once) to attach when the draft is saved.
#[tauri::command]
pub async fn draft_pick_files(app: AppHandle, state: State<'_, Shared>) -> R<DraftFiles> {
    let picked = file_dialog(&app).blocking_pick_files().unwrap_or_default();
    let mut st = state.lock();
    for path in picked.into_iter().filter_map(|p| p.into_path().ok()) {
        if !st.draft_files.contains(&path) {
            st.draft_files.push(path);
        }
    }
    Ok(draft_files_view(&st))
}

#[tauri::command]
pub async fn draft_remove_file(state: State<'_, Shared>, index: usize) -> R<DraftFiles> {
    let mut st = state.lock();
    if index < st.draft_files.len() {
        st.draft_files.remove(index);
    }
    Ok(draft_files_view(&st))
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
        quick_unlock: s.quick_unlock,
        clipboard_clear_seconds: s.clipboard_clear_seconds,
        auto_lock_minutes: s.auto_lock_minutes,
        lock_on_system_lock: s.lock_on_system_lock,
        quick_unlock_name: st.quick_unlock.map(|k| match k {
            biometric::Kind::WindowsHello => "Windows Hello".to_string(),
            biometric::Kind::TouchId => "Touch ID".to_string(),
        }),
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
        if s.quick_unlock && !settings.quick_unlock {
            // Turning it off removes every stored password, not just the offer.
            for r in &s.recent {
                biometric::forget(&r.path);
            }
        }
        s.quick_unlock = settings.quick_unlock;
        s.clipboard_clear_seconds = settings.clipboard_clear_seconds.min(3600);
        s.auto_lock_minutes = settings.auto_lock_minutes.min(24 * 60);
        s.lock_on_system_lock = settings.lock_on_system_lock;
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
        let mut st = state.lock();
        st.note_activity();
        (st.active_tab, selected_entry(&st)?.id)
    };
    typing::perform_hidden(&app, "main", tab, id, false, true)
}

// ---------------------------------------------------------------------------
// Quick Access (the popup the global hotkey opens)
// ---------------------------------------------------------------------------

/// Searches every open database; an empty query returns nothing (the popup keeps its matches).
#[tauri::command]
pub async fn quick_search(state: State<'_, Shared>, text: String) -> R<Vec<QuickItem>> {
    state.lock().note_activity();
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

/// Hides Quick Access so the window behind it gets the focus back, then types the account
/// (`password_only`: just the password and Enter, for pages that ask only for the password).
#[tauri::command]
pub async fn quick_type(app: AppHandle, state: State<'_, Shared>, db: usize, id: String, password_only: bool) -> R<()> {
    let entry = {
        let mut st = state.lock();
        st.note_activity();
        quick_entry(&st, db, &id)?
    };
    typing::perform_hidden(&app, "quick", db, entry.id, password_only, false)
}

/// Copies one field of the account and closes Quick Access.
#[tauri::command]
pub async fn quick_copy(app: AppHandle, state: State<'_, Shared>, db: usize, id: String, field: String) -> R<()> {
    let (value, clear) = {
        let mut st = state.lock();
        st.note_activity();
        (field_value(&quick_entry(&st, db, &id)?, &field)?, st.settings.clipboard_clear_seconds)
    };
    clipboard::copy(&value, clear)?;
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
