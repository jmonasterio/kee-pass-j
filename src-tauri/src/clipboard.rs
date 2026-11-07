//! Copying text. Everything this app copies is treated as secret: on Windows it stays out of
//! clipboard history (Win+V) and cloud clipboard sync; on macOS it carries the
//! nspasteboard.org marker that clipboard-history apps honour.

pub fn copy(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("Could not copy: {e}"))?;
    let set = clipboard.set();
    #[cfg(windows)]
    let set = {
        use arboard::SetExtWindows;
        set.exclude_from_history().exclude_from_cloud()
    };
    #[cfg(target_os = "macos")]
    let set = {
        use arboard::SetExtApple;
        set.exclude_from_history()
    };
    set.text(text).map_err(|e| format!("Could not copy: {e}"))
}
