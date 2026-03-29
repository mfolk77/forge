use serde::{Deserialize, Serialize};

/// Result type for a single tool invocation within a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolResultType {
    Success,
    Error(String),
    Timeout,
    Rejected,
    RuleBlocked,
}

/// Overall outcome of a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutcomeType {
    Success,
    PartialSuccess,
    Failure(String),
    Abandoned,
}

/// User feedback on an assistant response or session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UserFeedback {
    Accepted,
    Modified,
    Rejected,
    NoFeedback,
}

/// Record of a single tool call made during a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    /// Truncated summary of arguments (not full payload).
    pub arguments_summary: String,
    pub result_type: ToolResultType,
    pub duration_ms: u64,
}

/// Complete outcome record for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOutcome {
    pub session_id: String,
    pub project: String,
    pub timestamp: i64,
    pub task_description: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub success: OutcomeType,
    pub user_feedback: Option<UserFeedback>,
    pub total_tokens: usize,
    pub retries: usize,
}

impl OutcomeType {
    /// Whether this outcome counts as a success for pattern analysis.
    pub fn is_success(&self) -> bool {
        matches!(self, OutcomeType::Success | OutcomeType::PartialSuccess)
    }

    /// Serialize to a string for DB storage.
    pub fn to_db_string(&self) -> String {
        match self {
            OutcomeType::Success => "success".to_string(),
            OutcomeType::PartialSuccess => "partial_success".to_string(),
            OutcomeType::Failure(msg) => format!("failure:{msg}"),
            OutcomeType::Abandoned => "abandoned".to_string(),
        }
    }

    /// Deserialize from DB string.
    pub fn from_db_string(s: &str) -> Self {
        match s {
            "success" => OutcomeType::Success,
            "partial_success" => OutcomeType::PartialSuccess,
            "abandoned" => OutcomeType::Abandoned,
            other => {
                if let Some(msg) = other.strip_prefix("failure:") {
                    OutcomeType::Failure(msg.to_string())
                } else {
                    OutcomeType::Failure(other.to_string())
                }
            }
        }
    }
}

impl ToolResultType {
    pub fn to_db_string(&self) -> String {
        match self {
            ToolResultType::Success => "success".to_string(),
            ToolResultType::Error(msg) => format!("error:{msg}"),
            ToolResultType::Timeout => "timeout".to_string(),
            ToolResultType::Rejected => "rejected".to_string(),
            ToolResultType::RuleBlocked => "rule_blocked".to_string(),
        }
    }

    pub fn from_db_string(s: &str) -> Self {
        match s {
            "success" => ToolResultType::Success,
            "timeout" => ToolResultType::Timeout,
            "rejected" => ToolResultType::Rejected,
            "rule_blocked" => ToolResultType::RuleBlocked,
            other => {
                if let Some(msg) = other.strip_prefix("error:") {
                    ToolResultType::Error(msg.to_string())
                } else {
                    ToolResultType::Error(other.to_string())
                }
            }
        }
    }
}

impl UserFeedback {
    pub fn to_db_string(&self) -> &'static str {
        match self {
            UserFeedback::Accepted => "accepted",
            UserFeedback::Modified => "modified",
            UserFeedback::Rejected => "rejected",
            UserFeedback::NoFeedback => "no_feedback",
        }
    }

    pub fn from_db_string(s: &str) -> Self {
        match s {
            "accepted" => UserFeedback::Accepted,
            "modified" => UserFeedback::Modified,
            "rejected" => UserFeedback::Rejected,
            _ => UserFeedback::NoFeedback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_type_roundtrip() {
        let cases = vec![
            OutcomeType::Success,
            OutcomeType::PartialSuccess,
            OutcomeType::Failure("compile error".to_string()),
            OutcomeType::Abandoned,
        ];
        for case in cases {
            let s = case.to_db_string();
            let restored = OutcomeType::from_db_string(&s);
            assert_eq!(case, restored);
        }
    }

    #[test]
    fn test_outcome_is_success() {
        assert!(OutcomeType::Success.is_success());
        assert!(OutcomeType::PartialSuccess.is_success());
        assert!(!OutcomeType::Failure("oops".into()).is_success());
        assert!(!OutcomeType::Abandoned.is_success());
    }

    #[test]
    fn test_tool_result_type_roundtrip() {
        let cases = vec![
            ToolResultType::Success,
            ToolResultType::Error("not found".to_string()),
            ToolResultType::Timeout,
            ToolResultType::Rejected,
            ToolResultType::RuleBlocked,
        ];
        for case in cases {
            let s = case.to_db_string();
            let restored = ToolResultType::from_db_string(&s);
            assert_eq!(case, restored);
        }
    }

    #[test]
    fn test_user_feedback_roundtrip() {
        let cases = vec![
            UserFeedback::Accepted,
            UserFeedback::Modified,
            UserFeedback::Rejected,
            UserFeedback::NoFeedback,
        ];
        for case in cases {
            let s = case.to_db_string();
            let restored = UserFeedback::from_db_string(s);
            assert_eq!(case, restored);
        }
    }

    #[test]
    fn test_session_outcome_serialization() {
        let outcome = SessionOutcome {
            session_id: "sess-001".to_string(),
            project: "ftai".to_string(),
            timestamp: 1711500000,
            task_description: "Add tests".to_string(),
            tool_calls: vec![ToolCallRecord {
                tool_name: "file_read".to_string(),
                arguments_summary: "path=/src/main.rs".to_string(),
                result_type: ToolResultType::Success,
                duration_ms: 12,
            }],
            success: OutcomeType::Success,
            user_feedback: Some(UserFeedback::Accepted),
            total_tokens: 4500,
            retries: 0,
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let restored: SessionOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.session_id, "sess-001");
        assert_eq!(restored.tool_calls.len(), 1);
        assert_eq!(restored.success, OutcomeType::Success);
    }
}
