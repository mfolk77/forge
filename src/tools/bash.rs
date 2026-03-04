use anyhow::Result;
use serde_json::Value;
use std::sync::Mutex;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::registry::{Tool, ToolContext, ToolResult};

pub struct BashTool {
    cwd: Mutex<Option<PathBuf>>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            cwd: Mutex::new(None),
        }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command. Working directory persists between calls."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 120000, max: 600000)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let command = params["command"].as_str().unwrap_or("").to_string();
        let timeout_ms = params["timeout_ms"].as_u64().unwrap_or(120_000).min(600_000);
        let project_path = ctx.cwd.clone();

        Box::pin(async move {
            if command.is_empty() {
                return Ok(ToolResult::error("No command provided"));
            }

            let working_dir = {
                let guard = self.cwd.lock().unwrap();
                guard.clone().unwrap_or(project_path)
            };

            let result = timeout(
                Duration::from_millis(timeout_ms),
                Command::new("bash")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&working_dir)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output(),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    // Update cwd if the command was a cd
                    if command.starts_with("cd ") {
                        let dir = command.trim_start_matches("cd ").trim();
                        let new_dir = if dir.starts_with('/') {
                            PathBuf::from(dir)
                        } else {
                            working_dir.join(dir)
                        };
                        if new_dir.exists() {
                            *self.cwd.lock().unwrap() = Some(new_dir);
                        }
                    }

                    let mut result = String::new();
                    if !stdout.is_empty() {
                        result.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&format!("stderr: {stderr}"));
                    }

                    if output.status.success() {
                        Ok(ToolResult::success(if result.is_empty() {
                            "(no output)".to_string()
                        } else {
                            result
                        }))
                    } else {
                        Ok(ToolResult::error(format!(
                            "Exit code {}\n{result}",
                            output.status.code().unwrap_or(-1)
                        )))
                    }
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute: {e}"))),
                Err(_) => Ok(ToolResult::error(format!(
                    "Command timed out after {timeout_ms}ms"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn test_echo() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_failing_command() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "false"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_empty_command() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": ""}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_timeout() {
        let tool = BashTool::new();
        let result = tool
            .execute(
                serde_json::json!({"command": "sleep 10", "timeout_ms": 100}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }
}
