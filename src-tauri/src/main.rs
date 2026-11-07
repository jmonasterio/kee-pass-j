#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod biometric;
mod commands;
mod dto;
mod state;
mod typing;

use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime, WebviewWindow};

/// The webview may only show the app's own embedded pages. Links are opened through the
/// `open_url` command, never by navigating the webview.
fn navigation_guard<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_webview, url| url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"))
        .build()
}

/// Mica on Windows 11, sidebar vibrancy on macOS. Returns whether a backdrop is active; the
/// frontend paints solid backgrounds when it is not (the webview background is transparent).
fn apply_backdrop(window: &WebviewWindow) -> bool {
    use tauri::window::{Effect, EffectsBuilder};
    #[cfg(windows)]
    {
        if windows_version::OsVersion::current().build < 22000 {
            return false;
        }
        window.set_effects(EffectsBuilder::new().effect(Effect::Mica).build()).is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        use tauri::window::EffectState;
        window
            .set_effects(
                EffectsBuilder::new()
                    .effect(Effect::Sidebar)
                    .state(EffectState::FollowsWindowActiveState)
                    .build(),
            )
            .is_ok()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = window;
        false
    }
}

/// Turns off WebView2 form autofill and password autosave, so nothing typed into the app is
/// offered for storage in the WebView2 profile. Release builds also drop browser shortcuts
/// (reload, print, find, zoom, devtools); editing keys like Ctrl+C/V stay.
#[cfg(windows)]
fn harden_webview2(window: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings4};
    use windows_core::Interface;
    let _ = window.with_webview(|wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else { return };
        let Ok(settings) = core.Settings() else { return };
        if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
            let _ = s4.SetIsGeneralAutofillEnabled(false);
            let _ = s4.SetIsPasswordAutosaveEnabled(false);
        }
        if !cfg!(debug_assertions) {
            if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }
    });
}

fn main() {
    // Release builds ignore WebView2 overrides from the environment: a remote-debugging port,
    // another browser binary, or another profile folder. Runs before any thread exists.
    #[cfg(all(windows, not(debug_assertions)))]
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_ascii_uppercase().starts_with("WEBVIEW2_") {
            std::env::remove_var(key);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(navigation_guard())
        .manage(state::Shared::new())
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
            let backdrop = apply_backdrop(&window);
            let quick = app.get_webview_window("quick").expect("quick window is declared in tauri.conf.json");
            #[cfg(windows)]
            {
                harden_webview2(&window);
                harden_webview2(&quick);
            }
            // Quick Access behaves like Spotlight: clicking anywhere else closes it.
            let q = quick.clone();
            quick.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = q.hide();
                }
            });
            // Temporary copies of opened attachments left by a crash.
            commands::clear_open_dir();
            let notice = typing::init(app.handle());
            app.manage(commands::Platform { backdrop, notice });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::platform_info,
            commands::unlock_view,
            commands::unlock,
            commands::unlock_quick,
            commands::browse_open,
            commands::browse_new,
            commands::create_database,
            commands::cancel_create,
            commands::forget_recent,
            commands::choose_keyfile,
            commands::clear_keyfile,
            commands::main_view,
            commands::select_tab,
            commands::lock_tab,
            commands::select_filter,
            commands::add_group,
            commands::set_group_color,
            commands::empty_recycle_bin,
            commands::select_entry,
            commands::search,
            commands::toggle_stale,
            commands::delete_entry,
            commands::delete_group,
            commands::pick_attachment,
            commands::attach_picked,
            commands::cancel_attachment,
            commands::open_attachment,
            commands::set_document,
            commands::about_info,
            commands::save_attachment,
            commands::remove_attachment,
            commands::reveal_field,
            commands::copy_field,
            commands::copy_text,
            commands::open_url,
            commands::retry_save,
            commands::form_open,
            commands::generate,
            commands::find_duplicate,
            commands::form_save,
            commands::settings_get,
            commands::settings_save,
            commands::open_backup_folder,
            commands::autotype_selected,
            commands::quick_search,
            commands::quick_type,
            commands::quick_copy,
            commands::quick_hide,
            commands::quick_open_main,
        ])
        .build(tauri::generate_context!())
        .expect("Our Logins failed to start")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                commands::clear_open_dir();
            }
        });
}
