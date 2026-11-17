//! Keystrokes, the foreground window and the focused field, per platform. Windows calls the
//! Win32 API directly (`SendInput`, `GetForegroundWindow`, UI Automation). macOS types
//! through `enigo` and reads windows and fields through AppKit, Core Graphics and the
//! Accessibility API. Other systems type through `enigo` and report no window or field.

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

/// The text field that has the keyboard focus in the app being typed into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedField {
    /// A password box (Windows `IsPassword`, macOS `AXSecureTextField`).
    pub is_password: bool,
    /// Its text, when the app exposes it (never for password boxes).
    pub value: Option<String>,
}

pub use platform::{active_window_title, foreground_window, Keyboard};

/// How long to wait for the app to answer an accessibility query. Busy or unresponsive
/// apps can block these calls; typing then goes ahead as if nothing were known.
const FIELD_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);

/// The focused field of the foreground app, or `None` when the OS does not say (or does not
/// answer within [`FIELD_QUERY_TIMEOUT`]). Runs on its own thread so a hung app cannot stall
/// typing.
pub fn focused_field() -> Option<FocusedField> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(platform::focused_field());
    });
    rx.recv_timeout(FIELD_QUERY_TIMEOUT).ok().flatten()
}

#[cfg(windows)]
mod platform {
    use super::{Direction, FocusedField, Key, Modifier};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationValuePattern, UIA_ValuePatternId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
        VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR,
        VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW};

    /// Side-specific modifier keys, as `GetAsyncKeyState` reports them.
    const MODIFIERS: [(VIRTUAL_KEY, bool); 8] = [
        (VK_LSHIFT, false),
        (VK_RSHIFT, false),
        (VK_LCONTROL, false),
        (VK_RCONTROL, true),
        (VK_LMENU, false),
        (VK_RMENU, true),
        (VK_LWIN, true),
        (VK_RWIN, true),
    ];

    /// How long to wait for the user to let go of the hotkey before releasing it for them.
    const RELEASE_WAIT: std::time::Duration = std::time::Duration::from_millis(1000);

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

        /// `{CLEARFIELD}` as KeePass types it: Home, Shift+End, Backspace (Backspace rather
        /// than Delete, so a held Ctrl+Alt can never make Ctrl+Alt+Del).
        pub fn clear_field(&mut self) -> Result<(), String> {
            self.key(Key::Home, Direction::Click)?;
            self.modifier(Modifier::Shift, Direction::Press)?;
            let end = self.key(Key::End, Direction::Click);
            self.modifier(Modifier::Shift, Direction::Release)?;
            end?;
            self.key(Key::Backspace, Direction::Click)
        }

        /// Waits up to [`RELEASE_WAIT`] for the user to let go of Shift/Ctrl/Alt/Win (still
        /// down from the hotkey), then sends key-ups for any still down, as KeePass does.
        pub fn release_held_modifiers(&mut self) -> Result<(), String> {
            let held = || MODIFIERS.iter().copied().filter(|(vk, _)| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0).collect::<Vec<_>>();
            let start = std::time::Instant::now();
            let mut down = held();
            while !down.is_empty() && start.elapsed() < RELEASE_WAIT {
                std::thread::sleep(std::time::Duration::from_millis(20));
                down = held();
            }
            for (vk, extended) in down {
                self.vk(vk, extended, Direction::Release)?;
            }
            Ok(())
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

    /// The foreground window's handle.
    pub fn foreground_window() -> Option<isize> {
        let window = unsafe { GetForegroundWindow() };
        (!window.is_invalid()).then_some(window.0 as isize)
    }

    /// UI Automation's focused element: Chrome, Edge, Firefox and native apps report web and
    /// native text boxes, with `IsPassword` set on password boxes.
    pub fn focused_field() -> Option<FocusedField> {
        unsafe {
            // This thread is ours alone (see `super::focused_field`).
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let element = uia.GetFocusedElement().ok()?;
            let is_password = element.CurrentIsPassword().ok()?.as_bool();
            let value = if is_password {
                None
            } else {
                element
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .ok()
                    .and_then(|p| p.CurrentValue().ok())
                    .map(|s| s.to_string())
            };
            Some(FocusedField { is_password, value })
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Direction, FocusedField, Key, Modifier};
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

        /// `{CLEARFIELD}`: select everything in the field and delete it. macOS text fields
        /// use Cmd+A (Home/End scroll there); elsewhere Home, Shift+End, Backspace as KeePass.
        pub fn clear_field(&mut self) -> Result<(), String> {
            let (select_mod, select_keys): (enigo::Key, &[enigo::Key]) = if cfg!(target_os = "macos") {
                (enigo::Key::Meta, &[enigo::Key::Unicode('a')])
            } else {
                (enigo::Key::Shift, &[enigo::Key::End])
            };
            if !cfg!(target_os = "macos") {
                self.0.key(enigo::Key::Home, enigo::Direction::Click).map_err(|e| e.to_string())?;
            }
            self.0.key(select_mod, enigo::Direction::Press).map_err(|e| e.to_string())?;
            let selected = select_keys.iter().try_for_each(|k| self.0.key(*k, enigo::Direction::Click));
            self.0.key(select_mod, enigo::Direction::Release).map_err(|e| e.to_string())?;
            selected.map_err(|e| e.to_string())?;
            self.0.key(enigo::Key::Backspace, enigo::Direction::Click).map_err(|e| e.to_string())
        }

        /// Releases Shift/Control/Alt/Command, still down from the hotkey, as KeePass does.
        pub fn release_held_modifiers(&mut self) -> Result<(), String> {
            for key in [enigo::Key::Shift, enigo::Key::Control, enigo::Key::Alt, enigo::Key::Meta] {
                self.0.key(key, enigo::Direction::Release).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }

    /// The frontmost app's process id: typing stops when another app comes to the front.
    #[cfg(target_os = "macos")]
    pub fn foreground_window() -> Option<isize> {
        let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some(app.processIdentifier() as isize)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn foreground_window() -> Option<isize> {
        None
    }

    /// The Accessibility API's focused element. Needs the Accessibility permission, which
    /// typing keystrokes needs anyway.
    #[cfg(target_os = "macos")]
    pub fn focused_field() -> Option<FocusedField> {
        use objc2_application_services::{AXError, AXUIElement};
        use objc2_core_foundation::{CFRetained, CFString, CFType};
        use std::ptr::NonNull;

        fn attribute(element: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
            let mut value: *const CFType = std::ptr::null();
            let err = unsafe { element.copy_attribute_value(&CFString::from_str(name), NonNull::from(&mut value)) };
            if err != AXError::Success || value.is_null() {
                return None;
            }
            Some(unsafe { CFRetained::from_raw(NonNull::new(value as *mut CFType)?) })
        }

        let system = unsafe { AXUIElement::new_system_wide() };
        let focused = attribute(&system, "AXFocusedUIElement")?.downcast::<AXUIElement>().ok()?;
        let text = |name: &str| attribute(&focused, name).and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string());
        let is_password = text("AXSubrole").as_deref() == Some("AXSecureTextField");
        let value = if is_password { None } else { text("AXValue") };
        Some(FocusedField { is_password, value })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn focused_field() -> Option<FocusedField> {
        None
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
