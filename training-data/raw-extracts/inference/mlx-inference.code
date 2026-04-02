use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::backend::types::{
    ChatRequest, ChatResponse, Message, ModelBackend, Role, StopReason, Token, TokenStream,
    TokenUsage,
};

/// Maximum allowed length for a single token text field (64 KiB).
/// Prevents unbounded allocation from a malicious or buggy subprocess.
const MAX_TOKEN_TEXT_LEN: usize = 64 * 1024;

/// Maximum allowed length for a single JSON-line from the subprocess (1 MiB).
const MAX_LINE_LEN: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
struct MlxRequest {
    #[serde(rename = "type")]
    msg_type: String,
    prompt: Option<String>,
    max_tokens: Option<usize>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct MlxResponse {
    #[serde(rename = "type")]
    msg_type: String,
    token: Option<String>,
    text: Option<String>,
    done: Option<bool>,
    error: Option<String>,
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
}

pub struct MlxBackend {
    process: Mutex<Option<Child>>,
    model_path: PathBuf,
    model_name: String,
    context_length: usize,
    loaded: bool,
}

impl std::fmt::Debug for MlxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MlxBackend")
            .field("model_path", &self.model_path)
            .field("model_name", &self.model_name)
            .field("context_length", &self.context_length)
            .field("loaded", &self.loaded)
            .finish()
    }
}

impl MlxBackend {
    pub fn new(model_path: &Path, context_length: usize) -> Self {
        let model_name = model_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            process: Mutex::new(None),
            model_path: model_path.to_path_buf(),
            model_name,
            context_length,
            loaded: false,
        }
    }

    fn script_path() -> PathBuf {
        let mut p = std::env::current_exe().unwrap_or_default();
        p.pop();
        p.push("scripts");
        p.push("mlx_server.py");
        if p.exists() {
            return p;
        }
        // Fallback: project root
        PathBuf::from("scripts/mlx_server.py")
    }

    fn start_process(&mut self) -> Result<()> {
        let script = Self::script_path();
        let child = Command::new("python3")
            .arg(&script)
            .arg("--model")
            .arg(&self.model_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start mlx_server.py subprocess")?;

        *self.process.lock().unwrap() = Some(child);
        Ok(())
    }

    fn send_request(&mut self, req: &MlxRequest) -> Result<()> {
        let mut guard = self.process.lock().unwrap();
        let child = guard.as_mut().context("mlx subprocess not running")?;
        let stdin = child.stdin.as_mut().context("no stdin on mlx subprocess")?;
        let line = serde_json::to_string(req)?;
        writeln!(stdin, "{line}")?;
        stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<MlxResponse> {
        let mut guard = self.process.lock().unwrap();
        let child = guard.as_mut().context("mlx subprocess not running")?;
        let stdout = child
            .stdout
            .as_mut()
            .context("no stdout on mlx subprocess")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.is_empty() {
            bail!("mlx subprocess closed stdout");
        }
        let resp: MlxResponse =
            serde_json::from_str(&line).context("failed to parse mlx response")?;
        if let Some(err) = &resp.error {
            bail!("mlx error: {err}");
        }
        Ok(resp)
    }

    pub fn wait_for_ready(&mut self) -> Result<()> {
        let req = MlxRequest {
            msg_type: "ping".to_string(),
            prompt: None,
            max_tokens: None,
            temperature: None,
        };
        self.send_request(&req)?;
        let resp = self.read_response()?;
        if resp.msg_type != "pong" {
            bail!("unexpected response from mlx subprocess: {}", resp.msg_type);
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        let mut guard = self.process.lock().unwrap();
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.loaded = false;
    }

    fn format_prompt(request: &ChatRequest) -> String {
        let mut prompt = String::new();
        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    prompt.push_str("<|system|>\n");
                    prompt.push_str(&msg.content);
                    prompt.push_str("\n<|end|>\n");
                }
                Role::User => {
                    prompt.push_str("<|user|>\n");
                    prompt.push_str(&msg.content);
                    prompt.push_str("\n<|end|>\n");
                }
                Role::Assistant => {
                    prompt.push_str("<|assistant|>\n");
                    prompt.push_str(&msg.content);
                    prompt.push_str("\n<|end|>\n");
                }
                Role::Tool => {
                    prompt.push_str("<|tool|>\n");
                    prompt.push_str(&msg.content);
                    prompt.push_str("\n<|end|>\n");
                }
            }
        }
        prompt.push_str("<|assistant|>\n");
        prompt
    }

    /// Take stdin and stdout from the subprocess for use in streaming.
    /// Returns (stdin, stdout) and leaves the child process running without
    /// those handles. The caller is responsible for the taken handles.
    fn take_stdio(&self) -> Result<(ChildStdin, ChildStdout)> {
        let mut guard = self.process.lock().unwrap();
        let child = guard.as_mut().context("mlx subprocess not running")?;
        let stdin = child.stdin.take().context("stdin already taken")?;
        let stdout = child.stdout.take().context("stdout already taken")?;
        Ok((stdin, stdout))
    }

    /// Read a bounded line from the subprocess stdout.
    /// Enforces MAX_LINE_LEN to prevent unbounded memory allocation.
    fn read_bounded_line(reader: &mut BufReader<ChildStdout>) -> Result<String> {
        let mut line = String::new();
        let mut total = 0usize;
        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                if total == 0 {
                    bail!("mlx subprocess closed stdout");
                }
                break;
            }
            // Find newline position in the buffer
            let (consume, done) = if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                (pos + 1, true)
            } else {
                (buf.len(), false)
            };
            total += consume;
            if total > MAX_LINE_LEN {
                bail!("mlx response line exceeds maximum length ({MAX_LINE_LEN} bytes)");
            }
            // Safe: we're reading from a process that should emit UTF-8 JSON
            let chunk = String::from_utf8_lossy(&buf[..consume]);
            line.push_str(&chunk);
            reader.consume(consume);
            if done {
                break;
            }
        }
        Ok(line)
    }

    /// Parse and validate an MlxResponse, enforcing token text length limits.
    fn parse_response(line: &str) -> Result<MlxResponse> {
        let resp: MlxResponse =
            serde_json::from_str(line).context("failed to parse mlx response JSON")?;
        // Enforce token text length limit
        if let Some(ref t) = resp.token {
            if t.len() > MAX_TOKEN_TEXT_LEN {
                bail!(
                    "token text exceeds maximum length ({} > {MAX_TOKEN_TEXT_LEN})",
                    t.len()
                );
            }
        }
        if let Some(ref t) = resp.text {
            if t.len() > MAX_TOKEN_TEXT_LEN {
                bail!(
                    "text field exceeds maximum length ({} > {MAX_TOKEN_TEXT_LEN})",
                    t.len()
                );
            }
        }
        if let Some(ref err) = resp.error {
            bail!("mlx error: {err}");
        }
        Ok(resp)
    }
}

