use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Default, Debug)]
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
            text.split_word_bound_indices()
                .find(|(i, s)| *i > self.caret && s.chars().any(char::is_alphanumeric))
                .map_or(text.len(), |(i, _)| i)
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
    pub fn select_word(&mut self, text: &str) {
        if let Some((i, word)) = text
            .split_word_bound_indices()
            .find(|(i, word)| self.caret >= *i && self.caret < *i + word.len())
        {
            self.anchor = Some(i);
            self.caret = i + word.len();
        }
    }
    pub fn delete_word(&mut self, text: &mut String, backward: bool) {
        if self.selection().is_none() {
            self.horizontal(text, !backward, true, true);
        }
        self.delete(text, backward);
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

struct Snapshot {
    text: zeroize::Zeroizing<String>,
    edit: Editor,
}
#[derive(Default)]
pub struct History {
    undo: std::collections::VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    typing: Option<(std::time::Instant, usize)>,
}
impl History {
    pub fn record(&mut self, text: &str, edit: &Editor, typing: bool) {
        let coalesce = typing
            && edit.selection().is_none()
            && self.typing.is_some_and(|(time, caret)| {
                time.elapsed() < std::time::Duration::from_millis(700) && caret == edit.caret
            });
        if !coalesce {
            if self.undo.len() == 32 {
                self.undo.pop_front();
            }
            self.undo.push_back(Snapshot {
                text: zeroize::Zeroizing::new(text.to_owned()),
                edit: edit.clone(),
            });
        }
        self.redo.clear();
        self.typing = None;
    }
    pub fn typed(&mut self, caret: usize) {
        self.typing = Some((std::time::Instant::now(), caret));
    }
    pub fn break_group(&mut self) {
        self.typing = None;
    }
    pub fn undo(&mut self, text: &mut String, edit: &mut Editor, redo: bool) {
        self.typing = None;
        let snapshot = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        if let Some(snapshot) = snapshot {
            let previous = Snapshot {
                text: zeroize::Zeroizing::new(std::mem::replace(text, snapshot.text.to_string())),
                edit: std::mem::replace(edit, snapshot.edit),
            };
            if redo {
                self.undo.push_back(previous);
            } else {
                self.redo.push(previous);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_undo_coalesces_and_word_deletion_is_reversible() {
        let mut history = History::default();
        let mut edit = Editor::default();
        let mut text = String::new();
        for c in ["H", "i"] {
            history.record(&text, &edit, true);
            edit.insert(&mut text, c, 4096);
            history.typed(edit.caret);
        }
        history.undo(&mut text, &mut edit, false);
        assert_eq!(text, "");
        history.undo(&mut text, &mut edit, true);
        assert_eq!(text, "Hi");
        history.record(&text, &edit, false);
        edit.insert(&mut text, " maple leaf", 4096);
        history.record(&text, &edit, false);
        edit.delete_word(&mut text, true);
        assert_eq!(text, "Hi maple ");
        history.undo(&mut text, &mut edit, false);
        assert_eq!(text, "Hi maple leaf");
        edit.move_to(&text, 0, false);
        edit.horizontal(&text, true, false, true);
        assert_eq!(edit.caret, 3);
        edit.select_word(&text);
        assert_eq!(edit.selection(), Some((3, 8)));
    }
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
