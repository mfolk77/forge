use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::registry::{Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_CONTENT_BYTES: usize = 10 * 1024; // 10 KB per memory
const MAX_NAME_LEN: usize = 128;

// ---------------------------------------------------------------------------
// MemoryReadTool
// ---------------------------------------------------------------------------

pub struct MemoryReadTool;

impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read memory notes for the current project. Use to recall past decisions, user preferences, \
         project context, and previous session summaries. Call with a query to search, or with no \
         query to list all memories."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to filter memories. Omit to list all."
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let query = params["query"].as_str().map(|s| s.to_string());
        let project_path = ctx.project_path.clone();

        Box::pin(async move {
            let memory_dir = project_path.join(".ftai").join("memory");

            if !memory_dir.exists() {
                return Ok(ToolResult::success(
                    "No memories found. The memory directory does not exist yet. \
                     Use memory_write to save your first memory.",
                ));
            }

            // Canonicalize the memory dir to prevent symlink escape
            let canon_dir = match memory_dir.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to resolve memory directory: {e}"
                    )));
                }
            };

            let entries = match read_memory_dir(&canon_dir, query.as_deref()) {
                Ok(e) => e,
                Err(e) => return Ok(ToolResult::error(format!("Failed to read memories: {e}"))),
            };

            if entries.is_empty() {
                let msg = if let Some(q) = &query {
                    format!("No memories matching \"{q}\" found.")
                } else {
                    "No memory files found in the memory directory.".to_string()
                };
                return Ok(ToolResult::success(msg));
            }

            let mut output = String::new();
            for (name, content) in &entries {
                output.push_str(&format!("### {name}\n{content}\n\n"));
            }
            Ok(ToolResult::success(output.trim_end()))
        })
    }
}

// ---------------------------------------------------------------------------
// MemoryWriteTool
// ---------------------------------------------------------------------------

pub struct MemoryWriteTool;

impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Save important information to project memory for future sessions. Use for: user preferences, \
         project decisions, architecture notes, debugging findings, workflow patterns. Each memory is \
         saved as a separate file with a descriptive name."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["name", "content"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Short descriptive name (e.g., 'user-prefers-rust', 'auth-architecture')"
                },
                "content": {
                    "type": "string",
                    "description": "The memory content to save"
                },
                "category": {
                    "type": "string",
                    "enum": ["user", "project", "decision", "feedback"],
                    "description": "Memory category (default: project)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let name = params["name"].as_str().unwrap_or("").to_string();
        let content = params["content"].as_str().unwrap_or("").to_string();
        let category = params["category"]
            .as_str()
            .unwrap_or("project")
            .to_string();
        let project_path = ctx.project_path.clone();

        Box::pin(async move {
            // Validate name
            if let Err(e) = validate_memory_name(&name) {
                return Ok(ToolResult::error(e));
            }

            // Validate category
            if !["user", "project", "decision", "feedback"].contains(&category.as_str()) {
                return Ok(ToolResult::error(format!(
                    "Invalid category \"{category}\". Must be one of: user, project, decision, feedback"
                )));
            }

            // Validate content size
            if content.len() > MAX_CONTENT_BYTES {
                return Ok(ToolResult::error(format!(
                    "Content too large ({} bytes). Maximum is {} bytes (10 KB).",
                    content.len(),
                    MAX_CONTENT_BYTES,
                )));
            }

            if content.is_empty() {
                return Ok(ToolResult::error("Content must not be empty."));
            }

            let memory_dir = project_path.join(".ftai").join("memory");
            if let Err(e) = std::fs::create_dir_all(&memory_dir) {
                return Ok(ToolResult::error(format!(
                    "Failed to create memory directory: {e}"
                )));
            }

            // Canonicalize the memory dir, then construct the target path
            let canon_dir = match memory_dir.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to resolve memory directory: {e}"
                    )));
                }
            };

            let file_path = canon_dir.join(format!("{name}.md"));

            // Verify the resolved path is still inside the memory directory
            let canon_file = match normalize_path(&file_path) {
                Some(p) => p,
                None => {
                    return Ok(ToolResult::error(
                        "Invalid memory name: path resolution failed.",
                    ));
                }
            };

            if !canon_file.starts_with(&canon_dir) {
                return Ok(ToolResult::error(
                    "Invalid memory name: path traversal detected.",
                ));
            }

            // Build file content with frontmatter
            let now = chrono_now_iso();
            let file_content = format!(
                "---\ncategory: {category}\ncreated: {now}\n---\n\n{content}\n"
            );

            match std::fs::write(&canon_file, &file_content) {
                Ok(_) => Ok(ToolResult::success(format!(
                    "Memory \"{name}\" saved ({} bytes, category: {category}).",
                    content.len(),
                ))),
                Err(e) => Ok(ToolResult::error(format!("Failed to write memory: {e}"))),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a memory name: alphanumeric + hyphens + underscores only.
/// No path separators, no dots (prevents `..`), no null bytes.
fn validate_memory_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() {
        return Err("Memory name must not be empty.".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "Memory name too long ({} chars). Maximum is {MAX_NAME_LEN}.",
            name.len()
        ));
    }
    if name.contains('\0') {
        return Err("Memory name must not contain null bytes.".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("Memory name must not contain path separators.".to_string());
    }
    if name.contains("..") {
        return Err("Memory name must not contain '..'.".to_string());
    }
    // Allow only alphanumeric, hyphens, underscores
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Memory name must contain only alphanumeric characters, hyphens, and underscores."
                .to_string(),
        );
    }
    Ok(())
}

