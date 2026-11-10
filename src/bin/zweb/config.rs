//! Compile per-mode key bindings once, before starting the browser.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DEFAULTS: &str = include_str!("../../../config.example.toml");

macro_rules! actions {
    ($($variant:ident => $name:literal, $description:literal;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action { $($variant,)* }
        const ACTIONS: &[(&str, Action, &str)] = &[
            $(($name, Action::$variant, $description),)*
        ];
    };
}

actions! {
    Open => "open", "Address or web search";
    EditAddress => "edit_address", "Edit the current address";
    NewTab => "new_tab", "Open an address or web search in a new tab";
    CloseTab => "close_tab", "Close the current tab";
    NextTab => "next_tab", "Next browser tab";
    PreviousTab => "previous_tab", "Previous browser tab";
    Find => "find", "Find text in this page";
    NextMatch => "next_match", "Next page match";
    PreviousMatch => "previous_match", "Previous page match";
    Help => "help", "Keyboard help";
    Quit => "quit", "Quit the browser";
    QuitHint => "quit_hint", "Show how to quit, without quitting";
    FocusNext => "focus_next", "Focus the next page control";
    FocusPrevious => "focus_previous", "Focus the previous page control";
    Activate => "activate", "Activate the focused page control";
    PageEscape => "page_escape", "Send Escape to the page";
    ScrollDown => "scroll_down", "Scroll down";
    ScrollUp => "scroll_up", "Scroll up";
    ScrollLeft => "scroll_left", "Scroll left";
    ScrollRight => "scroll_right", "Scroll right";
    HalfDown => "half_down", "Scroll half a page down";
    HalfUp => "half_up", "Scroll half a page up";
    PageDown => "page_down", "Page down";
    PageUp => "page_up", "Page up";
    Top => "top", "Go to the top";
    Bottom => "bottom", "Go to the bottom";
    Back => "back", "History back";
    Forward => "forward", "History forward";
    Reload => "reload", "Reload the page";
    Stop => "stop", "Stop loading";
    Tab1 => "tab_1", "Select browser tab 1";
    Tab2 => "tab_2", "Select browser tab 2";
    Tab3 => "tab_3", "Select browser tab 3";
    Tab4 => "tab_4", "Select browser tab 4";
    Tab5 => "tab_5", "Select browser tab 5";
    Tab6 => "tab_6", "Select browser tab 6";
    Tab7 => "tab_7", "Select browser tab 7";
    Tab8 => "tab_8", "Select browser tab 8";
    Tab9 => "tab_9", "Select browser tab 9";
    Insert => "insert", "Send keys to the page";
    FocusInput => "focus_input", "Focus the first text field";
    Hints => "hints", "Show link hints";
    HintsNewTab => "hints_new_tab", "Show hints to open in a new tab";
    CopyUrl => "copy_url", "Copy the current URL";
    ToggleImages => "toggle_images", "Toggle image placeholders";
    ZoomIn => "zoom_in", "Zoom in";
    ZoomOut => "zoom_out", "Zoom out";
    ZoomReset => "zoom_reset", "Reset zoom";
    Cancel => "cancel", "Cancel or leave this mode";
    Accept => "accept", "Accept the prompt (next match in global find)";
    CursorLeft => "cursor_left", "Move the prompt cursor left";
    CursorRight => "cursor_right", "Move the prompt cursor right";
    CursorHome => "cursor_home", "Move to the start of the prompt";
    CursorEnd => "cursor_end", "Move to the end of the prompt";
    Backspace => "backspace", "Delete before the cursor";
    Delete => "delete", "Delete after the cursor";
    Clear => "clear", "Clear the prompt";
    DeleteWord => "delete_word", "Delete the word before the cursor";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Browse,
    Insert,
    Prompt,
    Hint,
    Help,
}

const SCOPES: &[(Scope, &str)] = &[
    (Scope::Global, "global"),
    (Scope::Browse, "browse"),
    (Scope::Insert, "insert"),
    (Scope::Prompt, "prompt"),
    (Scope::Hint, "hint"),
    (Scope::Help, "help"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stroke(KeyCode, KeyModifiers);

impl Stroke {
    pub fn from_event(event: KeyEvent) -> Self {
        let mut code = event.code;
        let mut mods = event.modifiers;
        if code == KeyCode::BackTab {
            code = KeyCode::Tab;
            mods.insert(KeyModifiers::SHIFT);
        }
        if let KeyCode::Char(mut c) = code {
            if mods.contains(KeyModifiers::CONTROL) {
                c = c.to_ascii_lowercase();
            } else if c.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(c) {
                mods.insert(KeyModifiers::SHIFT);
            } else if mods.contains(KeyModifiers::SHIFT) {
                c = c.to_ascii_uppercase();
            }
            code = KeyCode::Char(c);
        }
        Self(code, mods)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut text = text;
        let mut mods = KeyModifiers::NONE;
        loop {
            let modifier = [
                ("Ctrl-", KeyModifiers::CONTROL),
                ("Alt-", KeyModifiers::ALT),
                ("Shift-", KeyModifiers::SHIFT),
                ("Super-", KeyModifiers::SUPER),
            ]
            .into_iter()
            .find(|(prefix, _)| text.starts_with(prefix));
            let Some((prefix, flag)) = modifier else {
                break;
            };
            mods.insert(flag);
            text = &text[prefix.len()..];
        }
        let code = named_key(text)
            .or_else(|| {
                let mut chars = text.chars();
                let c = chars.next()?;
                chars.next().is_none().then_some(KeyCode::Char(c))
            })
            .ok_or_else(|| format!("unknown key {text:?}"))?;
        Ok(Self::from_event(KeyEvent::new(code, mods)))
    }
}

fn named_key(name: &str) -> Option<KeyCode> {
    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&number).then_some(KeyCode::F(number));
    }
    const NAMED: &[(&str, KeyCode)] = &[
        ("Space", KeyCode::Char(' ')),
        ("Tab", KeyCode::Tab),
        ("Enter", KeyCode::Enter),
        ("Esc", KeyCode::Esc),
        ("Escape", KeyCode::Esc),
        ("Backspace", KeyCode::Backspace),
        ("Delete", KeyCode::Delete),
        ("Left", KeyCode::Left),
        ("Right", KeyCode::Right),
        ("Up", KeyCode::Up),
        ("Down", KeyCode::Down),
        ("Home", KeyCode::Home),
        ("End", KeyCode::End),
        ("PageUp", KeyCode::PageUp),
        ("PageDown", KeyCode::PageDown),
    ];
    NAMED
        .iter()
        .find(|(text, _)| *text == name)
        .map(|(_, code)| *code)
}

#[derive(Debug, Default)]
struct ModeKeys {
    actions: HashMap<Vec<Stroke>, Action>,
    prefixes: HashSet<Stroke>,
    labels: HashMap<Action, Vec<String>>,
}

impl ModeKeys {
    fn add(&mut self, action: Action, label: &str, scope: Scope) -> Result<(), String> {
        let keys: Vec<Stroke> = label
            .split_whitespace()
            .map(Stroke::parse)
            .collect::<Result<_, _>>()?;
        if keys.is_empty() || keys.len() > 2 || (keys.len() == 2 && scope != Scope::Browse) {
            return Err(format!(
                "invalid sequence {label:?}; only browse supports two-key sequences"
            ));
        }
        if self.actions.insert(keys.clone(), action).is_some() {
            return Err(format!(
                "duplicate binding {label:?}; remove it from its other action"
            ));
        }
        if keys.len() == 2 {
            self.prefixes.insert(keys[0]);
        }
        self.labels.entry(action).or_default().push(label.into());
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self
            .prefixes
            .iter()
            .any(|prefix| self.actions.contains_key(&[*prefix][..]))
        {
            return Err("a sequence prefix is also bound to a single-key action".into());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Config {
    modes: [ModeKeys; 6],
}

pub enum Binding {
    Action(Action),
    Prefix(Stroke),
    Unbound,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let home =
            std::env::var_os("HOME").ok_or("HOME is not set; cannot locate webskiff config")?;
        let path = Path::new(&home).join(".config/webskiff/config.toml");
        Self::load_path(&path).map_err(|error| format!("{}: {error}", path.display()))
    }

    fn load_path(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::parse(""),
            Err(error) => Err(error.to_string()),
        }
    }

    fn parse(overrides: &str) -> Result<Self, String> {
        let defaults = DEFAULTS.parse::<toml::Table>().map_err(|e| e.to_string())?;
        let user = overrides
            .parse::<toml::Table>()
            .map_err(|e| e.to_string())?;
        validate_tables(&user)?;
        let mut config = Self {
            modes: std::array::from_fn(|_| ModeKeys::default()),
        };
        for &(scope, name) in SCOPES {
            let mut table = defaults["keys"][name]
                .as_table()
                .ok_or("invalid embedded defaults")?
                .clone();
            if let Some(custom) = user.get("keys").and_then(|keys| keys.get(name)) {
                let custom = custom
                    .as_table()
                    .ok_or_else(|| format!("keys.{name} must be a table"))?;
                table.extend(custom.clone());
            }
            config.compile_mode(scope, name, &table)?;
        }
        Ok(config)
    }

    fn compile_mode(
        &mut self,
        scope: Scope,
        name: &str,
        table: &toml::Table,
    ) -> Result<(), String> {
        let mode = &mut self.modes[scope as usize];
        for (command, value) in table {
            let action = ACTIONS
                .iter()
                .find(|(key, _, _)| *key == command)
                .map(|(_, action, _)| *action)
                .ok_or_else(|| format!("unknown action keys.{name}.{command}"))?;
            if !allowed(scope, action) {
                return Err(format!("{command} is not available in {name} mode"));
            }
            let keys = value
                .as_array()
                .ok_or_else(|| format!("keys.{name}.{command} must be an array of key strings"))?;
            for value in keys {
                let key = value
                    .as_str()
                    .ok_or_else(|| format!("keys.{name}.{command} contains a non-string key"))?;
                mode.add(action, key, scope)
                    .map_err(|e| format!("keys.{name}.{command}: {e}"))?;
            }
        }
        mode.validate().map_err(|e| format!("keys.{name}: {e}"))
    }

    pub fn resolve(&self, scope: Scope, key: KeyEvent, prefix: Option<Stroke>) -> Binding {
        let stroke = Stroke::from_event(key);
        let mode = &self.modes[scope as usize];
        if let Some(first) = prefix {
            if let Some(action) = mode.actions.get(&[first, stroke][..]) {
                return Binding::Action(*action);
            }
        }
        if let Some(action) = mode.actions.get(&[stroke][..]) {
            Binding::Action(*action)
        } else if mode.prefixes.contains(&stroke) {
            Binding::Prefix(stroke)
        } else {
            Binding::Unbound
        }
    }

    pub fn action(&self, scope: Scope, key: KeyEvent) -> Option<Action> {
        match self.resolve(scope, key, None) {
            Binding::Action(action) => Some(action),
            _ => None,
        }
    }

    pub fn labels(&self, scope: Scope, action: Action) -> String {
        self.modes[scope as usize]
            .labels
            .get(&action)
            .map_or_else(|| "unbound".into(), |keys| keys.join("/"))
    }

    pub fn help_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "Webskiff keyboard help - configured bindings".into(),
            "Config: ~/.config/webskiff/config.toml (missing entries use repo defaults)".into(),
            format!(
                "Help: {} scroll; {} page; {} close",
                self.labels(Scope::Help, Action::ScrollDown),
                self.labels(Scope::Help, Action::PageDown),
                self.labels(Scope::Help, Action::Cancel)
            ),
            "Find searches this page; Open accepts an address or web search.".into(),
            "Some terminal apps intercept keys.".into(),
        ];
        for &(scope, name) in SCOPES {
            lines.push(String::new());
            lines.push(format!(" {} MODE", name.to_ascii_uppercase()));
            let mode = &self.modes[scope as usize];
            for &(_, action, description) in ACTIONS {
                if let Some(keys) = mode.labels.get(&action) {
                    lines.push(format!("{}: {description}", keys.join(", ")));
                }
            }
        }
        lines
    }
}

