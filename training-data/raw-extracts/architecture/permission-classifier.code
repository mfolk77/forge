use serde_json::Value;

use super::grants::GrantCache;
use super::patterns::*;
use crate::config::PermissionMode;

/// Security tier for a tool action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionTier {
    Safe,
    Write,
    Destructive,
}

/// Result of a permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionVerdict {
    Approved,
    NeedsConfirmation(String),
    Blocked(String),
}

/// Check if a tool call is hard-blocked (compile-time safety, no override).
/// Returns Some(reason) if blocked, None if not.
pub fn hard_block_check(tool_name: &str, params: &Value) -> Option<String> {
    match tool_name {
        "bash" => {
            let command = params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let normalized = normalize_bash(command);

            for blocked in HARD_BLOCKED_COMMANDS {
                if normalized.contains(blocked) {
                    return Some(format!("Hard-blocked command: {blocked}"));
                }
            }

            for pattern in HARD_BLOCKED_PATTERNS {
                if command.contains(pattern) {
                    return Some(format!("Hard-blocked pattern detected: fork bomb or equivalent"));
                }
            }

            None
        }
        "file_write" | "file_edit" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");

            for blocked in HARD_BLOCKED_PATHS {
                if path == *blocked {
                    return Some(format!("Hard-blocked path: {blocked}"));
                }
            }

            for prefix in HARD_BLOCKED_PATH_PREFIXES {
                if path.starts_with(prefix) {
                    return Some(format!("Hard-blocked path prefix: {prefix}"));
                }
            }

            None
        }
        _ => None,
    }
}

/// Classify a tool call into a permission tier.
pub fn classify(tool_name: &str, params: &Value) -> PermissionTier {
    match tool_name {
        // Always safe: read-only tools
        "file_read" | "glob" | "grep" | "web_fetch" | "ask_user" => PermissionTier::Safe,

        // Always write: file modification tools
        "file_write" | "file_edit" => PermissionTier::Write,

        // Git: depends on subcommand
        "git" => classify_git(params),

        // Bash: depends on command content
        "bash" => classify_bash(params),

        // request_permissions is always safe (it's a meta-tool)
        "request_permissions" => PermissionTier::Safe,

        // Unknown tools default to Write
        _ => PermissionTier::Write,
    }
}

/// Determine the verdict for a tool call given the permission mode and grant cache.
pub fn check_permission(
    tier: PermissionTier,
    mode: &PermissionMode,
    grant_cache: &GrantCache,
    tool_name: &str,
    params: &Value,
) -> PermissionVerdict {
    match tier {
        PermissionTier::Safe => PermissionVerdict::Approved,

        PermissionTier::Write => match mode {
            PermissionMode::Ask => {
                if grant_cache.matches(tool_name, params) {
                    PermissionVerdict::Approved
                } else {
                    let desc = describe_action(tool_name, params);
                    PermissionVerdict::NeedsConfirmation(desc)
                }
            }
            PermissionMode::Auto | PermissionMode::Yolo => PermissionVerdict::Approved,
        },

        PermissionTier::Destructive => {
            // Destructive ALWAYS requires confirmation, regardless of mode.
            // Grant cache never covers destructive actions.
            let desc = describe_action(tool_name, params);
            PermissionVerdict::NeedsConfirmation(desc)
        }
    }
}

fn classify_git(params: &Value) -> PermissionTier {
    let subcommand = params
        .get("subcommand")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = params
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match subcommand {
        "status" | "diff" | "log" | "show" | "remote" | "tag" => PermissionTier::Safe,
        "branch" => {
            // branch -l / branch (list) is safe, branch -d / -D / create is Write
            if args.is_empty() || args.contains("-l") || args.contains("--list") {
                PermissionTier::Safe
            } else {
                PermissionTier::Write
            }
        }
        "add" | "commit" | "checkout" | "switch" | "merge" | "rebase" | "stash"
        | "pr_create" => PermissionTier::Write,
        "push" => PermissionTier::Destructive,
        "reset" => {
            if args.contains("--hard") {
                PermissionTier::Destructive
            } else {
                PermissionTier::Write
            }
        }
        _ => PermissionTier::Write,
    }
}

fn classify_bash(params: &Value) -> PermissionTier {
    let command = params
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Split compound commands and classify by worst segment
    let segments = split_compound_command(command);
    let mut worst = PermissionTier::Safe;

    for segment in &segments {
        let tier = classify_single_bash(segment);
        worst = worse_tier(worst, tier);
    }

    worst
}

