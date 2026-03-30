use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::config::{ThemeConfig, ThemePreset};

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

// ── Theme system ────────────────────────────────────────────────────────────

/// Resolved color palette used by all render functions.
#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub user_input: Color,
    pub assistant_text: Color,
    pub system_text: Color,
    pub error: Color,
    pub warning: Color,
    pub tool_border: Color,
    pub status_bar_fg: Color,
    pub status_bar_bg: Color,
    pub status_line_fg: Color,
    pub status_line_bg: Color,
    pub dim: Color,
    pub code_bg: Color,
}

impl Theme {
    /// Build a resolved Theme from the user's config.
    /// Starts from the preset, then applies any per-color overrides.
    pub fn from_config(config: &ThemeConfig) -> Self {
        let mut theme = Self::preset(&config.preset);

        // Apply overrides
        if let Some(c) = config.accent.as_deref().and_then(parse_color) { theme.accent = c; }
        if let Some(c) = config.user_input.as_deref().and_then(parse_color) { theme.user_input = c; }
        if let Some(c) = config.assistant_text.as_deref().and_then(parse_color) { theme.assistant_text = c; }
        if let Some(c) = config.system_text.as_deref().and_then(parse_color) { theme.system_text = c; }
        if let Some(c) = config.error.as_deref().and_then(parse_color) { theme.error = c; }
        if let Some(c) = config.warning.as_deref().and_then(parse_color) { theme.warning = c; }
        if let Some(c) = config.tool_border.as_deref().and_then(parse_color) { theme.tool_border = c; }
        if let Some(c) = config.status_bar_fg.as_deref().and_then(parse_color) { theme.status_bar_fg = c; }
        if let Some(c) = config.status_bar_bg.as_deref().and_then(parse_color) { theme.status_bar_bg = c; }
        if let Some(c) = config.status_line_fg.as_deref().and_then(parse_color) { theme.status_line_fg = c; }
        if let Some(c) = config.status_line_bg.as_deref().and_then(parse_color) { theme.status_line_bg = c; }

        theme
    }

    fn preset(preset: &ThemePreset) -> Self {
        match preset {
            ThemePreset::Dark => Self {
                accent:         Color::Cyan,
                user_input:     Color::Rgb(180, 200, 255),
                assistant_text: Color::Rgb(220, 220, 230),
                system_text:    Color::Rgb(200, 200, 210),
                error:          Color::Rgb(220, 80, 80),
                warning:        Color::Rgb(220, 180, 60),
                tool_border:    Color::Rgb(100, 160, 220),
                status_bar_fg:  Color::Black,
                status_bar_bg:  Color::Cyan,
                status_line_fg: Color::White,
                status_line_bg: Color::Rgb(50, 50, 55),
                dim:            Color::Rgb(100, 100, 110),
                code_bg:        Color::Rgb(30, 30, 40),
            },
            ThemePreset::Light => Self {
                accent:         Color::Rgb(0, 120, 140),
                user_input:     Color::Rgb(0, 60, 160),
                assistant_text: Color::Rgb(30, 30, 35),
                system_text:    Color::Rgb(80, 80, 90),
                error:          Color::Rgb(180, 30, 30),
                warning:        Color::Rgb(160, 120, 0),
                tool_border:    Color::Rgb(50, 120, 180),
                status_bar_fg:  Color::White,
                status_bar_bg:  Color::Rgb(0, 120, 140),
                status_line_fg: Color::Rgb(30, 30, 35),
                status_line_bg: Color::Rgb(220, 220, 225),
                dim:            Color::Rgb(150, 150, 160),
                code_bg:        Color::Rgb(240, 240, 245),
            },
            ThemePreset::HighContrast => Self {
                accent:         Color::Yellow,
                user_input:     Color::White,
                assistant_text: Color::White,
                system_text:    Color::White,
                error:          Color::Red,
                warning:        Color::Yellow,
                tool_border:    Color::White,
                status_bar_fg:  Color::Black,
                status_bar_bg:  Color::Yellow,
                status_line_fg: Color::Black,
                status_line_bg: Color::White,
                dim:            Color::Rgb(180, 180, 180),
                code_bg:        Color::Rgb(40, 40, 40),
            },
            ThemePreset::Solarized => Self {
                accent:         Color::Rgb(38, 139, 210),   // blue
                user_input:     Color::Rgb(147, 161, 161),  // base1
                assistant_text: Color::Rgb(131, 148, 150),  // base0
                system_text:    Color::Rgb(101, 123, 131),  // base00
                error:          Color::Rgb(220, 50, 47),    // red
                warning:        Color::Rgb(181, 137, 0),    // yellow
                tool_border:    Color::Rgb(42, 161, 152),   // cyan
                status_bar_fg:  Color::Rgb(253, 246, 227),  // base3
                status_bar_bg:  Color::Rgb(38, 139, 210),   // blue
                status_line_fg: Color::Rgb(147, 161, 161),  // base1
                status_line_bg: Color::Rgb(0, 43, 54),      // base03
                dim:            Color::Rgb(88, 110, 117),   // base01
                code_bg:        Color::Rgb(7, 54, 66),      // base02
            },
            ThemePreset::Dracula => Self {
                accent:         Color::Rgb(189, 147, 249),  // purple
                user_input:     Color::Rgb(80, 250, 123),   // green
                assistant_text: Color::Rgb(248, 248, 242),  // foreground
                system_text:    Color::Rgb(189, 193, 205),  // lighter comment
                error:          Color::Rgb(255, 85, 85),    // red
                warning:        Color::Rgb(241, 250, 140),  // yellow
                tool_border:    Color::Rgb(139, 233, 253),  // cyan
                status_bar_fg:  Color::Rgb(40, 42, 54),     // background
                status_bar_bg:  Color::Rgb(189, 147, 249),  // purple
                status_line_fg: Color::Rgb(248, 248, 242),  // foreground
                status_line_bg: Color::Rgb(68, 71, 90),     // current line
                dim:            Color::Rgb(98, 114, 164),   // comment
                code_bg:        Color::Rgb(40, 42, 54),     // background
            },
        }
    }
}

