use ratatui::prelude::*;
use ratatui::widgets::*;

/// A message in the TUI display
#[derive(Debug, Clone)]
pub enum DisplayMessage {
    User(String),
    Assistant(String),
    ToolCall {
        name: String,
        args_summary: String,
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

// ── Color palette (FolkTech brand — do NOT change) ──────────────────────────

const ACCENT: Color = Color::Cyan;

// ── Markdown parser ─────────────────────────────────────────────────────────

/// Parse markdown text into styled ratatui Lines.
///
/// Supported elements:
///  - Headers (# / ## / ###) → ACCENT + bold
///  - Fenced code blocks (```) → dark background
///  - Inline code (`...`) → dim background
///  - Bold (**text** or __text__) → bold
///  - Bullet lists (- or * at start) → indented with ACCENT bullet
///  - Plain text
pub fn parse_markdown_lines(text: &str) -> Vec<Line<'_>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        // ── fenced code blocks ──────────────────────────────────────
        if raw_line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            // Render the fence marker itself in dim style
            lines.push(Line::from(Span::styled(
                raw_line,
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                raw_line,
                Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40)),
            )));
            continue;
        }

        // ── headers ─────────────────────────────────────────────────
        if raw_line.starts_with("### ") {
            lines.push(Line::from(Span::styled(
                &raw_line[4..],
                Style::default().fg(ACCENT).bold(),
            )));
            continue;
        }
        if raw_line.starts_with("## ") {
            lines.push(Line::from(Span::styled(
                &raw_line[3..],
                Style::default().fg(ACCENT).bold(),
            )));
            continue;
        }
        if raw_line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                &raw_line[2..],
                Style::default().fg(ACCENT).bold(),
            )));
            continue;
        }

        // ── bullet lists ────────────────────────────────────────────
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let indent = raw_line.len() - trimmed.len();
            let body = &trimmed[2..];
            let mut spans = Vec::new();
            if indent > 0 {
                spans.push(Span::raw(" ".repeat(indent)));
            }
            spans.push(Span::styled("  ● ", Style::default().fg(ACCENT)));
            spans.extend(parse_inline_spans(body));
            lines.push(Line::from(spans));
            continue;
        }

        // ── plain / inline-styled text ──────────────────────────────
        let spans = parse_inline_spans(raw_line);
        lines.push(Line::from(spans));
    }

    lines
}

/// Parse inline markdown (bold, inline code) within a single line.
fn parse_inline_spans(text: &str) -> Vec<Span<'_>> {
    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Look for the next special marker
        if let Some(pos) = remaining.find('`') {
            // Text before the backtick
            if pos > 0 {
                let before = &remaining[..pos];
                spans.extend(parse_bold_spans(before));
            }
            remaining = &remaining[pos + 1..];
            // Find closing backtick
            if let Some(end) = remaining.find('`') {
                let code = &remaining[..end];
                spans.push(Span::styled(
                    code,
                    Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 50)),
                ));
                remaining = &remaining[end + 1..];
            } else {
                // No closing backtick — treat as literal
                spans.push(Span::raw("`"));
            }
        } else {
            spans.extend(parse_bold_spans(remaining));
            break;
        }
    }

    spans
}

/// Parse **bold** markers within text (no backticks expected here).
fn parse_bold_spans(text: &str) -> Vec<Span<'_>> {
    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if let Some(pos) = remaining.find("**") {
            if pos > 0 {
                spans.push(Span::raw(&remaining[..pos]));
            }
            remaining = &remaining[pos + 2..];
            if let Some(end) = remaining.find("**") {
                let bold = &remaining[..end];
                spans.push(Span::styled(bold, Style::default().bold()));
                remaining = &remaining[end + 2..];
            } else {
                spans.push(Span::raw("**"));
            }
        } else {
            spans.push(Span::raw(remaining));
            break;
        }
    }

    spans
}

// ── Public render functions ─────────────────────────────────────────────────

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

