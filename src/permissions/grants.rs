use serde_json::Value;
use std::time::Instant;

use super::classifier::{classify, PermissionTier};

/// The scope of a permission grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantScope {
    /// Grant for any call to this tool.
    Tool(String),
    /// Grant for this tool with a specific path.
    ToolWithPath(String, String),
    /// Grant for this tool with a specific command.
    ToolWithCommand(String, String),
}

/// A single permission grant from the user.
#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub tool_name: String,
    pub scope: GrantScope,
    #[allow(dead_code)]
    pub granted_at: Instant,
}

/// Cache of active permission grants. Cleared on `/clear` or `/permissions clear`.
pub struct GrantCache {
    grants: Vec<PermissionGrant>,
}

impl GrantCache {
    pub fn new() -> Self {
        Self {
            grants: Vec::new(),
        }
    }

    /// Add a new grant.
    pub fn add(&mut self, grant: PermissionGrant) {
        self.grants.push(grant);
    }

    /// Check if any grant covers this tool call.
    /// Grants NEVER cover Destructive-tier actions (enforced at the check_permission level).
    pub fn matches(&self, tool_name: &str, params: &Value) -> bool {
        self.grants.iter().any(|g| {
            if g.tool_name != tool_name {
                return false;
            }
            match &g.scope {
                GrantScope::Tool(_) => true,
                GrantScope::ToolWithPath(_, path) => {
                    params
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|p| p.starts_with(path.as_str()))
                        .unwrap_or(false)
                }
                GrantScope::ToolWithCommand(_, cmd) => {
                    params
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|c| {
                            // Prefix must match AND full command must not escalate to Destructive.
                            // This prevents "cargo ; rm -rf /" from matching a "cargo " grant.
                            c.starts_with(cmd.as_str())
                                && classify(tool_name, params) != PermissionTier::Destructive
                        })
                        .unwrap_or(false)
                }
            }
        })
    }

    /// Clear all grants.
    pub fn clear(&mut self) {
        self.grants.clear();
    }

    /// List all active grants for display.
    pub fn list(&self) -> Vec<String> {
        self.grants
            .iter()
            .map(|g| match &g.scope {
                GrantScope::Tool(name) => format!("{name} (all)"),
                GrantScope::ToolWithPath(name, path) => format!("{name} path:{path}"),
                GrantScope::ToolWithCommand(name, cmd) => {
                    let preview: String = cmd.chars().take(40).collect();
                    format!("{name} cmd:{preview}")
                }
            })
            .collect()
    }

    /// Number of active grants.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_grant_tool_scope_matches() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "file_write".to_string(),
            scope: GrantScope::Tool("file_write".to_string()),
            granted_at: Instant::now(),
        });

        assert!(cache.matches("file_write", &json!({"path": "/any/path"})));
        assert!(!cache.matches("file_edit", &json!({"path": "/any/path"})));
    }

    #[test]
    fn test_grant_path_scope_matches() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "file_write".to_string(),
            scope: GrantScope::ToolWithPath("file_write".to_string(), "/tmp/".to_string()),
            granted_at: Instant::now(),
        });

        assert!(cache.matches("file_write", &json!({"path": "/tmp/test.txt"})));
        assert!(!cache.matches("file_write", &json!({"path": "/etc/test.txt"})));
    }

    #[test]
    fn test_grant_command_scope_matches() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "bash".to_string(),
            scope: GrantScope::ToolWithCommand("bash".to_string(), "cargo ".to_string()),
            granted_at: Instant::now(),
        });

        assert!(cache.matches("bash", &json!({"command": "cargo build"})));
        assert!(!cache.matches("bash", &json!({"command": "npm install"})));
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "file_write".to_string(),
            scope: GrantScope::Tool("file_write".to_string()),
            granted_at: Instant::now(),
        });

        assert_eq!(cache.len(), 1);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(!cache.matches("file_write", &json!({})));
    }

    #[test]
    fn test_p0_grant_command_prefix_injection_blocked() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "bash".to_string(),
            scope: GrantScope::ToolWithCommand("bash".to_string(), "cargo ".to_string()),
            granted_at: Instant::now(),
        });

        // Clean cargo command should match
        assert!(cache.matches("bash", &json!({"command": "cargo build"})));
        // Compound command with destructive payload must NOT match
        assert!(
            !cache.matches("bash", &json!({"command": "cargo build ; rm -rf /"})),
            "Compound destructive command must not be covered by prefix grant"
        );
        assert!(
            !cache.matches("bash", &json!({"command": "cargo build && rm important.txt"})),
            "Compound destructive command must not be covered by prefix grant"
        );
    }

    #[test]
    fn test_cache_list() {
        let mut cache = GrantCache::new();
        cache.add(PermissionGrant {
            tool_name: "file_write".to_string(),
            scope: GrantScope::Tool("file_write".to_string()),
            granted_at: Instant::now(),
        });
        cache.add(PermissionGrant {
            tool_name: "bash".to_string(),
            scope: GrantScope::ToolWithCommand("bash".to_string(), "cargo".to_string()),
            granted_at: Instant::now(),
        });

        let list = cache.list();
        assert_eq!(list.len(), 2);
    }
}
