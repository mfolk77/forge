use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use tokio::process::Command;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct GitTool;

impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Execute git operations: status, diff, log, commit, branch, push, pr_create."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "commit", "branch", "push", "add", "pr_create"],
                    "description": "The git action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for commit action)"
                },
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Files to stage (for add action)"
                },
                "branch_name": {
                    "type": "string",
                    "description": "Branch name (for branch action)"
                },
                "args": {
                    "type": "string",
                    "description": "Additional arguments for the git command"
                },
                "body": {
                    "type": "string",
                    "description": "PR body/description (for pr_create action)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let action = params["action"].as_str().unwrap_or("").to_string();
        let message = params["message"].as_str().map(String::from);
        let files = params["files"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            });
        let branch_name = params["branch_name"].as_str().map(String::from);
        let body = params["body"].as_str().map(String::from);
        let extra_args = params["args"].as_str().map(String::from);
        let cwd = ctx.cwd.clone();

        Box::pin(async move {
            match action.as_str() {
                "status" => run_git(&cwd, &["status", "-u"]).await,
                "diff" => {
                    let args = extra_args.as_deref().unwrap_or("HEAD");
                    run_git(&cwd, &["diff", args]).await
                }
                "log" => {
                    let default = "--oneline -20".to_string();
                    let args = extra_args.as_deref().unwrap_or(&default);
                    let parts: Vec<&str> = args.split_whitespace().collect();
                    let mut cmd = vec!["log"];
                    cmd.extend(parts);
                    run_git(&cwd, &cmd).await
                }
                "add" => {
                    if let Some(files) = files {
                        let mut cmd = vec!["add".to_string()];
                        cmd.extend(files);
                        let refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
                        run_git(&cwd, &refs).await
                    } else {
                        Ok(ToolResult::error("No files specified for git add"))
                    }
                }
                "commit" => {
                    if let Some(msg) = message {
                        run_git(&cwd, &["commit", "-m", &msg]).await
                    } else {
                        Ok(ToolResult::error("No commit message provided"))
                    }
                }
                "branch" => {
                    if let Some(name) = branch_name {
                        run_git(&cwd, &["checkout", "-b", &name]).await
                    } else {
                        run_git(&cwd, &["branch", "-a"]).await
                    }
                }
                "push" => {
                    let args = extra_args.as_deref().unwrap_or("-u origin HEAD");
                    let parts: Vec<&str> = args.split_whitespace().collect();
                    let mut cmd = vec!["push"];
                    cmd.extend(parts);
                    run_git(&cwd, &cmd).await
                }
                "pr_create" => {
                    // Use gh CLI
                    let title = message.unwrap_or_else(|| "New PR".to_string());
                    let mut args = vec![
                        "pr".to_string(),
                        "create".to_string(),
                        "--title".to_string(),
                        title,
                    ];
                    if let Some(ref body_text) = body {
                        args.push("--body".to_string());
                        args.push(body_text.clone());
                    } else {
                        args.push("--fill".to_string());
                    }
                    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    let output = Command::new("gh")
                        .args(&arg_refs)
                        .current_dir(&cwd)
                        .output()
                        .await;

                    match output {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout);
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            if out.status.success() {
                                Ok(ToolResult::success(stdout.to_string()))
                            } else {
                                Ok(ToolResult::error(format!("{stdout}\n{stderr}")))
                            }
                        }
                        Err(e) => Ok(ToolResult::error(format!(
                            "gh CLI not found: {e}. Install: brew install gh"
                        ))),
                    }
                }
                _ => Ok(ToolResult::error(format!("Unknown git action: {action}"))),
            }
        })
    }
}

async fn run_git(cwd: &PathBuf, args: &[&str]) -> Result<ToolResult> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                Ok(ToolResult::success(if stdout.is_empty() {
                    "(no output)".to_string()
                } else {
                    stdout.to_string()
                }))
            } else {
                Ok(ToolResult::error(format!("{stdout}\n{stderr}")))
            }
        }
        Err(e) => Ok(ToolResult::error(format!("Failed to run git: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn git_ctx() -> (TempDir, ToolContext) {
        let tmp = TempDir::new().unwrap();
        // Init a git repo
        Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(tmp.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(tmp.path())
            .output()
            .await
            .unwrap();

        let ctx = ToolContext {
            cwd: tmp.path().to_path_buf(),
            project_path: tmp.path().to_path_buf(),
        };
        (tmp, ctx)
    }

    #[tokio::test]
    async fn test_git_status() {
        let (_tmp, ctx) = git_ctx().await;
        let tool = GitTool;
        let result = tool
            .execute(serde_json::json!({"action": "status"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_git_add_and_commit() {
        let (tmp, ctx) = git_ctx().await;
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

        let tool = GitTool;

        // Add
        let result = tool
            .execute(
                serde_json::json!({"action": "add", "files": ["test.txt"]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        // Commit
        let result = tool
            .execute(
                serde_json::json!({"action": "commit", "message": "Initial commit"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_git_unknown_action() {
        let (_tmp, ctx) = git_ctx().await;
        let tool = GitTool;
        let result = tool
            .execute(serde_json::json!({"action": "rebase_interactive"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
    }
}
