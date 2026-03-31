use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::render::Theme;

/// A single command entry in the autocomplete dropdown.
#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub trigger: String,
    pub description: String,
}

/// Result of handling a key event in the autocomplete.
#[derive(Debug)]
pub enum AutocompleteResult {
    /// Keep the autocomplete open, continue filtering.
    Continue,
    /// User selected a command — return the trigger string.
    Selected(String),
    /// Dismiss the autocomplete.
    Dismiss,
}

/// Slash-command autocomplete dropdown.
pub struct Autocomplete {
    pub commands: Vec<CommandEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub query: String,
    pub active: bool,
}

impl Autocomplete {
    pub fn new(commands: Vec<CommandEntry>) -> Self {
        let filtered: Vec<usize> = (0..commands.len()).collect();
        Self {
            commands,
            filtered,
            selected: 0,
            query: String::new(),
            active: false,
        }
    }

    /// Activate the autocomplete with an initial query (text after `/`).
    pub fn activate(&mut self, initial_query: &str) {
        self.active = true;
        self.query = initial_query.to_string();
        self.update_filter();
        self.selected = 0;
    }

    /// Deactivate the autocomplete.
    pub fn dismiss(&mut self) {
        self.active = false;
        self.query.clear();
        self.selected = 0;
    }

