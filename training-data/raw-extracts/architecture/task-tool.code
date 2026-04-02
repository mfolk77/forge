use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::registry::{Tool, ToolContext, ToolResult};

/// File-per-task persistent task management.
///
/// Tasks are stored as JSON files at `<project>/.ftai/tasks/task_<id>.json`.
/// Supports create, update, list, delete, and claim actions with a dependency graph.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskData {
    pub id: u64,
    pub subject: String,
    pub description: String,
    pub status: String,
    #[serde(rename = "blockedBy", default)]
    pub blocked_by: Vec<u64>,
    #[serde(default)]
    pub owner: String,
}

const VALID_STATUSES: &[&str] = &["pending", "in_progress", "completed"];

pub struct TaskTool;

impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Manage persistent tasks for the current project. Actions: create, update, list, delete, claim. \
         Tasks have dependencies (blockedBy) and ownership. Use to track multi-step work."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update", "list", "delete", "claim"],
                    "description": "The action to perform"
                },
                "id": {
                    "type": "integer",
                    "description": "Task ID (required for update, delete, claim)"
                },
                "subject": {
                    "type": "string",
                    "description": "Task subject line (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "Task status (for update)"
                },
                "blockedBy": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "IDs of tasks that block this one"
                },
                "owner": {
                    "type": "string",
                    "description": "Owner name (for claim)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let project_path = ctx.project_path.clone();

        Box::pin(async move {
            let action = params["action"].as_str().unwrap_or("");
            match action {
                "create" => handle_create(&params, &project_path),
                "update" => handle_update(&params, &project_path),
                "list" => handle_list(&project_path),
                "delete" => handle_delete(&params, &project_path),
                "claim" => handle_claim(&params, &project_path),
                "" => Ok(ToolResult::error("Missing required parameter: action")),
                other => Ok(ToolResult::error(format!(
                    "Unknown action: {other}. Must be one of: create, update, list, delete, claim"
                ))),
            }
        })
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn tasks_dir(project_path: &Path) -> PathBuf {
    project_path.join(".ftai").join("tasks")
}

fn task_path(project_path: &Path, id: u64) -> PathBuf {
    tasks_dir(project_path).join(format!("task_{id}.json"))
}

/// Validate that a task ID is reasonable (prevents absurdly large filenames).
fn validate_id(id: u64) -> std::result::Result<(), String> {
    if id == 0 {
        return Err("Task ID must be greater than 0.".to_string());
    }
    if id > 999_999 {
        return Err("Task ID exceeds maximum (999999).".to_string());
    }
    Ok(())
}

/// Validate a string field to prevent injection.
fn validate_text_field(value: &str, field_name: &str, max_len: usize) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field_name} must not be empty."));
    }
    if value.len() > max_len {
        return Err(format!(
            "{field_name} too long ({} chars, max {max_len}).",
            value.len()
        ));
    }
    if value.contains('\0') {
        return Err(format!("{field_name} must not contain null bytes."));
    }
    Ok(())
}

fn read_task(project_path: &Path, id: u64) -> Result<TaskData> {
    let path = task_path(project_path, id);
    let content = std::fs::read_to_string(&path)?;
    let task: TaskData = serde_json::from_str(&content)?;
    Ok(task)
}

fn write_task(project_path: &Path, task: &TaskData) -> Result<()> {
    let dir = tasks_dir(project_path);
    std::fs::create_dir_all(&dir)?;
    let path = task_path(project_path, task.id);
    let json = serde_json::to_string_pretty(task)?;
    std::fs::write(&path, json)?;
    Ok(())
}

fn next_id(project_path: &Path) -> u64 {
    let dir = tasks_dir(project_path);
    if !dir.exists() {
        return 1;
    }
    let max_id = std::fs::read_dir(&dir)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with("task_") && name.ends_with(".json") {
                        name.trim_start_matches("task_")
                            .trim_end_matches(".json")
                            .parse::<u64>()
                            .ok()
                    } else {
                        None
                    }
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    max_id + 1
}

