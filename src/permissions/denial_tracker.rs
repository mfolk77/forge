use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::classifier::PermissionTier;

/// Tracks consecutive denials per tool-key and maintains a session-scoped allowlist.
///
/// Used alongside the static classifier to escalate repeatedly-denied tool calls
/// and to fast-path allowlisted ones.
#[allow(dead_code)]
pub struct DenialTracker {
    denial_streak: HashMap<String, u32>,
    session_allowlist: HashSet<String>,
}

#[allow(dead_code)]
impl DenialTracker {
    pub fn new() -> Self {
        Self {
            denial_streak: HashMap::new(),
            session_allowlist: HashSet::new(),
        }
    }

    /// Compute a stable grouping key for a tool call.
    ///
    /// - File tools (`file_read`, `file_write`, `file_edit`): `"{tool}:{path}"`
    /// - Bash: `"bash:{first_two_words_of_command}"`
    /// - Everything else: `"{tool}"`
    pub fn tool_key(name: &str, args: &Value) -> String {
        match name {
            "file_read" | "file_write" | "file_edit" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{name}:{path}")
            }
            "bash" => {
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let prefix: String = command
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("bash:{prefix}")
            }
            _ => name.to_string(),
        }
    }

    /// Record a denial for a tool call. Increments the streak for its key.
    pub fn record_denial(&mut self, name: &str, args: &Value) {
        let key = Self::tool_key(name, args);
        *self.denial_streak.entry(key).or_insert(0) += 1;
    }

    /// Reset the denial streak for a tool call (called on approval).
    pub fn reset_denials(&mut self, name: &str, args: &Value) {
        let key = Self::tool_key(name, args);
        self.denial_streak.remove(&key);
    }

    /// Returns true if the tool call has been denied 3+ consecutive times.
    pub fn should_escalate(&self, name: &str, args: &Value) -> bool {
        let key = Self::tool_key(name, args);
        self.denial_streak.get(&key).copied().unwrap_or(0) >= 3
    }

    /// Add a tool call's key to the session allowlist.
    /// Destructive-tier actions are NEVER allowlisted — this is a hard invariant.
    pub fn add_to_allowlist(&mut self, name: &str, args: &Value, tier: PermissionTier) {
        if tier == PermissionTier::Destructive {
            return; // Destructive actions always require confirmation
        }
        let key = Self::tool_key(name, args);
        self.session_allowlist.insert(key);
    }

    /// Check if a tool call is on the session allowlist.
    /// Returns false for Destructive-tier actions regardless of allowlist state.
    pub fn is_allowlisted(&self, name: &str, args: &Value, tier: PermissionTier) -> bool {
        if tier == PermissionTier::Destructive {
            return false;
        }
        let key = Self::tool_key(name, args);
        self.session_allowlist.contains(&key)
    }

    /// Clear all state (denial streaks and allowlist). Called on session end.
    pub fn clear(&mut self) {
        self.denial_streak.clear();
        self.session_allowlist.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_default_state_empty() {
        let tracker = DenialTracker::new();
        assert!(!tracker.should_escalate("bash", &json!({"command": "ls"})));
        assert!(!tracker.is_allowlisted("bash", &json!({"command": "ls"}), PermissionTier::Safe));
    }

    #[test]
    fn test_record_denial_increments() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm -rf /"});
        tracker.record_denial("bash", &args);
        tracker.record_denial("bash", &args);
        assert!(!tracker.should_escalate("bash", &args));
        tracker.record_denial("bash", &args);
        assert!(tracker.should_escalate("bash", &args));
    }

    #[test]
    fn test_three_denials_escalates() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm -rf /"});
        for _ in 0..3 {
            tracker.record_denial("bash", &args);
        }
        assert!(tracker.should_escalate("bash", &args));
    }

    #[test]
    fn test_two_denials_no_escalation() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm -rf /"});
        tracker.record_denial("bash", &args);
        tracker.record_denial("bash", &args);
        assert!(!tracker.should_escalate("bash", &args));
    }

    #[test]
    fn test_reset_denials_clears_streak() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm -rf /"});
        for _ in 0..3 {
            tracker.record_denial("bash", &args);
        }
        assert!(tracker.should_escalate("bash", &args));
        tracker.reset_denials("bash", &args);
        assert!(!tracker.should_escalate("bash", &args));
    }

    #[test]
    fn test_allowlist_bypasses() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "cargo build"});
        assert!(!tracker.is_allowlisted("bash", &args, PermissionTier::Write));
        tracker.add_to_allowlist("bash", &args, PermissionTier::Write);
        assert!(tracker.is_allowlisted("bash", &args, PermissionTier::Write));
    }

    #[test]
    fn test_tool_key_file_ops_grouped_by_path() {
        let args = json!({"path": "/tmp/foo.txt"});
        assert_eq!(
            DenialTracker::tool_key("file_read", &args),
            "file_read:/tmp/foo.txt"
        );
        assert_eq!(
            DenialTracker::tool_key("file_write", &args),
            "file_write:/tmp/foo.txt"
        );
        assert_eq!(
            DenialTracker::tool_key("file_edit", &args),
            "file_edit:/tmp/foo.txt"
        );
    }

    #[test]
    fn test_tool_key_bash_grouped_by_command_prefix() {
        let args = json!({"command": "cargo build --release"});
        assert_eq!(DenialTracker::tool_key("bash", &args), "bash:cargo build");

        let args2 = json!({"command": "cargo test"});
        assert_eq!(DenialTracker::tool_key("bash", &args2), "bash:cargo test");
    }

    #[test]
    fn test_tool_key_generic_uses_name() {
        let args = json!({});
        assert_eq!(DenialTracker::tool_key("web_fetch", &args), "web_fetch");
        assert_eq!(DenialTracker::tool_key("ask_user", &args), "ask_user");
    }

    #[test]
    fn test_clear_resets_everything() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm file"});
        tracker.record_denial("bash", &args);
        tracker.record_denial("bash", &args);
        tracker.record_denial("bash", &args);
        tracker.add_to_allowlist("bash", &json!({"command": "cargo build"}), PermissionTier::Write);

        tracker.clear();

        assert!(!tracker.should_escalate("bash", &args));
        assert!(!tracker.is_allowlisted("bash", &json!({"command": "cargo build"}), PermissionTier::Write));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_p0_tool_key_no_panic_on_malformed_args() {
        // Missing expected fields
        assert_eq!(DenialTracker::tool_key("file_read", &json!({})), "file_read:");
        assert_eq!(DenialTracker::tool_key("bash", &json!({})), "bash:");
        assert_eq!(DenialTracker::tool_key("file_write", &json!(null)), "file_write:");
        assert_eq!(DenialTracker::tool_key("bash", &json!(null)), "bash:");

        // Wrong types for expected fields
        assert_eq!(DenialTracker::tool_key("file_read", &json!({"path": 42})), "file_read:");
        assert_eq!(DenialTracker::tool_key("bash", &json!({"command": true})), "bash:");

        // Nested garbage
        assert_eq!(
            DenialTracker::tool_key("file_edit", &json!({"path": {"nested": "obj"}})),
            "file_edit:"
        );
    }

    #[test]
    fn test_p0_destructive_never_allowlisted() {
        let mut tracker = DenialTracker::new();
        let args = json!({"command": "rm -rf /"});

        // Attempt to allowlist a destructive action — must be silently rejected
        tracker.add_to_allowlist("bash", &args, PermissionTier::Destructive);
        assert!(
            !tracker.is_allowlisted("bash", &args, PermissionTier::Destructive),
            "Destructive-tier actions must NEVER be allowlisted"
        );

        // Even if the key was somehow inserted (e.g. via Write then reclassified),
        // is_allowlisted must still return false for Destructive tier
        tracker.add_to_allowlist("bash", &args, PermissionTier::Write);
        assert!(tracker.is_allowlisted("bash", &args, PermissionTier::Write));
        assert!(
            !tracker.is_allowlisted("bash", &args, PermissionTier::Destructive),
            "Same key queried as Destructive must be rejected"
        );
    }

    #[test]
    fn test_denial_tracking_is_per_key() {
        let mut tracker = DenialTracker::new();
        let args_a = json!({"command": "rm file_a"});
        let args_b = json!({"command": "rm file_b"});

        for _ in 0..3 {
            tracker.record_denial("bash", &args_a);
        }

        // args_a escalated, args_b not
        assert!(tracker.should_escalate("bash", &args_a));
        assert!(!tracker.should_escalate("bash", &args_b));
    }
}
