#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

slint::include_modules!();

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use keepassj::autotype;
use keepassj::generator::{self, GenOptions};
use keepassj::settings::{self, Settings};
use keepassj::vault::{self, CustomField, EntryEdit, EntryId, EntryView, GroupId, GroupView, Vault};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct OpenDb {
    vault: Vault,
    selected_group: Option<GroupId>,
    selected_entry: Option<EntryId>,
    /// Groups as currently shown in the tree (pre-order, Recycle Bin subtree moved last).
    groups: Vec<GroupView>,
    /// Entries as currently listed (group contents or search result).
    entries: Vec<EntryView>,
}

enum FormMode {
    Add,
    Edit(EntryId),
}

struct FormCtx {
    mode: FormMode,
    /// Group ids in the same order as `App.form-groups`.
    group_ids: Vec<GroupId>,
    /// Existing entry for the same site, offered for replacement.
    duplicate: Option<EntryId>,
    custom: Vec<CustomField>,
}

struct AppState {
    settings: Settings,
    dbs: Vec<OpenDb>,
    active_tab: usize,
    search: String,
    /// Databases offered on the unlock page: settings.recent plus files picked this session.
    picker: Vec<PathBuf>,
    pending_new_db: Option<PathBuf>,
    form: Option<FormCtx>,
    /// (db index, entry) pairs behind the Auto-Type chooser list.
    chooser: Vec<(usize, EntryId)>,
    hotkey_manager: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    /// Tab whose last save failed; Retry saves this one.
    save_error_tab: Option<usize>,
}

impl AppState {
    fn active(&self) -> Option<&OpenDb> {
        self.dbs.get(self.active_tab)
    }
    fn active_mut(&mut self) -> Option<&mut OpenDb> {
        self.dbs.get_mut(self.active_tab)
    }
}

/// Every list model handed to Slint. Rebuilt with `set_vec` after each mutation.
struct Models {
    recent: Rc<VecModel<RecentItem>>,
    tabs: Rc<VecModel<TabItem>>,
    groups: Rc<VecModel<GroupItem>>,
    entries: Rc<VecModel<EntryItem>>,
    note_lines: Rc<VecModel<SharedString>>,
    attachments: Rc<VecModel<SharedString>>,
    form_groups: Rc<VecModel<SharedString>>,
    form_custom: Rc<VecModel<CustomFieldItem>>,
    chooser: Rc<VecModel<ChooserItem>>,
    toast_timer: slint::Timer,
}

impl Models {
    fn new(app: &App) -> Models {
        let m = Models {
            recent: Rc::new(VecModel::default()),
            tabs: Rc::new(VecModel::default()),
            groups: Rc::new(VecModel::default()),
            entries: Rc::new(VecModel::default()),
            note_lines: Rc::new(VecModel::default()),
            attachments: Rc::new(VecModel::default()),
            form_groups: Rc::new(VecModel::default()),
            form_custom: Rc::new(VecModel::default()),
            chooser: Rc::new(VecModel::default()),
            toast_timer: slint::Timer::default(),
        };
        app.set_recent(ModelRc::from(m.recent.clone()));
        app.set_tabs(ModelRc::from(m.tabs.clone()));
        app.set_groups(ModelRc::from(m.groups.clone()));
        app.set_entries(ModelRc::from(m.entries.clone()));
        app.set_detail_note_lines(ModelRc::from(m.note_lines.clone()));
        app.set_detail_attachments(ModelRc::from(m.attachments.clone()));
        app.set_form_groups(ModelRc::from(m.form_groups.clone()));
        app.set_form_custom(ModelRc::from(m.form_custom.clone()));
        app.set_chooser_items(ModelRc::from(m.chooser.clone()));
        m
    }
}

/// Everything a callback needs. Cloning is cheap (two Rc and a Weak).
#[derive(Clone)]
struct Ctx {
    ui: Weak<AppWindow>,
    state: Rc<RefCell<AppState>>,
    models: Rc<Models>,
}

impl Ctx {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }
}

fn ss(s: &str) -> SharedString {
    SharedString::from(s)
}

// ---------------------------------------------------------------------------
// Model refresh
// ---------------------------------------------------------------------------

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn refresh_picker(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let st = ctx.state.borrow();
    let items: Vec<RecentItem> = st
        .picker
        .iter()
        .map(|p| RecentItem {
            name: ss(&file_stem(p)),
            path: ss(&p.display().to_string()),
            missing: !p.is_file(),
            keyfile: ss(&st
                .settings
                .recent
                .iter()
                .find(|r| &r.path == p)
                .and_then(|r| r.keyfile.as_deref())
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()),
        })
        .collect();
    ctx.models.recent.set_vec(items);
    let app = ui.global::<App>();
    app.set_has_open_dbs(!st.dbs.is_empty());
    let sel = app.get_selected_recent();
    if st.picker.is_empty() {
        app.set_selected_recent(-1);
    } else if sel < 0 || sel as usize >= st.picker.len() {
        app.set_selected_recent(0);
    }
}

fn refresh_tabs(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let st = ctx.state.borrow();
    let items: Vec<TabItem> = st
        .dbs
        .iter()
        .map(|db| TabItem {
            name: ss(&db.vault.name()),
            path: ss(&db.vault.path().display().to_string()),
        })
        .collect();
    ctx.models.tabs.set_vec(items);
    ui.global::<App>().set_active_tab(st.active_tab as i32);
}

