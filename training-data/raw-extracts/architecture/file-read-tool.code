use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct FileReadTool;

impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read a file's contents. Supports optional line offset and limit."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of lines to read"
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
        let offset = params["offset"].as_u64().map(|v| v as usize);
        let limit = params["limit"].as_u64().map(|v| v as usize);
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

            if !path.exists() {
                return Ok(ToolResult::error(format!("File not found: {}", path.display())));
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!("Failed to read file: {e}"))),
            };

            let lines: Vec<&str> = content.lines().collect();
            let start = offset.unwrap_or(1).saturating_sub(1);
            let end = limit
                .map(|l| (start + l).min(lines.len()))
                .unwrap_or(lines.len());

            let mut output = String::new();
            for (i, line) in lines[start..end].iter().enumerate() {
                let line_num = start + i + 1;
                output.push_str(&format!("{line_num:>6}\t{line}\n"));
            }

            if output.is_empty() {
                Ok(ToolResult::success("(empty file)"))
            } else {
                Ok(ToolResult::success(output))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn test_read_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "line 3").unwrap();

        let tool = FileReadTool;
        let result = tool
            .execute(
                serde_json::json!({"path": f.path().to_str().unwrap()}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 3"));
    }

    #[tokio::test]
    async fn test_read_with_offset_and_limit() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(f, "line {i}").unwrap();
        }

        let tool = FileReadTool;
        let result = tool
            .execute(
                serde_json::json!({"path": f.path().to_str().unwrap(), "offset": 3, "limit": 2}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("line 3"));
        assert!(result.output.contains("line 4"));
        assert!(!result.output.contains("line 5"));
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let tool = FileReadTool;
        let result = tool
            .execute(
                serde_json::json!({"path": "/nonexistent/file.txt"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }
}
