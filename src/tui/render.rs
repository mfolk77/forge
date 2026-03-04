use ratatui::prelude::*;
use ratatui::widgets::*;

/// A message in the TUI display
#[derive(Debug, Clone)]
pub enum DisplayMessage {
    User(String),
    Assistant(String),
    ToolCall {
        name: String,
        result: String,
        is_error: bool,
    },
    RuleViolation {
        rule_name: String,
        reason: String,
    },
    PermissionBlocked {
        tool: String,
        reason: String,
    },
    PermissionDenied {
        tool: String,
    },
    System(String),
}

/// Render the status bar at the top
pub fn render_status_bar(
    model_name: &str,
    backend: &str,
    cwd: &str,
    area: Rect,
    buf: &mut Buffer,
) {
    let status = format!(" ftai | {model_name} ({backend}) | {cwd}");
    let bar = Paragraph::new(status)
        .style(Style::default().fg(Color::Black).bg(Color::Cyan));
    bar.render(area, buf);
}

/// Render the message area
pub fn render_messages(messages: &[DisplayMessage], area: Rect, buf: &mut Buffer) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in messages {
        match msg {
            DisplayMessage::User(text) => {
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Green).bold()),
                    Span::raw(text),
                ]));
            }
            DisplayMessage::Assistant(text) => {
                for line in text.lines() {
                    lines.push(Line::from(Span::raw(line)));
                }
            }
            DisplayMessage::ToolCall {
                name,
                result,
                is_error,
            } => {
                let color = if *is_error { Color::Red } else { Color::Blue };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("┌ {name} "),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        "─".repeat(40),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                for line in result.lines().take(20) {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", Style::default().fg(color)),
                        Span::raw(line),
                    ]));
                }
                lines.push(Line::from(Span::styled(
                    format!("└{}┘", "─".repeat(42)),
                    Style::default().fg(color),
                )));
            }
            DisplayMessage::RuleViolation { rule_name, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("⚠ Rule: ", Style::default().fg(Color::Yellow).bold()),
                    Span::styled(rule_name, Style::default().fg(Color::Yellow)),
                    Span::raw(" — "),
                    Span::raw(reason),
                ]));
            }
            DisplayMessage::PermissionBlocked { tool, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ BLOCKED: ", Style::default().fg(Color::Red).bold()),
                    Span::styled(tool, Style::default().fg(Color::Red)),
                    Span::raw(" — "),
                    Span::raw(reason),
                ]));
            }
            DisplayMessage::PermissionDenied { tool } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ Denied: ", Style::default().fg(Color::Yellow).bold()),
                    Span::styled(tool, Style::default().fg(Color::Yellow)),
                    Span::raw(" — user declined permission"),
                ]));
            }
            DisplayMessage::System(text) => {
                lines.push(Line::from(Span::styled(
                    text,
                    Style::default().fg(Color::DarkGray).italic(),
                )));
            }
        }
        lines.push(Line::from(""));
    }

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((0, 0));
    para.render(area, buf);
}

/// Render the bottom status line
pub fn render_status_line(
    tokens: usize,
    max_tokens: usize,
    rules_count: usize,
    area: Rect,
    buf: &mut Buffer,
) {
    let status = format!(
        " tokens: {tokens}/{max_tokens} | rules: {rules_count} active"
    );
    let bar = Paragraph::new(status)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    bar.render(area, buf);
}

/// Render the input area
pub fn render_input(text: &str, cursor_col: usize, area: Rect, buf: &mut Buffer) {
    let input_text = format!("> {text}");
    let para = Paragraph::new(input_text)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray)));
    para.render(area, buf);

    // Cursor position would be set by the terminal backend
    let _ = cursor_col; // Used by the app layer to set cursor
}