fn validate_tables(user: &toml::Table) -> Result<(), String> {
    if let Some(name) = user.keys().find(|key| key.as_str() != "keys") {
        return Err(format!(
            "unknown config section {name:?}; expected [keys.MODE]"
        ));
    }
    if let Some(keys) = user.get("keys") {
        let keys = keys.as_table().ok_or("keys must be a table")?;
        if let Some(name) = keys
            .keys()
            .find(|key| !SCOPES.iter().any(|(_, name)| name == key))
        {
            return Err(format!("unknown keyboard mode {name:?}"));
        }
    }
    Ok(())
}

fn allowed(scope: Scope, action: Action) -> bool {
    use Action::*;
    match scope {
        Scope::Global | Scope::Browse => !matches!(
            action,
            Cancel
                | Accept
                | CursorLeft
                | CursorRight
                | CursorHome
                | CursorEnd
                | Backspace
                | Delete
                | Clear
                | DeleteWord
        ),
        Scope::Insert => action == Cancel,
        Scope::Hint => matches!(action, Cancel | Backspace),
        Scope::Prompt => matches!(
            action,
            Cancel
                | Accept
                | PreviousMatch
                | NextMatch
                | CursorLeft
                | CursorRight
                | CursorHome
                | CursorEnd
                | Backspace
                | Delete
                | Clear
                | DeleteWord
        ),
        Scope::Help => matches!(
            action,
            Cancel | ScrollDown | ScrollUp | PageDown | PageUp | Top | Bottom
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(config: &Config, scope: Scope, key: &str) -> Option<Action> {
        let Stroke(code, modifiers) = Stroke::parse(key).unwrap();
        match config.resolve(scope, KeyEvent::new(code, modifiers), None) {
            Binding::Action(action) => Some(action),
            _ => None,
        }
    }

    #[test]
    fn embedded_defaults_and_partial_overrides() {
        let config =
            Config::parse("[keys.global]\nfind = ['Ctrl-S']\n[keys.browse]\nquit_hint = []")
                .unwrap();
        assert_eq!(action(&config, Scope::Global, "Ctrl-S"), Some(Action::Find));
        assert_eq!(action(&config, Scope::Global, "Ctrl-F"), None);
        assert_eq!(action(&config, Scope::Browse, "q"), None);
        assert_eq!(
            action(&config, Scope::Browse, "j"),
            Some(Action::ScrollDown)
        );
        assert!(config
            .help_lines()
            .iter()
            .any(|line| line.contains("Ctrl-S")));
        assert!(!config
            .help_lines()
            .iter()
            .any(|line| line.contains("Ctrl-F")));
    }

    #[test]
    fn rejects_invalid_and_conflicting_bindings() {
        for text in [
            "[keys.browse]\nscroll_down = ['k']",
            "[keys.browse]\nscroll_down = ['g']",
            "[keys.browse]\nscroll_down = ['UnknownKey']",
            "[keys.global]\nfind = ['g f']",
            "[keys.prompt]\nquit = ['Ctrl-X']",
            "[keys.global]\nfind = 42",
            "[keys.global]\nfind = [42]",
            "[keys.browse]\ntypo = []",
            "[keys.typo]",
            "[unknown]",
            "[keys.global]\nfind = [",
        ] {
            assert!(Config::parse(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn loading_a_file_reports_errors_instead_of_using_defaults() {
        let path =
            std::env::temp_dir().join(format!("webskiff-config-{}.toml", std::process::id()));
        assert!(!path.exists());
        assert!(Config::load_path(&path).is_ok());
        std::fs::write(&path, "[keys.global]\nfind = ['Ctrl-S']").unwrap();
        let config = Config::load_path(&path).unwrap();
        assert_eq!(action(&config, Scope::Global, "Ctrl-S"), Some(Action::Find));
        std::fs::write(&path, "invalid = [").unwrap();
        let invalid = Config::load_path(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(invalid.is_err());
        assert!(Config::load_path(&std::env::temp_dir()).is_err());
    }

    #[test]
    fn normalizes_terminal_shift_and_control_encodings() {
        for (text, event) in [
            ("N", KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE)),
            ("?", KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            (
                "Ctrl-F",
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            ),
            (
                "Shift-Tab",
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            ),
        ] {
            assert_eq!(Stroke::parse(text).unwrap(), Stroke::from_event(event));
        }
        let config = Config::parse("").unwrap();
        let prefix = match config.resolve(Scope::Browse, KeyEvent::from(KeyCode::Char('g')), None) {
            Binding::Prefix(prefix) => prefix,
            _ => panic!("missing g prefix"),
        };
        assert!(matches!(
            config.resolve(
                Scope::Browse,
                KeyEvent::from(KeyCode::Char('i')),
                Some(prefix)
            ),
            Binding::Action(Action::FocusInput)
        ));
    }
}
