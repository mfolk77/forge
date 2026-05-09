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
        self.estimated_tokens += estimate_tool_calls_tokens(&response.message.tool_calls);
        self.messages.push(response.message);
    }

    /// Add a tool result. The `result` is sanitized to neutralize Forge's
    /// XML tool-call markers so a fetched page or command output that
    /// contains `<tool_call>`, `</tool_response>`, etc. cannot be
    /// re-interpreted by the agentic loop as a model-emitted tool call.
    /// (CAT 7 — LLM Output Injection. AUDIT P0 #5.)
    pub fn add_tool_result(&mut self, tool_call_id: &str, result: &str) {
        let safe = crate::conversation::adapter::sanitize_tool_result_for_message(result);
        let token_estimate = estimate_tokens(&safe);
        self.messages.push(Message {
            role: Role::Tool,
            content: safe,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
        });
        self.estimated_tokens += token_estimate;
    }

    /// Build a ChatRequest for the model
    pub fn build_request(&self, config: &Config) -> ChatRequest {
        self.build_request_with_mode(config, false)
    }

    /// Build request optimized for mode. In chat mode, tools are omitted
    /// so the model doesn't waste prefill time on tool schemas.
    ///
    /// Merges all `Role::System` messages into a single system message at
    /// position 0. Many local models (Qwen, Llama) enforce "system must be
    /// first and only" via their Jinja chat templates.
    pub fn build_request_with_mode(&self, config: &Config, chat_mode: bool) -> ChatRequest {
        // Start with the main system prompt
        let mut system_content = self.system_prompt.clone();
        let mut messages = Vec::new();

        // Merge any mid-conversation system messages into the system prompt;
        // keep all other messages in order.
        for msg in &self.messages {
            if msg.role == Role::System {
                system_content.push_str("\n\n");
                system_content.push_str(&msg.content);
            } else {
                messages.push(msg.clone());
            }
        }

        // Insert the unified system message at position 0
        messages.insert(0, Message {
            role: Role::System,
            content: system_content,
            tool_calls: None,
            tool_call_id: None,
        });

        let use_native_tools = !chat_mode
            && !(config.model.backend == crate::config::BackendType::Mlx
                && config.model.tool_calling != crate::config::ToolCallingMode::Native);

        ChatRequest {
            messages,
            tools: if use_native_tools { self.tools.clone() } else { vec![] },
            temperature: config.model.temperature,
            max_tokens: Some(4096),
            model_id: config.model.path.as_deref()
                .map(|p| crate::backend::manager::BackendManager::resolve_path(p)),
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

    /// Forcibly shrink the conversation before retrying after a backend
    /// transport failure. This is independent of the tier threshold logic in
    /// `compact()` — when we've just hit a server disconnect, we want to send
    /// a smaller request next time regardless of where token counts sit.
    ///
    /// Order:
    /// 1. Tier 1 (microcompact) — drops old tool result bodies cheaply.
    /// 2. Tier 2 (snip) — removes oldest messages keeping the most recent 6.
    ///    A keep-window of 6 (vs. compact()'s 10) is intentionally tighter:
    ///    a recovery is a degraded state and aggressive shrinkage is the right
    ///    call.
    ///
    /// SECURITY (CAT 9 — Memory & Persistence): preserves at least the most
    /// recent user message so the request shape stays valid (server-side
    /// validators on some backends require a trailing user message).
    pub fn shrink_for_retry(&mut self) {
        self.micro_compact();

        const KEEP_RECENT_ON_RETRY: usize = 6;
        if self.messages.len() <= KEEP_RECENT_ON_RETRY {
            return;
        }

        if let Some(ref path) = self.project_path.clone() {
            let _ = compactor::snip_compact(&mut self.messages, path, KEEP_RECENT_ON_RETRY);
        } else {
            let keep_recent = KEEP_RECENT_ON_RETRY.min(self.messages.len());
            let summary = Self::summarize_messages(
                &self.messages[..self.messages.len() - keep_recent],
            );
            let mut new_messages = vec![Message {
                role: Role::System,
                content: format!("[Conversation summary (retry shrink): {summary}]"),
                tool_calls: None,
                tool_call_id: None,
            }];
            new_messages.extend_from_slice(
                &self.messages[self.messages.len() - keep_recent..],
            );
            self.messages = new_messages;
        }

        self.recalculate_tokens();
    }

    /// Check if compaction is needed based on token thresholds.
    /// Returns the tier (2 or 3) or None.
    pub fn should_compact(&self) -> Option<u8> {
        compactor::should_compact(self.estimated_request_tokens(false), self.usable_context_tokens())
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
                    // Reinject full system prompt (identity, rules, FTAI.md, skills, tools)
                    self.messages.insert(0, Message {
                        role: Role::System,
                        content: self.system_prompt.clone(),
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
                        let preview: String = msg.content.chars().take(100).collect();
                        summary_parts.push(format!("User asked: {preview}..."));
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
            .map(|m| estimate_tokens(&m.content) + estimate_tool_calls_tokens(&m.tool_calls))
            .sum();
    }

    /// Clear all messages
    pub fn clear(&mut self) {
        self.messages.clear();
        self.estimated_tokens = 0;
    }

    #[allow(dead_code)]
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn estimated_tokens(&self) -> usize {
        self.estimated_tokens
    }

    /// Estimate the full prompt sent to the model, including system prompt and
    /// tool schemas. `estimated_tokens` tracks only conversation messages.
    pub fn estimated_request_tokens(&self, chat_mode: bool) -> usize {
        let mut total = self.estimated_tokens + estimate_tokens(&self.system_prompt);
        if !chat_mode {
            total += self.tools.iter().map(|tool| {
                estimate_tokens(&tool.name)
                    + estimate_tokens(&tool.description)
                    + serde_json::to_string(&tool.parameters)
                        .map(|s| estimate_tokens(&s))
                        .unwrap_or(0)
            }).sum::<usize>();
        }
        total
    }

    /// Leave room for the response; prompt-only fit is not enough for local
    /// servers with fixed context windows.
    fn usable_context_tokens(&self) -> usize {
        self.max_context_tokens.saturating_sub(4096).max(self.max_context_tokens / 2)
    }

    #[allow(dead_code)]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    #[allow(dead_code)]
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn update_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    #[allow(dead_code)]
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

/// Estimate tokens for tool_calls payloads (serialized JSON size / 4).
fn estimate_tool_calls_tokens(tool_calls: &Option<Vec<crate::backend::types::ToolCall>>) -> usize {
    match tool_calls {
        Some(calls) if !calls.is_empty() => {
            serde_json::to_string(calls)
                .map(|s| s.len() / 4)
                .unwrap_or(0)
        }
        _ => 0,
    }
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
    fn test_mlx_hybrid_uses_prompted_tools() {
        let mut engine = ConversationEngine::new(
            "You are a coding assistant.".to_string(),
            vec![ToolDefinition {
                name: "bash".to_string(),
                description: "Run a command".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            32768,
        );
        engine.add_user_message("list files");

        let mut config = Config::default();
        config.model.backend = crate::config::BackendType::Mlx;
        config.model.tool_calling = crate::config::ToolCallingMode::Hybrid;

        let request = engine.build_request(&config);
        assert!(request.tools.is_empty());
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
    fn test_shrink_for_retry_reduces_message_count() {
        let mut engine = ConversationEngine::new(
            "system".to_string(),
            vec![],
            32768,
        );

        // Add many messages so shrink has work to do
        for i in 0..30 {
            engine.add_user_message(&format!("message {i}"));
            engine.add_assistant_message(ChatResponse {
                message: Message {
                    role: Role::Assistant,
                    content: format!("response {i}"),
                    tool_calls: None,
                    tool_call_id: None,
                },
                tokens_used: Default::default(),
                stop_reason: crate::backend::types::StopReason::EndOfText,
            });
        }
        let before = engine.message_count();
        assert!(before >= 60);

        engine.shrink_for_retry();

        let after = engine.message_count();
        assert!(
            after < before,
            "shrink_for_retry must reduce message count after a transport error (was {before}, now {after})"
        );
        // Keep window of 6 + 1 system summary = 7 expected
        assert!(
            after <= 8,
            "shrink_for_retry should keep no more than ~7 messages (got {after})"
        );
    }

    #[test]
    fn test_shrink_for_retry_preserves_recent_user_message() {
        // CRITICAL: some backends require a trailing user message in the
        // request shape. The shrink must NOT drop the most recent user
        // message or the retry will 404 with "No user query found in messages".
        let mut engine = ConversationEngine::new(
            "system".to_string(),
            vec![],
            32768,
        );

        for i in 0..10 {
            engine.add_user_message(&format!("user_{i}"));
            engine.add_assistant_message(ChatResponse {
                message: Message {
                    role: Role::Assistant,
                    content: format!("asst_{i}"),
                    tool_calls: None,
                    tool_call_id: None,
                },
                tokens_used: Default::default(),
                stop_reason: crate::backend::types::StopReason::EndOfText,
            });
        }
        engine.add_user_message("MOST_RECENT_USER_QUERY");

        engine.shrink_for_retry();

        let has_recent_user = engine.messages().iter().any(|m| {
            m.role == Role::User && m.content == "MOST_RECENT_USER_QUERY"
        });
        assert!(
            has_recent_user,
            "shrink_for_retry MUST preserve the most recent user message (request shape requirement)"
        );
    }

    #[test]
    fn test_shrink_for_retry_idempotent_on_short_conversation() {
        // If the conversation is already short, shrink should not corrupt it.
        let mut engine = ConversationEngine::new(
            "system".to_string(),
            vec![],
            32768,
        );
        engine.add_user_message("hi");
        let before = engine.message_count();

        engine.shrink_for_retry();

        assert_eq!(
            engine.message_count(),
            before,
            "short conversations must not be shrunk further"
        );
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
