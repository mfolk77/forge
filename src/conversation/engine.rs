use anyhow::Result;
use std::path::PathBuf;
use crate::backend::types::{ChatRequest, ChatResponse, Message, Role, ToolDefinition};
use crate::config::Config;
use crate::conversation::compactor;

/// Manages the conversation state and context window
pub struct ConversationEngine {
    messages: Vec<Message>,
    system_prompt: String,
    tools: Vec<ToolDefinition>,
    max_context_tokens: usize,
    estimated_tokens: usize,
    project_path: Option<PathBuf>,
}

impl ConversationEngine {
    pub fn new(system_prompt: String, tools: Vec<ToolDefinition>, max_context_tokens: usize) -> Self {
        Self {
            messages: Vec::new(),
            system_prompt,
            tools,
            max_context_tokens,
            estimated_tokens: 0,
            project_path: None,
        }
    }

    /// Set the project path for transcript saving during compaction.
    pub fn set_project_path(&mut self, path: PathBuf) {
        self.project_path = Some(path);
    }

    /// Add a user message
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(Message {
            role: Role::User,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
        self.estimated_tokens += estimate_tokens(content);
    }

    /// Add an assistant response
    pub fn add_assistant_message(&mut self, response: ChatResponse) {
        self.estimated_tokens += estimate_tokens(&response.message.content);
        self.messages.push(response.message);
    }

    /// Add a tool result
    pub fn add_tool_result(&mut self, tool_call_id: &str, result: &str) {
        self.messages.push(Message {
            role: Role::Tool,
            content: result.to_string(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
        });
        self.estimated_tokens += estimate_tokens(result);
    }

    /// Build a ChatRequest for the model
    pub fn build_request(&self, config: &Config) -> ChatRequest {
        self.build_request_with_mode(config, false)
    }

    /// Build request optimized for mode. In chat mode, tools are omitted
    /// so the model doesn't waste prefill time on tool schemas.
    pub fn build_request_with_mode(&self, config: &Config, chat_mode: bool) -> ChatRequest {
        let mut messages = vec![Message {
            role: Role::System,
            content: self.system_prompt.clone(),
            tool_calls: None,
            tool_call_id: None,
        }];
        messages.extend(self.messages.clone());

        ChatRequest {
            messages,
            tools: if chat_mode { vec![] } else { self.tools.clone() },
            temperature: config.model.temperature,
            max_tokens: Some(4096),
            model_id: config.model.path.clone(),
        }
    }

    /// Run Tier 1 microcompact: replace old tool_result content with placeholders.
    /// This is cheap (no model calls) and should run before every LLM call.
    pub fn micro_compact(&mut self) -> usize {
        let compacted = compactor::micro_compact(&mut self.messages);
        if compacted > 0 {
            self.recalculate_tokens();
        }
        compacted
    }

    /// Check if compaction is needed based on token thresholds.
    /// Returns the tier (2 or 3) or None.
    pub fn should_compact(&self) -> Option<u8> {
        compactor::should_compact(self.estimated_tokens, self.max_context_tokens)
    }

    /// Five-tier compaction system:
    /// - Always runs Tier 1 (micro) first
    /// - If tokens > 70%: Tier 2 (snip — remove oldest, keep last 10)
    /// - If tokens > 85%: Tier 3 (summarize — replace all with summary)
    /// - Tier 4 (session memory extraction) runs as part of tier 5
    /// - If tokens > 95%: Tier 5 (emergency truncation — keep system + last 3)
    pub fn compact(&mut self) {
        // Tier 1 always runs
        self.micro_compact();

        let project_path = self.project_path.clone();

        match self.should_compact() {
            Some(5) => {
                // Tier 5: emergency truncation (includes tier 4 checkpoint)
                if let Some(ref path) = project_path {
                    compactor::emergency_truncate(&mut self.messages, path);
                } else {
                    // Fallback: aggressive snip
                    self.snip_compact_fallback();
                }
            }
            Some(3) => {
                // Tier 3: summarize compact
                if let Some(ref path) = project_path {
                    // Tier 4: extract session memory before summarize
                    let _ = compactor::extract_session_memory(&self.messages, path);
                    let _ = compactor::summarize_compact(&mut self.messages, path);
                    // Reinject system identity as first message
                    self.messages.insert(0, Message {
                        role: Role::System,
                        content: format!(
                            "[System context reinjected after compaction]\nTools available: {}",
                            self.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
                        ),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                } else {
                    // Fallback: simple snip if no project path
                    self.snip_compact_fallback();
                }
            }
            Some(2) => {
                // Tier 2: snip compact
                if let Some(ref path) = project_path {
                    let _ = compactor::snip_compact(&mut self.messages, path, 10);
                } else {
                    self.snip_compact_fallback();
                }
            }
            _ => {
                // No compaction needed beyond tier 1
            }
        }

        self.recalculate_tokens();
    }

    /// Fallback snip when no project path is available (no transcript saving).
    fn snip_compact_fallback(&mut self) {
        let keep_recent = 10.min(self.messages.len());
        if self.messages.len() <= keep_recent {
            return;
        }

        let old_messages = &self.messages[..self.messages.len() - keep_recent];
        let summary = Self::summarize_messages(old_messages);

        let mut new_messages = vec![Message {
            role: Role::System,
            content: format!("[Conversation summary: {}]", summary),
            tool_calls: None,
            tool_call_id: None,
        }];
        new_messages.extend_from_slice(&self.messages[self.messages.len() - keep_recent..]);

        self.messages = new_messages;
    }

    fn summarize_messages(messages: &[Message]) -> String {
        let mut summary_parts = Vec::new();
        for msg in messages {
            match msg.role {
                Role::User => {
                    if msg.content.len() > 100 {
                        summary_parts.push(format!("User asked: {}...", &msg.content[..100]));
                    }
                }
                Role::Assistant => {
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            summary_parts.push(format!("Used tool: {}", tc.name));
                        }
                    }
                }
                _ => {}
            }
        }
        if summary_parts.is_empty() {
            "Previous conversation context".to_string()
        } else {
            summary_parts.join("; ")
        }
    }

    /// Recalculate estimated token count from current messages.
    fn recalculate_tokens(&mut self) {
        self.estimated_tokens = self
            .messages
            .iter()
            .map(|m| estimate_tokens(&m.content))
            .sum();
    }

    /// Clear all messages
    pub fn clear(&mut self) {
        self.messages.clear();
        self.estimated_tokens = 0;
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn estimated_tokens(&self) -> usize {
        self.estimated_tokens
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn update_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    pub fn update_tools(&mut self, tools: Vec<ToolDefinition>) {
        self.tools = tools;
    }

    /// Inject contextual content (e.g., skill content) as a system message in the conversation.
    pub fn add_system_context(&mut self, content: &str) {
        self.messages.push(Message {
            role: Role::System,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
        self.estimated_tokens += estimate_tokens(content);
    }
}

/// Rough token estimate: ~4 chars per token for English text
fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> ConversationEngine {
        ConversationEngine::new(
            "You are a coding assistant.".to_string(),
            vec![],
            32768,
        )
    }

    #[test]
    fn test_add_user_message() {
        let mut engine = test_engine();
        engine.add_user_message("hello");
        assert_eq!(engine.message_count(), 1);
        assert_eq!(engine.messages()[0].role, Role::User);
        assert_eq!(engine.messages()[0].content, "hello");
    }

    #[test]
    fn test_add_tool_result() {
        let mut engine = test_engine();
        engine.add_tool_result("tc_1", "file contents here");
        assert_eq!(engine.message_count(), 1);
        assert_eq!(engine.messages()[0].role, Role::Tool);
        assert_eq!(engine.messages()[0].tool_call_id, Some("tc_1".to_string()));
    }

    #[test]
    fn test_build_request() {
        let mut engine = test_engine();
        engine.add_user_message("fix the bug");

        let config = Config::default();
        let request = engine.build_request(&config);

        assert_eq!(request.messages.len(), 2); // system + user
        assert_eq!(request.messages[0].role, Role::System);
        assert_eq!(request.messages[1].role, Role::User);
        assert_eq!(request.temperature, 0.3_f64);
    }

    #[test]
    fn test_compact() {
        let mut engine = ConversationEngine::new(
            "system".to_string(),
            vec![],
            100, // very small context
        );

        // Add many messages to exceed context
        for i in 0..20 {
            engine.add_user_message(&format!("message {i} with some padding text to increase token count significantly"));
        }

        let before = engine.message_count();
        engine.compact();
        let after = engine.message_count();

        assert!(after < before, "compact should reduce message count");
    }

    #[test]
    fn test_clear() {
        let mut engine = test_engine();
        engine.add_user_message("hello");
        engine.add_user_message("world");
        assert_eq!(engine.message_count(), 2);

        engine.clear();
        assert_eq!(engine.message_count(), 0);
        assert_eq!(engine.estimated_tokens(), 0);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 1); // min 1
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 ≈ 2
        assert_eq!(estimate_tokens("a".repeat(100).as_str()), 25);
    }
}
