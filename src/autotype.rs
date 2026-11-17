//! Auto-Type: KeePass 2.x sequence grammar, keystroke playback, window matching, hotkey parsing.

use std::thread::sleep;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use thiserror::Error;

pub use crate::input::FocusedField;
use crate::input::{Direction, Keyboard, Modifier};

/// A named key in an Auto-Type sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Backspace,
    Delete,
    Escape,
    Home,
    End,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Insert,
    /// F1 to F12.
    F(u8),
}

/// Gap between two consecutive tokens while typing.
const TOKEN_GAP: Duration = Duration::from_millis(10);

/// Pause after `{CLEARFIELD}`, as KeePass, so the field has processed the deletion.
const CLEAR_PAUSE: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub enum AutotypeError {
    #[error("unknown placeholder {{{0}}}")]
    UnknownPlaceholder(String),
    #[error("unterminated placeholder starting at position {0}")]
    UnterminatedPlaceholder(usize),
    #[error("unterminated modifier group starting at position {0}")]
    UnterminatedGroup(usize),
    #[error("modifier prefix at position {0} has nothing to apply to")]
    DanglingModifier(usize),
    #[error("invalid count in {{{0}}}")]
    InvalidCount(String),
    #[error("invalid hotkey '{0}': {1}")]
    Hotkey(String, String),
    #[error("keyboard initialisation failed: {0}")]
    Init(String),
    #[error("keystroke failed: {0}")]
    Input(String),
    #[error("stopped because a different window came to the front")]
    WindowChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    fn is_empty(self) -> bool {
        !(self.shift || self.ctrl || self.alt)
    }

    fn keys(self) -> Vec<Modifier> {
        let mut keys = Vec::with_capacity(3);
        if self.shift {
            keys.push(Modifier::Shift);
        }
        if self.ctrl {
            keys.push(Modifier::Control);
        }
        if self.alt {
            keys.push(Modifier::Alt);
        }
        keys
    }
}

/// One unit of an Auto-Type sequence.
///
/// `Field` carries an upper-case standard name (`USERNAME`, `PASSWORD`, `URL`, `TITLE`,
/// `NOTES`) or `S:Name` verbatim for custom fields. `Delay` is milliseconds. `ClearField`
/// empties the focused text field.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Field(String),
    Key(Key),
    Char(char),
    Delay(u64),
    ClearField,
    Modified(Modifiers, Box<Token>),
}

/// Parses a KeePass 2.x Auto-Type sequence.
///
/// Placeholders: `{USERNAME}` `{PASSWORD}` `{URL}` `{TITLE}` `{NOTES}` `{S:Field Name}`, `{CLEARFIELD}`,
/// `{TAB}` `{ENTER}` `{SPACE}` `{BACKSPACE}` `{DELETE}` `{ESC}` `{HOME}` `{END}` `{UP}` `{DOWN}`
/// `{LEFT}` `{RIGHT}` `{F1}`..`{F12}`, `{TAB 3}` repeat counts, `{DELAY 500}`,
/// `{{}` `{}}` `{+}` `{^}` `{%}` `{(}` `{)}` escapes.
/// `+` `^` `%` prefixes apply shift/ctrl/alt to the next key or to a `(group)`.
/// Plain characters type literally.
pub fn parse_sequence(seq: &str) -> Result<Vec<Token>, AutotypeError> {
    let chars: Vec<char> = seq.chars().collect();
    let mut pos = 0;
    let tokens = parse_items(&chars, &mut pos, false)?;
    Ok(tokens)
}