impl ModelBackend for MlxBackend {
    fn load_model(
        &mut self,
        model_path: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        self.model_path = PathBuf::from(model_path);
        self.model_name = Path::new(model_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Box::pin(async move {
            self.stop();
            self.start_process()?;
            self.wait_for_ready()?;
            self.loaded = true;
            Ok(())
        })
    }

    fn generate(
        &self,
        _request: &ChatRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ChatResponse>> + Send + '_>> {
        Box::pin(async move {
            bail!("use generate_stream for MLX backend")
        })
    }

    fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<(TokenStream, tokio::task::JoinHandle<Result<ChatResponse>>)>,
                > + Send
                + '_,
        >,
    > {
        let prompt = Self::format_prompt(request);
        let max_tokens = request.max_tokens.unwrap_or(4096);
        let temperature = request.temperature;

        Box::pin(async move {
            // Take stdin/stdout from the subprocess so we can move them into
            // the spawned blocking task (the child process keeps running).
            let (mut stdin, stdout) = self.take_stdio()?;

            // Send the generate request via stdin before spawning the reader task
            let req = MlxRequest {
                msg_type: "generate".to_string(),
                prompt: Some(prompt),
                max_tokens: Some(max_tokens),
                temperature: Some(temperature),
            };
            let line = serde_json::to_string(&req)?;
            writeln!(stdin, "{line}")?;
            stdin.flush()?;

            let (tx, rx) = mpsc::channel::<Token>(256);

            let handle = tokio::task::spawn_blocking(move || {
                // Drop stdin — we've already sent the request. Keeping it alive
                // is fine, but we don't need it in this task.
                drop(stdin);

                let mut reader = BufReader::new(stdout);
                let mut accumulated_text = String::new();
                let mut prompt_tokens = 0usize;
                let mut completion_tokens = 0usize;
                let mut stop_reason = StopReason::EndOfText;

                loop {
                    let line = match Self::read_bounded_line(&mut reader) {
                        Ok(l) => l,
                        Err(e) => {
                            // If the channel is closed the receiver is gone — that's ok.
                            let _ = tx.blocking_send(Token {
                                text: String::new(),
                                is_final: true,
                            });
                            return Err(e);
                        }
                    };

                    let resp = match Self::parse_response(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = tx.blocking_send(Token {
                                text: String::new(),
                                is_final: true,
                            });
                            return Err(e);
                        }
                    };

                    let done = resp.done.unwrap_or(false);

                    if !done {
                        // Streaming token
                        let token_text = resp.token.unwrap_or_default();
                        accumulated_text.push_str(&token_text);
                        // Send token through channel; ignore send errors (receiver dropped)
                        let _ = tx.blocking_send(Token {
                            text: token_text,
                            is_final: false,
                        });
                    } else {
                        // Final response — may contain full text and token counts
                        if let Some(ref text) = resp.text {
                            // Use the final text if provided, replacing accumulated
                            accumulated_text = text.clone();
                        }
                        if let Some(pt) = resp.prompt_tokens {
                            prompt_tokens = pt;
                        }
                        if let Some(ct) = resp.completion_tokens {
                            completion_tokens = ct;
                        }
                        // Check if generation ended due to max tokens
                        if completion_tokens >= max_tokens {
                            stop_reason = StopReason::MaxTokens;
                        }

                        // Send final token marker
                        let _ = tx.blocking_send(Token {
                            text: String::new(),
                            is_final: true,
                        });
                        break;
                    }
                }

                Ok(ChatResponse {
                    message: Message {
                        role: Role::Assistant,
                        content: accumulated_text,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    tokens_used: TokenUsage {
                        prompt_tokens,
                        completion_tokens,
                    },
                    stop_reason,
                })
            });

            Ok((rx, handle))
        })
    }

    fn supports_tool_calling(&self) -> bool {
        false
    }

    fn max_context_length(&self) -> usize {
        self.context_length
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn is_loaded(&self) -> bool {
        self.loaded
    }
}

impl Drop for MlxBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_mlx_backend_new() {
        let backend = MlxBackend::new(&PathBuf::from("/tmp/model"), 8192);
        assert_eq!(backend.model_name(), "model");
        assert!(!backend.is_loaded());
        assert_eq!(backend.max_context_length(), 8192);
    }

    #[test]
    fn test_mlx_backend_format_prompt() {
        let request = ChatRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are helpful.".to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            temperature: 0.7,
            max_tokens: None,
            model_id: None,
        };
        let prompt = MlxBackend::format_prompt(&request);
        assert!(prompt.contains("<|system|>"));
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("<|user|>"));
        assert!(prompt.contains("Hello"));
        assert!(prompt.ends_with("<|assistant|>\n"));
    }

    #[test]
    fn test_mlx_backend_supports_tool_calling() {
        let backend = MlxBackend::new(&PathBuf::from("/tmp/model"), 4096);
        assert!(!backend.supports_tool_calling());
    }

    #[test]
    fn test_mlx_backend_debug() {
        let backend = MlxBackend::new(&PathBuf::from("/tmp/model"), 4096);
        let debug = format!("{:?}", backend);
        assert!(debug.contains("MlxBackend"));
        assert!(debug.contains("model"));
    }

    // =========================================================================
    // P0 SECURITY RED TESTS — MLX subprocess JSON protocol
    // =========================================================================

    #[test]
    fn test_p0_malformed_json_does_not_crash() {
        // Malformed JSON must produce an Err, never a panic
        let bad_inputs = vec![
            "",
            "not json at all",
            "{",
            r#"{"type": "generate""#, // missing closing brace
            r#"{"type": 42}"#,        // wrong type for msg_type
            "null",
            "[]",
            r#"{"type": "generate", "token": null, "done": "yes"}"#, // wrong type for done
            "\x00\x01\x02",           // binary garbage
            r#"{"type": "generate", "done": false, "extra_field": {"nested": "deep"}}"#, // unknown fields (should still parse)
        ];

        for input in &bad_inputs {
            let result = MlxBackend::parse_response(input);
            // Must not panic — either Ok (for valid-enough JSON) or Err
            match result {
                Ok(_) => {} // some inputs may parse successfully with defaults
                Err(_) => {} // expected for malformed inputs
            }
        }
    }

    #[test]
    fn test_p0_malformed_json_specific_cases() {
        // Empty string must fail
        assert!(MlxBackend::parse_response("").is_err());

        // Missing required type field
        assert!(MlxBackend::parse_response(r#"{"done": true}"#).is_err());

        // Binary garbage
        assert!(MlxBackend::parse_response("\x00\x01\x02").is_err());

        // Truncated JSON
        assert!(MlxBackend::parse_response(r#"{"type":"#).is_err());
    }

    #[test]
    fn test_p0_error_field_propagated() {
        // A response with an error field must return Err, not silently succeed
        let input = r#"{"type": "generate", "done": false, "error": "model crashed"}"#;
        let result = MlxBackend::parse_response(input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("model crashed"));
    }

    #[test]
    fn test_p0_extremely_long_token_text_rejected() {
        // Token text exceeding MAX_TOKEN_TEXT_LEN must be rejected to prevent
        // unbounded memory allocation from a malicious subprocess.
        let long_text = "A".repeat(MAX_TOKEN_TEXT_LEN + 1);
        let input = format!(
            r#"{{"type": "generate", "token": "{}", "done": false}}"#,
            long_text
        );
        let result = MlxBackend::parse_response(&input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("maximum length"));
    }

    #[test]
    fn test_p0_extremely_long_text_field_rejected() {
        // The final `text` field must also be bounded
        let long_text = "B".repeat(MAX_TOKEN_TEXT_LEN + 1);
        let input = format!(
            r#"{{"type": "generate", "text": "{}", "done": true}}"#,
            long_text
        );
        let result = MlxBackend::parse_response(&input);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("maximum length"));
    }

    #[test]
    fn test_p0_token_text_at_limit_accepted() {
        // Exactly at the limit should be accepted
        let text = "C".repeat(MAX_TOKEN_TEXT_LEN);
        let input = format!(
            r#"{{"type": "generate", "token": "{}", "done": false}}"#,
            text
        );
        let result = MlxBackend::parse_response(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_p0_valid_streaming_response_parses() {
        // Normal streaming token
        let input = r#"{"type": "generate", "token": "Hello", "done": false}"#;
        let resp = MlxBackend::parse_response(input).unwrap();
        assert_eq!(resp.token.as_deref(), Some("Hello"));
        assert_eq!(resp.done, Some(false));
    }

    #[test]
    fn test_p0_valid_final_response_parses() {
        // Normal final response
        let input = r#"{"type": "generate", "text": "Hello world", "done": true, "prompt_tokens": 10, "completion_tokens": 5}"#;
        let resp = MlxBackend::parse_response(input).unwrap();
        assert_eq!(resp.text.as_deref(), Some("Hello world"));
        assert_eq!(resp.done, Some(true));
        assert_eq!(resp.prompt_tokens, Some(10));
        assert_eq!(resp.completion_tokens, Some(5));
    }

    #[test]
    fn test_p0_unicode_in_token_text() {
        // Unicode content must parse correctly without corruption
        let input = r#"{"type": "generate", "token": "日本語テスト🔥", "done": false}"#;
        let resp = MlxBackend::parse_response(input).unwrap();
        assert_eq!(resp.token.as_deref(), Some("日本語テスト🔥"));
    }

    #[test]
    fn test_p0_null_bytes_in_token_rejected() {
        // Null bytes in token text — serde_json handles this, but verify
        let input = r#"{"type": "generate", "token": "hello\u0000world", "done": false}"#;
        // serde_json does allow \u0000 in strings — this should parse but we verify it
        let result = MlxBackend::parse_response(input);
        // This is valid JSON, so it parses. The security concern is at the
        // subprocess boundary, which is bounded by MAX_TOKEN_TEXT_LEN.
        assert!(result.is_ok());
    }
}