/// Pre-order groups with the Recycle Bin subtree moved to the end.
fn ordered_groups(vault: &Vault) -> Vec<GroupView> {
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

fn refresh_groups(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let mut st = ctx.state.borrow_mut();
    let Some(db) = st.active_mut() else {
        ctx.models.groups.set_vec(Vec::new());
        return;
    };
    db.groups = ordered_groups(&db.vault);
    if db.selected_group.map_or(true, |sel| !db.groups.iter().any(|g| g.id == sel)) {
        db.selected_group = Some(db.vault.root());
    }
    let items: Vec<GroupItem> = db
        .groups
        .iter()
        .map(|g| GroupItem {
            name: ss(&g.name),
            depth: g.depth as i32,
            is_recycle_bin: g.is_recycle_bin,
        })
        .collect();
    let idx = db
        .groups
        .iter()
        .position(|g| Some(g.id) == db.selected_group)
        .unwrap_or(0) as i32;
    ctx.models.groups.set_vec(items);
    ui.global::<App>().set_selected_group(idx);
}

fn refresh_entries(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let mut st = ctx.state.borrow_mut();
    let search = st.search.trim().to_string();
    let Some(db) = st.active_mut() else {
        ctx.models.entries.set_vec(Vec::new());
        return;
    };
    let mut list = if search.is_empty() {
        match db.selected_group {
            Some(g) => db.vault.entries_in(g),
            None => Vec::new(),
        }
    } else {
        db.vault.search(&search)
    };
    // The vault already orders stale entries last; keep that invariant even if a
    // future vault change forgets it.
    list.sort_by_key(|e| e.stale);
    let items: Vec<EntryItem> = list
        .iter()
        .map(|e| EntryItem {
            title: ss(&e.title),
            username: ss(&e.username),
            url: ss(&e.url),
            stale: e.stale,
        })
        .collect();
    let idx = list
        .iter()
        .position(|e| Some(e.id) == db.selected_entry)
        .map_or(-1, |i| i as i32);
    db.entries = list;
    ctx.models.entries.set_vec(items);
    ui.global::<App>().set_selected_entry(idx);
}

fn refresh_detail(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let st = ctx.state.borrow();
    let entry = st
        .active()
        .and_then(|db| db.selected_entry.and_then(|id| db.vault.entry(id)));
    let Some(e) = entry else {
        app.set_has_detail(false);
        ctx.models.note_lines.set_vec(Vec::new());
        ctx.models.attachments.set_vec(Vec::new());
        return;
    };
    let db = st.active().expect("entry implies active db");
    let in_bin = db
        .groups
        .iter()
        .find(|g| g.id == e.group)
        .map_or(false, |g| g.is_recycle_bin);
    app.set_detail_title(ss(&e.title));
    app.set_detail_username(ss(&e.username));
    app.set_detail_password(ss(&e.password));
    app.set_detail_url(ss(&e.url));
    app.set_detail_pin(ss(&e.recovery_pin));
    app.set_detail_phone(ss(&e.recovery_phone));
    app.set_detail_questions(ss(&e.security_questions));
    app.set_detail_tags(ss(&e.tags.join(", ")));
    app.set_detail_stale(e.stale);
    app.set_detail_in_bin(in_bin);
    app.set_detail_modified(ss(&e
        .modified
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()));
    let lines: Vec<SharedString> = if e.notes.trim().is_empty() {
        Vec::new()
    } else {
        e.notes.lines().map(|l| ss(l.trim_end_matches('\r'))).collect()
    };
    ctx.models.note_lines.set_vec(lines);
    ctx.models
        .attachments
        .set_vec(e.attachments.iter().map(|a| ss(a)).collect::<Vec<_>>());
    app.set_has_detail(true);
}

fn refresh_main(ctx: &Ctx) {
    refresh_tabs(ctx);
    refresh_groups(ctx);
    refresh_entries(ctx);
    refresh_detail(ctx);
}

fn toast(ctx: &Ctx, text: &str) {
    let Some(ui) = ctx.ui() else { return };
    ui.global::<App>().set_toast(ss(text));
    let weak = ctx.ui.clone();
    ctx.models.toast_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(2500),
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<App>().set_toast(ss(""));
            }
        },
    );
}

/// Saves tab `tab` and reports the outcome in the banner. Returns true on success.
fn save_tab(ctx: &Ctx, tab: usize) -> bool {
    let Some(ui) = ctx.ui() else { return false };
    let mut st = ctx.state.borrow_mut();
    let policy = settings::backup_policy();
    let result = match st.dbs.get_mut(tab) {
        Some(db) => db.vault.save(&policy),
        None => return true,
    };
    let app = ui.global::<App>();
    match result {
        Ok(()) => {
            st.save_error_tab = None;
            app.set_save_error(ss(""));
            true
        }
        Err(e) => {
            st.save_error_tab = Some(tab);
            app.set_save_error(ss(&e.to_string()));
            false
        }
    }
}

fn save_active(ctx: &Ctx) -> bool {
    let tab = ctx.state.borrow().active_tab;
    save_tab(ctx, tab)
}

fn save_settings_file(ctx: &Ctx) {
    let result = ctx.state.borrow().settings.save();
    if let Err(e) = result {
        toast(ctx, &format!("Could not save settings: {e}"));
    }
}

// ---------------------------------------------------------------------------
// OS helpers
// ---------------------------------------------------------------------------

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

fn open_url(ctx: &Ctx, url: &str) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    let target = if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    if let Err(e) = open_external(&target) {
        toast(ctx, &format!("Could not open browser: {e}"));
    }
}

fn kdbx_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("KeePass database", &["kdbx"])
}

fn confirm(title: &str, text: &str) -> bool {
    matches!(
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(title)
            .set_description(text)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show(),
        rfd::MessageDialogResult::Yes
    )
}

// ---------------------------------------------------------------------------
// Unlock / picker
// ---------------------------------------------------------------------------