/// Parses items until end of input, or until an unescaped `)` when `in_group` is set.
/// On return `pos` points past the closing `)` for a group.
fn parse_items(chars: &[char], pos: &mut usize, in_group: bool) -> Result<Vec<Token>, AutotypeError> {
    let mut out = Vec::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        match c {
            ')' if in_group => {
                *pos += 1;
                return Ok(out);
            }
            '+' | '^' | '%' => {
                let start = *pos;
                let mut mods = Modifiers::default();
                while *pos < chars.len() {
                    match chars[*pos] {
                        '+' => mods.shift = true,
                        '^' => mods.ctrl = true,
                        '%' => mods.alt = true,
                        _ => break,
                    }
                    *pos += 1;
                }
                if *pos >= chars.len() {
                    return Err(AutotypeError::DanglingModifier(start));
                }
                if chars[*pos] == '(' {
                    *pos += 1;
                    for t in parse_items(chars, pos, true)? {
                        out.push(Token::Modified(mods, Box::new(t)));
                    }
                } else {
                    // A repeated placeholder after a modifier: the modifier applies to the
                    // first repetition only, matching KeePass.
                    let mut single = parse_single(chars, pos)?.into_iter();
                    match single.next() {
                        Some(t) => {
                            out.push(Token::Modified(mods, Box::new(t)));
                            out.extend(single);
                        }
                        None => return Err(AutotypeError::DanglingModifier(start)),
                    }
                }
            }
            _ => out.extend(parse_single(chars, pos)?),
        }
    }
    if in_group {
        return Err(AutotypeError::UnterminatedGroup(*pos));
    }
    Ok(out)
}

/// Parses one plain character or one `{...}` placeholder. Repeat counts expand here, so
/// the result has one element except for `{KEY n}` with n > 1.
fn parse_single(chars: &[char], pos: &mut usize) -> Result<Vec<Token>, AutotypeError> {
    let c = chars[*pos];
    if c != '{' {
        *pos += 1;
        return Ok(vec![Token::Char(c)]);
    }
    let start = *pos;
    // `{{}` and `{}}` escapes: the body is a single brace followed by `}`.
    if let (Some(&body), Some('}')) = (chars.get(start + 1), chars.get(start + 2).copied()) {
        if body == '{' || body == '}' {
            *pos = start + 3;
            return Ok(vec![Token::Char(body)]);
        }
    }
    let close = chars[start + 1..]
        .iter()
        .position(|&c| c == '}')
        .map(|i| start + 1 + i)
        .ok_or(AutotypeError::UnterminatedPlaceholder(start))?;
    let body: String = chars[start + 1..close].iter().collect();
    *pos = close + 1;
    placeholder(&body)
}

/// Expands the text between braces into tokens, in typing order.
fn placeholder(body: &str) -> Result<Vec<Token>, AutotypeError> {
    if body.is_empty() {
        return Err(AutotypeError::UnknownPlaceholder(String::new()));
    }
    // Single-character escapes.
    let mut it = body.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        if matches!(c, '+' | '^' | '%' | '(' | ')' | '{' | '}' | '~') {
            return Ok(vec![Token::Char(c)]);
        }
    }
    if let Some(name) = body.strip_prefix("S:").or_else(|| body.strip_prefix("s:")) {
        return Ok(vec![Token::Field(format!("S:{name}"))]);
    }
    let (name, arg) = match body.split_once(' ') {
        Some((n, a)) => (n.trim(), Some(a.trim())),
        None => (body.trim(), None),
    };
    let upper = name.to_ascii_uppercase();
    if upper == "DELAY" {
        let ms = arg
            .and_then(|a| a.parse::<u64>().ok())
            .ok_or_else(|| AutotypeError::InvalidCount(body.to_string()))?;
        return Ok(vec![Token::Delay(ms)]);
    }
    if upper == "CLEARFIELD" && arg.is_none() {
        return Ok(vec![Token::ClearField]);
    }
    let token = match upper.as_str() {
        "USERNAME" | "PASSWORD" | "URL" | "TITLE" | "NOTES" => Token::Field(upper.clone()),
        "TAB" => Token::Key(Key::Tab),
        "ENTER" => Token::Key(Key::Enter),
        "SPACE" => Token::Key(Key::Space),
        "BACKSPACE" | "BS" | "BKSP" => Token::Key(Key::Backspace),
        "DELETE" | "DEL" => Token::Key(Key::Delete),
        "ESC" | "ESCAPE" => Token::Key(Key::Escape),
        "HOME" => Token::Key(Key::Home),
        "END" => Token::Key(Key::End),
        "UP" => Token::Key(Key::Up),
        "DOWN" => Token::Key(Key::Down),
        "LEFT" => Token::Key(Key::Left),
        "RIGHT" => Token::Key(Key::Right),
        "PGUP" | "PAGEUP" => Token::Key(Key::PageUp),
        "PGDN" | "PAGEDOWN" => Token::Key(Key::PageDown),
        // Mac keyboards have no Insert key; typing it there reports an error.
        "INSERT" | "INS" => Token::Key(Key::Insert),
        f if f.len() >= 2 && f.starts_with('F') && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => {
            Token::Key(Key::F(f[1..].parse().expect("checked above")))
        }
        _ => return Err(AutotypeError::UnknownPlaceholder(body.to_string())),
    };
    let count = match arg {
        Some(a) => a
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1)
            .ok_or_else(|| AutotypeError::InvalidCount(body.to_string()))?,
        None => 1,
    };
    Ok(vec![token; count])
}

