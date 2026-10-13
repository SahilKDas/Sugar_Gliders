use crate::editor::Editor;
use maple_protocol::{Command, Secret};
pub struct Form {
    pub edits: [Editor; 2],
    pub username: String,
    pub password: Secret,
    pub field: usize,
    pub register: bool,
    pub pending: bool,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            edits: [Editor::default(), Editor::default()],
            username: String::new(),
            password: Secret(String::new()),
            field: 0,
            register: false,
            pending: false,
        }
    }
}
impl Form {
    pub fn insert(&mut self, text: &str) {
        if self.pending {
            return;
        }
        if self.field == 0 {
            let filtered: String = text
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            self.edits[0].insert(&mut self.username, &filtered, 32);
        } else {
            self.edits[1].insert(&mut self.password.0, text, 256);
        }
    }
    pub fn backspace(&mut self) {
        self.delete(true);
    }
    pub fn delete(&mut self, backward: bool) {
        if self.pending {
            return;
        }
        if self.field == 0 {
            self.edits[0].delete(&mut self.username, backward);
        } else {
            self.edits[1].delete(&mut self.password.0, backward);
        }
    }
    pub fn navigate(&mut self, right: bool, selecting: bool, word: bool) {
        let text = if self.field == 0 {
            &self.username
        } else {
            &self.password.0
        };
        self.edits[self.field].horizontal(text, right, selecting, word);
    }
    pub fn edge(&mut self, end: bool, selecting: bool) {
        let text = if self.field == 0 {
            &self.username
        } else {
            &self.password.0
        };
        self.edits[self.field].move_to(text, if end { text.len() } else { 0 }, selecting);
    }
    pub fn select_all(&mut self) {
        let text = if self.field == 0 {
            &self.username
        } else {
            &self.password.0
        };
        self.edits[self.field].select_all(text);
    }
    pub fn click(&mut self, x: f64, y: f64, w: f64, h: f64) -> bool {
        let left = w / 2.0 - 250.0;
        let top = h / 2.0 - 200.0;
        if !(left..left + 500.0).contains(&x) {
            return false;
        }
        if (top + 90.0..top + 138.0).contains(&y) {
            self.field = 0;
        }
        if (top + 160.0..top + 208.0).contains(&y) {
            self.field = 1;
        }
        if (top + 294.0..top + 335.0).contains(&y) {
            self.register = !self.register;
            self.pending = false;
        }
        (top + 235.0..top + 280.0).contains(&y)
    }
    pub fn command(&mut self) -> Option<Command> {
        if self.pending || self.username.is_empty() || self.password.0.is_empty() {
            return None;
        }
        self.pending = true;
        let password = Secret(std::mem::take(&mut self.password.0));
        self.edits[1].end(&self.password.0);
        Some(if self.register {
            Command::Register {
                username: self.username.clone(),
                password,
            }
        } else {
            Command::Login {
                username: self.username.clone(),
                password,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_masked_bounded_and_removed_after_submission() {
        let mut form = Form::default();
        form.insert("Alice-_123");
        assert_eq!(form.username, "alice_123");
        form.field = 1;
        form.insert("long private password");
        assert!(matches!(form.command(), Some(Command::Login { .. })));
        assert!(form.password.0.is_empty());
        assert!(form.command().is_none());
    }
}
