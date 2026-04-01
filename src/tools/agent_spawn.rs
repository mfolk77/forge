use anyhow::Result;
use serde_json::Value;

use super::registry::{Tool, ToolContext, ToolResult};

/// Subagent spawning tool.
///
/// This tool is registered in the ToolRegistry so the model sees it in tool definitions,
/// but actual execution is handled as a special case in `process_response()` in app.rs.
/// The `execute()` method here is a fallback that returns an error directing the model
/// to try again (it should never be reached in practice).
pub struct AgentSpawnTool;

/// System prompt for subagents.
pub const SUBAGENT_SYSTEM_PROMPT: &str = "\
You are a focused subagent. Your task is described below. You have access to tools for reading files, \
running commands, and searching code. Complete the task thoroughly and provide a clear summary of \
your findings. Do not ask questions -- work autonomously with the information available.";

/// Maximum iterations for a subagent loop.
pub const SUBAGENT_MAX_ITERATIONS: usize = 30;

impl Tool for AgentSpawnTool {
    fn name(&self) -> &str {
        "agent_spawn"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a focused task. The subagent gets fresh context (no parent history), \
         runs its own tool loop (up to 30 iterations), and returns only a text summary. Use for: \
         research, code exploration, focused debugging, file analysis -- any task that would consume \
         too much context if done inline."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Detailed description of what the subagent should accomplish"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: restrict to specific tools (default: all except agent_spawn)"
                }
            }
        })
    }

    fn execute(
        &self,
        _params: Value,
        _ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        // This should never be called — agent_spawn is special-cased in process_response.
        // If it IS called, return an informative error.
        Box::pin(async {
            Ok(ToolResult::error(
                "agent_spawn must be handled by the agent loop, not executed directly. \
                 This is an internal error — please report it."
            ))
        })
    }
}

/// Build the initial message list for a subagent.
pub fn build_subagent_messages(task: &str) -> Vec<crate::backend::types::Message> {
    vec![
        crate::backend::types::Message {
            role: crate::backend::types::Role::System,
            content: SUBAGENT_SYSTEM_PROMPT.to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
        crate::backend::types::Message {
            role: crate::backend::types::Role::User,
            content: task.to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
    ]
}

/// Validate subagent parameters. Returns an error string if invalid, None if OK.
pub fn validate_params(params: &Value) -> Option<String> {
    let task = params["task"].as_str().unwrap_or("");
    if task.is_empty() {
        return Some("Missing required parameter: task".to_string());
    }
    if task.len() > 10_000 {
        return Some(format!(
            "Task description too long ({} chars, max 10000).",
            task.len()
        ));
    }
    if task.contains('\0') {
        return Some("Task must not contain null bytes.".to_string());
    }

    // Validate tool names if provided
    if let Some(tools) = params["tools"].as_array() {
        for t in tools {
            if let Some(name) = t.as_str() {
                if name == "agent_spawn" {
                    return Some(
                        "Subagents cannot use agent_spawn (recursion not allowed).".to_string(),
                    );
                }
                if name.contains('/') || name.contains('\\') || name.contains("..") {
                    return Some(format!("Invalid tool name: {name}"));
                }
            }
        }
    }

    None
}

/// Filter tool definitions to exclude agent_spawn and optionally restrict to a set.
pub fn filter_tools(
    all_tools: &[crate::backend::types::ToolDefinition],
    requested: Option<&[String]>,
) -> Vec<crate::backend::types::ToolDefinition> {
    all_tools
        .iter()
        .filter(|t| {
            // Always exclude agent_spawn
            if t.name == "agent_spawn" {
                return false;
            }
            // If specific tools requested, only include those
            if let Some(requested) = requested {
                return requested.iter().any(|r| r == &t.name);
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::{Role, ToolDefinition};

    // ── Message building tests ─────────────────────────────────────────────

    #[test]
    fn test_build_subagent_messages() {
        let messages = build_subagent_messages("Find all TODO comments");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("focused subagent"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "Find all TODO comments");
    }

    // ── Validation tests ───────────────────────────────────────────────────

    #[test]
    fn test_validate_empty_task() {
        let err = validate_params(&serde_json::json!({"task": ""}));
        assert!(err.is_some());
        assert!(err.unwrap().contains("Missing"));
    }

    #[test]
    fn test_validate_valid_task() {
        let err = validate_params(&serde_json::json!({"task": "Search for auth patterns"}));
        assert!(err.is_none());
    }

    #[test]
    fn test_validate_rejects_agent_spawn_in_tools() {
        let err = validate_params(&serde_json::json!({
            "task": "test",
            "tools": ["bash", "agent_spawn"]
        }));
        assert!(err.is_some());
        assert!(err.unwrap().contains("recursion"));
    }

    #[test]
    fn test_validate_rejects_null_bytes() {
        let err = validate_params(&serde_json::json!({"task": "test\u{0000}bad"}));
        assert!(err.is_some());
    }

    #[test]
    fn test_validate_rejects_oversized_task() {
        let big = "x".repeat(11_000);
        let err = validate_params(&serde_json::json!({"task": big}));
        assert!(err.is_some());
        assert!(err.unwrap().contains("too long"));
    }

    // ── Tool filtering tests ───────────────────────────────────────────────

    #[test]
    fn test_filter_tools_excludes_agent_spawn() {
        let tools = vec![
            ToolDefinition {
                name: "bash".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "agent_spawn".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "file_read".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
        ];

        let filtered = filter_tools(&tools, None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.name != "agent_spawn"));
    }

    #[test]
    fn test_filter_tools_with_requested_subset() {
        let tools = vec![
            ToolDefinition {
                name: "bash".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "file_read".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "grep".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
        ];

        let requested = vec!["bash".to_string(), "file_read".to_string()];
        let filtered = filter_tools(&tools, Some(&requested));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|t| t.name == "bash"));
        assert!(filtered.iter().any(|t| t.name == "file_read"));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_tool_name_path_traversal_rejected() {
        let err = validate_params(&serde_json::json!({
            "task": "test",
            "tools": ["../../../etc/passwd"]
        }));
        assert!(err.is_some());
        assert!(err.unwrap().contains("Invalid tool name"));
    }

    #[test]
    fn test_security_no_recursion() {
        // Verify agent_spawn is always excluded from subagent tool list
        let tools = vec![
            ToolDefinition {
                name: "agent_spawn".to_string(),
                description: "".to_string(),
                parameters: serde_json::json!({}),
            },
        ];

        let filtered = filter_tools(&tools, None);
        assert!(filtered.is_empty());

        // Even if explicitly requested
        let requested = vec!["agent_spawn".to_string()];
        let filtered = filter_tools(&tools, Some(&requested));
        assert!(filtered.is_empty());
    }
}
