use maple_protocol::{Command, Secret};
pub struct Form {
    pub username: String,
    pub password: Secret,
    pub field: usize,
    pub register: bool,
    pub pending: bool,
}
impl Default for Form {
    fn default() -> Self {
        Self {
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
            for c in text
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                if self.username.len() < 32 {
                    self.username.push(c.to_ascii_lowercase());
                }
            }
        } else {
            for c in text.chars().filter(|c| !c.is_control()) {
                if self.password.0.len() + c.len_utf8() <= 256 {
                    self.password.0.push(c);
                }
            }
        }
    }
    pub fn backspace(&mut self) {
        if self.field == 0 {
            self.username.pop();
        } else {
            self.password.0.pop();
        }
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