fn add_open_db(ctx: &Ctx, vault: Vault, keyfile: Option<PathBuf>) {
    let path = vault.path().to_path_buf();
    {
        let mut st = ctx.state.borrow_mut();
        let root = vault.root();
        st.dbs.push(OpenDb {
            vault,
            selected_group: Some(root),
            selected_entry: None,
            groups: Vec::new(),
            entries: Vec::new(),
        });
        st.active_tab = st.dbs.len() - 1;
        st.search.clear();
        st.settings.touch_recent(&path, keyfile.as_deref());
        if !st.picker.iter().any(|p| p == &path) {
            st.picker.insert(0, path.clone());
        }
    }
    save_settings_file(ctx);
    if let Some(ui) = ctx.ui() {
        let app = ui.global::<App>();
        app.set_unlock_error(ss(""));
        app.set_creating(false);
        app.set_search_text(ss(""));
        app.set_page(Page::Main);
    }
    refresh_picker(ctx);
    refresh_main(ctx);
}

fn on_unlock(ctx: &Ctx, index: i32, password: &str) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let (path, keyfile) = {
        let st = ctx.state.borrow();
        let Some(path) = st.picker.get(index.max(0) as usize).cloned() else {
            app.set_unlock_error(ss("Pick a database first."));
            return;
        };
        let keyfile = st
            .settings
            .recent
            .iter()
            .find(|r| r.path == path)
            .and_then(|r| r.keyfile.clone());
        (path, keyfile)
    };
    if !path.is_file() {
        app.set_unlock_error(ss(&format!("File not found: {}", path.display())));
        return;
    }
    if ctx
        .state
        .borrow()
        .dbs
        .iter()
        .any(|db| db.vault.path() == path)
    {
        app.set_unlock_error(ss("That database is already open."));
        return;
    }
    match Vault::open(&path, password, keyfile.as_deref()) {
        Ok(vault) => add_open_db(ctx, vault, keyfile),
        Err(e) => app.set_unlock_error(ss(&format!("Could not unlock: {e}"))),
    }
}

fn on_browse_open(ctx: &Ctx) {
    let Some(path) = kdbx_dialog().pick_file() else { return };
    {
        let mut st = ctx.state.borrow_mut();
        st.picker.retain(|p| p != &path);
        st.picker.insert(0, path);
    }
    if let Some(ui) = ctx.ui() {
        let app = ui.global::<App>();
        app.set_selected_recent(0);
        app.set_unlock_error(ss(""));
    }
    refresh_picker(ctx);
}

fn on_browse_new(ctx: &Ctx) {
    let Some(mut path) = kdbx_dialog().set_file_name("Passwords.kdbx").save_file() else {
        return;
    };
    if path.extension().map_or(true, |e| !e.eq_ignore_ascii_case("kdbx")) {
        path.set_extension("kdbx");
    }
    if let Some(ui) = ctx.ui() {
        let app = ui.global::<App>();
        app.set_new_db_path(ss(&path.display().to_string()));
        app.set_unlock_error(ss(""));
        app.set_creating(true);
    }
    ctx.state.borrow_mut().pending_new_db = Some(path);
}

fn on_create_database(ctx: &Ctx, password: &str, confirm_pw: &str) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    if password.is_empty() {
        app.set_unlock_error(ss("The master password cannot be empty."));
        return;
    }
    if password != confirm_pw {
        app.set_unlock_error(ss("The two passwords differ."));
        return;
    }
    let Some(path) = ctx.state.borrow().pending_new_db.clone() else {
        app.set_creating(false);
        return;
    };
    match Vault::create(&path, password) {
        Ok(vault) => {
            ctx.state.borrow_mut().pending_new_db = None;
            add_open_db(ctx, vault, None);
        }
        Err(e) => app.set_unlock_error(ss(&format!("Could not create the database: {e}"))),
    }
}

fn on_forget_recent(ctx: &Ctx, index: i32) {
    {
        let mut st = ctx.state.borrow_mut();
        let Some(path) = st.picker.get(index.max(0) as usize).cloned() else { return };
        st.picker.retain(|p| p != &path);
        st.settings.recent.retain(|r| r.path != path);
    }
    save_settings_file(ctx);
    refresh_picker(ctx);
}

/// Stores `keyfile` for the picker entry at `index`, adding it to the recent list
/// when it was only picked this session (Open file…).
fn set_recent_keyfile(ctx: &Ctx, index: i32, keyfile: Option<PathBuf>) {
    {
        let mut st = ctx.state.borrow_mut();
        let Some(path) = st.picker.get(index.max(0) as usize).cloned() else { return };
        match st.settings.recent.iter_mut().find(|r| r.path == path) {
            Some(r) => r.keyfile = keyfile,
            None => st.settings.touch_recent(&path, keyfile.as_deref()),
        }
    }
    save_settings_file(ctx);
    refresh_picker(ctx);
}

fn on_choose_keyfile(ctx: &Ctx, index: i32) {
    let Some(keyfile) = rfd::FileDialog::new().set_title("Choose key file").pick_file() else {
        return;
    };
    set_recent_keyfile(ctx, index, Some(keyfile));
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

fn on_select_tab(ctx: &Ctx, index: i32) {
    {
        let mut st = ctx.state.borrow_mut();
        if index < 0 || index as usize >= st.dbs.len() {
            return;
        }
        st.active_tab = index as usize;
        st.search.clear();
    }
    if let Some(ui) = ctx.ui() {
        ui.global::<App>().set_search_text(ss(""));
    }
    refresh_main(ctx);
}

fn on_lock_tab(ctx: &Ctx, index: i32) {
    let remaining = {
        let mut st = ctx.state.borrow_mut();
        if index < 0 || index as usize >= st.dbs.len() {
            return;
        }
        let idx = index as usize;
        if st.dbs[idx].vault.is_dirty() && !confirm("Unsaved changes", "The last save of this database failed. Lock it anyway and lose those changes?") {
            return;
        }
        st.dbs.remove(idx);
        if st.active_tab >= st.dbs.len() {
            st.active_tab = st.dbs.len().saturating_sub(1);
        }
        st.search.clear();
        st.dbs.len()
    };
    if let Some(ui) = ctx.ui() {
        let app = ui.global::<App>();
        app.set_search_text(ss(""));
        app.set_save_error(ss(""));
        if remaining == 0 {
            app.set_page(Page::Unlock);
        }
    }
    refresh_picker(ctx);
    refresh_main(ctx);
}

fn on_select_group(ctx: &Ctx, index: i32) {
    {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        let Some(g) = db.groups.get(index.max(0) as usize) else { return };
        db.selected_group = Some(g.id);
    }
    refresh_groups(ctx);
    refresh_entries(ctx);
    refresh_detail(ctx);
}

fn on_add_group(ctx: &Ctx, parent_index: i32, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        let parent = db
            .groups
            .get(parent_index.max(0) as usize)
            .map(|g| g.id)
            .unwrap_or(db.vault.root());
        db.vault.add_group(parent, name).map(|id| db.selected_group = Some(id))
    };
    match outcome {
        Ok(()) => {
            save_active(ctx);
            refresh_groups(ctx);
            refresh_entries(ctx);
            refresh_detail(ctx);
        }
        Err(e) => toast(ctx, &format!("Could not add group: {e}")),
    }
}