fn classify_single_bash(cmd: &str) -> PermissionTier {
    let trimmed = cmd.trim();

    // Strip leading subshell wrappers: $(...), `...`, (...)
    let inner = strip_subshell(trimmed);

    // Check for destructive patterns first
    for pattern in DESTRUCTIVE_BASH_PATTERNS {
        if inner.starts_with(pattern) || inner.contains(&format!(" {pattern}")) {
            return PermissionTier::Destructive;
        }
    }

    // Check for system path access in write-like commands
    let first_word = inner.split_whitespace().next().unwrap_or("");
    if matches!(first_word, "tee" | "cp" | "mv" | "mkdir" | "touch") {
        for prefix in SYSTEM_PATH_PREFIXES {
            if inner.contains(prefix) {
                return PermissionTier::Destructive;
            }
        }
    }

    // Check safe commands
    for safe in SAFE_BASH_COMMANDS {
        if inner == *safe || inner.starts_with(&format!("{safe} ")) || inner.starts_with(&format!("{safe}\t")) {
            return PermissionTier::Safe;
        }
    }

    // Check write commands
    for write_cmd in WRITE_BASH_COMMANDS {
        if inner.starts_with(write_cmd) {
            return PermissionTier::Write;
        }
    }

    // Default: Write (unknown commands are treated as potentially modifying)
    PermissionTier::Write
}

/// Normalize bash command for hard-block matching:
/// collapse whitespace, strip tabs.
fn normalize_bash(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split compound commands on `&&`, `||`, `;`, `|`.
fn split_compound_command(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                current.push(c);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                current.push(c);
            }
            '&' if !in_single_quote && !in_double_quote => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                    segments.push(current.clone());
                    current.clear();
                } else {
                    current.push(c);
                }
            }
            '|' if !in_single_quote && !in_double_quote => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                segments.push(current.clone());
                current.clear();
            }
            ';' if !in_single_quote && !in_double_quote => {
                segments.push(current.clone());
                current.clear();
            }
            _ => current.push(c),
        }
    }

    if !current.trim().is_empty() {
        segments.push(current);
    }

    segments
}

/// Strip subshell wrappers like $(...), `...`, (...) from command start.
fn strip_subshell(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    if let Some(inner) = trimmed.strip_prefix("$(") {
        if let Some(stripped) = inner.strip_suffix(')') {
            return stripped.trim();
        }
    }
    if let Some(inner) = trimmed.strip_prefix('`') {
        if let Some(stripped) = inner.strip_suffix('`') {
            return stripped.trim();
        }
    }
    if let Some(inner) = trimmed.strip_prefix('(') {
        if let Some(stripped) = inner.strip_suffix(')') {
            return stripped.trim();
        }
    }
    trimmed
}

fn worse_tier(a: PermissionTier, b: PermissionTier) -> PermissionTier {
    match (a, b) {
        (PermissionTier::Destructive, _) | (_, PermissionTier::Destructive) => {
            PermissionTier::Destructive
        }
        (PermissionTier::Write, _) | (_, PermissionTier::Write) => PermissionTier::Write,
        _ => PermissionTier::Safe,
    }
}

