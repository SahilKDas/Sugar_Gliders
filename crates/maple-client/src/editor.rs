use unicode_segmentation::UnicodeSegmentation;

#[derive(Default, Debug)]
pub struct Editor {
    pub caret: usize,
    anchor: Option<usize>,
}
impl Editor {
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .filter(|a| *a != self.caret)
            .map(|a| (a.min(self.caret), a.max(self.caret)))
    }
    pub fn end(&mut self, text: &str) {
        self.caret = text.len();
        self.anchor = None;
    }
    pub fn move_to(&mut self, text: &str, destination: usize, selecting: bool) {
        if selecting {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = destination.min(text.len());
        if self.caret < text.len() {
            self.caret = text
                .grapheme_indices(true)
                .take_while(|(i, _)| *i <= self.caret)
                .last()
                .map_or(0, |(i, _)| i);
        }
    }
    pub fn select_all(&mut self, text: &str) {
        self.anchor = Some(0);
        self.caret = text.len();
    }
    fn previous(&self, text: &str) -> usize {
        text[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self, text: &str) -> usize {
        self.caret
            + text[self.caret..]
                .graphemes(true)
                .next()
                .map_or(0, str::len)
    }
    pub fn horizontal(&mut self, text: &str, right: bool, selecting: bool, word: bool) {
        if !selecting {
            if let Some((a, b)) = self.selection() {
                self.move_to(text, if right { b } else { a }, false);
                return;
            }
        }
        let destination = if word && right {
            text[self.caret..]
                .split_word_bound_indices()
                .find(|(_, s)| s.chars().any(char::is_alphanumeric))
                .map_or(text.len(), |(i, s)| self.caret + i + s.len())
        } else if word {
            text[..self.caret]
                .split_word_bound_indices()
                .rev()
                .find(|(_, s)| s.chars().any(char::is_alphanumeric))
                .map_or(0, |(i, _)| i)
        } else if right {
            self.next(text)
        } else {
            self.previous(text)
        };
        self.move_to(text, destination, selecting);
    }
    pub fn delete(&mut self, text: &mut String, backward: bool) {
        let (a, b) = self.selection().unwrap_or_else(|| {
            if backward {
                (self.previous(text), self.caret)
            } else {
                (self.caret, self.next(text))
            }
        });
        text.replace_range(a..b, "");
        self.caret = a;
        self.anchor = None;
    }
    pub fn insert(&mut self, text: &mut String, value: &str, max: usize) {
        let (a, b) = self.selection().unwrap_or((self.caret, self.caret));
        let capacity = max.saturating_sub(text.len() - (b - a));
        let mut added = zeroize::Zeroizing::new(String::new());
        for grapheme in value.graphemes(true) {
            if grapheme.chars().any(char::is_control) {
                continue;
            }
            if added.len() + grapheme.len() > capacity {
                break;
            }
            added.push_str(grapheme);
        }
        if added.is_empty() {
            return;
        }
        text.replace_range(a..b, &added);
        self.caret = a + added.len();
        self.anchor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_navigation_selection_and_editing() {
        let mut text = "A👨‍👩‍👧‍👦e\u{301}B".to_string();
        let mut edit = Editor::default();
        edit.end(&text);
        edit.horizontal(&text, false, false, false);
        edit.delete(&mut text, true);
        assert_eq!(text, "A👨‍👩‍👧‍👦B");
        edit.delete(&mut text, true);
        assert_eq!(text, "AB");
        edit.insert(&mut text, " maple ", 4096);
        assert_eq!(text, "A maple B");
        edit.select_all(&text);
        edit.insert(&mut text, "Hello", 4096);
        assert_eq!(text, "Hello");
        edit.move_to(&text, 1, false);
        edit.move_to(&text, 4, true);
        edit.insert(&mut text, "i", 4096);
        assert_eq!(text, "Hio");
    }
    #[test]
    fn input_limits_do_not_split_graphemes_or_destroy_selection() {
        let mut text = "hello".to_string();
        let mut edit = Editor::default();
        edit.end(&text);
        edit.insert(&mut text, "🍁", 7);
        assert_eq!(text, "hello");
        edit.select_all(&text);
        edit.insert(&mut text, "\n", 7);
        assert_eq!(text, "hello");
        assert_eq!(edit.selection(), Some((0, 5)));
    }
}