/// Render the message area (or splash screen when no user messages yet).
pub fn render_messages(
    messages: &[DisplayMessage],
    mode: &str,
    area: Rect,
    buf: &mut Buffer,
) {
    let has_user_msg = messages.iter().any(|m| matches!(m, DisplayMessage::User(_)));

    if !has_user_msg {
        render_splash(messages, mode, area, buf);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let area_width = area.width.max(1) as usize;

    for msg in messages {
        match msg {
            DisplayMessage::User(text) => {
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Green).bold()),
                    Span::raw(text.as_str()),
                ]));
            }
            DisplayMessage::Assistant(text) => {
                lines.extend(parse_markdown_lines(text));
            }
            DisplayMessage::ToolCall {
                name,
                args_summary,
                result,
                is_error,
            } => {
                let (icon, color) = if *is_error {
                    ("\u{2718}", Color::Red) // ✘
                } else {
                    ("\u{2714}", Color::Green) // ✔
                };

                // Header with icon, name, and args summary
                let header_text = if args_summary.is_empty() {
                    format!("{icon} {name}")
                } else {
                    format!("{icon} {name}  {args_summary}")
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("┌ {header_text} "),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        "─".repeat(40),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));

                let result_lines: Vec<&str> = result.lines().collect();
                let max_display = 20;
                let total = result_lines.len();
                for line in result_lines.iter().take(max_display) {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", Style::default().fg(color)),
                        Span::raw(*line),
                    ]));
                }
                if total > max_display {
                    lines.push(Line::from(Span::styled(
                        format!("│ [... {} more lines]", total - max_display),
                        Style::default().fg(Color::DarkGray).italic(),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    format!("└{}┘", "─".repeat(42)),
                    Style::default().fg(color),
                )));
            }
            DisplayMessage::RuleViolation { rule_name, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("⚠ Rule: ", Style::default().fg(Color::Yellow).bold()),
                    Span::styled(rule_name.as_str(), Style::default().fg(Color::Yellow)),
                    Span::raw(" — "),
                    Span::raw(reason.as_str()),
                ]));
            }
            DisplayMessage::PermissionBlocked { tool, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ BLOCKED: ", Style::default().fg(Color::Red).bold()),
                    Span::styled(tool.as_str(), Style::default().fg(Color::Red)),
                    Span::raw(" — "),
                    Span::raw(reason.as_str()),
                ]));
            }
            DisplayMessage::PermissionDenied { tool } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ Denied: ", Style::default().fg(Color::Yellow).bold()),
                    Span::styled(tool.as_str(), Style::default().fg(Color::Yellow)),
                    Span::raw(" — user declined permission"),
                ]));
            }
            DisplayMessage::System(text) => {
                lines.push(Line::from(Span::styled(
                    text.as_str(),
                    Style::default().fg(Color::DarkGray).italic(),
                )));
            }
        }
        lines.push(Line::from(""));
    }

    // Word-wrap-aware scroll: estimate how many visual rows each line occupies
    let total_visual_rows: usize = lines
        .iter()
        .map(|line| {
            let char_count: usize = line.spans.iter().map(|s| s.content.len()).sum();
            (char_count / area_width).max(1)
        })
        .sum();
    let visible_rows = area.height as usize;
    let scroll_offset = total_visual_rows.saturating_sub(visible_rows) as u16;

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    para.render(area, buf);
}

