//! Minimal editable text buffer (search box, notes, calculator).

#[derive(Default, Clone)]
pub struct TextField {
    pub text: String,
    /// Caret position as a byte offset into `text` (always on a char boundary).
    pub caret: usize,
    pub multiline: bool,
}

impl TextField {
    pub fn new(text: &str, multiline: bool) -> Self {
        TextField { text: text.to_string(), caret: text.len(), multiline }
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.caret = self.text.len();
    }
    pub fn insert(&mut self, c: char) {
        if c == '\n' && !self.multiline {
            return;
        }
        self.text.insert(self.caret, c);
        self.caret += c.len_utf8();
    }
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\r' {
                continue;
            }
            if c == '\n' && !self.multiline {
                self.insert(' ');
            } else if c == '\n' || !c.is_control() {
                self.insert(c);
            }
        }
    }
    fn prev(&self) -> usize {
        self.text[..self.caret].char_indices().last().map(|(i, _)| i).unwrap_or(0)
    }
    fn next(&self) -> usize {
        self.text[self.caret..].chars().next().map(|c| self.caret + c.len_utf8()).unwrap_or(self.caret)
    }
    pub fn backspace(&mut self) {
        if self.caret > 0 {
            let p = self.prev();
            self.text.replace_range(p..self.caret, "");
            self.caret = p;
        }
    }
    pub fn delete(&mut self) {
        let n = self.next();
        if n > self.caret {
            self.text.replace_range(self.caret..n, "");
        }
    }
    pub fn left(&mut self) {
        self.caret = self.prev();
    }
    pub fn right(&mut self) {
        self.caret = self.next();
    }
    pub fn home(&mut self) {
        self.caret = self.text[..self.caret].rfind('\n').map(|i| i + 1).unwrap_or(0);
    }
    pub fn end(&mut self) {
        self.caret = self.text[self.caret..].find('\n').map(|i| self.caret + i).unwrap_or(self.text.len());
    }
    /// Caret position in UTF-16 code units (what DirectWrite expects).
    pub fn caret_utf16(&self) -> u32 {
        self.text[..self.caret].encode_utf16().count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_unicode() {
        let mut t = TextField::new("", false);
        t.insert_str("مرحبا a");
        assert_eq!(t.text, "مرحبا a");
        t.left();
        t.left();
        t.backspace();
        assert_eq!(t.text, "مرحب a");
        t.home();
        t.delete();
        assert_eq!(t.text, "رحب a");
        t.end();
        assert_eq!(t.caret, t.text.len());
        t.insert('\n');
        assert_eq!(t.text, "رحب a");
    }
}