/// Types `tokens` into the focused window. `resolve` maps field names to values; a field
/// that resolves to `None` types nothing, matching KeePass.
///
/// Like KeePass, modifier keys still held from the hotkey are released first (otherwise the
/// typed letters arrive as Ctrl/Alt shortcuts), and typing stops as soon as another window
/// comes to the front, so the rest of the sequence never lands in the wrong place.
pub fn perform(tokens: &[Token], resolve: &dyn Fn(&str) -> Option<String>) -> Result<(), AutotypeError> {
    let mut keyboard = Keyboard::new().map_err(AutotypeError::Init)?;
    keyboard.release_held_modifiers().map_err(AutotypeError::Input)?;
    let target = crate::input::foreground_window();
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 {
            sleep(TOKEN_GAP);
        }
        if target.is_some() && crate::input::foreground_window() != target {
            return Err(AutotypeError::WindowChanged);
        }
        type_token(&mut keyboard, token, resolve).map_err(AutotypeError::Input)?;
    }
    Ok(())
}

/// What the focused field is, as far as the OS accessibility API tells (Windows UI
/// Automation, macOS Accessibility). `None` where unavailable.
pub fn focused_field() -> Option<FocusedField> {
    crate::input::focused_field()
}

/// Adapts a sequence used as the default (no sequence of the entry's own) to the field that
/// has the focus:
/// - every `{USERNAME}` and `{PASSWORD}` is preceded by `{CLEARFIELD}`, so text a site or
///   browser already put there is replaced, not appended to;
/// - a sequence starting `{USERNAME}{TAB}` skips that part when the focus is in a password
///   field (pages that ask only for the password), and skips only the username when the
///   focused field already holds this username.
pub fn adapt_default(tokens: &[Token], focused: Option<&FocusedField>, username: &str) -> Vec<Token> {
    let user = Token::Field("USERNAME".into());
    let mut rest = tokens;
    let starts_with_user = rest.first() == Some(&user);
    if let (true, Some(f)) = (starts_with_user, focused) {
        let holds_username = !username.trim().is_empty()
            && f.value.as_deref().is_some_and(|v| v.trim().eq_ignore_ascii_case(username.trim()));
        if f.is_password && rest.get(1) == Some(&Token::Key(Key::Tab)) {
            rest = &rest[2..];
        } else if holds_username {
            rest = &rest[1..];
        }
    }
    let mut out = Vec::with_capacity(rest.len() + 2);
    for t in rest {
        if matches!(t, Token::Field(name) if name == "USERNAME" || name == "PASSWORD") {
            out.push(Token::ClearField);
        }
        out.push(t.clone());
    }
    out
}

