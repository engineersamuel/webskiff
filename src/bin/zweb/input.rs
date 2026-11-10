//! UTF-8 prompt editing, independent of page input.

use crate::config::Action;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn character(key: KeyEvent) -> Option<char> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    match key.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Changed,
    Moved,
    Ignored,
}

#[derive(Debug)]
pub struct TextInput {
    pub text: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            cursor: text.len(),
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    fn next(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
    }

    pub fn edit(&mut self, action: Action) -> Edit {
        match action {
            Action::Clear => {
                self.text.clear();
                self.cursor = 0;
            }
            Action::DeleteWord => {
                let before = self.text[..self.cursor].trim_end();
                let cut = before.rfind([' ', '/', '.']).map_or(0, |at| at + 1);
                self.text.drain(cut..self.cursor);
                self.cursor = cut;
            }
            Action::Backspace => {
                let previous = self.previous();
                self.text.drain(previous..self.cursor);
                self.cursor = previous;
            }
            Action::Delete => {
                self.text.drain(self.cursor..self.next());
            }
            Action::CursorLeft => {
                self.cursor = self.previous();
                return Edit::Moved;
            }
            Action::CursorRight => {
                self.cursor = self.next();
                return Edit::Moved;
            }
            Action::CursorHome => {
                self.cursor = 0;
                return Edit::Moved;
            }
            Action::CursorEnd => {
                self.cursor = self.text.len();
                return Edit::Moved;
            }
            _ => return Edit::Ignored,
        }
        Edit::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_at_the_cursor_without_splitting_utf8() {
        let mut input = TextInput::new("aéc");
        input.edit(Action::CursorLeft);
        input.insert("中");
        assert_eq!(input.text, "aé中c");
        input.edit(Action::Backspace);
        input.edit(Action::CursorLeft);
        input.edit(Action::Delete);
        assert_eq!(input.text, "ac");
        input.edit(Action::CursorHome);
        input.insert("b");
        input.edit(Action::CursorEnd);
        input.insert("d");
        assert_eq!(input.text, "bacd");
    }

    #[test]
    fn clear_and_word_delete_work_in_the_middle() {
        let mut input = TextInput::new("one two end");
        input.cursor = "one two".len();
        input.edit(Action::DeleteWord);
        assert_eq!(input.text, "one  end");
        assert_eq!(input.cursor, 4);
        input.edit(Action::Clear);
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
    }
}
