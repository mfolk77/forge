use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct FileWriteTool;

impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let path_str = params["path"].as_str().unwrap_or("").to_string();
        let content = params["content"].as_str().unwrap_or("").to_string();
        let cwd = ctx.cwd.clone();

        Box::pin(async move {
            if path_str.is_empty() {
                return Ok(ToolResult::error("No path provided"));
            }

            let path = if path_str.starts_with('/') {
                PathBuf::from(&path_str)
            } else {
                cwd.join(&path_str)
            };

            // Create parent directories
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Ok(ToolResult::error(format!(
                        "Failed to create directories: {e}"
                    )));
                }
            }

            match std::fs::write(&path, &content) {
                Ok(_) => Ok(ToolResult::success(format!(
                    "Wrote {} bytes to {}",
                    content.len(),
                    path.display()
                ))),
                Err(e) => Ok(ToolResult::error(format!("Failed to write file: {e}"))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn test_write_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        let tool = FileWriteTool;
        let result = tool
            .execute(
                serde_json::json!({"path": path.to_str().unwrap(), "content": "hello world"}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("sub").join("dir").join("test.txt");

        let tool = FileWriteTool;
        let result = tool
            .execute(
                serde_json::json!({"path": path.to_str().unwrap(), "content": "nested"}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(path.exists());
    }
}