    /// Update the filtered list based on the current query.
    pub fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.commands.len()).collect();
        } else {
            let q = self.query.to_lowercase();
            self.filtered = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| {
                    cmd.trigger.to_lowercase().contains(&q)
                        || cmd.description.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selection
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    /// Handle a key event. Returns the result.
    pub fn handle_key(&mut self, key: KeyEvent) -> AutocompleteResult {
        match key.code {
            KeyCode::Esc => {
                self.dismiss();
                AutocompleteResult::Dismiss
            }
            KeyCode::Enter => {
                if let Some(&idx) = self.filtered.get(self.selected) {
                    let trigger = self.commands[idx].trigger.clone();
                    self.dismiss();
                    AutocompleteResult::Selected(trigger)
                } else {
                    self.dismiss();
                    AutocompleteResult::Dismiss
                }
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                AutocompleteResult::Continue
            }
            KeyCode::Down => {
                if !self.filtered.is_empty() && self.selected < self.filtered.len() - 1 {
                    self.selected += 1;
                }
                AutocompleteResult::Continue
            }
            KeyCode::Backspace => {
                if self.query.is_empty() {
                    self.dismiss();
                    AutocompleteResult::Dismiss
                } else {
                    self.query.pop();
                    self.update_filter();
                    AutocompleteResult::Continue
                }
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.update_filter();
                AutocompleteResult::Continue
            }
            _ => AutocompleteResult::Continue,
        }
    }

    /// Render the autocomplete dropdown as an overlay above the input area.
    /// `input_area` is the Rect of the input widget — the dropdown renders above it.
    pub fn render(&self, theme: &Theme, input_area: Rect, buf: &mut Buffer) {
        if !self.active || self.filtered.is_empty() {
            return;
        }

        let max_visible = 8.min(self.filtered.len());
        let dropdown_height = max_visible as u16 + 2; // +2 for border
        let dropdown_width = (input_area.width).min(60);

        // Position above the input area
        let y = input_area.y.saturating_sub(dropdown_height);
        let area = Rect::new(input_area.x, y, dropdown_width, dropdown_height);

        // Determine scroll window
        let scroll_start = if self.selected >= max_visible {
            self.selected + 1 - max_visible
        } else {
            0
        };
        let scroll_end = (scroll_start + max_visible).min(self.filtered.len());

        let mut items: Vec<Line> = Vec::new();
        for (vi, fi) in (scroll_start..scroll_end).enumerate() {
            let idx = self.filtered[fi];
            let cmd = &self.commands[idx];
            let is_selected = fi == self.selected;

            let style = if is_selected {
                Style::default().fg(Color::Black).bg(theme.accent)
            } else {
                Style::default().fg(theme.assistant_text)
            };

            let desc_style = if is_selected {
                Style::default().fg(Color::Black).bg(theme.accent)
            } else {
                Style::default().fg(theme.dim)
            };

            let trigger_width = 18;
            let trigger_str = format!("{:<width$}", cmd.trigger, width = trigger_width);
            let desc_str = &cmd.description;

            let line = Line::from(vec![
                Span::styled(trigger_str, style),
                Span::styled(desc_str.as_str(), desc_style),
            ]);
            items.push(line);
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.tool_border))
            .title("Commands");

        let para = Paragraph::new(items).block(block);
        // Clear the area first
        for cy in area.y..area.y + area.height {
            for cx in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((cx, cy)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default());
                }
            }
        }
        para.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn make_commands() -> Vec<CommandEntry> {
        vec![
            CommandEntry { trigger: "/help".into(), description: "Show help".into() },
            CommandEntry { trigger: "/clear".into(), description: "Clear conversation".into() },
            CommandEntry { trigger: "/plugin".into(), description: "Plugin browser".into() },
            CommandEntry { trigger: "/skill".into(), description: "Skill browser".into() },
            CommandEntry { trigger: "/theme".into(), description: "Switch theme".into() },
        ]
    }

    #[test]
    fn test_new_autocomplete() {
        let ac = Autocomplete::new(make_commands());
        assert!(!ac.active);
        assert_eq!(ac.filtered.len(), 5);
    }

    #[test]
    fn test_activate_and_dismiss() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        assert!(ac.active);
        assert_eq!(ac.filtered.len(), 5);

        ac.dismiss();
        assert!(!ac.active);
    }

    #[test]
    fn test_filtering() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("hel");
        assert_eq!(ac.filtered.len(), 1);
        assert_eq!(ac.commands[ac.filtered[0]].trigger, "/help");
    }

    #[test]
    fn test_typing_narrows_filter() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        assert_eq!(ac.filtered.len(), 5);

        ac.handle_key(key(KeyCode::Char('p')));
        // "p" matches /plugin
        assert!(ac.filtered.len() < 5);

        ac.handle_key(key(KeyCode::Char('l')));
        // "pl" narrows further
        let count = ac.filtered.len();
        assert!(count >= 1);
    }

    #[test]
    fn test_up_down_selection() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        assert_eq!(ac.selected, 0);

        ac.handle_key(key(KeyCode::Down));
        assert_eq!(ac.selected, 1);

        ac.handle_key(key(KeyCode::Down));
        assert_eq!(ac.selected, 2);

        ac.handle_key(key(KeyCode::Up));
        assert_eq!(ac.selected, 1);
    }

    #[test]
    fn test_enter_selects() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        let result = ac.handle_key(key(KeyCode::Enter));
        match result {
            AutocompleteResult::Selected(trigger) => {
                assert_eq!(trigger, "/help");
            }
            _ => panic!("Expected Selected"),
        }
        assert!(!ac.active);
    }

    #[test]
    fn test_esc_dismisses() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        let result = ac.handle_key(key(KeyCode::Esc));
        assert!(matches!(result, AutocompleteResult::Dismiss));
        assert!(!ac.active);
    }

    #[test]
    fn test_backspace_on_empty_dismisses() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        let result = ac.handle_key(key(KeyCode::Backspace));
        assert!(matches!(result, AutocompleteResult::Dismiss));
    }

    #[test]
    fn test_backspace_removes_char() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("he");
        ac.handle_key(key(KeyCode::Backspace));
        assert_eq!(ac.query, "h");
        assert!(ac.active);
    }

    #[test]
    fn test_render_no_panic() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        let area = Rect::new(0, 20, 80, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        ac.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_inactive_does_nothing() {
        let ac = Autocomplete::new(make_commands());
        let area = Rect::new(0, 20, 80, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        ac.render(&theme, area, &mut buf);
        // No panic
    }

    #[test]
    fn test_empty_commands() {
        let mut ac = Autocomplete::new(vec![]);
        ac.activate("");
        assert_eq!(ac.filtered.len(), 0);
        let result = ac.handle_key(key(KeyCode::Enter));
        assert!(matches!(result, AutocompleteResult::Dismiss));
    }

    #[test]
    fn test_selection_clamped_on_filter() {
        let mut ac = Autocomplete::new(make_commands());
        ac.activate("");
        // Select last item
        for _ in 0..4 {
            ac.handle_key(key(KeyCode::Down));
        }
        assert_eq!(ac.selected, 4);

        // Type a filter that reduces results
        ac.handle_key(key(KeyCode::Char('h')));
        // selected should be clamped
        assert!(ac.selected < ac.filtered.len());
    }
}