/// Parse a color string: "#RRGGBB" hex or named colors.
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.starts_with('#') && s.len() == 7 {
        let r = u8::from_str_radix(&s[1..3], 16).ok()?;
        let g = u8::from_str_radix(&s[3..5], 16).ok()?;
        let b = u8::from_str_radix(&s[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    match s.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

/// Default accent color used by the markdown parser (which doesn't receive a Theme).
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
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
) {
    let status = format!(" forge | {model_name} ({backend}) | {cwd}");
    let bar = Paragraph::new(status)
        .style(Style::default().fg(theme.status_bar_fg).bg(theme.status_bar_bg));
    bar.render(area, buf);
}

/// Render the message area (or splash screen when no user messages yet).
pub fn render_messages(
    messages: &[DisplayMessage],
    mode: &str,
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
) {
    let has_user_msg = messages.iter().any(|m| matches!(m, DisplayMessage::User(_)));

    if !has_user_msg {
        render_splash(messages, mode, theme, area, buf);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let area_width = area.width.max(1) as usize;

    for msg in messages {
        match msg {
            DisplayMessage::User(text) => {
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(theme.user_input).bold()),
                    Span::styled(text.as_str(), Style::default().fg(theme.user_input)),
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
                    ("\u{2718}", theme.error)
                } else {
                    ("\u{2714}", theme.tool_border)
                };

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
                        Style::default().fg(theme.dim),
                    ),
                ]));

                let result_lines: Vec<&str> = result.lines().collect();
                let max_display = 20;
                let total = result_lines.len();
                for line in result_lines.iter().take(max_display) {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", Style::default().fg(color)),
                        Span::styled(*line, Style::default().fg(theme.assistant_text)),
                    ]));
                }
                if total > max_display {
                    lines.push(Line::from(Span::styled(
                        format!("│ [... {} more lines]", total - max_display),
                        Style::default().fg(theme.dim).italic(),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    format!("└{}┘", "─".repeat(42)),
                    Style::default().fg(color),
                )));
            }
            DisplayMessage::RuleViolation { rule_name, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("⚠ Rule: ", Style::default().fg(theme.warning).bold()),
                    Span::styled(rule_name.as_str(), Style::default().fg(theme.warning)),
                    Span::raw(" — "),
                    Span::styled(reason.as_str(), Style::default().fg(theme.assistant_text)),
                ]));
            }
            DisplayMessage::PermissionBlocked { tool, reason } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ BLOCKED: ", Style::default().fg(theme.error).bold()),
                    Span::styled(tool.as_str(), Style::default().fg(theme.error)),
                    Span::raw(" — "),
                    Span::styled(reason.as_str(), Style::default().fg(theme.assistant_text)),
                ]));
            }
            DisplayMessage::PermissionDenied { tool } => {
                lines.push(Line::from(vec![
                    Span::styled("✖ Denied: ", Style::default().fg(theme.warning).bold()),
                    Span::styled(tool.as_str(), Style::default().fg(theme.warning)),
                    Span::styled(" — user declined", Style::default().fg(theme.dim)),
                ]));
            }
            DisplayMessage::System(text) => {
                for line in text.lines() {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(theme.system_text),
                    )));
                }
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
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
) {
    let mut lines: Vec<Line> = Vec::new();

    // System messages (startup info)
    for msg in messages {
        if let DisplayMessage::System(text) = msg {
            for line in text.lines() {
                lines.push(Line::from(Span::styled(
                    line,
                    Style::default().fg(theme.system_text),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Mode indicator
    let mode_color = match mode {
        "coding" => Color::Green,
        "chat" => Color::Blue,
        _ => theme.assistant_text,
    };
    lines.push(Line::from(vec![
        Span::styled("  Mode: ", Style::default().fg(theme.system_text)),
        Span::styled(
            mode,
            Style::default().fg(mode_color).bold(),
        ),
    ]));

    lines.push(Line::from(""));

    // Keyboard shortcuts
    lines.push(Line::from(Span::styled(
        "  Enter: submit | Shift+Enter: newline | Ctrl+C: cancel | Ctrl+D: quit | /help: commands",
        Style::default().fg(theme.dim),
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
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
) {
    let status = format!(
        " tokens: {tokens}/{max_tokens} | rules: {rules_count} active"
    );
    let bar = Paragraph::new(status)
        .style(Style::default().fg(theme.status_line_fg).bg(theme.status_line_bg));
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
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
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
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
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
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
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
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
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

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_render_extremely_long_messages() {
        // P0 security red test
        // 100K character messages must not panic during render
        let long_text = "x".repeat(100_000);
        let messages = vec![
            DisplayMessage::User("trigger".to_string()),
            DisplayMessage::Assistant(long_text),
        ];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
        // No panic = pass
    }

    #[test]
    fn test_security_render_unicode_emoji() {
        // P0 security red test
        // Unicode and emoji in messages must not panic
        let messages = vec![
            DisplayMessage::User("trigger".to_string()),
            DisplayMessage::Assistant(
                "Hello \u{1F600}\u{1F4A9} \u{1F680} \u{2764}\u{FE0F} \u{1F1FA}\u{1F1F8} \u{0000} \u{FFFF}".to_string()
            ),
        ];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
    }

    #[test]
    fn test_security_render_ansi_escape_codes() {
        // P0 security red test
        // ANSI escape codes in message text must not break TUI rendering
        let ansi_text = "\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m \x1b[38;2;255;0;0mrgb\x1b[0m \x1b[2J\x1b[H";
        let messages = vec![
            DisplayMessage::User("trigger".to_string()),
            DisplayMessage::Assistant(ansi_text.to_string()),
        ];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
    }

    #[test]
    fn test_security_tool_call_empty_fields() {
        // P0 security red test
        // DisplayMessage::ToolCall with all empty fields must not panic
        let messages = vec![
            DisplayMessage::User("trigger".to_string()),
            DisplayMessage::ToolCall {
                name: String::new(),
                args_summary: String::new(),
                result: String::new(),
                is_error: false,
            },
            DisplayMessage::ToolCall {
                name: String::new(),
                args_summary: String::new(),
                result: String::new(),
                is_error: true,
            },
        ];
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        render_messages(&messages, "coding", &theme, area, &mut buf);
    }

    #[test]
    fn test_security_malicious_markdown() {
        // P0 security red test
        // Malicious markdown patterns must not panic or hang
        let long_bold = format!("**{}**", "a".repeat(50_000));
        let many_headers = "# \n".repeat(1000);
        let deep_bullets = "    - nested\n".repeat(500);
        let test_cases = vec![
            // Unclosed code block
            "```\ncode without closing fence",
            // Deeply nested headers (not real markdown but shouldn't crash)
            "# # # # # # # # # # # # deeply nested",
            // Many unclosed backticks
            "` ` ` ` ` ` ` ` ` ` ` ` ` ` ` `",
            // Unclosed bold markers
            "**unclosed bold **another** more **",
            // Mixed unclosed markers
            "**bold `code **still bold`",
            // Very long single line with markers
            long_bold.as_str(),
            // Many empty headers
            many_headers.as_str(),
            // Code block with special characters
            "```\n\x1b[31m\x00\n```",
            // Bullet list with deeply indented items
            deep_bullets.as_str(),
        ];

        for (i, input) in test_cases.iter().enumerate() {
            let lines = parse_markdown_lines(input);
            // Just verify no panic — the output format doesn't matter
            assert!(
                lines.len() > 0 || input.is_empty(),
                "Test case {i} produced no output for non-empty input"
            );
        }
    }
}
