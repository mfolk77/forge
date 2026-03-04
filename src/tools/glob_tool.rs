use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g., '**/*.rs', 'src/**/*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to project root)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let pattern = params["pattern"].as_str().unwrap_or("").to_string();
        let search_path = params["path"]
            .as_str()
            .map(|s| PathBuf::from(s))
            .unwrap_or_else(|| ctx.cwd.clone());

        Box::pin(async move {
            if pattern.is_empty() {
                return Ok(ToolResult::error("No pattern provided"));
            }

            let full_pattern = search_path.join(&pattern);
            let glob_str = full_pattern.to_string_lossy().to_string();

            match glob::glob(&glob_str) {
                Ok(entries) => {
                    let mut files: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();

                    files.sort();

                    if files.is_empty() {
                        Ok(ToolResult::success("No files matched"))
                    } else {
                        Ok(ToolResult::success(files.join("\n")))
                    }
                }
                Err(e) => Ok(ToolResult::error(format!("Invalid glob pattern: {e}"))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx_with(path: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: path.to_path_buf(),
            project_path: path.to_path_buf(),
        }
    }

    #[tokio::test]
    async fn test_glob_find_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "").unwrap();
        std::fs::write(tmp.path().join("c.txt"), "").unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "*.rs"}),
                &ctx_with(tmp.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("b.rs"));
        assert!(!result.output.contains("c.txt"));
    }

    #[tokio::test]
    async fn test_glob_no_match() {
        let tmp = TempDir::new().unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "*.xyz"}),
                &ctx_with(tmp.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("No files matched"));
    }
}
