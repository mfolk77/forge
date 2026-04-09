use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::types::{
    ChatRequest, ChatResponse, Message, Role, StopReason, Token, TokenUsage, ToolCall,
    ToolDefinition,
};

/// OpenAI-compatible HTTP client for local model servers
pub struct HttpModelClient {
    client: reqwest::Client,
    base_url: String,
}

// OpenAI API request/response types

#[derive(Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OaiTool>>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    stream: bool,
    /// MLX-specific: control chat template behavior (e.g. disable thinking mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct OaiMessage {
    role: String,
    content: Option<String>,
    /// Qwen3/3.5 thinking mode: reasoning goes here when enable_thinking=true
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiToolDef,
}

#[derive(Serialize)]
struct OaiToolDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Deserialize, Debug)]
struct OaiChoice {
    message: OaiMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OaiUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

#[derive(Deserialize, Debug)]
struct OaiStreamChunk {
    choices: Vec<OaiStreamChoice>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamChoice {
    delta: OaiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OaiStreamToolCall>>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamToolCall {
    index: Option<usize>,
    id: Option<String>,
    function: Option<OaiStreamFunction>,
}

#[derive(Deserialize, Debug)]
struct OaiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

impl HttpModelClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Sanitize messages for strict Jinja chat templates (Qwen, Llama):
    /// 1. Merge all system messages into a single one at position 0
    /// 2. Convert `tool` role messages to `user` role (many small models lack native tool support)
    /// 3. Flatten assistant tool_calls into text content (same reason)
    /// This ensures tool calling works with ANY model, not just those with native tool support.
    fn sanitize_messages(messages: &[Message]) -> Vec<OaiMessage> {
        let mut system_content = String::new();
        let mut non_system: Vec<OaiMessage> = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    if !system_content.is_empty() {
                        system_content.push_str("\n\n");
                    }
                    system_content.push_str(&msg.content);
                }
                Role::Tool => {
                    // Convert tool results to user messages for template compatibility
                    let tool_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                    non_system.push(OaiMessage {
                        role: "user".to_string(),
                        content: Some(format!(
                            "[Tool Result (call_id: {tool_id})]\n{}",
                            msg.content
                        )),
                        reasoning: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                Role::Assistant => {
                    // Flatten tool_calls into text content for template compatibility
                    let mut content = msg.content.clone();
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            let call_text = format!(
                                "\n[Tool Call: {} (call_id: {})]\n{}",
                                tc.name, tc.id, tc.arguments
                            );
                            content.push_str(&call_text);
                        }
                    }
                    non_system.push(OaiMessage {
                        role: "assistant".to_string(),
                        content: Some(content),
                        reasoning: None,
                        tool_calls: None, // flattened into content
                        tool_call_id: None,
                    });
                }
                Role::User => {
                    non_system.push(OaiMessage {
                        role: "user".to_string(),
                        content: Some(msg.content.clone()),
                        reasoning: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        let mut result = Vec::with_capacity(non_system.len() + 1);
        if !system_content.is_empty() {
            result.push(OaiMessage {
                role: "system".to_string(),
                content: Some(system_content),
                reasoning: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        result.extend(non_system);
        result
    }

    fn convert_message(msg: &Message) -> OaiMessage {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        OaiMessage {
            role: role.to_string(),
            content: Some(msg.content.clone()),
            reasoning: None,
            tool_calls: msg.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| OaiToolCall {
                        id: tc.id.clone(),
                        call_type: "function".to_string(),
                        function: OaiFunction {
                            name: tc.name.clone(),
                            arguments: tc.arguments.to_string(),
                        },
                    })
                    .collect()
            }),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OaiTool> {
        tools
            .iter()
            .map(|t| OaiTool {
                tool_type: "function".to_string(),
                function: OaiToolDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    }

    pub async fn generate(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let oai_req = OaiRequest {
            model: request.model_id.clone().unwrap_or_default(),
            messages: Self::sanitize_messages(&request.messages),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(&request.tools))
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: false,
            chat_template_kwargs: Some(serde_json::json!({"enable_thinking": false})),
        };

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&oai_req)
            .send()
            .await
            .context("Failed to connect to model server")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Model server returned {status}: {body}");
        }

        let oai_resp: OaiResponse = resp.json().await.context("Failed to parse model response")?;

        let choice = oai_resp
            .choices
            .into_iter()
            .next()
            .context("No choices in response")?;

        // If content is empty but reasoning has text (thinking mode), use reasoning
        let content = match (&choice.message.content, &choice.message.reasoning) {
            (Some(c), _) if !c.is_empty() => c.clone(),
            (_, Some(r)) if !r.is_empty() => r.clone(),
            (Some(c), _) => c.clone(),
            _ => String::new(),
        };

        let tool_calls = choice.message.tool_calls.map(|tcs| {
            tcs.into_iter()
                .map(|tc| ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Null),
                })
                .collect()
        });

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls") | Some("function_call") => StopReason::ToolCall,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndOfText,
        };

        let usage = oai_resp.usage.map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
        }).unwrap_or_default();

        Ok(ChatResponse {
            message: Message {
                role: Role::Assistant,
                content,
                tool_calls,
                tool_call_id: None,
            },
            tokens_used: usage,
            stop_reason,
        })
    }

