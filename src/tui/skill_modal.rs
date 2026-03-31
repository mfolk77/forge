use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;

use super::modal::{Modal, ModalAction};
use super::render::Theme;

/// A single skill entry displayed in the skill browser.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub source: String,
    pub description: String,
    pub content: String,
    pub token_estimate: usize,
}

/// Interactive skill browser modal.
pub struct SkillModal {
    pub skills: Vec<SkillEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
}

impl SkillModal {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self {
            skills,
            selected_index: 0,
            scroll_offset: 0,
        }
    }

    fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn move_down(&mut self) {
        if !self.skills.is_empty() && self.selected_index < self.skills.len() - 1 {
            self.selected_index += 1;
        }
    }
}

impl Modal for SkillModal {
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

        if area.height < 4 || area.width < 20 {
            return;
        }

        let footer_y = area.y + area.height - 1;
        let content_start = area.y + 1;
        let content_height = footer_y.saturating_sub(content_start) as usize;

        // ── Header ─────────────────────────────────────────────────────
        let header = Line::from(vec![
            Span::styled("  Skills", Style::default().fg(theme.accent).bold()),
            Span::styled(
                format!("  ({} available)", self.skills.len()),
                Style::default().fg(theme.dim),
            ),
        ]);
        buf.set_line(area.x, area.y, &header, area.width);

        // ── Group skills by source ─────────────────────────────────────
        // Collect unique sources in order
        let mut sources_seen: Vec<String> = Vec::new();
        for skill in &self.skills {
            if !sources_seen.contains(&skill.source) {
                sources_seen.push(skill.source.clone());
            }
        }

        // Build a flat list of renderable rows: headers + skill entries
        // Each item tracks whether it's a header or a skill (with global index)
        struct Row {
            text: String,
            is_header: bool,
            skill_index: Option<usize>,
        }

        let mut rows: Vec<Row> = Vec::new();
        let mut global_idx = 0;

        for source in &sources_seen {
            let header_text = if source == "builtin" {
                "  Builtin skills".to_string()
            } else {
                format!("  Plugin skills ({})", source)
            };
            rows.push(Row {
                text: header_text,
                is_header: true,
                skill_index: None,
            });

            for skill in &self.skills {
                if skill.source == *source {
                    let text = format!(
                        "    {} \u{00B7} {} \u{00B7} ~{} tokens   {}",
                        skill.name, skill.source, skill.token_estimate, skill.description
                    );
                    rows.push(Row {
                        text,
                        is_header: false,
                        skill_index: Some(global_idx),
                    });
                    global_idx += 1;
                }
            }
        }

        // ── Render rows ────────────────────────────────────────────────
        // Find which row contains selected_index for scroll adjustment
        let selected_row = rows
            .iter()
            .position(|r| r.skill_index == Some(self.selected_index))
            .unwrap_or(0);

        let scroll = if selected_row >= self.scroll_offset + content_height {
            selected_row + 1 - content_height
        } else if selected_row < self.scroll_offset {
            selected_row
        } else {
            self.scroll_offset
        };

        let end = (scroll + content_height).min(rows.len());
        for (i, row_idx) in (scroll..end).enumerate() {
            let row = &rows[row_idx];
            let y = content_start + i as u16;

            if row.is_header {
                let line = Line::from(Span::styled(
                    &row.text,
                    Style::default().fg(theme.accent).bold(),
                ));
                buf.set_line(area.x, y, &line, area.width);
            } else {
                let is_selected = row.skill_index == Some(self.selected_index);
                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(theme.accent)
                } else {
                    Style::default().fg(theme.assistant_text)
                };
                let line = Line::from(Span::styled(&row.text, style));
                buf.set_line(area.x, y, &line, area.width);
            }
        }

        // ── Footer ─────────────────────────────────────────────────────
        let footer = Line::from(Span::styled(
            "  \u{2191}\u{2193}: navigate \u{00B7} Enter: activate \u{00B7} Esc: close",
            Style::default().fg(theme.dim),
        ));
        buf.set_line(area.x, footer_y, &footer, area.width);
    }

    fn handle_key(&mut self, key: KeyEvent) -> ModalAction {
        match key.code {
            KeyCode::Esc => ModalAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                ModalAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                ModalAction::Continue
            }
            KeyCode::Enter => {
                if let Some(skill) = self.skills.get(self.selected_index) {
                    ModalAction::ActivateSkill {
                        name: skill.name.clone(),
                        content: skill.content.clone(),
                    }
                } else {
                    ModalAction::Continue
                }
            }
            _ => ModalAction::Continue,
        }
    }

    fn input_hint(&self) -> &str {
        "Up/Down: navigate, Enter: activate, Esc: close"
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

    fn make_skills() -> Vec<SkillEntry> {
        vec![
            SkillEntry {
                name: "commit".into(),
                source: "builtin".into(),
                description: "Commit workflow".into(),
                content: "# Commit\nDo the thing".into(),
                token_estimate: 5,
            },
            SkillEntry {
                name: "review".into(),
                source: "builtin".into(),
                description: "Code review".into(),
                content: "# Review\nReview the code".into(),
                token_estimate: 6,
            },
            SkillEntry {
                name: "deploy".into(),
                source: "ops-plugin".into(),
                description: "Deploy workflow".into(),
                content: "# Deploy\nDeploy instructions".into(),
                token_estimate: 7,
            },
        ]
    }

    #[test]
    fn test_new_modal() {
        let modal = SkillModal::new(make_skills());
        assert_eq!(modal.selected_index, 0);
        assert_eq!(modal.skills.len(), 3);
    }

    #[test]
    fn test_navigation() {
        let mut modal = SkillModal::new(make_skills());
        assert_eq!(modal.selected_index, 0);

        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_index, 1);

        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_index, 2);

        // Can't go past end
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_index, 2);

        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected_index, 1);
    }

    #[test]
    fn test_enter_activates_skill() {
        let mut modal = SkillModal::new(make_skills());
        let action = modal.handle_key(key(KeyCode::Enter));
        match action {
            ModalAction::ActivateSkill { name, content } => {
                assert_eq!(name, "commit");
                assert_eq!(content, "# Commit\nDo the thing");
            }
            _ => panic!("Expected ActivateSkill"),
        }
    }

    #[test]
    fn test_esc_closes() {
        let mut modal = SkillModal::new(make_skills());
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ModalAction::Close));
    }

    #[test]
    fn test_render_no_panic() {
        let modal = SkillModal::new(make_skills());
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_empty_skills() {
        let modal = SkillModal::new(vec![]);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_tiny_area() {
        let modal = SkillModal::new(make_skills());
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_enter_on_empty_does_nothing() {
        let mut modal = SkillModal::new(vec![]);
        let action = modal.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, ModalAction::Continue));
    }

    #[test]
    fn test_j_k_navigation() {
        let mut modal = SkillModal::new(make_skills());
        modal.handle_key(key(KeyCode::Char('j')));
        assert_eq!(modal.selected_index, 1);
        modal.handle_key(key(KeyCode::Char('k')));
        assert_eq!(modal.selected_index, 0);
    }
}