fn describe_action(tool_name: &str, params: &Value) -> String {
    match tool_name {
        "bash" => {
            let cmd = params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            let preview: String = cmd.chars().take(80).collect();
            format!("Execute bash: {preview}")
        }
        "file_write" => {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            format!("Write file: {path}")
        }
        "file_edit" => {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            format!("Edit file: {path}")
        }
        "git" => {
            let sub = params
                .get("subcommand")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            let args = params
                .get("args")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Git {sub} {args}")
        }
        _ => format!("Execute {tool_name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_classify_safe_tools() {
        assert_eq!(classify("file_read", &json!({})), PermissionTier::Safe);
        assert_eq!(classify("glob", &json!({})), PermissionTier::Safe);
        assert_eq!(classify("grep", &json!({})), PermissionTier::Safe);
        assert_eq!(classify("web_fetch", &json!({})), PermissionTier::Safe);
        assert_eq!(classify("ask_user", &json!({})), PermissionTier::Safe);
    }

    #[test]
    fn test_classify_write_tools() {
        assert_eq!(classify("file_write", &json!({})), PermissionTier::Write);
        assert_eq!(classify("file_edit", &json!({})), PermissionTier::Write);
    }

    #[test]
    fn test_classify_git_safe() {
        assert_eq!(
            classify("git", &json!({"subcommand": "status"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "diff"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "log"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "branch"})),
            PermissionTier::Safe
        );
    }

    #[test]
    fn test_classify_git_write() {
        assert_eq!(
            classify("git", &json!({"subcommand": "add"})),
            PermissionTier::Write
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "commit"})),
            PermissionTier::Write
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "branch", "args": "-d feature"})),
            PermissionTier::Write
        );
    }

    #[test]
    fn test_classify_git_destructive() {
        assert_eq!(
            classify("git", &json!({"subcommand": "push"})),
            PermissionTier::Destructive
        );
        assert_eq!(
            classify("git", &json!({"subcommand": "reset", "args": "--hard HEAD~1"})),
            PermissionTier::Destructive
        );
    }

    #[test]
    fn test_classify_bash_safe() {
        assert_eq!(
            classify("bash", &json!({"command": "ls -la"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("bash", &json!({"command": "cat file.rs"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("bash", &json!({"command": "cargo test"})),
            PermissionTier::Safe
        );
        assert_eq!(
            classify("bash", &json!({"command": "git status"})),
            PermissionTier::Safe
        );
    }

    #[test]
    fn test_classify_bash_destructive() {
        assert_eq!(
            classify("bash", &json!({"command": "rm file.txt"})),
            PermissionTier::Destructive
        );
        assert_eq!(
            classify("bash", &json!({"command": "sudo apt update"})),
            PermissionTier::Destructive
        );
        assert_eq!(
            classify("bash", &json!({"command": "kill -9 1234"})),
            PermissionTier::Destructive
        );
    }

    #[test]
    fn test_classify_bash_compound_worst_wins() {
        // Safe && Destructive = Destructive
        assert_eq!(
            classify("bash", &json!({"command": "ls -la && rm file.txt"})),
            PermissionTier::Destructive
        );
        // Safe | Safe = Safe
        assert_eq!(
            classify("bash", &json!({"command": "ls | grep foo"})),
            PermissionTier::Safe
        );
    }

    #[test]
    fn test_hard_block_rm_rf_root() {
        let result = hard_block_check("bash", &json!({"command": "rm -rf /"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_hard_block_rm_rf_home() {
        let result = hard_block_check("bash", &json!({"command": "rm -rf ~"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_hard_block_obfuscated_whitespace() {
        // Extra whitespace should be normalized
        let result = hard_block_check("bash", &json!({"command": "rm  -rf   /"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_hard_block_file_write_etc_passwd() {
        let result = hard_block_check("file_write", &json!({"path": "/etc/passwd"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_hard_block_file_write_system() {
        let result = hard_block_check("file_write", &json!({"path": "/System/Library/foo"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_no_hard_block_safe_command() {
        let result = hard_block_check("bash", &json!({"command": "ls -la"}));
        assert!(result.is_none());
    }

    #[test]
    fn test_hard_block_fork_bomb() {
        let result = hard_block_check("bash", &json!({"command": ":(){ :|:& };:"}));
        assert!(result.is_some());
    }

    #[test]
    fn test_verdict_safe_always_approved() {
        let cache = GrantCache::new();
        let v = check_permission(
            PermissionTier::Safe,
            &PermissionMode::Ask,
            &cache,
            "file_read",
            &json!({}),
        );
        assert_eq!(v, PermissionVerdict::Approved);
    }

    #[test]
    fn test_verdict_write_ask_needs_confirmation() {
        let cache = GrantCache::new();
        let v = check_permission(
            PermissionTier::Write,
            &PermissionMode::Ask,
            &cache,
            "file_write",
            &json!({"path": "/tmp/test.txt"}),
        );
        assert!(matches!(v, PermissionVerdict::NeedsConfirmation(_)));
    }

    #[test]
    fn test_verdict_write_auto_approved() {
        let cache = GrantCache::new();
        let v = check_permission(
            PermissionTier::Write,
            &PermissionMode::Auto,
            &cache,
            "file_write",
            &json!({"path": "/tmp/test.txt"}),
        );
        assert_eq!(v, PermissionVerdict::Approved);
    }

    #[test]
    fn test_verdict_write_yolo_approved() {
        let cache = GrantCache::new();
        let v = check_permission(
            PermissionTier::Write,
            &PermissionMode::Yolo,
            &cache,
            "file_write",
            &json!({"path": "/tmp/test.txt"}),
        );
        assert_eq!(v, PermissionVerdict::Approved);
    }

    #[test]
    fn test_verdict_destructive_always_needs_confirmation() {
        let cache = GrantCache::new();

        // Even in Yolo mode
        let v = check_permission(
            PermissionTier::Destructive,
            &PermissionMode::Yolo,
            &cache,
            "bash",
            &json!({"command": "rm important.txt"}),
        );
        assert!(matches!(v, PermissionVerdict::NeedsConfirmation(_)));
    }

    #[test]
    fn test_normalize_bash_collapses_whitespace() {
        assert_eq!(normalize_bash("rm  -rf   /"), "rm -rf /");
        assert_eq!(normalize_bash("rm\t-rf\t/"), "rm -rf /");
    }

    #[test]
    fn test_split_compound_commands() {
        let segs = split_compound_command("ls && rm file");
        assert_eq!(segs.len(), 2);

        let segs = split_compound_command("ls ; echo hi ; rm file");
        assert_eq!(segs.len(), 3);

        let segs = split_compound_command("ls | grep foo");
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn test_strip_subshell() {
        assert_eq!(strip_subshell("$(rm -rf /)"), "rm -rf /");
        assert_eq!(strip_subshell("`rm -rf /`"), "rm -rf /");
        assert_eq!(strip_subshell("(rm -rf /)"), "rm -rf /");
        assert_eq!(strip_subshell("ls -la"), "ls -la");
    }

    #[test]
    fn test_subshell_destructive_classified() {
        assert_eq!(
            classify("bash", &json!({"command": "$(rm file.txt)"})),
            PermissionTier::Destructive
        );
    }
}
