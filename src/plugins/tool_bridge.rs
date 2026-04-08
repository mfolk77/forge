use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::backend::types::ToolDefinition;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

const PLUGIN_TOOL_TIMEOUT_MS: u64 = 30_000;

/// A tool provided by a plugin, executed by shelling out to the plugin's command.
pub struct PluginTool {
    tool_name: String,
    tool_description: String,
    command_path: PathBuf,
    plugin_dir: PathBuf,
    params_schema: Value,
}

impl PluginTool {
    pub fn new(
        name: String,
        description: String,
        command: String,
        plugin_dir: PathBuf,
        params: Value,
    ) -> Self {
        let command_path = plugin_dir.join(&command);
        Self {
            tool_name: name,
            tool_description: description,
            command_path,
            plugin_dir,
            params_schema: params,
        }
    }

    #[allow(dead_code)]
    pub fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.tool_name.clone(),
            description: self.tool_description.clone(),
            parameters: self.params_schema.clone(),
        }
    }
}

impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters(&self) -> Value {
        self.params_schema.clone()
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let params_json = serde_json::to_string(&params).unwrap_or_default();
        let project_path = ctx.project_path.to_string_lossy().to_string();

        Box::pin(async move {
            // Verify command exists and is within plugin dir
            let canonical_cmd = match self.command_path.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "Plugin tool command not found: {}",
                        self.command_path.display()
                    )));
                }
            };
            let canonical_plugin = match self.plugin_dir.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    return Ok(ToolResult::error("Plugin directory not found"));
                }
            };

            if !canonical_cmd.starts_with(&canonical_plugin) {
                return Ok(ToolResult::error("Plugin tool command escapes plugin directory"));
            }

            let result = timeout(
                Duration::from_millis(PLUGIN_TOOL_TIMEOUT_MS),
                Command::new(&self.command_path)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .current_dir(&self.plugin_dir)
                    .env("FTAI_PARAMS", &params_json)
                    .env("FTAI_PROJECT_PATH", &project_path)
                    .env("FTAI_PLUGIN_DIR", self.plugin_dir.to_string_lossy().as_ref())
                    .kill_on_drop(true)
                    .output(),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                    if output.status.success() {
                        Ok(ToolResult::success(if stdout.is_empty() {
                            "(no output)".to_string()
                        } else {
                            stdout
                        }))
                    } else {
                        let mut msg = format!("Exit code {}", output.status.code().unwrap_or(-1));
                        if !stdout.is_empty() {
                            msg.push_str(&format!("\n{stdout}"));
                        }
                        if !stderr.is_empty() {
                            msg.push_str(&format!("\nstderr: {stderr}"));
                        }
                        Ok(ToolResult::error(msg))
                    }
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute plugin tool: {e}"))),
                Err(_) => Ok(ToolResult::error(format!(
                    "Plugin tool timed out after {PLUGIN_TOOL_TIMEOUT_MS}ms"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ctx() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            project_path: std::env::temp_dir(),
        }
    }

    /// Platform-aware script name and content for a tool that prints output.
    #[cfg(unix)]
    fn hello_script() -> (&'static str, &'static str) {
        ("hello.sh", "#!/bin/bash\necho \"hello from plugin\"")
    }

    #[cfg(windows)]
    fn hello_script() -> (&'static str, &'static str) {
        ("hello.bat", "@echo off\r\necho hello from plugin")
    }

    #[tokio::test]
    async fn test_plugin_tool_execution() {
        let tmp = TempDir::new().unwrap();
        let tool_dir = tmp.path().join("tools");
        std::fs::create_dir_all(&tool_dir).unwrap();

        let (script_name, script_content) = hello_script();
        let script = tool_dir.join(script_name);
        std::fs::write(&script, script_content).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tool = PluginTool::new(
            "hello".to_string(),
            "Says hello".to_string(),
            format!("tools/{script_name}"),
            tmp.path().to_path_buf(),
            serde_json::json!({"type": "object"}),
        );

        let result = tool.execute(serde_json::json!({}), &make_ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello from plugin"));
    }

    #[tokio::test]
    async fn test_plugin_tool_not_found() {
        let tmp = TempDir::new().unwrap();
        let missing_ext = if cfg!(windows) { "nope.bat" } else { "nope.sh" };
        let tool = PluginTool::new(
            "missing".to_string(),
            "Missing tool".to_string(),
            format!("tools/{missing_ext}"),
            tmp.path().to_path_buf(),
            serde_json::json!({"type": "object"}),
        );

        let result = tool.execute(serde_json::json!({}), &make_ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }
}
