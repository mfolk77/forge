use anyhow::Result;
use crate::backend::types::ToolCall;
use crate::config::ToolCallingMode;
use regex::Regex;

/// Parses tool calls from model output (for models without native function calling)
pub struct ToolCallParser {
    mode: ToolCallingMode,
}

impl ToolCallParser {
    pub fn new(mode: ToolCallingMode) -> Self {
        Self { mode }
    }

    /// Parse tool calls from assistant text output
    /// Returns (remaining_text, tool_calls)
    pub fn parse(&self, text: &str) -> (String, Vec<ToolCall>) {
        match self.mode {
            ToolCallingMode::Native => {
                // Native mode — tool calls come from the API, not from text
                (text.to_string(), vec![])
            }
            ToolCallingMode::Prompted => self.parse_prompted(text),
            ToolCallingMode::Hybrid => {
                // Try prompted parsing first; if nothing found, return as-is
                let (remaining, calls) = self.parse_prompted(text);
                if calls.is_empty() {
                    (text.to_string(), vec![])
                } else {
                    (remaining, calls)
                }
            }
        }
    }

    /// Parse XML-style tool calls from text:
    /// <tool_call>
    /// {"name": "bash", "arguments": {"command": "ls"}}
    /// </tool_call>
    fn parse_prompted(&self, text: &str) -> (String, Vec<ToolCall>) {
        let re = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap();
        let mut calls = Vec::new();
        let mut remaining = text.to_string();
        let mut call_counter = 0;

        for cap in re.captures_iter(text) {
            let full_match = cap.get(0).unwrap().as_str();
            let json_str = cap.get(1).unwrap().as_str();

            if let Ok(parsed) = Self::parse_tool_json(json_str, &mut call_counter) {
                calls.push(parsed);
                remaining = remaining.replace(full_match, "");
            }
        }

        // Also try JSON block format:
        // ```json
        // {"tool": "bash", "arguments": {"command": "ls"}}
        // ```
        if calls.is_empty() {
            let json_re =
                Regex::new(r#"(?s)```(?:json)?\s*(\{[^`]*?"(?:tool|name)"[^`]*?\})\s*```"#)
                    .unwrap();
            for cap in json_re.captures_iter(text) {
                let full_match = cap.get(0).unwrap().as_str();
                let json_str = cap.get(1).unwrap().as_str();

                if let Ok(parsed) = Self::parse_tool_json(json_str, &mut call_counter) {
                    calls.push(parsed);
                    remaining = remaining.replace(full_match, "");
                }
            }
        }

        (remaining.trim().to_string(), calls)
    }

    fn parse_tool_json(json_str: &str, counter: &mut usize) -> Result<ToolCall> {
        let val: serde_json::Value = serde_json::from_str(json_str)?;

        let name = val
            .get("name")
            .or_else(|| val.get("tool"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?
            .to_string();

        let arguments = val
            .get("arguments")
            .or_else(|| val.get("params"))
            .or_else(|| val.get("parameters"))
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        *counter += 1;
        Ok(ToolCall {
            id: format!("tc_{counter}"),
            name,
            arguments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_mode_passthrough() {
        let parser = ToolCallParser::new(ToolCallingMode::Native);
        let (text, calls) = parser.parse("some text with no tool calls");
        assert_eq!(text, "some text with no tool calls");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_xml_tool_call() {
        let parser = ToolCallParser::new(ToolCallingMode::Prompted);
        let input = r#"Let me read that file.
<tool_call>
{"name": "file_read", "arguments": {"path": "/foo/bar.rs"}}
</tool_call>"#;

        let (remaining, calls) = parser.parse(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(calls[0].arguments["path"], "/foo/bar.rs");
        assert!(remaining.contains("Let me read that file."));
        assert!(!remaining.contains("tool_call"));
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let parser = ToolCallParser::new(ToolCallingMode::Prompted);
        let input = r#"I'll read both files.
<tool_call>
{"name": "file_read", "arguments": {"path": "/a.rs"}}
</tool_call>
<tool_call>
{"name": "file_read", "arguments": {"path": "/b.rs"}}
</tool_call>"#;

        let (_, calls) = parser.parse(input);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["path"], "/a.rs");
        assert_eq!(calls[1].arguments["path"], "/b.rs");
    }

    #[test]
    fn test_parse_json_block_format() {
        let parser = ToolCallParser::new(ToolCallingMode::Prompted);
        let input = r#"Running command:
```json
{"name": "bash", "arguments": {"command": "ls -la"}}
```"#;

        let (_, calls) = parser.parse(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn test_parse_tool_key_variant() {
        let parser = ToolCallParser::new(ToolCallingMode::Prompted);
        let input = r#"<tool_call>
{"tool": "grep", "params": {"pattern": "TODO", "path": "."}}
</tool_call>"#;

        let (_, calls) = parser.parse(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].arguments["pattern"], "TODO");
    }

    #[test]
    fn test_hybrid_mode_with_tool_calls() {
        let parser = ToolCallParser::new(ToolCallingMode::Hybrid);
        let input = r#"Let me check.
<tool_call>
{"name": "bash", "arguments": {"command": "pwd"}}
</tool_call>"#;

        let (_, calls) = parser.parse(input);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_hybrid_mode_no_tool_calls() {
        let parser = ToolCallParser::new(ToolCallingMode::Hybrid);
        let (text, calls) = parser.parse("Just a regular response.");
        assert_eq!(text, "Just a regular response.");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_invalid_json_ignored() {
        let parser = ToolCallParser::new(ToolCallingMode::Prompted);
        let input = r#"<tool_call>
{not valid json}
</tool_call>"#;

        let (_, calls) = parser.parse(input);
        assert!(calls.is_empty());
    }
}
