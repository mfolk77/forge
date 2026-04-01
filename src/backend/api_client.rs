use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::types::{
    ChatRequest, ChatResponse, Message, Role, StopReason, Token, TokenUsage, ToolCall,
    ToolDefinition,
};
use crate::config::ApiConfig;

/// Supported cloud API providers
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiProvider {
    Anthropic,
    OpenAI,
    Custom,
}

impl ApiProvider {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => ApiProvider::Anthropic,
            "openai" | "gpt" => ApiProvider::OpenAI,
            _ => ApiProvider::Custom,
        }
    }

    pub fn default_base_url(&self) -> &str {
        match self {
            ApiProvider::Anthropic => "https://api.anthropic.com",
            ApiProvider::OpenAI => "https://api.openai.com",
            ApiProvider::Custom => "http://localhost:8080",
        }
    }
}

/// Multi-provider API client for cloud LLMs
pub struct ApiClient {
    provider: ApiProvider,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: usize,
    client: reqwest::Client,
}

// Manual Debug impl to prevent API key leakage in logs/debug output
impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// Resolve the API key from config (direct value or env var).
/// Returns None if neither is set or both are empty.
pub fn resolve_api_key(config: &ApiConfig) -> Option<String> {
    // First check direct key in config
    if let Some(key) = &config.api_key {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }
    // Then check env var
    if let Some(env_var) = &config.api_key_env {
        if let Ok(key) = std::env::var(env_var) {
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    None
}

/// Mask an API key for display: show first 4 + "..." + last 4 chars.
/// If the key is too short, mask entirely.
pub fn mask_api_key(key: &str) -> String {
    if key.len() <= 12 {
        "****".to_string()
    } else {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    }
}

impl ApiClient {
    pub fn new(provider: ApiProvider, base_url: &str, api_key: &str, model: &str, max_tokens: usize) -> Self {
        Self {
            provider,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_config(config: &ApiConfig) -> Result<Self> {
        let api_key = resolve_api_key(config)
            .context("API key not found. Set api_key_env in config or provide api_key directly.")?;

        let provider = ApiProvider::from_str_loose(&config.provider);
        let default_url = provider.default_base_url().to_string();
        let base_url = config
            .base_url
            .as_deref()
            .unwrap_or(&default_url);

        Ok(Self::new(provider, base_url, &api_key, &config.model, config.max_tokens))
    }

    pub fn provider(&self) -> &ApiProvider {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    pub fn set_provider(&mut self, provider: ApiProvider) {
        self.base_url = provider.default_base_url().to_string();
        self.provider = provider;
    }

    /// Return masked key for safe display
    pub fn masked_key(&self) -> String {
        mask_api_key(&self.api_key)
    }

    // ── Anthropic protocol ──────────────────────────────────────────────────

    pub async fn generate(&self, request: &ChatRequest) -> Result<ChatResponse> {
        match self.provider {
            ApiProvider::Anthropic => self.generate_anthropic(request).await,
            ApiProvider::OpenAI | ApiProvider::Custom => self.generate_openai(request).await,
        }
    }

    pub async fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        match self.provider {
            ApiProvider::Anthropic => self.stream_anthropic(request).await,
            ApiProvider::OpenAI | ApiProvider::Custom => self.stream_openai(request).await,
        }
    }

    pub async fn health_check(&self) -> bool {
        // Lightweight check — just verify we can reach the API
        match self.provider {
            ApiProvider::Anthropic => {
                // Anthropic doesn't have a /models endpoint; just return true.
                // The first real request will surface auth errors clearly.
                true
            }
            ApiProvider::OpenAI | ApiProvider::Custom => {
                self.client
                    .get(format!("{}/v1/models", self.base_url))
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false)
            }
        }
    }

    // ── Anthropic non-streaming ─────────────────────────────────────────────

    async fn generate_anthropic(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let (system, messages) = self.convert_to_anthropic(request);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens.unwrap_or(self.max_tokens),
            "messages": messages,
        });

        if let Some(sys) = &system {
            body["system"] = serde_json::Value::String(sys.clone());
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::to_value(
                request.tools.iter().map(|t| anthropic_tool_def(t)).collect::<Vec<_>>()
            )?;
        }

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to connect to Anthropic API")?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API returned {status}: {err_body}");
        }

        let raw: AnthropicResponse = resp.json().await.context("Failed to parse Anthropic response")?;
        Ok(self.anthropic_response_to_chat(raw))
    }

    // ── Anthropic streaming ─────────────────────────────────────────────────

    async fn stream_anthropic(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        let (system, messages) = self.convert_to_anthropic(request);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens.unwrap_or(self.max_tokens),
            "messages": messages,
            "stream": true,
        });

        if let Some(sys) = &system {
            body["system"] = serde_json::Value::String(sys.clone());
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::to_value(
                request.tools.iter().map(|t| anthropic_tool_def(t)).collect::<Vec<_>>()
            )?;
        }

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to connect to Anthropic API for streaming")?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API streaming error: {err_body}");
        }

        let (tx, rx) = mpsc::channel(256);

        let handle = tokio::spawn(async move {
            let mut full_content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut current_tool_id = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_json = String::new();
            let mut stop_reason = StopReason::EndOfText;
            let mut bytes = Vec::new();
            let mut input_tokens = 0usize;
            let mut output_tokens = 0usize;

            let mut stream = resp.bytes_stream();
            use futures_util::StreamExt;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("Anthropic stream read error")?;
                bytes.extend_from_slice(&chunk);

                let text = String::from_utf8_lossy(&bytes);
                let mut consumed = 0;

                for line in text.lines() {
                    consumed += line.len() + 1;

                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                            let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

                            match event_type {
                                "message_start" => {
                                    if let Some(usage) = event.pointer("/message/usage") {
                                        input_tokens = usage.get("input_tokens")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0) as usize;
                                    }
                                }
                                "content_block_start" => {
                                    if let Some(cb) = event.get("content_block") {
                                        let cb_type = cb.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                        if cb_type == "tool_use" {
                                            current_tool_id = cb.get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            current_tool_name = cb.get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            current_tool_json.clear();
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    if let Some(delta) = event.get("delta") {
                                        let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                        match delta_type {
                                            "text_delta" => {
                                                if let Some(text_val) = delta.get("text").and_then(|t| t.as_str()) {
                                                    full_content.push_str(text_val);
                                                    let _ = tx.send(Token {
                                                        text: text_val.to_string(),
                                                        is_final: false,
                                                    }).await;
                                                }
                                            }
                                            "input_json_delta" => {
                                                if let Some(json_str) = delta.get("partial_json").and_then(|t| t.as_str()) {
                                                    current_tool_json.push_str(json_str);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "content_block_stop" => {
                                    if !current_tool_name.is_empty() {
                                        let args = serde_json::from_str(&current_tool_json)
                                            .unwrap_or(serde_json::Value::Null);
                                        tool_calls.push(ToolCall {
                                            id: current_tool_id.clone(),
                                            name: current_tool_name.clone(),
                                            arguments: args,
                                        });
                                        current_tool_id.clear();
                                        current_tool_name.clear();
                                        current_tool_json.clear();
                                    }
                                }
                                "message_delta" => {
                                    if let Some(delta) = event.get("delta") {
                                        if let Some(sr) = delta.get("stop_reason").and_then(|v| v.as_str()) {
                                            stop_reason = match sr {
                                                "end_turn" => StopReason::EndOfText,
                                                "tool_use" => StopReason::ToolCall,
                                                "max_tokens" => StopReason::MaxTokens,
                                                _ => StopReason::EndOfText,
                                            };
                                        }
                                    }
                                    if let Some(usage) = event.get("usage") {
                                        output_tokens = usage.get("output_tokens")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0) as usize;
                                    }
                                }
                                "message_stop" => {
                                    let _ = tx.send(Token {
                                        text: String::new(),
                                        is_final: true,
                                    }).await;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                bytes = bytes[consumed.min(bytes.len())..].to_vec();
            }

            Ok(ChatResponse {
                message: Message {
                    role: Role::Assistant,
                    content: full_content,
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                    tool_call_id: None,
                },
                tokens_used: TokenUsage {
                    prompt_tokens: input_tokens,
                    completion_tokens: output_tokens,
                },
                stop_reason,
            })
        });

        Ok((rx, handle))
    }

    // ── OpenAI-compatible (with auth) ───────────────────────────────────────

    async fn generate_openai(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let oai_req = self.build_openai_request(request, false);

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&oai_req)
            .send()
            .await
            .context("Failed to connect to OpenAI-compatible API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API returned {status}: {body}");
        }

        let oai_resp: OaiResponse = resp.json().await.context("Failed to parse OpenAI response")?;
        let choice = oai_resp.choices.into_iter().next().context("No choices in response")?;

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

        let usage = oai_resp
            .usage
            .map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            })
            .unwrap_or_default();

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

    async fn stream_openai(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        let oai_req = self.build_openai_request(request, true);

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&oai_req)
            .send()
            .await
            .context("Failed to connect to OpenAI-compatible API for streaming")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API streaming error: {body}");
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

                let text = String::from_utf8_lossy(&bytes);
                let mut consumed = 0;

                for line in text.lines() {
                    consumed += line.len() + 1;

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            let _ = tx.send(Token { text: String::new(), is_final: true }).await;
                            break;
                        }

                        if let Ok(chunk) = serde_json::from_str::<OaiStreamChunk>(data) {
                            for choice in &chunk.choices {
                                if let Some(content) = &choice.delta.content {
                                    full_content.push_str(content);
                                    let _ = tx.send(Token {
                                        text: content.clone(),
                                        is_final: false,
                                    }).await;
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
                                                tool_calls[idx].function.arguments.push_str(args);
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

    // ── Conversion helpers ──────────────────────────────────────────────────

    /// Extract system prompt and convert messages to Anthropic format
    fn convert_to_anthropic(&self, request: &ChatRequest) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system = None;
        let mut messages = Vec::new();

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    // Anthropic: system prompt goes in the top-level `system` field
                    system = Some(msg.content.clone());
                }
                Role::User => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content.clone(),
                    }));
                }
                Role::Assistant => {
                    let mut content_blocks = Vec::new();
                    if !msg.content.is_empty() {
                        content_blocks.push(serde_json::json!({
                            "type": "text",
                            "text": msg.content.clone(),
                        }));
                    }
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments,
                            }));
                        }
                    }
                    if content_blocks.is_empty() {
                        content_blocks.push(serde_json::json!({
                            "type": "text",
                            "text": "",
                        }));
                    }
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                }
                Role::Tool => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                            "content": msg.content.clone(),
                        }],
                    }));
                }
            }
        }

        (system, messages)
    }

    fn anthropic_response_to_chat(&self, resp: AnthropicResponse) -> ChatResponse {
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for block in &resp.content {
            match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text_content.push_str(t);
                    }
                }
                "tool_use" => {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);
                    tool_calls.push(ToolCall { id, name, arguments: input });
                }
                _ => {}
            }
        }

        let stop_reason = match resp.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndOfText,
            Some("tool_use") => StopReason::ToolCall,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndOfText,
        };

        let usage = TokenUsage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
        };

        ChatResponse {
            message: Message {
                role: Role::Assistant,
                content: text_content,
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                tool_call_id: None,
            },
            tokens_used: usage,
            stop_reason,
        }
    }

    fn build_openai_request(&self, request: &ChatRequest, stream: bool) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let mut m = serde_json::json!({
                    "role": role,
                    "content": msg.content.clone(),
                });
                if let Some(tcs) = &msg.tool_calls {
                    m["tool_calls"] = serde_json::to_value(
                        tcs.iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments.to_string(),
                                    }
                                })
                            })
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default();
                }
                if let Some(tcid) = &msg.tool_call_id {
                    m["tool_call_id"] = serde_json::Value::String(tcid.clone());
                }
                m
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "temperature": request.temperature,
            "stream": stream,
        });

        if let Some(max) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max);
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::to_value(
                request
                    .tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
        }

        body
    }
}

