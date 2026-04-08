use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use crate::backend::types::ToolDefinition;

/// Result of a tool execution
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }

    pub fn error(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
        }
    }
}

/// Progress updates emitted by tools during execution
#[derive(Debug, Clone)]
pub enum ToolProgress {
    /// Completion percentage (0-100)
    Percent(u8),
    /// Human-readable status message
    Status(String),
    /// Partial output streamed during execution
    PartialOutput(String),
}

/// A cancellation token backed by a `tokio::sync::watch` channel.
///
/// The owner calls `cancel()` to signal; any holder of a cloned receiver
/// can poll `is_cancelled()` without overhead.
#[derive(Debug)]
pub struct CancelToken {
    sender: watch::Sender<bool>,
    receiver: watch::Receiver<bool>,
}

impl CancelToken {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(false);
        Self { sender, receiver }
    }

    /// Signal cancellation. Idempotent.
    pub fn cancel(&self) {
        self.sender.send(true).ok();
    }

    /// Check whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    /// Clone the receiver half so another task can observe the signal.
    pub fn clone_receiver(&self) -> watch::Receiver<bool> {
        self.receiver.clone()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle returned when launching a cancellable tool execution.
pub struct ToolExecution {
    pub cancel: CancelToken,
    pub progress_rx: mpsc::Receiver<ToolProgress>,
}

/// Context available to tools during execution
pub struct ToolContext {
    pub cwd: PathBuf,
    pub project_path: PathBuf,
}

/// A tool that the AI can invoke
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>;

    /// Execute with cancellation and progress support. Default delegates to `execute()`.
    fn execute_with_cancel(
        &self,
        params: Value,
        ctx: &ToolContext,
        _cancel: &CancelToken,
        _progress: Option<mpsc::Sender<ToolProgress>>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>> {
        self.execute(params, ctx)
    }

    /// Short summary of this tool call for logging/display.
    /// Default: `"tool_name({truncated_args})"`.
    fn classify_summary(&self, args: &Value) -> String {
        let s = args.to_string();
        let truncated = if s.len() > 80 {
            format!("{}...", &s[..77])
        } else {
            s
        };
        format!("{}({})", self.name(), truncated)
    }
}

/// Registry of all available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub async fn execute(&self, name: &str, params: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let cancel = CancelToken::new();
        self.execute_cancellable(name, params, ctx, &cancel, None).await
    }

    /// Create a cancellation token and progress channel for an execution.
    pub fn create_execution_context(&self) -> (CancelToken, mpsc::Receiver<ToolProgress>) {
        let cancel = CancelToken::new();
        let (_tx, rx) = mpsc::channel(64);
        (cancel, rx)
    }

    /// Execute a tool with external cancellation and optional progress.
    pub async fn execute_cancellable(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
        cancel: &CancelToken,
        progress: Option<mpsc::Sender<ToolProgress>>,
    ) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
        tool.execute_with_cancel(params, ctx, cancel, progress).await
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Returns true if this tool is read-only (safe for concurrent execution).
    pub fn is_read_only(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "file_read"
                | "grep"
                | "glob"
                | "web_fetch"
                | "ask_user"
                | "memory_read"
                | "research"
                | "request_permissions"
        )
    }

    /// Register multiple plugin tools at once.
    pub fn register_plugin_tools(&mut self, tools: Vec<impl Tool + 'static>) {
        for tool in tools {
            self.register(tool);
        }
    }

    /// Create registry with all default tools
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        reg.register(super::bash::BashTool::new());
        reg.register(super::file_read::FileReadTool);
        reg.register(super::file_write::FileWriteTool);
        reg.register(super::file_edit::FileEditTool);
        reg.register(super::glob_tool::GlobTool);
        reg.register(super::grep_tool::GrepTool);
        reg.register(super::git::GitTool);
        reg.register(super::web_fetch::WebFetchTool);
        reg.register(super::ask_user::AskUserTool);
        reg.register(super::request_permissions::RequestPermissionsTool);
        reg.register(super::memory_tool::MemoryReadTool);
        reg.register(super::memory_tool::MemoryWriteTool);
        reg.register(super::agent_spawn::AgentSpawnTool);
        reg.register(super::task_tool::TaskTool);
        reg.register(super::research_tool::ResearchTool);
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future;

    struct DummyTool;

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "A test tool"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        fn execute(
            &self,
            _params: Value,
            _ctx: &ToolContext,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>
        {
            Box::pin(async { Ok(ToolResult::success("done")) })
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        assert!(reg.get("dummy").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_tool_definitions() {
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let defs = reg.tool_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "dummy");
    }

    #[tokio::test]
    async fn test_execute() {
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };
        let result = reg.execute("dummy", serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output, "done");
    }

    #[tokio::test]
    async fn test_execute_unknown_tool() {
        let reg = ToolRegistry::new();
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };
        let result = reg.execute("nonexistent", serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_with_defaults() {
        let reg = ToolRegistry::with_defaults();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("file_read").is_some());
        assert!(reg.get("file_write").is_some());
        assert!(reg.get("file_edit").is_some());
        assert!(reg.get("glob").is_some());
        assert!(reg.get("grep").is_some());
        assert!(reg.get("git").is_some());
        assert!(reg.get("web_fetch").is_some());
        assert!(reg.get("ask_user").is_some());
        assert!(reg.get("memory_read").is_some());
        assert!(reg.get("memory_write").is_some());
        assert!(reg.get("agent_spawn").is_some());
        assert!(reg.get("task").is_some());
        assert!(reg.get("research").is_some());
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_security_execute_unknown_tool_returns_error() {
        // P0 security red test
        // Executing a non-existent tool must return Err, not panic
        let reg = ToolRegistry::new();
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        let result = reg.execute("does_not_exist", serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_security_execute_with_malformed_json_params() {
        // P0 security red test
        // Null and malformed JSON params must not panic
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        // null params
        let result = reg.execute("dummy", serde_json::Value::Null, &ctx).await;
        assert!(result.is_ok()); // DummyTool ignores params

        // array instead of object
        let result = reg.execute("dummy", serde_json::json!([1, 2, 3]), &ctx).await;
        assert!(result.is_ok());

        // deeply nested object
        let deep = serde_json::json!({"a": {"b": {"c": {"d": {"e": "deep"}}}}});
        let result = reg.execute("dummy", deep, &ctx).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_security_tool_name_with_path_traversal() {
        // P0 security red test
        // Tool names containing path traversal characters don't cause filesystem issues
        let reg = ToolRegistry::new();
        // Lookup of traversal names must return None
        assert!(reg.get("../../../etc/passwd").is_none());
        assert!(reg.get("tool/../../secret").is_none());
        assert!(reg.get("").is_none());
        assert!(reg.get("\x00").is_none());

        // Verify default tools don't have suspicious names
        let defaults = ToolRegistry::with_defaults();
        for name in defaults.tool_names() {
            assert!(!name.contains(".."), "Tool name contains path traversal: {}", name);
            assert!(!name.contains('/'), "Tool name contains slash: {}", name);
            assert!(!name.contains('\\'), "Tool name contains backslash: {}", name);
        }
    }

    #[tokio::test]
    async fn test_security_concurrent_tool_execution() {
        // P0 security red test
        // Multiple concurrent executions of the same tool don't race
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let reg = std::sync::Arc::new(reg);

        let mut handles = Vec::new();
        for _ in 0..10 {
            let reg = reg.clone();
            handles.push(tokio::spawn(async move {
                let ctx = ToolContext {
                    cwd: PathBuf::from("/tmp"),
                    project_path: PathBuf::from("/tmp"),
                };
                reg.execute("dummy", serde_json::json!({}), &ctx).await
            }));
        }

        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok());
            assert_eq!(result.unwrap().output, "done");
        }
    }

    // ── WS2: Tool Abort Signals + Progress Callbacks ──────────────────────

    #[test]
    fn test_classify_summary_truncates_long_args() {
        let tool = DummyTool;
        let long_val = "x".repeat(200);
        let args = serde_json::json!({"data": long_val});
        let summary = tool.classify_summary(&args);
        // Total arg string is >80 chars so it should be truncated with "..."
        assert!(summary.ends_with("...)"), "expected truncated summary, got: {summary}");
        assert!(summary.starts_with("dummy("));
        // The inner portion is 77 chars + "..."
        let inner = &summary["dummy(".len()..summary.len() - 1]; // strip trailing )
        // inner = first 77 chars of json + "..."
        assert_eq!(inner.len(), 80); // 77 + "..."
    }

    #[test]
    fn test_classify_summary_with_empty_args() {
        let tool = DummyTool;
        let args = serde_json::json!({});
        let summary = tool.classify_summary(&args);
        assert_eq!(summary, "dummy({})");
    }

    #[test]
    fn test_cancel_token_starts_uncancelled() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancel_token_cancel_sets_flag() {
        let token = CancelToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_cancel_token_clone_receiver_sees_cancellation() {
        let token = CancelToken::new();
        let mut rx = token.clone_receiver();
        assert!(!*rx.borrow());
        token.cancel();
        // The cloned receiver should see the updated value
        assert!(*rx.borrow_and_update());
    }

    #[test]
    fn test_create_execution_context_returns_working_pair() {
        let reg = ToolRegistry::new();
        let (cancel, _rx) = reg.create_execution_context();
        assert!(!cancel.is_cancelled());
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_bash_tool_cancel_mid_execution() {
        use crate::tools::bash::BashTool;

        let tool = BashTool::new();
        let cancel = CancelToken::new();
        let (tx, mut rx) = mpsc::channel(64);
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        // Spawn cancellation after a short delay
        let cancel_clone_rx = cancel.clone_receiver();
        let cancel_sender = CancelToken {
            sender: cancel.sender.clone(),
            receiver: cancel_clone_rx,
        };
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel_sender.cancel();
        });

        let result = tool
            .execute_with_cancel(
                serde_json::json!({"command": "sleep 10"}),
                &ctx,
                &cancel,
                Some(tx),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(
            result.output.contains("Cancelled"),
            "expected cancel message, got: {}",
            result.output
        );

        // Drain any partial output from the progress channel
        rx.close();
    }

    // ── Tool concurrency classification tests ──────────────────────────────

    #[test]
    fn test_is_read_only_true_for_read_tools() {
        assert!(ToolRegistry::is_read_only("file_read"));
        assert!(ToolRegistry::is_read_only("grep"));
        assert!(ToolRegistry::is_read_only("glob"));
        assert!(ToolRegistry::is_read_only("web_fetch"));
        assert!(ToolRegistry::is_read_only("ask_user"));
        assert!(ToolRegistry::is_read_only("memory_read"));
        assert!(ToolRegistry::is_read_only("research"));
        assert!(ToolRegistry::is_read_only("request_permissions"));
    }

    #[test]
    fn test_is_read_only_false_for_mutating_tools() {
        assert!(!ToolRegistry::is_read_only("file_write"));
        assert!(!ToolRegistry::is_read_only("file_edit"));
        assert!(!ToolRegistry::is_read_only("bash"));
        assert!(!ToolRegistry::is_read_only("git"));
        assert!(!ToolRegistry::is_read_only("memory_write"));
        assert!(!ToolRegistry::is_read_only("agent_spawn"));
        assert!(!ToolRegistry::is_read_only("task"));
    }

    #[tokio::test]
    async fn test_concurrent_read_only_execution() {
        // Multiple read-only tools should all complete successfully
        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let reg = std::sync::Arc::new(reg);
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        // Simulate concurrent execution pattern
        let futures: Vec<_> = (0..5).map(|_| {
            reg.execute("dummy", serde_json::json!({}), &ctx)
        }).collect();

        let results: Vec<_> = future::join_all(futures).await;
        for result in results {
            assert!(result.is_ok());
            assert_eq!(result.unwrap().output, "done");
        }
    }

    #[tokio::test]
    async fn test_serial_mutating_execution_order() {
        // Mutating tools execute serially — verify they all succeed in order
        use std::sync::atomic::{AtomicUsize, Ordering};
        

        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.store(0, Ordering::SeqCst);

        let mut reg = ToolRegistry::new();
        reg.register(DummyTool);
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        // Execute 3 times serially, verify all succeed
        let mut results = Vec::new();
        for _ in 0..3 {
            let result = reg.execute("dummy", serde_json::json!({}), &ctx).await;
            results.push(result);
        }

        for (i, result) in results.iter().enumerate() {
            assert!(result.is_ok(), "Tool call {} failed", i);
        }
    }

    #[tokio::test]
    async fn test_execute_cancellable_unknown_tool_returns_error() {
        let reg = ToolRegistry::new();
        let cancel = CancelToken::new();
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp"),
        };

        let result = reg
            .execute_cancellable("nonexistent", serde_json::json!({}), &ctx, &cancel, None)
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Unknown tool"));
    }
}
