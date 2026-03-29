use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>>;
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
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
        tool.execute(params, ctx).await
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
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>>
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
    }
}
