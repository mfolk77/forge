/// Builds a GBNF grammar that constrains model output to valid Qwen 3.5 XML
/// tool call format.
///
/// This grammar is intended for use ONLY during retry (attempt 3 of the recovery
/// pipeline), when we are confident the model should produce a tool call. GBNF
/// constrains the entire generation, so it must not be applied during normal
/// freeform generation.
///
/// The generated grammar forces:
/// - Exactly one `<tool_call>...</tool_call>` block
/// - One `<function=NAME>...</function>` block with a name from the allowed list
/// - One or more `<parameter=KEY>VALUE</parameter>` tags
/// - Arbitrary string values inside parameter tags (any chars except `<`)
/// - Alphanumeric + underscore parameter names
///
/// # Arguments
///
/// * `tool_names` - The set of allowed tool names. Must be non-empty.
///
/// # Returns
///
/// A String containing a valid GBNF grammar definition.
pub fn build_tool_call_grammar(tool_names: &[&str]) -> Result<String, String> {
    // SECURITY (P0 #6): Validate tool names before interpolating into GBNF.
    // A malicious tool name containing quotes, pipes, or newlines could inject
    // arbitrary GBNF rules (e.g., `" | ws | "` would add whitespace as a valid tool name).
    let valid_name_re = regex::Regex::new(r"^[a-zA-Z0-9_]+$").unwrap();

    let valid_names: Vec<&str> = tool_names
        .iter()
        .filter(|name| {
            if valid_name_re.is_match(name) {
                true
            } else {
                eprintln!(
                    "[SECURITY] Skipping invalid tool name for GBNF grammar: {:?}",
                    name
                );
                false
            }
        })
        .copied()
        .collect();

    if valid_names.is_empty() {
        return Err("No valid tool names remain after sanitization".to_string());
    }

    // Build a rule that matches one of the allowed tool names.
    let tool_name_alternatives: Vec<String> = valid_names
        .iter()
        .map(|name| format!("\"{}\"", name))
        .collect();
    let tool_name_rule = tool_name_alternatives.join(" | ");

    Ok(format!(
        r#"# GBNF grammar for Qwen 3.5 XML tool calls
# Generated for tools: {tool_list}

root        ::= ws tool-call ws
tool-call   ::= "<tool_call>" ws function ws "</tool_call>"
function    ::= "<function=" tool-name ">" ws params ws "</function>"
tool-name   ::= {tool_name_rule}
params      ::= param (ws param)*
param       ::= "<parameter=" param-name ">" param-value "</parameter>"
param-name  ::= [a-zA-Z_] [a-zA-Z0-9_]*
param-value ::= [^<]*
ws          ::= [ \t\n\r]*
"#,
        tool_list = valid_names.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_tool() {
        let grammar = build_tool_call_grammar(&["bash"]).unwrap();
        assert!(grammar.contains("\"bash\""));
        assert!(grammar.contains("root"));
        assert!(grammar.contains("tool-call"));
        assert!(grammar.contains("<tool_call>"));
        assert!(grammar.contains("</tool_call>"));
    }

    #[test]
    fn test_multiple_tools() {
        let grammar = build_tool_call_grammar(&["bash", "file_read", "grep"]).unwrap();
        assert!(grammar.contains("\"bash\""));
        assert!(grammar.contains("\"file_read\""));
        assert!(grammar.contains("\"grep\""));
        // Should use alternation.
        assert!(grammar.contains(" | "));
    }

    #[test]
    fn test_grammar_structure() {
        let grammar = build_tool_call_grammar(&["file_read"]).unwrap();

        // Verify key structural rules are present.
        assert!(grammar.contains("root"));
        assert!(grammar.contains("tool-call"));
        assert!(grammar.contains("function"));
        assert!(grammar.contains("tool-name"));
        assert!(grammar.contains("params"));
        assert!(grammar.contains("param"));
        assert!(grammar.contains("param-name"));
        assert!(grammar.contains("param-value"));
        assert!(grammar.contains("ws"));
    }

    #[test]
    fn test_param_value_allows_any_non_angle() {
        let grammar = build_tool_call_grammar(&["bash"]).unwrap();
        // param-value should match any character except '<'
        assert!(grammar.contains("[^<]*"));
    }

    #[test]
    fn test_param_name_alphanumeric_underscore() {
        let grammar = build_tool_call_grammar(&["bash"]).unwrap();
        assert!(grammar.contains("[a-zA-Z_]"));
        assert!(grammar.contains("[a-zA-Z0-9_]*"));
    }

    #[test]
    fn test_empty_tool_names_returns_error() {
        let result = build_tool_call_grammar(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_default_tools() {
        let tools = &[
            "bash",
            "file_read",
            "file_write",
            "file_edit",
            "glob",
            "grep",
            "git",
            "web_fetch",
            "ask_user",
            "request_permissions",
        ];
        let grammar = build_tool_call_grammar(tools).unwrap();

        for tool in tools {
            assert!(
                grammar.contains(&format!("\"{tool}\"")),
                "grammar should contain {tool}"
            );
        }
    }

    #[test]
    fn test_comment_lists_tools() {
        let grammar = build_tool_call_grammar(&["bash", "grep"]).unwrap();
        assert!(grammar.contains("bash, grep"));
    }

    #[test]
    fn test_invalid_tool_names_skipped() {
        // Valid name mixed with invalid ones -- only valid name survives.
        let grammar = build_tool_call_grammar(&["bash", "file\"_inject", "valid_tool"]).unwrap();
        assert!(grammar.contains("\"bash\""));
        assert!(grammar.contains("\"valid_tool\""));
        assert!(!grammar.contains("file\"_inject"));
    }

    #[test]
    fn test_all_invalid_tool_names_returns_error() {
        let result = build_tool_call_grammar(&["file\"_inject", "a\nb", "\" | ws | \""]);
        assert!(result.is_err());
    }
}