/// What "Type password" types: the password into the focused field, then Enter.
pub fn password_only() -> Vec<Token> {
    vec![Token::ClearField, Token::Field("PASSWORD".into()), Token::Key(Key::Enter)]
}

fn type_token(kb: &mut Keyboard, token: &Token, resolve: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    match token {
        Token::Field(name) => {
            if let Some(value) = resolve(name) {
                if !value.is_empty() {
                    kb.text(&value)?;
                }
            }
        }
        Token::Key(key) => kb.key(*key, Direction::Click)?,
        Token::ClearField => {
            kb.clear_field()?;
            sleep(CLEAR_PAUSE);
        }
        Token::Char(c) => {
            let mut buf = [0u8; 4];
            kb.text(c.encode_utf8(&mut buf))?;
        }
        Token::Delay(ms) => sleep(Duration::from_millis(*ms)),
        Token::Modified(mods, inner) => {
            if mods.is_empty() {
                return type_token(kb, inner, resolve);
            }
            let held = mods.keys();
            for m in &held {
                kb.modifier(*m, Direction::Press)?;
            }
            let result = match inner.as_ref() {
                Token::Key(key) => kb.key(*key, Direction::Click),
                Token::Char(c) => kb.char_key(*c, Direction::Click),
                other => type_token(kb, other, resolve),
            };
            for m in held.iter().rev() {
                kb.modifier(*m, Direction::Release)?;
            }
            result?;
        }
    }
    Ok(())
}

/// Title of the foreground window, or `None` when the OS reports no active window.
pub fn active_window_title() -> Option<String> {
    crate::input::active_window_title()
}

/// KeePass window association. A pattern written `//text//` is a regex in KeePass; this
/// crate has no regex engine, so the inner text is matched as a case-insensitive substring
/// of the title instead. Every other pattern is a case-insensitive glob over the whole
/// title with `*` (any run) and `?` (one character).
pub fn window_matches(pattern: &str, title: &str) -> bool {
    let title = title.to_lowercase();
    if pattern.len() >= 4 && pattern.starts_with("//") && pattern.ends_with("//") {
        let inner = pattern[2..pattern.len() - 2].to_lowercase();
        return title.contains(&inner);
    }
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = title.chars().collect();
    glob_match(&pat, &text)
}

