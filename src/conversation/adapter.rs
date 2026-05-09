use std::collections::HashMap;
use std::fmt;

use regex::Regex;

use crate::backend::types::ToolDefinition;

/// A parsed tool call extracted from model output.
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: HashMap<String, serde_json::Value>,
    pub raw_text: String,
}

/// Abstracts model-specific tool call formatting and parsing.
///
/// Different model families emit tool calls in different XML/JSON dialects.
/// Implementing this trait lets the rest of the system stay format-agnostic.
pub trait ModelAdapter: Send + Sync + fmt::Debug {
    /// Format tool definitions for inclusion in the system prompt.
    fn format_tools(&self, tools: &[ToolDefinition]) -> String;

    /// Format a tool result message to feed back to the model.
    fn format_tool_result(&self, tool_call_id: &str, result: &str) -> String;

    /// Parse tool calls out of raw model output text.
    fn parse_tool_calls(&self, text: &str) -> Vec<ParsedToolCall>;

    /// The chat template name this adapter targets (e.g. "qwen3.5", "chatml").
    fn chat_template_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Qwen 3.5 adapter  --  XML parameter format
// ---------------------------------------------------------------------------

/// Handles the Qwen 3.5 XML tool-call format:
/// ```text
/// <tool_call>
/// <function=tool_name>
/// <parameter=key>value</parameter>
/// </function>
/// </tool_call>
/// ```
#[derive(Debug)]
pub struct Qwen35Adapter;

impl ModelAdapter for Qwen35Adapter {
    fn format_tools(&self, tools: &[ToolDefinition]) -> String {
        // Qwen 3.5 expects a JSON array of tool definitions in the system prompt.
        let defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&defs).unwrap_or_else(|_| "[]".to_string());

        format!(
            "# Tools\n\n\
             You have access to the following tools. To call a tool, use XML format:\n\
             <tool_call>\n\
             <function=TOOL_NAME>\n\
             <parameter=PARAM_NAME>PARAM_VALUE</parameter>\n\
             </function>\n\
             </tool_call>\n\n\
             Available tools:\n{json}"
        )
    }

    fn format_tool_result(&self, tool_call_id: &str, result: &str) -> String {
        format!(
            "<tool_response>\n\
             <id>{tool_call_id}</id>\n\
             <result>{result}</result>\n\
             </tool_response>"
        )
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ParsedToolCall> {
        parse_qwen35_xml(text)
    }

    fn chat_template_name(&self) -> &str {
        "qwen3.5"
    }
}

/// Strip content inside markdown code fences (triple backticks) from text.
///
/// SECURITY: Prevents models from hiding tool calls inside code blocks that
/// are meant as examples or documentation. Without this, a model could output
/// "here is an example: ```<tool_call>...</tool_call>```" and the parser
/// would execute the "example" tool call. (P0 #7)
fn strip_code_fences(text: &str) -> String {
    let fence_re = Regex::new(r"(?s)```[^\n]*\n.*?```").unwrap();
    fence_re.replace_all(text, "").to_string()
}

/// Remove `<tool_call>...</tool_call>` blocks from assistant content after the
/// inline-tool-call fallback has lifted them into structured calls. Leaves
/// any natural-language prefix/suffix intact so the user still sees the
/// model's reasoning around the call.
pub fn strip_tool_call_blocks(text: &str) -> String {
    let block_re = Regex::new(r"(?s)<tool_call>.*?</tool_call>").unwrap();
    block_re.replace_all(text, "").trim().to_string()
}

/// Core Qwen 3.5 XML parsing logic, exposed for reuse by recovery/streaming.
pub fn parse_qwen35_xml(text: &str) -> Vec<ParsedToolCall> {
    // SECURITY (P0 #7): Strip markdown code fences before parsing to prevent
    // extraction of tool calls from example/documentation code blocks.
    let text = strip_code_fences(text);

    let block_re = Regex::new(r"(?s)<tool_call>(.*?)</tool_call>").unwrap();
    let func_re = Regex::new(r"(?s)<function=([^>]+)>(.*?)</function>").unwrap();
    let param_re = Regex::new(r"(?s)<parameter=([^>]+)>(.*?)</parameter>").unwrap();

    let mut calls = Vec::new();

    for block_cap in block_re.captures_iter(&text) {
        let block_body = block_cap.get(1).unwrap().as_str();
        let block_raw = block_cap.get(0).unwrap().as_str();

        for func_cap in func_re.captures_iter(block_body) {
            let func_name = func_cap.get(1).unwrap().as_str().trim().to_string();
            let func_body = func_cap.get(2).unwrap().as_str();

            let mut args = HashMap::new();
            for param_cap in param_re.captures_iter(func_body) {
                let key = param_cap.get(1).unwrap().as_str().trim().to_string();
                let val = param_cap.get(2).unwrap().as_str();

                // SECURITY (P0 #4): Reject duplicate parameter names.
                // An attacker can inject a second <parameter=path> tag after the
                // legitimate one. HashMap::insert overwrites, so the LAST value
                // wins -- the attacker's value. We keep only the FIRST occurrence.
                if args.contains_key(&key) {
                    eprintln!(
                        "[SECURITY] Duplicate parameter '{}' in tool call '{}' -- keeping first, ignoring duplicate",
                        key, func_name
                    );
                    continue;
                }

                // Try to parse as JSON first (numbers, bools, objects), fall back to string.
                let json_val = serde_json::from_str(val.trim())
                    .unwrap_or_else(|_| serde_json::Value::String(val.to_string()));
                args.insert(key, json_val);
            }

            calls.push(ParsedToolCall {
                name: func_name,
                arguments: args,
                raw_text: block_raw.to_string(),
            });
        }
    }

    calls
}

// ---------------------------------------------------------------------------
// Hermes adapter  --  JSON-in-XML format (Qwen 2.5 / Qwen 3)
// ---------------------------------------------------------------------------

/// Handles the Hermes/ChatML JSON tool-call format:
/// ```text
/// <tool_call>
/// {"name": "tool", "arguments": {"key": "value"}}
/// </tool_call>
/// ```
#[derive(Debug)]
pub struct HermesAdapter;

impl ModelAdapter for HermesAdapter {
    fn format_tools(&self, tools: &[ToolDefinition]) -> String {
        let defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&defs).unwrap_or_else(|_| "[]".to_string());

