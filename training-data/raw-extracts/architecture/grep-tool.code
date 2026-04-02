use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search (defaults to project root)"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob filter for files (e.g., '*.rs')"
                },
                "context": {
                    "type": "integer",
                    "description": "Lines of context around each match"
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
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.cwd.clone());
        let glob_filter = params["glob"].as_str().map(String::from);
        let context_lines = params["context"].as_u64().unwrap_or(0) as usize;

        Box::pin(async move {
            if pattern.is_empty() {
                return Ok(ToolResult::error("No pattern provided"));
            }

            let re = match Regex::new(&pattern) {
                Ok(r) => r,
                Err(e) => return Ok(ToolResult::error(format!("Invalid regex: {e}"))),
            };

            let mut results = Vec::new();
            search_recursive(&search_path, &re, &glob_filter, context_lines, &mut results, true);

            if results.is_empty() {
                Ok(ToolResult::success("No matches found"))
            } else {
                // Limit output
                let truncated = results.len() > 100;
                let output: Vec<&str> = results.iter().take(100).map(|s| s.as_str()).collect();
                let mut out = output.join("\n");
                if truncated {
                    out.push_str(&format!("\n... ({} more matches)", results.len() - 100));
                }
                Ok(ToolResult::success(out))
            }
        })
    }
}

fn search_recursive(
    path: &std::path::Path,
    re: &Regex,
    glob_filter: &Option<String>,
    context: usize,
    results: &mut Vec<String>,
    is_root: bool,
) {
    if path.is_file() {
        if let Some(filter) = glob_filter {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if let Ok(pat) = glob::Pattern::new(filter) {
                if !pat.matches(&name) {
                    return;
                }
            }
        }
        search_file(path, re, context, results);
    } else if path.is_dir() {
        // Skip hidden dirs and common noise (but not the root search path)
        if !is_root {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
                return;
            }
        }

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                search_recursive(&entry.path(), re, glob_filter, context, results, false);
            }
        }
    }
}

fn search_file(path: &std::path::Path, re: &Regex, context: usize, results: &mut Vec<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // Skip binary/unreadable files
    };

    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let start = i.saturating_sub(context);
            let end = (i + context + 1).min(lines.len());

            results.push(format!("{}:{}:{}", path.display(), i + 1, line));

            if context > 0 {
                for j in start..end {
                    if j != i {
                        let prefix = if j < i { "-" } else { "+" };
                        results.push(format!(
                            "{}:{}:{} {}",
                            path.display(),
                            j + 1,
                            prefix,
                            lines[j]
                        ));
                    }
                }
            }
        }
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
    async fn test_grep_basic() {
        let tmp = TempDir::new().unwrap();
        let subdir = tmp.path().join("src");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("test.rs"), "fn hello() {}\nfn world() {}").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "hello", "path": subdir.to_str().unwrap()}),
                &ctx_with(tmp.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("hello"), "output was: {}", result.output);
    }

    #[tokio::test]
    async fn test_grep_with_glob() {
        let tmp = TempDir::new().unwrap();
        let subdir = tmp.path().join("src");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("a.rs"), "fn target()").unwrap();
        std::fs::write(subdir.join("b.txt"), "fn target()").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "target", "glob": "*.rs", "path": subdir.to_str().unwrap()}),
                &ctx_with(tmp.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("a.rs"), "output was: {}", result.output);
        assert!(!result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_grep_no_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("test.rs"), "hello world").unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(
                serde_json::json!({"pattern": "nonexistent"}),
                &ctx_with(tmp.path()),
            )
            .await
            .unwrap();

        assert!(result.output.contains("No matches"));
    }
}
