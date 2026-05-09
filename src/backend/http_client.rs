use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;

use super::types::{
    ChatRequest, ChatResponse, Message, Role, StopReason, Token, TokenUsage, ToolCall,
    ToolDefinition,
};

/// Lift inline `<tool_call>` XML out of assistant content into structured `ToolCall`s.
///
/// When a backend returns `tool_calls = None` but the model emitted prompted-mode
/// XML directly into `content`, this fallback uses Forge's existing
/// `parse_qwen35_xml` to extract the calls and strips the XML blocks from the
/// visible content. If the structured `tool_calls` field was already populated,
/// returns it unchanged (no double-extraction).
///
/// SECURITY (CAT 7 — LLM Output Injection): delegates parsing to
/// `parse_qwen35_xml`, which strips markdown code fences and rejects duplicate
/// parameters before this function ever sees the inputs. Tool results that
/// echo `<tool_call>` text from upstream user input remain a pre-existing
/// risk handled by Forge's permissions layer; this fallback does not widen
/// that surface beyond what native parsers already do.
fn extract_inline_tool_calls(
    content: String,
    structured: Option<Vec<ToolCall>>,
) -> (String, Option<Vec<ToolCall>>) {
    if let Some(tcs) = structured {
        if !tcs.is_empty() {
            return (content, Some(tcs));
        }
    }

    let parsed = crate::conversation::adapter::parse_qwen35_xml(&content);
    if parsed.is_empty() {
        return (content, None);
    }

    let stripped = crate::conversation::adapter::strip_tool_call_blocks(&content);
    let lifted: Vec<ToolCall> = parsed
        .into_iter()
        .enumerate()
        .map(|(idx, p)| {
            let args = serde_json::Value::Object(p.arguments.into_iter().collect());
            ToolCall {
                id: format!("call_inline_{idx}"),
                name: p.name,
                arguments: args,
            }
        })
        .collect();

    (stripped, Some(lifted))
}

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
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Sanitize messages for strict Jinja chat templates (Qwen, Llama).
    /// Merges all `Role::System` messages into a single message at position 0,
    /// preserving native tool call structure (`tool_calls`, `role: "tool"`).
    /// The Qwen3.5 template enforces "system must be first and only" — this
    /// function guarantees that invariant.
    fn sanitize_messages(messages: &[Message]) -> Vec<OaiMessage> {
        let mut system_content = String::new();
        let mut non_system: Vec<OaiMessage> = Vec::new();

        for msg in messages {
            if msg.role == Role::System {
                if !system_content.is_empty() {
                    system_content.push_str("\n\n");
                }
                system_content.push_str(&msg.content);
            } else {
                non_system.push(Self::convert_message(msg));
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

        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&oai_req)
            .send()
            .await
            .with_context(|| format!("Failed to connect to model server at {url}"))?;

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

        let (content, tool_calls) = extract_inline_tool_calls(content, tool_calls);

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls") | Some("function_call") => StopReason::ToolCall,
            Some("length") => StopReason::MaxTokens,
            _ if tool_calls.as_ref().is_some_and(|tcs| !tcs.is_empty()) => StopReason::ToolCall,
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

        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&oai_req)
            .send()
            .await
            .with_context(|| format!("Failed to connect to model server at {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Model server returned {status}: {body}");
        }

        let (tx, rx) = mpsc::channel(256);

        let handle = tokio::spawn(async move {
            let mut full_content = String::new();
            let mut tool_calls: Vec<OaiToolCall> = Vec::new();
            let mut finish_reason = None;
            let mut bytes = Vec::new();
            let mut sent_final = false;

            let mut stream = resp.bytes_stream();
            use futures_util::StreamExt;

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        let _ = tx.send(Token { text: String::new(), is_final: true }).await;
                        return Err(e).context("Stream read error");
                    }
                };
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
                            sent_final = true;
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

            if !sent_final {
                let _ = tx.send(Token { text: String::new(), is_final: true }).await;
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

            let (full_content, parsed_tool_calls) =
                extract_inline_tool_calls(full_content, parsed_tool_calls);

            let stop_reason = match finish_reason.as_deref() {
                Some("tool_calls") | Some("function_call") => StopReason::ToolCall,
                Some("length") => StopReason::MaxTokens,
                _ if parsed_tool_calls.as_ref().is_some_and(|tcs| !tcs.is_empty()) => StopReason::ToolCall,
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
    fn test_sanitize_preserves_native_tool_calling() {
        // Simulate Michelle's exact scenario: user -> assistant (tool_call) -> tool result
        // Native tool calling must be PRESERVED so Qwen3.5's Jinja template can render it.
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

        assert_eq!(result.len(), 4);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].role, "assistant");
        assert_eq!(result[3].role, "tool"); // native role preserved

        // Only the first message is system
        for msg in &result[1..] {
            assert_ne!(msg.role, "system", "Only first message should be system");
        }

        // Assistant tool_calls are preserved (not flattened)
        assert!(result[2].tool_calls.is_some(), "Native tool_calls must be preserved");
        let tcs = result[2].tool_calls.as_ref().unwrap();
        assert_eq!(tcs[0].function.name, "bash");

        // Tool result preserves tool_call_id for matching
        assert_eq!(result[3].tool_call_id.as_deref(), Some("call_1"));
        assert!(result[3].content.as_ref().unwrap().contains("On branch main"));
    }

    // ── extract_inline_tool_calls tests (companion to A1 MLX parser disable) ─

    #[test]
    fn test_inline_extract_well_formed_qwen35() {
        // The MLX-after-A1 case: assistant content has prompted XML, structured is None.
        let content = "I'll check the files.\n\n<tool_call>\n<function=glob>\n<parameter=pattern>**/*.md</parameter>\n</function>\n</tool_call>".to_string();
        let (stripped, lifted) = extract_inline_tool_calls(content, None);

        let tcs = lifted.expect("inline parser must lift tool calls when content has them");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "glob");
        assert_eq!(tcs[0].arguments["pattern"], "**/*.md");
        assert!(!stripped.contains("<tool_call>"), "<tool_call> XML must be stripped from content");
        assert!(stripped.contains("I'll check the files."), "natural-language prefix must be preserved");
    }

    #[test]
    fn test_inline_extract_skipped_when_structured_present() {
        // Native path (llama.cpp + Jinja, API backends): if the server already
        // returned structured tool_calls, fallback must not double-extract.
        let content = "<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</function>\n</tool_call>".to_string();
        let structured = vec![ToolCall {
            id: "call_native_1".into(),
            name: "structured_tool".into(),
            arguments: serde_json::json!({}),
        }];
        let (out_content, lifted) = extract_inline_tool_calls(content.clone(), Some(structured));

        let tcs = lifted.expect("structured tool calls must pass through");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "structured_tool");
        // Content unchanged when structured calls were present
        assert_eq!(out_content, content);
    }

    #[test]
    fn test_inline_extract_returns_none_when_empty() {
        let content = "Just a plain assistant message with no tool calls.".to_string();
        let (out, lifted) = extract_inline_tool_calls(content.clone(), None);
        assert!(lifted.is_none());
        assert_eq!(out, content);
    }

    /// SECURITY — CAT 7 (LLM Output Injection)
    /// A model could output `<tool_call>` inside a markdown code block
    /// pretending to be an example. The fallback must NOT extract these.
    /// Defense lives in `strip_code_fences` inside `parse_qwen35_xml`.
    #[test]
    fn test_inline_extract_security_code_fence_escape() {
        let content = "Here's an example of how to call a tool:\n\n```xml\n<tool_call>\n<function=bash>\n<parameter=command>rm -rf /</parameter>\n</function>\n</tool_call>\n```\n\nDoes that help?".to_string();
        let (_out, lifted) = extract_inline_tool_calls(content, None);
        assert!(
            lifted.is_none(),
            "tool calls inside ```code fences``` MUST NOT be lifted (CAT 7 — LLM Output Injection)"
        );
    }

    /// SECURITY — CAT 7
    /// Malformed `<tool_call>` block with no inner `<function=...>` MUST NOT
    /// crash and MUST NOT lift any calls. This is the exact pattern that
    /// causes MLX's qwen3_coder.py:110 to raise `ValueError("No function provided.")`,
    /// which is what A1 sidesteps. Forge's fallback must be tolerant where
    /// MLX is brittle.
    #[test]
    fn test_inline_extract_security_malformed_block_no_panic() {
        let content = "<tool_call>\nthis is just garbage text\n</tool_call>".to_string();
        let (out, lifted) = extract_inline_tool_calls(content.clone(), None);
        assert!(
            lifted.is_none(),
            "malformed tool_call block (no <function=>) MUST return None, not panic"
        );
        // Content preserved when no extraction succeeded
        assert_eq!(out, content);
    }

    #[test]
    fn test_inline_extract_multiple_calls() {
        let content = "Running two checks.\n\n<tool_call>\n<function=glob>\n<parameter=pattern>*.md</parameter>\n</function>\n</tool_call>\n\n<tool_call>\n<function=glob>\n<parameter=pattern>*.toml</parameter>\n</function>\n</tool_call>".to_string();
        let (stripped, lifted) = extract_inline_tool_calls(content, None);

        let tcs = lifted.expect("multiple inline calls must all be lifted");
        assert_eq!(tcs.len(), 2);
        assert_eq!(tcs[0].name, "glob");
        assert_eq!(tcs[1].name, "glob");
        assert_eq!(tcs[0].arguments["pattern"], "*.md");
        assert_eq!(tcs[1].arguments["pattern"], "*.toml");
        assert!(!stripped.contains("<tool_call>"));
        assert!(stripped.contains("Running two checks."));
    }

    /// Empty `Some(vec![])` from upstream is treated the same as `None` —
    /// we should still try to extract from content. This matches the
    /// real shape of the OAI response when MLX returns
    /// `tool_calls: []` (which it does after A1).
    #[test]
    fn test_inline_extract_empty_structured_falls_through() {
        let content = "<tool_call>\n<function=bash>\n<parameter=command>ls</parameter>\n</function>\n</tool_call>".to_string();
        let (_out, lifted) = extract_inline_tool_calls(content, Some(vec![]));
        let tcs = lifted.expect("Some(empty) must fall through to inline extraction");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "bash");
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
