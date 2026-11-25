//! Updates from the GitHub release feed (`plugins.updater` in tauri.conf.json). The feed and
//! every package are signed with the key whose public half is in tauri.conf.json; the plugin
//! refuses a package whose signature does not match.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::state::Shared;
use crate::{clipboard, commands, saver};

type R<T> = Result<T, String>;

/// The update found by the last `update_check`; `update_install` installs it.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
}

/// Asks the release feed for a newer version. `None` when this one is the newest.
#[tauri::command]
pub async fn update_check(app: AppHandle, pending: State<'_, Pending>) -> R<Option<UpdateInfo>> {
    let updater = app.updater().map_err(|e| format!("Could not check for updates: {e}"))?;
    let found = updater.check().await.map_err(|e| format!("Could not check for updates: {e}"))?;
    let info = found.as_ref().map(|u| UpdateInfo { version: u.version.clone(), current: u.current_version.clone() });
    *pending.0.lock().unwrap_or_else(|e| e.into_inner()) = found;
    Ok(info)
}

/// Downloads the update found by `update_check`, saves every open database, then installs and
/// restarts. On Windows the installer closes this process itself, so saving comes first.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, Shared>, pending: State<'_, Pending>) -> R<()> {
    if state.lock().draft_pending {
        return Err("Save or discard the edits in the details pane first.".into());
    }
    let update = pending.0.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("No update to install. Check for updates first.")?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| format!("Could not download the update: {e}"))?;
    // Same cleanup as the Exit handler in main.rs, which does not run for an update.
    saver::flush_all(&app.state::<Shared>());
    commands::clear_open_dir();
    clipboard::clear_if_ours();
    update.install(bytes).map_err(|e| format!("Could not install the update: {e}"))?;
    app.restart();
}
