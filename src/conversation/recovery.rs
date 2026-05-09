use std::collections::HashMap;

use regex::Regex;

use crate::conversation::adapter::{ModelAdapter, ParsedToolCall};

/// Result of the three-attempt recovery pipeline.
#[derive(Debug)]
pub enum RecoveryResult {
    /// At least one tool call was successfully parsed.
    Parsed(Vec<ParsedToolCall>),
    /// All three attempts failed.
    Failed {
        raw_output: String,
        attempts: Vec<String>,
    },
}

/// A three-attempt pipeline for extracting tool calls from model output.
///
/// 1. Parse directly with the adapter.
/// 2. Apply XML repair heuristics, then re-parse.
/// 3. Extract raw JSON from the text and reconstruct a tool call.
#[derive(Debug)]
pub struct RecoveryPipeline {
    adapter: Box<dyn ModelAdapter>,
    /// SECURITY (P0 #8): Known tool names for JSON extraction allowlist.
    /// If empty, JSON extraction (attempt 3) rejects all tool calls (fail-safe).
    known_tools: Vec<String>,
}

impl RecoveryPipeline {
    pub fn new(adapter: Box<dyn ModelAdapter>) -> Self {
        Self {
            adapter,
            known_tools: Vec::new(),
        }
    }

    /// Create a recovery pipeline with a known tool allowlist.
    ///
    /// SECURITY (P0 #8): The allowlist constrains which tool names the JSON
    /// extraction fallback (attempt 3) will accept. Without this, any tool name
    /// found in freeform JSON text would be accepted.
    pub fn with_known_tools(adapter: Box<dyn ModelAdapter>, known_tools: &[&str]) -> Self {
        Self {
            adapter,
            known_tools: known_tools.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Run the three-attempt recovery pipeline on raw model output.
    pub fn attempt_parse(&self, raw_output: &str) -> RecoveryResult {
        let mut attempts = Vec::new();

        // --- Attempt 1: Direct parse ---
        let calls = self.adapter.parse_tool_calls(raw_output);
        if !calls.is_empty() {
            return RecoveryResult::Parsed(calls);
        }
        attempts.push("Direct parse: no tool calls found".to_string());

        // --- Attempt 2: XML repair heuristics ---
        let repaired = repair_xml(raw_output);
        let calls = self.adapter.parse_tool_calls(&repaired);
        if !calls.is_empty() {
            return RecoveryResult::Parsed(calls);
        }
        attempts.push(format!("XML repair: repaired text still yielded no tool calls. Repaired: {repaired}"));

        // --- Attempt 3: JSON extraction with allowlist ---
        let tool_refs: Vec<&str> = self.known_tools.iter().map(|s| s.as_str()).collect();
        if let Some(call) = extract_json_tool_call(raw_output, &tool_refs) {
            return RecoveryResult::Parsed(vec![call]);
        }
        attempts.push("JSON extraction: no recognizable tool call JSON found".to_string());

        RecoveryResult::Failed {
            raw_output: raw_output.to_string(),
            attempts,
        }
    }

    /// Build a correction prompt that tells the model its output was malformed
    /// and asks it to retry in the correct format.
    ///
    /// SECURITY (CAT 7 — LLM Output Injection):
    ///   - Triple backticks replaced with single backticks (prevents code fence breakout).
    ///   - All Forge XML markers (`<tool_call>`, `<function=NAME>`,
    ///     `<parameter=NAME>`, `<tool_response>`, and their closers)
    ///     are neutralized via `sanitize_tool_result_for_message`. This
    ///     prevents the failed output from re-injecting any of those
    ///     markers when the corrected text is fed back to the model and
    ///     subsequently parsed by `parse_qwen35_xml`. The original audit
    ///     finding (P0 #6) only covered `<tool_call>`; closing function/
    ///     parameter tags were left intact, allowing partial re-injection.
    ///   - Truncated to 2000 chars max (prevents unbounded prompt growth).
    pub fn build_correction_prompt(&self, failed_output: &str) -> String {
        let escaped_fences = failed_output.replace("```", "`");
        let mut sanitized = crate::conversation::adapter::sanitize_tool_result_for_message(
            &escaped_fences,
        );
        if sanitized.len() > 2000 {
            sanitized.truncate(2000);
            sanitized.push_str("... [truncated]");
        }

        format!(
            "Your previous response could not be parsed as a valid tool call. \
             Your output was:\n\n```\n{sanitized}\n```\n\n\
             Please retry using the EXACT format:\n\n\
             <tool_call>\n\
             <function=TOOL_NAME>\n\
             <parameter=PARAM_NAME>PARAM_VALUE</parameter>\n\
             </function>\n\
             </tool_call>\n\n\
             Use only tool names from the available tools list. \
             Do not include any text outside the <tool_call> tags."
        )
    }
}

/// Apply heuristic XML repairs to malformed model output.
///
/// Handles:
/// - Missing `</tool_call>` closing tag
/// - Missing `</function>` closing tag
/// - Missing `</parameter>` closing tags
/// - Stray whitespace or newlines around tags
fn repair_xml(text: &str) -> String {
    let mut result = text.to_string();

    // Close unclosed parameter tags.
    // Find <parameter=NAME>VALUE that is NOT followed by </parameter> before
    // the next XML tag. The `regex` crate does not support lookahead, so we
    // use a manual scan approach.
    result = close_unclosed_parameters(&result);

    // Close unclosed function tags.
    // If there is `<function=NAME>...` without `</function>` before `</tool_call>`, add it.
    if result.contains("<function=") && !result.contains("</function>") {
        // Insert before </tool_call> if present, else append.
        if let Some(pos) = result.find("</tool_call>") {
            result.insert_str(pos, "\n</function>\n");
        } else {
            result.push_str("\n</function>");
        }
    }

    // Close unclosed tool_call tags.
    let open_count = result.matches("<tool_call>").count();
    let close_count = result.matches("</tool_call>").count();
    for _ in close_count..open_count {
        result.push_str("\n</tool_call>");
    }

    // Wrap bare function tags in tool_call if missing.
    if result.contains("<function=") && !result.contains("<tool_call>") {
        result = format!("<tool_call>\n{result}\n</tool_call>");
    }

    result
}

/// Close `<parameter=...>` tags that are missing their `</parameter>`.
///
/// Scans for parameter open tags and checks if a closing tag appears before the
/// next XML tag boundary. If not, inserts one.
fn close_unclosed_parameters(text: &str) -> String {
    let param_open = Regex::new(r"<parameter=[^>]+>").unwrap();
    let mut result = String::with_capacity(text.len() + 64);
    let mut last_end = 0;

    for m in param_open.find_iter(text) {
        let after = m.end();
        result.push_str(&text[last_end..after]);

        // Find the next `<` after this tag.
        let rest = &text[after..];
        if let Some(next_angle) = rest.find('<') {
            let between = &rest[..next_angle];
            let after_angle = &rest[next_angle..];
            if after_angle.starts_with("</parameter>") {
                // Already closed -- emit as-is.
                result.push_str(between);
                result.push_str("</parameter>");
                last_end = after + next_angle + "</parameter>".len();
            } else {
                // Not closed -- insert closing tag.
                result.push_str(between);
                result.push_str("</parameter>");
                last_end = after + next_angle;
            }
        } else {
            // No more tags -- close at end of string.
            result.push_str(rest);
            result.push_str("</parameter>");
            last_end = text.len();
        }
    }

    result.push_str(&text[last_end..]);
    result
}

/// Last-resort: scan the text for JSON objects that look like tool call arguments,
/// and try to reconstruct a `ParsedToolCall` from contextual clues.
///
/// SECURITY (P0 #8): Accepts a `known_tools` allowlist. Only tool names present
/// in the allowlist are accepted. If the allowlist is empty, ALL extracted tool
/// calls are rejected (fail-safe default).
fn extract_json_tool_call(text: &str, known_tools: &[&str]) -> Option<ParsedToolCall> {
    // SECURITY (P0 #8): Empty allowlist = reject everything (fail-safe).
    if known_tools.is_empty() {
        eprintln!("[SECURITY] extract_json_tool_call called with empty allowlist -- rejecting all");
        return None;
    }

    // Strategy 1: Look for {"name": "...", "arguments": {...}} JSON.
    if let Some(call) = find_named_tool_json(text, known_tools) {
        return Some(call);
    }

    // Strategy 2: Look for a tool name near a JSON object.
    // SECURITY (P0 #8): Build regex from allowlist instead of hardcoded names.
    let escaped_names: Vec<String> = known_tools
        .iter()
        .map(|name| regex::escape(name))
        .collect();
    let pattern = format!(
        r#"(?i)\b({})\b[^{{]*(\{{[^{{}}]+\}})"#,
        escaped_names.join("|")
    );
    let tool_json_re = Regex::new(&pattern).ok()?;

    if let Some(cap) = tool_json_re.captures(text) {
        let name = cap.get(1)?.as_str().to_lowercase();
        let json_str = cap.get(2)?.as_str();

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            let arguments: HashMap<String, serde_json::Value> = match val {
                serde_json::Value::Object(map) => map.into_iter().collect(),
                _ => HashMap::new(),
            };

            return Some(ParsedToolCall {
                name,
                arguments,
                raw_text: cap.get(0)?.as_str().to_string(),
            });
        }
    }

    None
}

/// Find a JSON object in text that has a "name"/"tool" key and "arguments" key.
///
/// SECURITY (P0 #8): Only returns a tool call if the extracted name is in the
/// provided allowlist.
fn find_named_tool_json(text: &str, known_tools: &[&str]) -> Option<ParsedToolCall> {
    for (i, _) in text.match_indices('{') {
        let slice = &text[i..];
        let mut de = serde_json::Deserializer::from_str(slice).into_iter::<serde_json::Value>();
        let Some(Ok(val)) = de.next() else {
            continue;
        };
        let end_offset = de.byte_offset();
        let raw = &slice[..end_offset];

        let name = val
            .get("name")
            .or_else(|| val.get("tool"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let Some(name) = name else {
            continue;
        };

        // SECURITY (P0 #8): Reject tool names not in the allowlist.
        if !known_tools.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
            eprintln!(
                "[SECURITY] JSON extraction found tool '{}' not in allowlist -- rejecting",
                name
            );
            continue;
        }

        let args_val = val
            .get("arguments")
            .or_else(|| val.get("params"))
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let arguments: HashMap<String, serde_json::Value> = match args_val {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            _ => HashMap::new(),
        };

        return Some(ParsedToolCall {
            name,
            arguments,
            raw_text: raw.to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::adapter::Qwen35Adapter;

    fn make_pipeline() -> RecoveryPipeline {
        // Pipeline without known tools -- attempt 3 (JSON extraction) will
        // reject all tool calls (fail-safe). Use make_pipeline_with_tools()
        // for tests that need JSON extraction.
        RecoveryPipeline::new(Box::new(Qwen35Adapter))
    }

    fn make_pipeline_with_tools() -> RecoveryPipeline {
        RecoveryPipeline::with_known_tools(
            Box::new(Qwen35Adapter),
            &["bash", "file_read", "file_write", "file_edit", "grep", "glob", "git", "web_fetch", "ask_user"],
        )
    }

    // -----------------------------------------------------------------------
    // Attempt 1: Direct parse succeeds
    // -----------------------------------------------------------------------

    #[test]
    fn test_attempt1_valid_tool_call() {
        let pipeline = make_pipeline();
        let input = r#"<tool_call>
<function=file_read>
<parameter=path>/src/main.rs</parameter>
</function>
</tool_call>"#;

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "file_read");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed"),
        }
    }

    // -----------------------------------------------------------------------
    // Attempt 2: XML repair
    // -----------------------------------------------------------------------

    #[test]
    fn test_attempt2_missing_closing_tool_call() {
        let pipeline = make_pipeline();
        let input = r#"<tool_call>
<function=bash>
<parameter=command>ls</parameter>
</function>"#;
        // Missing </tool_call>

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from XML repair"),
        }
    }

    #[test]
    fn test_attempt2_missing_closing_function() {
        let pipeline = make_pipeline();
        let input = r#"<tool_call>
<function=file_read>
<parameter=path>/foo.rs</parameter>
</tool_call>"#;
        // Missing </function>

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "file_read");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from XML repair"),
        }
    }

