use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::config::Config;

fn default_timeout() -> u64 {
    10000
}

/// Configuration for a single user-defined hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: String,
    pub command: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

/// Runs user-configurable hooks defined in config.toml.
///
/// This is separate from the plugin hook system in `src/plugins/hooks.rs`.
/// Plugin hooks are sandboxed per-plugin scripts; these are user-level
/// shell commands triggered by lifecycle events.
pub struct HookRunner {
    hooks: Vec<HookConfig>,
}

impl HookRunner {
    pub fn new(hooks: Vec<HookConfig>) -> Self {
        Self { hooks }
    }

    /// Build a HookRunner from the loaded Config.
    pub fn from_config(config: &Config) -> Self {
        Self {
            hooks: config.hooks.clone(),
        }
    }

    /// Returns true for events whose hooks can block the action on non-zero exit.
    pub fn is_blocking_event(event: &str) -> bool {
        matches!(event, "before_tool" | "before_commit")
    }

    /// Run all hooks matching the given event.
    ///
    /// `env` is a map of environment variables to set for the subprocess
    /// (e.g. FORGE_TOOL_NAME, FORGE_FILE_PATH).
    ///
    /// For blocking events (`before_tool`, `before_commit`):
    ///   - A non-zero exit code causes this method to return `Err` with stderr.
    ///
    /// For non-blocking events:
    ///   - Errors are logged but do not propagate.
    pub async fn run(&self, event: &str, env: &HashMap<String, String>) -> Result<()> {
        let blocking = Self::is_blocking_event(event);

        for hook in &self.hooks {
            if hook.event != event {
                continue;
            }

            // If the hook has a tool filter, only run when FORGE_TOOL_NAME matches.
            if let Some(ref tool_filter) = hook.tool {
                match env.get("FORGE_TOOL_NAME") {
                    Some(tool_name) if tool_name == tool_filter => {}
                    _ => continue,
                }
            }

            let result = timeout(
                Duration::from_millis(hook.timeout_ms),
                Command::new("sh")
                    .arg("-c")
                    .arg(&hook.command)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .envs(env.iter())
                    .kill_on_drop(true)
                    .output(),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        if blocking {
                            bail!(
                                "Hook '{}' blocked: {}",
                                hook.description.as_deref().unwrap_or(&hook.command),
                                stderr.trim()
                            );
                        }
                        // Non-blocking: log and continue
                        eprintln!(
                            "[hook] '{}' failed (non-blocking): {}",
                            hook.description.as_deref().unwrap_or(&hook.command),
                            stderr.trim()
                        );
                    }
                }
                Ok(Err(e)) => {
                    let msg = format!(
                        "Hook '{}' error: {e}",
                        hook.description.as_deref().unwrap_or(&hook.command)
                    );
                    if blocking {
                        bail!("{msg}");
                    }
                    eprintln!("[hook] {msg}");
                }
                Err(_) => {
                    let msg = format!(
                        "Hook '{}' timed out after {}ms",
                        hook.description.as_deref().unwrap_or(&hook.command),
                        hook.timeout_ms
                    );
                    if blocking {
                        bail!("{msg}");
                    }
                    eprintln!("[hook] {msg}");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_is_blocking_event() {
        assert!(HookRunner::is_blocking_event("before_tool"));
        assert!(HookRunner::is_blocking_event("before_commit"));
        assert!(!HookRunner::is_blocking_event("session_start"));
        assert!(!HookRunner::is_blocking_event("session_end"));
        assert!(!HookRunner::is_blocking_event("after_tool"));
        assert!(!HookRunner::is_blocking_event("after_file_edit"));
    }

    #[tokio::test]
    async fn test_hook_runs_command() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "after_tool".to_string(),
            command: "echo hello".to_string(),
            tool: None,
            description: Some("echo test".to_string()),
            timeout_ms: 5000,
        }]);
        let env = HashMap::new();
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_hook_timeout_kills_process() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "after_tool".to_string(),
            command: "sleep 60".to_string(),
            tool: None,
            description: Some("slow hook".to_string()),
            timeout_ms: 100,
        }]);
        let env = HashMap::new();
        // Non-blocking event: timeout is logged but returns Ok
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_blocking_hook_timeout_returns_err() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "before_tool".to_string(),
            command: "sleep 60".to_string(),
            tool: None,
            description: Some("slow blocking hook".to_string()),
            timeout_ms: 100,
        }]);
        let env = HashMap::new();
        let result = runner.run("before_tool", &env).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_blocking_hook_blocks_on_nonzero_exit() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "before_tool".to_string(),
            command: "echo 'denied' >&2; exit 1".to_string(),
            tool: None,
            description: Some("deny hook".to_string()),
            timeout_ms: 5000,
        }]);
        let env = HashMap::new();
        let result = runner.run("before_tool", &env).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("denied"));
    }

    #[tokio::test]
    async fn test_nonblocking_hook_continues_on_nonzero_exit() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "after_tool".to_string(),
            command: "exit 1".to_string(),
            tool: None,
            description: Some("failing hook".to_string()),
            timeout_ms: 5000,
        }]);
        let env = HashMap::new();
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_filter_matches() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "before_tool".to_string(),
            command: "echo matched".to_string(),
            tool: Some("bash".to_string()),
            description: Some("bash-only hook".to_string()),
            timeout_ms: 5000,
        }]);
        let mut env = HashMap::new();
        env.insert("FORGE_TOOL_NAME".to_string(), "bash".to_string());
        let result = runner.run("before_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_filter_nonmatch_skips() {
        let runner = HookRunner::new(vec![HookConfig {
            event: "before_tool".to_string(),
            command: "exit 1".to_string(),
            tool: Some("bash".to_string()),
            description: Some("bash-only hook".to_string()),
            timeout_ms: 5000,
        }]);
        let mut env = HashMap::new();
        env.insert("FORGE_TOOL_NAME".to_string(), "file_write".to_string());
        // Hook should be skipped entirely (exit 1 would fail if it ran)
        let result = runner.run("before_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_multiple_hooks_same_event_all_run() {
        // Use two hooks that both write to a temp file to prove both ran.
        let tmp = std::env::temp_dir().join("forge_hook_test_multi");
        let _ = std::fs::remove_file(&tmp);
        let cmd1 = format!("printf A >> {}", tmp.display());
        let cmd2 = format!("printf B >> {}", tmp.display());

        let runner = HookRunner::new(vec![
            HookConfig {
                event: "after_tool".to_string(),
                command: cmd1,
                tool: None,
                description: None,
                timeout_ms: 5000,
            },
            HookConfig {
                event: "after_tool".to_string(),
                command: cmd2,
                tool: None,
                description: None,
                timeout_ms: 5000,
            },
        ]);
        let env = HashMap::new();
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "AB");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_empty_hooks_is_noop() {
        let runner = HookRunner::new(vec![]);
        let env = HashMap::new();
        let result = runner.run("before_tool", &env).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_env_vars_passed_to_subprocess() {
        let tmp = std::env::temp_dir().join("forge_hook_test_env");
        let _ = std::fs::remove_file(&tmp);
        let cmd = format!("printf '%s' \"$FORGE_TOOL_NAME\" > {}", tmp.display());

        let runner = HookRunner::new(vec![HookConfig {
            event: "after_tool".to_string(),
            command: cmd,
            tool: None,
            description: None,
            timeout_ms: 5000,
        }]);
        let mut env = HashMap::new();
        env.insert("FORGE_TOOL_NAME".to_string(), "file_write".to_string());
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "file_write");
        let _ = std::fs::remove_file(&tmp);
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_p0_command_injection_via_env_vars() {
        // FORGE_TOOL_NAME with semicolons should NOT execute injected commands.
        // The env var is passed as data, not interpolated into the command string.
        let tmp = std::env::temp_dir().join("forge_hook_test_injection");
        let _ = std::fs::remove_file(&tmp);

        // The hook command itself is safe — it just echoes the env var.
        // The attack is in the env var value containing shell metacharacters.
        let runner = HookRunner::new(vec![HookConfig {
            event: "after_tool".to_string(),
            command: "true".to_string(),
            tool: None,
            description: None,
            timeout_ms: 5000,
        }]);
        let mut env = HashMap::new();
        // Malicious value — should not create the file
        env.insert(
            "FORGE_TOOL_NAME".to_string(),
            format!("; touch {}", tmp.display()),
        );
        let result = runner.run("after_tool", &env).await;
        assert!(result.is_ok());

        // The injected `touch` should NOT have executed
        assert!(
            !tmp.exists(),
            "Command injection via env var succeeded — P0 security violation"
        );
    }

    #[test]
    fn test_hook_config_deserializes_from_toml() {
        let toml_str = r#"
[[hooks]]
event = "after_file_edit"
command = "rustfmt $FORGE_FILE_PATH 2>/dev/null || true"
description = "Auto-format Rust files after edit"
timeout_ms = 10000

[[hooks]]
event = "before_tool"
command = "echo check"
tool = "bash"
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            hooks: Vec<HookConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.hooks.len(), 2);
        assert_eq!(parsed.hooks[0].event, "after_file_edit");
        assert_eq!(
            parsed.hooks[0].description.as_deref(),
            Some("Auto-format Rust files after edit")
        );
        assert_eq!(parsed.hooks[0].timeout_ms, 10000);
        assert_eq!(parsed.hooks[1].tool.as_deref(), Some("bash"));
        // Second hook should get default timeout
        assert_eq!(parsed.hooks[1].timeout_ms, 10000);
    }
}
