//! Copying text. Everything this app copies is treated as secret: on Windows it stays out of
//! clipboard history (Win+V) and cloud clipboard sync; on macOS it carries the
//! nspasteboard.org marker that clipboard-history apps honour.
//!
//! After `clear_after` seconds the clipboard is emptied, but only if it still holds what this
//! app put there: anything the user copied since is left alone. The same check runs at exit.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Bumped on every copy, so only the timer of the latest copy clears.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Hash of the text this app copied last. The text itself is not kept.
static LAST_COPY: Mutex<Option<u64>> = Mutex::new(None);

fn digest(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Copies `text`. With `clear_after` above 0, empties the clipboard that many seconds later
/// if it still holds `text`.
pub fn copy(text: &str, clear_after: u32) -> Result<(), String> {
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
    set.text(text).map_err(|e| format!("Could not copy: {e}"))?;

    *LAST_COPY.lock().unwrap_or_else(|p| p.into_inner()) = Some(digest(text));
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if clear_after > 0 {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(u64::from(clear_after)));
            if GENERATION.load(Ordering::SeqCst) == generation {
                clear_if_ours();
            }
        });
    }
    Ok(())
}

/// Empties the clipboard if it still holds the text this app copied last.
pub fn clear_if_ours() {
    let mut last = LAST_COPY.lock().unwrap_or_else(|p| p.into_inner());
    let Some(expected) = *last else { return };
    let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
    // Anything other than our text (an image, a file, other text) belongs to the user.
    if clipboard.get_text().is_ok_and(|current| digest(&current) == expected) {
        let _ = clipboard.clear();
    }
    *last = None;
}