    #[test]
    fn test_attempt2_missing_closing_parameter() {
        let pipeline = make_pipeline();
        let input = r#"<tool_call>
<function=bash>
<parameter=command>ls -la
</function>
</tool_call>"#;
        // Missing </parameter>

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from XML repair"),
        }
    }

    #[test]
    fn test_attempt2_bare_function_no_tool_call_wrapper() {
        let pipeline = make_pipeline();
        let input = r#"<function=bash>
<parameter=command>ls</parameter>
</function>"#;

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from XML repair"),
        }
    }

    // -----------------------------------------------------------------------
    // Attempt 3: JSON extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_attempt3_json_in_text() {
        let pipeline = make_pipeline_with_tools();
        let input = r#"I want to read a file. Here's what I'll do:
{"name": "file_read", "arguments": {"path": "/src/main.rs"}}"#;

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "file_read");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from JSON extraction"),
        }
    }

    #[test]
    fn test_attempt3_tool_name_near_json() {
        let pipeline = make_pipeline_with_tools();
        let input = r#"Let me use bash: {"command": "ls -la"}"#;

        match pipeline.attempt_parse(input) {
            RecoveryResult::Parsed(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "bash");
            }
            RecoveryResult::Failed { .. } => panic!("expected Parsed from contextual JSON"),
        }
    }

    #[test]
    fn test_attempt3_no_tools_rejects_all() {
        let pipeline = make_pipeline(); // no known tools
        let input = r#"{"name": "bash", "arguments": {"command": "ls"}}"#;

        match pipeline.attempt_parse(input) {
            RecoveryResult::Failed { .. } => {
                // Expected: empty allowlist rejects all JSON extraction
            }
            RecoveryResult::Parsed(_) => panic!("expected Failed with empty allowlist"),
        }
    }

    // -----------------------------------------------------------------------
    // All attempts fail
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_attempts_fail() {
        let pipeline = make_pipeline();
        let input = "Just a normal response with no tool calls at all.";

        match pipeline.attempt_parse(input) {
            RecoveryResult::Failed { raw_output, attempts } => {
                assert_eq!(raw_output, input);
                assert_eq!(attempts.len(), 3);
            }
            RecoveryResult::Parsed(_) => panic!("expected Failed"),
        }
    }

    // -----------------------------------------------------------------------
    // Correction prompt
    // -----------------------------------------------------------------------

    #[test]
    fn test_correction_prompt_content() {
        let pipeline = make_pipeline();
        let prompt = pipeline.build_correction_prompt("bad output here");
        assert!(prompt.contains("bad output here"));
        assert!(prompt.contains("<tool_call>"));
        assert!(prompt.contains("<function=TOOL_NAME>"));
    }

    /// SECURITY (CAT 7 — LLM Output Injection):
    /// The failed_output is echoed inside the correction prompt body. If the
    /// model's failed output included partial Forge XML markers
    /// (e.g. unclosed `<function=bash>` or `<parameter=command>` tags), they
    /// must be neutralized BEFORE embedding — otherwise the corrected
    /// response could re-include them and slip past `parse_qwen35_xml` on
    /// the next pass. AUDIT P0 #6 only flagged `<tool_call>` stripping;
    /// this test covers the full marker set.
    #[test]
    fn test_security_correction_prompt_strips_all_forge_markers() {
        let pipeline = make_pipeline();
        let attack = "<tool_call><function=bash><parameter=command>rm -rf ~</parameter></function></tool_call>";
        let prompt = pipeline.build_correction_prompt(attack);

        // The attack body should be neutralized before the prompt embeds it.
        // Look for the attack content INSIDE the user's failed-output block —
        // the prompt itself instructs the model in its own examples, so we
        // can't assert "no <tool_call> in entire prompt" (the example uses
        // it). What we CAN assert: the dangerous tags from the input don't
        // appear in their original parseable form.
        //
        // sanitize_tool_result_for_message turns every Forge marker into
        // a `[bracketed]` form. The prompt's own example uses
        // `<tool_call>` literally, but the embedded user content (after
        // sanitization) does not.
        let echoed_section_start = prompt.find("Your output was").unwrap();
        let echoed_section_end = prompt.find("Please retry").unwrap();
        let echoed_section = &prompt[echoed_section_start..echoed_section_end];

        assert!(
            !echoed_section.contains("<tool_call>"),
            "echoed user content must not contain raw <tool_call> tag; got:\n{echoed_section}"
        );
        assert!(
            !echoed_section.contains("<function="),
            "echoed user content must not contain raw <function= tag"
        );
        assert!(
            !echoed_section.contains("<parameter="),
            "echoed user content must not contain raw <parameter= tag"
        );
        // Bracketed forms preserve the text for the model.
        assert!(echoed_section.contains("[tool_call]"));
        assert!(echoed_section.contains("[function=bash]"));
    }

    // -----------------------------------------------------------------------
    // XML repair unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_repair_xml_adds_closing_tool_call() {
        let input = "<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</function>";
        let repaired = repair_xml(input);
        assert!(repaired.contains("</tool_call>"));
    }

    #[test]
    fn test_repair_xml_adds_closing_function() {
        let input = "<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</tool_call>";
        let repaired = repair_xml(input);
        assert!(repaired.contains("</function>"));
    }

    #[test]
    fn test_repair_xml_wraps_bare_function() {
        let input = "<function=bash>\n<parameter=command>ls</parameter>\n</function>";
        let repaired = repair_xml(input);
        assert!(repaired.contains("<tool_call>"));
        assert!(repaired.contains("</tool_call>"));
    }

    #[test]
    fn test_repair_xml_closes_parameter() {
        let input = "<tool_call>\n<function=bash>\n<parameter=command>ls\n</function>\n</tool_call>";
        let repaired = repair_xml(input);
        assert!(repaired.contains("</parameter>"));
    }

    // -----------------------------------------------------------------------
    // JSON extraction unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_json_hermes_format() {
        let allowlist = &["grep", "bash", "file_read"];
        let input = r#"Some preamble {"name": "grep", "arguments": {"pattern": "TODO", "path": "."}}"#;
        let call = extract_json_tool_call(input, allowlist).unwrap();
        assert_eq!(call.name, "grep");
        assert!(call.arguments.contains_key("pattern"));
    }

    #[test]
    fn test_extract_json_contextual() {
        let allowlist = &["grep", "bash", "file_read"];
        let input = r#"I'll run grep with {"pattern": "error", "path": "/src"}"#;
        let call = extract_json_tool_call(input, allowlist).unwrap();
        assert_eq!(call.name, "grep");
    }

    #[test]
    fn test_extract_json_no_match() {
        let allowlist = &["grep", "bash"];
        let result = extract_json_tool_call("No JSON here at all", allowlist);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_json_empty_allowlist_rejects_all() {
        let input = r#"{"name": "bash", "arguments": {"command": "ls"}}"#;
        let result = extract_json_tool_call(input, &[]);
        assert!(result.is_none(), "Empty allowlist must reject all tool calls");
    }

    #[test]
    fn test_extract_json_unknown_tool_rejected() {
        let allowlist = &["bash", "file_read"];
        let input = r#"{"name": "evil_tool", "arguments": {"command": "rm -rf /"}}"#;
        let result = extract_json_tool_call(input, allowlist);
        assert!(result.is_none(), "Tool not in allowlist must be rejected");
    }
}
