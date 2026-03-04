use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct FileEditTool;

impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match with new content."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path", "old_string", "new_string"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
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
        let old_string = params["old_string"].as_str().unwrap_or("").to_string();
        let new_string = params["new_string"].as_str().unwrap_or("").to_string();
        let replace_all = params["replace_all"].as_bool().unwrap_or(false);
        let cwd = ctx.cwd.clone();

        Box::pin(async move {
            if path_str.is_empty() {
                return Ok(ToolResult::error("No path provided"));
            }
            if old_string.is_empty() {
                return Ok(ToolResult::error("old_string cannot be empty"));
            }
            if old_string == new_string {
                return Ok(ToolResult::error("old_string and new_string are identical"));
            }

            let path = if path_str.starts_with('/') {
                PathBuf::from(&path_str)
            } else {
                cwd.join(&path_str)
            };

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!("Failed to read file: {e}"))),
            };

            let count = content.matches(&old_string).count();
            if count == 0 {
                return Ok(ToolResult::error(
                    "old_string not found in file. Make sure the string matches exactly.",
                ));
            }

            if !replace_all && count > 1 {
                return Ok(ToolResult::error(format!(
                    "old_string found {count} times. Provide more context to make it unique, or set replace_all: true."
                )));
            }

            let new_content = if replace_all {
                content.replace(&old_string, &new_string)
            } else {
                content.replacen(&old_string, &new_string, 1)
            };

            match std::fs::write(&path, &new_content) {
                Ok(_) => Ok(ToolResult::success(format!(
                    "Replaced {count} occurrence(s) in {}",
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
    use tempfile::NamedTempFile;
    use std::io::Write;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn test_edit_unique_match() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "fn hello() {{}}\nfn world() {{}}").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let tool = FileEditTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path,
                    "old_string": "fn hello() {}",
                    "new_string": "fn hello() { println!(\"hi\"); }"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("println!"));
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello world").unwrap();

        let tool = FileEditTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": f.path().to_str().unwrap(),
                    "old_string": "nonexistent",
                    "new_string": "replacement"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_ambiguous() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "foo bar foo bar").unwrap();

        let tool = FileEditTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": f.path().to_str().unwrap(),
                    "old_string": "foo",
                    "new_string": "baz"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("2 times"));
    }

    #[tokio::test]
    async fn test_edit_replace_all() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "foo bar foo bar").unwrap();

        let tool = FileEditTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": f.path().to_str().unwrap(),
                    "old_string": "foo",
                    "new_string": "baz",
                    "replace_all": true
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(content, "baz bar baz bar");
    }
}
