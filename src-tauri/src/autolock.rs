//! Auto-lock: every open database locks after `settings.auto_lock_minutes` without use of the
//! app, and (Windows, macOS) when the computer locks or goes to sleep.
//!
//! Use is recorded in `AppState::last_activity`: the page reports input in its two windows
//! (`note_activity`, at most every 30 s), and commands that are use in themselves (Auto-Type,
//! Quick Access, copying) record it directly. A thread checks the deadline every 15 s.
//!
//! Locking goes through the page, which first saves unsaved edits in the details pane: the
//! backend emits `auto-lock` to the main window and the page answers with `lock_all`. If it
//! has not answered within 10 s (a hung webview), the backend locks without it, losing those
//! edits, and emits `auto-locked`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Emitter, Manager};

use ourlogins::settings;

use crate::clipboard;
use crate::commands::close_db;
use crate::dto::QuickOpen;
use crate::saver;
use crate::state::Shared;
use crate::typing;

const TICK: Duration = Duration::from_secs(15);
/// How long the page gets to save its edits and lock before the backend locks without it.
const PAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// Starts the inactivity timer and the watch for the computer locking or sleeping. Called
/// once from `setup`, on the main thread.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let due = {
            let shared = handle.state::<Shared>();
            let st = shared.lock();
            !st.dbs.is_empty()
                && st.auto_lock_pending.is_none()
                && settings::auto_lock_due(st.last_activity, st.settings.auto_lock_minutes, SystemTime::now())
        };
        if due {
            request(&handle);
        }
    });
    platform::watch(app);
}

/// The computer is locking or going to sleep.
fn system_locked(app: &AppHandle) {
    if app.state::<Shared>().lock().settings.lock_on_system_lock {
        request(app);
    }
}

/// Starts an auto-lock, unless one is under way or no database is open.
fn request(app: &AppHandle) {
    let seq = {
        let shared = app.state::<Shared>();
        let mut st = shared.lock();
        if st.dbs.is_empty() || st.auto_lock_pending.is_some() {
            return;
        }
        st.auto_lock_seq += 1;
        st.auto_lock_pending = Some(st.auto_lock_seq);
        st.auto_lock_seq
    };
    // A copied password must not wait in the clipboard, and Quick Access forgets its list.
    clipboard::clear_if_ours();
    if let Some(quick) = app.get_webview_window("quick") {
        let _ = quick.hide();
    }
    let _ = app.emit_to("quick", "quick-open", QuickOpen { locked: true, window_title: String::new(), matches: Vec::new() });
    let _ = app.emit_to("main", "auto-lock", ());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(PAGE_TIMEOUT);
        let shared = app.state::<Shared>();
        if shared.lock().auto_lock_pending != Some(seq) {
            return;
        }
        for problem in lock_everything(&shared) {
            typing::toast(&app, &problem);
        }
        let _ = app.emit_to("main", "auto-locked", ());
    });
}

/// Saves and locks every open database, and returns what could not be saved. A database
/// whose save fails is locked anyway: an open database left unattended is what auto-lock
/// exists to prevent.
pub fn lock_everything(shared: &Shared) -> Vec<String> {
    let mut problems = Vec::new();
    let paths: Vec<PathBuf> = shared.lock().dbs.iter().map(|d| d.vault.path().to_path_buf()).collect();
    for path in paths {
        let mut st = saver::wait_idle(shared, &path);
        let Some(index) = st.dbs.iter().position(|d| d.vault.path() == path) else { continue };
        if let Err(e) = saver::save_now(&mut st, index) {
            problems.push(format!("{} was locked without its latest changes, which could not be saved: {e}", st.dbs[index].vault.name()));
        }
        close_db(&mut st, index);
    }
    {
        let mut st = shared.lock();
        st.auto_lock_pending = None;
        st.draft_pending = false;
    }
    clipboard::clear_if_ours();
    problems
}

/// Windows: a hidden top-level window on its own thread receives the session-lock
/// notification (`WTSRegisterSessionNotification`) and the suspend broadcast.
#[cfg(windows)]
mod platform {
    use std::sync::OnceLock;

    use tauri::AppHandle;
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, MSG, PBT_APMSUSPEND,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK,
    };

    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub fn watch(app: &AppHandle) {
        if APP.set(app.clone()).is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return };
            let class = w!("OurLoginsSessionWatch");
            let wc = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
            if RegisterClassW(&wc) == 0 {
                return;
            }
            // Top-level (not message-only): broadcasts such as WM_POWERBROADCAST reach only
            // top-level windows. It is never shown.
            let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(module.into()), None)
            else {
                return;
            };
            let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let event = wparam.0 as u32;
        if (msg == WM_WTSSESSION_CHANGE && event == WTS_SESSION_LOCK) || (msg == WM_POWERBROADCAST && event == PBT_APMSUSPEND) {
            if let Some(app) = APP.get() {
                super::system_locked(app);
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// macOS: NSWorkspace's will-sleep notification and the distributed "screen is locked" one.
#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
    use tauri::AppHandle;

    /// Must run on the main thread: the observers deliver on its run loop.
    pub fn watch(app: &AppHandle) {
        let app = app.clone();
        let on_lock = RcBlock::new(move |_: NonNull<NSNotification>| super::system_locked(&app));
        let screen_locked = NSString::from_str("com.apple.screenIsLocked");
        unsafe {
            let sleep = NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(Some(NSWorkspaceWillSleepNotification), None, None, &on_lock);
            let screen = NSDistributedNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(&screen_locked),
                None,
                None,
                &on_lock,
            );
            // The observers stay for the life of the app.
            std::mem::forget(sleep);
            std::mem::forget(screen);
        }
    }
}

/// Linux: no lock or sleep notification is watched; only the inactivity timer applies.
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn watch(_app: &tauri::AppHandle) {}
}
