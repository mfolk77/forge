use std::collections::HashMap;

use crate::conversation::adapter::ParsedToolCall;

/// Events emitted by the streaming parser as tokens arrive.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Plain text that is not part of a tool call.
    Text(String),
    /// A `<tool_call>` opening tag was detected -- tool call is beginning.
    ToolCallStart,
    /// A complete tool call has been parsed and is ready for execution.
    ToolCallComplete(ParsedToolCall),
    /// Partial content inside an in-progress tool call (for display/logging).
    ToolCallPartial(String),
}

/// Tracks parse state across streaming token chunks.
///
/// Because tokens can arrive mid-tag (e.g. `<tool_` in one chunk and `call>` in
/// the next), the parser buffers partial content and only emits events once
/// boundaries are unambiguous.
#[derive(Debug)]
pub struct StreamingToolCallParser {
    /// Accumulated text that has not yet been fully classified.
    buffer: String,
    /// Whether we are currently inside a `<tool_call>` block.
    inside_tool_call: bool,
    /// Content accumulated inside the current tool call block.
    tool_call_buffer: String,
}

impl StreamingToolCallParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            inside_tool_call: false,
            tool_call_buffer: String::new(),
        }
    }

    /// Feed a new chunk of text and return any events that can be determined.
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        loop {
            if self.inside_tool_call {
                if let Some(end_pos) = self.buffer.find("</tool_call>") {
                    // Complete tool call block found.
                    let body = &self.buffer[..end_pos];
                    self.tool_call_buffer.push_str(body);

                    if let Some(parsed) = self.parse_tool_call_body(&self.tool_call_buffer.clone())
                    {
                        events.push(StreamEvent::ToolCallComplete(parsed));
                    }

                    // Advance past the closing tag.
                    let after = end_pos + "</tool_call>".len();
                    self.buffer = self.buffer[after..].to_string();
                    self.inside_tool_call = false;
                    self.tool_call_buffer.clear();
                    // Continue loop -- there may be more content or tool calls.
                } else {
                    // Still accumulating inside the tool call. Check for a possible
                    // partial closing tag at the end of the buffer (e.g. "</tool").
                    let safe_len = self.safe_emit_len("</tool_call>");
                    if safe_len > 0 {
                        let emittable = self.buffer[..safe_len].to_string();
                        self.tool_call_buffer.push_str(&emittable);
                        events.push(StreamEvent::ToolCallPartial(emittable));
                        self.buffer = self.buffer[safe_len..].to_string();
                    }
                    break;
                }
            } else {
                // Not inside a tool call -- look for an opening tag.
                if let Some(start_pos) = self.buffer.find("<tool_call>") {
                    // Emit any text before the tag.
                    if start_pos > 0 {
                        let text = self.buffer[..start_pos].to_string();
                        events.push(StreamEvent::Text(text));
                    }
                    events.push(StreamEvent::ToolCallStart);
                    let after = start_pos + "<tool_call>".len();
                    self.buffer = self.buffer[after..].to_string();
                    self.inside_tool_call = true;
                    self.tool_call_buffer.clear();
                    // Continue loop to handle the body.
                } else {
                    // No opening tag found. Emit text up to a potential partial tag.
                    let safe_len = self.safe_emit_len("<tool_call>");
                    if safe_len > 0 {
                        let text = self.buffer[..safe_len].to_string();
                        events.push(StreamEvent::Text(text));
                        self.buffer = self.buffer[safe_len..].to_string();
                    }
                    break;
                }
            }
        }

        events
    }

    /// Flush any remaining buffered content. Call this when the stream ends.
    pub fn flush(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if self.inside_tool_call {
            // Incomplete tool call -- emit whatever we have as partial.
            self.tool_call_buffer.push_str(&self.buffer);
            if !self.tool_call_buffer.is_empty() {
                events.push(StreamEvent::ToolCallPartial(
                    self.tool_call_buffer.clone(),
                ));
            }
        } else if !self.buffer.is_empty() {
            events.push(StreamEvent::Text(self.buffer.clone()));
        }
        self.buffer.clear();
        self.tool_call_buffer.clear();
        self.inside_tool_call = false;
        events
    }

    /// How many bytes from the start of `buffer` can safely be emitted without
    /// risking that they are part of a partial `tag` at the end.
    ///
    /// For example, if `buffer` ends with `"<tool_"` and `tag` is `"<tool_call>"`,
    /// we must not emit those last 6 bytes.
    fn safe_emit_len(&self, tag: &str) -> usize {
        let buf = self.buffer.as_bytes();
        let tag_bytes = tag.as_bytes();

        // Check every suffix of `buffer` to see if it matches a prefix of `tag`.
        for suffix_start in (1..=buf.len().min(tag_bytes.len())).rev() {
            let suffix = &buf[buf.len().saturating_sub(suffix_start)..];
            if tag_bytes.starts_with(suffix) {
                return buf.len() - suffix.len();
            }
        }
        buf.len()
    }

    /// Parse the body of a `<tool_call>` block into a `ParsedToolCall`.
    /// Handles the Qwen 3.5 XML parameter format.
    fn parse_tool_call_body(&self, body: &str) -> Option<ParsedToolCall> {
        let func_re = regex::Regex::new(r"(?s)<function=([^>]+)>(.*?)</function>").ok()?;
        let param_re = regex::Regex::new(r"(?s)<parameter=([^>]+)>(.*?)</parameter>").ok()?;

        let func_cap = func_re.captures(body)?;
        let func_name = func_cap.get(1)?.as_str().trim().to_string();
        let func_body = func_cap.get(2)?.as_str();

        let mut args = HashMap::new();
        for param_cap in param_re.captures_iter(func_body) {
            let key = param_cap.get(1).unwrap().as_str().trim().to_string();
            let val = param_cap.get(2).unwrap().as_str();
            let json_val = serde_json::from_str(val.trim())
                .unwrap_or_else(|_| serde_json::Value::String(val.to_string()));
            args.insert(key, json_val);
        }

        Some(ParsedToolCall {
            name: func_name,
            arguments: args,
            raw_text: format!("<tool_call>{body}</tool_call>"),
        })
    }
}

