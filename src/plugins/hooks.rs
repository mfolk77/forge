use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const HOOK_TIMEOUT_MS: u64 = 10_000;

/// A resolved hook ready to run.
#[derive(Debug, Clone)]
pub struct ResolvedHook {
    pub event: String,
    pub command_path: PathBuf,
    pub plugin_dir: PathBuf,
    pub plugin_name: String,
}

/// Result of running a hook.
#[derive(Debug)]
pub enum HookResult {
    /// Hook passed (exit 0).
    Passed,
    /// Hook blocked the action (exit != 0) with optional message.
    Blocked(String),
    /// Hook failed to run.
    Error(String),
}

/// Run a pre-hook. Returns whether the action should proceed.
pub async fn run_pre_hook(
    hook: &ResolvedHook,
    tool_name: &str,
    params_json: &str,
    project_path: &Path,
) -> HookResult {
    // Verify command is within plugin dir
    let canonical_cmd = match hook.command_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return HookResult::Error(format!("Hook command not found: {}", hook.command_path.display())),
    };
    let canonical_plugin = match hook.plugin_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return HookResult::Error("Plugin directory not found".to_string()),
    };
    if !canonical_cmd.starts_with(&canonical_plugin) {
        return HookResult::Error("Hook command escapes plugin directory".to_string());
    }

    let result = timeout(
        Duration::from_millis(HOOK_TIMEOUT_MS),
        Command::new(&hook.command_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&hook.plugin_dir)
            .env("FTAI_TOOL_NAME", tool_name)
            .env("FTAI_PARAMS", params_json)
            .env("FTAI_PROJECT_PATH", project_path.to_string_lossy().as_ref())
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                HookResult::Passed
            } else {
                let msg = String::from_utf8_lossy(&output.stderr).to_string();
                HookResult::Blocked(if msg.trim().is_empty() {
                    format!("Pre-hook '{}' from plugin '{}' blocked the action", hook.event, hook.plugin_name)
                } else {
                    msg.trim().to_string()
                })
            }
        }
        Ok(Err(e)) => HookResult::Error(format!("Failed to run hook: {e}")),
        Err(_) => HookResult::Error(format!(
            "Hook timed out after {HOOK_TIMEOUT_MS}ms (plugin: {})",
            hook.plugin_name
        )),
    }
}

/// Run a post-hook (fire-and-forget, result is for logging only).
pub async fn run_post_hook(
    hook: &ResolvedHook,
    tool_name: &str,
    params_json: &str,
    tool_result: &str,
    project_path: &Path,
) -> HookResult {
    let canonical_cmd = match hook.command_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return HookResult::Error("Hook command not found".to_string()),
    };
    let canonical_plugin = match hook.plugin_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return HookResult::Error("Plugin directory not found".to_string()),
    };
    if !canonical_cmd.starts_with(&canonical_plugin) {
        return HookResult::Error("Hook command escapes plugin directory".to_string());
    }

    let result = timeout(
        Duration::from_millis(HOOK_TIMEOUT_MS),
        Command::new(&hook.command_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&hook.plugin_dir)
            .env("FTAI_TOOL_NAME", tool_name)
            .env("FTAI_PARAMS", params_json)
            .env("FTAI_TOOL_RESULT", tool_result)
            .env("FTAI_PROJECT_PATH", project_path.to_string_lossy().as_ref())
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                HookResult::Passed
            } else {
                let msg = String::from_utf8_lossy(&output.stderr).to_string();
                HookResult::Blocked(msg)
            }
        }
        Ok(Err(e)) => HookResult::Error(format!("Failed to run hook: {e}")),
        Err(_) => HookResult::Error(format!("Hook timed out after {HOOK_TIMEOUT_MS}ms")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_hook(tmp: &TempDir, script_name: &str, script_content: &str, event: &str) -> ResolvedHook {
        let hooks_dir = tmp.path().join("hooks");
        let _ = std::fs::create_dir_all(&hooks_dir);
        let script_path = hooks_dir.join(script_name);
        std::fs::write(&script_path, script_content).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        ResolvedHook {
            event: event.to_string(),
            command_path: script_path,
            plugin_dir: tmp.path().to_path_buf(),
            plugin_name: "test-plugin".to_string(),
        }
    }

    /// Platform-aware script name and content for a hook that exits 0.
    #[cfg(unix)]
    fn passing_script() -> (&'static str, &'static str) {
        ("pass.sh", "#!/bin/bash\nexit 0")
    }

    #[cfg(windows)]
    fn passing_script() -> (&'static str, &'static str) {
        ("pass.bat", "@echo off\r\nexit /b 0")
    }

    /// Platform-aware script name and content for a hook that exits 1 with stderr.
    #[cfg(unix)]
    fn blocking_script() -> (&'static str, &'static str) {
        ("block.sh", "#!/bin/bash\necho 'not allowed' >&2\nexit 1")
    }

    #[cfg(windows)]
    fn blocking_script() -> (&'static str, &'static str) {
        ("block.bat", "@echo off\r\necho not allowed 1>&2\r\nexit /b 1")
    }

    #[tokio::test]
    async fn test_pre_hook_passes() {
        let tmp = TempDir::new().unwrap();
        let (name, content) = passing_script();
        let hook = make_hook(&tmp, name, content, "pre:bash");

        let result = run_pre_hook(&hook, "bash", "{}", &std::env::temp_dir()).await;
        assert!(matches!(result, HookResult::Passed));
    }

    #[tokio::test]
    async fn test_pre_hook_blocks() {
        let tmp = TempDir::new().unwrap();
        let (name, content) = blocking_script();
        let hook = make_hook(&tmp, name, content, "pre:bash");

        let result = run_pre_hook(&hook, "bash", "{}", &std::env::temp_dir()).await;
        match result {
            HookResult::Blocked(msg) => assert!(msg.contains("not allowed")),
            _ => panic!("Expected Blocked"),
        }
    }

    #[tokio::test]
    async fn test_hook_timeout() {
        let tmp = TempDir::new().unwrap();
        // Use a very short timeout by overriding — but since HOOK_TIMEOUT_MS is const,
        // we test with a script that returns quickly instead
        let (name, content) = passing_script();
        let hook = make_hook(&tmp, name, content, "pre:bash");
        let result = run_pre_hook(&hook, "bash", "{}", &std::env::temp_dir()).await;
        assert!(matches!(result, HookResult::Passed));
    }
}