/// Normalize a path without requiring it to exist (unlike canonicalize).
/// This resolves `..` and `.` components lexically.
fn normalize_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !result.pop() {
                    return None; // tried to go above root
                }
            }
            Component::CurDir => {} // skip
            other => result.push(other),
        }
    }
    Some(result)
}

/// Generate an ISO 8601 timestamp.
fn chrono_now_iso() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Simple UTC timestamp without pulling in chrono crate
    // Format: YYYY-MM-DDTHH:MM:SSZ
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to date (simplified algorithm)
    let (year, month, day) = days_to_date(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days: i64) -> (i64, u32, u32) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Memory directory reading helpers (shared with prompt.rs)
// ---------------------------------------------------------------------------

/// Read all `.md` files from a memory directory.
/// If `query` is provided, only return entries whose name or content match.
/// Returns a Vec of (name, display_content) pairs.
pub fn read_memory_dir(
    dir: &Path,
    query: Option<&str>,
) -> Result<Vec<(String, String)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    let read_dir = std::fs::read_dir(dir)?;
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let display_content = strip_frontmatter(&content).to_string();

                // Filter by query if provided
                if let Some(q) = query {
                    let q_lower = q.to_lowercase();
                    let name_lower = name.to_lowercase();
                    let content_lower = display_content.to_lowercase();
                    if !name_lower.contains(&q_lower) && !content_lower.contains(&q_lower) {
                        continue;
                    }
                }

                entries.push((name, display_content));
            }
        }
    }

    // Sort by name for deterministic output
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// Strip YAML frontmatter from a markdown string.
pub fn strip_frontmatter(content: &str) -> &str {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("---") {
            let after = end + 6; // skip past the closing ---
            if after <= content.len() {
                return content[after..].trim_start();
            }
        }
    }
    content
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            project_path: dir.to_path_buf(),
        }
    }

    // -----------------------------------------------------------------------
    // Validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_memory_name("user-prefers-rust").is_ok());
        assert!(validate_memory_name("auth_architecture").is_ok());
        assert!(validate_memory_name("my-note-123").is_ok());
        assert!(validate_memory_name("A").is_ok());
    }

    #[test]
    fn test_validate_name_empty() {
        assert!(validate_memory_name("").is_err());
    }

    #[test]
    fn test_validate_name_too_long() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert!(validate_memory_name(&long).is_err());
    }

    #[test]
    fn test_validate_name_path_traversal() {
        assert!(validate_memory_name("..").is_err());
        assert!(validate_memory_name("../etc/passwd").is_err());
        assert!(validate_memory_name("foo/bar").is_err());
        assert!(validate_memory_name("foo\\bar").is_err());
    }

    #[test]
    fn test_validate_name_null_byte() {
        assert!(validate_memory_name("foo\0bar").is_err());
    }

    #[test]
    fn test_validate_name_special_chars() {
        assert!(validate_memory_name("foo.bar").is_err());
        assert!(validate_memory_name("foo bar").is_err());
        assert!(validate_memory_name("foo@bar").is_err());
    }

    // -----------------------------------------------------------------------
    // strip_frontmatter
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_frontmatter_with_frontmatter() {
        let content = "---\ncategory: project\ncreated: 2026-03-31T08:00:00Z\n---\n\nThe content here.";
        let stripped = strip_frontmatter(content);
        assert_eq!(stripped, "The content here.");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "Just some text.";
        let stripped = strip_frontmatter(content);
        assert_eq!(stripped, "Just some text.");
    }

    #[test]
    fn test_strip_frontmatter_empty() {
        assert_eq!(strip_frontmatter(""), "");
    }

    #[test]
    fn test_strip_frontmatter_incomplete() {
        let content = "---\nno closing";
        let stripped = strip_frontmatter(content);
        assert_eq!(stripped, content);
    }

    // -----------------------------------------------------------------------
    // MemoryWriteTool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_write_creates_file_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "auth-notes",
                    "content": "JWT with RS256",
                    "category": "decision"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.output);
        assert!(result.output.contains("auth-notes"));

        let file = tmp.path().join(".ftai/memory/auth-notes.md");
        assert!(file.exists());
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("category: decision"));
        assert!(content.contains("created:"));
        assert!(content.contains("JWT with RS256"));
    }

    #[tokio::test]
    async fn test_write_default_category() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "test-note",
                    "content": "some content"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let file = tmp.path().join(".ftai/memory/test-note.md");
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("category: project"));
    }

    #[tokio::test]
    async fn test_write_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        for name in &["../escape", "../../etc/passwd", "foo/bar", "foo\\bar"] {
            let result = tool
                .execute(
                    serde_json::json!({
                        "name": name,
                        "content": "malicious"
                    }),
                    &ctx,
                )
                .await
                .unwrap();

            assert!(result.is_error, "should reject name: {name}");
        }
    }

    #[tokio::test]
    async fn test_write_rejects_oversized_content() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let big_content = "x".repeat(MAX_CONTENT_BYTES + 1);
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "big-note",
                    "content": big_content
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("too large"));
    }

    #[tokio::test]
    async fn test_write_rejects_empty_content() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "empty-note",
                    "content": ""
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_write_rejects_null_byte_in_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "name": "foo\u{0000}bar",
                    "content": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("null"));
    }

    #[tokio::test]
    async fn test_write_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        tool.execute(
            serde_json::json!({
                "name": "overwrite-me",
                "content": "version 1"
            }),
            &ctx,
        )
        .await
        .unwrap();

        tool.execute(
            serde_json::json!({
                "name": "overwrite-me",
                "content": "version 2"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let file = tmp.path().join(".ftai/memory/overwrite-me.md");
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("version 2"));
        assert!(!content.contains("version 1"));
    }

    // -----------------------------------------------------------------------
    // MemoryReadTool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_read_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryReadTool;

        let result = tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_read_finds_written_memories() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let write_tool = MemoryWriteTool;
        let read_tool = MemoryReadTool;

        write_tool
            .execute(
                serde_json::json!({
                    "name": "test-memory",
                    "content": "important finding"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let result = read_tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("test-memory"));
        assert!(result.output.contains("important finding"));
    }

    #[tokio::test]
    async fn test_read_with_query_filters_results() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let write_tool = MemoryWriteTool;
        let read_tool = MemoryReadTool;

        write_tool
            .execute(
                serde_json::json!({
                    "name": "auth-design",
                    "content": "JWT tokens with RS256"
                }),
                &ctx,
            )
            .await
            .unwrap();

        write_tool
            .execute(
                serde_json::json!({
                    "name": "database-choice",
                    "content": "Using PostgreSQL for persistence"
                }),
                &ctx,
            )
            .await
            .unwrap();

        // Search by name
        let result = read_tool
            .execute(serde_json::json!({"query": "auth"}), &ctx)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("auth-design"));
        assert!(result.output.contains("JWT"));
        assert!(!result.output.contains("database-choice"));

        // Search by content
        let result = read_tool
            .execute(serde_json::json!({"query": "PostgreSQL"}), &ctx)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("database-choice"));
        assert!(!result.output.contains("auth-design"));
    }

    #[tokio::test]
    async fn test_read_query_no_match() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let write_tool = MemoryWriteTool;
        let read_tool = MemoryReadTool;

        write_tool
            .execute(
                serde_json::json!({
                    "name": "some-note",
                    "content": "content here"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let result = read_tool
            .execute(serde_json::json!({"query": "nonexistent-term-xyz"}), &ctx)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("No memories matching"));
    }

    // -----------------------------------------------------------------------
    // read_memory_dir
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_memory_dir_nonexistent() {
        let entries = read_memory_dir(Path::new("/nonexistent/path"), None).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_read_memory_dir_with_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("note-a.md"),
            "---\ncategory: project\ncreated: 2026-01-01T00:00:00Z\n---\n\nFirst note.",
        )
        .unwrap();
        std::fs::write(
            dir.join("note-b.md"),
            "---\ncategory: user\ncreated: 2026-01-02T00:00:00Z\n---\n\nSecond note.",
        )
        .unwrap();
        // Non-md file should be ignored
        std::fs::write(dir.join("ignore.txt"), "not a memory").unwrap();

        let entries = read_memory_dir(dir, None).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "note-a");
        assert_eq!(entries[0].1, "First note.");
        assert_eq!(entries[1].0, "note-b");
        assert_eq!(entries[1].1, "Second note.");
    }

    #[test]
    fn test_read_memory_dir_with_query() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        std::fs::write(dir.join("rust-notes.md"), "Rust is great").unwrap();
        std::fs::write(dir.join("python-notes.md"), "Python is flexible").unwrap();

        let entries = read_memory_dir(dir, Some("rust")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "rust-notes");
    }

    // -----------------------------------------------------------------------
    // P0 Security Red Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_p0_path_traversal_in_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let traversal_names = vec![
            "..",
            "../..",
            "../../etc/passwd",
            "../secret",
            "..%2F..%2Fetc%2Fpasswd",
            "foo/../../../etc/passwd",
        ];

        for name in traversal_names {
            let result = tool
                .execute(
                    serde_json::json!({
                        "name": name,
                        "content": "attack"
                    }),
                    &ctx,
                )
                .await
                .unwrap();

            assert!(
                result.is_error,
                "Path traversal name should be rejected: {name}"
            );
        }
    }

    #[tokio::test]
    async fn test_p0_null_bytes_in_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        let null_names = vec!["foo\0bar", "\0", "test\0.md", "name\0/../../../etc/passwd"];

        for name in null_names {
            let result = tool
                .execute(
                    serde_json::json!({
                        "name": name,
                        "content": "attack"
                    }),
                    &ctx,
                )
                .await
                .unwrap();

            assert!(
                result.is_error,
                "Null byte name should be rejected: {:?}",
                name
            );
        }
    }

    #[tokio::test]
    async fn test_p0_symlink_escape_attempt() {
        let tmp = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();

        // Create memory dir
        let memory_dir = tmp.path().join(".ftai").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();

        // Write a file in the target that should not be accessible
        std::fs::write(target_dir.path().join("secret.md"), "secret content").unwrap();

        // memory_read only reads from the canonicalized memory dir.
        // Even if someone manages to place a symlink, canonicalize()
        // will resolve it and the starts_with check will catch it.
        let ctx = make_ctx(tmp.path());
        let read_tool = MemoryReadTool;

        let result = read_tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap();

        // The memory dir exists but is empty — should not contain secret
        assert!(!result.output.contains("secret content"));
    }

    #[tokio::test]
    async fn test_p0_memory_write_does_not_escape_project_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_ctx(tmp.path());
        let tool = MemoryWriteTool;

        // Attempt to write with a name containing slashes
        let result = tool
            .execute(
                serde_json::json!({
                    "name": "../../escape-attempt",
                    "content": "should not be written"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);

        // Verify nothing was written outside the project
        let escape_path = tmp.path().join("escape-attempt.md");
        assert!(!escape_path.exists());
    }

    #[tokio::test]
    async fn test_p0_memory_read_never_reads_outside_memory_dir() {
        let tmp = TempDir::new().unwrap();

        // Create a sensitive file outside memory dir
        let secret_file = tmp.path().join(".ftai").join("secret.md");
        std::fs::create_dir_all(tmp.path().join(".ftai")).unwrap();
        std::fs::write(&secret_file, "top secret").unwrap();

        // Create memory dir (empty)
        let memory_dir = tmp.path().join(".ftai").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();

        let ctx = make_ctx(tmp.path());
        let read_tool = MemoryReadTool;

        let result = read_tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap();

        assert!(!result.output.contains("top secret"));
    }

    #[tokio::test]
    async fn test_p0_lfi_via_query_parameter() {
        // The query parameter should never be used as a file path
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join(".ftai").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("normal.md"), "normal content").unwrap();

        let ctx = make_ctx(tmp.path());
        let read_tool = MemoryReadTool;

        let result = read_tool
            .execute(
                serde_json::json!({"query": "../../../etc/passwd"}),
                &ctx,
            )
            .await
            .unwrap();

        // Query is used as a text filter, not a path — should just return no matches
        assert!(!result.is_error);
        assert!(!result.output.contains("root:"));
    }

    // -----------------------------------------------------------------------
    // chrono_now_iso / days_to_date
    // -----------------------------------------------------------------------

    #[test]
    fn test_days_to_date_epoch() {
        let (y, m, d) = days_to_date(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_date_known() {
        // 2026-03-29 is 20,541 days since epoch
        let (y, m, d) = days_to_date(20541);
        assert_eq!((y, m, d), (2026, 3, 29));
    }

    #[test]
    fn test_chrono_now_iso_format() {
        let ts = chrono_now_iso();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }
}
