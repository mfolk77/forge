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
}

#[derive(Serialize, Deserialize, Debug)]
struct OaiMessage {
    role: String,
    content: Option<String>,
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
            messages: request.messages.iter().map(Self::convert_message).collect(),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(&request.tools))
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: false,
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
                content: choice.message.content.unwrap_or_default(),
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
            messages: request.messages.iter().map(Self::convert_message).collect(),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(&request.tools))
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
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

                // Parse SSE lines
                let text = String::from_utf8_lossy(&bytes);
                let mut consumed = 0;

                for line in text.lines() {
                    consumed += line.len() + 1; // +1 for newline

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
}
