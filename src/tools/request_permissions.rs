use anyhow::Result;
use serde_json::Value;

use super::registry::{Tool, ToolContext, ToolResult};

/// Meta-tool the AI calls to request batch pre-flight permission approval.
/// Handled specially in process_response() — the result populates the GrantCache.
pub struct RequestPermissionsTool;

impl Tool for RequestPermissionsTool {
    fn name(&self) -> &str {
        "request_permissions"
    }

    fn description(&self) -> &str {
        "Request batch permission approval for upcoming actions. Call this before starting a multi-step task to get pre-flight approval. Each permission specifies a tool and scope. Destructive actions (rm, kill, sudo, git push) cannot be pre-approved and will require per-action confirmation."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_description": {
                    "type": "string",
                    "description": "Brief description of the overall task requiring these permissions"
                },
                "permissions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": {
                                "type": "string",
                                "description": "Tool name (e.g. file_write, bash, git)"
                            },
                            "scope": {
                                "type": "string",
                                "description": "Scope description (e.g. 'src/' for path prefix, 'cargo' for command prefix, or 'all')"
                            }
                        },
                        "required": ["tool", "scope"]
                    },
                    "description": "List of permissions to request"
                }
            },
            "required": ["task_description", "permissions"]
        })
    }

    fn execute(
        &self,
        _params: Value,
        _ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        // This tool is handled specially in TuiApp::process_response().
        // If it somehow reaches here, return an informational message.
        Box::pin(async {
            Ok(ToolResult::success(
                "Permission request processed. Check grant cache for approved scopes.",
            ))
        })
    }
}