    pub async fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        let oai_req = OaiRequest {
            model: request.model_id.clone().unwrap_or_default(),
            messages: Self::sanitize_messages(&request.messages),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(&request.tools))
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
            chat_template_kwargs: Some(serde_json::json!({"enable_thinking": false})),
        };

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&oai_req)
            .send()
            .await
            .context("Failed to connect to model server")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Model server returned error: {body}");
        }

        let (tx, rx) = mpsc::channel(256);

        let handle = tokio::spawn(async move {
            let mut full_content = String::new();
            let mut tool_calls: Vec<OaiToolCall> = Vec::new();
            let mut finish_reason = None;
            let mut bytes = Vec::new();

            let mut stream = resp.bytes_stream();
            use futures_util::StreamExt;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("Stream read error")?;
                bytes.extend_from_slice(&chunk);

                // Parse SSE lines from raw bytes, tracking actual byte offsets
                let text = String::from_utf8_lossy(&bytes);
                let mut consumed = 0;

                for line in text.lines() {
                    // Advance past the line content + actual line ending (\n or \r\n)
                    consumed += line.len();
                    // Skip the line ending characters
                    if consumed < text.len() && text.as_bytes().get(consumed) == Some(&b'\r') {
                        consumed += 1;
                    }
                    if consumed < text.len() && text.as_bytes().get(consumed) == Some(&b'\n') {
                        consumed += 1;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            let _ = tx
                                .send(Token {
                                    text: String::new(),
                                    is_final: true,
                                })
                                .await;
                            break;
                        }

                        if let Ok(chunk) = serde_json::from_str::<OaiStreamChunk>(data) {
                            for choice in &chunk.choices {
                                if let Some(content) = &choice.delta.content {
                                    full_content.push_str(content);
                                    let _ = tx
                                        .send(Token {
                                            text: content.clone(),
                                            is_final: false,
                                        })
                                        .await;
                                }

                                if let Some(tcs) = &choice.delta.tool_calls {
                                    for tc in tcs {
                                        let idx = tc.index.unwrap_or(tool_calls.len());
                                        while tool_calls.len() <= idx {
                                            tool_calls.push(OaiToolCall {
                                                id: String::new(),
                                                call_type: "function".to_string(),
                                                function: OaiFunction {
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                },
                                            });
                                        }
                                        if let Some(id) = &tc.id {
                                            tool_calls[idx].id = id.clone();
                                        }
                                        if let Some(f) = &tc.function {
                                            if let Some(name) = &f.name {
                                                tool_calls[idx].function.name = name.clone();
                                            }
                                            if let Some(args) = &f.arguments {
                                                tool_calls[idx]
                                                    .function
                                                    .arguments
                                                    .push_str(args);
                                            }
                                        }
                                    }
                                }

                                if choice.finish_reason.is_some() {
                                    finish_reason = choice.finish_reason.clone();
                                }
                            }
                        }
                    }
                }

                bytes = bytes[consumed.min(bytes.len())..].to_vec();
            }

            let parsed_tool_calls: Option<Vec<ToolCall>> = if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .into_iter()
                        .map(|tc| ToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            arguments: serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::Value::Null),
                        })
                        .collect(),
                )
            };

            let stop_reason = match finish_reason.as_deref() {
                Some("tool_calls") | Some("function_call") => StopReason::ToolCall,
                Some("length") => StopReason::MaxTokens,
                _ => StopReason::EndOfText,
            };

            Ok(ChatResponse {
                message: Message {
                    role: Role::Assistant,
                    content: full_content,
                    tool_calls: parsed_tool_calls,
                    tool_call_id: None,
                },
                tokens_used: TokenUsage::default(),
                stop_reason,
            })
        });

        Ok((rx, handle))
    }

    /// Check if the server is reachable
    pub async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/v1/models", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_message() {
        let msg = Message {
            role: Role::User,
            content: "hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };
        let oai = HttpModelClient::convert_message(&msg);
        assert_eq!(oai.role, "user");
        assert_eq!(oai.content, Some("hello".to_string()));
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![ToolDefinition {
            name: "file_read".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let oai = HttpModelClient::convert_tools(&tools);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].function.name, "file_read");
    }

    #[test]
    fn test_convert_message_with_tool_calls() {
        let msg = Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "tc_1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            }]),
            tool_call_id: None,
        };
        let oai = HttpModelClient::convert_message(&msg);
        assert!(oai.tool_calls.is_some());
        let tcs = oai.tool_calls.unwrap();
        assert_eq!(tcs[0].function.name, "bash");
    }

    // ── sanitize_messages tests (Jinja template compatibility) ──────────

    #[test]
    fn test_sanitize_only_system_user_assistant() {
        let messages = vec![
            Message { role: Role::System, content: "You are an AI.".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::User, content: "Hello".into(), tool_calls: None, tool_call_id: None },
        ];
        let result = HttpModelClient::sanitize_messages(&messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
    }

    #[test]
    fn test_sanitize_merges_multiple_system_messages() {
        let messages = vec![
            Message { role: Role::System, content: "Identity.".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::User, content: "Hi".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::System, content: "Extra context.".into(), tool_calls: None, tool_call_id: None },
        ];
        let result = HttpModelClient::sanitize_messages(&messages);
        // Should be 2: one merged system + user. No system at position 2.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert!(result[0].content.as_ref().unwrap().contains("Identity."));
        assert!(result[0].content.as_ref().unwrap().contains("Extra context."));
        assert_eq!(result[1].role, "user");
    }

    #[test]
    fn test_sanitize_no_tool_role_in_output() {
        // Simulate Michelle's exact scenario: user -> assistant (tool_call) -> tool result
        let messages = vec![
            Message {
                role: Role::System,
                content: "You are Forge.".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: "Can you check the git?".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({"command": "git status"}),
                }]),
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: "On branch main\nYour branch is up to date.".into(),
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
            },
        ];

        let result = HttpModelClient::sanitize_messages(&messages);

        // Verify structure: system, user, assistant, user (converted from tool)
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
        assert_eq!(result[3].role, "user"); // NOT "tool"

        // No message after the first should have role "tool" or "system"
        assert_eq!(result[0].role, "system");
        for msg in &result[1..] {
            assert_ne!(msg.role, "tool", "No 'tool' role messages should reach the model");
            assert_ne!(msg.role, "system", "Only first message should be system");
        }

        // Assistant message should have tool call flattened into content
        let assistant_content = result[2].content.as_ref().unwrap();
        assert!(assistant_content.contains("[Tool Call: bash"));
        assert!(assistant_content.contains("git status"));
        // tool_calls field should be None (flattened)
        assert!(result[2].tool_calls.is_none());

        // Tool result should be wrapped as user message
        let tool_result_content = result[3].content.as_ref().unwrap();
        assert!(tool_result_content.contains("[Tool Result"));
        assert!(tool_result_content.contains("On branch main"));
    }

    #[test]
    fn test_sanitize_no_system_except_first() {
        // Even with compaction injecting system messages mid-conversation
        let messages = vec![
            Message { role: Role::System, content: "Main prompt.".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::User, content: "Do something".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::Assistant, content: "OK".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::System, content: "Compaction reinjection.".into(), tool_calls: None, tool_call_id: None },
            Message { role: Role::User, content: "Continue".into(), tool_calls: None, tool_call_id: None },
        ];

        let result = HttpModelClient::sanitize_messages(&messages);

        // Only the first message should be system
        assert_eq!(result[0].role, "system");
        for msg in &result[1..] {
            assert_ne!(msg.role, "system");
        }
        // System content should be merged
        assert!(result[0].content.as_ref().unwrap().contains("Main prompt."));
        assert!(result[0].content.as_ref().unwrap().contains("Compaction reinjection."));
    }
}
