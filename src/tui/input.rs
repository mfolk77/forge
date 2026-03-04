/// Multi-line text input with history
pub struct InputState {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            history: Vec::new(),
            history_pos: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    pub fn insert_char(&mut self, ch: char) {
        let line = &mut self.lines[self.cursor_line];
        if self.cursor_col >= line.len() {
            line.push(ch);
        } else {
            line.insert(self.cursor_col, ch);
        }
        self.cursor_col += 1;
    }

    pub fn insert_newline(&mut self) {
        let current = &self.lines[self.cursor_line];
        let rest = current[self.cursor_col..].to_string();
        self.lines[self.cursor_line] = current[..self.cursor_col].to_string();
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, rest);
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.lines[self.cursor_line].remove(self.cursor_col - 1);
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
        }
    }

    pub fn submit(&mut self) -> String {
        let text = self.text();
        if !text.trim().is_empty() {
            self.history.push(text.clone());
        }
        self.lines = vec![String::new()];
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.history_pos = None;
        text
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.history_pos {
            Some(0) => return,
            Some(p) => p - 1,
            None => self.history.len() - 1,
        };
        self.history_pos = Some(pos);
        let entry = self.history[pos].clone();
        self.lines = entry.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = self.lines.len() - 1;
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    pub fn history_down(&mut self) {
        match self.history_pos {
            Some(p) if p + 1 < self.history.len() => {
                self.history_pos = Some(p + 1);
                let entry = self.history[p + 1].clone();
                self.lines = entry.lines().map(String::from).collect();
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.cursor_line = self.lines.len() - 1;
                self.cursor_col = self.lines[self.cursor_line].len();
            }
            _ => {
                self.history_pos = None;
                self.lines = vec![String::new()];
                self.cursor_line = 0;
                self.cursor_col = 0;
            }
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor_col < self.lines[self.cursor_line].len() {
            self.cursor_col += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_input() {
        let mut input = InputState::new();
        assert!(input.is_empty());

        input.insert_char('h');
        input.insert_char('i');
        assert_eq!(input.text(), "hi");
        assert!(!input.is_empty());
    }

    #[test]
    fn test_newline() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_newline();
        input.insert_char('b');
        assert_eq!(input.text(), "a\nb");
        assert_eq!(input.lines.len(), 2);
    }

    #[test]
    fn test_backspace() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.backspace();
        assert_eq!(input.text(), "a");
    }

    #[test]
    fn test_backspace_across_lines() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_newline();
        input.backspace();
        assert_eq!(input.text(), "a");
        assert_eq!(input.lines.len(), 1);
    }

    #[test]
    fn test_submit() {
        let mut input = InputState::new();
        input.insert_char('h');
        input.insert_char('i');
        let text = input.submit();
        assert_eq!(text, "hi");
        assert!(input.is_empty());
        assert_eq!(input.history.len(), 1);
    }

    #[test]
    fn test_history() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.submit();
        input.insert_char('b');
        input.submit();

        input.history_up();
        assert_eq!(input.text(), "b");

        input.history_up();
        assert_eq!(input.text(), "a");

        input.history_down();
        assert_eq!(input.text(), "b");

        input.history_down();
        assert!(input.is_empty());
    }
}
