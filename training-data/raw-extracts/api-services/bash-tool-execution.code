use anyhow::Result;
use serde_json::Value;
use std::sync::Mutex;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::registry::{CancelToken, Tool, ToolContext, ToolProgress, ToolResult};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Returns the platform-appropriate shell executable and its prefix arguments.
///
/// On Unix: `("bash", &["-c"])` — commands are passed as a single string to bash.
/// On Windows: `("cmd.exe", &["/C"])` — commands are passed as a single string to cmd.exe.
///
/// # Security note
/// On both platforms the entire command string is interpreted by the shell, so
/// shell metacharacters (`&`, `|`, `>`, `<`, `;`) are live. This is by design —
/// the tool is a shell, not an exec wrapper. The timeout is the primary guard.
pub fn shell_command() -> (&'static str, &'static [&'static str]) {
    if cfg!(windows) {
        ("cmd.exe", &["/C"])
    } else {
        ("bash", &["-c"])
    }
}

/// Returns true if the given path string is absolute on the current platform.
fn is_absolute_path(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    // Windows drive-letter paths like C:\ or D:/
    if cfg!(windows) && path.len() >= 3 {
        let bytes = path.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
            return true;
        }
    }
    false
}

pub struct BashTool {
    cwd: Mutex<Option<PathBuf>>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            cwd: Mutex::new(None),
        }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        if cfg!(windows) {
            "Execute a shell command via cmd.exe. Working directory persists between calls."
        } else {
            "Execute a bash command. Working directory persists between calls."
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 120000, max: 600000)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let command = params["command"].as_str().unwrap_or("").to_string();
        let timeout_ms = params["timeout_ms"].as_u64().unwrap_or(120_000).min(600_000);
        let project_path = ctx.cwd.clone();

        Box::pin(async move {
            if command.is_empty() {
                return Ok(ToolResult::error("No command provided"));
            }

            let working_dir = {
                let guard = self.cwd.lock().unwrap();
                guard.clone().unwrap_or(project_path)
            };

            let (shell, prefix_args) = shell_command();

            let mut cmd = Command::new(shell);
            for arg in prefix_args {
                cmd.arg(arg);
            }
            cmd.arg(&command)
                .current_dir(&working_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // On Windows, set CREATE_BREAKAWAY_FROM_JOB to allow child process
            // management. Resource limits (RLIMIT_CPU) are Unix-only; on Windows
            // the timeout is the primary execution guard.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
                cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
            }

            let result = timeout(
                Duration::from_millis(timeout_ms),
                cmd.output(),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    // Update cwd if the command was a cd
                    if command.starts_with("cd ") {
                        let dir = command.trim_start_matches("cd ").trim();
                        let new_dir = if is_absolute_path(dir) {
                            PathBuf::from(dir)
                        } else {
                            working_dir.join(dir)
                        };
                        if new_dir.exists() {
                            *self.cwd.lock().unwrap() = Some(new_dir);
                        }
                    }

                    let mut result = String::new();
                    if !stdout.is_empty() {
                        result.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&format!("stderr: {stderr}"));
                    }

                    if output.status.success() {
                        Ok(ToolResult::success(if result.is_empty() {
                            "(no output)".to_string()
                        } else {
                            result
                        }))
                    } else {
                        Ok(ToolResult::error(format!(
                            "Exit code {}\n{result}",
                            output.status.code().unwrap_or(-1)
                        )))
                    }
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute: {e}"))),
                Err(_) => Ok(ToolResult::error(format!(
                    "Command timed out after {timeout_ms}ms"
                ))),
            }
        })
    }

    fn execute_with_cancel(
        &self,
        params: Value,
        ctx: &ToolContext,
        cancel: &CancelToken,
        progress: Option<tokio::sync::mpsc::Sender<ToolProgress>>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let command = params["command"].as_str().unwrap_or("").to_string();
        let timeout_ms = params["timeout_ms"].as_u64().unwrap_or(120_000).min(600_000);
        let project_path = ctx.cwd.clone();
        let cancel_rx = cancel.clone_receiver();

        Box::pin(async move {
            if command.is_empty() {
                return Ok(ToolResult::error("No command provided"));
            }

            let working_dir = {
                let guard = self.cwd.lock().unwrap();
                guard.clone().unwrap_or(project_path)
            };

            let (shell, prefix_args) = shell_command();

            let mut cmd = Command::new(shell);
            for arg in prefix_args {
                cmd.arg(arg);
            }
            cmd.arg(&command)
                .current_dir(&working_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
                cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
            }

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Ok(ToolResult::error(format!("Failed to execute: {e}"))),
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let mut collected_stdout = String::new();
            let mut collected_stderr = String::new();

            // Read stdout line-by-line, checking for cancellation
            if let Some(out) = stdout {
                let mut reader = BufReader::new(out).lines();
                let deadline = tokio::time::Instant::now()
                    + Duration::from_millis(timeout_ms);

                loop {
                    // Check cancellation
                    if *cancel_rx.borrow() {
                        let _ = child.kill().await;
                        return Ok(ToolResult::error("Cancelled by user"));
                    }

                    let line_result = tokio::select! {
                        line = reader.next_line() => line,
                        _ = tokio::time::sleep_until(deadline) => {
                            let _ = child.kill().await;
                            return Ok(ToolResult::error(
                                format!("Command timed out after {timeout_ms}ms"),
                            ));
                        }
                    };

                    match line_result {
                        Ok(Some(line)) => {
                            if let Some(ref tx) = progress {
                                let _ = tx.try_send(ToolProgress::PartialOutput(line.clone()));
                            }
                            if !collected_stdout.is_empty() {
                                collected_stdout.push('\n');
                            }
                            collected_stdout.push_str(&line);
                        }
                        Ok(None) => break, // EOF
                        Err(e) => {
                            collected_stderr
                                .push_str(&format!("Error reading stdout: {e}"));
                            break;
                        }
                    }
                }
            }

            // Read remaining stderr
            if let Some(err_out) = stderr {
                let mut reader = BufReader::new(err_out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !collected_stderr.is_empty() {
                        collected_stderr.push('\n');
                    }
                    collected_stderr.push_str(&line);
                }
            }

            // Final cancel check before waiting on exit status
            if *cancel_rx.borrow() {
                let _ = child.kill().await;
                return Ok(ToolResult::error("Cancelled by user"));
            }

            let status = match child.wait().await {
                Ok(s) => s,
                Err(e) => return Ok(ToolResult::error(format!("Failed to wait: {e}"))),
            };

            // Update cwd if the command was a cd
            if command.starts_with("cd ") {
                let dir = command.trim_start_matches("cd ").trim();
                let new_dir = if is_absolute_path(dir) {
                    PathBuf::from(dir)
                } else {
                    working_dir.join(dir)
                };
                if new_dir.exists() {
                    *self.cwd.lock().unwrap() = Some(new_dir);
                }
            }

            let mut result = String::new();
            if !collected_stdout.is_empty() {
                result.push_str(&collected_stdout);
            }
            if !collected_stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&format!("stderr: {collected_stderr}"));
            }

            if status.success() {
                Ok(ToolResult::success(if result.is_empty() {
                    "(no output)".to_string()
                } else {
                    result
                }))
            } else {
                Ok(ToolResult::error(format!(
                    "Exit code {}\n{result}",
                    status.code().unwrap_or(-1)
                )))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from(if cfg!(windows) { "C:\\Windows\\Temp" } else { "/tmp" }),
            project_path: PathBuf::from(if cfg!(windows) { "C:\\Windows\\Temp" } else { "/tmp" }),
        }
    }

    // -----------------------------------------------------------
    // shell_command() returns the correct shell for each platform
    // -----------------------------------------------------------

    #[test]
    fn test_shell_command_returns_correct_shell() {
        let (shell, args) = shell_command();
        if cfg!(windows) {
            assert_eq!(shell, "cmd.exe");
            assert_eq!(args, &["/C"]);
        } else {
            assert_eq!(shell, "bash");
            assert_eq!(args, &["-c"]);
        }
    }

    // -----------------------------------------------------------
    // is_absolute_path — unit tests for cross-platform detection
    // -----------------------------------------------------------

    #[test]
    fn test_unix_absolute_path() {
        assert!(is_absolute_path("/usr/bin"));
        assert!(is_absolute_path("/"));
        assert!(!is_absolute_path("relative/path"));
    }

    #[test]
    fn test_windows_absolute_path_detection() {
        // Drive-letter detection only activates on Windows builds,
        // but is_absolute_path("/foo") should always be true.
        if cfg!(windows) {
            assert!(is_absolute_path("C:\\Users"));
            assert!(is_absolute_path("D:/Projects"));
            assert!(!is_absolute_path("relative\\path"));
        }
    }

    // -----------------------------------------------------------
    // Existing tests — run on all platforms
    // -----------------------------------------------------------

    #[tokio::test]
    async fn test_echo() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_failing_command() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "false"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_empty_command() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": ""}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_timeout() {
        let tool = BashTool::new();
        let result = tool
            .execute(
                serde_json::json!({"command": "sleep 10", "timeout_ms": 100}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    // -----------------------------------------------------------
    // Windows-specific tests
    // -----------------------------------------------------------

    #[cfg(windows)]
    #[tokio::test]
    async fn test_windows_echo() {
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_windows_cd_drive_letter() {
        // Verify that cd with a drive letter is detected as absolute
        let tool = BashTool::new();
        // Just test the path detection logic directly
        assert!(is_absolute_path("C:\\Users"));
        assert!(is_absolute_path("D:\\Projects\\foo"));
        assert!(!is_absolute_path("Users\\test"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_windows_timeout() {
        let tool = BashTool::new();
        let result = tool
            .execute(
                serde_json::json!({"command": "ping -n 10 127.0.0.1", "timeout_ms": 100}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    // -----------------------------------------------------------
    // P0 Security: command injection via shell metacharacters
    // -----------------------------------------------------------
    // NOTE: On both platforms, the command string is passed to the shell
    // interpreter (bash -c / cmd.exe /C) as a single argument. This means
    // metacharacters (&, |, >, <, ;) ARE interpreted by the shell. This is
    // intentional — the tool IS a shell. The security boundary is the
    // timeout + the permission system, not argument escaping.
    //
    // Rust's Command API passes arguments safely to the OS, but the shell
    // itself will parse the string. This is identical behaviour on Unix and
    // Windows.

    #[cfg(unix)]
    #[tokio::test]
    async fn test_security_metacharacters_execute_in_shell() {
        let tool = BashTool::new();
        // The pipe should be interpreted by bash — this is expected behaviour
        let result = tool
            .execute(serde_json::json!({"command": "echo injected | cat"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("injected"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_security_windows_cmd_metacharacters() {
        // On Windows, & is a command separator in cmd.exe.
        // "echo safe & echo injected" should produce both outputs.
        // This documents that cmd.exe /C is equivalent to bash -c in risk profile.
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo safe & echo injected"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("safe"));
        assert!(result.output.contains("injected"));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn test_security_backtick_injection_passes_to_shell() {
        // P0 security red test
        // Backticks are interpreted by the shell — this is by design since BashTool
        // IS a shell. The permission system is the security boundary, not escaping.
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo `echo nested`"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        // The shell should have evaluated the backtick expression
        assert!(result.output.contains("nested"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_security_null_byte_in_command() {
        // P0 security red test
        // Null byte in command string — bash should handle this gracefully
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo before\x00after"}), &ctx())
            .await
            .unwrap();
        // Should not panic — bash may truncate at null or pass through
        // The important thing is no crash
        assert!(result.output.contains("before") || result.is_error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_security_cd_path_traversal_is_legitimate() {
        // P0 security red test
        // cd ../../.. is a legitimate shell operation — verify it works, not blocked
        let tool = BashTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "cd / && pwd"}), &ctx())
            .await
            .unwrap();
        // cd to root is valid
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_security_empty_command_returns_error() {
        // P0 security red test
        // Empty command must return error, not panic
        let tool = BashTool::new();

        // Completely empty
        let result = tool
            .execute(serde_json::json!({"command": ""}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);

        // Missing command key
        let result = tool
            .execute(serde_json::json!({}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);

        // Null command
        let result = tool
            .execute(serde_json::json!({"command": null}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_security_very_long_command_no_hang() {
        // P0 security red test
        // 100KB command string doesn't hang or crash (with short timeout)
        let tool = BashTool::new();
        let long_cmd = format!("echo {}", "A".repeat(100_000));
        let result = tool
            .execute(
                serde_json::json!({"command": long_cmd, "timeout_ms": 5000}),
                &ctx(),
            )
            .await
            .unwrap();
        // Should succeed or timeout — either is acceptable, no panic/hang
        assert!(!result.output.is_empty());
    }
}