        format!(
            "# Tools\n\n\
             You have access to the following tools. To call a tool, respond with JSON inside XML tags:\n\
             <tool_call>\n\
             {{\"name\": \"TOOL_NAME\", \"arguments\": {{\"PARAM\": \"VALUE\"}}}}\n\
             </tool_call>\n\n\
             Available tools:\n{json}"
        )
    }

    fn format_tool_result(&self, tool_call_id: &str, result: &str) -> String {
        format!(
            "<tool_response>\n\
             {{\"id\": \"{tool_call_id}\", \"result\": {result}}}\n\
             </tool_response>"
        )
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ParsedToolCall> {
        parse_hermes_json(text)
    }

    fn chat_template_name(&self) -> &str {
        "chatml"
    }
}

/// Core Hermes JSON parsing logic.
pub fn parse_hermes_json(text: &str) -> Vec<ParsedToolCall> {
    // SECURITY (P0 #7): Strip markdown code fences before parsing.
    let text = strip_code_fences(text);

    let block_re = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap();
    let mut calls = Vec::new();

    for cap in block_re.captures_iter(&text) {
        let json_str = cap.get(1).unwrap().as_str();
        let raw = cap.get(0).unwrap().as_str().to_string();

        let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) else {
            continue;
        };

        let name = val
            .get("name")
            .or_else(|| val.get("tool"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if name.is_empty() {
            continue;
        }

        let arguments = val
            .get("arguments")
            .or_else(|| val.get("params"))
            .or_else(|| val.get("parameters"))
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let args: HashMap<String, serde_json::Value> = match arguments {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            other => {
                let mut m = HashMap::new();
                m.insert("_raw".to_string(), other);
                m
            }
        };

        calls.push(ParsedToolCall {
            name,
            arguments: args,
            raw_text: raw,
        });
    }

    calls
}

// ---------------------------------------------------------------------------
// Generic adapter  --  prompted XML for unknown models
// ---------------------------------------------------------------------------

/// Fallback adapter that uses prompted XML for models we do not recognize.
/// Uses the same XML format as Qwen 3.5 but with more explicit prompting.
#[derive(Debug)]
pub struct GenericAdapter;

impl ModelAdapter for GenericAdapter {
    fn format_tools(&self, tools: &[ToolDefinition]) -> String {
        let mut out = String::from(
            "# Tools\n\n\
             You have access to the following tools. To call a tool, you MUST use this EXACT format:\n\n\
             <tool_call>\n\
             <function=TOOL_NAME>\n\
             <parameter=PARAM_NAME>PARAM_VALUE</parameter>\n\
             </function>\n\
             </tool_call>\n\n\
             Do NOT deviate from this format. Available tools:\n\n",
        );

        for tool in tools {
            out.push_str(&format!("## {}\n", tool.name));
            out.push_str(&format!("{}\n", tool.description));
            out.push_str(&format!(
                "Parameters: {}\n\n",
                serde_json::to_string_pretty(&tool.parameters)
                    .unwrap_or_else(|_| "{}".to_string())
            ));
        }

        out
    }

    fn format_tool_result(&self, tool_call_id: &str, result: &str) -> String {
        format!(
            "<tool_response>\n\
             <id>{tool_call_id}</id>\n\
             <result>{result}</result>\n\
             </tool_response>"
        )
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ParsedToolCall> {
        // Try Qwen 3.5 XML first, then Hermes JSON.
        let calls = parse_qwen35_xml(text);
        if !calls.is_empty() {
            return calls;
        }
        parse_hermes_json(text)
    }

    fn chat_template_name(&self) -> &str {
        "generic"
    }
}

// ---------------------------------------------------------------------------
// Auto-detection
// ---------------------------------------------------------------------------

/// Detect the appropriate adapter from a model name string.
///
/// Matching is case-insensitive. Qwen 3.5 variants get the XML adapter,
/// Qwen 2.5 / 3 get the Hermes JSON adapter, and everything else gets
/// the generic prompted adapter.
pub fn detect_adapter(model_name: &str) -> Box<dyn ModelAdapter> {
    let lower = model_name.to_lowercase();

    if lower.contains("qwen3.5") || lower.contains("qwen-3.5") || lower.contains("qwen_3.5") {
        Box::new(Qwen35Adapter)
    } else if lower.contains("qwen3") || lower.contains("qwen-3") || lower.contains("qwen_3")
        || lower.contains("qwen2.5") || lower.contains("qwen-2.5") || lower.contains("qwen_2.5")
        || lower.contains("hermes")
    {
        Box::new(HermesAdapter)
    } else {
        Box::new(GenericAdapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tools() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "file_read".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path" }
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "bash".to_string(),
                description: "Run a shell command".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }),
            },
        ]
    }

