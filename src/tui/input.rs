/// Multi-line text input with history.
///
/// `cursor_col` is a **byte offset** that ALWAYS sits on a UTF-8 char
/// boundary. We track byte positions (not character counts) so that
/// `String::insert`, `remove`, and slicing operations work without
/// panicking on multi-byte input.
///
/// BUG FIX (2026-05-10): before this revision the file used a hybrid
/// model — incrementing cursor_col by 1 per call (char count) while
/// passing it to byte-position APIs. Pasting any text with smart
/// quotes / em dashes / emoji / non-ASCII identifiers would drift the
/// cursor off a char boundary and the next operation would panic,
/// crashing forge. The user hit this on every paste.
pub struct InputState {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize, // byte offset, always on char boundary
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
        // Defensive: snap cursor_col to a valid char boundary if upstream
        // code ever lets it drift. Cheap (linear in cursor_col) and
        // prevents the entire panic class regardless of how we got here.
        while self.cursor_col > 0 && !line.is_char_boundary(self.cursor_col) {
            self.cursor_col -= 1;
        }
        if self.cursor_col >= line.len() {
            line.push(ch);
        } else {
            line.insert(self.cursor_col, ch);
        }
        // Advance by the byte width of the inserted character.
        self.cursor_col += ch.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        let current = &self.lines[self.cursor_line];
        let mut col = self.cursor_col;
        // Defensive: snap to char boundary before slicing.
        while col > 0 && !current.is_char_boundary(col) {
            col -= 1;
        }
        if col > current.len() {
            col = current.len();
        }
        let rest = current[col..].to_string();
        self.lines[self.cursor_line] = current[..col].to_string();
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, rest);
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_line];
            // Find the byte boundary of the character immediately before
            // the cursor. char_indices() yields (byte_idx, char) pairs
            // in ascending order; the largest byte_idx < cursor_col is
            // the start of the char we want to delete.
            let prev_boundary = line
                .char_indices()
                .map(|(b, _)| b)
                .take_while(|b| *b < self.cursor_col)
                .last()
                .unwrap_or(0);
            line.replace_range(prev_boundary..self.cursor_col, "");
            self.cursor_col = prev_boundary;
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
        if self.cursor_col == 0 {
            return;
        }
        let line = &self.lines[self.cursor_line];
        // Step back to the previous char boundary so cursor_col never
        // lands in the middle of a multi-byte character.
        let prev = line
            .char_indices()
            .map(|(b, _)| b)
            .take_while(|b| *b < self.cursor_col)
            .last()
            .unwrap_or(0);
        self.cursor_col = prev;
    }

    pub fn move_right(&mut self) {
        let line = &self.lines[self.cursor_line];
        if self.cursor_col >= line.len() {
            return;
        }
        // Step forward to the NEXT char boundary.
        let next = line
            .char_indices()
            .map(|(b, _)| b)
            .find(|b| *b > self.cursor_col)
            .unwrap_or(line.len());
        self.cursor_col = next;
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

    /// LIVE-OBSERVED 2026-05-10: forge crashed every time the user pasted
    /// text into the input box. Root cause: cursor_col was treated as a
    /// byte offset by String::insert/remove but incremented by 1 per char
    /// in insert_char. Any multi-byte UTF-8 character (smart quotes, em
    /// dashes, emoji, non-ASCII) would drift the cursor off a char
    /// boundary and the NEXT String operation would panic.
    #[test]
    fn test_paste_text_with_multibyte_chars_does_not_panic() {
        let mut input = InputState::new();
        // Simulate the paste loop: insert each char one by one.
        let pasted = "hello — “quoted” → arrow ✓ emoji 🚀 ok";
        for ch in pasted.chars() {
            input.insert_char(ch);
        }
        assert_eq!(input.text(), pasted);
    }

    #[test]
    fn test_insert_newline_at_multibyte_cursor() {
        let mut input = InputState::new();
        for ch in "café".chars() {
            input.insert_char(ch);
        }
        // Cursor is at the end (byte position 5 — 'é' is 2 bytes).
        input.insert_newline();
        input.insert_char('x');
        assert_eq!(input.text(), "café\nx");
    }

    #[test]
    fn test_backspace_across_multibyte_chars() {
        let mut input = InputState::new();
        for ch in "café 🚀".chars() {
            input.insert_char(ch);
        }
        // Three backspaces should remove rocket, space, and é cleanly.
        input.backspace();
        input.backspace();
        input.backspace();
        assert_eq!(input.text(), "caf");
    }

    #[test]
    fn test_move_left_right_with_multibyte() {
        let mut input = InputState::new();
        for ch in "a🚀b".chars() {
            input.insert_char(ch);
        }
        // Cursor at end (byte 6: a=1 + 🚀=4 + b=1).
        input.move_left();
        // Should be at byte 5 (start of 'b').
        input.move_left();
        // Should be at byte 1 (start of 🚀, skipped its 4 bytes).
        input.insert_char('X');
        // Inserted X before 🚀: "aX🚀b"
        assert_eq!(input.text(), "aX🚀b");
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