impl Default for StreamingToolCallParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_passthrough() {
        let mut parser = StreamingToolCallParser::new();
        let events = parser.feed("Hello, world!");
        // May not emit yet due to partial-tag buffering. Flush to get it.
        let mut all_events = events;
        all_events.extend(parser.flush());

        let texts: Vec<String> = all_events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.join(""), "Hello, world!");
    }

    #[test]
    fn test_complete_tool_call_single_chunk() {
        let mut parser = StreamingToolCallParser::new();
        let input = r#"<tool_call>
<function=file_read>
<parameter=path>/src/main.rs</parameter>
</function>
</tool_call>"#;

        let events = parser.feed(input);
        let complete: Vec<&ParsedToolCall> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallComplete(c) => Some(c),
                _ => None,
            })
            .collect();

        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].name, "file_read");
        assert_eq!(
            complete[0].arguments.get("path").unwrap(),
            &serde_json::Value::String("/src/main.rs".to_string())
        );
    }

    #[test]
    fn test_tool_call_split_across_chunks() {
        let mut parser = StreamingToolCallParser::new();

        // Split in the middle of the opening tag.
        let events1 = parser.feed("Some text <tool_");
        let events2 = parser.feed("call>\n<function=bash>\n<parameter=command>ls</para");
        let events3 = parser.feed("meter>\n</function>\n</tool_call> done");

        let mut all = Vec::new();
        all.extend(events1);
        all.extend(events2);
        all.extend(events3);
        all.extend(parser.flush());

        let has_start = all.iter().any(|e| matches!(e, StreamEvent::ToolCallStart));
        assert!(has_start, "should have ToolCallStart");

        let complete: Vec<&ParsedToolCall> = all
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallComplete(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].name, "bash");
    }

    #[test]
    fn test_multiple_tool_calls_streaming() {
        let mut parser = StreamingToolCallParser::new();

        let input = r#"<tool_call>
<function=file_read>
<parameter=path>/a.rs</parameter>
</function>
</tool_call>
<tool_call>
<function=file_read>
<parameter=path>/b.rs</parameter>
</function>
</tool_call>"#;

        let events = parser.feed(input);
        let complete: Vec<&ParsedToolCall> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallComplete(c) => Some(c),
                _ => None,
            })
            .collect();

        assert_eq!(complete.len(), 2);
        assert_eq!(
            complete[0].arguments.get("path").unwrap(),
            &serde_json::Value::String("/a.rs".to_string())
        );
        assert_eq!(
            complete[1].arguments.get("path").unwrap(),
            &serde_json::Value::String("/b.rs".to_string())
        );
    }

    #[test]
    fn test_text_before_and_after_tool_call() {
        let mut parser = StreamingToolCallParser::new();
        let input = r#"Let me check. <tool_call>
<function=bash>
<parameter=command>pwd</parameter>
</function>
</tool_call> Done."#;

        let mut events = parser.feed(input);
        events.extend(parser.flush());

        let texts: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();

        let joined = texts.join("");
        assert!(joined.contains("Let me check."), "text before tool call: {joined}");
        assert!(joined.contains("Done."), "text after tool call: {joined}");
    }

    #[test]
    fn test_partial_closing_tag_buffered() {
        let mut parser = StreamingToolCallParser::new();

        // Open a tool call and feed partial closing tag.
        let e1 = parser.feed("<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</function>\n</tool_");
        let e2 = parser.feed("call>");

        let mut all = Vec::new();
        all.extend(e1);
        all.extend(e2);

        let complete: Vec<&ParsedToolCall> = all
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallComplete(c) => Some(c),
                _ => None,
            })
            .collect();

        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].name, "bash");
    }

    #[test]
    fn test_flush_incomplete_tool_call() {
        let mut parser = StreamingToolCallParser::new();

        // Start a tool call but never close it.
        let _ = parser.feed("<tool_call>\n<function=bash>");
        let events = parser.flush();

        let has_partial = events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolCallPartial(_)));
        assert!(has_partial, "should emit partial on flush for incomplete tool call");
    }

    #[test]
    fn test_empty_chunks() {
        let mut parser = StreamingToolCallParser::new();
        let events = parser.feed("");
        assert!(events.is_empty());
    }

    #[test]
    fn test_single_char_chunks() {
        let mut parser = StreamingToolCallParser::new();
        let input = "<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</function>\n</tool_call>";

        let mut all = Vec::new();
        for ch in input.chars() {
            all.extend(parser.feed(&ch.to_string()));
        }
        all.extend(parser.flush());

        let complete: Vec<&ParsedToolCall> = all
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallComplete(c) => Some(c),
                _ => None,
            })
            .collect();

        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].name, "bash");
    }
}
