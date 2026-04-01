use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::modal::{Modal, ModalAction};
use super::render::Theme;

/// Interactive theme picker — shows all themes with live preview on selection.
pub struct ThemeModal {
    themes: Vec<ThemeEntry>,
    selected: usize,
}

struct ThemeEntry {
    name: &'static str,
    label: &'static str,
    description: &'static str,
    preview_colors: [Color; 4], // accent, user, assistant, system
}

impl ThemeModal {
    pub fn new(current_theme: &str) -> Self {
        let themes = vec![
            ThemeEntry {
                name: "dark",
                label: "Dark",
                description: "Default dark theme — cyan accent, warm grays",
                preview_colors: [
                    Color::Cyan,
                    Color::Rgb(180, 200, 255),
                    Color::Rgb(220, 220, 230),
                    Color::Rgb(200, 200, 210),
                ],
            },
            ThemeEntry {
                name: "light",
                label: "Light",
                description: "Light background — teal accent, dark text",
                preview_colors: [
                    Color::Rgb(0, 120, 140),
                    Color::Rgb(0, 60, 160),
                    Color::Rgb(30, 30, 35),
                    Color::Rgb(80, 80, 90),
                ],
            },
            ThemeEntry {
                name: "high-contrast",
                label: "High Contrast",
                description: "Maximum readability — yellow/white on black",
                preview_colors: [
                    Color::Yellow,
                    Color::White,
                    Color::White,
                    Color::White,
                ],
            },
            ThemeEntry {
                name: "solarized",
                label: "Solarized",
                description: "Classic Solarized dark — blue accent, muted tones",
                preview_colors: [
                    Color::Rgb(38, 139, 210),
                    Color::Rgb(147, 161, 161),
                    Color::Rgb(131, 148, 150),
                    Color::Rgb(101, 123, 131),
                ],
            },
            ThemeEntry {
                name: "dracula",
                label: "Dracula",
                description: "Dracula palette — purple accent, green input",
                preview_colors: [
                    Color::Rgb(189, 147, 249),
                    Color::Rgb(80, 250, 123),
                    Color::Rgb(248, 248, 242),
                    Color::Rgb(189, 193, 205),
                ],
            },
        ];

        let current_lower = current_theme.to_lowercase();
        let selected = themes
            .iter()
            .position(|t| t.name == current_lower)
            .unwrap_or(0);

        Self { themes, selected }
    }
}

impl Modal for ThemeModal {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        // Clear the area
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default());
                }
            }
        }

        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(Span::styled(
            "  Select Theme",
            Style::default().fg(theme.accent).bold(),
        )));
        lines.push(Line::from(""));

        for (i, entry) in self.themes.iter().enumerate() {
            let is_selected = i == self.selected;

            // Selection indicator
            let indicator = if is_selected { "▸ " } else { "  " };

            let label_style;
            let desc_style;
            let ind_style;
            if is_selected {
                ind_style = Style::default().fg(theme.accent).bold();
                label_style = Style::default().fg(theme.accent).bold();
                desc_style = Style::default().fg(theme.assistant_text);
            } else {
                ind_style = Style::default().fg(theme.dim);
                label_style = Style::default().fg(theme.system_text);
                desc_style = Style::default().fg(theme.dim);
            }

            lines.push(Line::from(vec![
                Span::styled(indicator, ind_style),
                Span::styled(format!("{:<16}", entry.label), label_style),
                Span::styled("█", Style::default().fg(entry.preview_colors[0])),
                Span::styled("█", Style::default().fg(entry.preview_colors[1])),
                Span::styled("█", Style::default().fg(entry.preview_colors[2])),
                Span::styled("█", Style::default().fg(entry.preview_colors[3])),
                Span::raw("  "),
                Span::styled(entry.description.to_string(), desc_style),
            ]));
            lines.push(Line::from(""));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ↑↓: navigate · Enter: apply · Esc: cancel",
            Style::default().fg(theme.dim),
        )));

        // Bottom-align
        let content_height = lines.len();
        let available = area.height as usize;
        if content_height < available {
            let pad = available - content_height;
            let mut padded = vec![Line::from(""); pad];
            padded.append(&mut lines);
            lines = padded;
        }

        let para = Paragraph::new(lines);
        para.render(area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> ModalAction {
        match key.code {
            KeyCode::Esc => ModalAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                } else {
                    self.selected = self.themes.len() - 1;
                }
                ModalAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1) % self.themes.len();
                ModalAction::Continue
            }
            KeyCode::Enter => {
                let name = self.themes[self.selected].name.to_string();
                ModalAction::SelectTheme(name)
            }
            KeyCode::Char('1') => ModalAction::SelectTheme("dark".into()),
            KeyCode::Char('2') => ModalAction::SelectTheme("light".into()),
            KeyCode::Char('3') => ModalAction::SelectTheme("high-contrast".into()),
            KeyCode::Char('4') => ModalAction::SelectTheme("solarized".into()),
            KeyCode::Char('5') => ModalAction::SelectTheme("dracula".into()),
            _ => ModalAction::Continue,
        }
    }

    fn input_hint(&self) -> &str {
        "↑↓ navigate · Enter apply · Esc cancel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn test_new_selects_current_theme() {
        let modal = ThemeModal::new("dracula");
        assert_eq!(modal.selected, 4);
    }

    #[test]
    fn test_new_defaults_to_first() {
        let modal = ThemeModal::new("unknown");
        assert_eq!(modal.selected, 0);
    }

    #[test]
    fn test_navigate_down_wraps() {
        let mut modal = ThemeModal::new("dark");
        for _ in 0..5 {
            modal.handle_key(key(KeyCode::Down));
        }
        assert_eq!(modal.selected, 0); // wrapped around
    }

    #[test]
    fn test_navigate_up_wraps() {
        let mut modal = ThemeModal::new("dark");
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected, 4); // wrapped to last
    }

    #[test]
    fn test_enter_selects() {
        let mut modal = ThemeModal::new("dark");
        modal.handle_key(key(KeyCode::Down)); // solarized? no, index 1 = light
        let action = modal.handle_key(key(KeyCode::Enter));
        match action {
            ModalAction::SelectTheme(name) => assert_eq!(name, "light"),
            _ => panic!("Expected SelectTheme"),
        }
    }

    #[test]
    fn test_number_keys_select() {
        let mut modal = ThemeModal::new("dark");
        let action = modal.handle_key(key(KeyCode::Char('5')));
        match action {
            ModalAction::SelectTheme(name) => assert_eq!(name, "dracula"),
            _ => panic!("Expected SelectTheme"),
        }
    }

    #[test]
    fn test_esc_closes() {
        let mut modal = ThemeModal::new("dark");
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ModalAction::Close));
    }
}