fn on_select_entry(ctx: &Ctx, index: i32) {
    {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        db.selected_entry = db.entries.get(index.max(0) as usize).map(|e| e.id);
    }
    refresh_entries(ctx);
    refresh_detail(ctx);
}

fn on_search_changed(ctx: &Ctx, text: &str) {
    ctx.state.borrow_mut().search = text.to_string();
    refresh_entries(ctx);
}

fn on_toggle_stale(ctx: &Ctx) {
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        let Some(id) = db.selected_entry else { return };
        let Some(e) = db.vault.entry(id) else { return };
        db.vault.set_stale(id, !e.stale)
    };
    match outcome {
        Ok(()) => {
            save_active(ctx);
            refresh_entries(ctx);
            refresh_detail(ctx);
        }
        Err(e) => toast(ctx, &format!("Could not change the flag: {e}")),
    }
}

fn on_delete_entry(ctx: &Ctx) {
    let (id, permanent, title) = {
        let st = ctx.state.borrow();
        let Some(db) = st.active() else { return };
        let Some(id) = db.selected_entry else { return };
        let Some(e) = db.vault.entry(id) else { return };
        let in_bin = db
            .groups
            .iter()
            .find(|g| g.id == e.group)
            .map_or(false, |g| g.is_recycle_bin);
        (id, in_bin, e.title)
    };
    if permanent
        && !confirm(
            "Delete permanently",
            &format!("\"{title}\" is already in the Recycle Bin. Delete it for good?"),
        )
    {
        return;
    }
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        let r = db.vault.delete_entry(id);
        if r.is_ok() {
            db.selected_entry = None;
        }
        r
    };
    match outcome {
        Ok(()) => {
            save_active(ctx);
            refresh_groups(ctx);
            refresh_entries(ctx);
            refresh_detail(ctx);
            toast(ctx, if permanent { "Deleted" } else { "Moved to Recycle Bin" });
        }
        Err(e) => toast(ctx, &format!("Could not delete: {e}")),
    }
}

fn on_add_attachment(ctx: &Ctx) {
    let id = {
        let st = ctx.state.borrow();
        let Some(id) = st.active().and_then(|db| db.selected_entry) else { return };
        id
    };
    let Some(path) = rfd::FileDialog::new().pick_file() else { return };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            toast(ctx, &format!("Could not read the file: {e}"));
            return;
        }
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into());
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        db.vault.add_attachment(id, &name, bytes)
    };
    match outcome {
        Ok(()) => {
            save_active(ctx);
            refresh_entries(ctx);
            refresh_detail(ctx);
        }
        Err(e) => toast(ctx, &format!("Could not attach: {e}")),
    }
}

fn selected_attachment(ctx: &Ctx, index: i32) -> Option<(EntryId, String)> {
    let st = ctx.state.borrow();
    let db = st.active()?;
    let id = db.selected_entry?;
    let e = db.vault.entry(id)?;
    let name = e.attachments.get(index.max(0) as usize)?.clone();
    Some((id, name))
}

fn on_save_attachment(ctx: &Ctx, index: i32) {
    let Some((id, name)) = selected_attachment(ctx, index) else { return };
    let bytes = {
        let st = ctx.state.borrow();
        st.active().and_then(|db| db.vault.attachment(id, &name))
    };
    let Some(bytes) = bytes else {
        toast(ctx, "Attachment data is missing.");
        return;
    };
    let Some(target) = rfd::FileDialog::new().set_file_name(&name).save_file() else { return };
    match std::fs::write(&target, bytes) {
        Ok(()) => toast(ctx, &format!("Saved {}", target.display())),
        Err(e) => toast(ctx, &format!("Could not write the file: {e}")),
    }
}

