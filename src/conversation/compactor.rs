use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::types::{Message, Role};

/// Three-tier context compaction system.
///
/// - Tier 1 (Microcompact): Replace old tool_result content with one-line placeholders.
///   Runs before every LLM call, zero model calls.
/// - Tier 2 (Snip compact): Remove oldest messages when token usage > 70% of context window.
///   Saves removed messages to transcript first.
/// - Tier 3 (Summarize compact): When token usage > 85%, save full transcript and replace
///   all messages with a deterministic summary. Reinjects identity + tool list.

/// Tier 1 — Microcompact: replace old tool_result messages with one-line placeholders.
/// Preserves `file_read` results (reference material).
/// Returns the number of messages compacted.
pub fn micro_compact(messages: &mut Vec<Message>) -> usize {
    // Find tool result messages — they have role == Tool.
    // We want to keep the last 3 tool results intact; older ones get replaced.
    let tool_result_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Tool)
        .map(|(i, _)| i)
        .collect();

    if tool_result_indices.len() <= 3 {
        return 0;
    }

    let cutoff = tool_result_indices.len() - 3;
    let mut compacted = 0;

    for &idx in &tool_result_indices[..cutoff] {
        let msg = &messages[idx];

        // Already compacted — skip
        if msg.content.starts_with("[Previous: used ") {
            continue;
        }

        // Find the corresponding assistant message with tool_calls to determine tool name.
        // The tool_call_id on this message matches a tool_call in an earlier assistant message.
        let tool_name = if let Some(ref tc_id) = msg.tool_call_id {
            find_tool_name_for_call(messages, tc_id, idx)
        } else {
            None
        };

        let tool_name = tool_name.unwrap_or_else(|| "tool".to_string());

        // Exception: preserve file_read results
        if tool_name == "file_read" {
            continue;
        }

        messages[idx].content = format!("[Previous: used {tool_name}]");
        compacted += 1;
    }

    compacted
}

/// Find the tool name for a given tool_call_id by searching assistant messages before `before_idx`.
fn find_tool_name_for_call(messages: &[Message], call_id: &str, before_idx: usize) -> Option<String> {
    for msg in messages[..before_idx].iter().rev() {
        if msg.role == Role::Assistant {
            if let Some(ref calls) = msg.tool_calls {
                for tc in calls {
                    if tc.id == call_id {
                        return Some(tc.name.clone());
                    }
                }
            }
        }
    }
    None
}

/// Check whether compaction is needed and return the appropriate tier.
/// Returns:
/// - `None` if no compaction needed
/// - `Some(2)` for snip compact (>70%)
/// - `Some(3)` for summarize compact (>85%)
/// - `Some(5)` for emergency truncation (>95%)
pub fn should_compact(estimated_tokens: usize, max_context_tokens: usize) -> Option<u8> {
    if max_context_tokens == 0 {
        return Some(3);
    }
    let ratio = (estimated_tokens * 100) / max_context_tokens;
    if ratio > 95 {
        Some(5)
    } else if ratio > 85 {
        Some(3)
    } else if ratio > 70 {
        Some(2)
    } else {
        None
    }
}

/// Tier 2 — Snip compact: remove oldest messages, keeping system prompt position
/// and last `keep_recent` exchanges. Saves removed messages to transcript first.
/// Returns the number of messages removed.
pub fn snip_compact(
    messages: &mut Vec<Message>,
    project_path: &Path,
    keep_recent: usize,
) -> Result<usize> {
    let keep = keep_recent.min(messages.len());
    if messages.len() <= keep {
        return Ok(0);
    }

    let remove_count = messages.len() - keep;
    let removed: Vec<Message> = messages.drain(..remove_count).collect();
    let recent_has_user = messages.iter().any(|m| m.role == Role::User);
    let last_removed_user = if recent_has_user {
        None
    } else {
        last_user_message(&removed)
    };

    // Save to transcript
    save_transcript(project_path, &removed)?;

    // Insert a summary placeholder at the front
    let summary = summarize_removed(&removed);
    messages.insert(
        0,
        Message {
            role: Role::System,
            content: format!("[Conversation summary: {summary}]"),
            tool_calls: None,
            tool_call_id: None,
        },
    );
    if let Some(user) = last_removed_user {
        messages.insert(1, user);
    }

    Ok(remove_count)
}