/// Render the splash screen shown before any user messages.
fn render_splash(
    messages: &[DisplayMessage],
    mode: &str,
    area: Rect,
    buf: &mut Buffer,
) {
    let mut lines: Vec<Line> = Vec::new();

    // System messages (startup info)
    for msg in messages {
        if let DisplayMessage::System(text) = msg {
            lines.push(Line::from(Span::styled(
                text.as_str(),
                Style::default().fg(Color::DarkGray).italic(),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Mode indicator
    let mode_color = match mode {
        "coding" => Color::Green,
        "chat" => Color::Blue,
        _ => Color::White,
    };
    lines.push(Line::from(vec![
        Span::raw("  Mode: "),
        Span::styled(
            mode,
            Style::default().fg(mode_color).bold(),
        ),
    ]));

    lines.push(Line::from(""));

    // Keyboard shortcuts
    lines.push(Line::from(Span::styled(
        "  Enter: submit | Shift+Enter: newline | Ctrl+C: cancel | Ctrl+D: quit | /help: commands",
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false });
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

/// Render the input area. Returns an optional (x, y) cursor position for the
/// terminal cursor so the caller can call `frame.set_cursor_position()`.
pub fn render_input(
    text: &str,
    cursor_col: usize,
    area: Rect,
    buf: &mut Buffer,
) -> Option<(u16, u16)> {
    let input_text = format!("> {text}");
    let para = Paragraph::new(input_text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    para.render(area, buf);

    // The block's TOP border occupies 1 row. Content starts at area.y + 1.
    // The "> " prefix occupies 2 columns.
    let cursor_x = area.x + 2 + cursor_col as u16;
    let cursor_y = area.y + 1;

    // Only return if cursor fits inside the area
    if cursor_x < area.x + area.width && cursor_y < area.y + area.height {
        Some((cursor_x, cursor_y))
    } else {
        None
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_headers() {
        let lines = parse_markdown_lines("# Title\n## Subtitle\n### Sub-sub");
        assert_eq!(lines.len(), 3);
        // All headers should be ACCENT + bold
        for line in &lines {
            assert_eq!(line.spans.len(), 1);
            let style = line.spans[0].style;
            assert!(style.add_modifier.contains(Modifier::BOLD));
            assert_eq!(style.fg, Some(ACCENT));
        }
        assert_eq!(lines[0].spans[0].content, "Title");
        assert_eq!(lines[1].spans[0].content, "Subtitle");
        assert_eq!(lines[2].spans[0].content, "Sub-sub");
    }

    #[test]
    fn markdown_code_block() {
        let input = "before\n```rust\nlet x = 1;\n```\nafter";
        let lines = parse_markdown_lines(input);
        // before, ```, let x = 1;, ```, after
        assert_eq!(lines.len(), 5);
        // The code line should have a dark background
        let code_line = &lines[2];
        assert_eq!(code_line.spans[0].content, "let x = 1;");
        assert_eq!(
            code_line.spans[0].style.bg,
            Some(Color::Rgb(30, 30, 40))
        );
    }

    #[test]
    fn markdown_inline_code() {
        let lines = parse_markdown_lines("Use `foo` here");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        // Should have: "Use ", styled "foo", " here"
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content, "foo");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(spans[1].style.bg, Some(Color::Rgb(40, 40, 50)));
    }

    #[test]
    fn markdown_bold() {
        let lines = parse_markdown_lines("Hello **world** today");
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content, "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn markdown_bullet_list() {
        let lines = parse_markdown_lines("- first\n* second");
        assert_eq!(lines.len(), 2);
        // Each bullet line should contain the ACCENT bullet character
        for line in &lines {
            let has_bullet = line
                .spans
                .iter()
                .any(|s| s.content.contains('●'));
            assert!(has_bullet, "Expected bullet character in line");
        }
    }

    #[test]
    fn markdown_empty_input() {
        let lines = parse_markdown_lines("");
        // str::lines() on "" yields no items, so parse returns empty vec
        assert_eq!(lines.len(), 0);
        // Verify no panic with empty input in render context
        let messages = vec![DisplayMessage::Assistant(String::new())];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_messages(&messages, "coding", area, &mut buf);
    }

    #[test]
    fn markdown_extremely_long_message() {
        // Should not panic
        let long = "a".repeat(100_000);
        let lines = parse_markdown_lines(&long);
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_empty_messages_no_panic() {
        let messages: Vec<DisplayMessage> = Vec::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_messages(&messages, "coding", area, &mut buf);
        // Just verifying no panic
    }

    #[test]
    fn render_messages_no_panic() {
        let messages = vec![
            DisplayMessage::User("hello".to_string()),
            DisplayMessage::Assistant("**bold** and `code`\n# header\n- bullet".to_string()),
            DisplayMessage::ToolCall {
                name: "bash".to_string(),
                args_summary: "ls -la".to_string(),
                result: "file1\nfile2".to_string(),
                is_error: false,
            },
            DisplayMessage::ToolCall {
                name: "write".to_string(),
                args_summary: "".to_string(),
                result: "error".to_string(),
                is_error: true,
            },
            DisplayMessage::System("sys msg".to_string()),
        ];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_messages(&messages, "coding", area, &mut buf);
    }

    #[test]
    fn tool_call_truncation_indicator() {
        // Generate a result with > 20 lines
        let long_result: String = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![
            DisplayMessage::User("go".to_string()),
            DisplayMessage::ToolCall {
                name: "bash".to_string(),
                args_summary: "cmd".to_string(),
                result: long_result,
                is_error: false,
            },
        ];
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_messages(&messages, "coding", area, &mut buf);
        // Check the buffer contains the truncation text
        let mut content = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                content.push_str(buf.cell((x, y)).unwrap().symbol());
            }
        }
        assert!(
            content.contains("30 more lines"),
            "Expected truncation indicator in rendered output"
        );
    }

    #[test]
    fn render_input_returns_cursor_position() {
        let area = Rect::new(0, 0, 80, 3);
        let mut buf = Buffer::empty(area);
        let pos = render_input("hello", 5, area, &mut buf);
        // cursor_x = 0 + 2 + 5 = 7, cursor_y = 0 + 1 = 1
        assert_eq!(pos, Some((7, 1)));
    }

    #[test]
    fn render_input_cursor_out_of_bounds() {
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        let pos = render_input("hello", 100, area, &mut buf);
        assert_eq!(pos, None);
    }
}
