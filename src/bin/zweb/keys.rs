//! Terminal keys (crossterm) to DOM key events (zebkit).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zebkit::offscreen::{EditCommand, KeyEvent as ZKey, KeyEventType, Modifiers};
use zebkit::View;

pub struct DomKey {
    pub key: String,
    pub code: &'static str,
    pub key_code: i32,
    /// What the key types; `None` for keys that type nothing.
    pub text: Option<String>,
    pub modifiers: Modifiers,
    /// Editing commands for Ctrl shortcuts (macOS has no native bindings here).
    pub command: Option<&'static str>,
}

fn modifiers(m: KeyModifiers) -> Modifiers {
    let mut out = Modifiers::NONE;
    if m.contains(KeyModifiers::SHIFT) {
        out = out | Modifiers::SHIFT;
    }
    if m.contains(KeyModifiers::CONTROL) {
        out = out | Modifiers::CONTROL;
    }
    if m.contains(KeyModifiers::ALT) {
        out = out | Modifiers::ALT;
    }
    out
}

/// DOM `code` and Windows key code of the key that types `c` on a US layout.
fn char_code(c: char) -> (&'static str, i32) {
    const LETTERS: [&str; 26] = [
        "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK",
        "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV",
        "KeyW", "KeyX", "KeyY", "KeyZ",
    ];
    const DIGITS: [&str; 10] = [
        "Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8",
        "Digit9",
    ];
    let l = c.to_ascii_lowercase();
    if l.is_ascii_lowercase() {
        return (
            LETTERS[(l as u8 - b'a') as usize],
            l.to_ascii_uppercase() as i32,
        );
    }
    if c.is_ascii_digit() {
        return (DIGITS[(c as u8 - b'0') as usize], c as i32);
    }
    let shifted_digit = ")!@#$%^&*(".find(c);
    if let Some(d) = shifted_digit {
        return (DIGITS[d], b'0' as i32 + d as i32);
    }
    punctuation_code(c)
}

fn punctuation_code(c: char) -> (&'static str, i32) {
    match c {
        ' ' => ("Space", 32),
        '-' | '_' => ("Minus", 189),
        '=' | '+' => ("Equal", 187),
        '[' | '{' => ("BracketLeft", 219),
        ']' | '}' => ("BracketRight", 221),
        '\\' | '|' => ("Backslash", 220),
        ';' | ':' => ("Semicolon", 186),
        '\'' | '"' => ("Quote", 222),
        ',' | '<' => ("Comma", 188),
        '.' | '>' => ("Period", 190),
        '/' | '?' => ("Slash", 191),
        '`' | '~' => ("Backquote", 192),
        _ => ("", 0),
    }
}

pub fn to_dom(ev: &KeyEvent) -> Option<DomKey> {
    let mut mods = modifiers(ev.modifiers);
    if ev.code == KeyCode::BackTab {
        mods = mods | Modifiers::SHIFT;
    }
    match ev.code {
        KeyCode::Char(c) => Some(character_key(c, mods, ev.modifiers)),
        code => named_key(code, mods),
    }
}

fn character_key(c: char, mods: Modifiers, terminal_mods: KeyModifiers) -> DomKey {
    let ctrl = terminal_mods.contains(KeyModifiers::CONTROL);
    let alt = terminal_mods.contains(KeyModifiers::ALT);
    let mods = if c.is_uppercase() {
        mods | Modifiers::SHIFT
    } else {
        mods
    };
    let (code, key_code) = char_code(c);
    DomKey {
        key: c.to_string(),
        code,
        key_code,
        text: (!ctrl && !alt).then(|| c.to_string()),
        modifiers: mods,
        command: if ctrl { editing_command(c) } else { None },
    }
}

fn editing_command(c: char) -> Option<&'static str> {
    match c.to_ascii_lowercase() {
        'a' => Some("SelectAll"),
        'c' => Some("Copy"),
        'x' => Some("Cut"),
        'v' => Some("Paste"),
        'z' => Some("Undo"),
        _ => None,
    }
}

fn named_key(code: KeyCode, mods: Modifiers) -> Option<DomKey> {
    let (name, key_code, text) = match code {
        KeyCode::Enter => ("Enter", 13, Some("\r")),
        KeyCode::Tab | KeyCode::BackTab => ("Tab", 9, None),
        KeyCode::Backspace => ("Backspace", 8, None),
        KeyCode::Delete => ("Delete", 46, None),
        KeyCode::Esc => ("Escape", 27, None),
        KeyCode::Insert => ("Insert", 45, None),
        KeyCode::F(n @ 1..=12) => {
            const F: [&str; 12] = [
                "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
            ];
            (F[n as usize - 1], 111 + n as i32, None)
        }
        code => {
            let (name, key_code) = navigation_key(code)?;
            (name, key_code, None)
        }
    };
    Some(DomKey {
        key: name.into(),
        code: name,
        key_code,
        text: text.map(Into::into),
        modifiers: mods,
        command: None,
    })
}

fn navigation_key(code: KeyCode) -> Option<(&'static str, i32)> {
    Some(match code {
        KeyCode::Left => ("ArrowLeft", 37),
        KeyCode::Up => ("ArrowUp", 38),
        KeyCode::Right => ("ArrowRight", 39),
        KeyCode::Down => ("ArrowDown", 40),
        KeyCode::Home => ("Home", 36),
        KeyCode::End => ("End", 35),
        KeyCode::PageUp => ("PageUp", 33),
        KeyCode::PageDown => ("PageDown", 34),
        _ => return None,
    })
}

/// Sends one key press to the page: down, the typed text, up.
pub fn send(view: &View, k: &DomKey) {
    fn make<'a>(k: &'a DomKey, kind: KeyEventType, text: Option<&'a str>) -> ZKey<'a> {
        ZKey {
            kind,
            modifiers: k.modifiers,
            key_code: k.key_code,
            native_key_code: 0,
            code: Some(k.code),
            key: Some(&k.key),
            text,
            unmodified_text: text,
        }
    }
    let event = |kind, text| make(k, kind, text);
    let down = event(KeyEventType::Down, None);
    match k.command {
        Some(name) => {
            view.send_key_with_commands(&down, &[EditCommand { name, value: None }]);
        }
        None => view.send_key(&down),
    }
    if let Some(text) = &k.text {
        view.send_key(&event(KeyEventType::Char, Some(text.as_str())));
    }
    view.send_key(&event(KeyEventType::Up, None));
}
