use anyhow::Result;
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

/// Rough token estimate: ~4 chars per token.
fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

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
pub fn should_compact(estimated_tokens: usize, max_context_tokens: usize) -> Option<u8> {
    if max_context_tokens == 0 {
        return Some(3);
    }
    let ratio = (estimated_tokens * 100) / max_context_tokens;
    if ratio > 85 {
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
        assert_eq!(should_compact(9000, 10000), Some(3));
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
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("Session summary"));
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
        assert_eq!(messages.len(), 1);

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
}