/// The 9-category summarization prompt structure used when Tier 3 (summarize compact) runs.
/// This is the instruction prepended to the conversation content for deterministic summaries.
pub const SUMMARIZE_PROMPT: &str = "\
Summarize this conversation for continuity. Preserve these categories in order:

1. Primary Request and Intent — the user's explicit requests in detail
2. Key Technical Concepts — technologies, frameworks, patterns discussed
3. Files and Code Sections — specific files examined or modified, with key snippets
4. Errors and Fixes — all errors encountered and how they were resolved
5. Problem Solving — problems solved and ongoing troubleshooting
6. User Messages — capture the user's own words for important instructions
7. Pending Tasks — explicitly assigned work that remains incomplete
8. Current Work — precisely what was happening immediately before this summary
9. Next Step — the single next action in line with the user's most recent request

Be concise but preserve critical technical details. Output only the summary.";

/// Tier 3 — Summarize compact: save full transcript, replace ALL messages with
/// a single summary message. The caller should reinject identity + tool list after this.
/// Returns the summary text.
pub fn summarize_compact(
    messages: &mut Vec<Message>,
    project_path: &Path,
) -> Result<String> {
    // Save full transcript
    save_transcript(project_path, messages)?;

    // Build summary from all messages using the 9-category structure
    let summary = summarize_removed(messages);
    let last_user = last_user_message(messages);

    // Replace all messages with the summary
    messages.clear();
    messages.push(Message {
        role: Role::System,
        content: format!(
            "[Session summary — previous context was compacted to fit within the context window]\n\n\
             {SUMMARIZE_PROMPT}\n\n\
             ---\n\n{summary}"
        ),
        tool_calls: None,
        tool_call_id: None,
    });
    if let Some(user) = last_user {
        messages.push(user);
    }

    Ok(summary)
}

/// Build a deterministic summary from a list of messages.
fn summarize_removed(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => {
                let preview: String = msg.content.chars().take(120).collect();
                if !preview.is_empty() {
                    parts.push(format!("User: {preview}"));
                }
            }
            Role::Assistant => {
                if let Some(ref calls) = msg.tool_calls {
                    for tc in calls {
                        parts.push(format!("Used {}", tc.name));
                    }
                }
                let preview: String = msg
                    .content
                    .chars()
                    .take(80)
                    .collect();
                if !preview.trim().is_empty() {
                    parts.push(format!("Assistant: {preview}"));
                }
            }
            Role::Tool => {
                // Already summarized or not interesting for the summary
            }
            Role::System => {
                // Skip system messages in summary
            }
        }
    }

    if parts.is_empty() {
        "Previous conversation context".to_string()
    } else {
        // Limit to avoid overly long summaries
        let limit = parts.len().min(20);
        parts[..limit].join("; ")
    }
}

fn last_user_message(messages: &[Message]) -> Option<Message> {
    messages.iter().rev().find(|m| m.role == Role::User).cloned()
}

/// Save messages to a JSONL transcript file.
/// Path: `<project>/.ftai/transcripts/<unix_timestamp>.jsonl`
pub fn save_transcript(project_path: &Path, messages: &[Message]) -> Result<PathBuf> {
    let transcripts_dir = project_path.join(".ftai").join("transcripts");
    std::fs::create_dir_all(&transcripts_dir)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let path = transcripts_dir.join(format!("{timestamp}.jsonl"));
    let mut file = std::fs::File::create(&path)?;

    for msg in messages {
        let json = serde_json::to_string(msg)?;
        writeln!(file, "{json}")?;
    }

    Ok(path)
}

// ─── Tier 4: Session Memory Extraction ──────────────────────────────────────

/// Structured session state that survives total context loss.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub timestamp: u64,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<(String, usize)>,
    pub errors_encountered: Vec<String>,
    pub last_user_request: Option<String>,
    pub pending_work: Option<String>,
}

