//! Keystrokes and the foreground window's title, per platform. Windows calls the Win32 API
//! directly (`SendInput`, `GetForegroundWindow`). macOS types through `enigo` and reads the
//! title through AppKit and Core Graphics.

use crate::autotype::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Press,
    Release,
    Click,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Control,
    Alt,
}

pub use platform::{active_window_title, Keyboard};

#[cfg(windows)]
mod platform {
    use super::{Direction, Key, Modifier};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1,
        VK_HOME, VK_INSERT, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW};

    pub struct Keyboard;

    impl Keyboard {
        pub fn new() -> Result<Keyboard, String> {
            Ok(Keyboard)
        }

        /// Types text as Unicode characters, so the keyboard layout does not matter. Line
        /// breaks press Enter and tabs press Tab, as KeePass does.
        pub fn text(&mut self, text: &str) -> Result<(), String> {
            let mut units = [0u16; 2];
            for c in text.chars() {
                match c {
                    '\r' => {}
                    '\n' => self.vk(VK_RETURN, false, Direction::Click)?,
                    '\t' => self.vk(VK_TAB, false, Direction::Click)?,
                    _ => {
                        let encoded = c.encode_utf16(&mut units);
                        let mut inputs = Vec::with_capacity(encoded.len() * 2);
                        for &unit in encoded.iter() {
                            inputs.push(input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
                        }
                        for &unit in encoded.iter() {
                            inputs.push(input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                        }
                        send(&inputs)?;
                    }
                }
            }
            Ok(())
        }

        pub fn key(&mut self, key: Key, direction: Direction) -> Result<(), String> {
            let (vk, extended) = match key {
                Key::Tab => (VK_TAB, false),
                Key::Enter => (VK_RETURN, false),
                Key::Space => (VK_SPACE, false),
                Key::Backspace => (VK_BACK, false),
                Key::Delete => (VK_DELETE, true),
                Key::Escape => (VK_ESCAPE, false),
                Key::Home => (VK_HOME, true),
                Key::End => (VK_END, true),
                Key::Up => (VK_UP, true),
                Key::Down => (VK_DOWN, true),
                Key::Left => (VK_LEFT, true),
                Key::Right => (VK_RIGHT, true),
                Key::PageUp => (VK_PRIOR, true),
                Key::PageDown => (VK_NEXT, true),
                Key::Insert => (VK_INSERT, true),
                Key::F(n) => (VIRTUAL_KEY(VK_F1.0 + u16::from(n) - 1), false),
            };
            self.vk(vk, extended, direction)
        }

        pub fn modifier(&mut self, modifier: Modifier, direction: Direction) -> Result<(), String> {
            let vk = match modifier {
                Modifier::Shift => VK_SHIFT,
                Modifier::Control => VK_CONTROL,
                Modifier::Alt => VK_MENU,
            };
            self.vk(vk, false, direction)
        }

        /// The key that produces `c` on the current layout, for shortcuts such as Ctrl+A. A
        /// character with no key on the layout is typed as Unicode instead.
        pub fn char_key(&mut self, c: char, direction: Direction) -> Result<(), String> {
            let mut units = [0u16; 2];
            if let [unit] = c.encode_utf16(&mut units) {
                let scan = unsafe { VkKeyScanW(*unit) };
                if scan != -1 {
                    return self.vk(VIRTUAL_KEY((scan as u16) & 0xff), false, direction);
                }
            }
            let mut buf = [0u8; 4];
            self.text(c.encode_utf8(&mut buf))
        }

        fn vk(&mut self, vk: VIRTUAL_KEY, extended: bool, direction: Direction) -> Result<(), String> {
            let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
            let base = if extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
            let down = input(vk, scan, base);
            let up = input(vk, scan, base | KEYEVENTF_KEYUP);
            match direction {
                Direction::Press => send(&[down]),
                Direction::Release => send(&[up]),
                Direction::Click => send(&[down, up]),
            }
        }
    }

    fn input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn send(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            // Windows blocks input into windows of programs running as administrator.
            Err(format!(
                "Windows accepted {sent} of {} keystrokes: {}",
                inputs.len(),
                windows::core::Error::from_thread()
            ))
        }
    }

    pub fn active_window_title() -> Option<String> {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return None;
            }
            let len = GetWindowTextLengthW(window);
            let mut buf = vec![0u16; usize::try_from(len).unwrap_or(0) + 1];
            let copied = GetWindowTextW(window, &mut buf);
            Some(String::from_utf16_lossy(&buf[..usize::try_from(copied).unwrap_or(0)]))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Direction, Key, Modifier};
    use enigo::{Enigo, Keyboard as _, Settings};

    pub struct Keyboard(Enigo);

    fn direction(d: Direction) -> enigo::Direction {
        match d {
            Direction::Press => enigo::Direction::Press,
            Direction::Release => enigo::Direction::Release,
            Direction::Click => enigo::Direction::Click,
        }
    }

    impl Keyboard {
        pub fn new() -> Result<Keyboard, String> {
            Enigo::new(&Settings::default()).map(Keyboard).map_err(|e| e.to_string())
        }

        pub fn text(&mut self, text: &str) -> Result<(), String> {
            self.0.text(text).map_err(|e| e.to_string())
        }

        pub fn key(&mut self, key: Key, d: Direction) -> Result<(), String> {
            let key = match key {
                Key::Tab => enigo::Key::Tab,
                Key::Enter => enigo::Key::Return,
                Key::Space => enigo::Key::Space,
                Key::Backspace => enigo::Key::Backspace,
                Key::Delete => enigo::Key::Delete,
                Key::Escape => enigo::Key::Escape,
                Key::Home => enigo::Key::Home,
                Key::End => enigo::Key::End,
                Key::Up => enigo::Key::UpArrow,
                Key::Down => enigo::Key::DownArrow,
                Key::Left => enigo::Key::LeftArrow,
                Key::Right => enigo::Key::RightArrow,
                Key::PageUp => enigo::Key::PageUp,
                Key::PageDown => enigo::Key::PageDown,
                // Mac keyboards have no Insert key.
                Key::Insert => return Err("this keyboard has no Insert key".into()),
                Key::F(n) => match n {
                    1 => enigo::Key::F1,
                    2 => enigo::Key::F2,
                    3 => enigo::Key::F3,
                    4 => enigo::Key::F4,
                    5 => enigo::Key::F5,
                    6 => enigo::Key::F6,
                    7 => enigo::Key::F7,
                    8 => enigo::Key::F8,
                    9 => enigo::Key::F9,
                    10 => enigo::Key::F10,
                    11 => enigo::Key::F11,
                    _ => enigo::Key::F12,
                },
            };
            self.0.key(key, direction(d)).map_err(|e| e.to_string())
        }

        pub fn modifier(&mut self, modifier: Modifier, d: Direction) -> Result<(), String> {
            let key = match modifier {
                Modifier::Shift => enigo::Key::Shift,
                Modifier::Control => enigo::Key::Control,
                Modifier::Alt => enigo::Key::Alt,
            };
            self.0.key(key, direction(d)).map_err(|e| e.to_string())
        }

        pub fn char_key(&mut self, c: char, d: Direction) -> Result<(), String> {
            self.0.key(enigo::Key::Unicode(c), direction(d)).map_err(|e| e.to_string())
        }
    }

    /// Title of the frontmost app's frontmost normal window. macOS returns an empty title
    /// until the user grants Screen Recording permission.
    #[cfg(target_os = "macos")]
    pub fn active_window_title() -> Option<String> {
        use objc2::runtime::AnyObject;
        use objc2_app_kit::NSWorkspace;
        use objc2_core_foundation::CFArray;
        use objc2_core_graphics::{kCGNullWindowID, CGWindowListCopyWindowInfo, CGWindowListOption};
        use objc2_foundation::{ns_string, NSArray, NSDictionary, NSNumber, NSString};

        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = i64::from(app.processIdentifier());
        let list = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
            kCGNullWindowID,
        )?;
        // A CFArray of CFDictionary is toll-free bridged to NSArray of NSDictionary.
        let windows: &NSArray<NSDictionary<NSString, AnyObject>> =
            unsafe { &*(&*list as *const CFArray as *const NSArray<NSDictionary<NSString, AnyObject>>) };
        for window in windows.iter() {
            let number = |key: &NSString| window.objectForKey(key).and_then(|v| v.downcast::<NSNumber>().ok()).map(|n| n.as_i64());
            // Layer 0 holds normal windows; menus, the Dock and popups sit above it.
            if number(ns_string!("kCGWindowOwnerPID")) != Some(pid) || number(ns_string!("kCGWindowLayer")) != Some(0) {
                continue;
            }
            let title = window
                .objectForKey(ns_string!("kCGWindowName"))
                .and_then(|v| v.downcast::<NSString>().ok())
                .map(|s| s.to_string());
            return Some(title.unwrap_or_default());
        }
        None
    }

    #[cfg(not(target_os = "macos"))]
    pub fn active_window_title() -> Option<String> {
        None
    }
}