/// Iterative wildcard matching with backtracking to the most recent `*`.
fn glob_match(pat: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// Default association: a non-empty entry title of at least 3 characters appears in the
/// window title, case-insensitive.
pub fn title_matches(entry_title: &str, window_title: &str) -> bool {
    let entry = entry_title.trim();
    if entry.chars().count() < 3 {
        return false;
    }
    window_title.to_lowercase().contains(&entry.to_lowercase())
}

/// KeePass's "URL host in window title": the host of the entry's URL, without a leading
/// `www.`, appears in the window title, case-insensitive. Browsers that show the address in
/// the title (and apps named after their site) then match even when the entry title differs.
pub fn url_host_matches(url: &str, window_title: &str) -> bool {
    let rest = url.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next().unwrap_or("").to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    host.contains('.') && host.len() >= 4 && window_title.to_lowercase().contains(host)
}

/// Parses "Ctrl+Alt+A", "Cmd+Shift+F1" and similar into a global hotkey.
/// Accepted modifiers: Ctrl/Control, Alt/Option, Shift, Cmd/Command/Super/Meta,
/// CmdOrCtrl. The final part is the key name (letters, digits, F1..F24, Space, Enter, ...).
pub fn parse_hotkey(s: &str) -> Result<HotKey, AutotypeError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(AutotypeError::Hotkey(s.to_string(), "empty".into()));
    }
    trimmed
        .parse::<HotKey>()
        .map_err(|e| AutotypeError::Hotkey(s.to_string(), e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers as HkMods};

    fn m(shift: bool, ctrl: bool, alt: bool) -> Modifiers {
        Modifiers { shift, ctrl, alt }
    }

    #[test]
    fn default_sequence() {
        let t = parse_sequence("{USERNAME}{TAB}{PASSWORD}{ENTER}").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Field("USERNAME".into()),
                Token::Key(Key::Tab),
                Token::Field("PASSWORD".into()),
                Token::Key(Key::Enter),
            ]
        );
    }

    #[test]
    fn repeat_counts() {
        let t = parse_sequence("{TAB 3}x").unwrap();
        assert_eq!(
            t,
            vec![Token::Key(Key::Tab), Token::Key(Key::Tab), Token::Key(Key::Tab), Token::Char('x')]
        );
        assert!(matches!(parse_sequence("{TAB 0}"), Err(AutotypeError::InvalidCount(_))));
        assert!(matches!(parse_sequence("{TAB abc}"), Err(AutotypeError::InvalidCount(_))));
    }

    #[test]
    fn delay() {
        assert_eq!(parse_sequence("{DELAY 500}").unwrap(), vec![Token::Delay(500)]);
        assert!(matches!(parse_sequence("{DELAY}"), Err(AutotypeError::InvalidCount(_))));
    }

    #[test]
    fn clearfield() {
        assert_eq!(parse_sequence("{CLEARFIELD}{PASSWORD}").unwrap(), vec![Token::ClearField, Token::Field("PASSWORD".into())]);
    }

    fn field(name: &str) -> Token {
        Token::Field(name.into())
    }

    #[test]
    fn adapt_default_to_focused_field() {
        let seq = parse_sequence("{USERNAME}{TAB}{PASSWORD}{ENTER}").unwrap();
        let full = vec![Token::ClearField, field("USERNAME"), Token::Key(Key::Tab), Token::ClearField, field("PASSWORD"), Token::Key(Key::Enter)];
        // Nothing known about the field: clear before each value.
        assert_eq!(adapt_default(&seq, None, "me@x.com"), full);
        let text = |v: &str| FocusedField { is_password: false, value: Some(v.into()) };
        // A username field with some other text: still typed, after clearing.
        assert_eq!(adapt_default(&seq, Some(&text("old")), "me@x.com"), full);
        // Already holds this username: go straight to the password.
        assert_eq!(
            adapt_default(&seq, Some(&text(" ME@x.com ")), "me@x.com"),
            vec![Token::Key(Key::Tab), Token::ClearField, field("PASSWORD"), Token::Key(Key::Enter)]
        );
        // Focus in a password field: password-only page.
        let pw = FocusedField { is_password: true, value: None };
        assert_eq!(adapt_default(&seq, Some(&pw), "me@x.com"), vec![Token::ClearField, field("PASSWORD"), Token::Key(Key::Enter)]);
        // A sequence of another shape is only given the clears.
        let other = parse_sequence("{PASSWORD}{ENTER}").unwrap();
        assert_eq!(adapt_default(&other, Some(&pw), "me"), vec![Token::ClearField, field("PASSWORD"), Token::Key(Key::Enter)]);
    }

    #[test]
    fn url_host_in_title() {
        assert!(url_host_matches("https://www.Netflix.com/login", "netflix.com - Sign in - Microsoft Edge"));
        assert!(url_host_matches("user@intranet.corp.local:8443", "Intranet.Corp.local portal"));
        assert!(!url_host_matches("https://netflix.com", "Netflix - Microsoft Edge"));
        assert!(!url_host_matches("", "anything"));
        assert!(!url_host_matches("localhost", "localhost"));
    }

    #[test]
    fn custom_fields() {
        let t = parse_sequence("{S:Recovery PIN}{title}{url}{notes}").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Field("S:Recovery PIN".into()),
                Token::Field("TITLE".into()),
                Token::Field("URL".into()),
                Token::Field("NOTES".into()),
            ]
        );
    }

    #[test]
    fn escapes() {
        let t = parse_sequence("{{}{}}{+}{^}{%}{(}{)}").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Char('{'),
                Token::Char('}'),
                Token::Char('+'),
                Token::Char('^'),
                Token::Char('%'),
                Token::Char('('),
                Token::Char(')'),
            ]
        );
    }

    #[test]
    fn plain_characters_and_unicode() {
        let t = parse_sequence("aé").unwrap();
        assert_eq!(t, vec![Token::Char('a'), Token::Char('é')]);
    }

    #[test]
    fn modifier_on_key() {
        let t = parse_sequence("^a+{TAB}%^x").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Modified(m(false, true, false), Box::new(Token::Char('a'))),
                Token::Modified(m(true, false, false), Box::new(Token::Key(Key::Tab))),
                Token::Modified(m(false, true, true), Box::new(Token::Char('x'))),
            ]
        );
    }

    #[test]
    fn modifier_on_group() {
        let t = parse_sequence("^(ac)b").unwrap();
        assert_eq!(
            t,
            vec![
                Token::Modified(m(false, true, false), Box::new(Token::Char('a'))),
                Token::Modified(m(false, true, false), Box::new(Token::Char('c'))),
                Token::Char('b'),
            ]
        );
        assert!(matches!(parse_sequence("^(ab"), Err(AutotypeError::UnterminatedGroup(_))));
        assert!(matches!(parse_sequence("ab^"), Err(AutotypeError::DanglingModifier(_))));
    }

    #[test]
    fn unknown_placeholder() {
        match parse_sequence("{USERNAME}{XYZ}") {
            Err(AutotypeError::UnknownPlaceholder(name)) => assert_eq!(name, "XYZ"),
            other => panic!("expected UnknownPlaceholder, got {other:?}"),
        }
        assert!(matches!(parse_sequence("{TAB"), Err(AutotypeError::UnterminatedPlaceholder(0))));
    }

    #[test]
    fn window_glob() {
        assert!(window_matches("*Login*", "My Bank - Login - Firefox"));
        assert!(window_matches("*login*", "My Bank - LOGIN - Firefox"));
        assert!(window_matches("My Bank*", "my bank - Login"));
        assert!(!window_matches("Login", "My Bank - Login"));
        assert!(window_matches("Login", "login"));
        assert!(window_matches("B?nk*", "Bank of X"));
        assert!(!window_matches("B?nk", "Bank of X"));
        assert!(window_matches("*", ""));
        assert!(window_matches("*a*b*", "xxaxxbxx"));
        assert!(!window_matches("*a*b*", "xxbxxaxx"));
        assert!(window_matches("//bank//", "My BANK - Login"));
        assert!(!window_matches("//bank//", "Nothing here"));
    }

    #[test]
    fn title_min_length() {
        assert!(!title_matches("", "anything"));
        assert!(!title_matches("ab", "ab - Firefox"));
        assert!(title_matches("abc", "ABC - Firefox"));
        assert!(title_matches("GitHub", "Sign in to github · Chrome"));
        assert!(!title_matches("GitHub", "Sign in to gitlab"));
    }

    #[test]
    fn hotkey_round_trip() {
        let h = parse_hotkey("Ctrl+Alt+A").unwrap();
        assert_eq!(h.mods, HkMods::CONTROL | HkMods::ALT);
        assert_eq!(h.key, Code::KeyA);

        let h = parse_hotkey("Cmd+Shift+F1").unwrap();
        assert_eq!(h.mods, HkMods::META | HkMods::SHIFT);
        assert_eq!(h.key, Code::F1);

        let h = parse_hotkey("Shift+Space").unwrap();
        assert_eq!(h.mods, HkMods::SHIFT);
        assert_eq!(h.key, Code::Space);

        assert!(matches!(parse_hotkey("Ctrl+Nope"), Err(AutotypeError::Hotkey(..))));
        assert!(matches!(parse_hotkey(""), Err(AutotypeError::Hotkey(..))));
    }
}
