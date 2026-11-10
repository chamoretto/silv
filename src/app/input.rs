//! Single line text editor with history.

#[derive(Clone, Debug, Default)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    pub history: Vec<String>,
    history_index: Option<usize>,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: &str) -> Self {
        let mut editor = Self::default();
        editor.set(text);
        editor
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub const fn len(&self) -> usize {
        self.chars.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self.history_index = None;
    }

    /// Loads text without disturbing history navigation state.
    fn set_recalled(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
        self.history_index = None;
    }

    pub fn insert(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
        self.history_index = None;
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Moves the cursor to `position` clamped to the text.
    pub fn move_to(&mut self, position: usize) {
        self.cursor = position.min(self.chars.len());
    }

    /// Records the current text in history and returns it.
    pub fn commit(&mut self) -> String {
        let text = self.text();
        if !text.is_empty() && self.history.last().map(|last| *last != text).unwrap_or(true) {
            self.history.push(text.clone());
            if self.history.len() > HISTORY_LIMIT {
                let overflow = self.history.len() - HISTORY_LIMIT;
                self.history.drain(0..overflow);
            }
        }
        self.history_index = None;
        text
    }

    /// History navigation: `backward` walks to older entries.
    pub fn history_previous(&mut self) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let next = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => self.history.len() - 1,
        };
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_recalled(&text);
        Some(text)
    }

    pub fn history_next(&mut self) -> Option<String> {
        let index = self.history_index?;
        let next = index + 1;
        if next >= self.history.len() {
            self.history_index = None;
            self.clear();
            return Some(String::new());
        }
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_recalled(&text);
        Some(text)
    }
}

pub const HISTORY_LIMIT: usize = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace() {
        let mut editor = Editor::new();
        editor.insert('a');
        editor.insert('b');
        assert_eq!(editor.text(), "ab");
        editor.left();
        editor.insert('c');
        assert_eq!(editor.text(), "acb");
        assert!(editor.backspace());
        assert_eq!(editor.text(), "ab");
        editor.home();
        assert!(!editor.backspace());
    }

    #[test]
    fn cursor_movement_is_clamped() {
        let mut editor = Editor::with_text("abc");
        editor.home();
        assert_eq!(editor.cursor(), 0);
        editor.end();
        assert_eq!(editor.cursor(), 3);
        editor.right();
        assert_eq!(editor.cursor(), 3);
        editor.delete();
        assert_eq!(editor.text(), "abc");
        editor.move_to(1);
        editor.delete();
        assert_eq!(editor.text(), "ac");
    }

    #[test]
    fn history_walks_back_and_forth() {
        let mut editor = Editor::new();
        editor.set("one");
        editor.commit();
        editor.set("two");
        editor.commit();
        editor.clear();
        editor.history_previous();
        assert_eq!(editor.text(), "two");
        editor.history_previous();
        assert_eq!(editor.text(), "one");
        editor.history_previous();
        assert_eq!(editor.text(), "one");
        editor.history_next();
        assert_eq!(editor.text(), "two");
        editor.history_next();
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn history_deduplicates_and_is_bounded() {
        let mut editor = Editor::new();
        for _ in 0..3 {
            editor.set("same");
            editor.commit();
        }
        assert_eq!(editor.history.len(), 1);
        for index in 0..(HISTORY_LIMIT + 5) {
            editor.set(&format!("value-{index}"));
            editor.commit();
        }
        assert!(editor.history.len() <= HISTORY_LIMIT);
        assert_eq!(editor.history.last().map(String::as_str), Some("value-54"));
    }
}