fn on_remove_attachment(ctx: &Ctx, index: i32) {
    let Some((id, name)) = selected_attachment(ctx, index) else { return };
    if !confirm("Remove attachment", &format!("Remove \"{name}\" from this entry?")) {
        return;
    }
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(db) = st.active_mut() else { return };
        db.vault.remove_attachment(id, &name)
    };
    match outcome {
        Ok(()) => {
            save_active(ctx);
            refresh_detail(ctx);
        }
        Err(e) => toast(ctx, &format!("Could not remove: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Add / edit form
// ---------------------------------------------------------------------------

fn form_gen_options(app: &App, base: &GenOptions) -> GenOptions {
    GenOptions {
        length: app.get_gen_length().clamp(4, 128) as usize,
        digits: app.get_gen_digits(),
        symbols: app.get_gen_symbols(),
        uppercase: base.uppercase,
        exclude_ambiguous: base.exclude_ambiguous,
    }
}

fn group_label(g: &GroupView) -> SharedString {
    ss(&format!("{}{}", "    ".repeat(g.depth), g.name))
}

/// Fills the form properties and opens the Add or Edit page.
fn open_form(ctx: &Ctx, mode: FormMode) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let st = ctx.state.borrow();
    let Some(db) = st.active() else { return };
    let group_ids: Vec<GroupId> = db.groups.iter().map(|g| g.id).collect();
    let labels: Vec<SharedString> = db.groups.iter().map(group_label).collect();
    ctx.models.form_groups.set_vec(labels);

    let gen = &st.settings.generator;
    app.set_gen_length(gen.length as i32);
    app.set_gen_digits(gen.digits);
    app.set_gen_symbols(gen.symbols);
    app.set_duplicate_title(ss(""));

    let (edit, custom) = match &mode {
        FormMode::Add => {
            let group = db
                .selected_group
                .filter(|g| db.groups.iter().any(|v| v.id == *g && !v.is_recycle_bin))
                .unwrap_or(db.vault.root());
            app.set_form_title(ss(""));
            app.set_form_url(ss(""));
            app.set_form_username(ss(""));
            app.set_form_password(ss(&generator::generate(gen)));
            app.set_form_pin(ss(""));
            app.set_form_phone(ss(""));
            app.set_form_questions(ss(""));
            app.set_form_notes(ss(""));
            app.set_form_tags(ss(""));
            app.set_form_autotype_sequence(ss(""));
            app.set_form_windows(ss(""));
            (group, Vec::new())
        }
        FormMode::Edit(id) => {
            let Some(e) = db.vault.entry(*id) else { return };
            app.set_form_title(ss(&e.title));
            app.set_form_url(ss(&e.url));
            app.set_form_username(ss(&e.username));
            app.set_form_password(ss(&e.password));
            app.set_form_pin(ss(&e.recovery_pin));
            app.set_form_phone(ss(&e.recovery_phone));
            app.set_form_questions(ss(&e.security_questions));
            app.set_form_notes(ss(&e.notes));
            app.set_form_tags(ss(&e.tags.join(", ")));
            app.set_form_autotype_sequence(ss(e.autotype_sequence.as_deref().unwrap_or("")));
            app.set_form_windows(ss(&e.autotype_windows.join("\n")));
            (e.group, e.custom)
        }
    };
    let group_index = group_ids.iter().position(|g| *g == edit).unwrap_or(0);
    app.set_form_group_index(group_index as i32);
    ctx.models.form_custom.set_vec(
        custom
            .iter()
            .map(|c| CustomFieldItem {
                key: ss(&c.key),
                value: ss(&c.value),
                protected: c.protected,
            })
            .collect::<Vec<_>>(),
    );
    let page = match mode {
        FormMode::Add => Page::AddAccount,
        FormMode::Edit(_) => Page::EditEntry,
    };
    drop(st);
    ctx.state.borrow_mut().form = Some(FormCtx {
        mode,
        group_ids,
        duplicate: None,
        custom,
    });
    app.set_page(page);
}

fn on_regenerate(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let opts = form_gen_options(&app, &ctx.state.borrow().settings.generator);
    app.set_form_password(ss(&generator::generate(&opts)));
}

fn on_site_edited(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let url = app.get_form_url();
    let title = app.get_form_title();
    let host = vault::url_host(url.trim())
        .or_else(|| vault::url_host(title.trim()))
        .filter(|h| h.len() >= 3);
    let mut st = ctx.state.borrow_mut();
    let editing = match st.form.as_ref().map(|f| &f.mode) {
        Some(FormMode::Edit(id)) => Some(*id),
        _ => None,
    };
    let found = match (&host, st.active()) {
        (Some(h), Some(db)) => db
            .vault
            .find_by_host(h)
            .into_iter()
            .find(|e| Some(e.id) != editing && !e.stale),
        _ => None,
    };
    if let Some(form) = st.form.as_mut() {
        form.duplicate = found.as_ref().map(|e| e.id);
    }
    app.set_duplicate_title(ss(found.as_ref().map(|e| e.title.as_str()).unwrap_or("")));
}

fn split_tags(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

fn on_save_form(ctx: &Ctx, replace: bool) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let title = app.get_form_title().trim().to_string();
    if title.is_empty() {
        toast(ctx, "Give the account a name first.");
        return;
    }
    let outcome = {
        let mut st = ctx.state.borrow_mut();
        let Some(form) = st.form.take() else { return };
        let Some(db) = st.active_mut() else { return };
        let group = form
            .group_ids
            .get(app.get_form_group_index().max(0) as usize)
            .copied()
            .unwrap_or(db.vault.root());
        let (id, stale) = match form.mode {
            FormMode::Add => (None, false),
            FormMode::Edit(id) => (Some(id), db.vault.entry(id).map_or(false, |e| e.stale)),
        };
        let sequence = app.get_form_autotype_sequence().trim().to_string();
        let edit = EntryEdit {
            id,
            group,
            title,
            username: app.get_form_username().trim().to_string(),
            password: app.get_form_password().to_string(),
            url: app.get_form_url().trim().to_string(),
            notes: app.get_form_notes().to_string(),
            tags: split_tags(&app.get_form_tags()),
            stale,
            recovery_pin: app.get_form_pin().trim().to_string(),
            recovery_phone: app.get_form_phone().trim().to_string(),
            security_questions: app.get_form_questions().to_string(),
            custom: form
                .custom
                .into_iter()
                .filter(|c| !c.key.trim().is_empty())
                .collect(),
            autotype_sequence: if sequence.is_empty() { None } else { Some(sequence) },
            autotype_windows: app
                .get_form_windows()
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
        };
        let result = db.vault.upsert_entry(edit).and_then(|new_id| {
            if replace {
                if let Some(old) = form.duplicate.filter(|old| *old != new_id) {
                    db.vault.set_stale(old, true)?;
                }
            }
            Ok(new_id)
        });
        if let Ok(new_id) = result {
            db.selected_entry = Some(new_id);
            db.selected_group = Some(group);
        }
        result
    };
    match outcome {
        Ok(_) => {
            save_active(ctx);
            ctx.state.borrow_mut().search.clear();
            app.set_search_text(ss(""));
            app.set_page(Page::Main);
            refresh_main(ctx);
        }
        Err(e) => toast(ctx, &format!("Could not save the account: {e}")),
    }
}

fn on_cancel_form(ctx: &Ctx) {
    ctx.state.borrow_mut().form = None;
    if let Some(ui) = ctx.ui() {
        ui.global::<App>().set_page(Page::Main);
    }
}

fn on_custom_add(ctx: &Ctx) {
    let mut st = ctx.state.borrow_mut();
    let Some(form) = st.form.as_mut() else { return };
    form.custom.push(CustomField {
        key: String::new(),
        value: String::new(),
        protected: false,
    });
    ctx.models.form_custom.push(CustomFieldItem {
        key: ss(""),
        value: ss(""),
        protected: false,
    });
}

fn on_custom_remove(ctx: &Ctx, index: i32) {
    let mut st = ctx.state.borrow_mut();
    let Some(form) = st.form.as_mut() else { return };
    let i = index.max(0) as usize;
    if i < form.custom.len() {
        form.custom.remove(i);
        ctx.models.form_custom.remove(i);
    }
}

fn on_custom_edited(ctx: &Ctx, index: i32, key: &str, value: &str, protected: bool) {
    let mut st = ctx.state.borrow_mut();
    let Some(form) = st.form.as_mut() else { return };
    let i = index.max(0) as usize;
    let Some(field) = form.custom.get_mut(i) else { return };
    field.key = key.to_string();
    field.value = value.to_string();
    field.protected = protected;
    ctx.models.form_custom.set_row_data(
        i,
        CustomFieldItem {
            key: ss(key),
            value: ss(value),
            protected,
        },
    );
}

// ---------------------------------------------------------------------------
// Settings page
// ---------------------------------------------------------------------------

fn on_open_settings(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let st = ctx.state.borrow();
    let s = &st.settings;
    app.set_settings_hotkey(ss(&s.autotype_hotkey));
    app.set_settings_sequence(ss(&s.autotype_default_sequence));
    app.set_settings_reopen_last(s.reopen_last);
    app.set_settings_notes_monospace(s.notes_monospace);
    app.set_settings_gen_length(s.generator.length as i32);
    app.set_settings_gen_digits(s.generator.digits);
    app.set_settings_gen_symbols(s.generator.symbols);
    app.set_settings_gen_uppercase(s.generator.uppercase);
    app.set_settings_gen_exclude_ambiguous(s.generator.exclude_ambiguous);
    app.set_settings_backup_dir(ss(&settings::backup_policy().dir.display().to_string()));
    app.set_settings_error(ss(""));
    app.set_page(Page::Settings);
}

fn on_save_settings(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let hotkey_text = app.get_settings_hotkey().trim().to_string();
    if let Err(e) = autotype::parse_hotkey(&hotkey_text) {
        app.set_settings_error(ss(&format!("Hotkey not understood: {e}")));
        return;
    }
    let sequence = app.get_settings_sequence().trim().to_string();
    if let Err(e) = autotype::parse_sequence(&sequence) {
        app.set_settings_error(ss(&format!("Sequence not understood: {e}")));
        return;
    }
    let save_result = {
        let mut st = ctx.state.borrow_mut();
        let s = &mut st.settings;
        s.autotype_hotkey = hotkey_text;
        s.autotype_default_sequence = sequence;
        s.reopen_last = app.get_settings_reopen_last();
        s.notes_monospace = app.get_settings_notes_monospace();
        s.generator = GenOptions {
            length: app.get_settings_gen_length().clamp(4, 128) as usize,
            digits: app.get_settings_gen_digits(),
            symbols: app.get_settings_gen_symbols(),
            uppercase: app.get_settings_gen_uppercase(),
            exclude_ambiguous: app.get_settings_gen_exclude_ambiguous(),
        };
        s.save()
    };
    if let Err(e) = save_result {
        app.set_settings_error(ss(&format!("Could not write settings: {e}")));
        return;
    }
    app.set_notes_monospace(app.get_settings_notes_monospace());
    register_hotkey(ctx);
    let has_db = !ctx.state.borrow().dbs.is_empty();
    app.set_page(if has_db { Page::Main } else { Page::Unlock });
    refresh_main(ctx);
}

fn on_open_backup_folder(ctx: &Ctx) {
    let dir = settings::backup_policy().dir;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        toast(ctx, &format!("Could not create the folder: {e}"));
        return;
    }
    if let Err(e) = open_external(&dir.display().to_string()) {
        toast(ctx, &format!("Could not open the folder: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Auto-Type
// ---------------------------------------------------------------------------

fn register_hotkey(ctx: &Ctx) {
    let mut st = ctx.state.borrow_mut();
    if st.hotkey_manager.is_none() {
        match GlobalHotKeyManager::new() {
            Ok(m) => st.hotkey_manager = Some(m),
            Err(e) => {
                drop(st);
                toast(ctx, &format!("Global hotkey unavailable: {e}"));
                return;
            }
        }
    }
    let parsed = autotype::parse_hotkey(&st.settings.autotype_hotkey);
    let manager = st.hotkey_manager.as_ref().expect("manager created above");
    if let Some(old) = st.hotkey {
        let _ = manager.unregister(old);
    }
    let message = match parsed {
        Ok(hk) => match manager.register(hk) {
            Ok(()) => {
                st.hotkey = Some(hk);
                None
            }
            Err(e) => {
                st.hotkey = None;
                Some(format!("Could not register the hotkey: {e}"))
            }
        },
        Err(e) => {
            st.hotkey = None;
            Some(format!("Hotkey not understood: {e}"))
        }
    };
    drop(st);
    if let Some(m) = message {
        toast(ctx, &m);
    }
}

/// Field values an Auto-Type sequence can reference, keyed as `perform`'s resolver sees them.
fn field_map(e: &EntryView) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("USERNAME".into(), e.username.clone());
    m.insert("PASSWORD".into(), e.password.clone());
    m.insert("URL".into(), e.url.clone());
    m.insert("TITLE".into(), e.title.clone());
    m.insert("NOTES".into(), e.notes.clone());
    m.insert("S:Recovery PIN".into(), e.recovery_pin.clone());
    m.insert("S:Recovery Phone".into(), e.recovery_phone.clone());
    m.insert("S:Security Questions".into(), e.security_questions.clone());
    for c in &e.custom {
        m.insert(format!("S:{}", c.key), c.value.clone());
    }
    m
}

fn resolve_from(map: &HashMap<String, String>, name: &str) -> Option<String> {
    if let Some(v) = map.get(name) {
        return Some(v.clone());
    }
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

struct TypingJob {
    tokens: Vec<autotype::Token>,
    fields: HashMap<String, String>,
}

fn typing_job(ctx: &Ctx, db_index: usize, id: EntryId) -> Option<TypingJob> {
    let st = ctx.state.borrow();
    let db = st.dbs.get(db_index)?;
    let e = db.vault.entry(id)?;
    let seq = e
        .autotype_sequence
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&st.settings.autotype_default_sequence)
        .to_string();
    let tokens = match autotype::parse_sequence(&seq) {
        Ok(t) => t,
        Err(err) => {
            drop(st);
            toast(ctx, &format!("Auto-Type sequence not understood: {err}"));
            return None;
        }
    };
    Some(TypingJob {
        tokens,
        fields: field_map(&e),
    })
}

fn run_typing(job: &TypingJob) -> Result<(), autotype::AutotypeError> {
    let fields = &job.fields;
    autotype::perform(&job.tokens, &|name| resolve_from(fields, name))
}

/// Types into the window that currently has focus. Used when the hotkey found exactly one match.
fn perform_now(ctx: &Ctx, db_index: usize, id: EntryId) {
    let Some(job) = typing_job(ctx, db_index, id) else { return };
    if let Err(e) = run_typing(&job) {
        toast(ctx, &format!("Auto-Type failed: {e}"));
    }
}

/// Hides our window so the previous window regains focus, waits 250 ms, types, then shows
/// our window again (minimized when `minimize_after` so the target keeps the focus).
fn perform_hidden(ctx: &Ctx, db_index: usize, id: EntryId, minimize_after: bool) {
    let Some(job) = typing_job(ctx, db_index, id) else { return };
    let Some(ui) = ctx.ui() else { return };
    if let Err(e) = ui.window().hide() {
        toast(ctx, &format!("Could not hide the window: {e}"));
        return;
    }
    let weak = ctx.ui.clone();
    let ctx = ctx.clone();
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let result = run_typing(&job);
        if let Some(ui) = weak.upgrade() {
            let _ = ui.window().show();
            if minimize_after {
                ui.window().set_minimized(true);
            }
        }
        if let Err(e) = result {
            toast(&ctx, &format!("Auto-Type failed: {e}"));
        }
    });
}

fn bring_to_front(ui: &AppWindow) {
    if ui.window().is_visible() {
        ui.window().set_minimized(false);
        let _ = ui.window().hide();
    }
    let _ = ui.window().show();
}

fn on_hotkey(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let app = ui.global::<App>();
    let Some(title) = autotype::active_window_title() else {
        toast(ctx, "Auto-Type: no active window.");
        return;
    };
    let matches: Vec<(usize, EntryView)> = {
        let st = ctx.state.borrow();
        st.dbs
            .iter()
            .enumerate()
            .flat_map(|(i, db)| {
                db.vault
                    .all_entries()
                    .into_iter()
                    .filter(|e| {
                        e.autotype_windows
                            .iter()
                            .any(|p| autotype::window_matches(p, &title))
                            || autotype::title_matches(&e.title, &title)
                    })
                    .map(move |e| (i, e))
            })
            .collect()
    };
    match matches.len() {
        0 => toast(ctx, &format!("Auto-Type: no account matches \"{title}\".")),
        1 => {
            let (db, e) = &matches[0];
            perform_now(ctx, *db, e.id);
        }
        _ => {
            let names: Vec<String> = {
                let st = ctx.state.borrow();
                st.dbs.iter().map(|d| d.vault.name()).collect()
            };
            ctx.models.chooser.set_vec(
                matches
                    .iter()
                    .map(|(db, e)| ChooserItem {
                        title: ss(&e.title),
                        username: ss(&e.username),
                        db: ss(names.get(*db).map(String::as_str).unwrap_or("")),
                    })
                    .collect::<Vec<_>>(),
            );
            ctx.state.borrow_mut().chooser = matches.iter().map(|(db, e)| (*db, e.id)).collect();
            app.set_chooser_visible(true);
            bring_to_front(&ui);
        }
    }
}

fn on_choose_autotype(ctx: &Ctx, index: i32) {
    let Some(ui) = ctx.ui() else { return };
    ui.global::<App>().set_chooser_visible(false);
    let pick = ctx.state.borrow().chooser.get(index.max(0) as usize).copied();
    ctx.state.borrow_mut().chooser.clear();
    if let Some((db, id)) = pick {
        perform_hidden(ctx, db, id, true);
    }
}

/// Select-entry mode: type the selected entry into whichever window was active before ours.
fn on_autotype_entry(ctx: &Ctx) {
    let target = {
        let st = ctx.state.borrow();
        st.active().and_then(|db| db.selected_entry).map(|id| (st.active_tab, id))
    };
    if let Some((db, id)) = target {
        perform_hidden(ctx, db, id, false);
    }
}

fn poll_hotkey(ctx: &Ctx) {
    let wanted = ctx.state.borrow().hotkey.map(|h| h.id());
    let Some(wanted) = wanted else { return };
    let mut fire = false;
    while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
        if ev.id == wanted && ev.state == HotKeyState::Pressed {
            fire = true;
        }
    }
    if fire {
        on_hotkey(ctx);
    }
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn wire(ctx: &Ctx, ui: &AppWindow) {
    let app = ui.global::<App>();

    macro_rules! on {
        ($setter:ident, || $body:expr) => {{
            let c = ctx.clone();
            app.$setter(move || $body(&c));
        }};
        ($setter:ident, |$($arg:ident),*| $body:expr) => {{
            let c = ctx.clone();
            app.$setter(move |$($arg),*| $body(&c, $($arg),*));
        }};
    }

    // shell
    on!(on_retry_save, || |c: &Ctx| {
        let tab = c.state.borrow().save_error_tab.unwrap_or(c.state.borrow().active_tab);
        if save_tab(c, tab) {
            toast(c, "Saved.");
        }
    });
    {
        let c = ctx.clone();
        app.on_copy_text(move |text| {
            if let Some(ui) = c.ui() {
                ui.invoke_copy_to_clipboard(text);
                toast(&c, "Copied.");
            }
        });
    }
    {
        let c = ctx.clone();
        app.on_open_url(move |url| open_url(&c, &url));
    }

    // unlock
    {
        let c = ctx.clone();
        app.on_unlock(move |index, password| on_unlock(&c, index, &password));
    }
    on!(on_browse_open, || on_browse_open);
    on!(on_browse_new, || on_browse_new);
    {
        let c = ctx.clone();
        app.on_create_database(move |pw, confirm| on_create_database(&c, &pw, &confirm));
    }
    on!(on_cancel_create, || |c: &Ctx| {
        c.state.borrow_mut().pending_new_db = None;
        if let Some(ui) = c.ui() {
            let app = ui.global::<App>();
            app.set_creating(false);
            app.set_unlock_error(ss(""));
        }
    });
    on!(on_forget_recent, |index| on_forget_recent);
    on!(on_choose_keyfile, |index| on_choose_keyfile);
    on!(on_clear_keyfile, |index| |c: &Ctx, i: i32| set_recent_keyfile(c, i, None));
    on!(on_back_to_main, || |c: &Ctx| {
        if let Some(ui) = c.ui() {
            if !c.state.borrow().dbs.is_empty() {
                ui.global::<App>().set_page(Page::Main);
            }
        }
    });

    // main
    on!(on_select_tab, |index| on_select_tab);
    on!(on_lock_tab, |index| on_lock_tab);
    on!(on_select_group, |index| on_select_group);
    {
        let c = ctx.clone();
        app.on_add_group(move |parent, name| on_add_group(&c, parent, &name));
    }
    on!(on_select_entry, |index| on_select_entry);
    {
        let c = ctx.clone();
        app.on_search_changed(move |text| on_search_changed(&c, &text));
    }
    on!(on_add_account, || |c: &Ctx| open_form(c, FormMode::Add));
    on!(on_open_settings, || on_open_settings);
    on!(on_edit_entry, || |c: &Ctx| {
        let id = c.state.borrow().active().and_then(|db| db.selected_entry);
        if let Some(id) = id {
            open_form(c, FormMode::Edit(id));
        }
    });
    on!(on_toggle_stale, || on_toggle_stale);
    on!(on_delete_entry, || on_delete_entry);
    on!(on_autotype_entry, || on_autotype_entry);
    on!(on_add_attachment, || on_add_attachment);
    on!(on_save_attachment, |index| on_save_attachment);
    on!(on_remove_attachment, |index| on_remove_attachment);

    // form
    on!(on_regenerate, || on_regenerate);
    on!(on_site_edited, || on_site_edited);
    on!(on_save_form, |replace| on_save_form);
    on!(on_cancel_form, || on_cancel_form);
    on!(on_custom_add, || on_custom_add);
    on!(on_custom_remove, |index| on_custom_remove);
    {
        let c = ctx.clone();
        app.on_custom_edited(move |index, key, value, protected| {
            on_custom_edited(&c, index, &key, &value, protected)
        });
    }

    // settings
    on!(on_save_settings, || on_save_settings);
    on!(on_cancel_settings, || |c: &Ctx| {
        if let Some(ui) = c.ui() {
            let has_db = !c.state.borrow().dbs.is_empty();
            ui.global::<App>().set_page(if has_db { Page::Main } else { Page::Unlock });
        }
    });
    on!(on_open_backup_folder, || on_open_backup_folder);

    // chooser
    on!(on_choose_autotype, |index| on_choose_autotype);
    on!(on_cancel_chooser, || |c: &Ctx| {
        c.state.borrow_mut().chooser.clear();
        if let Some(ui) = c.ui() {
            ui.global::<App>().set_chooser_visible(false);
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let app = ui.global::<App>();

    let settings = Settings::load();
    let picker: Vec<PathBuf> = settings.recent.iter().map(|r| r.path.clone()).collect();
    app.set_notes_monospace(settings.notes_monospace);
    app.set_mono_font(ss(if cfg!(windows) {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "monospace"
    }));
    app.set_selected_recent(if settings.reopen_last && !picker.is_empty() { 0 } else { -1 });
    app.set_page(Page::Unlock);

    let state = Rc::new(RefCell::new(AppState {
        settings,
        dbs: Vec::new(),
        active_tab: 0,
        search: String::new(),
        picker,
        pending_new_db: None,
        form: None,
        chooser: Vec::new(),
        hotkey_manager: None,
        hotkey: None,
        save_error_tab: None,
    }));
    let models = Rc::new(Models::new(&app));
    let ctx = Ctx {
        ui: ui.as_weak(),
        state,
        models,
    };

    wire(&ctx, &ui);
    refresh_picker(&ctx);
    register_hotkey(&ctx);

    let hotkey_timer = slint::Timer::default();
    {
        let c = ctx.clone();
        hotkey_timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
            poll_hotkey(&c)
        });
    }

    // The window is hidden while Auto-Type runs, so the event loop must outlive it.
    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(hotkey_timer);
    Ok(())
}