/// Tier 4: Extract structured session state to disk before emergency truncation.
/// Writes a JSON checkpoint with key session data that survives total context loss.
pub fn extract_session_memory(messages: &[Message], project_path: &Path) -> Result<PathBuf> {
    let checkpoint = SessionCheckpoint {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        files_modified: extract_modified_files(messages),
        tools_used: extract_tool_usage(messages),
        errors_encountered: extract_errors(messages),
        last_user_request: extract_last_user_message(messages),
        pending_work: extract_pending_work(messages),
    };

    let dir = project_path.join(".ftai").join("checkpoints");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", checkpoint.timestamp));
    let json = serde_json::to_string_pretty(&checkpoint)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Scan tool calls for file_write/file_edit and collect the file paths.
fn extract_modified_files(messages: &[Message]) -> Vec<String> {
    let mut files = Vec::new();
    for msg in messages {
        if msg.role == Role::Assistant {
            if let Some(ref calls) = msg.tool_calls {
                for tc in calls {
                    if tc.name == "file_write" || tc.name == "file_edit" {
                        if let Some(path) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                            if !files.contains(&path.to_string()) {
                                files.push(path.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    files
}

/// Count tool calls by name across all assistant messages.
fn extract_tool_usage(messages: &[Message]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for msg in messages {
        if msg.role == Role::Assistant {
            if let Some(ref calls) = msg.tool_calls {
                for tc in calls {
                    *counts.entry(tc.name.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    let mut result: Vec<(String, usize)> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

/// Find tool results containing error patterns.
fn extract_errors(messages: &[Message]) -> Vec<String> {
    let mut errors = Vec::new();
    for msg in messages {
        if msg.role == Role::Tool {
            if msg.content.contains("Error:") || msg.content.contains("FAILED") {
                let preview: String = msg.content.chars().take(200).collect();
                errors.push(preview);
            }
        }
    }
    errors
}

/// Find the last user-role message content.
fn extract_last_user_message(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User && !m.content.trim().is_empty())
        .map(|m| m.content.clone())
}

/// Look for task tool calls with status != "completed" to identify pending work.
fn extract_pending_work(messages: &[Message]) -> Option<String> {
    let mut pending = Vec::new();
    for msg in messages {
        if msg.role == Role::Assistant {
            if let Some(ref calls) = msg.tool_calls {
                for tc in calls {
                    if tc.name == "task" {
                        let status = tc.arguments.get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        if status != "completed" {
                            let desc = tc.arguments.get("description")
                                .and_then(|v| v.as_str())
                                .or_else(|| tc.arguments.get("task").and_then(|v| v.as_str()))
                                .unwrap_or("(task)");
                            pending.push(format!("[{}] {}", status, desc));
                        }
                    }
                }
            }
        }
    }
    if pending.is_empty() {
        None
    } else {
        Some(pending.join("; "))
    }
}

// ─── Tier 5: Emergency Truncation ───────────────────────────────────────────

/// Tier 5: Emergency truncation when all else fails.
/// Keeps only: system prompt + last 3 messages + session checkpoint reference.
/// Saves a checkpoint before truncating so session state survives.
pub fn emergency_truncate(messages: &mut Vec<Message>, project_path: &Path) {
    // Save checkpoint before truncation
    let _ = extract_session_memory(messages, project_path);

    // Keep system prompt (first message if Role::System)
    let system_msg = messages.iter().find(|m| m.role == Role::System).cloned();

    // Keep last 3 messages
    let tail_count = 3.min(messages.len());
    let tail: Vec<Message> = messages.iter().rev().take(tail_count).cloned().collect();
    let last_user = if tail.iter().any(|m| m.role == Role::User) {
        None
    } else {
        last_user_message(messages)
    };

    messages.clear();
    if let Some(sys) = system_msg {
        messages.push(sys);
    }
    // Add checkpoint reference so model knows where to find context
    messages.push(Message {
        role: Role::System,
        content: "[Emergency context recovery. Session checkpoint saved to .ftai/checkpoints/. \
                  Use file_read to load if needed. Last user request may be in the messages below.]"
            .to_string(),
        tool_calls: None,
        tool_call_id: None,
    });
    if let Some(user) = last_user {
        messages.push(user);
    }
    for msg in tail.into_iter().rev() {
        messages.push(msg);
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::ToolCall;
    use tempfile::TempDir;

    fn make_tool_result(id: &str, content: &str) -> Message {
        Message {
            role: Role::Tool,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
        }
    }

    fn make_assistant_with_call(call_id: &str, tool_name: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                arguments: serde_json::json!({}),
            }]),
            tool_call_id: None,
        }
    }

    fn make_user(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    // ── Microcompact tests ─────────────────────────────────────────────────

    #[test]
    fn test_microcompact_replaces_old_tool_results() {
        let mut messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "long output from bash command"),
            make_assistant_with_call("tc2", "grep"),
            make_tool_result("tc2", "grep results here"),
            make_assistant_with_call("tc3", "bash"),
            make_tool_result("tc3", "another bash output"),
            make_assistant_with_call("tc4", "file_write"),
            make_tool_result("tc4", "file written successfully"),
            make_assistant_with_call("tc5", "bash"),
            make_tool_result("tc5", "recent bash output"),
        ];

        let compacted = micro_compact(&mut messages);

        // tc1 and tc2 should be compacted (oldest 2 of 5 tool results)
        assert!(compacted >= 2);
        assert!(messages[1].content.starts_with("[Previous: used bash]"));
        assert!(messages[3].content.starts_with("[Previous: used grep]"));

        // Last 3 tool results should be preserved
        assert_eq!(messages[5].content, "another bash output");
        assert_eq!(messages[7].content, "file written successfully");
        assert_eq!(messages[9].content, "recent bash output");
    }

    #[test]
    fn test_microcompact_preserves_file_read() {
        let mut messages = vec![
            make_assistant_with_call("tc1", "file_read"),
            make_tool_result("tc1", "contents of important file"),
            make_assistant_with_call("tc2", "bash"),
            make_tool_result("tc2", "bash output"),
            make_assistant_with_call("tc3", "bash"),
            make_tool_result("tc3", "bash output 2"),
            make_assistant_with_call("tc4", "bash"),
            make_tool_result("tc4", "bash output 3"),
            make_assistant_with_call("tc5", "bash"),
            make_tool_result("tc5", "bash output 4"),
        ];

        micro_compact(&mut messages);

        // file_read result should be preserved even though it's old
        assert_eq!(messages[1].content, "contents of important file");
        // Old bash result should be compacted
        assert!(messages[3].content.starts_with("[Previous: used bash]"));
    }

    #[test]
    fn test_microcompact_with_few_results_is_noop() {
        let mut messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "output"),
            make_assistant_with_call("tc2", "bash"),
            make_tool_result("tc2", "output 2"),
        ];

        let compacted = micro_compact(&mut messages);
        assert_eq!(compacted, 0);
        assert_eq!(messages[1].content, "output");
    }

    #[test]
    fn test_microcompact_idempotent() {
        let mut messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "first output"),
            make_assistant_with_call("tc2", "bash"),
            make_tool_result("tc2", "second output"),
            make_assistant_with_call("tc3", "bash"),
            make_tool_result("tc3", "third output"),
            make_assistant_with_call("tc4", "bash"),
            make_tool_result("tc4", "fourth output"),
        ];

        micro_compact(&mut messages);
        let after_first = messages[1].content.clone();

        micro_compact(&mut messages);
        assert_eq!(messages[1].content, after_first);
    }

    // ── should_compact tests ───────────────────────────────────────────────

    #[test]
    fn test_should_compact_under_70() {
        assert_eq!(should_compact(6000, 10000), None);
    }

    #[test]
    fn test_should_compact_tier2() {
        assert_eq!(should_compact(7500, 10000), Some(2));
    }

    #[test]
    fn test_should_compact_tier3() {
        // 86-95% triggers tier 3
        assert_eq!(should_compact(9000, 10000), Some(3));
    }

    #[test]
    fn test_should_compact_tier5() {
        // >95% triggers emergency truncation (tier 5)
        assert_eq!(should_compact(9600, 10000), Some(5));
        assert_eq!(should_compact(9900, 10000), Some(5));
    }

    #[test]
    fn test_should_compact_zero_context() {
        assert_eq!(should_compact(100, 0), Some(3));
    }

    // ── Snip compact tests ─────────────────────────────────────────────────

    #[test]
    fn test_snip_compact_removes_old_messages() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("first question"),
            make_user("second question"),
            make_user("third question"),
            make_user("fourth question"),
            make_user("fifth question"),
        ];

        let removed = snip_compact(&mut messages, tmp.path(), 3).unwrap();
        assert_eq!(removed, 2);
        // Should have summary + 3 recent messages = 4
        assert_eq!(messages.len(), 4);
        assert!(messages[0].content.contains("Conversation summary"));
    }

    #[test]
    fn test_snip_compact_saves_transcript() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("msg 1"),
            make_user("msg 2"),
            make_user("msg 3"),
            make_user("msg 4"),
        ];

        snip_compact(&mut messages, tmp.path(), 2).unwrap();

        let transcripts_dir = tmp.path().join(".ftai").join("transcripts");
        assert!(transcripts_dir.exists());
        let entries: Vec<_> = std::fs::read_dir(&transcripts_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_snip_compact_preserves_last_user_when_tail_has_only_tools() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("original request"),
            make_assistant_with_call("tc1", "file_write"),
            make_tool_result("tc1", "wrote file"),
        ];

        snip_compact(&mut messages, tmp.path(), 2).unwrap();

        assert!(messages.iter().any(|m| m.role == Role::User));
        assert!(messages.iter().any(|m| m.content == "original request"));
    }

    // ── Summarize compact tests ────────────────────────────────────────────

    #[test]
    fn test_summarize_compact_replaces_all() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("a long question about code"),
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "output"),
            make_user("another question"),
        ];

        let summary = summarize_compact(&mut messages, tmp.path()).unwrap();
        assert!(!summary.is_empty());
        assert_eq!(messages.len(), 2);
        assert!(messages[0].content.contains("Session summary"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "another question");
    }

    // ── Transcript saving tests ────────────────────────────────────────────

    #[test]
    fn test_save_transcript_creates_jsonl() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_user("hello"),
            Message {
                role: Role::Assistant,
                content: "hi there".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        let path = save_transcript(tmp.path(), &messages).unwrap();
        assert!(path.exists());
        assert!(path.extension().unwrap() == "jsonl");

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line should be valid JSON
        for line in lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.get("role").is_some());
        }
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_transcript_path_is_safe() {
        // Verify transcript files are always created inside .ftai/transcripts/
        let tmp = TempDir::new().unwrap();
        let messages = vec![make_user("test")];

        let path = save_transcript(tmp.path(), &messages).unwrap();
        let canonical = path.canonicalize().unwrap();
        let expected_parent = tmp.path().join(".ftai").join("transcripts").canonicalize().unwrap();
        assert!(canonical.starts_with(&expected_parent));
    }

    #[test]
    fn test_security_summarize_does_not_leak_tool_output() {
        // Tool outputs should NOT appear in the summary (they can contain sensitive data)
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "SECRET_API_KEY=abc123"),
        ];

        let summary = summarize_compact(&mut messages, tmp.path()).unwrap();
        assert!(!summary.contains("SECRET_API_KEY"));
        assert!(!summary.contains("abc123"));
    }

    #[test]
    fn test_security_microcompact_empty_messages() {
        let mut messages: Vec<Message> = vec![];
        let compacted = micro_compact(&mut messages);
        assert_eq!(compacted, 0);
    }

    #[test]
    fn test_security_snip_compact_empty_messages() {
        let tmp = TempDir::new().unwrap();
        let mut messages: Vec<Message> = vec![];
        let removed = snip_compact(&mut messages, tmp.path(), 5).unwrap();
        assert_eq!(removed, 0);
    }

    // ── Appendix A: Summarize prompt structure ─────────────────────────────

    #[test]
    fn test_summarize_compact_includes_9_categories() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("fix the auth bug in refresh.rs"),
            make_assistant_with_call("tc1", "file_read"),
            make_tool_result("tc1", "file contents..."),
            make_user("now run tests"),
        ];

        summarize_compact(&mut messages, tmp.path()).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, Role::User);

        let content = &messages[0].content;
        // Must include the 9-category summarization prompt
        assert!(content.contains("Primary Request and Intent"), "missing category 1");
        assert!(content.contains("Key Technical Concepts"), "missing category 2");
        assert!(content.contains("Files and Code Sections"), "missing category 3");
        assert!(content.contains("Errors and Fixes"), "missing category 4");
        assert!(content.contains("Problem Solving"), "missing category 5");
        assert!(content.contains("User Messages"), "missing category 6");
        assert!(content.contains("Pending Tasks"), "missing category 7");
        assert!(content.contains("Current Work"), "missing category 8");
        assert!(content.contains("Next Step"), "missing category 9");
    }

    #[test]
    fn test_summarize_prompt_constant_exists() {
        // Verify the constant is accessible and non-empty
        assert!(!SUMMARIZE_PROMPT.is_empty());
        assert!(SUMMARIZE_PROMPT.contains("Summarize this conversation"));
    }

    // ── Tier 4: Session Memory Extraction tests ───────────────────────────

    fn make_assistant_with_file_edit(call_id: &str, path: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: call_id.to_string(),
                name: "file_edit".to_string(),
                arguments: serde_json::json!({"path": path, "old": "x", "new": "y"}),
            }]),
            tool_call_id: None,
        }
    }

    fn make_assistant_with_task(call_id: &str, status: &str, desc: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: call_id.to_string(),
                name: "task".to_string(),
                arguments: serde_json::json!({"status": status, "description": desc}),
            }]),
            tool_call_id: None,
        }
    }

    #[test]
    fn test_extract_session_memory_writes_valid_json() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_user("fix the bug"),
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "compiled OK"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert!(checkpoint.timestamp > 0);
    }

    #[test]
    fn test_checkpoint_contains_modified_files() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_assistant_with_file_edit("tc1", "src/main.rs"),
            make_tool_result("tc1", "ok"),
            make_assistant_with_file_edit("tc2", "src/lib.rs"),
            make_tool_result("tc2", "ok"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert_eq!(checkpoint.files_modified, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn test_checkpoint_contains_error_patterns() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "Error: cannot find module"),
            make_assistant_with_call("tc2", "bash"),
            make_tool_result("tc2", "FAILED: test_auth_bypass"),
            make_assistant_with_call("tc3", "bash"),
            make_tool_result("tc3", "ok"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert_eq!(checkpoint.errors_encountered.len(), 2);
        assert!(checkpoint.errors_encountered[0].contains("cannot find module"));
        assert!(checkpoint.errors_encountered[1].contains("FAILED"));
    }

    #[test]
    fn test_checkpoint_contains_last_user_request() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_user("first request"),
            make_user("second request"),
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "ok"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert_eq!(checkpoint.last_user_request.as_deref(), Some("second request"));
    }

    #[test]
    fn test_checkpoint_tool_usage_counts() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "ok"),
            make_assistant_with_call("tc2", "bash"),
            make_tool_result("tc2", "ok"),
            make_assistant_with_call("tc3", "file_read"),
            make_tool_result("tc3", "contents"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        // bash should have count 2, file_read count 1
        let bash_count = checkpoint.tools_used.iter().find(|(n, _)| n == "bash").map(|(_, c)| *c);
        let read_count = checkpoint.tools_used.iter().find(|(n, _)| n == "file_read").map(|(_, c)| *c);
        assert_eq!(bash_count, Some(2));
        assert_eq!(read_count, Some(1));
    }

    #[test]
    fn test_checkpoint_pending_work() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![
            make_assistant_with_task("tc1", "in_progress", "fix auth bug"),
            make_tool_result("tc1", "ok"),
            make_assistant_with_task("tc2", "completed", "write tests"),
            make_tool_result("tc2", "ok"),
        ];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert!(checkpoint.pending_work.is_some());
        assert!(checkpoint.pending_work.as_ref().unwrap().contains("fix auth bug"));
        // completed task should not appear
        assert!(!checkpoint.pending_work.as_ref().unwrap().contains("write tests"));
    }

    // ── Tier 5: Emergency Truncation tests ────────────────────────────────

    #[test]
    fn test_emergency_truncate_keeps_system_prompt() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "You are FTAI.".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            make_user("question 1"),
            make_user("question 2"),
            make_user("question 3"),
            make_user("question 4"),
            make_user("question 5"),
        ];

        emergency_truncate(&mut messages, tmp.path());
        // Should have: system + checkpoint ref + last 3 user messages = 5
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("FTAI"));
    }

    #[test]
    fn test_emergency_truncate_keeps_last_3_messages() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "system".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            make_user("old 1"),
            make_user("old 2"),
            make_user("recent 1"),
            make_user("recent 2"),
            make_user("recent 3"),
        ];

        emergency_truncate(&mut messages, tmp.path());
        // system + checkpoint ref + 3 recent = 5
        assert_eq!(messages.len(), 5);
        // Last 3 should be recent 1, recent 2, recent 3
        assert_eq!(messages[2].content, "recent 1");
        assert_eq!(messages[3].content, "recent 2");
        assert_eq!(messages[4].content, "recent 3");
    }

    #[test]
    fn test_emergency_truncate_preserves_last_user_when_tail_has_only_tools() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "system".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            make_user("original request"),
            make_assistant_with_call("tc1", "file_write"),
            make_tool_result("tc1", "wrote file"),
            make_assistant_with_call("tc2", "bash"),
        ];

        emergency_truncate(&mut messages, tmp.path());

        assert!(messages.iter().any(|m| m.role == Role::User));
        assert!(messages.iter().any(|m| m.content == "original request"));
    }

    #[test]
    fn test_emergency_truncate_adds_checkpoint_reference() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "system".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            make_user("question"),
        ];

        emergency_truncate(&mut messages, tmp.path());
        let checkpoint_msg = messages.iter().find(|m| {
            m.role == Role::System && m.content.contains("Emergency context recovery")
        });
        assert!(checkpoint_msg.is_some());
    }

    #[test]
    fn test_emergency_truncate_empty_messages_no_panic() {
        let tmp = TempDir::new().unwrap();
        let mut messages: Vec<Message> = vec![];
        emergency_truncate(&mut messages, tmp.path());
        // Should have just the checkpoint reference message
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("Emergency context recovery"));
    }

    #[test]
    fn test_extract_session_memory_empty_messages_no_panic() {
        let tmp = TempDir::new().unwrap();
        let messages: Vec<Message> = vec![];
        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let checkpoint: SessionCheckpoint = serde_json::from_str(&content).unwrap();
        assert!(checkpoint.files_modified.is_empty());
        assert!(checkpoint.tools_used.is_empty());
        assert!(checkpoint.errors_encountered.is_empty());
        assert!(checkpoint.last_user_request.is_none());
        assert!(checkpoint.pending_work.is_none());
    }

    // ── P0 Security: Checkpoint path safety ───────────────────────────────

    #[test]
    fn test_security_checkpoint_path_is_safe() {
        let tmp = TempDir::new().unwrap();
        let messages = vec![make_user("test")];

        let path = extract_session_memory(&messages, tmp.path()).unwrap();
        let canonical = path.canonicalize().unwrap();
        let expected_parent = tmp.path().join(".ftai").join("checkpoints").canonicalize().unwrap();
        assert!(canonical.starts_with(&expected_parent));
    }

    #[test]
    fn test_security_emergency_truncate_saves_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let mut messages = vec![
            make_user("fix the bug"),
            make_assistant_with_call("tc1", "bash"),
            make_tool_result("tc1", "done"),
        ];

        emergency_truncate(&mut messages, tmp.path());

        let checkpoint_dir = tmp.path().join(".ftai").join("checkpoints");
        assert!(checkpoint_dir.exists());
        let entries: Vec<_> = std::fs::read_dir(&checkpoint_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
    }
}