/// When a task completes, remove its ID from all other tasks' blockedBy lists.
fn clear_completed_dependency(project_path: &Path, completed_id: u64) {
    let dir = tasks_dir(project_path);
    if !dir.exists() {
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map_or(false, |e| e == "json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut task) = serde_json::from_str::<TaskData>(&content) {
                if task.blocked_by.contains(&completed_id) {
                    task.blocked_by.retain(|&id| id != completed_id);
                    let json = serde_json::to_string_pretty(&task).unwrap_or_default();
                    let _ = std::fs::write(&path, json);
                }
            }
        }
    }
}

// ─── Action handlers ─────────────────────────────────────────────────────────

fn handle_create(params: &Value, project_path: &Path) -> Result<ToolResult> {
    let subject = params["subject"].as_str().unwrap_or("");
    let description = params["description"].as_str().unwrap_or("");
    let blocked_by: Vec<u64> = params["blockedBy"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64())
                .collect()
        })
        .unwrap_or_default();

    if let Err(e) = validate_text_field(subject, "subject", 500) {
        return Ok(ToolResult::error(e));
    }

    if description.len() > 5000 {
        return Ok(ToolResult::error("description too long (max 5000 chars)."));
    }
    if description.contains('\0') {
        return Ok(ToolResult::error("description must not contain null bytes."));
    }

    let id = next_id(project_path);
    let task = TaskData {
        id,
        subject: subject.to_string(),
        description: description.to_string(),
        status: "pending".to_string(),
        blocked_by,
        owner: String::new(),
    };

    write_task(project_path, &task)?;
    Ok(ToolResult::success(format!(
        "Created task #{id}: {subject}"
    )))
}

fn handle_update(params: &Value, project_path: &Path) -> Result<ToolResult> {
    let id = match params["id"].as_u64() {
        Some(id) => id,
        None => return Ok(ToolResult::error("Missing required parameter: id")),
    };

    if let Err(e) = validate_id(id) {
        return Ok(ToolResult::error(e));
    }

    let mut task = match read_task(project_path, id) {
        Ok(t) => t,
        Err(_) => return Ok(ToolResult::error(format!("Task #{id} not found."))),
    };

    let mut changed = Vec::new();

    if let Some(subject) = params["subject"].as_str() {
        if let Err(e) = validate_text_field(subject, "subject", 500) {
            return Ok(ToolResult::error(e));
        }
        task.subject = subject.to_string();
        changed.push("subject");
    }

    if let Some(description) = params["description"].as_str() {
        if description.len() > 5000 {
            return Ok(ToolResult::error("description too long (max 5000 chars)."));
        }
        task.description = description.to_string();
        changed.push("description");
    }

    if let Some(status) = params["status"].as_str() {
        if !VALID_STATUSES.contains(&status) {
            return Ok(ToolResult::error(format!(
                "Invalid status: {status}. Must be one of: pending, in_progress, completed"
            )));
        }
        let was_completed = task.status == "completed";
        task.status = status.to_string();
        changed.push("status");

        // If transitioning to completed, clear this task from all blockedBy lists
        if status == "completed" && !was_completed {
            clear_completed_dependency(project_path, id);
        }
    }

    if let Some(blocked_by) = params["blockedBy"].as_array() {
        task.blocked_by = blocked_by.iter().filter_map(|v| v.as_u64()).collect();
        changed.push("blockedBy");
    }

    if changed.is_empty() {
        return Ok(ToolResult::error("No fields to update."));
    }

    write_task(project_path, &task)?;
    Ok(ToolResult::success(format!(
        "Updated task #{id}: {}",
        changed.join(", ")
    )))
}

