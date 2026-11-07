//! Background saving. A command that changes a database returns at once; one thread per
//! database file encrypts and writes it. Encrypting and writing a 14 MB file takes seconds,
//! more on a network drive, and the window stays usable meanwhile.

use std::path::{Path, PathBuf};
use std::sync::MutexGuard;
use std::time::Duration;

use ourlogins::settings;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, Shared};

/// Starts saving tab `tab` in the background unless its file is already being saved; a
/// running save picks up later changes before it finishes.
pub fn request(st: &mut AppState, tab: usize) {
    let Some(app) = st.app.clone() else {
        // Before setup (never in practice): save on this thread.
        let _ = save_now(st, tab);
        return;
    };
    let Some(db) = st.dbs.get(tab) else { return };
    if !db.vault.is_dirty() {
        return;
    }
    let path = db.vault.path().to_path_buf();
    if st.saving.contains(&path) {
        return;
    }
    st.saving.push(path.clone());
    std::thread::spawn(move || run(app, path));
}

fn run(app: AppHandle, path: PathBuf) {
    let shared = app.state::<Shared>();
    let policy = settings::backup_policy();
    loop {
        let job = {
            let mut st = shared.lock();
            match st.dbs.iter_mut().find(|d| d.vault.path() == path) {
                Some(db) if db.vault.is_dirty() => db.vault.start_save(),
                _ => {
                    st.saving.retain(|p| p != &path);
                    break;
                }
            }
        };
        let original = job.backs_up_original();
        let result = job.run(&policy);
        let mut st = shared.lock();
        match result {
            Ok(()) => {
                if st.save_error.as_ref().is_some_and(|(p, _)| p == &path) {
                    st.save_error = None;
                }
            }
            Err(e) => {
                if let Some(db) = st.dbs.iter_mut().find(|d| d.vault.path() == path) {
                    db.vault.save_failed(original);
                }
                st.save_error = Some((path.clone(), e.to_string()));
                st.saving.retain(|p| p != &path);
                break;
            }
        }
    }
    let _ = app.emit_to("main", "saved", ());
}

/// Waits until no background save of `path` is running and returns the state locked, so
/// none can start until the caller lets go.
pub fn wait_idle<'a>(shared: &'a Shared, path: &Path) -> MutexGuard<'a, AppState> {
    loop {
        let st = shared.lock();
        if !st.saving.iter().any(|p| p == path) {
            return st;
        }
        drop(st);
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Saves tab `tab` on this thread when it has unsaved changes, recording or clearing the
/// error shown in the window. Callers first wait for a background save with `wait_idle`.
pub fn save_now(st: &mut AppState, tab: usize) -> Result<(), String> {
    let Some(db) = st.dbs.get_mut(tab) else { return Ok(()) };
    if !db.vault.is_dirty() {
        return Ok(());
    }
    let path = db.vault.path().to_path_buf();
    match db.vault.save(&settings::backup_policy()) {
        Ok(()) => {
            if st.save_error.as_ref().is_some_and(|(p, _)| p == &path) {
                st.save_error = None;
            }
            Ok(())
        }
        Err(e) => {
            let text = e.to_string();
            st.save_error = Some((path, text.clone()));
            Err(text)
        }
    }
}

/// At exit: finishes running saves and writes whatever is still unsaved.
pub fn flush_all(shared: &Shared) {
    let paths: Vec<PathBuf> = shared.lock().dbs.iter().map(|d| d.vault.path().to_path_buf()).collect();
    for path in paths {
        let mut st = wait_idle(shared, &path);
        if let Some(tab) = st.dbs.iter().position(|d| d.vault.path() == path) {
            let _ = save_now(&mut st, tab);
        }
    }
}
