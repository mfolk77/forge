use std::collections::HashMap;

use crate::conversation::adapter::ParsedToolCall;

/// Result of validating a parsed tool call against known schemas.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    /// The tool call is valid and ready for execution.
    Valid,
    /// The tool name does not match any registered tool.
    UnknownTool(String),
    /// A required parameter is missing.
    MissingParam(String),
    /// A parameter has the wrong type.
    InvalidParamType {
        param: String,
        expected: String,
        got: String,
    },
}

/// Schema information for a single tool parameter.
#[derive(Debug, Clone)]
pub struct ParamSchema {
    pub param_type: String,
    pub required: bool,
}

/// Schema for a tool, used for validation.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub params: HashMap<String, ParamSchema>,
}

/// Validates tool calls against registered tool schemas.
#[derive(Debug)]
pub struct ToolCallValidator {
    schemas: HashMap<String, ToolSchema>,
}

impl ToolCallValidator {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
        }
    }

    /// Register a tool with its parameter schema.
    ///
    /// `schema` is expected to be a JSON Schema object with "properties" and
    /// optional "required" array, matching the format in `ToolDefinition.parameters`.
    pub fn register_tool(&mut self, name: &str, schema: &serde_json::Value) {
        let mut params = HashMap::new();

        let required_list: Vec<String> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, val) in props {
                let param_type = val
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("string")
                    .to_string();
                let required = required_list.contains(key);
                params.insert(
                    key.clone(),
                    ParamSchema {
                        param_type,
                        required,
                    },
                );
            }
        }

        self.schemas.insert(name.to_string(), ToolSchema { params });
    }

    /// Validate a parsed tool call.
    pub fn validate(&self, call: &ParsedToolCall) -> ValidationResult {
        let Some(schema) = self.schemas.get(&call.name) else {
            return ValidationResult::UnknownTool(call.name.clone());
        };

        // Check required params.
        for (param_name, param_schema) in &schema.params {
            if param_schema.required && !call.arguments.contains_key(param_name) {
                return ValidationResult::MissingParam(param_name.clone());
            }
        }

        // Check types of provided params.
        for (param_name, value) in &call.arguments {
            if let Some(param_schema) = schema.params.get(param_name) {
                let actual_type = json_value_type(value);
                if !types_compatible(&param_schema.param_type, &actual_type) {
                    return ValidationResult::InvalidParamType {
                        param: param_name.clone(),
                        expected: param_schema.param_type.clone(),
                        got: actual_type,
                    };
                }
            }
            // Unknown params are allowed -- the model may send extra fields.
        }

        ValidationResult::Valid
    }

    /// Attempt to repair common errors in a tool call, mutating it in place.
    ///
    /// Repairs applied:
    /// - Trim whitespace from string values
    /// - Convert `"true"` / `"false"` strings to JSON booleans
    /// - Unquote double-quoted string values (`"\"hello\""` -> `"hello"`)
    /// - Fix common parameter name typos
    pub fn repair_common_errors(&self, call: &mut ParsedToolCall) {
        // Parameter name typo mapping.
        let typo_fixes: HashMap<&str, &str> = HashMap::from([
            ("file_path", "path"),
            ("filepath", "path"),
            ("file", "path"),
            ("filename", "path"),
            ("cmd", "command"),
            ("dir", "path"),
            ("directory", "path"),
            ("search", "pattern"),
            ("query", "pattern"),
            ("text", "content"),
            ("data", "content"),
            ("old", "old_string"),
            ("new", "new_string"),
        ]);

        // Only apply name fixes if the tool schema exists and the typo key is not
        // already a valid parameter for this tool.
        if let Some(schema) = self.schemas.get(&call.name) {
            let mut renames = Vec::new();
            for key in call.arguments.keys() {
                if !schema.params.contains_key(key) {
                    if let Some(&correct) = typo_fixes.get(key.as_str()) {
                        if schema.params.contains_key(correct)
                            && !call.arguments.contains_key(correct)
                        {
                            renames.push((key.clone(), correct.to_string()));
                        }
                    }
                }
            }
            for (old_key, new_key) in renames {
                if let Some(val) = call.arguments.remove(&old_key) {
                    call.arguments.insert(new_key, val);
                }
            }
        }

        // Value-level repairs.
        let keys: Vec<String> = call.arguments.keys().cloned().collect();
        for key in keys {
            let Some(val) = call.arguments.get_mut(&key) else {
                continue;
            };

            if let Some(s) = val.as_str() {
                let trimmed = s.trim();

                // "true" / "false" -> boolean
                if trimmed.eq_ignore_ascii_case("true") {
                    *val = serde_json::Value::Bool(true);
                    continue;
                }
                if trimmed.eq_ignore_ascii_case("false") {
                    *val = serde_json::Value::Bool(false);
                    continue;
                }

                // Unquote double-quoted values: "\"hello\"" -> "hello"
                if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
                    let inner = &trimmed[1..trimmed.len() - 1];
                    *val = serde_json::Value::String(inner.to_string());
                    continue;
                }

                // Trim whitespace.
                if trimmed != s {
                    *val = serde_json::Value::String(trimmed.to_string());
                }
            }
        }
    }
}

impl Default for ToolCallValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a JSON value to its type name.
fn json_value_type(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(_) => "boolean".to_string(),
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer".to_string()
            } else {
                "number".to_string()
            }
        }
        serde_json::Value::String(_) => "string".to_string(),
        serde_json::Value::Array(_) => "array".to_string(),
        serde_json::Value::Object(_) => "object".to_string(),
    }
}