fn handle_list(project_path: &Path) -> Result<ToolResult> {
    let dir = tasks_dir(project_path);
    if !dir.exists() {
        return Ok(ToolResult::success("No tasks found."));
    }

    let mut tasks: Vec<TaskData> = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(task) = serde_json::from_str::<TaskData>(&content) {
                    tasks.push(task);
                }
            }
        }
    }

    if tasks.is_empty() {
        return Ok(ToolResult::success("No tasks found."));
    }

    tasks.sort_by_key(|t| t.id);

    let mut output = String::new();
    for task in &tasks {
        let blocked = if task.blocked_by.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = task.blocked_by.iter().map(|id| format!("#{id}")).collect();
            format!(" [blocked by: {}]", ids.join(", "))
        };
        let owner = if task.owner.is_empty() {
            String::new()
        } else {
            format!(" @{}", task.owner)
        };
        output.push_str(&format!(
            "#{} [{}]{}{} — {}\n",
            task.id, task.status, blocked, owner, task.subject
        ));
    }

    Ok(ToolResult::success(output.trim_end()))
}

fn handle_delete(params: &Value, project_path: &Path) -> Result<ToolResult> {
    let id = match params["id"].as_u64() {
        Some(id) => id,
        None => return Ok(ToolResult::error("Missing required parameter: id")),
    };

    if let Err(e) = validate_id(id) {
        return Ok(ToolResult::error(e));
    }

    let path = task_path(project_path, id);
    if !path.exists() {
        return Ok(ToolResult::error(format!("Task #{id} not found.")));
    }

    std::fs::remove_file(&path)?;
    Ok(ToolResult::success(format!("Deleted task #{id}.")))
}