// ── Anthropic response types ────────────────────────────────────────────────

fn anthropic_tool_def(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

#[derive(Deserialize, Debug)]
struct AnthropicResponse {
    content: Vec<serde_json::Value>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Deserialize, Debug)]
struct AnthropicUsage {
    input_tokens: usize,
    output_tokens: usize,
}

// ── OpenAI types (duplicated from http_client to avoid coupling) ────────────

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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_provider_from_str() {
        assert_eq!(ApiProvider::from_str_loose("anthropic"), ApiProvider::Anthropic);
        assert_eq!(ApiProvider::from_str_loose("claude"), ApiProvider::Anthropic);
        assert_eq!(ApiProvider::from_str_loose("openai"), ApiProvider::OpenAI);
        assert_eq!(ApiProvider::from_str_loose("gpt"), ApiProvider::OpenAI);
        assert_eq!(ApiProvider::from_str_loose("groq"), ApiProvider::Custom);
        assert_eq!(ApiProvider::from_str_loose("together"), ApiProvider::Custom);
    }

    #[test]
    fn test_from_config_valid() {
        // Set env var for test
        std::env::set_var("TEST_API_KEY_VALID", "sk-test-1234567890abcdef");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("TEST_API_KEY_VALID".into()),
            model: "claude-sonnet-4-20250514".into(),
            base_url: None,
            max_tokens: 4096,
        };
        let client = ApiClient::from_config(&config);
        assert!(client.is_ok());
        let client = client.unwrap();
        assert_eq!(*client.provider(), ApiProvider::Anthropic);
        assert_eq!(client.model(), "claude-sonnet-4-20250514");
        std::env::remove_var("TEST_API_KEY_VALID");
    }

    #[test]
    fn test_from_config_missing_key_returns_error() {
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("NONEXISTENT_KEY_VAR_12345".into()),
            model: "claude-sonnet-4-20250514".into(),
            base_url: None,
            max_tokens: 4096,
        };
        let result = ApiClient::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key not found"));
    }

    #[test]
    fn test_resolve_api_key_prefers_direct() {
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: Some("direct-key".into()),
            api_key_env: Some("NONEXISTENT_KEY_VAR_12345".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        assert_eq!(resolve_api_key(&config), Some("direct-key".to_string()));
    }

    #[test]
    fn test_resolve_api_key_falls_back_to_env() {
        std::env::set_var("TEST_FALLBACK_KEY", "env-key-value");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("TEST_FALLBACK_KEY".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        assert_eq!(resolve_api_key(&config), Some("env-key-value".to_string()));
        std::env::remove_var("TEST_FALLBACK_KEY");
    }

    #[test]
    fn test_resolve_api_key_returns_none_when_nothing_set() {
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: None,
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        assert_eq!(resolve_api_key(&config), None);
    }

    #[test]
    fn test_resolve_api_key_skips_empty_direct() {
        std::env::set_var("TEST_EMPTY_DIRECT_KEY", "from-env");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: Some("".into()),
            api_key_env: Some("TEST_EMPTY_DIRECT_KEY".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        assert_eq!(resolve_api_key(&config), Some("from-env".to_string()));
        std::env::remove_var("TEST_EMPTY_DIRECT_KEY");
    }

    #[test]
    fn test_mask_api_key_normal() {
        assert_eq!(mask_api_key("sk-1234567890abcdef"), "sk-1...cdef");
    }

    #[test]
    fn test_mask_api_key_short() {
        assert_eq!(mask_api_key("short"), "****");
        assert_eq!(mask_api_key(""), "****");
    }

    #[test]
    fn test_anthropic_message_format_system_extraction() {
        std::env::set_var("TEST_ANTHRO_FMT_KEY", "test-key");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("TEST_ANTHRO_FMT_KEY".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        let client = ApiClient::from_config(&config).unwrap();

        let request = ChatRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a helpful assistant.".into(),
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: "Hello".into(),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            temperature: 0.3,
            max_tokens: None,
            model_id: None,
        };

        let (system, messages) = client.convert_to_anthropic(&request);
        assert_eq!(system, Some("You are a helpful assistant.".to_string()));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");

        std::env::remove_var("TEST_ANTHRO_FMT_KEY");
    }

    #[test]
    fn test_anthropic_tool_format() {
        let tool = ToolDefinition {
            name: "file_read".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let def = anthropic_tool_def(&tool);
        assert_eq!(def["name"], "file_read");
        assert_eq!(def["description"], "Read a file");
        assert!(def["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_anthropic_tool_result_conversion() {
        std::env::set_var("TEST_TOOL_RESULT_KEY", "test-key");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("TEST_TOOL_RESULT_KEY".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        let client = ApiClient::from_config(&config).unwrap();

        let request = ChatRequest {
            messages: vec![
                Message {
                    role: Role::Tool,
                    content: "file contents here".into(),
                    tool_calls: None,
                    tool_call_id: Some("toolu_123".into()),
                },
            ],
            tools: vec![],
            temperature: 0.3,
            max_tokens: None,
            model_id: None,
        };

        let (_, messages) = client.convert_to_anthropic(&request);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let content = &messages[0]["content"][0];
        assert_eq!(content["type"], "tool_result");
        assert_eq!(content["tool_use_id"], "toolu_123");

        std::env::remove_var("TEST_TOOL_RESULT_KEY");
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_api_key_not_in_debug_output() {
        let client = ApiClient::new(
            ApiProvider::Anthropic,
            "https://api.anthropic.com",
            "sk-secret-key-12345678",
            "claude-sonnet-4-20250514",
            8192,
        );
        // The masked key must not expose the full key
        let masked = client.masked_key();
        assert!(!masked.contains("sk-secret-key-12345678"));
        assert!(masked.contains("..."));

        // Debug format must not expose the key
        let debug_str = format!("{:?}", client);
        assert!(!debug_str.contains("sk-secret-key-12345678"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_security_api_key_not_in_mask_output() {
        let key = "sk-ant-very-secret-key-0123456789";
        let masked = mask_api_key(key);
        assert_ne!(masked, key);
        assert!(!masked.contains("very-secret"));
        // First 4 and last 4 only
        assert!(masked.starts_with("sk-a"));
        assert!(masked.ends_with("6789"));
    }

    #[test]
    fn test_security_empty_api_key_handled() {
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: Some("".into()),
            api_key_env: None,
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        // Empty key should not resolve
        assert_eq!(resolve_api_key(&config), None);
        // from_config should error
        assert!(ApiClient::from_config(&config).is_err());
    }

    #[test]
    fn test_security_api_key_not_in_anthropic_messages() {
        // Ensure the API key is NOT injected into the message content
        std::env::set_var("TEST_SEC_MSG_KEY", "sk-secret-should-not-appear");
        let config = ApiConfig {
            enabled: true,
            provider: "anthropic".into(),
            api_key: None,
            api_key_env: Some("TEST_SEC_MSG_KEY".into()),
            model: "test".into(),
            base_url: None,
            max_tokens: 4096,
        };
        let client = ApiClient::from_config(&config).unwrap();

        let request = ChatRequest {
            messages: vec![
                Message {
                    role: Role::User,
                    content: "Tell me a joke".into(),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            temperature: 0.3,
            max_tokens: None,
            model_id: None,
        };

        let (system, messages) = client.convert_to_anthropic(&request);
        // API key must not appear in system prompt or any message content
        let all_json = serde_json::to_string(&messages).unwrap();
        assert!(!all_json.contains("sk-secret-should-not-appear"));
        if let Some(sys) = &system {
            assert!(!sys.contains("sk-secret-should-not-appear"));
        }

        std::env::remove_var("TEST_SEC_MSG_KEY");
    }

    #[test]
    fn test_config_roundtrip_with_api() {
        let toml_str = r#"
[api]
enabled = true
provider = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-20250514"
max_tokens = 8192
"#;
        let config: crate::config::Config = toml::from_str(toml_str).unwrap();
        assert!(config.api.enabled);
        assert_eq!(config.api.provider, "anthropic");
        assert_eq!(config.api.model, "claude-sonnet-4-20250514");
        assert_eq!(config.api.max_tokens, 8192);

        // Round-trip
        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: crate::config::Config = toml::from_str(&serialized).unwrap();
        assert!(reparsed.api.enabled);
        assert_eq!(reparsed.api.model, config.api.model);
    }
}