    // -----------------------------------------------------------------------
    // Qwen 3.5 parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_qwen35_single_param() {
        let input = r#"Let me read that file.
<tool_call>
<function=file_read>
<parameter=path>/src/main.rs</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(
            calls[0].arguments.get("path").unwrap(),
            &serde_json::Value::String("/src/main.rs".to_string())
        );
    }

    #[test]
    fn test_qwen35_multi_param() {
        let input = r#"<tool_call>
<function=file_edit>
<parameter=path>/src/lib.rs</parameter>
<parameter=old_string>fn old() {}</parameter>
<parameter=new_string>fn new() {}</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_edit");
        assert_eq!(calls[0].arguments.len(), 3);
        assert_eq!(
            calls[0].arguments.get("old_string").unwrap(),
            &serde_json::Value::String("fn old() {}".to_string())
        );
    }

    #[test]
    fn test_qwen35_multi_tool_call() {
        let input = r#"I'll read both files.
<tool_call>
<function=file_read>
<parameter=path>/a.rs</parameter>
</function>
</tool_call>
<tool_call>
<function=file_read>
<parameter=path>/b.rs</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].arguments.get("path").unwrap(),
            &serde_json::Value::String("/a.rs".to_string())
        );
        assert_eq!(
            calls[1].arguments.get("path").unwrap(),
            &serde_json::Value::String("/b.rs".to_string())
        );
    }

    #[test]
    fn test_qwen35_newlines_in_value() {
        let input = r#"<tool_call>
<function=file_write>
<parameter=path>/test.rs</parameter>
<parameter=content>line one
line two
line three</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 1);
        let content = calls[0].arguments.get("content").unwrap();
        assert!(content.as_str().unwrap().contains("line two"));
    }

    #[test]
    fn test_qwen35_numeric_param_value() {
        let input = r#"<tool_call>
<function=file_read>
<parameter=path>/src/main.rs</parameter>
<parameter=offset>42</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 1);
        // 42 should parse as a JSON number, not a string.
        assert_eq!(
            calls[0].arguments.get("offset").unwrap(),
            &serde_json::json!(42)
        );
    }

    #[test]
    fn test_qwen35_boolean_param_value() {
        let input = r#"<tool_call>
<function=bash>
<parameter=command>ls</parameter>
<parameter=background>true</parameter>
</function>
</tool_call>"#;

        let calls = parse_qwen35_xml(input);
        assert_eq!(calls[0].arguments.get("background").unwrap(), &serde_json::json!(true));
    }

    // -----------------------------------------------------------------------
    // Hermes parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_hermes_single_call() {
        let input = r#"<tool_call>
{"name": "file_read", "arguments": {"path": "/src/main.rs"}}
</tool_call>"#;

        let calls = parse_hermes_json(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(
            calls[0].arguments.get("path").unwrap(),
            &serde_json::Value::String("/src/main.rs".to_string())
        );
    }

    #[test]
    fn test_hermes_invalid_json_skipped() {
        let input = r#"<tool_call>
{not valid json}
</tool_call>"#;

        let calls = parse_hermes_json(input);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_hermes_tool_key_variant() {
        let input = r#"<tool_call>
{"tool": "grep", "params": {"pattern": "TODO"}}
</tool_call>"#;

        let calls = parse_hermes_json(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
    }

    // -----------------------------------------------------------------------
    // Adapter trait usage
    // -----------------------------------------------------------------------

    #[test]
    fn test_qwen35_adapter_format_tools() {
        let adapter = Qwen35Adapter;
        let output = adapter.format_tools(&sample_tools());
        assert!(output.contains("file_read"));
        assert!(output.contains("bash"));
        assert!(output.contains("<function=TOOL_NAME>"));
    }

    #[test]
    fn test_qwen35_adapter_format_tool_result() {
        let adapter = Qwen35Adapter;
        let output = adapter.format_tool_result("tc_1", "file contents here");
        assert!(output.contains("tc_1"));
        assert!(output.contains("file contents here"));
    }

    #[test]
    fn test_hermes_adapter_format_tools() {
        let adapter = HermesAdapter;
        let output = adapter.format_tools(&sample_tools());
        assert!(output.contains("file_read"));
        assert!(output.contains("JSON inside XML tags"));
    }

    #[test]
    fn test_generic_adapter_tries_both_formats() {
        let adapter = GenericAdapter;

        // Qwen 3.5 XML
        let input = r#"<tool_call>
<function=bash>
<parameter=command>ls</parameter>
</function>
</tool_call>"#;
        let calls = adapter.parse_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");

        // Hermes JSON
        let input2 = r#"<tool_call>
{"name": "bash", "arguments": {"command": "ls"}}
</tool_call>"#;
        let calls2 = adapter.parse_tool_calls(input2);
        assert_eq!(calls2.len(), 1);
        assert_eq!(calls2[0].name, "bash");
    }

    // -----------------------------------------------------------------------
    // Adapter detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_qwen35() {
        let adapter = detect_adapter("Qwen3.5-27B");
        assert_eq!(adapter.chat_template_name(), "qwen3.5");
    }

    #[test]
    fn test_detect_qwen35_variants() {
        for name in &["Qwen3.5-27B-4bit", "qwen-3.5-35B", "QWEN_3.5_4B"] {
            let adapter = detect_adapter(name);
            assert_eq!(adapter.chat_template_name(), "qwen3.5", "failed for {name}");
        }
    }

    #[test]
    fn test_detect_qwen3_hermes() {
        let adapter = detect_adapter("Qwen3-8B");
        assert_eq!(adapter.chat_template_name(), "chatml");
    }

    #[test]
    fn test_detect_qwen25_hermes() {
        let adapter = detect_adapter("Qwen2.5-Coder-32B");
        assert_eq!(adapter.chat_template_name(), "chatml");
    }

    #[test]
    fn test_detect_unknown_model() {
        let adapter = detect_adapter("unknown-model-7b");
        assert_eq!(adapter.chat_template_name(), "generic");
    }

    #[test]
    fn test_detect_hermes_in_name() {
        let adapter = detect_adapter("NousResearch-Hermes-2-Pro");
        assert_eq!(adapter.chat_template_name(), "chatml");
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_tool_calls_in_text() {
        let calls = parse_qwen35_xml("Just a regular response with no tool calls.");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_empty_input() {
        assert!(parse_qwen35_xml("").is_empty());
        assert!(parse_hermes_json("").is_empty());
    }

    #[test]
    fn test_qwen35_no_params() {
        let input = r#"<tool_call>
<function=ask_user>
</function>
</tool_call>"#;
        let calls = parse_qwen35_xml(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "ask_user");
        assert!(calls[0].arguments.is_empty());
    }
}