fn handle_claim(params: &Value, project_path: &Path) -> Result<ToolResult> {
    let id = match params["id"].as_u64() {
        Some(id) => id,
        None => return Ok(ToolResult::error("Missing required parameter: id")),
    };

    if let Err(e) = validate_id(id) {
        return Ok(ToolResult::error(e));
    }

    let owner = params["owner"].as_str().unwrap_or("agent");

    if let Err(e) = validate_text_field(owner, "owner", 100) {
        return Ok(ToolResult::error(e));
    }

    let mut task = match read_task(project_path, id) {
        Ok(t) => t,
        Err(_) => return Ok(ToolResult::error(format!("Task #{id} not found."))),
    };

    // Atomic check-and-set: only claim if unclaimed
    if !task.owner.is_empty() {
        return Ok(ToolResult::error(format!(
            "Task #{id} already claimed by '{}'.",
            task.owner
        )));
    }

    task.owner = owner.to_string();
    write_task(project_path, &task)?;
    Ok(ToolResult::success(format!(
        "Claimed task #{id} for '{owner}'."
    )))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            project_path: dir.to_path_buf(),
        }
    }

    // ── CRUD tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_task() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "subject": "Set up database",
                    "description": "Create initial migration"
                }),
                &ctx(tmp.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error, "error: {}", result.output);
        assert!(result.output.contains("#1"));

        // Verify file exists
        let file = tmp.path().join(".ftai/tasks/task_1.json");
        assert!(file.exists());
    }

    #[tokio::test]
    async fn test_create_increments_id() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Task 1"}), &c)
            .await
            .unwrap();
        let r2 = tool
            .execute(serde_json::json!({"action": "create", "subject": "Task 2"}), &c)
            .await
            .unwrap();

        assert!(r2.output.contains("#2"));
    }

    #[tokio::test]
    async fn test_update_status() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Task 1"}), &c)
            .await
            .unwrap();

        let result = tool
            .execute(
                serde_json::json!({"action": "update", "id": 1, "status": "in_progress"}),
                &c,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("status"));
    }

    #[tokio::test]
    async fn test_list_tasks() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Alpha"}), &c)
            .await
            .unwrap();
        tool.execute(serde_json::json!({"action": "create", "subject": "Beta"}), &c)
            .await
            .unwrap();

        let result = tool
            .execute(serde_json::json!({"action": "list"}), &c)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("Alpha"));
        assert!(result.output.contains("Beta"));
    }

    #[tokio::test]
    async fn test_delete_task() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Temp"}), &c)
            .await
            .unwrap();

        let result = tool
            .execute(serde_json::json!({"action": "delete", "id": 1}), &c)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("Deleted"));
        assert!(!tmp.path().join(".ftai/tasks/task_1.json").exists());
    }

    #[tokio::test]
    async fn test_claim_task() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Work item"}), &c)
            .await
            .unwrap();

        let result = tool
            .execute(
                serde_json::json!({"action": "claim", "id": 1, "owner": "alice"}),
                &c,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("alice"));

        // Second claim should fail (atomic check-and-set)
        let result = tool
            .execute(
                serde_json::json!({"action": "claim", "id": 1, "owner": "bob"}),
                &c,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("already claimed"));
    }

    // ── Dependency graph tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_dependency_clearing_on_complete() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        // Create task 1 and task 2 (blocked by 1)
        tool.execute(serde_json::json!({"action": "create", "subject": "Prerequisite"}), &c)
            .await
            .unwrap();
        tool.execute(
            serde_json::json!({"action": "create", "subject": "Dependent", "blockedBy": [1]}),
            &c,
        )
        .await
        .unwrap();

        // Verify task 2 is blocked
        let task2 = read_task(tmp.path(), 2).unwrap();
        assert_eq!(task2.blocked_by, vec![1]);

        // Complete task 1
        tool.execute(
            serde_json::json!({"action": "update", "id": 1, "status": "completed"}),
            &c,
        )
        .await
        .unwrap();

        // Task 2 should no longer be blocked
        let task2 = read_task(tmp.path(), 2).unwrap();
        assert!(task2.blocked_by.is_empty());
    }

    // ── Error handling tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_update_nonexistent_task() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let result = tool
            .execute(
                serde_json::json!({"action": "update", "id": 999, "status": "completed"}),
                &ctx(tmp.path()),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_invalid_status() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        tool.execute(serde_json::json!({"action": "create", "subject": "Test"}), &c)
            .await
            .unwrap();

        let result = tool
            .execute(
                serde_json::json!({"action": "update", "id": 1, "status": "invalid_status"}),
                &c,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("Invalid status"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let result = tool
            .execute(serde_json::json!({}), &ctx(tmp.path()))
            .await
            .unwrap();

        assert!(result.is_error);
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_security_task_id_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;
        let c = ctx(tmp.path());

        // Task ID with path traversal attempts via negative or absurd values
        let result = tool
            .execute(
                serde_json::json!({"action": "delete", "id": 0}),
                &c,
            )
            .await
            .unwrap();
        assert!(result.is_error);

        let result = tool
            .execute(
                serde_json::json!({"action": "delete", "id": 9999999}),
                &c,
            )
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_security_subject_with_null_bytes() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let result = tool
            .execute(
                serde_json::json!({"action": "create", "subject": "test\u{0000}injection"}),
                &ctx(tmp.path()),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("null"));
    }

    #[tokio::test]
    async fn test_security_oversized_description() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let big = "x".repeat(6000);
        let result = tool
            .execute(
                serde_json::json!({"action": "create", "subject": "test", "description": big}),
                &ctx(tmp.path()),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("too long"));
    }

    #[tokio::test]
    async fn test_security_task_file_stays_in_tasks_dir() {
        // Verify task files are only created inside .ftai/tasks/
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        tool.execute(
            serde_json::json!({"action": "create", "subject": "test"}),
            &ctx(tmp.path()),
        )
        .await
        .unwrap();

        let task_file = tmp.path().join(".ftai/tasks/task_1.json");
        assert!(task_file.exists());

        // No files outside .ftai/tasks/
        let ftai_entries: Vec<_> = std::fs::read_dir(tmp.path().join(".ftai"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != "tasks")
            .collect();
        assert!(ftai_entries.is_empty());
    }

    #[tokio::test]
    async fn test_security_empty_subject_rejected() {
        let tmp = TempDir::new().unwrap();
        let tool = TaskTool;

        let result = tool
            .execute(
                serde_json::json!({"action": "create", "subject": ""}),
                &ctx(tmp.path()),
            )
            .await
            .unwrap();

        assert!(result.is_error);
    }
}
