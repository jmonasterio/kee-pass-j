//! Global hotkey and Auto-Type: window matching, the chooser, hiding our window while typing.
//!
//! `GlobalHotKeyManager` and the keystroke injection (`enigo`) run on the main thread:
//! the manager's hidden window lives on the thread that created it, and macOS keyboard
//! layout lookups must happen on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tauri::{AppHandle, Emitter, Manager};

use ourlogins::autotype;
use ourlogins::vault::{EntryId, EntryView};

use crate::dto::ChooserItem;
use crate::state::Shared;

thread_local! {
    /// Main-thread only: the manager and the currently registered hotkey.
    static HOTKEY: RefCell<(Option<GlobalHotKeyManager>, Option<HotKey>)> = const { RefCell::new((None, None)) };
}

/// Id of the registered hotkey; 0 when none is registered.
static HOTKEY_ID: AtomicU32 = AtomicU32::new(0);

pub fn toast(app: &AppHandle, text: &str) {
    let _ = app.emit("toast", text);
}

/// Installs the hotkey event handler and registers the hotkey from the settings.
/// Must be called on the main thread (from `setup`). The page is not listening yet, so a
/// registration failure is returned for `platform_info` to report.
pub fn init(app: &AppHandle) -> Option<String> {
    let handle = app.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        let wanted = HOTKEY_ID.load(Ordering::SeqCst);
        if wanted != 0 && ev.id == wanted && ev.state == HotKeyState::Pressed {
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || on_hotkey(&app));
        }
    }));
    register_hotkey(app).err()
}

/// (Re)registers `settings.autotype_hotkey`. Must run on the main thread.
pub fn register_hotkey(app: &AppHandle) -> Result<(), String> {
    let text = app.state::<Shared>().lock().settings.autotype_hotkey.clone();
    HOTKEY.with(|cell| {
        let mut slot = cell.borrow_mut();
        let (manager, current) = &mut *slot;
        if manager.is_none() {
            *manager = Some(GlobalHotKeyManager::new().map_err(|e| format!("Global hotkey unavailable: {e}"))?);
        }
        let manager = manager.as_ref().expect("created above");
        if let Some(old) = current.take() {
            let _ = manager.unregister(old);
            HOTKEY_ID.store(0, Ordering::SeqCst);
        }
        let hk = autotype::parse_hotkey(&text).map_err(|e| format!("Hotkey not understood: {e}"))?;
        manager.register(hk).map_err(|e| format!("Could not register the hotkey: {e}"))?;
        HOTKEY_ID.store(hk.id(), Ordering::SeqCst);
        *current = Some(hk);
        Ok(())
    })
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

fn typing_job(app: &AppHandle, db_index: usize, id: EntryId) -> Result<TypingJob, String> {
    let st = app.state::<Shared>();
    let st = st.lock();
    let db = st.dbs.get(db_index).ok_or("That database is no longer open.")?;
    let e = db.vault.entry(id).ok_or("That account no longer exists.")?;
    let seq = e
        .autotype_sequence
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&st.settings.autotype_default_sequence);
    let tokens = autotype::parse_sequence(seq).map_err(|err| format!("Auto-Type sequence not understood: {err}"))?;
    Ok(TypingJob {
        tokens,
        fields: field_map(&e),
    })
}

fn run_typing(job: &TypingJob) -> Result<(), String> {
    let fields = &job.fields;
    autotype::perform(&job.tokens, &|name| resolve_from(fields, name)).map_err(|e| format!("Auto-Type failed: {e}"))
}

/// Hides our window so the previous window regains focus, waits 250 ms, types on the main
/// thread, then shows our window again (minimized when `minimize_after`, so the target
/// window keeps the focus).
pub fn perform_hidden(app: &AppHandle, db_index: usize, id: EntryId, minimize_after: bool) -> Result<(), String> {
    let job = typing_job(app, db_index, id)?;
    let window = app.get_webview_window("main").ok_or("Main window missing.")?;
    window.hide().map_err(|e| format!("Could not hide the window: {e}"))?;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || {
            let result = run_typing(&job);
            if minimize_after {
                let _ = window.minimize();
            }
            let _ = window.show();
            if let Err(e) = result {
                toast(&app, &e);
            }
        });
    });
    Ok(())
}

fn bring_to_front(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Hotkey pressed: type the single matching entry right away, or offer a chooser.
fn on_hotkey(app: &AppHandle) {
    let Some(title) = autotype::active_window_title() else {
        toast(app, "Auto-Type: no active window.");
        return;
    };
    let shared = app.state::<Shared>();
    let (matches, names): (Vec<(usize, EntryView)>, Vec<String>) = {
        let st = shared.lock();
        let matches = st
            .dbs
            .iter()
            .enumerate()
            .flat_map(|(i, db)| {
                db.vault
                    .all_entries()
                    .into_iter()
                    .filter(|e| {
                        e.autotype_windows.iter().any(|p| autotype::window_matches(p, &title))
                            || autotype::title_matches(&e.title, &title)
                    })
                    .map(move |e| (i, e))
            })
            .collect();
        (matches, st.dbs.iter().map(|d| d.vault.name()).collect())
    };
    match matches.len() {
        0 => toast(app, &format!("Auto-Type: no account matches \"{title}\".")),
        1 => {
            let (db, e) = &matches[0];
            if let Err(err) = typing_job(app, *db, e.id).and_then(|job| run_typing(&job)) {
                toast(app, &err);
            }
        }
        _ => {
            let items: Vec<ChooserItem> = matches
                .iter()
                .map(|(db, e)| ChooserItem {
                    title: e.title.clone(),
                    username: e.username.clone(),
                    db: names.get(*db).cloned().unwrap_or_default(),
                })
                .collect();
            shared.lock().chooser = matches.iter().map(|(db, e)| (*db, e.id)).collect();
            bring_to_front(app);
            let _ = app.emit("autotype-choose", items);
        }
    }
}