/// Check if a JSON value type is compatible with an expected schema type.
fn types_compatible(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    // "number" accepts "integer"
    if expected == "number" && actual == "integer" {
        return true;
    }
    // Strings are broadly accepted (models often stringify everything)
    if actual == "string" {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validator() -> ToolCallValidator {
        let mut v = ToolCallValidator::new();
        v.register_tool(
            "file_read",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "required": ["path"]
            }),
        );
        v.register_tool(
            "bash",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "background": { "type": "boolean" }
                },
                "required": ["command"]
            }),
        );
        v.register_tool(
            "file_edit",
            &serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        );
        v
    }

    #[test]
    fn test_valid_call() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([(
                "path".to_string(),
                serde_json::Value::String("/src/main.rs".to_string()),
            )]),
            raw_text: String::new(),
        };
        assert_eq!(v.validate(&call), ValidationResult::Valid);
    }

    #[test]
    fn test_unknown_tool() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "nonexistent".to_string(),
            arguments: HashMap::new(),
            raw_text: String::new(),
        };
        assert_eq!(
            v.validate(&call),
            ValidationResult::UnknownTool("nonexistent".to_string())
        );
    }

    #[test]
    fn test_missing_required_param() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::new(), // missing "path"
            raw_text: String::new(),
        };
        assert_eq!(
            v.validate(&call),
            ValidationResult::MissingParam("path".to_string())
        );
    }

    #[test]
    fn test_invalid_param_type() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([
                (
                    "path".to_string(),
                    serde_json::Value::String("/foo".to_string()),
                ),
                ("offset".to_string(), serde_json::Value::Bool(true)),
            ]),
            raw_text: String::new(),
        };
        assert_eq!(
            v.validate(&call),
            ValidationResult::InvalidParamType {
                param: "offset".to_string(),
                expected: "integer".to_string(),
                got: "boolean".to_string(),
            }
        );
    }

    #[test]
    fn test_optional_param_missing_is_ok() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([(
                "path".to_string(),
                serde_json::Value::String("/foo".to_string()),
            )]),
            raw_text: String::new(),
        };
        // offset and limit are optional
        assert_eq!(v.validate(&call), ValidationResult::Valid);
    }

    #[test]
    fn test_extra_params_allowed() {
        let v = make_validator();
        let call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([
                (
                    "path".to_string(),
                    serde_json::Value::String("/foo".to_string()),
                ),
                (
                    "unknown_param".to_string(),
                    serde_json::Value::String("val".to_string()),
                ),
            ]),
            raw_text: String::new(),
        };
        assert_eq!(v.validate(&call), ValidationResult::Valid);
    }

    // -----------------------------------------------------------------------
    // Repair tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_repair_whitespace_trim() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([(
                "path".to_string(),
                serde_json::Value::String("  /src/main.rs  ".to_string()),
            )]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        assert_eq!(
            call.arguments.get("path").unwrap(),
            &serde_json::Value::String("/src/main.rs".to_string())
        );
    }

    #[test]
    fn test_repair_string_to_bool() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "bash".to_string(),
            arguments: HashMap::from([
                (
                    "command".to_string(),
                    serde_json::Value::String("ls".to_string()),
                ),
                (
                    "background".to_string(),
                    serde_json::Value::String("true".to_string()),
                ),
            ]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        assert_eq!(
            call.arguments.get("background").unwrap(),
            &serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn test_repair_unquote() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([(
                "path".to_string(),
                serde_json::Value::String("\"/src/main.rs\"".to_string()),
            )]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        assert_eq!(
            call.arguments.get("path").unwrap(),
            &serde_json::Value::String("/src/main.rs".to_string())
        );
    }

    #[test]
    fn test_repair_param_name_typo() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([(
                "file_path".to_string(),
                serde_json::Value::String("/src/main.rs".to_string()),
            )]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        assert!(call.arguments.contains_key("path"));
        assert!(!call.arguments.contains_key("file_path"));
    }

    #[test]
    fn test_repair_no_rename_if_correct_key_exists() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "file_read".to_string(),
            arguments: HashMap::from([
                (
                    "path".to_string(),
                    serde_json::Value::String("/real.rs".to_string()),
                ),
                (
                    "file_path".to_string(),
                    serde_json::Value::String("/wrong.rs".to_string()),
                ),
            ]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        // "file_path" should NOT overwrite existing "path".
        assert_eq!(
            call.arguments.get("path").unwrap(),
            &serde_json::Value::String("/real.rs".to_string())
        );
    }

    #[test]
    fn test_repair_edit_typos() {
        let v = make_validator();
        let mut call = ParsedToolCall {
            name: "file_edit".to_string(),
            arguments: HashMap::from([
                (
                    "file_path".to_string(),
                    serde_json::Value::String("/lib.rs".to_string()),
                ),
                (
                    "old".to_string(),
                    serde_json::Value::String("fn old() {}".to_string()),
                ),
                (
                    "new".to_string(),
                    serde_json::Value::String("fn new() {}".to_string()),
                ),
            ]),
            raw_text: String::new(),
        };

        v.repair_common_errors(&mut call);
        assert!(call.arguments.contains_key("path"));
        assert!(call.arguments.contains_key("old_string"));
        assert!(call.arguments.contains_key("new_string"));
    }
}
